//! Outbound task delegation lifecycle.
//!
//! `DelegationManager` owns the outbound side of federation: it selects
//! cerebrates, sends tasks over the wire (A2A or legacy HTTP), tracks
//! in-flight delegations, and runs the stall / orphan / reconciliation
//! monitors that correct drift in delegation state.
//!
//! This type is a private implementation detail of `FederationService`;
//! it shares state with the service via `Arc`s rather than owning it
//! outright, so test assertions against `FederationService` fields stay
//! consistent with the manager's view.
//!
//! Not re-exported from the module root.
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use uuid::Uuid;

use crate::adapters::a2a::A2AClient;
use crate::domain::models::a2a::{
    CerebrateStatus, ConnectionState, FederationTaskEnvelope, MessagePriority,
};
use crate::domain::models::a2a_protocol::{
    A2APart, A2AProtocolMessage, A2ARole, TaskSendParams,
};
use crate::domain::models::goal::{Goal, GoalPriority};
use crate::domain::models::goal_federation::{
    ConvergenceContract, FederatedGoal, FederatedGoalState,
};
use crate::services::event_bus::{EventBus, EventPayload, EventSeverity};
use crate::services::event_factory;

use super::config::FederationConfig;
use super::service::FederationHttpClient;
use super::traits::{
    DelegationDecision, FederationDelegationStrategy, FederationTaskTransformer,
};

/// Outbound delegation manager — see module docs.
pub(super) struct DelegationManager {
    config: FederationConfig,
    cerebrates: Arc<RwLock<HashMap<String, CerebrateStatus>>>,
    in_flight: Arc<RwLock<HashMap<Uuid, String>>>,
    delegated_envelopes: Arc<RwLock<HashMap<Uuid, FederationTaskEnvelope>>>,
    rejection_history: Arc<RwLock<HashMap<Uuid, Vec<String>>>>,
    last_activity: Arc<RwLock<HashMap<Uuid, chrono::DateTime<chrono::Utc>>>>,
    task_to_federated_goal: Arc<RwLock<HashMap<String, Uuid>>>,
    event_bus: Arc<EventBus>,
    http_client: FederationHttpClient,
    delegation_strategy: Arc<dyn FederationDelegationStrategy>,
    // reason: shared with the federation service so callers can register
    // task transformers; consumed via the trait impl rather than directly
    // off this struct's field.
    #[allow(dead_code)]
    task_transformer: Arc<dyn FederationTaskTransformer>,
    a2a_client: Option<Arc<dyn A2AClient>>,
}

/// Inputs for [`DelegationManager::new`].
pub(super) struct DelegationManagerParams {
    pub config: FederationConfig,
    pub cerebrates: Arc<RwLock<HashMap<String, CerebrateStatus>>>,
    pub in_flight: Arc<RwLock<HashMap<Uuid, String>>>,
    pub delegated_envelopes: Arc<RwLock<HashMap<Uuid, FederationTaskEnvelope>>>,
    pub rejection_history: Arc<RwLock<HashMap<Uuid, Vec<String>>>>,
    pub last_activity: Arc<RwLock<HashMap<Uuid, chrono::DateTime<chrono::Utc>>>>,
    pub task_to_federated_goal: Arc<RwLock<HashMap<String, Uuid>>>,
    pub event_bus: Arc<EventBus>,
    pub http_client: FederationHttpClient,
    pub delegation_strategy: Arc<dyn FederationDelegationStrategy>,
    pub task_transformer: Arc<dyn FederationTaskTransformer>,
    pub a2a_client: Option<Arc<dyn A2AClient>>,
}

impl DelegationManager {
    pub(super) fn new(params: DelegationManagerParams) -> Self {
        Self {
            config: params.config,
            cerebrates: params.cerebrates,
            in_flight: params.in_flight,
            delegated_envelopes: params.delegated_envelopes,
            rejection_history: params.rejection_history,
            last_activity: params.last_activity,
            task_to_federated_goal: params.task_to_federated_goal,
            event_bus: params.event_bus,
            http_client: params.http_client,
            delegation_strategy: params.delegation_strategy,
            task_transformer: params.task_transformer,
            a2a_client: params.a2a_client,
        }
    }

    /// Delegate a task to the best available cerebrate.
    pub(super) async fn delegate(
        &self,
        envelope: FederationTaskEnvelope,
    ) -> Result<String, String> {
        let cerebrates = self.list_cerebrates().await;
        let cerebrate_id = self
            .delegation_strategy
            .select_cerebrate(&envelope, &cerebrates)
            .ok_or_else(|| "No suitable cerebrate available".to_string())?;

        self.delegate_to(&envelope, &cerebrate_id).await
    }

