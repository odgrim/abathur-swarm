//! SQLite implementation of the TrajectoryRepository.

use std::collections::HashMap;

use async_trait::async_trait;
use sqlx::SqlitePool;

use crate::domain::errors::{DomainError, DomainResult};
use crate::domain::models::task::Complexity;
use crate::domain::models::{
    AttractorState, AttractorType, ContextHealth, ConvergenceBudget, ConvergencePhase,
    ConvergencePolicy, Observation, SpecificationEvolution, StrategyEntry, StrategyKind, Trajectory,
};
use crate::domain::ports::{StrategyStats, TrajectoryRepository};

#[derive(Clone)]
pub struct SqliteTrajectoryRepository {
    pool: SqlitePool,
}

impl SqliteTrajectoryRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

/// Derive a stable string key from an `AttractorType` variant, matching the
/// convention used by the strategy bandit.
fn attractor_type_name(attractor: &AttractorType) -> &'static str {
    match attractor {
        AttractorType::FixedPoint { .. } => "fixed_point",
        AttractorType::LimitCycle { .. } => "limit_cycle",
        AttractorType::Divergent { .. } => "divergent",
        AttractorType::Plateau { .. } => "plateau",
        AttractorType::Indeterminate { .. } => "indeterminate",
    }
}

#[async_trait]
impl TrajectoryRepository for SqliteTrajectoryRepository {
    async fn save(&self, trajectory: &Trajectory) -> DomainResult<()> {
        let id = trajectory.id.to_string();
        let task_id = trajectory.task_id.to_string();
        let goal_id = trajectory.goal_id.map(|g| g.to_string());
        let total_fresh_starts = trajectory.total_fresh_starts as i32;

        let specification_json = serde_json::to_string(&trajectory.specification)?;
        let observations_json = serde_json::to_string(&trajectory.observations)?;
        let attractor_state_json = serde_json::to_string(&trajectory.attractor_state)?;
        let budget_json = serde_json::to_string(&trajectory.budget)?;
        let policy_json = serde_json::to_string(&trajectory.policy)?;
        let strategy_log_json = serde_json::to_string(&trajectory.strategy_log)?;
        let context_health_json = serde_json::to_string(&trajectory.context_health)?;
        let hints_json = serde_json::to_string(&trajectory.hints)?;
        let forced_strategy_json = trajectory
            .forced_strategy
            .as_ref()
            .map(|s| serde_json::to_string(s))
            .transpose()?;

        let created_at = trajectory.created_at.to_rfc3339();
        let updated_at = trajectory.updated_at.to_rfc3339();

        // Store the full phase JSON so Coordinating's children are preserved.
        let phase_json = serde_json::to_string(&trajectory.phase)?;

        sqlx::query(
            r#"INSERT INTO convergence_trajectories (
                id, task_id, goal_id, phase, total_fresh_starts,
                specification_json, observations_json, attractor_state_json,
                budget_json, policy_json, strategy_log_json, context_health_json,
                hints_json, forced_strategy_json, created_at, updated_at
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            ON CONFLICT(id) DO UPDATE SET
                task_id = excluded.task_id,
                goal_id = excluded.goal_id,
                phase = excluded.phase,
                total_fresh_starts = excluded.total_fresh_starts,
                specification_json = excluded.specification_json,
                observations_json = excluded.observations_json,
                attractor_state_json = excluded.attractor_state_json,
                budget_json = excluded.budget_json,
                policy_json = excluded.policy_json,
                strategy_log_json = excluded.strategy_log_json,
                context_health_json = excluded.context_health_json,
                hints_json = excluded.hints_json,
                forced_strategy_json = excluded.forced_strategy_json,
                created_at = excluded.created_at,
                updated_at = excluded.updated_at"#,
        )
        .bind(&id)
        .bind(&task_id)
        .bind(&goal_id)
        .bind(&phase_json)
        .bind(total_fresh_starts)
        .bind(&specification_json)
        .bind(&observations_json)
        .bind(&attractor_state_json)
        .bind(&budget_json)
        .bind(&policy_json)
        .bind(&strategy_log_json)
        .bind(&context_health_json)
        .bind(&hints_json)
        .bind(&forced_strategy_json)
        .bind(&created_at)
        .bind(&updated_at)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    async fn get(&self, trajectory_id: &str) -> DomainResult<Option<Trajectory>> {
        let row: Option<TrajectoryRow> =
            sqlx::query_as("SELECT * FROM convergence_trajectories WHERE id = ?")
                .bind(trajectory_id)
                .fetch_optional(&self.pool)
                .await?;

        row.map(|r| r.try_into()).transpose()
    }

    async fn get_by_task(&self, task_id: &str) -> DomainResult<Vec<Trajectory>> {
        let rows: Vec<TrajectoryRow> = sqlx::query_as(
            "SELECT * FROM convergence_trajectories WHERE task_id = ? ORDER BY updated_at DESC",
        )
        .bind(task_id)
        .fetch_all(&self.pool)
        .await?;

        rows.into_iter().map(|r| r.try_into()).collect()
    }

    async fn get_by_goal(&self, goal_id: &str) -> DomainResult<Vec<Trajectory>> {
        let rows: Vec<TrajectoryRow> = sqlx::query_as(
            "SELECT * FROM convergence_trajectories WHERE goal_id = ? ORDER BY updated_at DESC",
        )
        .bind(goal_id)
        .fetch_all(&self.pool)
        .await?;

        rows.into_iter().map(|r| r.try_into()).collect()
    }

    async fn get_recent(&self, limit: usize) -> DomainResult<Vec<Trajectory>> {
        let rows: Vec<TrajectoryRow> = sqlx::query_as(
            "SELECT * FROM convergence_trajectories ORDER BY updated_at DESC LIMIT ?",
        )
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await?;

        rows.into_iter().map(|r| r.try_into()).collect()
    }

    async fn get_successful_strategies(
        &self,
        attractor_type: &AttractorType,
        limit: usize,
    ) -> DomainResult<Vec<StrategyEntry>> {
        // Query converged trajectories, then filter strategy entries in application code
        // for those with positive convergence_delta_achieved and matching attractor type.
        let rows: Vec<TrajectoryRow> = sqlx::query_as(
            r#"SELECT * FROM convergence_trajectories
               WHERE phase = '"converged"'
               ORDER BY updated_at DESC
               LIMIT ?"#,
        )
        .bind((limit * 10) as i64) // Fetch more rows to account for filtering
        .fetch_all(&self.pool)
        .await?;

        let target_attractor_name = attractor_type_name(attractor_type);
        let mut successful_entries: Vec<StrategyEntry> = Vec::new();

        for row in rows {
            let trajectory: Trajectory = row.try_into()?;

            // Check if this trajectory's attractor classification matches the requested type
            let trajectory_attractor_name =
                attractor_type_name(&trajectory.attractor_state.classification);
            if trajectory_attractor_name != target_attractor_name {
                continue;
            }

            // Collect strategy entries with positive convergence delta
            for entry in &trajectory.strategy_log {
                if let Some(delta) = entry.convergence_delta_achieved {
                    if delta > 0.0 {
                        successful_entries.push(entry.clone());
                    }
                }
            }

            if successful_entries.len() >= limit {
                break;
            }
        }

        successful_entries.truncate(limit);
        Ok(successful_entries)
    }

    async fn delete(&self, trajectory_id: &str) -> DomainResult<()> {
        sqlx::query("DELETE FROM convergence_trajectories WHERE id = ?")
            .bind(trajectory_id)
            .execute(&self.pool)
            .await?;

        Ok(())
    }

    async fn avg_iterations_by_complexity(&self, complexity: Complexity) -> DomainResult<f64> {
        // Map complexity to the token budget that `allocate_budget` would assign.
        // Since ConvergenceBudget is stored as JSON in budget_json, we match on
        // the max_tokens field which uniquely identifies each complexity tier.
        let max_tokens: u64 = match complexity {
            Complexity::Trivial => 50_000,
            Complexity::Simple => 150_000,
            Complexity::Moderate => 400_000,
            Complexity::Complex => 1_000_000,
        };

        // Use json_extract to read max_tokens from budget_json and
        // json_array_length to count observations (iterations).
        // Only consider terminal trajectories (converged or exhausted).
        let row: Option<(f64,)> = sqlx::query_as(
            r#"SELECT AVG(json_array_length(observations_json)) as avg_iters
               FROM convergence_trajectories
               WHERE (phase = '"converged"' OR phase = '"exhausted"')
                 AND json_extract(budget_json, '$.max_tokens') = ?"#,
        )
        .bind(max_tokens as i64)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(|r| r.0).unwrap_or(0.0))
    }

