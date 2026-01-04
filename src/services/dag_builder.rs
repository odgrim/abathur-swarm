//! DAG Builder Service
//!
//! Service for constructing directed acyclic graphs (DAGs) representing task execution
//! dependencies. Provides validation to ensure graph integrity and prevent cycles.

use anyhow::{anyhow, Context, Result};
use std::collections::{HashMap, HashSet};
use tracing::{instrument, warn};
use uuid::Uuid;

use crate::domain::models::{DAGEdge, DAGNode, DependencyType};

/// Service for building and managing execution DAGs
///
/// The DAGBuilder maintains a graph structure representing task dependencies,
/// ensuring no cycles are created and all edges reference valid nodes.
///
/// # Examples
///
/// ```
/// use uuid::Uuid;
/// use crate::services::DAGBuilder;
/// use crate::domain::models::DependencyType;
///
/// let mut builder = DAGBuilder::new();
/// let task_a = Uuid::new_v4();
/// let task_b = Uuid::new_v4();
///
/// builder.add_node(task_a)?;
/// builder.add_node(task_b)?;
/// builder.add_edge(task_a, task_b, DependencyType::Sequential)?;
/// ```
#[derive(Debug, Clone)]
pub struct DAGBuilder {
    /// Map of task IDs to their DAG nodes
    nodes: HashMap<Uuid, DAGNode>,
    /// List of all edges in the graph
    edges: Vec<DAGEdge>,
    /// Adjacency list for efficient dependency lookups (task_id -> list of tasks it depends on)
    adjacency_list: HashMap<Uuid, Vec<Uuid>>,
}

impl DAGBuilder {
    /// Create a new empty DAG builder
    pub fn new() -> Self {
        Self {
            nodes: HashMap::new(),
            edges: Vec::new(),
            adjacency_list: HashMap::new(),
        }
    }

    /// Add a node to the DAG
    ///
    /// # Arguments
    ///
    /// * `task_id` - The UUID of the task to add as a node
    ///
    /// # Returns
    ///
    /// * `Ok(())` if the node was added successfully
    /// * `Err` if the node already exists
    ///
    /// # Examples
    ///
    /// ```
    /// let mut builder = DAGBuilder::new();
    /// let task_id = Uuid::new_v4();
    /// builder.add_node(task_id)?;
    /// ```
    #[instrument(skip(self), fields(task_id = %task_id))]
    pub fn add_node(&mut self, task_id: Uuid) -> Result<()> {
        if self.nodes.contains_key(&task_id) {
            return Err(anyhow!("Node {} already exists in DAG", task_id));
        }

        self.nodes.insert(task_id, DAGNode::new(task_id));
        self.adjacency_list.insert(task_id, Vec::new());

        Ok(())
    }

    /// Add an edge to the DAG representing a dependency relationship
    ///
    /// # Arguments
    ///
    /// * `from` - The source task ID (the dependency)
    /// * `to` - The target task ID (the dependent task)
    /// * `edge_type` - The type of dependency relationship
    ///
    /// # Returns
    ///
    /// * `Ok(())` if the edge was added successfully
    /// * `Err` if either node doesn't exist, the edge already exists, or adding it would create a cycle
    ///
    /// # Examples
    ///
    /// ```
    /// let mut builder = DAGBuilder::new();
    /// builder.add_node(task_a)?;
    /// builder.add_node(task_b)?;
    /// builder.add_edge(task_a, task_b, DependencyType::Sequential)?;
    /// ```
    #[instrument(skip(self), fields(from = %from, to = %to, edge_type = ?edge_type))]
    pub fn add_edge(&mut self, from: Uuid, to: Uuid, edge_type: DependencyType) -> Result<()> {
        // Validate that both nodes exist
        if !self.nodes.contains_key(&from) {
            return Err(anyhow!(
                "Source node {} does not exist in DAG. Add the node before creating edges.",
                from
            ));
        }

        if !self.nodes.contains_key(&to) {
            return Err(anyhow!(
                "Target node {} does not exist in DAG. Add the node before creating edges.",
                to
            ));
        }

        // Check for duplicate edges
        if self.edge_exists(from, to) {
            warn!(
                "Edge from {} to {} already exists, skipping duplicate",
                from, to
            );
            return Err(anyhow!("Edge from {} to {} already exists", from, to));
        }

        // Check if adding this edge would create a cycle
        if self.would_create_cycle(from, to)? {
            return Err(anyhow!(
                "Adding edge from {} to {} would create a cycle in the DAG",
                from,
                to
            ));
        }

        // Add the edge
        let edge = DAGEdge::new(from, to, edge_type);
        self.edges.push(edge);

        // Update adjacency list: 'to' depends on 'from'
        self.adjacency_list
            .entry(to)
            .or_insert_with(Vec::new)
            .push(from);

        Ok(())
    }

