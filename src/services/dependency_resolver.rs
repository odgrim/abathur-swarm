use crate::domain::models::task::Task;
use anyhow::{Context, Result, anyhow};
use std::collections::{HashMap, HashSet, VecDeque};
use tracing::{instrument, warn};
use uuid::Uuid;

/// Service for resolving task dependencies using topological sorting
///
/// Coordinates task dependency resolution, cycle detection, and ordering
/// using domain models following Clean Architecture principles.
///
/// # Examples
///
/// ```no_run
/// use abathur::services::DependencyResolver;
/// use abathur::domain::models::task::Task;
/// # use anyhow::Result;
///
/// # fn main() -> Result<()> {
/// let resolver = DependencyResolver::new();
/// # let tasks: Vec<Task> = vec![];
/// let sorted_tasks = resolver.resolve(&tasks)?;
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct DependencyResolver;

impl DependencyResolver {
    /// Create a new `DependencyResolver` instance
    pub const fn new() -> Self {
        Self
    }

    /// Resolve task dependencies using topological sort
    ///
    /// Returns tasks in execution order where all dependencies come before dependents.
    ///
    /// # Arguments
    ///
    /// * `tasks` - Slice of tasks to resolve
    ///
    /// # Returns
    ///
    /// * `Ok(Vec<Task>)` - Tasks in dependency order
    /// * `Err` - If circular dependencies are detected
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - Circular dependencies are detected
    /// - A task depends on a non-existent task
    #[instrument(skip(self, tasks), fields(task_count = tasks.len()))]
    pub fn resolve(&self, tasks: &[Task]) -> Result<Vec<Task>> {
        // First check for cycles
        if let Some(cycle) = self.detect_cycle(tasks) {
            return Err(anyhow!("Circular dependency detected: {cycle:?}"))
                .context("Failed to resolve task dependencies");
        }

        // Perform topological sort
        self.topological_sort(tasks)
            .context("Failed to topologically sort tasks")
    }

