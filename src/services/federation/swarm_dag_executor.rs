//! Swarm DAG executor — orchestrates cross-swarm goal delegation with dependencies.

use std::sync::Arc;
use uuid::Uuid;

use crate::domain::errors::DomainResult;
use crate::domain::models::goal::Goal;
use crate::domain::models::swarm_dag::*;
use crate::services::event_bus::{EventBus, EventPayload, EventSeverity};
use crate::services::event_factory;
use crate::services::federation::FederationService;

/// Orchestrates execution of a `SwarmDag`, delegating nodes to child swarms
/// as their dependencies are satisfied.
pub struct SwarmDagExecutor {
    federation_service: Arc<FederationService>,
    event_bus: Arc<EventBus>,
}

impl SwarmDagExecutor {
    pub fn new(federation_service: Arc<FederationService>, event_bus: Arc<EventBus>) -> Self {
        Self {
            federation_service,
            event_bus,
        }
    }

    /// Start executing a DAG — validate and delegate all root nodes.
    pub async fn start(&self, dag: &mut SwarmDag, goal: &Goal) -> DomainResult<Vec<Uuid>> {
        dag.validate().map_err(|e| {
            crate::domain::errors::DomainError::ValidationFailed(format!(
                "SwarmDag validation failed: {}",
                e
            ))
        })?;

        // Emit DAG created event.
        self.event_bus
            .publish(event_factory::federation_event(
                EventSeverity::Info,
                None,
                EventPayload::SwarmDagCreated {
                    dag_id: dag.id,
                    dag_name: dag.name.clone(),
                    node_count: dag.nodes.len(),
                },
            ))
            .await;

        // Find root node IDs (nodes with no dependencies).
        let root_ids: Vec<Uuid> = dag.roots().iter().map(|n| n.id).collect();

        let mut delegated = Vec::new();
        for node_id in root_ids {
            let fed_id = self.delegate_node(dag, node_id, goal).await?;
            delegated.push(fed_id);
        }

        Ok(delegated)
    }

    /// Called when a federated goal converges — unblock and delegate dependents.
    pub async fn on_node_converged(
        &self,
        dag: &mut SwarmDag,
        node_id: Uuid,
        goal: &Goal,
    ) -> DomainResult<Vec<Uuid>> {
        // Mark the node as Converged.
        if let Some(node) = dag.get_node_mut(node_id) {
            node.state = SwarmDagNodeState::Converged;
        }

        // Find dependents that are now ready.
        let dependent_ids: Vec<Uuid> = dag.dependents_of(node_id).iter().map(|n| n.id).collect();

        let mut newly_delegated = Vec::new();
        for dep_id in dependent_ids {
            // Check if ALL dependencies of this dependent are Converged.
            let all_deps_converged = {
                let dep_node = dag.get_node(dep_id);
                dep_node
                    .map(|n| {
                        n.state == SwarmDagNodeState::Waiting
                            && n.dependencies.iter().all(|d| {
                                dag.get_node(*d)
                                    .map(|dn| dn.state == SwarmDagNodeState::Converged)
                                    .unwrap_or(false)
                            })
                    })
                    .unwrap_or(false)
            };

            if all_deps_converged {
                // Emit unblocked event.
                let label = dag
                    .get_node(dep_id)
                    .map(|n| n.label.clone())
                    .unwrap_or_default();
                self.event_bus
                    .publish(event_factory::federation_event(
                        EventSeverity::Info,
                        None,
                        EventPayload::SwarmDagNodeUnblocked {
                            dag_id: dag.id,
                            node_id: dep_id,
                            node_label: label,
                        },
                    ))
                    .await;

                let fed_id = self.delegate_node(dag, dep_id, goal).await?;
                newly_delegated.push(fed_id);
            }
        }

        // Check if DAG is now complete.
        if dag.is_complete() {
            let converged_count = dag
                .nodes
                .iter()
                .filter(|n| n.state == SwarmDagNodeState::Converged)
                .count();
            let failed_count = dag
                .nodes
                .iter()
                .filter(|n| n.state == SwarmDagNodeState::Failed)
                .count();

            self.event_bus
                .publish(event_factory::federation_event(
                    EventSeverity::Info,
                    None,
                    EventPayload::SwarmDagCompleted {
                        dag_id: dag.id,
                        dag_name: dag.name.clone(),
                        converged_count,
                        failed_count,
                    },
                ))
                .await;
        }

        Ok(newly_delegated)
    }

