//! End-to-end workflow tests.
//!
//! These tests exercise the full lifecycle starting from the CLI binary (via
//! `assert_cmd`) through the workflow engine, validating phase progression,
//! gate verdicts, verification, fan-out/aggregation, and convergent skip.
//!
//! Each test creates its own `TempDir` for full isolation.  The CLI creates the
//! database via `abathur init`, then subsequent assertions open the same database
//! through the library API to inspect workflow state.

use std::path::Path;
use std::sync::Arc;

use assert_cmd::Command;
use serde_json::Value;
use tempfile::TempDir;

use abathur::adapters::sqlite::{initialize_database, SqliteTaskRepository};
use abathur::domain::models::task::{Task, TaskStatus};
use abathur::domain::models::workflow_state::{FanOutSlice, GateVerdict, WorkflowState};
use abathur::domain::ports::TaskRepository;
use abathur::services::event_bus::{EventBus, EventBusConfig};
use abathur::services::task_service::TaskService;
use abathur::services::workflow_engine::{AdvanceResult, WorkflowEngine};

// ============================================================================
// Test harness
// ============================================================================

mod harness {
    use super::*;

    /// Build an `assert_cmd::Command` pointing at the `abathur` binary with its
    /// working directory set to `dir`.
    pub fn abathur_cmd(dir: &Path) -> Command {
        let mut cmd = assert_cmd::cargo_bin_cmd!("abathur");
        cmd.current_dir(dir);
        cmd
    }

    /// Run `abathur init` in the given directory.
    pub fn init_project(dir: &Path) {
        abathur_cmd(dir)
            .args(["init"])
            .assert()
            .success()
            .stdout(predicates::str::contains("initialized"));
    }

    /// Run a CLI command with `--json`, assert success, return parsed JSON.
    pub fn run_json(dir: &Path, args: &[&str]) -> Value {
        let output = abathur_cmd(dir)
            .args(args)
            .assert()
            .success()
            .get_output()
            .stdout
            .clone();
        serde_json::from_slice(&output)
            .unwrap_or_else(|e| panic!("Failed to parse JSON from {:?}: {}", args, e))
    }

    /// Extract a string field from a JSON value.
    pub fn json_str(val: &Value, key: &str) -> String {
        val.get(key)
            .and_then(|v| v.as_str())
            .unwrap_or_else(|| panic!("Missing string field '{}' in {}", key, val))
            .to_string()
    }

    /// Open the CLI-created database and return shared service handles.
    pub async fn open_db(
        dir: &Path,
    ) -> (
        TaskService<SqliteTaskRepository>,
        WorkflowEngine<SqliteTaskRepository>,
        Arc<SqliteTaskRepository>,
    ) {
        let db_path = dir.join(".abathur/abathur.db");
        let url = format!("sqlite:{}", db_path.display());
        let pool = initialize_database(&url)
            .await
            .expect("Failed to open CLI database");
        let repo = Arc::new(SqliteTaskRepository::new(pool));
        let event_bus = Arc::new(EventBus::new(EventBusConfig::default()));
        let task_service = TaskService::new(repo.clone());
        let engine = WorkflowEngine::new(repo.clone(), event_bus, true);
        (task_service, engine, repo)
    }

    /// Read the `WorkflowState` from a task's context.
    pub fn read_workflow_state(task: &Task) -> WorkflowState {
        let raw = task
            .context
            .custom
            .get("workflow_state")
            .expect("Task should have workflow_state");
        serde_json::from_value(raw.clone()).expect("Failed to deserialize WorkflowState")
    }

    /// Complete a subtask: claim → complete.
    pub async fn complete_subtask(
        service: &TaskService<SqliteTaskRepository>,
        subtask_id: uuid::Uuid,
    ) {
        service
            .claim_task(subtask_id, "test-agent")
            .await
            .expect("Failed to claim subtask");
        service
            .complete_task(subtask_id)
            .await
            .expect("Failed to complete subtask");
    }

