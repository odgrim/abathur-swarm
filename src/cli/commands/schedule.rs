//! Schedule CLI commands for managing periodic task schedules.

use anyhow::{Context, Result};
use clap::{Args, Subcommand};
use std::sync::Arc;

use crate::adapters::sqlite::{initialize_default_database, SqliteTaskScheduleRepository};
use crate::cli::id_resolver::resolve_schedule_id;
use crate::cli::output::{output, truncate, CommandOutput};
use crate::domain::models::task_schedule::*;
use crate::domain::models::TaskPriority;
use crate::domain::ports::task_schedule_repository::{TaskScheduleFilter, TaskScheduleRepository};
use crate::services::task_schedule_service::TaskScheduleService;

#[derive(Args, Debug)]
pub struct ScheduleArgs {
    #[command(subcommand)]
    pub command: ScheduleCommands,
}

#[derive(Subcommand, Debug)]
pub enum ScheduleCommands {
    /// Create a new periodic task schedule
    Create {
        /// Schedule name (unique identifier)
        #[arg(long)]
        name: String,

        /// Description of the schedule
        #[arg(long, default_value = "")]
        description: String,

        /// Cron expression (5-field: min hour dom month dow)
        #[arg(long, group = "schedule_type")]
        cron: Option<String>,

        /// Interval in seconds
        #[arg(long, group = "schedule_type")]
        interval: Option<u64>,

        /// One-shot at a specific time (RFC3339)
        #[arg(long, group = "schedule_type")]
        at: Option<String>,

        /// Title for created tasks
        #[arg(long)]
        task_title: String,

        /// Description/prompt for created tasks
        #[arg(long)]
        task_description: String,

        /// Priority for created tasks (low, normal, high, critical)
        #[arg(long, default_value = "normal")]
        priority: String,

        /// Agent type to assign created tasks to
        #[arg(long)]
        agent_type: Option<String>,

        /// Overlap policy (skip, allow, cancel_previous)
        #[arg(long, default_value = "skip")]
        overlap: String,
    },

    /// List all task schedules
    List {
        /// Filter by status (active, paused, completed)
        #[arg(long)]
        status: Option<String>,
    },

    /// Show details of a task schedule
    Show {
        /// Schedule ID or name
        id_or_name: String,
    },

    /// Enable (unpause) a task schedule
    Enable {
        /// Schedule ID or name
        id_or_name: String,
    },

    /// Disable (pause) a task schedule
    Disable {
        /// Schedule ID or name
        id_or_name: String,
    },

    /// Delete a task schedule
    Delete {
        /// Schedule ID or name
        id_or_name: String,
    },
}

// -- Output structs --

#[derive(Debug, serde::Serialize)]
pub struct ScheduleOutput {
    pub id: String,
    pub name: String,
    pub description: String,
    pub schedule_type: String,
    pub schedule_detail: String,
    pub status: String,
    pub task_title: String,
    pub fire_count: u64,
    pub last_fired_at: Option<String>,
}

impl From<&TaskSchedule> for ScheduleOutput {
    fn from(s: &TaskSchedule) -> Self {
        Self {
            id: s.id.to_string(),
            name: s.name.clone(),
            description: truncate(&s.description, 40),
            schedule_type: s.schedule.as_str().to_string(),
            schedule_detail: s.schedule.description(),
            status: s.status.as_str().to_string(),
            task_title: truncate(&s.task_title, 30),
            fire_count: s.fire_count,
            last_fired_at: s.last_fired_at.map(|t| t.to_rfc3339()),
        }
    }
}

#[derive(Debug, serde::Serialize)]
pub struct ScheduleListOutput {
    pub schedules: Vec<ScheduleOutput>,
    pub total: usize,
}

impl CommandOutput for ScheduleListOutput {
    fn to_human(&self) -> String {
        if self.schedules.is_empty() {
            return "No task schedules found.".to_string();
        }

        let mut lines = vec![format!("Found {} task schedule(s):\n", self.total)];
        lines.push(format!(
            "{:<12} {:<20} {:<8} {:<22} {:<8} {:<25}",
            "ID", "NAME", "STATUS", "SCHEDULE", "FIRED", "TASK TITLE"
        ));
        lines.push("-".repeat(97));

        for s in &self.schedules {
            lines.push(format!(
                "{:<12} {:<20} {:<8} {:<22} {:<8} {:<25}",
                &s.id[..8],
                truncate(&s.name, 18),
                s.status,
                truncate(&s.schedule_detail, 20),
                s.fire_count,
                truncate(&s.task_title, 23),
            ));
        }

        lines.join("\n")
    }

    fn to_json(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or_default()
    }
}

#[derive(Debug, serde::Serialize)]
pub struct ScheduleDetailOutput {
    pub schedule: ScheduleOutput,
    pub task_description: String,
    pub task_priority: String,
    pub task_agent_type: Option<String>,
    pub overlap_policy: String,
    pub last_task_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

impl CommandOutput for ScheduleDetailOutput {
    fn to_human(&self) -> String {
        let mut lines = vec![
            format!("Schedule: {}", self.schedule.name),
            format!("ID: {}", self.schedule.id),
            format!("Description: {}", self.schedule.description),
            format!("Status: {}", self.schedule.status),
            format!("Schedule: {}", self.schedule.schedule_detail),
            format!("Overlap Policy: {}", self.overlap_policy),
            String::new(),
            "Task Template:".to_string(),
            format!("  Title: {}", self.schedule.task_title),
            format!("  Description: {}", truncate(&self.task_description, 80)),
            format!("  Priority: {}", self.task_priority),
        ];

        if let Some(ref agent) = self.task_agent_type {
            lines.push(format!("  Agent Type: {}", agent));
        }

        lines.push(String::new());
        lines.push(format!("Fire Count: {}", self.schedule.fire_count));

        if let Some(ref last) = self.schedule.last_fired_at {
            lines.push(format!("Last Fired: {}", last));
        }

        if let Some(ref task_id) = self.last_task_id {
            lines.push(format!("Last Task ID: {}", task_id));
        }

        lines.push(format!("Created: {}", self.created_at));
        lines.push(format!("Updated: {}", self.updated_at));

        lines.join("\n")
    }

