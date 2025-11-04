use crate::domain::models::prompt_chain::{ChainExecution, PromptChain};
use anyhow::Result;
use async_trait::async_trait;

/// Repository interface for prompt chain storage operations
///
/// Provides CRUD operations for chains and their executions.
/// Implementations should handle database-specific details while maintaining
/// the interface contract.
#[async_trait]
pub trait ChainRepository: Send + Sync {
    /// Insert a new prompt chain
    ///
    /// # Arguments
    /// * `chain` - The chain to insert
    ///
    /// # Returns
    /// * `Ok(())` - If insertion succeeds
    /// * `Err(_)` - If insertion fails
    async fn insert_chain(&self, chain: &PromptChain) -> Result<()>;

    /// Get a chain by ID
    ///
    /// # Arguments
    /// * `chain_id` - The chain identifier
    ///
    /// # Returns
    /// * `Ok(Some(PromptChain))` - The chain if found
    /// * `Ok(None)` - If not found
    /// * `Err(_)` - If query fails
    async fn get_chain(&self, chain_id: &str) -> Result<Option<PromptChain>>;

    /// Get a chain by name
    ///
    /// # Arguments
    /// * `name` - The chain name
    ///
    /// # Returns
    /// * `Ok(Some(PromptChain))` - The chain if found
    /// * `Ok(None)` - If not found
    /// * `Err(_)` - If query fails
    async fn get_chain_by_name(&self, name: &str) -> Result<Option<PromptChain>>;

    /// List all chains
    ///
    /// # Returns
    /// * `Ok(Vec<PromptChain>)` - List of all chains
    /// * `Err(_)` - If query fails
    async fn list_chains(&self) -> Result<Vec<PromptChain>>;

    /// Update a chain
    ///
    /// # Arguments
    /// * `chain` - The updated chain
    ///
    /// # Returns
    /// * `Ok(())` - If update succeeds
    /// * `Err(_)` - If update fails
    async fn update_chain(&self, chain: &PromptChain) -> Result<()>;

    /// Delete a chain
    ///
    /// # Arguments
    /// * `chain_id` - The chain identifier
    ///
    /// # Returns
    /// * `Ok(())` - If deletion succeeds
    /// * `Err(_)` - If deletion fails
    async fn delete_chain(&self, chain_id: &str) -> Result<()>;

    /// Insert a new chain execution record
    ///
    /// # Arguments
    /// * `execution` - The execution to insert
    ///
    /// # Returns
    /// * `Ok(())` - If insertion succeeds
    /// * `Err(_)` - If insertion fails
    async fn insert_execution(&self, execution: &ChainExecution) -> Result<()>;

    /// Get an execution by ID
    ///
    /// # Arguments
    /// * `execution_id` - The execution identifier
    ///
    /// # Returns
    /// * `Ok(Some(ChainExecution))` - The execution if found
    /// * `Ok(None)` - If not found
    /// * `Err(_)` - If query fails
    async fn get_execution(&self, execution_id: &str) -> Result<Option<ChainExecution>>;

    /// List executions for a chain
    ///
    /// # Arguments
    /// * `chain_id` - The chain identifier
    /// * `limit` - Maximum number of results
    ///
    /// # Returns
    /// * `Ok(Vec<ChainExecution>)` - List of executions
    /// * `Err(_)` - If query fails
    async fn list_executions_for_chain(
        &self,
        chain_id: &str,
        limit: usize,
    ) -> Result<Vec<ChainExecution>>;

    /// List executions for a task
    ///
    /// # Arguments
    /// * `task_id` - The task identifier
    ///
    /// # Returns
    /// * `Ok(Vec<ChainExecution>)` - List of executions
    /// * `Err(_)` - If query fails
    async fn list_executions_for_task(&self, task_id: &str) -> Result<Vec<ChainExecution>>;

    /// Update an execution
    ///
    /// # Arguments
    /// * `execution` - The updated execution
    ///
    /// # Returns
    /// * `Ok(())` - If update succeeds
    /// * `Err(_)` - If update fails
    async fn update_execution(&self, execution: &ChainExecution) -> Result<()>;

    /// Get execution statistics for a chain
    ///
    /// # Arguments
    /// * `chain_id` - The chain identifier
    ///
    /// # Returns
    /// * `Ok(ChainStats)` - Statistics about executions
    /// * `Err(_)` - If query fails
    async fn get_chain_stats(&self, chain_id: &str) -> Result<ChainStats>;
}

/// Statistics about chain executions
#[derive(Debug, Clone)]
pub struct ChainStats {
    pub total_executions: usize,
    pub completed: usize,
    pub failed: usize,
    pub validation_failed: usize,
    pub running: usize,
    pub avg_duration_secs: Option<f64>,
}
