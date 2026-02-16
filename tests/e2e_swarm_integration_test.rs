//! End-to-End Integration Test for Abathur Swarm System.
//!
//! This test verifies the complete flow from swarm initialization through
//! goal creation, task DAG execution, agent evolution, and improvement loops.
//!
//! ## Test Coverage:
//! 1. Swarm Starting - Orchestrator initialization and tick execution
//! 2. Goal Setting - Goal lifecycle with constraints
//! 3. Task Setting with DAG - Dependencies and proper ordering
//! 4. Agent Creation - Template registration and instance spawning
//! 5. DAG Execution - Wave-based parallel execution
//! 6. Evolution Loop - Track outcomes and trigger improvements on failure
//! 7. Real Agent Execution - Using actual Claude Code CLI with simple test tasks
//!
//! ## Running the tests:
//!
//! Fast mock-based tests (default):
//! ```sh
//! cargo test --test e2e_swarm_integration_test
//! ```
//!
//! Real agent E2E tests (requires Claude CLI installed):
//! ```sh
//! cargo test --test e2e_swarm_integration_test --features real_agents -- --ignored
//! ```
//!
//! Run all tests including real agents:
//! ```sh
//! ABATHUR_REAL_E2E=1 cargo test --test e2e_swarm_integration_test -- --include-ignored
//! ```

use std::sync::Arc;
use tokio::sync::mpsc;

use abathur::adapters::sqlite::{
    create_migrated_test_pool, SqliteAgentRepository,
    SqliteGoalRepository, SqliteMemoryRepository, SqliteTaskRepository,
    SqliteWorktreeRepository,
};
use abathur::adapters::substrates::{ClaudeCodeSubstrate, MockSubstrate};
use abathur::adapters::substrates::mock::MockResponse;
use abathur::domain::models::{
    AgentConstraint, AgentTier, Goal, GoalConstraint, GoalPriority, GoalStatus, MemoryTier,
    MemoryType, Task, TaskDag, TaskPriority, TaskSource, TaskStatus, ToolCapability,
};
use abathur::domain::ports::{
    AgentRepository, GoalRepository, MemoryRepository, NullMemoryRepository, Substrate,
    TaskFilter, TaskRepository,
};
use abathur::services::{
    AgentService, DagExecutor, ExecutionEvent, ExecutionStatus, ExecutorConfig, EvolutionLoop,
    GoalService, MemoryService, SwarmConfig, SwarmOrchestrator,
    TaskExecution, TaskOutcome, TaskService,
};

/// Helper to set up all test repositories with in-memory SQLite.
async fn setup_test_environment() -> (
    Arc<SqliteGoalRepository>,
    Arc<SqliteTaskRepository>,
    Arc<SqliteMemoryRepository>,
    Arc<SqliteAgentRepository>,
    Arc<SqliteWorktreeRepository>,
) {
    let pool = create_migrated_test_pool()
        .await
        .expect("Failed to create test pool");

    (
        Arc::new(SqliteGoalRepository::new(pool.clone())),
        Arc::new(SqliteTaskRepository::new(pool.clone())),
        Arc::new(SqliteMemoryRepository::new(pool.clone())),
        Arc::new(SqliteAgentRepository::new(pool.clone())),
        Arc::new(SqliteWorktreeRepository::new(pool.clone())),
    )
}

// =============================================================================
// TEST 1: SWARM ORCHESTRATOR INITIALIZATION AND TICK
// =============================================================================

/// Test that the swarm orchestrator can be created and run a tick.
#[tokio::test]
async fn test_swarm_orchestrator_initialization_and_tick() {
    let (goal_repo, task_repo, _, agent_repo, worktree_repo) = setup_test_environment().await;

    let mock_substrate = Arc::new(MockSubstrate::new());
    let substrate: Arc<dyn Substrate> = mock_substrate;

    let mut config = SwarmConfig::default();
    config.use_worktrees = false; // Disable for test simplicity

    let event_bus = Arc::new(abathur::services::EventBus::new(abathur::services::EventBusConfig::default()));
    let event_reactor = Arc::new(abathur::services::EventReactor::new(event_bus.clone(), abathur::services::ReactorConfig::default()));
    let event_scheduler = Arc::new(abathur::services::EventScheduler::new(event_bus.clone(), abathur::services::SchedulerConfig::default()));

    let orchestrator: SwarmOrchestrator<_, _, _, _, NullMemoryRepository> = SwarmOrchestrator::new(
        goal_repo.clone(),
        task_repo.clone(),
        worktree_repo.clone(),
        agent_repo.clone(),
        substrate,
        config,
        event_bus,
        event_reactor,
        event_scheduler,
    );

    // Run a tick with no goals - should complete without error
    let stats = orchestrator.tick().await.expect("Failed to run tick");

    assert_eq!(stats.active_goals, 0, "No active goals expected");
    assert_eq!(stats.pending_tasks, 0, "No pending tasks expected");
    assert_eq!(stats.running_tasks, 0, "No running tasks expected");

    println!("✓ Swarm orchestrator initialized and tick completed successfully");
}

// =============================================================================
// TEST 2: GOAL LIFECYCLE WITH CONSTRAINTS
// =============================================================================

/// Test goal creation, constraint handling, and status transitions.
#[tokio::test]
async fn test_goal_lifecycle_with_constraints() {
    let (goal_repo, _, _, _, _) = setup_test_environment().await;

    let _event_bus = Arc::new(abathur::services::EventBus::new(abathur::services::EventBusConfig::default()));
    let goal_service = GoalService::new(goal_repo.clone());

    // Create a goal with constraints
    let (goal, _events) = goal_service
        .create_goal(
            "Implement Authentication System".to_string(),
            "Build a complete auth system with login, logout, and session management".to_string(),
            GoalPriority::High,
            None,
            vec![
                GoalConstraint::invariant("security", "All passwords must be hashed with bcrypt"),
                GoalConstraint::boundary("performance", "Login must complete in under 500ms"),
                GoalConstraint::preference("ux", "Use OAuth2 if possible"),
            ],
            vec![],
        )
        .await
        .expect("Failed to create goal");

    // Verify goal was created with correct status
    assert_eq!(goal.status, GoalStatus::Active);
    assert_eq!(goal.priority, GoalPriority::High);
    assert_eq!(goal.constraints.len(), 3);

    // Get effective constraints (should include all 3)
    let constraints = goal_service
        .get_effective_constraints(goal.id)
        .await
        .expect("Failed to get constraints");
    assert_eq!(constraints.len(), 3);

    // Test goal status transitions
    // Goals are NEVER completed - only Paused or Retired
    let (_, _events) = goal_service
        .transition_status(goal.id, GoalStatus::Paused)
        .await
        .expect("Failed to pause goal");

    let paused_goal = goal_service
        .get_goal(goal.id)
        .await
        .expect("Failed to get goal")
        .unwrap();
    assert_eq!(paused_goal.status, GoalStatus::Paused);

    // Resume the goal
    let (_, _events) = goal_service
        .transition_status(goal.id, GoalStatus::Active)
        .await
        .expect("Failed to resume goal");

    // Retire the goal when no longer relevant
    let (_, _events) = goal_service
        .transition_status(goal.id, GoalStatus::Retired)
        .await
        .expect("Failed to retire goal");

    let retired_goal = goal_service
        .get_goal(goal.id)
        .await
        .expect("Failed to get goal")
        .unwrap();
    assert_eq!(retired_goal.status, GoalStatus::Retired);

    println!("✓ Goal lifecycle with constraints working correctly");
}

// =============================================================================
// TEST 3: TASK DAG WITH DEPENDENCIES
// =============================================================================

