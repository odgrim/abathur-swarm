//! Integration tests for the rebuilt goal-task system.
//!
//! These tests validate the new architecture where:
//! - Goals are aspirational context (never "completed", only retired)
//! - Tasks don't have goal_id; they have a `source: TaskSource` field
//! - GoalContextService infers task domains and loads relevant goals
//! - GoalEvaluationService evaluates goals and creates corrective tasks
//!
//! ## Test coverage:
//! 1. Goal creation with applicability_domains and evaluation_criteria
//! 2. Task creation with TaskSource variants
//! 3. GoalContextService domain inference and goal loading
//! 4. GoalContextService context formatting
//! 5. GoalEvaluationService evaluation cycle
//! 6. Full lifecycle simulation

use std::sync::Arc;

use abathur::adapters::sqlite::{
    all_embedded_migrations, create_test_pool, Migrator, SqliteGoalRepository,
    SqliteTaskRepository,
};
use abathur::domain::models::{
    Goal, GoalConstraint, GoalPriority, GoalStatus, Task, TaskPriority, TaskSource, TaskStatus,
};
use abathur::domain::ports::{GoalRepository, TaskRepository};
use abathur::services::{
    GoalContextService, GoalEvaluationService, GoalService, SatisfactionLevel, TaskService,
};

/// Set up in-memory SQLite repos with all migrations applied.
async fn setup_repos() -> (Arc<SqliteGoalRepository>, Arc<SqliteTaskRepository>) {
    let pool = create_test_pool().await.expect("Failed to create test pool");
    let migrator = Migrator::new(pool.clone());
    migrator
        .run_embedded_migrations(all_embedded_migrations())
        .await
        .expect("Failed to run migrations");

    (
        Arc::new(SqliteGoalRepository::new(pool.clone())),
        Arc::new(SqliteTaskRepository::new(pool)),
    )
}

// =============================================================================
// 1. GOAL CREATION WITH DOMAINS AND EVALUATION CRITERIA
// =============================================================================

#[tokio::test]
async fn test_goal_creation_with_domains_and_criteria() {
    let (goal_repo, _) = setup_repos().await;
    let goal_service = GoalService::new(goal_repo.clone());

    let goal = goal_service
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
            vec![
                "All modules have unit tests".to_string(),
                "Integration tests cover critical paths".to_string(),
                "Test coverage exceeds 80%".to_string(),
            ],
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
    assert_eq!(goal.evaluation_criteria.len(), 3);
    assert_eq!(goal.constraints.len(), 1);

    // Verify retrieval
    let retrieved = goal_service
        .get_goal(goal.id)
        .await
        .expect("Failed to get goal")
        .expect("Goal not found");
    assert_eq!(retrieved.applicability_domains, goal.applicability_domains);
    assert_eq!(retrieved.evaluation_criteria, goal.evaluation_criteria);
}

