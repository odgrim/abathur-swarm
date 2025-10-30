//! Task pruning domain models
//!
//! Contains domain types for task deletion operations with dependency validation.
//! These models represent the business concepts around safely removing tasks
//! from the queue while respecting dependency relationships.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Result of a task pruning operation.
///
/// Represents the outcome of attempting to delete one or more tasks from the queue.
/// The operation validates dependencies before deletion to ensure referential integrity
/// and prevent breaking dependency chains.
///
/// # Business Rules
///
/// - Tasks can only be deleted if ALL their dependent tasks are in terminal states
///   (completed, failed, or cancelled)
/// - Terminal states indicate a task will never execute again, making deletion safe
/// - Non-terminal dependents (pending, blocked, ready, running) prevent deletion
/// - Dry-run mode performs validation without actual deletion
///
/// # Examples
///
/// ```rust
/// use abathur::domain::models::{PruneResult, BlockedTask};
/// use uuid::Uuid;
///
/// // Successful dry-run with no blockers
/// let result = PruneResult {
///     deleted_count: 0,
///     deleted_ids: vec![],
///     blocked_tasks: vec![],
///     dry_run: true,
/// };
///
/// // Actual deletion with some blocked tasks
/// let blocked = BlockedTask {
///     task_id: Uuid::new_v4(),
///     reason: "Has 2 non-terminal dependent(s)".to_string(),
///     non_terminal_dependents: vec![Uuid::new_v4(), Uuid::new_v4()],
/// };
///
/// let result = PruneResult {
///     deleted_count: 5,
///     deleted_ids: vec![Uuid::new_v4(); 5],
///     blocked_tasks: vec![blocked],
///     dry_run: false,
/// };
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PruneResult {
    /// Number of tasks successfully deleted.
    ///
    /// In dry-run mode, this will always be 0.
    /// In actual deletion mode, represents tasks that passed validation and were removed.
    pub deleted_count: usize,

    /// UUIDs of tasks that were deleted.
    ///
    /// Empty vector in dry-run mode.
    /// Contains the full UUIDs of successfully deleted tasks in actual deletion mode.
    pub deleted_ids: Vec<Uuid>,

    /// Tasks that could not be deleted due to active dependents.
    ///
    /// Each entry explains why a specific task cannot be safely deleted,
    /// along with the IDs of blocking dependent tasks.
    pub blocked_tasks: Vec<BlockedTask>,

    /// Whether this was a dry-run (validation only, no deletion).
    ///
    /// - `true`: Only validation was performed, no tasks were deleted
    /// - `false`: Actual deletion was attempted for validated tasks
    pub dry_run: bool,
}

impl PruneResult {
    /// Create a new PruneResult for a dry-run operation.
    ///
    /// Dry-run results always have zero deleted tasks and empty deleted_ids.
    ///
    /// # Arguments
    ///
    /// * `blocked_tasks` - Tasks that would be blocked from deletion
    ///
    /// # Examples
    ///
    /// ```rust
    /// use abathur::domain::models::PruneResult;
    ///
    /// let result = PruneResult::dry_run(vec![]);
    /// assert!(result.dry_run);
    /// assert_eq!(result.deleted_count, 0);
    /// ```
    pub fn dry_run(blocked_tasks: Vec<BlockedTask>) -> Self {
        Self {
            deleted_count: 0,
            deleted_ids: vec![],
            blocked_tasks,
            dry_run: true,
        }
    }

    /// Create a new PruneResult for an actual deletion operation.
    ///
    /// # Arguments
    ///
    /// * `deleted_ids` - UUIDs of successfully deleted tasks
    /// * `blocked_tasks` - Tasks that were blocked from deletion
    ///
    /// # Examples
    ///
    /// ```rust
    /// use abathur::domain::models::PruneResult;
    /// use uuid::Uuid;
    ///
    /// let deleted = vec![Uuid::new_v4(), Uuid::new_v4()];
    /// let result = PruneResult::actual_deletion(deleted.clone(), vec![]);
    /// assert!(!result.dry_run);
    /// assert_eq!(result.deleted_count, 2);
    /// assert_eq!(result.deleted_ids, deleted);
    /// ```
    pub fn actual_deletion(deleted_ids: Vec<Uuid>, blocked_tasks: Vec<BlockedTask>) -> Self {
        let deleted_count = deleted_ids.len();
        Self {
            deleted_count,
            deleted_ids,
            blocked_tasks,
            dry_run: false,
        }
    }