    async fn strategy_effectiveness(&self, strategy: StrategyKind) -> DomainResult<StrategyStats> {
        let strategy_name = strategy.kind_name().to_string();

        // Fetch all strategy_log_json columns from the database and compute
        // statistics in application code. This avoids complex JSON iteration
        // in SQLite which can be fragile across versions.
        let rows: Vec<(String,)> = sqlx::query_as(
            r#"SELECT strategy_log_json
               FROM convergence_trajectories
               WHERE strategy_log_json != '[]'"#,
        )
        .fetch_all(&self.pool)
        .await?;

        let mut total_uses: u64 = 0;
        let mut success_count: u64 = 0;
        let mut total_delta: f64 = 0.0;
        let mut total_tokens: u64 = 0;

        for (log_json,) in &rows {
            let entries: Vec<StrategyEntry> = serde_json::from_str(log_json)
                .map_err(|e| {
                    DomainError::SerializationError(format!("Invalid strategy_log: {}", e))
                })?;

            for entry in &entries {
                if entry.strategy_kind.kind_name() == strategy_name {
                    total_uses += 1;
                    total_tokens += entry.tokens_used;

                    if let Some(delta) = entry.convergence_delta_achieved {
                        total_delta += delta;
                        if delta > 0.0 {
                            success_count += 1;
                        }
                    }
                }
            }
        }

        let average_delta = if total_uses > 0 {
            total_delta / total_uses as f64
        } else {
            0.0
        };

        let average_tokens = if total_uses > 0 {
            total_tokens / total_uses
        } else {
            0
        };

        Ok(StrategyStats {
            strategy: strategy_name,
            total_uses,
            success_count,
            average_delta,
            average_tokens,
        })
    }

