use std::cmp::Ordering;
use std::collections::VecDeque;

/// Priority queue item wrapper
///
/// Wraps any item type with a priority value for priority-based ordering.
/// Higher priority values are dequeued first.
#[derive(Debug, Clone)]
pub struct QueueItem<T> {
    /// Priority value (higher values = higher priority)
    pub priority: u8,
    /// The wrapped item
    pub item: T,
}

impl<T> PartialEq for QueueItem<T> {
    fn eq(&self, other: &Self) -> bool {
        self.priority == other.priority
    }
}

impl<T> Eq for QueueItem<T> {}

impl<T> PartialOrd for QueueItem<T> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl<T> Ord for QueueItem<T> {
    fn cmp(&self, other: &Self) -> Ordering {
        // Higher priority comes first (reverse ordering)
        other.priority.cmp(&self.priority)
    }
}

/// Generic priority-based task queue
///
/// Implements a priority queue where items with higher priority values
/// are dequeued before items with lower priority values. Items with
/// equal priority are dequeued in FIFO order.
///
/// # Type Parameters
///
/// * `T` - The type of items stored in the queue
///
/// # Examples
///
/// ```
/// use abathur::domain::models::TaskQueue;
///
/// let mut queue = TaskQueue::new();
/// queue.enqueue("low priority", 1);
/// queue.enqueue("high priority", 10);
/// queue.enqueue("medium priority", 5);
///
/// assert_eq!(queue.dequeue(), Some("high priority"));
/// assert_eq!(queue.dequeue(), Some("medium priority"));
/// assert_eq!(queue.dequeue(), Some("low priority"));
/// ```
#[derive(Debug, Clone)]
pub struct TaskQueue<T> {
    items: VecDeque<QueueItem<T>>,
}

impl<T> TaskQueue<T> {
    /// Creates a new empty task queue
    ///
    /// # Examples
    ///
    /// ```
    /// use abathur::domain::models::TaskQueue;
    ///
    /// let queue: TaskQueue<String> = TaskQueue::new();
    /// assert!(queue.is_empty());
    /// ```
    #[allow(clippy::missing_const_for_fn)]
    pub fn new() -> Self {
        Self {
            items: VecDeque::new(),
        }
    }