    /// Ensure the task is in PhaseReady (call advance if needed), then fan_out with a
    /// single slice to create a subtask. Returns the subtask_id.
    pub async fn advance_and_get_subtask(
        engine: &WorkflowEngine<SqliteTaskRepository>,
        task_id: uuid::Uuid,
    ) -> uuid::Uuid {
        // Try advance — if already in PhaseReady, skip advance and go straight to fan_out
        match engine.advance(task_id).await {
            Ok(AdvanceResult::PhaseReady { .. }) => {}
            Ok(AdvanceResult::Completed) => panic!("Expected PhaseReady, got Completed"),
            Err(e) if e.to_string().contains("already in PhaseReady") => {
                // Already in PhaseReady — just proceed to fan_out
            }
            Err(e) => panic!("advance failed: {}", e),
        }
        let fan_result = engine
            .fan_out(
                task_id,
                vec![FanOutSlice {
                    description: "phase work".to_string(),
                    context: Default::default(),
                }],
            )
            .await
            .expect("fan_out failed");
        assert_eq!(fan_result.subtask_ids.len(), 1);
        fan_result.subtask_ids[0]
    }

    /// Advance a phase, complete the subtask, handle aggregation, then complete.
    /// Since fan_out creates FanningOut state, completion goes through aggregation.
    pub async fn run_phase(
        service: &TaskService<SqliteTaskRepository>,
        engine: &WorkflowEngine<SqliteTaskRepository>,
        task_id: uuid::Uuid,
    ) -> uuid::Uuid {
        use abathur::domain::ports::TaskRepository;

        let subtask_id = advance_and_get_subtask(engine, task_id).await;
        complete_subtask(service, subtask_id).await;
        engine
            .handle_phase_complete(task_id, subtask_id)
            .await
            .expect("handle_phase_complete failed");

        // After fan_out subtask completes, state is Aggregating. Complete the aggregation subtask.
        let task = service.repo().get(task_id).await.unwrap().unwrap();
        let ws = read_workflow_state(&task);
        if let WorkflowState::Aggregating { subtask_ids, .. } = ws {
            let agg_id = *subtask_ids.last().unwrap();
            complete_subtask(service, agg_id).await;
            engine
                .handle_phase_complete(task_id, agg_id)
                .await
                .expect("handle aggregation phase_complete failed");
        }

        subtask_id
    }
}

// ============================================================================
// Test 1: CLI submit auto-enrolls in workflow with Pending state
// ============================================================================

#[tokio::test]
async fn test_cli_submit_auto_enrolls_workflow() {
    let tmp = TempDir::new().expect("tempdir");
    let dir = tmp.path();
    harness::init_project(dir);

    // Submit a task via CLI
    let json = harness::run_json(
        dir,
        &["task", "submit", "Implement feature X", "-t", "Feature X", "--json"],
    );
    assert_eq!(json["success"], true);
    let task_id_str = harness::json_str(&json["task"], "id");

    // Open the same database and inspect
    let (_service, _engine, repo) = harness::open_db(dir).await;
    let task_id: uuid::Uuid = task_id_str.parse().expect("invalid UUID");
    let task = repo.get(task_id).await.expect("db error").expect("task not found");

    // Verify workflow auto-enrollment
    let ws = harness::read_workflow_state(&task);
    match ws {
        WorkflowState::Pending { ref workflow_name } => {
            assert_eq!(workflow_name, "code", "Root human tasks default to 'code' workflow");
        }
        other => panic!("Expected Pending, got {:?}", other),
    }

    // Verify routing hints
    assert_eq!(
        task.routing_hints.workflow_name,
        Some("code".to_string()),
        "routing_hints should record the workflow name"
    );
}

// ============================================================================
// Test 2: Full phase progression through the code workflow
// ============================================================================

