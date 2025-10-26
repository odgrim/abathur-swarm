use abathur::domain::models::{Task, TaskSource, TaskStatus};
use abathur::domain::ports::{TaskFilters, TaskRepository};
use abathur::infrastructure::database::{DatabaseConnection, TaskRepositoryImpl};
use chrono::Utc;
use serde_json::json;
use sqlx::SqlitePool;
use uuid::Uuid;

// Test helper functions
async fn setup_test_db() -> SqlitePool {
    let db = DatabaseConnection::new("sqlite::memory:")
        .await
        .expect("Failed to create test database");
    db.migrate().await.expect("Failed to run migrations");
    db.pool().clone()
}

async fn teardown_test_db(pool: SqlitePool) {
    pool.close().await;
}

#[tokio::test]
async fn test_insert_and_get_task() {
    let pool = setup_test_db().await;
    let repo = TaskRepositoryImpl::new(pool.clone());

    let task = Task::new("Test task".to_string(), "Test description".to_string());

    repo.insert(&task).await.expect("Failed to insert task");

    let retrieved = repo.get(task.id).await.expect("Failed to get task");
    assert!(retrieved.is_some());
    let retrieved = retrieved.unwrap();
    assert_eq!(retrieved.id, task.id);
    assert_eq!(retrieved.summary, task.summary);
    assert_eq!(retrieved.description, task.description);
    assert_eq!(retrieved.status, TaskStatus::Pending);

    teardown_test_db(pool).await;
}

#[tokio::test]
async fn test_insert_task_with_all_fields() {
    let pool = setup_test_db().await;
    let repo = TaskRepositoryImpl::new(pool.clone());

    let now = Utc::now();
    let task = Task {
        id: Uuid::new_v4(),
        summary: "Complex task".to_string(),
        description: "Complex description".to_string(),
        agent_type: "test-agent".to_string(),
        priority: 8,
        calculated_priority: 8.5,
        status: TaskStatus::Ready,
        dependencies: Some(vec![Uuid::new_v4(), Uuid::new_v4()]),
        dependency_type: abathur::DependencyType::Parallel,
        dependency_depth: 2,
        input_data: Some(json!({"key": "value"})),
        result_data: Some(json!({"result": "success"})),
        error_message: Some("Test error".to_string()),
        retry_count: 1,
        max_retries: 5,
        max_execution_timeout_seconds: 7200,
        submitted_at: now,
        started_at: Some(now),
        completed_at: None,
        last_updated_at: now,
        created_by: Some("test-user".to_string()),
        parent_task_id: Some(Uuid::new_v4()),
        session_id: Some(Uuid::new_v4()),
        source: TaskSource::AgentPlanner,
        deadline: Some(now),
        estimated_duration_seconds: Some(3600),
        feature_branch: Some("feature/test".to_string()),
        task_branch: Some("task/test-123".to_string()),
        worktree_path: Some("/path/to/worktree".to_string()),
    };

    repo.insert(&task).await.expect("Failed to insert task");

    let retrieved = repo
        .get(task.id)
        .await
        .expect("Failed to get task")
        .unwrap();
    assert_eq!(retrieved.id, task.id);
    assert_eq!(retrieved.summary, task.summary);
    assert_eq!(retrieved.priority, 8);
    assert_eq!(retrieved.calculated_priority, 8.5);
    assert_eq!(retrieved.status, TaskStatus::Ready);
    assert_eq!(retrieved.dependencies, task.dependencies);
    assert_eq!(retrieved.dependency_type, abathur::DependencyType::Parallel);
    assert_eq!(retrieved.dependency_depth, 2);
    assert_eq!(retrieved.source, TaskSource::AgentPlanner);
    assert_eq!(retrieved.feature_branch, Some("feature/test".to_string()));

    teardown_test_db(pool).await;
}

#[tokio::test]
async fn test_update_task() {
    let pool = setup_test_db().await;
    let repo = TaskRepositoryImpl::new(pool.clone());

    let mut task = Task::new("Test task".to_string(), "Test description".to_string());
    repo.insert(&task).await.expect("Failed to insert");

    task.summary = "Updated summary".to_string();
    task.status = TaskStatus::Running;
    task.started_at = Some(Utc::now());
    repo.update(&task).await.expect("Failed to update");

    let updated = repo.get(task.id).await.expect("Failed to get").unwrap();
    assert_eq!(updated.summary, "Updated summary");
    assert_eq!(updated.status, TaskStatus::Running);
    assert!(updated.started_at.is_some());

    teardown_test_db(pool).await;
}

#[tokio::test]
async fn test_delete_task() {
    let pool = setup_test_db().await;
    let repo = TaskRepositoryImpl::new(pool.clone());

    let task = Task::new("Test task".to_string(), "Test description".to_string());
    repo.insert(&task).await.expect("Failed to insert");

    repo.delete(task.id).await.expect("Failed to delete");

    let deleted = repo.get(task.id).await.expect("Failed to get");
    assert!(deleted.is_none());

    teardown_test_db(pool).await;
}

