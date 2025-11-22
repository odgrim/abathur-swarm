use anyhow::{anyhow, Context, Result};
use uuid::Uuid;

use crate::cli::models::{QueueStats, Task as CliTask, TaskStatus as CliTaskStatus};
use crate::domain::models::{Task as DomainTask, TaskStatus as DomainTaskStatus, PruneResult};
use crate::domain::ports::TaskFilters;
use crate::services::TaskQueueService as RealTaskQueueService;

/// Adapter to make the domain TaskQueueService compatible with CLI commands
pub struct TaskQueueServiceAdapter {
    service: RealTaskQueueService,
}

impl TaskQueueServiceAdapter {
    pub fn new(service: RealTaskQueueService) -> Self {
        Self { service }
    }

    /// Submit a new task to the queue
    pub async fn submit_task(
        &self,
        summary: String,
        description: String,
        agent_type: String,
        priority: u8,
        dependencies: Vec<Uuid>,
        chain_id: Option<String>,
        feature_branch: Option<String>,
        needs_worktree: bool,
    ) -> Result<Uuid> {
        // Create domain task
        let mut task = DomainTask::new(summary.clone(), description);
        task.agent_type = agent_type;
        task.priority = priority;
        if !dependencies.is_empty() {
            task.dependencies = Some(dependencies);
        }
        task.chain_id = chain_id;
        task.feature_branch = feature_branch.clone();

        // Generate branch and worktree_path if needs_worktree is true
        if needs_worktree && feature_branch.is_some() {
            let feature_name = feature_branch
                .as_ref()
                .and_then(|fb| fb.strip_prefix("feature/"))
                .unwrap_or("unknown");

            let task_uuid = Uuid::new_v4();

            // Generate task_id slug from summary (simplified version - take first few words)
            let task_id_slug = summary
                .to_lowercase()
                .chars()
                .map(|c| if c.is_alphanumeric() { c } else { '-' })
                .collect::<String>()
                .split('-')
                .filter(|s| !s.is_empty())
                .take(3)
                .collect::<Vec<&str>>()
                .join("-");

            // Generate branch name: task/{feature_name}/{task_id_slug}
            task.branch = Some(format!("task/{}/{}", feature_name, task_id_slug));

            // Generate worktree path: .abathur/worktrees/task-{uuid}
            task.worktree_path = Some(format!(".abathur/worktrees/task-{}", task_uuid));
        }

        // Submit via real service
        self.service.submit(task).await
    }

