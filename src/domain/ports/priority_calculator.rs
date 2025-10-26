use crate::domain::models::task::Task;
use anyhow::Result;
use async_trait::async_trait;

/// Port for task priority calculation following hexagonal architecture
///
/// Defines the interface for calculating task priorities based on various factors:
/// - Base priority (user-specified urgency)
/// - Dependency depth (tasks deeper in the graph get lower priority)
/// - Age (older tasks get slightly higher priority)
/// - Deadline proximity (tasks near deadline get higher priority)
///
/// # Examples
///
/// ```no_run
/// use abathur::domain::ports::PriorityCalculator;
/// use abathur::domain::models::task::Task;
/// use anyhow::Result;
///
/// async fn example(calc: &dyn PriorityCalculator, task: &Task) -> Result<()> {
///     let priority = calc.calculate_priority(task).await?;
///     Ok(())
/// }
/// ```
#[async_trait]
pub trait PriorityCalculator: Send + Sync {
    /// Calculate the priority for a task
    ///
    /// Takes into account:
    /// - Base priority (1-10 scale from user)
    /// - Dependency depth (deeper tasks have lower priority)
    /// - Age factor (older tasks get slight boost)
    /// - Deadline proximity (tasks near deadline get boost)
    ///
    /// # Arguments
    ///
    /// * `task` - The task to calculate priority for
    ///
    /// # Returns
    ///
    /// * `Ok(f64)` - Calculated priority value (higher is more urgent)
    /// * `Err` - If calculation fails
    async fn calculate_priority(&self, task: &Task) -> Result<f64>;

    /// Recalculate priorities for multiple tasks
    ///
    /// Useful when dependency relationships change and multiple tasks
    /// need their priorities updated.
    ///
    /// # Arguments
    ///
    /// * `tasks` - Tasks to recalculate priorities for
    ///
    /// # Returns
    ///
    /// * `Ok(Vec<(Uuid, f64)>)` - List of (`task_id`, `new_priority`) tuples
    /// * `Err` - If calculation fails
    async fn recalculate_priorities(&self, tasks: &[Task]) -> Result<Vec<(uuid::Uuid, f64)>>;
}