#[tokio::test]
async fn test_full_phase_progression_code_workflow() {
    let tmp = TempDir::new().expect("tempdir");
    let dir = tmp.path();
    harness::init_project(dir);

    // Submit via CLI
    let json = harness::run_json(
        dir,
        &["task", "submit", "Build a widget", "-t", "Widget", "--json"],
    );
    let task_id_str = harness::json_str(&json["task"], "id");

    let (service, engine, repo) = harness::open_db(dir).await;
    let task_id: uuid::Uuid = task_id_str.parse().unwrap();

    // Claim parent (must be Running before advance)
    service.claim_task(task_id, "overmind").await.expect("claim parent");

    // Phase 0: research — advance → fan_out → complete → aggregation → PhaseReady
    harness::run_phase(&service, &engine, task_id).await;

    // Phase 1: plan — should be PhaseReady after research completes
    {
        let task = repo.get(task_id).await.unwrap().unwrap();
        let ws = harness::read_workflow_state(&task);
        match &ws {
            WorkflowState::PhaseReady { phase_index, phase_name, .. } => {
                assert_eq!(*phase_index, 1);
                assert_eq!(phase_name, "plan");
            }
            other => panic!("Expected PhaseReady(plan), got {:?}", other),
        }
    }
    harness::run_phase(&service, &engine, task_id).await;

    // Phase 2: implement — should be PhaseReady after plan completes
    {
        let task = repo.get(task_id).await.unwrap().unwrap();
        let ws = harness::read_workflow_state(&task);
        match &ws {
            WorkflowState::PhaseReady { phase_index, phase_name, .. } => {
                assert_eq!(*phase_index, 2);
                assert_eq!(phase_name, "implement");
            }
            other => panic!("Expected PhaseReady(implement), got {:?}", other),
        }
    }
    // Implement phase: advance → fan_out → complete → aggregation → then verification kicks in
    let implement_sub = harness::advance_and_get_subtask(&engine, task_id).await;
    harness::complete_subtask(&service, implement_sub).await;
    engine.handle_phase_complete(task_id, implement_sub).await.expect("handle implement fan_out");

    // Complete the aggregation subtask for implement phase
    {
        let task = repo.get(task_id).await.unwrap().unwrap();
        let ws = harness::read_workflow_state(&task);
        if let WorkflowState::Aggregating { subtask_ids, .. } = ws {
            let agg_id = *subtask_ids.last().unwrap();
            harness::complete_subtask(&service, agg_id).await;
            engine.handle_phase_complete(task_id, agg_id).await.expect("handle implement aggregation");
        } else {
            panic!("Expected Aggregating after implement fan_out, got {:?}", ws);
        }
    }

    // After implement completes, we should be in Verifying (implement has verify: true)
    {
        let task = repo.get(task_id).await.unwrap().unwrap();
        let ws = harness::read_workflow_state(&task);
        match &ws {
            WorkflowState::Verifying { phase_index, phase_name, retry_count, .. } => {
                assert_eq!(*phase_index, 2);
                assert_eq!(phase_name, "implement");
                assert_eq!(*retry_count, 0);
            }
            other => panic!("Expected Verifying(implement), got {:?}", other),
        }
    }

    // Simulate verification passing by advancing from Verifying state
    // (The subtask is already complete, so advance should work)
    let review_result = engine.advance(task_id).await.expect("advance past verification");
    match review_result {
        AdvanceResult::PhaseReady { phase_index, phase_name } => {
            assert_eq!(phase_index, 3);
            assert_eq!(phase_name, "review");

            // fan_out to create review subtask, then complete and handle
            let fan_result = engine
                .fan_out(
                    task_id,
                    vec![FanOutSlice {
                        description: "review work".to_string(),
                        context: Default::default(),
                    }],
                )
                .await
                .expect("fan_out failed");
            let subtask_id = fan_result.subtask_ids[0];
            harness::complete_subtask(&service, subtask_id).await;
            engine.handle_phase_complete(task_id, subtask_id).await.expect("handle review fan_out");

            // Complete aggregation subtask for review phase
            let task = repo.get(task_id).await.unwrap().unwrap();
            let ws = harness::read_workflow_state(&task);
            if let WorkflowState::Aggregating { subtask_ids, .. } = ws {
                let agg_id = *subtask_ids.last().unwrap();
                harness::complete_subtask(&service, agg_id).await;
                engine.handle_phase_complete(task_id, agg_id).await.expect("handle review aggregation");
            }
        }
        AdvanceResult::Completed => panic!("Expected review phase, got Completed"),
    }

    // review is a gate phase → should be at PhaseGate
    {
        let task = repo.get(task_id).await.unwrap().unwrap();
        let ws = harness::read_workflow_state(&task);
        match &ws {
            WorkflowState::PhaseGate { phase_index, phase_name, .. } => {
                assert_eq!(*phase_index, 3);
                assert_eq!(phase_name, "review");
            }
            other => panic!("Expected PhaseGate(review), got {:?}", other),
        }
    }

    // Approve the gate → should complete
    let verdict_result = engine
        .provide_verdict(task_id, GateVerdict::Approve, "Looks good")
        .await
        .expect("provide_verdict");
    match verdict_result {
        Some(AdvanceResult::Completed) => { /* expected */ }
        other => panic!("Expected Completed after gate approval, got {:?}", other),
    }

    // Verify final state
    let final_task = repo.get(task_id).await.unwrap().unwrap();
    let final_ws = harness::read_workflow_state(&final_task);
    assert!(
        matches!(final_ws, WorkflowState::Completed { .. }),
        "Workflow should be Completed, got {:?}",
        final_ws
    );

    // Task itself should be Complete
    assert_eq!(final_task.status, TaskStatus::Complete);
}