#[tokio::test]
async fn test_find_goals_by_domains() {
    let (goal_repo, _) = setup_repos().await;
    let goal_service = GoalService::new(goal_repo.clone());

    // Create goals with different domains
    let _testing_goal = goal_service
        .create_goal(
            "Testing goal".to_string(),
            "Everything should be tested".to_string(),
            GoalPriority::Normal,
            None,
            vec![],
            vec!["testing".to_string()],
            vec!["All code has tests".to_string()],
        )
        .await
        .unwrap();

    let _security_goal = goal_service
        .create_goal(
            "Security goal".to_string(),
            "Code should be secure".to_string(),
            GoalPriority::High,
            None,
            vec![],
            vec!["security".to_string()],
            vec!["No SQL injection".to_string()],
        )
        .await
        .unwrap();

    let _perf_goal = goal_service
        .create_goal(
            "Performance goal".to_string(),
            "System should be fast".to_string(),
            GoalPriority::Normal,
            None,
            vec![],
            vec!["performance".to_string()],
            vec!["Response time < 100ms".to_string()],
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

    let task = task_service
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

    let task = task_service
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
async fn test_task_creation_with_goal_evaluation_source() {
    let (goal_repo, task_repo) = setup_repos().await;
    let task_service = TaskService::new(task_repo.clone());

    // Create a goal first (to have a valid goal_id)
    let goal = Goal::new("Test coverage", "Ensure test coverage");
    goal_repo.create(&goal).await.unwrap();

    let task = task_service
        .submit_task(
            Some("Add tests for auth module".to_string()),
            "Corrective task from goal evaluation".to_string(),
            None,
            TaskPriority::Normal,
            None,
            vec![],
            None,
            None,
            TaskSource::GoalEvaluation(goal.id),
        )
        .await
        .expect("Failed to submit task");

    assert_eq!(task.source, TaskSource::GoalEvaluation(goal.id));

    // Verify persistence: retrieve and check source
    let retrieved = task_repo
        .get(task.id)
        .await
        .unwrap()
        .expect("Task not found");
    assert_eq!(retrieved.source, TaskSource::GoalEvaluation(goal.id));
}

#[tokio::test]
async fn test_list_tasks_by_source() {
    let (goal_repo, task_repo) = setup_repos().await;

    let goal = Goal::new("Test goal", "For evaluation tasks");
    goal_repo.create(&goal).await.unwrap();

    // Create tasks with different sources
    let human_task = Task::with_title("Human task", "Created by human")
        .with_source(TaskSource::Human);
    let system_task = Task::with_title("System task", "Created by system")
        .with_source(TaskSource::System);
    let eval_task = Task::with_title("Eval task", "Created by goal evaluation")
        .with_source(TaskSource::GoalEvaluation(goal.id));

    task_repo.create(&human_task).await.unwrap();
    task_repo.create(&system_task).await.unwrap();
    task_repo.create(&eval_task).await.unwrap();

    // Query by source type
    let human_tasks = task_repo.list_by_source("human").await.unwrap();
    assert_eq!(human_tasks.len(), 1);
    assert_eq!(human_tasks[0].title, "Human task");

    let system_tasks = task_repo.list_by_source("system").await.unwrap();
    assert_eq!(system_tasks.len(), 1);

    let eval_tasks = task_repo.list_by_source("goal_evaluation").await.unwrap();
    assert_eq!(eval_tasks.len(), 1);
    assert_eq!(eval_tasks[0].source, TaskSource::GoalEvaluation(goal.id));
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
    goal_service
        .create_goal(
            "Well-tested code".to_string(),
            "All code should have tests".to_string(),
            GoalPriority::High,
            None,
            vec![],
            vec!["testing".to_string()],
            vec!["All modules have tests".to_string()],
        )
        .await
        .unwrap();

    goal_service
        .create_goal(
            "Secure code".to_string(),
            "No security vulnerabilities".to_string(),
            GoalPriority::Critical,
            None,
            vec![],
            vec!["security".to_string()],
            vec!["No injection vulnerabilities".to_string()],
        )
        .await
        .unwrap();

    goal_service
        .create_goal(
            "Fast infrastructure".to_string(),
            "Quick deployments".to_string(),
            GoalPriority::Normal,
            None,
            vec![],
            vec!["infrastructure".to_string()],
            vec![],
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

    let goal = goal_service
        .create_goal(
            "Retired testing goal".to_string(),
            "This goal is retired".to_string(),
            GoalPriority::Normal,
            None,
            vec![],
            vec!["testing".to_string()],
            vec!["Has tests".to_string()],
        )
        .await
        .unwrap();

    // Retire the goal
    goal_service
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
async fn test_format_goal_context_with_constraints_and_criteria() {
    let goal = Goal::new("Well-tested code", "All code should have comprehensive tests")
        .with_priority(GoalPriority::High)
        .with_constraint(GoalConstraint::invariant(
            "min-coverage",
            "Test coverage must be at least 80%",
        ))
        .with_constraint(GoalConstraint::preference(
            "test-style",
            "Prefer property-based tests where applicable",
        ))
        .with_evaluation_criterion("All modules have unit tests".to_string())
        .with_evaluation_criterion("Integration tests cover critical paths".to_string());

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
    assert!(
        output.contains("Success criteria:"),
        "Should have criteria section"
    );
    assert!(
        output.contains("All modules have unit tests"),
        "Should list criteria"
    );
    assert!(
        output.contains("Integration tests cover critical paths"),
        "Should list all criteria"
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
// 5. GOAL EVALUATION SERVICE
// =============================================================================

#[tokio::test]
async fn test_evaluate_goal_no_criteria() {
    let (goal_repo, task_repo) = setup_repos().await;
    let eval_service = GoalEvaluationService::new(goal_repo.clone(), task_repo.clone());

    // Goal with no evaluation criteria -> Unknown satisfaction
    let goal = Goal::new("Vague goal", "Do stuff well")
        .with_applicability_domain("code-quality");
    goal_repo.create(&goal).await.unwrap();

    let results = eval_service.evaluate_all_goals().await.unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].satisfaction_level, SatisfactionLevel::Unknown);
    assert!(results[0].gaps.is_empty());
    assert!(results[0].suggested_tasks.is_empty());
}

#[tokio::test]
async fn test_evaluate_goal_not_met() {
    let (goal_repo, task_repo) = setup_repos().await;
    let eval_service = GoalEvaluationService::new(goal_repo.clone(), task_repo.clone());

    // Create a goal with criteria but no completed tasks
    let goal = Goal::new("Well-tested code", "All code should have tests")
        .with_applicability_domain("testing")
        .with_evaluation_criterion("All modules have unit tests".to_string())
        .with_evaluation_criterion("Integration tests cover critical paths".to_string());
    goal_repo.create(&goal).await.unwrap();

    let results = eval_service.evaluate_all_goals().await.unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].satisfaction_level, SatisfactionLevel::NotMet);
    assert_eq!(results[0].gaps.len(), 2);
    assert_eq!(results[0].suggested_tasks.len(), 2);
}

#[tokio::test]
async fn test_evaluate_goal_partially_met() {
    let (goal_repo, task_repo) = setup_repos().await;
    let eval_service = GoalEvaluationService::new(goal_repo.clone(), task_repo.clone());

    // Create goal with 2 criteria
    let goal = Goal::new("Well-tested code", "All code should have tests")
        .with_applicability_domain("testing")
        .with_evaluation_criterion("All modules have unit tests".to_string())
        .with_evaluation_criterion("Integration tests cover critical paths".to_string());
    goal_repo.create(&goal).await.unwrap();

    // Create and complete a task that addresses one criterion
    let mut task = Task::with_title(
        "Write unit tests for all modules",
        "Added comprehensive unit tests across the codebase",
    );
    task.status = TaskStatus::Complete;
    task.completed_at = Some(chrono::Utc::now());
    task_repo.create(&task).await.unwrap();

    let results = eval_service.evaluate_all_goals().await.unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(
        results[0].satisfaction_level,
        SatisfactionLevel::PartiallyMet
    );
    assert_eq!(results[0].evidence.len(), 1);
    assert_eq!(results[0].gaps.len(), 1); // Integration tests not addressed
}

#[tokio::test]
async fn test_evaluate_goal_fully_met() {
    let (goal_repo, task_repo) = setup_repos().await;
    let eval_service = GoalEvaluationService::new(goal_repo.clone(), task_repo.clone());

    // Create goal with criteria
    let goal = Goal::new("Well-tested code", "All code should have tests")
        .with_applicability_domain("testing")
        .with_evaluation_criterion("All modules have unit tests".to_string())
        .with_evaluation_criterion("Integration tests cover critical paths".to_string());
    goal_repo.create(&goal).await.unwrap();

    // Create completed tasks that address both criteria
    let mut task1 = Task::with_title(
        "Write unit tests for all modules",
        "Added comprehensive unit tests across the codebase",
    );
    task1.status = TaskStatus::Complete;
    task1.completed_at = Some(chrono::Utc::now());
    task_repo.create(&task1).await.unwrap();

    let mut task2 = Task::with_title(
        "Add integration tests for critical paths",
        "Implemented integration tests covering all critical user flows",
    );
    task2.status = TaskStatus::Complete;
    task2.completed_at = Some(chrono::Utc::now());
    task_repo.create(&task2).await.unwrap();

    let results = eval_service.evaluate_all_goals().await.unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].satisfaction_level, SatisfactionLevel::Met);
    assert_eq!(results[0].evidence.len(), 2);
    assert!(results[0].gaps.is_empty());
    assert!(results[0].suggested_tasks.is_empty());
}