    async fn list_cerebrates(&self) -> Vec<CerebrateStatus> {
        let cerebrates = self.cerebrates.read().await;
        cerebrates.values().cloned().collect()
    }

    /// Delegate a task to a specific cerebrate.
    ///
    /// NOTE: The `active_delegations` counter on `CerebrateStatus` can become
    /// stale if a cerebrate restarts or the overmind crashes mid-delegation.
    /// The `reconcile_on_reconnect()` mechanism is designed to correct this
    /// drift when a cerebrate reconnects after being unreachable.
    pub(super) async fn delegate_to(
        &self,
        envelope: &FederationTaskEnvelope,
        cerebrate_id: &str,
    ) -> Result<String, String> {
        // Verify cerebrate can accept
        {
            let cerebrates = self.cerebrates.read().await;
            let status = cerebrates
                .get(cerebrate_id)
                .ok_or_else(|| format!("Unknown cerebrate: {}", cerebrate_id))?;

            if !status.can_accept_task() {
                return Err(format!(
                    "Cerebrate {} cannot accept tasks (state: {}, delegations: {}/{})",
                    cerebrate_id,
                    status.connection_state,
                    status.active_delegations,
                    status.max_concurrent_delegations
                ));
            }
        }

        // Get the cerebrate URL before attempting to send
        let url = {
            let cerebrates = self.cerebrates.read().await;
            cerebrates.get(cerebrate_id).and_then(|s| s.url.clone())
        };

        // Send the envelope to the remote cerebrate.
        // Prefer a2a_client when available; fall back to legacy http_client.
        if let Some(ref url) = url {
            let mut sent_via_a2a = false;

            if let Some(ref a2a) = self.a2a_client {
                let params = TaskSendParams::from(envelope);
                match a2a.send_message(url, params).await {
                    Ok(task) => {
                        tracing::info!(
                            cerebrate_id = %cerebrate_id,
                            task_id = %envelope.task_id,
                            a2a_task_id = %task.id,
                            "Delegated via A2A tasks/send"
                        );
                        sent_via_a2a = true;
                    }
                    Err(e) => {
                        tracing::warn!(
                            cerebrate_id = %cerebrate_id,
                            task_id = %envelope.task_id,
                            error = %e,
                            "A2A delegate failed, falling back to legacy HTTP"
                        );
                    }
                }
            }

            if !sent_via_a2a
                && let Err(e) = self.http_client.delegate(url, envelope).await {
                    if self.a2a_client.is_some() {
                        // Both A2A and legacy HTTP failed — return error to the
                        // caller rather than silently continuing (Issue #8).
                        return Err(format!(
                            "Failed to delegate task {} to cerebrate {}: {}",
                            envelope.task_id, cerebrate_id, e
                        ));
                    }
                    // Legacy-only path: HTTP failure is non-fatal. The task is
                    // tracked in-flight and the cerebrate may still process it
                    // (e.g. local/test setups without a real HTTP endpoint).
                    tracing::warn!(
                        cerebrate_id = %cerebrate_id,
                        task_id = %envelope.task_id,
                        error = %e,
                        "HTTP delegate call failed, task tracked in-flight for monitoring"
                    );
                }
        }

        // Track in-flight, activity timestamp, and increment active delegations
        // AFTER the send succeeds to avoid stale counters on failure.
        {
            let mut in_flight = self.in_flight.write().await;
            in_flight.insert(envelope.task_id, cerebrate_id.to_string());
        }
        {
            // Snapshot envelope for later context lookup (e.g. rejection
            // handling rebuilding a context-carrying envelope for the
            // delegation strategy).
            let mut envs = self.delegated_envelopes.write().await;
            envs.insert(envelope.task_id, envelope.clone());
        }
        {
            let mut activity = self.last_activity.write().await;
            activity.insert(envelope.task_id, chrono::Utc::now());
        }
        {
            let mut cerebrates = self.cerebrates.write().await;
            if let Some(status) = cerebrates.get_mut(cerebrate_id) {
                status.active_delegations += 1;
            }
        }

        self.event_bus
            .publish(event_factory::federation_event(
                EventSeverity::Info,
                Some(envelope.task_id),
                EventPayload::FederationTaskDelegated {
                    task_id: envelope.task_id,
                    cerebrate_id: cerebrate_id.to_string(),
                },
            ))
            .await;

        Ok(cerebrate_id.to_string())
    }

