//! Agent CLI commands.

use anyhow::{Context, Result};
use clap::{Args, Subcommand};
use std::sync::Arc;

use crate::adapters::sqlite::{SqliteAgentRepository, initialize_database};
use crate::cli::output::{output, CommandOutput};
use crate::domain::models::{AgentTemplate, AgentTier, ToolCapability};
use crate::domain::ports::AgentFilter;
use crate::services::AgentService;

#[derive(Args, Debug)]
pub struct AgentArgs {
    #[command(subcommand)]
    pub command: AgentCommands,
}

#[derive(Subcommand, Debug)]
pub enum AgentCommands {
    /// Register a new agent template
    Register {
        /// Agent name/type
        name: String,
        /// Agent description
        #[arg(short, long)]
        description: Option<String>,
        /// Agent tier (architect, specialist, worker)
        #[arg(short, long, default_value = "worker")]
        tier: String,
        /// System prompt
        #[arg(short, long)]
        prompt: String,
        /// Tools (format: "name:description")
        #[arg(long)]
        tool: Vec<String>,
        /// Max turns per task
        #[arg(long)]
        max_turns: Option<u32>,
    },
    /// List agent templates
    List {
        /// Filter by tier
        #[arg(short, long)]
        tier: Option<String>,
        /// Show only active templates
        #[arg(long)]
        active_only: bool,
    },
    /// Show agent template details
    Show {
        /// Agent name
        name: String,
        /// Specific version
        #[arg(short, long)]
        version: Option<u32>,
    },
    /// Disable an agent template
    Disable {
        /// Agent name
        name: String,
    },
    /// Enable an agent template
    Enable {
        /// Agent name
        name: String,
    },
    /// Show running instances
    Instances,
    /// Show agent stats
    Stats,
}

#[derive(Debug, serde::Serialize)]
pub struct AgentOutput {
    pub id: String,
    pub name: String,
    pub tier: String,
    pub version: u32,
    pub status: String,
    pub tools_count: usize,
    pub max_turns: u32,
}

impl From<&AgentTemplate> for AgentOutput {
    fn from(agent: &AgentTemplate) -> Self {
        Self {
            id: agent.id.to_string(),
            name: agent.name.clone(),
            tier: agent.tier.as_str().to_string(),
            version: agent.version,
            status: agent.status.as_str().to_string(),
            tools_count: agent.tools.len(),
            max_turns: agent.max_turns,
        }
    }
}

#[derive(Debug, serde::Serialize)]
pub struct AgentListOutput {
    pub agents: Vec<AgentOutput>,
    pub total: usize,
}

impl CommandOutput for AgentListOutput {
    fn to_human(&self) -> String {
        if self.agents.is_empty() {
            return "No agents found.".to_string();
        }

        let mut lines = vec![format!("Found {} agent(s):\n", self.total)];
        lines.push(format!(
            "{:<20} {:<12} {:<4} {:<10} {:<6}",
            "NAME", "TIER", "VER", "STATUS", "TOOLS"
        ));
        lines.push("-".repeat(55));

        for agent in &self.agents {
            lines.push(format!(
                "{:<20} {:<12} {:<4} {:<10} {:<6}",
                truncate(&agent.name, 18),
                agent.tier,
                agent.version,
                agent.status,
                agent.tools_count
            ));
        }

        lines.join("\n")
    }

    fn to_json(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or_default()
    }
}

#[derive(Debug, serde::Serialize)]
pub struct AgentDetailOutput {
    pub agent: AgentOutput,
    pub description: String,
    pub prompt_preview: String,
    pub tools: Vec<String>,
    pub constraints: Vec<String>,
    pub handoff_targets: Vec<String>,
}

