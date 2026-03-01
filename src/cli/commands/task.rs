//! Task CLI commands.

use anyhow::{Context, Result};
use clap::{Args, Subcommand};
use std::sync::Arc;

use crate::adapters::sqlite::{SqliteTaskRepository, initialize_default_database};
use crate::cli::command_dispatcher::CliCommandDispatcher;
use crate::cli::id_resolver::{resolve_goal_id, resolve_task_id};
use crate::cli::display::{
    action_success, colorize_priority, colorize_status, list_table, output, render_list,
    short_id, relative_time_str, truncate_ellipsis, CommandOutput, DetailView,
};
use crate::domain::models::{Task, TaskContext, TaskPriority, TaskSource, TaskStatus, TaskType};
use crate::domain::ports::TaskFilter;
use crate::services::command_bus::{CommandResult, DomainCommand, TaskCommand};
use crate::services::TaskService;

/// CLI-local priority enum — maps to `TaskPriority` after clap parsing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum CliPriority {
    Low,
    Normal,
    High,
    Critical,
}

impl From<CliPriority> for TaskPriority {
    fn from(p: CliPriority) -> Self {
        match p {
            CliPriority::Low => TaskPriority::Low,
            CliPriority::Normal => TaskPriority::Normal,
            CliPriority::High => TaskPriority::High,
            CliPriority::Critical => TaskPriority::Critical,
        }
    }
}

#[derive(Args, Debug)]
pub struct TaskArgs {
    #[command(subcommand)]
    pub command: TaskCommands,
}

#[derive(Subcommand, Debug)]
pub enum TaskCommands {
    /// Submit a new task
    #[command(after_help = "\
Examples:
  abathur task submit \"Fix the login bug\"
  abathur task submit \"Review PR #42\" --priority high
  abathur task submit \"Implement feature X\" --goal abc123 --agent rust-impl
  abathur task submit \"Subtask\" --parent def456 --depends-on ghi789
")]
    Submit {
        /// The prompt to send to the agent
        prompt: String,
        /// Optional custom title (auto-generated from prompt if omitted)
        #[arg(short, long)]
        title: Option<String>,
        /// Priority level
        #[arg(short, long, default_value = "normal")]
        priority: CliPriority,
        /// Parent task ID
        #[arg(long)]
        parent: Option<String>,
        /// Agent type to assign
        #[arg(short, long)]
        agent: Option<String>,
        /// Dependencies (task IDs)
        #[arg(long)]
        depends_on: Vec<String>,
        /// Input context for the task
        #[arg(long)]
        input: Option<String>,
        /// Idempotency key
        #[arg(long)]
        idempotency_key: Option<String>,
        /// Deadline for SLA enforcement (ISO 8601 datetime, e.g. "2025-12-31T23:59:59Z")
        #[arg(long)]
        deadline: Option<String>,
        /// Associate with a goal (UUID or prefix)
        #[arg(long)]
        goal: Option<String>,
    },
    /// List tasks
    List {
        /// Filter by status
        #[arg(short, long)]
        status: Option<String>,
        /// Filter by priority
        #[arg(short, long)]
        priority: Option<String>,
        /// Filter by agent type
        #[arg(short, long)]
        agent: Option<String>,
        /// Filter by task type (standard, verification, research, review)
        #[arg(long = "type")]
        task_type: Option<String>,
        /// Show only ready tasks
        #[arg(long)]
        ready: bool,
        /// Maximum number of results
        #[arg(short, long, default_value = "50")]
        limit: usize,
    },
    /// Show task details
    Show {
        /// Task ID
        id: String,
    },
    /// Cancel a task
    Cancel {
        /// Task ID
        id: String,
    },
    /// Retry a failed task
    Retry {
        /// Task ID
        id: String,
    },
    /// Show task status summary
    Status,
}

#[derive(Debug, serde::Serialize)]
pub struct TaskOutput {
    pub id: String,
    pub title: String,
    pub status: String,
    pub priority: String,
    pub agent_type: Option<String>,
    pub depends_on: Vec<String>,
    pub retry_count: u32,
    pub task_type: String,
    pub created_at: String,
}

impl From<&Task> for TaskOutput {
    fn from(task: &Task) -> Self {
        Self {
            id: task.id.to_string(),
            title: task.title.clone(),
            status: task.status.as_str().to_string(),
            priority: task.priority.as_str().to_string(),
            agent_type: task.agent_type.clone(),
            depends_on: task.depends_on.iter().map(|id| id.to_string()).collect(),
            retry_count: task.retry_count,
            task_type: task.task_type.as_str().to_string(),
            created_at: task.created_at.to_rfc3339(),
        }
    }
}

