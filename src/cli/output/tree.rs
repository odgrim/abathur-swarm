use comfy_table::Color;
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

use crate::cli::models::{Task, TaskStatus};

/// Unicode box-drawing characters for tree visualization
const TREE_BRANCH: &str = "├── ";
const TREE_LAST: &str = "└── ";
const TREE_PIPE: &str = "│   ";
const TREE_SPACE: &str = "    ";

/// Render a dependency tree for a single task
///
/// # Arguments
/// * `task_id` - The ID of the task to render
/// * `tasks` - Map of all tasks by ID
/// * `depth` - Current depth in the tree (0 for root)
/// * `is_last` - Whether this is the last child at this level
/// * `prefix` - Accumulated prefix string for indentation
///
/// # Returns
/// String representation of the tree structure
pub fn render_dependency_tree(
    task_id: Uuid,
    tasks: &HashMap<Uuid, Task>,
    depth: usize,
    is_last: bool,
    prefix: &str,
) -> String {
    let task = match tasks.get(&task_id) {
        Some(t) => t,
        None => return format!("{}[Task not found: {}]\n", prefix, task_id),
    };

    let mut output = String::new();

    // Current node connector
    let connector = if depth == 0 {
        ""
    } else if is_last {
        TREE_LAST
    } else {
        TREE_BRANCH
    };

    let status_icon = status_icon(task.status);
    let short_id = truncate_uuid(&task.id);

    output.push_str(&format!(
        "{}{}{} {} [{}]\n",
        prefix, connector, status_icon, task.description, short_id
    ));

    // Render children (dependencies)
    if !task.dependencies.is_empty() {
        let child_prefix = if depth == 0 {
            String::new()
        } else if is_last {
            format!("{}{}", prefix, TREE_SPACE)
        } else {
            format!("{}{}", prefix, TREE_PIPE)
        };

        for (i, dep_id) in task.dependencies.iter().enumerate() {
            let is_last_child = i == task.dependencies.len() - 1;
            output.push_str(&render_dependency_tree(
                *dep_id,
                tasks,
                depth + 1,
                is_last_child,
                &child_prefix,
            ));
        }
    }

    output
}

/// Render multiple trees (for tasks with no dependencies)
///
/// # Arguments
/// * `root_tasks` - List of task IDs that have no dependencies
/// * `tasks` - Map of all tasks by ID
///
/// # Returns
/// String representation of all trees
pub fn render_multiple_trees(root_tasks: &[Uuid], tasks: &HashMap<Uuid, Task>) -> String {
    let mut output = String::new();

    for (i, task_id) in root_tasks.iter().enumerate() {
        output.push_str(&render_dependency_tree(*task_id, tasks, 0, true, ""));

        // Add blank line between trees (except after last one)
        if i < root_tasks.len() - 1 {
            output.push('\n');
        }
    }

    output
}

/// Find all root tasks (tasks with no dependencies or whose dependencies are all satisfied)
///
/// # Arguments
/// * `tasks` - Slice of all tasks
///
/// # Returns
/// Vector of task IDs that are roots
pub fn find_root_tasks(tasks: &[Task]) -> Vec<Uuid> {
    tasks
        .iter()
        .filter(|t| t.dependencies.is_empty())
        .map(|t| t.id)
        .collect()
}

/// Render a colored status icon with ANSI color codes
///
/// # Arguments
/// * `status` - Task status
/// * `use_color` - Whether to use color codes
///
/// # Returns
/// String with status icon (optionally colored)
pub fn render_status_colored(status: TaskStatus, use_color: bool) -> String {
    let icon = status_icon(status);

    if use_color {
        let color_code = status_ansi_color(status);
        format!("\x1b[{}m{}\x1b[0m", color_code, icon)
    } else {
        icon.to_string()
    }
}

/// Map status to visual icon
fn status_icon(status: TaskStatus) -> &'static str {
    match status {
        TaskStatus::Completed => "✓",
        TaskStatus::Running => "⟳",
        TaskStatus::Failed => "✗",
        TaskStatus::Cancelled => "⊘",
        TaskStatus::Ready => "●",
        TaskStatus::Blocked => "⊗",
        TaskStatus::Pending => "○",
    }
}

