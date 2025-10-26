//! LoopExecutor - Iterative refinement loops with convergence detection
//!
//! Implements async orchestration for iterative task execution with:
//! - Multiple convergence strategies (fixed, adaptive, threshold)
//! - Checkpointing for crash recovery
//! - Graceful shutdown with cancellation tokens
//! - Timeout handling per iteration
//! - Structured error handling with context

use crate::domain::models::Task;
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::fs;
use tokio::select;
use tokio::sync::RwLock;
use tokio::sync::broadcast;
use tokio::time::{interval, timeout};
use tracing::{debug, info, warn};
use uuid::Uuid;

/// Convergence detection strategy for loop termination
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ConvergenceStrategy {
    /// Fixed number of iterations
    Fixed(u32),

    /// Adaptive threshold based on change rate
    /// Converges when relative change < threshold between iterations
    Adaptive(f64),

    /// Quality threshold
    /// Converges when quality metric >= threshold
    Threshold(f64),
}

impl ConvergenceStrategy {
    /// Check if loop has converged based on strategy
    fn is_converged(&self, state: &LoopState) -> bool {
        match self {
            ConvergenceStrategy::Fixed(max_iter) => state.iteration >= *max_iter,

            ConvergenceStrategy::Adaptive(threshold) => {
                if let Some(change_rate) = state.change_rate {
                    change_rate < *threshold
                } else {
                    false // No change rate yet, can't converge
                }
            }

            ConvergenceStrategy::Threshold(quality_threshold) => {
                if let Some(quality) = state.quality_metric {
                    quality >= *quality_threshold
                } else {
                    false // No quality metric yet, can't converge
                }
            }
        }
    }
}

/// Loop execution state (shared between iterations)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoopState {
    /// Current iteration number (0-based)
    pub iteration: u32,

    /// Result from last iteration
    pub last_result: Option<String>,

    /// Previous result (for change detection)
    pub previous_result: Option<String>,

    /// Whether loop has converged
    pub converged: bool,

    /// Change rate between last two iterations (0.0 = no change, 1.0 = complete change)
    pub change_rate: Option<f64>,

    /// Quality metric (0.0-1.0, higher is better)
    pub quality_metric: Option<f64>,

    /// Iteration history (result summaries)
    pub iteration_history: Vec<IterationResult>,

    /// Loop start time
    pub started_at: DateTime<Utc>,

    /// Last checkpoint time
    pub last_checkpoint_at: Option<DateTime<Utc>>,

    /// Loop ID for tracking
    pub loop_id: Uuid,
}

impl LoopState {
    /// Create new loop state
    pub fn new(loop_id: Uuid) -> Self {
        Self {
            iteration: 0,
            last_result: None,
            previous_result: None,
            converged: false,
            change_rate: None,
            quality_metric: None,
            iteration_history: Vec::new(),
            started_at: Utc::now(),
            last_checkpoint_at: None,
            loop_id,
        }
    }

    /// Update state after iteration
    fn update_iteration(&mut self, result: String, quality_metric: Option<f64>) -> Result<()> {
        // Calculate change rate if we have previous result
        if let Some(prev) = &self.last_result {
            self.change_rate = Some(calculate_change_rate(prev, &result));
        }

        // Update results
        self.previous_result = self.last_result.clone();
        self.last_result = Some(result.clone());
        self.quality_metric = quality_metric;

        // Add to history
        self.iteration_history.push(IterationResult {
            iteration: self.iteration,
            result_summary: result.chars().take(200).collect(),
            quality_metric,
            change_rate: self.change_rate,
            timestamp: Utc::now(),
        });

        // Increment iteration counter
        self.iteration += 1;

        Ok(())
    }
}

/// Result from a single iteration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IterationResult {
    pub iteration: u32,
    pub result_summary: String,
    pub quality_metric: Option<f64>,
    pub change_rate: Option<f64>,
    pub timestamp: DateTime<Utc>,
}

/// LoopExecutor configuration
#[derive(Debug, Clone)]
pub struct LoopExecutorConfig {
    /// Maximum iterations (safety limit)
    pub max_iterations: u32,

    /// Timeout per iteration (seconds)
    pub iteration_timeout_seconds: u64,

    /// Checkpoint interval (iterations)
    pub checkpoint_interval: u32,

    /// Checkpoint directory
    pub checkpoint_dir: PathBuf,
}