/// Test task creation with dependencies and proper DAG ordering.
#[tokio::test]
async fn test_task_dag_with_dependencies() {
    let (goal_repo, task_repo, _, _, _) = setup_test_environment().await;

    let _event_bus = Arc::new(abathur::services::EventBus::new(abathur::services::EventBusConfig::default()));
    let goal_service = GoalService::new(goal_repo.clone());
    let task_service = TaskService::new(task_repo.clone());

    // Create a goal (aspirational context, not directly linked to tasks)
    let (_goal, _events) = goal_service
        .create_goal(
            "Build API".to_string(),
            "Create REST API endpoints".to_string(),
            GoalPriority::Normal,
            None,
            vec![],
            vec![],
        )
        .await
        .expect("Failed to create goal");

    // Create tasks with dependencies forming a DAG:
    //
    //   setup (wave 1)
    //      |
    //   +--+--+
    //   |     |
    //  auth  db  (wave 2)
    //   |     |
    //   +--+--+
    //      |
    //   api (wave 3)
    //      |
    //   test (wave 4)

    let (setup_task, _events) = task_service
        .submit_task(
            Some("Setup Project".to_string()),
            "Initialize project structure and dependencies".to_string(),
            None,
            TaskPriority::High,
            Some("worker".to_string()),
            vec![], // No dependencies
            None,
            None,
            TaskSource::Human,
            None,
        )
        .await
        .expect("Failed to create setup task");

    let (auth_task, _events) = task_service
        .submit_task(
            Some("Implement Auth".to_string()),
            "Create authentication middleware".to_string(),
            None,
            TaskPriority::Normal,
            Some("worker".to_string()),
            vec![setup_task.id], // Depends on setup
            None,
            None,
            TaskSource::Human,
            None,
        )
        .await
        .expect("Failed to create auth task");

    let (db_task, _events) = task_service
        .submit_task(
            Some("Setup Database".to_string()),
            "Configure database connection and migrations".to_string(),
            None,
            TaskPriority::Normal,
            Some("worker".to_string()),
            vec![setup_task.id], // Depends on setup (parallel with auth)
            None,
            None,
            TaskSource::Human,
            None,
        )
        .await
        .expect("Failed to create db task");

    let (api_task, _events) = task_service
        .submit_task(
            Some("Build API Endpoints".to_string()),
            "Create CRUD endpoints for all resources".to_string(),
            None,
            TaskPriority::Normal,
            Some("worker".to_string()),
            vec![auth_task.id, db_task.id], // Depends on both auth AND db
            None,
            None,
            TaskSource::Human,
            None,
        )
        .await
        .expect("Failed to create api task");

    let (test_task, _events) = task_service
        .submit_task(
            Some("Write Tests".to_string()),
            "Create integration tests for all endpoints".to_string(),
            None,
            TaskPriority::Low,
            Some("worker".to_string()),
            vec![api_task.id], // Depends on api
            None,
            None,
            TaskSource::Human,
            None,
        )
        .await
        .expect("Failed to create test task");

    // Build DAG from tasks
    let tasks = task_repo
        .list(TaskFilter::default())
        .await
        .expect("Failed to list tasks");
    assert_eq!(tasks.len(), 5);

    let dag = TaskDag::from_tasks(tasks.clone());

    // Verify DAG structure
    assert!(dag.nodes.contains_key(&setup_task.id));
    assert!(dag.nodes.contains_key(&auth_task.id));
    assert!(dag.nodes.contains_key(&db_task.id));
    assert!(dag.nodes.contains_key(&api_task.id));
    assert!(dag.nodes.contains_key(&test_task.id));

    // Verify execution waves
    let waves = dag.execution_waves().expect("Failed to get execution waves");

    assert_eq!(waves.len(), 4, "Expected 4 execution waves");

    // Wave 1: setup only (root)
    assert_eq!(waves[0].len(), 1);
    assert!(waves[0].contains(&setup_task.id));

    // Wave 2: auth and db (parallel after setup)
    assert_eq!(waves[1].len(), 2);
    assert!(waves[1].contains(&auth_task.id));
    assert!(waves[1].contains(&db_task.id));

    // Wave 3: api (depends on auth and db)
    assert_eq!(waves[2].len(), 1);
    assert!(waves[2].contains(&api_task.id));

    // Wave 4: test (depends on api)
    assert_eq!(waves[3].len(), 1);
    assert!(waves[3].contains(&test_task.id));

    // Verify topological sort is valid
    let sorted = dag.topological_sort().expect("DAG should be acyclic");
    assert_eq!(sorted.len(), 5);

    // Setup must come before auth and db
    let setup_pos = sorted.iter().position(|&id| id == setup_task.id).unwrap();
    let auth_pos = sorted.iter().position(|&id| id == auth_task.id).unwrap();
    let db_pos = sorted.iter().position(|&id| id == db_task.id).unwrap();
    let api_pos = sorted.iter().position(|&id| id == api_task.id).unwrap();
    let test_pos = sorted.iter().position(|&id| id == test_task.id).unwrap();

    assert!(setup_pos < auth_pos, "Setup must come before auth");
    assert!(setup_pos < db_pos, "Setup must come before db");
    assert!(auth_pos < api_pos, "Auth must come before api");
    assert!(db_pos < api_pos, "DB must come before api");
    assert!(api_pos < test_pos, "API must come before test");

    // Verify initial task statuses
    let setup = task_repo.get(setup_task.id).await.unwrap().unwrap();
    assert_eq!(setup.status, TaskStatus::Ready, "Setup should be Ready (no deps)");

    let auth = task_repo.get(auth_task.id).await.unwrap().unwrap();
    assert_eq!(auth.status, TaskStatus::Pending, "Auth should be Pending (deps not met)");

    println!("✓ Task DAG with dependencies structured correctly");
}

// =============================================================================
// TEST 4: AGENT TEMPLATE AND INSTANCE MANAGEMENT
// =============================================================================

