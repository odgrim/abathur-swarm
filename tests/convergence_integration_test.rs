//! Integration tests for the convergence-task integration layer.
//!
//! These tests verify the bridge between the task system and the convergence
//! engine, including execution mode classification, prompt construction,
//! engine configuration, event emission, and trajectory repository delegation.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use uuid::Uuid;

use abathur::adapters::sqlite::{create_migrated_test_pool, SqliteTaskRepository};
use abathur::domain::errors::DomainResult;
use abathur::domain::models::convergence::{
    ArtifactReference, AttractorType, ConvergenceBudget, ConvergenceTendency,
    ConvergencePolicy, OverseerSignals, BuildResult, TestResults, Observation,
    PriorityHint, SpecificationEvolution, SpecificationSnapshot, StrategyEntry,
    StrategyKind, Trajectory,
};
use abathur::domain::models::task::{Complexity, ExecutionMode, Task, TaskPriority};
use abathur::domain::ports::{StrategyStats, TaskRepository, TrajectoryRepository};
use abathur::services::convergence_bridge::{
    build_convergent_prompt, build_engine_config, collect_artifact, task_to_submission,
    DynTrajectoryRepository,
};
use abathur::services::event_bus::{
    EventBus, EventBusConfig, EventCategory, EventPayload, EventSeverity,
};
use abathur::services::event_factory;
use abathur::services::swarm_orchestrator::types::SwarmConfig;
use abathur::services::TaskService;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

async fn setup_task_repo() -> Arc<SqliteTaskRepository> {
    let pool = create_migrated_test_pool()
        .await
        .expect("Failed to create test pool");
    Arc::new(SqliteTaskRepository::new(pool))
}

/// A minimal in-memory TrajectoryRepository for testing delegation.
struct InMemoryTrajectoryRepo {
    saved: tokio::sync::Mutex<Vec<Trajectory>>,
}

impl InMemoryTrajectoryRepo {
    fn new() -> Self {
        Self {
            saved: tokio::sync::Mutex::new(Vec::new()),
        }
    }
}

#[async_trait]
impl TrajectoryRepository for InMemoryTrajectoryRepo {
    async fn save(&self, trajectory: &Trajectory) -> DomainResult<()> {
        self.saved.lock().await.push(trajectory.clone());
        Ok(())
    }

    async fn get(&self, trajectory_id: &str) -> DomainResult<Option<Trajectory>> {
        let guard = self.saved.lock().await;
        let id: Uuid = trajectory_id.parse().unwrap_or_default();
        Ok(guard.iter().find(|t| t.id == id).cloned())
    }

    async fn get_by_task(&self, task_id: &str) -> DomainResult<Vec<Trajectory>> {
        let guard = self.saved.lock().await;
        let id: Uuid = task_id.parse().unwrap_or_default();
        Ok(guard.iter().filter(|t| t.task_id == id).cloned().collect())
    }

    async fn get_by_goal(&self, goal_id: &str) -> DomainResult<Vec<Trajectory>> {
        let guard = self.saved.lock().await;
        let id: Uuid = goal_id.parse().unwrap_or_default();
        Ok(guard
            .iter()
            .filter(|t| t.goal_id == Some(id))
            .cloned()
            .collect())
    }

    async fn get_recent(&self, limit: usize) -> DomainResult<Vec<Trajectory>> {
        let guard = self.saved.lock().await;
        Ok(guard.iter().rev().take(limit).cloned().collect())
    }

    async fn get_successful_strategies(
        &self,
        _attractor_type: &AttractorType,
        _limit: usize,
    ) -> DomainResult<Vec<StrategyEntry>> {
        Ok(vec![])
    }

    async fn delete(&self, trajectory_id: &str) -> DomainResult<()> {
        let mut guard = self.saved.lock().await;
        let id: Uuid = trajectory_id.parse().unwrap_or_default();
        guard.retain(|t| t.id != id);
        Ok(())
    }

    async fn avg_iterations_by_complexity(&self, _complexity: Complexity) -> DomainResult<f64> {
        Ok(0.0)
    }

    async fn strategy_effectiveness(
        &self,
        _strategy: StrategyKind,
    ) -> DomainResult<StrategyStats> {
        Ok(StrategyStats {
            strategy: String::new(),
            total_uses: 0,
            success_count: 0,
            average_delta: 0.0,
            average_tokens: 0,
        })
    }

    async fn attractor_distribution(&self) -> DomainResult<HashMap<String, u32>> {
        Ok(HashMap::new())
    }

    async fn convergence_rate_by_task_type(&self, _category: &str) -> DomainResult<f64> {
        Ok(0.0)
    }

    async fn get_similar_trajectories(
        &self,
        _description: &str,
        _tags: &[String],
        _limit: usize,
    ) -> DomainResult<Vec<Trajectory>> {
        Ok(vec![])
    }
}

fn make_trajectory(task_id: Uuid, goal_id: Option<Uuid>) -> Trajectory {
    let spec = SpecificationEvolution::new(SpecificationSnapshot::new(
        "Implement the frobnicator module".into(),
    ));
    Trajectory::new(
        task_id,
        goal_id,
        spec,
        ConvergenceBudget::default(),
        ConvergencePolicy::default(),
    )
}

// ---------------------------------------------------------------------------
// 1. test_task_to_submission_roundtrip
// ---------------------------------------------------------------------------

