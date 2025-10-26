use std::collections::BTreeMap;
use uuid::Uuid;

use super::Task;

/// Priority-based task queue using BTreeMap for efficient priority ordering.
/// Tasks are stored in a BTreeMap keyed by priority (0-10), with higher priority first.
/// Tasks of the same priority are stored in FIFO order within a Vec.
#[derive(Debug, Clone, Default)]
pub struct Queue {
    /// BTreeMap storing tasks grouped by priority level (reversed for highest-first)
    /// Key: Priority (reversed to get descending order)
    /// Value: Vector of tasks at that priority level (FIFO order)
    tasks: BTreeMap<ReversePriority, Vec<Task>>,
    /// Total number of tasks in the queue (cached for O(1) access)
    total_count: usize,
}

/// Wrapper type for priority that implements reverse ordering
/// This allows BTreeMap to store tasks with highest priority first
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct ReversePriority(u8);

impl ReversePriority {
    fn new(priority: u8) -> Self {
        // Reverse the priority: 10 -> 0, 9 -> 1, ..., 0 -> 10
        // This makes higher priorities sort first in BTreeMap
        ReversePriority(10 - priority)
    }

    #[allow(dead_code)]
    fn original(&self) -> u8 {
        10 - self.0
    }
}

impl Queue {
    /// Create a new empty queue
    pub fn new() -> Self {
        Self {
            tasks: BTreeMap::new(),
            total_count: 0,
        }
    }

    /// Add a task to the queue based on its priority
    ///
    /// Tasks with higher priority values will be dequeued first.
    /// Tasks with the same priority are dequeued in FIFO order.
    ///
    /// # Arguments
    /// * `task` - The task to enqueue
    ///
    /// # Returns
    /// * `Ok(())` if the task was successfully enqueued
    /// * `Err(QueueError)` if the task's priority is invalid
    pub fn enqueue(&mut self, task: Task) -> Result<(), QueueError> {
        // Validate priority range
        if task.priority > 10 {
            return Err(QueueError::InvalidPriority {
                priority: task.priority,
                max: 10,
            });
        }

        let priority = ReversePriority::new(task.priority);

        // Add task to the priority bucket (creates new Vec if needed)
        self.tasks.entry(priority).or_default().push(task);
        self.total_count += 1;

        Ok(())
    }

    /// Remove and return the highest priority task from the queue
    ///
    /// Returns the task with the highest priority value.
    /// If multiple tasks have the same priority, returns the oldest (FIFO).
    ///
    /// # Returns
    /// * `Some(Task)` if the queue is not empty
    /// * `None` if the queue is empty
    pub fn dequeue(&mut self) -> Option<Task> {
        // Get the first (highest priority) entry
        // BTreeMap iteration is in sorted order (lowest ReversePriority = highest actual priority)
        let priority = *self.tasks.keys().next()?;

        // Get the task vector for this priority level
        let tasks = self.tasks.get_mut(&priority)?;

        // Remove the first task (FIFO within priority level)
        let task = tasks.remove(0);

        // If this was the last task at this priority, remove the empty Vec
        if tasks.is_empty() {
            self.tasks.remove(&priority);
        }

        self.total_count -= 1;
        Some(task)
    }

    /// Peek at the highest priority task without removing it
    ///
    /// # Returns
    /// * `Some(&Task)` - Reference to the highest priority task
    /// * `None` - If the queue is empty
    pub fn peek(&self) -> Option<&Task> {
        let priority = *self.tasks.keys().next()?;
        let tasks = self.tasks.get(&priority)?;
        tasks.first()
    }

    /// Check if the queue is empty
    ///
    /// # Returns
    /// * `true` if the queue contains no tasks
    /// * `false` otherwise
    pub fn is_empty(&self) -> bool {
        self.total_count == 0
    }

    /// Get the total number of tasks in the queue
    ///
    /// # Returns
    /// The number of tasks currently in the queue
    pub fn len(&self) -> usize {
        self.total_count
    }