    /// List tasks with optional filtering
    pub async fn list_tasks(
        &self,
        status_filter: Option<CliTaskStatus>,
        limit: usize,
    ) -> Result<Vec<CliTask>> {
        let domain_status = status_filter.map(convert_cli_to_domain_status);

        let filters = TaskFilters {
            status: domain_status,
            limit: Some(limit),
            ..Default::default()
        };

        let domain_tasks = self.service.list(filters).await?;

        // Convert to CLI tasks and apply limit
        let mut cli_tasks: Vec<CliTask> = domain_tasks
            .into_iter()
            .map(convert_domain_to_cli_task)
            .collect();

        // Sort by computed priority (highest first)
        cli_tasks.sort_by(|a, b| {
            b.computed_priority
                .partial_cmp(&a.computed_priority)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        Ok(cli_tasks)
    }

    /// Get task by ID
    pub async fn get_task(&self, task_id: Uuid) -> Result<Option<CliTask>> {
        let domain_task = self.service.get(task_id).await?;
        Ok(domain_task.map(convert_domain_to_cli_task))
    }

    /// Update task fields
    pub async fn update_task(
        &self,
        task_id: Uuid,
        status: Option<&str>,
        priority: Option<u8>,
        agent_type: Option<String>,
        add_dependencies: Vec<Uuid>,
        remove_dependencies: Vec<Uuid>,
        retry: bool,
        cancel: bool,
    ) -> Result<()> {
        // Get the task
        let mut task = self
            .service
            .get(task_id)
            .await?
            .ok_or_else(|| anyhow!("Task {} not found", task_id))?;

        // Handle special operations first
        if cancel {
            return self
                .service
                .cancel(task_id)
                .await
                .context(format!("Failed to cancel task {}", task_id));
        }

        if retry {
            if task.status != DomainTaskStatus::Failed {
                return Err(anyhow!(
                    "Can only retry failed tasks. Task {} is in {:?} state",
                    task_id,
                    task.status
                ));
            }
            task.retry()
                .context("Failed to retry task")?;
        }

        // Update status if provided
        if let Some(status_str) = status {
            let new_status = match status_str.to_lowercase().as_str() {
                "pending" => DomainTaskStatus::Pending,
                "blocked" => DomainTaskStatus::Blocked,
                "ready" => DomainTaskStatus::Ready,
                "running" => DomainTaskStatus::Running,
                "completed" => DomainTaskStatus::Completed,
                "failed" => DomainTaskStatus::Failed,
                "cancelled" => DomainTaskStatus::Cancelled,
                _ => return Err(anyhow!("Invalid status: {}", status_str)),
            };
            task.status = new_status;
        }

        // Update priority if provided
        if let Some(new_priority) = priority {
            if new_priority > 10 {
                return Err(anyhow!("Priority must be between 0 and 10"));
            }
            task.priority = new_priority;
            task.update_calculated_priority();
        }

        // Update agent type if provided
        if let Some(new_agent_type) = agent_type {
            task.agent_type = new_agent_type;
        }

        // Handle dependency modifications
        if !add_dependencies.is_empty() || !remove_dependencies.is_empty() {
            let mut deps = task.dependencies.unwrap_or_default();

            // Add new dependencies
            for dep in add_dependencies {
                if !deps.contains(&dep) {
                    deps.push(dep);
                }
            }

            // Remove dependencies
            deps.retain(|dep| !remove_dependencies.contains(dep));

            task.dependencies = if deps.is_empty() { None } else { Some(deps) };
        }

        // Update timestamp
        task.last_updated_at = chrono::Utc::now();

        // Save the updated task
        self.service
            .repo
            .update(&task)
            .await
            .context("Failed to update task in repository")?;

        Ok(())
    }

    /// Get queue statistics
    pub async fn get_queue_stats(&self) -> Result<QueueStats> {
        let all_tasks = self
            .service
            .list(TaskFilters::default())
            .await
            .context("Failed to list all tasks")?;

        let total = all_tasks.len();
        let mut stats = QueueStats {
            total,
            pending: 0,
            blocked: 0,
            ready: 0,
            running: 0,
            completed: 0,
            failed: 0,
            cancelled: 0,
        };

        for task in all_tasks {
            match task.status {
                DomainTaskStatus::Pending => stats.pending += 1,
                DomainTaskStatus::Blocked => stats.blocked += 1,
                DomainTaskStatus::Ready => stats.ready += 1,
                DomainTaskStatus::Running => stats.running += 1,
                DomainTaskStatus::AwaitingValidation |
                DomainTaskStatus::ValidationRunning => stats.running += 1, // Count validation as running
                DomainTaskStatus::ValidationFailed => stats.failed += 1, // Count validation failures as failed
                DomainTaskStatus::Completed => stats.completed += 1,
                DomainTaskStatus::Failed => stats.failed += 1,
                DomainTaskStatus::Cancelled => stats.cancelled += 1,
            }
        }

        Ok(stats)
    }

    /// Resolve task dependencies
    ///
    /// Updates Pending/Blocked tasks to Ready if their dependencies are met.
    /// Returns the number of tasks updated.
    pub async fn resolve_dependencies(&self) -> Result<usize> {
        self.service
            .resolve_dependencies()
            .await
            .context("Failed to resolve dependencies")
    }

    /// Resolve a task ID prefix to a full UUID
    ///
    /// Searches for tasks whose ID starts with the given prefix.
    /// Returns an error if the prefix matches zero or multiple tasks.
    pub async fn resolve_task_id_prefix(&self, prefix: &str) -> Result<Uuid> {
        // Try to parse as full UUID first
        if let Ok(uuid) = Uuid::parse_str(prefix) {
            return Ok(uuid);
        }

        // Validate prefix format (should be hex)
        if !prefix.chars().all(|c| c.is_ascii_hexdigit() || c == '-') {
            return Err(anyhow!(
                "Invalid task ID prefix '{}': must contain only hexadecimal characters",
                prefix
            ));
        }

        // Get all tasks and search for matching prefixes
        let all_tasks = self
            .service
            .list(TaskFilters::default())
            .await
            .context("Failed to list tasks for prefix matching")?;

        let prefix_lower = prefix.to_lowercase();
        let matches: Vec<&DomainTask> = all_tasks
            .iter()
            .filter(|task| task.id.to_string().to_lowercase().starts_with(&prefix_lower))
            .collect();

        match matches.len() {
            0 => Err(anyhow!(
                "No task found with ID prefix '{}'. Use 'abathur task list' to see available tasks.",
                prefix
            )),
            1 => Ok(matches[0].id),
            n => {
                let matching_ids: Vec<String> = matches
                    .iter()
                    .map(|t| t.id.to_string()[..8].to_string())
                    .collect();
                Err(anyhow!(
                    "Task ID prefix '{}' is ambiguous, matches {} tasks: {}. Please provide a longer prefix.",
                    prefix,
                    n,
                    matching_ids.join(", ")
                ))
            }
        }
    }

    /// Prune (delete) tasks with dependency validation
    ///
    /// Validates and deletes tasks from the queue. Tasks can only be deleted if all their
    /// dependent tasks are in terminal states (completed, failed, or cancelled).
    ///
    /// # Arguments
    /// * `task_ids` - UUIDs of tasks to prune
    /// * `dry_run` - If true, only validate without performing deletion
    ///
    /// # Returns
    /// `PruneResult` containing deletion results and any blocked tasks
    pub async fn prune_tasks(&self, task_ids: Vec<Uuid>, dry_run: bool) -> Result<PruneResult> {
        self.service
            .validate_and_prune_tasks(task_ids, dry_run)
            .await
            .context("Failed to prune tasks")
    }
}

// Conversion functions
fn convert_domain_to_cli_task(domain_task: DomainTask) -> CliTask {
    CliTask {
        id: domain_task.id,
        summary: domain_task.summary,
        description: domain_task.description,
        status: convert_domain_to_cli_status(domain_task.status),
        agent_type: domain_task.agent_type,
        priority: domain_task.priority,
        base_priority: domain_task.priority,
        computed_priority: domain_task.calculated_priority,
        dependencies: domain_task.dependencies.unwrap_or_default(),
        created_at: domain_task.submitted_at,
        updated_at: domain_task.last_updated_at,
        started_at: domain_task.started_at,
        completed_at: domain_task.completed_at,
        chain_id: domain_task.chain_id,
        feature_branch: domain_task.feature_branch,
        branch: domain_task.branch,
        worktree_path: domain_task.worktree_path,
    }
}

fn convert_domain_to_cli_status(status: DomainTaskStatus) -> CliTaskStatus {
    match status {
        DomainTaskStatus::Pending => CliTaskStatus::Pending,
        DomainTaskStatus::Blocked => CliTaskStatus::Blocked,
        DomainTaskStatus::Ready => CliTaskStatus::Ready,
        DomainTaskStatus::Running => CliTaskStatus::Running,
        DomainTaskStatus::AwaitingValidation |
        DomainTaskStatus::ValidationRunning => CliTaskStatus::Running, // Map validation statuses to Running
        DomainTaskStatus::ValidationFailed => CliTaskStatus::Failed, // Map validation failure to Failed
        DomainTaskStatus::Completed => CliTaskStatus::Completed,
        DomainTaskStatus::Failed => CliTaskStatus::Failed,
        DomainTaskStatus::Cancelled => CliTaskStatus::Cancelled,
    }
}

fn convert_cli_to_domain_status(status: CliTaskStatus) -> DomainTaskStatus {
    match status {
        CliTaskStatus::Pending => DomainTaskStatus::Pending,
        CliTaskStatus::Blocked => DomainTaskStatus::Blocked,
        CliTaskStatus::Ready => DomainTaskStatus::Ready,
        CliTaskStatus::Running => DomainTaskStatus::Running,
        CliTaskStatus::Completed => DomainTaskStatus::Completed,
        CliTaskStatus::Failed => DomainTaskStatus::Failed,
        CliTaskStatus::Cancelled => DomainTaskStatus::Cancelled,
    }
}
