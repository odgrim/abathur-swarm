//! Trajectory repository port for convergence system persistence.

use std::collections::HashMap;

use async_trait::async_trait;

use crate::domain::errors::DomainResult;
use crate::domain::models::task::Complexity;
use crate::domain::models::{AttractorType, StrategyEntry, StrategyKind, Trajectory};

// ---------------------------------------------------------------------------
// StrategyStats
// ---------------------------------------------------------------------------

/// Aggregated effectiveness statistics for a single strategy kind.
///
/// Computed by the repository from historical trajectory data. Used by the
/// convergence engine's reporting and bandit priming logic to understand
/// how well a strategy performs across the entire trajectory corpus.
#[derive(Debug, Clone)]
pub struct StrategyStats {
    /// The strategy kind name (e.g., `"retry_with_feedback"`).
    pub strategy: String,
    /// Total number of times this strategy was used across all trajectories.
    pub total_uses: u64,
    /// Number of uses that achieved a positive convergence delta.
    pub success_count: u64,
    /// Average convergence delta across all uses (including negative deltas).
    pub average_delta: f64,
    /// Average token cost per use.
    pub average_tokens: u64,
}

/// Repository interface for Trajectory persistence.
///
/// Provides storage and retrieval of convergence trajectories, including
/// queries by task, goal, recency, and strategy effectiveness. Abstracts
/// the underlying storage backend (SQLite, Postgres, in-memory, etc.).
#[async_trait]
pub trait TrajectoryRepository: Send + Sync {
    /// Persist or upsert a trajectory.
    ///
    /// If a trajectory with the same ID already exists, it is replaced.
    async fn save(&self, trajectory: &Trajectory) -> DomainResult<()>;

    /// Load a trajectory by its ID.
    async fn get(&self, trajectory_id: &str) -> DomainResult<Option<Trajectory>>;

    /// Get all trajectories associated with a task.
    async fn get_by_task(&self, task_id: &str) -> DomainResult<Vec<Trajectory>>;

    /// Get all trajectories associated with a goal.
    async fn get_by_goal(&self, goal_id: &str) -> DomainResult<Vec<Trajectory>>;

    /// Get the most recent trajectories, ordered by `updated_at` descending.
    async fn get_recent(&self, limit: usize) -> DomainResult<Vec<Trajectory>>;

    /// Get successful strategy entries for a given attractor type.
    ///
    /// Used for convergence memory and learning: retrieves strategies that
    /// led to positive convergence outcomes when facing a particular attractor
    /// classification, enabling the bandit to prime its distributions for
    /// similar future trajectories.
    async fn get_successful_strategies(
        &self,
        attractor_type: &AttractorType,
        limit: usize,
    ) -> DomainResult<Vec<StrategyEntry>>;

    /// Delete a trajectory by its ID.
    async fn delete(&self, trajectory_id: &str) -> DomainResult<()>;

    // -------------------------------------------------------------------
    // Convergence analytics queries (spec 10.2)
    // -------------------------------------------------------------------

    /// Compute the average number of iterations for completed trajectories
    /// whose budget was allocated at the given complexity level.
    ///
    /// Only considers trajectories in terminal phases (`converged` or
    /// `exhausted`). The iteration count is derived from the length of the
    /// observations array. Returns `0.0` if no matching trajectories exist.
    async fn avg_iterations_by_complexity(&self, complexity: Complexity) -> DomainResult<f64>;

    /// Compute aggregated effectiveness statistics for a strategy kind
    /// across all trajectories.
    ///
    /// Scans strategy log entries matching the requested `StrategyKind`,
    /// computing total uses, success count, average convergence delta, and
    /// average token cost.
    async fn strategy_effectiveness(&self, strategy: StrategyKind) -> DomainResult<StrategyStats>;

    /// Count the number of trajectories grouped by attractor classification.
    ///
    /// Returns a map from each `AttractorType` variant name to the number of
    /// trajectories whose most recent attractor state matches that type. The
    /// map keys use the top-level serde tag (e.g., `"fixed_point"`,
    /// `"limit_cycle"`).
    async fn attractor_distribution(&self) -> DomainResult<HashMap<String, u32>>;

    /// Compute the convergence rate for trajectories matching a task category.
    ///
    /// The category is matched against the specification content (a
    /// case-insensitive substring search on `specification_json`). The rate
    /// is the fraction of matching trajectories in the `converged` phase
    /// versus total matching trajectories in any terminal phase (`converged`,
    /// `exhausted`, `trapped`). Returns `0.0` if no matching trajectories
    /// exist.
    async fn convergence_rate_by_task_type(&self, category: &str) -> DomainResult<f64>;

    /// Find trajectories with similar specifications.
    ///
    /// Performs text-based similarity matching on the specification content
    /// and optional tag keywords. Results are ordered by recency (most
    /// recently updated first) and limited to `limit` entries.
    async fn get_similar_trajectories(
        &self,
        description: &str,
        tags: &[String],
        limit: usize,
    ) -> DomainResult<Vec<Trajectory>>;
}