// ============================================================================
// Test 3: Gate verdict — reject halts the workflow
// ============================================================================

#[tokio::test]
async fn test_gate_verdict_reject() {
    let tmp = TempDir::new().expect("tempdir");
    let dir = tmp.path();
    harness::init_project(dir);

    let json = harness::run_json(
        dir,
        &["task", "submit", "Rejected task", "-t", "Reject Me", "--json"],
    );
    let task_id: uuid::Uuid = harness::json_str(&json["task"], "id").parse().unwrap();

    let (service, engine, repo) = harness::open_db(dir).await;
    service.claim_task(task_id, "overmind").await.unwrap();

    // Run through research, plan, implement+verify, to reach review gate
    // Phase 0: research
    harness::run_phase(&service, &engine, task_id).await;
    // Phase 1: plan
    harness::run_phase(&service, &engine, task_id).await;
    // Phase 2: implement
    harness::run_phase(&service, &engine, task_id).await;

    // After implement, should be in Verifying — advance past it
    engine.advance(task_id).await.unwrap();

    // Phase 3: review — advance and fan_out to create subtask
    let review_sub = harness::advance_and_get_subtask(&engine, task_id).await;
    harness::complete_subtask(&service, review_sub).await;
    engine.handle_phase_complete(task_id, review_sub).await.unwrap();

    // Complete aggregation for review
    {
        let task = repo.get(task_id).await.unwrap().unwrap();
        let ws = harness::read_workflow_state(&task);
        if let WorkflowState::Aggregating { subtask_ids, .. } = ws {
            let agg_id = *subtask_ids.last().unwrap();
            harness::complete_subtask(&service, agg_id).await;
            engine.handle_phase_complete(task_id, agg_id).await.unwrap();
        }
    }

    // Should be at PhaseGate(review)
    let ws = harness::read_workflow_state(&repo.get(task_id).await.unwrap().unwrap());
    assert!(matches!(ws, WorkflowState::PhaseGate { .. }), "Expected PhaseGate, got {:?}", ws);

    // Reject at the gate
    let result = engine
        .provide_verdict(task_id, GateVerdict::Reject, "Code quality insufficient")
        .await
        .expect("provide_verdict");
    assert!(result.is_none(), "Reject should return None");

    // Verify rejected state
    let task = repo.get(task_id).await.unwrap().unwrap();
    let ws = harness::read_workflow_state(&task);
    match ws {
        WorkflowState::Rejected { phase_index, reason, .. } => {
            assert_eq!(phase_index, 3);
            assert!(reason.contains("quality"), "Reason should contain rejection text");
        }
        other => panic!("Expected Rejected, got {:?}", other),
    }
}

// ============================================================================
// Test 4: Verification triggers on implement phase
// ============================================================================

