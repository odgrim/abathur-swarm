//! Convenience `abathur cron` subcommand for quick cron schedule management.
//!
//! Provides a streamlined interface for the most common use case: scheduling a
//! prompt to run on a cron expression. Delegates to the TaskSchedule infrastructure.
//!
//! # Examples
//!
//! ```sh
//! abathur cron "*/5 * * * *" "run code analysis on the project"
//! abathur cron "0 9 * * MON-FRI" "check for stale goals" --name daily-goal-check
//! abathur cron list
//! abathur cron show my-schedule
//! abathur cron disable my-schedule
//! ```

use anyhow::{Context, Result};
use chrono::Utc;
use clap::{Args, Subcommand};
use std::str::FromStr;
use std::sync::Arc;

use crate::adapters::sqlite::{SqliteTaskScheduleRepository, initialize_default_database};
use crate::cli::commands::schedule::{self, ScheduleArgs, ScheduleCommands};
use crate::cli::display::{CommandOutput, action_success, output};
use crate::domain::models::TaskPriority;
use crate::domain::models::task_schedule::*;
use crate::domain::ports::task_schedule_repository::TaskScheduleRepository;
use crate::services::task_schedule_service::TaskScheduleService;
use crate::services::trigger_rules::{normalize_cron_expression, validate_cron_expression};

#[derive(Args, Debug)]
pub struct CronArgs {
    #[command(subcommand)]
    pub command: CronCommands,
}

#[derive(Subcommand, Debug)]
pub enum CronCommands {
    /// Create a new cron schedule (shorthand for `schedule create --cron`)
    #[command(name = "create", aliases = ["new", "add"])]
    Create {
        /// Cron expression (5-field: min hour dom month dow)
        expression: String,

        /// Prompt / task description to run on each fire
        prompt: String,

        /// Schedule name (auto-generated from prompt if omitted)
        #[arg(long)]
        name: Option<String>,

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

    /// List all cron schedules
    List {
        /// Filter by status (active, paused, completed)
        #[arg(long)]
        status: Option<String>,
    },

    /// Show details of a cron schedule
    Show {
        /// Schedule ID or name
        id_or_name: String,
    },

    /// Enable (unpause) a cron schedule
    Enable {
        /// Schedule ID or name
        id_or_name: String,
    },

    /// Disable (pause) a cron schedule
    Disable {
        /// Schedule ID or name
        id_or_name: String,
    },

    /// Delete a cron schedule
    Delete {
        /// Schedule ID or name
        id_or_name: String,
    },
}

/// Derive a slug-style name from a prompt string.
///
/// Takes the first few words, lowercases, replaces non-alphanumeric with hyphens,
/// and truncates to a reasonable length.
fn slugify_prompt(prompt: &str) -> String {
    let slug: String = prompt
        .split_whitespace()
        .take(5)
        .collect::<Vec<_>>()
        .join("-")
        .to_lowercase()
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' {
                c
            } else {
                '-'
            }
        })
        .collect();

    // Trim trailing hyphens and truncate
    let slug = slug.trim_matches('-').to_string();
    if slug.len() > 40 {
        slug[..40].trim_end_matches('-').to_string()
    } else {
        slug
    }
}

/// Compute the next fire time for a cron expression and format it as a
/// human-readable relative time string.
fn next_fire_description(expression: &str) -> Option<String> {
    let normalized = normalize_cron_expression(expression);
    let schedule = cron::Schedule::from_str(&normalized).ok()?;
    let next = schedule.upcoming(Utc).next()?;
    let duration = next - Utc::now();

    // Format as relative time
    let secs = duration.num_seconds();
    if secs < 60 {
        Some(format!("in {} seconds", secs))
    } else if secs < 3600 {
        let mins = secs / 60;
        Some(format!(
            "in {} minute{}",
            mins,
            if mins == 1 { "" } else { "s" }
        ))
    } else if secs < 86400 {
        let hours = secs / 3600;
        let mins = (secs % 3600) / 60;
        if mins > 0 {
            Some(format!("in {}h {}m", hours, mins))
        } else {
            Some(format!(
                "in {} hour{}",
                hours,
                if hours == 1 { "" } else { "s" }
            ))
        }
    } else {
        let days = secs / 86400;
        Some(format!(
            "in {} day{}",
            days,
            if days == 1 { "" } else { "s" }
        ))
    }
}