/// Map status to ANSI color code
fn status_ansi_color(status: TaskStatus) -> &'static str {
    match status {
        TaskStatus::Completed => "32",  // Green
        TaskStatus::Running => "36",    // Cyan
        TaskStatus::Failed => "31",     // Red
        TaskStatus::Cancelled => "90",  // Dark Grey
        TaskStatus::Ready => "33",      // Yellow
        TaskStatus::Blocked => "35",    // Magenta
        TaskStatus::Pending => "37",    // White
    }
}

/// Map status to comfy-table color (for compatibility)
pub fn status_color(status: TaskStatus) -> Color {
    match status {
        TaskStatus::Completed => Color::Green,
        TaskStatus::Running => Color::Cyan,
        TaskStatus::Failed => Color::Red,
        TaskStatus::Cancelled => Color::DarkGrey,
        TaskStatus::Ready => Color::Yellow,
        TaskStatus::Blocked => Color::Magenta,
        TaskStatus::Pending => Color::White,
    }
}

/// Truncate UUID to first 8 characters
fn truncate_uuid(uuid: &Uuid) -> String {
    uuid.to_string()[..8].to_string()
}

/// Validate tree structure (detect cycles)
///
/// # Arguments
/// * `tasks` - Map of all tasks by ID
///
/// # Returns
/// Result with list of task IDs involved in cycles, if any
pub fn validate_tree_structure(tasks: &HashMap<Uuid, Task>) -> Result<(), Vec<Uuid>> {
    let mut visiting = HashSet::new();
    let mut visited = HashSet::new();
    let mut cycles = Vec::new();

    for task_id in tasks.keys() {
        if !visited.contains(task_id) {
            if has_cycle(*task_id, tasks, &mut visiting, &mut visited) {
                cycles.push(*task_id);
            }
        }
    }

    if cycles.is_empty() {
        Ok(())
    } else {
        Err(cycles)
    }
}