    /// Called when a federated goal fails — propagate failure to dependents.
    pub async fn on_node_failed(
        &self,
        dag: &mut SwarmDag,
        node_id: Uuid,
        reason: &str,
    ) -> DomainResult<Vec<Uuid>> {
        // Mark the node as Failed.
        let node_label = {
            let node = dag.get_node_mut(node_id);
            if let Some(n) = node {
                n.state = SwarmDagNodeState::Failed;
                n.label.clone()
            } else {
                String::new()
            }
        };

        self.event_bus
            .publish(event_factory::federation_event(
                EventSeverity::Error,
                None,
                EventPayload::SwarmDagNodeFailed {
                    dag_id: dag.id,
                    node_id,
                    node_label: node_label.clone(),
                    reason: reason.to_string(),
                },
            ))
            .await;

        // Find all transitive dependents and fail them too.
        let transitive = dag.transitive_dependents(node_id);

        let mut failed_ids = Vec::new();
        for dep_id in &transitive {
            if let Some(dep_node) = dag.get_node_mut(*dep_id)
                && !dep_node.state.is_terminal() {
                    dep_node.state = SwarmDagNodeState::Failed;
                    failed_ids.push(*dep_id);
                }
        }

        // Emit failure events for cascaded nodes.
        for dep_id in &failed_ids {
            let label = dag
                .get_node(*dep_id)
                .map(|n| n.label.clone())
                .unwrap_or_default();
            self.event_bus
                .publish(event_factory::federation_event(
                    EventSeverity::Error,
                    None,
                    EventPayload::SwarmDagNodeFailed {
                        dag_id: dag.id,
                        node_id: *dep_id,
                        node_label: label,
                        reason: format!(
                            "Cascaded failure from node '{}': {}",
                            node_label, reason
                        ),
                    },
                ))
                .await;
        }

        // Check if DAG is now complete.
        if dag.is_complete() {
            let converged_count = dag
                .nodes
                .iter()
                .filter(|n| n.state == SwarmDagNodeState::Converged)
                .count();
            let failed_count = dag
                .nodes
                .iter()
                .filter(|n| n.state == SwarmDagNodeState::Failed)
                .count();

            self.event_bus
                .publish(event_factory::federation_event(
                    EventSeverity::Warning,
                    None,
                    EventPayload::SwarmDagCompleted {
                        dag_id: dag.id,
                        dag_name: dag.name.clone(),
                        converged_count,
                        failed_count,
                    },
                ))
                .await;
        }

        Ok(failed_ids)
    }