    /// Get a node by its task ID
    ///
    /// # Arguments
    ///
    /// * `id` - The task ID to look up
    ///
    /// # Returns
    ///
    /// * `Some(&DAGNode)` if the node exists
    /// * `None` if the node doesn't exist
    pub fn get_node(&self, id: Uuid) -> Option<&DAGNode> {
        self.nodes.get(&id)
    }

    /// Get all dependencies for a given task (tasks it depends on)
    ///
    /// # Arguments
    ///
    /// * `id` - The task ID to get dependencies for
    ///
    /// # Returns
    ///
    /// A vector of task IDs that the given task depends on
    pub fn get_dependencies(&self, id: Uuid) -> Vec<Uuid> {
        self.adjacency_list
            .get(&id)
            .cloned()
            .unwrap_or_default()
    }

    /// Get all dependents for a given task (tasks that depend on it)
    ///
    /// # Arguments
    ///
    /// * `id` - The task ID to get dependents for
    ///
    /// # Returns
    ///
    /// A vector of task IDs that depend on the given task
    pub fn get_dependents(&self, id: Uuid) -> Vec<Uuid> {
        self.adjacency_list
            .iter()
            .filter_map(|(task_id, deps)| {
                if deps.contains(&id) {
                    Some(*task_id)
                } else {
                    None
                }
            })
            .collect()
    }

    /// Check if an edge already exists between two nodes
    fn edge_exists(&self, from: Uuid, to: Uuid) -> bool {
        self.edges.iter().any(|e| e.from == from && e.to == to)
    }

    /// Check if adding an edge would create a cycle using DFS
    ///
    /// This performs a depth-first search from the 'to' node to see if we can reach
    /// the 'from' node. If we can, then adding an edge from 'from' to 'to' would
    /// create a cycle.
    fn would_create_cycle(&self, from: Uuid, to: Uuid) -> Result<bool> {
        // If 'to' can already reach 'from' via existing edges, adding from->to creates a cycle
        let mut visited = HashSet::new();
        let mut stack = vec![to];

        while let Some(current) = stack.pop() {
            if current == from {
                return Ok(true); // Cycle detected
            }

            if visited.contains(&current) {
                continue;
            }

            visited.insert(current);

            // Get all tasks that 'current' depends on (outgoing edges from current's perspective)
            if let Some(dependencies) = self.adjacency_list.get(&current) {
                for &dep in dependencies {
                    if !visited.contains(&dep) {
                        stack.push(dep);
                    }
                }
            }
        }

        Ok(false) // No cycle
    }
}