#[derive(Debug, serde::Serialize)]
pub struct TaskListOutput {
    pub tasks: Vec<TaskOutput>,
    pub total: usize,
}

impl CommandOutput for TaskListOutput {
    fn to_human(&self) -> String {
        if self.tasks.is_empty() {
            return "No tasks found.".to_string();
        }

        let mut table = list_table(&["ID", "Title", "Status", "Priority", "Type", "Agent", "Age"]);

        for task in &self.tasks {
            table.add_row(vec![
                short_id(&task.id).to_string(),
                truncate_ellipsis(&task.title, 35),
                colorize_status(&task.status).to_string(),
                colorize_priority(&task.priority).to_string(),
                task.task_type.clone(),
                task.agent_type.as_deref().unwrap_or("-").to_string(),
                relative_time_str(&task.created_at),
            ]);
        }

        render_list("task", table, self.total)
    }

    fn to_json(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or_default()
    }
}

#[derive(Debug, serde::Serialize)]
pub struct TaskDetailOutput {
    pub task: TaskOutput,
    pub description: String,
    pub context_input: String,
    pub worktree_path: Option<String>,
    pub created_at: String,
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
    pub context_custom: std::collections::HashMap<String, serde_json::Value>,
}

impl CommandOutput for TaskDetailOutput {
    fn to_human(&self) -> String {
        let mut view = DetailView::new(&self.task.title)
            .field("ID", &self.task.id)
            .field("Status", &colorize_status(&self.task.status).to_string())
            .field("Priority", &colorize_priority(&self.task.priority).to_string())
            .field("Type", &self.task.task_type)
            .field_opt("Agent", self.task.agent_type.as_deref())
            .field("Source", "human")
            .section("Description");

        if self.description.is_empty() {
            view = view.item("(none)");
        } else {
            view = view.item(&self.description);
        }

        if !self.context_input.is_empty() {
            view = view.section("Input")
                .item(&truncate_ellipsis(&self.context_input, 200));
        }

        if !self.task.depends_on.is_empty() {
            view = view.section(&format!("Dependencies ({})", self.task.depends_on.len()));
            for dep in &self.task.depends_on {
                view = view.item(short_id(dep));
            }
        }

        // Verification-specific details
        if self.task.task_type == "verification" {
            view = view.section("Verification Details");
            if let Some(serde_json::Value::String(sat)) = self.context_custom.get("satisfaction") {
                view = view.field("Satisfaction", sat);
            }
            if let Some(serde_json::Value::Number(conf)) = self.context_custom.get("confidence") {
                if let Some(c) = conf.as_f64() {
                    view = view.field("Confidence", &format!("{:.0}%", c * 100.0));
                }
            }
            if let Some(serde_json::Value::Number(iter)) = self.context_custom.get("iteration") {
                view = view.field("Iteration", &iter.to_string());
            }
            if let Some(serde_json::Value::Number(gc)) = self.context_custom.get("gaps_count") {
                view = view.field("Gaps", &gc.to_string());
            }
            if let Some(serde_json::Value::Array(gaps)) = self.context_custom.get("gaps") {
                for gap in gaps {
                    if let Some(desc) = gap.get("description").and_then(|d| d.as_str()) {
                        let severity = gap.get("severity").and_then(|s| s.as_str()).unwrap_or("?");
                        view = view.item(&format!("[{}] {}", severity, desc));
                    }
                }
            }
            if let Some(serde_json::Value::String(summary)) = self.context_custom.get("accomplishment_summary") {
                view = view.field("Summary", &truncate_ellipsis(summary, 120));
            }
        }

        view = view.section("Timing")
            .field("Created", &format!("{} ({})", relative_time_str(&self.created_at), &self.created_at))
            .field("Started", &self.started_at.as_deref().map(|s| format!("{} ({})", relative_time_str(s), s)).unwrap_or_else(|| "-".to_string()))
            .field("Completed", &self.completed_at.as_deref().map(|s| format!("{} ({})", relative_time_str(s), s)).unwrap_or_else(|| "-".to_string()))
            .field("Retries", &self.task.retry_count.to_string());

        if let Some(path) = &self.worktree_path {
            view = view.field("Worktree", path);
        }

        view.render()
    }

