//! Convergence engine -- `run()` entrypoint (Phase A of PR 4 in the
//! engine-as-core refactor chain, #13/#21).
//!
//! The [`ConvergenceEngine::run`] method is the new primary entrypoint: unlike
//! the legacy [`ConvergenceEngine::converge`] which treats `IntentCheck` as a
//! continue and has no notion of cancellation or per-iteration policy
//! adjustment, `run()` dispatches every finality-gate decision and the
//! top-of-iteration gate through the [`ConvergenceAdvisor`] port, dispatches
//! substrate invocations through the [`StrategyExecutor`] port, and dispatches
//! FreshStart / RevertAndBranch side effects through the [`StrategyEffects`]
//! port.
//!
//! # Phase A vs Phase B
//!
//! Phase A (this PR) introduces `run()` alongside the existing `converge()`
//! and leaves the orchestrator's `run_convergent_execution_inner` untouched.
//! `run()` is fully wired but unused in production until Phase B (PR 4b) flips
//! the orchestrator's inner loop to build the port impls and call it.
//!
//! # Required ports
//!
//! `run()` requires all three ports to be installed via the `with_executor` /
//! `with_effects` / `with_advisor` builders. Missing ports produce a
//! [`DomainError::InvalidConfiguration`]-equivalent error rather than a panic
//! so test harnesses that mis-configure an engine surface the mistake
//! explicitly.

use std::sync::Arc;
use uuid::Uuid;

use crate::domain::errors::{DomainError, DomainResult};
use crate::domain::models::convergence::*;
use crate::domain::ports::{MemoryRepository, TrajectoryRepository};

use super::ports::{
    AdvisorDirective, ConvergenceAdvisor, ConvergenceDomainEvent, ConvergenceRunOutcome,
    IterationGate, PolicyOverlay, PromptBuilder, StrategyExecutionContext, StrategyExecutionOutput,
    StrategyExecutor,
};
use super::{ConvergenceEngine, OverseerMeasurer};

impl<T: TrajectoryRepository, M: MemoryRepository, O: OverseerMeasurer> ConvergenceEngine<T, M, O> {
    /// Per-call variant of [`Self::run`] that takes the ports as arguments
    /// instead of reading them from the engine's installed builder fields.
    ///
    /// Used by the orchestrator's `run_convergent_execution_inner` (PR 4b)
    /// where the engine's caller already holds a constructed engine and
    /// building up a new one via `with_executor` etc. is awkward.
    ///
    /// `effects`, `prompt_builder`, and `event_sink` are optional; when
    /// `event_sink` is `None`, the engine's existing sink (default
    /// `TracingEventSink`) is used.
    #[allow(clippy::too_many_arguments)]
    pub async fn run_with_ports(
        &self,
        submission: TaskSubmission,
        task_id: Uuid,
        resume: Option<Uuid>,
        executor: Arc<dyn StrategyExecutor>,
        effects: Option<Arc<dyn super::ports::StrategyEffects>>,
        advisor: Arc<dyn ConvergenceAdvisor>,
        prompt_builder: Option<Arc<dyn PromptBuilder>>,
        event_sink: Option<Arc<dyn super::ports::ConvergenceEventSink>>,
    ) -> DomainResult<ConvergenceRunOutcome> {
        self.run_inner(
            submission,
            task_id,
            resume,
            executor,
            effects,
            advisor,
            prompt_builder,
            event_sink,
        )
        .await
    }