    fn to_json(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or_default()
    }
}

#[derive(Debug, serde::Serialize)]
pub struct ScheduleActionOutput {
    pub success: bool,
    pub message: String,
}

impl CommandOutput for ScheduleActionOutput {
    fn to_human(&self) -> String {
        self.message.clone()
    }

    fn to_json(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or_default()
    }
}

// -- Execute --

pub async fn execute(args: ScheduleArgs, json_mode: bool) -> Result<()> {
    let pool = initialize_default_database()
        .await
        .context("Failed to initialize database. Run 'abathur init' first.")?;

    let repo = Arc::new(SqliteTaskScheduleRepository::new(pool.clone()));
    let service = TaskScheduleService::new(repo.clone());

    match args.command {
        ScheduleCommands::Create {
            name, description, cron, interval, at,
            task_title, task_description, priority,
            agent_type, overlap,
        } => {
            // Parse schedule type
            let schedule_type = if let Some(expr) = cron {
                TaskScheduleType::Cron { expression: expr }
            } else if let Some(secs) = interval {
                TaskScheduleType::Interval { every_secs: secs }
            } else if let Some(at_str) = at {
                let at = chrono::DateTime::parse_from_rfc3339(&at_str)
                    .context("Invalid datetime format. Use RFC3339 (e.g., 2025-01-01T00:00:00Z)")?
                    .with_timezone(&chrono::Utc);
                TaskScheduleType::Once { at }
            } else {
                anyhow::bail!("Must specify one of: --cron, --interval, or --at");
            };

            let priority = TaskPriority::from_str(&priority)
                .unwrap_or(TaskPriority::Normal);
            let overlap_policy = OverlapPolicy::from_str(&overlap)
                .unwrap_or(OverlapPolicy::Skip);

            let mut schedule = TaskSchedule::new(
                name, description, schedule_type, task_title, task_description,
            );
            schedule.task_priority = priority;
            schedule.task_agent_type = agent_type;
            schedule.overlap_policy = overlap_policy;

            let schedule = service.create_schedule(schedule).await?;

            let out = ScheduleActionOutput {
                success: true,
                message: format!(
                    "Created task schedule '{}' ({})\nID: {}\nNote: Schedule will be active on next swarm start.",
                    schedule.name,
                    schedule.schedule.description(),
                    schedule.id,
                ),
            };
            output(&out, json_mode);
        }

        ScheduleCommands::List { status } => {
            let filter = TaskScheduleFilter {
                status: status.and_then(|s| TaskScheduleStatus::from_str(&s)),
            };
            let schedules = service.list_schedules(filter).await?;

            let out = ScheduleListOutput {
                total: schedules.len(),
                schedules: schedules.iter().map(ScheduleOutput::from).collect(),
            };
            output(&out, json_mode);
        }

        ScheduleCommands::Show { id_or_name } => {
            let schedule = find_schedule(&repo, &pool, &id_or_name).await?;

            let out = ScheduleDetailOutput {
                schedule: ScheduleOutput::from(&schedule),
                task_description: schedule.task_description.clone(),
                task_priority: schedule.task_priority.as_str().to_string(),
                task_agent_type: schedule.task_agent_type.clone(),
                overlap_policy: schedule.overlap_policy.as_str().to_string(),
                last_task_id: schedule.last_task_id.map(|u| u.to_string()),
                created_at: schedule.created_at.to_rfc3339(),
                updated_at: schedule.updated_at.to_rfc3339(),
            };
            output(&out, json_mode);
        }

        ScheduleCommands::Enable { id_or_name } => {
            let schedule = find_schedule(&repo, &pool, &id_or_name).await?;
            service.enable_schedule(schedule.id).await?;

            let out = ScheduleActionOutput {
                success: true,
                message: format!("Task schedule enabled: {}", schedule.name),
            };
            output(&out, json_mode);
        }

        ScheduleCommands::Disable { id_or_name } => {
            let schedule = find_schedule(&repo, &pool, &id_or_name).await?;
            service.disable_schedule(schedule.id).await?;

            let out = ScheduleActionOutput {
                success: true,
                message: format!("Task schedule disabled: {}", schedule.name),
            };
            output(&out, json_mode);
        }

        ScheduleCommands::Delete { id_or_name } => {
            let schedule = find_schedule(&repo, &pool, &id_or_name).await?;
            let name = schedule.name.clone();
            service.delete_schedule(schedule.id).await?;

            let out = ScheduleActionOutput {
                success: true,
                message: format!("Task schedule deleted: {}", name),
            };
            output(&out, json_mode);
        }
    }

    Ok(())
}

async fn find_schedule(
    repo: &Arc<SqliteTaskScheduleRepository>,
    pool: &sqlx::SqlitePool,
    id_or_name: &str,
) -> Result<TaskSchedule> {
    // Try by name first
    if let Some(schedule) = repo.get_by_name(id_or_name).await? {
        return Ok(schedule);
    }

    // Try by UUID prefix
    if let Ok(uuid) = resolve_schedule_id(pool, id_or_name).await
        && let Some(schedule) = repo.get(uuid).await? {
            return Ok(schedule);
        }

    anyhow::bail!("Task schedule not found: {}", id_or_name)
}
