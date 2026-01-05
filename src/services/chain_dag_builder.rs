//! Chain DAG Builder Service
//!
//! Service for building directed acyclic graphs (DAGs) from prompt chain steps,
//! computing execution levels for parallel execution, and validating dependency structures.
//!
//! This service coordinates the following operations:
//! - Building DAG structures from chain step dependencies
//! - Computing topological execution levels using Kahn's algorithm
//! - Detecting cycles in dependency graphs
//! - Validating dependency references

use anyhow::{anyhow, Context, Result};
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use tracing::{instrument, warn};

use crate::domain::models::dag_execution::{ChainDAG, ChainStepNode, StepExecutionLevel};
use crate::domain::models::PromptChain;
use crate::domain::ports::StepDependencyResolverPort;

/// Service for building DAGs from prompt chain steps
///
/// The ChainDAGBuilder uses dependency resolution to construct a directed acyclic
/// graph representing step execution order, then computes execution levels for
/// parallel execution planning.
///
/// # Architecture
///
/// This service follows Clean Architecture principles:
/// - Depends on `StepDependencyResolverPort` trait (port) for dependency resolution
/// - Uses domain models (`ChainDAG`, `PromptChain`) for data structures
/// - Implements pure business logic for DAG construction and level computation
///
/// # Examples
///
/// ```no_run
/// use std::sync::Arc;
/// use abathur::services::ChainDAGBuilder;
/// use abathur::domain::models::PromptChain;
///
/// async fn build_dag_example(
///     chain: &PromptChain,
///     builder: Arc<ChainDAGBuilder>
/// ) -> anyhow::Result<()> {
///     let dag = builder.build_dag(chain).await?;
///     println!("DAG has {} levels", dag.execution_levels.len());
///     Ok(())
/// }
/// ```
pub struct ChainDAGBuilder {
    /// Dependency resolver for extracting step dependencies from chain
    dependency_resolver: Arc<dyn StepDependencyResolverPort>,
}

impl ChainDAGBuilder {
    /// Create a new ChainDAGBuilder with dependency injection
    ///
    /// # Arguments
    ///
    /// * `dependency_resolver` - Port implementation for resolving step dependencies
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use std::sync::Arc;
    /// use abathur::services::ChainDAGBuilder;
    /// use abathur::infrastructure::DefaultStepDependencyResolver;
    ///
    /// let resolver = Arc::new(DefaultStepDependencyResolver::new());
    /// let builder = ChainDAGBuilder::new(resolver);
    /// ```
    pub fn new(dependency_resolver: Arc<dyn StepDependencyResolverPort>) -> Self {
        Self {
            dependency_resolver,
        }
    }

    /// Build a DAG from a prompt chain
    ///
    /// This method coordinates the DAG construction process:
    /// 1. Resolves dependencies using the injected resolver
    /// 2. Builds nodes HashMap from chain steps
    /// 3. Builds adjacency_list from dependency map
    /// 4. Detects cycles in the graph
    /// 5. Computes execution levels using topological sort
    ///
    /// # Arguments
    ///
    /// * `chain` - The prompt chain to build a DAG from
    ///
    /// # Returns
    ///
    /// A `ChainDAG` with computed execution levels
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - Dependency resolution fails
    /// - Cycles are detected in the dependency graph
    /// - Level computation fails
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use abathur::services::ChainDAGBuilder;
    /// # use abathur::domain::models::PromptChain;
    /// # async fn example(builder: ChainDAGBuilder, chain: PromptChain) -> anyhow::Result<()> {
    /// let dag = builder.build_dag(&chain).await?;
    /// assert_eq!(dag.node_count(), chain.steps.len());
    /// # Ok(())
    /// # }
    /// ```
    #[instrument(skip(self, chain), fields(chain_id = %chain.id, step_count = chain.steps.len()))]
    pub async fn build_dag(&self, chain: &PromptChain) -> Result<ChainDAG> {
        // 1. Use dependency_resolver to get all step dependencies
        let dependency_map = self
            .dependency_resolver
            .resolve_dependencies(chain)
            .await
            .context("Failed to resolve step dependencies")?;

        // 2. Build nodes HashMap from steps
        let nodes: HashMap<String, ChainStepNode> = chain
            .steps
            .iter()
            .map(|step| (step.id.clone(), ChainStepNode::new(step.id.clone())))
            .collect();

        // 3. Build adjacency_list from dependencies
        let adjacency_list = dependency_map;

        // 4. Validate for cycles using DFS
        self.detect_cycles(&adjacency_list)
            .context("Cycle detection failed")?;

        // 5. Compute execution levels
        let execution_levels = self
            .compute_execution_levels(&adjacency_list)
            .context("Failed to compute execution levels")?;

        // Create DAG with computed levels
        Ok(ChainDAG::with_levels(
            nodes,
            adjacency_list,
            execution_levels,
        ))
    }