/// Verify that task_to_submission correctly maps Task fields into a
/// TaskSubmission, including goal linkage, complexity, execution mode,
/// constraints, anti-patterns, relevant files, and priority hints.
#[tokio::test]
async fn test_task_to_submission_roundtrip() {
    let goal_id = Uuid::new_v4();
    let mut task = Task::new("Implement OAuth2 login flow with acceptance criteria")
        .with_priority(TaskPriority::Critical)
        .with_execution_mode(ExecutionMode::Convergent {
            parallel_samples: Some(3),
        });

    // Add hints
    task.context
        .hints
        .push("constraint: must use async".to_string());
    task.context
        .hints
        .push("anti-pattern: no unwrap()".to_string());

    // Add relevant files
    task.context
        .relevant_files
        .push("src/auth/oauth.rs".to_string());
    task.context
        .relevant_files
        .push("src/auth/mod.rs".to_string());

    // Set complexity
    task.routing_hints.complexity = Complexity::Complex;

    let submission = task_to_submission(&task, Some(goal_id));

    // Goal linkage
    assert_eq!(submission.goal_id, Some(goal_id));

    // Complexity propagation
    assert_eq!(submission.inferred_complexity, Complexity::Complex);

    // Parallel samples from convergent mode
    assert_eq!(submission.parallel_samples, Some(3));

    // Constraints extracted from hints
    assert_eq!(submission.constraints.len(), 1);
    assert!(submission.constraints[0].contains("must use async"));

    // Anti-patterns extracted from hints
    assert_eq!(submission.anti_patterns.len(), 1);
    assert!(submission.anti_patterns[0].contains("no unwrap()"));

    // Relevant files mapped to references
    assert_eq!(submission.references.len(), 2);
    assert_eq!(submission.references[0].path, "src/auth/oauth.rs");
    assert_eq!(submission.references[1].path, "src/auth/mod.rs");

    // Priority hint from Critical -> Thorough
    assert_eq!(submission.priority_hint, Some(PriorityHint::Thorough));

    // Description preserved
    assert_eq!(
        submission.description,
        "Implement OAuth2 login flow with acceptance criteria"
    );
}

// ---------------------------------------------------------------------------
// 2. test_classify_execution_mode_complex_gets_convergent
// ---------------------------------------------------------------------------

/// Verify that tasks with signals strongly suggesting convergent execution
/// (Complex complexity, acceptance keywords) are inferred as Convergent
/// when submitted through the TaskService.
#[tokio::test]
async fn test_classify_execution_mode_complex_gets_convergent() {
    let task_repo = setup_task_repo().await;
    let task_service = TaskService::new(task_repo.clone());

    // Submit a task whose description and complexity strongly signal convergent
    let mut context = abathur::domain::models::task::TaskContext::default();
    context
        .hints
        .push("constraint: must use async".to_string());
    context
        .hints
        .push("anti-pattern: no unwrap()".to_string());

    let (task, _events) = task_service
        .submit_task(
            Some("Complex task".to_string()),
            // Include acceptance keywords to push score higher
            "Implement a complex module with acceptance criteria. Verify that the output matches expected output and ensure that all test cases pass.".to_string(),
            None,
            TaskPriority::Normal,
            None,
            vec![],
            Some(context),
            None,
            abathur::domain::models::task::TaskSource::System,
            None,
            None,
        )
        .await
        .expect("Failed to submit task");

    // The heuristic should have classified this as Convergent because:
    // - acceptance keywords ("acceptance criteria", "test case", "expected output", "verify that", "ensure that") += 2
    // - anti-pattern + constraint hints += 2
    // Total >= 4, threshold is 3 => Convergent
    assert!(
        task.execution_mode.is_convergent(),
        "Expected Convergent execution mode, got {:?}",
        task.execution_mode
    );
}

// ---------------------------------------------------------------------------
// 3. test_classify_execution_mode_trivial_stays_direct
// ---------------------------------------------------------------------------

/// Verify that simple tasks without convergent signals remain in Direct mode
/// after submission.
#[tokio::test]
async fn test_classify_execution_mode_trivial_stays_direct() {
    let task_repo = setup_task_repo().await;
    let task_service = TaskService::new(task_repo.clone());

    let (task, _events) = task_service
        .submit_task(
            Some("Fix typo".to_string()),
            "Fix a typo in README.md".to_string(),
            None,
            TaskPriority::Low,
            None,
            vec![],
            None,
            None,
            abathur::domain::models::task::TaskSource::System,
            None,
            None,
        )
        .await
        .expect("Failed to submit task");

    // The heuristic should keep this as Direct because:
    // - No acceptance keywords, no hints, no complexity signal
    // - Low priority -= 2
    // Total is well below threshold
    assert!(
        task.execution_mode.is_direct(),
        "Expected Direct execution mode, got {:?}",
        task.execution_mode
    );
}

// ---------------------------------------------------------------------------
// 4. test_build_convergent_prompt_includes_strategy_context
// ---------------------------------------------------------------------------

/// Verify that build_convergent_prompt produces a prompt with strategy-specific
/// sections: specification content, strategy instructions, and overseer feedback.
#[tokio::test]
async fn test_build_convergent_prompt_includes_strategy_context() {
    let task = Task::new("Implement frobnicator");
    let task_id = task.id;

    // Build a trajectory whose specification matches the task description.
    // build_convergent_prompt uses trajectory.specification.effective.content,
    // not the task description directly.
    let spec = SpecificationEvolution::new(SpecificationSnapshot::new(
        "Implement frobnicator".into(),
    ));
    let mut trajectory = Trajectory::new(
        task_id,
        None,
        spec,
        ConvergenceBudget::default(),
        ConvergencePolicy::default(),
    );

    // Add an observation with failing tests so RetryWithFeedback has feedback to include
    let mut signals = OverseerSignals::empty();
    signals.test_results = Some(TestResults {
        passed: 7,
        failed: 3,
        skipped: 0,
        total: 10,
        regression_count: 0,
        failing_test_names: vec![
            "test_parse_input".to_string(),
            "test_validate_output".to_string(),
            "test_edge_case".to_string(),
        ],
    });
    signals.build_result = Some(BuildResult {
        success: false,
        error_count: 2,
        errors: vec![
            "cannot find type `Frobnicator`".to_string(),
            "missing lifetime specifier".to_string(),
        ],
    });

    let obs = Observation::new(
        0,
        ArtifactReference::new("/tmp/worktree", "hash456"),
        signals,
        StrategyKind::RetryWithFeedback,
        10000,
        2000,
    );
    trajectory.observations.push(obs);

    // Test RetryWithFeedback -- should include previous attempt feedback
    let prompt = build_convergent_prompt(&task, &trajectory, &StrategyKind::RetryWithFeedback, None);
    assert!(
        prompt.contains("Implement frobnicator"),
        "Prompt should include the specification content"
    );
    assert!(
        prompt.contains("Previous attempt feedback"),
        "RetryWithFeedback should include feedback section"
    );
    assert!(
        prompt.contains("7/10 passed"),
        "Feedback should include test results summary"
    );
    assert!(
        prompt.contains("Failing tests that must pass"),
        "Prompt should list failing tests as acceptance criteria"
    );
    assert!(
        prompt.contains("test_parse_input"),
        "Prompt should name the specific failing tests"
    );
    assert!(
        prompt.contains("Build errors that must be fixed"),
        "Prompt should include build errors"
    );
    assert!(
        prompt.contains("cannot find type `Frobnicator`"),
        "Prompt should include specific build error text"
    );

    // Test IncrementalRefinement -- should include refinement instructions
    let prompt =
        build_convergent_prompt(&task, &trajectory, &StrategyKind::IncrementalRefinement, None);
    assert!(
        prompt.contains("partially correct"),
        "IncrementalRefinement should include refinement guidance"
    );

    // Test Reframe -- should include reframe instructions
    let prompt = build_convergent_prompt(&task, &trajectory, &StrategyKind::Reframe, None);
    assert!(
        prompt.contains("Reconsider the approach"),
        "Reframe should include reconsideration guidance"
    );

    // Test Decompose -- should include decomposition instructions
    let prompt = build_convergent_prompt(&task, &trajectory, &StrategyKind::Decompose, None);
    assert!(
        prompt.contains("Break it into"),
        "Decompose should include decomposition guidance"
    );
}