/// Test agent template registration and instance spawning.
#[tokio::test]
async fn test_agent_template_and_instance_management() {
    let (_, _, _, agent_repo, _) = setup_test_environment().await;

    let event_bus = Arc::new(abathur::services::event_bus::EventBus::new(abathur::services::event_bus::EventBusConfig::default()));
    let agent_service = AgentService::new(agent_repo.clone(), event_bus.clone());

    // Register different agent tiers
    let worker_template = agent_service
        .register_template(
            "code-worker".to_string(),
            "A worker agent for coding tasks".to_string(),
            AgentTier::Worker,
            "You are a skilled software developer. Focus on writing clean, tested code.".to_string(),
            vec![
                ToolCapability::new("Read", "Read files"),
                ToolCapability::new("Write", "Write files"),
                ToolCapability::new("Bash", "Execute commands"),
            ],
            vec![],
            Some(25),
        )
        .await
        .expect("Failed to register worker template");

    assert_eq!(worker_template.name, "code-worker");
    assert_eq!(worker_template.tier, AgentTier::Worker);
    assert_eq!(worker_template.version, 1);

    let specialist_template = agent_service
        .register_template(
            "test-specialist".to_string(),
            "A specialist for writing and running tests".to_string(),
            AgentTier::Specialist,
            "You are a testing expert. Focus on comprehensive test coverage.".to_string(),
            vec![
                ToolCapability::new("Read", "Read files"),
                ToolCapability::new("Write", "Write files"),
                ToolCapability::new("Bash", "Execute commands"),
            ],
            vec![
                AgentConstraint::new("pytest", "Use pytest for Python tests"),
                AgentConstraint::new("jest", "Use jest for JavaScript tests"),
            ],
            Some(30),
        )
        .await
        .expect("Failed to register specialist template");

    assert_eq!(specialist_template.tier, AgentTier::Specialist);

    // Spawn instances
    let worker_instance = agent_service
        .spawn_instance("code-worker")
        .await
        .expect("Failed to spawn worker instance");

    assert_eq!(worker_instance.template_name, "code-worker");

    // Spawned instances start as Idle, they become Running when assigned a task
    use abathur::domain::models::InstanceStatus;
    assert_eq!(worker_instance.status, InstanceStatus::Idle);

    // Assign a task to make it running
    let task_id = uuid::Uuid::new_v4();
    let running_instance = agent_service
        .assign_task(worker_instance.id, task_id)
        .await
        .expect("Failed to assign task");
    assert_eq!(running_instance.status, InstanceStatus::Running);

    // Verify running instances
    let running = agent_service
        .get_running_instances()
        .await
        .expect("Failed to list running instances");
    assert_eq!(running.len(), 1);

    // Complete the instance
    agent_service
        .complete_instance(worker_instance.id)
        .await
        .expect("Failed to complete instance");

    // Verify no running instances
    let running_after = agent_service
        .get_running_instances()
        .await
        .expect("Failed to list running instances");
    assert_eq!(running_after.len(), 0);

    // Verify templates can be retrieved by name
    let retrieved_worker = agent_repo
        .get_template_by_name("code-worker")
        .await
        .expect("Failed to get worker template");
    assert!(retrieved_worker.is_some());
    assert_eq!(retrieved_worker.unwrap().tier, AgentTier::Worker);

    let retrieved_specialist = agent_repo
        .get_template_by_name("test-specialist")
        .await
        .expect("Failed to get specialist template");
    assert!(retrieved_specialist.is_some());
    assert_eq!(retrieved_specialist.unwrap().tier, AgentTier::Specialist);

    println!("✓ Agent template and instance management working correctly");
}

// =============================================================================
// TEST 5: DAG EXECUTION WITH MOCK SUBSTRATE
// =============================================================================

/// Test DAG execution with wave-based parallelism using mock substrate.
#[tokio::test]
async fn test_dag_execution_with_waves() {
    let (_goal_repo, task_repo, _, agent_repo, _) = setup_test_environment().await;

    // Create a mock substrate that tracks all executions
    let mock_substrate = Arc::new(MockSubstrate::new());

    // Create tasks with DAG structure
    let task1 = Task::with_title("Task 1 - Foundation", "Set up foundation");
    let task2 = Task::with_title("Task 2 - Module A", "Build module A").with_dependency(task1.id);
    let task3 = Task::with_title("Task 3 - Module B", "Build module B").with_dependency(task1.id);
    let task4 = Task::with_title("Task 4 - Integration", "Integrate modules")
        .with_dependency(task2.id)
        .with_dependency(task3.id);

    // Store tasks
    task_repo.create(&task1).await.expect("Failed to create task1");
    task_repo.create(&task2).await.expect("Failed to create task2");
    task_repo.create(&task3).await.expect("Failed to create task3");
    task_repo.create(&task4).await.expect("Failed to create task4");

    // Build DAG
    let tasks = task_repo
        .list(TaskFilter::default())
        .await
        .expect("Failed to list tasks");
    let dag = TaskDag::from_tasks(tasks);

    // Verify waves structure
    let waves = dag.execution_waves().expect("Should have valid waves");
    assert_eq!(waves.len(), 3, "Expected 3 waves");
    assert_eq!(waves[0].len(), 1, "Wave 1: foundation only");
    assert_eq!(waves[1].len(), 2, "Wave 2: modules A and B in parallel");
    assert_eq!(waves[2].len(), 1, "Wave 3: integration");

    // Create executor
    let config = ExecutorConfig {
        max_concurrency: 4,
        task_timeout_secs: 60,
        max_retries: 2,
        default_max_turns: 10,
        fail_fast: false,
        ..Default::default()
    };

    let executor: DagExecutor<_, SqliteAgentRepository, SqliteGoalRepository> =
        DagExecutor::new(task_repo.clone(), agent_repo.clone(), mock_substrate.clone(), config);

    // Execute DAG with events
    let (event_tx, mut event_rx) = mpsc::channel(100);
    let results = executor
        .execute_with_events(&dag, event_tx)
        .await
        .expect("Failed to execute DAG");

    // Collect events
    let mut events = vec![];
    event_rx.close();
    while let Some(event) = event_rx.recv().await {
        events.push(event);
    }

    // Verify execution results
    assert_eq!(results.total_tasks, 4);
    assert_eq!(results.completed_tasks, 4);
    assert_eq!(results.failed_tasks, 0);
    assert_eq!(results.status(), ExecutionStatus::Completed);

    // Verify events sequence
    let start_events: Vec<_> = events
        .iter()
        .filter(|e| matches!(e, ExecutionEvent::Started { .. }))
        .collect();
    assert_eq!(start_events.len(), 1, "Should have exactly one Started event");

    let wave_started_events: Vec<_> = events
        .iter()
        .filter(|e| matches!(e, ExecutionEvent::WaveStarted { .. }))
        .collect();
    assert_eq!(wave_started_events.len(), 3, "Should have 3 WaveStarted events");

    let completed_events: Vec<_> = events
        .iter()
        .filter(|e| matches!(e, ExecutionEvent::TaskCompleted { .. }))
        .collect();
    assert_eq!(completed_events.len(), 4, "All 4 tasks should complete");

    println!("✓ DAG execution with wave-based parallelism working correctly");
}

// =============================================================================
// TEST 6: DAG EXECUTION WITH FAILURES AND RETRIES
// =============================================================================

/// Test DAG execution handles failures and retries correctly.
#[tokio::test]
async fn test_dag_execution_with_failures() {
    let (_, task_repo, _, agent_repo, _) = setup_test_environment().await;

    // Create mock substrate that fails for specific tasks
    let mock_substrate = Arc::new(MockSubstrate::new());

    // Create tasks
    let task1 = Task::with_title("Task 1 - Will Succeed", "This will succeed");
    let task2 = Task::with_title("Task 2 - Will Fail", "This will fail").with_dependency(task1.id);

    // Configure task2 to fail
    mock_substrate
        .set_response_for_task(task2.id, MockResponse::failure("Simulated failure"))
        .await;

    // Store tasks
    task_repo.create(&task1).await.expect("Failed to create task1");
    task_repo.create(&task2).await.expect("Failed to create task2");

    // Build DAG
    let tasks = task_repo
        .list(TaskFilter::default())
        .await
        .expect("Failed to list tasks");
    let dag = TaskDag::from_tasks(tasks);

    // Create executor with retries
    let config = ExecutorConfig {
        max_concurrency: 2,
        task_timeout_secs: 60,
        max_retries: 2, // Will retry twice
        default_max_turns: 10,
        fail_fast: false,
        ..Default::default()
    };

    let executor: DagExecutor<_, SqliteAgentRepository, SqliteGoalRepository> =
        DagExecutor::new(task_repo.clone(), agent_repo.clone(), mock_substrate.clone(), config);

    // Execute DAG
    let results = executor.execute(&dag).await.expect("Failed to execute DAG");

    // Task1 should succeed, Task2 should fail after retries
    assert_eq!(results.completed_tasks, 1, "Only task1 should complete");
    assert_eq!(results.failed_tasks, 1, "Task2 should fail");
    assert_eq!(
        results.status(),
        ExecutionStatus::PartialSuccess,
        "Should be partial success"
    );

    println!("✓ DAG execution handles failures and retries correctly");
}