    /// Detect circular dependencies using DFS with graph coloring
    ///
    /// Uses three-color DFS algorithm:
    /// - White (not visited)
    /// - Gray (currently visiting)
    /// - Black (completely visited)
    ///
    /// # Arguments
    ///
    /// * `tasks` - Slice of tasks to check
    ///
    /// # Returns
    ///
    /// * `Some(Vec<Uuid>)` - IDs in the detected cycle
    /// * `None` - No cycles detected
    #[instrument(skip(self, tasks), fields(task_count = tasks.len()))]
    pub fn detect_cycle(&self, tasks: &[Task]) -> Option<Vec<Uuid>> {
        // Build adjacency list
        let graph = Self::build_adjacency_list(tasks);
        let task_ids: Vec<Uuid> = tasks.iter().map(|t| t.id).collect();

        // Track colors: 0 = white (unvisited), 1 = gray (visiting), 2 = black (visited)
        let mut colors: HashMap<Uuid, u8> = task_ids.iter().map(|&id| (id, 0)).collect();
        let mut path: Vec<Uuid> = Vec::new();

        for &task_id in &task_ids {
            if colors[&task_id] == 0
                && let Some(cycle) = Self::dfs_detect_cycle(task_id, &graph, &mut colors, &mut path)
            {
                warn!("Circular dependency detected: {:?}", cycle);
                return Some(cycle);
            }
        }

        None
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

    /// Topological sort using Kahn's algorithm
    ///
    /// Returns tasks in dependency order using in-degree tracking.
    ///
    /// # Arguments
    ///
    /// * `tasks` - Slice of tasks to sort
    ///
    /// # Returns
    ///
    /// * `Ok(Vec<Task>)` - Tasks sorted by dependency order
    /// * `Err` - If a task depends on a non-existent task
    #[instrument(skip(self, tasks), fields(task_count = tasks.len()))]
    pub fn topological_sort(&self, tasks: &[Task]) -> Result<Vec<Task>> {
        if tasks.is_empty() {
            return Ok(Vec::new());
        }

        // Build task map for quick lookup
        let task_map: HashMap<Uuid, &Task> = tasks.iter().map(|t| (t.id, t)).collect();

        // Build adjacency list and in-degree map
        let graph = Self::build_adjacency_list(tasks);
        let mut in_degree = Self::calculate_in_degrees(tasks, &graph)?;

        // Queue of tasks with no dependencies (in-degree = 0)
        let mut queue: VecDeque<Uuid> = tasks
            .iter()
            .filter(|t| in_degree[&t.id] == 0)
            .map(|t| t.id)
            .collect();

        let mut sorted: Vec<Task> = Vec::with_capacity(tasks.len());

        while let Some(task_id) = queue.pop_front() {
            // Add task to result
            if let Some(&task) = task_map.get(&task_id) {
                sorted.push(task.clone());
            }

            // Reduce in-degree of dependent tasks
            if let Some(dependents) = graph.get(&task_id) {
                for &dependent_id in dependents {
                    if let Some(degree) = in_degree.get_mut(&dependent_id) {
                        *degree -= 1;
                        if *degree == 0 {
                            queue.push_back(dependent_id);
                        }
                    }
                }
            }
        }

        // Verify all tasks were sorted (no cycles)
        if sorted.len() != tasks.len() {
            return Err(anyhow!(
                "Not all tasks could be sorted. Expected {}, got {}. Possible cycle.",
                tasks.len(),
                sorted.len()
            ));
        }

        Ok(sorted)
    }

    // Private helper methods

    /// Build adjacency list representation of task dependency graph
    ///
    /// Returns a map where each task ID maps to a list of tasks that depend on it.
    fn build_adjacency_list(tasks: &[Task]) -> HashMap<Uuid, Vec<Uuid>> {
        let mut graph: HashMap<Uuid, Vec<Uuid>> = HashMap::new();

        // Initialize all task IDs in the graph
        for task in tasks {
            graph.entry(task.id).or_default();
        }

        // Build edges: if B depends on A, add edge A -> B
        for task in tasks {
            if let Some(ref deps) = task.dependencies {
                for &dep_id in deps {
                    graph.entry(dep_id).or_default().push(task.id);
                }
            }
        }

        graph
    }

    /// Calculate in-degrees for all tasks
    ///
    /// In-degree = number of tasks this task depends on
    fn calculate_in_degrees(
        tasks: &[Task],
        _graph: &HashMap<Uuid, Vec<Uuid>>,
    ) -> Result<HashMap<Uuid, usize>> {
        let task_ids: HashSet<Uuid> = tasks.iter().map(|t| t.id).collect();
        let mut in_degree: HashMap<Uuid, usize> = tasks.iter().map(|t| (t.id, 0)).collect();

        for task in tasks {
            if let Some(ref deps) = task.dependencies {
                for &dep_id in deps {
                    // Validate that dependency exists
                    if !task_ids.contains(&dep_id) {
                        return Err(anyhow!(
                            "Task {} depends on non-existent task {}",
                            task.id,
                            dep_id
                        ));
                    }
                    *in_degree.entry(task.id).or_insert(0) += 1;
                }
            }
        }

        Ok(in_degree)
    }

    /// DFS helper for cycle detection
    ///
    /// Returns the cycle path if found
    fn dfs_detect_cycle(
        node: Uuid,
        graph: &HashMap<Uuid, Vec<Uuid>>,
        colors: &mut HashMap<Uuid, u8>,
        path: &mut Vec<Uuid>,
    ) -> Option<Vec<Uuid>> {
        // Mark as gray (visiting)
        colors.insert(node, 1);
        path.push(node);

        // Visit all neighbors
        if let Some(neighbors) = graph.get(&node) {
            for &neighbor in neighbors {
                let color = colors.get(&neighbor).copied().unwrap_or(0);

                if color == 1 {
                    // Gray node - cycle detected
                    // Extract cycle from path
                    if let Some(cycle_start) = path.iter().position(|&id| id == neighbor) {
                        let cycle = path[cycle_start..].to_vec();
                        return Some(cycle);
                    }
                } else if color == 0 {
                    // White node - continue DFS
                    if let Some(cycle) = Self::dfs_detect_cycle(neighbor, graph, colors, path) {
                        return Some(cycle);
                    }
                }
            }
        }

        // Mark as black (visited)
        colors.insert(node, 2);
        path.pop();

        None
    }

    /// Calculate the dependency depth for a task
    /// Returns the maximum depth in the dependency chain
    pub fn calculate_depth(&self, task: &Task, all_tasks: &[Task]) -> Result<u32> {
        let task_map: HashMap<Uuid, &Task> = all_tasks.iter().map(|t| (t.id, t)).collect();
        let mut visited = HashSet::new();
        calculate_depth_recursive(task, &task_map, &mut visited)
    }

    /// Check if all dependencies for a task are met (completed)
    ///
    /// Returns true if the task has no dependencies or all dependencies are completed
    ///
    /// # Arguments
    ///
    /// * `task` - The task to check
    /// * `all_tasks` - All available tasks to check against
    ///
    /// # Returns
    ///
    /// * `true` - All dependencies are completed
    /// * `false` - One or more dependencies are not completed
    pub fn check_dependencies_met(&self, task: &Task, all_tasks: &[Task]) -> bool {
        use crate::domain::models::task::TaskStatus;

        // If no dependencies, they're all met
        let Some(ref deps) = task.dependencies else {
            return true;
        };

        // Build a map of task statuses for quick lookup
        let status_map: HashMap<Uuid, TaskStatus> =
            all_tasks.iter().map(|t| (t.id, t.status)).collect();

        // Check if all dependencies are completed
        deps.iter().all(|&dep_id| {
            status_map
                .get(&dep_id)
                .map_or(false, |&status| status == TaskStatus::Completed)
        })
    }
}

impl Default for DependencyResolver {
    fn default() -> Self {
        Self::new()
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
    use crate::domain::models::task::{DependencyType, TaskSource, TaskStatus, ValidationRequirement};
    use chrono::Utc;

    fn create_test_task(id: &str, dependencies: Option<Vec<&str>>) -> Task {
        let task_id = Uuid::parse_str(id).unwrap();
        let deps = dependencies.map(|d| d.iter().map(|&s| Uuid::parse_str(s).unwrap()).collect());

        Task {
            id: task_id,
            summary: format!("Task {id}"),
            description: "Test task".to_string(),
            agent_type: "test".to_string(),
            priority: 5,
            calculated_priority: 5.0,
            status: TaskStatus::Pending,
            dependencies: deps,
            dependency_type: DependencyType::Sequential,
            dependency_depth: 0,
            input_data: None,
            result_data: None,
            error_message: None,
            retry_count: 0,
            max_retries: 3,
            max_execution_timeout_seconds: 3600,
            submitted_at: Utc::now(),
            started_at: None,
            completed_at: None,
            last_updated_at: Utc::now(),
            created_by: None,
            parent_task_id: None,
            session_id: None,
            source: TaskSource::Human,
            deadline: None,
            estimated_duration_seconds: None,
            feature_branch: None,
            task_branch: None,
            worktree_path: None,
            validation_requirement: ValidationRequirement::None,
            validation_task_id: None,
            validating_task_id: None,
            remediation_count: 0,
            is_remediation: false,
            workflow_state: None,
            workflow_expectations: None,
            chain_id: None,
        }
    }

    #[test]
    fn test_empty_tasks() {
        let resolver = DependencyResolver::new();
        let tasks: Vec<Task> = vec![];
        let result = resolver.resolve(&tasks).unwrap();
        assert_eq!(result.len(), 0);
    }

    #[test]
    fn test_single_task_no_dependencies() {
        let resolver = DependencyResolver::new();
        let tasks = vec![create_test_task(
            "00000000-0000-0000-0000-000000000001",
            None,
        )];

        let result = resolver.resolve(&tasks).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].id, tasks[0].id);
    }

