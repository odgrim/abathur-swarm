//! Directed Acyclic Graph (DAG) domain models
//!
//! Models for representing task execution dependencies as a directed acyclic graph.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::DependencyType;

/// A node in the execution DAG representing a task
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DAGNode {
    /// The task ID this node represents
    pub task_id: Uuid,
}

impl DAGNode {
    /// Create a new DAG node for a task
    pub fn new(task_id: Uuid) -> Self {
        Self { task_id }
    }
}

/// An edge in the execution DAG representing a dependency relationship
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DAGEdge {
    /// The source node (the dependency)
    pub from: Uuid,
    /// The target node (the dependent task)
    pub to: Uuid,
    /// The type of dependency (Sequential or Parallel)
    pub edge_type: DependencyType,
}

impl DAGEdge {
    /// Create a new DAG edge
    pub fn new(from: Uuid, to: Uuid, edge_type: DependencyType) -> Self {
        Self {
            from,
            to,
            edge_type,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dag_node_creation() {
        let task_id = Uuid::new_v4();
        let node = DAGNode::new(task_id);
        assert_eq!(node.task_id, task_id);
    }

    #[test]
    fn test_dag_edge_creation() {
        let from = Uuid::new_v4();
        let to = Uuid::new_v4();
        let edge = DAGEdge::new(from, to, DependencyType::Sequential);

        assert_eq!(edge.from, from);
        assert_eq!(edge.to, to);
        assert_eq!(edge.edge_type, DependencyType::Sequential);
    }
}
