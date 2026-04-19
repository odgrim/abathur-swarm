use std::sync::Arc;

use uuid::Uuid;

use crate::adapters::sqlite::test_support;
use crate::domain::models::task::{Task, TaskSource, TaskStatus};
use crate::domain::models::workflow_state::WorkflowState;
use crate::domain::models::workflow_template::WorkflowTemplate;
use crate::domain::ports::TaskRepository;
use crate::services::event_bus::{EventBus, EventBusConfig, EventPayload};
use crate::services::task_service::TaskService;

use super::validators::{is_gate_phase, validate_state_consistency};
use super::{AdvanceResult, WorkflowEngine};

/// Load the embedded default workflow YAMLs into a name→template map.
fn default_templates() -> std::collections::HashMap<String, WorkflowTemplate> {
    WorkflowTemplate::parse_all_embedded_defaults().expect("embedded test fixture must parse")
}

/// Construct a `WorkflowEngine` preloaded with the default workflow templates.
fn test_engine<T>(
    task_repo: Arc<T>,
    task_service: TaskService<T>,
    event_bus: Arc<EventBus>,
    verification_enabled: bool,
) -> WorkflowEngine<T>
where
    T: crate::domain::ports::TaskRepository + 'static,
{
    WorkflowEngine::new(task_repo, task_service, event_bus, verification_enabled)
        .with_templates(default_templates())
}

#[test]
fn test_is_gate_phase() {
    let templates = default_templates();
    // External workflow: triage (idx 0) and validation (idx 1) are gates
    assert!(is_gate_phase(&templates, "external", 0, "triage"));
    assert!(is_gate_phase(&templates, "external", 1, "validation"));
    // External workflow: review (idx 5) is a gate
    assert!(is_gate_phase(&templates, "external", 5, "review"));
    // Code workflow: review (idx 3) is a gate
    assert!(is_gate_phase(&templates, "code", 3, "review"));
    // Code workflow: implement (idx 2) is NOT a gate
    assert!(!is_gate_phase(&templates, "code", 2, "implement"));
    // Code workflow: research (idx 0) is NOT a gate
    assert!(!is_gate_phase(&templates, "code", 0, "research"));
    // Fallback for unknown workflow
    assert!(is_gate_phase(&templates, "unknown", 0, "review"));
    assert!(!is_gate_phase(&templates, "unknown", 0, "implement"));
}

#[tokio::test]
async fn test_handle_phase_complete_fails_parent_task() {
    // Setup
    let task_repo = test_support::setup_task_repo().await;
    let event_bus = Arc::new(EventBus::new(EventBusConfig::default()));

    let task_service = TaskService::new(task_repo.clone());
    let engine = test_engine(task_repo.clone(), task_service, event_bus.clone(), false);

    // Create a parent task enrolled in a workflow, in Running state
    let mut parent = Task::with_title("Parent workflow task", "Do work");
    parent.transition_to(TaskStatus::Ready).unwrap();
    parent.transition_to(TaskStatus::Running).unwrap();

    // Set workflow state to PhaseRunning with a subtask
    let mut subtask = Task::with_title("Phase subtask", "Subtask work");
    subtask.parent_id = Some(parent.id);
    subtask.source = TaskSource::SubtaskOf(parent.id);
    subtask.transition_to(TaskStatus::Ready).unwrap();
    subtask.transition_to(TaskStatus::Running).unwrap();
    subtask.transition_to(TaskStatus::Failed).unwrap();
    subtask.max_retries = 0; // No retries allowed

    task_repo.create(&parent).await.unwrap();
    task_repo.create(&subtask).await.unwrap();

    // Write PhaseRunning workflow state on parent
    let ws = WorkflowState::PhaseRunning {
        workflow_name: "code".to_string(),
        phase_index: 0,
        phase_name: "implement".to_string(),
        subtask_ids: vec![subtask.id],
    };
    engine.write_state(parent.id, &ws).await.unwrap();

    // Now call handle_phase_complete — the subtask has failed and has no retries
    let result = engine.handle_phase_complete(parent.id, subtask.id).await;
    assert!(
        result.is_ok(),
        "handle_phase_complete should succeed: {:?}",
        result.err()
    );

    // Verify parent task is now Failed
    let updated_parent = task_repo.get(parent.id).await.unwrap().unwrap();
    assert_eq!(
        updated_parent.status,
        TaskStatus::Failed,
        "Parent task should be Failed after phase failure"
    );

    // Verify workflow state is Failed
    let ws = updated_parent
        .workflow_state()
        .expect("workflow_state present");
    assert!(
        matches!(ws, WorkflowState::Failed { .. }),
        "workflow_state should be Failed"
    );
}