// ---------------------------------------------------------------------------
// 5. test_build_engine_config_from_swarm_config
// ---------------------------------------------------------------------------

/// Verify that build_engine_config correctly maps SwarmConfig convergence
/// settings into a ConvergenceEngineConfig.
#[tokio::test]
async fn test_build_engine_config_from_swarm_config() {
    let mut config = SwarmConfig::default();
    config.convergence.min_confidence_threshold = 0.85;
    config.convergence.auto_retry_partial = false;
    config.polling.task_learning_enabled = false;

    let engine_config = build_engine_config(&config);

    // acceptance_threshold should map from min_confidence_threshold
    assert!(
        (engine_config.default_policy.acceptance_threshold - 0.85).abs() < f64::EPSILON,
        "acceptance_threshold should be 0.85, got {}",
        engine_config.default_policy.acceptance_threshold
    );

    // partial_acceptance should map from auto_retry_partial
    assert!(
        !engine_config.default_policy.partial_acceptance,
        "partial_acceptance should be false"
    );

    // memory_enabled from polling.task_learning_enabled
    assert!(
        !engine_config.memory_enabled,
        "memory_enabled should be false"
    );

    // Fixed defaults
    assert_eq!(engine_config.max_parallel_trajectories, 3);
    assert!(engine_config.enable_proactive_decomposition);
    assert!(engine_config.event_emission_enabled);
}

// ---------------------------------------------------------------------------
// 6. test_convergent_outcome_mapping
// ---------------------------------------------------------------------------

/// Verify that TaskService maps convergent execution outcomes correctly:
/// complete_task emits TaskExecutionRecorded with "convergent" mode,
/// and fail_task emits the failure equivalent.
#[tokio::test]
async fn test_convergent_outcome_mapping() {
    let task_repo = setup_task_repo().await;
    let task_service = TaskService::new(task_repo.clone());

    // Submit a task explicitly set to Convergent
    let (task, _events) = task_service
        .submit_task(
            Some("Convergent task".to_string()),
            "A convergent task".to_string(),
            None,
            TaskPriority::Normal,
            None,
            vec![],
            None,
            None,
            abathur::domain::models::task::TaskSource::System,
            None,
            None,
        )
        .await
        .expect("Failed to submit task");

    // Directly set it to convergent mode and save
    let mut task_mut = task.clone();
    task_mut.execution_mode = ExecutionMode::Convergent {
        parallel_samples: None,
    };
    task_repo.update(&task_mut).await.expect("Failed to update task");

    // Claim the task (Ready -> Running)
    let (_, _events) = task_service
        .claim_task(task.id, "test-agent")
        .await
        .expect("Failed to claim task");

    // Complete the task
    let (completed_task, events) = task_service
        .complete_task(task.id)
        .await
        .expect("Failed to complete task");

    // Verify the task completed
    assert_eq!(
        completed_task.status,
        abathur::domain::models::task::TaskStatus::Complete
    );

    // Find the TaskExecutionRecorded event
    let exec_recorded = events
        .iter()
        .find(|e| matches!(e.payload, EventPayload::TaskExecutionRecorded { .. }))
        .expect("Should emit TaskExecutionRecorded event on completion");

    if let EventPayload::TaskExecutionRecorded {
        execution_mode,
        succeeded,
        ..
    } = &exec_recorded.payload
    {
        assert_eq!(execution_mode, "convergent");
        assert!(succeeded, "Should indicate success on complete_task");
    }

    // Now test the failure path: submit a new convergent task and fail it
    let (task2, _) = task_service
        .submit_task(
            Some("Failing convergent task".to_string()),
            "Will fail".to_string(),
            None,
            TaskPriority::Normal,
            None,
            vec![],
            None,
            None,
            abathur::domain::models::task::TaskSource::System,
            None,
            None,
        )
        .await
        .expect("Failed to submit task");

    let mut task2_mut = task2.clone();
    task2_mut.execution_mode = ExecutionMode::Convergent {
        parallel_samples: None,
    };
    task_repo.update(&task2_mut).await.expect("Failed to update task");

    let _ = task_service
        .claim_task(task2.id, "test-agent")
        .await
        .expect("Failed to claim task");

    let (_, fail_events) = task_service
        .fail_task(task2.id, Some("trapped in limit cycle".to_string()))
        .await
        .expect("Failed to fail task");

    let fail_recorded = fail_events
        .iter()
        .find(|e| matches!(e.payload, EventPayload::TaskExecutionRecorded { .. }))
        .expect("Should emit TaskExecutionRecorded event on failure");

    if let EventPayload::TaskExecutionRecorded {
        execution_mode,
        succeeded,
        ..
    } = &fail_recorded.payload
    {
        assert_eq!(execution_mode, "convergent");
        assert!(!succeeded, "Should indicate failure on fail_task");
    }
}

