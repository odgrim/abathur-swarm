mod helpers;

use abathur::domain::models::{DependencyType, Task, TaskSource, TaskStatus};
use abathur::domain::ports::task_repository::{TaskFilters, TaskRepository};
use abathur::infrastructure::database::TaskRepositoryImpl;
use chrono::Utc;
use serde_json::json;
use uuid::Uuid;

use helpers::database::{setup_test_db, teardown_test_db};

fn create_test_task(summary: &str, status: TaskStatus) -> Task {
    Task {
        id: Uuid::new_v4(),
        summary: summary.to_string(),
        description: "Test task description".to_string(),
        agent_type: "test-agent".to_string(),
        priority: 5,
        calculated_priority: 5.0,
        status,
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
    }
}

#[tokio::test]
async fn test_insert_and_get_task() {
    let pool = setup_test_db().await;
    let repo = TaskRepositoryImpl::new(pool.clone());

    let task = create_test_task("Test Task 1", TaskStatus::Pending);
    let task_id = task.id;

    // Insert task
    repo.insert(&task)
        .await
        .expect("failed to insert task");

    // Get task
    let retrieved = repo.get(task_id).await.expect("failed to get task");
    assert!(retrieved.is_some());

    let retrieved = retrieved.unwrap();
    assert_eq!(retrieved.id, task_id);
    assert_eq!(retrieved.summary, "Test Task 1");
    assert_eq!(retrieved.status, TaskStatus::Pending);
    assert_eq!(retrieved.agent_type, "test-agent");
    assert_eq!(retrieved.priority, 5);

    teardown_test_db(pool).await;
}

#[tokio::test]
async fn test_get_nonexistent_task() {
    let pool = setup_test_db().await;
    let repo = TaskRepositoryImpl::new(pool.clone());

    let nonexistent_id = Uuid::new_v4();
    let result = repo.get(nonexistent_id).await.expect("failed to query");
    assert!(result.is_none());

    teardown_test_db(pool).await;
}

#[tokio::test]
async fn test_update_task() {
    let pool = setup_test_db().await;
    let repo = TaskRepositoryImpl::new(pool.clone());

    let mut task = create_test_task("Original Summary", TaskStatus::Pending);
    repo.insert(&task)
        .await
        .expect("failed to insert task");

    // Update task
    task.summary = "Updated Summary".to_string();
    task.priority = 8;
    task.status = TaskStatus::Running;

    repo.update(&task)
        .await
        .expect("failed to update task");

    // Verify update
    let retrieved = repo.get(task.id).await.expect("failed to get task").unwrap();
    assert_eq!(retrieved.summary, "Updated Summary");
    assert_eq!(retrieved.priority, 8);
    assert_eq!(retrieved.status, TaskStatus::Running);

    teardown_test_db(pool).await;
}

#[tokio::test]
async fn test_delete_task() {
    let pool = setup_test_db().await;
    let repo = TaskRepositoryImpl::new(pool.clone());

    let task = create_test_task("Task to Delete", TaskStatus::Pending);
    let task_id = task.id;

    repo.insert(&task)
        .await
        .expect("failed to insert task");

    // Delete task
    repo.delete(task_id)
        .await
        .expect("failed to delete task");

    // Verify deletion
    let result = repo.get(task_id).await.expect("failed to query");
    assert!(result.is_none());

    teardown_test_db(pool).await;
}

#[tokio::test]
async fn test_list_tasks_with_filters() {
    let pool = setup_test_db().await;
    let repo = TaskRepositoryImpl::new(pool.clone());

    // Insert multiple tasks
    let task1 = create_test_task("Pending Task 1", TaskStatus::Pending);
    let task2 = create_test_task("Pending Task 2", TaskStatus::Pending);
    let task3 = create_test_task("Running Task", TaskStatus::Running);
    let task4 = create_test_task("Completed Task", TaskStatus::Completed);

    repo.insert(&task1).await.expect("failed to insert task1");
    repo.insert(&task2).await.expect("failed to insert task2");
    repo.insert(&task3).await.expect("failed to insert task3");
    repo.insert(&task4).await.expect("failed to insert task4");

    // Filter by status
    let filters = TaskFilters {
        status: Some(TaskStatus::Pending),
        ..Default::default()
    };

    let pending_tasks = repo.list(&filters).await.expect("failed to list tasks");
    assert_eq!(pending_tasks.len(), 2);

    // Filter by running status
    let filters = TaskFilters {
        status: Some(TaskStatus::Running),
        ..Default::default()
    };

    let running_tasks = repo.list(&filters).await.expect("failed to list tasks");
    assert_eq!(running_tasks.len(), 1);
    assert_eq!(running_tasks[0].summary, "Running Task");

    teardown_test_db(pool).await;
}

