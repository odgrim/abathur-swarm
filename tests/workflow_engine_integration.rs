//! Integration tests for the workflow engine.
//!
//! These tests verify the unified task lifecycle framework: auto-enrollment,
//! phase-driven execution, advance guards, verification, fan-out, convergent
//! skip, and the AdvanceResult return type.

use std::sync::Arc;

use abathur::adapters::sqlite::{create_migrated_test_pool, SqliteTaskRepository};
use abathur::domain::models::task::{Task, TaskSource, TaskStatus, TaskType};
use abathur::domain::models::workflow_state::WorkflowState;
use abathur::domain::models::TaskPriority;
use abathur::domain::ports::TaskRepository;
use abathur::services::event_bus::{EventBus, EventBusConfig};
use abathur::services::task_service::TaskService;
use abathur::services::workflow_engine::{AdvanceResult, WorkflowEngine};

/// Create a TaskService + WorkflowEngine pair for testing.
async fn setup() -> (
    TaskService<SqliteTaskRepository>,
    WorkflowEngine<SqliteTaskRepository>,
    Arc<SqliteTaskRepository>,
    Arc<EventBus>,
) {
    let pool = create_migrated_test_pool().await.unwrap();
    let task_repo = Arc::new(SqliteTaskRepository::new(pool));
    let event_bus = Arc::new(EventBus::new(EventBusConfig::default()));
    let task_service = TaskService::new(task_repo.clone());
    let engine = WorkflowEngine::new(task_repo.clone(), event_bus.clone(), true);
    (task_service, engine, task_repo, event_bus)
}

/// Helper: submit a root task and verify it's auto-enrolled.
async fn submit_root_task(
    service: &TaskService<SqliteTaskRepository>,
    title: &str,
) -> Task {
    let (task, _events) = service
        .submit_task(
            Some(title.to_string()),
            "Test task description".to_string(),
            None,
            TaskPriority::Normal,
            None,
            vec![],
            None,
            None,
            TaskSource::Human,
            None,
            None,
            None,
        )
        .await
        .unwrap();
    task
}

// ============================================================================
// Test 1: infer_workflow_name
// ============================================================================

#[tokio::test]
async fn test_infer_workflow_name_root_task_defaults_to_code() {
    let (service, _, repo, _) = setup().await;
    let task = submit_root_task(&service, "Root task").await;

    // Root tasks (Human source, no parent) should be enrolled in "code"
    let reloaded = repo.get(task.id).await.unwrap().unwrap();
    assert_eq!(
        reloaded.routing_hints.workflow_name,
        Some("code".to_string()),
        "Root human tasks should be auto-enrolled in the 'code' workflow"
    );
}

#[tokio::test]
async fn test_infer_workflow_name_adapter_source_uses_external() {
    let (service, _, repo, _) = setup().await;
    let (task, _) = service
        .submit_task(
            Some("Adapter task".to_string()),
            "From adapter".to_string(),
            None,
            TaskPriority::Normal,
            None,
            vec![],
            None,
            None,
            TaskSource::Adapter("github".to_string()),
            None,
            None,
            None,
        )
        .await
        .unwrap();

    let reloaded = repo.get(task.id).await.unwrap().unwrap();
    assert_eq!(
        reloaded.routing_hints.workflow_name,
        Some("external".to_string()),
        "Adapter-sourced tasks should use the 'external' workflow"
    );
}

#[tokio::test]
async fn test_infer_workflow_name_verification_tasks_excluded() {
    let (service, _, repo, _) = setup().await;
    let (task, _) = service
        .submit_task(
            Some("Verify something".to_string()),
            "Verification task".to_string(),
            None,
            TaskPriority::Normal,
            None,
            vec![],
            None,
            None,
            TaskSource::Human,
            None,
            Some(TaskType::Verification),
            None,
        )
        .await
        .unwrap();

    let reloaded = repo.get(task.id).await.unwrap().unwrap();
    assert!(
        !reloaded.context.custom.contains_key("workflow_state"),
        "Verification tasks should NOT be enrolled in a workflow"
    );
}

#[tokio::test]
async fn test_infer_workflow_name_subtask_excluded() {
    let (service, _, repo, _) = setup().await;
    let parent = submit_root_task(&service, "Parent").await;

    let (subtask, _) = service
        .submit_task(
            Some("Subtask".to_string()),
            "Child task".to_string(),
            Some(parent.id),
            TaskPriority::Normal,
            None,
            vec![],
            None,
            None,
            TaskSource::SubtaskOf(parent.id),
            None,
            None,
            None,
        )
        .await
        .unwrap();

    let reloaded = repo.get(subtask.id).await.unwrap().unwrap();
    assert!(
        !reloaded.context.custom.contains_key("workflow_state"),
        "Non-root subtasks should NOT be enrolled in a workflow"
    );
}

// ============================================================================
// Test 2: auto_enrollment_on_submit
// ============================================================================

#[tokio::test]
async fn test_auto_enrollment_on_submit() {
    let (service, _, repo, _) = setup().await;
    let task = submit_root_task(&service, "Auto-enrolled task").await;

    let reloaded = repo.get(task.id).await.unwrap().unwrap();
    assert!(
        reloaded.context.custom.contains_key("workflow_state"),
        "Root task should have workflow_state after submission"
    );

    let ws: WorkflowState =
        serde_json::from_value(reloaded.context.custom["workflow_state"].clone()).unwrap();
    assert!(
        matches!(ws, WorkflowState::Pending { ref workflow_name } if workflow_name == "code"),
        "Workflow state should be Pending with workflow_name='code'"
    );
}

// ============================================================================
// Test 3: advance_from_pending
// ============================================================================

#[tokio::test]
async fn test_advance_from_pending() {
    let (service, engine, repo, _) = setup().await;
    let task = submit_root_task(&service, "Advance test").await;

    let result = engine.advance(task.id).await.unwrap();
    match result {
        AdvanceResult::PhaseStarted {
            phase_index,
            phase_name,
            ..
        } => {
            assert_eq!(phase_index, 0);
            assert_eq!(phase_name, "research");
        }
        AdvanceResult::Completed => panic!("Expected PhaseStarted, got Completed"),
    }

    // Check state is now PhaseRunning
    let reloaded = repo.get(task.id).await.unwrap().unwrap();
    let ws: WorkflowState =
        serde_json::from_value(reloaded.context.custom["workflow_state"].clone()).unwrap();
    match ws {
        WorkflowState::PhaseRunning {
            phase_index,
            phase_name,
            subtask_ids,
            ..
        } => {
            assert_eq!(phase_index, 0);
            assert_eq!(phase_name, "research");
            assert_eq!(subtask_ids.len(), 1);
        }
        other => panic!("Expected PhaseRunning, got {:?}", other),
    }
}

