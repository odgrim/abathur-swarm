//! Integration tests for the rebuilt goal-task system.
//!
//! These tests validate the new architecture where:
//! - Goals are aspirational context (never "completed", only retired)
//! - Tasks don't have goal_id; they have a `source: TaskSource` field
//! - GoalContextService infers task domains and loads relevant goals
//!
//! ## Test coverage:
//! 1. Goal creation with applicability_domains
//! 2. Task creation with TaskSource variants
//! 3. GoalContextService domain inference and goal loading
//! 4. GoalContextService context formatting
//! 5. Edge cases and additional scenarios

use std::sync::Arc;

use abathur::adapters::sqlite::{
    create_migrated_test_pool, SqliteGoalRepository, SqliteTaskRepository,
};
use abathur::domain::models::{
    Goal, GoalConstraint, GoalPriority, GoalStatus, Task, TaskPriority, TaskSource, TaskStatus,
};
use abathur::domain::ports::{GoalRepository, TaskRepository};
use abathur::services::{
    GoalContextService, GoalService, TaskService,
};

/// Set up in-memory SQLite repos with all migrations applied.
async fn setup_repos() -> (Arc<SqliteGoalRepository>, Arc<SqliteTaskRepository>) {
    let pool = create_migrated_test_pool().await.expect("Failed to create test pool");

    (
        Arc::new(SqliteGoalRepository::new(pool.clone())),
        Arc::new(SqliteTaskRepository::new(pool)),
    )
}

// =============================================================================
// 1. GOAL CREATION WITH DOMAINS
// =============================================================================

#[tokio::test]
async fn test_goal_creation_with_domains() {
    let (goal_repo, _) = setup_repos().await;
    let goal_service = GoalService::new(goal_repo.clone());

    let (goal, _events) = goal_service
        .create_goal(
            "Code should be well-tested".to_string(),
            "All production code should have comprehensive test coverage".to_string(),
            GoalPriority::High,
            None,
            vec![GoalConstraint::invariant(
                "minimum-coverage",
                "Test coverage must be at least 80%",
            )],
            vec!["testing".to_string(), "code-quality".to_string()],
        )
        .await
        .expect("Failed to create goal");

    assert_eq!(goal.name, "Code should be well-tested");
    assert_eq!(goal.status, GoalStatus::Active);
    assert_eq!(goal.priority, GoalPriority::High);
    assert_eq!(
        goal.applicability_domains,
        vec!["testing", "code-quality"]
    );
    assert_eq!(goal.constraints.len(), 1);

    // Verify retrieval
    let retrieved = goal_service
        .get_goal(goal.id)
        .await
        .expect("Failed to get goal")
        .expect("Goal not found");
    assert_eq!(retrieved.applicability_domains, goal.applicability_domains);
}

#[tokio::test]
async fn test_find_goals_by_domains() {
    let (goal_repo, _) = setup_repos().await;
    let goal_service = GoalService::new(goal_repo.clone());

    // Create goals with different domains
    let (_testing_goal, _events) = goal_service
        .create_goal(
            "Testing goal".to_string(),
            "Everything should be tested".to_string(),
            GoalPriority::Normal,
            None,
            vec![],
            vec!["testing".to_string()],
        )
        .await
        .unwrap();

    let (_security_goal, _events) = goal_service
        .create_goal(
            "Security goal".to_string(),
            "Code should be secure".to_string(),
            GoalPriority::High,
            None,
            vec![],
            vec!["security".to_string()],
        )
        .await
        .unwrap();

    let (_perf_goal, _events) = goal_service
        .create_goal(
            "Performance goal".to_string(),
            "System should be fast".to_string(),
            GoalPriority::Normal,
            None,
            vec![],
            vec!["performance".to_string()],
        )
        .await
        .unwrap();

    // Find by testing domain
    let testing_goals = goal_repo
        .find_by_domains(&["testing".to_string()])
        .await
        .unwrap();
    assert_eq!(testing_goals.len(), 1);
    assert_eq!(testing_goals[0].name, "Testing goal");

    // Find by security domain
    let security_goals = goal_repo
        .find_by_domains(&["security".to_string()])
        .await
        .unwrap();
    assert_eq!(security_goals.len(), 1);
    assert_eq!(security_goals[0].name, "Security goal");

    // Find by multiple domains - should get goals matching any
    let multi_goals = goal_repo
        .find_by_domains(&["testing".to_string(), "security".to_string()])
        .await
        .unwrap();
    assert_eq!(multi_goals.len(), 2);

    // Find by nonexistent domain
    let no_goals = goal_repo
        .find_by_domains(&["nonexistent".to_string()])
        .await
        .unwrap();
    assert!(no_goals.is_empty());
}