    /// Remove a specific task from the queue by its ID
    ///
    /// Searches through all priority levels to find and remove the task.
    ///
    /// # Arguments
    /// * `task_id` - The UUID of the task to remove
    ///
    /// # Returns
    /// * `Some(Task)` - The removed task if found
    /// * `None` - If no task with the given ID exists
    pub fn remove(&mut self, task_id: Uuid) -> Option<Task> {
        // Search through all priority levels
        for tasks in self.tasks.values_mut() {
            // Find the task in this priority level
            if let Some(pos) = tasks.iter().position(|t| t.id == task_id) {
                let task = tasks.remove(pos);
                self.total_count -= 1;
                return Some(task);
            }
        }

        None
    }

    /// Get a reference to a specific task by its ID without removing it
    ///
    /// # Arguments
    /// * `task_id` - The UUID of the task to find
    ///
    /// # Returns
    /// * `Some(&Task)` - Reference to the task if found
    /// * `None` - If no task with the given ID exists
    pub fn get(&self, task_id: Uuid) -> Option<&Task> {
        for tasks in self.tasks.values() {
            if let Some(task) = tasks.iter().find(|t| t.id == task_id) {
                return Some(task);
            }
        }
        None
    }

    /// Get an iterator over all tasks in the queue, ordered by priority (highest first)
    ///
    /// # Returns
    /// An iterator yielding references to all tasks in priority order
    pub fn iter(&self) -> impl Iterator<Item = &Task> {
        self.tasks.values().flat_map(|tasks| tasks.iter())
    }
}

/// Errors that can occur during queue operations
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueueError {
    /// Task priority exceeds maximum allowed value
    InvalidPriority { priority: u8, max: u8 },
}

impl std::fmt::Display for QueueError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            QueueError::InvalidPriority { priority, max } => {
                write!(
                    f,
                    "Invalid priority: {} (must be between 0 and {})",
                    priority, max
                )
            }
        }
    }
}