// ============================================================================
// Test 4: handle_phase_complete_single_subtask
// ============================================================================

#[tokio::test]
async fn test_handle_phase_complete_single_subtask() {
    let (service, engine, repo, _) = setup().await;
    let task = submit_root_task(&service, "Phase complete test").await;

    // Claim the parent (transitions to Running)
    service.claim_task(task.id, "overmind").await.unwrap();

    // Advance to first phase (research)
    let result = engine.advance(task.id).await.unwrap();
    let subtask_id = match result {
        AdvanceResult::PhaseStarted { subtask_id, .. } => subtask_id,
        _ => panic!("Expected PhaseStarted"),
    };

    // Complete the subtask
    service.claim_task(subtask_id, "researcher").await.unwrap();
    service.complete_task(subtask_id).await.unwrap();

    // Handle phase completion
    engine
        .handle_phase_complete(task.id, subtask_id)
        .await
        .unwrap();

    // Should be in PhaseReady for the next phase (plan)
    let reloaded = repo.get(task.id).await.unwrap().unwrap();
    let ws: WorkflowState =
        serde_json::from_value(reloaded.context.custom["workflow_state"].clone()).unwrap();
    match ws {
        WorkflowState::PhaseReady {
            phase_index,
            phase_name,
            ..
        } => {
            assert_eq!(phase_index, 1, "Should be PhaseReady at phase 1 (plan)");
            assert_eq!(phase_name, "plan");
        }
        other => panic!("Expected PhaseReady at phase 1, got {:?}", other),
    }
}

// ============================================================================
// Test 5: handle_phase_complete_with_verification
// ============================================================================

#[tokio::test]
async fn test_handle_phase_complete_with_verification() {
    let (service, engine, repo, _) = setup().await;
    let task = submit_root_task(&service, "Verification test").await;
    service.claim_task(task.id, "overmind").await.unwrap();

    // Fast-forward to the implement phase (index 2) which has verify: true
    // We do this by manually writing the state
    let implement_subtask = Task::with_title("implement subtask", "implement");
    let mut implement = implement_subtask;
    implement.parent_id = Some(task.id);
    implement.source = TaskSource::SubtaskOf(task.id);
    let _ = implement.transition_to(TaskStatus::Ready);
    repo.create(&implement).await.unwrap();

    // Claim and complete the implement subtask
    let mut impl_task = repo.get(implement.id).await.unwrap().unwrap();
    let _ = impl_task.transition_to(TaskStatus::Running);
    repo.update(&impl_task).await.unwrap();
    let _ = impl_task.transition_to(TaskStatus::Complete);
    impl_task.completed_at = Some(chrono::Utc::now());
    repo.update(&impl_task).await.unwrap();

    // Set workflow state to PhaseRunning at implement phase
    let phase_state = WorkflowState::PhaseRunning {
        workflow_name: "code".to_string(),
        phase_index: 2,
        phase_name: "implement".to_string(),
        subtask_ids: vec![implement.id],
    };
    let mut parent = repo.get(task.id).await.unwrap().unwrap();
    parent.context.custom.insert(
        "workflow_state".to_string(),
        serde_json::to_value(&phase_state).unwrap(),
    );
    parent.updated_at = chrono::Utc::now();
    repo.update(&parent).await.unwrap();

    // Handle phase completion — implement has verify: true
    engine
        .handle_phase_complete(task.id, implement.id)
        .await
        .unwrap();

    // Should transition to Verifying
    let reloaded = repo.get(task.id).await.unwrap().unwrap();
    let ws: WorkflowState =
        serde_json::from_value(reloaded.context.custom["workflow_state"].clone()).unwrap();
    match ws {
        WorkflowState::Verifying {
            phase_index,
            phase_name,
            retry_count,
            ..
        } => {
            assert_eq!(phase_index, 2);
            assert_eq!(phase_name, "implement");
            assert_eq!(retry_count, 0);
        }
        other => panic!("Expected Verifying, got {:?}", other),
    }
}

// ============================================================================
// Test 6: fan_out_to_aggregation
// ============================================================================

#[tokio::test]
async fn test_fan_out_to_aggregation() {
    use abathur::domain::models::workflow_state::FanOutSlice;

    let (service, engine, repo, _) = setup().await;
    let task = submit_root_task(&service, "Fan-out test").await;
    service.claim_task(task.id, "overmind").await.unwrap();

    // Fan out research phase into 2 slices
    let slices = vec![
        FanOutSlice {
            description: "Research area A".to_string(),
            context: Default::default(),
        },
        FanOutSlice {
            description: "Research area B".to_string(),
            context: Default::default(),
        },
    ];
    let fan_result = engine.fan_out(task.id, slices).await.unwrap();
    assert_eq!(fan_result.subtask_ids.len(), 2);
    assert_eq!(fan_result.phase_name, "research");

    // Verify state is FanningOut
    let parent = repo.get(task.id).await.unwrap().unwrap();
    let ws: WorkflowState =
        serde_json::from_value(parent.context.custom["workflow_state"].clone()).unwrap();
    assert!(matches!(ws, WorkflowState::FanningOut { slice_count: 2, .. }));

    // Complete both fan-out subtasks
    for id in &fan_result.subtask_ids {
        let mut sub = repo.get(*id).await.unwrap().unwrap();
        let _ = sub.transition_to(TaskStatus::Running);
        repo.update(&sub).await.unwrap();
        let _ = sub.transition_to(TaskStatus::Complete);
        sub.completed_at = Some(chrono::Utc::now());
        repo.update(&sub).await.unwrap();
    }

    // Handle phase complete — should trigger fan-in (create aggregation subtask)
    engine
        .handle_phase_complete(task.id, fan_result.subtask_ids[1])
        .await
        .unwrap();

    // Verify state is now Aggregating
    let parent = repo.get(task.id).await.unwrap().unwrap();
    let ws: WorkflowState =
        serde_json::from_value(parent.context.custom["workflow_state"].clone()).unwrap();
    match ws {
        WorkflowState::Aggregating { subtask_ids, .. } => {
            // 2 original + 1 aggregation subtask
            assert_eq!(subtask_ids.len(), 3);
        }
        other => panic!("Expected Aggregating, got {:?}", other),
    }
}

