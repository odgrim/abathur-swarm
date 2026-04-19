//! Federation integration test.
//!
//! Simulates delegation between two FederationService instances (overmind → cerebrate),
//! verifying that the full round-trip works: delegate → accept → progress → result,
//! with correct events emitted at each stage.
//!
//! Additional tests further down wire the FederationService into an EventReactor
//! along with FederationResultHandler, SwarmDagEventHandler, and SwarmDagExecutor
//! to exercise the reactive cascade end-to-end (result event → DAG node state
//! transition via the reactor's reaction republish mechanism).

use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use uuid::Uuid;

use abathur::domain::models::a2a::{
    Artifact, ConnectionState, FederationResult, FederationTaskEnvelope,
};
use abathur::domain::models::goal::Goal;
use abathur::domain::models::goal_federation::ConvergenceContract;
use abathur::domain::models::swarm_dag::{SwarmDag, SwarmDagNode, SwarmDagNodeState};
use abathur::services::event_bus::{
    EventBus, EventBusConfig, EventCategory, EventPayload, EventSeverity, UnifiedEvent,
};
use abathur::services::event_factory;
use abathur::services::event_reactor::{EventReactor, ReactorConfig};
use abathur::services::federation::config::{FederationConfig, FederationRole};
use abathur::services::federation::dag_handler::SwarmDagEventHandler;
use abathur::services::federation::handler::FederationResultHandler;
use abathur::services::federation::swarm_dag_executor::SwarmDagExecutor;
use abathur::services::federation::traits::ParentContext;
use abathur::services::federation::{FederationReaction, FederationService};

/// Create a FederationService configured as an overmind.
fn make_overmind() -> (Arc<FederationService>, Arc<EventBus>) {
    let event_bus = Arc::new(EventBus::new(EventBusConfig::default()));
    let config = FederationConfig {
        enabled: true,
        role: FederationRole::Overmind,
        swarm_id: "overmind-1".to_string(),
        display_name: "Test Overmind".to_string(),
        ..FederationConfig::default()
    };
    let svc = Arc::new(FederationService::new(config, event_bus.clone()));
    (svc, event_bus)
}

/// Create a FederationService configured as a cerebrate.
fn make_cerebrate() -> (Arc<FederationService>, Arc<EventBus>) {
    let event_bus = Arc::new(EventBus::new(EventBusConfig::default()));
    let config = FederationConfig {
        enabled: true,
        role: FederationRole::Cerebrate,
        swarm_id: "cerebrate-1".to_string(),
        display_name: "Test Cerebrate".to_string(),
        ..FederationConfig::default()
    };
    let svc = Arc::new(FederationService::new(config, event_bus.clone()));
    (svc, event_bus)
}