    /// Delegate a goal to a specific cerebrate via A2A.
    ///
    /// Builds an A2A message with `abathur:federation` metadata containing
    /// the `goal_delegate` intent, sends it to the target cerebrate, creates
    /// an in-memory `FederatedGoal`, and emits a `FederatedGoalCreated` event.
    pub(super) async fn delegate_goal(
        &self,
        goal: &Goal,
        cerebrate_id: &str,
        contract: ConvergenceContract,
    ) -> Result<FederatedGoal, String> {
        // 1. Verify the cerebrate exists, is connected, and can accept tasks.
        let url = {
            let cerebrates = self.cerebrates.read().await;
            let status = cerebrates
                .get(cerebrate_id)
                .ok_or_else(|| format!("Unknown cerebrate: {}", cerebrate_id))?;

            if !status.can_accept_task() {
                return Err(format!(
                    "Cerebrate {} cannot accept tasks (state: {}, delegations: {}/{})",
                    cerebrate_id,
                    status.connection_state,
                    status.active_delegations,
                    status.max_concurrent_delegations,
                ));
            }
            status.url.clone()
        };

        // 2. Build federation metadata.
        let contract_json = serde_json::to_value(&contract)
            .map_err(|e| format!("Failed to serialize convergence contract: {}", e))?;

        let constraints_strs: Vec<String> = goal
            .constraints
            .iter()
            .map(|c| c.description.clone())
            .collect();

        let mut federation_data = serde_json::Map::new();
        federation_data.insert(
            "intent".to_string(),
            serde_json::Value::String("goal_delegate".to_string()),
        );
        federation_data.insert(
            "goal_id".to_string(),
            serde_json::Value::String(goal.id.to_string()),
        );
        federation_data.insert(
            "goal_name".to_string(),
            serde_json::Value::String(goal.name.clone()),
        );
        federation_data.insert(
            "goal_description".to_string(),
            serde_json::Value::String(goal.description.clone()),
        );
        federation_data.insert(
            "constraints".to_string(),
            serde_json::to_value(&constraints_strs).unwrap_or_default(),
        );
        federation_data.insert(
            "priority".to_string(),
            serde_json::Value::String(goal.priority.as_str().to_string()),
        );
        federation_data.insert(
            "convergence_contract".to_string(),
            contract_json,
        );

        let mut metadata = std::collections::HashMap::new();
        metadata.insert(
            "abathur:federation".to_string(),
            serde_json::Value::Object(federation_data),
        );

        let params = TaskSendParams {
            message: A2AProtocolMessage {
                role: A2ARole::User,
                parts: vec![
                    A2APart::Text {
                        text: format!("{}\n\n{}", goal.name, goal.description),
                    },
                    A2APart::Data {
                        data: serde_json::json!({
                            "goal_name": goal.name,
                            "goal_description": goal.description,
                            "priority": goal.priority.as_str(),
                            "constraints": constraints_strs,
                        }),
                        metadata: None,
                    },
                ],
                metadata: None,
            },
            metadata: Some(metadata),
            history_length: None,
            push_notification_config: None,
        };

        // 3 & 4. Send via A2A client or fall back to legacy delegation.
        let remote_task_id = if let Some(ref a2a) = self.a2a_client {
            if let Some(ref url) = url {
                match a2a.send_message(url, params).await {
                    Ok(task) => {
                        tracing::info!(
                            cerebrate_id = %cerebrate_id,
                            goal_id = %goal.id,
                            a2a_task_id = %task.id,
                            "Goal delegated via A2A tasks/send"
                        );
                        task.id
                    }
                    Err(e) => {
                        return Err(format!(
                            "A2A goal delegation failed for cerebrate {}: {}",
                            cerebrate_id, e
                        ));
                    }
                }
            } else {
                return Err(format!(
                    "Cerebrate {} has no URL configured",
                    cerebrate_id
                ));
            }
        } else {
            // Fall back: create a FederationTaskEnvelope and use existing delegate path
            let mut envelope =
                FederationTaskEnvelope::new(Uuid::new_v4(), &goal.name, &goal.description);
            envelope.constraints = constraints_strs;
            envelope.parent_goal_id = Some(goal.id);
            envelope.priority = match goal.priority {
                GoalPriority::Low => MessagePriority::Low,
                GoalPriority::Normal => MessagePriority::Normal,
                GoalPriority::High => MessagePriority::High,
                GoalPriority::Critical => MessagePriority::Urgent,
            };

            let delegated_cerebrate = self.delegate_to(&envelope, cerebrate_id).await?;
            tracing::info!(
                cerebrate_id = %delegated_cerebrate,
                goal_id = %goal.id,
                task_id = %envelope.task_id,
                "Goal delegated via legacy federation envelope"
            );
            envelope.task_id.to_string()
        };

        // 5. Create a FederatedGoal in the Delegated state.
        let federated_goal = FederatedGoal::new(goal.id, cerebrate_id, &goal.description)
            .with_convergence_contract(contract)
            .with_remote_task_id(&remote_task_id);
        // State set directly: this is valid because the goal was just created above
        // with state Pending, and Pending -> Delegated is a valid transition.
        debug_assert!(FederatedGoalState::Pending.can_transition_to(FederatedGoalState::Delegated));
        let mut federated_goal = federated_goal;
        federated_goal.state = FederatedGoalState::Delegated;
        // Carry over constraints from the goal.
        for c in &goal.constraints {
            federated_goal.constraints.push(c.description.clone());
        }

        // 6. Emit FederatedGoalCreated event.
        self.event_bus
            .publish(event_factory::federation_event(
                EventSeverity::Info,
                None,
                EventPayload::FederatedGoalCreated {
                    local_goal_id: goal.id,
                    cerebrate_id: cerebrate_id.to_string(),
                    remote_task_id: remote_task_id.clone(),
                },
            ))
            .await;

        // 7. Record the task_id → federated_goal.id mapping so result handlers
        //    can correlate incoming FederationResultReceived events back to the
        //    FederatedGoal (and therefore the DAG node).
        {
            let mut map = self.task_to_federated_goal.write().await;
            map.insert(remote_task_id.clone(), federated_goal.id);
        }

        // 8. Return the FederatedGoal.
        Ok(federated_goal)
    }