// =============================================================================
// 2. TASK CREATION WITH TASKSOURCE VARIANTS
// =============================================================================

#[tokio::test]
async fn test_task_creation_with_human_source() {
    let (_, task_repo) = setup_repos().await;
    let task_service = TaskService::new(task_repo.clone());

    let (task, _events) = task_service
        .submit_task(
            Some("Implement user authentication".to_string()),
            "Build login and registration endpoints".to_string(),
            None,
            TaskPriority::High,
            Some("developer".to_string()),
            vec![],
            None,
            None,
            TaskSource::Human,
        )
        .await
        .expect("Failed to submit task");

    assert_eq!(task.title, "Implement user authentication");
    assert_eq!(task.source, TaskSource::Human);
    assert_eq!(task.status, TaskStatus::Ready); // No deps -> ready
}

#[tokio::test]
async fn test_task_creation_with_system_source() {
    let (_, task_repo) = setup_repos().await;
    let task_service = TaskService::new(task_repo.clone());

    let (task, _events) = task_service
        .submit_task(
            Some("Run diagnostics".to_string()),
            "System-triggered diagnostic check".to_string(),
            None,
            TaskPriority::Normal,
            None,
            vec![],
            None,
            None,
            TaskSource::System,
        )
        .await
        .expect("Failed to submit task");

    assert_eq!(task.source, TaskSource::System);
}

#[tokio::test]
async fn test_list_tasks_by_source() {
    let (_, task_repo) = setup_repos().await;

    // Create tasks with different sources
    let human_task = Task::with_title("Human task", "Created by human")
        .with_source(TaskSource::Human);
    let system_task = Task::with_title("System task", "Created by system")
        .with_source(TaskSource::System);

    task_repo.create(&human_task).await.unwrap();
    task_repo.create(&system_task).await.unwrap();

    // Query by source type
    let human_tasks = task_repo.list_by_source("human").await.unwrap();
    assert_eq!(human_tasks.len(), 1);
    assert_eq!(human_tasks[0].title, "Human task");

    let system_tasks = task_repo.list_by_source("system").await.unwrap();
    assert_eq!(system_tasks.len(), 1);
}

// =============================================================================
// 3. GOAL CONTEXT SERVICE - DOMAIN INFERENCE
// =============================================================================

#[tokio::test]
async fn test_infer_task_domains_code_quality() {
    // Task about implementing something -> should infer code-quality
    let task = Task::with_title(
        "Implement user authentication",
        "Build login and registration endpoints with OAuth2",
    );
    let domains = GoalContextService::<SqliteGoalRepository>::infer_task_domains(&task);
    assert!(
        domains.contains(&"code-quality".to_string()),
        "Should infer code-quality for 'implement' task. Got: {:?}",
        domains
    );
}

#[tokio::test]
async fn test_infer_task_domains_testing() {
    let task = Task::with_title(
        "Write unit tests for auth module",
        "Add comprehensive test coverage for the authentication module",
    );
    let domains = GoalContextService::<SqliteGoalRepository>::infer_task_domains(&task);
    assert!(
        domains.contains(&"testing".to_string()),
        "Should infer testing domain. Got: {:?}",
        domains
    );
}

#[tokio::test]
async fn test_infer_task_domains_security() {
    let task = Task::with_title(
        "Review authentication flow",
        "Check for credential handling vulnerabilities and token management",
    );
    let domains = GoalContextService::<SqliteGoalRepository>::infer_task_domains(&task);
    assert!(
        domains.contains(&"security".to_string()),
        "Should infer security domain. Got: {:?}",
        domains
    );
}

#[tokio::test]
async fn test_infer_task_domains_performance() {
    let task = Task::with_title(
        "Optimize database queries",
        "Add caching layer and optimize slow queries for better throughput",
    );
    let domains = GoalContextService::<SqliteGoalRepository>::infer_task_domains(&task);
    assert!(
        domains.contains(&"performance".to_string()),
        "Should infer performance domain. Got: {:?}",
        domains
    );
}

#[tokio::test]
async fn test_infer_task_domains_infrastructure() {
    let task = Task::with_title(
        "Set up CI/CD pipeline",
        "Configure docker deployment with kubernetes",
    );
    let domains = GoalContextService::<SqliteGoalRepository>::infer_task_domains(&task);
    assert!(
        domains.contains(&"infrastructure".to_string()),
        "Should infer infrastructure domain. Got: {:?}",
        domains
    );
}