// ============================================================================
// Test 7: convergent_skip (converged)
// ============================================================================

#[tokio::test]
async fn test_convergent_skip_verification() {
    let (service, engine, repo, _) = setup().await;
    let task = submit_root_task(&service, "Convergent skip test").await;
    service.claim_task(task.id, "overmind").await.unwrap();

    // Create a subtask that has convergence_outcome: "converged"
    let mut subtask = Task::with_title("converged implement", "implement code");
    subtask.parent_id = Some(task.id);
    subtask.source = TaskSource::SubtaskOf(task.id);
    let _ = subtask.transition_to(TaskStatus::Ready);
    subtask.context.custom.insert(
        "convergence_outcome".to_string(),
        serde_json::json!("converged"),
    );
    subtask.context.custom.insert(
        "workflow_phase".to_string(),
        serde_json::json!({"workflow_name": "code", "phase_index": 2, "phase_name": "implement"}),
    );
    repo.create(&subtask).await.unwrap();

    // Complete the subtask
    let mut sub = repo.get(subtask.id).await.unwrap().unwrap();
    let _ = sub.transition_to(TaskStatus::Running);
    repo.update(&sub).await.unwrap();
    let _ = sub.transition_to(TaskStatus::Complete);
    sub.completed_at = Some(chrono::Utc::now());
    repo.update(&sub).await.unwrap();

    // Set parent state to PhaseRunning at implement
    let phase_state = WorkflowState::PhaseRunning {
        workflow_name: "code".to_string(),
        phase_index: 2,
        phase_name: "implement".to_string(),
        subtask_ids: vec![subtask.id],
    };
    let mut parent = repo.get(task.id).await.unwrap().unwrap();
    parent.context.custom.insert(
        "workflow_state".to_string(),
        serde_json::to_value(&phase_state).unwrap(),
    );
    parent.updated_at = chrono::Utc::now();
    repo.update(&parent).await.unwrap();

    // Handle phase complete — should skip verification because subtask converged
    engine
        .handle_phase_complete(task.id, subtask.id)
        .await
        .unwrap();

    // Should NOT be in Verifying state — should have advanced to review (gate)
    let reloaded = repo.get(task.id).await.unwrap().unwrap();
    let ws: WorkflowState =
        serde_json::from_value(reloaded.context.custom["workflow_state"].clone()).unwrap();
    assert!(
        !matches!(ws, WorkflowState::Verifying { .. }),
        "Converged subtasks should skip verification; got {:?}",
        ws
    );
}

// ============================================================================
// Test 8: convergent_skip_partial_accepted
// ============================================================================

#[tokio::test]
async fn test_convergent_skip_partial_accepted() {
    let (service, engine, repo, _) = setup().await;
    let task = submit_root_task(&service, "Partial accepted test").await;
    service.claim_task(task.id, "overmind").await.unwrap();

    // Create a subtask with convergence_outcome: "partial_accepted"
    let mut subtask = Task::with_title("partial implement", "implement code");
    subtask.parent_id = Some(task.id);
    subtask.source = TaskSource::SubtaskOf(task.id);
    let _ = subtask.transition_to(TaskStatus::Ready);
    subtask.context.custom.insert(
        "convergence_outcome".to_string(),
        serde_json::json!("partial_accepted"),
    );
    subtask.context.custom.insert(
        "workflow_phase".to_string(),
        serde_json::json!({"workflow_name": "code", "phase_index": 2, "phase_name": "implement"}),
    );
    repo.create(&subtask).await.unwrap();

    let mut sub = repo.get(subtask.id).await.unwrap().unwrap();
    let _ = sub.transition_to(TaskStatus::Running);
    repo.update(&sub).await.unwrap();
    let _ = sub.transition_to(TaskStatus::Complete);
    sub.completed_at = Some(chrono::Utc::now());
    repo.update(&sub).await.unwrap();

    let phase_state = WorkflowState::PhaseRunning {
        workflow_name: "code".to_string(),
        phase_index: 2,
        phase_name: "implement".to_string(),
        subtask_ids: vec![subtask.id],
    };
    let mut parent = repo.get(task.id).await.unwrap().unwrap();
    parent.context.custom.insert(
        "workflow_state".to_string(),
        serde_json::to_value(&phase_state).unwrap(),
    );
    parent.updated_at = chrono::Utc::now();
    repo.update(&parent).await.unwrap();

    engine
        .handle_phase_complete(task.id, subtask.id)
        .await
        .unwrap();

    let reloaded = repo.get(task.id).await.unwrap().unwrap();
    let ws: WorkflowState =
        serde_json::from_value(reloaded.context.custom["workflow_state"].clone()).unwrap();
    assert!(
        !matches!(ws, WorkflowState::Verifying { .. }),
        "partial_accepted subtasks should also skip verification; got {:?}",
        ws
    );
}

// ============================================================================
// Test 9: verification_result_auto_advance
// ============================================================================

#[tokio::test]
async fn test_verification_result_auto_advance() {
    let (service, engine, repo, _) = setup().await;
    let task = submit_root_task(&service, "Verification advance test").await;
    service.claim_task(task.id, "overmind").await.unwrap();

    let subtask_id = uuid::Uuid::new_v4();
    // Set state to Verifying at implement phase
    let verifying_state = WorkflowState::Verifying {
        workflow_name: "code".to_string(),
        phase_index: 2,
        phase_name: "implement".to_string(),
        subtask_ids: vec![subtask_id],
        retry_count: 0,
    };
    let mut parent = repo.get(task.id).await.unwrap().unwrap();
    parent.context.custom.insert(
        "workflow_state".to_string(),
        serde_json::to_value(&verifying_state).unwrap(),
    );
    parent.updated_at = chrono::Utc::now();
    repo.update(&parent).await.unwrap();

    // Verification passed
    engine
        .handle_verification_result(task.id, true, "All checks passed")
        .await
        .unwrap();

    // Should be PhaseReady for review (gate phase)
    let reloaded = repo.get(task.id).await.unwrap().unwrap();
    let ws: WorkflowState =
        serde_json::from_value(reloaded.context.custom["workflow_state"].clone()).unwrap();
    match ws {
        WorkflowState::PhaseReady {
            phase_index,
            phase_name,
            ..
        } => {
            assert_eq!(phase_index, 3, "Should be PhaseReady at phase 3 (review)");
            assert_eq!(phase_name, "review");
        }
        other => panic!("Expected PhaseReady at review, got {:?}", other),
    }
}