#[tokio::test]
async fn test_create_corrective_tasks() {
    let (goal_repo, task_repo) = setup_repos().await;
    let eval_service = GoalEvaluationService::new(goal_repo.clone(), task_repo.clone());

    // Create goal with unmet criteria
    let goal = Goal::new("Well-tested code", "All code should have tests")
        .with_applicability_domain("testing")
        .with_evaluation_criterion("All modules have unit tests".to_string())
        .with_evaluation_criterion("Integration tests cover critical paths".to_string());
    goal_repo.create(&goal).await.unwrap();

    // Evaluate - should find gaps
    let results = eval_service.evaluate_all_goals().await.unwrap();
    assert_eq!(results[0].gaps.len(), 2);

    // Create corrective tasks
    let corrective_tasks = eval_service
        .create_corrective_tasks(&results)
        .await
        .unwrap();
    assert_eq!(corrective_tasks.len(), 2);

    // Verify corrective tasks have correct source
    for task in &corrective_tasks {
        assert_eq!(task.source, TaskSource::GoalEvaluation(goal.id));
        assert!(task.idempotency_key.is_some());
    }

    // Verify tasks are persisted
    let eval_tasks = task_repo.list_by_source("goal_evaluation").await.unwrap();
    assert_eq!(eval_tasks.len(), 2);
}

