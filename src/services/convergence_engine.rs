//! Convergence engine service (Spec Parts 6, 8, 9).
//!
//! The `ConvergenceEngine` owns the full lifecycle of a trajectory from task
//! submission to terminal outcome. It orchestrates:
//!
//! - **SETUP** -- Basin width estimation, budget allocation, policy assembly.
//! - **PREPARE** -- Acceptance test generation, ambiguity detection, memory recall.
//! - **DECIDE** -- Proactive decomposition check, convergence mode selection.
//! - **ITERATE** -- Strategy selection, execution, measurement, attractor
//!   classification, bandit update, loop control.
//! - **RESOLVE** -- Memory persistence, bandit state persistence, terminal events.

use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use uuid::Uuid;

use crate::domain::errors::{DomainError, DomainResult};
use crate::domain::models::convergence::*;
use crate::domain::models::task::Complexity;
use crate::domain::models::{Memory, MemoryQuery, MemoryType};
use crate::domain::ports::{MemoryRepository, TrajectoryRepository};

// ---------------------------------------------------------------------------
// OverseerMeasurer trait
// ---------------------------------------------------------------------------

/// Trait for overseer measurement. The OverseerCluster implements this.
///
/// This trait decouples the convergence engine from the concrete OverseerCluster
/// implementation, allowing independent development and testing. The engine
/// delegates all artifact measurement to this trait, receiving aggregated
/// overseer signals in return.
#[async_trait]
pub trait OverseerMeasurer: Send + Sync {
    /// Measure an artifact using the configured overseers and return aggregated signals.
    ///
    /// The implementation should run overseers in cost-ordered phases (cheap first,
    /// expensive last) and respect the policy's `skip_expensive_overseers` flag.
    async fn measure(
        &self,
        artifact: &ArtifactReference,
        policy: &ConvergencePolicy,
    ) -> DomainResult<OverseerSignals>;
}

// ---------------------------------------------------------------------------
// StrategyContext
// ---------------------------------------------------------------------------

/// Context assembled for a strategy execution.
///
/// Contains everything the agent runtime needs to execute a convergence strategy:
/// the strategy type, current specification state, latest overseer signals,
/// carry-forward data for fresh starts, and focus hints.
#[derive(Debug, Clone)]
pub struct StrategyContext {
    /// The strategy being executed.
    pub strategy: StrategyKind,
    /// The current effective specification snapshot.
    pub specification: SpecificationSnapshot,
    /// The most recent overseer signals, if any observations exist.
    pub latest_signals: Option<OverseerSignals>,
    /// Carry-forward data for fresh start strategies.
    pub carry_forward: Option<CarryForward>,
    /// Hints derived from the trajectory and strategy type.
    pub hints: Vec<String>,
    /// Areas to focus on based on recent overseer feedback.
    pub focus_areas: Vec<String>,
}

// ---------------------------------------------------------------------------
// ConvergenceEngine
// ---------------------------------------------------------------------------

/// The main convergence engine service.
///
/// Orchestrates the full convergence lifecycle for a task trajectory:
/// estimation, preparation, iteration, and resolution. Uses generic type
/// parameters for repository dependencies following the codebase pattern.
pub struct ConvergenceEngine<T: TrajectoryRepository, M: MemoryRepository, O: OverseerMeasurer> {
    trajectory_store: Arc<T>,
    memory_repository: Arc<M>,
    overseer_measurer: Arc<O>,
    config: ConvergenceEngineConfig,
}