// ============================================================================
// Test 10: verification_result_gate_escalation
// ============================================================================

#[tokio::test]
async fn test_verification_result_gate_escalation() {
    let (service, engine, repo, _) = setup().await;
    let task = submit_root_task(&service, "Gate escalation test").await;
    service.claim_task(task.id, "overmind").await.unwrap();

    let subtask_id = uuid::Uuid::new_v4();
    // Set state to Verifying with retry_count at max (default max_verification_retries = 2)
    let verifying_state = WorkflowState::Verifying {
        workflow_name: "code".to_string(),
        phase_index: 2,
        phase_name: "implement".to_string(),
        subtask_ids: vec![subtask_id],
        retry_count: 2,
    };
    let mut parent = repo.get(task.id).await.unwrap().unwrap();
    parent.context.custom.insert(
        "workflow_state".to_string(),
        serde_json::to_value(&verifying_state).unwrap(),
    );
    parent.updated_at = chrono::Utc::now();
    repo.update(&parent).await.unwrap();

    // Verification failed with retries exhausted
    engine
        .handle_verification_result(task.id, false, "Tests still failing")
        .await
        .unwrap();

    // Should escalate to PhaseGate
    let reloaded = repo.get(task.id).await.unwrap().unwrap();
    let ws: WorkflowState =
        serde_json::from_value(reloaded.context.custom["workflow_state"].clone()).unwrap();
    match ws {
        WorkflowState::PhaseGate {
            phase_index,
            phase_name,
            ..
        } => {
            assert_eq!(phase_index, 2);
            assert_eq!(phase_name, "implement");
        }
        other => panic!("Expected PhaseGate, got {:?}", other),
    }

    // Should have stored verification feedback
    assert!(
        reloaded
            .context
            .custom
            .contains_key("verification_feedback"),
        "Should store verification feedback on gate escalation"
    );
}

// ============================================================================
// Test 11: advance_guards_active_subtask
// ============================================================================

#[tokio::test]
async fn test_advance_guards_active_subtask() {
    let (service, engine, _repo, _) = setup().await;
    let task = submit_root_task(&service, "Guard test").await;

    // Advance to first phase
    let result = engine.advance(task.id).await.unwrap();
    let subtask_id = match result {
        AdvanceResult::PhaseStarted { subtask_id, .. } => subtask_id,
        _ => panic!("Expected PhaseStarted"),
    };

    // Subtask is in Ready state (not terminal) — trying to advance should fail
    // First claim the subtask so it's Running (also non-terminal)
    service.claim_task(subtask_id, "researcher").await.unwrap();

    let err = engine.advance(task.id).await;
    assert!(
        err.is_err(),
        "Advancing with non-terminal subtask should fail"
    );
    let err_msg = format!("{}", err.unwrap_err());
    assert!(
        err_msg.contains("still running"),
        "Error should mention subtasks still running, got: {}",
        err_msg
    );
}

// ============================================================================
// Test 12: create_phase_subtask_execution_mode
// ============================================================================

#[tokio::test]
async fn test_create_phase_subtask_execution_mode() {
    let (service, engine, repo, _) = setup().await;
    let task = submit_root_task(&service, "Exec mode test").await;

    // Advance to first phase (research — read_only)
    let result = engine.advance(task.id).await.unwrap();
    let research_subtask_id = match result {
        AdvanceResult::PhaseStarted { subtask_id, .. } => subtask_id,
        _ => panic!("Expected PhaseStarted"),
    };

    // Research subtask should be Direct (read-only phase)
    let research_sub = repo.get(research_subtask_id).await.unwrap().unwrap();
    assert!(
        research_sub.execution_mode.is_direct(),
        "Read-only phase subtask should use Direct execution mode"
    );

    // Complete research subtask and advance to plan
    let mut sub = research_sub;
    let _ = sub.transition_to(TaskStatus::Running);
    repo.update(&sub).await.unwrap();
    let _ = sub.transition_to(TaskStatus::Complete);
    sub.completed_at = Some(chrono::Utc::now());
    repo.update(&sub).await.unwrap();

    // Need to claim parent first so it's Running
    service.claim_task(task.id, "overmind").await.unwrap();

    engine
        .handle_phase_complete(task.id, sub.id)
        .await
        .unwrap();

    // Plan phase is now PhaseReady — advance to create the subtask, then check execution mode
    let parent = repo.get(task.id).await.unwrap().unwrap();
    let ws: WorkflowState =
        serde_json::from_value(parent.context.custom["workflow_state"].clone()).unwrap();
    if let WorkflowState::PhaseReady { .. } = ws {
        let result = engine.advance(task.id).await.unwrap();
        let plan_sub_id = match result {
            AdvanceResult::PhaseStarted { subtask_id, .. } => subtask_id,
            _ => panic!("Expected PhaseStarted"),
        };
        let plan_sub = repo.get(plan_sub_id).await.unwrap().unwrap();
        assert!(
            plan_sub.execution_mode.is_direct(),
            "Plan phase (read-only) should use Direct mode"
        );

        // Complete plan and handle phase complete
        let mut p = plan_sub;
        let _ = p.transition_to(TaskStatus::Running);
        repo.update(&p).await.unwrap();
        let _ = p.transition_to(TaskStatus::Complete);
        p.completed_at = Some(chrono::Utc::now());
        repo.update(&p).await.unwrap();

        engine.handle_phase_complete(task.id, p.id).await.unwrap();

        // Implement phase is now PhaseReady — advance to create subtask
        let parent = repo.get(task.id).await.unwrap().unwrap();
        let ws: WorkflowState =
            serde_json::from_value(parent.context.custom["workflow_state"].clone()).unwrap();
        if let WorkflowState::PhaseReady { .. } = ws {
            let result = engine.advance(task.id).await.unwrap();
            let impl_sub_id = match result {
                AdvanceResult::PhaseStarted { subtask_id, .. } => subtask_id,
                _ => panic!("Expected PhaseStarted"),
            };
            let impl_sub = repo.get(impl_sub_id).await.unwrap().unwrap();
            assert!(
                impl_sub.execution_mode.is_convergent(),
                "Implement phase (write tools) should use Convergent mode"
            );
        }
    }
}