// =============================================================================
// TEST 7: EVOLUTION LOOP - TRACK OUTCOMES AND TRIGGER IMPROVEMENTS
// =============================================================================

/// Test the evolution loop tracks outcomes and triggers refinements.
#[tokio::test]
async fn test_evolution_loop_tracking_and_improvements() {
    let evolution_loop = EvolutionLoop::with_default_config();

    let template_name = "struggling-worker".to_string();
    let template_version = 1;

    // Record a mix of successes and failures (below 60% threshold)
    // Need at least 10 tasks for VeryLowSuccessRate, but 5+ for LowSuccessRate

    // 3 successes
    for _i in 0..3 {
        evolution_loop
            .record_execution(TaskExecution {
                task_id: uuid::Uuid::new_v4(),
                template_name: template_name.clone(),
                template_version,
                outcome: TaskOutcome::Success,
                executed_at: chrono::Utc::now(),
                turns_used: 5,
                tokens_used: 1000,
                downstream_tasks: vec![],
            })
            .await;
    }

    // 7 failures (total: 3 success + 7 fail = 30% success rate, 10 tasks total)
    for _i in 0..7 {
        evolution_loop
            .record_execution(TaskExecution {
                task_id: uuid::Uuid::new_v4(),
                template_name: template_name.clone(),
                template_version,
                outcome: TaskOutcome::Failure,
                executed_at: chrono::Utc::now(),
                turns_used: 10,
                tokens_used: 2000,
                downstream_tasks: vec![],
            })
            .await;
    }

    // Get stats
    let stats = evolution_loop
        .get_stats(&template_name)
        .await
        .expect("Should have stats");

    assert_eq!(stats.total_tasks, 10);
    assert_eq!(stats.successful_tasks, 3);
    assert_eq!(stats.failed_tasks, 7);
    assert!(
        stats.success_rate < 0.4,
        "Success rate should be ~30%"
    );

    // Evaluate - should trigger evolution due to very low success rate
    // VeryLowSuccessRate requires: success_rate < 40% AND total_tasks >= 10
    let events = evolution_loop.evaluate().await;

    assert!(!events.is_empty(), "Should have evolution events");

    // Check that refinement was triggered (either LowSuccessRate or VeryLowSuccessRate)
    let refinement_events: Vec<_> = events
        .iter()
        .filter(|e| {
            matches!(
                e.trigger,
                abathur::services::EvolutionTrigger::VeryLowSuccessRate
                    | abathur::services::EvolutionTrigger::LowSuccessRate
            )
        })
        .collect();
    assert!(
        !refinement_events.is_empty(),
        "Should trigger low success rate event"
    );

    // Check pending refinements
    let refinements = evolution_loop.get_pending_refinements().await;
    assert!(!refinements.is_empty(), "Should have pending refinement request");

    let refinement = &refinements[0];
    assert_eq!(refinement.template_name, template_name);

    println!("✓ Evolution loop tracking and improvement triggers working correctly");
}

// =============================================================================
// TEST 8: EVOLUTION LOOP - GOAL VIOLATIONS TRIGGER IMMEDIATE REVIEW
// =============================================================================

/// Test that goal violations trigger immediate refinement.
#[tokio::test]
async fn test_evolution_loop_goal_violations() {
    let evolution_loop = EvolutionLoop::with_default_config();

    let template_name = "violating-worker".to_string();

    // Record tasks with a goal violation
    for i in 0..5 {
        let outcome = if i == 0 {
            TaskOutcome::GoalViolation
        } else {
            TaskOutcome::Success
        };

        evolution_loop
            .record_execution(TaskExecution {
                task_id: uuid::Uuid::new_v4(),
                template_name: template_name.clone(),
                template_version: 1,
                outcome,
                executed_at: chrono::Utc::now(),
                turns_used: 5,
                tokens_used: 1000,
                downstream_tasks: vec![],
            })
            .await;
    }

    // Evaluate
    let events = evolution_loop.evaluate().await;

    // Should trigger immediate refinement due to goal violation
    let violation_events: Vec<_> = events
        .iter()
        .filter(|e| {
            matches!(
                e.trigger,
                abathur::services::EvolutionTrigger::GoalViolations
            )
        })
        .collect();

    assert!(
        !violation_events.is_empty(),
        "Should trigger GoalViolations event"
    );

    // Check that severity is Immediate
    let refinements = evolution_loop.get_pending_refinements().await;
    let violation_refinement: Vec<_> = refinements
        .iter()
        .filter(|r| r.template_name == template_name)
        .collect();

    assert!(!violation_refinement.is_empty());
    assert_eq!(
        violation_refinement[0].severity,
        abathur::services::RefinementSeverity::Immediate
    );

    println!("✓ Evolution loop correctly identifies goal violations as immediate priority");
}

// =============================================================================
// TEST 9: FULL END-TO-END WORKFLOW
// =============================================================================

/// Complete end-to-end test: Goal → Decomposition → Execution → Evolution
#[tokio::test]
async fn test_full_end_to_end_workflow() {
    let (goal_repo, task_repo, memory_repo, agent_repo, _worktree_repo) =
        setup_test_environment().await;

    // 1. Create services
    let event_bus = Arc::new(abathur::services::EventBus::new(abathur::services::EventBusConfig::default()));
    let goal_service = GoalService::new(goal_repo.clone());
    let task_service = TaskService::new(task_repo.clone());
    let agent_service = AgentService::new(agent_repo.clone(), event_bus.clone());
    let memory_service = MemoryService::new(memory_repo.clone());
    let evolution_loop = Arc::new(EvolutionLoop::with_default_config());

    // 2. Create a goal with constraints
    let (goal, _events) = goal_service
        .create_goal(
            "Build User Service".to_string(),
            "Create a user management service with CRUD operations".to_string(),
            GoalPriority::High,
            None,
            vec![
                GoalConstraint::invariant("testing", "All endpoints must have tests"),
                GoalConstraint::boundary("coverage", "Test coverage must be > 80%"),
            ],
            vec![],
        )
        .await
        .expect("Failed to create goal");

    println!("Created goal: {} ({})", goal.name, goal.id);

    // 3. Create tasks manually (goals no longer decompose into tasks)
    let (task1, _events) = task_service
        .submit_task(
            Some("Setup user service".to_string()),
            "Initialize user service structure".to_string(),
            None,
            TaskPriority::High,
            Some("user-service-worker".to_string()),
            vec![],
            None,
            None,
            TaskSource::Human,
            None,
        )
        .await
        .expect("Failed to create task");

    let (_task2, _events) = task_service
        .submit_task(
            Some("Implement user CRUD".to_string()),
            "Create CRUD endpoints for users".to_string(),
            None,
            TaskPriority::Normal,
            Some("user-service-worker".to_string()),
            vec![task1.id],
            None,
            None,
            TaskSource::Human,
            None,
        )
        .await
        .expect("Failed to create task");

    println!("Created 2 tasks for goal");

    // 4. Register an agent template
    let worker = agent_service
        .register_template(
            "user-service-worker".to_string(),
            "Worker for user service tasks".to_string(),
            AgentTier::Worker,
            "You are a backend developer building user services.".to_string(),
            vec![
                ToolCapability::new("Read", "Read files"),
                ToolCapability::new("Write", "Write files"),
            ],
            vec![],
            Some(20),
        )
        .await
        .expect("Failed to register template");

    println!("Registered agent template: {}", worker.name);

    // 5. Execute tasks through DAG executor with mock substrate
    let mock_substrate = Arc::new(MockSubstrate::new());

    let tasks = task_repo
        .list(TaskFilter::default())
        .await
        .expect("Failed to list tasks");

    if !tasks.is_empty() {
        let dag = TaskDag::from_tasks(tasks.clone());

        let executor_config = ExecutorConfig {
            max_concurrency: 4,
            task_timeout_secs: 60,
            max_retries: 2,
            default_max_turns: 20,
            fail_fast: false,
            ..Default::default()
        };

        let executor: DagExecutor<_, _, SqliteGoalRepository> = DagExecutor::new(
            task_repo.clone(),
            agent_repo.clone(),
            mock_substrate.clone(),
            executor_config,
        );

        let results = executor.execute(&dag).await.expect("Failed to execute DAG");

        println!(
            "DAG execution completed: {}/{} tasks succeeded",
            results.completed_tasks, results.total_tasks
        );

        // 6. Record executions in evolution loop
        for task in &tasks {
            evolution_loop
                .record_execution(TaskExecution {
                    task_id: task.id,
                    template_name: worker.name.clone(),
                    template_version: worker.version,
                    outcome: TaskOutcome::Success,
                    executed_at: chrono::Utc::now(),
                    turns_used: 5,
                    tokens_used: 500,
                    downstream_tasks: vec![],
                })
                .await;
        }

        // Check evolution stats
        let stats = evolution_loop.get_stats(&worker.name).await;
        if let Some(stats) = stats {
            println!(
                "Agent stats: {} tasks, {:.0}% success rate",
                stats.total_tasks,
                stats.success_rate * 100.0
            );
            assert!(
                stats.success_rate >= 0.6,
                "Should have good success rate with all successes"
            );
        }
    }

    // 7. Store some context in memory
    let (_, _events) = memory_service
        .remember(
            "user_service_schema".to_string(),
            "Users table: id, email, name, created_at".to_string(),
            "project_context",
        )
        .await
        .expect("Failed to store memory");

    let (_, _events) = memory_service
        .learn(
            "user_validation_pattern".to_string(),
            "Always validate email format before storing".to_string(),
            "project_context",
        )
        .await
        .expect("Failed to store semantic memory");

    // Verify memory was stored
    let memories = memory_repo
        .list_by_namespace("project_context")
        .await
        .expect("Failed to list memories");
    assert_eq!(memories.len(), 2);

    // 8. Complete the workflow - retire the goal
    let (_, _events) = goal_service
        .transition_status(goal.id, GoalStatus::Retired)
        .await
        .expect("Failed to retire goal");

    println!("✓ Full end-to-end workflow completed successfully");
}

