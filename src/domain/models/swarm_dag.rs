//! Swarm DAG — cross-swarm dependency coordination.
//!
//! A SwarmDag expresses dependencies between federated goals across child swarms.
//! For example: "deploy waits for code, E2E waits for deploy."

use std::collections::{HashSet, VecDeque};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::goal_federation::ConvergenceContract;

/// State of a node in the swarm DAG.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SwarmDagNodeState {
    /// Dependencies not yet met.
    Waiting,
    /// Dependencies met, not yet delegated.
    Ready,
    /// Sent to child swarm.
    Delegated,
    /// Child swarm is actively working.
    Converging,
    /// Contract satisfied.
    Converged,
    /// Child failed.
    Failed,
}

impl SwarmDagNodeState {
    /// Return the string representation of this state.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Waiting => "waiting",
            Self::Ready => "ready",
            Self::Delegated => "delegated",
            Self::Converging => "converging",
            Self::Converged => "converged",
            Self::Failed => "failed",
        }
    }

    /// Returns true if this is a terminal state (no further transitions expected).
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Converged | Self::Failed)
    }
}

/// A node in the swarm DAG representing a federated goal to delegate.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SwarmDagNode {
    /// Unique node ID.
    pub id: Uuid,
    /// Human-readable label (e.g., "code", "deploy", "e2e").
    pub label: String,
    /// Target child swarm.
    pub cerebrate_id: String,
    /// What to tell the child swarm.
    pub intent: String,
    /// Convergence criteria.
    pub contract: ConvergenceContract,
    /// IDs of nodes this depends on.
    pub dependencies: Vec<Uuid>,
    /// Set once delegated.
    pub federated_goal_id: Option<Uuid>,
    /// Current state of this node.
    pub state: SwarmDagNodeState,
}

/// A DAG expressing dependencies between federated goals across child swarms.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SwarmDag {
    /// Unique DAG ID.
    pub id: Uuid,
    /// Human-readable name for this DAG.
    pub name: String,
    /// All nodes in this DAG.
    pub nodes: Vec<SwarmDagNode>,
    /// When this DAG was created.
    pub created_at: DateTime<Utc>,
}