#[tokio::test]
async fn test_validating_to_canceled_transition_allowed() {
    // This tests Fix 1: Validating → Canceled is a valid state transition
    let mut task = Task::new("Test validating cancel");
    task.transition_to(TaskStatus::Ready).unwrap();
    task.transition_to(TaskStatus::Running).unwrap();
    task.transition_to(TaskStatus::Validating).unwrap();

    // Validating → Canceled should succeed
    assert!(
        task.can_transition_to(TaskStatus::Canceled),
        "Validating → Canceled should be a valid transition"
    );
    task.transition_to(TaskStatus::Canceled).unwrap();
    assert_eq!(task.status, TaskStatus::Canceled);
    assert!(task.is_terminal());
    assert!(task.completed_at.is_some());
}

#[tokio::test]
async fn test_workflow_phase_retried_event_emitted_on_retry() {
    let task_repo = test_support::setup_task_repo().await;
    let event_bus = Arc::new(EventBus::new(EventBusConfig::default()));

    let task_service = TaskService::new(task_repo.clone());
    let engine = test_engine(task_repo.clone(), task_service, event_bus.clone(), false);

    // Create parent task in Running state
    let mut parent = Task::with_title("Parent workflow task", "Do work");
    parent.transition_to(TaskStatus::Ready).unwrap();
    parent.transition_to(TaskStatus::Running).unwrap();

    // Create a failed subtask that CAN be retried
    let mut subtask = Task::with_title("Phase subtask", "Subtask work");
    subtask.parent_id = Some(parent.id);
    subtask.source = TaskSource::SubtaskOf(parent.id);
    subtask.max_retries = 3;
    subtask.transition_to(TaskStatus::Ready).unwrap();
    subtask.transition_to(TaskStatus::Running).unwrap();
    subtask.transition_to(TaskStatus::Failed).unwrap();

    task_repo.create(&parent).await.unwrap();
    task_repo.create(&subtask).await.unwrap();

    // Write PhaseRunning workflow state on parent
    let ws = WorkflowState::PhaseRunning {
        workflow_name: "code".to_string(),
        phase_index: 1,
        phase_name: "implement".to_string(),
        subtask_ids: vec![subtask.id],
    };
    engine.write_state(parent.id, &ws).await.unwrap();

    // Subscribe to events before the action
    let mut rx = event_bus.subscribe();

    // Call handle_phase_complete — subtask failed but can retry
    let result = engine.handle_phase_complete(parent.id, subtask.id).await;
    assert!(
        result.is_ok(),
        "handle_phase_complete should succeed: {:?}",
        result.err()
    );

    // Collect emitted events
    let mut found_retried = false;
    while let Ok(event) = rx.try_recv() {
        if let EventPayload::WorkflowPhaseRetried {
            task_id,
            phase_index,
            phase_name,
            retry_count,
        } = &event.payload
        {
            assert_eq!(*task_id, parent.id);
            assert_eq!(*phase_index, 1);
            assert_eq!(phase_name, "implement");
            assert_eq!(*retry_count, 1);
            found_retried = true;
        }
    }
    assert!(
        found_retried,
        "WorkflowPhaseRetried event should have been emitted"
    );
}