impl Default for LoopExecutorConfig {
    fn default() -> Self {
        Self {
            max_iterations: 100,
            iteration_timeout_seconds: 300, // 5 minutes
            checkpoint_interval: 5,
            checkpoint_dir: PathBuf::from(".abathur/checkpoints"),
        }
    }
}

/// LoopExecutor - orchestrates iterative refinement with convergence detection
///
/// Uses tokio async runtime for:
/// - Concurrent iteration execution
/// - Timeout handling per iteration
/// - Periodic checkpointing
/// - Graceful shutdown via broadcast channel
///
/// # Examples
///
/// ```no_run
/// use abathur::application::{LoopExecutor, ConvergenceStrategy};
/// use abathur::domain::models::Task;
///
/// #[tokio::main]
/// async fn main() -> anyhow::Result<()> {
///     let executor = LoopExecutor::new(
///         ConvergenceStrategy::Adaptive(0.05),
///         Default::default()
///     );
///
///     let task = Task::new("Iterative refinement".into(), "Refine output".into());
///     let result = executor.execute(task, |iter, _task| async move {
///         // Your iteration logic here
///         Ok(format!("Iteration {}", iter))
///     }).await?;
///
///     println!("Converged after {} iterations", result.iteration);
///     Ok(())
/// }
/// ```
pub struct LoopExecutor {
    convergence: ConvergenceStrategy,
    config: LoopExecutorConfig,
    state: Arc<RwLock<LoopState>>,
    shutdown_tx: broadcast::Sender<()>,
}

impl LoopExecutor {
    /// Create new LoopExecutor with convergence strategy
    pub fn new(convergence: ConvergenceStrategy, config: LoopExecutorConfig) -> Self {
        let (shutdown_tx, _) = broadcast::channel(1);

        Self {
            convergence,
            config,
            state: Arc::new(RwLock::new(LoopState::new(Uuid::new_v4()))),
            shutdown_tx,
        }
    }

    /// Execute iterative loop with convergence detection
    ///
    /// # Type Parameters
    /// - `F`: Async function that executes one iteration
    ///
    /// # Arguments
    /// - `task`: Task to execute iteratively
    /// - `iteration_fn`: Async function `Fn(u32, Task) -> Result<String>`
    ///
    /// # Returns
    /// Final loop state after convergence or max iterations
    pub async fn execute<F, Fut>(&self, task: Task, iteration_fn: F) -> Result<LoopState>
    where
        F: Fn(u32, Task) -> Fut + Send + Sync,
        Fut: std::future::Future<Output = Result<String>> + Send,
    {
        let loop_id = self.state.read().await.loop_id;
        info!(loop_id = %loop_id, "Starting loop execution with strategy: {:?}", self.convergence);

        // Create checkpoint directory
        fs::create_dir_all(&self.config.checkpoint_dir)
            .await
            .context("Failed to create checkpoint directory")?;

        // Try to recover from checkpoint
        if let Some(recovered_state) = self.try_recover_checkpoint().await? {
            info!(
                "Recovered from checkpoint at iteration {}",
                recovered_state.iteration
            );
            *self.state.write().await = recovered_state;
        }

        // Spawn background checkpointing task
        let checkpoint_handle = self.spawn_checkpointer();

        // Main loop execution
        let result = self.run_loop(task, iteration_fn).await;

        // Cancel checkpointer
        drop(checkpoint_handle);

        // Final checkpoint on completion
        self.save_checkpoint().await?;

        result
    }

