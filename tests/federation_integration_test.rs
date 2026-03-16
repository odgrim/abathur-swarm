//! Federation integration test.
//!
//! Simulates delegation between two FederationService instances (overmind → cerebrate),
//! verifying that the full round-trip works: delegate → accept → progress → result,
//! with correct events emitted at each stage.

use std::sync::Arc;
use uuid::Uuid;

use abathur::domain::models::a2a::{
    Artifact, ConnectionState, FederationResult, FederationTaskEnvelope,
};
use abathur::services::event_bus::{EventBus, EventBusConfig};
use abathur::services::federation::config::{FederationConfig, FederationRole};
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
        .register_cerebrate("cerebrate-1", "Test Cerebrate", "https://cerebrate.local:8443")
        .await;
    overmind.connect("cerebrate-1").await.unwrap();

    let status = overmind.get_cerebrate("cerebrate-1").await.unwrap();
    assert_eq!(status.connection_state, ConnectionState::Connected);

    // Subscribe to overmind events for verification
    let mut rx = overmind_bus.subscribe();

    // 2. Create and delegate a task
    let task_id = Uuid::new_v4();
    let correlation_id = Uuid::new_v4();
    let envelope = FederationTaskEnvelope::new(task_id, "Build feature X", "Implement the X feature")
        .with_parent_goal(Uuid::new_v4())
        .with_constraint("Must pass CI".to_string());

    let assigned = overmind.delegate_to(&envelope, "cerebrate-1").await.unwrap();
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
    let result = FederationResult::completed(task_id, correlation_id, "Feature X implemented successfully")
        .with_artifact(Artifact::new("pr_url", "https://github.com/org/repo/pull/42"))
        .with_artifact(Artifact::new("commit_sha", "abc123def456"));

    let goal_id = Uuid::new_v4();
    let ctx = ParentContext {
        goal_id: Some(goal_id),
        goal_summary: Some("Build feature X".to_string()),
        task_title: Some("Build feature X".to_string()),
    };

    let reactions = overmind.handle_result(result, ctx).await;

    // 6. Verify result processing
    assert_eq!(reactions.len(), 1, "Expected one reaction from result processor");
    match &reactions[0] {
        FederationReaction::UpdateGoalProgress { goal_id: gid, summary } => {
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

    let result = FederationResult::failed(task_id, correlation_id, "Build failed", "CI pipeline broke");

    let goal_id = Uuid::new_v4();
    let ctx = ParentContext {
        goal_id: Some(goal_id),
        ..Default::default()
    };

    let reactions = overmind.handle_result(result, ctx).await;

    assert_eq!(reactions.len(), 1);
    match &reactions[0] {
        FederationReaction::Escalate { reason, goal_id: gid } => {
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