#[tokio::test]
async fn test_verification_triggers_on_implement() {
    let tmp = TempDir::new().expect("tempdir");
    let dir = tmp.path();
    harness::init_project(dir);

    let json = harness::run_json(
        dir,
        &["task", "submit", "Verify test", "-t", "Verified", "--json"],
    );
    let task_id: uuid::Uuid = harness::json_str(&json["task"], "id").parse().unwrap();

    let (service, engine, repo) = harness::open_db(dir).await;
    service.claim_task(task_id, "overmind").await.unwrap();

    // Advance through research, plan, implement (all via run_phase which handles aggregation)
    harness::run_phase(&service, &engine, task_id).await; // research
    harness::run_phase(&service, &engine, task_id).await; // plan
    harness::run_phase(&service, &engine, task_id).await; // implement

    // Should now be in Verifying state (implement has verify: true)
    let task = repo.get(task_id).await.unwrap().unwrap();
    let ws = harness::read_workflow_state(&task);
    match ws {
        WorkflowState::Verifying { phase_index, phase_name, retry_count, .. } => {
            assert_eq!(phase_index, 2, "implement is phase index 2");
            assert_eq!(phase_name, "implement");
            assert_eq!(retry_count, 0, "First attempt, no retries yet");
        }
        other => panic!("Expected Verifying(implement), got {:?}", other),
    }

    // Also check that the parent task status transitioned to Validating
    assert_eq!(
        task.status,
        TaskStatus::Validating,
        "Parent should be in Validating status during verification"
    );
}

// ============================================================================
// Test 5: Fan-out and aggregation
// ============================================================================

#[tokio::test]
async fn test_fan_out_and_aggregation() {
    let tmp = TempDir::new().expect("tempdir");
    let dir = tmp.path();
    harness::init_project(dir);

    let json = harness::run_json(
        dir,
        &["task", "submit", "Fan-out task", "-t", "Parallel Work", "--json"],
    );
    let task_id: uuid::Uuid = harness::json_str(&json["task"], "id").parse().unwrap();

    let (service, engine, repo) = harness::open_db(dir).await;
    service.claim_task(task_id, "overmind").await.unwrap();

    // Advance to PhaseReady first, then fan out research phase into 3 slices
    engine.advance(task_id).await.expect("advance to PhaseReady");
    let slices = vec![
        FanOutSlice { description: "Research area A".into(), context: Default::default() },
        FanOutSlice { description: "Research area B".into(), context: Default::default() },
        FanOutSlice { description: "Research area C".into(), context: Default::default() },
    ];
    let fan_result = engine.fan_out(task_id, slices).await.expect("fan_out");
    assert_eq!(fan_result.subtask_ids.len(), 3, "Should have 3 fan-out subtasks");
    assert_eq!(fan_result.phase_name, "research");
    assert_eq!(fan_result.phase_index, 0);

    // Verify FanningOut state
    {
        let task = repo.get(task_id).await.unwrap().unwrap();
        let ws = harness::read_workflow_state(&task);
        match &ws {
            WorkflowState::FanningOut { slice_count, subtask_ids, .. } => {
                assert_eq!(*slice_count, 3);
                assert_eq!(subtask_ids.len(), 3);
            }
            other => panic!("Expected FanningOut, got {:?}", other),
        }
    }

    // Complete all fan-out subtasks
    for &sub_id in &fan_result.subtask_ids {
        // Transition through Ready → Running → Complete via repo (subtasks may be in Ready)
        let mut sub = repo.get(sub_id).await.unwrap().unwrap();
        if sub.status == TaskStatus::Ready {
            let _ = sub.transition_to(TaskStatus::Running);
            repo.update(&sub).await.unwrap();
        }
        let mut sub = repo.get(sub_id).await.unwrap().unwrap();
        let _ = sub.transition_to(TaskStatus::Complete);
        sub.completed_at = Some(chrono::Utc::now());
        repo.update(&sub).await.unwrap();
    }

    // Handle phase complete — should trigger fan-in (aggregation)
    engine
        .handle_phase_complete(task_id, *fan_result.subtask_ids.last().unwrap())
        .await
        .expect("handle_phase_complete after fan-out");

    // Should be in Aggregating state
    let task = repo.get(task_id).await.unwrap().unwrap();
    let ws = harness::read_workflow_state(&task);
    match &ws {
        WorkflowState::Aggregating { subtask_ids, phase_name, .. } => {
            assert_eq!(phase_name, "research");
            // 3 original + 1 aggregation subtask
            assert_eq!(subtask_ids.len(), 4, "Should have 3 fan-out + 1 aggregation subtask");
        }
        other => panic!("Expected Aggregating, got {:?}", other),
    }
}