impl Default for DAGBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_builder() {
        let builder = DAGBuilder::new();
        assert_eq!(builder.nodes.len(), 0);
        assert_eq!(builder.edges.len(), 0);
        assert_eq!(builder.adjacency_list.len(), 0);
    }

    #[test]
    fn test_add_node() {
        let mut builder = DAGBuilder::new();
        let task_id = Uuid::new_v4();

        let result = builder.add_node(task_id);
        assert!(result.is_ok());
        assert_eq!(builder.nodes.len(), 1);
        assert!(builder.nodes.contains_key(&task_id));
    }

    #[test]
    fn test_add_duplicate_node() {
        let mut builder = DAGBuilder::new();
        let task_id = Uuid::new_v4();

        builder.add_node(task_id).unwrap();
        let result = builder.add_node(task_id);

        assert!(result.is_err());
        assert_eq!(builder.nodes.len(), 1);
    }

    #[test]
    fn test_add_edge() {
        let mut builder = DAGBuilder::new();
        let task_a = Uuid::new_v4();
        let task_b = Uuid::new_v4();

        builder.add_node(task_a).unwrap();
        builder.add_node(task_b).unwrap();

        let result = builder.add_edge(task_a, task_b, DependencyType::Sequential);
        assert!(result.is_ok());
        assert_eq!(builder.edges.len(), 1);
    }

    #[test]
    fn test_add_edge_missing_source_node() {
        let mut builder = DAGBuilder::new();
        let task_a = Uuid::new_v4();
        let task_b = Uuid::new_v4();

        builder.add_node(task_b).unwrap();

        let result = builder.add_edge(task_a, task_b, DependencyType::Sequential);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Source node"));
    }

    #[test]
    fn test_add_edge_missing_target_node() {
        let mut builder = DAGBuilder::new();
        let task_a = Uuid::new_v4();
        let task_b = Uuid::new_v4();

        builder.add_node(task_a).unwrap();

        let result = builder.add_edge(task_a, task_b, DependencyType::Sequential);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Target node"));
    }

    #[test]
    fn test_add_duplicate_edge() {
        let mut builder = DAGBuilder::new();
        let task_a = Uuid::new_v4();
        let task_b = Uuid::new_v4();

        builder.add_node(task_a).unwrap();
        builder.add_node(task_b).unwrap();

        builder
            .add_edge(task_a, task_b, DependencyType::Sequential)
            .unwrap();
        let result = builder.add_edge(task_a, task_b, DependencyType::Parallel);

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("already exists"));
        assert_eq!(builder.edges.len(), 1);
    }

    #[test]
    fn test_cycle_detection() {
        let mut builder = DAGBuilder::new();
        let task_a = Uuid::new_v4();
        let task_b = Uuid::new_v4();
        let task_c = Uuid::new_v4();

        builder.add_node(task_a).unwrap();
        builder.add_node(task_b).unwrap();
        builder.add_node(task_c).unwrap();

        // Create chain: A -> B -> C
        builder
            .add_edge(task_a, task_b, DependencyType::Sequential)
            .unwrap();
        builder
            .add_edge(task_b, task_c, DependencyType::Sequential)
            .unwrap();

        // Try to create cycle: C -> A (which would create A -> B -> C -> A)
        let result = builder.add_edge(task_c, task_a, DependencyType::Sequential);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("would create a cycle"));
    }

    #[test]
    fn test_get_node() {
        let mut builder = DAGBuilder::new();
        let task_id = Uuid::new_v4();

        builder.add_node(task_id).unwrap();

        let node = builder.get_node(task_id);
        assert!(node.is_some());
        assert_eq!(node.unwrap().task_id, task_id);

        let missing = builder.get_node(Uuid::new_v4());
        assert!(missing.is_none());
    }

    #[test]
    fn test_get_dependencies() {
        let mut builder = DAGBuilder::new();
        let task_a = Uuid::new_v4();
        let task_b = Uuid::new_v4();
        let task_c = Uuid::new_v4();

        builder.add_node(task_a).unwrap();
        builder.add_node(task_b).unwrap();
        builder.add_node(task_c).unwrap();

        // C depends on both A and B
        builder
            .add_edge(task_a, task_c, DependencyType::Sequential)
            .unwrap();
        builder
            .add_edge(task_b, task_c, DependencyType::Parallel)
            .unwrap();

        let deps = builder.get_dependencies(task_c);
        assert_eq!(deps.len(), 2);
        assert!(deps.contains(&task_a));
        assert!(deps.contains(&task_b));

        let no_deps = builder.get_dependencies(task_a);
        assert_eq!(no_deps.len(), 0);
    }

    #[test]
    fn test_get_dependents() {
        let mut builder = DAGBuilder::new();
        let task_a = Uuid::new_v4();
        let task_b = Uuid::new_v4();
        let task_c = Uuid::new_v4();

        builder.add_node(task_a).unwrap();
        builder.add_node(task_b).unwrap();
        builder.add_node(task_c).unwrap();

        // Both B and C depend on A
        builder
            .add_edge(task_a, task_b, DependencyType::Sequential)
            .unwrap();
        builder
            .add_edge(task_a, task_c, DependencyType::Sequential)
            .unwrap();

        let dependents = builder.get_dependents(task_a);
        assert_eq!(dependents.len(), 2);
        assert!(dependents.contains(&task_b));
        assert!(dependents.contains(&task_c));

        let no_dependents = builder.get_dependents(task_b);
        assert_eq!(no_dependents.len(), 0);
    }

    #[test]
    fn test_complex_dag() {
        let mut builder = DAGBuilder::new();
        let task_a = Uuid::new_v4();
        let task_b = Uuid::new_v4();
        let task_c = Uuid::new_v4();
        let task_d = Uuid::new_v4();

        builder.add_node(task_a).unwrap();
        builder.add_node(task_b).unwrap();
        builder.add_node(task_c).unwrap();
        builder.add_node(task_d).unwrap();

        // Create diamond pattern: A -> B -> D, A -> C -> D
        builder
            .add_edge(task_a, task_b, DependencyType::Sequential)
            .unwrap();
        builder
            .add_edge(task_a, task_c, DependencyType::Sequential)
            .unwrap();
        builder
            .add_edge(task_b, task_d, DependencyType::Parallel)
            .unwrap();
        builder
            .add_edge(task_c, task_d, DependencyType::Parallel)
            .unwrap();

        // Verify structure
        assert_eq!(builder.get_dependencies(task_d).len(), 2);
        assert_eq!(builder.get_dependents(task_a).len(), 2);

        // Verify no cycles can be added
        let cycle_result = builder.add_edge(task_d, task_a, DependencyType::Sequential);
        assert!(cycle_result.is_err());
    }
}