    fn to_json(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or_default()
    }
}

#[derive(Debug, serde::Serialize)]
pub struct TaskActionOutput {
    pub success: bool,
    pub message: String,
    pub task: Option<TaskOutput>,
}

impl CommandOutput for TaskActionOutput {
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

#[derive(Debug, serde::Serialize)]
pub struct TaskStatusOutput {
    pub pending: u64,
    pub ready: u64,
    pub blocked: u64,
    pub running: u64,
    pub complete: u64,
    pub failed: u64,
    pub canceled: u64,
    pub total: u64,
}

impl CommandOutput for TaskStatusOutput {
    fn to_human(&self) -> String {
        use colored::Colorize;

        let statuses = [
            ("pending", self.pending, "blue"),
            ("ready", self.ready, "blue"),
            ("running", self.running, "yellow"),
            ("blocked", self.blocked, "cyan"),
            ("complete", self.complete, "green"),
            ("failed", self.failed, "red"),
            ("canceled", self.canceled, "white"),
        ];

        let max_count = statuses.iter().map(|(_, c, _)| *c).max().unwrap_or(1).max(1);
        let bar_width = 20u64;

        let mut lines = vec![format!("{}", "Task Status Summary".bold())];
        for (label, count, _) in &statuses {
            let bar_len = if *count > 0 {
                ((*count as f64 / max_count as f64) * bar_width as f64).ceil() as usize
            } else {
                0
            };
            let bar = "\u{2588}".repeat(bar_len);
            let colored_bar = match *label {
                "complete" => bar.green().to_string(),
                "failed" => bar.red().to_string(),
                "running" => bar.yellow().to_string(),
                "blocked" => bar.cyan().to_string(),
                "pending" | "ready" => bar.blue().to_string(),
                _ => bar.dimmed().to_string(),
            };
            lines.push(format!(
                "  {:<12} {:>4}  {}",
                colorize_status(label),
                count,
                colored_bar,
            ));
        }
        lines.push(format!("  {:<12} {:>4}", "Total".bold(), self.total.to_string().bold()));

        lines.join("\n")
    }

