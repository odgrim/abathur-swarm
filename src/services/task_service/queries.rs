//! Read-only and prune queries: get, list, ready_tasks, status counts, prune.

use uuid::Uuid;

use crate::domain::errors::DomainResult;
use crate::domain::models::{Task, TaskStatus};
use crate::domain::ports::{TaskFilter, TaskRepository};

use super::TaskService;

/// Result of a prune operation.
#[derive(Debug, Clone)]
pub struct PruneResult {
    /// Number of tasks that were (or would be) deleted.
    pub pruned_count: usize,
    /// IDs of tasks that were (or would be) deleted.
    pub pruned_ids: Vec<Uuid>,
    /// Tasks that were skipped (e.g. part of an active DAG).
    pub skipped: Vec<PruneSkipped>,
    /// Whether this was a dry run (no actual deletions).
    pub dry_run: bool,
}

/// A task that was skipped during pruning.
#[derive(Debug, Clone)]
pub struct PruneSkipped {
    pub id: Uuid,
    pub title: String,
    pub reason: String,
}

impl<T: TaskRepository> TaskService<T> {
    /// Get a task by ID.
    pub async fn get_task(&self, id: Uuid) -> DomainResult<Option<Task>> {
        self.task_repo.get(id).await
    }

    /// List tasks with optional filters.
    pub async fn list_tasks(&self, filter: TaskFilter) -> DomainResult<Vec<Task>> {
        self.task_repo.list(filter).await
    }

    /// Get ready tasks ordered by priority.
    pub async fn get_ready_tasks(&self, limit: usize) -> DomainResult<Vec<Task>> {
        self.task_repo.get_ready_tasks(limit).await
    }
    /// Delete a single task by ID.
    pub async fn delete_task(&self, task_id: Uuid) -> DomainResult<()> {
        self.task_repo.delete(task_id).await
    }

    /// Get task status counts.
    pub async fn get_status_counts(
        &self,
    ) -> DomainResult<std::collections::HashMap<TaskStatus, u64>> {
        self.task_repo.count_by_status().await
    }

    /// Prune (delete) tasks matching the given filter.
    ///
    /// By default, tasks that belong to an active DAG (have active ancestors or
    /// descendants) are skipped to avoid breaking running workflows. Pass
    /// `force = true` to override this safety check.
    ///
    /// When `dry_run = true`, no deletions occur — only a preview is returned.
    pub async fn prune_tasks(
        &self,
        filter: TaskFilter,
        force: bool,
        dry_run: bool,
    ) -> DomainResult<PruneResult> {
        let candidates = self.task_repo.list(filter).await?;
        let mut pruned = Vec::new();
        let mut skipped = Vec::new();

        for task in &candidates {
            if !force && self.is_in_active_dag(task).await? {
                skipped.push(PruneSkipped {
                    id: task.id,
                    title: task.title.clone(),
                    reason: "part of an active task DAG".to_string(),
                });
                continue;
            }

            if !dry_run {
                self.task_repo.delete(task.id).await?;
            }
            pruned.push(task.id);
        }

        Ok(PruneResult {
            pruned_count: pruned.len(),
            pruned_ids: pruned,
            skipped,
            dry_run,
        })
    }

    /// Check whether a task belongs to an active DAG.
    ///
    /// A task is in an active DAG if:
    /// - It is itself active (non-terminal), OR
    /// - Any of its dependents (tasks that depend on it) are active, OR
    /// - Any of its dependencies are active, OR
    /// - Its parent is active, OR
    /// - Any sibling under the same parent is active.
    async fn is_in_active_dag(&self, task: &Task) -> DomainResult<bool> {
        // The task itself is active
        if task.status.is_active() {
            return Ok(true);
        }

        // Check dependents (tasks that depend on this one)
        let dependents = self.task_repo.get_dependents(task.id).await?;
        if dependents.iter().any(|t| t.status.is_active()) {
            return Ok(true);
        }

        // Check dependencies
        let deps = self.task_repo.get_dependencies(task.id).await?;
        if deps.iter().any(|t| t.status.is_active()) {
            return Ok(true);
        }

        // Check parent
        if let Some(parent_id) = task.parent_id
            && let Some(parent) = self.task_repo.get(parent_id).await?
            && parent.status.is_active()
        {
            return Ok(true);
        }

        Ok(false)
    }
}