    #[test]
    fn test_linear_dependencies() {
        let resolver = DependencyResolver::new();
        let id1 = "00000000-0000-0000-0000-000000000001";
        let id2 = "00000000-0000-0000-0000-000000000002";
        let id3 = "00000000-0000-0000-0000-000000000003";

        // Task 3 depends on task 2, task 2 depends on task 1
        let tasks = vec![
            create_test_task(id3, Some(vec![id2])),
            create_test_task(id1, None),
            create_test_task(id2, Some(vec![id1])),
        ];

        let result = resolver.resolve(&tasks).unwrap();
        assert_eq!(result.len(), 3);

        // Verify order: task1 -> task2 -> task3
        assert_eq!(result[0].id, Uuid::parse_str(id1).unwrap());
        assert_eq!(result[1].id, Uuid::parse_str(id2).unwrap());
        assert_eq!(result[2].id, Uuid::parse_str(id3).unwrap());
    }

    #[test]
    fn test_diamond_dependencies() {
        let resolver = DependencyResolver::new();
        let id1 = "00000000-0000-0000-0000-000000000001";
        let id2 = "00000000-0000-0000-0000-000000000002";
        let id3 = "00000000-0000-0000-0000-000000000003";
        let id4 = "00000000-0000-0000-0000-000000000004";

        // Diamond: 1 -> {2, 3} -> 4
        let tasks = vec![
            create_test_task(id1, None),
            create_test_task(id2, Some(vec![id1])),
            create_test_task(id3, Some(vec![id1])),
            create_test_task(id4, Some(vec![id2, id3])),
        ];

        let result = resolver.resolve(&tasks).unwrap();
        assert_eq!(result.len(), 4);

        // Task 1 must come first
        assert_eq!(result[0].id, Uuid::parse_str(id1).unwrap());

        // Task 4 must come last
        assert_eq!(result[3].id, Uuid::parse_str(id4).unwrap());

        // Tasks 2 and 3 must be in the middle (order doesn't matter)
        let middle_ids: Vec<Uuid> = result[1..3].iter().map(|t| t.id).collect();
        assert!(middle_ids.contains(&Uuid::parse_str(id2).unwrap()));
        assert!(middle_ids.contains(&Uuid::parse_str(id3).unwrap()));
    }

