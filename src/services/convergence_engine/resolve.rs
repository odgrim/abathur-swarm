//! Convergence engine -- 6.5/6.7/8.x resolve phase.
//!
//! Terminal classification, budget extension, memory persistence, bandit
//! state persistence.

use chrono::Utc;

use crate::domain::errors::{DomainError, DomainResult};
use crate::domain::models::convergence::*;
use crate::domain::models::{Memory, MemoryQuery, MemoryType};
use crate::domain::ports::{MemoryRepository, TrajectoryRepository};

use super::{ConvergenceDomainEvent, ConvergenceEngine, OverseerMeasurer};

impl<T: TrajectoryRepository, M: MemoryRepository, O: OverseerMeasurer> ConvergenceEngine<T, M, O> {
    // -----------------------------------------------------------------------
    // 6.5 request_extension -- Budget extension flow
    // -----------------------------------------------------------------------

    /// Request a budget extension (spec 6.5).
    ///
    /// When the trajectory is converging but running low on budget, this
    /// method requests additional resources. In a full implementation, this
    /// would interact with an intervention point for user approval.
    ///
    /// Returns `true` if the extension was granted, `false` if denied.
    pub async fn request_extension(&self, trajectory: &mut Trajectory) -> DomainResult<bool> {
        trajectory.budget.extensions_requested += 1;

        let remaining_fraction = trajectory.budget.remaining_fraction();

        // Estimate how much additional budget is needed
        let additional_tokens = (trajectory.budget.max_tokens as f64 * 0.3) as u64;
        let additional_iterations = 3u32;

        // Emit extension request event
        self.emit_event(ConvergenceEvent::BudgetExtensionRequested {
            trajectory_id: trajectory.id.to_string(),
            current_usage_fraction: 1.0 - remaining_fraction,
            requested_extension_tokens: additional_tokens,
            convergence_evidence: format!(
                "Convergence delta: {:.3}, attractor: {}",
                trajectory.latest_convergence_delta(),
                self.attractor_type_name(&trajectory.attractor_state.classification),
            ),
        });

        // Auto-approve extensions for trajectories showing progress.
        // The should_request_extension() gate already verified positive
        // convergence delta, so we broaden beyond just FixedPoint.
        let approved = match &trajectory.attractor_state.classification {
            // Converging toward solution — always extend
            AttractorType::FixedPoint { .. } => true,
            // Not enough data for full classification but trending positive — extend
            AttractorType::Indeterminate { tendency } => {
                matches!(tendency, ConvergenceTendency::Improving)
            }
            // Positive delta triggered the request, so plateau is breaking — extend
            AttractorType::Plateau { .. } => true,
            // Oscillating or diverging — extension won't help
            AttractorType::LimitCycle { .. } | AttractorType::Divergent { .. } => false,
        };

        if approved {
            trajectory
                .budget
                .extend(additional_tokens, additional_iterations);
            self.emit_event(ConvergenceEvent::BudgetExtensionGranted {
                trajectory_id: trajectory.id.to_string(),
                additional_tokens,
            });
            self.trajectory_store.save(trajectory).await?;
            Ok(true)
        } else {
            self.emit_event(ConvergenceEvent::BudgetExtensionDenied {
                trajectory_id: trajectory.id.to_string(),
                reason: format!(
                    "Trajectory attractor is {} — only FixedPoint, Improving Indeterminate, or Plateau qualify for extension",
                    self.attractor_type_name(&trajectory.attractor_state.classification),
                ),
            });
            Ok(false)
        }
    }

    // -----------------------------------------------------------------------
    // 6.7 finalize -- End-of-trajectory work
    // -----------------------------------------------------------------------