    async fn attractor_distribution(&self) -> DomainResult<HashMap<String, u32>> {
        // Extract the top-level attractor classification tag from the JSON.
        // AttractorState is serialized as { "classification": { "<type>": {...} }, ... }
        // We need to get the key name inside the classification object.
        let rows: Vec<(String,)> = sqlx::query_as(
            "SELECT attractor_state_json FROM convergence_trajectories",
        )
        .fetch_all(&self.pool)
        .await?;

        let mut distribution: HashMap<String, u32> = HashMap::new();

        for (state_json,) in &rows {
            let state: AttractorState = serde_json::from_str(state_json).map_err(|e| {
                DomainError::SerializationError(format!("Invalid attractor_state: {}", e))
            })?;

            let type_name = attractor_type_name(&state.classification);
            *distribution.entry(type_name.to_string()).or_insert(0) += 1;
        }

        Ok(distribution)
    }

    async fn convergence_rate_by_task_type(&self, category: &str) -> DomainResult<f64> {
        // Match category against specification_json content using LIKE.
        // Count converged vs total terminal trajectories.
        let pattern = format!("%{}%", category);

        let row: Option<(i64, i64)> = sqlx::query_as(
            r#"SELECT
                 COUNT(CASE WHEN phase = '"converged"' THEN 1 END) as converged,
                 COUNT(*) as total
               FROM convergence_trajectories
               WHERE (phase = '"converged"' OR phase = '"exhausted"' OR phase = '"trapped"')
                 AND specification_json LIKE ?"#,
        )
        .bind(&pattern)
        .fetch_optional(&self.pool)
        .await?;

        match row {
            Some((converged, total)) if total > 0 => {
                Ok(converged as f64 / total as f64)
            }
            _ => Ok(0.0),
        }
    }