// -- Output struct for create confirmation --

#[derive(Debug, serde::Serialize)]
pub struct CronCreateOutput {
    pub success: bool,
    pub id: String,
    pub name: String,
    pub expression: String,
    pub next_fire: Option<String>,
    pub message: String,
}

impl CommandOutput for CronCreateOutput {
    fn to_human(&self) -> String {
        let mut msg = action_success(&format!("Cron schedule '{}' created", self.name,));
        msg.push_str(&format!("\n  ID:         {}", self.id));
        msg.push_str(&format!("\n  Expression: {}", self.expression));
        if let Some(ref next) = self.next_fire {
            msg.push_str(&format!("\n  Next fire:  {}", next));
        }
        msg
    }

    fn to_json(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or_default()
    }
}

pub async fn execute(args: CronArgs, json_mode: bool) -> Result<()> {
    match args.command {
        CronCommands::Create {
            expression,
            prompt,
            name,
            priority,
            agent_type,
            overlap,
        } => {
            // Validate cron expression early with a clear error
            validate_cron_expression(&expression).map_err(|e| anyhow::anyhow!(e))?;

            let normalized = normalize_cron_expression(&expression);
            let schedule_name = name.unwrap_or_else(|| slugify_prompt(&prompt));

            let pool = initialize_default_database()
                .await
                .context("Failed to initialize database. Run 'abathur init' first.")?;

            let repo = Arc::new(SqliteTaskScheduleRepository::new(pool.clone()));
            let service = TaskScheduleService::new(repo.clone());

            // Check for duplicate name
            if repo.get_by_name(&schedule_name).await?.is_some() {
                anyhow::bail!(
                    "A schedule named '{}' already exists. Use --name to specify a different name.",
                    schedule_name
                );
            }

            let priority = TaskPriority::from_str(&priority).unwrap_or(TaskPriority::Normal);
            let overlap_policy = OverlapPolicy::from_str(&overlap).unwrap_or(OverlapPolicy::Skip);

            // Use the prompt as both title (truncated) and description
            let task_title = if prompt.len() > 60 {
                format!("{}…", &prompt[..59])
            } else {
                prompt.clone()
            };

            let mut sched = TaskSchedule::new(
                schedule_name.clone(),
                format!("Cron schedule: {}", expression),
                TaskScheduleType::Cron {
                    expression: normalized.clone(),
                },
                task_title,
                prompt,
            );
            sched.task_priority = priority;
            sched.task_agent_type = agent_type;
            sched.overlap_policy = overlap_policy;

            let sched = service.create_schedule(sched).await?;
            let next_fire = next_fire_description(&expression);

            let out = CronCreateOutput {
                success: true,
                id: sched.id.to_string(),
                name: sched.name.clone(),
                expression: expression.clone(),
                next_fire,
                message: format!("Created cron schedule '{}'", sched.name),
            };
            output(&out, json_mode);
        }

        // Delegate list/show/enable/disable/delete to the schedule command
        CronCommands::List { status } => {
            let sched_args = ScheduleArgs {
                command: ScheduleCommands::List { status },
            };
            schedule::execute(sched_args, json_mode).await?;
        }

        CronCommands::Show { id_or_name } => {
            let sched_args = ScheduleArgs {
                command: ScheduleCommands::Show { id_or_name },
            };
            schedule::execute(sched_args, json_mode).await?;
        }

        CronCommands::Enable { id_or_name } => {
            let sched_args = ScheduleArgs {
                command: ScheduleCommands::Enable { id_or_name },
            };
            schedule::execute(sched_args, json_mode).await?;
        }

        CronCommands::Disable { id_or_name } => {
            let sched_args = ScheduleArgs {
                command: ScheduleCommands::Disable { id_or_name },
            };
            schedule::execute(sched_args, json_mode).await?;
        }

        CronCommands::Delete { id_or_name } => {
            let sched_args = ScheduleArgs {
                command: ScheduleCommands::Delete { id_or_name },
            };
            schedule::execute(sched_args, json_mode).await?;
        }
    }

    Ok(())
}