    /// Finalize a trajectory after the convergence loop exits (spec 6.7).
    ///
    /// Performs end-of-trajectory work:
    /// 1. Set the terminal phase on the trajectory.
    /// 2. Emit the appropriate terminal event.
    /// 3. Store convergence memory (success or failure).
    /// 4. Persist bandit state.
    /// 5. Save the final trajectory state.
    pub async fn finalize(
        &self,
        trajectory: &mut Trajectory,
        outcome: &ConvergenceOutcome,
        bandit: &StrategyBandit,
    ) -> DomainResult<()> {
        // 1. Set terminal phase
        match outcome {
            ConvergenceOutcome::Converged {
                final_observation_sequence,
                ..
            } => {
                trajectory.phase = ConvergencePhase::Converged;
                // 2. Emit terminal event
                self.emit_event(ConvergenceEvent::TrajectoryConverged {
                    trajectory_id: trajectory.id.to_string(),
                    total_observations: trajectory.observations.len() as u32,
                    total_tokens_used: trajectory.budget.tokens_used,
                    total_fresh_starts: trajectory.total_fresh_starts,
                    timestamp: Utc::now(),
                });
                // 3. Store success memory
                if self.config.memory_enabled {
                    self.store_success_memory(trajectory, *final_observation_sequence)
                        .await?;
                }
            }
            ConvergenceOutcome::Exhausted { .. } => {
                trajectory.phase = ConvergencePhase::Exhausted;
                let best_seq = trajectory
                    .best_observation()
                    .map(|o| o.sequence)
                    .unwrap_or(0);
                self.emit_event(ConvergenceEvent::TrajectoryExhausted {
                    trajectory_id: trajectory.id.to_string(),
                    best_observation_sequence: best_seq,
                    budget_consumed_fraction: 1.0 - trajectory.budget.remaining_fraction(),
                    reason: "Budget exhausted without reaching acceptance threshold".to_string(),
                    timestamp: Utc::now(),
                });
                if self.config.memory_enabled {
                    self.store_failure_memory(trajectory, "exhausted").await?;
                }
            }
            ConvergenceOutcome::Trapped { attractor_type, .. } => {
                trajectory.phase = ConvergencePhase::Trapped;
                self.emit_event(ConvergenceEvent::TrajectoryTrapped {
                    trajectory_id: trajectory.id.to_string(),
                    attractor_type: attractor_type.clone(),
                    cycle_period: match attractor_type {
                        AttractorType::LimitCycle { period, .. } => Some(*period),
                        _ => None,
                    },
                    escape_attempts: trajectory
                        .strategy_log
                        .iter()
                        .filter(|e| e.strategy_kind.is_exploration())
                        .count() as u32,
                    timestamp: Utc::now(),
                });
                if self.config.memory_enabled {
                    self.store_failure_memory(trajectory, "trapped").await?;
                }
            }
            ConvergenceOutcome::Decomposed {
                child_trajectory_ids,
                ..
            } => {
                let children: Vec<uuid::Uuid> = child_trajectory_ids
                    .iter()
                    .filter_map(|id| id.parse().ok())
                    .collect();
                trajectory.phase = ConvergencePhase::Coordinating { children };
                self.emit_event(ConvergenceEvent::DecompositionTriggered {
                    parent_trajectory_id: trajectory.id.to_string(),
                    child_count: child_trajectory_ids.len(),
                });
            }
            ConvergenceOutcome::BudgetDenied { .. } => {
                trajectory.phase = ConvergencePhase::Exhausted;
                self.emit_event(ConvergenceEvent::TrajectoryExhausted {
                    trajectory_id: trajectory.id.to_string(),
                    best_observation_sequence: trajectory
                        .best_observation()
                        .map(|o| o.sequence)
                        .unwrap_or(0),
                    budget_consumed_fraction: 1.0 - trajectory.budget.remaining_fraction(),
                    reason: "Budget extension denied".to_string(),
                    timestamp: Utc::now(),
                });
                if self.config.memory_enabled {
                    self.store_failure_memory(trajectory, "budget_denied")
                        .await?;
                }
            }
        }

        // 4. Persist bandit state
        if self.config.memory_enabled {
            self.persist_bandit_state(bandit, trajectory).await?;
        }

        // 5. Record token usage for budget calibration tracking
        if let (Some(tier), Ok(mut tracker)) =
            (trajectory.complexity, self.calibration_tracker.lock())
        {
            tracker.record_completion(tier, trajectory.budget.tokens_used);

            // 5b. Emit budget calibration alerts if P95 exceeds allocated budget
            let now = Utc::now();
            for alert in tracker.calibration_alerts() {
                self.emit_event(ConvergenceEvent::BudgetCalibrationExceeded {
                    tier: alert.tier,
                    p95_tokens: alert.p95_tokens,
                    allocated_tokens: alert.allocated_tokens,
                    overshoot_pct: alert.overshoot_pct,
                    timestamp: now,
                });
            }
        }

        // 6. Save final trajectory state
        trajectory.updated_at = Utc::now();
        self.trajectory_store.save(trajectory).await?;

        Ok(())
    }