// ---------------------------------------------------------------------------
// 7. test_task_retry_preserves_trajectory_id
// ---------------------------------------------------------------------------

/// Verify that retrying a convergent task preserves its trajectory_id so
/// the convergence engine can resume the existing trajectory rather than
/// starting from scratch. Also verify that trapped tasks get a fresh_start hint.
#[tokio::test]
async fn test_task_retry_preserves_trajectory_id() {
    let task_repo = setup_task_repo().await;
    let task_service = TaskService::new(task_repo.clone());

    // Submit a task
    let (task, _events) = task_service
        .submit_task(
            Some("Convergent retry test".to_string()),
            "A convergent task to retry".to_string(),
            None,
            TaskPriority::Normal,
            None,
            vec![],
            None,
            None,
            abathur::domain::models::task::TaskSource::System,
            None,
            None,
        )
        .await
        .expect("Failed to submit task");

    // Manually set convergent mode and trajectory_id
    let trajectory_id = Uuid::new_v4();
    let mut task_mut = task.clone();
    task_mut.execution_mode = ExecutionMode::Convergent {
        parallel_samples: None,
    };
    task_mut.trajectory_id = Some(trajectory_id);
    task_repo.update(&task_mut).await.expect("Failed to update");

    // Claim and fail the task
    let _ = task_service
        .claim_task(task.id, "test-agent")
        .await
        .expect("Failed to claim");

    let _ = task_service
        .fail_task(task.id, Some("Error: trapped in limit cycle".to_string()))
        .await
        .expect("Failed to fail task");

    // Retry the task
    let (retried, _events) = task_service
        .retry_task(task.id)
        .await
        .expect("Failed to retry task");

    // The trajectory_id should be preserved
    assert_eq!(
        retried.trajectory_id,
        Some(trajectory_id),
        "trajectory_id should be preserved across retries"
    );

    // Because the failure mentioned "trapped", the retry should add a fresh_start hint
    let has_fresh_start = retried
        .context
        .hints
        .iter()
        .any(|h| h.contains("convergence:fresh_start"));
    assert!(
        has_fresh_start,
        "Trapped convergent task retry should add convergence:fresh_start hint"
    );
}

// ---------------------------------------------------------------------------
// 8. test_convergence_events_emitted
// ---------------------------------------------------------------------------

/// Verify that convergence events can be published and received through the
/// EventBus. This tests the event wiring rather than the engine itself.
#[tokio::test]
async fn test_convergence_events_emitted() {
    let event_bus = Arc::new(EventBus::new(EventBusConfig::default()));
    let mut receiver = event_bus.subscribe();

    let task_id = Uuid::new_v4();
    let trajectory_id = Uuid::new_v4();

    // Publish a ConvergenceStarted event
    let started_event = event_factory::make_event(
        EventSeverity::Info,
        EventCategory::Convergence,
        None,
        Some(task_id),
        EventPayload::ConvergenceStarted {
            task_id,
            trajectory_id,
            estimated_iterations: 5,
            basin_width: "moderate".to_string(),
            convergence_mode: "iterative".to_string(),
        },
    );
    event_bus.publish(started_event).await;

    // Publish a ConvergenceIteration event
    let iter_event = event_factory::make_event(
        EventSeverity::Info,
        EventCategory::Convergence,
        None,
        Some(task_id),
        EventPayload::ConvergenceIteration {
            task_id,
            trajectory_id,
            iteration: 1,
            strategy: "retry_with_feedback".to_string(),
            convergence_delta: 0.15,
            convergence_level: 0.45,
            attractor_type: "indeterminate".to_string(),
            budget_remaining_fraction: 0.8,
        },
    );
    event_bus.publish(iter_event).await;

    // Publish a ConvergenceTerminated event
    let term_event = event_factory::make_event(
        EventSeverity::Info,
        EventCategory::Convergence,
        None,
        Some(task_id),
        EventPayload::ConvergenceTerminated {
            task_id,
            trajectory_id,
            outcome: "converged".to_string(),
            total_iterations: 3,
            total_tokens: 45000,
            final_convergence_level: 0.97,
        },
    );
    event_bus.publish(term_event).await;

    // Receive and verify all three events
    let received_started = receiver
        .recv()
        .await
        .expect("Should receive ConvergenceStarted event");
    assert!(matches!(
        received_started.payload,
        EventPayload::ConvergenceStarted { .. }
    ));
    assert_eq!(received_started.category, EventCategory::Convergence);
    if let EventPayload::ConvergenceStarted {
        task_id: recv_task_id,
        trajectory_id: recv_traj_id,
        estimated_iterations,
        ..
    } = received_started.payload
    {
        assert_eq!(recv_task_id, task_id);
        assert_eq!(recv_traj_id, trajectory_id);
        assert_eq!(estimated_iterations, 5);
    }

    let received_iter = receiver
        .recv()
        .await
        .expect("Should receive ConvergenceIteration event");
    assert!(matches!(
        received_iter.payload,
        EventPayload::ConvergenceIteration { .. }
    ));
    if let EventPayload::ConvergenceIteration {
        iteration,
        strategy,
        convergence_delta,
        ..
    } = received_iter.payload
    {
        assert_eq!(iteration, 1);
        assert_eq!(strategy, "retry_with_feedback");
        assert!((convergence_delta - 0.15).abs() < f64::EPSILON);
    }

    let received_term = receiver
        .recv()
        .await
        .expect("Should receive ConvergenceTerminated event");
    assert!(matches!(
        received_term.payload,
        EventPayload::ConvergenceTerminated { .. }
    ));
    if let EventPayload::ConvergenceTerminated {
        outcome,
        total_iterations,
        final_convergence_level,
        ..
    } = received_term.payload
    {
        assert_eq!(outcome, "converged");
        assert_eq!(total_iterations, 3);
        assert!((final_convergence_level - 0.97).abs() < f64::EPSILON);
    }
}