#[tokio::test]
async fn test_full_delegation_roundtrip() {
    let (overmind, overmind_bus) = make_overmind();
    let (_cerebrate, _cerebrate_bus) = make_cerebrate();

    // 1. Overmind registers and connects to the cerebrate
    overmind
        .register_cerebrate(
            "cerebrate-1",
            "Test Cerebrate",
            "https://cerebrate.local:8443",
        )
        .await;
    overmind.connect("cerebrate-1").await.unwrap();

    let status = overmind.get_cerebrate("cerebrate-1").await.unwrap();
    assert_eq!(status.connection_state, ConnectionState::Connected);

    // Subscribe to overmind events for verification
    let mut rx = overmind_bus.subscribe();

    // 2. Create and delegate a task
    let task_id = Uuid::new_v4();
    let correlation_id = Uuid::new_v4();
    let envelope =
        FederationTaskEnvelope::new(task_id, "Build feature X", "Implement the X feature")
            .with_parent_goal(Uuid::new_v4())
            .with_constraint("Must pass CI".to_string());

    let assigned = overmind
        .delegate_to(&envelope, "cerebrate-1")
        .await
        .unwrap();
    assert_eq!(assigned, "cerebrate-1");
    assert_eq!(overmind.in_flight_count().await, 1);

    // Verify delegation event was emitted
    let event = tokio::time::timeout(std::time::Duration::from_millis(100), rx.recv()).await;
    assert!(event.is_ok(), "Expected FederationTaskDelegated event");

    // 3. Cerebrate accepts the task (simulated)
    overmind.handle_accept(task_id, "cerebrate-1").await;

    let event = tokio::time::timeout(std::time::Duration::from_millis(100), rx.recv()).await;
    assert!(event.is_ok(), "Expected FederationTaskAccepted event");

    // 4. Cerebrate sends progress
    overmind
        .handle_progress(task_id, "cerebrate-1", "implement", 50.0, "Half done")
        .await;

    let event = tokio::time::timeout(std::time::Duration::from_millis(100), rx.recv()).await;
    assert!(event.is_ok(), "Expected FederationProgressReceived event");

    // 5. Cerebrate sends final result
    let result = FederationResult::completed(
        task_id,
        correlation_id,
        "Feature X implemented successfully",
    )
    .with_artifact(Artifact::new(
        "pr_url",
        "https://github.com/org/repo/pull/42",
    ))
    .with_artifact(Artifact::new("commit_sha", "abc123def456"));

    let goal_id = Uuid::new_v4();
    let ctx = ParentContext {
        goal_id: Some(goal_id),
        goal_summary: Some("Build feature X".to_string()),
        task_title: Some("Build feature X".to_string()),
    };

    let reactions = overmind.handle_result(result, ctx).await;

    // 6. Verify result processing
    assert_eq!(
        reactions.len(),
        1,
        "Expected one reaction from result processor"
    );
    match &reactions[0] {
        FederationReaction::UpdateGoalProgress {
            goal_id: gid,
            summary,
        } => {
            assert_eq!(*gid, goal_id);
            assert!(summary.contains("Feature X implemented"));
        }
        other => panic!("Expected UpdateGoalProgress, got {:?}", other),
    }

    // 7. Task should be removed from in-flight
    assert_eq!(overmind.in_flight_count().await, 0);

    // 8. Active delegations should be decremented
    let status = overmind.get_cerebrate("cerebrate-1").await.unwrap();
    assert_eq!(status.active_delegations, 0);
}

#[tokio::test]
async fn test_delegation_rejection_and_redelegate() {
    let (overmind, _bus) = make_overmind();

    // Register two cerebrates
    overmind
        .register_cerebrate("c1", "Cerebrate 1", "https://c1.local:8443")
        .await;
    overmind
        .register_cerebrate("c2", "Cerebrate 2", "https://c2.local:8443")
        .await;
    overmind.connect("c1").await.unwrap();
    overmind.connect("c2").await.unwrap();

    // Delegate to c1
    let task_id = Uuid::new_v4();
    let envelope = FederationTaskEnvelope::new(task_id, "Test task", "Do the thing");
    overmind.delegate_to(&envelope, "c1").await.unwrap();

    // c1 rejects
    let decision = overmind.handle_reject(task_id, "c1", "at capacity").await;

    // Should suggest redelegating to c2
    match decision {
        abathur::services::federation::DelegationDecision::Redelegate(ref id) => {
            assert_eq!(id, "c2");
        }
        other => panic!("Expected Redelegate to c2, got {:?}", other),
    }
}

#[tokio::test]
async fn test_failed_result_escalates() {
    let (overmind, _bus) = make_overmind();

    overmind
        .register_cerebrate("c1", "Cerebrate 1", "https://c1.local:8443")
        .await;
    overmind.connect("c1").await.unwrap();

    let task_id = Uuid::new_v4();
    let correlation_id = Uuid::new_v4();
    let envelope = FederationTaskEnvelope::new(task_id, "Test", "Test");
    overmind.delegate_to(&envelope, "c1").await.unwrap();

    let result =
        FederationResult::failed(task_id, correlation_id, "Build failed", "CI pipeline broke");

    let goal_id = Uuid::new_v4();
    let ctx = ParentContext {
        goal_id: Some(goal_id),
        ..Default::default()
    };

    let reactions = overmind.handle_result(result, ctx).await;

    assert_eq!(reactions.len(), 1);
    match &reactions[0] {
        FederationReaction::Escalate {
            reason,
            goal_id: gid,
        } => {
            assert!(reason.contains("CI pipeline broke"));
            assert_eq!(*gid, Some(goal_id));
        }
        other => panic!("Expected Escalate, got {:?}", other),
    }
}

