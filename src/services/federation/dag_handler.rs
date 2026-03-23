//! Swarm DAG event handler for the EventReactor.
//!
//! Listens for `FederatedGoalConverged` and `FederatedGoalFailed` events,
//! looks up which DAG node corresponds to the federated goal, and drives
//! the `SwarmDagExecutor` state machine forward.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::RwLock;
use uuid::Uuid;

use crate::domain::models::goal::Goal;
use crate::domain::models::swarm_dag::SwarmDag;
use crate::services::event_bus::{EventCategory, EventPayload, UnifiedEvent};
use crate::services::event_reactor::{
    ErrorStrategy, EventFilter, EventHandler, HandlerContext, HandlerId, HandlerMetadata,
    HandlerPriority, Reaction,
};

use super::swarm_dag_executor::SwarmDagExecutor;

/// Reactive handler that processes `FederatedGoalConverged` and
/// `FederatedGoalFailed` events, driving the DAG executor forward.
pub struct SwarmDagEventHandler {
    /// All active DAGs keyed by DAG ID.
    swarm_dags: Arc<RwLock<HashMap<Uuid, SwarmDag>>>,
    /// The executor that performs state transitions and delegation.
    dag_executor: Arc<SwarmDagExecutor>,
    /// A placeholder goal used when driving the executor (the real goal
    /// context is baked into each DAG node's intent/contract).
    placeholder_goal: Arc<RwLock<HashMap<Uuid, Goal>>>,
}

impl SwarmDagEventHandler {
    pub fn new(
        swarm_dags: Arc<RwLock<HashMap<Uuid, SwarmDag>>>,
        dag_executor: Arc<SwarmDagExecutor>,
        placeholder_goal: Arc<RwLock<HashMap<Uuid, Goal>>>,
    ) -> Self {
        Self {
            swarm_dags,
            dag_executor,
            placeholder_goal,
        }
    }

    /// Search all DAGs for a node whose `federated_goal_id` matches the given ID.
    /// Returns `(dag_id, node_id)` if found.
    fn find_node_by_federated_goal(
        dags: &HashMap<Uuid, SwarmDag>,
        federated_goal_id: Uuid,
    ) -> Option<(Uuid, Uuid)> {
        for (dag_id, dag) in dags {
            for node in &dag.nodes {
                if node.federated_goal_id == Some(federated_goal_id) {
                    return Some((*dag_id, node.id));
                }
            }
        }
        None
    }
}

#[async_trait]
impl EventHandler for SwarmDagEventHandler {
    fn metadata(&self) -> HandlerMetadata {
        HandlerMetadata {
            id: HandlerId::new(),
            name: "SwarmDagEventHandler".to_string(),
            filter: EventFilter {
                categories: vec![EventCategory::Federation],
                min_severity: None,
                goal_id: None,
                task_id: None,
                payload_types: vec![
                    "FederatedGoalConverged".to_string(),
                    "FederatedGoalFailed".to_string(),
                ],
                custom_predicate: None,
            },
            priority: HandlerPriority::NORMAL,
            error_strategy: ErrorStrategy::LogAndContinue,
            critical: false,
        }
    }

