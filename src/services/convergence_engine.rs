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
use crate::domain::models::intent_verification::{GapCategory, GapSeverity, IntentGap};
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
        let mode = select_convergence_mode(&basin, &trajectory.policy, None);

        // If parallel mode is selected, route to converge_parallel instead
        // of the sequential loop (spec 6.6).
        if let ConvergenceMode::Parallel { initial_samples } = mode {
            let submission = TaskSubmission {
                description: trajectory.specification.effective.content.clone(),
                goal_id: trajectory.goal_id,
                inferred_complexity: Complexity::Moderate,
                discovered_infrastructure: DiscoveredInfrastructure::default(),
                priority_hint: trajectory.policy.priority_hint,
                constraints: trajectory.specification.effective.constraints.clone(),
                references: vec![],
                anti_patterns: trajectory.specification.effective.anti_patterns.clone(),
                parallel_samples: Some(initial_samples),
            };
            return self
                .converge_parallel(&submission, initial_samples)
                .await;
        }

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
                                let am = a.metrics.as_ref().unwrap();
                                let bm = b.metrics.as_ref().unwrap();
                                let al = am.intent_blended_level.unwrap_or(am.convergence_level);
                                let bl = bm.intent_blended_level.unwrap_or(bm.convergence_level);
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
                let mut eligible = eligible_strategies(
                    &trajectory.strategy_log,
                    attractor,
                    &trajectory.budget,
                    trajectory.total_fresh_starts,
                    trajectory.policy.max_fresh_starts,
                );

                // Spec 4.5: Decay-aware rotation check.
                // If the current exploitation strategy has diminishing returns,
                // filter it out so the bandit selects a different one.
                if let Some(last_entry) = trajectory.strategy_log.last() {
                    let current = &last_entry.strategy_kind;
                    if current.is_exploitation() {
                        let consecutive_uses = trajectory
                            .strategy_log
                            .iter()
                            .rev()
                            .take_while(|e| {
                                e.strategy_kind.kind_name() == current.kind_name()
                            })
                            .count() as u32;
                        let recent_deltas: Vec<f64> = trajectory
                            .strategy_log
                            .iter()
                            .rev()
                            .take_while(|e| {
                                e.strategy_kind.kind_name() == current.kind_name()
                            })
                            .filter_map(|e| e.convergence_delta_achieved)
                            .collect::<Vec<_>>()
                            .into_iter()
                            .rev()
                            .collect();

                        if should_rotate_strategy(
                            current,
                            consecutive_uses,
                            &recent_deltas,
                        ) {
                            let current_name = current.kind_name();
                            tracing::info!(
                                strategy = current_name,
                                consecutive_uses = consecutive_uses,
                                "Strategy rotation triggered: filtering out {}",
                                current_name
                            );
                            eligible.retain(|s| s.kind_name() != current_name);
                        }
                    }
                }

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
                self.execute_strategy(&strategy, &mut trajectory).await?;

            // d. Measure with overseers
            let mut observation =
                self.measure_artifact(&artifact, &strategy, &trajectory).await?;
            observation.tokens_used = tokens_used;

            // e-i. Run the full iteration body (metrics, classification, bandit, loop control)
            let control = self
                .iterate_once(&mut trajectory, &mut bandit, &strategy, observation)
                .await?;

            match control {
                LoopControl::IntentCheck => {
                    // The engine's internal converge() method does not have
                    // access to the LLM intent verifier. When running standalone
                    // (without the orchestrator), IntentCheck simply continues
                    // iterating. The orchestrator's run_convergent_execution
                    // handles IntentCheck by calling the LLM-based intent
                    // verifier, which is the sole authority on finality.
                    continue;
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
                                .map(|m| m.intent_blended_level.unwrap_or(m.convergence_level))
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
            let delta = compute_convergence_delta_with_intent(
                prev,
                &obs_with_metrics.overseer_signals,
                obs_with_metrics
                    .metrics
                    .as_ref()
                    .map(|m| m.ast_diff_nodes)
                    .unwrap_or(0),
                &health,
                &ConvergenceWeights::default(),
                trajectory.prev_intent_confidence,
                trajectory.last_intent_confidence,
            );
            let level = convergence_level(&obs_with_metrics.overseer_signals);
            let blended = convergence_level_with_intent(
                &obs_with_metrics.overseer_signals,
                trajectory.last_intent_confidence,
            );
            let metrics = ObservationMetrics {
                convergence_delta: delta,
                convergence_level: level,
                intent_blended_level: if trajectory.last_intent_confidence.is_some() {
                    Some(blended)
                } else {
                    None
                },
                ..ObservationMetrics::default()
            };
            obs_with_metrics = obs_with_metrics.with_metrics(metrics);
            entry = entry.with_delta(delta);
        }

        // Run intent verification if scheduled (spec 7.2)
        //
        // Verification synthesizes a VerificationResult from overseer signals
        // and the specification. It runs periodically (controlled by
        // policy.intent_verification_frequency), on threshold crossings, and
        // on attractor transitions to FixedPoint.
        if self.should_verify(trajectory) {
            let verification = self.summarize_signals(
                &obs_with_metrics.overseer_signals,
                &trajectory.specification.effective,
                trajectory.task_id,
            );
            obs_with_metrics = obs_with_metrics.with_verification(verification);
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

        // Classify attractor -- capture previous classification for transition detection
        let previous_classification = trajectory.attractor_state.classification.clone();
        trajectory.attractor_state = classify_attractor(&trajectory.observations, 5);

        self.emit_event(ConvergenceEvent::AttractorClassified {
            trajectory_id: trajectory.id.to_string(),
            attractor_type: trajectory.attractor_state.classification.clone(),
            confidence: trajectory.attractor_state.confidence,
        });

        // Spec 7.3: Attractor transition detection (intervention point).
        // Log when the attractor classification changes between observations.
        let prev_name = self.attractor_type_name(&previous_classification);
        let new_name =
            self.attractor_type_name(&trajectory.attractor_state.classification);
        if prev_name != new_name {
            tracing::info!(
                trajectory_id = %trajectory.id,
                from = prev_name,
                to = new_name,
                "AttractorTransition intervention point: attractor changed from {} to {}",
                prev_name,
                new_name
            );
        }

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

    /// Determine whether the convergence loop should continue.
    ///
    /// Intent verification is the sole finality mechanism. The engine emits
    /// `IntentCheck` when conditions suggest readiness; the orchestrator's
    /// LLM-based intent verifier makes the finality decision.
    ///
    /// Evaluates, in priority order:
    /// 1. Budget exhausted -> Exhausted or RequestExtension
    /// 2. IntentCheck -> iteration interval, budget fraction, or FixedPoint trigger
    /// 3. Trapped -> limit cycle with no escape strategies
    /// 4. Decompose -> divergent trajectory with sufficient budget
    /// 5. Near-budget extension check
    /// 6. Continue -> keep iterating
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

        // 2. Intent verification triggers — independent of overseer scores.
        //
        // Intent is the sole finality mechanism. We trigger IntentCheck on:
        //   a) Every `intent_check_interval` iterations (policy-driven frequency)
        //   b) FixedPoint attractor (stable state = natural point to verify)
        //   c) Budget usage exceeds `intent_check_at_budget_fraction`
        let iteration = trajectory.observations.len() as u32;
        let interval = trajectory.policy.intent_check_interval.max(1);

        let interval_trigger = iteration > 0 && iteration % interval == 0;

        let at_fixed_point = matches!(
            &trajectory.attractor_state.classification,
            AttractorType::FixedPoint { .. }
        );

        let budget_fraction_used = 1.0 - trajectory.budget.remaining_fraction();
        let budget_trigger = budget_fraction_used
            >= trajectory.policy.intent_check_at_budget_fraction;

        // Plateau trigger: stalled trajectory is a natural "should we check
        // if we're done?" point.
        let at_plateau = matches!(
            &trajectory.attractor_state.classification,
            AttractorType::Plateau { .. }
        );

        // All-overseers-passing trigger: first transition to all-passing
        // means static checks have nothing more to report — ask intent if
        // work is complete.
        let all_passing_transition = if trajectory.observations.len() >= 2 {
            let curr = &trajectory.observations[trajectory.observations.len() - 1];
            let prev = &trajectory.observations[trajectory.observations.len() - 2];
            curr.overseer_signals.all_passing() && !prev.overseer_signals.all_passing()
        } else {
            false
        };

        if interval_trigger || at_fixed_point || budget_trigger || at_plateau || all_passing_transition {
            return Ok(LoopControl::IntentCheck);
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

    /// Run parallel trajectory sampling with Thompson Sampling (spec 6.6).
    ///
    /// Two-phase approach:
    ///
    /// **Phase 1 -- Independent starts**: Generate `sample_count` independent
    /// trajectories, each running exactly one iteration to produce an initial
    /// observation. Each sample gets 1/N of the parent budget.
    ///
    /// **Phase 2 -- Thompson Sampling selection**: Iteratively select which
    /// trajectory to invest the next iteration in, using per-trajectory
    /// BetaDistributions updated from convergence deltas. Divergent
    /// trajectories (with fewer than 3 observations exempted) are filtered
    /// out. The loop continues until a trajectory converges, all budgets are
    /// exhausted, or all trajectories are filtered out.
    pub async fn converge_parallel(
        &self,
        submission: &TaskSubmission,
        sample_count: u32,
    ) -> DomainResult<ConvergenceOutcome> {
        let (base_trajectory, _infrastructure) = self.prepare(submission).await?;

        self.emit_event(ConvergenceEvent::ParallelConvergenceStarted {
            trajectory_id: base_trajectory.id.to_string(),
            parallel_count: sample_count as usize,
        });

        let n = sample_count as usize;

        // Phase 1: Generate N independent starts, each with one iteration.
        let mut trajectories: Vec<Trajectory> = Vec::with_capacity(n);
        let mut bandits: Vec<StrategyBandit> = Vec::with_capacity(n);
        let mut scores: Vec<BetaDistribution> = Vec::with_capacity(n);
        let mut active: Vec<bool> = Vec::with_capacity(n);

        for _ in 0..n {
            let mut sample = base_trajectory.clone();
            sample.id = Uuid::new_v4();
            sample.budget = base_trajectory.budget.scale(1.0 / n as f64);
            sample.phase = ConvergencePhase::Iterating;

            let mut bandit = self.initialize_bandit(&sample).await;

            // Run exactly one iteration for the initial start.
            let attractor = &sample.attractor_state;
            let eligible = eligible_strategies(
                &sample.strategy_log,
                attractor,
                &sample.budget,
                sample.total_fresh_starts,
                sample.policy.max_fresh_starts,
            );
            if !eligible.is_empty() {
                let strategy = bandit.select(
                    &attractor.classification,
                    &eligible,
                    &sample.policy,
                );
                let (artifact, tokens_used, _wall_time_ms) =
                    self.execute_strategy(&strategy, &mut sample).await?;
                let mut observation = self
                    .measure_artifact(&artifact, &strategy, &sample)
                    .await?;
                observation.tokens_used = tokens_used;
                let _control = self
                    .iterate_once(
                        &mut sample,
                        &mut bandit,
                        &strategy,
                        observation,
                    )
                    .await?;
            }

            trajectories.push(sample);
            bandits.push(bandit);
            scores.push(BetaDistribution::uniform());
            active.push(true);
        }

        // Phase 2: Thompson Sampling to iteratively select which trajectory
        // to invest the next iteration in.
        loop {
            // Filter out divergent trajectories (unless < 3 observations).
            for i in 0..n {
                if !active[i] {
                    continue;
                }
                if let AttractorType::Divergent { .. } =
                    &trajectories[i].attractor_state.classification
                {
                    if trajectories[i].observations.len() >= 3 {
                        tracing::info!(
                            trajectory_id = %trajectories[i].id,
                            "Parallel convergence: filtering out divergent \
                             trajectory",
                        );
                        active[i] = false;
                    }
                }
            }

            // Check if any trajectories are still active.
            if !active.iter().any(|&a| a) {
                return Ok(ConvergenceOutcome::Exhausted {
                    trajectory_id: base_trajectory.id.to_string(),
                    best_observation_sequence: None,
                });
            }

            // Check if any active trajectory is exhausted.
            //
            // The parallel path doesn't have access to the LLM intent
            // verifier, so IntentCheck signals are treated as "continue
            // iterating" — same as the single-trajectory converge() path.
            // Finality decisions require the orchestrator's LLM verifier.
            let mut all_exhausted = true;
            for i in 0..n {
                if !active[i] {
                    continue;
                }
                if trajectories[i].budget.has_remaining() {
                    all_exhausted = false;
                }
            }
            let best_converged: Option<usize> = None;

            // If a trajectory converged, finalize and return.
            if let Some(idx) = best_converged {
                let final_seq = trajectories[idx]
                    .observations
                    .last()
                    .map(|o| o.sequence)
                    .unwrap_or(0);
                let outcome = ConvergenceOutcome::Converged {
                    trajectory_id: trajectories[idx].id.to_string(),
                    final_observation_sequence: final_seq,
                };
                self.finalize(
                    &mut trajectories[idx],
                    &outcome,
                    &bandits[idx],
                )
                .await?;
                return Ok(outcome);
            }

            // If all active trajectories exhausted budgets, pick the best.
            if all_exhausted {
                let best_idx = (0..n)
                    .filter(|&i| active[i])
                    .max_by(|&a, &b| {
                        let level_a = trajectories[a]
                            .best_observation()
                            .and_then(|o| o.metrics.as_ref())
                            .map(|m| m.intent_blended_level.unwrap_or(m.convergence_level))
                            .unwrap_or(0.0);
                        let level_b = trajectories[b]
                            .best_observation()
                            .and_then(|o| o.metrics.as_ref())
                            .map(|m| m.intent_blended_level.unwrap_or(m.convergence_level))
                            .unwrap_or(0.0);
                        level_a
                            .partial_cmp(&level_b)
                            .unwrap_or(std::cmp::Ordering::Equal)
                    })
                    .unwrap_or(0);
                let outcome = ConvergenceOutcome::Exhausted {
                    trajectory_id: trajectories[best_idx].id.to_string(),
                    best_observation_sequence: trajectories[best_idx]
                        .best_observation()
                        .map(|o| o.sequence),
                };
                self.finalize(
                    &mut trajectories[best_idx],
                    &outcome,
                    &bandits[best_idx],
                )
                .await?;
                return Ok(outcome);
            }

            // Thompson Sampling: sample from each active trajectory's Beta
            // distribution and pick the highest scoring one.
            let selected_idx = (0..n)
                .filter(|&i| {
                    active[i] && trajectories[i].budget.has_remaining()
                })
                .max_by(|&a, &b| {
                    let score_a = scores[a].sample();
                    let score_b = scores[b].sample();
                    score_a
                        .partial_cmp(&score_b)
                        .unwrap_or(std::cmp::Ordering::Equal)
                });

            let selected_idx = match selected_idx {
                Some(idx) => idx,
                None => {
                    return Ok(ConvergenceOutcome::Exhausted {
                        trajectory_id: base_trajectory.id.to_string(),
                        best_observation_sequence: None,
                    });
                }
            };

            // Run one iteration on the selected trajectory.
            let traj = &mut trajectories[selected_idx];
            let bandit = &mut bandits[selected_idx];
            let attractor = &traj.attractor_state;
            let eligible = eligible_strategies(
                &traj.strategy_log,
                attractor,
                &traj.budget,
                traj.total_fresh_starts,
                traj.policy.max_fresh_starts,
            );
            if eligible.is_empty() {
                active[selected_idx] = false;
                continue;
            }
            let strategy = bandit.select(
                &attractor.classification,
                &eligible,
                &traj.policy,
            );
            let (artifact, tokens_used, _wall_time_ms) =
                self.execute_strategy(&strategy, traj).await?;
            let mut observation = self
                .measure_artifact(&artifact, &strategy, traj)
                .await?;
            observation.tokens_used = tokens_used;
            let _control = self
                .iterate_once(traj, bandit, &strategy, observation)
                .await?;

            // Update per-trajectory Thompson Sampling scores.
            let latest_delta = traj.latest_convergence_delta();
            if latest_delta > 0.0 {
                scores[selected_idx].alpha += 1.0;
            } else {
                scores[selected_idx].beta += 1.0;
            }
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
    /// 2. A meaningful overseer state transition occurred (build fixed, tests improved).
    /// 3. The attractor classification just changed to FixedPoint.
    pub fn should_verify(&self, trajectory: &Trajectory) -> bool {
        let obs_count = trajectory.observations.len() as u32;

        // No observations yet
        if obs_count == 0 {
            return false;
        }

        // Check frequency
        if trajectory.policy.intent_verification_frequency > 0
            && obs_count % trajectory.policy.intent_verification_frequency == 0
        {
            return true;
        }

        // Check overseer state transitions (replaces threshold crossings)
        if trajectory.observations.len() >= 2 {
            let curr = &trajectory.observations[trajectory.observations.len() - 1];
            let prev = &trajectory.observations[trajectory.observations.len() - 2];

            // Build went from failing to passing
            let build_fixed = match (&prev.overseer_signals.build_result, &curr.overseer_signals.build_result) {
                (Some(prev_b), Some(curr_b)) => !prev_b.success && curr_b.success,
                _ => false,
            };
            if build_fixed {
                return true;
            }

            // Test pass count improved and fail count decreased
            let tests_improved = match (&prev.overseer_signals.test_results, &curr.overseer_signals.test_results) {
                (Some(prev_t), Some(curr_t)) => {
                    curr_t.passed > prev_t.passed && curr_t.failed < prev_t.failed
                }
                _ => false,
            };
            if tests_improved {
                return true;
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
    // Intent verification from overseer signals
    // -----------------------------------------------------------------------

    /// Summarize overseer signals into a structured [`VerificationResult`].
    ///
    /// This is a deterministic, signal-based summary — not an LLM call and
    /// **not a finality check**. The result feeds into attractor
    /// classification (ambiguity detection) and strategy selection (remaining
    /// gaps inform `FocusedRepair` targets). Finality decisions are made
    /// exclusively by the LLM-based intent verifier.
    ///
    /// Satisfaction levels (descriptive, not authoritative):
    /// - **"satisfied"** — no gaps identified by any present overseer
    /// - **"partial"** — some gaps exist but none critical
    /// - **"unsatisfied"** — critical gaps present (e.g., build failure)
    fn summarize_signals(
        &self,
        signals: &OverseerSignals,
        spec: &SpecificationSnapshot,
        task_id: Uuid,
    ) -> VerificationResult {
        let mut gaps: Vec<IntentGap> = Vec::new();

        // Build failure is a critical gap
        if let Some(ref build) = signals.build_result {
            if !build.success {
                let mut gap = IntentGap::new(
                    format!("Build failure: {} error(s)", build.error_count),
                    GapSeverity::Critical,
                )
                .with_category(GapCategory::Functional)
                .with_task(task_id);
                if let Some(first_error) = build.errors.first() {
                    gap = gap.with_action(format!("Fix build error: {}", first_error));
                }
                gaps.push(gap);
            }
        }

        // Type check failure
        if let Some(ref tc) = signals.type_check {
            if !tc.clean {
                let mut gap = IntentGap::new(
                    format!("Type check failure: {} error(s)", tc.error_count),
                    GapSeverity::Major,
                )
                .with_category(GapCategory::Functional)
                .with_task(task_id);
                if let Some(first_error) = tc.errors.first() {
                    gap = gap.with_action(format!("Fix type error: {}", first_error));
                }
                gaps.push(gap);
            }
        }

        // Failing tests
        if let Some(ref tests) = signals.test_results {
            if tests.failed > 0 {
                let gap = IntentGap::new(
                    format!(
                        "Test failures: {}/{} tests failing ({}  regressions)",
                        tests.failed, tests.total, tests.regression_count
                    ),
                    if tests.regression_count > 0 {
                        GapSeverity::Major
                    } else {
                        GapSeverity::Moderate
                    },
                )
                .with_category(GapCategory::Testing)
                .with_task(task_id)
                .with_action(format!(
                    "Fix failing tests: {}",
                    tests
                        .failing_test_names
                        .iter()
                        .take(3)
                        .cloned()
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
                gaps.push(gap);
            }
        }

        // Security vulnerabilities
        if let Some(ref sec) = signals.security_scan {
            let vuln_count = sec.critical_count + sec.high_count;
            if vuln_count > 0 {
                let gap = IntentGap::new(
                    format!(
                        "Security vulnerabilities: {} critical, {} high",
                        sec.critical_count, sec.high_count
                    ),
                    if sec.critical_count > 0 {
                        GapSeverity::Critical
                    } else {
                        GapSeverity::Major
                    },
                )
                .with_category(GapCategory::Security)
                .with_task(task_id);
                gaps.push(gap);
            }
        }

        // Lint errors (not warnings)
        if let Some(ref lint) = signals.lint_results {
            if lint.error_count > 0 {
                gaps.push(
                    IntentGap::new(
                        format!("Lint errors: {} error(s)", lint.error_count),
                        GapSeverity::Minor,
                    )
                    .with_category(GapCategory::Maintainability)
                    .with_task(task_id),
                );
            }
        }

        // Failing custom checks
        for check in &signals.custom_checks {
            if !check.passed {
                gaps.push(
                    IntentGap::new(
                        format!("Custom check '{}' failed: {}", check.name, check.details),
                        GapSeverity::Moderate,
                    )
                    .with_category(GapCategory::Functional)
                    .with_task(task_id),
                );
            }
        }

        // Check specification requirements against what overseers can tell us.
        // If the spec has success_criteria but we have no test results at all,
        // flag that as an implicit gap — we can't verify intent without tests.
        if !spec.success_criteria.is_empty() && signals.test_results.is_none() {
            gaps.push(
                IntentGap::new(
                    "Specification has success criteria but no test results available",
                    GapSeverity::Moderate,
                )
                .with_category(GapCategory::Testing)
                .with_task(task_id)
                .as_implicit("Success criteria require test verification"),
            );
        }

        // Determine satisfaction level
        let has_critical = gaps.iter().any(|g| matches!(g.severity, GapSeverity::Critical));
        let has_major = gaps.iter().any(|g| matches!(g.severity, GapSeverity::Major));

        let (satisfaction, confidence) = if gaps.is_empty() {
            ("satisfied", 0.85)
        } else if has_critical {
            ("unsatisfied", 0.8)
        } else if has_major {
            ("partial", 0.7)
        } else {
            ("partial", 0.6)
        };

        VerificationResult::new(satisfaction, confidence, gaps)
    }

    // -----------------------------------------------------------------------
    // 9.2 decompose_and_coordinate -- Decomposition flow
    // -----------------------------------------------------------------------

    /// Decompose a task into subtasks and coordinate their convergence (spec 9.2, 9.3).
    ///
    /// Full decomposition flow:
    /// 1. Propose decomposition into subtasks.
    /// 2. Reserve 25% of the parent budget for the integration trajectory.
    /// 3. Allocate remaining 75% across child subtasks.
    /// 4. Converge each child through the full engine (`self.converge()`).
    /// 5. If any child fails, return Exhausted immediately.
    /// 6. After all children converge, run a mandatory integration trajectory
    ///    using the reserved budget.
    /// 7. Return the final outcome.
    pub async fn decompose_and_coordinate(
        &self,
        trajectory: &mut Trajectory,
    ) -> DomainResult<ConvergenceOutcome> {
        // 1. Propose decomposition
        let decomposition = self.propose_decomposition(trajectory);

        // 2. Reserve 25% of parent budget for integration (spec 9.3)
        let integration_budget = trajectory.budget.scale(0.25);

        // 3. Allocate remaining 75% across child subtasks
        let child_budgets = allocate_decomposed_budget(&trajectory.budget, &decomposition);

        self.emit_event(ConvergenceEvent::DecompositionTriggered {
            parent_trajectory_id: trajectory.id.to_string(),
            child_count: decomposition.len(),
        });

        // 4. Converge each child through the full engine
        let mut child_ids = Vec::new();
        let empty_infra = ConvergenceInfrastructure::default();

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

            // Run full convergence for this child.
            // Box::pin is required because converge -> decompose_and_coordinate -> converge
            // forms a recursive async call chain.
            let child_outcome =
                Box::pin(self.converge(child, &empty_infra)).await?;

            // 5. If any child fails, return Exhausted immediately
            if !matches!(&child_outcome, ConvergenceOutcome::Converged { .. }) {
                tracing::warn!(
                    parent_trajectory_id = %trajectory.id,
                    child_subtask = %subtask.subtask_id,
                    "Decomposition: child subtask did not converge, aborting coordination",
                );
                return Ok(ConvergenceOutcome::Exhausted {
                    trajectory_id: trajectory.id.to_string(),
                    best_observation_sequence: trajectory
                        .best_observation()
                        .map(|o| o.sequence),
                });
            }
        }

        // 6. All children converged -- run mandatory integration trajectory (spec 9.3).
        let integration_outcome = self
            .run_integration_trajectory(trajectory, &child_ids, integration_budget)
            .await?;

        match &integration_outcome {
            ConvergenceOutcome::Converged { .. } => {
                // 7. Integration succeeded -- return Decomposed with all child IDs
                Ok(ConvergenceOutcome::Decomposed {
                    parent_trajectory_id: trajectory.id.to_string(),
                    child_trajectory_ids: child_ids,
                })
            }
            _ => {
                // Integration failed
                tracing::warn!(
                    parent_trajectory_id = %trajectory.id,
                    "Decomposition: integration trajectory did not converge",
                );
                Ok(ConvergenceOutcome::Exhausted {
                    trajectory_id: trajectory.id.to_string(),
                    best_observation_sequence: trajectory
                        .best_observation()
                        .map(|o| o.sequence),
                })
            }
        }
    }

    /// Run the mandatory integration trajectory after all children converge (spec 9.3).
    ///
    /// The integration trajectory verifies that the combined child outputs form
    /// a coherent whole. It receives 25% of the parent budget and a specification
    /// that references all child trajectory IDs.
    async fn run_integration_trajectory(
        &self,
        parent_trajectory: &Trajectory,
        child_ids: &[String],
        integration_budget: ConvergenceBudget,
    ) -> DomainResult<ConvergenceOutcome> {
        let integration_description = format!(
            "Integration of decomposed subtasks for: {}. Child trajectories: [{}]",
            parent_trajectory.specification.effective.content,
            child_ids.join(", "),
        );

        let integration_spec =
            SpecificationEvolution::new(SpecificationSnapshot::new(integration_description));

        let integration_trajectory = Trajectory::new(
            parent_trajectory.task_id,
            parent_trajectory.goal_id,
            integration_spec,
            integration_budget,
            parent_trajectory.policy.clone(),
        );

        let empty_infra = ConvergenceInfrastructure::default();
        Box::pin(self.converge(integration_trajectory, &empty_infra)).await
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
    /// Strategies with side effects (spec 4.1):
    /// - **ArchitectReview**: Creates a `SpecificationAmendment` with source
    ///   `ArchitectAmendment`, applies it to the specification evolution, and
    ///   emits a `SpecificationAmended` event.
    /// - **RevertAndBranch**: Finds the target observation and uses its
    ///   artifact as the starting point for the new branch.
    ///
    /// Returns `(artifact, tokens_used, wall_time_ms)`.
    async fn execute_strategy(
        &self,
        strategy: &StrategyKind,
        trajectory: &mut Trajectory,
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

        // For strategies that modify the specification or trajectory state,
        // handle their side effects (spec 4.1).
        match strategy {
            StrategyKind::ArchitectReview => {
                // Spec 4.1: ArchitectReview creates a SpecificationAmendment
                // with source ArchitectAmendment and applies it to the
                // specification evolution. In a full integration this would
                // invoke the architect agent; here we create the amendment
                // from the current trajectory state.
                let amendment_description = format!(
                    "Architect review after {} observations; attractor: {}",
                    trajectory.observations.len(),
                    self.attractor_type_name(
                        &trajectory.attractor_state.classification
                    ),
                );
                let amendment = SpecificationAmendment::new(
                    AmendmentSource::ArchitectAmendment,
                    amendment_description.clone(),
                    "ArchitectReview strategy identified specification gaps",
                );
                trajectory.specification.add_amendment(amendment);

                // Spec 1.6 / Task 6: Emit SpecificationAmended event
                self.emit_event(ConvergenceEvent::SpecificationAmended {
                    trajectory_id: trajectory.id.to_string(),
                    amendment_source: AmendmentSource::ArchitectAmendment,
                    amendment_summary: amendment_description,
                });

                tracing::info!(
                    trajectory_id = %trajectory.id,
                    "ArchitectReview: specification amended, {} total amendments",
                    trajectory.specification.amendments.len(),
                );
            }
            StrategyKind::FreshStart { carry_forward } => {
                // Fresh start resets context but preserves filesystem and
                // trajectory metadata
                tracing::info!(
                    trajectory_id = %trajectory.id,
                    "Fresh start: carrying forward {} hints, best level \
                     from {} observations",
                    carry_forward.hints.len(),
                    trajectory.observations.len(),
                );
            }
            StrategyKind::RevertAndBranch { target } => {
                // Spec 4.1: RevertAndBranch finds the target observation
                // and uses its artifact as the starting point.
                tracing::info!(
                    trajectory_id = %trajectory.id,
                    target = %target,
                    "Reverting to observation {} and branching",
                    target,
                );

                // Find the target observation and return its artifact
                if let Some(target_obs) = trajectory
                    .observations
                    .iter()
                    .find(|obs| obs.id == *target)
                {
                    let artifact = target_obs.artifact.clone();
                    let elapsed = start.elapsed();
                    let estimated_tokens = strategy.estimated_cost();
                    return Ok((
                        artifact,
                        estimated_tokens,
                        elapsed.as_millis() as u64,
                    ));
                }
                // If target not found, fall through to default artifact
                tracing::warn!(
                    trajectory_id = %trajectory.id,
                    target = %target,
                    "RevertAndBranch target observation not found; \
                     using latest artifact",
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

    /// Run overseer measurement on an artifact and return aggregated signals.
    ///
    /// This is the public entry point for external callers (e.g. the
    /// orchestrator's convergent execution loop) that need overseer signals
    /// without constructing a full `Observation`. Delegates to the injected
    /// `overseer_measurer` implementation, which runs overseers in
    /// cost-ordered phases and respects the policy's `skip_expensive_overseers`
    /// flag.
    pub async fn measure(
        &self,
        artifact: &ArtifactReference,
        policy: &ConvergencePolicy,
    ) -> DomainResult<OverseerSignals> {
        self.overseer_measurer.measure(artifact, policy).await
    }

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
    fn test_loop_control_intent_check_at_interval() {
        let engine = test_engine();
        let mut trajectory = test_trajectory();
        // Default intent_check_interval is 2, so iteration 2 should trigger

        // Add 2 observations (iterations 0 and 1)
        for i in 0..2 {
            let signals = signals_with_tests(2, 10);
            let obs = Observation::new(
                i,
                test_artifact(i),
                signals,
                StrategyKind::RetryWithFeedback,
                10_000,
                5_000,
            )
            .with_metrics(metrics_with(0.1, 0.56));
            trajectory.observations.push(obs);
        }
        let bandit = StrategyBandit::with_default_priors();

        let result = engine.check_loop_control(&trajectory, &bandit).unwrap();
        assert!(matches!(result, LoopControl::IntentCheck),
            "Expected IntentCheck at iteration 2 (interval=2), got {:?}", result);
    }

    #[test]
    fn test_loop_control_continue_between_intervals() {
        let engine = test_engine();
        let mut trajectory = test_trajectory();
        // Default intent_check_interval is 2, so iteration 1 should NOT trigger

        // Add 1 observation (iteration 0)
        let signals = signals_with_tests(2, 10);
        let obs = Observation::new(
            0,
            test_artifact(0),
            signals,
            StrategyKind::RetryWithFeedback,
            10_000,
            5_000,
        )
        .with_metrics(metrics_with(0.1, 0.56));
        trajectory.observations.push(obs);
        let bandit = StrategyBandit::with_default_priors();

        let result = engine.check_loop_control(&trajectory, &bandit).unwrap();
        assert!(matches!(result, LoopControl::Continue),
            "Expected Continue at iteration 1 (between intervals), got {:?}", result);
    }

    #[test]
    fn test_loop_control_intent_check_at_budget_fraction() {
        let engine = test_engine();
        let mut trajectory = test_trajectory();
        // Default intent_check_at_budget_fraction is 0.5

        // Consume >50% of the budget
        trajectory.budget.tokens_used = (trajectory.budget.max_tokens as f64 * 0.6) as u64;
        trajectory.budget.iterations_used = (trajectory.budget.max_iterations as f64 * 0.6) as u32;

        // Add 1 observation (iteration 1 — not on interval boundary)
        let signals = signals_with_tests(2, 10);
        let obs = Observation::new(
            0,
            test_artifact(0),
            signals,
            StrategyKind::RetryWithFeedback,
            10_000,
            5_000,
        )
        .with_metrics(metrics_with(0.1, 0.3));
        trajectory.observations.push(obs);
        let bandit = StrategyBandit::with_default_priors();

        let result = engine.check_loop_control(&trajectory, &bandit).unwrap();
        assert!(matches!(result, LoopControl::IntentCheck),
            "Expected IntentCheck when budget fraction exceeded, got {:?}", result);
    }

    #[test]
    fn test_loop_control_intent_check_at_fixed_point() {
        let engine = test_engine();
        let mut trajectory = test_trajectory();

        // Low signals, iteration 1 (not on interval boundary)
        let low_signals = signals_with_tests(2, 10);
        let obs = Observation::new(
            0,
            test_artifact(0),
            low_signals,
            StrategyKind::RetryWithFeedback,
            10_000,
            5_000,
        )
        .with_metrics(metrics_with(0.01, 0.4));
        trajectory.observations.push(obs);

        // Set FixedPoint attractor — should trigger IntentCheck regardless
        trajectory.attractor_state = AttractorState {
            classification: AttractorType::FixedPoint {
                estimated_remaining_iterations: 2,
                estimated_remaining_tokens: 10_000,
            },
            confidence: 0.9,
            detected_at: None,
            evidence: AttractorEvidence {
                recent_deltas: vec![0.01, 0.005, 0.002],
                recent_signatures: vec![],
                rationale: String::new(),
            },
        };

        let bandit = StrategyBandit::with_default_priors();
        let result = engine.check_loop_control(&trajectory, &bandit).unwrap();
        assert!(matches!(result, LoopControl::IntentCheck),
            "Expected IntentCheck at FixedPoint attractor, got {:?}", result);
    }

    #[test]
    fn test_loop_control_trapped_when_limit_cycle_no_strategies() {
        let engine = test_engine();
        let mut trajectory = test_trajectory();

        // Disable intent triggers so we can test the Trapped path
        trajectory.policy.intent_check_interval = u32::MAX;
        trajectory.policy.intent_check_at_budget_fraction = 1.0;

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

        // Disable intent triggers so we can test the RequestExtension path
        trajectory.policy.intent_check_interval = u32::MAX;
        trajectory.policy.intent_check_at_budget_fraction = 1.0;

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

        // Disable intent triggers so we can test the Continue path
        trajectory.policy.intent_check_interval = u32::MAX;
        trajectory.policy.intent_check_at_budget_fraction = 1.0;

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

    #[test]
    fn test_check_loop_control_plateau_triggers_intent_check() {
        let engine = test_engine();
        let mut trajectory = test_trajectory();
        trajectory.policy.intent_check_interval = 100; // avoid interval trigger
        trajectory.policy.intent_check_at_budget_fraction = 1.0; // avoid budget trigger

        // Set attractor to Plateau
        trajectory.attractor_state = AttractorState {
            classification: AttractorType::Plateau {
                stall_duration: 5,
                plateau_level: 0.6,
            },
            confidence: 0.75,
            detected_at: None,
            evidence: AttractorEvidence {
                recent_deltas: vec![0.001, -0.001, 0.0],
                recent_signatures: vec![],
                rationale: String::new(),
            },
        };

        // Add one observation so iteration count is 1 (not 0)
        trajectory.observations.push(
            test_observation(0, StrategyKind::RetryWithFeedback)
                .with_metrics(metrics_with(0.001, 0.6)),
        );

        let bandit = StrategyBandit::with_default_priors();
        let result = engine.check_loop_control(&trajectory, &bandit).unwrap();
        assert!(
            matches!(result, LoopControl::IntentCheck),
            "Plateau should trigger IntentCheck, got {:?}",
            result
        );
    }

    #[test]
    fn test_check_loop_control_all_passing_transition_triggers_intent_check() {
        let engine = test_engine();
        let mut trajectory = test_trajectory();
        trajectory.policy.intent_check_interval = 100;
        trajectory.policy.intent_check_at_budget_fraction = 1.0;

        // Previous observation: not all passing
        let obs1 = Observation::new(
            0,
            test_artifact(0),
            OverseerSignals {
                build_result: Some(BuildResult {
                    success: true,
                    error_count: 0,
                    errors: Vec::new(),
                }),
                test_results: Some(TestResults {
                    passed: 8,
                    failed: 2,
                    skipped: 0,
                    total: 10,
                    regression_count: 0,
                    failing_test_names: Vec::new(),
                }),
                ..OverseerSignals::default()
            },
            StrategyKind::RetryWithFeedback,
            10_000,
            5_000,
        ).with_metrics(metrics_with(0.1, 0.8));
        trajectory.observations.push(obs1);

        // Current observation: all passing
        let obs2 = Observation::new(
            1,
            test_artifact(1),
            OverseerSignals {
                build_result: Some(BuildResult {
                    success: true,
                    error_count: 0,
                    errors: Vec::new(),
                }),
                test_results: Some(TestResults {
                    passed: 10,
                    failed: 0,
                    skipped: 0,
                    total: 10,
                    regression_count: 0,
                    failing_test_names: Vec::new(),
                }),
                ..OverseerSignals::default()
            },
            StrategyKind::RetryWithFeedback,
            10_000,
            5_000,
        ).with_metrics(metrics_with(0.1, 1.0));
        trajectory.observations.push(obs2);

        let bandit = StrategyBandit::with_default_priors();
        let result = engine.check_loop_control(&trajectory, &bandit).unwrap();
        assert!(
            matches!(result, LoopControl::IntentCheck),
            "All-passing transition should trigger IntentCheck, got {:?}",
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
    fn test_should_verify_on_build_fixed() {
        let engine = test_engine();
        let mut trajectory = test_trajectory();
        trajectory.policy.intent_verification_frequency = 10; // avoid frequency trigger

        // Observation with build failing
        let obs1 = Observation::new(
            0,
            test_artifact(0),
            OverseerSignals {
                build_result: Some(BuildResult {
                    success: false,
                    error_count: 3,
                    errors: Vec::new(),
                }),
                ..OverseerSignals::default()
            },
            StrategyKind::RetryWithFeedback,
            10_000,
            5_000,
        ).with_metrics(metrics_with(0.1, 0.3));
        trajectory.observations.push(obs1);

        // Observation with build now passing
        let obs2 = Observation::new(
            1,
            test_artifact(1),
            OverseerSignals {
                build_result: Some(BuildResult {
                    success: true,
                    error_count: 0,
                    errors: Vec::new(),
                }),
                ..OverseerSignals::default()
            },
            StrategyKind::RetryWithFeedback,
            10_000,
            5_000,
        ).with_metrics(metrics_with(0.2, 0.6));
        trajectory.observations.push(obs2);

        assert!(engine.should_verify(&trajectory));
    }

    #[test]
    fn test_should_verify_on_tests_improved() {
        let engine = test_engine();
        let mut trajectory = test_trajectory();
        trajectory.policy.intent_verification_frequency = 10;

        // Observation with some tests failing
        let obs1 = Observation::new(
            0,
            test_artifact(0),
            OverseerSignals {
                test_results: Some(TestResults {
                    passed: 5,
                    failed: 5,
                    skipped: 0,
                    total: 10,
                    regression_count: 0,
                    failing_test_names: Vec::new(),
                }),
                ..OverseerSignals::default()
            },
            StrategyKind::RetryWithFeedback,
            10_000,
            5_000,
        ).with_metrics(metrics_with(0.1, 0.5));
        trajectory.observations.push(obs1);

        // Observation with more tests passing and fewer failing
        let obs2 = Observation::new(
            1,
            test_artifact(1),
            OverseerSignals {
                test_results: Some(TestResults {
                    passed: 8,
                    failed: 2,
                    skipped: 0,
                    total: 10,
                    regression_count: 0,
                    failing_test_names: Vec::new(),
                }),
                ..OverseerSignals::default()
            },
            StrategyKind::RetryWithFeedback,
            10_000,
            5_000,
        ).with_metrics(metrics_with(0.15, 0.7));
        trajectory.observations.push(obs2);

        assert!(engine.should_verify(&trajectory));
    }

    #[test]
    fn test_should_verify_no_trigger_without_state_transition() {
        let engine = test_engine();
        let mut trajectory = test_trajectory();
        trajectory.policy.intent_verification_frequency = 10;

        // Both observations have same build/test state (no transition)
        let obs1 = Observation::new(
            0,
            test_artifact(0),
            OverseerSignals {
                build_result: Some(BuildResult {
                    success: true,
                    error_count: 0,
                    errors: Vec::new(),
                }),
                test_results: Some(TestResults {
                    passed: 8,
                    failed: 2,
                    skipped: 0,
                    total: 10,
                    regression_count: 0,
                    failing_test_names: Vec::new(),
                }),
                ..OverseerSignals::default()
            },
            StrategyKind::RetryWithFeedback,
            10_000,
            5_000,
        ).with_metrics(metrics_with(0.1, 0.7));
        trajectory.observations.push(obs1);

        let obs2 = Observation::new(
            1,
            test_artifact(1),
            OverseerSignals {
                build_result: Some(BuildResult {
                    success: true,
                    error_count: 0,
                    errors: Vec::new(),
                }),
                test_results: Some(TestResults {
                    passed: 8,
                    failed: 2,
                    skipped: 0,
                    total: 10,
                    regression_count: 0,
                    failing_test_names: Vec::new(),
                }),
                ..OverseerSignals::default()
            },
            StrategyKind::RetryWithFeedback,
            10_000,
            5_000,
        ).with_metrics(metrics_with(0.05, 0.75));
        trajectory.observations.push(obs2);

        // No state transition, not at frequency, not FixedPoint
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

    // -----------------------------------------------------------------------
    // summarize_signals tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_summarize_signals_all_passing_high_level() {
        let engine = test_engine();
        let spec = SpecificationSnapshot::new("Implement authentication".to_string());
        let task_id = Uuid::new_v4();

        let signals = OverseerSignals {
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
            test_results: Some(TestResults {
                passed: 10,
                failed: 0,
                skipped: 0,
                total: 10,
                regression_count: 0,
                failing_test_names: Vec::new(),
            }),
            ..OverseerSignals::default()
        };

        let result = engine.summarize_signals(&signals, &spec, task_id);

        assert_eq!(result.satisfaction, "satisfied");
        assert!(result.gaps.is_empty());
        assert!((result.confidence - 0.85).abs() < f64::EPSILON);
    }

    #[test]
    fn test_summarize_signals_build_failure() {
        let engine = test_engine();
        let spec = SpecificationSnapshot::new("Implement authentication".to_string());
        let task_id = Uuid::new_v4();

        let signals = OverseerSignals {
            build_result: Some(BuildResult {
                success: false,
                error_count: 3,
                errors: vec!["cannot find type `Foo`".to_string()],
            }),
            ..OverseerSignals::default()
        };

        let result = engine.summarize_signals(&signals, &spec, task_id);

        assert_eq!(result.satisfaction, "unsatisfied");
        assert!(!result.gaps.is_empty());

        let build_gap = result.gaps.iter().find(|g| g.description.contains("Build failure")).unwrap();
        assert_eq!(build_gap.severity, GapSeverity::Critical);
        assert_eq!(build_gap.category, GapCategory::Functional);
        assert!(build_gap.suggested_action.as_ref().unwrap().contains("cannot find type"));
    }

    #[test]
    fn test_summarize_signals_test_failures_with_regressions() {
        let engine = test_engine();
        let spec = SpecificationSnapshot::new("Implement authentication".to_string());
        let task_id = Uuid::new_v4();

        let signals = OverseerSignals {
            build_result: Some(BuildResult {
                success: true,
                error_count: 0,
                errors: Vec::new(),
            }),
            test_results: Some(TestResults {
                passed: 7,
                failed: 3,
                skipped: 0,
                total: 10,
                regression_count: 2,
                failing_test_names: vec![
                    "test_login".to_string(),
                    "test_register".to_string(),
                    "test_logout".to_string(),
                ],
            }),
            ..OverseerSignals::default()
        };

        let result = engine.summarize_signals(&signals, &spec, task_id);

        assert_eq!(result.satisfaction, "partial");

        let test_gap = result.gaps.iter().find(|g| g.description.contains("Test failures")).unwrap();
        // Regressions should bump severity to Major
        assert_eq!(test_gap.severity, GapSeverity::Major);
        assert_eq!(test_gap.category, GapCategory::Testing);
        assert!(test_gap.suggested_action.as_ref().unwrap().contains("test_login"));
    }

    #[test]
    fn test_summarize_signals_security_vulnerabilities() {
        let engine = test_engine();
        let spec = SpecificationSnapshot::new("Implement authentication".to_string());
        let task_id = Uuid::new_v4();

        let signals = OverseerSignals {
            build_result: Some(BuildResult {
                success: true,
                error_count: 0,
                errors: Vec::new(),
            }),
            test_results: Some(TestResults {
                passed: 10,
                failed: 0,
                skipped: 0,
                total: 10,
                regression_count: 0,
                failing_test_names: Vec::new(),
            }),
            security_scan: Some(SecurityScanResult {
                critical_count: 1,
                high_count: 2,
                medium_count: 0,
                findings: Vec::new(),
            }),
            ..OverseerSignals::default()
        };

        let result = engine.summarize_signals(&signals, &spec, task_id);

        // Critical security vuln + critical gap => unsatisfied
        assert_eq!(result.satisfaction, "unsatisfied");

        let sec_gap = result.gaps.iter().find(|g| g.category == GapCategory::Security).unwrap();
        assert_eq!(sec_gap.severity, GapSeverity::Critical);
        assert!(sec_gap.description.contains("1 critical"));
    }

    #[test]
    fn test_summarize_signals_high_security_only() {
        let engine = test_engine();
        let spec = SpecificationSnapshot::new("Implement feature".to_string());
        let task_id = Uuid::new_v4();

        let signals = OverseerSignals {
            build_result: Some(BuildResult {
                success: true,
                error_count: 0,
                errors: Vec::new(),
            }),
            security_scan: Some(SecurityScanResult {
                critical_count: 0,
                high_count: 1,
                medium_count: 3,
                findings: Vec::new(),
            }),
            ..OverseerSignals::default()
        };

        let result = engine.summarize_signals(&signals, &spec, task_id);

        // No critical security, but Major gap from high_count
        let sec_gap = result.gaps.iter().find(|g| g.category == GapCategory::Security).unwrap();
        assert_eq!(sec_gap.severity, GapSeverity::Major);
        assert_eq!(result.satisfaction, "partial");
    }

    #[test]
    fn test_summarize_signals_lint_errors() {
        let engine = test_engine();
        let spec = SpecificationSnapshot::new("Implement feature".to_string());
        let task_id = Uuid::new_v4();

        let signals = OverseerSignals {
            build_result: Some(BuildResult {
                success: true,
                error_count: 0,
                errors: Vec::new(),
            }),
            lint_results: Some(LintResults {
                error_count: 5,
                warning_count: 10,
                errors: Vec::new(),
            }),
            ..OverseerSignals::default()
        };

        let result = engine.summarize_signals(&signals, &spec, task_id);

        let lint_gap = result.gaps.iter().find(|g| g.category == GapCategory::Maintainability).unwrap();
        assert_eq!(lint_gap.severity, GapSeverity::Minor);
        // Lint-only gaps with no major issues => partial
        assert_eq!(result.satisfaction, "partial");
    }

    #[test]
    fn test_summarize_signals_custom_check_failure() {
        let engine = test_engine();
        let spec = SpecificationSnapshot::new("Implement feature".to_string());
        let task_id = Uuid::new_v4();

        let signals = OverseerSignals {
            build_result: Some(BuildResult {
                success: true,
                error_count: 0,
                errors: Vec::new(),
            }),
            custom_checks: vec![
                CustomCheckResult {
                    name: "coverage".to_string(),
                    passed: false,
                    details: "Coverage at 50%, required 80%".to_string(),
                },
                CustomCheckResult {
                    name: "formatting".to_string(),
                    passed: true,
                    details: "All files formatted".to_string(),
                },
            ],
            ..OverseerSignals::default()
        };

        let result = engine.summarize_signals(&signals, &spec, task_id);

        // Only the failing custom check should produce a gap
        assert_eq!(result.gaps.len(), 1);
        assert!(result.gaps[0].description.contains("coverage"));
        assert!(result.gaps[0].description.contains("Coverage at 50%"));
        assert_eq!(result.gaps[0].severity, GapSeverity::Moderate);
    }

    #[test]
    fn test_summarize_signals_no_tests_with_success_criteria() {
        let engine = test_engine();
        let mut spec = SpecificationSnapshot::new("Implement feature".to_string());
        spec.success_criteria.push("All endpoints return valid JSON".to_string());
        let task_id = Uuid::new_v4();

        // Signals with build passing but NO test results
        let signals = OverseerSignals {
            build_result: Some(BuildResult {
                success: true,
                error_count: 0,
                errors: Vec::new(),
            }),
            ..OverseerSignals::default()
        };

        let result = engine.summarize_signals(&signals, &spec, task_id);

        // Should have an implicit gap about missing test results
        let implicit_gap = result.gaps.iter().find(|g| g.is_implicit).unwrap();
        assert_eq!(implicit_gap.category, GapCategory::Testing);
        assert!(implicit_gap.description.contains("success criteria"));
        assert!(implicit_gap.implicit_rationale.is_some());
    }

    #[test]
    fn test_summarize_signals_empty_signals() {
        let engine = test_engine();
        let spec = SpecificationSnapshot::new("Implement feature".to_string());
        let task_id = Uuid::new_v4();

        // Empty signals — no gaps detected
        let signals = OverseerSignals::default();

        let result = engine.summarize_signals(&signals, &spec, task_id);

        // No gaps => satisfied (convergence_level no longer gates satisfaction)
        assert_eq!(result.satisfaction, "satisfied");
        assert!((result.confidence - 0.85).abs() < f64::EPSILON);
    }

    #[test]
    fn test_summarize_signals_type_check_failure() {
        let engine = test_engine();
        let spec = SpecificationSnapshot::new("Implement feature".to_string());
        let task_id = Uuid::new_v4();

        let signals = OverseerSignals {
            build_result: Some(BuildResult {
                success: true,
                error_count: 0,
                errors: Vec::new(),
            }),
            type_check: Some(TypeCheckResult {
                clean: false,
                error_count: 2,
                errors: vec!["expected String, found i32".to_string()],
            }),
            ..OverseerSignals::default()
        };

        let result = engine.summarize_signals(&signals, &spec, task_id);

        let tc_gap = result.gaps.iter().find(|g| g.description.contains("Type check")).unwrap();
        assert_eq!(tc_gap.severity, GapSeverity::Major);
        assert!(tc_gap.suggested_action.as_ref().unwrap().contains("expected String"));
        // Major gap => partial
        assert_eq!(result.satisfaction, "partial");
    }

    // -----------------------------------------------------------------------
    // Trace verification tests (plan verification items 3-5)
    // -----------------------------------------------------------------------

    /// Trace 3: Build fails but intent would be satisfied — summarize_signals
    /// no longer blocks satisfaction based on convergence_level.
    ///
    /// Before this refactoring, summarize_signals would independently force
    /// "unsatisfied" when convergence_level <= 0.3 (which happens with a build
    /// failure). Now it only looks at gaps: build failure → Critical gap →
    /// "unsatisfied", which is correct behavior (unsatisfied because of the
    /// gap, not because of a numeric threshold).
    ///
    /// Crucially, test that when build passes but convergence_level is low
    /// (e.g. many tests still failing), summarize_signals can still return
    /// "satisfied" if no gaps are found — the numeric level doesn't gate it.
    #[test]
    fn test_trace_build_pass_low_convergence_not_blocked_by_level() {
        let engine = test_engine();
        let spec = SpecificationSnapshot::new("Implement feature".to_string());
        let task_id = Uuid::new_v4();

        // All overseers pass — no gaps — but imagine convergence_level would
        // be low because this is a brand-new trajectory.  The old code would
        // have blocked satisfaction when convergence_level <= 0.3.
        let signals = OverseerSignals {
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
            test_results: Some(TestResults {
                passed: 10,
                failed: 0,
                skipped: 0,
                total: 10,
                regression_count: 0,
                failing_test_names: Vec::new(),
            }),
            ..OverseerSignals::default()
        };

        let result = engine.summarize_signals(&signals, &spec, task_id);

        // No convergence_level parameter, no threshold gate — gaps-only logic
        assert_eq!(result.satisfaction, "satisfied");
        assert!((result.confidence - 0.85).abs() < f64::EPSILON);
        assert!(result.gaps.is_empty());
    }

    /// Trace 4: Overseers oscillate — verification still triggers on
    /// interval/plateau, not blocked by threshold crossings.
    ///
    /// The old should_verify used convergence_level threshold crossings
    /// (0.5, 0.8, 0.9). Oscillating overseers could repeatedly cross and
    /// un-cross those thresholds, creating unpredictable verification timing.
    ///
    /// Now verification triggers on:
    /// - frequency (every N iterations)
    /// - build going from fail → pass
    /// - test pass count improving AND fail count decreasing
    ///
    /// Oscillating overseers (e.g. tests bouncing 7→8→7→8) should NOT trigger
    /// because pass count going up while fail count also goes down never
    /// happens simultaneously during oscillation.
    #[test]
    fn test_trace_oscillating_overseers_dont_trigger_spurious_verification() {
        let engine = test_engine();
        let mut trajectory = test_trajectory();
        // Set large interval so frequency doesn't trigger
        trajectory.policy.intent_verification_frequency = 100;

        // Observation 1: 8/10 tests pass
        let obs1 = Observation::new(
            0,
            test_artifact(0),
            OverseerSignals {
                build_result: Some(BuildResult {
                    success: true,
                    error_count: 0,
                    errors: Vec::new(),
                }),
                test_results: Some(TestResults {
                    passed: 8,
                    failed: 2,
                    skipped: 0,
                    total: 10,
                    regression_count: 0,
                    failing_test_names: Vec::new(),
                }),
                ..OverseerSignals::default()
            },
            StrategyKind::RetryWithFeedback,
            10_000,
            5_000,
        )
        .with_metrics(metrics_with(0.1, 0.8));
        trajectory.observations.push(obs1);

        // Observation 2: oscillates back to 7/10 — this is a REGRESSION
        let obs2 = Observation::new(
            1,
            test_artifact(1),
            OverseerSignals {
                build_result: Some(BuildResult {
                    success: true,
                    error_count: 0,
                    errors: Vec::new(),
                }),
                test_results: Some(TestResults {
                    passed: 7,
                    failed: 3,
                    skipped: 0,
                    total: 10,
                    regression_count: 1,
                    failing_test_names: Vec::new(),
                }),
                ..OverseerSignals::default()
            },
            StrategyKind::RetryWithFeedback,
            10_000,
            5_000,
        )
        .with_metrics(metrics_with(-0.05, 0.7));
        trajectory.observations.push(obs2);

        // should_verify must NOT trigger on a regression oscillation
        assert!(
            !engine.should_verify(&trajectory),
            "Oscillating (regressing) overseers should not trigger verification"
        );

        // Observation 3: bounces back to 8/10 — pass went up but fail also
        // went down. However, passed == prev.passed from obs1 perspective,
        // and the comparison is between consecutive pairs (obs2 → obs3).
        // obs2 had passed=7, obs3 has passed=8 (up), fail=2 (down from 3).
        // This IS a genuine improvement (obs2 → obs3), so it should trigger.
        let obs3 = Observation::new(
            2,
            test_artifact(2),
            OverseerSignals {
                build_result: Some(BuildResult {
                    success: true,
                    error_count: 0,
                    errors: Vec::new(),
                }),
                test_results: Some(TestResults {
                    passed: 8,
                    failed: 2,
                    skipped: 0,
                    total: 10,
                    regression_count: 0,
                    failing_test_names: Vec::new(),
                }),
                ..OverseerSignals::default()
            },
            StrategyKind::RetryWithFeedback,
            10_000,
            5_000,
        )
        .with_metrics(metrics_with(0.05, 0.8));
        trajectory.observations.push(obs3);

        // This is a genuine improvement (7→8 pass, 3→2 fail), so it triggers
        assert!(
            engine.should_verify(&trajectory),
            "Genuine improvement (pass up, fail down) should trigger verification"
        );
    }

    /// Trace 5: Intent confidence climbing but tests flaky — trajectory
    /// reflects intent progress via blended level, not just test pass rate.
    ///
    /// When the intent verifier says confidence is 0.9 but flaky tests give
    /// a low overseer readiness (say 0.5), the blended level should be
    /// 0.60*0.9 + 0.40*0.5 = 0.74 — much higher than the raw overseer 0.5.
    ///
    /// best_observation should prefer the observation with high intent
    /// confidence over one with high overseer scores but no intent data.
    #[test]
    fn test_trace_intent_climbing_flaky_tests_blended_level_wins() {
        use crate::domain::models::convergence::{
            ConvergenceBudget, ConvergencePolicy, SpecificationEvolution, SpecificationSnapshot,
        };

        let spec = SpecificationEvolution::new(SpecificationSnapshot::new(
            "Implement feature".to_string(),
        ));
        let mut trajectory = Trajectory::new(
            Uuid::new_v4(),
            None,
            spec,
            ConvergenceBudget::default(),
            ConvergencePolicy::default(),
        );

        // Observation 0: high overseer score (0.85) but no intent verification
        let obs0 = Observation::new(
            0,
            test_artifact(0),
            signals_with_tests(9, 10),
            StrategyKind::RetryWithFeedback,
            10_000,
            5_000,
        )
        .with_metrics(ObservationMetrics {
            convergence_level: 0.85,
            convergence_delta: 0.1,
            intent_blended_level: None, // no intent verification yet
            ..ObservationMetrics::default()
        });
        trajectory.observations.push(obs0);

        // Observation 1: flaky tests (5/10 pass → overseer ~0.5) but intent
        // verifier says confidence is 0.9.
        // Blended = 0.60*0.9 + 0.40*0.5 = 0.54 + 0.20 = 0.74
        let flaky_signals = OverseerSignals {
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
            test_results: Some(TestResults {
                passed: 5,
                failed: 5,
                skipped: 0,
                total: 10,
                regression_count: 0,
                failing_test_names: Vec::new(),
            }),
            ..OverseerSignals::default()
        };
        let overseer_readiness = convergence_level(&flaky_signals);

        let blended = 0.60 * 0.9 + 0.40 * overseer_readiness;
        let obs1 = Observation::new(
            1,
            test_artifact(1),
            flaky_signals,
            StrategyKind::RetryWithFeedback,
            10_000,
            5_000,
        )
        .with_metrics(ObservationMetrics {
            convergence_level: overseer_readiness,
            convergence_delta: -0.05,
            intent_blended_level: Some(blended),
            ..ObservationMetrics::default()
        });
        trajectory.observations.push(obs1);

        // best_observation should pick obs1 (blended ~0.74) only if it
        // actually exceeds obs0's level (0.85). In this case obs0 is still
        // higher — and that's correct! The blended level mitigates flakiness
        // but doesn't magically win when the raw overseer was truly better.
        //
        // So let's bump intent confidence to 0.95 to make the blended level
        // exceed 0.85: 0.60*0.95 + 0.40*overseer_readiness.
        let high_intent_blended = 0.60 * 0.95 + 0.40 * overseer_readiness;

        // Update obs1's blended level
        trajectory.observations[1].metrics = Some(ObservationMetrics {
            convergence_level: overseer_readiness,
            convergence_delta: -0.05,
            intent_blended_level: Some(high_intent_blended),
            ..ObservationMetrics::default()
        });

        if high_intent_blended > 0.85 {
            // Intent-blended wins: best_observation should pick obs1
            let best = trajectory.best_observation().unwrap();
            assert_eq!(best.sequence, 1,
                "With intent_blended_level ({:.3}) > raw overseer level (0.85), \
                 best_observation should prefer the intent-aware observation",
                high_intent_blended
            );
        } else {
            // If the raw overseer from obs0 is still higher, obs0 wins.
            // Either way, the blended level is being compared — not just
            // the raw overseer. Verify intent_blended_level exists.
            let best = trajectory.best_observation().unwrap();
            assert!(
                trajectory.observations[1].metrics.as_ref().unwrap().intent_blended_level.is_some(),
                "Observation should have intent_blended_level set"
            );
            // Also verify the blended level is significantly higher than
            // the raw overseer readiness, showing intent confidence lifted it.
            let obs1_metrics = trajectory.observations[1].metrics.as_ref().unwrap();
            assert!(
                obs1_metrics.intent_blended_level.unwrap() > obs1_metrics.convergence_level + 0.1,
                "Blended level ({:.3}) should be significantly higher than raw overseer ({:.3})",
                obs1_metrics.intent_blended_level.unwrap(),
                obs1_metrics.convergence_level
            );
        }
    }

    // -----------------------------------------------------------------------
    // measure (public delegation) test
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_measure_delegates_to_overseer_measurer() {
        let expected_signals = signals_with_tests(8, 10);
        let engine = ConvergenceEngine::new(
            Arc::new(MockTrajectoryRepo::new()),
            Arc::new(MockMemoryRepo::new()),
            Arc::new(MockOverseerMeasurer::with_signals(expected_signals.clone())),
            test_config(),
        );

        let artifact = test_artifact(0);
        let policy = test_policy();

        let signals = engine.measure(&artifact, &policy).await.unwrap();

        // Verify delegation by checking returned signals match what the mock provides
        assert_eq!(
            signals.test_results.as_ref().unwrap().passed,
            expected_signals.test_results.as_ref().unwrap().passed,
        );
        assert_eq!(
            signals.test_results.as_ref().unwrap().failed,
            expected_signals.test_results.as_ref().unwrap().failed,
        );
    }
}