    /// Check if the operation completed successfully without any blockers.
    ///
    /// Returns `true` if no tasks were blocked from deletion.
    /// Useful for determining if a dry-run can proceed to actual deletion.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use abathur::domain::models::PruneResult;
    ///
    /// let result = PruneResult::dry_run(vec![]);
    /// assert!(result.is_fully_successful());
    ///
    /// let result_with_blockers = PruneResult::dry_run(vec![/* some blocked tasks */]);
    /// assert!(!result_with_blockers.is_fully_successful());
    /// ```
    pub fn is_fully_successful(&self) -> bool {
        self.blocked_tasks.is_empty()
    }

    /// Get the total number of tasks that were blocked from deletion.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use abathur::domain::models::{PruneResult, BlockedTask};
    /// use uuid::Uuid;
    ///
    /// let blocked = vec![
    ///     BlockedTask {
    ///         task_id: Uuid::new_v4(),
    ///         reason: "reason".to_string(),
    ///         non_terminal_dependents: vec![],
    ///     }
    /// ];
    /// let result = PruneResult::dry_run(blocked);
    /// assert_eq!(result.blocked_count(), 1);
    /// ```
    pub fn blocked_count(&self) -> usize {
        self.blocked_tasks.len()
    }

    /// Get total number of non-terminal dependents across all blocked tasks.
    ///
    /// This represents the total number of tasks that are preventing deletion.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use abathur::domain::models::{PruneResult, BlockedTask};
    /// use uuid::Uuid;
    ///
    /// let blocked = vec![
    ///     BlockedTask {
    ///         task_id: Uuid::new_v4(),
    ///         reason: "reason".to_string(),
    ///         non_terminal_dependents: vec![Uuid::new_v4(), Uuid::new_v4()],
    ///     },
    ///     BlockedTask {
    ///         task_id: Uuid::new_v4(),
    ///         reason: "reason".to_string(),
    ///         non_terminal_dependents: vec![Uuid::new_v4()],
    ///     },
    /// ];
    /// let result = PruneResult::dry_run(blocked);
    /// assert_eq!(result.total_blocking_dependents(), 3);
    /// ```
    pub fn total_blocking_dependents(&self) -> usize {
        self.blocked_tasks
            .iter()
            .map(|b| b.non_terminal_dependents.len())
            .sum()
    }
}

/// A task that was blocked from deletion due to non-terminal dependents.
///
/// Represents a task that cannot be safely deleted because other tasks
/// still depend on it and are not in terminal states.
///
/// # Business Logic
///
/// A task is considered "blocked" from deletion when:
/// - It has one or more dependent tasks (tasks that list it as a dependency)
/// - At least one of those dependents is in a non-terminal state:
///   - Pending: Not yet ready to run
///   - Blocked: Waiting for other dependencies
///   - Ready: Queued for execution
///   - Running: Currently executing
///   - AwaitingValidation: Execution complete, awaiting validation
///   - ValidationRunning: Validation in progress
///   - ValidationFailed: Validation found issues
///
/// Terminal states that allow deletion:
/// - Completed: Successfully finished
/// - Failed: Execution failed
/// - Cancelled: Manually cancelled
///
/// # Examples
///
/// ```rust
/// use abathur::domain::models::BlockedTask;
/// use uuid::Uuid;
///
/// let task_id = Uuid::new_v4();
/// let dep1 = Uuid::new_v4();
/// let dep2 = Uuid::new_v4();
///
/// let blocked = BlockedTask {
///     task_id,
///     reason: "Has 2 non-terminal dependent(s)".to_string(),
///     non_terminal_dependents: vec![dep1, dep2],
/// };
///
/// assert_eq!(blocked.dependent_count(), 2);
/// assert!(blocked.has_blocking_dependents());
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BlockedTask {
    /// UUID of the task that cannot be deleted.
    pub task_id: Uuid,

    /// Human-readable explanation of why the task is blocked.
    ///
    /// Typically includes the number of non-terminal dependents.
    /// Examples:
    /// - "Has 3 non-terminal dependent(s)"
    /// - "Task not found"
    /// - "Failed to fetch task: database error"
    pub reason: String,

    /// UUIDs of dependent tasks that are not in terminal states.
    ///
    /// These are the tasks preventing deletion. They are in states like:
    /// pending, blocked, ready, running, awaiting_validation, validation_running,
    /// or validation_failed.
    ///
    /// Empty if the task is blocked for a reason other than dependents
    /// (e.g., "Task not found").
    pub non_terminal_dependents: Vec<Uuid>,
}