    async fn get_similar_trajectories(
        &self,
        description: &str,
        tags: &[String],
        limit: usize,
    ) -> DomainResult<Vec<Trajectory>> {
        // Build a query that matches the description and any of the tags
        // against the specification_json content. We use multiple LIKE
        // conditions combined with OR so that any matching keyword counts.
        //
        // The description is split into significant keywords (>= 4 chars)
        // to broaden matching beyond exact substring.

        let keywords: Vec<String> = description
            .split_whitespace()
            .filter(|w| w.len() >= 4)
            .map(|w| w.to_lowercase())
            .collect();

        let mut all_terms: Vec<String> = keywords;
        for tag in tags {
            if !tag.is_empty() {
                all_terms.push(tag.to_lowercase());
            }
        }

        if all_terms.is_empty() {
            // No meaningful search terms; fall back to recency-based retrieval.
            return self.get_recent(limit).await;
        }

        // Build WHERE clause with LIKE conditions for each term.
        let like_clauses: Vec<String> = all_terms
            .iter()
            .map(|_| "LOWER(specification_json) LIKE ?".to_string())
            .collect();
        let where_clause = like_clauses.join(" OR ");

        let query = format!(
            r#"SELECT * FROM convergence_trajectories
               WHERE {}
               ORDER BY updated_at DESC
               LIMIT ?"#,
            where_clause
        );

        // sqlx requires statically-known bind count, so we build the query
        // dynamically using sqlx::query_as with runtime binds.
        let mut q = sqlx::query_as::<_, TrajectoryRow>(&query);
        for term in &all_terms {
            q = q.bind(format!("%{}%", term));
        }
        q = q.bind(limit as i64);

        let rows: Vec<TrajectoryRow> = q.fetch_all(&self.pool).await?;

        rows.into_iter().map(|r| r.try_into()).collect()
    }
}

#[derive(sqlx::FromRow)]
struct TrajectoryRow {
    id: String,
    task_id: String,
    goal_id: Option<String>,
    phase: String,
    total_fresh_starts: i32,
    specification_json: String,
    observations_json: String,
    attractor_state_json: String,
    budget_json: String,
    policy_json: String,
    strategy_log_json: String,
    context_health_json: String,
    hints_json: String,
    forced_strategy_json: Option<String>,
    created_at: String,
    updated_at: String,
}

impl TryFrom<TrajectoryRow> for Trajectory {
    type Error = DomainError;

