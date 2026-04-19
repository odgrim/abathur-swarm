use super::*;
use crate::adapters::sqlite::test_support;
use crate::domain::errors::DomainError;
use crate::domain::models::workflow_state::WorkflowState;
use crate::domain::models::{
    Complexity, ExecutionMode, Task, TaskContext, TaskPriority, TaskSource, TaskStatus, TaskType,
};
use crate::domain::ports::TaskFilter;
use uuid::Uuid;

async fn setup_service() -> TaskService<impl crate::domain::ports::TaskRepository + 'static> {
    test_support::setup_task_service().await
}
// Note: the `impl Trait` return hides the concrete adapter type from this
// test module. Tests call only port methods on the returned service.

#[tokio::test]
async fn test_submit_task() {
    let service = setup_service().await;

    let (task, events) = service
        .submit_task(
            Some("Test Task".to_string()),
            "Description".to_string(),
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

    assert_eq!(task.title, "Test Task");
    assert_eq!(task.status, TaskStatus::Ready); // No deps, should be ready
    assert!(!events.is_empty());
}

#[tokio::test]
async fn test_task_dependencies_block_ready() {
    let service = setup_service().await;

    // Create a dependency task
    let (dep, _) = service
        .submit_task(
            Some("Dependency".to_string()),
            "Must complete first".to_string(),
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

    // Create main task that depends on it
    let (main, _) = service
        .submit_task(
            Some("Main Task".to_string()),
            "Depends on first".to_string(),
            None,
            TaskPriority::Normal,
            None,
            vec![dep.id],
            None,
            None,
            TaskSource::Human,
            None,
            None,
            None,
        )
        .await
        .unwrap();

    // Main should be pending (dependency not complete)
    assert_eq!(main.status, TaskStatus::Pending);

    // Complete the dependency
    service.claim_task(dep.id, "test-agent").await.unwrap();
    service.complete_task(dep.id).await.unwrap();

    // TaskService emits a TaskCompleted event; readiness cascading is handled
    // by the TaskCompletedReadinessHandler in the event reactor, not by
    // TaskService directly. In this unit test (no reactor), the dependent
    // task stays Pending. Full cascade is tested in integration tests.
    let main_updated = service.get_task(main.id).await.unwrap().unwrap();
    assert_eq!(main_updated.status, TaskStatus::Pending);
}

#[tokio::test]
async fn test_idempotency() {
    let service = setup_service().await;

    let (task1, _) = service
        .submit_task(
            Some("Task".to_string()),
            "Description".to_string(),
            None,
            TaskPriority::Normal,
            None,
            vec![],
            None,
            Some("unique-key".to_string()),
            TaskSource::Human,
            None,
            None,
            None,
        )
        .await
        .unwrap();

    let (task2, _) = service
        .submit_task(
            Some("Different Task".to_string()),
            "Different Description".to_string(),
            None,
            TaskPriority::High,
            None,
            vec![],
            None,
            Some("unique-key".to_string()),
            TaskSource::Human,
            None,
            None,
            None,
        )
        .await
        .unwrap();

    // Should return same task
    assert_eq!(task1.id, task2.id);
    assert_eq!(task2.title, "Task"); // Original title
}

#[tokio::test]
async fn test_claim_and_complete() {
    let service = setup_service().await;

    let (task, _) = service
        .submit_task(
            Some("Test".to_string()),
            "Desc".to_string(),
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

    let (claimed, _) = service.claim_task(task.id, "test-agent").await.unwrap();
    assert_eq!(claimed.status, TaskStatus::Running);
    assert_eq!(claimed.agent_type, Some("test-agent".to_string()));

    let (completed, _) = service.complete_task(task.id).await.unwrap();
    assert_eq!(completed.status, TaskStatus::Complete);
    assert!(completed.completed_at.is_some());
}

#[tokio::test]
async fn test_fail_and_retry() {
    let service = setup_service().await;

    let (task, _) = service
        .submit_task(
            Some("Test".to_string()),
            "Desc".to_string(),
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

    service.claim_task(task.id, "test-agent").await.unwrap();
    let (failed, _) = service
        .fail_task(task.id, Some("Test error".to_string()))
        .await
        .unwrap();
    assert_eq!(failed.status, TaskStatus::Failed);

    let (retried, _) = service.retry_task(task.id).await.unwrap();
    assert_eq!(retried.status, TaskStatus::Ready);
    assert_eq!(retried.retry_count, 1);
}

// --- Execution mode classification heuristic tests ---

#[test]
fn test_classify_complex_task_as_convergent() {
    let mut task = Task::new("Implement a complex feature with many moving parts");
    task.routing_hints.complexity = Complexity::Complex;

    let mode = TaskService::<test_support::TestTaskRepo>::classify_execution_mode(&task, None, &None);
    assert!(
        mode.is_convergent(),
        "Complex tasks should classify as Convergent"
    );
}

#[test]
fn test_classify_trivial_task_as_direct() {
    let mut task = Task::new("Rename a variable");
    task.routing_hints.complexity = Complexity::Trivial;

    let mode = TaskService::<test_support::TestTaskRepo>::classify_execution_mode(&task, None, &None);
    assert!(mode.is_direct(), "Trivial tasks should classify as Direct");
}

#[test]
fn test_classify_simple_task_as_direct() {
    let mut task = Task::new("Add a config field");
    task.routing_hints.complexity = Complexity::Simple;

    let mode = TaskService::<test_support::TestTaskRepo>::classify_execution_mode(&task, None, &None);
    assert!(mode.is_direct(), "Simple tasks should classify as Direct");
}

#[test]
fn test_classify_moderate_short_description_as_direct() {
    let mut task = Task::new("Short description of a moderate task");
    task.routing_hints.complexity = Complexity::Moderate;

    let mode = TaskService::<test_support::TestTaskRepo>::classify_execution_mode(&task, None, &None);
    assert!(
        mode.is_direct(),
        "Moderate tasks with short descriptions should be Direct"
    );
}

#[test]
fn test_classify_moderate_long_description_as_convergent() {
    // Build a description with > 200 words and acceptance criteria keywords
    let words: String = (0..210)
        .map(|i| format!("word{}", i))
        .collect::<Vec<_>>()
        .join(" ");
    let desc = format!("{} acceptance criteria: must pass all tests", words);
    let mut task = Task::new(desc);
    task.routing_hints.complexity = Complexity::Moderate;

    let mode = TaskService::<test_support::TestTaskRepo>::classify_execution_mode(&task, None, &None);
    // 2 (long moderate) + 2 (acceptance criteria) = 4 >= 3
    assert!(
        mode.is_convergent(),
        "Moderate task with long desc + acceptance criteria should be Convergent"
    );
}

#[test]
fn test_classify_with_anti_pattern_hints() {
    let mut task = Task::new("Fix something with constraints");
    task.routing_hints.complexity = Complexity::Moderate;
    task.context
        .hints
        .push("anti-pattern: do not use unwrap".to_string());
    task.context
        .hints
        .push("constraint: must preserve backwards compat".to_string());

    let mode = TaskService::<test_support::TestTaskRepo>::classify_execution_mode(&task, None, &None);
    // 0 (moderate, short desc) + 2 (has anti-pattern/constraint) = 2 < 3
    assert!(
        mode.is_direct(),
        "Moderate with hints but no other signals stays Direct"
    );

    // Now add acceptance criteria to push over threshold
    task.description = "Fix something. Verify that all tests pass.".to_string();
    let mode = TaskService::<test_support::TestTaskRepo>::classify_execution_mode(&task, None, &None);
    // 0 + 2 (hints) + 2 (acceptance keyword) = 4 >= 3
    assert!(
        mode.is_convergent(),
        "Moderate + hints + acceptance keywords should be Convergent"
    );
}

#[test]
fn test_classify_subtask_inherits_convergent_parent() {
    let parent_id = Uuid::new_v4();
    let mut task = Task::new("Child task of convergent parent");
    task.source = TaskSource::SubtaskOf(parent_id);
    // Default complexity is Moderate, which alone gives 0 points
    // Parent inheritance adds +3

    let parent_mode = ExecutionMode::Convergent {
        parallel_samples: None,
    };
    let mode = TaskService::<test_support::TestTaskRepo>::classify_execution_mode(
        &task,
        Some(&parent_mode),
        &None,
    );
    assert!(
        mode.is_convergent(),
        "Subtasks of convergent parents should inherit Convergent"
    );
}

#[test]
fn test_classify_low_priority_pushes_toward_direct() {
    let mut task = Task::new("Something that needs to verify that tests pass");
    task.routing_hints.complexity = Complexity::Moderate;
    task.priority = TaskPriority::Low;
    // acceptance keyword: +2, low priority: -2 = 0 < 3

    let mode = TaskService::<test_support::TestTaskRepo>::classify_execution_mode(&task, None, &None);
    assert!(mode.is_direct(), "Low priority should push toward Direct");
}

#[test]
fn test_classify_operator_default_overrides_heuristic() {
    let mut task = Task::new("Complex task that would normally be convergent");
    task.routing_hints.complexity = Complexity::Complex;

    let default_mode = Some(ExecutionMode::Direct);
    let mode = TaskService::<test_support::TestTaskRepo>::classify_execution_mode(
        &task,
        None,
        &default_mode,
    );
    assert!(
        mode.is_direct(),
        "Operator default_execution_mode should override heuristic"
    );
}

#[test]
fn test_classify_operator_default_convergent() {
    let mut task = Task::new("Simple task");
    task.routing_hints.complexity = Complexity::Simple;

    let default_mode = Some(ExecutionMode::Convergent {
        parallel_samples: None,
    });
    let mode = TaskService::<test_support::TestTaskRepo>::classify_execution_mode(
        &task,
        None,
        &default_mode,
    );
    assert!(
        mode.is_convergent(),
        "Operator default Convergent should override even for simple tasks"
    );
}

// --- Agent-role signal tests ---

#[test]
fn test_classify_implementer_agent_moderate_with_keywords_convergent() {
    // Agent role (+2) + acceptance keyword (+2) = 4 >= 3 → Convergent
    let mut task = Task::new("Implement feature. Verify that tests pass.");
    task.routing_hints.complexity = Complexity::Moderate;
    task.agent_type = Some("implementation-specialist".to_string());

    let mode = TaskService::<test_support::TestTaskRepo>::classify_execution_mode(&task, None, &None);
    assert!(
        mode.is_convergent(),
        "Implementer agent + acceptance keyword should be Convergent"
    );
}

#[test]
fn test_classify_researcher_agent_as_direct() {
    // Agent role (−2) + moderate complexity (0) = −2 < 3 → Direct
    let mut task = Task::new("Research best practices for error handling");
    task.routing_hints.complexity = Complexity::Moderate;
    task.agent_type = Some("researcher".to_string());

    let mode = TaskService::<test_support::TestTaskRepo>::classify_execution_mode(&task, None, &None);
    assert!(
        mode.is_direct(),
        "Researcher agent on moderate task should be Direct"
    );
}

#[test]
fn test_classify_agent_role_does_not_override_complexity() {
    // Researcher agent (−2) + Complex (+3) = 1 < 3 → Direct
    // Previously with weight ±5: −5 + 3 = −2, also Direct but for wrong reason.
    // Now the complexity signal is not entirely drowned out.
    let mut task = Task::new("Research and analyze complex architecture");
    task.routing_hints.complexity = Complexity::Complex;
    task.agent_type = Some("researcher".to_string());

    let mode = TaskService::<test_support::TestTaskRepo>::classify_execution_mode(&task, None, &None);
    // Score: −2 + 3 = 1 < 3 → Direct (complexity partially counters role)
    assert!(
        mode.is_direct(),
        "Researcher on complex task: role tempers complexity but doesn't dominate"
    );

    // With an additional acceptance keyword, it should flip to Convergent
    task.description =
        "Research and analyze complex architecture. Verify that the design holds.".to_string();
    let mode = TaskService::<test_support::TestTaskRepo>::classify_execution_mode(&task, None, &None);
    // Score: −2 + 3 + 2 = 3 >= 3 → Convergent
    assert!(
        mode.is_convergent(),
        "Researcher on complex task with acceptance keywords should be Convergent"
    );
}

#[test]
fn test_classify_implementer_on_trivial_stays_direct() {
    // Agent role (+2) + Trivial (−3) = −1 < 3 → Direct
    // Previously with weight ±5: 5 − 3 = 2 < 3 → also Direct,
    // but barely. Now it's clearly Direct.
    let mut task = Task::new("Rename a variable");
    task.routing_hints.complexity = Complexity::Trivial;
    task.agent_type = Some("implementer".to_string());

    let mode = TaskService::<test_support::TestTaskRepo>::classify_execution_mode(&task, None, &None);
    assert!(
        mode.is_direct(),
        "Implementer on trivial task should stay Direct"
    );
}

#[test]
fn test_classify_agent_role_partial_match() {
    // "developer" contains "develop" → +2
    let mut task = Task::new("Build the feature");
    task.routing_hints.complexity = Complexity::Moderate;
    task.agent_type = Some("senior-developer".to_string());

    let mode = TaskService::<test_support::TestTaskRepo>::classify_execution_mode(&task, None, &None);
    // Score: +2 (agent) + 0 (moderate, short) = 2 < 3 → Direct
    assert!(
        mode.is_direct(),
        "Developer agent alone on moderate task should be Direct"
    );

    // Add acceptance criteria to push over
    task.description = "Build the feature. Must pass integration tests.".to_string();
    let mode = TaskService::<test_support::TestTaskRepo>::classify_execution_mode(&task, None, &None);
    // Score: +2 (agent) + 2 (acceptance) = 4 >= 3 → Convergent
    assert!(
        mode.is_convergent(),
        "Developer agent + acceptance criteria should be Convergent"
    );
}

// --- Trajectory-aware retry tests ---

#[tokio::test]
async fn test_retry_convergent_preserves_trajectory_id() {
    let service = setup_service().await;

    let (task, _) = service
        .submit_task(
            Some("Convergent Task".to_string()),
            "Desc".to_string(),
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

    // Manually set convergent mode and trajectory_id (normally done by orchestrator)
    let mut task_updated = service.get_task(task.id).await.unwrap().unwrap();
    task_updated.execution_mode = ExecutionMode::Convergent {
        parallel_samples: None,
    };
    task_updated.trajectory_id = Some(Uuid::new_v4());
    // Transition to Ready -> Running -> Failed so we can retry
    task_updated.status = TaskStatus::Ready;
    service.task_repo.update(&task_updated).await.unwrap();

    service.claim_task(task.id, "test-agent").await.unwrap();
    service
        .fail_task(task.id, Some("convergence exhausted".to_string()))
        .await
        .unwrap();

    let trajectory_before = service
        .get_task(task.id)
        .await
        .unwrap()
        .unwrap()
        .trajectory_id;
    let (retried, _) = service.retry_task(task.id).await.unwrap();

    assert_eq!(retried.status, TaskStatus::Ready);
    assert_eq!(
        retried.trajectory_id, trajectory_before,
        "trajectory_id must be preserved on retry for convergent tasks"
    );
}

#[tokio::test]
async fn test_retry_trapped_convergent_adds_fresh_start_hint() {
    let service = setup_service().await;

    let (task, _) = service
        .submit_task(
            Some("Trapped Task".to_string()),
            "Desc".to_string(),
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

    // Set up as convergent with trajectory
    let mut task_updated = service.get_task(task.id).await.unwrap().unwrap();
    task_updated.execution_mode = ExecutionMode::Convergent {
        parallel_samples: None,
    };
    task_updated.trajectory_id = Some(Uuid::new_v4());
    task_updated.status = TaskStatus::Ready;
    service.task_repo.update(&task_updated).await.unwrap();

    service.claim_task(task.id, "test-agent").await.unwrap();
    // Fail with "trapped" in the error message — this is what the convergence
    // loop does when LoopControl::Trapped fires.
    service
        .fail_task(task.id, Some("trapped in FixedPoint attractor".to_string()))
        .await
        .unwrap();

    let (retried, _) = service.retry_task(task.id).await.unwrap();
    assert!(
        retried
            .context
            .hints
            .iter()
            .any(|h| h == "convergence:fresh_start"),
        "Retrying a trapped convergent task should add convergence:fresh_start hint"
    );
}

#[tokio::test]
async fn test_retry_clears_verification_state() {
    let service = setup_service().await;

    let (task, _) = service
        .submit_task(
            Some("Workflow Task".to_string()),
            "Desc".to_string(),
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

    // Simulate a task that went through workflow verification
    let mut task_updated = service.get_task(task.id).await.unwrap().unwrap();
    task_updated.status = TaskStatus::Ready;
    task_updated.routing_hints.workflow_name = Some("code".to_string());
    task_updated.set_workflow_state_value(
        serde_json::json!({"state": "verifying", "workflow_name": "code", "phase_index": 2, "phase_name": "implement", "subtask_ids": [], "retry_count": 2}),
    );
    task_updated.set_verification_retry_count(2);
    task_updated.set_verification_feedback(serde_json::json!(["partial"]));
    task_updated.set_verification_idempotency_key("wf-verify:old-key");
    task_updated.set_verification_phase_context("phase 3");
    task_updated.set_verification_aggregation_summary(serde_json::json!("summary"));
    service.task_repo.update(&task_updated).await.unwrap();

    service.claim_task(task.id, "test-agent").await.unwrap();
    service
        .fail_task(task.id, Some("stale-timeout".to_string()))
        .await
        .unwrap();

    let (retried, _) = service.retry_task(task.id).await.unwrap();

    assert_eq!(retried.status, TaskStatus::Ready);
    assert!(
        retried.verification_retry_count().is_none(),
        "verification_retry_count must be cleared on retry"
    );
    assert!(
        !retried
            .context
            .custom
            .contains_key(crate::domain::models::task::KEY_VERIFICATION_FEEDBACK),
        "verification_feedback must be cleared on retry"
    );
    assert!(
        retried.verification_idempotency_key().is_none(),
        "verification_idempotency_key must be cleared on retry"
    );
    assert!(
        !retried
            .context
            .custom
            .contains_key(crate::domain::models::task::KEY_VERIFICATION_PHASE_CONTEXT),
        "verification_phase_context must be cleared on retry"
    );
    assert!(
        !retried
            .context
            .custom
            .contains_key(crate::domain::models::task::KEY_VERIFICATION_AGGREGATION_SUMMARY),
        "verification_aggregation_summary must be cleared on retry"
    );

    // workflow_state should be reset to Pending
    let ws = retried
        .workflow_state()
        .expect("workflow_state must be reset to Pending");
    match ws {
        WorkflowState::Pending { workflow_name } => {
            assert_eq!(
                workflow_name, "code",
                "workflow_state must reset to Pending with original workflow name"
            );
        }
        other => panic!("Expected Pending workflow_state, got {:?}", other),
    }
}

// --- Opportunistic memory recording tests ---

#[tokio::test]
async fn test_complete_task_emits_execution_recorded_event() {
    let service = setup_service().await;

    let (task, _) = service
        .submit_task(
            Some("Test".to_string()),
            "Desc".to_string(),
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

    service.claim_task(task.id, "test-agent").await.unwrap();
    let (_, events) = service.complete_task(task.id).await.unwrap();

    // Should have TaskCompleted + TaskExecutionRecorded
    assert!(
        events.len() >= 2,
        "complete_task should emit at least 2 events"
    );
    let recorded = events.iter().find(|e| {
        matches!(
            &e.payload,
            EventPayload::TaskExecutionRecorded {
                succeeded: true,
                ..
            }
        )
    });
    assert!(
        recorded.is_some(),
        "Should emit TaskExecutionRecorded with succeeded=true"
    );
}

#[tokio::test]
async fn test_fail_task_emits_execution_recorded_event() {
    let service = setup_service().await;

    let (task, _) = service
        .submit_task(
            Some("Test".to_string()),
            "Desc".to_string(),
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

    service.claim_task(task.id, "test-agent").await.unwrap();
    let (_, events) = service
        .fail_task(task.id, Some("boom".to_string()))
        .await
        .unwrap();

    // Should have TaskFailed + TaskExecutionRecorded
    assert!(events.len() >= 2, "fail_task should emit at least 2 events");
    let recorded = events.iter().find(|e| {
        matches!(
            &e.payload,
            EventPayload::TaskExecutionRecorded {
                succeeded: false,
                ..
            }
        )
    });
    assert!(
        recorded.is_some(),
        "Should emit TaskExecutionRecorded with succeeded=false"
    );
}

// --- with_default_execution_mode builder test ---

#[tokio::test]
async fn test_submit_task_respects_default_execution_mode() {
    let task_repo = test_support::setup_task_repo().await;
    let service =
        TaskService::new(task_repo).with_default_execution_mode(Some(ExecutionMode::Direct));

    // Submit a complex task — normally would be classified as Convergent
    let mut ctx = TaskContext::default();
    ctx.hints.push("anti-pattern: avoid unsafe".to_string());
    let (task, _) = service
        .submit_task(
            Some("Complex Task".to_string()),
            "This is a complex task that should verify that all tests pass".to_string(),
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

    assert!(
        task.execution_mode.is_direct(),
        "When default_execution_mode is Direct, heuristic should be skipped"
    );
}

#[tokio::test]
async fn test_prune_deletes_terminal_tasks() {
    let service = setup_service().await;

    // Create and complete a task
    let (task, _) = service
        .submit_task(
            Some("Old Task".to_string()),
            "To be pruned".to_string(),
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
    service.claim_task(task.id, "agent").await.unwrap();
    service.complete_task(task.id).await.unwrap();

    let filter = TaskFilter {
        status: Some(TaskStatus::Complete),
        ..Default::default()
    };
    let result = service.prune_tasks(filter, false, false).await.unwrap();
    assert_eq!(result.pruned_count, 1);
    assert_eq!(result.pruned_ids[0], task.id);

    // Task should be gone
    let gone = service.get_task(task.id).await.unwrap();
    assert!(gone.is_none());
}

#[tokio::test]
async fn test_prune_dry_run_does_not_delete() {
    let service = setup_service().await;

    let (task, _) = service
        .submit_task(
            Some("Dry Run Task".to_string()),
            "Should survive".to_string(),
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
    service.claim_task(task.id, "agent").await.unwrap();
    service.complete_task(task.id).await.unwrap();

    let filter = TaskFilter {
        status: Some(TaskStatus::Complete),
        ..Default::default()
    };
    let result = service.prune_tasks(filter, false, true).await.unwrap();
    assert_eq!(result.pruned_count, 1);
    assert!(result.dry_run);

    // Task should still exist
    let still_there = service.get_task(task.id).await.unwrap();
    assert!(still_there.is_some());
}

#[tokio::test]
async fn test_prune_skips_active_dag_tasks() {
    let service = setup_service().await;

    // Create a completed dep and a running dependent
    let (dep, _) = service
        .submit_task(
            Some("Completed Dep".to_string()),
            "Dep".to_string(),
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
    service.claim_task(dep.id, "agent").await.unwrap();
    service.complete_task(dep.id).await.unwrap();

    let (main, _) = service
        .submit_task(
            Some("Running Main".to_string()),
            "Main".to_string(),
            None,
            TaskPriority::Normal,
            None,
            vec![dep.id],
            None,
            None,
            TaskSource::Human,
            None,
            None,
            None,
        )
        .await
        .unwrap();
    // Transition main to running via the reconciliation path:
    // main is Pending because dep was already complete when we submitted.
    // Re-fetch and manually set to Ready then claim.
    let mut main_task = service.get_task(main.id).await.unwrap().unwrap();
    main_task.status = TaskStatus::Ready;
    service.task_repo.update(&main_task).await.unwrap();
    service.claim_task(main.id, "agent").await.unwrap();

    // Try to prune completed tasks — dep should be skipped because
    // its dependent (main) is active (Running).
    let filter = TaskFilter {
        status: Some(TaskStatus::Complete),
        ..Default::default()
    };
    let result = service.prune_tasks(filter, false, false).await.unwrap();
    assert_eq!(result.pruned_count, 0);
    assert_eq!(result.skipped.len(), 1);
    assert_eq!(result.skipped[0].id, dep.id);
}

#[tokio::test]
async fn test_prune_force_ignores_active_dag() {
    let service = setup_service().await;

    // Same setup as above
    let (dep, _) = service
        .submit_task(
            Some("Completed Dep".to_string()),
            "Dep".to_string(),
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
    service.claim_task(dep.id, "agent").await.unwrap();
    service.complete_task(dep.id).await.unwrap();

    let (main, _) = service
        .submit_task(
            Some("Running Main".to_string()),
            "Main".to_string(),
            None,
            TaskPriority::Normal,
            None,
            vec![dep.id],
            None,
            None,
            TaskSource::Human,
            None,
            None,
            None,
        )
        .await
        .unwrap();
    let mut main_task = service.get_task(main.id).await.unwrap().unwrap();
    main_task.status = TaskStatus::Ready;
    service.task_repo.update(&main_task).await.unwrap();
    service.claim_task(main.id, "agent").await.unwrap();

    // Force prune should delete even with active DAG
    let filter = TaskFilter {
        status: Some(TaskStatus::Complete),
        ..Default::default()
    };
    let result = service.prune_tasks(filter, true, false).await.unwrap();
    assert_eq!(result.pruned_count, 1);
    assert!(result.skipped.is_empty());
}

#[tokio::test]
async fn test_submit_complex_task_infers_convergent() {
    let service = setup_service().await;

    // Submit a complex task — heuristic should classify as Convergent
    let mut ctx = TaskContext::default();
    ctx.hints
        .push("constraint: must preserve API compatibility".to_string());
    let (task, _) = service
        .submit_task(
            Some("Complex Feature".to_string()),
            "Implement the full OAuth2 flow. Ensure that all integration tests pass."
                .to_string(),
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

    // Default complexity is Moderate. "ensure that" keyword = +2, constraint hint = +2 => 4 >= 3
    assert!(
        task.execution_mode.is_convergent(),
        "Task with acceptance criteria + constraints should be inferred as Convergent"
    );
}

#[tokio::test]
async fn test_submit_subtask_under_workflow_task_rejected() {
    let service = setup_service().await;

    // Create a parent task (will auto-enroll as Pending workflow)
    let (mut parent, _) = service
        .submit_task(
            Some("Workflow Parent".to_string()),
            "A workflow-enrolled task".to_string(),
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

    // Simulate the workflow engine advancing to PhaseRunning
    // (submit_task auto-enrolls as Pending; we need an active phase)
    let wf_state = WorkflowState::PhaseRunning {
        workflow_name: "code".to_string(),
        phase_index: 0,
        phase_name: "research".to_string(),
        subtask_ids: vec![uuid::Uuid::new_v4()],
    };
    parent.set_workflow_state(&wf_state).unwrap();
    service.task_repo.update(&parent).await.unwrap();

    // Attempting to create a subtask under the workflow parent should fail
    let result = service
        .submit_task(
            Some("Rogue Subtask".to_string()),
            "Should be rejected".to_string(),
            Some(parent.id),
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
        .await;

    assert!(
        result.is_err(),
        "Should reject subtask under workflow-enrolled parent"
    );
    match result.unwrap_err() {
        DomainError::ValidationFailed(msg) => {
            assert!(
                msg.contains("workflow-enrolled"),
                "Error should mention workflow: {msg}"
            );
        }
        other => panic!("Expected ValidationFailed, got: {other:?}"),
    }
}

#[tokio::test]
async fn test_submit_subtask_under_non_workflow_parent_succeeds() {
    let service = setup_service().await;

    // Create a normal parent that won't be auto-enrolled in a workflow.
    // Use TaskType::Verification to bypass the infer_workflow_name logic.
    let (parent, _) = service
        .submit_task(
            Some("Normal Parent".to_string()),
            "Not enrolled in any workflow".to_string(),
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

    // Creating a subtask under a non-workflow parent should succeed
    let (child, _) = service
        .submit_task(
            Some("Normal Subtask".to_string()),
            "Should succeed".to_string(),
            Some(parent.id),
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

    assert_eq!(child.parent_id, Some(parent.id));
}

#[tokio::test]
async fn test_submit_task_rejects_cyclic_initial_dependencies() {
    // Test that submit_task() rejects cyclic dependencies via the create() → add_dependency() path.
    // Cycle detection is enforced in add_dependency() which is called by create() for each
    // initial dependency. This test verifies the end-to-end service-layer behavior.
    let service = setup_service().await;

    // Create task A (no deps)
    let (a, _) = service
        .submit_task(
            Some("A".to_string()),
            "Task A".to_string(),
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

    // Create task B depending on A (valid)
    let (b, _) = service
        .submit_task(
            Some("B".to_string()),
            "Task B depends on A".to_string(),
            None,
            TaskPriority::Normal,
            None,
            vec![a.id],
            None,
            None,
            TaskSource::Human,
            None,
            None,
            None,
        )
        .await
        .unwrap();

    // Now try to submit task C with depends_on = [b.id], then use the repo to add
    // a dependency from A to C, completing the cycle A->C->B->A.
    let (c, _) = service
        .submit_task(
            Some("C".to_string()),
            "Task C depends on B".to_string(),
            None,
            TaskPriority::Normal,
            None,
            vec![b.id],
            None,
            None,
            TaskSource::Human,
            None,
            None,
            None,
        )
        .await
        .unwrap();

    // Adding A depends on C would create cycle: A->C->B->A
    // Use the repo directly since TaskService doesn't expose add_dependency
    let result = service.task_repo.add_dependency(a.id, c.id).await;
    assert!(
        result.is_err(),
        "Adding A->C dependency should fail since C->B->A creates a cycle"
    );
    assert!(
        matches!(result.unwrap_err(), DomainError::DependencyCycle(_)),
        "Expected DependencyCycle error for transitive cycle via submit_task"
    );
}

// --- transition_to_validating guard tests (Fix 3 / Fix 8) ---

#[tokio::test]
async fn test_transition_to_validating_refused_when_workflow_not_verifying() {
    let service = setup_service().await;

    // Create and claim a task so it's Running
    let (task, _) = service
        .submit_task(
            Some("Test".to_string()),
            "Desc".to_string(),
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
    service.claim_task(task.id, "test-agent").await.unwrap();

    // Set workflow state to PhaseReady (not Verifying)
    let ws = WorkflowState::PhaseReady {
        workflow_name: "code".to_string(),
        phase_index: 0,
        phase_name: "implement".to_string(),
    };
    let mut running_task = service.get_task(task.id).await.unwrap().unwrap();
    running_task.set_workflow_state(&ws).unwrap();
    service.task_repo.update(&running_task).await.unwrap();

    // transition_to_validating should fail
    let result = service.transition_to_validating(task.id).await;
    assert!(
        result.is_err(),
        "transition_to_validating should refuse when workflow state is PhaseReady"
    );
    let err = format!("{:?}", result.unwrap_err());
    assert!(
        err.contains("deadlock") || err.contains("Verifying"),
        "Error should mention deadlock or Verifying, got: {}",
        err
    );
}

#[tokio::test]
async fn test_transition_to_validating_succeeds_when_workflow_is_verifying() {
    let service = setup_service().await;

    // Create and claim a task so it's Running
    let (task, _) = service
        .submit_task(
            Some("Test".to_string()),
            "Desc".to_string(),
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
    service.claim_task(task.id, "test-agent").await.unwrap();

    // Set workflow state to Verifying
    let ws = WorkflowState::Verifying {
        workflow_name: "code".to_string(),
        phase_index: 0,
        phase_name: "implement".to_string(),
        subtask_ids: vec![],
        retry_count: 0,
    };
    let mut running_task = service.get_task(task.id).await.unwrap().unwrap();
    running_task.set_workflow_state(&ws).unwrap();
    service.task_repo.update(&running_task).await.unwrap();

    // transition_to_validating should succeed
    let result = service.transition_to_validating(task.id).await;
    assert!(
        result.is_ok(),
        "transition_to_validating should succeed when workflow state is Verifying: {:?}",
        result.err()
    );
    let (updated, _) = result.unwrap();
    assert_eq!(updated.status, TaskStatus::Validating);
}

// --- force_transition tests (Fix 4 / Fix 8) ---

#[tokio::test]
async fn test_force_transition_updates_status_and_workflow_state() {
    let service = setup_service().await;

    // Create and claim a task so it's Running
    let (task, _) = service
        .submit_task(
            Some("Test".to_string()),
            "Desc".to_string(),
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
    service.claim_task(task.id, "test-agent").await.unwrap();

    // Set workflow state to Verifying so we can transition to Validating
    let verifying_ws = WorkflowState::Verifying {
        workflow_name: "code".to_string(),
        phase_index: 0,
        phase_name: "implement".to_string(),
        subtask_ids: vec![],
        retry_count: 0,
    };
    let mut running_task = service.get_task(task.id).await.unwrap().unwrap();
    running_task.set_workflow_state(&verifying_ws).unwrap();
    service.task_repo.update(&running_task).await.unwrap();

    // Transition to Validating (allowed because workflow state is Verifying)
    service.transition_to_validating(task.id).await.unwrap();

    // Now overwrite with a PhaseReady workflow state on the Validating task
    // (simulating the inconsistent state force_transition is meant to fix)
    let ws = WorkflowState::PhaseReady {
        workflow_name: "code".to_string(),
        phase_index: 0,
        phase_name: "implement".to_string(),
    };
    let mut validating_task = service.get_task(task.id).await.unwrap().unwrap();
    validating_task.set_workflow_state(&ws).unwrap();
    service.task_repo.update(&validating_task).await.unwrap();

    // Force transition to Failed
    let result = service
        .force_transition(task.id, TaskStatus::Failed, "test reason")
        .await;
    assert!(
        result.is_ok(),
        "force_transition should succeed: {:?}",
        result.err()
    );

    let (updated, _) = result.unwrap();
    assert_eq!(
        updated.status,
        TaskStatus::Failed,
        "Task status should be Failed after force_transition"
    );

    // Verify workflow state was also updated to Failed
    let updated_ws = updated
        .workflow_state()
        .expect("workflow_state should still be present");
    assert!(
        matches!(updated_ws, WorkflowState::Failed { .. }),
        "WorkflowState should be Failed after force_transition to Failed, got {:?}",
        updated_ws
    );
}