#[tokio::test]
async fn test_count_tasks() {
    let pool = setup_test_db().await;
    let repo = TaskRepositoryImpl::new(pool.clone());

    // Insert multiple tasks
    for i in 0..5 {
        let task = create_test_task(&format!("Task {}", i), TaskStatus::Pending);
        repo.insert(&task).await.expect("failed to insert task");
    }

    for i in 0..3 {
        let task = create_test_task(&format!("Running Task {}", i), TaskStatus::Running);
        repo.insert(&task).await.expect("failed to insert task");
    }

    // Count all tasks
    let filters = TaskFilters::default();
    let total = repo.count(&filters).await.expect("failed to count tasks");
    assert_eq!(total, 8);

    // Count pending tasks
    let filters = TaskFilters {
        status: Some(TaskStatus::Pending),
        ..Default::default()
    };
    let pending_count = repo.count(&filters).await.expect("failed to count");
    assert_eq!(pending_count, 5);

    teardown_test_db(pool).await;
}

#[tokio::test]
async fn test_get_ready_tasks() {
    let pool = setup_test_db().await;
    let repo = TaskRepositoryImpl::new(pool.clone());

    // Insert tasks with different statuses and priorities
    let mut task1 = create_test_task("Ready Task 1", TaskStatus::Ready);
    task1.calculated_priority = 10.0;

    let mut task2 = create_test_task("Ready Task 2", TaskStatus::Ready);
    task2.calculated_priority = 5.0;

    let mut task3 = create_test_task("Ready Task 3", TaskStatus::Ready);
    task3.calculated_priority = 8.0;

    let task4 = create_test_task("Pending Task", TaskStatus::Pending);

    repo.insert(&task1).await.expect("failed to insert task1");
    repo.insert(&task2).await.expect("failed to insert task2");
    repo.insert(&task3).await.expect("failed to insert task3");
    repo.insert(&task4).await.expect("failed to insert task4");

    // Get ready tasks (should be ordered by priority)
    let ready_tasks = repo.get_ready_tasks(10).await.expect("failed to get ready tasks");

    assert_eq!(ready_tasks.len(), 3);
    assert_eq!(ready_tasks[0].summary, "Ready Task 1"); // Highest priority
    assert_eq!(ready_tasks[1].summary, "Ready Task 3");
    assert_eq!(ready_tasks[2].summary, "Ready Task 2"); // Lowest priority

    teardown_test_db(pool).await;
}

#[tokio::test]
async fn test_update_status() {
    let pool = setup_test_db().await;
    let repo = TaskRepositoryImpl::new(pool.clone());

    let task = create_test_task("Test Task", TaskStatus::Pending);
    let task_id = task.id;

    repo.insert(&task)
        .await
        .expect("failed to insert task");

    // Update status
    repo.update_status(task_id, TaskStatus::Running)
        .await
        .expect("failed to update status");

    // Verify status update
    let retrieved = repo.get(task_id).await.expect("failed to get task").unwrap();
    assert_eq!(retrieved.status, TaskStatus::Running);

    teardown_test_db(pool).await;
}

#[tokio::test]
async fn test_get_by_feature_branch() {
    let pool = setup_test_db().await;
    let repo = TaskRepositoryImpl::new(pool.clone());

    let mut task1 = create_test_task("Feature Task 1", TaskStatus::Pending);
    task1.feature_branch = Some("feature/new-feature".to_string());

    let mut task2 = create_test_task("Feature Task 2", TaskStatus::Running);
    task2.feature_branch = Some("feature/new-feature".to_string());

    let mut task3 = create_test_task("Other Task", TaskStatus::Pending);
    task3.feature_branch = Some("feature/other".to_string());

    repo.insert(&task1).await.expect("failed to insert task1");
    repo.insert(&task2).await.expect("failed to insert task2");
    repo.insert(&task3).await.expect("failed to insert task3");

    // Get tasks by feature branch
    let feature_tasks = repo
        .get_by_feature_branch("feature/new-feature")
        .await
        .expect("failed to get tasks by feature branch");

    assert_eq!(feature_tasks.len(), 2);

    teardown_test_db(pool).await;
}