#[tokio::test]
async fn test_corrective_tasks_idempotency() {
    let (goal_repo, task_repo) = setup_repos().await;
    let eval_service = GoalEvaluationService::new(goal_repo.clone(), task_repo.clone());

    // Create goal with unmet criteria
    let goal = Goal::new("Well-tested code", "All code should have tests")
        .with_applicability_domain("testing")
        .with_evaluation_criterion("All modules have unit tests".to_string());
    goal_repo.create(&goal).await.unwrap();

    // First evaluation + create corrective tasks
    let results1 = eval_service.evaluate_all_goals().await.unwrap();
    let tasks1 = eval_service
        .create_corrective_tasks(&results1)
        .await
        .unwrap();
    assert_eq!(tasks1.len(), 1);

    // Second evaluation + create corrective tasks
    let results2 = eval_service.evaluate_all_goals().await.unwrap();
    let tasks2 = eval_service
        .create_corrective_tasks(&results2)
        .await
        .unwrap();
    // Should be empty because the idempotency key already exists
    assert_eq!(
        tasks2.len(),
        0,
        "Duplicate corrective tasks should not be created"
    );

    // Verify only 1 corrective task exists total
    let all_eval_tasks = task_repo.list_by_source("goal_evaluation").await.unwrap();
    assert_eq!(all_eval_tasks.len(), 1);
}

#[tokio::test]
async fn test_run_evaluation_cycle() {
    let (goal_repo, task_repo) = setup_repos().await;
    let eval_service = GoalEvaluationService::new(goal_repo.clone(), task_repo.clone());

    // Create 2 goals
    let goal1 = Goal::new("Well-tested", "Tests everywhere")
        .with_applicability_domain("testing")
        .with_evaluation_criterion("Unit tests exist".to_string());
    goal_repo.create(&goal1).await.unwrap();

    let goal2 = Goal::new("Secure code", "No vulnerabilities")
        .with_applicability_domain("security")
        .with_evaluation_criterion("Auth is reviewed".to_string());
    goal_repo.create(&goal2).await.unwrap();

    // Complete a task that addresses goal2's criterion
    let mut auth_task = Task::with_title(
        "Review authentication and security",
        "Reviewed all auth code for vulnerabilities",
    );
    auth_task.status = TaskStatus::Complete;
    auth_task.completed_at = Some(chrono::Utc::now());
    task_repo.create(&auth_task).await.unwrap();

    // Run evaluation cycle
    let report = eval_service.run_evaluation_cycle().await.unwrap();

    assert_eq!(report.evaluated_count, 2);
    assert_eq!(report.goals_met, 1); // goal2 is met
    assert_eq!(report.goals_partially_met, 0);
    assert_eq!(report.gaps_found, 1); // goal1 has 1 unmet criterion
    assert_eq!(report.tasks_created, 1); // 1 corrective task for goal1
}