    /// Run a full convergence from submission to terminal outcome, driving the
    /// inner loop through the installed [`StrategyExecutor`],
    /// [`StrategyEffects`](super::ports::StrategyEffects), and
    /// [`ConvergenceAdvisor`] ports.
    ///
    /// This is the engine-as-core entrypoint scheduled to replace the
    /// orchestrator's `run_convergent_execution_inner` outer loop. See the
    /// module doc for Phase A / Phase B milestones.
    ///
    /// When `resume` is `Some`, the trajectory with that id is loaded from the
    /// trajectory store and the engine continues from its current state
    /// (bandit re-initialized from memory). When `None`, the engine runs
    /// [`ConvergenceEngine::prepare`] to create a fresh trajectory.
    pub async fn run(
        &self,
        submission: TaskSubmission,
        task_id: Uuid,
        resume: Option<Uuid>,
    ) -> DomainResult<ConvergenceRunOutcome> {
        // ------------------------------------------------------------------
        // Port availability checks.
        // ------------------------------------------------------------------
        let executor: Arc<dyn StrategyExecutor> = self.executor.clone().ok_or_else(|| {
            DomainError::ExecutionFailed(
                "ConvergenceEngine::run requires a StrategyExecutor (install with with_executor)"
                    .to_string(),
            )
        })?;
        let advisor: Arc<dyn ConvergenceAdvisor> = self.advisor.clone().ok_or_else(|| {
            DomainError::ExecutionFailed(
                "ConvergenceEngine::run requires a ConvergenceAdvisor (install with with_advisor)"
                    .to_string(),
            )
        })?;
        let effects = self.effects.clone();
        let prompt_builder: Option<Arc<dyn PromptBuilder>> = self.prompt_builder.clone();
        self.run_inner(
            submission,
            task_id,
            resume,
            executor,
            effects,
            advisor,
            prompt_builder,
            None,
        )
        .await
    }