impl BlockedTask {
    /// Create a new BlockedTask for a task with non-terminal dependents.
    ///
    /// Automatically generates a human-readable reason based on the dependent count.
    ///
    /// # Arguments
    ///
    /// * `task_id` - UUID of the task that cannot be deleted
    /// * `non_terminal_dependents` - UUIDs of blocking dependent tasks
    ///
    /// # Examples
    ///
    /// ```rust
    /// use abathur::domain::models::BlockedTask;
    /// use uuid::Uuid;
    ///
    /// let task_id = Uuid::new_v4();
    /// let dependents = vec![Uuid::new_v4(), Uuid::new_v4()];
    ///
    /// let blocked = BlockedTask::with_dependents(task_id, dependents);
    /// assert_eq!(blocked.reason, "Has 2 non-terminal dependent(s)");
    /// ```
    pub fn with_dependents(task_id: Uuid, non_terminal_dependents: Vec<Uuid>) -> Self {
        let count = non_terminal_dependents.len();
        let reason = format!("Has {} non-terminal dependent(s)", count);

        Self {
            task_id,
            reason,
            non_terminal_dependents,
        }
    }

    /// Create a new BlockedTask with a custom reason (no dependents).
    ///
    /// Use this for tasks that are blocked for reasons other than having
    /// non-terminal dependents (e.g., task not found, database errors).
    ///
    /// # Arguments
    ///
    /// * `task_id` - UUID of the task that cannot be deleted
    /// * `reason` - Human-readable explanation
    ///
    /// # Examples
    ///
    /// ```rust
    /// use abathur::domain::models::BlockedTask;
    /// use uuid::Uuid;
    ///
    /// let task_id = Uuid::new_v4();
    /// let blocked = BlockedTask::with_reason(task_id, "Task not found".to_string());
    ///
    /// assert_eq!(blocked.reason, "Task not found");
    /// assert_eq!(blocked.non_terminal_dependents.len(), 0);
    /// ```
    pub fn with_reason(task_id: Uuid, reason: String) -> Self {
        Self {
            task_id,
            reason,
            non_terminal_dependents: vec![],
        }
    }

    /// Check if this task has any blocking dependents.
    ///
    /// Returns `true` if there are non-terminal dependents preventing deletion.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use abathur::domain::models::BlockedTask;
    /// use uuid::Uuid;
    ///
    /// let with_deps = BlockedTask::with_dependents(Uuid::new_v4(), vec![Uuid::new_v4()]);
    /// assert!(with_deps.has_blocking_dependents());
    ///
    /// let without_deps = BlockedTask::with_reason(Uuid::new_v4(), "Not found".to_string());
    /// assert!(!without_deps.has_blocking_dependents());
    /// ```
    pub fn has_blocking_dependents(&self) -> bool {
        !self.non_terminal_dependents.is_empty()
    }

