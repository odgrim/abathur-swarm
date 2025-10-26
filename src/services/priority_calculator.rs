use crate::domain::models::Task;
use chrono::Utc;

/// Service for calculating dynamic task priorities
///
/// Priority formula: base_priority + (dependency_depth * 0.5) + deadline_boost
#[derive(Debug, Clone)]
pub struct PriorityCalculator {
    depth_weight: f64,
    deadline_boost_max: f64,
}

impl Default for PriorityCalculator {
    fn default() -> Self {
        Self::new()
    }
}

impl PriorityCalculator {
    /// Create a new priority calculator with default weights
    pub fn new() -> Self {
        Self {
            depth_weight: 0.5,
            deadline_boost_max: 3.0,
        }
    }

    /// Create a priority calculator with custom weights
    pub fn with_weights(depth_weight: f64, deadline_boost_max: f64) -> Self {
        Self {
            depth_weight,
            deadline_boost_max,
        }
    }

    /// Calculate the priority for a task
    ///
    /// # Arguments
    /// * `task` - The task to calculate priority for
    /// * `dependency_depth` - The depth in the dependency chain (0 = no dependencies)
    ///
    /// # Returns
    /// The calculated priority as f64
    pub fn calculate(&self, task: &Task, dependency_depth: u32) -> f64 {
        let base = task.priority as f64;
        let depth_boost = dependency_depth as f64 * self.depth_weight;
        let deadline_boost = self.calculate_deadline_boost(task);

        base + depth_boost + deadline_boost
    }

    /// Calculate the deadline boost based on time remaining
    ///
    /// Returns a boost between 0 and deadline_boost_max
    /// - Tasks with no deadline get 0 boost
    /// - Tasks past deadline get max boost
    /// - Tasks with deadline approaching get proportional boost
    fn calculate_deadline_boost(&self, task: &Task) -> f64 {
        if let Some(deadline) = task.deadline {
            let now = Utc::now();

            if deadline <= now {
                // Deadline passed - maximum boost
                return self.deadline_boost_max;
            }

            // Calculate time until deadline
            let total_duration = deadline - task.submitted_at;
            let remaining = deadline - now;

            if total_duration.num_seconds() <= 0 {
                return 0.0;
            }

            // Calculate proportional boost (higher as deadline approaches)
            let ratio =
                1.0 - (remaining.num_seconds() as f64 / total_duration.num_seconds() as f64);
            ratio.clamp(0.0, 1.0) * self.deadline_boost_max
        } else {
            0.0
        }
    }

    /// Update task's calculated priority field
    pub fn update_task_priority(&self, task: &mut Task, dependency_depth: u32) {
        task.calculated_priority = self.calculate(task, dependency_depth);
        task.dependency_depth = dependency_depth;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    fn create_test_task(priority: u8) -> Task {
        let mut task = Task::new("Test".to_string(), "Description".to_string());
        task.priority = priority;
        task
    }

    #[test]
    fn test_calculate_base_priority_only() {
        let calc = PriorityCalculator::new();
        let task = create_test_task(5);

        let priority = calc.calculate(&task, 0);
        assert_eq!(priority, 5.0);
    }

    #[test]
    fn test_calculate_with_depth() {
        let calc = PriorityCalculator::new();
        let task = create_test_task(5);

        let priority = calc.calculate(&task, 2);
        // 5 + (2 * 0.5) = 6.0
        assert_eq!(priority, 6.0);
    }

    #[test]
    fn test_calculate_with_custom_weights() {
        let calc = PriorityCalculator::with_weights(1.0, 5.0);
        let task = create_test_task(5);

        let priority = calc.calculate(&task, 2);
        // 5 + (2 * 1.0) = 7.0
        assert_eq!(priority, 7.0);
    }

    #[test]
    fn test_calculate_deadline_boost_no_deadline() {
        let calc = PriorityCalculator::new();
        let task = create_test_task(5);

        let boost = calc.calculate_deadline_boost(&task);
        assert_eq!(boost, 0.0);
    }

    #[test]
    fn test_calculate_deadline_boost_past_deadline() {
        let calc = PriorityCalculator::new();
        let mut task = create_test_task(5);
        task.deadline = Some(Utc::now() - Duration::hours(1));

        let boost = calc.calculate_deadline_boost(&task);
        assert_eq!(boost, calc.deadline_boost_max);
    }

    #[test]
    fn test_calculate_deadline_boost_far_future() {
        let calc = PriorityCalculator::new();
        let mut task = create_test_task(5);
        let now = Utc::now();
        task.submitted_at = now;
        task.deadline = Some(now + Duration::days(30));

        let boost = calc.calculate_deadline_boost(&task);
        // Should be close to 0 since deadline is far away
        assert!(boost < 0.1);
    }

    #[test]
    fn test_calculate_deadline_boost_approaching() {
        let calc = PriorityCalculator::new();
        let mut task = create_test_task(5);
        let now = Utc::now();
        task.submitted_at = now - Duration::hours(10);
        task.deadline = Some(now + Duration::hours(2)); // 20% time remaining

        let boost = calc.calculate_deadline_boost(&task);
        // Should be around 80% of max boost
        assert!(boost > 2.0 && boost < calc.deadline_boost_max);
    }

    #[test]
    fn test_update_task_priority() {
        let calc = PriorityCalculator::new();
        let mut task = create_test_task(5);

        calc.update_task_priority(&mut task, 3);

        assert_eq!(task.calculated_priority, 6.5); // 5 + (3 * 0.5)
        assert_eq!(task.dependency_depth, 3);
    }

    #[test]
    fn test_full_priority_calculation() {
        let calc = PriorityCalculator::new();
        let mut task = create_test_task(7);
        let now = Utc::now();
        task.submitted_at = now - Duration::hours(5);
        task.deadline = Some(now + Duration::hours(1)); // 1/6 time remaining, 5/6 elapsed

        let priority = calc.calculate(&task, 2);
        // base=7, depth=2*0.5=1.0, deadline~=(5/6)*3.0=2.5
        // Total should be around 10.5
        assert!(priority > 10.0 && priority < 11.0);
    }
}