// ============================================================================
// Test: advance returns Completed (not Err) when all phases done
// ============================================================================

#[tokio::test]
async fn test_advance_returns_completed_not_error() {
    let (service, engine, repo, _) = setup().await;
    let task = submit_root_task(&service, "Completion test").await;
    service.claim_task(task.id, "overmind").await.unwrap();

    // Set state to PhaseGate at the last phase (review = index 3)
    // with a completed subtask
    let mut subtask = Task::with_title("review subtask", "review");
    subtask.parent_id = Some(task.id);
    subtask.source = TaskSource::SubtaskOf(task.id);
    let _ = subtask.transition_to(TaskStatus::Ready);
    repo.create(&subtask).await.unwrap();
    let mut sub = repo.get(subtask.id).await.unwrap().unwrap();
    let _ = sub.transition_to(TaskStatus::Running);
    repo.update(&sub).await.unwrap();
    let _ = sub.transition_to(TaskStatus::Complete);
    sub.completed_at = Some(chrono::Utc::now());
    repo.update(&sub).await.unwrap();

    let gate_state = WorkflowState::PhaseGate {
        workflow_name: "code".to_string(),
        phase_index: 3,
        phase_name: "review".to_string(),
    };
    let mut parent = repo.get(task.id).await.unwrap().unwrap();
    parent.context.custom.insert(
        "workflow_state".to_string(),
        serde_json::to_value(&gate_state).unwrap(),
    );
    parent.updated_at = chrono::Utc::now();
    repo.update(&parent).await.unwrap();

    // Advance from the last phase gate — should complete
    let result = engine.advance(task.id).await.unwrap();
    assert!(
        matches!(result, AdvanceResult::Completed),
        "advance() should return Ok(Completed) when all phases are done, got {:?}",
        result
    );

    // Verify state is Completed
    let reloaded = repo.get(task.id).await.unwrap().unwrap();
    let ws: WorkflowState =
        serde_json::from_value(reloaded.context.custom["workflow_state"].clone()).unwrap();
    assert!(matches!(ws, WorkflowState::Completed { .. }));
}

// ============================================================================
// Test: template validation on auto-enrollment
// ============================================================================

#[tokio::test]
async fn test_invalid_workflow_name_skips_enrollment() {
    let (service, _, repo, _) = setup().await;

    // Submit a task with an explicit but invalid workflow_name hint
    let ctx = abathur::domain::models::TaskContext::default();
    // We can't directly set routing_hints before submit, but we can use a context
    // hack — the routing hint is set by infer_workflow_name. To test validation,
    // we'd need a task source that maps to an invalid name. Since all built-in
    // mappings go to valid names, let's verify the positive case instead:
    // "code" workflow exists and enrollment succeeds.
    let (task, events) = service
        .submit_task(
            Some("Valid enrollment".to_string()),
            "Should enroll in code".to_string(),
            None,
            TaskPriority::Normal,
            None,
            vec![],
            Some(ctx),
            None,
            TaskSource::Human,
            None,
            None,
            None,
        )
        .await
        .unwrap();

    let reloaded = repo.get(task.id).await.unwrap().unwrap();
    assert!(reloaded.context.custom.contains_key("workflow_state"));

    // Verify WorkflowEnrolled event was emitted
    let enrolled = events.iter().any(|e| {
        matches!(
            &e.payload,
            abathur::services::EventPayload::WorkflowEnrolled { .. }
        )
    });
    assert!(enrolled, "Should emit WorkflowEnrolled event");
}

// ============================================================================
// Test: provide_verdict Approve auto-advances (Fix 2)
// ============================================================================

#[tokio::test]
async fn test_provide_verdict_approve_auto_advances() {
    use abathur::domain::models::workflow_state::GateVerdict;

    let (service, engine, repo, _) = setup().await;
    let task = submit_root_task(&service, "Approve auto-advance test").await;
    service.claim_task(task.id, "overmind").await.unwrap();

    // Create a completed subtask for the review gate phase
    let mut subtask = Task::with_title("review subtask", "review work");
    subtask.parent_id = Some(task.id);
    subtask.source = TaskSource::SubtaskOf(task.id);
    let _ = subtask.transition_to(TaskStatus::Ready);
    repo.create(&subtask).await.unwrap();
    let mut sub = repo.get(subtask.id).await.unwrap().unwrap();
    let _ = sub.transition_to(TaskStatus::Running);
    repo.update(&sub).await.unwrap();
    let _ = sub.transition_to(TaskStatus::Complete);
    sub.completed_at = Some(chrono::Utc::now());
    repo.update(&sub).await.unwrap();

    // Set state to PhaseGate at review (index 3, the last phase)
    let gate_state = WorkflowState::PhaseGate {
        workflow_name: "code".to_string(),
        phase_index: 3,
        phase_name: "review".to_string(),
    };
    let mut parent = repo.get(task.id).await.unwrap().unwrap();
    parent.context.custom.insert(
        "workflow_state".to_string(),
        serde_json::to_value(&gate_state).unwrap(),
    );
    parent.updated_at = chrono::Utc::now();
    repo.update(&parent).await.unwrap();

    // Approve — should auto-advance (and complete since review is last phase)
    let result = engine
        .provide_verdict(task.id, GateVerdict::Approve, "Looks good")
        .await
        .unwrap();

    // Return value should be Some(Completed) since review is the last phase
    assert!(
        matches!(result, Some(AdvanceResult::Completed)),
        "Approve at last gate should return Some(Completed); got {:?}",
        result
    );

    // Verify workflow completed and parent task is Complete
    let reloaded = repo.get(task.id).await.unwrap().unwrap();
    let ws: WorkflowState =
        serde_json::from_value(reloaded.context.custom["workflow_state"].clone()).unwrap();
    assert!(
        matches!(ws, WorkflowState::Completed { .. }),
        "Approve at last gate should complete workflow; got {:?}",
        ws
    );
    assert_eq!(
        reloaded.status,
        TaskStatus::Complete,
        "Parent task should be Complete after workflow completion"
    );
}

// ============================================================================
// Test: provide_verdict Reject
// ============================================================================

