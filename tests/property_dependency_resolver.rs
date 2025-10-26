use abathur::domain::models::task::{DependencyType, Task, TaskSource, TaskStatus};
use abathur::services::DependencyResolver;
use chrono::Utc;
use proptest::prelude::*;
use proptest::test_runner::TestCaseError;
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

proptest! {
    /// Property: Topological sort never produces cycles
    ///
    /// For any acyclic task graph, the resolved order should maintain
    /// the property that all dependencies come before their dependents.
    #[test]
    fn prop_topological_sort_no_cycles(
        size in 1usize..20
    ) {
        let resolver = DependencyResolver::new();

        // Generate acyclic graph
        let task_ids: Vec<Uuid> = (0..size).map(|_| Uuid::new_v4()).collect();
        let mut tasks = Vec::new();

        for (i, &id) in task_ids.iter().enumerate() {
            let deps = if i > 0 && i % 2 == 0 {
                // Every even task depends on the previous task
                Some(vec![task_ids[i - 1]])
            } else {
                None
            };

            tasks.push(Task {
                id,
                summary: format!("Task {id}"),
                description: "Property test task".to_string(),
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
            });
        }

        let result = resolver.topological_sort(&tasks)
            .map_err(|e| TestCaseError::fail(e.to_string()))?;

        // Verify: All dependencies come before dependents
        let position_map: HashMap<Uuid, usize> = result
            .iter()
            .enumerate()
            .map(|(i, t)| (t.id, i))
            .collect();

        for task in &result {
            if let Some(ref deps) = task.dependencies {
                for &dep_id in deps {
                    let dep_pos = position_map.get(&dep_id).unwrap();
                    let task_pos = position_map.get(&task.id).unwrap();
                    prop_assert!(dep_pos < task_pos,
                        "Dependency {} at position {} should come before task {} at position {}",
                        dep_id, dep_pos, task.id, task_pos);
                }
            }
        }
    }

    /// Property: Resolved tasks contain all input tasks
    ///
    /// The topological sort should not lose or duplicate any tasks.
    #[test]
    fn prop_topological_sort_preserves_tasks(
        size in 1usize..20
    ) {
        let resolver = DependencyResolver::new();

        // Generate simple task graph
        let task_ids: Vec<Uuid> = (0..size).map(|_| Uuid::new_v4()).collect();
        let mut tasks = Vec::new();

        for &id in &task_ids {
            tasks.push(Task {
                id,
                summary: format!("Task {id}"),
                description: "Property test task".to_string(),
                agent_type: "test".to_string(),
                priority: 5,
                calculated_priority: 5.0,
                status: TaskStatus::Pending,
                dependencies: None,
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
            });
        }

        let result = resolver.topological_sort(&tasks)
            .map_err(|e| TestCaseError::fail(e.to_string()))?;

        // Verify: Same number of tasks
        prop_assert_eq!(result.len(), tasks.len());

        // Verify: All original task IDs are present
        let input_ids: HashSet<Uuid> = tasks.iter().map(|t| t.id).collect();
        let output_ids: HashSet<Uuid> = result.iter().map(|t| t.id).collect();
        prop_assert_eq!(input_ids, output_ids);
    }

    /// Property: Cycle detection is consistent
    ///
    /// If cycle detection fails, resolve should also fail.
    /// If no cycle is detected, resolve should succeed.
    #[test]
    fn prop_cycle_detection_consistency(
        size in 1usize..15
    ) {
        let resolver = DependencyResolver::new();

        // Generate linear dependency chain (no cycles)
        let task_ids: Vec<Uuid> = (0..size).map(|_| Uuid::new_v4()).collect();
        let mut tasks = Vec::new();

        for (i, &id) in task_ids.iter().enumerate() {
            let deps = if i > 0 {
                Some(vec![task_ids[i - 1]])
            } else {
                None
            };

            tasks.push(Task {
                id,
                summary: format!("Task {id}"),
                description: "Property test task".to_string(),
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
            });
        }

        let has_cycle = resolver.detect_cycle(&tasks).is_some();
        let resolve_result = resolver.resolve(&tasks);

        if has_cycle {
            prop_assert!(resolve_result.is_err(), "Resolve should fail when cycle detected");
        } else {
            prop_assert!(resolve_result.is_ok(), "Resolve should succeed when no cycle");
        }
    }

    /// Property: Independent tasks can be in any order
    ///
    /// Tasks with no dependencies between them can appear in any order
    /// in the result (but all should be present).
    #[test]
    fn prop_independent_tasks_all_present(
        size in 1usize..20
    ) {
        let resolver = DependencyResolver::new();

        // Generate independent tasks (no dependencies)
        let task_ids: Vec<Uuid> = (0..size).map(|_| Uuid::new_v4()).collect();
        let mut tasks = Vec::new();

        for &id in &task_ids {
            tasks.push(Task {
                id,
                summary: format!("Task {id}"),
                description: "Property test task".to_string(),
                agent_type: "test".to_string(),
                priority: 5,
                calculated_priority: 5.0,
                status: TaskStatus::Pending,
                dependencies: None,
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
            });
        }

        let result = resolver.topological_sort(&tasks)
            .map_err(|e| TestCaseError::fail(e.to_string()))?;

        // All tasks should be present
        prop_assert_eq!(result.len(), size);

        // All IDs should match
        let input_ids: HashSet<Uuid> = task_ids.into_iter().collect();
        let output_ids: HashSet<Uuid> = result.iter().map(|t| t.id).collect();
        prop_assert_eq!(input_ids, output_ids);
    }
}