// ============================================================================
// Test 6: Convergent skip — converged subtask skips verification
// ============================================================================

#[tokio::test]
async fn test_convergent_skip_verification() {
    let tmp = TempDir::new().expect("tempdir");
    let dir = tmp.path();
    harness::init_project(dir);

    let json = harness::run_json(
        dir,
        &["task", "submit", "Converge test", "-t", "Converged", "--json"],
    );
    let task_id: uuid::Uuid = harness::json_str(&json["task"], "id").parse().unwrap();

    let (service, engine, repo) = harness::open_db(dir).await;
    service.claim_task(task_id, "overmind").await.unwrap();

    // Advance through research and plan normally (run_phase handles aggregation)
    harness::run_phase(&service, &engine, task_id).await; // research
    harness::run_phase(&service, &engine, task_id).await; // plan

    // PhaseReady(implement) → advance and fan_out
    let impl_sub = harness::advance_and_get_subtask(&engine, task_id).await;

    // Set convergence_outcome on the subtask before completing
    {
        let mut sub = repo.get(impl_sub).await.unwrap().unwrap();
        sub.context.custom.insert(
            "convergence_outcome".to_string(),
            serde_json::json!("converged"),
        );
        repo.update(&sub).await.unwrap();
    }

    harness::complete_subtask(&service, impl_sub).await;
    engine.handle_phase_complete(task_id, impl_sub).await.unwrap();

    // After fan_out subtask completes → Aggregating. Complete aggregation.
    {
        let task = repo.get(task_id).await.unwrap().unwrap();
        let ws = harness::read_workflow_state(&task);
        if let WorkflowState::Aggregating { subtask_ids, .. } = ws {
            let agg_id = *subtask_ids.last().unwrap();
            // Set convergence_outcome on aggregation subtask too
            {
                let mut agg = repo.get(agg_id).await.unwrap().unwrap();
                agg.context.custom.insert(
                    "convergence_outcome".to_string(),
                    serde_json::json!("converged"),
                );
                repo.update(&agg).await.unwrap();
            }
            harness::complete_subtask(&service, agg_id).await;
            engine.handle_phase_complete(task_id, agg_id).await.unwrap();
        }
    }

    // Should NOT be in Verifying — converged subtasks skip verification
    let task = repo.get(task_id).await.unwrap().unwrap();
    let ws = harness::read_workflow_state(&task);
    assert!(
        !matches!(ws, WorkflowState::Verifying { .. }),
        "Converged subtask should skip verification; got {:?}",
        ws
    );

    // Should be in PhaseReady for review (converged subtasks skip verification, no auto-advance)
    match &ws {
        WorkflowState::PhaseReady { phase_name, .. } => {
            assert_eq!(phase_name, "review", "Should be PhaseReady for review");
        }
        other => panic!("Expected PhaseReady(review), got {:?}", other),
    }
}

// ============================================================================
// Test 7: Task cancel during active workflow
// ============================================================================

#[tokio::test]
async fn test_cancel_task_during_workflow() {
    let tmp = TempDir::new().expect("tempdir");
    let dir = tmp.path();
    harness::init_project(dir);

    // Submit and start workflow
    let json = harness::run_json(
        dir,
        &["task", "submit", "Cancel during workflow", "-t", "Cancel Mid-flow", "--json"],
    );
    let task_id_str = harness::json_str(&json["task"], "id");

    // Cancel via CLI
    let cancel_json = harness::run_json(dir, &["task", "cancel", &task_id_str, "--json"]);
    assert_eq!(cancel_json["success"], true);
    assert_eq!(harness::json_str(&cancel_json["task"], "status"), "canceled");

    // Verify in database
    let (_service, _engine, repo) = harness::open_db(dir).await;
    let task_id: uuid::Uuid = task_id_str.parse().unwrap();
    let task = repo.get(task_id).await.unwrap().unwrap();
    assert_eq!(task.status, TaskStatus::Canceled);

    // Workflow state should still be Pending (never advanced)
    let ws = harness::read_workflow_state(&task);
    assert!(
        matches!(ws, WorkflowState::Pending { .. }),
        "Workflow state should remain Pending after cancel, got {:?}",
        ws
    );
}

