//! Agent CLI commands.

use anyhow::{Context, Result};
use clap::{Args, Subcommand};
use sqlx::SqlitePool;
use std::sync::Arc;
use uuid::Uuid;

use crate::adapters::sqlite::{initialize_database, SqliteAgentRepository};
use crate::cli::id_resolver::resolve_task_id;
use crate::cli::output::{output, CommandOutput};
use crate::domain::models::a2a::{A2AAgentCard, A2AMessage, MessageType};
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
    /// Send an A2A message to an agent
    Send {
        /// Target agent ID
        #[arg(short, long)]
        to: String,
        /// Message type (handoff, delegate, progress, completion, error)
        #[arg(short = 'm', long, default_value = "delegate")]
        message_type: String,
        /// Message subject
        #[arg(short, long)]
        subject: String,
        /// Message body
        body: String,
        /// Sender agent ID (defaults to "cli")
        #[arg(short, long, default_value = "cli")]
        from: String,
        /// Related task ID
        #[arg(long)]
        task_id: Option<String>,
        /// A2A gateway URL
        #[arg(long, default_value = "http://127.0.0.1:8080")]
        gateway: String,
    },
    /// Show A2A gateway status
    GatewayStatus {
        /// A2A gateway URL
        #[arg(long, default_value = "http://127.0.0.1:8080")]
        gateway: String,
    },
    /// Manage agent cards
    Cards {
        #[command(subcommand)]
        command: CardsCommands,
    },
}

#[derive(Subcommand, Debug)]
pub enum CardsCommands {
    /// List all agent cards
    List {
        /// A2A gateway URL
        #[arg(long, default_value = "http://127.0.0.1:8080")]
        gateway: String,
    },
    /// Export agent cards for federation
    Export {
        /// Output file (defaults to stdout)
        #[arg(short, long)]
        output: Option<String>,
        /// Format (json, yaml)
        #[arg(short, long, default_value = "json")]
        format: String,
        /// A2A gateway URL
        #[arg(long, default_value = "http://127.0.0.1:8080")]
        gateway: String,
    },
    /// Show a specific agent card
    Show {
        /// Agent ID
        agent_id: String,
        /// A2A gateway URL
        #[arg(long, default_value = "http://127.0.0.1:8080")]
        gateway: String,
    },
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

#[derive(Debug, serde::Serialize)]
pub struct A2AMessageOutput {
    pub success: bool,
    pub message_id: String,
    pub message: String,
}

impl CommandOutput for A2AMessageOutput {
    fn to_human(&self) -> String {
        if self.success {
            format!("Message sent successfully (ID: {})", self.message_id)
        } else {
            format!("Failed to send message: {}", self.message)
        }
    }

    fn to_json(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or_default()
    }
}

#[derive(Debug, serde::Serialize)]
pub struct GatewayStatusOutput {
    pub running: bool,
    pub url: String,
    pub agents: usize,
    pub message: String,
}

impl CommandOutput for GatewayStatusOutput {
    fn to_human(&self) -> String {
        if self.running {
            format!(
                "A2A Gateway Status:\n  URL: {}\n  Status: RUNNING\n  Registered agents: {}",
                self.url, self.agents
            )
        } else {
            format!(
                "A2A Gateway Status:\n  URL: {}\n  Status: NOT RUNNING\n  {}",
                self.url, self.message
            )
        }
    }

    fn to_json(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or_default()
    }
}

#[derive(Debug, serde::Serialize)]
pub struct AgentCardOutput {
    pub agent_id: String,
    pub display_name: String,
    pub description: String,
    pub tier: String,
    pub capabilities: Vec<String>,
    pub available: bool,
}

impl From<&A2AAgentCard> for AgentCardOutput {
    fn from(card: &A2AAgentCard) -> Self {
        Self {
            agent_id: card.agent_id.clone(),
            display_name: card.display_name.clone(),
            description: card.description.clone(),
            tier: card.tier.clone(),
            capabilities: card.capabilities.clone(),
            available: card.available,
        }
    }
}

#[derive(Debug, serde::Serialize)]
pub struct CardsListOutput {
    pub cards: Vec<AgentCardOutput>,
    pub total: usize,
}

impl CommandOutput for CardsListOutput {
    fn to_human(&self) -> String {
        if self.cards.is_empty() {
            return "No agent cards found.".to_string();
        }

        let mut lines = vec![format!("Found {} agent card(s):\n", self.total)];
        lines.push(format!(
            "{:<25} {:<20} {:<12} {:<10}",
            "AGENT ID", "DISPLAY NAME", "TIER", "AVAILABLE"
        ));
        lines.push("-".repeat(70));

        for card in &self.cards {
            lines.push(format!(
                "{:<25} {:<20} {:<12} {:<10}",
                truncate(&card.agent_id, 23),
                truncate(&card.display_name, 18),
                card.tier,
                if card.available { "Yes" } else { "No" }
            ));
        }

        lines.join("\n")
    }

