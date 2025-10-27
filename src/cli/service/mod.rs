mod swarm_service;
mod task_service_adapter;

use anyhow::{anyhow, Result};
use chrono::Utc;
use uuid::Uuid;

use super::models::{QueueStats, Task, TaskStatus};

pub use swarm_service::SwarmService;
pub use task_service_adapter::TaskQueueServiceAdapter;

/// Mock task queue service for CLI development
/// This will be replaced with the actual service layer implementation
pub struct TaskQueueService {
    // In a real implementation, this would connect to the database
    // For now, we'll use in-memory storage for demonstration
    tasks: std::sync::Arc<tokio::sync::Mutex<Vec<Task>>>,
}

impl TaskQueueService {
    pub fn new() -> Self {
        Self {
            tasks: std::sync::Arc::new(tokio::sync::Mutex::new(Vec::new())),
        }
    }

    /// Submit a new task to the queue
    pub async fn submit_task(
        &self,
        description: String,
        agent_type: String,
        priority: u8,
        dependencies: Vec<Uuid>,
    ) -> Result<Uuid> {
        let task_id = Uuid::new_v4();
        let now = Utc::now();

        let status = if dependencies.is_empty() {
            TaskStatus::Ready
        } else {
            TaskStatus::Blocked
        };

        let task = Task {
            id: task_id,
            description,
            status,
            agent_type,
            priority,
            base_priority: priority,
            computed_priority: priority as f64,
            dependencies,
            created_at: now,
            updated_at: now,
            started_at: None,
            completed_at: None,
        };

        let mut tasks = self.tasks.lock().await;
        tasks.push(task);

        Ok(task_id)
    }