    /// Run the main iteration loop
    async fn run_loop<F, Fut>(&self, task: Task, iteration_fn: F) -> Result<LoopState>
    where
        F: Fn(u32, Task) -> Fut + Send + Sync,
        Fut: std::future::Future<Output = Result<String>> + Send,
    {
        let mut shutdown_rx = self.shutdown_tx.subscribe();

        loop {
            let current_iteration = {
                let state = self.state.read().await;
                state.iteration
            };

            // Check max iterations safety limit
            if current_iteration >= self.config.max_iterations {
                warn!(
                    "Reached maximum iteration limit: {}",
                    self.config.max_iterations
                );
                break;
            }

            // Check convergence BEFORE iteration
            if self.check_convergence().await {
                info!("Loop converged at iteration {}", current_iteration);
                break;
            }

            debug!("Starting iteration {}", current_iteration);

            // Execute iteration with timeout and shutdown handling
            select! {
                // Normal iteration execution with timeout
                iteration_result = timeout(
                    Duration::from_secs(self.config.iteration_timeout_seconds),
                    iteration_fn(current_iteration, task.clone())
                ) => {
                    match iteration_result {
                        Ok(Ok(result)) => {
                            // Iteration succeeded
                            let quality = self.calculate_quality_metric(&result);

                            self.state.write().await
                                .update_iteration(result, quality)
                                .context("Failed to update iteration state")?;

                            debug!("Iteration {} completed successfully", current_iteration);
                        }
                        Ok(Err(e)) => {
                            // Iteration failed
                            warn!("Iteration {} failed: {:#}", current_iteration, e);
                            return Err(e).context(format!("Iteration {} failed", current_iteration));
                        }
                        Err(_) => {
                            // Timeout
                            warn!("Iteration {} timed out after {}s",
                                current_iteration,
                                self.config.iteration_timeout_seconds
                            );
                            return Err(anyhow::anyhow!(
                                "Iteration {} timed out after {}s",
                                current_iteration,
                                self.config.iteration_timeout_seconds
                            ));
                        }
                    }
                }

                // Shutdown signal received
                _ = shutdown_rx.recv() => {
                    info!("Shutdown signal received, stopping loop execution");
                    break;
                }
            }
        }

        // Mark as converged and return final state
        let mut state = self.state.write().await;
        state.converged = true;
        Ok(state.clone())
    }

    /// Check if loop has converged based on strategy
    async fn check_convergence(&self) -> bool {
        let state = self.state.read().await;
        self.convergence.is_converged(&state)
    }

    /// Calculate quality metric for iteration result
    ///
    /// This is a placeholder - actual implementation would use domain-specific metrics
    fn calculate_quality_metric(&self, _result: &str) -> Option<f64> {
        // TODO: Implement domain-specific quality metric
        // For now, return None (quality not measured)
        None
    }

    /// Spawn background checkpointing task
    fn spawn_checkpointer(&self) -> tokio::task::JoinHandle<()> {
        let state = Arc::clone(&self.state);
        let checkpoint_dir = self.config.checkpoint_dir.clone();
        let checkpoint_interval = self.config.checkpoint_interval;
        let mut shutdown_rx = self.shutdown_tx.subscribe();

        tokio::spawn(async move {
            let mut checkpoint_ticker = interval(Duration::from_secs(30)); // Every 30s

            loop {
                select! {
                    _ = checkpoint_ticker.tick() => {
                        let state_guard = state.read().await;

                        // Only checkpoint at interval boundaries
                        if state_guard.iteration % checkpoint_interval == 0 && state_guard.iteration > 0 {
                            let loop_id = state_guard.loop_id;
                            let checkpoint_path = checkpoint_dir.join(format!("{}.json", loop_id));

                            match serde_json::to_string_pretty(&*state_guard) {
                                Ok(json) => {
                                    if let Err(e) = fs::write(&checkpoint_path, json).await {
                                        warn!("Failed to write checkpoint: {:#}", e);
                                    } else {
                                        debug!("Checkpoint saved at iteration {}", state_guard.iteration);
                                    }
                                }
                                Err(e) => {
                                    warn!("Failed to serialize checkpoint: {:#}", e);
                                }
                            }
                        }
                    }

                    _ = shutdown_rx.recv() => {
                        debug!("Checkpointer shutting down");
                        break;
                    }
                }
            }
        })
    }

    /// Save checkpoint to disk
    async fn save_checkpoint(&self) -> Result<()> {
        let state = self.state.read().await;
        let checkpoint_path = self
            .config
            .checkpoint_dir
            .join(format!("{}.json", state.loop_id));

        let json =
            serde_json::to_string_pretty(&*state).context("Failed to serialize checkpoint")?;

        fs::write(&checkpoint_path, json)
            .await
            .context("Failed to write checkpoint file")?;

        debug!("Final checkpoint saved to {:?}", checkpoint_path);
        Ok(())
    }