#[tokio::test]
async fn test_list_tasks_with_status_filter() {
    let pool = setup_test_db().await;
    let repo = TaskRepositoryImpl::new(pool.clone());

    // Insert multiple tasks with different statuses
    for i in 0..5 {
        let mut task = Task::new(format!("Task {}", i), format!("Description {}", i));
        task.priority = i as u8;
        task.calculated_priority = i as f64;
        task.status = if i % 2 == 0 {
            TaskStatus::Ready
        } else {
            TaskStatus::Pending
        };
        repo.insert(&task).await.expect("Failed to insert");
    }

    let filters = TaskFilters {
        status: Some(TaskStatus::Ready),
        ..Default::default()
    };

    let tasks = repo.list(filters).await.expect("Failed to list tasks");
    assert_eq!(tasks.len(), 3); // Tasks 0, 2, 4
    for task in &tasks {
        assert_eq!(task.status, TaskStatus::Ready);
    }

    teardown_test_db(pool).await;
}

#[tokio::test]
async fn test_list_tasks_with_agent_type_filter() {
    let pool = setup_test_db().await;
    let repo = TaskRepositoryImpl::new(pool.clone());

    for i in 0..5 {
        let mut task = Task::new(format!("Task {}", i), format!("Description {}", i));
        task.agent_type = if i < 3 {
            "agent-a".to_string()
        } else {
            "agent-b".to_string()
        };
        repo.insert(&task).await.expect("Failed to insert");
    }

    let filters = TaskFilters {
        agent_type: Some("agent-a".to_string()),
        ..Default::default()
    };

    let tasks = repo.list(filters).await.expect("Failed to list tasks");
    assert_eq!(tasks.len(), 3);
    for task in &tasks {
        assert_eq!(task.agent_type, "agent-a");
    }

    teardown_test_db(pool).await;
}

#[tokio::test]
async fn test_list_tasks_with_limit() {
    let pool = setup_test_db().await;
    let repo = TaskRepositoryImpl::new(pool.clone());

    for i in 0..10 {
        let task = Task::new(format!("Task {}", i), format!("Description {}", i));
        repo.insert(&task).await.expect("Failed to insert");
    }

    let filters = TaskFilters {
        limit: Some(5),
        ..Default::default()
    };

    let tasks = repo.list(filters).await.expect("Failed to list tasks");
    assert_eq!(tasks.len(), 5);

    teardown_test_db(pool).await;
}

#[tokio::test]
async fn test_list_tasks_ordered_by_priority() {
    let pool = setup_test_db().await;
    let repo = TaskRepositoryImpl::new(pool.clone());

    // Insert tasks with different priorities (not in order)
    let priorities = [3, 7, 1, 9, 5];
    for (i, priority) in priorities.iter().enumerate() {
        let mut task = Task::new(format!("Task {}", i), format!("Description {}", i));
        task.priority = *priority;
        task.calculated_priority = *priority as f64;
        repo.insert(&task).await.expect("Failed to insert");
    }

    let tasks = repo
        .list(TaskFilters::default())
        .await
        .expect("Failed to list tasks");

    // Verify descending order by calculated_priority
    for i in 1..tasks.len() {
        assert!(tasks[i - 1].calculated_priority >= tasks[i].calculated_priority);
    }

    teardown_test_db(pool).await;
}

#[tokio::test]
async fn test_get_ready_tasks() {
    let pool = setup_test_db().await;
    let repo = TaskRepositoryImpl::new(pool.clone());

    for i in 0..5 {
        let mut task = Task::new(format!("Task {}", i), format!("Description {}", i));
        task.priority = i as u8;
        task.calculated_priority = i as f64;
        task.status = if i >= 2 {
            TaskStatus::Ready
        } else {
            TaskStatus::Pending
        };
        repo.insert(&task).await.expect("Failed to insert");
    }

    let ready_tasks = repo
        .get_ready_tasks(10)
        .await
        .expect("Failed to get ready tasks");
    assert_eq!(ready_tasks.len(), 3); // Tasks 2, 3, 4

    // Verify they're all in Ready status
    for task in &ready_tasks {
        assert_eq!(task.status, TaskStatus::Ready);
    }

    // Verify they're ordered by priority descending
    for i in 1..ready_tasks.len() {
        assert!(ready_tasks[i - 1].calculated_priority >= ready_tasks[i].calculated_priority);
    }

    teardown_test_db(pool).await;
}

#[tokio::test]
async fn test_get_ready_tasks_respects_limit() {
    let pool = setup_test_db().await;
    let repo = TaskRepositoryImpl::new(pool.clone());

    for i in 0..10 {
        let mut task = Task::new(format!("Task {}", i), format!("Description {}", i));
        task.status = TaskStatus::Ready;
        repo.insert(&task).await.expect("Failed to insert");
    }

    let ready_tasks = repo
        .get_ready_tasks(5)
        .await
        .expect("Failed to get ready tasks");
    assert_eq!(ready_tasks.len(), 5);

    teardown_test_db(pool).await;
}