#[tokio::test]
async fn test_workflow_phase_failed_event_emitted_when_retries_exhausted() {
    let task_repo = test_support::setup_task_repo().await;
    let event_bus = Arc::new(EventBus::new(EventBusConfig::default()));

    let task_service = TaskService::new(task_repo.clone());
    let engine = test_engine(task_repo.clone(), task_service, event_bus.clone(), false);

    // Create parent task in Running state
    let mut parent = Task::with_title("Parent workflow task", "Do work");
    parent.transition_to(TaskStatus::Ready).unwrap();
    parent.transition_to(TaskStatus::Running).unwrap();

    // Create a failed subtask with NO retries
    let mut subtask = Task::with_title("Phase subtask", "Subtask work");
    subtask.parent_id = Some(parent.id);
    subtask.source = TaskSource::SubtaskOf(parent.id);
    subtask.max_retries = 0;
    subtask.transition_to(TaskStatus::Ready).unwrap();
    subtask.transition_to(TaskStatus::Running).unwrap();
    subtask.transition_to(TaskStatus::Failed).unwrap();

    task_repo.create(&parent).await.unwrap();
    task_repo.create(&subtask).await.unwrap();

    // Write PhaseRunning workflow state on parent
    let ws = WorkflowState::PhaseRunning {
        workflow_name: "code".to_string(),
        phase_index: 0,
        phase_name: "implement".to_string(),
        subtask_ids: vec![subtask.id],
    };
    engine.write_state(parent.id, &ws).await.unwrap();

    // Subscribe to events before the action
    let mut rx = event_bus.subscribe();

    // Call handle_phase_complete — subtask failed with no retries
    let result = engine.handle_phase_complete(parent.id, subtask.id).await;
    assert!(
        result.is_ok(),
        "handle_phase_complete should succeed: {:?}",
        result.err()
    );

    // Collect emitted events
    let mut found_failed = false;
    while let Ok(event) = rx.try_recv() {
        if let EventPayload::WorkflowPhaseFailed {
            task_id,
            phase_index,
            phase_name,
            reason,
        } = &event.payload
        {
            assert_eq!(*task_id, parent.id);
            assert_eq!(*phase_index, 0);
            assert_eq!(phase_name, "implement");
            assert!(
                reason.contains("failed after"),
                "reason should mention retries: {}",
                reason
            );
            found_failed = true;
        }
    }
    assert!(
        found_failed,
        "WorkflowPhaseFailed event should have been emitted"
    );
}

// --- validate_state_consistency tests (Fix 1 / Fix 8) ---

#[test]
fn test_validate_state_consistency_catches_validating_plus_phase_ready() {
    let ws = WorkflowState::PhaseReady {
        workflow_name: "code".to_string(),
        phase_index: 0,
        phase_name: "implement".to_string(),
    };
    let result = validate_state_consistency(TaskStatus::Validating, &ws);
    assert!(
        result.is_err(),
        "Validating + PhaseReady should be inconsistent"
    );
    let msg = result.unwrap_err();
    assert!(
        msg.contains("Validating") && msg.contains("Verifying"),
        "Error should mention Validating/Verifying, got: {}",
        msg
    );
}