    /// Compute execution levels using Kahn's algorithm (topological sort)
    ///
    /// This method implements a level-based topological sort:
    /// - Level 0: Steps with no dependencies
    /// - Level N: Steps whose dependencies are all in levels < N
    ///
    /// # Algorithm
    ///
    /// Uses Kahn's algorithm variant that tracks levels:
    /// 1. Calculate in-degree for each node (number of dependencies)
    /// 2. Add all nodes with in-degree 0 to level 0
    /// 3. Process each level in order:
    ///    - For each node in current level, decrement in-degree of dependents
    ///    - Add nodes that now have in-degree 0 to next level
    /// 4. Continue until all nodes are assigned to levels
    ///
    /// # Arguments
    ///
    /// * `adjacency_list` - Map of step_id -> Vec<dependency_step_ids>
    ///
    /// # Returns
    ///
    /// A vector of `StepExecutionLevel` sorted by level number
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - Not all nodes can be assigned to a level (indicates a cycle)
    /// - The adjacency list is invalid
    ///
    /// # Business Logic
    ///
    /// This is a pure business logic method with no I/O operations.
    /// It implements the algorithm for determining which steps can execute
    /// in parallel based on their dependencies.
    #[instrument(skip(self, adjacency_list), fields(node_count = adjacency_list.len()))]
    fn compute_execution_levels(
        &self,
        adjacency_list: &HashMap<String, Vec<String>>,
    ) -> Result<Vec<StepExecutionLevel>> {
        if adjacency_list.is_empty() {
            return Ok(Vec::new());
        }

        // Calculate in-degree for each node (number of dependencies)
        let mut in_degree: HashMap<String, usize> = adjacency_list
            .keys()
            .map(|step_id| (step_id.clone(), 0))
            .collect();

        for dependencies in adjacency_list.values() {
            for dep in dependencies {
                *in_degree.get_mut(dep).unwrap_or(&mut 0) += 0; // Dep is a prerequisite, not dependent
            }
        }

        // Count incoming edges for each node
        for (step_id, dependencies) in adjacency_list {
            *in_degree.get_mut(step_id).unwrap() = dependencies.len();
        }

        // Track which level each node belongs to
        let mut node_levels: HashMap<String, usize> = HashMap::new();
        let mut queue: VecDeque<String> = VecDeque::new();

        // Initialize queue with nodes that have no dependencies (level 0)
        for (step_id, &degree) in &in_degree {
            if degree == 0 {
                queue.push_back(step_id.clone());
                node_levels.insert(step_id.clone(), 0);
            }
        }

        // Process nodes level by level
        let mut processed_count = 0;
        while !queue.is_empty() {
            let current = queue.pop_front().unwrap();
            processed_count += 1;

            let current_level = *node_levels.get(&current).unwrap();

            // Find all nodes that depend on the current node
            for (step_id, dependencies) in adjacency_list {
                if dependencies.contains(&current) {
                    // Decrement in-degree (we've processed one of its dependencies)
                    let degree = in_degree.get_mut(step_id).unwrap();
                    *degree -= 1;

                    // If all dependencies are satisfied, assign to next level
                    if *degree == 0 {
                        let next_level = current_level + 1;
                        node_levels.insert(step_id.clone(), next_level);
                        queue.push_back(step_id.clone());
                    }
                }
            }
        }

        // Verify all nodes were processed (if not, there's a cycle)
        if processed_count != adjacency_list.len() {
            return Err(anyhow!(
                "Cannot compute execution levels: graph contains a cycle. Processed {} of {} nodes",
                processed_count,
                adjacency_list.len()
            ));
        }

        // Group nodes by level
        let mut levels_map: HashMap<usize, Vec<String>> = HashMap::new();
        for (step_id, level) in node_levels {
            levels_map
                .entry(level)
                .or_insert_with(Vec::new)
                .push(step_id);
        }

        // Convert to sorted vector of StepExecutionLevel
        let mut levels: Vec<StepExecutionLevel> = levels_map
            .into_iter()
            .map(|(level, step_ids)| StepExecutionLevel::new(level, step_ids))
            .collect();

        levels.sort_by_key(|l| l.level);

        Ok(levels)
    }