#[tokio::test]
async fn test_get_dependents() {
    let pool = setup_test_db().await;
    let repo = TaskRepositoryImpl::new(pool.clone());

    let dependency_task = create_test_task("Dependency Task", TaskStatus::Completed);
    let dependency_id = dependency_task.id;

    let mut dependent_task1 = create_test_task("Dependent Task 1", TaskStatus::Pending);
    dependent_task1.dependencies = Some(vec![dependency_id]);

    let mut dependent_task2 = create_test_task("Dependent Task 2", TaskStatus::Pending);
    dependent_task2.dependencies = Some(vec![dependency_id]);

    let independent_task = create_test_task("Independent Task", TaskStatus::Pending);

    repo.insert(&dependency_task).await.expect("failed to insert dependency");
    repo.insert(&dependent_task1).await.expect("failed to insert dependent1");
    repo.insert(&dependent_task2).await.expect("failed to insert dependent2");
    repo.insert(&independent_task).await.expect("failed to insert independent");

    // Get dependents
    let dependents = repo
        .get_dependents(dependency_id)
        .await
        .expect("failed to get dependents");

    assert_eq!(dependents.len(), 2);

    teardown_test_db(pool).await;
}

#[tokio::test]
async fn test_get_by_session() {
    let pool = setup_test_db().await;
    let repo = TaskRepositoryImpl::new(pool.clone());

    let session_id = Uuid::new_v4();

    let mut task1 = create_test_task("Session Task 1", TaskStatus::Pending);
    task1.session_id = Some(session_id);

    let mut task2 = create_test_task("Session Task 2", TaskStatus::Running);
    task2.session_id = Some(session_id);

    let task3 = create_test_task("Other Task", TaskStatus::Pending);

    repo.insert(&task1).await.expect("failed to insert task1");
    repo.insert(&task2).await.expect("failed to insert task2");
    repo.insert(&task3).await.expect("failed to insert task3");

    // Get tasks by session
    let session_tasks = repo
        .get_by_session(session_id)
        .await
        .expect("failed to get tasks by session");

    assert_eq!(session_tasks.len(), 2);

    teardown_test_db(pool).await;
}

#[tokio::test]
async fn test_get_by_parent() {
    let pool = setup_test_db().await;
    let repo = TaskRepositoryImpl::new(pool.clone());

    let parent_task = create_test_task("Parent Task", TaskStatus::Running);
    let parent_id = parent_task.id;

    let mut child_task1 = create_test_task("Child Task 1", TaskStatus::Pending);
    child_task1.parent_task_id = Some(parent_id);

    let mut child_task2 = create_test_task("Child Task 2", TaskStatus::Pending);
    child_task2.parent_task_id = Some(parent_id);

    let independent_task = create_test_task("Independent Task", TaskStatus::Pending);

    repo.insert(&parent_task).await.expect("failed to insert parent");
    repo.insert(&child_task1).await.expect("failed to insert child1");
    repo.insert(&child_task2).await.expect("failed to insert child2");
    repo.insert(&independent_task).await.expect("failed to insert independent");

    // Get child tasks
    let children = repo
        .get_by_parent(parent_id)
        .await
        .expect("failed to get children");

    assert_eq!(children.len(), 2);

    teardown_test_db(pool).await;
}

#[tokio::test]
async fn test_task_with_input_and_result_data() {
    let pool = setup_test_db().await;
    let repo = TaskRepositoryImpl::new(pool.clone());

    let mut task = create_test_task("Data Task", TaskStatus::Running);
    task.input_data = Some(json!({"input": "test data"}));
    task.result_data = Some(json!({"output": "test result"}));

    repo.insert(&task)
        .await
        .expect("failed to insert task");

    let retrieved = repo.get(task.id).await.expect("failed to get task").unwrap();
    assert_eq!(retrieved.input_data, Some(json!({"input": "test data"})));
    assert_eq!(retrieved.result_data, Some(json!({"output": "test result"})));

    teardown_test_db(pool).await;
}

#[tokio::test]
async fn test_pagination() {
    let pool = setup_test_db().await;
    let repo = TaskRepositoryImpl::new(pool.clone());

    // Insert 10 tasks
    for i in 0..10 {
        let task = create_test_task(&format!("Task {}", i), TaskStatus::Pending);
        repo.insert(&task).await.expect("failed to insert task");
    }

    // Get first page
    let filters = TaskFilters {
        status: Some(TaskStatus::Pending),
        limit: Some(5),
        offset: Some(0),
        ..Default::default()
    };

    let page1 = repo.list(&filters).await.expect("failed to list tasks");
    assert_eq!(page1.len(), 5);

    // Get second page
    let filters = TaskFilters {
        status: Some(TaskStatus::Pending),
        limit: Some(5),
        offset: Some(5),
        ..Default::default()
    };

    let page2 = repo.list(&filters).await.expect("failed to list tasks");
    assert_eq!(page2.len(), 5);

    teardown_test_db(pool).await;
}