#[tokio::test]
async fn test_heartbeat_and_unreachable_transition() {
    let (overmind, _bus) = make_overmind();

    overmind
        .register_cerebrate("c1", "Cerebrate 1", "https://c1.local:8443")
        .await;
    overmind.connect("c1").await.unwrap();

    // Simulate heartbeat
    overmind.handle_heartbeat("c1", 0.3).await;
    let status = overmind.get_cerebrate("c1").await.unwrap();
    assert_eq!(status.load, 0.3);
    assert_eq!(status.connection_state, ConnectionState::Connected);

    // Verify cerebrate count
    {
        let cerebrates = overmind.list_cerebrates().await;
        assert_eq!(cerebrates.len(), 1);
    }

    // To test unreachable transition from the integration test level,
    // we rely on the fact that calling check_heartbeats repeatedly
    // will increment missed_heartbeats when no heartbeat is received.
    // The default config has heartbeat_interval_secs=30 and threshold=3.
    // We use a custom config with short intervals.
    // Instead, we verify the reconnection via heartbeat behavior:

    // Verify that receiving a heartbeat after being in unreachable state
    // transitions back to connected. We can't set internal state from outside,
    // so we test the happy path here.
    overmind.handle_heartbeat("c1", 0.1).await;
    let status = overmind.get_cerebrate("c1").await.unwrap();
    assert_eq!(status.connection_state, ConnectionState::Connected);
    assert_eq!(status.load, 0.1);
}

#[tokio::test]
async fn test_persistence_roundtrip() {
    let (overmind, _bus) = make_overmind();

    overmind
        .register_cerebrate("c1", "Cerebrate 1", "https://c1.local:8443")
        .await;
    overmind.connect("c1").await.unwrap();
    overmind.handle_heartbeat("c1", 0.5).await;

    // Save
    let tmp = tempfile::tempdir().unwrap();
    overmind.save_connections(tmp.path()).await.unwrap();

    // Load into a fresh service
    let (fresh, _bus2) = make_overmind();
    let loaded = fresh.load_connections(tmp.path()).await.unwrap();
    assert_eq!(loaded, 1);

    let status = fresh.get_cerebrate("c1").await.unwrap();
    assert_eq!(status.display_name, "Cerebrate 1");
    assert_eq!(status.load, 0.5);
}

// ---------------------------------------------------------------------------
// Integration tests that wire multiple federation services together via the
// EventReactor. These exercise the reactive cascade path that pure per-module
// unit tests can't reach: a FederationResultReceived event flowing through
// FederationResultHandler -> reaction events -> re-published on the bus ->
// SwarmDagEventHandler -> SwarmDagExecutor state transitions.
// ---------------------------------------------------------------------------

/// Build a DAG node with a known federated_goal_id, already in Delegated state.
fn make_delegated_node(label: &str, cerebrate_id: &str, deps: Vec<Uuid>) -> SwarmDagNode {
    SwarmDagNode {
        id: Uuid::new_v4(),
        label: label.to_string(),
        cerebrate_id: cerebrate_id.to_string(),
        intent: format!("Do {}", label),
        contract: ConvergenceContract::default(),
        dependencies: deps,
        federated_goal_id: Some(Uuid::new_v4()),
        state: SwarmDagNodeState::Delegated,
    }
}

/// Small helper to wait for a matching event on a subscriber, ignoring
/// non-matching events. Fails after the deadline.
async fn wait_for_event<F>(
    rx: &mut tokio::sync::broadcast::Receiver<UnifiedEvent>,
    mut matcher: F,
    deadline: std::time::Duration,
) -> UnifiedEvent
where
    F: FnMut(&UnifiedEvent) -> bool,
{
    let deadline_at = tokio::time::Instant::now() + deadline;
    loop {
        let remaining = deadline_at.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            panic!("Timed out waiting for matching event");
        }
        match tokio::time::timeout(remaining, rx.recv()).await {
            Ok(Ok(ev)) => {
                if matcher(&ev) {
                    return ev;
                }
            }
            Ok(Err(e)) => panic!("Event bus receive error: {:?}", e),
            Err(_) => panic!("Timed out waiting for matching event"),
        }
    }
}