    /// Handle rejection of a delegated task by a cerebrate.
    pub(super) async fn handle_reject(
        &self,
        task_id: Uuid,
        cerebrate_id: &str,
        reason: &str,
    ) -> DelegationDecision {
        // Remove from in-flight
        {
            let mut in_flight = self.in_flight.write().await;
            in_flight.remove(&task_id);
        }

        // Decrement active delegations
        {
            let mut cerebrates = self.cerebrates.write().await;
            if let Some(status) = cerebrates.get_mut(cerebrate_id) {
                status.active_delegations = status.active_delegations.saturating_sub(1);
            }
        }

        // Record rejection in history (append cerebrate_id).
        let rejected_by: Vec<String> = {
            let mut history = self.rejection_history.write().await;
            let entry = history.entry(task_id).or_default();
            entry.push(cerebrate_id.to_string());
            entry.clone()
        };

        self.event_bus
            .publish(event_factory::federation_event(
                EventSeverity::Warning,
                Some(task_id),
                EventPayload::FederationTaskRejected {
                    task_id,
                    cerebrate_id: cerebrate_id.to_string(),
                    reason: reason.to_string(),
                },
            ))
            .await;

        // Get remaining cerebrates for strategy decision
        let remaining = self.list_cerebrates().await;

        // Compute peer load hints from current in_flight tracking:
        // count per cerebrate_id.
        let peer_load_hints: Vec<(String, u32)> = {
            let in_flight = self.in_flight.read().await;
            let mut counts: HashMap<String, u32> = HashMap::new();
            for cid in in_flight.values() {
                *counts.entry(cid.clone()).or_insert(0) += 1;
            }
            // Ensure every known cerebrate appears (with 0 if idle) so the
            // strategy can reason about the whole peer set.
            for c in &remaining {
                counts.entry(c.id.clone()).or_insert(0);
            }
            counts.into_iter().collect()
        };

        // Build the context-carrying envelope for the strategy. Prefer the
        // original envelope snapshot (carries title/description/capabilities/
        // parent_task_id); fall back to a minimal shell if missing.
        let envelope = {
            let envs = self.delegated_envelopes.read().await;
            envs.get(&task_id).cloned()
        }
        .unwrap_or_else(|| FederationTaskEnvelope::new(task_id, "", ""))
        .with_rejection_history(rejected_by)
        .with_peer_load_hints(peer_load_hints);

        let decision = self
            .delegation_strategy
            .on_rejection(&envelope, cerebrate_id, reason, &remaining);

        // If the strategy decides to redelegate, update in_flight to point at
        // the new cerebrate so subsequent result/progress messages are routed
        // correctly. Also refresh the stored envelope so subsequent rejections
        // see the updated rejection history.
        if let DelegationDecision::Redelegate(ref new_cerebrate_id) = decision {
            {
                let mut in_flight = self.in_flight.write().await;
                in_flight.insert(task_id, new_cerebrate_id.clone());
            }
            {
                let mut envs = self.delegated_envelopes.write().await;
                envs.insert(task_id, envelope);
            }
        }

        decision
    }

