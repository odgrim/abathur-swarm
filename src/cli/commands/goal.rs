//! Goal CLI commands.

use anyhow::{Context, Result};
use clap::{Args, Subcommand};
use std::sync::Arc;

use crate::adapters::sqlite::{goal_repository::SqliteGoalRepository, initialize_default_database};
use crate::cli::command_dispatcher::CliCommandDispatcher;
use crate::cli::id_resolver::resolve_goal_id;
use crate::cli::display::{
    action_success, colorize_priority, colorize_status, list_table, output, render_list,
    short_id, relative_time_str, truncate_ellipsis, CommandOutput, DetailView,
};
use crate::domain::models::{Goal, GoalConstraint, GoalPriority, GoalStatus};
use crate::domain::ports::GoalFilter;
use crate::services::command_bus::{CommandResult, DomainCommand, GoalCommand};
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
    pub created_at: String,
    pub updated_at: String,
    pub last_check: Option<String>,
    pub domains: Vec<String>,
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
            created_at: goal.created_at.to_rfc3339(),
            updated_at: goal.updated_at.to_rfc3339(),
            last_check: goal.last_convergence_check_at.map(|t| t.to_rfc3339()),
            domains: goal.applicability_domains.clone(),
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

        let mut table = list_table(&["ID", "Name", "Status", "Priority", "Constraints", "Last Check"]);

        for goal in &self.goals {
            table.add_row(vec![
                short_id(&goal.id).to_string(),
                truncate_ellipsis(&goal.name, 30),
                colorize_status(&goal.status).to_string(),
                colorize_priority(&goal.priority).to_string(),
                goal.constraints_count.to_string(),
                goal.last_check.as_deref().map(relative_time_str).unwrap_or_else(|| "-".to_string()),
            ]);
        }

        render_list("goal", table, self.total)
    }

    fn to_json(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or_default()
    }
}

#[derive(Debug, serde::Serialize)]
pub struct GoalDetailOutput {
    pub goal: GoalOutput,
    pub constraints: Vec<ConstraintDisplay>,
}

#[derive(Debug, serde::Serialize)]
pub struct ConstraintDisplay {
    pub name: String,
    pub description: String,
    pub constraint_type: String,
}

impl CommandOutput for GoalDetailOutput {
    fn to_human(&self) -> String {
        let mut view = DetailView::new(&self.goal.name)
            .field("ID", &self.goal.id)
            .field("Status", &colorize_status(&self.goal.status).to_string())
            .field("Priority", &colorize_priority(&self.goal.priority).to_string())
            .field_opt("Parent", self.goal.parent_id.as_deref())
            .section("Description")
            .item(if self.goal.description.is_empty() { "(none)" } else { &self.goal.description });

        if !self.constraints.is_empty() {
            view = view.section(&format!("Constraints ({})", self.constraints.len()));
            for c in &self.constraints {
                view = view.item(&format!("[{}] {}: {}", c.constraint_type, c.name, c.description));
            }
        }

        if !self.goal.domains.is_empty() {
            view = view.section("Domains")
                .item(&self.goal.domains.join(", "));
        }

        view = view.section("Timing")
            .field("Created", &format!("{} ({})", relative_time_str(&self.goal.created_at), &self.goal.created_at))
            .field("Updated", &relative_time_str(&self.goal.updated_at))
            .field("Last Check", &self.goal.last_check.as_deref().map(|s| format!("{} ({})", relative_time_str(s), s)).unwrap_or_else(|| "-".to_string()));

        view.render()
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
        if self.success {
            action_success(&self.message)
        } else {
            crate::cli::display::action_failure(&self.message)
        }
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
    let event_bus = crate::cli::event_helpers::create_persistent_event_bus(pool.clone()).await;
    let service = GoalService::new(repo);
    let dispatcher = CliCommandDispatcher::new(pool.clone(), event_bus);

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

            let cmd = DomainCommand::Goal(GoalCommand::Create {
                name,
                description: description.unwrap_or_default(),
                priority,
                parent_id,
                constraints,
                domains: vec![],
            });

            let result = dispatcher.dispatch(cmd).await
                .map_err(|e| anyhow::anyhow!("{}", e))?;

            let goal = match result {
                CommandResult::Goal(g) => g,
                _ => anyhow::bail!("Unexpected command result"),
            };

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
                .map(|c| ConstraintDisplay {
                    name: c.name.clone(),
                    description: c.description.clone(),
                    constraint_type: format!("{:?}", c.constraint_type),
                })
                .collect();

            let out = GoalDetailOutput {
                goal: GoalOutput::from(&goal),
                constraints,
            };
            output(&out, json_mode);
        }

        GoalCommands::Pause { id } => {
            let uuid = resolve_goal_id(&pool, &id).await?;

            let cmd = DomainCommand::Goal(GoalCommand::TransitionStatus {
                goal_id: uuid,
                new_status: GoalStatus::Paused,
            });

            let result = dispatcher.dispatch(cmd).await
                .map_err(|e| anyhow::anyhow!("{}", e))?;

            let goal = match result {
                CommandResult::Goal(g) => g,
                _ => anyhow::bail!("Unexpected command result"),
            };

            let out = GoalActionOutput {
                success: true,
                message: format!("Goal paused: {}", goal.id),
                goal: Some(GoalOutput::from(&goal)),
            };
            output(&out, json_mode);
        }

        GoalCommands::Resume { id } => {
            let uuid = resolve_goal_id(&pool, &id).await?;

            let cmd = DomainCommand::Goal(GoalCommand::TransitionStatus {
                goal_id: uuid,
                new_status: GoalStatus::Active,
            });

            let result = dispatcher.dispatch(cmd).await
                .map_err(|e| anyhow::anyhow!("{}", e))?;

            let goal = match result {
                CommandResult::Goal(g) => g,
                _ => anyhow::bail!("Unexpected command result"),
            };

            let out = GoalActionOutput {
                success: true,
                message: format!("Goal resumed: {}", goal.id),
                goal: Some(GoalOutput::from(&goal)),
            };
            output(&out, json_mode);
        }

        GoalCommands::Retire { id } => {
            let uuid = resolve_goal_id(&pool, &id).await?;

            let cmd = DomainCommand::Goal(GoalCommand::TransitionStatus {
                goal_id: uuid,
                new_status: GoalStatus::Retired,
            });

            let result = dispatcher.dispatch(cmd).await
                .map_err(|e| anyhow::anyhow!("{}", e))?;

            let goal = match result {
                CommandResult::Goal(g) => g,
                _ => anyhow::bail!("Unexpected command result"),
            };

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