#[tokio::test]
async fn test_count_tasks() {
    let pool = setup_test_db().await;
    let repo = TaskRepositoryImpl::new(pool.clone());

    for i in 0..7 {
        let task = Task::new(format!("Task {}", i), format!("Description {}", i));
        repo.insert(&task).await.expect("Failed to insert");
    }

    let count = repo
        .count(TaskFilters::default())
        .await
        .expect("Failed to count tasks");
    assert_eq!(count, 7);

    teardown_test_db(pool).await;
}

#[tokio::test]
async fn test_count_tasks_with_filter() {
    let pool = setup_test_db().await;
    let repo = TaskRepositoryImpl::new(pool.clone());

    for i in 0..5 {
        let mut task = Task::new(format!("Task {}", i), format!("Description {}", i));
        task.status = if i < 3 {
            TaskStatus::Ready
        } else {
            TaskStatus::Pending
        };
        repo.insert(&task).await.expect("Failed to insert");
    }

    let count = repo
        .count(TaskFilters {
            status: Some(TaskStatus::Ready),
            ..Default::default()
        })
        .await
        .expect("Failed to count tasks");
    assert_eq!(count, 3);

    teardown_test_db(pool).await;
}

#[tokio::test]
async fn test_update_status() {
    let pool = setup_test_db().await;
    let repo = TaskRepositoryImpl::new(pool.clone());

    let task = Task::new("Test task".to_string(), "Test description".to_string());
    repo.insert(&task).await.expect("Failed to insert");

    repo.update_status(task.id, TaskStatus::Running)
        .await
        .expect("Failed to update status");

    let updated = repo.get(task.id).await.expect("Failed to get").unwrap();
    assert_eq!(updated.status, TaskStatus::Running);

    teardown_test_db(pool).await;
}

#[tokio::test]
async fn test_get_by_feature_branch() {
    let pool = setup_test_db().await;
    let repo = TaskRepositoryImpl::new(pool.clone());

    for i in 0..5 {
        let mut task = Task::new(format!("Task {}", i), format!("Description {}", i));
        task.feature_branch = if i < 3 {
            Some("feature/test-1".to_string())
        } else {
            Some("feature/test-2".to_string())
        };
        repo.insert(&task).await.expect("Failed to insert");
    }

    let tasks = repo
        .get_by_feature_branch("feature/test-1")
        .await
        .expect("Failed to get tasks");
    assert_eq!(tasks.len(), 3);
    for task in &tasks {
        assert_eq!(task.feature_branch, Some("feature/test-1".to_string()));
    }

    teardown_test_db(pool).await;
}

#[tokio::test]
async fn test_get_by_parent() {
    let pool = setup_test_db().await;
    let repo = TaskRepositoryImpl::new(pool.clone());

    let parent_id = Uuid::new_v4();

    for i in 0..5 {
        let mut task = Task::new(format!("Task {}", i), format!("Description {}", i));
        task.parent_task_id = if i < 3 { Some(parent_id) } else { None };
        repo.insert(&task).await.expect("Failed to insert");
    }

    let tasks = repo
        .get_by_parent(parent_id)
        .await
        .expect("Failed to get tasks");
    assert_eq!(tasks.len(), 3);
    for task in &tasks {
        assert_eq!(task.parent_task_id, Some(parent_id));
    }

    teardown_test_db(pool).await;
}

#[tokio::test]
async fn test_json_serialization() {
    let pool = setup_test_db().await;
    let repo = TaskRepositoryImpl::new(pool.clone());

    let mut task = Task::new("Test task".to_string(), "Test description".to_string());
    task.input_data = Some(json!({
        "key": "value",
        "nested": {
            "array": [1, 2, 3]
        }
    }));
    task.dependencies = Some(vec![Uuid::new_v4(), Uuid::new_v4()]);

    repo.insert(&task).await.expect("Failed to insert");

    let retrieved = repo.get(task.id).await.expect("Failed to get").unwrap();
    assert_eq!(retrieved.input_data, task.input_data);
    assert_eq!(retrieved.dependencies, task.dependencies);

    teardown_test_db(pool).await;
}

#[tokio::test]
async fn test_summary_length_validation() {
    let task = Task::new("a".repeat(140), "Test description".to_string());
    assert!(task.validate_summary().is_ok());

    let task = Task::new("a".repeat(141), "Test description".to_string());
    assert!(task.validate_summary().is_err());
}

#[tokio::test]
async fn test_priority_validation() {
    let mut task = Task::new("Test".to_string(), "Test description".to_string());
    task.priority = 10;
    assert!(task.validate_priority().is_ok());

    task.priority = 11;
    assert!(task.validate_priority().is_err());
}
