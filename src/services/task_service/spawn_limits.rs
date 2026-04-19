//! Spawn-limit configuration, result types, and enforcement logic.

use uuid::Uuid;

use crate::domain::errors::{DomainError, DomainResult};
use crate::domain::models::Task;
use crate::domain::ports::TaskRepository;

use super::TaskService;

/// Configuration for spawn limits.
#[derive(Debug, Clone)]
pub struct SpawnLimitConfig {
    /// Maximum depth of subtask nesting.
    pub max_subtask_depth: u32,
    /// Maximum number of direct subtasks per task.
    pub max_subtasks_per_task: u32,
    /// Maximum total descendants from a root task.
    pub max_total_descendants: u32,
    /// Whether to allow extension requests when limits are reached.
    pub allow_limit_extensions: bool,
}

impl Default for SpawnLimitConfig {
    fn default() -> Self {
        Self {
            max_subtask_depth: 5,
            max_subtasks_per_task: 10,
            max_total_descendants: 100,
            allow_limit_extensions: true,
        }
    }
}

/// Result of spawn limit checking.
#[derive(Debug, Clone)]
pub enum SpawnLimitResult {
    /// Task creation is allowed.
    Allowed,
    /// Limit exceeded but extension may be granted.
    LimitExceeded {
        limit_type: SpawnLimitType,
        current_value: u32,
        limit_value: u32,
        can_request_extension: bool,
    },
    /// Hard limit - cannot create task.
    HardLimit {
        limit_type: SpawnLimitType,
        reason: String,
    },
}

impl SpawnLimitResult {
    pub fn is_allowed(&self) -> bool {
        matches!(self, Self::Allowed)
    }

    pub fn requires_specialist(&self) -> bool {
        matches!(
            self,
            Self::LimitExceeded {
                can_request_extension: true,
                ..
            }
        )
    }
}

/// Type of spawn limit that was exceeded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpawnLimitType {
    SubtaskDepth,
    SubtasksPerTask,
    TotalDescendants,
}

impl SpawnLimitType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::SubtaskDepth => "subtask_depth",
            Self::SubtasksPerTask => "subtasks_per_task",
            Self::TotalDescendants => "total_descendants",
        }
    }
}

impl<T: TaskRepository> TaskService<T> {
    /// Check spawn limits for creating a subtask under a parent.
    ///
    /// Returns `SpawnLimitResult` indicating whether the task can be created,
    /// and if not, whether a limit evaluation specialist should be triggered.
    pub async fn check_spawn_limits(
        &self,
        parent_id: Option<Uuid>,
    ) -> DomainResult<SpawnLimitResult> {
        let Some(parent_id) = parent_id else {
            // No parent = root task, no spawn limits apply
            return Ok(SpawnLimitResult::Allowed);
        };

        let parent = self
            .task_repo
            .get(parent_id)
            .await?
            .ok_or(DomainError::TaskNotFound(parent_id))?;

        // Check subtask depth
        let depth = self.calculate_depth(&parent).await?;
        if depth >= self.spawn_limits.max_subtask_depth {
            tracing::warn!(%parent_id, current_depth = depth, max_depth = self.spawn_limits.max_subtask_depth, "spawn limit exceeded: subtask depth");
            return Ok(SpawnLimitResult::LimitExceeded {
                limit_type: SpawnLimitType::SubtaskDepth,
                current_value: depth,
                limit_value: self.spawn_limits.max_subtask_depth,
                can_request_extension: self.spawn_limits.allow_limit_extensions,
            });
        }

        // Check direct subtasks count
        let direct_subtasks = self.count_direct_subtasks(parent_id).await?;
        if direct_subtasks >= self.spawn_limits.max_subtasks_per_task {
            tracing::warn!(%parent_id, current_count = direct_subtasks, max_count = self.spawn_limits.max_subtasks_per_task, "spawn limit exceeded: subtasks per task");
            return Ok(SpawnLimitResult::LimitExceeded {
                limit_type: SpawnLimitType::SubtasksPerTask,
                current_value: direct_subtasks,
                limit_value: self.spawn_limits.max_subtasks_per_task,
                can_request_extension: self.spawn_limits.allow_limit_extensions,
            });
        }

        // Check total descendants from root
        let root_id = self.find_root_task(&parent).await?;
        let total_descendants = self.count_all_descendants(root_id).await?;
        if total_descendants >= self.spawn_limits.max_total_descendants {
            tracing::warn!(%parent_id, current_count = total_descendants, max_count = self.spawn_limits.max_total_descendants, "spawn limit exceeded: total descendants");
            return Ok(SpawnLimitResult::LimitExceeded {
                limit_type: SpawnLimitType::TotalDescendants,
                current_value: total_descendants,
                limit_value: self.spawn_limits.max_total_descendants,
                can_request_extension: self.spawn_limits.allow_limit_extensions,
            });
        }

        Ok(SpawnLimitResult::Allowed)
    }

    /// Calculate the depth of a task in the hierarchy (0 = root).
    ///
    /// Delegates to a single recursive CTE query in the repository layer,
    /// eliminating the N+1 query pattern of walking up one parent at a time.
    async fn calculate_depth(&self, task: &Task) -> DomainResult<u32> {
        self.task_repo.calculate_depth(task.id).await
    }

    /// Count direct subtasks of a task (single COUNT query, no row loading).
    async fn count_direct_subtasks(&self, parent_id: Uuid) -> DomainResult<u32> {
        self.task_repo.count_children(parent_id).await
    }

    /// Find the root task (task with no parent).
    ///
    /// Delegates to a single recursive CTE query in the repository layer,
    /// eliminating the N+1 query pattern of walking up one parent at a time.
    async fn find_root_task(&self, task: &Task) -> DomainResult<Uuid> {
        self.task_repo.find_root_task_id(task.id).await
    }

    /// Count all descendants of a task using a single recursive CTE query.
    async fn count_all_descendants(&self, task_id: Uuid) -> DomainResult<u32> {
        Ok(self.task_repo.count_descendants(task_id).await? as u32)
    }
}