    #[test]
    fn test_detect_simple_cycle() {
        let resolver = DependencyResolver::new();
        let id1 = "00000000-0000-0000-0000-000000000001";
        let id2 = "00000000-0000-0000-0000-000000000002";

        // Task 1 depends on task 2, task 2 depends on task 1
        let tasks = vec![
            create_test_task(id1, Some(vec![id2])),
            create_test_task(id2, Some(vec![id1])),
        ];

        let cycle = resolver.detect_cycle(&tasks);
        assert!(cycle.is_some());

        let cycle_ids = cycle.unwrap();
        assert_eq!(cycle_ids.len(), 2);
    }

    #[test]
    fn test_detect_three_node_cycle() {
        let resolver = DependencyResolver::new();
        let id1 = "00000000-0000-0000-0000-000000000001";
        let id2 = "00000000-0000-0000-0000-000000000002";
        let id3 = "00000000-0000-0000-0000-000000000003";

        // Cycle: 1 -> 2 -> 3 -> 1
        let tasks = vec![
            create_test_task(id1, Some(vec![id3])),
            create_test_task(id2, Some(vec![id1])),
            create_test_task(id3, Some(vec![id2])),
        ];

        let cycle = resolver.detect_cycle(&tasks);
        assert!(cycle.is_some());

        let cycle_ids = cycle.unwrap();
        assert_eq!(cycle_ids.len(), 3);
    }

    #[test]
    fn test_no_cycle_detection() {
        let resolver = DependencyResolver::new();
        let id1 = "00000000-0000-0000-0000-000000000001";
        let id2 = "00000000-0000-0000-0000-000000000002";
        let id3 = "00000000-0000-0000-0000-000000000003";

        // Linear: 1 -> 2 -> 3 (no cycle)
        let tasks = vec![
            create_test_task(id1, None),
            create_test_task(id2, Some(vec![id1])),
            create_test_task(id3, Some(vec![id2])),
        ];

        let cycle = resolver.detect_cycle(&tasks);
        assert!(cycle.is_none());
    }