// =============================================================================
// 6. FULL LIFECYCLE SIMULATION
// =============================================================================

#[tokio::test]
async fn test_full_lifecycle_goal_context_evaluation() {
    let (goal_repo, task_repo) = setup_repos().await;

    let goal_service = GoalService::new(goal_repo.clone());
    let task_service = TaskService::new(task_repo.clone());
    let ctx_service = GoalContextService::new(goal_repo.clone());
    let eval_service = GoalEvaluationService::new(goal_repo.clone(), task_repo.clone());

    // Step 1: Human creates a goal - "code should be well-tested"
    let testing_goal = goal_service
        .create_goal(
            "Code should be well-tested".to_string(),
            "All production code should have comprehensive test coverage".to_string(),
            GoalPriority::High,
            None,
            vec![GoalConstraint::invariant(
                "min-coverage",
                "Test coverage must be at least 80%",
            )],
            vec!["testing".to_string(), "code-quality".to_string()],
            vec![
                "All modules have unit tests".to_string(),
                "Integration tests cover critical paths".to_string(),
            ],
        )
        .await
        .expect("Failed to create goal");

    assert_eq!(testing_goal.status, GoalStatus::Active);

    // Step 2: Human creates a task - "implement user authentication"
    let auth_task = task_service
        .submit_task(
            Some("Implement user authentication".to_string()),
            "Build login, registration, and session management".to_string(),
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

    assert_eq!(auth_task.source, TaskSource::Human);
    assert_eq!(auth_task.status, TaskStatus::Ready);

    // Step 3: System loads relevant goals as context for the task
    let relevant_goals = ctx_service.get_goals_for_task(&auth_task).await.unwrap();
    // The auth task should match code-quality domain
    assert!(
        !relevant_goals.is_empty(),
        "Should find relevant goals for auth task"
    );

    // Format the goal context for the agent
    let context =
        GoalContextService::<SqliteGoalRepository>::format_goal_context(&relevant_goals);
    assert!(
        !context.is_empty(),
        "Goal context should be non-empty"
    );
    assert!(
        context.contains("well-tested"),
        "Context should mention testing goal"
    );

    // Step 4: Task is executed and completed (simulated)
    let claimed = task_service
        .claim_task(auth_task.id, "developer-agent")
        .await
        .unwrap();
    assert_eq!(claimed.status, TaskStatus::Running);

    let completed = task_service.complete_task(auth_task.id).await.unwrap();
    assert_eq!(completed.status, TaskStatus::Complete);

    // Step 5: Goal evaluation detects the testing goal isn't fully met
    let eval_results = eval_service.evaluate_all_goals().await.unwrap();
    assert_eq!(eval_results.len(), 1);
    let testing_eval = &eval_results[0];
    assert_eq!(testing_eval.goal_id, testing_goal.id);

    // The auth task was completed, but it doesn't directly address
    // "all modules have unit tests" or "integration tests cover critical paths"
    // in a way the keyword matching would pick up.
    // So the goal should be NotMet or PartiallyMet.
    assert_ne!(
        testing_eval.satisfaction_level,
        SatisfactionLevel::Met,
        "Goal should not be fully met yet"
    );
    assert!(
        !testing_eval.gaps.is_empty(),
        "Should identify gaps"
    );

    // Step 6: Evaluation service creates corrective tasks
    let corrective_tasks = eval_service
        .create_corrective_tasks(&eval_results)
        .await
        .unwrap();

    assert!(
        !corrective_tasks.is_empty(),
        "Should create corrective tasks for unmet criteria"
    );

    // All corrective tasks should have GoalEvaluation source
    for task in &corrective_tasks {
        assert_eq!(
            task.source,
            TaskSource::GoalEvaluation(testing_goal.id),
            "Corrective task should reference the goal"
        );
    }

    // Step 7: Verify the corrective tasks are in the task repo
    let eval_tasks = task_repo.list_by_source("goal_evaluation").await.unwrap();
    assert_eq!(eval_tasks.len(), corrective_tasks.len());

    // Step 8: Verify idempotency - running again should not create duplicates
    let eval_results_2 = eval_service.evaluate_all_goals().await.unwrap();
    let corrective_2 = eval_service
        .create_corrective_tasks(&eval_results_2)
        .await
        .unwrap();
    assert_eq!(
        corrective_2.len(),
        0,
        "Running evaluation again should not create duplicate tasks"
    );

    // Step 9: Complete the corrective tasks
    for eval_task in &eval_tasks {
        // Transition to ready first (they start as pending)
        let mut t = task_repo.get(eval_task.id).await.unwrap().unwrap();
        if t.status == TaskStatus::Pending {
            t.transition_to(TaskStatus::Ready).ok();
            task_repo.update(&t).await.unwrap();
        }
        task_service.claim_task(eval_task.id, "test-writer").await.unwrap();
        task_service.complete_task(eval_task.id).await.unwrap();
    }

    // Step 10: The goal remains active (goals are never completed)
    let final_goal = goal_service.get_goal(testing_goal.id).await.unwrap().unwrap();
    assert_eq!(
        final_goal.status,
        GoalStatus::Active,
        "Goal should still be active - goals are never completed"
    );
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
        .with_applicability_domain("testing")
        .with_evaluation_criterion("Coverage > 80%".to_string());
    goal_repo.create(&goal1).await.unwrap();

    let goal2 = Goal::new("Speed goal", "Fast test execution")
        .with_applicability_domain("testing")
        .with_evaluation_criterion("Tests run in < 60s".to_string());
    goal_repo.create(&goal2).await.unwrap();

    // A testing task should pick up both goals
    let task = Task::with_title("Write tests", "Add comprehensive test suite");
    let goals = ctx_service.get_goals_for_task(&task).await.unwrap();
    assert_eq!(goals.len(), 2, "Should match both testing goals");
}

#[tokio::test]
async fn test_evaluation_only_considers_relevant_completed_tasks() {
    let (goal_repo, task_repo) = setup_repos().await;
    let eval_service = GoalEvaluationService::new(goal_repo.clone(), task_repo.clone());

    // Create a testing goal
    let goal = Goal::new("Well-tested", "Tests everywhere")
        .with_applicability_domain("testing")
        .with_evaluation_criterion("All modules have unit tests".to_string());
    goal_repo.create(&goal).await.unwrap();

    // Create a completed task in a different domain (infrastructure)
    // This should NOT satisfy the testing criterion
    let mut infra_task = Task::with_title(
        "Set up CI/CD pipeline",
        "Configure deployment infrastructure",
    );
    infra_task.status = TaskStatus::Complete;
    infra_task.completed_at = Some(chrono::Utc::now());
    task_repo.create(&infra_task).await.unwrap();

    let results = eval_service.evaluate_all_goals().await.unwrap();
    assert_eq!(results[0].satisfaction_level, SatisfactionLevel::NotMet);
    assert_eq!(results[0].gaps.len(), 1);
}

#[tokio::test]
async fn test_paused_goals_not_evaluated() {
    let (goal_repo, task_repo) = setup_repos().await;
    let goal_service = GoalService::new(goal_repo.clone());
    let eval_service = GoalEvaluationService::new(goal_repo.clone(), task_repo.clone());

    let goal = goal_service
        .create_goal(
            "Paused goal".to_string(),
            "This goal is paused".to_string(),
            GoalPriority::Normal,
            None,
            vec![],
            vec!["testing".to_string()],
            vec!["Has tests".to_string()],
        )
        .await
        .unwrap();

    // Pause the goal
    goal_service
        .transition_status(goal.id, GoalStatus::Paused)
        .await
        .unwrap();

    // Evaluation should not include paused goals
    // (get_active_with_constraints only returns active goals)
    let results = eval_service.evaluate_all_goals().await.unwrap();
    assert!(
        results.is_empty(),
        "Paused goals should not be evaluated"
    );
}