#[test]
fn test_validate_state_consistency_accepts_valid_combinations() {
    // Running + PhaseReady → Ok
    assert!(
        validate_state_consistency(
            TaskStatus::Running,
            &WorkflowState::PhaseReady {
                workflow_name: "code".to_string(),
                phase_index: 0,
                phase_name: "implement".to_string(),
            },
        )
        .is_ok(),
        "Running + PhaseReady should be valid"
    );

    // Running + PhaseRunning → Ok
    assert!(
        validate_state_consistency(
            TaskStatus::Running,
            &WorkflowState::PhaseRunning {
                workflow_name: "code".to_string(),
                phase_index: 0,
                phase_name: "implement".to_string(),
                subtask_ids: vec![],
            },
        )
        .is_ok(),
        "Running + PhaseRunning should be valid"
    );

    // Validating + Verifying → Ok
    assert!(
        validate_state_consistency(
            TaskStatus::Validating,
            &WorkflowState::Verifying {
                workflow_name: "code".to_string(),
                phase_index: 0,
                phase_name: "implement".to_string(),
                subtask_ids: vec![],
                retry_count: 0,
            },
        )
        .is_ok(),
        "Validating + Verifying should be valid"
    );

    // Complete + Completed → Ok
    assert!(
        validate_state_consistency(
            TaskStatus::Complete,
            &WorkflowState::Completed {
                workflow_name: "code".to_string(),
            },
        )
        .is_ok(),
        "Complete + Completed should be valid"
    );

    // Pending + Pending → Ok
    assert!(
        validate_state_consistency(
            TaskStatus::Pending,
            &WorkflowState::Pending {
                workflow_name: "code".to_string(),
            },
        )
        .is_ok(),
        "Pending + Pending should be valid"
    );
}

// ── advance() tests ──────────────────────────────────────────────

#[tokio::test]
async fn test_advance_from_pending_succeeds() {
    let task_repo = test_support::setup_task_repo().await;
    let event_bus = Arc::new(EventBus::new(EventBusConfig::default()));
    let task_service = TaskService::new(task_repo.clone());
    let engine = test_engine(task_repo.clone(), task_service, event_bus.clone(), false);

    // Create a task in Running status with Pending workflow state
    let mut task = Task::with_title("Advance pending test", "desc");
    task.transition_to(TaskStatus::Ready).unwrap();
    task.transition_to(TaskStatus::Running).unwrap();
    task_repo.create(&task).await.unwrap();

    let ws = WorkflowState::Pending {
        workflow_name: "code".to_string(),
    };
    engine.write_state(task.id, &ws).await.unwrap();

    let result = engine.advance(task.id).await;
    assert!(
        result.is_ok(),
        "advance from Pending should succeed: {:?}",
        result.err()
    );

    match result.unwrap() {
        AdvanceResult::PhaseReady { phase_index, .. } => {
            assert_eq!(phase_index, 0, "Should advance to phase 0");
        }
        other => panic!("Expected PhaseReady, got {:?}", other),
    }
}

#[tokio::test]
async fn test_advance_from_phase_gate_moves_to_next_phase() {
    let task_repo = test_support::setup_task_repo().await;
    let event_bus = Arc::new(EventBus::new(EventBusConfig::default()));
    let task_service = TaskService::new(task_repo.clone());
    let engine = test_engine(task_repo.clone(), task_service, event_bus.clone(), false);

    let mut task = Task::with_title("Advance gate test", "desc");
    task.transition_to(TaskStatus::Ready).unwrap();
    task.transition_to(TaskStatus::Running).unwrap();
    task_repo.create(&task).await.unwrap();

    let ws = WorkflowState::PhaseGate {
        workflow_name: "code".to_string(),
        phase_index: 0,
        phase_name: "research".to_string(),
    };
    engine.write_state(task.id, &ws).await.unwrap();

    let result = engine.advance(task.id).await;
    assert!(
        result.is_ok(),
        "advance from PhaseGate should succeed: {:?}",
        result.err()
    );

    match result.unwrap() {
        AdvanceResult::PhaseReady { phase_index, .. } => {
            assert_eq!(phase_index, 1, "Should advance to phase 1");
        }
        other => panic!("Expected PhaseReady, got {:?}", other),
    }
}