    /// Shared implementation used by [`Self::run`] and [`Self::run_with_ports`].
    #[allow(clippy::too_many_arguments)]
    async fn run_inner(
        &self,
        submission: TaskSubmission,
        task_id: Uuid,
        resume: Option<Uuid>,
        executor: Arc<dyn StrategyExecutor>,
        effects: Option<Arc<dyn super::ports::StrategyEffects>>,
        advisor: Arc<dyn ConvergenceAdvisor>,
        prompt_builder: Option<Arc<dyn PromptBuilder>>,
        event_sink_override: Option<Arc<dyn super::ports::ConvergenceEventSink>>,
    ) -> DomainResult<ConvergenceRunOutcome> {
        // When a per-call sink is supplied, route events through it; otherwise
        // use the engine's installed sink (default TracingEventSink).
        let event_sink: Arc<dyn super::ports::ConvergenceEventSink> =
            event_sink_override.unwrap_or_else(|| self.event_sink.clone());

        // ------------------------------------------------------------------
        // PREPARE or RESUME.
        // ------------------------------------------------------------------
        let (mut trajectory, mut bandit) = if let Some(tid) = resume {
            let loaded = self
                .trajectory_store
                .get(&tid.to_string())
                .await?
                .ok_or_else(|| {
                    DomainError::ExecutionFailed(format!(
                        "trajectory {} referenced by resume request not found",
                        tid
                    ))
                })?;
            let bandit = self.initialize_bandit(&loaded).await;
            (loaded, bandit)
        } else {
            let (traj, _infra) = self.prepare(&submission, task_id).await?;
            let bandit = self.initialize_bandit(&traj).await;
            (traj, bandit)
        };

        trajectory.phase = ConvergencePhase::Iterating;

        // Track attractor classification across iterations so we can emit
        // AttractorTransitionChanged on real transitions.
        let mut prev_attractor = trajectory.attractor_state.classification.clone();

        // ------------------------------------------------------------------
        // Main loop.
        // ------------------------------------------------------------------
        loop {
            // 1. Advisor's top-of-iteration gate.
            match advisor.on_iteration_start(&mut trajectory).await? {
                IterationGate::Continue => {}
                IterationGate::Cancel => {
                    self.trajectory_store.save(&trajectory).await?;
                    return Ok(ConvergenceRunOutcome::Cancelled);
                }
                IterationGate::AdjustPolicy(overlay) => {
                    apply_policy_overlay(&mut trajectory, &overlay);
                }
            }

            // 2. Select strategy (forced or bandit-selected). Mirrors
            // converge() / inner-loop logic.
            let strategy = if let Some(forced) = trajectory.forced_strategy.take() {
                forced
            } else {
                let eligible = eligible_strategies(
                    &trajectory.strategy_log,
                    &trajectory.attractor_state,
                    &trajectory.budget,
                    trajectory.total_fresh_starts,
                    trajectory.policy.max_fresh_starts,
                );
                if eligible.is_empty() {
                    let outcome = ConvergenceOutcome::Trapped {
                        trajectory_id: trajectory.id.to_string(),
                        attractor_type: trajectory.attractor_state.classification.clone(),
                    };
                    self.finalize(&mut trajectory, &outcome, &bandit).await?;
                    return Ok(ConvergenceRunOutcome::Failed(format!(
                        "trapped in {:?} attractor -- no eligible escape strategies",
                        trajectory.attractor_state.classification
                    )));
                }
                bandit.select(
                    &trajectory.attractor_state.classification,
                    &eligible,
                    &trajectory.policy,
                )
            };

            // 3. Strategy side effects via StrategyEffects.
            if matches!(strategy, StrategyKind::FreshStart { .. }) {
                trajectory.total_fresh_starts += 1;
                if let Some(ref fx) = effects {
                    fx.on_fresh_start(&trajectory).await?;
                }
            } else if let StrategyKind::RevertAndBranch { target } = &strategy
                && let Some(ref fx) = effects
            {
                fx.on_revert(&trajectory, target).await?;
            }

            // 4. Build prompt. Prefer the installed PromptBuilder port if
            // present (orchestrator path); otherwise fall back to a minimal
            // default for engine-only tests.
            let iteration_seq = trajectory.observations.len() as u32;
            let prompt = if let Some(ref pb) = prompt_builder {
                pb.build(&trajectory, &strategy, iteration_seq).await?
            } else {
                default_prompt(&trajectory, &strategy)
            };

            // 5. Execute strategy via StrategyExecutor.
            let strategy_context = self.build_strategy_context(&strategy, &trajectory);
            let exec_ctx = StrategyExecutionContext {
                trajectory: &trajectory,
                strategy: &strategy,
                strategy_context: &strategy_context,
                iteration_seq,
                prompt: &prompt,
            };
            let StrategyExecutionOutput {
                artifact,
                tokens_used,
                wall_time_ms,
            } = executor.execute(&exec_ctx).await?;

            // 6. Measure with overseers.
            let overseer_signals = self
                .measure(&artifact, &trajectory.policy)
                .await
                .unwrap_or_else(|e| {
                    tracing::warn!(
                        trajectory_id = %trajectory.id,
                        error = %e,
                        "Overseer measurement failed; using empty signals"
                    );
                    OverseerSignals::default()
                });

            if trajectory.observations.is_empty() && trajectory.lint_baseline == 0 {
                trajectory.lint_baseline = overseer_signals
                    .lint_results
                    .as_ref()
                    .map(|l| l.error_count)
                    .unwrap_or(0);
            }

            let sequence = trajectory.observations.len() as u32;
            let observation = Observation::new(
                sequence,
                artifact,
                overseer_signals,
                strategy.clone(),
                tokens_used,
                wall_time_ms,
            );

            // 7. Core iteration.
            let control = self
                .iterate_once(&mut trajectory, &mut bandit, &strategy, observation)
                .await?;

            // 7b. Emit AttractorTransitionChanged if classification changed.
            let current_attractor = trajectory.attractor_state.classification.clone();
            if self.attractor_type_name(&current_attractor)
                != self.attractor_type_name(&prev_attractor)
            {
                event_sink
                    .emit(ConvergenceDomainEvent::AttractorTransitionChanged {
                        trajectory_id: trajectory.id,
                        from: self.attractor_type_name(&prev_attractor).to_string(),
                        to: self.attractor_type_name(&current_attractor).to_string(),
                        confidence: trajectory.attractor_state.confidence,
                    })
                    .await;
                prev_attractor = current_attractor.clone();
            }

            // 7c. Emit IterationCompleted with post-iteration metrics.
            let (delta, level) = trajectory
                .observations
                .last()
                .and_then(|o| o.metrics.as_ref())
                .map(|m| (m.convergence_delta, m.convergence_level))
                .unwrap_or((0.0, 0.0));
            event_sink
                .emit(ConvergenceDomainEvent::IterationCompleted {
                    trajectory_id: trajectory.id,
                    iteration: sequence,
                    strategy: strategy.kind_name().to_string(),
                    convergence_delta: delta,
                    convergence_level: level,
                    attractor_type: self
                        .attractor_type_name(&trajectory.attractor_state.classification)
                        .to_string(),
                    budget_remaining_fraction: trajectory.budget.remaining_fraction(),
                })
                .await;

            // 8. Route loop control through the advisor.
            match control {
                LoopControl::Continue => continue,
                LoopControl::IntentCheck => {
                    let directive = advisor.on_intent_check(&mut trajectory, sequence).await?;
                    if let Some(outcome) =
                        self.apply_directive(directive, &mut trajectory, &bandit).await?
                    {
                        return Ok(outcome);
                    }
                    continue;
                }
                LoopControl::Exhausted => {
                    let directive = advisor.on_pre_exhaustion(&trajectory).await?;
                    if let Some(outcome) =
                        self.apply_directive(directive, &mut trajectory, &bandit).await?
                    {
                        return Ok(outcome);
                    }
                    // Advisor said Continue from Exhausted — try an extension
                    // before falling back to actual exhaustion to match the
                    // legacy converge() behaviour.
                    if self.request_extension(&mut trajectory).await? {
                        continue;
                    }
                    let outcome = ConvergenceOutcome::Exhausted {
                        trajectory_id: trajectory.id.to_string(),
                        best_observation_sequence: trajectory
                            .best_observation()
                            .map(|o| o.sequence),
                    };
                    self.finalize(&mut trajectory, &outcome, &bandit).await?;
                    return Ok(ConvergenceRunOutcome::Exhausted(
                        "convergence budget exhausted".to_string(),
                    ));
                }
                LoopControl::Trapped => {
                    let outcome = ConvergenceOutcome::Trapped {
                        trajectory_id: trajectory.id.to_string(),
                        attractor_type: trajectory.attractor_state.classification.clone(),
                    };
                    self.finalize(&mut trajectory, &outcome, &bandit).await?;
                    return Ok(ConvergenceRunOutcome::Failed(format!(
                        "trapped in {:?} attractor",
                        trajectory.attractor_state.classification
                    )));
                }
                LoopControl::Decompose => {
                    return Ok(ConvergenceRunOutcome::Decomposed(trajectory));
                }
                LoopControl::OverseerConverged => {
                    let directive = advisor.on_overseer_converged(&trajectory).await?;
                    if let Some(outcome) =
                        self.apply_directive(directive, &mut trajectory, &bandit).await?
                    {
                        return Ok(outcome);
                    }
                    continue;
                }
                LoopControl::RequestExtension => {
                    if self.request_extension(&mut trajectory).await? {
                        continue;
                    }
                    let outcome = ConvergenceOutcome::BudgetDenied {
                        trajectory_id: trajectory.id.to_string(),
                    };
                    self.finalize(&mut trajectory, &outcome, &bandit).await?;
                    return Ok(ConvergenceRunOutcome::Failed(
                        "budget extension denied".to_string(),
                    ));
                }
            }
        }
    }

