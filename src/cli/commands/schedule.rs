//! Schedule CLI commands for managing periodic task schedules.

use anyhow::{Context, Result};
use chrono::Utc;
use clap::{Args, Subcommand};
use std::str::FromStr;
use std::sync::Arc;

use crate::adapters::sqlite::{SqliteTaskScheduleRepository, initialize_default_database};
use crate::cli::display::{
    CommandOutput, DetailView, action_failure, action_success, colorize_status, list_table, output,
    relative_time_str, render_list, short_id, truncate_ellipsis,
};
use crate::cli::id_resolver::resolve_schedule_id;
use crate::domain::models::TaskPriority;
use crate::domain::models::task_schedule::*;
use crate::domain::ports::task_schedule_repository::{TaskScheduleFilter, TaskScheduleRepository};
use crate::services::task_schedule_service::TaskScheduleService;
use crate::services::trigger_rules::normalize_cron_expression;

#[derive(Args, Debug)]
pub struct ScheduleArgs {
    #[command(subcommand)]
    pub command: ScheduleCommands,
}

/// Arguments for `schedule create`. Extracted into a named [`Args`] struct
/// so the large payload lives behind a pointer once — keeps `ScheduleCommands`
/// lean for pattern matches and codegen.
#[derive(Args, Debug)]
pub struct ScheduleCreateArgs {
    /// Schedule name (unique identifier)
    #[arg(long)]
    pub name: String,

    /// Description of the schedule
    #[arg(long, default_value = "")]
    pub description: String,

    /// Cron expression (5-field: min hour dom month dow)
    #[arg(long, group = "schedule_type")]
    pub cron: Option<String>,

    /// Interval in seconds
    #[arg(long, group = "schedule_type")]
    pub interval: Option<u64>,

    /// One-shot at a specific time (RFC3339)
    #[arg(long, group = "schedule_type")]
    pub at: Option<String>,

    /// Title for created tasks
    #[arg(long)]
    pub task_title: String,

    /// Description/prompt for created tasks
    #[arg(long)]
    pub task_description: String,

    /// Priority for created tasks (low, normal, high, critical)
    #[arg(long, default_value = "normal")]
    pub priority: String,

    /// Agent type to assign created tasks to
    #[arg(long)]
    pub agent_type: Option<String>,

    /// Overlap policy (skip, allow, cancel_previous)
    #[arg(long, default_value = "skip")]
    pub overlap: String,
}

#[derive(Subcommand, Debug)]
pub enum ScheduleCommands {
    /// Create a new periodic task schedule
    Create(Box<ScheduleCreateArgs>),

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