    /// Creates a new task queue with specified capacity
    ///
    /// Pre-allocates space for at least the specified number of items,
    /// which can improve performance when the approximate queue size is known.
    ///
    /// # Arguments
    ///
    /// * `capacity` - The initial capacity to allocate
    ///
    /// # Examples
    ///
    /// ```
    /// use abathur::domain::models::TaskQueue;
    ///
    /// let queue: TaskQueue<String> = TaskQueue::with_capacity(100);
    /// assert!(queue.is_empty());
    /// ```
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            items: VecDeque::with_capacity(capacity),
        }
    }

    /// Adds an item to the queue with the specified priority
    ///
    /// Items are inserted in priority order. Higher priority values
    /// will be dequeued before lower priority values. Items with equal
    /// priority maintain FIFO ordering.
    ///
    /// # Arguments
    ///
    /// * `item` - The item to enqueue
    /// * `priority` - The priority value (0-255, higher = more important)
    ///
    /// # Examples
    ///
    /// ```
    /// use abathur::domain::models::TaskQueue;
    ///
    /// let mut queue = TaskQueue::new();
    /// queue.enqueue("task 1", 5);
    /// queue.enqueue("task 2", 10);
    /// assert_eq!(queue.len(), 2);
    /// ```
    pub fn enqueue(&mut self, item: T, priority: u8) {
        let queue_item = QueueItem { priority, item };

        // Find the correct position to insert while maintaining priority order
        // Items with higher priority (lower Ord value) come first
        let position = self
            .items
            .iter()
            .position(|existing| queue_item < *existing)
            .unwrap_or(self.items.len());

        self.items.insert(position, queue_item);
    }

    /// Removes and returns the highest priority item from the queue
    ///
    /// Returns `None` if the queue is empty.
    ///
    /// # Examples
    ///
    /// ```
    /// use abathur::domain::models::TaskQueue;
    ///
    /// let mut queue = TaskQueue::new();
    /// queue.enqueue("low", 1);
    /// queue.enqueue("high", 10);
    ///
    /// assert_eq!(queue.dequeue(), Some("high"));
    /// assert_eq!(queue.dequeue(), Some("low"));
    /// assert_eq!(queue.dequeue(), None);
    /// ```
    pub fn dequeue(&mut self) -> Option<T> {
        self.items.pop_front().map(|queue_item| queue_item.item)
    }

    /// Returns a reference to the highest priority item without removing it
    ///
    /// Returns `None` if the queue is empty.
    ///
    /// # Examples
    ///
    /// ```
    /// use abathur::domain::models::TaskQueue;
    ///
    /// let mut queue = TaskQueue::new();
    /// queue.enqueue("item", 5);
    ///
    /// assert_eq!(queue.peek(), Some(&"item"));
    /// assert_eq!(queue.len(), 1); // Item still in queue
    /// ```
    pub fn peek(&self) -> Option<&T> {
        self.items.front().map(|queue_item| &queue_item.item)
    }

    /// Returns the number of items in the queue
    ///
    /// # Examples
    ///
    /// ```
    /// use abathur::domain::models::TaskQueue;
    ///
    /// let mut queue = TaskQueue::new();
    /// assert_eq!(queue.len(), 0);
    ///
    /// queue.enqueue("item", 5);
    /// assert_eq!(queue.len(), 1);
    /// ```
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Returns `true` if the queue contains no items
    ///
    /// # Examples
    ///
    /// ```
    /// use abathur::domain::models::TaskQueue;
    ///
    /// let mut queue = TaskQueue::new();
    /// assert!(queue.is_empty());
    ///
    /// queue.enqueue("item", 5);
    /// assert!(!queue.is_empty());
    /// ```
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Removes all items from the queue
    ///
    /// # Examples
    ///
    /// ```
    /// use abathur::domain::models::TaskQueue;
    ///
    /// let mut queue = TaskQueue::new();
    /// queue.enqueue("item1", 5);
    /// queue.enqueue("item2", 10);
    ///
    /// queue.clear();
    /// assert!(queue.is_empty());
    /// ```
    pub fn clear(&mut self) {
        self.items.clear();
    }

    /// Returns an iterator over the queue items in priority order
    ///
    /// The iterator yields references to items without removing them from the queue.
    ///
    /// # Examples
    ///
    /// ```
    /// use abathur::domain::models::TaskQueue;
    ///
    /// let mut queue = TaskQueue::new();
    /// queue.enqueue("low", 1);
    /// queue.enqueue("high", 10);
    ///
    /// let items: Vec<&str> = queue.iter().map(|s| *s).collect();
    /// assert_eq!(items, vec!["high", "low"]);
    /// ```
    pub fn iter(&self) -> impl Iterator<Item = &T> {
        self.items.iter().map(|queue_item| &queue_item.item)
    }

    /// Removes and returns items matching a predicate
    ///
    /// This method removes all items for which the predicate returns `true`,
    /// returning them in a vector while maintaining priority order.
    ///
    /// # Arguments
    ///
    /// * `predicate` - A function that returns `true` for items to remove
    ///
    /// # Examples
    ///
    /// ```
    /// use abathur::domain::models::TaskQueue;
    ///
    /// let mut queue = TaskQueue::new();
    /// queue.enqueue(1, 5);
    /// queue.enqueue(2, 10);
    /// queue.enqueue(3, 5);
    ///
    /// let removed = queue.remove_matching(|&item| item % 2 == 0);
    /// assert_eq!(removed, vec![2]);
    /// assert_eq!(queue.len(), 2);
    /// ```
    pub fn remove_matching<F>(&mut self, predicate: F) -> Vec<T>
    where
        F: Fn(&T) -> bool,
    {
        let mut removed = Vec::new();
        let mut i = 0;

        while i < self.items.len() {
            if predicate(&self.items[i].item) {
                if let Some(queue_item) = self.items.remove(i) {
                    removed.push(queue_item.item);
                }
            } else {
                i += 1;
            }
        }

        removed
    }

    /// Returns the number of items matching a predicate
    ///
    /// # Arguments
    ///
    /// * `predicate` - A function that returns `true` for items to count
    ///
    /// # Examples
    ///
    /// ```
    /// use abathur::domain::models::TaskQueue;
    ///
    /// let mut queue = TaskQueue::new();
    /// queue.enqueue(1, 5);
    /// queue.enqueue(2, 10);
    /// queue.enqueue(3, 5);
    ///
    /// let count = queue.count_matching(|&item| item > 1);
    /// assert_eq!(count, 2);
    /// ```
    pub fn count_matching<F>(&self, predicate: F) -> usize
    where
        F: Fn(&T) -> bool,
    {
        self.items
            .iter()
            .filter(|queue_item| predicate(&queue_item.item))
            .count()
    }
}