impl<T: TrajectoryRepository, M: MemoryRepository, O: OverseerMeasurer>
    ConvergenceEngine<T, M, O>
{
    // -----------------------------------------------------------------------
    // Constructor
    // -----------------------------------------------------------------------

    /// Create a new convergence engine with the given dependencies.
    pub fn new(
        trajectory_store: Arc<T>,
        memory_repository: Arc<M>,
        overseer_measurer: Arc<O>,
        config: ConvergenceEngineConfig,
    ) -> Self {
        Self {
            trajectory_store,
            memory_repository,
            overseer_measurer,
            config,
        }
    }

    // -----------------------------------------------------------------------
    // 6.2 prepare -- Prepare convergence infrastructure from a TaskSubmission
    // -----------------------------------------------------------------------

    /// Prepare convergence infrastructure from a task submission (spec 6.2).
    ///
    /// This is the SETUP + PREPARE phase combined:
    /// 1. Estimate basin width from the submission's specification signals.
    /// 2. Allocate a convergence budget based on inferred complexity.
    /// 3. Apply basin width adjustments to budget and policy.
    /// 4. Apply priority hint adjustments (if any).
    /// 5. Build convergence infrastructure from discovered project assets.
    /// 6. Fold submission constraints and anti-patterns into the specification
    ///    as amendments.
    /// 7. Create the trajectory.
    /// 8. Initialize the strategy bandit from memory (if enabled).
    ///
    /// Returns the prepared `Trajectory` and the convergence `ConvergenceInfrastructure`.
    pub async fn prepare(
        &self,
        submission: &TaskSubmission,
    ) -> DomainResult<(Trajectory, ConvergenceInfrastructure)> {
        // 1. Estimate basin width
        let infra = &submission.discovered_infrastructure;
        let basin = estimate_basin_width(
            &submission.description,
            !infra.acceptance_tests.is_empty(),
            !infra.examples.is_empty(),
            !infra.invariants.is_empty(),
            !infra.anti_examples.is_empty(),
            !infra.context_files.is_empty(),
        );

        // 2. Allocate budget from complexity
        let mut budget = allocate_budget(submission.inferred_complexity);

        // 3. Assemble default policy
        let mut policy = self.config.default_policy.clone();

        // 4. Apply basin width adjustments
        apply_basin_width(&basin, &mut budget, &mut policy);

        // 5. Apply priority hint overlay (if any)
        if let Some(hint) = submission.priority_hint {
            hint.apply(&mut policy, &mut budget);
            policy.priority_hint = Some(hint);
        }

        // 6. Build convergence infrastructure
        let mut convergence_infra =
            ConvergenceInfrastructure::from_discovered(&submission.discovered_infrastructure);
        convergence_infra.merge_user_references(&submission.references);
        convergence_infra.add_invariants(&submission.constraints);
        convergence_infra.add_anti_patterns(&submission.anti_patterns);

        // 7. Build the specification and create trajectory
        let spec_snapshot = SpecificationSnapshot::new(submission.description.clone());
        let mut spec_evolution = SpecificationEvolution::new(spec_snapshot);

        // Fold constraints into specification as amendments
        for constraint in &submission.constraints {
            spec_evolution.add_amendment(SpecificationAmendment::new(
                AmendmentSource::SubmissionConstraint,
                constraint.clone(),
                "User-provided constraint from task submission",
            ));
        }

        let task_id = Uuid::new_v4(); // In production, this comes from the task service
        let trajectory = Trajectory::new(
            task_id,
            submission.goal_id,
            spec_evolution,
            budget,
            policy,
        );

        // 8. Persist the trajectory
        self.trajectory_store.save(&trajectory).await?;

        Ok((trajectory, convergence_infra))
    }

    // -----------------------------------------------------------------------
    // 6.3 converge -- The main convergence loop
    // -----------------------------------------------------------------------

    /// Run the main convergence loop (spec 6.3).
    ///
    /// Phases: SETUP (done in prepare) -> PREPARE -> DECIDE -> ITERATE
    ///
    /// The loop iterates until a terminal condition is reached:
    /// - Converged: acceptance threshold met
    /// - Exhausted: budget consumed
    /// - Trapped: all escape strategies exhausted for a limit cycle
    /// - Decomposed: task decomposed into subtasks
    /// - BudgetDenied: extension request denied
    pub async fn converge(
        &self,
        mut trajectory: Trajectory,
        infrastructure: &ConvergenceInfrastructure,
    ) -> DomainResult<ConvergenceOutcome> {
        // -- PREPARE phase --
        trajectory.phase = ConvergencePhase::Preparing;

        // Generate acceptance tests
        if trajectory.policy.generate_acceptance_tests
            || infrastructure.acceptance_tests.is_empty()
        {
            let tests = self
                .generate_acceptance_tests(&trajectory, infrastructure)
                .await?;
            if !tests.is_empty() {
                tracing::info!("Generated {} acceptance tests", tests.len());
            }
        }

        // Detect test contradictions
        let contradictions = self
            .detect_test_contradictions(&trajectory, infrastructure)
            .await?;
        if !contradictions.is_empty() {
            self.emit_event(ConvergenceEvent::SpecificationAmbiguityDetected {
                task_id: trajectory.task_id.to_string(),
                contradictions: contradictions.clone(),
                suggested_clarifications: vec![],
            });
        }

        // Infer invariants
        if infrastructure.invariants.is_empty() {
            let invariants = self
                .infer_invariants(&trajectory, infrastructure)
                .await?;
            tracing::debug!("Inferred {} invariants", invariants.len());
        }

        // Emit TrajectoryStarted event
        self.emit_event(ConvergenceEvent::TrajectoryStarted {
            trajectory_id: trajectory.id.to_string(),
            task_id: trajectory.task_id.to_string(),
            goal_id: trajectory.goal_id.map(|id| id.to_string()),
            budget: trajectory.budget.clone(),
            timestamp: Utc::now(),
        });

        // -- DECIDE phase --
        // Check for proactive decomposition
        if self.config.enable_proactive_decomposition {
            if let Some(outcome) = self
                .maybe_decompose_proactively(&mut trajectory)
                .await?
            {
                return Ok(outcome);
            }
        }

        // Select convergence mode
        let basin = estimate_basin_width(
            &trajectory.specification.effective.content,
            !trajectory.specification.effective.success_criteria.is_empty(),
            false,
            !trajectory.specification.effective.constraints.is_empty(),
            !trajectory.specification.effective.anti_patterns.is_empty(),
            false,
        );
        let _mode = select_convergence_mode(&basin, &trajectory.policy, None);

        // -- ITERATE phase --
        trajectory.phase = ConvergencePhase::Iterating;

        // Initialize strategy bandit
        let mut bandit = self.initialize_bandit(&trajectory).await;

        loop {
            // a. Check context degradation (spec 6.4 pre-check)
            if context_is_degraded(
                &trajectory.observations,
                trajectory.total_fresh_starts,
                trajectory.policy.max_fresh_starts,
            ) {
                let health = estimate_context_health(&trajectory.observations);

                self.emit_event(ConvergenceEvent::ContextDegradationDetected {
                    trajectory_id: trajectory.id.to_string(),
                    health_score: health.signal_to_noise,
                    fresh_start_number: trajectory.total_fresh_starts + 1,
                });

                // Force a fresh start strategy
                let carry = extract_carry_forward(
                    &trajectory.observations,
                    trajectory.specification.effective.clone(),
                    &trajectory.hints,
                    |obs| {
                        obs.iter()
                            .filter(|o| o.metrics.is_some())
                            .max_by(|a, b| {
                                let al = a.metrics.as_ref().unwrap().convergence_level;
                                let bl = b.metrics.as_ref().unwrap().convergence_level;
                                al.partial_cmp(&bl).unwrap_or(std::cmp::Ordering::Equal)
                            })
                    },
                );
                trajectory.forced_strategy = Some(StrategyKind::FreshStart {
                    carry_forward: carry,
                });
                trajectory.total_fresh_starts += 1;
            }

            // b. Select strategy (forced or bandit-selected)
            let attractor = &trajectory.attractor_state;
            let strategy = if let Some(forced) = trajectory.forced_strategy.take() {
                forced
            } else {
                let eligible = eligible_strategies(
                    &trajectory.strategy_log,
                    attractor,
                    &trajectory.budget,
                    trajectory.total_fresh_starts,
                    trajectory.policy.max_fresh_starts,
                );

                if eligible.is_empty() {
                    // No strategies available -- trapped
                    let outcome = ConvergenceOutcome::Trapped {
                        trajectory_id: trajectory.id.to_string(),
                        attractor_type: attractor.classification.clone(),
                    };
                    self.finalize(&mut trajectory, &outcome, &bandit).await?;
                    return Ok(outcome);
                }

                bandit.select(&attractor.classification, &eligible, &trajectory.policy)
            };

            // Emit StrategySelected event
            self.emit_event(ConvergenceEvent::StrategySelected {
                trajectory_id: trajectory.id.to_string(),
                strategy: strategy.clone(),
                attractor_type: attractor.classification.clone(),
                reason: format!(
                    "Selected {} for {} attractor",
                    strategy.kind_name(),
                    self.attractor_type_name(&attractor.classification)
                ),
                budget_remaining_fraction: trajectory.budget.remaining_fraction(),
            });

            // c. Execute strategy -> produce artifact
            let (artifact, tokens_used, _wall_time_ms) =
                self.execute_strategy(&strategy, &trajectory).await?;

            // d. Measure with overseers
            let mut observation =
                self.measure_artifact(&artifact, &strategy, &trajectory).await?;
            observation.tokens_used = tokens_used;

            // e-i. Run the full iteration body (metrics, classification, bandit, loop control)
            let control = self
                .iterate_once(&mut trajectory, &mut bandit, &strategy, observation)
                .await?;

            match control {
                LoopControl::Converged => {
                    let final_seq = trajectory
                        .observations
                        .last()
                        .map(|o| o.sequence)
                        .unwrap_or(0);
                    let outcome = ConvergenceOutcome::Converged {
                        trajectory_id: trajectory.id.to_string(),
                        final_observation_sequence: final_seq,
                    };
                    self.finalize(&mut trajectory, &outcome, &bandit).await?;
                    return Ok(outcome);
                }
                LoopControl::Exhausted => {
                    // Try extension first
                    if self.request_extension(&mut trajectory).await? {
                        continue;
                    }
                    // Check partial acceptance
                    if trajectory.policy.partial_acceptance {
                        if let Some(best) = trajectory.best_observation() {
                            let level = best
                                .metrics
                                .as_ref()
                                .map(|m| m.convergence_level)
                                .unwrap_or(0.0);
                            if level >= trajectory.policy.partial_threshold {
                                let outcome = ConvergenceOutcome::Converged {
                                    trajectory_id: trajectory.id.to_string(),
                                    final_observation_sequence: best.sequence,
                                };
                                self.finalize(&mut trajectory, &outcome, &bandit)
                                    .await?;
                                return Ok(outcome);
                            }
                        }
                    }
                    let outcome = ConvergenceOutcome::Exhausted {
                        trajectory_id: trajectory.id.to_string(),
                        best_observation_sequence: trajectory
                            .best_observation()
                            .map(|o| o.sequence),
                    };
                    self.finalize(&mut trajectory, &outcome, &bandit).await?;
                    return Ok(outcome);
                }
                LoopControl::Trapped => {
                    let outcome = ConvergenceOutcome::Trapped {
                        trajectory_id: trajectory.id.to_string(),
                        attractor_type: trajectory.attractor_state.classification.clone(),
                    };
                    self.finalize(&mut trajectory, &outcome, &bandit).await?;
                    return Ok(outcome);
                }
                LoopControl::Decompose => {
                    return self.decompose_and_coordinate(&mut trajectory).await;
                }
                LoopControl::RequestExtension => {
                    if !self.request_extension(&mut trajectory).await? {
                        let outcome = ConvergenceOutcome::BudgetDenied {
                            trajectory_id: trajectory.id.to_string(),
                        };
                        self.finalize(&mut trajectory, &outcome, &bandit).await?;
                        return Ok(outcome);
                    }
                    continue;
                }
                LoopControl::Continue => continue,
            }
        }
    }

    /// The full iteration loop body.
    ///
    /// Handles a single iteration: records budget consumption, computes
    /// convergence metrics, classifies the attractor, updates the bandit,
    /// persists state, and checks loop control.
    pub async fn iterate_once(
        &self,
        trajectory: &mut Trajectory,
        bandit: &mut StrategyBandit,
        strategy: &StrategyKind,
        observation: Observation,
    ) -> DomainResult<LoopControl> {
        // Record budget consumption
        trajectory
            .budget
            .consume(observation.tokens_used, observation.wall_time_ms);

        // Record the strategy entry
        let mut entry = StrategyEntry::new(
            strategy.clone(),
            observation.sequence,
            observation.tokens_used,
            trajectory.forced_strategy.is_some(),
        );

        // Compute convergence metrics if we have a previous observation
        let mut obs_with_metrics = observation;
        if let Some(prev) = trajectory.observations.last() {
            let health = estimate_context_health(&trajectory.observations);
            let delta = compute_convergence_delta(
                prev,
                &obs_with_metrics.overseer_signals,
                obs_with_metrics
                    .metrics
                    .as_ref()
                    .map(|m| m.ast_diff_nodes)
                    .unwrap_or(0),
                &health,
                &ConvergenceWeights::default(),
            );
            let level = convergence_level(&obs_with_metrics.overseer_signals);
            let metrics = ObservationMetrics {
                convergence_delta: delta,
                convergence_level: level,
                ..ObservationMetrics::default()
            };
            obs_with_metrics = obs_with_metrics.with_metrics(metrics);
            entry = entry.with_delta(delta);
        }

        // Run intent verification if scheduled
        if self.should_verify(trajectory) {
            // verification = self.verify_intent(&trajectory).await?;
            // obs_with_metrics = obs_with_metrics.with_verification(verification);
        }

        // Push observation
        trajectory.observations.push(obs_with_metrics);
        trajectory.strategy_log.push(entry);

        // Emit ObservationRecorded
        if let Some(obs) = trajectory.observations.last() {
            let (delta, level) = obs
                .metrics
                .as_ref()
                .map(|m| (m.convergence_delta, m.convergence_level))
                .unwrap_or((0.0, 0.0));

            self.emit_event(ConvergenceEvent::ObservationRecorded {
                trajectory_id: trajectory.id.to_string(),
                observation_sequence: obs.sequence,
                convergence_delta: delta,
                convergence_level: level,
                strategy_used: obs.strategy_used.clone(),
                budget_remaining_fraction: trajectory.budget.remaining_fraction(),
            });
        }

        // Classify attractor
        trajectory.attractor_state = classify_attractor(&trajectory.observations, 5);

        self.emit_event(ConvergenceEvent::AttractorClassified {
            trajectory_id: trajectory.id.to_string(),
            attractor_type: trajectory.attractor_state.classification.clone(),
            confidence: trajectory.attractor_state.confidence,
        });

        // Update bandit
        if let Some(obs) = trajectory.observations.last() {
            bandit.update(strategy, &trajectory.attractor_state.classification, obs);
        }

        // Update context health
        trajectory.context_health = estimate_context_health(&trajectory.observations);

        // Persist trajectory
        self.trajectory_store.save(trajectory).await?;

        // Check loop control
        self.check_loop_control(trajectory, bandit)
    }

    // -----------------------------------------------------------------------
    // 6.4 check_loop_control -- Determine if the loop should continue
    // -----------------------------------------------------------------------

    /// Determine whether the convergence loop should continue (spec 6.4).
    ///
    /// Evaluates, in priority order:
    /// 1. Budget exhausted -> Exhausted or RequestExtension
    /// 2. Converged -> acceptance threshold met
    /// 3. Trapped -> limit cycle with no escape strategies
    /// 4. Decompose -> convergence delta suggests decomposition
    /// 5. Continue -> keep iterating
    pub fn check_loop_control(
        &self,
        trajectory: &Trajectory,
        _bandit: &StrategyBandit,
    ) -> DomainResult<LoopControl> {
        // 1. Budget exhaustion check
        if !trajectory.budget.has_remaining() {
            // Check if we should request an extension
            let delta_positive = trajectory.latest_convergence_delta() > 0.0;
            if trajectory
                .budget
                .should_request_extension(delta_positive)
            {
                return Ok(LoopControl::RequestExtension);
            }
            return Ok(LoopControl::Exhausted);
        }

        // 2. Convergence check -- has the trajectory reached the acceptance threshold?
        if let Some(obs) = trajectory.observations.last() {
            if let Some(ref metrics) = obs.metrics {
                if metrics.convergence_level >= trajectory.policy.acceptance_threshold {
                    // Check if overseer signals are all passing
                    if obs.overseer_signals.all_passing() {
                        return Ok(LoopControl::Converged);
                    }
                    // High level but not all passing -- keep iterating
                }
            }
        }

        // 3. Trapped check -- limit cycle with no eligible escape strategies
        if let AttractorType::LimitCycle { .. } = &trajectory.attractor_state.classification {
            let eligible = eligible_strategies(
                &trajectory.strategy_log,
                &trajectory.attractor_state,
                &trajectory.budget,
                trajectory.total_fresh_starts,
                trajectory.policy.max_fresh_starts,
            );
            if eligible.is_empty() {
                return Ok(LoopControl::Trapped);
            }
        }

        // 4. Decomposition check -- divergent trajectory with sufficient budget
        if let AttractorType::Divergent { .. } = &trajectory.attractor_state.classification {
            // If the trajectory has been diverging for a while and the budget
            // allows decomposition, suggest it
            let divergent_observations = trajectory
                .observations
                .iter()
                .rev()
                .take(3)
                .filter(|o| {
                    o.metrics
                        .as_ref()
                        .map(|m| m.convergence_delta < -0.05)
                        .unwrap_or(false)
                })
                .count();
            if divergent_observations >= 3
                && trajectory.budget.allows_strategy_cost(&StrategyKind::Decompose)
            {
                return Ok(LoopControl::Decompose);
            }
        }

        // 5. Near-budget extension check
        let delta_positive = trajectory.latest_convergence_delta() > 0.0;
        if trajectory
            .budget
            .should_request_extension(delta_positive)
        {
            return Ok(LoopControl::RequestExtension);
        }

        // Default: continue iterating
        Ok(LoopControl::Continue)
    }

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
    pub async fn request_extension(
        &self,
        trajectory: &mut Trajectory,
    ) -> DomainResult<bool> {
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

        // In a real implementation, this would pause at an InterventionPoint
        // and wait for user approval. For now, we auto-approve if the
        // trajectory is approaching a fixed point.
        let approved = matches!(
            &trajectory.attractor_state.classification,
            AttractorType::FixedPoint { .. }
        );

        if approved {
            trajectory.budget.extend(additional_tokens, additional_iterations);
            self.emit_event(ConvergenceEvent::BudgetExtensionGranted {
                trajectory_id: trajectory.id.to_string(),
                additional_tokens,
            });
            self.trajectory_store.save(trajectory).await?;
            Ok(true)
        } else {
            self.emit_event(ConvergenceEvent::BudgetExtensionDenied {
                trajectory_id: trajectory.id.to_string(),
                reason: "Trajectory is not approaching a fixed point".to_string(),
            });
            Ok(false)
        }
    }

    // -----------------------------------------------------------------------
    // 6.6 converge_parallel -- Parallel trajectory sampling
    // -----------------------------------------------------------------------

    /// Run parallel trajectory sampling (spec 6.6).
    ///
    /// Spawns `sample_count` independent trajectories and selects the best
    /// outcome. Each sample gets a fraction of the parent budget.
    pub async fn converge_parallel(
        &self,
        submission: &TaskSubmission,
        sample_count: u32,
    ) -> DomainResult<ConvergenceOutcome> {
        let (base_trajectory, infrastructure) = self.prepare(submission).await?;

        self.emit_event(ConvergenceEvent::ParallelConvergenceStarted {
            trajectory_id: base_trajectory.id.to_string(),
            parallel_count: sample_count as usize,
        });

        // Create sample trajectories, each with a fraction of the budget
        let mut outcomes = Vec::new();
        for _ in 0..sample_count {
            let mut sample = base_trajectory.clone();
            sample.id = Uuid::new_v4();
            // Each sample gets 1/N of the budget
            sample.budget = base_trajectory
                .budget
                .scale(1.0 / sample_count as f64);

            let outcome = self.converge(sample, &infrastructure).await?;
            outcomes.push(outcome);
        }

        // Select the best outcome: prefer Converged, then best Exhausted
        let best = outcomes
            .into_iter()
            .min_by_key(|o| match o {
                ConvergenceOutcome::Converged { .. } => 0,
                ConvergenceOutcome::Decomposed { .. } => 1,
                ConvergenceOutcome::Exhausted { .. } => 2,
                ConvergenceOutcome::BudgetDenied { .. } => 3,
                ConvergenceOutcome::Trapped { .. } => 4,
            })
            .unwrap_or(ConvergenceOutcome::Exhausted {
                trajectory_id: base_trajectory.id.to_string(),
                best_observation_sequence: None,
            });

        Ok(best)
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
                let children: Vec<Uuid> = child_trajectory_ids
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
                    self.store_failure_memory(trajectory, "budget_denied").await?;
                }
            }
        }

        // 4. Persist bandit state
        if self.config.memory_enabled {
            self.persist_bandit_state(bandit, trajectory).await?;
        }

        // 5. Save final trajectory state
        trajectory.updated_at = Utc::now();
        self.trajectory_store.save(trajectory).await?;

        Ok(())
    }

    // -----------------------------------------------------------------------
    // should_verify -- When to run intent verification
    // -----------------------------------------------------------------------

    /// Determine whether intent verification should run on this iteration.
    ///
    /// Verification runs when:
    /// 1. The observation count is a multiple of `policy.intent_verification_frequency`.
    /// 2. The convergence level has crossed a significant threshold (0.5, 0.8, 0.9).
    /// 3. The attractor classification just changed to FixedPoint.
    pub fn should_verify(&self, trajectory: &Trajectory) -> bool {
        let obs_count = trajectory.observations.len() as u32;

        // Always verify on the first observation
        if obs_count == 0 {
            return false;
        }

        // Check frequency
        if trajectory.policy.intent_verification_frequency > 0
            && obs_count % trajectory.policy.intent_verification_frequency == 0
        {
            return true;
        }

        // Check threshold crossings
        if trajectory.observations.len() >= 2 {
            let current_level = trajectory
                .observations
                .last()
                .and_then(|o| o.metrics.as_ref())
                .map(|m| m.convergence_level)
                .unwrap_or(0.0);
            let prev_level = trajectory
                .observations
                .get(trajectory.observations.len() - 2)
                .and_then(|o| o.metrics.as_ref())
                .map(|m| m.convergence_level)
                .unwrap_or(0.0);

            let thresholds = [0.5, 0.8, 0.9];
            for threshold in &thresholds {
                if prev_level < *threshold && current_level >= *threshold {
                    return true;
                }
            }
        }

        // Check if we just transitioned to FixedPoint
        if matches!(
            &trajectory.attractor_state.classification,
            AttractorType::FixedPoint { .. }
        ) {
            // Only verify if we have enough observations to have a meaningful
            // prior classification (at least 4 observations for classification
            // + 1 for transition detection)
            if trajectory.observations.len() >= 5 {
                return true;
            }
        }

        false
    }

    // -----------------------------------------------------------------------
    // 9.2 decompose_and_coordinate -- Decomposition flow
    // -----------------------------------------------------------------------

    /// Decompose a task into subtasks and coordinate their convergence (spec 9.2).
    ///
    /// 1. Propose decomposition.
    /// 2. Allocate budgets for subtasks.
    /// 3. Create child trajectories.
    /// 4. Emit DecompositionTriggered event.
    /// 5. Return Decomposed outcome.
    pub async fn decompose_and_coordinate(
        &self,
        trajectory: &mut Trajectory,
    ) -> DomainResult<ConvergenceOutcome> {
        // Propose decomposition
        let decomposition = self.propose_decomposition(trajectory);

        // Allocate budgets
        let child_budgets = allocate_decomposed_budget(&trajectory.budget, &decomposition);

        // Create child trajectories
        let mut child_ids = Vec::new();
        for (subtask, budget) in decomposition.iter().zip(child_budgets.iter()) {
            let spec = SpecificationEvolution::new(subtask.specification.clone());
            let child = Trajectory::new(
                trajectory.task_id,
                trajectory.goal_id,
                spec,
                budget.clone(),
                trajectory.policy.clone(),
            );
            child_ids.push(child.id.to_string());
            self.trajectory_store.save(&child).await?;
        }

        self.emit_event(ConvergenceEvent::DecompositionTriggered {
            parent_trajectory_id: trajectory.id.to_string(),
            child_count: child_ids.len(),
        });

        Ok(ConvergenceOutcome::Decomposed {
            parent_trajectory_id: trajectory.id.to_string(),
            child_trajectory_ids: child_ids,
        })
    }

    // -----------------------------------------------------------------------
    // 9.1 maybe_decompose_proactively -- Proactive decomposition check
    // -----------------------------------------------------------------------

    /// Check whether the task should be proactively decomposed (spec 9.1).
    ///
    /// This runs during the DECIDE phase before entering the iteration loop.
    /// A task is a candidate for proactive decomposition when:
    /// - The basin is narrow (many starting points do not converge).
    /// - The estimated convergence cost exceeds the allocated budget.
    /// - The task complexity is Complex.
    ///
    /// Returns `Some(outcome)` if decomposition was triggered, `None` to proceed.
    pub async fn maybe_decompose_proactively(
        &self,
        trajectory: &mut Trajectory,
    ) -> DomainResult<Option<ConvergenceOutcome>> {
        let basin = estimate_basin_width(
            &trajectory.specification.effective.content,
            !trajectory.specification.effective.success_criteria.is_empty(),
            false,
            !trajectory.specification.effective.constraints.is_empty(),
            !trajectory.specification.effective.anti_patterns.is_empty(),
            false,
        );

        // Only consider proactive decomposition for narrow basins
        if basin.classification != BasinClassification::Narrow {
            return Ok(None);
        }

        // Estimate convergence cost
        let estimate =
            estimate_convergence_heuristic(Complexity::Complex, &basin);

        // If the estimated cost exceeds the budget, recommend decomposition
        if estimate.expected_tokens > trajectory.budget.max_tokens
            || estimate.convergence_probability < 0.4
        {
            self.emit_event(ConvergenceEvent::DecompositionRecommended {
                task_id: trajectory.task_id.to_string(),
                subtask_count: 0, // Unknown until decomposition is proposed
                savings_estimate: 1.0 - estimate.convergence_probability,
            });

            let outcome = self.decompose_and_coordinate(trajectory).await?;
            return Ok(Some(outcome));
        }

        Ok(None)
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

        let memory = Memory::semantic(
            format!("convergence-success-{}", trajectory.id),
            content,
        )
        .with_namespace("convergence")
        .with_type(MemoryType::Pattern)
        .with_source("convergence_engine")
        .with_task(trajectory.task_id)
        .with_tag("convergence")
        .with_tag("success");

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

        let memory = Memory::episodic(
            format!("convergence-failure-{}", trajectory.id),
            content,
        )
        .with_namespace("convergence")
        .with_type(MemoryType::Error)
        .with_source("convergence_engine")
        .with_task(trajectory.task_id)
        .with_tag("convergence")
        .with_tag("failure");

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

        let memory = Memory::semantic(
            format!("strategy-bandit-{}", trajectory.task_id),
            content,
        )
        .with_namespace("convergence")
        .with_type(MemoryType::Pattern)
        .with_source("convergence_engine")
        .with_task(trajectory.task_id)
        .with_tag("strategy-bandit");

        self.memory_repository.store(&memory).await?;

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
                            tracing::warn!(
                                "Failed to deserialize bandit state: {}; using defaults",
                                e
                            );
                        }
                    }
                }
            }
            Err(e) => {
                tracing::warn!(
                    "Failed to query bandit memory: {}; using defaults",
                    e
                );
            }
        }

        StrategyBandit::with_default_priors()
    }

    // -----------------------------------------------------------------------
    // Strategy execution
    // -----------------------------------------------------------------------

    /// Execute a strategy to produce an artifact.
    ///
    /// Builds strategy-specific context, logs the execution, and returns an
    /// artifact reference along with token and time cost. The actual LLM
    /// execution is delegated to the agent runtime; this method records the
    /// strategy context and returns the artifact from the trajectory's
    /// worktree.
    ///
    /// Returns `(artifact, tokens_used, wall_time_ms)`.
    async fn execute_strategy(
        &self,
        strategy: &StrategyKind,
        trajectory: &Trajectory,
    ) -> DomainResult<(ArtifactReference, u64, u64)> {
        let start = std::time::Instant::now();

        // Build strategy-specific context
        let _context = self.build_strategy_context(strategy, trajectory);

        // Log the strategy execution
        tracing::info!(
            strategy = strategy.kind_name(),
            trajectory_id = %trajectory.id,
            "Executing convergence strategy"
        );

        // For strategies that modify the specification (ArchitectReview, FreshStart),
        // handle their side effects
        match strategy {
            StrategyKind::FreshStart { carry_forward } => {
                // Fresh start resets context but preserves filesystem and trajectory metadata
                tracing::info!(
                    trajectory_id = %trajectory.id,
                    "Fresh start: carrying forward {} hints, best level from {} observations",
                    carry_forward.hints.len(),
                    trajectory.observations.len(),
                );
            }
            StrategyKind::RevertAndBranch { target } => {
                tracing::info!(
                    trajectory_id = %trajectory.id,
                    target = %target,
                    "Reverting to observation {} and branching",
                    target,
                );
            }
            _ => {}
        }

        // The actual LLM execution is delegated to the caller / agent runtime.
        // The engine records the strategy context and expects the artifact to be
        // produced by the runtime. For now, return the latest artifact or a
        // placeholder. In integration, this would await the agent's execution.
        let artifact = trajectory
            .latest_artifact()
            .cloned()
            .unwrap_or_else(|| {
                ArtifactReference::new(
                    format!("/worktree/{}/artifact", trajectory.task_id),
                    format!("pending-{}", trajectory.observations.len()),
                )
            });

        let elapsed = start.elapsed();
        let estimated_tokens = strategy.estimated_cost();

        Ok((artifact, estimated_tokens, elapsed.as_millis() as u64))
    }

    /// Build strategy-specific context for a strategy execution.
    ///
    /// Assembles the prompt fragments, carry-forward data, hints, and focus
    /// areas based on the strategy type and current trajectory state.
    fn build_strategy_context(
        &self,
        strategy: &StrategyKind,
        trajectory: &Trajectory,
    ) -> StrategyContext {
        let latest_signals = trajectory.latest_overseer_signals().cloned();

        let carry_forward = match strategy {
            StrategyKind::FreshStart { carry_forward } => Some(carry_forward.clone()),
            _ => None,
        };

        // Build hints from trajectory hints and strategy-specific guidance
        let mut hints = trajectory.hints.clone();
        match strategy {
            StrategyKind::FocusedRepair => {
                // Add failing test names as focus hints
                if let Some(signals) = &latest_signals {
                    if let Some(ref tests) = signals.test_results {
                        for test_name in &tests.failing_test_names {
                            hints.push(format!("Focus on fixing: {}", test_name));
                        }
                    }
                }
            }
            StrategyKind::Reframe => {
                hints.push(
                    "Approach the problem from a different angle; \
                     restructure rather than patch"
                        .to_string(),
                );
            }
            StrategyKind::AlternativeApproach => {
                hints.push(
                    "Try a fundamentally different implementation strategy".to_string(),
                );
            }
            StrategyKind::IncrementalRefinement => {
                hints.push("Address one gap at a time, smallest first".to_string());
            }
            _ => {}
        }

        // Build focus areas from recent overseer feedback
        let mut focus_areas = Vec::new();
        if let Some(signals) = &latest_signals {
            if let Some(ref build) = signals.build_result {
                if !build.success {
                    focus_areas.push("Fix build errors first".to_string());
                    for error in &build.errors {
                        focus_areas.push(format!("Build error: {}", error));
                    }
                }
            }
            if let Some(ref tc) = signals.type_check {
                if !tc.clean {
                    focus_areas.push("Resolve type errors".to_string());
                }
            }
        }

        StrategyContext {
            strategy: strategy.clone(),
            specification: trajectory.specification.effective.clone(),
            latest_signals,
            carry_forward,
            hints,
            focus_areas,
        }
    }

    // -----------------------------------------------------------------------
    // Overseer measurement
    // -----------------------------------------------------------------------

    /// Measure an artifact with overseers using the OverseerMeasurer trait.
    ///
    /// Delegates to the injected `overseer_measurer` implementation, then
    /// constructs an `Observation` from the resulting signals.
    async fn measure_artifact(
        &self,
        artifact: &ArtifactReference,
        strategy: &StrategyKind,
        trajectory: &Trajectory,
    ) -> DomainResult<Observation> {
        let start = std::time::Instant::now();

        let overseer_signals = self
            .overseer_measurer
            .measure(artifact, &trajectory.policy)
            .await?;

        let elapsed = start.elapsed();
        let sequence = trajectory.observations.len() as u32;

        let observation = Observation::new(
            sequence,
            artifact.clone(),
            overseer_signals,
            strategy.clone(),
            0, // tokens tracked at strategy execution
            elapsed.as_millis() as u64,
        );

        Ok(observation)
    }

    // -----------------------------------------------------------------------
    // LLM-dependent methods (graceful fallback without LLM)
    // -----------------------------------------------------------------------

    /// Generate acceptance tests from the specification.
    ///
    /// When no LLM substrate is available, returns the tests already
    /// discovered in the infrastructure. When an LLM is integrated, this
    /// would generate additional acceptance tests from the specification.
    async fn generate_acceptance_tests(
        &self,
        trajectory: &Trajectory,
        infrastructure: &ConvergenceInfrastructure,
    ) -> DomainResult<Vec<String>> {
        tracing::debug!(
            trajectory_id = %trajectory.id,
            "Acceptance test generation: using {} discovered tests",
            infrastructure.acceptance_tests.len(),
        );
        Ok(infrastructure.acceptance_tests.clone())
    }

    /// Infer invariants from the specification.
    ///
    /// When no LLM substrate is available, returns the invariants already
    /// discovered in the infrastructure.
    async fn infer_invariants(
        &self,
        trajectory: &Trajectory,
        infrastructure: &ConvergenceInfrastructure,
    ) -> DomainResult<Vec<String>> {
        tracing::debug!(
            trajectory_id = %trajectory.id,
            "Invariant inference: using {} discovered invariants",
            infrastructure.invariants.len(),
        );
        Ok(infrastructure.invariants.clone())
    }

    /// Detect test contradictions.
    ///
    /// Test contradiction detection requires LLM analysis. Returns empty
    /// when LLM is not available.
    async fn detect_test_contradictions(
        &self,
        _trajectory: &Trajectory,
        _infrastructure: &ConvergenceInfrastructure,
    ) -> DomainResult<Vec<String>> {
        Ok(vec![])
    }

    /// Propose a task decomposition.
    ///
    /// Without an LLM, creates a simple 2-way split based on the
    /// specification content.
    fn propose_decomposition(&self, trajectory: &Trajectory) -> Vec<TaskDecomposition> {
        let spec = &trajectory.specification.effective;
        vec![
            TaskDecomposition {
                subtask_id: Uuid::new_v4().to_string(),
                description: format!("Part 1 of: {}", spec.content),
                specification: SpecificationSnapshot::new(format!(
                    "Part 1 of: {}",
                    spec.content
                )),
                budget_fraction: 0.5,
                dependencies: vec![],
            },
            TaskDecomposition {
                subtask_id: Uuid::new_v4().to_string(),
                description: format!("Part 2 of: {}", spec.content),
                specification: SpecificationSnapshot::new(format!(
                    "Part 2 of: {}",
                    spec.content
                )),
                budget_fraction: 0.5,
                dependencies: vec![],
            },
        ]
    }

    // -----------------------------------------------------------------------
    // Internal helpers
    // -----------------------------------------------------------------------

    /// Emit a convergence event if event emission is enabled.
    fn emit_event(&self, event: ConvergenceEvent) {
        if self.config.event_emission_enabled {
            tracing::info!(
                event_name = event.event_name(),
                trajectory_id = ?event.trajectory_id(),
                "Convergence event: {}",
                event.event_name()
            );
        }
    }

    /// Get a human-readable name for an attractor type.
    fn attractor_type_name(&self, attractor: &AttractorType) -> &'static str {
        match attractor {
            AttractorType::FixedPoint { .. } => "fixed_point",
            AttractorType::LimitCycle { .. } => "limit_cycle",
            AttractorType::Divergent { .. } => "divergent",
            AttractorType::Plateau { .. } => "plateau",
            AttractorType::Indeterminate { .. } => "indeterminate",
        }
    }
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use crate::domain::models::MemoryTier;
    use std::collections::HashMap;
    use std::sync::Mutex;

    // -----------------------------------------------------------------------
    // Mock TrajectoryRepository
    // -----------------------------------------------------------------------

    struct MockTrajectoryRepo {
        trajectories: Mutex<HashMap<String, Trajectory>>,
    }

    impl MockTrajectoryRepo {
        fn new() -> Self {
            Self {
                trajectories: Mutex::new(HashMap::new()),
            }
        }
    }

    #[async_trait]
    impl TrajectoryRepository for MockTrajectoryRepo {
        async fn save(&self, trajectory: &Trajectory) -> DomainResult<()> {
            let mut map = self.trajectories.lock().unwrap();
            map.insert(trajectory.id.to_string(), trajectory.clone());
            Ok(())
        }

        async fn get(&self, trajectory_id: &str) -> DomainResult<Option<Trajectory>> {
            let map = self.trajectories.lock().unwrap();
            Ok(map.get(trajectory_id).cloned())
        }

        async fn get_by_task(&self, _task_id: &str) -> DomainResult<Vec<Trajectory>> {
            Ok(vec![])
        }

        async fn get_by_goal(&self, _goal_id: &str) -> DomainResult<Vec<Trajectory>> {
            Ok(vec![])
        }

        async fn get_recent(&self, _limit: usize) -> DomainResult<Vec<Trajectory>> {
            Ok(vec![])
        }

        async fn get_successful_strategies(
            &self,
            _attractor_type: &AttractorType,
            _limit: usize,
        ) -> DomainResult<Vec<StrategyEntry>> {
            Ok(vec![])
        }

        async fn delete(&self, _trajectory_id: &str) -> DomainResult<()> {
            Ok(())
        }

        async fn avg_iterations_by_complexity(&self, _complexity: Complexity) -> DomainResult<f64> {
            Ok(0.0)
        }

        async fn strategy_effectiveness(
            &self,
            _strategy: StrategyKind,
        ) -> DomainResult<crate::domain::ports::trajectory_repository::StrategyStats> {
            Ok(crate::domain::ports::trajectory_repository::StrategyStats {
                strategy: String::new(),
                total_uses: 0,
                success_count: 0,
                average_delta: 0.0,
                average_tokens: 0,
            })
        }

        async fn attractor_distribution(&self) -> DomainResult<HashMap<String, u32>> {
            Ok(HashMap::new())
        }

        async fn convergence_rate_by_task_type(&self, _category: &str) -> DomainResult<f64> {
            Ok(0.0)
        }

        async fn get_similar_trajectories(
            &self,
            _description: &str,
            _tags: &[String],
            _limit: usize,
        ) -> DomainResult<Vec<Trajectory>> {
            Ok(vec![])
        }
    }

    // -----------------------------------------------------------------------
    // Mock MemoryRepository
    // -----------------------------------------------------------------------

    struct MockMemoryRepo {
        memories: Mutex<Vec<Memory>>,
    }

    impl MockMemoryRepo {
        fn new() -> Self {
            Self {
                memories: Mutex::new(Vec::new()),
            }
        }
    }

    #[async_trait]
    impl MemoryRepository for MockMemoryRepo {
        async fn store(&self, memory: &Memory) -> DomainResult<()> {
            let mut memories = self.memories.lock().unwrap();
            memories.push(memory.clone());
            Ok(())
        }

        async fn get(&self, _id: Uuid) -> DomainResult<Option<Memory>> {
            Ok(None)
        }

        async fn get_by_key(
            &self,
            _key: &str,
            _namespace: &str,
        ) -> DomainResult<Option<Memory>> {
            Ok(None)
        }

        async fn update(&self, _memory: &Memory) -> DomainResult<()> {
            Ok(())
        }

        async fn delete(&self, _id: Uuid) -> DomainResult<()> {
            Ok(())
        }

        async fn query(&self, query: MemoryQuery) -> DomainResult<Vec<Memory>> {
            let memories = self.memories.lock().unwrap();
            let results: Vec<Memory> = memories
                .iter()
                .filter(|m| {
                    if !query.tags.is_empty() {
                        query
                            .tags
                            .iter()
                            .any(|t| m.metadata.tags.contains(t))
                    } else {
                        true
                    }
                })
                .cloned()
                .collect();
            Ok(results)
        }

        async fn search(
            &self,
            _query: &str,
            _namespace: Option<&str>,
            _limit: usize,
        ) -> DomainResult<Vec<Memory>> {
            Ok(vec![])
        }

        async fn list_by_tier(&self, _tier: MemoryTier) -> DomainResult<Vec<Memory>> {
            Ok(vec![])
        }

        async fn list_by_namespace(&self, _namespace: &str) -> DomainResult<Vec<Memory>> {
            Ok(vec![])
        }

        async fn get_expired(&self) -> DomainResult<Vec<Memory>> {
            Ok(vec![])
        }

        async fn prune_expired(&self) -> DomainResult<u64> {
            Ok(0)
        }

        async fn get_decayed(&self, _threshold: f32) -> DomainResult<Vec<Memory>> {
            Ok(vec![])
        }

        async fn get_for_task(&self, _task_id: Uuid) -> DomainResult<Vec<Memory>> {
            Ok(vec![])
        }

        async fn get_for_goal(&self, _goal_id: Uuid) -> DomainResult<Vec<Memory>> {
            Ok(vec![])
        }

        async fn count_by_tier(
            &self,
        ) -> DomainResult<std::collections::HashMap<MemoryTier, u64>> {
            Ok(HashMap::new())
        }
    }

    // -----------------------------------------------------------------------
    // Mock OverseerMeasurer
    // -----------------------------------------------------------------------

    struct MockOverseerMeasurer {
        signals: Mutex<OverseerSignals>,
    }

    impl MockOverseerMeasurer {
        fn new() -> Self {
            Self {
                signals: Mutex::new(OverseerSignals::default()),
            }
        }

        fn with_signals(signals: OverseerSignals) -> Self {
            Self {
                signals: Mutex::new(signals),
            }
        }
    }

    #[async_trait]
    impl OverseerMeasurer for MockOverseerMeasurer {
        async fn measure(
            &self,
            _artifact: &ArtifactReference,
            _policy: &ConvergencePolicy,
        ) -> DomainResult<OverseerSignals> {
            Ok(self.signals.lock().unwrap().clone())
        }
    }

    // -----------------------------------------------------------------------
    // Test helpers
    // -----------------------------------------------------------------------

    fn test_config() -> ConvergenceEngineConfig {
        ConvergenceEngineConfig {
            default_policy: ConvergencePolicy::default(),
            max_parallel_trajectories: 3,
            enable_proactive_decomposition: false,
            memory_enabled: true,
            event_emission_enabled: false,
        }
    }

    fn test_engine(
    ) -> ConvergenceEngine<MockTrajectoryRepo, MockMemoryRepo, MockOverseerMeasurer> {
        ConvergenceEngine::new(
            Arc::new(MockTrajectoryRepo::new()),
            Arc::new(MockMemoryRepo::new()),
            Arc::new(MockOverseerMeasurer::new()),
            test_config(),
        )
    }

    fn test_spec() -> SpecificationEvolution {
        SpecificationEvolution::new(SpecificationSnapshot::new(
            "Implement a REST API endpoint for user authentication".to_string(),
        ))
    }

    fn test_budget() -> ConvergenceBudget {
        ConvergenceBudget::default()
    }

    fn test_policy() -> ConvergencePolicy {
        ConvergencePolicy::default()
    }

    fn test_trajectory() -> Trajectory {
        Trajectory::new(
            Uuid::new_v4(),
            None,
            test_spec(),
            test_budget(),
            test_policy(),
        )
    }

    fn test_artifact(seq: u32) -> ArtifactReference {
        ArtifactReference::new(
            format!("/worktree/task/artifact_{}.rs", seq),
            format!("hash_{}", seq),
        )
    }

    fn test_observation(seq: u32, strategy: StrategyKind) -> Observation {
        Observation::new(
            seq,
            test_artifact(seq),
            OverseerSignals::default(),
            strategy,
            10_000,
            5_000,
        )
    }

    fn metrics_with(delta: f64, level: f64) -> ObservationMetrics {
        ObservationMetrics {
            convergence_delta: delta,
            convergence_level: level,
            ..ObservationMetrics::default()
        }
    }

    fn signals_with_tests(passed: u32, total: u32) -> OverseerSignals {
        OverseerSignals {
            test_results: Some(TestResults {
                passed,
                failed: total - passed,
                skipped: 0,
                total,
                regression_count: 0,
                failing_test_names: Vec::new(),
            }),
            build_result: Some(BuildResult {
                success: true,
                error_count: 0,
                errors: Vec::new(),
            }),
            type_check: Some(TypeCheckResult {
                clean: true,
                error_count: 0,
                errors: Vec::new(),
            }),
            ..OverseerSignals::default()
        }
    }

    // -----------------------------------------------------------------------
    // check_loop_control tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_loop_control_continue_when_budget_remains_and_no_observations() {
        let engine = test_engine();
        let trajectory = test_trajectory();
        let bandit = StrategyBandit::with_default_priors();

        let result = engine.check_loop_control(&trajectory, &bandit).unwrap();
        assert!(matches!(result, LoopControl::Continue));
    }

    #[test]
    fn test_loop_control_exhausted_when_budget_consumed() {
        let engine = test_engine();
        let mut trajectory = test_trajectory();
        trajectory.budget.tokens_used = trajectory.budget.max_tokens;
        trajectory.budget.iterations_used = trajectory.budget.max_iterations;
        // Also exhaust extensions so we don't get RequestExtension
        trajectory.budget.extensions_requested = trajectory.budget.max_extensions;
        let bandit = StrategyBandit::with_default_priors();

        let result = engine.check_loop_control(&trajectory, &bandit).unwrap();
        assert!(matches!(result, LoopControl::Exhausted));
    }

    #[test]
    fn test_loop_control_converged_when_threshold_met() {
        let engine = test_engine();
        let mut trajectory = test_trajectory();

        // Add an observation that meets the acceptance threshold with all passing signals
        let all_passing_signals = signals_with_tests(10, 10);
        let obs = Observation::new(
            0,
            test_artifact(0),
            all_passing_signals,
            StrategyKind::RetryWithFeedback,
            10_000,
            5_000,
        )
        .with_metrics(metrics_with(0.1, 0.95));
        trajectory.observations.push(obs);
        let bandit = StrategyBandit::with_default_priors();

        let result = engine.check_loop_control(&trajectory, &bandit).unwrap();
        assert!(matches!(result, LoopControl::Converged));
    }

    #[test]
    fn test_loop_control_not_converged_when_signals_not_passing() {
        let engine = test_engine();
        let mut trajectory = test_trajectory();

        // High level but tests are failing
        let failing_signals = signals_with_tests(5, 10);
        let obs = Observation::new(
            0,
            test_artifact(0),
            failing_signals,
            StrategyKind::RetryWithFeedback,
            10_000,
            5_000,
        )
        .with_metrics(metrics_with(0.1, 0.96));
        trajectory.observations.push(obs);
        let bandit = StrategyBandit::with_default_priors();

        let result = engine.check_loop_control(&trajectory, &bandit).unwrap();
        // Should NOT be converged because signals are not all passing
        assert!(
            !matches!(result, LoopControl::Converged),
            "Should not converge when signals are not all passing"
        );
    }

    #[test]
    fn test_loop_control_trapped_when_limit_cycle_no_strategies() {
        let engine = test_engine();
        let mut trajectory = test_trajectory();

        // Set up a limit cycle classification
        trajectory.attractor_state = AttractorState {
            classification: AttractorType::LimitCycle {
                period: 2,
                cycle_signatures: vec!["sig1".to_string(), "sig2".to_string()],
            },
            confidence: 0.85,
            detected_at: None,
            evidence: AttractorEvidence {
                recent_deltas: vec![],
                recent_signatures: vec![],
                rationale: String::new(),
            },
        };

        // Exhaust the budget so no strategies are eligible
        trajectory.budget.tokens_used = trajectory.budget.max_tokens - 1;
        // Use all iterations except 1
        trajectory.budget.iterations_used = trajectory.budget.max_iterations - 1;
        // Make the budget very tight so no strategies can afford it
        trajectory.budget.max_tokens = trajectory.budget.tokens_used + 100;

        let bandit = StrategyBandit::with_default_priors();
        let result = engine.check_loop_control(&trajectory, &bandit).unwrap();
        assert!(
            matches!(result, LoopControl::Trapped),
            "Expected Trapped, got {:?}",
            result
        );
    }

    #[test]
    fn test_loop_control_request_extension_when_converging_and_low_budget() {
        let engine = test_engine();
        let mut trajectory = test_trajectory();

        // Set up near-budget-exhaustion with positive delta
        trajectory.budget.tokens_used =
            (trajectory.budget.max_tokens as f64 * 0.9) as u64;

        // Add observation with positive delta
        let obs = test_observation(0, StrategyKind::RetryWithFeedback)
            .with_metrics(metrics_with(0.15, 0.7));
        trajectory.observations.push(obs);

        let bandit = StrategyBandit::with_default_priors();
        let result = engine.check_loop_control(&trajectory, &bandit).unwrap();
        assert!(
            matches!(result, LoopControl::RequestExtension),
            "Expected RequestExtension, got {:?}",
            result
        );
    }

    #[test]
    fn test_loop_control_no_extension_when_extensions_exhausted() {
        let engine = test_engine();
        let mut trajectory = test_trajectory();

        // Near budget exhaustion, converging, but extensions already requested
        trajectory.budget.tokens_used =
            (trajectory.budget.max_tokens as f64 * 0.9) as u64;
        trajectory.budget.extensions_requested = trajectory.budget.max_extensions;

        let obs = test_observation(0, StrategyKind::RetryWithFeedback)
            .with_metrics(metrics_with(0.15, 0.7));
        trajectory.observations.push(obs);

        let bandit = StrategyBandit::with_default_priors();
        let result = engine.check_loop_control(&trajectory, &bandit).unwrap();
        // Should continue since budget is not fully exhausted and extensions are used
        assert!(
            matches!(result, LoopControl::Continue),
            "Expected Continue, got {:?}",
            result
        );
    }

    #[test]
    fn test_loop_control_decompose_on_persistent_divergence() {
        let engine = test_engine();
        let mut trajectory = test_trajectory();

        // Set divergent attractor
        trajectory.attractor_state = AttractorState {
            classification: AttractorType::Divergent {
                divergence_rate: -0.1,
                probable_cause: DivergenceCause::WrongApproach,
            },
            confidence: 0.8,
            detected_at: None,
            evidence: AttractorEvidence {
                recent_deltas: vec![-0.1, -0.08, -0.12],
                recent_signatures: vec![],
                rationale: String::new(),
            },
        };

        // Add 3 observations with negative deltas
        for i in 0..3 {
            let obs = test_observation(i, StrategyKind::RetryWithFeedback)
                .with_metrics(metrics_with(-0.1, 0.3));
            trajectory.observations.push(obs);
        }

        let bandit = StrategyBandit::with_default_priors();
        let result = engine.check_loop_control(&trajectory, &bandit).unwrap();
        assert!(
            matches!(result, LoopControl::Decompose),
            "Expected Decompose, got {:?}",
            result
        );
    }

    // -----------------------------------------------------------------------
    // should_verify tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_should_verify_false_with_no_observations() {
        let engine = test_engine();
        let trajectory = test_trajectory();

        assert!(!engine.should_verify(&trajectory));
    }

    #[test]
    fn test_should_verify_at_frequency() {
        let engine = test_engine();
        let mut trajectory = test_trajectory();
        // Default frequency is 2

        // Add 2 observations (seq 0 and 1)
        trajectory
            .observations
            .push(test_observation(0, StrategyKind::RetryWithFeedback));
        trajectory
            .observations
            .push(test_observation(1, StrategyKind::RetryWithFeedback));

        // 2 observations, frequency 2: 2 % 2 == 0, should verify
        assert!(engine.should_verify(&trajectory));
    }

    #[test]
    fn test_should_verify_not_at_non_multiple() {
        let engine = test_engine();
        let mut trajectory = test_trajectory();

        // Add 3 observations
        for i in 0..3 {
            trajectory
                .observations
                .push(test_observation(i, StrategyKind::RetryWithFeedback));
        }

        // 3 observations, frequency 2: 3 % 2 == 1, should not verify
        // (unless a threshold crossing occurred)
        assert!(!engine.should_verify(&trajectory));
    }

    #[test]
    fn test_should_verify_on_threshold_crossing_0_5() {
        let engine = test_engine();
        let mut trajectory = test_trajectory();

        // Observation below 0.5
        let obs1 = test_observation(0, StrategyKind::RetryWithFeedback)
            .with_metrics(metrics_with(0.1, 0.45));
        trajectory.observations.push(obs1);

        // Observation crossing 0.5
        let obs2 = test_observation(1, StrategyKind::RetryWithFeedback)
            .with_metrics(metrics_with(0.1, 0.55));
        trajectory.observations.push(obs2);

        // 2 observations at frequency 2: also triggers by frequency,
        // but the threshold crossing is the key condition here
        assert!(engine.should_verify(&trajectory));
    }

    #[test]
    fn test_should_verify_on_threshold_crossing_0_8() {
        let engine = test_engine();
        let mut trajectory = test_trajectory();
        trajectory.policy.intent_verification_frequency = 10; // high freq to avoid frequency trigger

        // Observation below 0.8
        let obs1 = test_observation(0, StrategyKind::RetryWithFeedback)
            .with_metrics(metrics_with(0.1, 0.75));
        trajectory.observations.push(obs1);

        // Observation crossing 0.8
        let obs2 = test_observation(1, StrategyKind::RetryWithFeedback)
            .with_metrics(metrics_with(0.1, 0.85));
        trajectory.observations.push(obs2);

        assert!(engine.should_verify(&trajectory));
    }

    #[test]
    fn test_should_verify_on_threshold_crossing_0_9() {
        let engine = test_engine();
        let mut trajectory = test_trajectory();
        trajectory.policy.intent_verification_frequency = 10;

        // Observation below 0.9
        let obs1 = test_observation(0, StrategyKind::RetryWithFeedback)
            .with_metrics(metrics_with(0.1, 0.88));
        trajectory.observations.push(obs1);

        // Observation crossing 0.9
        let obs2 = test_observation(1, StrategyKind::RetryWithFeedback)
            .with_metrics(metrics_with(0.05, 0.92));
        trajectory.observations.push(obs2);

        assert!(engine.should_verify(&trajectory));
    }

    #[test]
    fn test_should_verify_no_crossing_when_already_above() {
        let engine = test_engine();
        let mut trajectory = test_trajectory();
        trajectory.policy.intent_verification_frequency = 10;

        // Both observations above 0.5 threshold
        let obs1 = test_observation(0, StrategyKind::RetryWithFeedback)
            .with_metrics(metrics_with(0.1, 0.55));
        trajectory.observations.push(obs1);

        let obs2 = test_observation(1, StrategyKind::RetryWithFeedback)
            .with_metrics(metrics_with(0.1, 0.60));
        trajectory.observations.push(obs2);

        // Neither crosses 0.8 or 0.9 and already above 0.5
        assert!(!engine.should_verify(&trajectory));
    }

    // -----------------------------------------------------------------------
    // Helper method tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_attractor_type_name() {
        let engine = test_engine();

        assert_eq!(
            engine.attractor_type_name(&AttractorType::FixedPoint {
                estimated_remaining_iterations: 3,
                estimated_remaining_tokens: 60_000,
            }),
            "fixed_point"
        );
        assert_eq!(
            engine.attractor_type_name(&AttractorType::LimitCycle {
                period: 2,
                cycle_signatures: vec![],
            }),
            "limit_cycle"
        );
        assert_eq!(
            engine.attractor_type_name(&AttractorType::Divergent {
                divergence_rate: -0.1,
                probable_cause: DivergenceCause::Unknown,
            }),
            "divergent"
        );
        assert_eq!(
            engine.attractor_type_name(&AttractorType::Plateau {
                stall_duration: 5,
                plateau_level: 0.5,
            }),
            "plateau"
        );
        assert_eq!(
            engine.attractor_type_name(&AttractorType::Indeterminate {
                tendency: ConvergenceTendency::Flat,
            }),
            "indeterminate"
        );
    }

    // -----------------------------------------------------------------------
    // prepare tests
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_prepare_creates_trajectory() {
        let engine = test_engine();
        let submission = TaskSubmission::new(
            "Implement user authentication with bcrypt password hashing and JWT tokens for session management".to_string(),
        );

        let (trajectory, infra) = engine.prepare(&submission).await.unwrap();

        assert_eq!(trajectory.phase, ConvergencePhase::Preparing);
        assert!(trajectory.observations.is_empty());
        assert!(trajectory.strategy_log.is_empty());
        assert!(!trajectory.specification.effective.content.is_empty());
        // Infrastructure should be initialized from the submission
        assert!(infra.acceptance_tests.is_empty()); // No discovered tests
    }

    #[tokio::test]
    async fn test_prepare_applies_priority_hint() {
        let engine = test_engine();
        let mut submission = TaskSubmission::new(
            "Implement a simple health check endpoint for the API with a detailed specification that covers all the necessary aspects of monitoring".to_string(),
        );
        submission.priority_hint = Some(PriorityHint::Fast);

        let (trajectory, _) = engine.prepare(&submission).await.unwrap();

        assert!(
            (trajectory.policy.acceptance_threshold - 0.85).abs() < f64::EPSILON,
            "Fast hint should set threshold to 0.85"
        );
        assert!(trajectory.policy.skip_expensive_overseers);
    }

    #[tokio::test]
    async fn test_prepare_folds_constraints_into_spec() {
        let engine = test_engine();
        let mut submission = TaskSubmission::new(
            "Build a REST API for user management with proper authentication and authorization controls".to_string(),
        );
        submission.constraints = vec![
            "Must use bcrypt for passwords".to_string(),
            "Must validate all inputs".to_string(),
        ];

        let (trajectory, _) = engine.prepare(&submission).await.unwrap();

        assert_eq!(trajectory.specification.amendments.len(), 2);
        assert!(trajectory
            .specification
            .effective
            .content
            .contains("Must use bcrypt"));
    }

    // -----------------------------------------------------------------------
    // finalize tests
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_finalize_converged_stores_success_memory() {
        let mem_repo = Arc::new(MockMemoryRepo::new());
        let engine = ConvergenceEngine::new(
            Arc::new(MockTrajectoryRepo::new()),
            mem_repo.clone(),
            Arc::new(MockOverseerMeasurer::new()),
            test_config(),
        );

        let mut trajectory = test_trajectory();
        let bandit = StrategyBandit::with_default_priors();
        let outcome = ConvergenceOutcome::Converged {
            trajectory_id: trajectory.id.to_string(),
            final_observation_sequence: 5,
        };

        engine
            .finalize(&mut trajectory, &outcome, &bandit)
            .await
            .unwrap();

        assert_eq!(trajectory.phase, ConvergencePhase::Converged);
        let memories = mem_repo.memories.lock().unwrap();
        // Should have success memory + bandit state
        assert!(
            memories.len() >= 2,
            "Expected at least 2 memories, got {}",
            memories.len()
        );
        assert!(memories.iter().any(|m| m.metadata.tags.contains(&"success".to_string())));
        assert!(memories.iter().any(|m| m.metadata.tags.contains(&"strategy-bandit".to_string())));
    }

    #[tokio::test]
    async fn test_finalize_exhausted_stores_failure_memory() {
        let mem_repo = Arc::new(MockMemoryRepo::new());
        let engine = ConvergenceEngine::new(
            Arc::new(MockTrajectoryRepo::new()),
            mem_repo.clone(),
            Arc::new(MockOverseerMeasurer::new()),
            test_config(),
        );

        let mut trajectory = test_trajectory();
        let bandit = StrategyBandit::with_default_priors();
        let outcome = ConvergenceOutcome::Exhausted {
            trajectory_id: trajectory.id.to_string(),
            best_observation_sequence: Some(3),
        };

        engine
            .finalize(&mut trajectory, &outcome, &bandit)
            .await
            .unwrap();

        assert_eq!(trajectory.phase, ConvergencePhase::Exhausted);
        let memories = mem_repo.memories.lock().unwrap();
        assert!(memories.iter().any(|m| m.metadata.tags.contains(&"failure".to_string())));
    }

    #[tokio::test]
    async fn test_finalize_trapped_stores_failure_memory() {
        let mem_repo = Arc::new(MockMemoryRepo::new());
        let engine = ConvergenceEngine::new(
            Arc::new(MockTrajectoryRepo::new()),
            mem_repo.clone(),
            Arc::new(MockOverseerMeasurer::new()),
            test_config(),
        );

        let mut trajectory = test_trajectory();
        let bandit = StrategyBandit::with_default_priors();
        let outcome = ConvergenceOutcome::Trapped {
            trajectory_id: trajectory.id.to_string(),
            attractor_type: AttractorType::LimitCycle {
                period: 2,
                cycle_signatures: vec![],
            },
        };

        engine
            .finalize(&mut trajectory, &outcome, &bandit)
            .await
            .unwrap();

        assert_eq!(trajectory.phase, ConvergencePhase::Trapped);
        let memories = mem_repo.memories.lock().unwrap();
        assert!(memories.iter().any(|m| m.metadata.tags.contains(&"failure".to_string())));
    }

    // -----------------------------------------------------------------------
    // request_extension tests
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_request_extension_granted_for_fixed_point() {
        let engine = test_engine();
        let mut trajectory = test_trajectory();

        // Set up as approaching fixed point
        trajectory.attractor_state = AttractorState {
            classification: AttractorType::FixedPoint {
                estimated_remaining_iterations: 2,
                estimated_remaining_tokens: 30_000,
            },
            confidence: 0.8,
            detected_at: None,
            evidence: AttractorEvidence {
                recent_deltas: vec![0.1, 0.08],
                recent_signatures: vec![],
                rationale: String::new(),
            },
        };

        let result = engine.request_extension(&mut trajectory).await.unwrap();
        assert!(result, "Extension should be granted for fixed point");
        assert_eq!(trajectory.budget.extensions_requested, 1);
        assert_eq!(trajectory.budget.extensions_granted, 1);
    }

    #[tokio::test]
    async fn test_request_extension_denied_for_non_fixed_point() {
        let engine = test_engine();
        let mut trajectory = test_trajectory();

        // Trajectory in limit cycle
        trajectory.attractor_state = AttractorState {
            classification: AttractorType::LimitCycle {
                period: 2,
                cycle_signatures: vec![],
            },
            confidence: 0.85,
            detected_at: None,
            evidence: AttractorEvidence {
                recent_deltas: vec![],
                recent_signatures: vec![],
                rationale: String::new(),
            },
        };

        let result = engine.request_extension(&mut trajectory).await.unwrap();
        assert!(!result, "Extension should be denied for limit cycle");
        assert_eq!(trajectory.budget.extensions_requested, 1);
        assert_eq!(trajectory.budget.extensions_granted, 0);
    }

    // -----------------------------------------------------------------------
    // initialize_bandit tests
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_initialize_bandit_returns_defaults_without_memory() {
        let engine = test_engine();
        let trajectory = test_trajectory();

        let bandit = engine.initialize_bandit(&trajectory).await;

        // Should have default priors
        assert!(bandit.context_arms.contains_key("fixed_point"));
        assert!(bandit.context_arms.contains_key("limit_cycle"));
    }

    #[tokio::test]
    async fn test_initialize_bandit_restores_from_memory() {
        let mem_repo = Arc::new(MockMemoryRepo::new());
        let trajectory = test_trajectory();

        // Pre-populate memory with a bandit state
        let mut original_bandit = StrategyBandit::with_default_priors();
        original_bandit.nudge("fixed_point", "focused_repair", 5.0);

        let content = serde_json::to_string(&original_bandit).unwrap();
        let memory = Memory::semantic(
            format!("strategy-bandit-{}", trajectory.task_id),
            content,
        )
        .with_namespace("convergence")
        .with_task(trajectory.task_id)
        .with_tag("strategy-bandit");
        mem_repo.store(&memory).await.unwrap();

        let engine = ConvergenceEngine::new(
            Arc::new(MockTrajectoryRepo::new()),
            mem_repo,
            Arc::new(MockOverseerMeasurer::new()),
            test_config(),
        );

        let bandit = engine.initialize_bandit(&trajectory).await;

        // Should have the nudged value
        let dist = &bandit.context_arms["fixed_point"]["focused_repair"];
        assert!(
            (dist.alpha - 6.0).abs() < f64::EPSILON,
            "Expected alpha=6.0 (1.0 + 5.0), got {}",
            dist.alpha
        );
    }

    #[tokio::test]
    async fn test_initialize_bandit_defaults_when_memory_disabled() {
        let mut config = test_config();
        config.memory_enabled = false;

        let engine = ConvergenceEngine::new(
            Arc::new(MockTrajectoryRepo::new()),
            Arc::new(MockMemoryRepo::new()),
            Arc::new(MockOverseerMeasurer::new()),
            config,
        );

        let trajectory = test_trajectory();
        let bandit = engine.initialize_bandit(&trajectory).await;

        // Should still have default priors
        assert!(bandit.context_arms.contains_key("fixed_point"));
    }

    // -----------------------------------------------------------------------
    // store_success_memory / store_failure_memory tests
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_store_success_memory_content() {
        let mem_repo = Arc::new(MockMemoryRepo::new());
        let engine = ConvergenceEngine::new(
            Arc::new(MockTrajectoryRepo::new()),
            mem_repo.clone(),
            Arc::new(MockOverseerMeasurer::new()),
            test_config(),
        );

        let mut trajectory = test_trajectory();
        trajectory.strategy_log.push(StrategyEntry::new(
            StrategyKind::RetryWithFeedback,
            0,
            10_000,
            false,
        ));

        engine
            .store_success_memory(&trajectory, 0)
            .await
            .unwrap();

        let memories = mem_repo.memories.lock().unwrap();
        assert_eq!(memories.len(), 1);
        let memory = &memories[0];
        assert!(memory.content.contains("SUCCESS"));
        assert!(memory.content.contains("retry_with_feedback"));
        assert_eq!(memory.namespace, "convergence");
        assert!(memory.metadata.tags.contains(&"convergence".to_string()));
        assert!(memory.metadata.tags.contains(&"success".to_string()));
        assert_eq!(memory.tier, MemoryTier::Semantic);
    }

    #[tokio::test]
    async fn test_store_failure_memory_content() {
        let mem_repo = Arc::new(MockMemoryRepo::new());
        let engine = ConvergenceEngine::new(
            Arc::new(MockTrajectoryRepo::new()),
            mem_repo.clone(),
            Arc::new(MockOverseerMeasurer::new()),
            test_config(),
        );

        let trajectory = test_trajectory();
        engine
            .store_failure_memory(&trajectory, "trapped")
            .await
            .unwrap();

        let memories = mem_repo.memories.lock().unwrap();
        assert_eq!(memories.len(), 1);
        let memory = &memories[0];
        assert!(memory.content.contains("FAILURE"));
        assert!(memory.content.contains("trapped"));
        assert_eq!(memory.namespace, "convergence");
        assert!(memory.metadata.tags.contains(&"failure".to_string()));
        assert_eq!(memory.tier, MemoryTier::Episodic);
    }

    // -----------------------------------------------------------------------
    // persist_bandit_state tests
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_persist_bandit_state_serializes_correctly() {
        let mem_repo = Arc::new(MockMemoryRepo::new());
        let engine = ConvergenceEngine::new(
            Arc::new(MockTrajectoryRepo::new()),
            mem_repo.clone(),
            Arc::new(MockOverseerMeasurer::new()),
            test_config(),
        );

        let trajectory = test_trajectory();
        let mut bandit = StrategyBandit::with_default_priors();
        bandit.nudge("fixed_point", "focused_repair", 3.0);

        engine
            .persist_bandit_state(&bandit, &trajectory)
            .await
            .unwrap();

        let memories = mem_repo.memories.lock().unwrap();
        assert_eq!(memories.len(), 1);
        let memory = &memories[0];
        assert!(memory.metadata.tags.contains(&"strategy-bandit".to_string()));

        // Verify deserializable
        let restored: StrategyBandit =
            serde_json::from_str(&memory.content).unwrap();
        let dist = &restored.context_arms["fixed_point"]["focused_repair"];
        assert!((dist.alpha - 4.0).abs() < f64::EPSILON);
    }
}