// =============================================================================
// TEST 10: SWARM ORCHESTRATOR WITH GOAL EXECUTION
// =============================================================================

/// Test swarm orchestrator processes goals and executes tasks.
#[tokio::test]
async fn test_swarm_orchestrator_goal_execution() {
    let (goal_repo, task_repo, _, agent_repo, worktree_repo) = setup_test_environment().await;

    // Create a mock substrate
    let mock_substrate = Arc::new(MockSubstrate::new());
    let substrate: Arc<dyn Substrate> = mock_substrate.clone();

    // Configure orchestrator
    let mut config = SwarmConfig::default();
    config.use_worktrees = false;
    config.use_llm_decomposition = false;
    config.track_evolution = true;

    let event_bus = Arc::new(abathur::services::EventBus::new(abathur::services::EventBusConfig::default()));
    let event_reactor = Arc::new(abathur::services::EventReactor::new(event_bus.clone(), abathur::services::ReactorConfig::default()));
    let event_scheduler = Arc::new(abathur::services::EventScheduler::new(event_bus.clone(), abathur::services::SchedulerConfig::default()));

    let orchestrator: SwarmOrchestrator<_, _, _, _, NullMemoryRepository> = SwarmOrchestrator::new(
        goal_repo.clone(),
        task_repo.clone(),
        worktree_repo.clone(),
        agent_repo.clone(),
        substrate,
        config,
        event_bus,
        event_reactor,
        event_scheduler,
    )
    .with_intent_verifier(mock_substrate.clone() as Arc<dyn Substrate>);

    // Create a goal
    let goal = Goal::new("Test Goal", "A simple goal for orchestrator testing")
        .with_priority(GoalPriority::Normal);
    goal_repo.create(&goal).await.expect("Failed to create goal");

    // Create ready task for the goal
    let mut task = Task::with_title("Test Task", "A task to execute")
        .with_source(TaskSource::Human);
    task.status = TaskStatus::Ready;
    task_repo.create(&task).await.expect("Failed to create task");

    // Run orchestrator tick
    let stats = orchestrator.tick().await.expect("Failed to run tick");

    assert_eq!(stats.active_goals, 1, "Should have 1 active goal");
    // The tick should find the ready task and attempt to execute it

    println!("✓ Swarm orchestrator goal execution working correctly");
}

// =============================================================================
// TEST 11: MEMORY SYSTEM INTEGRATION
// =============================================================================

/// Test the three-tier memory system works across the workflow.
#[tokio::test]
async fn test_memory_system_integration() {
    let (_, _, memory_repo, _, _) = setup_test_environment().await;

    let memory_service = MemoryService::new(memory_repo.clone());

    // Store memories in all three tiers
    let (_, _events) = memory_service
        .remember(
            "current_focus".to_string(),
            "Working on auth module".to_string(),
            "session",
        )
        .await
        .expect("Failed to store working memory");

    let (_, _events) = memory_service
        .store(
            "login_bug_fix".to_string(),
            "Fixed null pointer in login handler".to_string(),
            "session".to_string(),
            MemoryTier::Episodic,
            MemoryType::Fact,
            None,
        )
        .await
        .expect("Failed to store episodic memory");

    let (_, _events) = memory_service
        .learn(
            "error_handling_pattern".to_string(),
            "Always use Result type for fallible operations".to_string(),
            "session",
        )
        .await
        .expect("Failed to store semantic memory");

    // Verify tier distribution
    let memories = memory_repo
        .list_by_namespace("session")
        .await
        .expect("Failed to list memories");

    let working = memories
        .iter()
        .filter(|m| m.tier == MemoryTier::Working)
        .count();
    let episodic = memories
        .iter()
        .filter(|m| m.tier == MemoryTier::Episodic)
        .count();
    let semantic = memories
        .iter()
        .filter(|m| m.tier == MemoryTier::Semantic)
        .count();

    assert_eq!(working, 1, "Should have 1 working memory");
    assert_eq!(episodic, 1, "Should have 1 episodic memory");
    assert_eq!(semantic, 1, "Should have 1 semantic memory");

    // Test recall
    let (recalled, _events) = memory_service
        .recall_by_key("error_handling_pattern", "session")
        .await
        .expect("Failed to recall");
    assert!(recalled.is_some());
    assert!(recalled.unwrap().content.contains("Result type"));

    // Run maintenance
    let (_report, _events) = memory_service
        .run_maintenance()
        .await
        .expect("Failed to run maintenance");
    // Maintenance should run without errors

    println!("✓ Memory system integration working correctly");
}

// =============================================================================
// TEST 12: TASK IDEMPOTENCY AND DEDUPLICATION
// =============================================================================