    /// Translate an [`AdvisorDirective`] into a terminal
    /// [`ConvergenceRunOutcome`] (finalizing the trajectory as needed) or
    /// `None` to continue iterating.
    async fn apply_directive(
        &self,
        directive: AdvisorDirective,
        trajectory: &mut Trajectory,
        bandit: &StrategyBandit,
    ) -> DomainResult<Option<ConvergenceRunOutcome>> {
        Ok(match directive {
            AdvisorDirective::FinalizeConverged => {
                let final_seq = trajectory
                    .observations
                    .last()
                    .map(|o| o.sequence)
                    .unwrap_or(0);
                let outcome = ConvergenceOutcome::Converged {
                    trajectory_id: trajectory.id.to_string(),
                    final_observation_sequence: final_seq,
                };
                self.finalize(trajectory, &outcome, bandit).await?;
                Some(ConvergenceRunOutcome::Converged)
            }
            AdvisorDirective::FinalizeExhausted(reason) => {
                let best_seq = trajectory.best_observation().map(|o| o.sequence);
                let outcome = ConvergenceOutcome::Exhausted {
                    trajectory_id: trajectory.id.to_string(),
                    best_observation_sequence: best_seq,
                };
                self.finalize(trajectory, &outcome, bandit).await?;
                Some(ConvergenceRunOutcome::Exhausted(reason))
            }
            AdvisorDirective::FinalizeIntentGaps(ivr) => {
                let final_seq = trajectory
                    .observations
                    .last()
                    .map(|o| o.sequence)
                    .unwrap_or(0);
                let outcome = ConvergenceOutcome::Exhausted {
                    trajectory_id: trajectory.id.to_string(),
                    best_observation_sequence: Some(final_seq),
                };
                self.finalize(trajectory, &outcome, bandit).await?;
                Some(ConvergenceRunOutcome::IntentGapsFound(ivr))
            }
            AdvisorDirective::FinalizePartialAccepted => {
                let final_seq = trajectory
                    .observations
                    .last()
                    .map(|o| o.sequence)
                    .unwrap_or(0);
                let outcome = ConvergenceOutcome::Converged {
                    trajectory_id: trajectory.id.to_string(),
                    final_observation_sequence: final_seq,
                };
                self.finalize(trajectory, &outcome, bandit).await?;
                Some(ConvergenceRunOutcome::PartialAccepted)
            }
            AdvisorDirective::FinalizeIndeterminateAccepted => {
                let final_seq = trajectory
                    .observations
                    .last()
                    .map(|o| o.sequence)
                    .unwrap_or(0);
                let outcome = ConvergenceOutcome::Converged {
                    trajectory_id: trajectory.id.to_string(),
                    final_observation_sequence: final_seq,
                };
                self.finalize(trajectory, &outcome, bandit).await?;
                Some(ConvergenceRunOutcome::IndeterminateAccepted)
            }
            AdvisorDirective::FinalizeCancelled => {
                self.trajectory_store.save(trajectory).await?;
                Some(ConvergenceRunOutcome::Cancelled)
            }
            AdvisorDirective::Continue { policy_overlay } => {
                if let Some(overlay) = policy_overlay {
                    apply_policy_overlay(trajectory, &overlay);
                }
                None
            }
        })
    }
}