    /// Detect cycles in the dependency graph using DFS
    ///
    /// This method implements depth-first search cycle detection by tracking
    /// nodes in three states:
    /// - Unvisited (not in any set)
    /// - Currently visiting (in recursion stack)
    /// - Fully visited (in visited set)
    ///
    /// A cycle exists if we encounter a node that's currently in the recursion stack.
    ///
    /// # Algorithm
    ///
    /// For each unvisited node:
    /// 1. Mark it as currently visiting (add to stack)
    /// 2. Recursively visit all nodes it depends on
    /// 3. If we encounter a node in the stack, cycle detected
    /// 4. Mark as fully visited when done
    ///
    /// # Arguments
    ///
    /// * `adjacency_list` - Map of step_id -> Vec<dependency_step_ids>
    ///
    /// # Returns
    ///
    /// - `Ok(())` if no cycles are detected
    /// - `Err` if a cycle is found
    ///
    /// # Errors
    ///
    /// Returns an error with details about the cycle if one is detected
    ///
    /// # Business Logic
    ///
    /// This is a pure validation method with no I/O. It ensures the DAG
    /// constraint is maintained - cycles would cause infinite loops in execution.
    #[instrument(skip(self, adjacency_list), fields(node_count = adjacency_list.len()))]
    fn detect_cycles(&self, adjacency_list: &HashMap<String, Vec<String>>) -> Result<()> {
        let mut visited: HashSet<String> = HashSet::new();
        let mut rec_stack: HashSet<String> = HashSet::new();

        // Helper function for DFS
        fn dfs_visit(
            node: &str,
            adjacency_list: &HashMap<String, Vec<String>>,
            visited: &mut HashSet<String>,
            rec_stack: &mut HashSet<String>,
        ) -> Result<()> {
            // Mark current node as being visited
            visited.insert(node.to_string());
            rec_stack.insert(node.to_string());

            // Visit all dependencies
            if let Some(dependencies) = adjacency_list.get(node) {
                for dep in dependencies {
                    if !visited.contains(dep) {
                        // Recursively visit unvisited dependency
                        dfs_visit(dep, adjacency_list, visited, rec_stack)?;
                    } else if rec_stack.contains(dep) {
                        // Found a back edge - cycle detected
                        return Err(anyhow!(
                            "Cycle detected: step '{}' depends on '{}' which creates a circular dependency",
                            node,
                            dep
                        ));
                    }
                }
            }

            // Remove from recursion stack (backtrack)
            rec_stack.remove(node);
            Ok(())
        }

        // Run DFS from each unvisited node
        for node in adjacency_list.keys() {
            if !visited.contains(node) {
                dfs_visit(node, adjacency_list, &mut visited, &mut rec_stack)
                    .context(format!("Cycle detection starting from step '{}'", node))?;
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::models::{OutputFormat, PromptStep};
    use async_trait::async_trait;
    use std::collections::HashMap;

    // Mock dependency resolver for testing
    struct MockDependencyResolver {
        dependencies: HashMap<String, Vec<String>>,
    }

    impl MockDependencyResolver {
        fn new(dependencies: HashMap<String, Vec<String>>) -> Self {
            Self { dependencies }
        }
    }

    #[async_trait]
    impl StepDependencyResolverPort for MockDependencyResolver {
        async fn resolve_dependencies(
            &self,
            _chain: &PromptChain,
        ) -> Result<HashMap<String, Vec<String>>> {
            Ok(self.dependencies.clone())
        }

        async fn validate_dependencies(&self, _chain: &PromptChain) -> Result<()> {
            Ok(())
        }
    }

    fn create_test_chain(steps: Vec<(&str, Option<Vec<&str>>)>) -> PromptChain {
        let mut chain = PromptChain::new("test_chain".to_string(), "Test chain".to_string());

        for (step_id, depends_on) in steps {
            let mut step = PromptStep::new(
                step_id.to_string(),
                format!("Prompt for {}", step_id),
                "test_role".to_string(),
                OutputFormat::Plain,
            );
            step.depends_on = depends_on.map(|deps| deps.iter().map(|s| s.to_string()).collect());
            chain.add_step(step);
        }

        chain
    }

    #[tokio::test]
    async fn test_simple_linear_chain() {
        // Chain: A -> B -> C (sequential)
        let chain = create_test_chain(vec![
            ("A", Some(vec![])),       // No dependencies
            ("B", Some(vec!["A"])),    // Depends on A
            ("C", Some(vec!["B"])),    // Depends on B
        ]);

        let dependencies = HashMap::from([
            ("A".to_string(), vec![]),
            ("B".to_string(), vec!["A".to_string()]),
            ("C".to_string(), vec!["B".to_string()]),
        ]);

        let resolver = Arc::new(MockDependencyResolver::new(dependencies));
        let builder = ChainDAGBuilder::new(resolver);

        let dag = builder.build_dag(&chain).await.unwrap();

        // Verify DAG structure
        assert_eq!(dag.node_count(), 3);
        assert_eq!(dag.total_levels(), 3);

        // Verify execution levels
        assert_eq!(dag.execution_levels[0].level, 0);
        assert_eq!(dag.execution_levels[0].step_ids, vec!["A"]);

        assert_eq!(dag.execution_levels[1].level, 1);
        assert_eq!(dag.execution_levels[1].step_ids, vec!["B"]);

        assert_eq!(dag.execution_levels[2].level, 2);
        assert_eq!(dag.execution_levels[2].step_ids, vec!["C"]);
    }

    #[tokio::test]
    async fn test_diamond_pattern() {
        // Diamond: A -> B, A -> C, B -> D, C -> D
        let chain = create_test_chain(vec![
            ("A", Some(vec![])),
            ("B", Some(vec!["A"])),
            ("C", Some(vec!["A"])),
            ("D", Some(vec!["B", "C"])),
        ]);

        let dependencies = HashMap::from([
            ("A".to_string(), vec![]),
            ("B".to_string(), vec!["A".to_string()]),
            ("C".to_string(), vec!["A".to_string()]),
            ("D".to_string(), vec!["B".to_string(), "C".to_string()]),
        ]);

        let resolver = Arc::new(MockDependencyResolver::new(dependencies));
        let builder = ChainDAGBuilder::new(resolver);

        let dag = builder.build_dag(&chain).await.unwrap();

        // Verify levels
        assert_eq!(dag.total_levels(), 3);

        // Level 0: A
        assert_eq!(dag.execution_levels[0].step_ids.len(), 1);
        assert!(dag.execution_levels[0].contains_step("A"));

        // Level 1: B and C (parallel)
        assert_eq!(dag.execution_levels[1].step_ids.len(), 2);
        assert!(dag.execution_levels[1].contains_step("B"));
        assert!(dag.execution_levels[1].contains_step("C"));

        // Level 2: D
        assert_eq!(dag.execution_levels[2].step_ids.len(), 1);
        assert!(dag.execution_levels[2].contains_step("D"));
    }

    #[tokio::test]
    async fn test_fan_out_pattern() {
        // Fan-out: A -> B, A -> C, A -> D
        let chain = create_test_chain(vec![
            ("A", Some(vec![])),
            ("B", Some(vec!["A"])),
            ("C", Some(vec!["A"])),
            ("D", Some(vec!["A"])),
        ]);

        let dependencies = HashMap::from([
            ("A".to_string(), vec![]),
            ("B".to_string(), vec!["A".to_string()]),
            ("C".to_string(), vec!["A".to_string()]),
            ("D".to_string(), vec!["A".to_string()]),
        ]);

        let resolver = Arc::new(MockDependencyResolver::new(dependencies));
        let builder = ChainDAGBuilder::new(resolver);

        let dag = builder.build_dag(&chain).await.unwrap();

        // Verify levels
        assert_eq!(dag.total_levels(), 2);

        // Level 0: A
        assert_eq!(dag.execution_levels[0].step_ids.len(), 1);
        assert!(dag.execution_levels[0].contains_step("A"));

        // Level 1: B, C, D (all parallel)
        assert_eq!(dag.execution_levels[1].step_ids.len(), 3);
        assert!(dag.execution_levels[1].contains_step("B"));
        assert!(dag.execution_levels[1].contains_step("C"));
        assert!(dag.execution_levels[1].contains_step("D"));
    }

    #[tokio::test]
    async fn test_cycle_detection() {
        // Cycle: A -> B -> C -> A
        let chain = create_test_chain(vec![
            ("A", Some(vec!["C"])),    // A depends on C (creates cycle)
            ("B", Some(vec!["A"])),
            ("C", Some(vec!["B"])),
        ]);

        let dependencies = HashMap::from([
            ("A".to_string(), vec!["C".to_string()]),
            ("B".to_string(), vec!["A".to_string()]),
            ("C".to_string(), vec!["B".to_string()]),
        ]);

        let resolver = Arc::new(MockDependencyResolver::new(dependencies));
        let builder = ChainDAGBuilder::new(resolver);

        let result = builder.build_dag(&chain).await;

        // Should fail due to cycle
        assert!(result.is_err());
        let error_msg = result.unwrap_err().to_string();
        assert!(error_msg.contains("Cycle detected") || error_msg.contains("cycle"));
    }

    #[tokio::test]
    async fn test_level_computation_complex() {
        // Complex graph:
        // Level 0: A, B (no dependencies)
        // Level 1: C (depends on A), D (depends on B)
        // Level 2: E (depends on C and D)
        let chain = create_test_chain(vec![
            ("A", Some(vec![])),
            ("B", Some(vec![])),
            ("C", Some(vec!["A"])),
            ("D", Some(vec!["B"])),
            ("E", Some(vec!["C", "D"])),
        ]);

        let dependencies = HashMap::from([
            ("A".to_string(), vec![]),
            ("B".to_string(), vec![]),
            ("C".to_string(), vec!["A".to_string()]),
            ("D".to_string(), vec!["B".to_string()]),
            ("E".to_string(), vec!["C".to_string(), "D".to_string()]),
        ]);

        let resolver = Arc::new(MockDependencyResolver::new(dependencies));
        let builder = ChainDAGBuilder::new(resolver);

        let dag = builder.build_dag(&chain).await.unwrap();

        // Verify levels
        assert_eq!(dag.total_levels(), 3);

        // Level 0: A and B
        assert_eq!(dag.execution_levels[0].step_ids.len(), 2);
        assert!(dag.execution_levels[0].contains_step("A"));
        assert!(dag.execution_levels[0].contains_step("B"));

        // Level 1: C and D
        assert_eq!(dag.execution_levels[1].step_ids.len(), 2);
        assert!(dag.execution_levels[1].contains_step("C"));
        assert!(dag.execution_levels[1].contains_step("D"));

        // Level 2: E
        assert_eq!(dag.execution_levels[2].step_ids.len(), 1);
        assert!(dag.execution_levels[2].contains_step("E"));
    }

    #[tokio::test]
    async fn test_empty_chain() {
        let chain = PromptChain::new("empty".to_string(), "Empty chain".to_string());
        let dependencies = HashMap::new();

        let resolver = Arc::new(MockDependencyResolver::new(dependencies));
        let builder = ChainDAGBuilder::new(resolver);

        let dag = builder.build_dag(&chain).await.unwrap();

        assert_eq!(dag.node_count(), 0);
        assert_eq!(dag.total_levels(), 0);
    }

    #[tokio::test]
    async fn test_single_step() {
        let chain = create_test_chain(vec![("A", Some(vec![]))]);
        let dependencies = HashMap::from([("A".to_string(), vec![])]);

        let resolver = Arc::new(MockDependencyResolver::new(dependencies));
        let builder = ChainDAGBuilder::new(resolver);

        let dag = builder.build_dag(&chain).await.unwrap();

        assert_eq!(dag.node_count(), 1);
        assert_eq!(dag.total_levels(), 1);
        assert_eq!(dag.execution_levels[0].step_ids, vec!["A"]);
    }

    #[test]
    fn test_detect_cycles_direct() {
        let resolver = Arc::new(MockDependencyResolver::new(HashMap::new()));
        let builder = ChainDAGBuilder::new(resolver);

        // Simple cycle: A -> B -> A
        let adjacency_list = HashMap::from([
            ("A".to_string(), vec!["B".to_string()]),
            ("B".to_string(), vec!["A".to_string()]),
        ]);

        let result = builder.detect_cycles(&adjacency_list);
        assert!(result.is_err());
    }

    #[test]
    fn test_detect_cycles_no_cycle() {
        let resolver = Arc::new(MockDependencyResolver::new(HashMap::new()));
        let builder = ChainDAGBuilder::new(resolver);

        // No cycle: A -> B -> C
        let adjacency_list = HashMap::from([
            ("A".to_string(), vec![]),
            ("B".to_string(), vec!["A".to_string()]),
            ("C".to_string(), vec!["B".to_string()]),
        ]);

        let result = builder.detect_cycles(&adjacency_list);
        assert!(result.is_ok());
    }
}