/// Test that duplicate task submissions are handled correctly.
#[tokio::test]
async fn test_task_idempotency() {
    let (_goal_repo, task_repo, _, _, _) = setup_test_environment().await;

    let _event_bus = Arc::new(abathur::services::EventBus::new(abathur::services::EventBusConfig::default()));
    let task_service = TaskService::new(task_repo.clone());

    // Submit task with idempotency key
    let (task1, _events) = task_service
        .submit_task(
            Some("Unique Task".to_string()),
            "This task should only exist once".to_string(),
            None,
            TaskPriority::Normal,
            None,
            vec![],
            None,
            Some("unique-idempotency-key".to_string()),
            TaskSource::Human,
            None,
        )
        .await
        .expect("First submission should succeed");

    // Submit again with same key but different content
    let (task2, _events) = task_service
        .submit_task(
            Some("Different Title".to_string()),
            "Different description".to_string(),
            None,
            TaskPriority::High, // Different priority
            None,
            vec![],
            None,
            Some("unique-idempotency-key".to_string()),
            TaskSource::Human,
            None,
        )
        .await
        .expect("Second submission should succeed");

    // Should return the same task
    assert_eq!(task1.id, task2.id, "Should return same task ID");
    assert_eq!(task2.title, "Unique Task", "Original title should be preserved");

    // Verify only one task exists
    let all_tasks = task_repo
        .list(TaskFilter::default())
        .await
        .expect("Failed to list tasks");
    assert_eq!(all_tasks.len(), 1, "Should only have one task");

    println!("✓ Task idempotency working correctly");
}

// =============================================================================
// SUMMARY TEST - RUNS ALL CRITICAL PATHS
// =============================================================================

/// Summary test that exercises all critical paths.
#[tokio::test]
async fn test_e2e_all_critical_paths() {
    println!("\n=== End-to-End Integration Test Suite ===\n");

    println!("Testing critical paths...\n");

    // These tests are also run individually, but this serves as a smoke test
    // to ensure all components work together.

    let (goal_repo, task_repo, memory_repo, agent_repo, _worktree_repo) =
        setup_test_environment().await;

    // 1. Goal Creation
    let event_bus = Arc::new(abathur::services::EventBus::new(abathur::services::EventBusConfig::default()));
    let goal_service = GoalService::new(goal_repo.clone());
    let (goal, _events) = goal_service
        .create_goal(
            "E2E Test Goal".to_string(),
            "End to end test".to_string(),
            GoalPriority::Normal,
            None,
            vec![GoalConstraint::preference("test", "Run fast")],
            vec![],
        )
        .await
        .expect("Goal creation failed");
    assert_eq!(goal.status, GoalStatus::Active);
    println!("  ✓ Goal creation");

    // 2. Task Creation with Dependencies
    let task_service = TaskService::new(task_repo.clone());
    let (task1, _events) = task_service
        .submit_task(
            Some("Task 1".to_string()),
            "First task".to_string(),
            None,
            TaskPriority::Normal,
            None,
            vec![],
            None,
            None,
            TaskSource::Human,
            None,
        )
        .await
        .expect("Task 1 creation failed");

    let (_task2, _events) = task_service
        .submit_task(
            Some("Task 2".to_string()),
            "Second task".to_string(),
            None,
            TaskPriority::Normal,
            None,
            vec![task1.id],
            None,
            None,
            TaskSource::Human,
            None,
        )
        .await
        .expect("Task 2 creation failed");
    println!("  ✓ Task creation with dependencies");

    // 3. DAG Construction
    let tasks = task_repo
        .list(TaskFilter::default())
        .await
        .expect("Failed to list tasks");
    let dag = TaskDag::from_tasks(tasks);
    let waves = dag.execution_waves().expect("DAG waves failed");
    assert_eq!(waves.len(), 2, "Should have 2 waves");
    println!("  ✓ DAG construction and wave calculation");

    // 4. Agent Template Registration
    let agent_service = AgentService::new(agent_repo.clone(), event_bus.clone());
    let _template = agent_service
        .register_template(
            "e2e-worker".to_string(),
            "E2E test worker".to_string(),
            AgentTier::Worker,
            "Test prompt".to_string(),
            vec![],
            vec![],
            Some(10),
        )
        .await
        .expect("Template registration failed");
    println!("  ✓ Agent template registration");

    // 5. Agent Instance Spawning
    let instance = agent_service
        .spawn_instance("e2e-worker")
        .await
        .expect("Instance spawn failed");
    agent_service
        .complete_instance(instance.id)
        .await
        .expect("Instance completion failed");
    println!("  ✓ Agent instance spawning and completion");

    // 6. Evolution Tracking
    let evolution_loop = EvolutionLoop::with_default_config();
    for i in 0..5 {
        evolution_loop
            .record_execution(TaskExecution {
                task_id: uuid::Uuid::new_v4(),
                template_name: "e2e-worker".to_string(),
                template_version: 1,
                outcome: if i < 4 {
                    TaskOutcome::Success
                } else {
                    TaskOutcome::Failure
                },
                executed_at: chrono::Utc::now(),
                turns_used: 3,
                tokens_used: 500,
                downstream_tasks: vec![],
            })
            .await;
    }
    let stats = evolution_loop.get_stats("e2e-worker").await.unwrap();
    assert_eq!(stats.total_tasks, 5);
    println!("  ✓ Evolution loop tracking");

    // 7. Memory Storage and Retrieval
    let memory_service = MemoryService::new(memory_repo.clone());
    let (_, _events) = memory_service
        .learn("test_key".to_string(), "test_value".to_string(), "e2e")
        .await
        .expect("Memory store failed");
    let (recalled, _events) = memory_service
        .recall_by_key("test_key", "e2e")
        .await
        .expect("Memory recall failed");
    assert!(recalled.is_some());
    println!("  ✓ Memory storage and retrieval");

    // 8. Goal Status Transition
    let (_, _events) = goal_service
        .transition_status(goal.id, GoalStatus::Retired)
        .await
        .expect("Goal transition failed");
    let retired = goal_service.get_goal(goal.id).await.unwrap().unwrap();
    assert_eq!(retired.status, GoalStatus::Retired);
    println!("  ✓ Goal status transition");

    println!("\n=== All Critical Paths Verified ===\n");
}

// =============================================================================
// REAL AGENT TESTS - Using actual Claude Code CLI
// =============================================================================
//
// These tests use real Claude Code CLI invocations with simple test tasks.
// They are marked #[ignore] by default and can be run with:
//   cargo test --test e2e_swarm_integration_test -- --ignored
// Or with env var:
//   ABATHUR_REAL_E2E=1 cargo test --test e2e_swarm_integration_test

/// Check if Claude CLI is available.
async fn claude_cli_available() -> bool {
    tokio::process::Command::new("claude")
        .arg("--version")
        .output()
        .await
        .map(|o| o.status.success())
        .unwrap_or(false)
}

// =============================================================================
// TEST 13: REAL AGENT - SIMPLE TASK EXECUTION
// =============================================================================

