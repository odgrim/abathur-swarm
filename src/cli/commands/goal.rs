//! Goal CLI commands.

use anyhow::{Context, Result};
use clap::{Args, Subcommand};
use std::sync::Arc;

use crate::adapters::sqlite::{goal_repository::SqliteGoalRepository, initialize_default_database};
use crate::cli::id_resolver::resolve_goal_id;
use crate::cli::output::{output, truncate, CommandOutput};
use crate::domain::models::{Goal, GoalConstraint, GoalPriority, GoalStatus};
use crate::domain::ports::GoalFilter;
use crate::services::GoalService;

#[derive(Args, Debug)]
pub struct GoalArgs {
    #[command(subcommand)]
    pub command: GoalCommands,
}

#[derive(Subcommand, Debug)]
pub enum GoalCommands {
    /// Create a new goal
    Set {
        /// Goal name
        name: String,
        /// Goal description
        #[arg(short, long)]
        description: Option<String>,
        /// Priority (low, normal, high, critical)
        #[arg(short, long, default_value = "normal")]
        priority: String,
        /// Parent goal ID
        #[arg(long)]
        parent: Option<String>,
        /// Constraints (format: "name:description")
        #[arg(short, long)]
        constraint: Vec<String>,
    },
    /// List goals
    List {
        /// Filter by status (active, paused, retired)
        #[arg(short, long)]
        status: Option<String>,
        /// Filter by priority
        #[arg(short, long)]
        priority: Option<String>,
        /// Show as tree
        #[arg(long)]
        tree: bool,
    },
    /// Show goal details
    Show {
        /// Goal ID
        id: String,
    },
    /// Pause a goal
    Pause {
        /// Goal ID
        id: String,
    },
    /// Resume a paused goal
    Resume {
        /// Goal ID
        id: String,
    },
    /// Retire a goal
    Retire {
        /// Goal ID
        id: String,
    },
}

#[derive(Debug, serde::Serialize)]
pub struct GoalOutput {
    pub id: String,
    pub name: String,
    pub description: String,
    pub status: String,
    pub priority: String,
    pub parent_id: Option<String>,
    pub constraints_count: usize,
}

impl From<&Goal> for GoalOutput {
    fn from(goal: &Goal) -> Self {
        Self {
            id: goal.id.to_string(),
            name: goal.name.clone(),
            description: goal.description.clone(),
            status: goal.status.as_str().to_string(),
            priority: goal.priority.as_str().to_string(),
            parent_id: goal.parent_id.map(|id| id.to_string()),
            constraints_count: goal.constraints.len(),
        }
    }
}

#[derive(Debug, serde::Serialize)]
pub struct GoalListOutput {
    pub goals: Vec<GoalOutput>,
    pub total: usize,
}

impl CommandOutput for GoalListOutput {
    fn to_human(&self) -> String {
        if self.goals.is_empty() {
            return "No goals found.".to_string();
        }

        let mut lines = vec![format!("Found {} goal(s):\n", self.total)];
        lines.push(format!("{:<36} {:<20} {:<10} {:<10}", "ID", "NAME", "STATUS", "PRIORITY"));
        lines.push("-".repeat(76));

        for goal in &self.goals {
            lines.push(format!(
                "{:<36} {:<20} {:<10} {:<10}",
                &goal.id[..8],
                truncate(&goal.name, 18),
                goal.status,
                goal.priority
            ));
        }

        lines.join("\n")
    }

    fn to_json(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or_default()
    }
}

#[derive(Debug, serde::Serialize)]
pub struct GoalDetailOutput {
    pub goal: GoalOutput,
    pub constraints: Vec<String>,
}

impl CommandOutput for GoalDetailOutput {
    fn to_human(&self) -> String {
        let mut lines = vec![
            format!("Goal: {}", self.goal.name),
            format!("ID: {}", self.goal.id),
            format!("Status: {}", self.goal.status),
            format!("Priority: {}", self.goal.priority),
            format!("Description: {}", self.goal.description),
        ];

        if let Some(parent) = &self.goal.parent_id {
            lines.push(format!("Parent: {}", parent));
        }

        if !self.constraints.is_empty() {
            lines.push("\nConstraints:".to_string());
            for c in &self.constraints {
                lines.push(format!("  - {}", c));
            }
        }

        lines.join("\n")
    }