    fn try_from(row: TrajectoryRow) -> Result<Self, Self::Error> {
        let id = super::parse_uuid(&row.id)?;
        let task_id = super::parse_uuid(&row.task_id)?;
        let goal_id = super::parse_optional_uuid(row.goal_id)?;

        let phase: ConvergencePhase = serde_json::from_str(&row.phase)
            .map_err(|e| DomainError::SerializationError(format!("Invalid phase: {}", e)))?;

        let specification: SpecificationEvolution =
            serde_json::from_str(&row.specification_json)
                .map_err(|e| DomainError::SerializationError(format!("Invalid specification: {}", e)))?;

        let observations: Vec<Observation> =
            serde_json::from_str(&row.observations_json)
                .map_err(|e| DomainError::SerializationError(format!("Invalid observations: {}", e)))?;

        let attractor_state: AttractorState =
            serde_json::from_str(&row.attractor_state_json)
                .map_err(|e| DomainError::SerializationError(format!("Invalid attractor_state: {}", e)))?;

        let budget: ConvergenceBudget =
            serde_json::from_str(&row.budget_json)
                .map_err(|e| DomainError::SerializationError(format!("Invalid budget: {}", e)))?;

        let policy: ConvergencePolicy =
            serde_json::from_str(&row.policy_json)
                .map_err(|e| DomainError::SerializationError(format!("Invalid policy: {}", e)))?;

        let strategy_log: Vec<StrategyEntry> =
            serde_json::from_str(&row.strategy_log_json)
                .map_err(|e| DomainError::SerializationError(format!("Invalid strategy_log: {}", e)))?;

        let context_health: ContextHealth =
            serde_json::from_str(&row.context_health_json)
                .map_err(|e| DomainError::SerializationError(format!("Invalid context_health: {}", e)))?;

        let hints: Vec<String> =
            serde_json::from_str(&row.hints_json)
                .map_err(|e| DomainError::SerializationError(format!("Invalid hints: {}", e)))?;

        let forced_strategy: Option<StrategyKind> = row
            .forced_strategy_json
            .map(|s| serde_json::from_str(&s))
            .transpose()
            .map_err(|e| DomainError::SerializationError(format!("Invalid forced_strategy: {}", e)))?;

        let created_at = super::parse_datetime(&row.created_at)?;
        let updated_at = super::parse_datetime(&row.updated_at)?;

        Ok(Trajectory {
            id,
            task_id,
            goal_id,
            specification,
            observations,
            attractor_state,
            budget,
            policy,
            strategy_log,
            phase,
            context_health,
            hints,
            forced_strategy,
            total_fresh_starts: row.total_fresh_starts as u32,
            prev_intent_confidence: None,
            last_intent_confidence: None,
            lint_baseline: 0,
            created_at,
            updated_at,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::sqlite::create_migrated_test_pool;
    use crate::domain::models::{
        ArtifactReference, AttractorEvidence, ConvergenceBudget, ConvergencePhase,
        ConvergencePolicy, OverseerSignals, SpecificationEvolution, SpecificationSnapshot,
    };
    use uuid::Uuid;

    async fn setup_test_repo() -> SqliteTrajectoryRepository {
        let pool = create_migrated_test_pool().await.unwrap();
        SqliteTrajectoryRepository::new(pool)
    }

    fn test_trajectory() -> Trajectory {
        Trajectory::new(
            Uuid::new_v4(),
            Some(Uuid::new_v4()),
            SpecificationEvolution::new(SpecificationSnapshot::new("test spec".into())),
            ConvergenceBudget::default(),
            ConvergencePolicy::default(),
        )
    }

    #[tokio::test]
    async fn test_save_and_get() {
        let repo = setup_test_repo().await;
        let trajectory = test_trajectory();

        repo.save(&trajectory).await.unwrap();

        let retrieved = repo.get(&trajectory.id.to_string()).await.unwrap();
        assert!(retrieved.is_some());
        let retrieved = retrieved.unwrap();
        assert_eq!(retrieved.id, trajectory.id);
        assert_eq!(retrieved.task_id, trajectory.task_id);
        assert_eq!(retrieved.goal_id, trajectory.goal_id);
    }

    #[tokio::test]
    async fn test_save_upsert() {
        let repo = setup_test_repo().await;
        let mut trajectory = test_trajectory();

        repo.save(&trajectory).await.unwrap();

        trajectory.phase = ConvergencePhase::Iterating;
        trajectory.total_fresh_starts = 2;
        repo.save(&trajectory).await.unwrap();

        let retrieved = repo.get(&trajectory.id.to_string()).await.unwrap().unwrap();
        assert_eq!(retrieved.total_fresh_starts, 2);
    }

    #[tokio::test]
    async fn test_get_by_task() {
        let repo = setup_test_repo().await;
        let task_id = Uuid::new_v4();

        let mut t1 = test_trajectory();
        t1.task_id = task_id;
        let mut t2 = test_trajectory();
        t2.task_id = task_id;
        let t3 = test_trajectory(); // different task

        repo.save(&t1).await.unwrap();
        repo.save(&t2).await.unwrap();
        repo.save(&t3).await.unwrap();

        let results = repo.get_by_task(&task_id.to_string()).await.unwrap();
        assert_eq!(results.len(), 2);
    }

    #[tokio::test]
    async fn test_get_by_goal() {
        let repo = setup_test_repo().await;
        let goal_id = Uuid::new_v4();

        let mut t1 = test_trajectory();
        t1.goal_id = Some(goal_id);
        let mut t2 = test_trajectory();
        t2.goal_id = Some(goal_id);

        repo.save(&t1).await.unwrap();
        repo.save(&t2).await.unwrap();

        let results = repo.get_by_goal(&goal_id.to_string()).await.unwrap();
        assert_eq!(results.len(), 2);
    }

    #[tokio::test]
    async fn test_get_recent() {
        let repo = setup_test_repo().await;

        for _ in 0..5 {
            repo.save(&test_trajectory()).await.unwrap();
        }

        let results = repo.get_recent(3).await.unwrap();
        assert_eq!(results.len(), 3);
    }

    #[tokio::test]
    async fn test_delete() {
        let repo = setup_test_repo().await;
        let trajectory = test_trajectory();

        repo.save(&trajectory).await.unwrap();
        repo.delete(&trajectory.id.to_string()).await.unwrap();

        let retrieved = repo.get(&trajectory.id.to_string()).await.unwrap();
        assert!(retrieved.is_none());
    }

    #[tokio::test]
    async fn test_get_not_found() {
        let repo = setup_test_repo().await;
        let result = repo.get(&Uuid::new_v4().to_string()).await.unwrap();
        assert!(result.is_none());
    }

    // -- Analytics test helpers -----------------------------------------------

    /// Create a trajectory in a terminal phase with the given complexity budget,
    /// observations, and strategy log entries for analytics testing.
    fn converged_trajectory_with_budget(
        complexity: Complexity,
        observation_count: usize,
        strategy_entries: Vec<StrategyEntry>,
    ) -> Trajectory {
        use crate::domain::models::convergence::allocate_budget;

        let budget = allocate_budget(complexity);
        let mut t = Trajectory::new(
            Uuid::new_v4(),
            Some(Uuid::new_v4()),
            SpecificationEvolution::new(SpecificationSnapshot::new("analytics test spec".into())),
            budget,
            ConvergencePolicy::default(),
        );
        t.phase = ConvergencePhase::Converged;

        // Add observations with default signals and strategy.
        for i in 0..observation_count {
            let obs = Observation::new(
                i as u32,
                ArtifactReference::new(format!("/test/path/{}", i), format!("hash_{}", i)),
                OverseerSignals::default(),
                StrategyKind::RetryWithFeedback,
                10_000,
                5_000,
            );
            t.observations.push(obs);
        }

        t.strategy_log = strategy_entries;
        t
    }

    /// Create a trajectory with a specific attractor classification.
    fn trajectory_with_attractor(attractor_type: AttractorType) -> Trajectory {
        let mut t = test_trajectory();
        t.attractor_state = AttractorState {
            classification: attractor_type,
            confidence: 0.8,
            detected_at: None,
            evidence: AttractorEvidence {
                recent_deltas: vec![],
                recent_signatures: vec![],
                rationale: "test".into(),
            },
        };
        t
    }

    // -- Analytics tests ------------------------------------------------------

    #[tokio::test]
    async fn test_avg_iterations_by_complexity() {
        let repo = setup_test_repo().await;

        // Create two converged Simple trajectories with 3 and 5 observations.
        let t1 = converged_trajectory_with_budget(Complexity::Simple, 3, vec![]);
        let t2 = converged_trajectory_with_budget(Complexity::Simple, 5, vec![]);

        repo.save(&t1).await.unwrap();
        repo.save(&t2).await.unwrap();

        let avg = repo
            .avg_iterations_by_complexity(Complexity::Simple)
            .await
            .unwrap();

        // Average of 3 and 5 = 4.0
        assert!(
            (avg - 4.0).abs() < f64::EPSILON,
            "Expected avg 4.0, got {}",
            avg
        );
    }

    #[tokio::test]
    async fn test_avg_iterations_no_matching_data() {
        let repo = setup_test_repo().await;

        // No trajectories at all.
        let avg = repo
            .avg_iterations_by_complexity(Complexity::Complex)
            .await
            .unwrap();

        assert!(
            (avg - 0.0).abs() < f64::EPSILON,
            "Expected 0.0 for no data, got {}",
            avg
        );
    }

    #[tokio::test]
    async fn test_strategy_effectiveness() {
        let repo = setup_test_repo().await;

        // Create a trajectory with strategy log entries.
        let entries = vec![
            StrategyEntry::new(StrategyKind::RetryWithFeedback, 0, 10_000, false)
                .with_delta(0.2),
            StrategyEntry::new(StrategyKind::RetryWithFeedback, 1, 20_000, false)
                .with_delta(-0.1),
            StrategyEntry::new(StrategyKind::FocusedRepair, 2, 15_000, false)
                .with_delta(0.3),
        ];

        let t = converged_trajectory_with_budget(Complexity::Simple, 3, entries);
        repo.save(&t).await.unwrap();

        let stats = repo
            .strategy_effectiveness(StrategyKind::RetryWithFeedback)
            .await
            .unwrap();

        assert_eq!(stats.strategy, "retry_with_feedback");
        assert_eq!(stats.total_uses, 2);
        assert_eq!(stats.success_count, 1); // Only delta 0.2 > 0.0
        // Average delta: (0.2 + -0.1) / 2 = 0.05
        assert!(
            (stats.average_delta - 0.05).abs() < 1e-10,
            "Expected avg delta 0.05, got {}",
            stats.average_delta
        );
        // Average tokens: (10_000 + 20_000) / 2 = 15_000
        assert_eq!(stats.average_tokens, 15_000);
    }

    #[tokio::test]
    async fn test_strategy_effectiveness_no_uses() {
        let repo = setup_test_repo().await;

        let stats = repo
            .strategy_effectiveness(StrategyKind::Decompose)
            .await
            .unwrap();

        assert_eq!(stats.strategy, "decompose");
        assert_eq!(stats.total_uses, 0);
        assert_eq!(stats.success_count, 0);
        assert!((stats.average_delta - 0.0).abs() < f64::EPSILON);
    }

    #[tokio::test]
    async fn test_attractor_distribution() {
        let repo = setup_test_repo().await;

        // Create trajectories with different attractor types.
        let t1 = trajectory_with_attractor(AttractorType::FixedPoint {
            estimated_remaining_iterations: 2,
            estimated_remaining_tokens: 40_000,
        });
        let t2 = trajectory_with_attractor(AttractorType::FixedPoint {
            estimated_remaining_iterations: 3,
            estimated_remaining_tokens: 60_000,
        });
        let t3 = trajectory_with_attractor(AttractorType::Plateau {
            stall_duration: 4,
            plateau_level: 0.6,
        });

        repo.save(&t1).await.unwrap();
        repo.save(&t2).await.unwrap();
        repo.save(&t3).await.unwrap();

        let dist = repo.attractor_distribution().await.unwrap();

        assert_eq!(dist.get("fixed_point"), Some(&2));
        assert_eq!(dist.get("plateau"), Some(&1));
        // No other types should be present.
        assert!(dist.get("limit_cycle").is_none());
    }

    #[tokio::test]
    async fn test_convergence_rate_by_task_type() {
        let repo = setup_test_repo().await;

        // Create trajectories with "rust" in specification.
        let mut t1 = test_trajectory();
        t1.specification = SpecificationEvolution::new(
            SpecificationSnapshot::new("implement a rust parser".into()),
        );
        t1.phase = ConvergencePhase::Converged;
        repo.save(&t1).await.unwrap();

        let mut t2 = test_trajectory();
        t2.specification = SpecificationEvolution::new(
            SpecificationSnapshot::new("implement a rust formatter".into()),
        );
        t2.phase = ConvergencePhase::Exhausted;
        repo.save(&t2).await.unwrap();

        // One unrelated trajectory.
        let mut t3 = test_trajectory();
        t3.specification = SpecificationEvolution::new(
            SpecificationSnapshot::new("implement a python linter".into()),
        );
        t3.phase = ConvergencePhase::Converged;
        repo.save(&t3).await.unwrap();

        let rate = repo
            .convergence_rate_by_task_type("rust")
            .await
            .unwrap();

        // 1 converged out of 2 terminal "rust" trajectories = 0.5
        assert!(
            (rate - 0.5).abs() < f64::EPSILON,
            "Expected rate 0.5, got {}",
            rate
        );
    }

    #[tokio::test]
    async fn test_convergence_rate_no_matching_category() {
        let repo = setup_test_repo().await;

        let rate = repo
            .convergence_rate_by_task_type("nonexistent_category_xyz")
            .await
            .unwrap();

        assert!(
            (rate - 0.0).abs() < f64::EPSILON,
            "Expected 0.0 for no matches, got {}",
            rate
        );
    }

    #[tokio::test]
    async fn test_get_similar_trajectories() {
        let repo = setup_test_repo().await;

        // Create trajectories with varying specifications.
        let mut t1 = test_trajectory();
        t1.specification = SpecificationEvolution::new(
            SpecificationSnapshot::new("implement authentication middleware".into()),
        );
        repo.save(&t1).await.unwrap();

        let mut t2 = test_trajectory();
        t2.specification = SpecificationEvolution::new(
            SpecificationSnapshot::new("implement database connection pooling".into()),
        );
        repo.save(&t2).await.unwrap();

        let mut t3 = test_trajectory();
        t3.specification = SpecificationEvolution::new(
            SpecificationSnapshot::new("fix authentication token expiry".into()),
        );
        repo.save(&t3).await.unwrap();

        // Search for "authentication" related trajectories.
        let results = repo
            .get_similar_trajectories("authentication system", &[], 10)
            .await
            .unwrap();

        // Should find t1 and t3 (both mention "authentication").
        assert_eq!(results.len(), 2);
    }

    #[tokio::test]
    async fn test_get_similar_trajectories_with_tags() {
        let repo = setup_test_repo().await;

        let mut t1 = test_trajectory();
        t1.specification = SpecificationEvolution::new(
            SpecificationSnapshot::new("implement REST API endpoint".into()),
        );
        repo.save(&t1).await.unwrap();

        let mut t2 = test_trajectory();
        t2.specification = SpecificationEvolution::new(
            SpecificationSnapshot::new("implement GraphQL resolver".into()),
        );
        repo.save(&t2).await.unwrap();

        // Search with tags that match t2.
        let results = repo
            .get_similar_trajectories("api", &["graphql".to_string()], 10)
            .await
            .unwrap();

        // Should find t2 (matches "graphql" tag).
        assert!(results.len() >= 1);
        // Verify at least one result has GraphQL in spec.
        let has_graphql = results.iter().any(|t| {
            let spec_json = serde_json::to_string(&t.specification).unwrap_or_default();
            spec_json.to_lowercase().contains("graphql")
        });
        assert!(has_graphql);
    }

    #[tokio::test]
    async fn test_get_similar_trajectories_empty_terms() {
        let repo = setup_test_repo().await;

        // Save some trajectories.
        for _ in 0..3 {
            repo.save(&test_trajectory()).await.unwrap();
        }

        // Empty description with short words should fall back to get_recent.
        let results = repo
            .get_similar_trajectories("a b c", &[], 10)
            .await
            .unwrap();

        assert_eq!(results.len(), 3);
    }

    #[tokio::test]
    async fn test_get_successful_strategies() {
        let repo = setup_test_repo().await;

        // Create a converged trajectory with a fixed_point attractor and
        // strategy entries that have positive deltas.
        let mut t = test_trajectory();
        t.phase = ConvergencePhase::Converged;
        t.attractor_state = AttractorState {
            classification: AttractorType::FixedPoint {
                estimated_remaining_iterations: 0,
                estimated_remaining_tokens: 0,
            },
            confidence: 0.9,
            detected_at: None,
            evidence: AttractorEvidence {
                recent_deltas: vec![0.1, 0.2],
                recent_signatures: vec![],
                rationale: "test".into(),
            },
        };
        t.strategy_log = vec![
            StrategyEntry::new(StrategyKind::RetryWithFeedback, 0, 10_000, false)
                .with_delta(0.15),
            StrategyEntry::new(StrategyKind::FocusedRepair, 1, 15_000, false)
                .with_delta(-0.05), // negative, should not be included
            StrategyEntry::new(StrategyKind::RetryAugmented, 2, 20_000, false)
                .with_delta(0.25),
        ];
        repo.save(&t).await.unwrap();

        let attractor_type = AttractorType::FixedPoint {
            estimated_remaining_iterations: 0,
            estimated_remaining_tokens: 0,
        };

        let entries = repo
            .get_successful_strategies(&attractor_type, 10)
            .await
            .unwrap();

        // Should return 2 entries (delta 0.15 and 0.25), not the negative one.
        assert_eq!(entries.len(), 2);
        for entry in &entries {
            assert!(entry.convergence_delta_achieved.unwrap() > 0.0);
        }
    }

    #[tokio::test]
    async fn test_get_successful_strategies_wrong_attractor() {
        let repo = setup_test_repo().await;

        // Create a converged trajectory with fixed_point attractor.
        let mut t = test_trajectory();
        t.phase = ConvergencePhase::Converged;
        t.attractor_state = AttractorState {
            classification: AttractorType::FixedPoint {
                estimated_remaining_iterations: 0,
                estimated_remaining_tokens: 0,
            },
            confidence: 0.9,
            detected_at: None,
            evidence: AttractorEvidence {
                recent_deltas: vec![],
                recent_signatures: vec![],
                rationale: "test".into(),
            },
        };
        t.strategy_log = vec![
            StrategyEntry::new(StrategyKind::RetryWithFeedback, 0, 10_000, false)
                .with_delta(0.3),
        ];
        repo.save(&t).await.unwrap();

        // Query for plateau attractor -- should find nothing because the
        // trajectory has a fixed_point attractor.
        let attractor_type = AttractorType::Plateau {
            stall_duration: 3,
            plateau_level: 0.5,
        };

        let entries = repo
            .get_successful_strategies(&attractor_type, 10)
            .await
            .unwrap();

        assert!(entries.is_empty());
    }
}