impl SwarmDag {
    /// Create a new empty swarm DAG.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            nodes: Vec::new(),
            created_at: Utc::now(),
        }
    }

    /// Add a node to the DAG.
    pub fn add_node(&mut self, node: SwarmDagNode) {
        self.nodes.push(node);
    }

    /// Return root nodes (nodes with no dependencies).
    pub fn roots(&self) -> Vec<&SwarmDagNode> {
        self.nodes
            .iter()
            .filter(|n| n.dependencies.is_empty())
            .collect()
    }

    /// Return nodes that depend on the given node.
    pub fn dependents_of(&self, node_id: Uuid) -> Vec<&SwarmDagNode> {
        self.nodes
            .iter()
            .filter(|n| n.dependencies.contains(&node_id))
            .collect()
    }

    /// Return nodes whose dependencies are all `Converged` and whose state is `Waiting`.
    pub fn ready_nodes(&self) -> Vec<&SwarmDagNode> {
        self.nodes
            .iter()
            .filter(|n| {
                n.state == SwarmDagNodeState::Waiting
                    && n.dependencies.iter().all(|dep_id| {
                        self.get_node(*dep_id)
                            .map(|dep| dep.state == SwarmDagNodeState::Converged)
                            .unwrap_or(false)
                    })
            })
            .collect()
    }

    /// Returns true if all nodes are in a terminal state.
    pub fn is_complete(&self) -> bool {
        self.nodes.iter().all(|n| n.state.is_terminal())
    }

    /// Validate the DAG: check for cycles, missing dependencies, etc.
    pub fn validate(&self) -> Result<(), String> {
        let node_ids: HashSet<Uuid> = self.nodes.iter().map(|n| n.id).collect();

        // Check for missing dependencies.
        for node in &self.nodes {
            for dep_id in &node.dependencies {
                if !node_ids.contains(dep_id) {
                    return Err(format!(
                        "Node '{}' depends on missing node {}",
                        node.label, dep_id
                    ));
                }
            }
        }

        // Check for self-dependencies.
        for node in &self.nodes {
            if node.dependencies.contains(&node.id) {
                return Err(format!("Node '{}' depends on itself", node.label));
            }
        }

        // Check for cycles using DFS.
        if self.has_cycle() {
            return Err("Cycle detected in DAG".to_string());
        }

        Ok(())
    }

    /// Get a node by ID.
    pub fn get_node(&self, id: Uuid) -> Option<&SwarmDagNode> {
        self.nodes.iter().find(|n| n.id == id)
    }

    /// Get a mutable reference to a node by ID.
    pub fn get_node_mut(&mut self, id: Uuid) -> Option<&mut SwarmDagNode> {
        self.nodes.iter_mut().find(|n| n.id == id)
    }

    /// Check if the DAG contains a cycle using DFS.
    fn has_cycle(&self) -> bool {
        let mut visited = HashSet::new();
        let mut rec_stack = HashSet::new();

        for node in &self.nodes {
            if self.detect_cycle_dfs(node.id, &mut visited, &mut rec_stack) {
                return true;
            }
        }

        false
    }

    fn detect_cycle_dfs(
        &self,
        node_id: Uuid,
        visited: &mut HashSet<Uuid>,
        rec_stack: &mut HashSet<Uuid>,
    ) -> bool {
        if rec_stack.contains(&node_id) {
            return true;
        }
        if visited.contains(&node_id) {
            return false;
        }

        visited.insert(node_id);
        rec_stack.insert(node_id);

        // Follow dependents (nodes that depend on this node).
        for dependent in self.dependents_of(node_id) {
            if self.detect_cycle_dfs(dependent.id, visited, rec_stack) {
                return true;
            }
        }

        rec_stack.remove(&node_id);
        false
    }

    /// Get all transitive dependents of a node (BFS).
    pub fn transitive_dependents(&self, node_id: Uuid) -> Vec<Uuid> {
        let mut result = Vec::new();
        let mut visited = HashSet::new();
        let mut queue = VecDeque::new();

        for dep in self.dependents_of(node_id) {
            queue.push_back(dep.id);
        }

        while let Some(id) = queue.pop_front() {
            if visited.insert(id) {
                result.push(id);
                for dep in self.dependents_of(id) {
                    queue.push_back(dep.id);
                }
            }
        }

        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    fn make_three_node_dag() -> (SwarmDag, Uuid, Uuid, Uuid) {
        let mut dag = SwarmDag::new("test-pipeline");

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
    fn test_create_three_node_dag() {
        let (dag, _, _, _) = make_three_node_dag();
        assert_eq!(dag.nodes.len(), 3);
        assert_eq!(dag.name, "test-pipeline");
    }

    #[test]
    fn test_roots_returns_only_root_nodes() {
        let (dag, code_id, _, _) = make_three_node_dag();
        let roots = dag.roots();
        assert_eq!(roots.len(), 1);
        assert_eq!(roots[0].id, code_id);
    }

    #[test]
    fn test_ready_nodes_returns_roots_initially() {
        // Roots have no deps, so all deps are trivially Converged => they are ready.
        let (dag, code_id, _, _) = make_three_node_dag();
        let ready = dag.ready_nodes();
        assert_eq!(ready.len(), 1);
        assert_eq!(ready[0].id, code_id);
    }

    #[test]
    fn test_when_root_converges_deploy_becomes_ready() {
        let (mut dag, code_id, deploy_id, _) = make_three_node_dag();

        // Converge the root.
        dag.get_node_mut(code_id).unwrap().state = SwarmDagNodeState::Converged;

        let ready = dag.ready_nodes();
        assert_eq!(ready.len(), 1);
        assert_eq!(ready[0].id, deploy_id);
    }

    #[test]
    fn test_validate_catches_cycles() {
        let mut dag = SwarmDag::new("cyclic");

        let id_a = Uuid::new_v4();
        let id_b = Uuid::new_v4();
        let id_c = Uuid::new_v4();

        // a -> b -> c -> a (cycle)
        dag.add_node(SwarmDagNode {
            id: id_a,
            label: "a".to_string(),
            cerebrate_id: "c1".to_string(),
            intent: "do a".to_string(),
            contract: ConvergenceContract::default(),
            dependencies: vec![id_c],
            federated_goal_id: None,
            state: SwarmDagNodeState::Waiting,
        });
        dag.add_node(SwarmDagNode {
            id: id_b,
            label: "b".to_string(),
            cerebrate_id: "c1".to_string(),
            intent: "do b".to_string(),
            contract: ConvergenceContract::default(),
            dependencies: vec![id_a],
            federated_goal_id: None,
            state: SwarmDagNodeState::Waiting,
        });
        dag.add_node(SwarmDagNode {
            id: id_c,
            label: "c".to_string(),
            cerebrate_id: "c1".to_string(),
            intent: "do c".to_string(),
            contract: ConvergenceContract::default(),
            dependencies: vec![id_b],
            federated_goal_id: None,
            state: SwarmDagNodeState::Waiting,
        });

        let result = dag.validate();
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Cycle"));
    }

    #[test]
    fn test_validate_catches_missing_dependency() {
        let mut dag = SwarmDag::new("missing-dep");
        let missing_id = Uuid::new_v4();
        dag.add_node(make_node("lonely", "c1", vec![missing_id]));

        let result = dag.validate();
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("missing node"));
    }

    #[test]
    fn test_is_complete_when_all_terminal() {
        let (mut dag, code_id, deploy_id, e2e_id) = make_three_node_dag();

        assert!(!dag.is_complete());

        dag.get_node_mut(code_id).unwrap().state = SwarmDagNodeState::Converged;
        dag.get_node_mut(deploy_id).unwrap().state = SwarmDagNodeState::Converged;
        dag.get_node_mut(e2e_id).unwrap().state = SwarmDagNodeState::Converged;

        assert!(dag.is_complete());
    }

    #[test]
    fn test_is_complete_with_failures() {
        let (mut dag, code_id, deploy_id, e2e_id) = make_three_node_dag();

        dag.get_node_mut(code_id).unwrap().state = SwarmDagNodeState::Converged;
        dag.get_node_mut(deploy_id).unwrap().state = SwarmDagNodeState::Failed;
        dag.get_node_mut(e2e_id).unwrap().state = SwarmDagNodeState::Failed;

        assert!(dag.is_complete());
    }

    #[test]
    fn test_dependents_of() {
        let (dag, code_id, deploy_id, _) = make_three_node_dag();

        let dependents = dag.dependents_of(code_id);
        assert_eq!(dependents.len(), 1);
        assert_eq!(dependents[0].id, deploy_id);
    }

    #[test]
    fn test_transitive_dependents() {
        let (dag, code_id, deploy_id, e2e_id) = make_three_node_dag();

        let transitive = dag.transitive_dependents(code_id);
        assert_eq!(transitive.len(), 2);
        assert!(transitive.contains(&deploy_id));
        assert!(transitive.contains(&e2e_id));
    }

    #[test]
    fn test_node_state_as_str() {
        assert_eq!(SwarmDagNodeState::Waiting.as_str(), "waiting");
        assert_eq!(SwarmDagNodeState::Ready.as_str(), "ready");
        assert_eq!(SwarmDagNodeState::Delegated.as_str(), "delegated");
        assert_eq!(SwarmDagNodeState::Converging.as_str(), "converging");
        assert_eq!(SwarmDagNodeState::Converged.as_str(), "converged");
        assert_eq!(SwarmDagNodeState::Failed.as_str(), "failed");
    }

    #[test]
    fn test_node_state_is_terminal() {
        assert!(!SwarmDagNodeState::Waiting.is_terminal());
        assert!(!SwarmDagNodeState::Ready.is_terminal());
        assert!(!SwarmDagNodeState::Delegated.is_terminal());
        assert!(!SwarmDagNodeState::Converging.is_terminal());
        assert!(SwarmDagNodeState::Converged.is_terminal());
        assert!(SwarmDagNodeState::Failed.is_terminal());
    }

    #[test]
    fn test_validate_valid_dag() {
        let (dag, _, _, _) = make_three_node_dag();
        assert!(dag.validate().is_ok());
    }
}
