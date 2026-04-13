//! Workflow template CLI commands.

use anyhow::{Context, Result};
use clap::{Args, Subcommand};

use crate::cli::display::{
    list_table, output, render_list, truncate_ellipsis, CommandOutput, DetailView,
};
use crate::domain::models::workflow_template::WorkflowTemplate;
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
    /// Export a workflow template to YAML
    Export {
        /// Workflow name to export
        name: String,
        /// Output directory (defaults to configured workflows_dir)
        #[arg(short, long)]
        output: Option<String>,
    },
    /// Export all configured workflow templates to YAML
    ExportAll {
        /// Output directory (defaults to configured workflows_dir)
        #[arg(short, long)]
        output: Option<String>,
    },
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
        if self.workflows.is_empty() {
            return "No workflows found.".to_string();
        }

        let mut table = list_table(&["Name", "Phases", "Source", "Description"]);

        for wf in &self.workflows {
            let name = if wf.is_default {
                format!("{} (default)", wf.name)
            } else {
                wf.name.clone()
            };
            table.add_row(vec![
                name,
                wf.phase_count.to_string(),
                wf.source.clone(),
                truncate_ellipsis(&wf.description, 50),
            ]);
        }

        render_list("workflow", table, self.workflows.len())
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
        let mut view = DetailView::new(&self.name)
            .field("Description", &self.description)
            .field("Phases", &self.phases.len().to_string());

        for (i, phase) in self.phases.iter().enumerate() {
            let ro = if phase.read_only { " (read-only)" } else { "" };
            view = view.section(&format!("{}. {} [{}]", i + 1, phase.name, phase.dependency))
                .item(&phase.description)
                .field("Role", &phase.role)
                .field("Tools", &phase.tools.join(", "));
            if !ro.is_empty() {
                view = view.field("Read-only", "yes");
            }
        }

        view.render()
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
        WorkflowCommands::Export { name, output } => export_workflow(&name, output.as_deref(), json_mode),
        WorkflowCommands::ExportAll { output } => export_all_workflows(output.as_deref(), json_mode),
    }
}

fn list_workflows(json_mode: bool) -> Result<()> {
    let config = Config::load()
        .context("Failed to load configuration")?;

    let available = config.available_workflows();

    let workflows: Vec<WorkflowSummary> = available
        .into_iter()
        .map(|(name, description, phase_count, is_default)| {
            let source = if config.workflows.iter().any(|wf| wf.name == name) {
                "abathur.toml".to_string()
            } else {
                "yaml".to_string()
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

    // Resolve the effective template for each known workflow name (inline >
    // YAML) so each name is validated exactly once against the source that
    // will actually be used at runtime.
    let mut names: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    names.extend(config.load_yaml_workflows().into_keys());
    names.extend(config.workflows.iter().map(|wf| wf.name.clone()));

    for name in names {
        if let Some(wf) = config.resolve_workflow(&name) {
            let valid = wf.validate();
            results.push(ValidationResult {
                name: wf.name.clone(),
                valid: valid.is_ok(),
                error: valid.err(),
            });
        }
    }

    let all_valid = results.iter().all(|r| r.valid);
    let out = ValidateOutput { results, all_valid };
    output(&out, json_mode);
    Ok(())
}

// ── Export commands ─────────────────────────────────────────────────────

#[derive(Debug, serde::Serialize)]
struct ExportOutput {
    workflow: String,
    path: String,
}

impl CommandOutput for ExportOutput {
    fn to_human(&self) -> String {
        format!("Exported workflow '{}' to {}", self.workflow, self.path)
    }

    fn to_json(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or_default()
    }
}

#[derive(Debug, serde::Serialize)]
struct ExportAllOutput {
    exported: Vec<ExportOutput>,
}

impl CommandOutput for ExportAllOutput {
    fn to_human(&self) -> String {
        if self.exported.is_empty() {
            return "No workflows exported.".to_string();
        }
        let mut lines = vec![format!("Exported {} workflows:", self.exported.len())];
        for e in &self.exported {
            lines.push(format!("  {} -> {}", e.workflow, e.path));
        }
        lines.join("\n")
    }

    fn to_json(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or_default()
    }
}

fn write_workflow_yaml(wf: &WorkflowTemplate, dir: &str) -> Result<String> {
    std::fs::create_dir_all(dir)
        .with_context(|| format!("Failed to create directory: {}", dir))?;
    let yaml = wf.to_yaml().map_err(|e| anyhow::anyhow!("{}", e))?;
    let path = std::path::Path::new(dir).join(format!("{}.yaml", wf.name));
    std::fs::write(&path, &yaml)
        .with_context(|| format!("Failed to write {}", path.display()))?;
    Ok(path.display().to_string())
}

fn export_workflow(name: &str, output_dir: Option<&str>, json_mode: bool) -> Result<()> {
    let config = Config::load().context("Failed to load configuration")?;
    let dir = output_dir.unwrap_or(&config.workflows_dir);

    let wf = config
        .resolve_workflow(name)
        .ok_or_else(|| anyhow::anyhow!("Workflow '{}' not found", name))?;

    let path = write_workflow_yaml(&wf, dir)?;
    let out = ExportOutput {
        workflow: wf.name,
        path,
    };
    output(&out, json_mode);
    Ok(())
}

fn export_all_workflows(output_dir: Option<&str>, json_mode: bool) -> Result<()> {
    let config = Config::load().context("Failed to load configuration")?;
    let dir = output_dir.unwrap_or(&config.workflows_dir);

    let mut exported = Vec::new();
    let mut names: Vec<String> = config
        .available_workflows()
        .into_iter()
        .map(|(name, _, _, _)| name)
        .collect();
    names.sort();
    for name in &names {
        if let Some(wf) = config.resolve_workflow(name) {
            let path = write_workflow_yaml(&wf, dir)?;
            exported.push(ExportOutput {
                workflow: wf.name,
                path,
            });
        }
    }

    let out = ExportAllOutput { exported };
    output(&out, json_mode);
    Ok(())
}