impl CommandOutput for AgentDetailOutput {
    fn to_human(&self) -> String {
        let mut lines = vec![
            format!("Agent: {}", self.agent.name),
            format!("ID: {}", self.agent.id),
            format!("Tier: {}", self.agent.tier),
            format!("Version: {}", self.agent.version),
            format!("Status: {}", self.agent.status),
            format!("Max Turns: {}", self.agent.max_turns),
        ];

        if !self.description.is_empty() {
            lines.push(format!("\nDescription: {}", self.description));
        }

        lines.push(format!("\nPrompt:\n{}", self.prompt_preview));

        if !self.tools.is_empty() {
            lines.push(format!("\nTools ({}):", self.tools.len()));
            for tool in &self.tools {
                lines.push(format!("  - {}", tool));
            }
        }

        if !self.constraints.is_empty() {
            lines.push("\nConstraints:".to_string());
            for c in &self.constraints {
                lines.push(format!("  - {}", c));
            }
        }

        if !self.handoff_targets.is_empty() {
            lines.push(format!("\nHandoff targets: {}", self.handoff_targets.join(", ")));
        }

        lines.join("\n")
    }

    fn to_json(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or_default()
    }
}

#[derive(Debug, serde::Serialize)]
pub struct AgentActionOutput {
    pub success: bool,
    pub message: String,
    pub agent: Option<AgentOutput>,
}

impl CommandOutput for AgentActionOutput {
    fn to_human(&self) -> String {
        self.message.clone()
    }

    fn to_json(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or_default()
    }
}

#[derive(Debug, serde::Serialize)]
pub struct InstancesOutput {
    pub instances: Vec<InstanceInfo>,
    pub total: usize,
}

#[derive(Debug, serde::Serialize)]
pub struct InstanceInfo {
    pub template: String,
    pub count: u32,
    pub max: u32,
}

impl CommandOutput for InstancesOutput {
    fn to_human(&self) -> String {
        if self.instances.is_empty() {
            return "No running instances.".to_string();
        }

        let mut lines = vec!["Running agent instances:\n".to_string()];
        lines.push(format!("{:<20} {:<10} {:<10}", "TEMPLATE", "RUNNING", "MAX"));
        lines.push("-".repeat(40));

        for inst in &self.instances {
            lines.push(format!("{:<20} {:<10} {:<10}", inst.template, inst.count, inst.max));
        }

        lines.push(format!("\nTotal: {} running", self.total));
        lines.join("\n")
    }

    fn to_json(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or_default()
    }
}

#[derive(Debug, serde::Serialize)]
pub struct StatsOutput {
    pub architect_count: usize,
    pub specialist_count: usize,
    pub worker_count: usize,
    pub total: usize,
    pub running_instances: u32,
}

impl CommandOutput for StatsOutput {
    fn to_human(&self) -> String {
        let mut lines = vec!["Agent Statistics:".to_string()];
        lines.push(format!("  Architects:   {}", self.architect_count));
        lines.push(format!("  Specialists:  {}", self.specialist_count));
        lines.push(format!("  Workers:      {}", self.worker_count));
        lines.push("  ------------".to_string());
        lines.push(format!("  Total:        {}", self.total));
        lines.push(format!("\n  Running instances: {}", self.running_instances));
        lines.join("\n")
    }

    fn to_json(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or_default()
    }
}