impl std::error::Error for QueueError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_task(summary: &str, priority: u8) -> Task {
        let mut task = Task::new(summary.to_string(), "test description".to_string());
        task.priority = priority;
        task
    }

    #[test]
    fn test_queue_new() {
        let queue = Queue::new();
        assert!(queue.is_empty());
        assert_eq!(queue.len(), 0);
    }

    #[test]
    fn test_queue_enqueue_single() {
        let mut queue = Queue::new();
        let task = create_test_task("Task 1", 5);
        let task_id = task.id;

        assert!(queue.enqueue(task).is_ok());
        assert_eq!(queue.len(), 1);
        assert!(!queue.is_empty());
        assert!(queue.get(task_id).is_some());
    }

    #[test]
    fn test_queue_enqueue_invalid_priority() {
        let mut queue = Queue::new();
        let task = create_test_task("Invalid", 11);

        let result = queue.enqueue(task);
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err(),
            QueueError::InvalidPriority {
                priority: 11,
                max: 10
            }
        );
        assert_eq!(queue.len(), 0);
    }

    #[test]
    fn test_queue_dequeue_single() {
        let mut queue = Queue::new();
        let task = create_test_task("Task 1", 5);
        let task_id = task.id;

        queue.enqueue(task).unwrap();

        let dequeued = queue.dequeue();
        assert!(dequeued.is_some());
        assert_eq!(dequeued.unwrap().id, task_id);
        assert_eq!(queue.len(), 0);
        assert!(queue.is_empty());
    }

    #[test]
    fn test_queue_dequeue_empty() {
        let mut queue = Queue::new();
        assert!(queue.dequeue().is_none());
    }

    #[test]
    fn test_queue_priority_ordering() {
        let mut queue = Queue::new();

        // Enqueue tasks in random priority order
        let task_low = create_test_task("Low priority", 2);
        let task_high = create_test_task("High priority", 8);
        let task_med = create_test_task("Medium priority", 5);

        let high_id = task_high.id;
        let med_id = task_med.id;
        let low_id = task_low.id;

        queue.enqueue(task_low).unwrap();
        queue.enqueue(task_high).unwrap();
        queue.enqueue(task_med).unwrap();

        assert_eq!(queue.len(), 3);

        // Should dequeue in priority order: high, medium, low
        let first = queue.dequeue().unwrap();
        assert_eq!(first.id, high_id);
        assert_eq!(first.priority, 8);

        let second = queue.dequeue().unwrap();
        assert_eq!(second.id, med_id);
        assert_eq!(second.priority, 5);

        let third = queue.dequeue().unwrap();
        assert_eq!(third.id, low_id);
        assert_eq!(third.priority, 2);

        assert!(queue.is_empty());
    }

    #[test]
    fn test_queue_fifo_within_priority() {
        let mut queue = Queue::new();

        // Enqueue multiple tasks with same priority
        let task1 = create_test_task("First", 5);
        let task2 = create_test_task("Second", 5);
        let task3 = create_test_task("Third", 5);

        let id1 = task1.id;
        let id2 = task2.id;
        let id3 = task3.id;

        queue.enqueue(task1).unwrap();
        queue.enqueue(task2).unwrap();
        queue.enqueue(task3).unwrap();

        // Should dequeue in FIFO order
        assert_eq!(queue.dequeue().unwrap().id, id1);
        assert_eq!(queue.dequeue().unwrap().id, id2);
        assert_eq!(queue.dequeue().unwrap().id, id3);
    }

    #[test]
    fn test_queue_peek() {
        let mut queue = Queue::new();

        let task = create_test_task("Task", 5);
        let task_id = task.id;
        queue.enqueue(task).unwrap();

        // Peek should return task without removing it
        let peeked = queue.peek();
        assert!(peeked.is_some());
        assert_eq!(peeked.unwrap().id, task_id);
        assert_eq!(queue.len(), 1);

        // Peek again should return same task
        let peeked2 = queue.peek();
        assert_eq!(peeked2.unwrap().id, task_id);
        assert_eq!(queue.len(), 1);
    }

    #[test]
    fn test_queue_peek_empty() {
        let queue = Queue::new();
        assert!(queue.peek().is_none());
    }

    #[test]
    fn test_queue_peek_priority() {
        let mut queue = Queue::new();

        let task_low = create_test_task("Low", 2);
        let task_high = create_test_task("High", 8);
        let high_id = task_high.id;

        queue.enqueue(task_low).unwrap();
        queue.enqueue(task_high).unwrap();

        // Should peek at highest priority task
        let peeked = queue.peek();
        assert_eq!(peeked.unwrap().id, high_id);
    }

    #[test]
    fn test_queue_remove_existing() {
        let mut queue = Queue::new();

        let task1 = create_test_task("Task 1", 5);
        let task2 = create_test_task("Task 2", 3);
        let task3 = create_test_task("Task 3", 8);

        let id1 = task1.id;
        let id2 = task2.id;
        let id3 = task3.id;

        queue.enqueue(task1).unwrap();
        queue.enqueue(task2).unwrap();
        queue.enqueue(task3).unwrap();

        assert_eq!(queue.len(), 3);

        // Remove middle priority task
        let removed = queue.remove(id2);
        assert!(removed.is_some());
        assert_eq!(removed.unwrap().id, id2);
        assert_eq!(queue.len(), 2);

        // Remaining tasks should still be in priority order
        assert_eq!(queue.dequeue().unwrap().id, id3); // priority 8
        assert_eq!(queue.dequeue().unwrap().id, id1); // priority 5
    }

    #[test]
    fn test_queue_remove_nonexistent() {
        let mut queue = Queue::new();
        let task = create_test_task("Task", 5);
        queue.enqueue(task).unwrap();

        let fake_id = Uuid::new_v4();
        let removed = queue.remove(fake_id);
        assert!(removed.is_none());
        assert_eq!(queue.len(), 1);
    }

    #[test]
    fn test_queue_remove_from_empty() {
        let mut queue = Queue::new();
        let fake_id = Uuid::new_v4();
        assert!(queue.remove(fake_id).is_none());
    }

    #[test]
    fn test_queue_get_existing() {
        let mut queue = Queue::new();
        let task = create_test_task("Task", 5);
        let task_id = task.id;
        let task_summary = task.summary.clone();

        queue.enqueue(task).unwrap();

        let found = queue.get(task_id);
        assert!(found.is_some());
        assert_eq!(found.unwrap().id, task_id);
        assert_eq!(found.unwrap().summary, task_summary);
        assert_eq!(queue.len(), 1); // Should not remove
    }

    #[test]
    fn test_queue_get_nonexistent() {
        let mut queue = Queue::new();
        let task = create_test_task("Task", 5);
        queue.enqueue(task).unwrap();

        let fake_id = Uuid::new_v4();
        assert!(queue.get(fake_id).is_none());
    }

    #[test]
    fn test_queue_len_and_is_empty() {
        let mut queue = Queue::new();

        assert_eq!(queue.len(), 0);
        assert!(queue.is_empty());

        queue.enqueue(create_test_task("Task 1", 5)).unwrap();
        assert_eq!(queue.len(), 1);
        assert!(!queue.is_empty());

        queue.enqueue(create_test_task("Task 2", 3)).unwrap();
        assert_eq!(queue.len(), 2);
        assert!(!queue.is_empty());

        queue.dequeue();
        assert_eq!(queue.len(), 1);
        assert!(!queue.is_empty());

        queue.dequeue();
        assert_eq!(queue.len(), 0);
        assert!(queue.is_empty());
    }

    #[test]
    fn test_queue_iter() {
        let mut queue = Queue::new();

        let task1 = create_test_task("Low", 2);
        let task2 = create_test_task("High", 8);
        let task3 = create_test_task("Medium", 5);

        queue.enqueue(task1).unwrap();
        queue.enqueue(task2).unwrap();
        queue.enqueue(task3).unwrap();

        let priorities: Vec<u8> = queue.iter().map(|t| t.priority).collect();

        // Should iterate in priority order: high to low
        assert_eq!(priorities, vec![8, 5, 2]);
    }

    #[test]
    fn test_queue_all_priorities() {
        let mut queue = Queue::new();

        // Test all valid priorities (0-10)
        for priority in 0..=10 {
            let task = create_test_task(&format!("Priority {}", priority), priority);
            assert!(queue.enqueue(task).is_ok());
        }

        assert_eq!(queue.len(), 11);

        // Should dequeue in descending priority order
        for expected_priority in (0..=10).rev() {
            let task = queue.dequeue().unwrap();
            assert_eq!(task.priority, expected_priority);
        }

        assert!(queue.is_empty());
    }

    #[test]
    fn test_queue_edge_case_priority_0() {
        let mut queue = Queue::new();

        let task_zero = create_test_task("Zero priority", 0);
        let task_high = create_test_task("High priority", 10);

        let high_id = task_high.id;
        let zero_id = task_zero.id;

        queue.enqueue(task_zero).unwrap();
        queue.enqueue(task_high).unwrap();

        // Priority 10 should come first
        assert_eq!(queue.dequeue().unwrap().id, high_id);
        // Priority 0 should come last
        assert_eq!(queue.dequeue().unwrap().id, zero_id);
    }

    #[test]
    fn test_queue_error_display() {
        let error = QueueError::InvalidPriority {
            priority: 15,
            max: 10,
        };

        let error_msg = error.to_string();
        assert!(error_msg.contains("Invalid priority"));
        assert!(error_msg.contains("15"));
        assert!(error_msg.contains("10"));
    }

    #[test]
    fn test_reverse_priority_ordering() {
        // Verify ReversePriority implements correct ordering
        let p10 = ReversePriority::new(10);
        let p5 = ReversePriority::new(5);
        let p0 = ReversePriority::new(0);

        // Higher priority should sort first (lower ReversePriority value)
        assert!(p10 < p5);
        assert!(p5 < p0);
        assert!(p10 < p0);

        // Verify original values
        assert_eq!(p10.original(), 10);
        assert_eq!(p5.original(), 5);
        assert_eq!(p0.original(), 0);
    }
}