    /// Get the number of in-flight tasks.
    pub(super) async fn in_flight_count(&self) -> usize {
        self.in_flight.read().await.len()
    }

    /// Get in-flight tasks for a specific cerebrate.
    pub(super) async fn in_flight_for_cerebrate(&self, cerebrate_id: &str) -> Vec<Uuid> {
        let in_flight = self.in_flight.read().await;
        in_flight
            .iter()
            .filter(|(_, cid)| cid.as_str() == cerebrate_id)
            .map(|(tid, _)| *tid)
            .collect()
    }

    /// Check for stalled delegations (no progress within stall_timeout_secs).
    /// Emits `FederationStallDetected` events for any stalled tasks.
    pub(super) async fn check_stalls(&self) {
        let stall_secs = self.config.stall_timeout_secs;
        if stall_secs == 0 {
            return;
        }
        let now = chrono::Utc::now();
        let in_flight = self.in_flight.read().await;
        let last_activity = self.last_activity.read().await;

        // Collect stalled tasks before dropping locks
        let mut stalled = Vec::new();
        for (task_id, cerebrate_id) in in_flight.iter() {
            let last = last_activity.get(task_id).copied();
            if let Some(last) = last {
                let elapsed = (now - last).num_seconds().unsigned_abs();
                if elapsed >= stall_secs {
                    stalled.push((*task_id, cerebrate_id.clone(), elapsed));
                }
            }
        }

        drop(in_flight);
        drop(last_activity);

        for (task_id, cerebrate_id, elapsed) in stalled {
            self.event_bus
                .publish(event_factory::federation_event(
                    EventSeverity::Warning,
                    Some(task_id),
                    EventPayload::FederationStallDetected {
                        task_id,
                        cerebrate_id,
                        stall_duration_secs: elapsed,
                    },
                ))
                .await;
        }
    }

    /// Fail tasks delegated to unreachable cerebrates after the orphan timeout.
    pub(super) async fn check_orphans(&self, orphan_timeout_secs: u64) {
        let now = chrono::Utc::now();
        let cerebrates = self.cerebrates.read().await;
        let in_flight = self.in_flight.read().await;
        let last_activity = self.last_activity.read().await;

        let mut orphaned = Vec::new();

        for (task_id, cerebrate_id) in in_flight.iter() {
            if let Some(status) = cerebrates.get(cerebrate_id)
                && status.connection_state == ConnectionState::Unreachable
            {
                // Check how long since last activity on this task
                let activity_time = last_activity
                    .get(task_id)
                    .copied()
                    .or(status.last_heartbeat_at)
                    .unwrap_or(now);
                let elapsed = (now - activity_time).num_seconds().unsigned_abs();
                if elapsed >= orphan_timeout_secs {
                    orphaned.push((*task_id, cerebrate_id.clone()));
                }
            }
        }
        drop(in_flight);
        drop(cerebrates);
        drop(last_activity);

        // Remove orphaned tasks from in-flight and emit events
        for (task_id, cerebrate_id) in orphaned {
            {
                let mut in_flight = self.in_flight.write().await;
                in_flight.remove(&task_id);
            }
            {
                let mut last_activity = self.last_activity.write().await;
                last_activity.remove(&task_id);
            }
            {
                let mut cerebrates = self.cerebrates.write().await;
                if let Some(status) = cerebrates.get_mut(&cerebrate_id) {
                    status.active_delegations = status.active_delegations.saturating_sub(1);
                }
            }

            tracing::warn!(
                task_id = %task_id,
                cerebrate_id = %cerebrate_id,
                "Failing orphaned federation task (cerebrate unreachable)"
            );

            // Emit a result event as if the task failed
            self.event_bus
                .publish(event_factory::federation_event(
                    EventSeverity::Error,
                    Some(task_id),
                    EventPayload::FederationResultReceived {
                        task_id,
                        cerebrate_id,
                        status: "failed".to_string(),
                        summary: "Task orphaned: cerebrate unreachable beyond timeout".to_string(),
                        artifacts: Vec::new(),
                    },
                ))
                .await;
        }
    }