#[tokio::test]
async fn test_advance_from_phase_ready_returns_error() {
    let task_repo = test_support::setup_task_repo().await;
    let event_bus = Arc::new(EventBus::new(EventBusConfig::default()));
    let task_service = TaskService::new(task_repo.clone());
    let engine = test_engine(task_repo.clone(), task_service, event_bus.clone(), false);

    let mut task = Task::with_title("Advance ready test", "desc");
    task.transition_to(TaskStatus::Ready).unwrap();
    task.transition_to(TaskStatus::Running).unwrap();
    task_repo.create(&task).await.unwrap();

    let ws = WorkflowState::PhaseReady {
        workflow_name: "code".to_string(),
        phase_index: 0,
        phase_name: "research".to_string(),
    };
    engine.write_state(task.id, &ws).await.unwrap();

    let result = engine.advance(task.id).await;
    assert!(result.is_err(), "advance from PhaseReady should fail");
    let err_msg = format!("{:?}", result.unwrap_err());
    assert!(
        err_msg.contains("PhaseReady"),
        "Error should mention PhaseReady, got: {}",
        err_msg
    );
}

#[tokio::test]
async fn test_advance_from_phase_running_with_active_subtasks_fails() {
    let task_repo = test_support::setup_task_repo().await;
    let event_bus = Arc::new(EventBus::new(EventBusConfig::default()));
    let task_service = TaskService::new(task_repo.clone());
    let engine = test_engine(task_repo.clone(), task_service, event_bus.clone(), false);

    // Create parent task
    let mut parent = Task::with_title("Advance running test", "desc");
    parent.transition_to(TaskStatus::Ready).unwrap();
    parent.transition_to(TaskStatus::Running).unwrap();

    // Create a still-running subtask
    let mut subtask = Task::with_title("Running subtask", "work");
    subtask.parent_id = Some(parent.id);
    subtask.source = TaskSource::SubtaskOf(parent.id);
    subtask.transition_to(TaskStatus::Ready).unwrap();
    subtask.transition_to(TaskStatus::Running).unwrap();

    task_repo.create(&parent).await.unwrap();
    task_repo.create(&subtask).await.unwrap();

    let ws = WorkflowState::PhaseRunning {
        workflow_name: "code".to_string(),
        phase_index: 0,
        phase_name: "research".to_string(),
        subtask_ids: vec![subtask.id],
    };
    engine.write_state(parent.id, &ws).await.unwrap();

    let result = engine.advance(parent.id).await;
    assert!(result.is_err(), "advance with active subtasks should fail");
    let err_msg = format!("{:?}", result.unwrap_err());
    assert!(
        err_msg.contains("still running"),
        "Error should mention still running, got: {}",
        err_msg
    );
}

#[tokio::test]
async fn test_advance_nonexistent_task_returns_not_found() {
    let task_repo = test_support::setup_task_repo().await;
    let event_bus = Arc::new(EventBus::new(EventBusConfig::default()));
    let task_service = TaskService::new(task_repo.clone());
    let engine = test_engine(task_repo.clone(), task_service, event_bus.clone(), false);

    let fake_id = Uuid::new_v4();
    let result = engine.advance(fake_id).await;
    assert!(result.is_err(), "advance on nonexistent task should fail");
    match result.unwrap_err() {
        crate::domain::errors::DomainError::TaskNotFound(id) => assert_eq!(id, fake_id),
        other => panic!("Expected TaskNotFound, got: {:?}", other),
    }
}

// ── fan_out() tests ──────────────────────────────────────────────

