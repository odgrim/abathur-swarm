//! Convergence engine -- 6.3/6.4/6.6 iterate phase.
//!
//! Main convergence loop, per-iteration body, loop-control decisions,
//! parallel trajectory sampling, strategy execution and measurement.

use chrono::Utc;
use uuid::Uuid;

use crate::domain::errors::DomainResult;
use crate::domain::models::convergence::*;
use crate::domain::models::intent_verification::{GapCategory, GapSeverity, IntentGap};
use crate::domain::ports::{MemoryRepository, TrajectoryRepository};

use super::{ConvergenceDomainEvent, ConvergenceEngine, OverseerMeasurer, StrategyContext};

impl<T: TrajectoryRepository, M: MemoryRepository, O: OverseerMeasurer> ConvergenceEngine<T, M, O> {
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
        if trajectory.policy.generate_acceptance_tests || infrastructure.acceptance_tests.is_empty()
        {
            let tests = self
                .generate_acceptance_tests(&trajectory, infrastructure)
                .await?;
            if !tests.is_empty() {
                self.event_sink
                    .emit(ConvergenceDomainEvent::AcceptanceTestsGenerated { count: tests.len() })
                    .await;
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
            let invariants = self.infer_invariants(&trajectory, infrastructure).await?;
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
        if self.config.enable_proactive_decomposition
            && let Some(outcome) = self.maybe_decompose_proactively(&mut trajectory).await?
        {
            return Ok(outcome);
        }

        // Note: Parallel-mode routing was removed in PR 5 along with
        // `converge_parallel` (architecturally unsound under real substrate:
        // Thompson sampling over paid agent sessions). Parallel exploration
        // is now handled exclusively by the orchestrator's
        // `run_parallel_convergent_execution` path, which implements a
        // correct Phase-1 fan-out + sequential-continuation scheme.

        // -- ITERATE phase --
        trajectory.phase = ConvergencePhase::Iterating;

        // Initialize strategy bandit
        let mut bandit = self.initialize_bandit(&trajectory).await;

        loop {
            // 0. Global budget pressure check (S8 fix).
            //
            // If a global BudgetTracker is wired in, verify that we are not at
            // Critical pressure before proceeding with the next iteration.
            if let Some(ref tracker) = self.budget_tracker
                && tracker.should_pause_new_work().await
            {
                self.event_sink
                    .emit(ConvergenceDomainEvent::BudgetCriticalTerminating {
                        trajectory_id: trajectory.id.to_string(),
                    })
                    .await;
                return Ok(ConvergenceOutcome::BudgetDenied {
                    trajectory_id: trajectory.id.to_string(),
                });
            }

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
                        obs.iter().filter(|o| o.metrics.is_some()).max_by(|a, b| {
                            let am = a.metrics.as_ref().unwrap();
                            let bm = b.metrics.as_ref().unwrap();
                            let al = am.intent_blended_level.unwrap_or(am.convergence_level);
                            let bl = bm.intent_blended_level.unwrap_or(bm.convergence_level);
                            al.partial_cmp(&bl).unwrap_or(std::cmp::Ordering::Equal)
                        })
                    },
                );
                trajectory.forced_strategy = Some(StrategyKind::FreshStart {
                    carry_forward: Box::new(carry),
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
                            .take_while(|e| e.strategy_kind.kind_name() == current.kind_name())
                            .count() as u32;
                        let recent_deltas: Vec<f64> = trajectory
                            .strategy_log
                            .iter()
                            .rev()
                            .take_while(|e| e.strategy_kind.kind_name() == current.kind_name())
                            .filter_map(|e| e.convergence_delta_achieved)
                            .collect::<Vec<_>>()
                            .into_iter()
                            .rev()
                            .collect();

                        if should_rotate_strategy(current, consecutive_uses, &recent_deltas) {
                            let current_name = current.kind_name();
                            self.event_sink
                                .emit(ConvergenceDomainEvent::StrategyRotationTriggered {
                                    strategy: current_name,
                                    consecutive_uses,
                                })
                                .await;
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
            let mut observation = self
                .measure_artifact(&artifact, &strategy, &trajectory)
                .await?;
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
                    if trajectory.policy.partial_acceptance
                        && let Some(best) = trajectory.best_observation()
                    {
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
                            self.finalize(&mut trajectory, &outcome, &bandit).await?;
                            return Ok(outcome);
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
                LoopControl::OverseerConverged => {
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

        // Push observation (bounded to prevent unbounded JSON growth)
        trajectory.push_observation_bounded(obs_with_metrics);
        trajectory.push_strategy_log_bounded(entry);

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
        let new_name = self.attractor_type_name(&trajectory.attractor_state.classification);
        if prev_name != new_name {
            self.event_sink
                .emit(ConvergenceDomainEvent::AttractorTransition {
                    trajectory_id: trajectory.id.to_string(),
                    from: prev_name,
                    to: new_name,
                })
                .await;
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

    /// Returns `true` when the latest observation's overseer signals are ambiguous
    /// for the `OverseerConverged` shortcircuit.
    ///
    /// "Ambiguous" means the specification has success criteria that require test
    /// evidence to verify, but the latest observation has no `test_results`.
    /// `all_passing_relative()` treats absent `test_results` as passing, so a
    /// build-only result can satisfy the all-passing gate without any test evidence.
    ///
    /// When signals are ambiguous the caller should emit `IntentCheck` instead of
    /// `OverseerConverged`, satisfying the no-premature-termination constraint.
    fn overseer_signals_are_ambiguous(trajectory: &Trajectory) -> bool {
        if trajectory
            .specification
            .effective
            .success_criteria
            .is_empty()
        {
            return false; // task has no testable criteria → build-only is sufficient
        }
        trajectory
            .observations
            .last()
            .map(|o| o.overseer_signals.test_results.is_none())
            .unwrap_or(true)
    }

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
        let lint_baseline = trajectory.lint_baseline;
        // 1. Budget exhaustion check
        if !trajectory.budget.has_remaining() {
            // Check if we should request an extension
            let delta_positive = trajectory.latest_convergence_delta() > 0.0;
            if trajectory.budget.should_request_extension(delta_positive) {
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

        let interval_trigger = iteration > 0 && iteration.is_multiple_of(interval);

        let at_fixed_point = matches!(
            &trajectory.attractor_state.classification,
            AttractorType::FixedPoint { .. }
        );

        let budget_fraction_used = 1.0 - trajectory.budget.remaining_fraction();
        let budget_trigger =
            budget_fraction_used >= trajectory.policy.intent_check_at_budget_fraction;

        // Plateau trigger: stalled trajectory is a natural "should we check
        // if we're done?" point.
        let at_plateau = matches!(
            &trajectory.attractor_state.classification,
            AttractorType::Plateau { .. }
        );

        // All-overseers-passing trigger: first transition to all-passing
        // means static checks have nothing more to report — ask intent if
        // work is complete. Uses relative lint check so pre-existing warnings
        // don't block the transition.
        let all_passing_transition = if trajectory.observations.len() >= 2 {
            let curr = &trajectory.observations[trajectory.observations.len() - 1];
            let prev = &trajectory.observations[trajectory.observations.len() - 2];
            curr.overseer_signals.all_passing_relative(lint_baseline)
                && !prev.overseer_signals.all_passing_relative(lint_baseline)
        } else {
            false
        };

        // 2b. OverseerConverged shortcircuit: FixedPoint + all overseers passing
        // for 2+ consecutive observations = overseer-confirmed convergence.
        // Skip IntentCheck entirely since static checks objectively confirm
        // the work is correct and the trajectory has stabilized.
        if at_fixed_point {
            let consecutive_all_passing = trajectory
                .observations
                .iter()
                .rev()
                .take_while(|o| {
                    o.overseer_signals.all_passing_relative(lint_baseline)
                        && o.overseer_signals.has_any_signal()
                })
                .count();

            if consecutive_all_passing >= 2 {
                // Guard: fall back to IntentCheck when signals are ambiguous (success
                // criteria present but no test evidence). Satisfies the
                // no-premature-termination constraint.
                if Self::overseer_signals_are_ambiguous(trajectory) {
                    return Ok(LoopControl::IntentCheck);
                }
                return Ok(LoopControl::OverseerConverged);
            }
        }

        if interval_trigger
            || at_fixed_point
            || budget_trigger
            || at_plateau
            || all_passing_transition
        {
            return Ok(LoopControl::IntentCheck);
        }

        // 3. Trapped check -- limit cycle with no eligible escape strategies
        if let AttractorType::LimitCycle { .. } = &trajectory.attractor_state.classification {
            // If overseers confirm work is correct, don't try escape strategies —
            // the work is done, escape strategies just waste tokens.
            let latest_passing = trajectory
                .observations
                .last()
                .map(|o| {
                    o.overseer_signals.all_passing_relative(lint_baseline)
                        && o.overseer_signals.has_any_signal()
                })
                .unwrap_or(false);
            if latest_passing {
                // Same ambiguity guard as FixedPoint case above.
                if Self::overseer_signals_are_ambiguous(trajectory) {
                    return Ok(LoopControl::IntentCheck);
                }
                return Ok(LoopControl::OverseerConverged);
            }

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
                && trajectory
                    .budget
                    .allows_strategy_cost(&StrategyKind::Decompose)
            {
                return Ok(LoopControl::Decompose);
            }
        }

        // 5. Near-budget extension check
        let delta_positive = trajectory.latest_convergence_delta() > 0.0;
        if trajectory.budget.should_request_extension(delta_positive) {
            return Ok(LoopControl::RequestExtension);
        }

        // Default: continue iterating
        Ok(LoopControl::Continue)
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
            && obs_count.is_multiple_of(trajectory.policy.intent_verification_frequency)
        {
            return true;
        }

        // Check overseer state transitions (replaces threshold crossings)
        if trajectory.observations.len() >= 2 {
            let curr = &trajectory.observations[trajectory.observations.len() - 1];
            let prev = &trajectory.observations[trajectory.observations.len() - 2];

            // Build went from failing to passing
            let build_fixed = match (
                &prev.overseer_signals.build_result,
                &curr.overseer_signals.build_result,
            ) {
                (Some(prev_b), Some(curr_b)) => !prev_b.success && curr_b.success,
                _ => false,
            };
            if build_fixed {
                return true;
            }

            // Test pass count improved and fail count decreased
            let tests_improved = match (
                &prev.overseer_signals.test_results,
                &curr.overseer_signals.test_results,
            ) {
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
    pub(super) fn summarize_signals(
        &self,
        signals: &OverseerSignals,
        spec: &SpecificationSnapshot,
        task_id: Uuid,
    ) -> VerificationResult {
        let mut gaps: Vec<IntentGap> = Vec::new();

        // Build failure is a critical gap
        if let Some(ref build) = signals.build_result
            && !build.success
        {
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

        // Type check failure
        if let Some(ref tc) = signals.type_check
            && !tc.clean
        {
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

        // Failing tests
        if let Some(ref tests) = signals.test_results
            && tests.failed > 0
        {
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
        if let Some(ref lint) = signals.lint_results
            && lint.error_count > 0
        {
            gaps.push(
                IntentGap::new(
                    format!("Lint errors: {} error(s)", lint.error_count),
                    GapSeverity::Minor,
                )
                .with_category(GapCategory::Maintainability)
                .with_task(task_id),
            );
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
        let has_critical = gaps
            .iter()
            .any(|g| matches!(g.severity, GapSeverity::Critical));
        let has_major = gaps
            .iter()
            .any(|g| matches!(g.severity, GapSeverity::Major));

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
    pub(super) async fn execute_strategy(
        &self,
        strategy: &StrategyKind,
        trajectory: &mut Trajectory,
    ) -> DomainResult<(ArtifactReference, u64, u64)> {
        let start = std::time::Instant::now();

        // Build strategy-specific context
        let _context = self.build_strategy_context(strategy, trajectory);

        // Log the strategy execution
        self.event_sink
            .emit(ConvergenceDomainEvent::StrategyExecutionStarted {
                strategy: strategy.kind_name(),
                trajectory_id: trajectory.id.to_string(),
            })
            .await;

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
                    self.attractor_type_name(&trajectory.attractor_state.classification),
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

                self.event_sink
                    .emit(ConvergenceDomainEvent::ArchitectReviewAmended {
                        trajectory_id: trajectory.id.to_string(),
                        total_amendments: trajectory.specification.amendments.len(),
                    })
                    .await;
            }
            StrategyKind::FreshStart { carry_forward } => {
                // Fresh start resets context but preserves filesystem and
                // trajectory metadata
                self.event_sink
                    .emit(ConvergenceDomainEvent::FreshStartInitiated {
                        trajectory_id: trajectory.id.to_string(),
                        carry_forward_hints: carry_forward.hints.len(),
                        observation_count: trajectory.observations.len(),
                    })
                    .await;
            }
            StrategyKind::RevertAndBranch { target } => {
                // Spec 4.1: RevertAndBranch finds the target observation
                // and uses its artifact as the starting point.
                self.event_sink
                    .emit(ConvergenceDomainEvent::RevertAndBranchInitiated {
                        trajectory_id: trajectory.id.to_string(),
                        target: target.to_string(),
                    })
                    .await;

                // Find the target observation and return its artifact
                if let Some(target_obs) =
                    trajectory.observations.iter().find(|obs| obs.id == *target)
                {
                    let artifact = target_obs.artifact.clone();
                    let elapsed = start.elapsed();
                    let estimated_tokens = strategy.estimated_cost();
                    return Ok((artifact, estimated_tokens, elapsed.as_millis() as u64));
                }
                // If target not found, fall through to default artifact
                self.event_sink
                    .emit(ConvergenceDomainEvent::RevertAndBranchTargetMissing {
                        trajectory_id: trajectory.id.to_string(),
                        target: target.to_string(),
                    })
                    .await;
            }
            _ => {}
        }

        // The actual LLM execution is delegated to the caller / agent runtime.
        // The engine records the strategy context and expects the artifact to be
        // produced by the runtime. For now, return the latest artifact or a
        // placeholder. In integration, this would await the agent's execution.
        let artifact = trajectory.latest_artifact().cloned().unwrap_or_else(|| {
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
    pub(crate) fn build_strategy_context(
        &self,
        strategy: &StrategyKind,
        trajectory: &Trajectory,
    ) -> StrategyContext {
        let latest_signals = trajectory.latest_overseer_signals().cloned();

        let carry_forward = match strategy {
            StrategyKind::FreshStart { carry_forward } => Some((**carry_forward).clone()),
            _ => None,
        };

        // Build hints from trajectory hints and strategy-specific guidance
        let mut hints = trajectory.hints.clone();
        match strategy {
            StrategyKind::FocusedRepair => {
                // Add failing test names as focus hints
                if let Some(signals) = &latest_signals
                    && let Some(ref tests) = signals.test_results
                {
                    for test_name in &tests.failing_test_names {
                        hints.push(format!("Focus on fixing: {}", test_name));
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
                hints.push("Try a fundamentally different implementation strategy".to_string());
            }
            StrategyKind::IncrementalRefinement => {
                hints.push("Address one gap at a time, smallest first".to_string());
            }
            _ => {}
        }

        // Build focus areas from recent overseer feedback
        let mut focus_areas = Vec::new();
        if let Some(signals) = &latest_signals {
            if let Some(ref build) = signals.build_result
                && !build.success
            {
                focus_areas.push("Fix build errors first".to_string());
                for error in &build.errors {
                    focus_areas.push(format!("Build error: {}", error));
                }
            }
            if let Some(ref tc) = signals.type_check
                && !tc.clean
            {
                focus_areas.push("Resolve type errors".to_string());
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
    pub(super) async fn measure_artifact(
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
    pub(super) async fn generate_acceptance_tests(
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
    pub(super) async fn infer_invariants(
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
    pub(super) async fn detect_test_contradictions(
        &self,
        _trajectory: &Trajectory,
        _infrastructure: &ConvergenceInfrastructure,
    ) -> DomainResult<Vec<String>> {
        Ok(vec![])
    }
}
