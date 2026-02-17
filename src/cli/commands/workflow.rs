//! Workflow template CLI commands.

use anyhow::{Context, Result};
use clap::{Args, Subcommand};

use crate::cli::output::{output, CommandOutput};
use crate::services::config::Config;

#[derive(Args, Debug)]
pub struct WorkflowArgs {
    #[command(subcommand)]
    pub command: WorkflowCommands,
}

#[derive(Subcommand, Debug)]
pub enum WorkflowCommands {
    /// List available workflow templates
    List,
    /// Show details of a specific workflow
    Show {
        /// Workflow name (e.g., "code", "analysis")
        name: String,
    },
    /// Validate all configured workflow templates
    Validate,
}

// ── Output structs ──────────────────────────────────────────────────────

#[derive(Debug, serde::Serialize)]
struct WorkflowSummary {
    name: String,
    description: String,
    phase_count: usize,
    source: String,
    is_default: bool,
}

#[derive(Debug, serde::Serialize)]
struct WorkflowListOutput {
    workflows: Vec<WorkflowSummary>,
    default_workflow: String,
}

impl CommandOutput for WorkflowListOutput {
    fn to_human(&self) -> String {
        let mut lines = vec!["Available workflows:".to_string()];
        for wf in &self.workflows {
            let default_marker = if wf.is_default { " (default)" } else { "" };
            lines.push(format!(
                "  {} — {} [{} phases, {}]{}",
                wf.name, wf.description, wf.phase_count, wf.source, default_marker
            ));
        }
        lines.join("\n")
    }

    fn to_json(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or_default()
    }
}

#[derive(Debug, serde::Serialize)]
struct PhaseDetail {
    name: String,
    description: String,
    role: String,
    tools: Vec<String>,
    read_only: bool,
    dependency: String,
}

#[derive(Debug, serde::Serialize)]
struct WorkflowDetailOutput {
    name: String,
    description: String,
    phases: Vec<PhaseDetail>,
}

impl CommandOutput for WorkflowDetailOutput {
    fn to_human(&self) -> String {
        let mut lines = vec![
            format!("Workflow: {}", self.name),
            format!("Description: {}", self.description),
            format!("Phases ({}):", self.phases.len()),
        ];

        for (i, phase) in self.phases.iter().enumerate() {
            lines.push(format!(
                "\n  {}. {} ({})",
                i + 1,
                phase.name,
                phase.dependency
            ));
            lines.push(format!("     {}", phase.description));
            lines.push(format!("     Role: {}", phase.role));
            lines.push(format!("     Tools: {}", phase.tools.join(", ")));
            if phase.read_only {
                lines.push("     Read-only: yes".to_string());
            }
        }

        lines.join("\n")
    }

    fn to_json(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or_default()
    }
}

#[derive(Debug, serde::Serialize)]
struct ValidationResult {
    name: String,
    valid: bool,
    error: Option<String>,
}

#[derive(Debug, serde::Serialize)]
struct ValidateOutput {
    results: Vec<ValidationResult>,
    all_valid: bool,
}

impl CommandOutput for ValidateOutput {
    fn to_human(&self) -> String {
        let mut lines = vec!["Workflow validation:".to_string()];
        for r in &self.results {
            let status = if r.valid { "OK" } else { "FAIL" };
            let mut line = format!("  {} — {}", r.name, status);
            if let Some(err) = &r.error {
                line.push_str(&format!(": {}", err));
            }
            lines.push(line);
        }
        if self.all_valid {
            lines.push("\nAll workflows are valid.".to_string());
        } else {
            lines.push("\nSome workflows have errors.".to_string());
        }
        lines.join("\n")
    }

    fn to_json(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or_default()
    }
}

// ── Command execution ───────────────────────────────────────────────────

pub async fn execute(args: WorkflowArgs, json_mode: bool) -> Result<()> {
    match args.command {
        WorkflowCommands::List => list_workflows(json_mode),
        WorkflowCommands::Show { name } => show_workflow(&name, json_mode),
        WorkflowCommands::Validate => validate_workflows(json_mode),
    }
}

fn list_workflows(json_mode: bool) -> Result<()> {
    let config = Config::load()
        .context("Failed to load configuration")?;

    let available = config.available_workflows();

    let workflows: Vec<WorkflowSummary> = available
        .into_iter()
        .map(|(name, description, phase_count, is_default)| {
            let source = if name == "code"
                && !config.workflows.iter().any(|wf| wf.name == "code")
            {
                "built-in".to_string()
            } else {
                "abathur.toml".to_string()
            };
            WorkflowSummary {
                name,
                description,
                phase_count,
                source,
                is_default,
            }
        })
        .collect();

    let out = WorkflowListOutput {
        default_workflow: config.default_workflow.clone(),
        workflows,
    };
    output(&out, json_mode);
    Ok(())
}

fn show_workflow(name: &str, json_mode: bool) -> Result<()> {
    let config = Config::load()
        .context("Failed to load configuration")?;

    let wf = config
        .resolve_workflow(name)
        .ok_or_else(|| anyhow::anyhow!("Workflow '{}' not found", name))?;

    let phases: Vec<PhaseDetail> = wf
        .phases
        .iter()
        .map(|p| PhaseDetail {
            name: p.name.clone(),
            description: p.description.clone(),
            role: p.role.clone(),
            tools: p.tools.clone(),
            read_only: p.read_only,
            dependency: format!("{:?}", p.dependency),
        })
        .collect();

    let out = WorkflowDetailOutput {
        name: wf.name,
        description: wf.description,
        phases,
    };
    output(&out, json_mode);
    Ok(())
}

fn validate_workflows(json_mode: bool) -> Result<()> {
    let config = Config::load()
        .context("Failed to load configuration")?;

    let mut results = Vec::new();

    // Validate the built-in "code" workflow
    let code_wf = crate::domain::models::workflow_template::WorkflowTemplate::default_code_workflow();
    let valid = code_wf.validate();
    results.push(ValidationResult {
        name: code_wf.name,
        valid: valid.is_ok(),
        error: valid.err(),
    });

    // Validate each user-defined workflow
    for wf in &config.workflows {
        let valid = wf.validate();
        results.push(ValidationResult {
            name: wf.name.clone(),
            valid: valid.is_ok(),
            error: valid.err(),
        });
    }

    let all_valid = results.iter().all(|r| r.valid);
    let out = ValidateOutput { results, all_valid };
    output(&out, json_mode);
    Ok(())
}
