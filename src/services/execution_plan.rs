//! Execution Plan for DAG-based Task Execution
//!
//! Provides a topologically-sorted execution plan where tasks are organized
//! into levels that can be executed in parallel within each level.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A level in the execution plan containing tasks that can run in parallel
///
/// All tasks within a level have no dependencies on each other, making them
/// safe to execute concurrently.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExecutionLevel {
    /// Level number (0-indexed, 0 = no dependencies)
    pub level: usize,

    /// Task IDs that can be executed concurrently at this level
    pub task_ids: Vec<Uuid>,
}

impl ExecutionLevel {
    /// Create a new execution level
    pub fn new(level: usize, task_ids: Vec<Uuid>) -> Self {
        Self { level, task_ids }
    }

    /// Get the number of tasks in this level
    pub fn task_count(&self) -> usize {
        self.task_ids.len()
    }

    /// Check if this level is empty
    pub fn is_empty(&self) -> bool {
        self.task_ids.is_empty()
    }
}

/// Execution plan representing a DAG of tasks organized into levels
///
/// Tasks are organized into levels based on their dependency depth.
/// Level 0 contains tasks with no dependencies, level 1 contains tasks
/// that only depend on level 0 tasks, and so on.
///
/// Within each level, all tasks can be executed in parallel since they
/// have no dependencies on each other.
///
/// # Examples
///
/// ```no_run
/// use abathur::services::execution_plan::{ExecutionPlan, ExecutionLevel};
/// use uuid::Uuid;
///
/// let level0 = ExecutionLevel::new(0, vec![Uuid::new_v4(), Uuid::new_v4()]);
/// let level1 = ExecutionLevel::new(1, vec![Uuid::new_v4()]);
///
/// let plan = ExecutionPlan::new(vec![level0, level1]);
/// assert_eq!(plan.total_levels(), 2);
/// assert_eq!(plan.total_tasks(), 3);
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionPlan {
    /// Levels of execution in topological order
    pub levels: Vec<ExecutionLevel>,
}

impl ExecutionPlan {
    /// Create a new execution plan
    ///
    /// # Arguments
    ///
    /// * `levels` - Execution levels in topological order
    ///
    /// # Panics
    ///
    /// Panics if levels are not in sequential order starting from 0
    pub fn new(levels: Vec<ExecutionLevel>) -> Self {
        // Validate that levels are sequential and start from 0
        for (idx, level) in levels.iter().enumerate() {
            assert_eq!(
                level.level, idx,
                "Execution plan levels must be sequential starting from 0"
            );
        }

        Self { levels }
    }

    /// Create an empty execution plan
    pub fn empty() -> Self {
        Self { levels: Vec::new() }
    }

    /// Get the total number of levels in the plan
    pub fn total_levels(&self) -> usize {
        self.levels.len()
    }

    /// Get the total number of tasks across all levels
    pub fn total_tasks(&self) -> usize {
        self.levels.iter().map(|l| l.task_count()).sum()
    }

    /// Get a specific level by index
    pub fn get_level(&self, level: usize) -> Option<&ExecutionLevel> {
        self.levels.get(level)
    }

    /// Check if the plan is empty
    pub fn is_empty(&self) -> bool {
        self.levels.is_empty()
    }

    /// Get all task IDs in the plan
    pub fn all_task_ids(&self) -> Vec<Uuid> {
        self.levels
            .iter()
            .flat_map(|level| level.task_ids.iter())
            .copied()
            .collect()
    }

    /// Validate the execution plan structure
    ///
    /// Checks that:
    /// - Levels are sequential starting from 0
    /// - No duplicate task IDs across levels
    /// - No empty levels (except the plan can be entirely empty)
    ///
    /// # Returns
    ///
    /// * `Ok(())` - Plan is valid
    /// * `Err(String)` - Validation error message
    pub fn validate(&self) -> Result<(), String> {
        // Check levels are sequential
        for (idx, level) in self.levels.iter().enumerate() {
            if level.level != idx {
                return Err(format!(
                    "Level {} has incorrect level number {}",
                    idx, level.level
                ));
            }

            // Check for empty levels
            if level.is_empty() {
                return Err(format!("Level {} is empty", idx));
            }
        }

        // Check for duplicate task IDs
        let all_ids = self.all_task_ids();
        let mut seen = std::collections::HashSet::new();
        for id in &all_ids {
            if !seen.insert(id) {
                return Err(format!("Duplicate task ID found: {}", id));
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_execution_level_creation() {
        let task_ids = vec![Uuid::new_v4(), Uuid::new_v4()];
        let level = ExecutionLevel::new(0, task_ids.clone());

        assert_eq!(level.level, 0);
        assert_eq!(level.task_count(), 2);
        assert!(!level.is_empty());
        assert_eq!(level.task_ids, task_ids);
    }

    #[test]
    fn test_execution_plan_creation() {
        let level0 = ExecutionLevel::new(0, vec![Uuid::new_v4()]);
        let level1 = ExecutionLevel::new(1, vec![Uuid::new_v4(), Uuid::new_v4()]);

        let plan = ExecutionPlan::new(vec![level0, level1]);

        assert_eq!(plan.total_levels(), 2);
        assert_eq!(plan.total_tasks(), 3);
        assert!(!plan.is_empty());
    }

    #[test]
    fn test_execution_plan_empty() {
        let plan = ExecutionPlan::empty();

        assert_eq!(plan.total_levels(), 0);
        assert_eq!(plan.total_tasks(), 0);
        assert!(plan.is_empty());
    }

    #[test]
    #[should_panic(expected = "Execution plan levels must be sequential starting from 0")]
    fn test_execution_plan_non_sequential_levels() {
        let level1 = ExecutionLevel::new(1, vec![Uuid::new_v4()]);
        ExecutionPlan::new(vec![level1]);
    }

    #[test]
    fn test_execution_plan_validation_success() {
        let level0 = ExecutionLevel::new(0, vec![Uuid::new_v4()]);
        let level1 = ExecutionLevel::new(1, vec![Uuid::new_v4()]);

        let plan = ExecutionPlan::new(vec![level0, level1]);
        assert!(plan.validate().is_ok());
    }

    #[test]
    fn test_execution_plan_validation_empty_level() {
        let level0 = ExecutionLevel::new(0, vec![Uuid::new_v4()]);
        let level1 = ExecutionLevel::new(1, vec![]); // Empty level

        let plan = ExecutionPlan { levels: vec![level0, level1] };
        let result = plan.validate();

        assert!(result.is_err());
        assert!(result.unwrap_err().contains("empty"));
    }

    #[test]
    fn test_execution_plan_validation_duplicate_tasks() {
        let task_id = Uuid::new_v4();
        let level0 = ExecutionLevel::new(0, vec![task_id]);
        let level1 = ExecutionLevel::new(1, vec![task_id]); // Duplicate

        let plan = ExecutionPlan { levels: vec![level0, level1] };
        let result = plan.validate();

        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Duplicate"));
    }

    #[test]
    fn test_execution_plan_all_task_ids() {
        let id1 = Uuid::new_v4();
        let id2 = Uuid::new_v4();
        let id3 = Uuid::new_v4();

        let level0 = ExecutionLevel::new(0, vec![id1, id2]);
        let level1 = ExecutionLevel::new(1, vec![id3]);

        let plan = ExecutionPlan::new(vec![level0, level1]);
        let all_ids = plan.all_task_ids();

        assert_eq!(all_ids.len(), 3);
        assert!(all_ids.contains(&id1));
        assert!(all_ids.contains(&id2));
        assert!(all_ids.contains(&id3));
    }
}
