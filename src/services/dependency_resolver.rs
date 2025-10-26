use crate::domain::models::Task;
use anyhow::Result;
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

/// Service for resolving task dependencies and detecting circular dependencies
#[derive(Debug, Clone, Default)]
pub struct DependencyResolver;

// Standalone helper for cycle detection (no self needed)
fn detect_cycle_util(
    node: Uuid,
    graph: &HashMap<Uuid, Vec<Uuid>>,
    visited: &mut HashSet<Uuid>,
    rec_stack: &mut HashSet<Uuid>,
    path: &mut Vec<Uuid>,
) -> bool {
    visited.insert(node);
    rec_stack.insert(node);
    path.push(node);

    if let Some(neighbors) = graph.get(&node) {
        for &neighbor in neighbors {
            if !visited.contains(&neighbor) {
                if detect_cycle_util(neighbor, graph, visited, rec_stack, path) {
                    return true;
                }
            } else if rec_stack.contains(&neighbor) {
                // Cycle detected
                if let Some(cycle_start) = path.iter().position(|&id| id == neighbor) {
                    path.drain(0..cycle_start);
                    return true;
                }
            }
        }
    }

    rec_stack.remove(&node);
    path.pop();
    false
}

impl DependencyResolver {
    pub fn new() -> Self {
        Self
    }

    /// Validate that all dependencies exist
    pub fn validate_dependencies(&self, task: &Task, available_tasks: &[Task]) -> Result<()> {
        if let Some(deps) = &task.dependencies {
            let available_ids: HashSet<Uuid> = available_tasks.iter().map(|t| t.id).collect();

            for dep_id in deps {
                if !available_ids.contains(dep_id) && *dep_id != task.id {
                    return Err(anyhow::anyhow!("Dependency task {} not found", dep_id));
                }
            }
        }
        Ok(())
    }

    /// Detect circular dependencies in a set of tasks
    pub fn detect_cycle(&self, tasks: &[Task]) -> Option<Vec<Uuid>> {
        let mut graph: HashMap<Uuid, Vec<Uuid>> = HashMap::new();

        // Build adjacency list
        for task in tasks {
            graph
                .entry(task.id)
                .or_default()
                .extend(task.get_dependencies().iter().copied());
        }

        // DFS-based cycle detection
        let mut visited = HashSet::new();
        let mut rec_stack = HashSet::new();
        let mut path = Vec::new();

        for task_id in graph.keys() {
            if !visited.contains(task_id)
                && detect_cycle_util(*task_id, &graph, &mut visited, &mut rec_stack, &mut path)
            {
                return Some(path);
            }
        }

        None
    }

    /// Perform topological sort on tasks based on dependencies
    /// Returns tasks in dependency order (dependencies before dependents)
    pub fn topological_sort(&self, tasks: &[Task]) -> Result<Vec<Task>> {
        // Check for cycles first
        if let Some(cycle) = self.detect_cycle(tasks) {
            return Err(anyhow::anyhow!("Circular dependency detected: {:?}", cycle));
        }

        let mut task_map: HashMap<Uuid, Task> = tasks.iter().map(|t| (t.id, t.clone())).collect();
        let mut in_degree: HashMap<Uuid, usize> = HashMap::new();
        let mut graph: HashMap<Uuid, Vec<Uuid>> = HashMap::new();

        // Build graph and calculate in-degrees
        for task in tasks {
            in_degree.entry(task.id).or_insert(0);
            if let Some(deps) = &task.dependencies {
                for &dep_id in deps {
                    graph.entry(dep_id).or_default().push(task.id);
                    *in_degree.entry(task.id).or_insert(0) += 1;
                }
            }
        }

        // Kahn's algorithm for topological sort
        let mut queue: Vec<Uuid> = in_degree
            .iter()
            .filter(|&(_, &degree)| degree == 0)
            .map(|(&id, _)| id)
            .collect();

        let mut sorted = Vec::new();

        while let Some(node_id) = queue.pop() {
            if let Some(task) = task_map.remove(&node_id) {
                sorted.push(task);
            }

            if let Some(neighbors) = graph.get(&node_id) {
                for &neighbor in neighbors {
                    if let Some(degree) = in_degree.get_mut(&neighbor) {
                        *degree -= 1;
                        if *degree == 0 {
                            queue.push(neighbor);
                        }
                    }
                }
            }
        }

        if sorted.len() != tasks.len() {
            return Err(anyhow::anyhow!(
                "Topological sort failed: possible cycle or disconnected graph"
            ));
        }

        Ok(sorted)
    }

    /// Calculate the dependency depth for a task
    /// Returns the maximum depth in the dependency chain
    pub fn calculate_depth(&self, task: &Task, all_tasks: &[Task]) -> Result<u32> {
        let task_map: HashMap<Uuid, &Task> = all_tasks.iter().map(|t| (t.id, t)).collect();
        let mut visited = HashSet::new();
        calculate_depth_recursive(task, &task_map, &mut visited)
    }
}