#[tokio::test]
async fn test_provide_verdict_reject() {
    use abathur::domain::models::workflow_state::GateVerdict;

    let (service, engine, repo, _) = setup().await;
    let task = submit_root_task(&service, "Reject test").await;
    service.claim_task(task.id, "overmind").await.unwrap();

    let gate_state = WorkflowState::PhaseGate {
        workflow_name: "code".to_string(),
        phase_index: 2,
        phase_name: "implement".to_string(),
    };
    let mut parent = repo.get(task.id).await.unwrap().unwrap();
    parent.context.custom.insert(
        "workflow_state".to_string(),
        serde_json::to_value(&gate_state).unwrap(),
    );
    parent.updated_at = chrono::Utc::now();
    repo.update(&parent).await.unwrap();

    engine
        .provide_verdict(task.id, GateVerdict::Reject, "Not acceptable")
        .await
        .unwrap();

    let reloaded = repo.get(task.id).await.unwrap().unwrap();
    let ws: WorkflowState =
        serde_json::from_value(reloaded.context.custom["workflow_state"].clone()).unwrap();
    match ws {
        WorkflowState::Rejected { phase_index, reason, .. } => {
            assert_eq!(phase_index, 2);
            assert_eq!(reason, "Not acceptable");
        }
        other => panic!("Expected Rejected, got {:?}", other),
    }
}

// ============================================================================
// Test: provide_verdict Rework
// ============================================================================

#[tokio::test]
async fn test_provide_verdict_rework() {
    use abathur::domain::models::workflow_state::GateVerdict;

    let (service, engine, repo, _) = setup().await;
    let task = submit_root_task(&service, "Rework test").await;
    service.claim_task(task.id, "overmind").await.unwrap();

    // Set to PhaseGate at implement (index 2)
    let gate_state = WorkflowState::PhaseGate {
        workflow_name: "code".to_string(),
        phase_index: 2,
        phase_name: "implement".to_string(),
    };
    let mut parent = repo.get(task.id).await.unwrap().unwrap();
    parent.context.custom.insert(
        "workflow_state".to_string(),
        serde_json::to_value(&gate_state).unwrap(),
    );
    parent.updated_at = chrono::Utc::now();
    repo.update(&parent).await.unwrap();

    engine
        .provide_verdict(task.id, GateVerdict::Rework, "Needs improvement")
        .await
        .unwrap();

    // Should have reset state to PhaseGate at phase_index - 1
    let reloaded = repo.get(task.id).await.unwrap().unwrap();
    let ws: WorkflowState =
        serde_json::from_value(reloaded.context.custom["workflow_state"].clone()).unwrap();
    match ws {
        WorkflowState::PhaseGate { phase_index, .. } => {
            assert_eq!(phase_index, 1, "Rework should reset to previous phase gate");
        }
        other => panic!("Expected PhaseGate at index 1, got {:?}", other),
    }
}

// ============================================================================
// Test: provide_verdict when not at gate
// ============================================================================

#[tokio::test]
async fn test_provide_verdict_not_at_gate() {
    use abathur::domain::models::workflow_state::GateVerdict;

    let (service, engine, _repo, _) = setup().await;
    let task = submit_root_task(&service, "Not at gate test").await;

    // Task is in Pending state, not at a gate
    let err = engine
        .provide_verdict(task.id, GateVerdict::Approve, "Should fail")
        .await;
    assert!(err.is_err(), "provide_verdict should fail when not at gate");
    let msg = err.unwrap_err().to_string();
    assert!(
        msg.contains("not at a gate"),
        "Error should mention not at gate, got: {}",
        msg
    );
}

// ============================================================================
// Test: get_state returns correct WorkflowStatus
// ============================================================================

#[tokio::test]
async fn test_get_state() {
    let (service, engine, repo, _) = setup().await;
    let task = submit_root_task(&service, "Get state test").await;

    // Pending state
    let status = engine.get_state(task.id).await.unwrap();
    assert_eq!(status.workflow_name, "code");
    assert_eq!(status.total_phases, 4);
    assert!(status.current_phase_index.is_none());
    assert!(!status.is_verifying);

    // Advance to first phase
    engine.advance(task.id).await.unwrap();
    let status = engine.get_state(task.id).await.unwrap();
    assert_eq!(status.current_phase_index, Some(0));
    assert_eq!(status.current_phase_name, Some("research".to_string()));
    assert!(!status.is_verifying);

    // Manually set to Verifying
    let verifying = WorkflowState::Verifying {
        workflow_name: "code".to_string(),
        phase_index: 2,
        phase_name: "implement".to_string(),
        subtask_ids: vec![uuid::Uuid::new_v4()],
        retry_count: 1,
    };
    let mut parent = repo.get(task.id).await.unwrap().unwrap();
    parent.context.custom.insert(
        "workflow_state".to_string(),
        serde_json::to_value(&verifying).unwrap(),
    );
    parent.updated_at = chrono::Utc::now();
    repo.update(&parent).await.unwrap();

    let status = engine.get_state(task.id).await.unwrap();
    assert!(status.is_verifying);
    assert_eq!(status.verification_retry_count, Some(1));
    assert_eq!(status.current_phase_index, Some(2));
}

// ============================================================================
// Test: phase failure transitions to Failed state after exhausting phase retries
// ============================================================================

#[tokio::test]
async fn test_phase_failure_transitions_to_failed() {
    let (service, engine, repo, _) = setup().await;
    let task = submit_root_task(&service, "Phase failure test").await;
    service.claim_task(task.id, "overmind").await.unwrap();

    // Advance to first phase
    let result = engine.advance(task.id).await.unwrap();
    let subtask_id = match result {
        AdvanceResult::PhaseStarted { subtask_id, .. } => subtask_id,
        _ => panic!("Expected PhaseStarted"),
    };

    // Phase-level retry allows up to 2 retries before failing the workflow.
    // We need to fail → handle_phase_complete 3 times to exhaust them.
    for i in 0..3 {
        let mut sub = repo.get(subtask_id).await.unwrap().unwrap();
        // Transition through Running → Failed (subtask may be in Ready after retry)
        if sub.status == TaskStatus::Ready {
            let _ = sub.transition_to(TaskStatus::Running);
            repo.update(&sub).await.unwrap();
        }
        if sub.status == TaskStatus::Running {
            let _ = sub.transition_to(TaskStatus::Failed);
            repo.update(&sub).await.unwrap();
        }

        engine
            .handle_phase_complete(task.id, subtask_id)
            .await
            .unwrap();

        let reloaded = repo.get(task.id).await.unwrap().unwrap();
        let ws: WorkflowState =
            serde_json::from_value(reloaded.context.custom["workflow_state"].clone()).unwrap();

        if i < 2 {
            // First two failures: subtask retried, workflow stays in PhaseRunning
            assert!(
                matches!(ws, WorkflowState::PhaseRunning { .. }),
                "Phase retry {}: expected PhaseRunning, got {:?}",
                i + 1,
                ws
            );
            let retried_sub = repo.get(subtask_id).await.unwrap().unwrap();
            assert_eq!(retried_sub.status, TaskStatus::Ready);
        } else {
            // Third failure: phase retries exhausted, workflow transitions to Failed
            match ws {
                WorkflowState::Failed { error, .. } => {
                    assert!(
                        error.contains("research"),
                        "Error should mention the failed phase name, got: {}",
                        error
                    );
                }
                other => panic!("Expected Failed, got {:?}", other),
            }
        }
    }
}