#[tokio::test]
async fn test_fan_out_creates_subtasks() {
    use crate::domain::models::workflow_state::FanOutSlice;

    let task_repo = test_support::setup_task_repo().await;
    let event_bus = Arc::new(EventBus::new(EventBusConfig::default()));
    let task_service = TaskService::new(task_repo.clone());
    let engine = test_engine(task_repo.clone(), task_service, event_bus.clone(), false);

    // Create parent in PhaseReady
    let mut task = Task::with_title("Fan out test", "desc");
    task.transition_to(TaskStatus::Ready).unwrap();
    task.transition_to(TaskStatus::Running).unwrap();
    task_repo.create(&task).await.unwrap();

    let ws = WorkflowState::PhaseReady {
        workflow_name: "code".to_string(),
        phase_index: 0,
        phase_name: "research".to_string(),
    };
    engine.write_state(task.id, &ws).await.unwrap();

    let slices = vec![
        FanOutSlice {
            description: "Slice A".to_string(),
            agent: Some("rust-implementer".to_string()),
            context: Default::default(),
        },
        FanOutSlice {
            description: "Slice B".to_string(),
            agent: Some("rust-implementer".to_string()),
            context: Default::default(),
        },
    ];

    let result = engine.fan_out(task.id, slices).await;
    assert!(result.is_ok(), "fan_out should succeed: {:?}", result.err());

    let fan_out_result = result.unwrap();
    assert_eq!(
        fan_out_result.subtask_ids.len(),
        2,
        "Should create 2 subtasks"
    );
    assert_eq!(fan_out_result.phase_index, 0);
}

#[tokio::test]
async fn test_fan_out_empty_slices_returns_error() {
    let task_repo = test_support::setup_task_repo().await;
    let event_bus = Arc::new(EventBus::new(EventBusConfig::default()));
    let task_service = TaskService::new(task_repo.clone());
    let engine = test_engine(task_repo.clone(), task_service, event_bus.clone(), false);

    let fake_id = Uuid::new_v4();
    let result = engine.fan_out(fake_id, vec![]).await;
    assert!(result.is_err(), "fan_out with empty slices should fail");
    let err_msg = format!("{:?}", result.unwrap_err());
    assert!(
        err_msg.contains("at least one slice"),
        "Error should mention needing slices, got: {}",
        err_msg
    );
}

#[tokio::test]
async fn test_fan_out_missing_agent_returns_error() {
    use crate::domain::models::workflow_state::FanOutSlice;

    let task_repo = test_support::setup_task_repo().await;
    let event_bus = Arc::new(EventBus::new(EventBusConfig::default()));
    let task_service = TaskService::new(task_repo.clone());
    let engine = test_engine(task_repo.clone(), task_service, event_bus.clone(), false);

    let fake_id = Uuid::new_v4();
    let slices = vec![FanOutSlice {
        description: "No agent slice".to_string(),
        agent: None,
        context: Default::default(),
    }];

    let result = engine.fan_out(fake_id, slices).await;
    assert!(result.is_err(), "fan_out with missing agent should fail");
    let err_msg = format!("{:?}", result.unwrap_err());
    assert!(
        err_msg.contains("agent"),
        "Error should mention agent, got: {}",
        err_msg
    );
}

#[tokio::test]
async fn test_fan_out_wrong_state_returns_error() {
    use crate::domain::models::workflow_state::FanOutSlice;

    let task_repo = test_support::setup_task_repo().await;
    let event_bus = Arc::new(EventBus::new(EventBusConfig::default()));
    let task_service = TaskService::new(task_repo.clone());
    let engine = test_engine(task_repo.clone(), task_service, event_bus.clone(), false);

    // Create parent in PhaseRunning (not PhaseReady)
    let mut task = Task::with_title("Fan out wrong state", "desc");
    task.transition_to(TaskStatus::Ready).unwrap();
    task.transition_to(TaskStatus::Running).unwrap();
    task_repo.create(&task).await.unwrap();

    let ws = WorkflowState::PhaseRunning {
        workflow_name: "code".to_string(),
        phase_index: 0,
        phase_name: "research".to_string(),
        subtask_ids: vec![],
    };
    engine.write_state(task.id, &ws).await.unwrap();

    let slices = vec![FanOutSlice {
        description: "Should fail".to_string(),
        agent: Some("rust-implementer".to_string()),
        context: Default::default(),
    }];

    let result = engine.fan_out(task.id, slices).await;
    assert!(result.is_err(), "fan_out from PhaseRunning should fail");
    let err_msg = format!("{:?}", result.unwrap_err());
    assert!(
        err_msg.contains("PhaseReady"),
        "Error should mention PhaseReady, got: {}",
        err_msg
    );
}
