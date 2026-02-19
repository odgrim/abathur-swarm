//! DAG (Directed Acyclic Graph) domain models.
//!
//! Represents task dependency graphs and provides utilities for
//! topological sorting, cycle detection, and wave-based execution.

use std::collections::{HashMap, HashSet, VecDeque};
use uuid::Uuid;

use super::task::{Task, TaskStatus};

/// A node in the DAG representing a task.
#[derive(Debug, Clone)]
pub struct DagNode {
    pub task_id: Uuid,
    pub task_title: String,
    pub status: TaskStatus,
    pub dependencies: Vec<Uuid>,
    pub dependents: Vec<Uuid>,
}

impl DagNode {
    pub fn from_task(task: &Task) -> Self {
        Self {
            task_id: task.id,
            task_title: task.title.clone(),
            status: task.status,
            dependencies: task.depends_on.clone(),
            dependents: vec![],
        }
    }

    pub fn is_ready(&self, completed: &HashSet<Uuid>) -> bool {
        self.dependencies.iter().all(|dep| completed.contains(dep))
    }
}

/// A DAG representing task dependencies.
#[derive(Debug, Clone)]
pub struct TaskDag {
    /// All nodes in the graph.
    pub nodes: HashMap<Uuid, DagNode>,
    /// Root nodes (no dependencies).
    pub roots: Vec<Uuid>,
    /// Leaf nodes (no dependents).
    pub leaves: Vec<Uuid>,
}

impl TaskDag {
    /// Build a DAG from a list of tasks.
    pub fn from_tasks(tasks: Vec<Task>) -> Self {
        let mut nodes: HashMap<Uuid, DagNode> = HashMap::new();
        let mut all_deps: HashSet<Uuid> = HashSet::new();
        let mut has_dependents: HashSet<Uuid> = HashSet::new();

        // First pass: create nodes and collect dependencies
        for task in &tasks {
            let node = DagNode::from_task(task);
            all_deps.extend(&node.dependencies);
            nodes.insert(task.id, node);
        }

        // Second pass: populate dependents
        for task in &tasks {
            for dep_id in &task.depends_on {
                if let Some(dep_node) = nodes.get_mut(dep_id) {
                    dep_node.dependents.push(task.id);
                    has_dependents.insert(*dep_id);
                }
            }
        }

        // Find roots (nodes with no dependencies in this DAG)
        let roots: Vec<Uuid> = nodes.iter()
            .filter(|(_, node)| {
                node.dependencies.is_empty() ||
                node.dependencies.iter().all(|d| !nodes.contains_key(d))
            })
            .map(|(id, _)| *id)
            .collect();

        // Find leaves (nodes with no dependents)
        let leaves: Vec<Uuid> = nodes.iter()
            .filter(|(id, _)| !has_dependents.contains(id))
            .map(|(id, _)| *id)
            .collect();

        Self { nodes, roots, leaves }
    }

    /// Check if the DAG contains a cycle.
    pub fn has_cycle(&self) -> bool {
        let mut visited = HashSet::new();
        let mut rec_stack = HashSet::new();

        for &node_id in self.nodes.keys() {
            if self.detect_cycle_dfs(node_id, &mut visited, &mut rec_stack) {
                return true;
            }
        }

        false
    }

    fn detect_cycle_dfs(&self, node_id: Uuid, visited: &mut HashSet<Uuid>, rec_stack: &mut HashSet<Uuid>) -> bool {
        if rec_stack.contains(&node_id) {
            return true;
        }
        if visited.contains(&node_id) {
            return false;
        }

        visited.insert(node_id);
        rec_stack.insert(node_id);

        if let Some(node) = self.nodes.get(&node_id) {
            for &dep in &node.dependents {
                if self.detect_cycle_dfs(dep, visited, rec_stack) {
                    return true;
                }
            }
        }

        rec_stack.remove(&node_id);
        false
    }

    /// Perform topological sort and return tasks in execution order.
    pub fn topological_sort(&self) -> Result<Vec<Uuid>, DagError> {
        if self.has_cycle() {
            return Err(DagError::CycleDetected);
        }

        let mut result = Vec::new();
        let mut in_degree: HashMap<Uuid, usize> = HashMap::new();
        let mut queue = VecDeque::new();

        // Calculate in-degrees (number of dependencies)
        for (id, node) in &self.nodes {
            let deps_in_dag = node.dependencies.iter()
                .filter(|d| self.nodes.contains_key(d))
                .count();
            in_degree.insert(*id, deps_in_dag);
        }

        // Start with nodes that have no dependencies
        for (&id, &degree) in &in_degree {
            if degree == 0 {
                queue.push_back(id);
            }
        }

        while let Some(node_id) = queue.pop_front() {
            result.push(node_id);

            if let Some(node) = self.nodes.get(&node_id) {
                for &dependent in &node.dependents {
                    if let Some(degree) = in_degree.get_mut(&dependent) {
                        *degree -= 1;
                        if *degree == 0 {
                            queue.push_back(dependent);
                        }
                    }
                }
            }
        }

        if result.len() != self.nodes.len() {
            return Err(DagError::CycleDetected);
        }

        Ok(result)
    }