    /// Get the number of blocking dependents.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use abathur::domain::models::BlockedTask;
    /// use uuid::Uuid;
    ///
    /// let blocked = BlockedTask::with_dependents(
    ///     Uuid::new_v4(),
    ///     vec![Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4()]
    /// );
    /// assert_eq!(blocked.dependent_count(), 3);
    /// ```
    pub fn dependent_count(&self) -> usize {
        self.non_terminal_dependents.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_prune_result_dry_run() {
        let result = PruneResult::dry_run(vec![]);

        assert!(result.dry_run);
        assert_eq!(result.deleted_count, 0);
        assert_eq!(result.deleted_ids.len(), 0);
        assert_eq!(result.blocked_tasks.len(), 0);
        assert!(result.is_fully_successful());
    }

    #[test]
    fn test_prune_result_actual_deletion() {
        let id1 = Uuid::new_v4();
        let id2 = Uuid::new_v4();
        let deleted = vec![id1, id2];

        let result = PruneResult::actual_deletion(deleted.clone(), vec![]);

        assert!(!result.dry_run);
        assert_eq!(result.deleted_count, 2);
        assert_eq!(result.deleted_ids, deleted);
        assert!(result.is_fully_successful());
    }

    #[test]
    fn test_prune_result_with_blocked_tasks() {
        let blocked_task = BlockedTask::with_dependents(Uuid::new_v4(), vec![Uuid::new_v4()]);

        let result = PruneResult::dry_run(vec![blocked_task]);

        assert!(!result.is_fully_successful());
        assert_eq!(result.blocked_count(), 1);
    }

    #[test]
    fn test_prune_result_total_blocking_dependents() {
        let blocked1 = BlockedTask::with_dependents(
            Uuid::new_v4(),
            vec![Uuid::new_v4(), Uuid::new_v4()],
        );
        let blocked2 =
            BlockedTask::with_dependents(Uuid::new_v4(), vec![Uuid::new_v4()]);

        let result = PruneResult::dry_run(vec![blocked1, blocked2]);

        assert_eq!(result.total_blocking_dependents(), 3);
    }

    #[test]
    fn test_blocked_task_with_dependents() {
        let task_id = Uuid::new_v4();
        let dep1 = Uuid::new_v4();
        let dep2 = Uuid::new_v4();
        let dependents = vec![dep1, dep2];

        let blocked = BlockedTask::with_dependents(task_id, dependents.clone());

        assert_eq!(blocked.task_id, task_id);
        assert_eq!(blocked.reason, "Has 2 non-terminal dependent(s)");
        assert_eq!(blocked.non_terminal_dependents, dependents);
        assert!(blocked.has_blocking_dependents());
        assert_eq!(blocked.dependent_count(), 2);
    }

    #[test]
    fn test_blocked_task_with_reason() {
        let task_id = Uuid::new_v4();
        let reason = "Task not found".to_string();

        let blocked = BlockedTask::with_reason(task_id, reason.clone());

        assert_eq!(blocked.task_id, task_id);
        assert_eq!(blocked.reason, reason);
        assert_eq!(blocked.non_terminal_dependents.len(), 0);
        assert!(!blocked.has_blocking_dependents());
        assert_eq!(blocked.dependent_count(), 0);
    }

    #[test]
    fn test_blocked_task_serialization() {
        let task_id = Uuid::new_v4();
        let blocked = BlockedTask::with_reason(task_id, "Test reason".to_string());

        // Test that it can be serialized
        let json = serde_json::to_string(&blocked).expect("Failed to serialize");
        assert!(json.contains(&task_id.to_string()));
        assert!(json.contains("Test reason"));

        // Test that it can be deserialized
        let deserialized: BlockedTask =
            serde_json::from_str(&json).expect("Failed to deserialize");
        assert_eq!(deserialized, blocked);
    }

    #[test]
    fn test_prune_result_serialization() {
        let id = Uuid::new_v4();
        let result = PruneResult::actual_deletion(vec![id], vec![]);

        // Test that it can be serialized
        let json = serde_json::to_string(&result).expect("Failed to serialize");
        assert!(json.contains(&id.to_string()));
        assert!(json.contains("\"deleted_count\":1"));
        assert!(json.contains("\"dry_run\":false"));

        // Test that it can be deserialized
        let deserialized: PruneResult =
            serde_json::from_str(&json).expect("Failed to deserialize");
        assert_eq!(deserialized, result);
    }

    #[test]
    fn test_prune_result_equality() {
        let id1 = Uuid::new_v4();
        let id2 = Uuid::new_v4();

        let result1 = PruneResult::actual_deletion(vec![id1], vec![]);
        let result2 = PruneResult::actual_deletion(vec![id1], vec![]);
        let result3 = PruneResult::actual_deletion(vec![id2], vec![]);

        assert_eq!(result1, result2);
        assert_ne!(result1, result3);
    }

    #[test]
    fn test_blocked_task_equality() {
        let task_id = Uuid::new_v4();
        let dep_id = Uuid::new_v4();

        let blocked1 = BlockedTask::with_dependents(task_id, vec![dep_id]);
        let blocked2 = BlockedTask::with_dependents(task_id, vec![dep_id]);
        let blocked3 = BlockedTask::with_reason(task_id, "Different".to_string());

        assert_eq!(blocked1, blocked2);
        assert_ne!(blocked1, blocked3);
    }

    #[test]
    fn test_prune_result_methods_with_complex_scenario() {
        // Create a complex scenario with multiple blocked tasks
        let deleted = vec![Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4()];

        let blocked1 = BlockedTask::with_dependents(
            Uuid::new_v4(),
            vec![Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4()],
        );
        let blocked2 = BlockedTask::with_dependents(Uuid::new_v4(), vec![Uuid::new_v4()]);
        let blocked3 = BlockedTask::with_reason(Uuid::new_v4(), "Not found".to_string());

        let result = PruneResult::actual_deletion(
            deleted.clone(),
            vec![blocked1, blocked2, blocked3],
        );

        assert!(!result.dry_run);
        assert_eq!(result.deleted_count, 3);
        assert_eq!(result.deleted_ids, deleted);
        assert!(!result.is_fully_successful());
        assert_eq!(result.blocked_count(), 3);
        assert_eq!(result.total_blocking_dependents(), 4); // 3 + 1 + 0
    }

    #[test]
    fn test_blocked_task_dependent_count_edge_cases() {
        // Zero dependents
        let blocked_zero = BlockedTask::with_dependents(Uuid::new_v4(), vec![]);
        assert_eq!(blocked_zero.dependent_count(), 0);
        assert!(!blocked_zero.has_blocking_dependents());

        // Single dependent
        let blocked_one = BlockedTask::with_dependents(Uuid::new_v4(), vec![Uuid::new_v4()]);
        assert_eq!(blocked_one.dependent_count(), 1);
        assert!(blocked_one.has_blocking_dependents());

        // Many dependents
        let many_deps: Vec<Uuid> = (0..10).map(|_| Uuid::new_v4()).collect();
        let blocked_many = BlockedTask::with_dependents(Uuid::new_v4(), many_deps);
        assert_eq!(blocked_many.dependent_count(), 10);
        assert!(blocked_many.has_blocking_dependents());
    }
}