#[tokio::test]
async fn test_infer_task_domains_frontend() {
    let task = Task::with_title(
        "Build login component",
        "Create the UI component for the login form with proper layout",
    );
    let domains = GoalContextService::<SqliteGoalRepository>::infer_task_domains(&task);
    assert!(
        domains.contains(&"frontend".to_string()),
        "Should infer frontend domain. Got: {:?}",
        domains
    );
}

#[tokio::test]
async fn test_infer_task_domains_multiple() {
    // A task that touches multiple domains
    let task = Task::with_title(
        "Implement secure API endpoint",
        "Build a server endpoint with authentication token handling and tests",
    );
    let domains = GoalContextService::<SqliteGoalRepository>::infer_task_domains(&task);
    assert!(
        domains.contains(&"security".to_string()),
        "Should infer security. Got: {:?}",
        domains
    );
    assert!(
        domains.contains(&"backend".to_string()),
        "Should infer backend. Got: {:?}",
        domains
    );
    assert!(
        domains.contains(&"testing".to_string()),
        "Should infer testing. Got: {:?}",
        domains
    );
}

#[tokio::test]
async fn test_get_goals_for_task() {
    let (goal_repo, _) = setup_repos().await;
    let goal_service = GoalService::new(goal_repo.clone());
    let ctx_service = GoalContextService::new(goal_repo.clone());

    // Create goals with different domains
    let (_, _events) = goal_service
        .create_goal(
            "Well-tested code".to_string(),
            "All code should have tests".to_string(),
            GoalPriority::High,
            None,
            vec![],
            vec!["testing".to_string()],
        )
        .await
        .unwrap();

    let (_, _events) = goal_service
        .create_goal(
            "Secure code".to_string(),
            "No security vulnerabilities".to_string(),
            GoalPriority::Critical,
            None,
            vec![],
            vec!["security".to_string()],
        )
        .await
        .unwrap();

    let (_, _events) = goal_service
        .create_goal(
            "Fast infrastructure".to_string(),
            "Quick deployments".to_string(),
            GoalPriority::Normal,
            None,
            vec![],
            vec!["infrastructure".to_string()],
        )
        .await
        .unwrap();

    // Task about writing tests -> should get testing goal
    let test_task = Task::with_title(
        "Write unit tests for user service",
        "Add test coverage for user CRUD operations",
    );
    let goals = ctx_service.get_goals_for_task(&test_task).await.unwrap();
    assert_eq!(goals.len(), 1);
    assert_eq!(goals[0].name, "Well-tested code");

    // Task about auth -> should get security goal
    let auth_task = Task::with_title(
        "Review authentication",
        "Audit credential handling and token validation",
    );
    let goals = ctx_service.get_goals_for_task(&auth_task).await.unwrap();
    assert!(
        goals.iter().any(|g| g.name == "Secure code"),
        "Should find security goal. Got: {:?}",
        goals.iter().map(|g| &g.name).collect::<Vec<_>>()
    );

    // Task about deployment -> should get infrastructure goal
    let deploy_task = Task::with_title(
        "Configure CI/CD pipeline",
        "Set up docker and kubernetes deployment",
    );
    let goals = ctx_service.get_goals_for_task(&deploy_task).await.unwrap();
    assert!(
        goals.iter().any(|g| g.name == "Fast infrastructure"),
        "Should find infrastructure goal. Got: {:?}",
        goals.iter().map(|g| &g.name).collect::<Vec<_>>()
    );
}

#[tokio::test]
async fn test_retired_goals_not_returned() {
    let (goal_repo, _) = setup_repos().await;
    let goal_service = GoalService::new(goal_repo.clone());
    let ctx_service = GoalContextService::new(goal_repo.clone());

    let (goal, _events) = goal_service
        .create_goal(
            "Retired testing goal".to_string(),
            "This goal is retired".to_string(),
            GoalPriority::Normal,
            None,
            vec![],
            vec!["testing".to_string()],
        )
        .await
        .unwrap();

    // Retire the goal
    let (_, _events) = goal_service
        .transition_status(goal.id, GoalStatus::Retired)
        .await
        .unwrap();

    // A testing task should not pick up the retired goal
    let task = Task::with_title("Write tests", "Add test coverage");
    let goals = ctx_service.get_goals_for_task(&task).await.unwrap();
    assert!(
        goals.is_empty(),
        "Retired goals should not be returned. Got: {:?}",
        goals.iter().map(|g| &g.name).collect::<Vec<_>>()
    );
}