    /// Update a task schedule's properties
    Update {
        /// Schedule ID or name
        id_or_name: String,
        /// New description
        #[arg(long)]
        description: Option<String>,
        /// New cron expression (replaces the schedule type with cron)
        #[arg(long)]
        cron: Option<String>,
        /// New task title
        #[arg(long)]
        task_title: Option<String>,
        /// New task description/prompt
        #[arg(long)]
        task_description: Option<String>,
        /// New priority (low, normal, high, critical)
        #[arg(long)]
        priority: Option<String>,
        /// New agent type
        #[arg(long)]
        agent_type: Option<String>,
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
    pub next_fire_at: Option<String>,
}

/// Compute the next fire time for a cron expression, returning an ISO string.
fn compute_next_fire(schedule: &TaskScheduleType) -> Option<String> {
    if let TaskScheduleType::Cron { expression } = schedule {
        let normalized = normalize_cron_expression(expression);
        let sched = cron::Schedule::from_str(&normalized).ok()?;
        let next = sched.upcoming(Utc).next()?;
        Some(next.to_rfc3339())
    } else {
        None
    }
}

impl From<&TaskSchedule> for ScheduleOutput {
    fn from(s: &TaskSchedule) -> Self {
        let next_fire_at = if s.status == TaskScheduleStatus::Active {
            compute_next_fire(&s.schedule)
        } else {
            None
        };
        Self {
            id: s.id.to_string(),
            name: s.name.clone(),
            description: truncate_ellipsis(&s.description, 40),
            schedule_type: s.schedule.as_str().to_string(),
            schedule_detail: s.schedule.description(),
            status: s.status.as_str().to_string(),
            task_title: truncate_ellipsis(&s.task_title, 30),
            fire_count: s.fire_count,
            last_fired_at: s.last_fired_at.map(|t| t.to_rfc3339()),
            next_fire_at,
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

        let mut table = list_table(&[
            "ID",
            "Name",
            "Status",
            "Schedule",
            "Next Fire",
            "Fires",
            "Task Title",
        ]);

        for s in &self.schedules {
            let next_fire = s
                .next_fire_at
                .as_deref()
                .map(relative_time_str)
                .unwrap_or_else(|| "-".to_string());
            table.add_row(vec![
                short_id(&s.id).to_string(),
                truncate_ellipsis(&s.name, 20),
                colorize_status(&s.status).to_string(),
                truncate_ellipsis(&s.schedule_detail, 20),
                next_fire,
                s.fire_count.to_string(),
                truncate_ellipsis(&s.task_title, 30),
            ]);
        }

        render_list("task schedule", table, self.total)
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
        let mut view = DetailView::new(&self.schedule.name)
            .field("ID", &self.schedule.id)
            .field(
                "Status",
                &colorize_status(&self.schedule.status).to_string(),
            )
            .field("Schedule", &self.schedule.schedule_detail);

        if let Some(ref next) = self.schedule.next_fire_at {
            view = view.field("Next Fire", &relative_time_str(next));
        }

        view = view
            .field("Overlap", &self.overlap_policy)
            .section("Task Template")
            .field("Title", &self.schedule.task_title)
            .field(
                "Description",
                &truncate_ellipsis(&self.task_description, 80),
            )
            .field("Priority", &self.task_priority);

        if let Some(ref agent) = self.task_agent_type {
            view = view.field("Agent", agent);
        }

        view = view
            .section("History")
            .field("Fires", &self.schedule.fire_count.to_string());

        if let Some(ref last) = self.schedule.last_fired_at {
            view = view.field("Last Fired", &relative_time_str(last));
        }

        if let Some(ref task_id) = self.last_task_id {
            view = view.field("Last Task", short_id(task_id));
        }

        view = view
            .section("Timing")
            .field("Created", &relative_time_str(&self.created_at))
            .field("Updated", &relative_time_str(&self.updated_at));

        view.render()
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
        if self.success {
            action_success(&self.message)
        } else {
            action_failure(&self.message)
        }
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
        ScheduleCommands::Create(args) => {
            let ScheduleCreateArgs {
                name,
                description,
                cron,
                interval,
                at,
                task_title,
                task_description,
                priority,
                agent_type,
                overlap,
            } = *args;
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

            let priority = TaskPriority::parse(&priority).unwrap_or(TaskPriority::Normal);
            let overlap_policy = OverlapPolicy::parse(&overlap).unwrap_or(OverlapPolicy::Skip);

            let mut schedule = TaskSchedule::new(
                name,
                description,
                schedule_type,
                task_title,
                task_description,
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
                status: status.and_then(|s| TaskScheduleStatus::parse(&s)),
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

        ScheduleCommands::Update {
            id_or_name,
            description,
            cron,
            task_title,
            task_description,
            priority,
            agent_type,
        } => {
            let mut schedule = find_schedule(&repo, &pool, &id_or_name).await?;

            if let Some(desc) = description {
                schedule.description = desc;
            }
            if let Some(expr) = cron {
                schedule.schedule = TaskScheduleType::Cron { expression: expr };
            }
            if let Some(title) = task_title {
                schedule.task_title = title;
            }
            if let Some(desc) = task_description {
                schedule.task_description = desc;
            }
            if let Some(p) = priority {
                schedule.task_priority = TaskPriority::parse(&p).unwrap_or(TaskPriority::Normal);
            }
            if let Some(agent) = agent_type {
                schedule.task_agent_type = Some(agent);
            }

            schedule.updated_at = chrono::Utc::now();
            repo.update(&schedule).await?;

            let out = ScheduleActionOutput {
                success: true,
                message: format!("Task schedule updated: {}", schedule.name),
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
        && let Some(schedule) = repo.get(uuid).await?
    {
        return Ok(schedule);
    }

    anyhow::bail!("Task schedule not found: {}", id_or_name)
}
