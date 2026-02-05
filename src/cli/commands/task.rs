//! Task CLI commands.

use anyhow::{Context, Result};
use clap::{Args, Subcommand};
use std::sync::Arc;
use uuid::Uuid;

use crate::adapters::sqlite::{
    SqliteGoalRepository, SqliteTaskRepository, initialize_database
};
use crate::cli::id_resolver::resolve_task_id;
use crate::cli::output::{output, CommandOutput};
use crate::domain::models::{Task, TaskContext, TaskPriority, TaskStatus};
use crate::domain::ports::TaskFilter;
use crate::services::TaskService;

#[derive(Args, Debug)]
pub struct TaskArgs {
    #[command(subcommand)]
    pub command: TaskCommands,
}

#[derive(Subcommand, Debug)]
pub enum TaskCommands {
    /// Submit a new task
    Submit {
        /// Task title
        title: String,
        /// Task description
        #[arg(short, long)]
        description: Option<String>,
        /// Priority (low, normal, high, critical)
        #[arg(short, long, default_value = "normal")]
        priority: String,
        /// Associated goal ID
        #[arg(short, long)]
        goal: Option<String>,
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
    },
    /// List tasks
    List {
        /// Filter by status
        #[arg(short, long)]
        status: Option<String>,
        /// Filter by priority
        #[arg(short, long)]
        priority: Option<String>,
        /// Filter by goal ID
        #[arg(short, long)]
        goal: Option<String>,
        /// Filter by agent type
        #[arg(short, long)]
        agent: Option<String>,
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
    pub goal_id: Option<String>,
    pub agent_type: Option<String>,
    pub depends_on: Vec<String>,
    pub retry_count: u32,
}

impl From<&Task> for TaskOutput {
    fn from(task: &Task) -> Self {
        Self {
            id: task.id.to_string(),
            title: task.title.clone(),
            status: task.status.as_str().to_string(),
            priority: task.priority.as_str().to_string(),
            goal_id: task.goal_id.map(|id| id.to_string()),
            agent_type: task.agent_type.clone(),
            depends_on: task.depends_on.iter().map(|id| id.to_string()).collect(),
            retry_count: task.retry_count,
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

        let mut lines = vec![format!("Found {} task(s):\n", self.total)];
        lines.push(format!(
            "{:<12} {:<25} {:<10} {:<10} {:<12}",
            "ID", "TITLE", "STATUS", "PRIORITY", "AGENT"
        ));
        lines.push("-".repeat(70));

        for task in &self.tasks {
            lines.push(format!(
                "{:<12} {:<25} {:<10} {:<10} {:<12}",
                &task.id[..8],
                truncate(&task.title, 23),
                task.status,
                task.priority,
                task.agent_type.as_deref().unwrap_or("-")
            ));
        }

        lines.join("\n")
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
}

impl CommandOutput for TaskDetailOutput {
    fn to_human(&self) -> String {
        let mut lines = vec![
            format!("Task: {}", self.task.title),
            format!("ID: {}", self.task.id),
            format!("Status: {}", self.task.status),
            format!("Priority: {}", self.task.priority),
        ];

        if let Some(goal) = &self.task.goal_id {
            lines.push(format!("Goal: {}", goal));
        }
        if let Some(agent) = &self.task.agent_type {
            lines.push(format!("Agent: {}", agent));
        }

        lines.push(format!("Description: {}", self.description));

        if !self.context_input.is_empty() {
            lines.push(format!("Input: {}", truncate(&self.context_input, 100)));
        }

        if !self.task.depends_on.is_empty() {
            lines.push(format!("\nDependencies ({}):", self.task.depends_on.len()));
            for dep in &self.task.depends_on {
                lines.push(format!("  - {}", &dep[..8]));
            }
        }

        if let Some(path) = &self.worktree_path {
            lines.push(format!("Worktree: {}", path));
        }

        lines.push(format!("\nCreated: {}", self.created_at));
        if let Some(started) = &self.started_at {
            lines.push(format!("Started: {}", started));
        }
        if let Some(completed) = &self.completed_at {
            lines.push(format!("Completed: {}", completed));
        }
        lines.push(format!("Retries: {}", self.task.retry_count));

        lines.join("\n")
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
        self.message.clone()
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
        let mut lines = vec!["Task Status Summary:".to_string()];
        lines.push(format!("  Pending:   {}", self.pending));
        lines.push(format!("  Ready:     {}", self.ready));
        lines.push(format!("  Running:   {}", self.running));
        lines.push(format!("  Blocked:   {}", self.blocked));
        lines.push(format!("  Complete:  {}", self.complete));
        lines.push(format!("  Failed:    {}", self.failed));
        lines.push(format!("  Canceled:  {}", self.canceled));
        lines.push("  -----------".to_string());
        lines.push(format!("  Total:     {}", self.total));
        lines.join("\n")
    }

    fn to_json(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or_default()
    }
}

pub async fn execute(args: TaskArgs, json_mode: bool) -> Result<()> {
    let pool = initialize_database("sqlite:.abathur/abathur.db")
        .await
        .context("Failed to initialize database. Run 'abathur init' first.")?;

    let task_repo = Arc::new(SqliteTaskRepository::new(pool.clone()));
    let goal_repo = Arc::new(SqliteGoalRepository::new(pool.clone()));
    let service = TaskService::new(task_repo, goal_repo);

    match args.command {
        TaskCommands::Submit {
            title,
            description,
            priority,
            goal,
            parent,
            agent,
            depends_on,
            input,
            idempotency_key,
        } => {
            let priority = TaskPriority::from_str(&priority)
                .ok_or_else(|| anyhow::anyhow!("Invalid priority: {}", priority))?;

            let goal_id = goal
                .map(|g| Uuid::parse_str(&g))
                .transpose()
                .context("Invalid goal ID")?;

            let parent_id = parent
                .map(|p| Uuid::parse_str(&p))
                .transpose()
                .context("Invalid parent ID")?;

            let deps: Vec<Uuid> = depends_on
                .iter()
                .map(|d| Uuid::parse_str(d))
                .collect::<std::result::Result<Vec<_>, _>>()
                .context("Invalid dependency ID")?;

            let context = input.map(|i| TaskContext {
                input: i,
                ..Default::default()
            });

            let task = service.submit_task(
                title,
                description.unwrap_or_default(),
                goal_id,
                parent_id,
                priority,
                agent,
                deps,
                context,
                idempotency_key,
            ).await?;

            let out = TaskActionOutput {
                success: true,
                message: format!("Task submitted: {} (status: {})", task.id, task.status.as_str()),
                task: Some(TaskOutput::from(&task)),
            };
            output(&out, json_mode);
        }

        TaskCommands::List { status, priority, goal, agent, ready, limit } => {
            let tasks = if ready {
                service.get_ready_tasks(limit).await?
            } else {
                let filter = TaskFilter {
                    status: status.as_ref().and_then(|s| TaskStatus::from_str(s)),
                    priority: priority.as_ref().and_then(|p| TaskPriority::from_str(p)),
                    goal_id: goal.as_ref().and_then(|g| Uuid::parse_str(g).ok()),
                    agent_type: agent,
                    parent_id: None,
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
            };
            output(&out, json_mode);
        }

        TaskCommands::Cancel { id } => {
            let uuid = Uuid::parse_str(&id).context("Invalid task ID")?;
            let task = service.cancel_task(uuid).await?;

            let out = TaskActionOutput {
                success: true,
                message: format!("Task canceled: {}", task.id),
                task: Some(TaskOutput::from(&task)),
            };
            output(&out, json_mode);
        }

        TaskCommands::Retry { id } => {
            let uuid = Uuid::parse_str(&id).context("Invalid task ID")?;
            let task = service.retry_task(uuid).await?;

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

fn truncate(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        format!("{}...", &s[..max_len - 3])
    }
}