    async fn handle(&self, event: &UnifiedEvent, _ctx: &HandlerContext) -> Result<Reaction, String> {
        match &event.payload {
            EventPayload::FederatedGoalConverged {
                local_goal_id,
                cerebrate_id,
            } => {
                tracing::info!(
                    local_goal_id = %local_goal_id,
                    cerebrate_id = %cerebrate_id,
                    "DAG handler: federated goal converged, checking DAG nodes"
                );

                let mut dags = self.swarm_dags.write().await;
                let lookup = Self::find_node_by_federated_goal(&dags, *local_goal_id);

                if let Some((dag_id, node_id)) = lookup {
                    tracing::info!(
                        dag_id = %dag_id,
                        node_id = %node_id,
                        "DAG handler: found matching node, advancing DAG"
                    );

                    let dag = dags.get_mut(&dag_id).ok_or_else(|| {
                        format!("DAG {} disappeared during lookup", dag_id)
                    })?;

                    // Get the goal for this DAG (or use a minimal placeholder).
                    let goals = self.placeholder_goal.read().await;
                    let goal = goals.get(&dag_id);

                    if let Some(goal) = goal {
                        match self.dag_executor.on_node_converged(dag, node_id, goal).await {
                            Ok(newly_delegated) => {
                                tracing::info!(
                                    dag_id = %dag_id,
                                    newly_delegated = newly_delegated.len(),
                                    "DAG handler: node converged, delegated {} new nodes",
                                    newly_delegated.len()
                                );
                            }
                            Err(e) => {
                                tracing::error!(
                                    dag_id = %dag_id,
                                    node_id = %node_id,
                                    error = %e,
                                    "DAG handler: failed to process node convergence"
                                );
                                return Err(format!(
                                    "Failed to process node convergence: {}",
                                    e
                                ));
                            }
                        }
                    } else {
                        tracing::warn!(
                            dag_id = %dag_id,
                            "DAG handler: no goal found for DAG, skipping executor call"
                        );
                    }
                } else {
                    tracing::debug!(
                        local_goal_id = %local_goal_id,
                        "DAG handler: no DAG node found for converged goal, ignoring"
                    );
                }

                Ok(Reaction::None)
            }

            EventPayload::FederatedGoalFailed {
                local_goal_id,
                cerebrate_id,
                reason,
            } => {
                tracing::info!(
                    local_goal_id = %local_goal_id,
                    cerebrate_id = %cerebrate_id,
                    reason = %reason,
                    "DAG handler: federated goal failed, checking DAG nodes"
                );

                let mut dags = self.swarm_dags.write().await;
                let lookup = Self::find_node_by_federated_goal(&dags, *local_goal_id);

                if let Some((dag_id, node_id)) = lookup {
                    tracing::info!(
                        dag_id = %dag_id,
                        node_id = %node_id,
                        "DAG handler: found matching node, failing DAG node"
                    );

                    let dag = dags.get_mut(&dag_id).ok_or_else(|| {
                        format!("DAG {} disappeared during lookup", dag_id)
                    })?;

                    match self.dag_executor.on_node_failed(dag, node_id, reason).await {
                        Ok(cascaded_failures) => {
                            tracing::info!(
                                dag_id = %dag_id,
                                cascaded = cascaded_failures.len(),
                                "DAG handler: node failed, cascaded to {} dependent nodes",
                                cascaded_failures.len()
                            );
                        }
                        Err(e) => {
                            tracing::error!(
                                dag_id = %dag_id,
                                node_id = %node_id,
                                error = %e,
                                "DAG handler: failed to process node failure"
                            );
                            return Err(format!(
                                "Failed to process node failure: {}",
                                e
                            ));
                        }
                    }
                } else {
                    tracing::debug!(
                        local_goal_id = %local_goal_id,
                        "DAG handler: no DAG node found for failed goal, ignoring"
                    );
                }

                Ok(Reaction::None)
            }

            _ => Ok(Reaction::None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::models::goal_federation::ConvergenceContract;
    use crate::domain::models::swarm_dag::{SwarmDagNode, SwarmDagNodeState};
    use crate::services::event_bus::{EventBus, EventBusConfig, EventId, EventSeverity, SequenceNumber};
    use crate::services::federation::config::FederationConfig;
    use crate::services::federation::service::FederationService;

    fn make_event(payload: EventPayload) -> UnifiedEvent {
        UnifiedEvent {
            id: EventId::new(),
            sequence: SequenceNumber::zero(),
            timestamp: chrono::Utc::now(),
            severity: EventSeverity::Info,
            category: EventCategory::Federation,
            goal_id: None,
            task_id: None,
            correlation_id: None,
            source_process_id: None,
            payload,
        }
    }

    fn make_test_dag() -> (SwarmDag, Uuid, Uuid) {
        let mut dag = SwarmDag::new("test-dag");

        let fed_goal_id = Uuid::new_v4();
        let node = SwarmDagNode {
            id: Uuid::new_v4(),
            label: "code".to_string(),
            cerebrate_id: "code-swarm".to_string(),
            intent: "Build the code".to_string(),
            contract: ConvergenceContract::default(),
            dependencies: vec![],
            federated_goal_id: Some(fed_goal_id),
            state: SwarmDagNodeState::Delegated,
        };
        let node_id = node.id;
        dag.add_node(node);

        let deploy_node = SwarmDagNode {
            id: Uuid::new_v4(),
            label: "deploy".to_string(),
            cerebrate_id: "deploy-swarm".to_string(),
            intent: "Deploy".to_string(),
            contract: ConvergenceContract::default(),
            dependencies: vec![node_id],
            federated_goal_id: None,
            state: SwarmDagNodeState::Waiting,
        };
        dag.add_node(deploy_node);

        (dag, fed_goal_id, node_id)
    }

    fn make_handler(
        dags: Arc<RwLock<HashMap<Uuid, SwarmDag>>>,
    ) -> SwarmDagEventHandler {
        let config = FederationConfig::default();
        let event_bus = Arc::new(EventBus::new(EventBusConfig::default()));
        let federation_service = Arc::new(FederationService::new(config, event_bus.clone()));
        let dag_executor = Arc::new(SwarmDagExecutor::new(federation_service, event_bus));
        let goals = Arc::new(RwLock::new(HashMap::new()));
        SwarmDagEventHandler::new(dags, dag_executor, goals)
    }

    #[test]
    fn test_metadata() {
        let dags = Arc::new(RwLock::new(HashMap::new()));
        let handler = make_handler(dags);
        let meta = handler.metadata();
        assert_eq!(meta.name, "SwarmDagEventHandler");
        assert!(meta.filter.categories.contains(&EventCategory::Federation));
        assert!(meta.filter.payload_types.contains(&"FederatedGoalConverged".to_string()));
        assert!(meta.filter.payload_types.contains(&"FederatedGoalFailed".to_string()));
    }

    #[test]
    fn test_find_node_by_federated_goal_found() {
        let (dag, fed_goal_id, node_id) = make_test_dag();
        let dag_id = dag.id;
        let mut dags = HashMap::new();
        dags.insert(dag_id, dag);

        let result = SwarmDagEventHandler::find_node_by_federated_goal(&dags, fed_goal_id);
        assert!(result.is_some());
        let (found_dag_id, found_node_id) = result.unwrap();
        assert_eq!(found_dag_id, dag_id);
        assert_eq!(found_node_id, node_id);
    }

    #[test]
    fn test_find_node_by_federated_goal_not_found() {
        let (dag, _, _) = make_test_dag();
        let mut dags = HashMap::new();
        dags.insert(dag.id, dag);

        let result =
            SwarmDagEventHandler::find_node_by_federated_goal(&dags, Uuid::new_v4());
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_handle_converged_no_matching_dag() {
        let dags = Arc::new(RwLock::new(HashMap::new()));
        let handler = make_handler(dags);
        let event = make_event(EventPayload::FederatedGoalConverged {
            local_goal_id: Uuid::new_v4(),
            cerebrate_id: "test-cerebrate".to_string(),
        });
        let ctx = HandlerContext {
            chain_depth: 0,
            correlation_id: None,
        };
        let result = handler.handle(&event, &ctx).await;
        assert!(result.is_ok());
        assert!(matches!(result.unwrap(), Reaction::None));
    }

    #[tokio::test]
    async fn test_handle_failed_no_matching_dag() {
        let dags = Arc::new(RwLock::new(HashMap::new()));
        let handler = make_handler(dags);
        let event = make_event(EventPayload::FederatedGoalFailed {
            local_goal_id: Uuid::new_v4(),
            cerebrate_id: "test-cerebrate".to_string(),
            reason: "something broke".to_string(),
        });
        let ctx = HandlerContext {
            chain_depth: 0,
            correlation_id: None,
        };
        let result = handler.handle(&event, &ctx).await;
        assert!(result.is_ok());
        assert!(matches!(result.unwrap(), Reaction::None));
    }

    #[tokio::test]
    async fn test_handle_unrelated_event() {
        let dags = Arc::new(RwLock::new(HashMap::new()));
        let handler = make_handler(dags);
        let event = make_event(EventPayload::FederationHeartbeatMissed {
            cerebrate_id: "test".to_string(),
            missed_count: 1,
        });
        let ctx = HandlerContext {
            chain_depth: 0,
            correlation_id: None,
        };
        let result = handler.handle(&event, &ctx).await;
        assert!(result.is_ok());
        assert!(matches!(result.unwrap(), Reaction::None));
    }
}