    fn to_json(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or_default()
    }
}

#[derive(Debug, serde::Serialize)]
pub struct CardDetailOutput {
    pub card: AgentCardOutput,
}

impl CommandOutput for CardDetailOutput {
    fn to_human(&self) -> String {
        let mut lines = vec![
            format!("Agent Card: {}", self.card.agent_id),
            format!("Display Name: {}", self.card.display_name),
            format!("Tier: {}", self.card.tier),
            format!("Available: {}", if self.card.available { "Yes" } else { "No" }),
        ];

        if !self.card.description.is_empty() {
            lines.push(format!("\nDescription:\n  {}", self.card.description));
        }

        if !self.card.capabilities.is_empty() {
            lines.push(format!("\nCapabilities ({}):", self.card.capabilities.len()));
            for cap in &self.card.capabilities {
                lines.push(format!("  - {}", cap));
            }
        }

        lines.join("\n")
    }

    fn to_json(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or_default()
    }
}

#[derive(Debug, serde::Serialize)]
pub struct ExportOutput {
    pub success: bool,
    pub message: String,
    pub cards_count: usize,
}

impl CommandOutput for ExportOutput {
    fn to_human(&self) -> String {
        self.message.clone()
    }

    fn to_json(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or_default()
    }
}

pub async fn execute(args: AgentArgs, json_mode: bool) -> Result<()> {
    let pool = initialize_database("sqlite:.abathur/abathur.db")
        .await
        .context("Failed to initialize database. Run 'abathur init' first.")?;

    let repo = Arc::new(SqliteAgentRepository::new(pool.clone()));
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

        AgentCommands::Send {
            to,
            message_type,
            subject,
            body,
            from,
            task_id,
            gateway,
        } => {
            send_a2a_message(&pool, to, message_type, subject, body, from, task_id, gateway, json_mode).await?;
        }

        AgentCommands::GatewayStatus { gateway } => {
            check_gateway_status(gateway, json_mode).await?;
        }

        AgentCommands::Cards { command } => {
            handle_cards_command(command, json_mode).await?;
        }
    }

    Ok(())
}

fn parse_message_type(s: &str) -> Option<MessageType> {
    match s.to_lowercase().as_str() {
        "handoff" | "handoff_request" => Some(MessageType::HandoffRequest),
        "handoff_accept" => Some(MessageType::HandoffAccept),
        "handoff_reject" => Some(MessageType::HandoffReject),
        "delegate" | "delegate_task" => Some(MessageType::DelegateTask),
        "progress" | "progress_report" => Some(MessageType::ProgressReport),
        "assistance" | "assistance_request" => Some(MessageType::AssistanceRequest),
        "assistance_response" => Some(MessageType::AssistanceResponse),
        "completion" | "completion_notify" => Some(MessageType::CompletionNotify),
        "error" | "error_report" => Some(MessageType::ErrorReport),
        _ => None,
    }
}

async fn send_a2a_message(
    pool: &SqlitePool,
    to: String,
    message_type: String,
    subject: String,
    body: String,
    from: String,
    task_id: Option<String>,
    gateway: String,
    json_mode: bool,
) -> Result<()> {
    let msg_type = parse_message_type(&message_type)
        .ok_or_else(|| anyhow::anyhow!("Invalid message type: {}", message_type))?;

    let mut message = A2AMessage::new(msg_type, &from, &to, &subject, &body);

    if let Some(ref tid) = task_id {
        let task_uuid = resolve_task_id(pool, tid).await?;
        message = message.with_task(task_uuid);
    }

    // Build JSON-RPC request
    let request = serde_json::json!({
        "jsonrpc": "2.0",
        "id": Uuid::new_v4().to_string(),
        "method": "tasks/send",
        "params": {
            "id": Uuid::new_v4().to_string(),
            "message": {
                "role": "user",
                "parts": [
                    {
                        "type": "text",
                        "text": body
                    }
                ]
            },
            "metadata": {
                "a2a_message": {
                    "from": from,
                    "to": to,
                    "message_type": message_type,
                    "subject": subject,
                    "message_id": message.id.to_string(),
                    "task_id": task_id
                }
            }
        }
    });

    // Send to gateway
    let client = reqwest::Client::new();
    let response = client
        .post(&format!("{}/rpc", gateway))
        .header("Content-Type", "application/json")
        .json(&request)
        .send()
        .await;

    match response {
        Ok(resp) => {
            if resp.status().is_success() {
                let out = A2AMessageOutput {
                    success: true,
                    message_id: message.id.to_string(),
                    message: format!("Sent {} message to {}", message_type, to),
                };
                output(&out, json_mode);
            } else {
                let error_text = resp.text().await.unwrap_or_default();
                let out = A2AMessageOutput {
                    success: false,
                    message_id: message.id.to_string(),
                    message: format!("Gateway returned error: {}", error_text),
                };
                output(&out, json_mode);
            }
        }
        Err(e) => {
            let out = A2AMessageOutput {
                success: false,
                message_id: message.id.to_string(),
                message: format!("Failed to connect to gateway: {}", e),
            };
            output(&out, json_mode);
        }
    }

    Ok(())
}