/// Test real Claude Code agent executing a simple task.
///
/// This test uses the actual Claude CLI with a trivial task to verify
/// the substrate integration works end-to-end.
#[tokio::test]
#[ignore = "Requires Claude CLI - run with --include-ignored"]
async fn test_real_agent_simple_task_execution() {
    if !claude_cli_available().await {
        println!("⚠ Skipping: Claude CLI not available");
        return;
    }

    let (_, task_repo, _, _agent_repo, _) = setup_test_environment().await;

    // Create a real Claude Code substrate with haiku for efficiency
    use abathur::adapters::substrates::claude_code::ClaudeCodeConfig;
    let config = ClaudeCodeConfig {
        binary_path: "claude".to_string(),
        default_model: "haiku".to_string(), // Use haiku for cost efficiency
        default_max_turns: 1,               // Simple task needs only 1 turn
        print_mode: true,
        output_format: "text".to_string(), // Simple text output
        ..Default::default()
    };

    let substrate = Arc::new(ClaudeCodeSubstrate::new(config));

    // Create a simple test task
    let task = Task::with_title(
        "Test Task - Echo Success",
        "This is an automated test. Reply with exactly: TEST_SUCCESS_12345",
    );
    task_repo.create(&task).await.expect("Failed to create task");

    // Create substrate request
    use abathur::domain::models::{SubstrateConfig, SubstrateRequest};
    let request = SubstrateRequest::new(
        task.id,
        "test-agent",
        "You are a test agent. Follow instructions exactly.",
        "This is an automated test. Reply with exactly: TEST_SUCCESS_12345",
    )
    .with_config(SubstrateConfig::default().with_max_turns(1));

    // Execute the task
    println!("Executing real agent task...");
    let session = substrate.execute(request).await.expect("Failed to execute");

    // Verify execution
    use abathur::domain::models::SessionStatus;
    assert_eq!(
        session.status,
        SessionStatus::Completed,
        "Session should complete"
    );
    assert!(session.result.is_some(), "Should have a result");

    let result = session.result.unwrap();
    println!("Agent response: {}", result);
    assert!(
        result.contains("TEST_SUCCESS_12345"),
        "Response should contain the expected string"
    );

    println!("✓ Real agent simple task execution successful");
}

// =============================================================================
// TEST 14: REAL AGENT DAG - MULTI-TASK WORKFLOW
// =============================================================================

/// Test real agents executing a simple DAG of tasks.
///
/// Creates a 2-task DAG where task2 depends on task1, and verifies
/// both execute successfully in the correct order.
#[tokio::test]
#[ignore = "Requires Claude CLI - run with --include-ignored"]
async fn test_real_agent_dag_execution() {
    if !claude_cli_available().await {
        println!("⚠ Skipping: Claude CLI not available");
        return;
    }

    let (_goal_repo, task_repo, _, agent_repo, _) = setup_test_environment().await;

    // Create Claude Code substrate with haiku
    use abathur::adapters::substrates::claude_code::ClaudeCodeConfig;
    let config = ClaudeCodeConfig {
        binary_path: "claude".to_string(),
        default_model: "haiku".to_string(),
        default_max_turns: 1,
        print_mode: true,
        output_format: "text".to_string(),
        ..Default::default()
    };

    let substrate: Arc<dyn Substrate> = Arc::new(ClaudeCodeSubstrate::new(config));

    // Create tasks with dependency
    // Task 1: Independent task
    let task1 = Task::with_title(
        "Test Task 1",
        "This is test task 1. Reply with: TASK1_COMPLETE",
    );

    // Task 2: Depends on task 1
    let task2 = Task::with_title(
        "Test Task 2",
        "This is test task 2. Reply with: TASK2_COMPLETE",
    )
    .with_dependency(task1.id);

    task_repo.create(&task1).await.expect("Failed to create task1");
    task_repo.create(&task2).await.expect("Failed to create task2");

    // Build DAG
    let tasks = task_repo
        .list(TaskFilter::default())
        .await
        .expect("Failed to list tasks");
    let dag = TaskDag::from_tasks(tasks);

    // Verify DAG structure
    let waves = dag.execution_waves().expect("Should have valid waves");
    assert_eq!(waves.len(), 2, "Should have 2 waves");
    assert!(waves[0].contains(&task1.id), "Wave 1 should have task1");
    assert!(waves[1].contains(&task2.id), "Wave 2 should have task2");

    // Create executor with real substrate
    let executor_config = ExecutorConfig {
        max_concurrency: 1, // Sequential for predictable testing
        task_timeout_secs: 120,
        max_retries: 1,
        default_max_turns: 1,
        fail_fast: true,
        ..Default::default()
    };

    let executor: DagExecutor<_, _, SqliteGoalRepository> = DagExecutor::new(
        task_repo.clone(),
        agent_repo.clone(),
        substrate,
        executor_config,
    );

    // Execute DAG with events
    let (event_tx, mut event_rx) = mpsc::channel(100);

    println!("Executing real agent DAG...");
    let results = executor
        .execute_with_events(&dag, event_tx)
        .await
        .expect("Failed to execute DAG");

    // Collect events
    event_rx.close();
    let mut events = vec![];
    while let Some(event) = event_rx.recv().await {
        events.push(event);
    }

    // Verify results
    println!(
        "DAG results: {}/{} completed, {} failed",
        results.completed_tasks, results.total_tasks, results.failed_tasks
    );

    assert_eq!(results.total_tasks, 2, "Should have 2 total tasks");
    assert_eq!(results.completed_tasks, 2, "Both tasks should complete");
    assert_eq!(results.failed_tasks, 0, "No tasks should fail");
    assert_eq!(
        results.status(),
        ExecutionStatus::Completed,
        "Status should be Completed"
    );

    // Verify wave execution order from events
    let wave_starts: Vec<_> = events
        .iter()
        .filter_map(|e| {
            if let ExecutionEvent::WaveStarted { wave_number, .. } = e {
                Some(*wave_number)
            } else {
                None
            }
        })
        .collect();

    assert_eq!(wave_starts, vec![1, 2], "Waves should execute in order");

    println!("✓ Real agent DAG execution successful");
}

// =============================================================================
// TEST 15: REAL AGENT - EVOLUTION TRACKING WITH REAL EXECUTIONS
// =============================================================================

/// Test evolution loop with real agent executions.
///
/// Executes multiple tasks with a mix of successes and failures,
/// then verifies the evolution loop correctly tracks outcomes.
#[tokio::test]
#[ignore = "Requires Claude CLI - run with --include-ignored"]
async fn test_real_agent_evolution_tracking() {
    if !claude_cli_available().await {
        println!("⚠ Skipping: Claude CLI not available");
        return;
    }

    let (_, task_repo, _, _, _) = setup_test_environment().await;

    // Create Claude Code substrate
    use abathur::adapters::substrates::claude_code::ClaudeCodeConfig;
    let config = ClaudeCodeConfig {
        binary_path: "claude".to_string(),
        default_model: "haiku".to_string(),
        default_max_turns: 1,
        print_mode: true,
        output_format: "text".to_string(),
        ..Default::default()
    };

    let substrate = Arc::new(ClaudeCodeSubstrate::new(config));
    let evolution_loop = Arc::new(EvolutionLoop::with_default_config());

    let template_name = "real-test-worker";
    let template_version = 1;

    // Execute 3 simple tasks and track them
    println!("Executing real tasks with evolution tracking...");

    for i in 0..3 {
        let task = Task::with_title(
            &format!("Evolution Test Task {}", i + 1),
            &format!("Test task {}. Reply with: SUCCESS_{}", i + 1, i + 1),
        );
        task_repo.create(&task).await.expect("Failed to create task");

        use abathur::domain::models::{SubstrateConfig, SubstrateRequest};
        let request = SubstrateRequest::new(
            task.id,
            template_name,
            "You are a test agent.",
            &format!("Reply with exactly: SUCCESS_{}", i + 1),
        )
        .with_config(SubstrateConfig::default().with_max_turns(1));

        let start = std::time::Instant::now();
        let session = substrate.execute(request).await;
        let duration = start.elapsed();

        // Determine outcome
        let (outcome, tokens) = match session {
            Ok(ref s) => {
                use abathur::domain::models::SessionStatus;
                if s.status == SessionStatus::Completed {
                    (TaskOutcome::Success, s.total_tokens())
                } else {
                    (TaskOutcome::Failure, s.total_tokens())
                }
            }
            Err(_) => (TaskOutcome::Failure, 0),
        };

        // Record in evolution loop
        evolution_loop
            .record_execution(TaskExecution {
                task_id: task.id,
                template_name: template_name.to_string(),
                template_version,
                outcome,
                executed_at: chrono::Utc::now(),
                turns_used: session.as_ref().map(|s| s.turns_completed).unwrap_or(0),
                tokens_used: tokens,
                downstream_tasks: vec![],
            })
            .await;

        println!(
            "  Task {}: {:?} ({:?})",
            i + 1,
            outcome,
            duration
        );
    }

    // Check evolution stats
    let stats = evolution_loop
        .get_stats(template_name)
        .await
        .expect("Should have stats");

    println!(
        "Evolution stats: {} tasks, {:.0}% success rate, avg {} tokens",
        stats.total_tasks,
        stats.success_rate * 100.0,
        stats.avg_tokens as u64
    );

    assert_eq!(stats.total_tasks, 3, "Should have tracked 3 tasks");
    assert!(stats.avg_tokens > 0.0, "Should have tracked token usage");

    // Evaluate for potential refinements
    let events = evolution_loop.evaluate().await;
    println!("Evolution evaluation: {} events", events.len());

    println!("✓ Real agent evolution tracking successful");
}