// ---------------------------------------------------------------------------
// 9. test_dyn_trajectory_repository_delegation
// ---------------------------------------------------------------------------

/// Verify that DynTrajectoryRepository correctly delegates all
/// TrajectoryRepository methods to the inner Arc<dyn TrajectoryRepository>.
#[tokio::test]
async fn test_dyn_trajectory_repository_delegation() {
    let inner = Arc::new(InMemoryTrajectoryRepo::new());
    let dyn_repo = DynTrajectoryRepository(inner.clone());

    let task_id = Uuid::new_v4();
    let goal_id = Uuid::new_v4();

    // Create and save a trajectory through the DynTrajectoryRepository
    let trajectory = make_trajectory(task_id, Some(goal_id));
    let trajectory_id = trajectory.id;

    dyn_repo
        .save(&trajectory)
        .await
        .expect("save should delegate successfully");

    // Verify get delegates
    let retrieved = dyn_repo
        .get(&trajectory_id.to_string())
        .await
        .expect("get should delegate successfully");
    assert!(retrieved.is_some(), "Should retrieve saved trajectory");
    assert_eq!(retrieved.unwrap().id, trajectory_id);

    // Verify get_by_task delegates
    let by_task = dyn_repo
        .get_by_task(&task_id.to_string())
        .await
        .expect("get_by_task should delegate successfully");
    assert_eq!(by_task.len(), 1);

    // Verify get_by_goal delegates
    let by_goal = dyn_repo
        .get_by_goal(&goal_id.to_string())
        .await
        .expect("get_by_goal should delegate successfully");
    assert_eq!(by_goal.len(), 1);

    // Verify get_recent delegates
    let recent = dyn_repo
        .get_recent(10)
        .await
        .expect("get_recent should delegate successfully");
    assert_eq!(recent.len(), 1);

    // Verify get_successful_strategies delegates
    let strategies = dyn_repo
        .get_successful_strategies(&AttractorType::Indeterminate { tendency: ConvergenceTendency::Flat }, 10)
        .await
        .expect("get_successful_strategies should delegate successfully");
    assert!(strategies.is_empty());

    // Verify analytics queries delegate
    let avg = dyn_repo
        .avg_iterations_by_complexity(Complexity::Moderate)
        .await
        .expect("avg_iterations_by_complexity should delegate");
    assert!((avg - 0.0).abs() < f64::EPSILON);

    let stats = dyn_repo
        .strategy_effectiveness(StrategyKind::RetryWithFeedback)
        .await
        .expect("strategy_effectiveness should delegate");
    assert_eq!(stats.total_uses, 0);

    let distribution = dyn_repo
        .attractor_distribution()
        .await
        .expect("attractor_distribution should delegate");
    assert!(distribution.is_empty());

    let rate = dyn_repo
        .convergence_rate_by_task_type("auth")
        .await
        .expect("convergence_rate_by_task_type should delegate");
    assert!((rate - 0.0).abs() < f64::EPSILON);

    let similar = dyn_repo
        .get_similar_trajectories("test", &[], 5)
        .await
        .expect("get_similar_trajectories should delegate");
    assert!(similar.is_empty());

    // Verify delete delegates
    dyn_repo
        .delete(&trajectory_id.to_string())
        .await
        .expect("delete should delegate successfully");

    let after_delete = dyn_repo
        .get(&trajectory_id.to_string())
        .await
        .expect("get after delete should work");
    assert!(after_delete.is_none(), "Trajectory should be deleted");
}

// ---------------------------------------------------------------------------
// 10. test_convergence_bridge_collect_artifact
// ---------------------------------------------------------------------------

/// Verify that collect_artifact produces a well-formed ArtifactReference
/// from worktree path and content hash.
#[tokio::test]
async fn test_convergence_bridge_collect_artifact() {
    let worktree_path = "/home/user/.abathur/worktrees/task-123";
    let content_hash = "sha256:abc123def456";

    let artifact = collect_artifact(worktree_path, content_hash);

    assert_eq!(
        artifact.path, worktree_path,
        "Artifact path should match worktree path"
    );
    assert_eq!(
        artifact.content_hash, content_hash,
        "Artifact content hash should match"
    );

    // Also verify that ArtifactReference::new produces the same result
    let direct = ArtifactReference::new(worktree_path, content_hash);
    assert_eq!(artifact.path, direct.path);
    assert_eq!(artifact.content_hash, direct.content_hash);
}

// ---------------------------------------------------------------------------
// 11. test_apply_sla_pressure_warning
// ---------------------------------------------------------------------------