    // -----------------------------------------------------------------------
    // 8.1 store_success_memory -- Store success memory
    // -----------------------------------------------------------------------

    /// Store a success memory for the converged trajectory (spec 8.1).
    ///
    /// Records the effective strategy sequence, attractor path, and key
    /// overseer transitions so that future similar tasks can benefit from
    /// this trajectory's experience.
    pub async fn store_success_memory(
        &self,
        trajectory: &Trajectory,
        final_sequence: u32,
    ) -> DomainResult<()> {
        let strategy_sequence: Vec<String> = trajectory
            .effective_strategy_sequence()
            .iter()
            .map(|s| s.kind_name().to_string())
            .collect();

        let attractor_path = trajectory.attractor_path();

        let content = format!(
            "Convergence SUCCESS for task {}\n\
             Strategy sequence: [{}]\n\
             Attractor path: [{}]\n\
             Total observations: {}\n\
             Total fresh starts: {}\n\
             Final observation: {}\n\
             Tokens used: {}",
            trajectory.task_id,
            strategy_sequence.join(" -> "),
            attractor_path.join(", "),
            trajectory.observations.len(),
            trajectory.total_fresh_starts,
            final_sequence,
            trajectory.budget.tokens_used,
        );

        let key = format!("convergence-success-{}", trajectory.id);

        // Auto-increment version so retries don't hit UNIQUE(namespace, key, version).
        let next_version = match self
            .memory_repository
            .get_by_key(&key, "convergence")
            .await?
        {
            Some(existing) => existing.version + 1,
            None => 1,
        };

        let mut memory = Memory::semantic(key, content)
            .with_namespace("convergence")
            .with_type(MemoryType::Pattern)
            .with_source("convergence_engine")
            .with_task(trajectory.task_id)
            .with_tag("convergence")
            .with_tag("success");
        memory.version = next_version;

        if let Some(goal_id) = trajectory.goal_id {
            let memory = memory.with_goal(goal_id);
            self.memory_repository.store(&memory).await?;
        } else {
            self.memory_repository.store(&memory).await?;
        }

        Ok(())
    }

    // -----------------------------------------------------------------------
    // 8.1 store_failure_memory -- Store failure memory
    // -----------------------------------------------------------------------