// ============================================================================
// Test 8: Gate verdict — rework sends back to previous phase
// ============================================================================

#[tokio::test]
async fn test_gate_verdict_rework() {
    let tmp = TempDir::new().expect("tempdir");
    let dir = tmp.path();
    harness::init_project(dir);

    let json = harness::run_json(
        dir,
        &["task", "submit", "Rework test", "-t", "Rework Me", "--json"],
    );
    let task_id: uuid::Uuid = harness::json_str(&json["task"], "id").parse().unwrap();

    let (service, engine, repo) = harness::open_db(dir).await;
    service.claim_task(task_id, "overmind").await.unwrap();

    // Advance through all phases to reach the review gate
    harness::run_phase(&service, &engine, task_id).await; // research
    harness::run_phase(&service, &engine, task_id).await; // plan
    harness::run_phase(&service, &engine, task_id).await; // implement
    // After implement, should be in Verifying — advance past it
    engine.advance(task_id).await.unwrap();
    // Phase 3: review — advance and fan_out, then complete with aggregation
    harness::run_phase(&service, &engine, task_id).await;

    // Should be at PhaseGate(review)
    let ws = harness::read_workflow_state(&repo.get(task_id).await.unwrap().unwrap());
    assert!(matches!(ws, WorkflowState::PhaseGate { phase_name, .. } if phase_name == "review"));

    // Rework verdict — should go back so review can be re-run
    let result = engine
        .provide_verdict(task_id, GateVerdict::Rework, "Need more test coverage")
        .await
        .expect("provide_verdict rework");
    assert!(result.is_none(), "Rework should return None");

    // After rework, the state should be set to a prior gate so advance() re-creates the review phase
    let task = repo.get(task_id).await.unwrap().unwrap();
    let ws = harness::read_workflow_state(&task);
    match &ws {
        WorkflowState::PhaseGate { phase_index, phase_name, .. } => {
            // Rework sets state to PhaseGate at phase_index - 1
            assert_eq!(*phase_index, 2, "Should be set back to phase 2 (implement)");
            assert_eq!(phase_name, "implement");
        }
        other => panic!("Expected PhaseGate at previous phase, got {:?}", other),
    }

    // Can advance again to re-run review
    let re_review = engine.advance(task_id).await.expect("re-advance after rework");
    match re_review {
        AdvanceResult::PhaseReady { phase_index, phase_name } => {
            assert_eq!(phase_index, 3);
            assert_eq!(phase_name, "review");
        }
        AdvanceResult::Completed => panic!("Expected PhaseReady, got Completed"),
    }
}

// ============================================================================
// Test 9: Workflow CLI commands — list and show
// ============================================================================

#[test]
fn test_workflow_list_and_show_cli() {
    let tmp = TempDir::new().expect("tempdir");
    let dir = tmp.path();
    harness::init_project(dir);

    // workflow list
    let list_json = harness::run_json(dir, &["workflow", "list", "--json"]);
    let workflows = list_json["workflows"]
        .as_array()
        .expect("workflows should be array");
    assert!(
        workflows.iter().any(|w| w["name"] == "code"),
        "Should list the 'code' workflow"
    );

    // workflow show code
    let show_json = harness::run_json(dir, &["workflow", "show", "code", "--json"]);
    assert_eq!(show_json["name"], "code");
    let phases = show_json["phases"]
        .as_array()
        .expect("phases should be array");
    assert_eq!(phases.len(), 4, "code workflow has 4 phases");
    assert_eq!(phases[0]["name"], "research");
    assert_eq!(phases[1]["name"], "plan");
    assert_eq!(phases[2]["name"], "implement");
    assert_eq!(phases[3]["name"], "review");

    // workflow validate
    let validate_json = harness::run_json(dir, &["workflow", "validate", "--json"]);
    assert_eq!(validate_json["all_valid"], true);
}