    /// Delegate a single node to its target cerebrate.
    async fn delegate_node(
        &self,
        dag: &mut SwarmDag,
        node_id: Uuid,
        goal: &Goal,
    ) -> DomainResult<Uuid> {
        let (cerebrate_id, contract, label) = {
            let node = dag.get_node(node_id).ok_or_else(|| {
                crate::domain::errors::DomainError::ValidationFailed(format!(
                    "Node {} not found in DAG",
                    node_id
                ))
            })?;
            (
                node.cerebrate_id.clone(),
                node.contract.clone(),
                node.label.clone(),
            )
        };

        let federated_goal = self
            .federation_service
            .delegate_goal(goal, &cerebrate_id, contract)
            .await
            .map_err(|e| {
                crate::domain::errors::DomainError::ValidationFailed(format!(
                    "Failed to delegate node '{}': {}",
                    label, e
                ))
            })?;

        let fed_goal_id = federated_goal.id;

        // Update node state.
        if let Some(node) = dag.get_node_mut(node_id) {
            node.state = SwarmDagNodeState::Delegated;
            node.federated_goal_id = Some(fed_goal_id);
        }

        // Emit delegation event.
        self.event_bus
            .publish(event_factory::federation_event(
                EventSeverity::Info,
                None,
                EventPayload::SwarmDagNodeDelegated {
                    dag_id: dag.id,
                    node_id,
                    node_label: label,
                    cerebrate_id,
                },
            ))
            .await;

        Ok(fed_goal_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::models::goal_federation::ConvergenceContract;

    /// Test DAG state machine transitions without requiring a real FederationService.
    /// These tests exercise the SwarmDag model directly.
    fn make_node(label: &str, cerebrate_id: &str, deps: Vec<Uuid>) -> SwarmDagNode {
        SwarmDagNode {
            id: Uuid::new_v4(),
            label: label.to_string(),
            cerebrate_id: cerebrate_id.to_string(),
            intent: format!("Do {}", label),
            contract: ConvergenceContract::default(),
            dependencies: deps,
            federated_goal_id: None,
            state: SwarmDagNodeState::Waiting,
        }
    }

    fn make_pipeline_dag() -> (SwarmDag, Uuid, Uuid, Uuid) {
        let mut dag = SwarmDag::new("pipeline");

        let code = make_node("code", "cerebrate-code", vec![]);
        let code_id = code.id;
        dag.add_node(code);

        let deploy = make_node("deploy", "cerebrate-deploy", vec![code_id]);
        let deploy_id = deploy.id;
        dag.add_node(deploy);

        let e2e = make_node("e2e", "cerebrate-e2e", vec![deploy_id]);
        let e2e_id = e2e.id;
        dag.add_node(e2e);

        (dag, code_id, deploy_id, e2e_id)
    }

    #[test]
    fn test_dag_initial_ready_nodes_are_roots() {
        let (dag, code_id, _, _) = make_pipeline_dag();
        let ready = dag.ready_nodes();
        assert_eq!(ready.len(), 1);
        assert_eq!(ready[0].id, code_id);
    }

    #[test]
    fn test_dag_state_machine_full_convergence() {
        let (mut dag, code_id, deploy_id, e2e_id) = make_pipeline_dag();

        // Initially only code is ready.
        assert_eq!(dag.ready_nodes().len(), 1);
        assert!(!dag.is_complete());

        // Simulate delegation of code.
        dag.get_node_mut(code_id).unwrap().state = SwarmDagNodeState::Delegated;
        assert_eq!(dag.ready_nodes().len(), 0);

        // Code converges.
        dag.get_node_mut(code_id).unwrap().state = SwarmDagNodeState::Converged;
        let ready = dag.ready_nodes();
        assert_eq!(ready.len(), 1);
        assert_eq!(ready[0].id, deploy_id);

        // Delegate and converge deploy.
        dag.get_node_mut(deploy_id).unwrap().state = SwarmDagNodeState::Delegated;
        dag.get_node_mut(deploy_id).unwrap().state = SwarmDagNodeState::Converged;
        let ready = dag.ready_nodes();
        assert_eq!(ready.len(), 1);
        assert_eq!(ready[0].id, e2e_id);

        // Delegate and converge e2e.
        dag.get_node_mut(e2e_id).unwrap().state = SwarmDagNodeState::Delegated;
        dag.get_node_mut(e2e_id).unwrap().state = SwarmDagNodeState::Converged;

        assert!(dag.is_complete());
        assert_eq!(dag.ready_nodes().len(), 0);
    }

    #[test]
    fn test_dag_state_machine_failure_cascade() {
        let (mut dag, code_id, deploy_id, e2e_id) = make_pipeline_dag();

        // Code fails.
        dag.get_node_mut(code_id).unwrap().state = SwarmDagNodeState::Failed;

        // Cascade failure to transitive dependents.
        let transitive = dag.transitive_dependents(code_id);
        assert_eq!(transitive.len(), 2);

        for dep_id in &transitive {
            dag.get_node_mut(*dep_id).unwrap().state = SwarmDagNodeState::Failed;
        }

        assert!(dag.is_complete());
        assert_eq!(dag.get_node(code_id).unwrap().state, SwarmDagNodeState::Failed);
        assert_eq!(dag.get_node(deploy_id).unwrap().state, SwarmDagNodeState::Failed);
        assert_eq!(dag.get_node(e2e_id).unwrap().state, SwarmDagNodeState::Failed);
    }

    #[test]
    fn test_dag_partial_failure() {
        // DAG with parallel branches:
        //   code -> deploy
        //   code -> test
        let mut dag = SwarmDag::new("branching");

        let code = make_node("code", "c1", vec![]);
        let code_id = code.id;
        dag.add_node(code);

        let deploy = make_node("deploy", "c2", vec![code_id]);
        let deploy_id = deploy.id;
        dag.add_node(deploy);

        let test = make_node("test", "c3", vec![code_id]);
        let test_id = test.id;
        dag.add_node(test);

        // Code converges.
        dag.get_node_mut(code_id).unwrap().state = SwarmDagNodeState::Converged;

        // Both deploy and test become ready.
        let ready = dag.ready_nodes();
        assert_eq!(ready.len(), 2);

        // Deploy fails, test converges.
        dag.get_node_mut(deploy_id).unwrap().state = SwarmDagNodeState::Failed;
        dag.get_node_mut(test_id).unwrap().state = SwarmDagNodeState::Converged;

        assert!(dag.is_complete());
    }

    #[test]
    fn test_dag_diamond_dependency() {
        // DAG diamond:
        //   A -> B -> D
        //   A -> C -> D
        let mut dag = SwarmDag::new("diamond");

        let a = make_node("a", "c1", vec![]);
        let a_id = a.id;
        dag.add_node(a);

        let b = make_node("b", "c2", vec![a_id]);
        let b_id = b.id;
        dag.add_node(b);

        let c = make_node("c", "c3", vec![a_id]);
        let c_id = c.id;
        dag.add_node(c);

        let d = make_node("d", "c4", vec![b_id, c_id]);
        let d_id = d.id;
        dag.add_node(d);

        assert!(dag.validate().is_ok());

        // A converges -> B and C become ready.
        dag.get_node_mut(a_id).unwrap().state = SwarmDagNodeState::Converged;
        let ready = dag.ready_nodes();
        assert_eq!(ready.len(), 2);

        // B converges but C hasn't yet -> D not ready.
        dag.get_node_mut(b_id).unwrap().state = SwarmDagNodeState::Converged;
        let ready = dag.ready_nodes();
        assert_eq!(ready.len(), 1); // only C
        assert_eq!(ready[0].id, c_id);

        // C converges -> D becomes ready.
        dag.get_node_mut(c_id).unwrap().state = SwarmDagNodeState::Converged;
        let ready = dag.ready_nodes();
        assert_eq!(ready.len(), 1);
        assert_eq!(ready[0].id, d_id);
    }
}