/// Verify that `ConvergenceSLAPressureHandler` adds an "sla:warning" hint to
/// a convergent task's context when it processes a `TaskSLAWarning` event.
///
/// Since `apply_sla_pressure` is an internal function, this test validates the
/// end-to-end SLA pressure behavior by:
/// 1. Creating a convergent task with a trajectory_id.
/// 2. Constructing a `TaskSLAWarning` event.
/// 3. Invoking the handler directly.
/// 4. Verifying the task now has "sla:warning" in its hints.
#[tokio::test]
async fn test_apply_sla_pressure_warning() {
    use abathur::services::builtin_handlers::ConvergenceSLAPressureHandler;
    use abathur::services::event_reactor::{EventHandler, HandlerContext};
    use chrono::Utc;

    let task_repo = setup_task_repo().await;
    let task_service = TaskService::new(task_repo.clone());

    // Submit a task and set it to convergent mode with a trajectory_id
    let (task, _events) = task_service
        .submit_task(
            Some("SLA pressure test".to_string()),
            "A convergent task under SLA pressure".to_string(),
            None,
            TaskPriority::Normal,
            None,
            vec![],
            None,
            None,
            abathur::domain::models::task::TaskSource::System,
            None,
            None,
        )
        .await
        .expect("Failed to submit task");

    let trajectory_id = Uuid::new_v4();
    let mut task_mut = task.clone();
    task_mut.execution_mode = ExecutionMode::Convergent {
        parallel_samples: None,
    };
    task_mut.trajectory_id = Some(trajectory_id);
    task_repo
        .update(&task_mut)
        .await
        .expect("Failed to update task");

    // Construct a TaskSLAWarning event
    let sla_event = abathur::services::event_bus::UnifiedEvent {
        id: abathur::services::event_bus::EventId::new(),
        sequence: abathur::services::event_bus::SequenceNumber::zero(),
        timestamp: Utc::now(),
        severity: EventSeverity::Warning,
        category: EventCategory::Task,
        goal_id: None,
        task_id: Some(task.id),
        correlation_id: None,
        source_process_id: None,
        payload: EventPayload::TaskSLAWarning {
            task_id: task.id,
            deadline: "2026-02-11T00:00:00Z".to_string(),
            remaining_secs: 3600,
        },
    };

    // Create the handler and invoke it
    let handler = ConvergenceSLAPressureHandler::new(task_repo.clone());
    let ctx = HandlerContext {
        chain_depth: 0,
        correlation_id: None,
    };
    let reaction = handler
        .handle(&sla_event, &ctx)
        .await
        .expect("Handler should not fail");

    // The handler should not emit events -- it directly updates the task
    assert!(
        matches!(reaction, abathur::services::event_reactor::Reaction::None),
        "Handler should return Reaction::None (updates task in-place)"
    );

    // Reload the task and verify the hint was added
    let updated_task = task_repo
        .get(task.id)
        .await
        .expect("Failed to get task")
        .expect("Task should exist");

    let has_sla_warning = updated_task
        .context
        .hints
        .iter()
        .any(|h| h == "sla:warning");
    assert!(
        has_sla_warning,
        "Convergent task should have 'sla:warning' hint after TaskSLAWarning event. Hints: {:?}",
        updated_task.context.hints
    );

    // Verify idempotency: calling the handler again should not duplicate the hint
    let reaction2 = handler
        .handle(&sla_event, &ctx)
        .await
        .expect("Handler should not fail on second call");
    assert!(matches!(
        reaction2,
        abathur::services::event_reactor::Reaction::None
    ));

    let updated_task2 = task_repo
        .get(task.id)
        .await
        .expect("Failed to get task")
        .expect("Task should exist");
    let sla_warning_count = updated_task2
        .context
        .hints
        .iter()
        .filter(|h| h.as_str() == "sla:warning")
        .count();
    assert_eq!(
        sla_warning_count, 1,
        "sla:warning hint should appear exactly once (idempotent)"
    );
}

// ---------------------------------------------------------------------------
// 12. test_build_convergent_prompt_fresh_start
// ---------------------------------------------------------------------------

/// Verify that `build_convergent_prompt` with a `FreshStart` strategy includes
/// carry-forward context: the failure summary, remaining gaps, and "Start fresh"
/// preamble.
#[tokio::test]
async fn test_build_convergent_prompt_fresh_start() {
    use abathur::domain::models::convergence::CarryForward;
    use abathur::domain::models::intent_verification::{GapSeverity, IntentGap};

    let task = Task::new("Implement the widget renderer");
    let task_id = task.id;

    let spec = SpecificationEvolution::new(SpecificationSnapshot::new(
        "Implement the widget renderer with full test coverage".into(),
    ));
    let trajectory = Trajectory::new(
        task_id,
        None,
        spec,
        ConvergenceBudget::default(),
        ConvergencePolicy::default(),
    );

    // Build a FreshStart strategy with meaningful carry-forward data
    let carry_forward = CarryForward {
        specification: SpecificationSnapshot::new("Implement the widget renderer".into()),
        best_signals: OverseerSignals::empty(),
        best_artifact: ArtifactReference::default(),
        failure_summary: "RetryWithFeedback failed 3 times due to incorrect state management"
            .to_string(),
        remaining_gaps: vec![
            IntentGap::new("Missing error boundary handling", GapSeverity::Major),
            IntentGap::new("No accessibility attributes", GapSeverity::Moderate),
        ],
        hints: vec![],
    };

    let strategy = StrategyKind::FreshStart { carry_forward };

    let prompt = build_convergent_prompt(&task, &trajectory, &strategy, None);

    // Should include the "Start fresh" preamble
    assert!(
        prompt.contains("Start fresh"),
        "FreshStart prompt should include 'Start fresh'. Got:\n{}",
        prompt
    );

    // Should include the failure summary
    assert!(
        prompt.contains("incorrect state management"),
        "FreshStart prompt should include the failure summary. Got:\n{}",
        prompt
    );

    // Should include remaining gaps
    assert!(
        prompt.contains("Missing error boundary handling"),
        "FreshStart prompt should include remaining gap descriptions. Got:\n{}",
        prompt
    );
    assert!(
        prompt.contains("No accessibility attributes"),
        "FreshStart prompt should include all remaining gaps. Got:\n{}",
        prompt
    );

    // Should include the specification content
    assert!(
        prompt.contains("Implement the widget renderer with full test coverage"),
        "FreshStart prompt should include the specification content. Got:\n{}",
        prompt
    );
}

// ---------------------------------------------------------------------------
// 13. test_build_convergent_prompt_alternative_approach
// ---------------------------------------------------------------------------