    /// Group tasks into waves for parallel execution.
    ///
    /// Each wave contains tasks that can run concurrently (all dependencies satisfied).
    pub fn execution_waves(&self) -> Result<Vec<Vec<Uuid>>, DagError> {
        if self.has_cycle() {
            return Err(DagError::CycleDetected);
        }

        let mut waves = Vec::new();
        let mut remaining: HashSet<Uuid> = self.nodes.keys().copied().collect();
        let mut completed: HashSet<Uuid> = HashSet::new();

        while !remaining.is_empty() {
            // Find all nodes whose dependencies are complete
            let wave: Vec<Uuid> = remaining.iter()
                .filter(|id| {
                    self.nodes.get(id)
                        .map(|node| node.is_ready(&completed))
                        .unwrap_or(false)
                })
                .copied()
                .collect();

            if wave.is_empty() {
                // This shouldn't happen if we've validated no cycles
                return Err(DagError::CycleDetected);
            }

            // Move wave nodes to completed
            for id in &wave {
                remaining.remove(id);
                completed.insert(*id);
            }

            waves.push(wave);
        }

        Ok(waves)
    }

    /// Get the critical path (longest dependency chain).
    pub fn critical_path(&self) -> Result<Vec<Uuid>, DagError> {
        let sorted = self.topological_sort()?;
        let mut distances: HashMap<Uuid, usize> = HashMap::new();
        let mut predecessors: HashMap<Uuid, Option<Uuid>> = HashMap::new();

        // Initialize distances
        for &id in &sorted {
            distances.insert(id, 0);
            predecessors.insert(id, None);
        }

        // Process in topological order
        for &node_id in &sorted {
            if let Some(node) = self.nodes.get(&node_id) {
                let current_dist = *distances.get(&node_id).unwrap_or(&0);
                for &dependent in &node.dependents {
                    let new_dist = current_dist + 1;
                    if new_dist > *distances.get(&dependent).unwrap_or(&0) {
                        distances.insert(dependent, new_dist);
                        predecessors.insert(dependent, Some(node_id));
                    }
                }
            }
        }

        // Find the node with maximum distance
        let end_node = distances.iter()
            .max_by_key(|&(_, &dist)| dist)
            .map(|(&id, _)| id);

        if let Some(mut current) = end_node {
            let mut path = vec![current];
            while let Some(&Some(pred)) = predecessors.get(&current) {
                path.push(pred);
                current = pred;
            }
            path.reverse();
            Ok(path)
        } else {
            Ok(vec![])
        }
    }

    /// Get all tasks that depend on a given task (transitively).
    pub fn get_all_dependents(&self, task_id: Uuid) -> HashSet<Uuid> {
        let mut result = HashSet::new();
        let mut queue = VecDeque::new();

        if let Some(node) = self.nodes.get(&task_id) {
            queue.extend(&node.dependents);
        }

        while let Some(id) = queue.pop_front() {
            if result.insert(id) {
                if let Some(node) = self.nodes.get(&id) {
                    queue.extend(&node.dependents);
                }
            }
        }

        result
    }

    /// Get all tasks that a given task depends on (transitively).
    pub fn get_all_dependencies(&self, task_id: Uuid) -> HashSet<Uuid> {
        let mut result = HashSet::new();
        let mut queue = VecDeque::new();

        if let Some(node) = self.nodes.get(&task_id) {
            queue.extend(&node.dependencies);
        }

        while let Some(id) = queue.pop_front() {
            if result.insert(id) {
                if let Some(node) = self.nodes.get(&id) {
                    queue.extend(&node.dependencies);
                }
            }
        }

        result
    }

    /// Get statistics about the DAG.
    pub fn stats(&self) -> DagStats {
        let waves = self.execution_waves().unwrap_or_default();
        let critical = self.critical_path().unwrap_or_default();

        DagStats {
            total_nodes: self.nodes.len(),
            root_count: self.roots.len(),
            leaf_count: self.leaves.len(),
            wave_count: waves.len(),
            max_parallelism: waves.iter().map(|w| w.len()).max().unwrap_or(0),
            critical_path_length: critical.len(),
        }
    }
}

/// DAG statistics.
#[derive(Debug, Clone, Default)]
pub struct DagStats {
    pub total_nodes: usize,
    pub root_count: usize,
    pub leaf_count: usize,
    pub wave_count: usize,
    pub max_parallelism: usize,
    pub critical_path_length: usize,
}

