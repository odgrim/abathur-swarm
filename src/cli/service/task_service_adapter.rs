use anyhow::{anyhow, Context, Result};
use uuid::Uuid;

use crate::cli::models::{QueueStats, Task as CliTask, TaskStatus as CliTaskStatus};
use crate::domain::models::{Task as DomainTask, TaskStatus as DomainTaskStatus};
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
        description: String,
        agent_type: String,
        priority: u8,
        dependencies: Vec<Uuid>,
    ) -> Result<Uuid> {
        // Create domain task
        let mut task = DomainTask::new(description.clone(), description.clone());
        task.agent_type = agent_type;
        task.priority = priority;
        if !dependencies.is_empty() {
            task.dependencies = Some(dependencies);
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

        cli_tasks.truncate(limit);
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
                DomainTaskStatus::Completed => stats.completed += 1,
                DomainTaskStatus::Failed => stats.failed += 1,
                DomainTaskStatus::Cancelled => stats.cancelled += 1,
            }
        }

        Ok(stats)
    }
}

// Conversion functions
fn convert_domain_to_cli_task(domain_task: DomainTask) -> CliTask {
    CliTask {
        id: domain_task.id,
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
    }
}

fn convert_domain_to_cli_status(status: DomainTaskStatus) -> CliTaskStatus {
    match status {
        DomainTaskStatus::Pending => CliTaskStatus::Pending,
        DomainTaskStatus::Blocked => CliTaskStatus::Blocked,
        DomainTaskStatus::Ready => CliTaskStatus::Ready,
        DomainTaskStatus::Running => CliTaskStatus::Running,
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