/// Verify that `build_convergent_prompt` with `AlternativeApproach` strategy
/// lists previously-tried strategies and prompts for a fundamentally different
/// approach.
#[tokio::test]
async fn test_build_convergent_prompt_alternative_approach() {
    let task = Task::new("Optimize database query performance");
    let task_id = task.id;

    let spec = SpecificationEvolution::new(SpecificationSnapshot::new(
        "Optimize database query performance to < 100ms".into(),
    ));
    let mut trajectory = Trajectory::new(
        task_id,
        None,
        spec,
        ConvergenceBudget::default(),
        ConvergencePolicy::default(),
    );

    // Add strategy log entries for previously tried approaches
    trajectory.strategy_log.push(StrategyEntry::new(
        StrategyKind::RetryWithFeedback,
        0,
        20_000,
        false,
    ));
    trajectory.strategy_log.push(StrategyEntry::new(
        StrategyKind::FocusedRepair,
        1,
        15_000,
        false,
    ));
    trajectory.strategy_log.push(StrategyEntry::new(
        StrategyKind::IncrementalRefinement,
        2,
        25_000,
        false,
    ));

    let prompt = build_convergent_prompt(&task, &trajectory, &StrategyKind::AlternativeApproach, None);

    // Should include the "Previous approaches" header
    assert!(
        prompt.contains("Previous approaches that did not converge"),
        "AlternativeApproach prompt should list prior approaches. Got:\n{}",
        prompt
    );

    // Should list the specific strategies that were tried
    assert!(
        prompt.contains("retry_with_feedback"),
        "Prompt should mention retry_with_feedback. Got:\n{}",
        prompt
    );
    assert!(
        prompt.contains("focused_repair"),
        "Prompt should mention focused_repair. Got:\n{}",
        prompt
    );
    assert!(
        prompt.contains("incremental_refinement"),
        "Prompt should mention incremental_refinement. Got:\n{}",
        prompt
    );

    // Should prompt for a different approach
    assert!(
        prompt.contains("fundamentally different approach"),
        "AlternativeApproach should request a different approach. Got:\n{}",
        prompt
    );

    // Should include the specification content
    assert!(
        prompt.contains("Optimize database query performance"),
        "Prompt should include the specification. Got:\n{}",
        prompt
    );
}

// ---------------------------------------------------------------------------
// 14. test_build_convergent_prompt_focused_repair
// ---------------------------------------------------------------------------

/// Verify that `build_convergent_prompt` with `FocusedRepair` strategy
/// includes persistent gaps extracted from failing tests and build errors.
#[tokio::test]
async fn test_build_convergent_prompt_focused_repair() {
    let task = Task::new("Fix authentication module");
    let task_id = task.id;

    let spec = SpecificationEvolution::new(SpecificationSnapshot::new(
        "Fix authentication module to pass all security tests".into(),
    ));
    let mut trajectory = Trajectory::new(
        task_id,
        None,
        spec,
        ConvergenceBudget::default(),
        ConvergencePolicy::default(),
    );

    // Add an observation with failing tests so FocusedRepair has gaps to report
    let mut signals = OverseerSignals::empty();
    signals.test_results = Some(TestResults {
        passed: 8,
        failed: 2,
        skipped: 0,
        total: 10,
        regression_count: 0,
        failing_test_names: vec![
            "test_token_expiry".to_string(),
            "test_refresh_flow".to_string(),
        ],
    });
    signals.build_result = Some(BuildResult {
        success: false,
        error_count: 1,
        errors: vec!["unresolved import `jwt::Claims`".to_string()],
    });

    let obs = Observation::new(
        0,
        ArtifactReference::new("/tmp/worktree-auth", "hash-auth-001"),
        signals,
        StrategyKind::RetryWithFeedback,
        15_000,
        3_000,
    );
    trajectory.observations.push(obs);

    let prompt = build_convergent_prompt(&task, &trajectory, &StrategyKind::FocusedRepair, None);

    // Should include the "Focus on fixing" header
    assert!(
        prompt.contains("Focus on fixing these specific issues"),
        "FocusedRepair prompt should include focused repair instructions. Got:\n{}",
        prompt
    );

    // Should include the specification content
    assert!(
        prompt.contains("Fix authentication module to pass all security tests"),
        "Prompt should include the specification. Got:\n{}",
        prompt
    );

    // The gaps should include the failing tests
    assert!(
        prompt.contains("test_token_expiry"),
        "FocusedRepair should surface failing test names. Got:\n{}",
        prompt
    );
    assert!(
        prompt.contains("test_refresh_flow"),
        "FocusedRepair should surface all failing test names. Got:\n{}",
        prompt
    );

    // The gaps should include the build error
    assert!(
        prompt.contains("unresolved import"),
        "FocusedRepair should surface build errors. Got:\n{}",
        prompt
    );
}

// ---------------------------------------------------------------------------
// 15. test_convergence_event_attractor_transition
// ---------------------------------------------------------------------------

/// Verify that `ConvergenceAttractorTransition` events can be constructed,
/// published through the EventBus, and received with correct fields.
#[tokio::test]
async fn test_convergence_event_attractor_transition() {
    let event_bus = Arc::new(EventBus::new(EventBusConfig::default()));
    let mut receiver = event_bus.subscribe();

    let task_id = Uuid::new_v4();
    let trajectory_id = Uuid::new_v4();

    let event = event_factory::make_event(
        EventSeverity::Info,
        EventCategory::Convergence,
        None,
        Some(task_id),
        EventPayload::ConvergenceAttractorTransition {
            task_id,
            trajectory_id,
            from: "indeterminate".to_string(),
            to: "fixed_point".to_string(),
            confidence: 0.87,
        },
    );
    event_bus.publish(event).await;

    let received = receiver
        .recv()
        .await
        .expect("Should receive ConvergenceAttractorTransition event");

    assert_eq!(received.category, EventCategory::Convergence);
    assert!(matches!(
        received.payload,
        EventPayload::ConvergenceAttractorTransition { .. }
    ));

    if let EventPayload::ConvergenceAttractorTransition {
        task_id: recv_task_id,
        trajectory_id: recv_traj_id,
        from,
        to,
        confidence,
    } = received.payload
    {
        assert_eq!(recv_task_id, task_id);
        assert_eq!(recv_traj_id, trajectory_id);
        assert_eq!(from, "indeterminate");
        assert_eq!(to, "fixed_point");
        assert!((confidence - 0.87).abs() < f64::EPSILON);
    } else {
        panic!("Expected ConvergenceAttractorTransition payload");
    }
}

// ---------------------------------------------------------------------------
// 16. test_convergence_event_budget_extension
// ---------------------------------------------------------------------------