pub async fn execute(args: AgentArgs, json_mode: bool) -> Result<()> {
    let pool = initialize_database("sqlite:.abathur/abathur.db")
        .await
        .context("Failed to initialize database. Run 'abathur init' first.")?;

    let repo = Arc::new(SqliteAgentRepository::new(pool));
    let service = AgentService::new(repo.clone());

    match args.command {
        AgentCommands::Register { name, description, tier, prompt, tool, max_turns } => {
            let tier = AgentTier::parse_str(&tier)
                .ok_or_else(|| anyhow::anyhow!("Invalid tier: {}", tier))?;

            let tools: Vec<ToolCapability> = tool
                .iter()
                .filter_map(|t| {
                    let parts: Vec<&str> = t.splitn(2, ':').collect();
                    if parts.len() == 2 {
                        Some(ToolCapability::new(parts[0], parts[1]))
                    } else {
                        None
                    }
                })
                .collect();

            let agent = service.register_template(
                name,
                description.unwrap_or_default(),
                tier,
                prompt,
                tools,
                vec![],
                max_turns,
            ).await?;

            let out = AgentActionOutput {
                success: true,
                message: format!("Agent registered: {} (version {})", agent.name, agent.version),
                agent: Some(AgentOutput::from(&agent)),
            };
            output(&out, json_mode);
        }

        AgentCommands::List { tier, active_only } => {
            let filter = AgentFilter {
                tier: tier.as_ref().and_then(|t| AgentTier::parse_str(t)),
                status: if active_only {
                    Some(crate::domain::models::AgentStatus::Active)
                } else {
                    None
                },
                ..Default::default()
            };

            let agents = service.list_templates(filter).await?;

            let out = AgentListOutput {
                total: agents.len(),
                agents: agents.iter().map(AgentOutput::from).collect(),
            };
            output(&out, json_mode);
        }

        AgentCommands::Show { name, version } => {
            let agent = if let Some(v) = version {
                service.get_template_version(&name, v).await?
            } else {
                service.get_template(&name).await?
            };

            match agent {
                Some(a) => {
                    let out = AgentDetailOutput {
                        agent: AgentOutput::from(&a),
                        description: a.description.clone(),
                        prompt_preview: truncate(&a.system_prompt, 500),
                        tools: a.tools.iter().map(|t| format!("{}: {}", t.name, t.description)).collect(),
                        constraints: a.constraints.iter().map(|c| format!("{}: {}", c.name, c.description)).collect(),
                        handoff_targets: a.agent_card.handoff_targets.clone(),
                    };
                    output(&out, json_mode);
                }
                None => {
                    let out = AgentActionOutput {
                        success: false,
                        message: format!("Agent not found: {}", name),
                        agent: None,
                    };
                    output(&out, json_mode);
                }
            }
        }

        AgentCommands::Disable { name } => {
            let agent = service.disable_template(&name).await?;

            let out = AgentActionOutput {
                success: true,
                message: format!("Agent disabled: {}", agent.name),
                agent: Some(AgentOutput::from(&agent)),
            };
            output(&out, json_mode);
        }

        AgentCommands::Enable { name } => {
            let agent = service.enable_template(&name).await?;

            let out = AgentActionOutput {
                success: true,
                message: format!("Agent enabled: {}", agent.name),
                agent: Some(AgentOutput::from(&agent)),
            };
            output(&out, json_mode);
        }

        AgentCommands::Instances => {
            let counts = service.get_instance_counts().await?;
            let templates = service.list_templates(AgentFilter::default()).await?;

            let instances: Vec<InstanceInfo> = templates
                .iter()
                .filter_map(|t| {
                    let count = *counts.get(&t.name).unwrap_or(&0);
                    if count > 0 {
                        Some(InstanceInfo {
                            template: t.name.clone(),
                            count,
                            max: t.tier.max_instances(),
                        })
                    } else {
                        None
                    }
                })
                .collect();

            let total: u32 = instances.iter().map(|i| i.count).sum();

            let out = InstancesOutput {
                total: total as usize,
                instances,
            };
            output(&out, json_mode);
        }

        AgentCommands::Stats => {
            let templates = service.list_templates(AgentFilter::default()).await?;
            let counts = service.get_instance_counts().await?;
            let running: u32 = counts.values().sum();

            let architects = templates.iter().filter(|t| t.tier == AgentTier::Architect).count();
            let specialists = templates.iter().filter(|t| t.tier == AgentTier::Specialist).count();
            let workers = templates.iter().filter(|t| t.tier == AgentTier::Worker).count();

            let out = StatsOutput {
                architect_count: architects,
                specialist_count: specialists,
                worker_count: workers,
                total: templates.len(),
                running_instances: running,
            };
            output(&out, json_mode);
        }
    }

    Ok(())
}

fn truncate(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        format!("{}...", &s[..max_len - 3])
    }
}
