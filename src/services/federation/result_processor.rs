//! Inbound federation result / progress / accept ingest.
//!
//! `ResultProcessor` owns the inbound side of federation: the messages a
//! cerebrate sends back when it accepts a task, makes progress, or returns
//! a final result. It updates shared in-flight / activity state, decrements
//! delegation counters on terminal statuses, emits the corresponding
//! `FederationResultReceived` / `FederationProgressReceived` events, and
//! runs the user-supplied result processor to produce `FederationReaction`s.
//!
//! This type is a private implementation detail of `FederationService`;
//! it shares state with the service via `Arc`s rather than owning it
//! outright.
//!
//! Not re-exported from the module root.
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use uuid::Uuid;

use crate::domain::models::a2a::{
    CerebrateStatus, FederationResult, FederationTaskEnvelope, FederationTaskStatus,
};
use crate::services::event_bus::{EventBus, EventPayload, EventSeverity};
use crate::services::event_factory;

use super::traits::{
    FederationReaction, FederationResultProcessor as FederationResultProcessorTrait,
    ParentContext, ResultSchema,
};

/// Inbound result ingest — see module docs.
pub(super) struct ResultProcessor {
    cerebrates: Arc<RwLock<HashMap<String, CerebrateStatus>>>,
    in_flight: Arc<RwLock<HashMap<Uuid, String>>>,
    delegated_envelopes: Arc<RwLock<HashMap<Uuid, FederationTaskEnvelope>>>,
    rejection_history: Arc<RwLock<HashMap<Uuid, Vec<String>>>>,
    last_activity: Arc<RwLock<HashMap<Uuid, chrono::DateTime<chrono::Utc>>>>,
    event_bus: Arc<EventBus>,
    result_processor: Arc<dyn FederationResultProcessorTrait>,
    #[allow(dead_code)]
    schemas: Arc<RwLock<HashMap<String, Arc<dyn ResultSchema>>>>,
}

impl ResultProcessor {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn new(
        cerebrates: Arc<RwLock<HashMap<String, CerebrateStatus>>>,
        in_flight: Arc<RwLock<HashMap<Uuid, String>>>,
        delegated_envelopes: Arc<RwLock<HashMap<Uuid, FederationTaskEnvelope>>>,
        rejection_history: Arc<RwLock<HashMap<Uuid, Vec<String>>>>,
        last_activity: Arc<RwLock<HashMap<Uuid, chrono::DateTime<chrono::Utc>>>>,
        event_bus: Arc<EventBus>,
        result_processor: Arc<dyn FederationResultProcessorTrait>,
        schemas: Arc<RwLock<HashMap<String, Arc<dyn ResultSchema>>>>,
    ) -> Self {
        Self {
            cerebrates,
            in_flight,
            delegated_envelopes,
            rejection_history,
            last_activity,
            event_bus,
            result_processor,
            schemas,
        }
    }

    /// Handle acceptance of a delegated task by a cerebrate.
    pub(super) async fn handle_accept(&self, task_id: Uuid, cerebrate_id: &str) {
        self.event_bus
            .publish(event_factory::federation_event(
                EventSeverity::Info,
                Some(task_id),
                EventPayload::FederationTaskAccepted {
                    task_id,
                    cerebrate_id: cerebrate_id.to_string(),
                },
            ))
            .await;
    }

    /// Handle a progress update from a cerebrate.
    pub(super) async fn handle_progress(
        &self,
        task_id: Uuid,
        cerebrate_id: &str,
        phase: &str,
        progress_pct: f64,
        summary: &str,
    ) {
        // Update last activity timestamp for stall detection
        {
            let mut activity = self.last_activity.write().await;
            activity.insert(task_id, chrono::Utc::now());
        }

        self.event_bus
            .publish(event_factory::federation_event(
                EventSeverity::Debug,
                Some(task_id),
                EventPayload::FederationProgressReceived {
                    task_id,
                    cerebrate_id: cerebrate_id.to_string(),
                    phase: phase.to_string(),
                    progress_pct,
                    summary: summary.to_string(),
                },
            ))
            .await;
    }

    /// Handle a final result from a cerebrate.
    pub(super) async fn handle_result(
        &self,
        result: FederationResult,
        parent_context: ParentContext,
    ) -> Vec<FederationReaction> {
        let task_id = result.task_id;
        let cerebrate_id = {
            let in_flight = self.in_flight.read().await;
            in_flight.get(&task_id).cloned().unwrap_or_default()
        };

        let is_terminal = matches!(
            result.status,
            FederationTaskStatus::Completed | FederationTaskStatus::Failed
        );

        // Only remove from in-flight and activity tracking on terminal statuses.
        // Partial results mean the task is still running on the cerebrate.
        if is_terminal {
            {
                let mut in_flight = self.in_flight.write().await;
                in_flight.remove(&task_id);
            }
            {
                let mut activity = self.last_activity.write().await;
                activity.remove(&task_id);
            }
            {
                let mut envs = self.delegated_envelopes.write().await;
                envs.remove(&task_id);
            }
            {
                let mut history = self.rejection_history.write().await;
                history.remove(&task_id);
            }
            // Note: task_to_federated_goal is intentionally NOT cleaned up here.
            // The FederationResultHandler needs to look up the mapping when it
            // processes the FederationResultReceived event that we emit below.
            // The mapping is small and bounded by the number of delegations.

            // Decrement active delegations only on terminal statuses
            if !cerebrate_id.is_empty() {
                let mut cerebrates = self.cerebrates.write().await;
                if let Some(status) = cerebrates.get_mut(&cerebrate_id) {
                    status.active_delegations = status.active_delegations.saturating_sub(1);
                }
            }
        } else {
            // Partial result: update last activity for stall detection
            let mut activity = self.last_activity.write().await;
            activity.insert(task_id, chrono::Utc::now());
        }

        // Validate against schema if specified
        // (Schema validation would happen on the raw JSON payload in a real impl)

        // Emit result event
        self.event_bus
            .publish(event_factory::federation_event(
                EventSeverity::Info,
                Some(task_id),
                EventPayload::FederationResultReceived {
                    task_id,
                    cerebrate_id: cerebrate_id.clone(),
                    status: result.status.to_string(),
                    summary: result.summary.clone(),
                    artifacts: result.artifacts.clone(),
                },
            ))
            .await;

        // Process through result processor
        match result.status {
            FederationTaskStatus::Completed | FederationTaskStatus::Partial => {
                self.result_processor
                    .process_result(&result, &parent_context)
            }
            FederationTaskStatus::Failed => {
                self.result_processor
                    .process_failure(&result, &parent_context)
            }
        }
    }
}