    #[test]
    fn test_resolve_fails_on_cycle() {
        let resolver = DependencyResolver::new();
        let id1 = "00000000-0000-0000-0000-000000000001";
        let id2 = "00000000-0000-0000-0000-000000000002";

        // Cycle: 1 -> 2 -> 1
        let tasks = vec![
            create_test_task(id1, Some(vec![id2])),
            create_test_task(id2, Some(vec![id1])),
        ];

        let result = resolver.resolve(&tasks);
        assert!(result.is_err());
        let err_msg = format!("{:?}", result.as_ref().unwrap_err());
        assert!(
            err_msg.contains("Circular dependency"),
            "Expected 'Circular dependency' in: {err_msg}"
        );
    }

    #[test]
    fn test_nonexistent_dependency() {
        let resolver = DependencyResolver::new();
        let id1 = "00000000-0000-0000-0000-000000000001";
        let id_missing = "00000000-0000-0000-0000-000000000999";

        // Task 1 depends on non-existent task
        let tasks = vec![create_test_task(id1, Some(vec![id_missing]))];

        let result = resolver.resolve(&tasks);
        assert!(result.is_err());
        let err_msg = format!("{:?}", result.as_ref().unwrap_err());
        assert!(
            err_msg.contains("non-existent"),
            "Expected 'non-existent' in: {err_msg}"
        );
    }

    #[test]
    fn test_multiple_independent_tasks() {
        let resolver = DependencyResolver::new();
        let id1 = "00000000-0000-0000-0000-000000000001";
        let id2 = "00000000-0000-0000-0000-000000000002";
        let id3 = "00000000-0000-0000-0000-000000000003";

        // Three independent tasks
        let tasks = vec![
            create_test_task(id1, None),
            create_test_task(id2, None),
            create_test_task(id3, None),
        ];

        let result = resolver.resolve(&tasks).unwrap();
        assert_eq!(result.len(), 3);

        // All tasks should be included (order doesn't matter)
        let result_ids: HashSet<Uuid> = result.iter().map(|t| t.id).collect();
        assert!(result_ids.contains(&Uuid::parse_str(id1).unwrap()));
        assert!(result_ids.contains(&Uuid::parse_str(id2).unwrap()));
        assert!(result_ids.contains(&Uuid::parse_str(id3).unwrap()));
    }

    #[test]
    fn test_complex_dependency_graph() {
        let resolver = DependencyResolver::new();
        let id1 = "00000000-0000-0000-0000-000000000001";
        let id2 = "00000000-0000-0000-0000-000000000002";
        let id3 = "00000000-0000-0000-0000-000000000003";
        let id4 = "00000000-0000-0000-0000-000000000004";
        let id5 = "00000000-0000-0000-0000-000000000005";
        let id6 = "00000000-0000-0000-0000-000000000006";

        // Complex graph:
        // 1 -> 3 -> 5
        // 2 -> 4 -> 6
        // 3 -> 4
        let tasks = vec![
            create_test_task(id1, None),
            create_test_task(id2, None),
            create_test_task(id3, Some(vec![id1])),
            create_test_task(id4, Some(vec![id2, id3])),
            create_test_task(id5, Some(vec![id3])),
            create_test_task(id6, Some(vec![id4])),
        ];

        let result = resolver.resolve(&tasks).unwrap();
        assert_eq!(result.len(), 6);

        // Convert to position map for easier verification
        let positions: HashMap<Uuid, usize> =
            result.iter().enumerate().map(|(i, t)| (t.id, i)).collect();

        // Verify dependency constraints
        assert!(
            positions[&Uuid::parse_str(id1).unwrap()] < positions[&Uuid::parse_str(id3).unwrap()]
        );
        assert!(
            positions[&Uuid::parse_str(id2).unwrap()] < positions[&Uuid::parse_str(id4).unwrap()]
        );
        assert!(
            positions[&Uuid::parse_str(id3).unwrap()] < positions[&Uuid::parse_str(id4).unwrap()]
        );
        assert!(
            positions[&Uuid::parse_str(id3).unwrap()] < positions[&Uuid::parse_str(id5).unwrap()]
        );
        assert!(
            positions[&Uuid::parse_str(id4).unwrap()] < positions[&Uuid::parse_str(id6).unwrap()]
        );
    }
}