    /// Try to recover from most recent checkpoint
    async fn try_recover_checkpoint(&self) -> Result<Option<LoopState>> {
        // Find most recent checkpoint file
        let mut entries = match fs::read_dir(&self.config.checkpoint_dir).await {
            Ok(e) => e,
            Err(_) => return Ok(None), // No checkpoint dir, no recovery
        };

        let mut latest_checkpoint: Option<(DateTime<Utc>, PathBuf)> = None;

        while let Ok(Some(entry)) = entries.next_entry().await {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) == Some("json")
                && let Ok(metadata) = entry.metadata().await
                && let Ok(modified) = metadata.modified()
            {
                let modified_dt: DateTime<Utc> = modified.into();

<<<<<<< HEAD
                if latest_checkpoint
                    .as_ref()
                    .is_none_or(|(dt, _)| modified_dt > *dt)
                {
                    latest_checkpoint = Some((modified_dt, path));
=======
                        if latest_checkpoint
                            .as_ref()
                            .is_none_or(|(dt, _)| modified_dt > *dt)
                        {
                            latest_checkpoint = Some((modified_dt, path));
                        }
                    }
>>>>>>> task_claude-api-request-response-types_20251025-205946
                }
            }
        }

        if let Some((_, checkpoint_path)) = latest_checkpoint {
            info!("Found checkpoint at {:?}", checkpoint_path);

            let json = fs::read_to_string(&checkpoint_path)
                .await
                .context("Failed to read checkpoint file")?;

            let state: LoopState =
                serde_json::from_str(&json).context("Failed to deserialize checkpoint")?;

            return Ok(Some(state));
        }

        Ok(None)
    }

    /// Trigger graceful shutdown
    pub async fn shutdown(&self) {
        info!("Triggering loop executor shutdown");
        let _ = self.shutdown_tx.send(());
    }

    /// Get current loop state (read-only)
    pub async fn get_state(&self) -> LoopState {
        self.state.read().await.clone()
    }
}