// =============================================================================
// 4. GOAL CONTEXT SERVICE - FORMATTING
// =============================================================================

#[tokio::test]
async fn test_format_goal_context_empty() {
    let output = GoalContextService::<SqliteGoalRepository>::format_goal_context(&[]);
    assert!(output.is_empty());
}

#[tokio::test]
async fn test_format_goal_context_with_constraints() {
    let goal = Goal::new("Well-tested code", "All code should have comprehensive tests")
        .with_priority(GoalPriority::High)
        .with_constraint(GoalConstraint::invariant(
            "min-coverage",
            "Test coverage must be at least 80%",
        ))
        .with_constraint(GoalConstraint::preference(
            "test-style",
            "Prefer property-based tests where applicable",
        ));

    let output =
        GoalContextService::<SqliteGoalRepository>::format_goal_context(&[goal]);

    // Verify structure
    assert!(output.contains("## Guiding Goals"), "Should have header");
    assert!(
        output.contains("### Well-tested code"),
        "Should have goal name"
    );
    assert!(
        output.contains("All code should have comprehensive tests"),
        "Should have description"
    );
    assert!(output.contains("Constraints:"), "Should have constraints section");
    assert!(
        output.contains("min-coverage"),
        "Should list constraint names"
    );
    assert!(
        output.contains("Test coverage must be at least 80%"),
        "Should list constraint descriptions"
    );
}

#[tokio::test]
async fn test_collect_constraints_deduplicates() {
    let goal1 = Goal::new("Goal 1", "First goal")
        .with_constraint(GoalConstraint::invariant("safety", "Must be safe"))
        .with_constraint(GoalConstraint::preference("style", "Clean code"));

    let goal2 = Goal::new("Goal 2", "Second goal")
        .with_constraint(GoalConstraint::invariant("safety", "Must be safe")) // duplicate name
        .with_constraint(GoalConstraint::boundary("budget", "Under $100"));

    let constraints =
        GoalContextService::<SqliteGoalRepository>::collect_constraints(&[goal1, goal2]);

    // "safety" appears in both goals but should be deduplicated by name
    assert_eq!(constraints.len(), 3); // safety, style, budget
    let names: Vec<&str> = constraints.iter().map(|c| c.name.as_str()).collect();
    assert!(names.contains(&"safety"));
    assert!(names.contains(&"style"));
    assert!(names.contains(&"budget"));
}


// =============================================================================
// 7. EDGE CASES AND ADDITIONAL SCENARIOS
// =============================================================================

#[tokio::test]
async fn test_goal_with_no_domains_matches_nothing() {
    let (goal_repo, _) = setup_repos().await;
    let ctx_service = GoalContextService::new(goal_repo.clone());

    // Create a goal with no applicability domains
    let goal = Goal::new("Vague aspiration", "Be great at everything");
    goal_repo.create(&goal).await.unwrap();

    // A task should not pick this up through domain matching
    let task = Task::with_title("Write some code", "Implement a feature");
    let goals = ctx_service.get_goals_for_task(&task).await.unwrap();
    // find_by_domains only returns goals with matching domains
    assert!(
        goals.is_empty(),
        "Goal with no domains should not match via domain search"
    );
}

#[tokio::test]
async fn test_subtask_source_persisted() {
    let (_, task_repo) = setup_repos().await;

    let parent_id = uuid::Uuid::new_v4();
    let task = Task::with_title("Subtask", "A child task")
        .with_source(TaskSource::SubtaskOf(parent_id));

    task_repo.create(&task).await.unwrap();

    let retrieved = task_repo.get(task.id).await.unwrap().unwrap();
    assert_eq!(retrieved.source, TaskSource::SubtaskOf(parent_id));
}

#[tokio::test]
async fn test_multiple_goals_same_domain() {
    let (goal_repo, _) = setup_repos().await;
    let ctx_service = GoalContextService::new(goal_repo.clone());

    // Create multiple goals in the same domain
    let goal1 = Goal::new("Coverage goal", "High test coverage")
        .with_applicability_domain("testing");
    goal_repo.create(&goal1).await.unwrap();

    let goal2 = Goal::new("Speed goal", "Fast test execution")
        .with_applicability_domain("testing");
    goal_repo.create(&goal2).await.unwrap();

    // A testing task should pick up both goals
    let task = Task::with_title("Write tests", "Add comprehensive test suite");
    let goals = ctx_service.get_goals_for_task(&task).await.unwrap();
    assert_eq!(goals.len(), 2, "Should match both testing goals");
}