async fn check_gateway_status(gateway: String, json_mode: bool) -> Result<()> {
    let client = reqwest::Client::new();
    let health_url = format!("{}/health", gateway);
    let agents_url = format!("{}/agents", gateway);

    let health_response = client.get(&health_url).send().await;

    match health_response {
        Ok(resp) if resp.status().is_success() => {
            // Gateway is running, get agent count
            let agents_response = client.get(&agents_url).send().await;
            let agent_count = match agents_response {
                Ok(r) if r.status().is_success() => {
                    r.json::<Vec<serde_json::Value>>()
                        .await
                        .map(|v| v.len())
                        .unwrap_or(0)
                }
                _ => 0,
            };

            let out = GatewayStatusOutput {
                running: true,
                url: gateway,
                agents: agent_count,
                message: "Gateway is operational".to_string(),
            };
            output(&out, json_mode);
        }
        Ok(resp) => {
            let out = GatewayStatusOutput {
                running: false,
                url: gateway,
                agents: 0,
                message: format!("Gateway returned status: {}", resp.status()),
            };
            output(&out, json_mode);
        }
        Err(e) => {
            let out = GatewayStatusOutput {
                running: false,
                url: gateway,
                agents: 0,
                message: format!("Cannot connect to gateway: {}", e),
            };
            output(&out, json_mode);
        }
    }

    Ok(())
}

async fn handle_cards_command(command: CardsCommands, json_mode: bool) -> Result<()> {
    match command {
        CardsCommands::List { gateway } => {
            let client = reqwest::Client::new();
            let response = client.get(&format!("{}/agents", gateway)).send().await;

            match response {
                Ok(resp) if resp.status().is_success() => {
                    let cards: Vec<A2AAgentCard> = resp.json().await.unwrap_or_default();
                    let out = CardsListOutput {
                        total: cards.len(),
                        cards: cards.iter().map(AgentCardOutput::from).collect(),
                    };
                    output(&out, json_mode);
                }
                Ok(resp) => {
                    let out = AgentActionOutput {
                        success: false,
                        message: format!("Gateway returned error: {}", resp.status()),
                        agent: None,
                    };
                    output(&out, json_mode);
                }
                Err(e) => {
                    let out = AgentActionOutput {
                        success: false,
                        message: format!("Cannot connect to gateway: {}", e),
                        agent: None,
                    };
                    output(&out, json_mode);
                }
            }
        }

        CardsCommands::Export {
            output: output_file,
            format,
            gateway,
        } => {
            let client = reqwest::Client::new();
            let response = client.get(&format!("{}/agents", gateway)).send().await;

            match response {
                Ok(resp) if resp.status().is_success() => {
                    let cards: Vec<A2AAgentCard> = resp.json().await.unwrap_or_default();

                    let content = match format.to_lowercase().as_str() {
                        "yaml" | "yml" => {
                            serde_yaml::to_string(&cards).context("Failed to serialize to YAML")?
                        }
                        _ => {
                            serde_json::to_string_pretty(&cards)
                                .context("Failed to serialize to JSON")?
                        }
                    };

                    if let Some(path) = output_file {
                        std::fs::write(&path, &content)
                            .context(format!("Failed to write to {}", path))?;
                        let out = ExportOutput {
                            success: true,
                            message: format!("Exported {} agent cards to {}", cards.len(), path),
                            cards_count: cards.len(),
                        };
                        output(&out, json_mode);
                    } else {
                        // Print to stdout
                        println!("{}", content);
                    }
                }
                Ok(resp) => {
                    let out = ExportOutput {
                        success: false,
                        message: format!("Gateway returned error: {}", resp.status()),
                        cards_count: 0,
                    };
                    output(&out, json_mode);
                }
                Err(e) => {
                    let out = ExportOutput {
                        success: false,
                        message: format!("Cannot connect to gateway: {}", e),
                        cards_count: 0,
                    };
                    output(&out, json_mode);
                }
            }
        }

        CardsCommands::Show { agent_id, gateway } => {
            let client = reqwest::Client::new();
            let response = client
                .get(&format!("{}/agents/{}", gateway, agent_id))
                .send()
                .await;

            match response {
                Ok(resp) if resp.status().is_success() => {
                    let card: A2AAgentCard = resp.json().await.context("Failed to parse agent card")?;
                    let out = CardDetailOutput {
                        card: AgentCardOutput::from(&card),
                    };
                    output(&out, json_mode);
                }
                Ok(resp) if resp.status() == reqwest::StatusCode::NOT_FOUND => {
                    let out = AgentActionOutput {
                        success: false,
                        message: format!("Agent not found: {}", agent_id),
                        agent: None,
                    };
                    output(&out, json_mode);
                }
                Ok(resp) => {
                    let out = AgentActionOutput {
                        success: false,
                        message: format!("Gateway returned error: {}", resp.status()),
                        agent: None,
                    };
                    output(&out, json_mode);
                }
                Err(e) => {
                    let out = AgentActionOutput {
                        success: false,
                        message: format!("Cannot connect to gateway: {}", e),
                        agent: None,
                    };
                    output(&out, json_mode);
                }
            }
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
