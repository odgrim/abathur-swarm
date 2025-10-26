use abathur_cli::cli::models::{Task, TaskStatus};
use abathur_cli::cli::output::tree;
use chrono::Utc;
use std::collections::HashMap;
use uuid::Uuid;

#[test]
fn test_tree_visualization_output() {
    // Create a realistic dependency tree
    let task1 = Task {
        id: Uuid::new_v4(),
        description: "Database schema migration".to_string(),
        status: TaskStatus::Completed,
        agent_type: "db-agent".to_string(),
        priority: 10,
        base_priority: 10,
        computed_priority: 10.0,
        dependencies: vec![],
        created_at: Utc::now(),
        updated_at: Utc::now(),
        started_at: Some(Utc::now()),
        completed_at: Some(Utc::now()),
    };

    let task2 = Task {
        id: Uuid::new_v4(),
        description: "Implement repository layer".to_string(),
        status: TaskStatus::Completed,
        agent_type: "repo-agent".to_string(),
        priority: 9,
        base_priority: 9,
        computed_priority: 9.0,
        dependencies: vec![task1.id],
        created_at: Utc::now(),
        updated_at: Utc::now(),
        started_at: Some(Utc::now()),
        completed_at: Some(Utc::now()),
    };

    let task3 = Task {
        id: Uuid::new_v4(),
        description: "Create API endpoints".to_string(),
        status: TaskStatus::Running,
        agent_type: "api-agent".to_string(),
        priority: 8,
        base_priority: 8,
        computed_priority: 8.5,
        dependencies: vec![task2.id],
        created_at: Utc::now(),
        updated_at: Utc::now(),
        started_at: Some(Utc::now()),
        completed_at: None,
    };

    let task4 = Task {
        id: Uuid::new_v4(),
        description: "Write API integration tests".to_string(),
        status: TaskStatus::Ready,
        agent_type: "test-agent".to_string(),
        priority: 7,
        base_priority: 7,
        computed_priority: 7.2,
        dependencies: vec![task3.id],
        created_at: Utc::now(),
        updated_at: Utc::now(),
        started_at: None,
        completed_at: None,
    };

    let task5 = Task {
        id: Uuid::new_v4(),
        description: "Frontend integration".to_string(),
        status: TaskStatus::Blocked,
        agent_type: "frontend-agent".to_string(),
        priority: 6,
        base_priority: 6,
        computed_priority: 6.0,
        dependencies: vec![task3.id],
        created_at: Utc::now(),
        updated_at: Utc::now(),
        started_at: None,
        completed_at: None,
    };

    let task6 = Task {
        id: Uuid::new_v4(),
        description: "Deploy to staging".to_string(),
        status: TaskStatus::Pending,
        agent_type: "deploy-agent".to_string(),
        priority: 5,
        base_priority: 5,
        computed_priority: 5.0,
        dependencies: vec![task4.id, task5.id],
        created_at: Utc::now(),
        updated_at: Utc::now(),
        started_at: None,
        completed_at: None,
    };

    let mut tasks = HashMap::new();
    tasks.insert(task1.id, task1);
    tasks.insert(task2.id, task2);
    tasks.insert(task3.id, task3);
    tasks.insert(task4.id, task4);
    tasks.insert(task5.id, task5);
    tasks.insert(task6.id, task6.clone());

    // Render the tree
    let output = tree::render_dependency_tree(task6.id, &tasks, 0, true, "");

    println!("\n=== Tree Visualization Demo ===");
    println!("{}", output);
    println!("=== End Demo ===\n");

    // Verify structure
    assert!(output.contains("Deploy to staging"));
    assert!(output.contains("Write API integration tests"));
    assert!(output.contains("Frontend integration"));
    assert!(output.contains("Create API endpoints"));
    assert!(output.contains("Implement repository layer"));
    assert!(output.contains("Database schema migration"));

    // Verify Unicode box-drawing characters
    assert!(output.contains("├──") || output.contains("└──"));
    assert!(output.contains("│") || output.contains("    "));

    // Verify status icons
    assert!(output.contains("✓")); // Completed
    assert!(output.contains("⟳")); // Running
    assert!(output.contains("●")); // Ready
    assert!(output.contains("⊗")); // Blocked
    assert!(output.contains("○")); // Pending
}

#[test]
fn test_multiple_independent_trees() {
    let tree1_root = Task {
        id: Uuid::new_v4(),
        description: "Feature A - Root".to_string(),
        status: TaskStatus::Completed,
        agent_type: "agent-a".to_string(),
        priority: 5,
        base_priority: 5,
        computed_priority: 5.0,
        dependencies: vec![],
        created_at: Utc::now(),
        updated_at: Utc::now(),
        started_at: Some(Utc::now()),
        completed_at: Some(Utc::now()),
    };

    let tree2_root = Task {
        id: Uuid::new_v4(),
        description: "Feature B - Root".to_string(),
        status: TaskStatus::Running,
        agent_type: "agent-b".to_string(),
        priority: 5,
        base_priority: 5,
        computed_priority: 5.0,
        dependencies: vec![],
        created_at: Utc::now(),
        updated_at: Utc::now(),
        started_at: Some(Utc::now()),
        completed_at: None,
    };

    let mut tasks = HashMap::new();
    tasks.insert(tree1_root.id, tree1_root.clone());
    tasks.insert(tree2_root.id, tree2_root.clone());

    let output = tree::render_multiple_trees(&[tree1_root.id, tree2_root.id], &tasks);

    println!("\n=== Multiple Trees Demo ===");
    println!("{}", output);
    println!("=== End Demo ===\n");

    assert!(output.contains("Feature A - Root"));
    assert!(output.contains("Feature B - Root"));
}

#[test]
fn test_colored_status_output() {
    let statuses = vec![
        TaskStatus::Completed,
        TaskStatus::Running,
        TaskStatus::Failed,
        TaskStatus::Cancelled,
        TaskStatus::Ready,
        TaskStatus::Blocked,
        TaskStatus::Pending,
    ];

    println!("\n=== Status Icons with Colors ===");
    for status in statuses {
        let colored = tree::render_status_colored(status, true);
        let plain = tree::render_status_colored(status, false);
        println!("Status: {:?} | Colored: {} | Plain: {}", status, colored, plain);
    }
    println!("=== End Demo ===\n");
}