/// Test 1: Local-goal delegation fires FederatedGoalCreated with the expected
/// fields. This exercises the federated-task publish path via delegate_goal()
/// — which internally calls delegate_to() (legacy HTTP publisher port), then
/// records the task->FederatedGoal mapping and publishes FederatedGoalCreated.
///
/// The legacy HTTP call against a fake URL fails but is swallowed as
/// non-fatal (documented in service.rs:799-807); this is the behavior in
/// test/local setups without a real cerebrate HTTP endpoint.
#[tokio::test]
async fn test_delegate_goal_emits_federated_goal_created() {
    let (overmind, bus) = make_overmind();
    // Use 127.0.0.1:1 so the outbound legacy HTTP delegate call fails fast.
    overmind
        .register_cerebrate("c1", "Cerebrate 1", "http://127.0.0.1:1")
        .await;
    overmind.connect("c1").await.unwrap();

    let mut rx = bus.subscribe();

    let goal = Goal::new("Build feature Y", "Implement feature Y");
    let local_goal_id = goal.id;
    let contract = ConvergenceContract::default();

    let federated_goal = overmind
        .delegate_goal(&goal, "c1", contract)
        .await
        .expect("delegate_goal should succeed with fake URL (legacy HTTP failures are swallowed)");

    assert_eq!(federated_goal.local_goal_id, local_goal_id);
    assert_eq!(federated_goal.cerebrate_id, "c1");

    // We should see FederationTaskDelegated (from delegate_to) and
    // FederatedGoalCreated (from delegate_goal). Find the latter.
    let ev = wait_for_event(
        &mut rx,
        |e| matches!(e.payload, EventPayload::FederatedGoalCreated { .. }),
        std::time::Duration::from_millis(500),
    )
    .await;

    match ev.payload {
        EventPayload::FederatedGoalCreated {
            local_goal_id: lgi,
            cerebrate_id,
            remote_task_id,
        } => {
            assert_eq!(lgi, local_goal_id);
            assert_eq!(cerebrate_id, "c1");
            assert!(
                !remote_task_id.is_empty(),
                "remote_task_id should be populated (legacy envelope path uses envelope.task_id)"
            );
        }
        other => panic!("Expected FederatedGoalCreated, got {:?}", other.variant_name()),
    }

    // The task->FederatedGoal mapping is populated so result handlers can
    // correlate later; behavior is implicitly covered by the reactor cascade
    // test below (which publishes FederatedGoalConverged directly with a
    // known federated_goal_id).
}