impl<T> Default for TaskQueue<T> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_queue_is_empty() {
        let queue: TaskQueue<String> = TaskQueue::new();
        assert!(queue.is_empty());
        assert_eq!(queue.len(), 0);
    }

    #[test]
    fn test_priority_ordering() {
        let mut queue = TaskQueue::new();
        queue.enqueue("low", 1);
        queue.enqueue("high", 10);
        queue.enqueue("medium", 5);

        assert_eq!(queue.dequeue(), Some("high"));
        assert_eq!(queue.dequeue(), Some("medium"));
        assert_eq!(queue.dequeue(), Some("low"));
        assert_eq!(queue.dequeue(), None);
    }

    #[test]
    fn test_fifo_ordering_for_equal_priority() {
        let mut queue = TaskQueue::new();
        queue.enqueue("first", 5);
        queue.enqueue("second", 5);
        queue.enqueue("third", 5);

        assert_eq!(queue.dequeue(), Some("first"));
        assert_eq!(queue.dequeue(), Some("second"));
        assert_eq!(queue.dequeue(), Some("third"));
    }

    #[test]
    fn test_peek_does_not_remove() {
        let mut queue = TaskQueue::new();
        queue.enqueue("item", 5);

        assert_eq!(queue.peek(), Some(&"item"));
        assert_eq!(queue.len(), 1);
        assert_eq!(queue.peek(), Some(&"item"));
    }

    #[test]
    fn test_clear() {
        let mut queue = TaskQueue::new();
        queue.enqueue("item1", 5);
        queue.enqueue("item2", 10);

        queue.clear();
        assert!(queue.is_empty());
        assert_eq!(queue.len(), 0);
    }

    #[test]
    fn test_with_capacity() {
        let queue: TaskQueue<i32> = TaskQueue::with_capacity(100);
        assert!(queue.is_empty());
    }

    #[test]
    fn test_iter() {
        let mut queue = TaskQueue::new();
        queue.enqueue(1, 5);
        queue.enqueue(2, 10);
        queue.enqueue(3, 1);

        let items: Vec<i32> = queue.iter().copied().collect();
        assert_eq!(items, vec![2, 1, 3]);
    }

    #[test]
    fn test_remove_matching() {
        let mut queue = TaskQueue::new();
        queue.enqueue(1, 5);
        queue.enqueue(2, 10);
        queue.enqueue(3, 5);
        queue.enqueue(4, 1);

        let removed = queue.remove_matching(|&item| item % 2 == 0);
        assert_eq!(removed, vec![2, 4]);
        assert_eq!(queue.len(), 2);

        let remaining: Vec<i32> = queue.iter().copied().collect();
        assert_eq!(remaining, vec![1, 3]);
    }

    #[test]
    fn test_count_matching() {
        let mut queue = TaskQueue::new();
        queue.enqueue(1, 5);
        queue.enqueue(2, 10);
        queue.enqueue(3, 5);
        queue.enqueue(4, 1);

        assert_eq!(queue.count_matching(|&item| item > 2), 2);
        assert_eq!(queue.count_matching(|&item| item % 2 == 0), 2);
        assert_eq!(queue.count_matching(|&_item| true), 4);
        assert_eq!(queue.count_matching(|&item| item > 10), 0);
    }

    #[test]
    fn test_complex_priority_scenario() {
        let mut queue = TaskQueue::new();

        // Add tasks with different priorities
        queue.enqueue("P10-A", 10);
        queue.enqueue("P5-A", 5);
        queue.enqueue("P10-B", 10);
        queue.enqueue("P1-A", 1);
        queue.enqueue("P5-B", 5);

        // Verify priority ordering with FIFO for equal priorities
        assert_eq!(queue.dequeue(), Some("P10-A"));
        assert_eq!(queue.dequeue(), Some("P10-B"));
        assert_eq!(queue.dequeue(), Some("P5-A"));
        assert_eq!(queue.dequeue(), Some("P5-B"));
        assert_eq!(queue.dequeue(), Some("P1-A"));
    }

    #[test]
    fn test_queue_item_ordering() {
        let high = QueueItem {
            priority: 10,
            item: "high",
        };
        let low = QueueItem {
            priority: 1,
            item: "low",
        };

        // Higher priority should be "less than" (comes first in ordering)
        assert!(high < low);
        assert!(low >= high);
    }

    #[test]
    fn test_empty_queue_operations() {
        let mut queue: TaskQueue<i32> = TaskQueue::new();

        assert_eq!(queue.dequeue(), None);
        assert_eq!(queue.peek(), None);
        assert_eq!(queue.count_matching(|_| true), 0);
        assert_eq!(queue.remove_matching(|_| true), Vec::<i32>::new());
    }

    #[test]
    fn test_single_item() {
        let mut queue = TaskQueue::new();
        queue.enqueue(42, 5);

        assert_eq!(queue.len(), 1);
        assert_eq!(queue.peek(), Some(&42));
        assert_eq!(queue.dequeue(), Some(42));
        assert!(queue.is_empty());
    }

    #[test]
    fn test_maximum_priority() {
        let mut queue = TaskQueue::new();
        queue.enqueue("min", u8::MIN);
        queue.enqueue("max", u8::MAX);
        queue.enqueue("mid", 128);

        assert_eq!(queue.dequeue(), Some("max"));
        assert_eq!(queue.dequeue(), Some("mid"));
        assert_eq!(queue.dequeue(), Some("min"));
    }
}