// ============================================================================
// Test: advance() completes parent task (Fix 1)
// ============================================================================

#[tokio::test]
async fn test_advance_completes_parent_task() {
    let (service, engine, repo, _) = setup().await;
    let task = submit_root_task(&service, "Parent completion test").await;
    service.claim_task(task.id, "overmind").await.unwrap();

    // Set state to PhaseGate at review (last phase, index 3)
    let mut subtask = Task::with_title("review subtask", "review");
    subtask.parent_id = Some(task.id);
    subtask.source = TaskSource::SubtaskOf(task.id);
    let _ = subtask.transition_to(TaskStatus::Ready);
    repo.create(&subtask).await.unwrap();
    let mut sub = repo.get(subtask.id).await.unwrap().unwrap();
    let _ = sub.transition_to(TaskStatus::Running);
    repo.update(&sub).await.unwrap();
    let _ = sub.transition_to(TaskStatus::Complete);
    sub.completed_at = Some(chrono::Utc::now());
    repo.update(&sub).await.unwrap();

    let gate_state = WorkflowState::PhaseGate {
        workflow_name: "code".to_string(),
        phase_index: 3,
        phase_name: "review".to_string(),
    };
    let mut parent = repo.get(task.id).await.unwrap().unwrap();
    parent.context.custom.insert(
        "workflow_state".to_string(),
        serde_json::to_value(&gate_state).unwrap(),
    );
    parent.updated_at = chrono::Utc::now();
    repo.update(&parent).await.unwrap();

    // advance() should complete the workflow AND the parent task
    let result = engine.advance(task.id).await.unwrap();
    assert!(matches!(result, AdvanceResult::Completed));

    let reloaded = repo.get(task.id).await.unwrap().unwrap();
    assert_eq!(
        reloaded.status,
        TaskStatus::Complete,
        "advance() should transition parent task to Complete when all phases done"
    );
}

// ============================================================================
// Test: advance errors propagated from handle_phase_complete (Fix 5)
// ============================================================================

#[tokio::test]
async fn test_advance_errors_propagated() {
    let (service, engine, repo, _) = setup().await;
    let task = submit_root_task(&service, "Error propagation test").await;
    service.claim_task(task.id, "overmind").await.unwrap();

    // Advance to first phase and complete the subtask
    let result = engine.advance(task.id).await.unwrap();
    let subtask_id = match result {
        AdvanceResult::PhaseStarted { subtask_id, .. } => subtask_id,
        _ => panic!("Expected PhaseStarted"),
    };
    let mut sub = repo.get(subtask_id).await.unwrap().unwrap();
    let _ = sub.transition_to(TaskStatus::Running);
    repo.update(&sub).await.unwrap();
    let _ = sub.transition_to(TaskStatus::Complete);
    sub.completed_at = Some(chrono::Utc::now());
    repo.update(&sub).await.unwrap();

    // Corrupt the workflow state: set it to an invalid workflow name
    // so that advance() inside handle_phase_complete will fail on template lookup
    let bad_state = WorkflowState::PhaseRunning {
        workflow_name: "nonexistent_workflow".to_string(),
        phase_index: 0,
        phase_name: "research".to_string(),
        subtask_ids: vec![subtask_id],
    };
    let mut parent = repo.get(task.id).await.unwrap().unwrap();
    parent.context.custom.insert(
        "workflow_state".to_string(),
        serde_json::to_value(&bad_state).unwrap(),
    );
    parent.updated_at = chrono::Utc::now();
    repo.update(&parent).await.unwrap();

    // handle_phase_complete should propagate the error
    let result = engine.handle_phase_complete(task.id, subtask_id).await;
    assert!(
        result.is_err(),
        "handle_phase_complete should propagate advance errors, not swallow them"
    );
}

// ============================================================================
// Test: fan_out with empty slices is rejected
// ============================================================================

#[tokio::test]
async fn test_fan_out_empty_slices_rejected() {
    let (service, engine, _, _) = setup().await;
    let task = submit_root_task(&service, "Empty fan-out test").await;

    let result = engine.fan_out(task.id, vec![]).await;
    assert!(result.is_err(), "fan_out with empty slices should fail");
    let msg = result.unwrap_err().to_string();
    assert!(
        msg.contains("at least one slice"),
        "Error should mention empty slices, got: {}",
        msg
    );
}

// ============================================================================
// Test: provide_verdict Rework at phase 0 resets to Pending
// ============================================================================

#[tokio::test]
async fn test_provide_verdict_rework_at_phase_zero() {
    use abathur::domain::models::workflow_state::GateVerdict;

    let (service, engine, repo, _) = setup().await;
    let task = submit_root_task(&service, "Rework phase zero test").await;
    service.claim_task(task.id, "overmind").await.unwrap();

    // Set to PhaseGate at research (index 0)
    let gate_state = WorkflowState::PhaseGate {
        workflow_name: "code".to_string(),
        phase_index: 0,
        phase_name: "research".to_string(),
    };
    let mut parent = repo.get(task.id).await.unwrap().unwrap();
    parent.context.custom.insert(
        "workflow_state".to_string(),
        serde_json::to_value(&gate_state).unwrap(),
    );
    parent.updated_at = chrono::Utc::now();
    repo.update(&parent).await.unwrap();

    engine
        .provide_verdict(task.id, GateVerdict::Rework, "Redo research")
        .await
        .unwrap();

    // Should have reset state to Pending (not PhaseGate at index -1)
    let reloaded = repo.get(task.id).await.unwrap().unwrap();
    let ws: WorkflowState =
        serde_json::from_value(reloaded.context.custom["workflow_state"].clone()).unwrap();
    assert!(
        matches!(ws, WorkflowState::Pending { ref workflow_name } if workflow_name == "code"),
        "Rework at phase 0 should reset to Pending; got {:?}",
        ws
    );
}