/// DAG validation errors.
#[derive(Debug, Clone, thiserror::Error)]
pub enum DagError {
    #[error("Cycle detected in task dependencies")]
    CycleDetected,
    #[error("Missing dependency: {0}")]
    MissingDependency(Uuid),
    #[error("Invalid DAG structure")]
    InvalidStructure,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_task(id: Uuid, title: &str, deps: Vec<Uuid>) -> Task {
        let mut task = Task::with_title(title, "description");
        task.id = id;
        for dep in deps {
            task.depends_on.push(dep);
        }
        task
    }

    #[test]
    fn test_simple_dag() {
        let id1 = Uuid::new_v4();
        let id2 = Uuid::new_v4();
        let id3 = Uuid::new_v4();

        let tasks = vec![
            make_task(id1, "Task 1", vec![]),
            make_task(id2, "Task 2", vec![id1]),
            make_task(id3, "Task 3", vec![id2]),
        ];

        let dag = TaskDag::from_tasks(tasks);

        assert_eq!(dag.nodes.len(), 3);
        assert_eq!(dag.roots.len(), 1);
        assert_eq!(dag.leaves.len(), 1);
        assert!(!dag.has_cycle());
    }

    #[test]
    fn test_topological_sort() {
        let id1 = Uuid::new_v4();
        let id2 = Uuid::new_v4();
        let id3 = Uuid::new_v4();

        let tasks = vec![
            make_task(id1, "Task 1", vec![]),
            make_task(id2, "Task 2", vec![id1]),
            make_task(id3, "Task 3", vec![id1]),
        ];

        let dag = TaskDag::from_tasks(tasks);
        let sorted = dag.topological_sort().unwrap();

        // id1 must come before id2 and id3
        let pos1 = sorted.iter().position(|&x| x == id1).unwrap();
        let pos2 = sorted.iter().position(|&x| x == id2).unwrap();
        let pos3 = sorted.iter().position(|&x| x == id3).unwrap();

        assert!(pos1 < pos2);
        assert!(pos1 < pos3);
    }

    #[test]
    fn test_execution_waves() {
        let id1 = Uuid::new_v4();
        let id2 = Uuid::new_v4();
        let id3 = Uuid::new_v4();
        let id4 = Uuid::new_v4();

        let tasks = vec![
            make_task(id1, "Task 1", vec![]),        // Wave 1
            make_task(id2, "Task 2", vec![]),        // Wave 1
            make_task(id3, "Task 3", vec![id1, id2]), // Wave 2
            make_task(id4, "Task 4", vec![id3]),     // Wave 3
        ];

        let dag = TaskDag::from_tasks(tasks);
        let waves = dag.execution_waves().unwrap();

        assert_eq!(waves.len(), 3);
        assert_eq!(waves[0].len(), 2); // id1 and id2 can run in parallel
        assert_eq!(waves[1].len(), 1); // id3
        assert_eq!(waves[2].len(), 1); // id4
    }

    #[test]
    fn test_cycle_detection() {
        let id1 = Uuid::new_v4();
        let id2 = Uuid::new_v4();
        let id3 = Uuid::new_v4();

        // Create a cycle: 1 -> 2 -> 3 -> 1
        let tasks = vec![
            make_task(id1, "Task 1", vec![id3]),
            make_task(id2, "Task 2", vec![id1]),
            make_task(id3, "Task 3", vec![id2]),
        ];

        let dag = TaskDag::from_tasks(tasks);
        assert!(dag.has_cycle());
        assert!(dag.topological_sort().is_err());
    }

    #[test]
    fn test_critical_path() {
        let id1 = Uuid::new_v4();
        let id2 = Uuid::new_v4();
        let id3 = Uuid::new_v4();
        let id4 = Uuid::new_v4();
        let id5 = Uuid::new_v4();

        //     id1 -> id2 -> id4 -> id5
        //     id3 ------------^
        let tasks = vec![
            make_task(id1, "Task 1", vec![]),
            make_task(id2, "Task 2", vec![id1]),
            make_task(id3, "Task 3", vec![]),
            make_task(id4, "Task 4", vec![id2, id3]),
            make_task(id5, "Task 5", vec![id4]),
        ];

        let dag = TaskDag::from_tasks(tasks);
        let critical = dag.critical_path().unwrap();

        // Critical path should be: id1 -> id2 -> id4 -> id5 (length 4)
        assert_eq!(critical.len(), 4);
    }

    #[test]
    fn test_dag_stats() {
        let id1 = Uuid::new_v4();
        let id2 = Uuid::new_v4();
        let id3 = Uuid::new_v4();

        let tasks = vec![
            make_task(id1, "Task 1", vec![]),
            make_task(id2, "Task 2", vec![id1]),
            make_task(id3, "Task 3", vec![id1]),
        ];

        let dag = TaskDag::from_tasks(tasks);
        let stats = dag.stats();

        assert_eq!(stats.total_nodes, 3);
        assert_eq!(stats.root_count, 1);
        assert_eq!(stats.leaf_count, 2);
        assert_eq!(stats.wave_count, 2);
        assert_eq!(stats.max_parallelism, 2);
    }
}