    fn to_json(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or_default()
    }
}

#[derive(Debug, serde::Serialize)]
pub struct GoalActionOutput {
    pub success: bool,
    pub message: String,
    pub goal: Option<GoalOutput>,
}

impl CommandOutput for GoalActionOutput {
    fn to_human(&self) -> String {
        self.message.clone()
    }

    fn to_json(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or_default()
    }
}

pub async fn execute(args: GoalArgs, json_mode: bool) -> Result<()> {
    let pool = initialize_default_database()
        .await
        .context("Failed to initialize database. Run 'abathur init' first.")?;

    let repo = Arc::new(SqliteGoalRepository::new(pool.clone()));
    let event_bus = crate::cli::event_helpers::create_persistent_event_bus(pool.clone());
    let service = GoalService::new(repo, event_bus);

    match args.command {
        GoalCommands::Set { name, description, priority, parent, constraint } => {
            let priority = GoalPriority::from_str(&priority)
                .ok_or_else(|| anyhow::anyhow!("Invalid priority: {}", priority))?;

            let parent_id = match parent {
                Some(p) => Some(resolve_goal_id(&pool, &p).await?),
                None => None,
            };

            let constraints: Vec<GoalConstraint> = constraint
                .iter()
                .filter_map(|c| {
                    let parts: Vec<&str> = c.splitn(2, ':').collect();
                    if parts.len() == 2 {
                        Some(GoalConstraint::preference(parts[0], parts[1]))
                    } else {
                        None
                    }
                })
                .collect();

            let goal = service.create_goal(
                name,
                description.unwrap_or_default(),
                priority,
                parent_id,
                constraints,
                vec![],
            ).await?;

            let out = GoalActionOutput {
                success: true,
                message: format!("Goal created: {}", goal.id),
                goal: Some(GoalOutput::from(&goal)),
            };
            output(&out, json_mode);
        }

        GoalCommands::List { status, priority, tree: _ } => {
            let filter = GoalFilter {
                status: status.as_ref().and_then(|s| GoalStatus::from_str(s)),
                priority: priority.as_ref().and_then(|p| GoalPriority::from_str(p)),
                parent_id: None,
            };

            let goals = service.list_goals(filter).await?;
            let out = GoalListOutput {
                total: goals.len(),
                goals: goals.iter().map(GoalOutput::from).collect(),
            };
            output(&out, json_mode);
        }

        GoalCommands::Show { id } => {
            let uuid = resolve_goal_id(&pool, &id).await?;
            let goal = service.get_goal(uuid).await?
                .ok_or_else(|| anyhow::anyhow!("Goal not found: {}", id))?;

            let constraints = goal.constraints.iter()
                .map(|c| format!("{}: {}", c.name, c.description))
                .collect();

            let out = GoalDetailOutput {
                goal: GoalOutput::from(&goal),
                constraints,
            };
            output(&out, json_mode);
        }

        GoalCommands::Pause { id } => {
            let uuid = resolve_goal_id(&pool, &id).await?;
            let goal = service.transition_status(uuid, GoalStatus::Paused).await?;

            let out = GoalActionOutput {
                success: true,
                message: format!("Goal paused: {}", goal.id),
                goal: Some(GoalOutput::from(&goal)),
            };
            output(&out, json_mode);
        }

        GoalCommands::Resume { id } => {
            let uuid = resolve_goal_id(&pool, &id).await?;
            let goal = service.transition_status(uuid, GoalStatus::Active).await?;

            let out = GoalActionOutput {
                success: true,
                message: format!("Goal resumed: {}", goal.id),
                goal: Some(GoalOutput::from(&goal)),
            };
            output(&out, json_mode);
        }

        GoalCommands::Retire { id } => {
            let uuid = resolve_goal_id(&pool, &id).await?;
            let goal = service.transition_status(uuid, GoalStatus::Retired).await?;

            let out = GoalActionOutput {
                success: true,
                message: format!("Goal retired: {}", goal.id),
                goal: Some(GoalOutput::from(&goal)),
            };
            output(&out, json_mode);
        }
    }

    Ok(())
}