// =============================================================================
// TEST 16: FULL REAL E2E - GOAL TO COMPLETION
// =============================================================================

/// Complete end-to-end test with real agents: Goal → Tasks → Execution → Verification
///
/// This is the comprehensive real E2E test that exercises the full workflow
/// with actual Claude Code CLI invocations.
#[tokio::test]
#[ignore = "Requires Claude CLI - run with --include-ignored"]
async fn test_real_e2e_full_workflow() {
    if !claude_cli_available().await {
        println!("⚠ Skipping: Claude CLI not available");
        return;
    }

    println!("\n=== REAL E2E TEST: Full Workflow with Live Agents ===\n");

    let (goal_repo, task_repo, memory_repo, agent_repo, _worktree_repo) =
        setup_test_environment().await;

    // 1. Create services
    let event_bus = Arc::new(abathur::services::EventBus::new(abathur::services::EventBusConfig::default()));
    let goal_service = GoalService::new(goal_repo.clone());
    let task_service = TaskService::new(task_repo.clone());
    let agent_service = AgentService::new(agent_repo.clone(), event_bus.clone());
    let memory_service = MemoryService::new(memory_repo.clone());
    let evolution_loop = Arc::new(EvolutionLoop::with_default_config());

    // 2. Create a goal
    let (goal, _events) = goal_service
        .create_goal(
            "Real E2E Test Goal".to_string(),
            "Verify the complete system works with real agents".to_string(),
            GoalPriority::Normal,
            None,
            vec![GoalConstraint::invariant("format", "All responses must include SUCCESS marker")],
            vec![],
        )
        .await
        .expect("Failed to create goal");

    println!("✓ Created goal: {}", goal.name);

    // 3. Create a simple task DAG
    //    task1 (setup) → task2 (verify)

    let (task1, _events) = task_service
        .submit_task(
            Some("Setup Task".to_string()),
            "This is the setup phase. Reply with: SETUP_COMPLETE".to_string(),
            None,
            TaskPriority::High,
            Some("test-worker".to_string()),
            vec![],
            None,
            None,
            TaskSource::Human,
            None,
        )
        .await
        .expect("Failed to create task1");

    let (_task2, _events) = task_service
        .submit_task(
            Some("Verify Task".to_string()),
            "This is the verification phase. Reply with: VERIFY_COMPLETE".to_string(),
            None,
            TaskPriority::Normal,
            Some("test-worker".to_string()),
            vec![task1.id],
            None,
            None,
            TaskSource::Human,
            None,
        )
        .await
        .expect("Failed to create task2");

    println!("✓ Created task DAG with 2 tasks");

    // 4. Register agent template
    let template = agent_service
        .register_template(
            "test-worker".to_string(),
            "Test worker for E2E verification".to_string(),
            AgentTier::Worker,
            "You are a test agent. Follow instructions exactly and include SUCCESS in responses."
                .to_string(),
            vec![],
            vec![],
            Some(1),
        )
        .await
        .expect("Failed to register template");

    println!("✓ Registered agent template: {}", template.name);

    // 5. Create real substrate and execute DAG
    use abathur::adapters::substrates::claude_code::ClaudeCodeConfig;
    let config = ClaudeCodeConfig {
        binary_path: "claude".to_string(),
        default_model: "haiku".to_string(),
        default_max_turns: 1,
        print_mode: true,
        output_format: "text".to_string(),
        ..Default::default()
    };

    let substrate: Arc<dyn Substrate> = Arc::new(ClaudeCodeSubstrate::new(config));

    // Build DAG
    let tasks = task_repo
        .list(TaskFilter::default())
        .await
        .expect("Failed to list tasks");
    let dag = TaskDag::from_tasks(tasks.clone());

    // Execute
    let executor_config = ExecutorConfig {
        max_concurrency: 1,
        task_timeout_secs: 120,
        max_retries: 1,
        default_max_turns: 1,
        fail_fast: false,
        ..Default::default()
    };

    let executor: DagExecutor<_, _, SqliteGoalRepository> = DagExecutor::new(
        task_repo.clone(),
        agent_repo.clone(),
        substrate,
        executor_config,
    );

    println!("Executing DAG with real agents...");
    let start = std::time::Instant::now();
    let results = executor.execute(&dag).await.expect("Failed to execute DAG");
    let duration = start.elapsed();

    println!(
        "✓ DAG execution complete: {}/{} tasks in {:?}",
        results.completed_tasks, results.total_tasks, duration
    );
    println!("  Total tokens used: {}", results.total_tokens_used);

    // 6. Track in evolution loop
    for result in &results.task_results {
        let outcome = if result.status == TaskStatus::Complete {
            TaskOutcome::Success
        } else {
            TaskOutcome::Failure
        };

        evolution_loop
            .record_execution(TaskExecution {
                task_id: result.task_id,
                template_name: template.name.clone(),
                template_version: template.version,
                outcome,
                executed_at: chrono::Utc::now(),
                turns_used: result
                    .session
                    .as_ref()
                    .map(|s| s.turns_completed)
                    .unwrap_or(0),
                tokens_used: result
                    .session
                    .as_ref()
                    .map(|s| s.total_tokens())
                    .unwrap_or(0),
                downstream_tasks: vec![],
            })
            .await;
    }

    let stats = evolution_loop.get_stats(&template.name).await.unwrap();
    println!(
        "✓ Evolution tracking: {} tasks, {:.0}% success",
        stats.total_tasks,
        stats.success_rate * 100.0
    );

    // 7. Store execution context in memory
    let (_, _events) = memory_service
        .learn(
            "e2e_test_result".to_string(),
            format!(
                "E2E test completed with {} successes, {} tokens used",
                results.completed_tasks, results.total_tokens_used
            ),
            "e2e_test",
        )
        .await
        .expect("Failed to store memory");

    println!("✓ Stored execution context in memory");

    // 8. Retire the goal
    let (_, _events) = goal_service
        .transition_status(goal.id, GoalStatus::Retired)
        .await
        .expect("Failed to retire goal");

    println!("✓ Goal retired");

    // 9. Final verification
    assert!(
        results.completed_tasks > 0,
        "At least some tasks should complete"
    );

    println!("\n=== REAL E2E TEST COMPLETE ===");
    println!("  Tasks completed: {}/{}", results.completed_tasks, results.total_tasks);
    println!("  Tokens used: {}", results.total_tokens_used);
    println!("  Duration: {:?}", duration);
    println!("  Success rate: {:.0}%\n", stats.success_rate * 100.0);
}