    /// List tasks with optional filtering
    pub async fn list_tasks(
        &self,
        status_filter: Option<TaskStatus>,
        limit: usize,
    ) -> Result<Vec<Task>> {
        let tasks = self.tasks.lock().await;
        let mut filtered: Vec<Task> = tasks
            .iter()
            .filter(|t| match status_filter {
                Some(status) => t.status == status,
                None => true,
            })
            .cloned()
            .collect();

        // Sort by computed priority (highest first)
        filtered.sort_by(|a, b| {
            b.computed_priority
                .partial_cmp(&a.computed_priority)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        filtered.truncate(limit);
        Ok(filtered)
    }

    /// Get task by ID
    pub async fn get_task(&self, task_id: Uuid) -> Result<Option<Task>> {
        let tasks = self.tasks.lock().await;
        Ok(tasks.iter().find(|t| t.id == task_id).cloned())
    }

    /// Cancel a task
    pub async fn cancel_task(&self, task_id: Uuid) -> Result<()> {
        let mut tasks = self.tasks.lock().await;
        let task = tasks
            .iter_mut()
            .find(|t| t.id == task_id)
            .ok_or_else(|| anyhow!("Task {} not found", task_id))?;

        if matches!(task.status, TaskStatus::Completed | TaskStatus::Failed) {
            return Err(anyhow!("Cannot cancel task in {} state", task.status));
        }

        task.status = TaskStatus::Cancelled;
        task.updated_at = Utc::now();
        Ok(())
    }

    /// Retry a failed task
    pub async fn retry_task(&self, task_id: Uuid) -> Result<Uuid> {
        // First, get and validate the original task
        let (description, agent_type, base_priority, dependencies) = {
            let tasks = self.tasks.lock().await;
            let original_task = tasks
                .iter()
                .find(|t| t.id == task_id)
                .ok_or_else(|| anyhow!("Task {} not found", task_id))?;

            if original_task.status != TaskStatus::Failed {
                return Err(anyhow!(
                    "Can only retry failed tasks. Task {} is in {} state",
                    task_id,
                    original_task.status
                ));
            }

            // Clone the data we need before releasing the lock
            (
                original_task.description.clone(),
                original_task.agent_type.clone(),
                original_task.base_priority,
                original_task.dependencies.clone(),
            )
        }; // Lock is released here

        // Create a new task with the same parameters
        self.submit_task(description, agent_type, base_priority, dependencies)
            .await
    }

    /// Get queue statistics
    pub async fn get_queue_stats(&self) -> Result<QueueStats> {
        let tasks = self.tasks.lock().await;
        let total = tasks.len();

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

        for task in tasks.iter() {
            match task.status {
                TaskStatus::Pending => stats.pending += 1,
                TaskStatus::Blocked => stats.blocked += 1,
                TaskStatus::Ready => stats.ready += 1,
                TaskStatus::Running => stats.running += 1,
                TaskStatus::Completed => stats.completed += 1,
                TaskStatus::Failed => stats.failed += 1,
                TaskStatus::Cancelled => stats.cancelled += 1,
            }
        }

        Ok(stats)
    }
}

impl Default for TaskQueueService {
    fn default() -> Self {
        Self::new()
    }
}

/// Mock memory service for CLI development
/// This will be replaced with the actual service layer implementation
pub struct MemoryService {
    // In a real implementation, this would connect to the database
    // For now, we'll use in-memory storage for demonstration
    memories: std::sync::Arc<tokio::sync::Mutex<Vec<crate::domain::models::Memory>>>,
}

impl MemoryService {
    pub fn new() -> Self {
        Self {
            memories: std::sync::Arc::new(tokio::sync::Mutex::new(Vec::new())),
        }
    }

    /// Search memories by namespace prefix and type
    pub async fn search(
        &self,
        namespace_prefix: &str,
        memory_type: Option<crate::domain::models::MemoryType>,
        limit: Option<usize>,
    ) -> Result<Vec<crate::domain::models::Memory>> {
        let memories = self.memories.lock().await;
        let limit = limit.unwrap_or(50);

        let filtered: Vec<_> = memories
            .iter()
            .filter(|m| {
                let namespace_matches = m.namespace.starts_with(namespace_prefix);
                let type_matches = memory_type.map_or(true, |mt| m.memory_type == mt);
                let is_active = m.is_active();
                namespace_matches && type_matches && is_active
            })
            .take(limit)
            .cloned()
            .collect();

        Ok(filtered)
    }

    /// Get latest version of a memory
    pub async fn get(
        &self,
        namespace: &str,
        key: &str,
    ) -> Result<Option<crate::domain::models::Memory>> {
        let memories = self.memories.lock().await;
        Ok(memories
            .iter()
            .filter(|m| m.namespace == namespace && m.key == key && m.is_active())
            .max_by_key(|m| m.version)
            .cloned())
    }

    /// Get specific version of a memory
    pub async fn get_version(
        &self,
        namespace: &str,
        key: &str,
        version: u32,
    ) -> Result<Option<crate::domain::models::Memory>> {
        let memories = self.memories.lock().await;
        Ok(memories
            .iter()
            .find(|m| m.namespace == namespace && m.key == key && m.version == version)
            .cloned())
    }

    /// List all versions of a memory
    pub async fn list_versions(
        &self,
        namespace: &str,
        key: &str,
    ) -> Result<Vec<crate::domain::models::Memory>> {
        let memories = self.memories.lock().await;
        let mut versions: Vec<_> = memories
            .iter()
            .filter(|m| m.namespace == namespace && m.key == key)
            .cloned()
            .collect();

        versions.sort_by_key(|m| m.version);
        Ok(versions)
    }

    /// Count memories matching criteria
    pub async fn count(
        &self,
        namespace_prefix: &str,
        memory_type: Option<crate::domain::models::MemoryType>,
    ) -> Result<usize> {
        let memories = self.memories.lock().await;
        let count = memories
            .iter()
            .filter(|m| {
                let namespace_matches = m.namespace.starts_with(namespace_prefix);
                let type_matches = memory_type.map_or(true, |mt| m.memory_type == mt);
                let is_active = m.is_active();
                namespace_matches && type_matches && is_active
            })
            .count();

        Ok(count)
    }
}

impl Default for MemoryService {
    fn default() -> Self {
        Self::new()
    }
}