    /// Store a failure memory for the trajectory (spec 8.1).
    ///
    /// Records what was tried and why it failed, expressed as anti-patterns,
    /// so that future similar tasks avoid the same pitfalls.
    pub async fn store_failure_memory(
        &self,
        trajectory: &Trajectory,
        failure_reason: &str,
    ) -> DomainResult<()> {
        let strategy_sequence: Vec<String> = trajectory
            .effective_strategy_sequence()
            .iter()
            .map(|s| s.kind_name().to_string())
            .collect();

        let persistent_gaps = trajectory.persistent_gaps();
        let decisive_changes = trajectory.decisive_overseer_changes();

        let content = format!(
            "Convergence FAILURE for task {} (reason: {})\n\
             Strategy sequence: [{}]\n\
             Persistent gaps: [{}]\n\
             Decisive overseer changes: [{}]\n\
             Total observations: {}\n\
             Total fresh starts: {}\n\
             Tokens used: {}",
            trajectory.task_id,
            failure_reason,
            strategy_sequence.join(" -> "),
            persistent_gaps.join(", "),
            decisive_changes.join(", "),
            trajectory.observations.len(),
            trajectory.total_fresh_starts,
            trajectory.budget.tokens_used,
        );

        let key = format!("convergence-failure-{}", trajectory.id);

        // Auto-increment version so retries don't hit UNIQUE(namespace, key, version).
        let next_version = match self
            .memory_repository
            .get_by_key(&key, "convergence")
            .await?
        {
            Some(existing) => existing.version + 1,
            None => 1,
        };

        let mut memory = Memory::episodic(key, content)
            .with_namespace("convergence")
            .with_type(MemoryType::Error)
            .with_source("convergence_engine")
            .with_task(trajectory.task_id)
            .with_tag("convergence")
            .with_tag("failure");
        memory.version = next_version;

        if let Some(goal_id) = trajectory.goal_id {
            let memory = memory.with_goal(goal_id);
            self.memory_repository.store(&memory).await?;
        } else {
            self.memory_repository.store(&memory).await?;
        }

        Ok(())
    }

    // -----------------------------------------------------------------------
    // 8.2 persist_bandit_state -- Persist bandit state
    // -----------------------------------------------------------------------

    /// Persist the strategy bandit state to memory (spec 8.2).
    ///
    /// Serializes the bandit's learned Beta distributions so they can be
    /// recalled for future trajectories.
    pub async fn persist_bandit_state(
        &self,
        bandit: &StrategyBandit,
        trajectory: &Trajectory,
    ) -> DomainResult<()> {
        let content = serde_json::to_string(bandit)
            .map_err(|e| DomainError::SerializationError(e.to_string()))?;

        let key = format!("strategy-bandit-{}", trajectory.task_id);

        // Bandit state is a singleton — update in place if it already exists.
        match self
            .memory_repository
            .get_by_key(&key, "convergence")
            .await?
        {
            Some(mut existing) => {
                existing.content = content;
                existing.updated_at = Utc::now();
                self.memory_repository.update(&existing).await?;
            }
            None => {
                let memory = Memory::semantic(key, content)
                    .with_namespace("convergence")
                    .with_type(MemoryType::Pattern)
                    .with_source("convergence_engine")
                    .with_task(trajectory.task_id)
                    .with_tag("strategy-bandit");

                self.memory_repository.store(&memory).await?;
            }
        }

        Ok(())
    }

    // -----------------------------------------------------------------------
    // 8.2 initialize_bandit -- Initialize bandit from memory
    // -----------------------------------------------------------------------

    /// Initialize the strategy bandit from persisted memory (spec 8.2).
    ///
    /// Searches for previously persisted bandit state in memory. If found,
    /// deserializes and returns it. Otherwise, returns a bandit with default
    /// priors.
    pub async fn initialize_bandit(&self, trajectory: &Trajectory) -> StrategyBandit {
        if !self.config.memory_enabled {
            return StrategyBandit::with_default_priors();
        }

        // Search for persisted bandit state
        let query = MemoryQuery {
            tags: vec!["strategy-bandit".to_string()],
            task_id: Some(trajectory.task_id),
            namespace: Some("convergence".to_string()),
            limit: Some(1),
            ..Default::default()
        };

        match self.memory_repository.query(query).await {
            Ok(memories) => {
                if let Some(memory) = memories.first() {
                    match serde_json::from_str::<StrategyBandit>(&memory.content) {
                        Ok(bandit) => return bandit,
                        Err(e) => {
                            self.event_sink
                                .emit(ConvergenceDomainEvent::BanditDeserializationFailed {
                                    error: e.to_string(),
                                })
                                .await;
                        }
                    }
                }
            }
            Err(e) => {
                self.event_sink
                    .emit(ConvergenceDomainEvent::BanditQueryFailed {
                        error: e.to_string(),
                    })
                    .await;
            }
        }

        StrategyBandit::with_default_priors()
    }
}