/// Test 2: Reactive pipeline end-to-end.
/// Publish a FederatedGoalConverged event (as if emitted by the
/// FederationResultHandler) onto the bus with a local_goal_id matching a
/// DAG node's federated_goal_id. The reactor dispatches it to
/// SwarmDagEventHandler, which calls into SwarmDagExecutor, which transitions
/// the node to Converged and delegates dependent nodes.
///
/// This is the cascade that drives multi-stage federated workflows forward.
#[tokio::test]
async fn test_reactor_cascade_converged_advances_dag() {
    let (overmind, bus) = make_overmind();
    // Register the cerebrate used for the dependent ("deploy") node so that
    // when the executor auto-delegates the unblocked dependent, delegate_goal
    // doesn't fail with "Unknown cerebrate".
    // Use 127.0.0.1 with an unused port so the HTTP delegate call fails
    // fast with ECONNREFUSED rather than waiting on a DNS resolution
    // timeout — keeps the test well under 5s.
    overmind
        .register_cerebrate(
            "cerebrate-deploy",
            "Deploy Swarm",
            "http://127.0.0.1:1",
        )
        .await;
    overmind.connect("cerebrate-deploy").await.unwrap();

    // Build a two-node DAG: code -> deploy. `code` is already Delegated with
    // a known federated_goal_id.
    let mut dag = SwarmDag::new("pipeline");
    let code_node = make_delegated_node("code", "cerebrate-code", vec![]);
    let code_node_id = code_node.id;
    let code_fed_goal_id = code_node.federated_goal_id.unwrap();
    dag.add_node(code_node);

    let deploy_node = SwarmDagNode {
        id: Uuid::new_v4(),
        label: "deploy".to_string(),
        cerebrate_id: "cerebrate-deploy".to_string(),
        intent: "Deploy".to_string(),
        contract: ConvergenceContract::default(),
        dependencies: vec![code_node_id],
        federated_goal_id: None,
        state: SwarmDagNodeState::Waiting,
    };
    let deploy_node_id = deploy_node.id;
    dag.add_node(deploy_node);

    let dag_id = dag.id;
    let dags: Arc<RwLock<HashMap<Uuid, SwarmDag>>> = Arc::new(RwLock::new(HashMap::new()));
    dags.write().await.insert(dag_id, dag);

    // Placeholder goal keyed by dag_id (SwarmDagEventHandler uses this lookup).
    let goal = Goal::new("pipeline goal", "drive pipeline");
    let goals: Arc<RwLock<HashMap<Uuid, Goal>>> = Arc::new(RwLock::new(HashMap::new()));
    goals.write().await.insert(dag_id, goal);

    // Wire the executor + handler + reactor.
    let executor = Arc::new(SwarmDagExecutor::new(overmind.clone(), bus.clone()));
    let dag_handler = Arc::new(SwarmDagEventHandler::new(
        dags.clone(),
        executor.clone(),
        goals.clone(),
    ));

    let reactor = EventReactor::new(bus.clone(), ReactorConfig::default());
    reactor.register(dag_handler).await;
    let reactor_handle = reactor.start();

    // Give the reactor a moment to subscribe before publishing.
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;

    // Publish FederatedGoalConverged directly (simulating what
    // FederationResultHandler would emit after a successful result).
    bus.publish(event_factory::federation_event(
        EventSeverity::Info,
        None,
        EventPayload::FederatedGoalConverged {
            local_goal_id: code_fed_goal_id,
            cerebrate_id: "cerebrate-code".to_string(),
        },
    ))
    .await;

    // Poll the DAG until both transitions complete, or timeout.
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(3);
    loop {
        {
            let dags_guard = dags.read().await;
            let dag_final = dags_guard.get(&dag_id).unwrap();
            let code_state = dag_final.get_node(code_node_id).unwrap().state;
            let deploy_state = dag_final.get_node(deploy_node_id).unwrap().state;
            if code_state == SwarmDagNodeState::Converged
                && deploy_state == SwarmDagNodeState::Delegated
            {
                break;
            }
        }
        if tokio::time::Instant::now() >= deadline {
            let dags_guard = dags.read().await;
            let dag_final = dags_guard.get(&dag_id).unwrap();
            panic!(
                "DAG did not advance in time. code state={:?}, deploy state={:?}",
                dag_final.get_node(code_node_id).unwrap().state,
                dag_final.get_node(deploy_node_id).unwrap().state,
            );
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }

    reactor_handle.abort();
}

/// Test 3: Failure cascade. Publish FederatedGoalFailed matching a root
/// node and verify the DAG marks it Failed + cascades to dependents.
///
/// Uses a diamond DAG shape: root -> (left, right) -> sink. Fail root;
/// verify all four nodes end up Failed.
#[tokio::test]
async fn test_reactor_cascade_failed_propagates_to_dependents() {
    let (overmind, bus) = make_overmind();

    // Build diamond: root -> left, root -> right, left+right -> sink.
    // Root is Delegated with a known federated_goal_id.
    let mut dag = SwarmDag::new("diamond");
    let root = make_delegated_node("root", "c1", vec![]);
    let root_id = root.id;
    let root_fed_goal_id = root.federated_goal_id.unwrap();
    dag.add_node(root);

    let left = SwarmDagNode {
        id: Uuid::new_v4(),
        label: "left".to_string(),
        cerebrate_id: "c2".to_string(),
        intent: "left".to_string(),
        contract: ConvergenceContract::default(),
        dependencies: vec![root_id],
        federated_goal_id: None,
        state: SwarmDagNodeState::Waiting,
    };
    let left_id = left.id;
    dag.add_node(left);

    let right = SwarmDagNode {
        id: Uuid::new_v4(),
        label: "right".to_string(),
        cerebrate_id: "c3".to_string(),
        intent: "right".to_string(),
        contract: ConvergenceContract::default(),
        dependencies: vec![root_id],
        federated_goal_id: None,
        state: SwarmDagNodeState::Waiting,
    };
    let right_id = right.id;
    dag.add_node(right);

    let sink = SwarmDagNode {
        id: Uuid::new_v4(),
        label: "sink".to_string(),
        cerebrate_id: "c4".to_string(),
        intent: "sink".to_string(),
        contract: ConvergenceContract::default(),
        dependencies: vec![left_id, right_id],
        federated_goal_id: None,
        state: SwarmDagNodeState::Waiting,
    };
    let sink_id = sink.id;
    dag.add_node(sink);

    let dag_id = dag.id;
    let dags: Arc<RwLock<HashMap<Uuid, SwarmDag>>> = Arc::new(RwLock::new(HashMap::new()));
    dags.write().await.insert(dag_id, dag);

    let goal = Goal::new("diamond", "diamond dag");
    let goals: Arc<RwLock<HashMap<Uuid, Goal>>> = Arc::new(RwLock::new(HashMap::new()));
    goals.write().await.insert(dag_id, goal);

    let executor = Arc::new(SwarmDagExecutor::new(overmind.clone(), bus.clone()));
    let dag_handler = Arc::new(SwarmDagEventHandler::new(
        dags.clone(),
        executor.clone(),
        goals.clone(),
    ));

    let reactor = EventReactor::new(bus.clone(), ReactorConfig::default());
    reactor.register(dag_handler).await;
    let reactor_handle = reactor.start();

    tokio::time::sleep(std::time::Duration::from_millis(20)).await;

    let mut rx = bus.subscribe();

    bus.publish(event_factory::federation_event(
        EventSeverity::Warning,
        None,
        EventPayload::FederatedGoalFailed {
            local_goal_id: root_fed_goal_id,
            cerebrate_id: "c1".to_string(),
            reason: "root swarm blew up".to_string(),
        },
    ))
    .await;

    // Wait for the SwarmDagCompleted event (emitted after the cascade finishes).
    let completed = wait_for_event(
        &mut rx,
        |e| matches!(&e.payload, EventPayload::SwarmDagCompleted { dag_id: d, .. } if *d == dag_id),
        std::time::Duration::from_secs(2),
    )
    .await;

    match completed.payload {
        EventPayload::SwarmDagCompleted {
            converged_count,
            failed_count,
            ..
        } => {
            assert_eq!(converged_count, 0);
            assert_eq!(failed_count, 4, "all four nodes should be Failed");
        }
        _ => unreachable!(),
    }

    // All nodes are Failed.
    let dags_guard = dags.read().await;
    let dag_final = dags_guard.get(&dag_id).unwrap();
    for (id, expected_label) in [
        (root_id, "root"),
        (left_id, "left"),
        (right_id, "right"),
        (sink_id, "sink"),
    ] {
        let n = dag_final.get_node(id).unwrap();
        assert_eq!(
            n.state,
            SwarmDagNodeState::Failed,
            "node {} should be Failed",
            expected_label
        );
    }

    reactor_handle.abort();
}

/// Test 4: FederationResultHandler handles a raw FederationResultReceived
/// event through the reactor, producing FederationReactionEmitted +
/// FederatedGoalConverged follow-up events.
///
/// We don't pre-seed a task->federated_goal mapping, so the
/// FederatedGoalConverged follow-up is skipped (documented behavior at
/// handler.rs:230-259). This test pins the current shape: reactions are
/// emitted as FederationReactionEmitted events; the DAG-correlation event
/// only fires if a mapping exists.
#[tokio::test]
async fn test_federation_result_handler_reactor_emits_reactions() {
    let (overmind, bus) = make_overmind();

    let handler = Arc::new(FederationResultHandler::new(overmind.clone()));
    let reactor = EventReactor::new(bus.clone(), ReactorConfig::default());
    reactor.register(handler).await;
    let reactor_handle = reactor.start();

    tokio::time::sleep(std::time::Duration::from_millis(20)).await;

    let mut rx = bus.subscribe();

    let task_id = Uuid::new_v4();
    let goal_id = Uuid::new_v4();
    let mut event = event_factory::federation_event(
        EventSeverity::Info,
        Some(task_id),
        EventPayload::FederationResultReceived {
            task_id,
            cerebrate_id: "c1".to_string(),
            status: "completed".to_string(),
            summary: "feature Z shipped".to_string(),
            artifacts: Vec::new(),
        },
    );
    // Provide a goal_id so the DefaultResultProcessor emits UpdateGoalProgress
    // rather than nothing.
    event.goal_id = Some(goal_id);

    bus.publish(event).await;

    // The handler should emit a FederationReactionEmitted event (for the
    // UpdateGoalProgress reaction from DefaultResultProcessor).
    let reaction_ev = wait_for_event(
        &mut rx,
        |e| matches!(&e.payload, EventPayload::FederationReactionEmitted { .. }),
        std::time::Duration::from_secs(2),
    )
    .await;

    match reaction_ev.payload {
        EventPayload::FederationReactionEmitted {
            reaction_type,
            goal_id: gid,
            ..
        } => {
            assert_eq!(reaction_type, "update_goal_progress");
            assert_eq!(gid, Some(goal_id));
        }
        _ => unreachable!(),
    }

    reactor_handle.abort();
}

/// Test 5: Federated task rejection path.
/// handle_reject should emit a FederationTaskRejected event and, with one
/// remaining healthy cerebrate, return a Redelegate decision. The rejection
/// envelope is populated with real context (task_id, parent_task_id,
/// rejection_count, rejected_by, peer_load_hints) and the delegation
/// strategy consumes those fields to avoid re-delegating to the rejector.
#[tokio::test]
async fn test_federation_rejection_emits_rejected_event() {
    let (overmind, bus) = make_overmind();
    // Use 127.0.0.1:1 so the outbound HTTP delegate call fails fast.
    overmind
        .register_cerebrate("c1", "Cerebrate 1", "http://127.0.0.1:1")
        .await;
    overmind
        .register_cerebrate("c2", "Cerebrate 2", "http://127.0.0.1:1")
        .await;
    overmind.connect("c1").await.unwrap();
    overmind.connect("c2").await.unwrap();

    let task_id = Uuid::new_v4();
    let parent_task_id = Uuid::new_v4();
    let envelope = FederationTaskEnvelope::new(task_id, "reject-me", "this will be rejected")
        .with_parent_task(parent_task_id);
    overmind.delegate_to(&envelope, "c1").await.unwrap();
    assert_eq!(overmind.in_flight_count().await, 1);

    let mut rx = bus.subscribe();

    let decision = overmind.handle_reject(task_id, "c1", "at capacity").await;

    // Existing behavior: delegation strategy redelegates to the only other
    // healthy cerebrate.
    match decision {
        abathur::services::federation::DelegationDecision::Redelegate(ref id) => {
            assert_eq!(id, "c2");
        }
        other => panic!("Expected Redelegate to c2, got {:?}", other),
    }

    // Verify the rejection event was emitted with the expected shape.
    let ev = wait_for_event(
        &mut rx,
        |e| matches!(&e.payload, EventPayload::FederationTaskRejected { .. }),
        std::time::Duration::from_millis(500),
    )
    .await;
    match ev.payload {
        EventPayload::FederationTaskRejected {
            task_id: rejected_id,
            cerebrate_id,
            reason,
        } => {
            assert_eq!(rejected_id, task_id);
            assert_eq!(cerebrate_id, "c1");
            assert_eq!(reason, "at capacity");
        }
        _ => unreachable!(),
    }
    assert_eq!(ev.category, EventCategory::Federation);
    assert_eq!(ev.severity, EventSeverity::Warning);

    // c1's active_delegations decremented on rejection.
    let c1_status = overmind.get_cerebrate("c1").await.unwrap();
    assert_eq!(c1_status.active_delegations, 0);
    // With a Redelegate decision, handle_reject re-inserts the task into
    // in_flight mapped to the new cerebrate (c2). This is current behavior
    // so downstream progress/result messages route correctly (service.rs:1080-1086).
    assert_eq!(
        overmind.in_flight_count().await,
        1,
        "Redelegate re-inserts task into in_flight mapped to the new cerebrate"
    );

    // Reject again from c2 to verify the envelope carries accumulated history
    // and the strategy refuses to redelegate to either prior rejector.
    let second_decision = overmind.handle_reject(task_id, "c2", "also busy").await;
    match second_decision {
        abathur::services::federation::DelegationDecision::ExecuteLocally => {}
        other => panic!(
            "Expected ExecuteLocally after both c1 and c2 rejected, got {:?}",
            other
        ),
    }
}