    fn to_json(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or_default()
    }
}

pub async fn execute(args: TaskArgs, json_mode: bool) -> Result<()> {
    let pool = initialize_default_database()
        .await
        .context("Failed to initialize database. Run 'abathur init' first.")?;

    let task_repo = Arc::new(SqliteTaskRepository::new(pool.clone()));
    let event_bus = crate::cli::event_helpers::create_persistent_event_bus(pool.clone()).await;
    let service = TaskService::new(task_repo);
    let dispatcher = CliCommandDispatcher::new(pool.clone(), event_bus);

    match args.command {
        TaskCommands::Submit {
            prompt,
            title,
            priority,
            parent,
            agent,
            depends_on,
            input,
            idempotency_key,
            deadline,
            goal,
        } => {
            if prompt.trim().is_empty() {
                anyhow::bail!("task description cannot be empty");
            }

            let priority = TaskPriority::from(priority);

            let parent_id = match parent {
                Some(p) => Some(resolve_task_id(&pool, &p).await?),
                None => None,
            };

            let mut deps = Vec::new();
            for d in &depends_on {
                deps.push(resolve_task_id(&pool, d).await?);
            }

            let goal_id = match goal {
                Some(ref g) => {
                    if !g.chars().all(|c| c.is_ascii_hexdigit() || c == '-') {
                        anyhow::bail!("'{}' is not a valid UUID", g);
                    }
                    Some(resolve_goal_id(&pool, g).await?)
                }
                None => None,
            };

            let mut ctx = TaskContext {
                input: input.unwrap_or_default(),
                ..Default::default()
            };
            if let Some(gid) = goal_id {
                ctx.custom.insert(
                    "goal_id".to_string(),
                    serde_json::Value::String(gid.to_string()),
                );
            }
            let context = Box::new(Some(ctx));

            let deadline = deadline
                .map(|d| chrono::DateTime::parse_from_rfc3339(&d))
                .transpose()
                .map_err(|e| anyhow::anyhow!("Invalid deadline: {}", e))?
                .map(|d| d.with_timezone(&chrono::Utc));

            let cmd = DomainCommand::Task(TaskCommand::Submit {
                title,
                description: prompt,
                parent_id,
                priority,
                agent_type: agent,
                depends_on: deps,
                context,
                idempotency_key,
                source: TaskSource::Human,
                deadline,
                task_type: None,
                execution_mode: None,
            });

            let result = dispatcher.dispatch(cmd).await
                .map_err(|e| anyhow::anyhow!("{}", e))?;

            let task = match result {
                CommandResult::Task(t) => t,
                _ => anyhow::bail!("Unexpected command result"),
            };

            let out = TaskActionOutput {
                success: true,
                message: format!("Task submitted: {} (status: {})", task.id, task.status.as_str()),
                task: Some(TaskOutput::from(&task)),
            };
            output(&out, json_mode);
        }

        TaskCommands::List { status, priority, agent, task_type, ready, limit } => {
            let tasks = if ready {
                service.get_ready_tasks(limit).await?
            } else {
                let filter = TaskFilter {
                    status: status.as_ref().and_then(|s| TaskStatus::from_str(s)),
                    priority: priority.as_ref().and_then(|p| TaskPriority::from_str(p)),
                    agent_type: agent,
                    parent_id: None,
                    task_type: task_type.as_ref().and_then(|t| TaskType::from_str(t)),
                };
                service.list_tasks(filter).await?
            };

            let out = TaskListOutput {
                total: tasks.len(),
                tasks: tasks.iter().map(TaskOutput::from).collect(),
            };
            output(&out, json_mode);
        }

        TaskCommands::Show { id } => {
            let uuid = resolve_task_id(&pool, &id).await?;
            let task = service.get_task(uuid).await?
                .ok_or_else(|| anyhow::anyhow!("Task not found: {}", id))?;

            let out = TaskDetailOutput {
                task: TaskOutput::from(&task),
                description: task.description.clone(),
                context_input: task.context.input.clone(),
                worktree_path: task.worktree_path.clone(),
                created_at: task.created_at.to_rfc3339(),
                started_at: task.started_at.map(|t| t.to_rfc3339()),
                completed_at: task.completed_at.map(|t| t.to_rfc3339()),
                context_custom: task.context.custom.clone(),
            };
            output(&out, json_mode);
        }

        TaskCommands::Cancel { id } => {
            let uuid = resolve_task_id(&pool, &id).await?;

            let cmd = DomainCommand::Task(TaskCommand::Cancel {
                task_id: uuid,
                reason: "user-requested".to_string(),
            });

            let result = dispatcher.dispatch(cmd).await
                .map_err(|e| anyhow::anyhow!("{}", e))?;

            let task = match result {
                CommandResult::Task(t) => t,
                _ => anyhow::bail!("Unexpected command result"),
            };

            let out = TaskActionOutput {
                success: true,
                message: format!("Task canceled: {}", task.id),
                task: Some(TaskOutput::from(&task)),
            };
            output(&out, json_mode);
        }

        TaskCommands::Retry { id } => {
            let uuid = resolve_task_id(&pool, &id).await?;

            let cmd = DomainCommand::Task(TaskCommand::Retry { task_id: uuid });

            let result = dispatcher.dispatch(cmd).await
                .map_err(|e| anyhow::anyhow!("{}", e))?;

            let task = match result {
                CommandResult::Task(t) => t,
                _ => anyhow::bail!("Unexpected command result"),
            };

            let out = TaskActionOutput {
                success: true,
                message: format!("Task retried: {} (retry #{})", task.id, task.retry_count),
                task: Some(TaskOutput::from(&task)),
            };
            output(&out, json_mode);
        }

        TaskCommands::Status => {
            let counts = service.get_status_counts().await?;

            let pending = *counts.get(&TaskStatus::Pending).unwrap_or(&0);
            let ready = *counts.get(&TaskStatus::Ready).unwrap_or(&0);
            let blocked = *counts.get(&TaskStatus::Blocked).unwrap_or(&0);
            let running = *counts.get(&TaskStatus::Running).unwrap_or(&0);
            let complete = *counts.get(&TaskStatus::Complete).unwrap_or(&0);
            let failed = *counts.get(&TaskStatus::Failed).unwrap_or(&0);
            let canceled = *counts.get(&TaskStatus::Canceled).unwrap_or(&0);

            let out = TaskStatusOutput {
                pending,
                ready,
                blocked,
                running,
                complete,
                failed,
                canceled,
                total: pending + ready + blocked + running + complete + failed + canceled,
            };
            output(&out, json_mode);
        }
    }

    Ok(())
}