/// Calculate change rate between two results (0.0 = identical, 1.0 = completely different)
///
/// Uses simple character-level diff ratio as a proxy for semantic change.
/// In production, you might use:
/// - Levenshtein distance
/// - Semantic embeddings (cosine similarity)
/// - Domain-specific metrics
fn calculate_change_rate(previous: &str, current: &str) -> f64 {
    if previous == current {
        return 0.0;
    }

    // Simple character-level diff (placeholder)
    let max_len = previous.len().max(current.len()) as f64;
    let common_prefix_len = previous
        .chars()
        .zip(current.chars())
        .take_while(|(a, b)| a == b)
        .count() as f64;

    1.0 - (common_prefix_len / max_len)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_convergence_strategy_fixed() {
        let strategy = ConvergenceStrategy::Fixed(10);
        let mut state = LoopState::new(Uuid::new_v4());

        assert!(!strategy.is_converged(&state));

        state.iteration = 10;
        assert!(strategy.is_converged(&state));
    }

    #[test]
    fn test_convergence_strategy_adaptive() {
        let strategy = ConvergenceStrategy::Adaptive(0.05);
        let mut state = LoopState::new(Uuid::new_v4());

        // No change rate yet
        assert!(!strategy.is_converged(&state));

        // High change rate
        state.change_rate = Some(0.8);
        assert!(!strategy.is_converged(&state));

        // Low change rate (converged)
        state.change_rate = Some(0.02);
        assert!(strategy.is_converged(&state));
    }

    #[test]
    fn test_convergence_strategy_threshold() {
        let strategy = ConvergenceStrategy::Threshold(0.9);
        let mut state = LoopState::new(Uuid::new_v4());

        // No quality metric yet
        assert!(!strategy.is_converged(&state));

        // Low quality
        state.quality_metric = Some(0.5);
        assert!(!strategy.is_converged(&state));

        // High quality (converged)
        state.quality_metric = Some(0.95);
        assert!(strategy.is_converged(&state));
    }

    #[test]
    fn test_calculate_change_rate() {
        assert!((calculate_change_rate("hello", "hello") - 0.0).abs() < f64::EPSILON);
        assert!(calculate_change_rate("hello", "world") > 0.5);
        assert!(calculate_change_rate("", "something") > 0.9);
    }

    #[tokio::test]
    async fn test_loop_state_update() {
        let mut state = LoopState::new(Uuid::new_v4());

        state
            .update_iteration("First result".into(), Some(0.5))
            .unwrap();
        assert_eq!(state.iteration, 1);
        assert_eq!(state.last_result, Some("First result".into()));
        assert_eq!(state.quality_metric, Some(0.5));

        state
            .update_iteration("Second result".into(), Some(0.8))
            .unwrap();
        assert_eq!(state.iteration, 2);
        assert_eq!(state.previous_result, Some("First result".into()));
        assert!(state.change_rate.is_some());
    }

    #[tokio::test]
    async fn test_loop_executor_fixed_convergence() {
        let temp_dir = TempDir::new().unwrap();
        let config = LoopExecutorConfig {
            max_iterations: 100,
            iteration_timeout_seconds: 5,
            checkpoint_interval: 2,
            checkpoint_dir: temp_dir.path().to_path_buf(),
        };

        let executor = LoopExecutor::new(ConvergenceStrategy::Fixed(5), config);

        let task = Task::new("Test task".into(), "Test description".into());

        let result = executor
            .execute(task, |iter, _task| async move {
<<<<<<< HEAD
                Ok(format!("Iteration {iter}"))
=======
                Ok(format!("Iteration {}", iter))
>>>>>>> task_claude-api-request-response-types_20251025-205946
            })
            .await
            .unwrap();

        assert_eq!(result.iteration, 5);
        assert!(result.converged);
        assert_eq!(result.iteration_history.len(), 5);
    }

    #[tokio::test]
    async fn test_loop_executor_timeout() {
        let temp_dir = TempDir::new().unwrap();
        let config = LoopExecutorConfig {
            max_iterations: 10,
            iteration_timeout_seconds: 1, // 1 second timeout
            checkpoint_interval: 5,
            checkpoint_dir: temp_dir.path().to_path_buf(),
        };

        let executor = LoopExecutor::new(ConvergenceStrategy::Fixed(5), config);

        let task = Task::new("Test task".into(), "Test description".into());

        let result = executor
            .execute(task, |_iter, _task| async move {
                tokio::time::sleep(Duration::from_secs(2)).await; // Exceeds timeout
                Ok("Result".into())
            })
            .await;

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("timed out"));
    }

    #[tokio::test]
    async fn test_loop_executor_graceful_shutdown() {
        let temp_dir = TempDir::new().unwrap();
        let config = LoopExecutorConfig {
            max_iterations: 100,
            iteration_timeout_seconds: 10,
            checkpoint_interval: 5,
            checkpoint_dir: temp_dir.path().to_path_buf(),
        };

        let executor = Arc::new(LoopExecutor::new(
            ConvergenceStrategy::Fixed(100), // Would take a long time
            config,
        ));

        let executor_clone = Arc::clone(&executor);
        let task = Task::new("Test task".into(), "Test description".into());

        // Spawn loop execution
        let exec_handle = tokio::spawn(async move {
            executor_clone
                .execute(task, |iter, _task| async move {
                    tokio::time::sleep(Duration::from_millis(100)).await;
<<<<<<< HEAD
                    Ok(format!("Iteration {iter}"))
=======
                    Ok(format!("Iteration {}", iter))
>>>>>>> task_claude-api-request-response-types_20251025-205946
                })
                .await
        });

        // Let it run a few iterations
        tokio::time::sleep(Duration::from_millis(300)).await;

        // Trigger shutdown
        executor.shutdown().await;

        // Should complete gracefully
        let result = exec_handle.await.unwrap().unwrap();
        assert!(result.iteration < 100); // Stopped early
        assert!(result.converged); // Marked as converged despite early stop
    }

    #[tokio::test]
    async fn test_checkpoint_save_and_recover() {
        let temp_dir = TempDir::new().unwrap();
        let config = LoopExecutorConfig {
            max_iterations: 10,
            iteration_timeout_seconds: 5,
            checkpoint_interval: 2,
            checkpoint_dir: temp_dir.path().to_path_buf(),
        };

        let executor = LoopExecutor::new(ConvergenceStrategy::Fixed(5), config.clone());

        let task = Task::new("Test task".into(), "Test description".into());

        // Run to completion
        let result = executor
            .execute(task, |iter, _task| async move {
<<<<<<< HEAD
                Ok(format!("Iteration {iter}"))
=======
                Ok(format!("Iteration {}", iter))
>>>>>>> task_claude-api-request-response-types_20251025-205946
            })
            .await
            .unwrap();

        assert_eq!(result.iteration, 5);

        // Create new executor with same config (simulating restart)
        let executor2 = LoopExecutor::new(ConvergenceStrategy::Fixed(10), config);

        // Should recover checkpoint
        let recovered = executor2.try_recover_checkpoint().await.unwrap();
        assert!(recovered.is_some());

        let recovered_state = recovered.unwrap();
        assert_eq!(recovered_state.iteration, 5);
        assert_eq!(recovered_state.loop_id, result.loop_id);
    }
}