/// Verify that `ConvergenceBudgetExtension` events can be constructed,
/// published through the EventBus, and received with correct fields.
#[tokio::test]
async fn test_convergence_event_budget_extension() {
    let event_bus = Arc::new(EventBus::new(EventBusConfig::default()));
    let mut receiver = event_bus.subscribe();

    let task_id = Uuid::new_v4();
    let trajectory_id = Uuid::new_v4();

    let event = event_factory::make_event(
        EventSeverity::Info,
        EventCategory::Convergence,
        None,
        Some(task_id),
        EventPayload::ConvergenceBudgetExtension {
            task_id,
            trajectory_id,
            granted: true,
            additional_iterations: 3,
            additional_tokens: 50_000,
        },
    );
    event_bus.publish(event).await;

    let received = receiver
        .recv()
        .await
        .expect("Should receive ConvergenceBudgetExtension event");

    assert_eq!(received.category, EventCategory::Convergence);
    assert!(matches!(
        received.payload,
        EventPayload::ConvergenceBudgetExtension { .. }
    ));

    if let EventPayload::ConvergenceBudgetExtension {
        task_id: recv_task_id,
        trajectory_id: recv_traj_id,
        granted,
        additional_iterations,
        additional_tokens,
    } = received.payload
    {
        assert_eq!(recv_task_id, task_id);
        assert_eq!(recv_traj_id, trajectory_id);
        assert!(granted);
        assert_eq!(additional_iterations, 3);
        assert_eq!(additional_tokens, 50_000);
    } else {
        panic!("Expected ConvergenceBudgetExtension payload");
    }
}

// ---------------------------------------------------------------------------
// 17. test_convergence_event_fresh_start
// ---------------------------------------------------------------------------

/// Verify that `ConvergenceFreshStart` events can be constructed, published
/// through the EventBus, and received with correct fields.
#[tokio::test]
async fn test_convergence_event_fresh_start() {
    let event_bus = Arc::new(EventBus::new(EventBusConfig::default()));
    let mut receiver = event_bus.subscribe();

    let task_id = Uuid::new_v4();
    let trajectory_id = Uuid::new_v4();

    let event = event_factory::make_event(
        EventSeverity::Warning,
        EventCategory::Convergence,
        None,
        Some(task_id),
        EventPayload::ConvergenceFreshStart {
            task_id,
            trajectory_id,
            fresh_start_number: 2,
            reason: "context degradation detected".to_string(),
        },
    );
    event_bus.publish(event).await;

    let received = receiver
        .recv()
        .await
        .expect("Should receive ConvergenceFreshStart event");

    assert_eq!(received.category, EventCategory::Convergence);
    assert_eq!(received.severity, EventSeverity::Warning);
    assert!(matches!(
        received.payload,
        EventPayload::ConvergenceFreshStart { .. }
    ));

    if let EventPayload::ConvergenceFreshStart {
        task_id: recv_task_id,
        trajectory_id: recv_traj_id,
        fresh_start_number,
        reason,
    } = received.payload
    {
        assert_eq!(recv_task_id, task_id);
        assert_eq!(recv_traj_id, trajectory_id);
        assert_eq!(fresh_start_number, 2);
        assert_eq!(reason, "context degradation detected");
    } else {
        panic!("Expected ConvergenceFreshStart payload");
    }
}

// ---------------------------------------------------------------------------
// 18. test_execution_mode_persistence_roundtrip
// ---------------------------------------------------------------------------

/// Verify that `ExecutionMode::Convergent { parallel_samples: Some(3) }` is
/// correctly persisted through the SQLite task repository and restored on
/// load, including the `parallel_samples` field.
#[tokio::test]
async fn test_execution_mode_persistence_roundtrip() {
    let task_repo = setup_task_repo().await;
    let task_service = TaskService::new(task_repo.clone());

    // Submit a task
    let (task, _events) = task_service
        .submit_task(
            Some("Persistence roundtrip test".to_string()),
            "Test execution mode persistence".to_string(),
            None,
            TaskPriority::Normal,
            None,
            vec![],
            None,
            None,
            abathur::domain::models::task::TaskSource::System,
            None,
            None,
        )
        .await
        .expect("Failed to submit task");

    // Manually set convergent mode with parallel_samples
    let mut task_mut = task.clone();
    task_mut.execution_mode = ExecutionMode::Convergent {
        parallel_samples: Some(3),
    };
    task_mut.trajectory_id = Some(Uuid::new_v4());
    task_repo
        .update(&task_mut)
        .await
        .expect("Failed to update task");

    // Load the task back from the database
    let loaded_task = task_repo
        .get(task.id)
        .await
        .expect("Failed to get task")
        .expect("Task should exist");

    // Verify execution_mode roundtrips correctly
    assert!(
        loaded_task.execution_mode.is_convergent(),
        "Loaded task should be convergent, got {:?}",
        loaded_task.execution_mode
    );
    assert_eq!(
        loaded_task.execution_mode.parallel_samples(),
        Some(3),
        "parallel_samples should roundtrip through persistence"
    );

    // Verify trajectory_id also roundtrips
    assert_eq!(
        loaded_task.trajectory_id, task_mut.trajectory_id,
        "trajectory_id should roundtrip through persistence"
    );

    // Also test the Direct mode roundtrip for completeness
    let mut task_direct = task.clone();
    task_direct.execution_mode = ExecutionMode::Direct;
    task_repo
        .update(&task_direct)
        .await
        .expect("Failed to update task to direct mode");

    let loaded_direct = task_repo
        .get(task.id)
        .await
        .expect("Failed to get task")
        .expect("Task should exist");

    assert!(
        loaded_direct.execution_mode.is_direct(),
        "Loaded task should be direct after update, got {:?}",
        loaded_direct.execution_mode
    );

    // Also test Convergent with parallel_samples: None
    let mut task_convergent_none = task.clone();
    task_convergent_none.execution_mode = ExecutionMode::Convergent {
        parallel_samples: None,
    };
    task_repo
        .update(&task_convergent_none)
        .await
        .expect("Failed to update task to convergent without parallel samples");

    let loaded_convergent_none = task_repo
        .get(task.id)
        .await
        .expect("Failed to get task")
        .expect("Task should exist");

    assert!(
        loaded_convergent_none.execution_mode.is_convergent(),
        "Loaded task should be convergent"
    );
    assert_eq!(
        loaded_convergent_none.execution_mode.parallel_samples(),
        None,
        "parallel_samples should be None when not set"
    );
}