/// DFS helper to detect cycles
fn has_cycle(
    task_id: Uuid,
    tasks: &HashMap<Uuid, Task>,
    visiting: &mut HashSet<Uuid>,
    visited: &mut HashSet<Uuid>,
) -> bool {
    if visiting.contains(&task_id) {
        return true; // Cycle detected
    }

    if visited.contains(&task_id) {
        return false; // Already processed
    }

    visiting.insert(task_id);

    let task = match tasks.get(&task_id) {
        Some(t) => t,
        None => {
            visiting.remove(&task_id);
            return false;
        }
    };

    for dep_id in &task.dependencies {
        if has_cycle(*dep_id, tasks, visiting, visited) {
            return true;
        }
    }

    visiting.remove(&task_id);
    visited.insert(task_id);
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn create_test_task(
        description: &str,
        status: TaskStatus,
        dependencies: Vec<Uuid>,
    ) -> Task {
        Task {
            id: Uuid::new_v4(),
            description: description.to_string(),
            status,
            agent_type: "test-agent".to_string(),
            priority: 5,
            base_priority: 5,
            computed_priority: 5.0,
            dependencies,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            started_at: None,
            completed_at: None,
        }
    }

    #[test]
    fn test_status_icon_mapping() {
        assert_eq!(status_icon(TaskStatus::Completed), "✓");
        assert_eq!(status_icon(TaskStatus::Running), "⟳");
        assert_eq!(status_icon(TaskStatus::Failed), "✗");
        assert_eq!(status_icon(TaskStatus::Cancelled), "⊘");
        assert_eq!(status_icon(TaskStatus::Ready), "●");
        assert_eq!(status_icon(TaskStatus::Blocked), "⊗");
        assert_eq!(status_icon(TaskStatus::Pending), "○");
    }

    #[test]
    fn test_truncate_uuid() {
        let uuid = Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap();
        assert_eq!(truncate_uuid(&uuid), "550e8400");
    }

    #[test]
    fn test_render_single_task_no_dependencies() {
        let task = create_test_task("Test task", TaskStatus::Pending, vec![]);
        let mut tasks = HashMap::new();
        tasks.insert(task.id, task.clone());

        let tree = render_dependency_tree(task.id, &tasks, 0, true, "");

        assert!(tree.contains("Test task"));
        assert!(tree.contains("○")); // Pending icon
    }

    #[test]
    fn test_render_task_with_dependencies() {
        let dep1 = create_test_task("Dependency 1", TaskStatus::Completed, vec![]);
        let dep2 = create_test_task("Dependency 2", TaskStatus::Running, vec![]);
        let parent = create_test_task(
            "Parent task",
            TaskStatus::Blocked,
            vec![dep1.id, dep2.id],
        );

        let mut tasks = HashMap::new();
        tasks.insert(dep1.id, dep1);
        tasks.insert(dep2.id, dep2);
        tasks.insert(parent.id, parent.clone());

        let tree = render_dependency_tree(parent.id, &tasks, 0, true, "");

        assert!(tree.contains("Parent task"));
        assert!(tree.contains("Dependency 1"));
        assert!(tree.contains("Dependency 2"));
        assert!(tree.contains("├──") || tree.contains("└──"));
    }

    #[test]
    fn test_find_root_tasks() {
        let root1 = create_test_task("Root 1", TaskStatus::Pending, vec![]);
        let root2 = create_test_task("Root 2", TaskStatus::Pending, vec![]);
        let child = create_test_task("Child", TaskStatus::Pending, vec![root1.id]);

        let tasks = vec![root1.clone(), root2.clone(), child];
        let roots = find_root_tasks(&tasks);

        assert_eq!(roots.len(), 2);
        assert!(roots.contains(&root1.id));
        assert!(roots.contains(&root2.id));
    }

    #[test]
    fn test_render_multiple_trees() {
        let root1 = create_test_task("Root 1", TaskStatus::Completed, vec![]);
        let root2 = create_test_task("Root 2", TaskStatus::Running, vec![]);

        let mut tasks = HashMap::new();
        tasks.insert(root1.id, root1.clone());
        tasks.insert(root2.id, root2.clone());

        let output = render_multiple_trees(&[root1.id, root2.id], &tasks);

        assert!(output.contains("Root 1"));
        assert!(output.contains("Root 2"));
    }

    #[test]
    fn test_render_status_colored() {
        let colored = render_status_colored(TaskStatus::Completed, true);
        assert!(colored.contains("\x1b[32m")); // Green ANSI code
        assert!(colored.contains("✓"));

        let plain = render_status_colored(TaskStatus::Completed, false);
        assert!(!plain.contains("\x1b[")); // No ANSI codes
        assert!(plain.contains("✓"));
    }

    #[test]
    fn test_validate_tree_structure_no_cycles() {
        let task1 = create_test_task("Task 1", TaskStatus::Pending, vec![]);
        let task2 = create_test_task("Task 2", TaskStatus::Pending, vec![task1.id]);

        let mut tasks = HashMap::new();
        tasks.insert(task1.id, task1);
        tasks.insert(task2.id, task2);

        assert!(validate_tree_structure(&tasks).is_ok());
    }

    #[test]
    fn test_validate_tree_structure_with_cycle() {
        let task1_id = Uuid::new_v4();
        let task2_id = Uuid::new_v4();

        let mut task1 = create_test_task("Task 1", TaskStatus::Pending, vec![task2_id]);
        task1.id = task1_id;

        let mut task2 = create_test_task("Task 2", TaskStatus::Pending, vec![task1_id]);
        task2.id = task2_id;

        let mut tasks = HashMap::new();
        tasks.insert(task1.id, task1);
        tasks.insert(task2.id, task2);

        assert!(validate_tree_structure(&tasks).is_err());
    }

    #[test]
    fn test_unicode_box_drawing_characters() {
        let dep = create_test_task("Dependency", TaskStatus::Completed, vec![]);
        let parent = create_test_task("Parent", TaskStatus::Ready, vec![dep.id]);

        let mut tasks = HashMap::new();
        tasks.insert(dep.id, dep);
        tasks.insert(parent.id, parent.clone());

        let tree = render_dependency_tree(parent.id, &tasks, 0, true, "");

        // Should contain Unicode box-drawing characters
        assert!(tree.contains("└──") || tree.contains("├──"));
    }

    #[test]
    fn test_deep_nesting() {
        let task1 = create_test_task("Task 1", TaskStatus::Completed, vec![]);
        let task2 = create_test_task("Task 2", TaskStatus::Running, vec![task1.id]);
        let task3 = create_test_task("Task 3", TaskStatus::Pending, vec![task2.id]);

        let mut tasks = HashMap::new();
        tasks.insert(task1.id, task1);
        tasks.insert(task2.id, task2);
        tasks.insert(task3.id, task3.clone());

        let tree = render_dependency_tree(task3.id, &tasks, 0, true, "");

        // Should render all three levels
        assert!(tree.contains("Task 1"));
        assert!(tree.contains("Task 2"));
        assert!(tree.contains("Task 3"));
    }
}