// Standalone helper for depth calculation
fn calculate_depth_recursive(
    task: &Task,
    task_map: &HashMap<Uuid, &Task>,
    visited: &mut HashSet<Uuid>,
) -> Result<u32> {
    if visited.contains(&task.id) {
        return Err(anyhow::anyhow!("Circular dependency detected"));
    }

    visited.insert(task.id);

    let max_depth = if let Some(deps) = &task.dependencies {
        let mut depths = Vec::new();
        for &dep_id in deps {
            if let Some(&dep_task) = task_map.get(&dep_id) {
                let depth = calculate_depth_recursive(dep_task, task_map, visited)?;
                depths.push(depth);
            }
        }
        depths.into_iter().max().unwrap_or(0) + 1
    } else {
        0
    };

    visited.remove(&task.id);
    Ok(max_depth)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_task(id: Uuid, dependencies: Option<Vec<Uuid>>) -> Task {
        let mut task = Task::new("Test".to_string(), "Description".to_string());
        task.id = id;
        task.dependencies = dependencies;
        task
    }

    #[test]
    fn test_validate_dependencies_success() {
        let resolver = DependencyResolver::new();
        let id1 = Uuid::new_v4();
        let id2 = Uuid::new_v4();

        let task1 = create_test_task(id1, None);
        let task2 = create_test_task(id2, Some(vec![id1]));

        let available = vec![task1.clone()];
        assert!(resolver.validate_dependencies(&task2, &available).is_ok());
    }

    #[test]
    fn test_validate_dependencies_missing() {
        let resolver = DependencyResolver::new();
        let id1 = Uuid::new_v4();
        let id2 = Uuid::new_v4();

        let task = create_test_task(id1, Some(vec![id2]));
        let available = vec![];

        assert!(resolver.validate_dependencies(&task, &available).is_err());
    }

    #[test]
    fn test_detect_cycle_no_cycle() {
        let resolver = DependencyResolver::new();
        let id1 = Uuid::new_v4();
        let id2 = Uuid::new_v4();

        let task1 = create_test_task(id1, None);
        let task2 = create_test_task(id2, Some(vec![id1]));

        let tasks = vec![task1, task2];
        assert!(resolver.detect_cycle(&tasks).is_none());
    }

    #[test]
    fn test_detect_cycle_with_cycle() {
        let resolver = DependencyResolver::new();
        let id1 = Uuid::new_v4();
        let id2 = Uuid::new_v4();

        let task1 = create_test_task(id1, Some(vec![id2]));
        let task2 = create_test_task(id2, Some(vec![id1]));

        let tasks = vec![task1, task2];
        assert!(resolver.detect_cycle(&tasks).is_some());
    }

    #[test]
    fn test_topological_sort_simple() {
        let resolver = DependencyResolver::new();
        let id1 = Uuid::new_v4();
        let id2 = Uuid::new_v4();
        let id3 = Uuid::new_v4();

        let task1 = create_test_task(id1, None);
        let task2 = create_test_task(id2, Some(vec![id1]));
        let task3 = create_test_task(id3, Some(vec![id2]));

        let tasks = vec![task3.clone(), task1.clone(), task2.clone()];
        let sorted = resolver.topological_sort(&tasks).unwrap();

        assert_eq!(sorted[0].id, id1);
        assert_eq!(sorted[1].id, id2);
        assert_eq!(sorted[2].id, id3);
    }

    #[test]
    fn test_topological_sort_with_cycle() {
        let resolver = DependencyResolver::new();
        let id1 = Uuid::new_v4();
        let id2 = Uuid::new_v4();

        let task1 = create_test_task(id1, Some(vec![id2]));
        let task2 = create_test_task(id2, Some(vec![id1]));

        let tasks = vec![task1, task2];
        assert!(resolver.topological_sort(&tasks).is_err());
    }

    #[test]
    fn test_calculate_depth() {
        let resolver = DependencyResolver::new();
        let id1 = Uuid::new_v4();
        let id2 = Uuid::new_v4();
        let id3 = Uuid::new_v4();

        let task1 = create_test_task(id1, None);
        let task2 = create_test_task(id2, Some(vec![id1]));
        let task3 = create_test_task(id3, Some(vec![id2]));

        let all_tasks = vec![task1.clone(), task2.clone(), task3.clone()];

        assert_eq!(resolver.calculate_depth(&task1, &all_tasks).unwrap(), 0);
        assert_eq!(resolver.calculate_depth(&task2, &all_tasks).unwrap(), 1);
        assert_eq!(resolver.calculate_depth(&task3, &all_tasks).unwrap(), 2);
    }
}