/// Apply a [`PolicyOverlay`] to the trajectory's in-flight policy / budget.
///
/// Kept as a free function so it can be unit-tested without constructing a
/// full engine. Today only `max_iterations_delta` is load-bearing;
/// `budget_pressure` is advisory and has no engine-side effect yet.
fn apply_policy_overlay(trajectory: &mut Trajectory, overlay: &PolicyOverlay) {
    if let Some(delta) = overlay.max_iterations_delta {
        let current = trajectory.budget.max_iterations as i64;
        let adjusted = (current + delta as i64).max(0) as u32;
        trajectory.budget.max_iterations = adjusted;
    }
    // `budget_pressure` is advisory; the engine defers to its own
    // BudgetTracker for termination decisions.
    let _ = overlay.budget_pressure;
}

/// Minimal prompt builder used when no richer prompt is provided by the
/// caller. The orchestrator's production path supplies its own prompt via the
/// executor; engine-owned tests / harnesses that don't care about prompt
/// quality can rely on this default.
fn default_prompt(trajectory: &Trajectory, strategy: &StrategyKind) -> String {
    format!(
        "Task: {}\nStrategy: {}\nIteration: {}",
        trajectory.specification.effective.content,
        strategy.kind_name(),
        trajectory.observations.len()
    )
}

#[cfg(test)]
mod tests {
    use super::super::test_support::build_test_engine;
    use super::super::tests::test_trajectory;
    use super::*;
    use crate::domain::models::task::Complexity;

    #[test]
    fn apply_policy_overlay_extends_iterations() {
        let mut t = test_trajectory();
        t.budget.max_iterations = 5;
        apply_policy_overlay(
            &mut t,
            &PolicyOverlay {
                max_iterations_delta: Some(3),
                budget_pressure: None,
            },
        );
        assert_eq!(t.budget.max_iterations, 8);
    }

    #[test]
    fn apply_policy_overlay_saturates_at_zero() {
        let mut t = test_trajectory();
        t.budget.max_iterations = 2;
        apply_policy_overlay(
            &mut t,
            &PolicyOverlay {
                max_iterations_delta: Some(-10),
                budget_pressure: None,
            },
        );
        assert_eq!(t.budget.max_iterations, 0);
    }

    #[tokio::test]
    async fn run_errors_without_executor() {
        let engine = build_test_engine();
        let submission = TaskSubmission {
            description: "test".to_string(),
            goal_id: None,
            inferred_complexity: Complexity::Moderate,
            discovered_infrastructure: DiscoveredInfrastructure::default(),
            priority_hint: None,
            constraints: vec![],
            references: vec![],
            anti_patterns: vec![],
            parallel_samples: None,
        };
        let res = engine.run(submission, Uuid::new_v4(), None).await;
        let err = res.unwrap_err();
        assert!(format!("{err}").contains("StrategyExecutor"));
    }
}