    /// Reconcile in-flight task state after reconnecting to a cerebrate.
    ///
    /// Exchanges in-flight task ID lists over the wire. Tasks known locally
    /// but not remotely are failed as orphaned. Tasks known remotely but not
    /// locally are re-tracked.
    pub(super) async fn reconcile_on_reconnect(&self, cerebrate_id: &str) {
        let local_tasks = self.in_flight_for_cerebrate(cerebrate_id).await;

        tracing::info!(
            cerebrate_id = %cerebrate_id,
            task_count = local_tasks.len(),
            "Reconciling in-flight tasks after reconnection"
        );

        // Get the cerebrate URL for the HTTP reconcile call
        let url = {
            let cerebrates = self.cerebrates.read().await;
            cerebrates.get(cerebrate_id).and_then(|s| s.url.clone())
        };

        if let Some(ref url) = url {
            match self
                .http_client
                .reconcile(url, cerebrate_id, &local_tasks)
                .await
            {
                Ok(remote_tasks) => {
                    // Tasks we track locally but the remote doesn't know about → orphaned
                    let orphaned: Vec<Uuid> = local_tasks
                        .iter()
                        .filter(|t| !remote_tasks.contains(t))
                        .copied()
                        .collect();

                    for task_id in orphaned {
                        tracing::warn!(
                            task_id = %task_id,
                            cerebrate_id = %cerebrate_id,
                            "Task unknown to remote cerebrate after reconnect, failing as orphaned"
                        );
                        let mut in_flight = self.in_flight.write().await;
                        in_flight.remove(&task_id);
                        let mut activity = self.last_activity.write().await;
                        activity.remove(&task_id);

                        self.event_bus
                            .publish(event_factory::federation_event(
                                EventSeverity::Warning,
                                Some(task_id),
                                EventPayload::FederationResultReceived {
                                    task_id,
                                    cerebrate_id: cerebrate_id.to_string(),
                                    status: "failed".to_string(),
                                    summary: "Task lost during disconnection".to_string(),
                                    artifacts: Vec::new(),
                                },
                            ))
                            .await;
                    }

                    // Tasks the remote knows about but we don't track locally → rediscovered.
                    // This can happen if the overmind restarted while the cerebrate kept working.
                    let rediscovered: Vec<Uuid> = remote_tasks
                        .iter()
                        .filter(|t| !local_tasks.contains(t))
                        .copied()
                        .collect();

                    for task_id in rediscovered {
                        tracing::warn!(
                            task_id = %task_id,
                            cerebrate_id = %cerebrate_id,
                            "Task exists on remote cerebrate but not tracked locally, re-adding to in_flight"
                        );
                        {
                            let mut in_flight = self.in_flight.write().await;
                            in_flight.insert(task_id, cerebrate_id.to_string());
                        }
                        {
                            let mut activity = self.last_activity.write().await;
                            activity.insert(task_id, chrono::Utc::now());
                        }

                        self.event_bus
                            .publish(event_factory::federation_event(
                                EventSeverity::Warning,
                                Some(task_id),
                                EventPayload::FederationTaskAccepted {
                                    task_id,
                                    cerebrate_id: cerebrate_id.to_string(),
                                },
                            ))
                            .await;
                    }

                    // Re-sync the `active_delegations` counter on the
                    // cerebrate's status with the reconciled in_flight view.
                    // The counter can drift from reality across restarts and
                    // mid-delegation crashes (see NOTE on `delegate_to`); the
                    // post-reconcile in_flight set is the source of truth, so
                    // align the counter to it here.
                    let new_count = self.in_flight_for_cerebrate(cerebrate_id).await.len();
                    let mut cerebrates = self.cerebrates.write().await;
                    if let Some(status) = cerebrates.get_mut(cerebrate_id) {
                        let prev = status.active_delegations;
                        status.active_delegations = new_count as u32;
                        if prev != new_count as u32 {
                            tracing::info!(
                                cerebrate_id = %cerebrate_id,
                                previous = prev,
                                reconciled = new_count,
                                "Reconciled active_delegations counter to in_flight truth"
                            );
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!(
                        cerebrate_id = %cerebrate_id,
                        error = %e,
                        "Reconciliation HTTP call failed, keeping local state"
                    );
                }
            }
        }
    }
}