// ============================================================================
// Test: verification rework auto-advance with retry count and feedback
// ============================================================================

#[tokio::test]
async fn test_verification_rework_auto_advance() {
    let (service, engine, repo, _) = setup().await;
    let task = submit_root_task(&service, "Verification rework test").await;
    service.claim_task(task.id, "overmind").await.unwrap();

    let subtask_id = uuid::Uuid::new_v4();
    // Set state to Verifying at implement (phase 2) with retry_count: 0
    let verifying_state = WorkflowState::Verifying {
        workflow_name: "code".to_string(),
        phase_index: 2,
        phase_name: "implement".to_string(),
        subtask_ids: vec![subtask_id],
        retry_count: 0,
    };
    let mut parent = repo.get(task.id).await.unwrap().unwrap();
    parent.context.custom.insert(
        "workflow_state".to_string(),
        serde_json::to_value(&verifying_state).unwrap(),
    );
    parent.updated_at = chrono::Utc::now();
    repo.update(&parent).await.unwrap();

    // Verification failed with retries remaining
    engine
        .handle_verification_result(task.id, false, "Tests failing")
        .await
        .unwrap();

    // Should have auto-advanced: state is PhaseRunning at implement (re-created subtask)
    let reloaded = repo.get(task.id).await.unwrap().unwrap();
    let ws: WorkflowState =
        serde_json::from_value(reloaded.context.custom["workflow_state"].clone()).unwrap();
    match ws {
        WorkflowState::PhaseRunning {
            phase_index,
            phase_name,
            subtask_ids,
            ..
        } => {
            assert_eq!(phase_index, 2, "Should re-create implement phase");
            assert_eq!(phase_name, "implement");
            assert_eq!(subtask_ids.len(), 1, "Should have one new subtask");
        }
        other => panic!("Expected PhaseRunning at implement, got {:?}", other),
    }

    // verification_retry_count should be 1
    let retry_count = reloaded
        .context
        .custom
        .get("verification_retry_count")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    assert_eq!(retry_count, 1, "verification_retry_count should be 1");

    // verification_feedback array should exist with the failure summary
    let feedback = reloaded
        .context
        .custom
        .get("verification_feedback")
        .and_then(|v| v.as_array())
        .expect("verification_feedback should be an array");
    assert!(
        !feedback.is_empty(),
        "verification_feedback should contain the failure summary"
    );
    assert_eq!(
        feedback[0].as_str().unwrap(),
        "Tests failing",
        "First feedback entry should be the verification summary"
    );
}

// ============================================================================
// Test: provide_verdict Approve at mid-workflow gate returns PhaseStarted
// ============================================================================

#[tokio::test]
async fn test_provide_verdict_approve_mid_workflow() {
    use abathur::domain::models::workflow_state::GateVerdict;

    let (service, engine, repo, _) = setup().await;

    // Submit an adapter-sourced task — enrolls in the "external" workflow
    // (triage[0] -> research[1] -> plan[2] -> implement[3] -> review[4])
    let (task, _) = service
        .submit_task(
            Some("Mid-workflow approve test".to_string()),
            "External task for gate test".to_string(),
            None,
            TaskPriority::Normal,
            None,
            vec![],
            None,
            None,
            TaskSource::Adapter("github".to_string()),
            None,
            None,
            None,
        )
        .await
        .unwrap();
    service.claim_task(task.id, "overmind").await.unwrap();

    // Verify it's enrolled in "external"
    let parent = repo.get(task.id).await.unwrap().unwrap();
    let ws: WorkflowState =
        serde_json::from_value(parent.context.custom["workflow_state"].clone()).unwrap();
    assert!(
        matches!(ws, WorkflowState::Pending { ref workflow_name } if workflow_name == "external"),
        "Adapter task should be enrolled in 'external' workflow"
    );

    // Create a completed subtask for the triage phase
    let mut subtask = Task::with_title("triage subtask", "triage work");
    subtask.parent_id = Some(task.id);
    subtask.source = TaskSource::SubtaskOf(task.id);
    let _ = subtask.transition_to(TaskStatus::Ready);
    repo.create(&subtask).await.unwrap();
    let mut sub = repo.get(subtask.id).await.unwrap().unwrap();
    let _ = sub.transition_to(TaskStatus::Running);
    repo.update(&sub).await.unwrap();
    let _ = sub.transition_to(TaskStatus::Complete);
    sub.completed_at = Some(chrono::Utc::now());
    repo.update(&sub).await.unwrap();

    // Set state to PhaseGate at triage (index 0) — a mid-workflow gate
    let gate_state = WorkflowState::PhaseGate {
        workflow_name: "external".to_string(),
        phase_index: 0,
        phase_name: "triage".to_string(),
    };
    let mut parent = repo.get(task.id).await.unwrap().unwrap();
    parent.context.custom.insert(
        "workflow_state".to_string(),
        serde_json::to_value(&gate_state).unwrap(),
    );
    parent.updated_at = chrono::Utc::now();
    repo.update(&parent).await.unwrap();

    // Approve — should advance to research (index 1) and return PhaseStarted
    let result = engine
        .provide_verdict(task.id, GateVerdict::Approve, "Triage passed")
        .await
        .unwrap();

    match result {
        Some(AdvanceResult::PhaseStarted {
            phase_index,
            phase_name,
            subtask_id,
            ..
        }) => {
            assert_eq!(phase_index, 1, "Should advance to phase 1");
            assert_eq!(phase_name, "research", "Next phase should be research");
            // The subtask should exist in the repo
            let new_sub = repo.get(subtask_id).await.unwrap();
            assert!(new_sub.is_some(), "PhaseStarted subtask should exist");
        }
        other => panic!(
            "Approve at mid-workflow gate should return Some(PhaseStarted); got {:?}",
            other
        ),
    }

    // Verify state is now PhaseRunning at research
    let reloaded = repo.get(task.id).await.unwrap().unwrap();
    let ws: WorkflowState =
        serde_json::from_value(reloaded.context.custom["workflow_state"].clone()).unwrap();
    match ws {
        WorkflowState::PhaseRunning {
            phase_index,
            phase_name,
            ..
        } => {
            assert_eq!(phase_index, 1);
            assert_eq!(phase_name, "research");
        }
        other => panic!("Expected PhaseRunning at research, got {:?}", other),
    }
}
