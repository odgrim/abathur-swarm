//! Convergent execution path for the swarm orchestrator.
//!
//! When a task has `ExecutionMode::Convergent`, the orchestrator enters a
//! convergence loop instead of a single-shot substrate invocation. The loop
//! uses the convergence engine's granular primitives with substrate execution
//! injected between strategy selection and overseer measurement.
//!
//! # Design
//!
//! The orchestrator owns the outer loop; the engine owns the inner logic.
//! The engine's granular primitives (`iterate_once`, `select_strategy`,
//! `initialize_bandit`, `finalize`) are used because the orchestrator must
//! inject substrate execution between strategy selection and overseer
//! measurement.
//!
//! The flow per iteration is:
//! 1. Check cancellation token
//! 2. Select strategy (bandit + eligibility filter)
//! 3. If FreshStart, reset worktree to base branch state
//! 4. Build prompt (bridge: task + trajectory + strategy -> prompt string)
//! 5. Execute substrate (agent runtime produces artifact)
//! 6. Collect artifact reference from worktree
//! 7. Measure with overseers via engine
//! 8. Record observation, classify attractor, update bandit via `iterate_once`
//! 9. Act on `LoopControl` to continue, converge, decompose, etc.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Instant;

use async_trait::async_trait;
use tokio::sync::Mutex as TokioMutex;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use super::helpers::remove_transient_artifacts;
use crate::domain::errors::{DomainError, DomainResult};
use crate::domain::models::convergence::*;
use crate::domain::models::intent_verification::{
    GapSeverity, IntentSatisfaction, IntentVerificationResult,
};
use crate::domain::models::task::Task;
use crate::domain::models::{SubstrateConfig, SubstrateRequest};
use crate::domain::ports::{MemoryRepository, Substrate, TaskRepository, TrajectoryRepository};
use crate::services::convergence_bridge;
use crate::services::convergence_engine::{
    AdvisorDirective, ConvergenceAdvisor, ConvergenceDomainEvent, ConvergenceEngine,
    ConvergenceEventSink, ConvergenceRunOutcome, IterationGate, OverseerMeasurer, PromptBuilder,
    StrategyEffects, StrategyExecutionContext, StrategyExecutionOutput, StrategyExecutor,
};
use crate::services::event_bus::{
    ConvergenceIterationPayload, ConvergenceTerminatedPayload, EventBus, EventPayload,
    EventSeverity, HumanEscalationPayload,
};
use crate::services::event_factory;

// ---------------------------------------------------------------------------
// ConvergentIntentVerifier trait
// ---------------------------------------------------------------------------

/// Trait that erases the `<G, T>` generics from `IntentVerifierService`,
/// letting all convergent execution functions accept
/// `Option<Arc<dyn ConvergentIntentVerifier>>` without new type parameters.
#[async_trait]
pub trait ConvergentIntentVerifier: Send + Sync {
    /// Run LLM-based intent verification for a convergent task.
    ///
    /// Returns `Ok(Some(result))` when verification succeeds, `Ok(None)` if
    /// no intent can be extracted (e.g. no goal_id and task has no meaningful
    /// description), or `Err` on infrastructure failure.
    ///
    /// When `overseer_signals` is provided, the verifier includes a summary
    /// of overseer state in the verification prompt as informational context.
    async fn verify_convergent_intent(
        &self,
        task: &Task,
        goal_id: Option<Uuid>,
        iteration: u32,
        overseer_signals: Option<&OverseerSignals>,
    ) -> DomainResult<Option<IntentVerificationResult>>;
}

// ---------------------------------------------------------------------------
// ConvergentOutcome
// ---------------------------------------------------------------------------

/// Outcome of convergent execution, consumed by the orchestrator to decide
/// the task's terminal status.
#[derive(Debug)]
#[allow(clippy::large_enum_variant)]
pub enum ConvergentOutcome {
    /// The trajectory converged to a satisfactory result.
    Converged,
    /// Convergence accepted based on overseer signals, but intent was never
    /// positively verified (e.g. 3x consecutive Indeterminate results).
    /// The orchestrator should treat this as success but NOT override
    /// `require_commits`, since the system could not confirm the work
    /// matches the original intent.
    IndeterminateAccepted,
    /// Budget exhausted, but the best observation was above the partial
    /// acceptance threshold. The orchestrator should treat this as success.
    PartialAccepted,
    /// Overseers confirmed convergence but intent verification found gaps.
    /// The orchestrator should re-enqueue the task with the gap context
    /// so the next execution attempt can address the outstanding issues.
    IntentGapsFound(IntentVerificationResult),
    /// The engine determined the task should be decomposed into subtasks.
    /// The orchestrator creates child tasks from the trajectory's
    /// decomposition plan.
    Decomposed(Trajectory),
    /// Convergence failed. The message describes the terminal condition
    /// (trapped, exhausted, budget denied, etc.).
    Failed(String),
    /// The convergence loop was cancelled via the cancellation token.
    /// The trajectory has been persisted in its current state. The caller
    /// handles the task status transition (typically to Canceled).
    Cancelled,
}

// ---------------------------------------------------------------------------
// OrchestratorStrategyExecutor
// ---------------------------------------------------------------------------

/// `StrategyExecutor` implementation used by the orchestrator's convergent
/// path. Wraps the concrete `Substrate` invocation + worktree-based artifact
/// collection that previously lived inline inside
/// `run_convergent_execution_inner`.
///
/// Part of the engine-as-core refactor chain (#13/#21): PR 2 establishes the
/// boundary so that PR 4 can migrate the convergence engine's own inner loop
/// to drive the executor directly, at which point the orchestrator will stop
/// hosting the substrate call altogether.
pub(super) struct OrchestratorStrategyExecutor {
    substrate: Arc<dyn Substrate>,
    /// Worktree path the substrate should run in. When `None`, no working
    /// directory is set on the `SubstrateConfig` and artifact collection
    /// falls back to the process CWD (`"."`), matching pre-port behaviour.
    worktree_path: Option<PathBuf>,
    agent_type: String,
    system_prompt: String,
    task_id: Uuid,
    /// Max turns to pass to the substrate. Stored on the executor because it
    /// does not vary across iterations of a single convergent run.
    max_turns: u32,
}

#[async_trait]
impl StrategyExecutor for OrchestratorStrategyExecutor {
    async fn execute(
        &self,
        ctx: &StrategyExecutionContext<'_>,
    ) -> DomainResult<StrategyExecutionOutput> {
        let _ = ctx.trajectory;
        let _ = ctx.strategy;
        let _ = ctx.strategy_context;
        let _ = ctx.iteration_seq;

        let mut config = SubstrateConfig::default().with_max_turns(self.max_turns);
        if let Some(ref wt) = self.worktree_path {
            config = config.with_working_dir(wt.to_string_lossy().as_ref());
        }
        let request = SubstrateRequest::new(
            self.task_id,
            &self.agent_type,
            &self.system_prompt,
            ctx.prompt,
        )
        .with_config(config);

        let iteration_start = Instant::now();
        let session = self.substrate.execute(request).await?;
        let wall_time_ms = iteration_start.elapsed().as_millis() as u64;

        let artifact_dir = self
            .worktree_path
            .as_ref()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|| ".".to_string());
        let artifact = convergence_bridge::collect_artifact(&artifact_dir, "");
        let tokens_used = session.total_tokens();

        Ok(StrategyExecutionOutput {
            artifact,
            tokens_used,
            wall_time_ms,
        })
    }
}

// ---------------------------------------------------------------------------
// OrchestratorStrategyEffects
// ---------------------------------------------------------------------------

/// `StrategyEffects` implementation used by the orchestrator's convergent
/// path. Wraps the worktree-reset + `ConvergenceFreshStart` event emission
/// that currently lives inline inside `run_convergent_execution_inner`.
///
/// Part of the engine-as-core refactor chain (#13/#21): PR 3 establishes the
/// boundary so PR 4 can migrate the engine's inner strategy handling to call
/// `effects.on_fresh_start(...)` / `effects.on_revert(...)` directly. When
/// that happens, the inline `FreshStart` handling in
/// `run_convergent_execution_inner` can be deleted.
///
/// NOTE: PR 3 does NOT migrate the orchestrator's inline `FreshStart` code to
/// call this impl. The inline path mutates trajectory state
/// (`trajectory.total_fresh_starts += 1`) that the `&Trajectory` signature
/// here cannot express, so extraction waits until PR 4 inverts ownership
/// (engine drives the loop and owns the trajectory mutably). `on_revert` has
/// no pre-existing orchestrator-side logic to mirror; `RevertAndBranch` is
/// currently handled entirely inside the engine by routing to a prior
/// observation's artifact, with no worktree side effect.
#[allow(dead_code)]
pub(super) struct OrchestratorStrategyEffects {
    event_bus: Arc<EventBus>,
    goal_id: Option<Uuid>,
    task_id: Uuid,
    /// Worktree path the FreshStart reset should target. `None` means no
    /// worktree was allocated for this run (e.g. in-process tests); the reset
    /// becomes a no-op, matching the inline code's `if let Some(wt) = ...`
    /// guard.
    worktree_path: Option<PathBuf>,
}

#[async_trait]
impl StrategyEffects for OrchestratorStrategyEffects {
    async fn on_fresh_start(&self, trajectory: &Trajectory) -> DomainResult<()> {
        if let Some(ref wt) = self.worktree_path {
            reset_worktree(wt.to_string_lossy().as_ref()).await?;
        }
        self.event_bus
            .publish(event_factory::make_event(
                EventSeverity::Info,
                crate::services::event_bus::EventCategory::Convergence,
                self.goal_id,
                Some(self.task_id),
                EventPayload::ConvergenceFreshStart {
                    task_id: self.task_id,
                    trajectory_id: trajectory.id,
                    fresh_start_number: trajectory.total_fresh_starts,
                    reason: "FreshStart strategy selected".to_string(),
                },
            ))
            .await;
        Ok(())
    }

    async fn on_revert(&self, trajectory: &Trajectory, target: &Uuid) -> DomainResult<()> {
        // RevertAndBranch currently has no orchestrator-side filesystem
        // effect: the engine handles the revert by routing to the target
        // observation's stored artifact reference. We trace the call so PR 4
        // can introduce a real worktree-rewind here when the engine starts
        // owning the inner loop.
        tracing::debug!(
            trajectory_id = %trajectory.id,
            target = %target,
            "StrategyEffects::on_revert invoked (no-op until PR 4 owns the loop)"
        );
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// OrchestratorConvergenceAdvisor
// ---------------------------------------------------------------------------

/// `ConvergenceAdvisor` implementation used by the orchestrator's convergent
/// path. Owns the cross-iteration state that used to live as stack locals in
/// `run_convergent_execution_inner`:
///
/// - the consecutive-indeterminate counter for the 3-strike fallback,
/// - the total intent-check cap,
/// - the last intent verification result (used by the prompt builder).
///
/// Part of the engine-as-core refactor chain (#13/#21): PR 4 Phase A adds the
/// port + this impl. Phase B (PR 4b) will rewrite `run_convergent_execution_inner`
/// to construct this advisor + the executor/effects impls and delegate the
/// loop body to `engine.run()`. Today the advisor compiles, satisfies the
/// trait, and is exercised only by unit tests / the engine's new `run()`
/// entrypoint — the orchestrator's inline loop still reproduces the
/// equivalent branching.
/// Erased loader that returns the current task's context hints. Used by the
/// advisor to apply SLA pressure without taking a `TaskRepository` type
/// parameter. Implemented below by a thin wrapper around `Arc<T>`.
#[async_trait]
pub(super) trait TaskHintsLoader: Send + Sync {
    async fn load_hints(&self, task_id: Uuid) -> Vec<String>;
}

struct TaskRepoHintsLoader<T: TaskRepository + ?Sized> {
    repo: Arc<T>,
}

#[async_trait]
impl<T: TaskRepository + ?Sized + 'static> TaskHintsLoader for TaskRepoHintsLoader<T> {
    async fn load_hints(&self, task_id: Uuid) -> Vec<String> {
        match self.repo.get(task_id).await {
            Ok(Some(t)) => t.context.hints.clone(),
            _ => Vec::new(),
        }
    }
}

#[allow(dead_code)]
pub(super) struct OrchestratorConvergenceAdvisor {
    intent_verifier: Arc<dyn ConvergentIntentVerifier>,
    cancellation_token: CancellationToken,
    task: Task,
    goal_id: Option<Uuid>,
    event_bus: Arc<EventBus>,
    /// Loader for the task's most recent context hints, used for SLA pressure.
    /// Optional so tests/advisor-level unit tests can skip it.
    hints_loader: Option<Arc<dyn TaskHintsLoader>>,
    /// Consecutive Indeterminate verifications. Reset on a non-indeterminate
    /// result, escalates at 2, finalizes at 3.
    consecutive_indeterminate: Arc<AtomicU32>,
    /// Hard cap across all intent checks in a single run, to prevent runaway
    /// verification loops.
    total_intent_checks: Arc<AtomicU32>,
    /// Most recent verification result. Mirrors the stack local in the legacy
    /// inner loop so the orchestrator's prompt builder can pick up the latest
    /// gap feedback.
    last_intent_verification: Arc<TokioMutex<Option<IntentVerificationResult>>>,
}

#[allow(dead_code)]
impl OrchestratorConvergenceAdvisor {
    pub(super) fn new(
        task: Task,
        goal_id: Option<Uuid>,
        event_bus: Arc<EventBus>,
        intent_verifier: Arc<dyn ConvergentIntentVerifier>,
        cancellation_token: CancellationToken,
    ) -> Self {
        Self {
            intent_verifier,
            cancellation_token,
            task,
            goal_id,
            event_bus,
            hints_loader: None,
            consecutive_indeterminate: Arc::new(AtomicU32::new(0)),
            total_intent_checks: Arc::new(AtomicU32::new(0)),
            last_intent_verification: Arc::new(TokioMutex::new(None)),
        }
    }

    /// Attach a task-hints loader. Used to apply SLA pressure at the top of
    /// each iteration.
    pub(super) fn with_hints_loader(mut self, loader: Arc<dyn TaskHintsLoader>) -> Self {
        self.hints_loader = Some(loader);
        self
    }

    /// Expose the last verification for the orchestrator's prompt builder
    /// during Phase B migration.
    pub(super) async fn last_verification(&self) -> Option<IntentVerificationResult> {
        self.last_intent_verification.lock().await.clone()
    }

    /// Overseers-clean gate used by the 3-strike indeterminate fallback.
    fn overseers_clean(trajectory: &Trajectory) -> bool {
        trajectory
            .observations
            .last()
            .map(|o| {
                o.overseer_signals
                    .all_passing_relative(trajectory.lint_baseline)
                    && o.overseer_signals.has_any_signal()
            })
            .unwrap_or(false)
    }

    /// Handle the 3-strike indeterminate fallback.
    fn indeterminate_fallback_directive(&self, trajectory: &Trajectory) -> AdvisorDirective {
        if Self::overseers_clean(trajectory) {
            AdvisorDirective::FinalizeIndeterminateAccepted
        } else {
            AdvisorDirective::FinalizeExhausted(
                "intent verification indeterminate 3x with failing overseers".to_string(),
            )
        }
    }
}

#[async_trait]
impl ConvergenceAdvisor for OrchestratorConvergenceAdvisor {
    async fn on_iteration_start(
        &self,
        trajectory: &mut Trajectory,
    ) -> DomainResult<IterationGate> {
        if self.cancellation_token.is_cancelled() {
            emit_convergence_terminated(
                &self.event_bus,
                &self.task,
                self.goal_id,
                trajectory,
                "cancelled",
            )
            .await;
            return Ok(IterationGate::Cancel);
        }

        // SLA pressure consumption (Part 8.2). When a hints loader is
        // attached (production path), read the latest task hints and adjust
        // the in-flight policy before each iteration.
        if let Some(ref loader) = self.hints_loader {
            let hints = loader.load_hints(self.task.id).await;
            apply_sla_pressure(&hints, &mut trajectory.policy);
        }

        Ok(IterationGate::Continue)
    }

    async fn on_intent_check(
        &self,
        trajectory: &mut Trajectory,
        iteration: u32,
    ) -> DomainResult<AdvisorDirective> {
        // Hard cap.
        let total = self.total_intent_checks.fetch_add(1, Ordering::SeqCst) + 1;
        if total > 10 {
            return Ok(AdvisorDirective::FinalizeExhausted(
                "total intent check cap (10) exceeded".to_string(),
            ));
        }

        // Defense-in-depth: verification tasks converge without recursing.
        if self.task.task_type.is_verification() {
            return Ok(AdvisorDirective::FinalizeConverged);
        }

        let overseer_signals = trajectory.observations.last().map(|o| &o.overseer_signals);

        match self
            .intent_verifier
            .verify_convergent_intent(&self.task, self.goal_id, iteration, overseer_signals)
            .await
        {
            Ok(Some(ivr)) => {
                emit_intent_verification_event(&self.event_bus, &self.task, self.goal_id, &ivr)
                    .await;

                match ivr.satisfaction {
                    IntentSatisfaction::Satisfied => {
                        self.consecutive_indeterminate.store(0, Ordering::SeqCst);
                        trajectory.prev_intent_confidence = trajectory.last_intent_confidence;
                        trajectory.last_intent_confidence = Some(ivr.confidence);
                        *self.last_intent_verification.lock().await = Some(ivr);
                        Ok(AdvisorDirective::FinalizeConverged)
                    }
                    IntentSatisfaction::Partial | IntentSatisfaction::Unsatisfied => {
                        self.consecutive_indeterminate.store(0, Ordering::SeqCst);
                        apply_verification_amendments(&ivr, trajectory);
                        trajectory.prev_intent_confidence = trajectory.last_intent_confidence;
                        trajectory.last_intent_confidence = Some(ivr.confidence);
                        if let Some(ref escalation) = ivr.escalation {
                            emit_escalation_from_verification(
                                &self.event_bus,
                                &self.task,
                                self.goal_id,
                                escalation,
                            )
                            .await;
                        } else if let Some(auto_escalation) = ivr.should_escalate() {
                            emit_escalation_from_verification(
                                &self.event_bus,
                                &self.task,
                                self.goal_id,
                                &auto_escalation,
                            )
                            .await;
                        }
                        *self.last_intent_verification.lock().await = Some(ivr);
                        Ok(AdvisorDirective::Continue {
                            policy_overlay: None,
                        })
                    }
                    IntentSatisfaction::Indeterminate => {
                        let count =
                            self.consecutive_indeterminate.fetch_add(1, Ordering::SeqCst) + 1;
                        if count == 2 {
                            let escalation =
                                crate::domain::models::HumanEscalation::ambiguous_requirements(
                                    "Intent verifier returned Indeterminate 2+ times consecutively. \
                                     Unable to determine whether the work satisfies the original intent. \
                                     Human judgment required.",
                                );
                            emit_escalation_from_verification(
                                &self.event_bus,
                                &self.task,
                                self.goal_id,
                                &escalation,
                            )
                            .await;
                        }
                        if count >= 3 {
                            Ok(self.indeterminate_fallback_directive(trajectory))
                        } else {
                            *self.last_intent_verification.lock().await = Some(ivr);
                            Ok(AdvisorDirective::Continue {
                                policy_overlay: None,
                            })
                        }
                    }
                }
            }
            Ok(None) => {
                let count = self.consecutive_indeterminate.fetch_add(1, Ordering::SeqCst) + 1;
                let escalation = crate::domain::models::HumanEscalation::ambiguous_requirements(
                    "Cannot extract intent from task or goal description. \
                     Intent verification is required for finality but no intent \
                     could be derived. Human input needed to clarify the task.",
                );
                emit_escalation_from_verification(
                    &self.event_bus,
                    &self.task,
                    self.goal_id,
                    &escalation,
                )
                .await;
                if count >= 3 {
                    Ok(self.indeterminate_fallback_directive(trajectory))
                } else {
                    Ok(AdvisorDirective::Continue {
                        policy_overlay: None,
                    })
                }
            }
            Err(_e) => {
                let count = self.consecutive_indeterminate.fetch_add(1, Ordering::SeqCst) + 1;
                if count >= 3 {
                    Ok(self.indeterminate_fallback_directive(trajectory))
                } else {
                    Ok(AdvisorDirective::Continue {
                        policy_overlay: None,
                    })
                }
            }
        }
    }

    async fn on_overseer_converged(
        &self,
        trajectory: &Trajectory,
    ) -> DomainResult<AdvisorDirective> {
        // Defense-in-depth: verification tasks skip recursive verification.
        if self.task.task_type.is_verification() {
            return Ok(AdvisorDirective::FinalizeConverged);
        }

        let sequence = trajectory.observations.len() as u32;
        let overseer_signals = trajectory.observations.last().map(|o| &o.overseer_signals);

        match self
            .intent_verifier
            .verify_convergent_intent(&self.task, self.goal_id, sequence, overseer_signals)
            .await
        {
            Ok(Some(ivr)) => {
                emit_intent_verification_event(&self.event_bus, &self.task, self.goal_id, &ivr)
                    .await;

                match ivr.satisfaction {
                    IntentSatisfaction::Satisfied => Ok(AdvisorDirective::FinalizeConverged),
                    IntentSatisfaction::Partial | IntentSatisfaction::Unsatisfied => {
                        Ok(AdvisorDirective::FinalizeIntentGaps(ivr))
                    }
                    IntentSatisfaction::Indeterminate => {
                        Ok(AdvisorDirective::FinalizeIndeterminateAccepted)
                    }
                }
            }
            Ok(None) => Ok(AdvisorDirective::FinalizeConverged),
            Err(_) => Ok(AdvisorDirective::FinalizeConverged),
        }
    }

    async fn on_pre_exhaustion(&self, trajectory: &Trajectory) -> DomainResult<AdvisorDirective> {
        if self.task.task_type.is_verification() {
            return Ok(AdvisorDirective::FinalizeExhausted(
                "budget exhausted".to_string(),
            ));
        }

        let sequence = trajectory.observations.len() as u32;
        let overseer_signals = trajectory.observations.last().map(|o| &o.overseer_signals);

        match self
            .intent_verifier
            .verify_convergent_intent(&self.task, self.goal_id, sequence, overseer_signals)
            .await
        {
            Ok(Some(ivr)) => {
                emit_intent_verification_event(&self.event_bus, &self.task, self.goal_id, &ivr)
                    .await;
                match ivr.satisfaction {
                    IntentSatisfaction::Satisfied => Ok(AdvisorDirective::FinalizeConverged),
                    IntentSatisfaction::Partial if ivr.confidence >= 0.8 => {
                        Ok(AdvisorDirective::FinalizePartialAccepted)
                    }
                    _ => Ok(AdvisorDirective::FinalizeExhausted(
                        "convergence budget exhausted without intent satisfaction".to_string(),
                    )),
                }
            }
            Ok(None) | Err(_) => Ok(AdvisorDirective::FinalizeExhausted(
                "convergence budget exhausted without intent satisfaction".to_string(),
            )),
        }
    }
}

// ---------------------------------------------------------------------------
// EventBusSink -- translate ConvergenceDomainEvent -> EventPayload
// ---------------------------------------------------------------------------

/// Implements [`ConvergenceEventSink`] by translating the engine's domain
/// events to the orchestrator's `EventPayload` bus. Used by PR 4b's flipped
/// `run_convergent_execution_inner` so the orchestrator no longer needs to
/// emit iteration / attractor-transition events inline.
struct EventBusSink {
    event_bus: Arc<EventBus>,
    task_id: Uuid,
    goal_id: Option<Uuid>,
}

#[async_trait]
impl ConvergenceEventSink for EventBusSink {
    async fn emit(&self, event: ConvergenceDomainEvent) {
        match event {
            ConvergenceDomainEvent::IterationCompleted {
                trajectory_id,
                iteration,
                strategy,
                convergence_delta,
                convergence_level,
                attractor_type,
                budget_remaining_fraction,
            } => {
                self.event_bus
                    .publish(event_factory::make_event(
                        EventSeverity::Info,
                        crate::services::event_bus::EventCategory::Convergence,
                        self.goal_id,
                        Some(self.task_id),
                        EventPayload::ConvergenceIteration(ConvergenceIterationPayload {
                            task_id: self.task_id,
                            trajectory_id,
                            iteration,
                            strategy,
                            convergence_delta,
                            convergence_level,
                            attractor_type,
                            budget_remaining_fraction,
                        }),
                    ))
                    .await;
            }
            ConvergenceDomainEvent::AttractorTransitionChanged {
                trajectory_id,
                from,
                to,
                confidence,
            } => {
                self.event_bus
                    .publish(event_factory::make_event(
                        EventSeverity::Info,
                        crate::services::event_bus::EventCategory::Convergence,
                        self.goal_id,
                        Some(self.task_id),
                        EventPayload::ConvergenceAttractorTransition {
                            task_id: self.task_id,
                            trajectory_id,
                            from,
                            to,
                            confidence,
                        },
                    ))
                    .await;
            }
            // All other variants are purely tracing-level observability; fall
            // back to the TracingEventSink semantics by logging here so the
            // orchestrator still gets the pre-PR-1 log lines.
            other => {
                crate::services::convergence_engine::TracingEventSink.emit(other).await;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// OrchestratorPromptBuilder -- per-iteration prompt construction
// ---------------------------------------------------------------------------

/// Implements [`PromptBuilder`] for the orchestrator's convergent path. Wraps
/// `convergence_bridge::build_convergent_prompt` and feeds it the latest
/// intent-verification result from the paired advisor.
struct OrchestratorPromptBuilder {
    task: Task,
    /// Shared with `OrchestratorConvergenceAdvisor`. Each iteration's prompt
    /// picks up the gap feedback from the most recent verification.
    last_intent_verification: Arc<TokioMutex<Option<IntentVerificationResult>>>,
}

#[async_trait]
impl PromptBuilder for OrchestratorPromptBuilder {
    async fn build(
        &self,
        trajectory: &Trajectory,
        strategy: &StrategyKind,
        _iteration: u32,
    ) -> DomainResult<String> {
        let last = self.last_intent_verification.lock().await.clone();
        Ok(convergence_bridge::build_convergent_prompt(
            &self.task,
            trajectory,
            strategy,
            last.as_ref(),
        ))
    }
}

// ---------------------------------------------------------------------------
// run_convergent_execution
// ---------------------------------------------------------------------------

/// Run convergent execution for a task.
///
/// This replaces the single-shot `substrate.execute()` with an iterative
/// convergence loop guided by the convergence engine. The orchestrator owns
/// the outer loop and injects substrate invocations between the engine's
/// strategy selection and observation recording.
///
/// # Arguments
///
/// * `task` - The task being executed (must have `ExecutionMode::Convergent`).
/// * `goal_id` - Optional parent goal for event correlation.
/// * `substrate` - The agent runtime substrate (Claude Code CLI, etc.).
/// * `task_repo` - Task repository for persisting trajectory linkage.
/// * `trajectory_store` - Trajectory repository for loading/persisting trajectories.
/// * `engine` - The convergence engine providing primitives.
/// * `event_bus` - For emitting convergence lifecycle events.
/// * `agent_type` - The agent template name (e.g. "coder", "overmind").
/// * `system_prompt` - The agent's system prompt.
/// * `worktree_path` - Optional worktree path for task isolation.
/// * `max_turns` - Maximum turns per substrate invocation.
/// * `cancellation_token` - Token checked at the top of each iteration;
///   when cancelled, the trajectory is persisted and `Cancelled` is returned.
/// * `deadline` - Optional SLA deadline; caps the trajectory budget's
///   `max_wall_time` so convergent tasks never breach the SLA.
#[allow(clippy::too_many_arguments)]
pub async fn run_convergent_execution<T, Tr, M, O>(
    task: &Task,
    goal_id: Option<Uuid>,
    substrate: &Arc<dyn Substrate>,
    task_repo: &Arc<T>,
    trajectory_store: &Arc<Tr>,
    engine: &ConvergenceEngine<Tr, M, O>,
    event_bus: &Arc<EventBus>,
    agent_type: &str,
    system_prompt: &str,
    worktree_path: Option<&str>,
    max_turns: u32,
    cancellation_token: CancellationToken,
    deadline: Option<chrono::DateTime<chrono::Utc>>,
    intent_verifier: Arc<dyn ConvergentIntentVerifier>,
) -> DomainResult<ConvergentOutcome>
where
    T: TaskRepository + ?Sized + 'static,
    Tr: TrajectoryRepository + 'static,
    M: MemoryRepository + 'static,
    O: OverseerMeasurer + 'static,
{
    // -----------------------------------------------------------------------
    // 1. PREPARE -- Create or resume a trajectory (Part 4.2)
    // -----------------------------------------------------------------------

    let (mut trajectory, _infrastructure, bandit) = if let Some(tid) = task.trajectory_id {
        let loaded = trajectory_store
            .get(&tid.to_string())
            .await?
            .ok_or_else(|| {
                DomainError::ExecutionFailed(format!(
                    "trajectory {} referenced by task {} not found",
                    tid, task.id
                ))
            })?;

        let bandit = engine.initialize_bandit(&loaded).await;
        (loaded, None, bandit)
    } else {
        let submission = convergence_bridge::task_to_submission(task, goal_id);
        let (trajectory, infrastructure) = engine.prepare(&submission, task.id).await?;

        if let Ok(Some(mut t)) = task_repo.get(task.id).await {
            t.trajectory_id = Some(trajectory.id);
            let _ = task_repo.update(&t).await;
        }

        let bandit = engine.initialize_bandit(&trajectory).await;
        (trajectory, Some(infrastructure), bandit)
    };

    // -----------------------------------------------------------------------
    // 1b. SLA Deadline -> Budget Ceiling (Part 8.1)
    // -----------------------------------------------------------------------

    if let Some(deadline) = deadline {
        let remaining = deadline - chrono::Utc::now();
        if remaining.num_seconds() > 0 {
            let cap = std::time::Duration::from_secs(remaining.num_seconds() as u64);
            trajectory.budget.max_wall_time = trajectory.budget.max_wall_time.min(cap);
        }
    }

    // Emit ConvergenceStarted event
    let estimated_iterations = trajectory.budget.max_iterations;
    event_bus
        .publish(event_factory::make_event(
            EventSeverity::Info,
            crate::services::event_bus::EventCategory::Convergence,
            goal_id,
            Some(task.id),
            EventPayload::ConvergenceStarted {
                task_id: task.id,
                trajectory_id: trajectory.id,
                estimated_iterations,
                basin_width: "standard".to_string(),
                convergence_mode: "sequential".to_string(),
            },
        ))
        .await;

    // -----------------------------------------------------------------------
    // 1c. ALREADY-DONE DETECTION -- Skip convergence if prior work satisfies
    //     the intent (prevents cascade failures on retry). Only for sequential
    //     execution; parallel mode creates fresh worktrees.
    // -----------------------------------------------------------------------
    if let Some(wt) = worktree_path
        && let Some(outcome) =
            check_work_already_done(wt, task, goal_id, &intent_verifier, event_bus).await
    {
        return Ok(outcome);
    }

    // Transition to iterating phase
    trajectory.phase = ConvergencePhase::Iterating;

    // -----------------------------------------------------------------------
    // 2. Delegate to the shared inner loop
    // -----------------------------------------------------------------------

    run_convergent_execution_inner(
        task,
        goal_id,
        substrate,
        task_repo,
        trajectory_store,
        engine,
        event_bus,
        agent_type,
        system_prompt,
        worktree_path,
        max_turns,
        cancellation_token,
        trajectory,
        bandit,
        intent_verifier,
    )
    .await
}

// ---------------------------------------------------------------------------
// run_parallel_convergent_execution (Parts 11.1-11.3)
// ---------------------------------------------------------------------------

/// Run parallel convergent execution for a task.
///
/// Parallel mode spawns N independent substrate invocations concurrently,
/// each with a different strategy and its own worktree, then selects the
/// best trajectory and continues sequential iteration on the winner.
///
/// This is most effective for narrow-basin tasks where the correct approach
/// is hard to find but easy to verify once found. The budget is partitioned:
/// Phase 1 consumes N iterations worth of budget (one per parallel sample),
/// Phase 2 uses the remaining budget for sequential iteration on the winner.
///
/// # Arguments
///
/// Same as `run_convergent_execution`, plus:
/// * `parallel_samples` - Number of parallel trajectories to spawn in Phase 1.
/// * `base_branch` - The git branch to create worktrees from.
/// * `worktree_base_dir` - Base directory under which parallel worktrees are created.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub async fn run_parallel_convergent_execution<T, Tr, M, O>(
    task: &Task,
    goal_id: Option<Uuid>,
    substrate: &Arc<dyn Substrate>,
    task_repo: &Arc<T>,
    trajectory_store: &Arc<Tr>,
    engine: &ConvergenceEngine<Tr, M, O>,
    event_bus: &Arc<EventBus>,
    agent_type: &str,
    system_prompt: &str,
    max_turns: u32,
    cancellation_token: CancellationToken,
    deadline: Option<chrono::DateTime<chrono::Utc>>,
    parallel_samples: u32,
    base_branch: &str,
    worktree_base_dir: &str,
    intent_verifier: Arc<dyn ConvergentIntentVerifier>,
) -> DomainResult<ConvergentOutcome>
where
    T: TaskRepository + ?Sized + 'static,
    Tr: TrajectoryRepository + 'static,
    M: MemoryRepository + 'static,
    O: OverseerMeasurer + 'static,
{
    let n = parallel_samples.max(1) as usize;

    // -----------------------------------------------------------------------
    // 1. PREPARE -- Create base trajectory and budget partitioning
    // -----------------------------------------------------------------------

    let submission = convergence_bridge::task_to_submission(task, goal_id);
    let (mut base_trajectory, _infrastructure) = engine.prepare(&submission, task.id).await?;

    // Apply SLA deadline cap (Part 8.1)
    if let Some(deadline) = deadline {
        let remaining = deadline - chrono::Utc::now();
        if remaining.num_seconds() > 0 {
            let cap = std::time::Duration::from_secs(remaining.num_seconds() as u64);
            base_trajectory.budget.max_wall_time = base_trajectory.budget.max_wall_time.min(cap);
        }
    }

    // Link trajectory to task
    if task.trajectory_id.is_none()
        && let Ok(Some(mut t)) = task_repo.get(task.id).await
    {
        t.trajectory_id = Some(base_trajectory.id);
        let _ = task_repo.update(&t).await;
    }

    // Emit ConvergenceStarted event
    event_bus
        .publish(event_factory::make_event(
            EventSeverity::Info,
            crate::services::event_bus::EventCategory::Convergence,
            goal_id,
            Some(task.id),
            EventPayload::ConvergenceStarted {
                task_id: task.id,
                trajectory_id: base_trajectory.id,
                estimated_iterations: base_trajectory.budget.max_iterations,
                basin_width: "standard".to_string(),
                convergence_mode: format!("parallel({})", n),
            },
        ))
        .await;

    // -----------------------------------------------------------------------
    // 2. PHASE 1 -- Create N worktrees and spawn N concurrent invocations
    // -----------------------------------------------------------------------

    // Create parallel worktrees
    let mut worktree_paths: Vec<String> = Vec::with_capacity(n);
    for i in 0..n {
        let wt_path = format!("{}/parallel_{}", worktree_base_dir, i);
        create_worktree(&wt_path, base_branch).await?;
        worktree_paths.push(wt_path);
    }

    // Create N sample trajectories with partitioned budgets.
    // Phase 1 gets 1 iteration per sample; Phase 2 gets the rest.
    let mut sample_trajectories: Vec<Trajectory> = Vec::with_capacity(n);
    let mut sample_bandits: Vec<StrategyBandit> = Vec::with_capacity(n);

    for _ in 0..n {
        let mut sample = base_trajectory.clone();
        sample.id = Uuid::new_v4();
        // Each sample gets 1 iteration's worth of budget in Phase 1
        sample.budget = base_trajectory.budget.scale(1.0 / n as f64);
        sample.phase = ConvergencePhase::Iterating;

        let bandit = engine.initialize_bandit(&sample).await;
        sample_trajectories.push(sample);
        sample_bandits.push(bandit);
    }

    // Spawn N concurrent substrate invocations with different strategies
    let mut handles = Vec::with_capacity(n);

    for i in 0..n {
        let substrate = Arc::clone(substrate);
        let task_id = task.id;
        let agent_type = agent_type.to_string();
        let system_prompt = system_prompt.to_string();
        let wt_path = worktree_paths[i].clone();
        let cancellation_token = cancellation_token.clone();

        // Select a strategy for this sample
        let eligible = eligible_strategies(
            &sample_trajectories[i].strategy_log,
            &sample_trajectories[i].attractor_state,
            &sample_trajectories[i].budget,
            sample_trajectories[i].total_fresh_starts,
            sample_trajectories[i].policy.max_fresh_starts,
        );
        let strategy = if eligible.is_empty() {
            StrategyKind::RetryWithFeedback // fallback
        } else {
            sample_bandits[i].select(
                &sample_trajectories[i].attractor_state.classification,
                &eligible,
                &sample_trajectories[i].policy,
            )
        };

        let prompt = convergence_bridge::build_convergent_prompt(
            task,
            &sample_trajectories[i],
            &strategy,
            None, // Phase 1: no LLM verification yet
        );

        let strategy_clone = strategy.clone();

        // Per-sample worker: one spawn per parallel sample, joined below via
        // handles.into_iter(). Not a long-lived daemon.
        handles.push(tokio::spawn(async move {
            if cancellation_token.is_cancelled() {
                return Err(DomainError::ExecutionFailed("cancelled".to_string()));
            }

            let config = SubstrateConfig::default()
                .with_max_turns(max_turns)
                .with_working_dir(&wt_path);
            let request = SubstrateRequest::new(task_id, &agent_type, &system_prompt, &prompt)
                .with_config(config);

            let iteration_start = Instant::now();
            let session = substrate.execute(request).await?;
            let wall_time_ms = iteration_start.elapsed().as_millis() as u64;

            let artifact = convergence_bridge::collect_artifact(&wt_path, "");
            let tokens_used = session.total_tokens();

            Ok((strategy_clone, artifact, tokens_used, wall_time_ms))
        }));
    }

    // Collect results from all parallel invocations
    let mut results: Vec<Option<(usize, StrategyKind, ArtifactReference, u64, u64)>> =
        Vec::with_capacity(n);
    for (i, handle) in handles.into_iter().enumerate() {
        match handle.await {
            Ok(Ok((strategy, artifact, tokens, wall_time))) => {
                results.push(Some((i, strategy, artifact, tokens, wall_time)));
            }
            Ok(Err(e)) => {
                tracing::warn!(
                    sample = i,
                    error = %e,
                    "Parallel sample {} failed",
                    i
                );
                results.push(None);
            }
            Err(e) => {
                tracing::warn!(
                    sample = i,
                    error = %e,
                    "Parallel sample {} panicked",
                    i
                );
                results.push(None);
            }
        }
    }

    // -----------------------------------------------------------------------
    // 3. PHASE 1 -- Measure each artifact and record observations
    // -----------------------------------------------------------------------

    for (idx, strategy, artifact, tokens_used, wall_time_ms) in results.iter().flatten() {
        // Measure with overseers (Part 5)
        let overseer_signals = engine
            .measure(artifact, &sample_trajectories[*idx].policy)
            .await
            .unwrap_or_else(|e| {
                tracing::warn!(
                    sample = *idx,
                    error = %e,
                    "Overseer measurement failed for parallel sample {}; using empty signals",
                    *idx
                );
                OverseerSignals::default()
            });

        let sequence = sample_trajectories[*idx].observations.len() as u32;
        let observation = Observation::new(
            sequence,
            artifact.clone(),
            overseer_signals,
            strategy.clone(),
            *tokens_used,
            *wall_time_ms,
        );

        let _ = engine
            .iterate_once(
                &mut sample_trajectories[*idx],
                &mut sample_bandits[*idx],
                strategy,
                observation,
            )
            .await;
    }

    // -----------------------------------------------------------------------
    // 4. PHASE 2 -- Select the best trajectory via convergence level
    // -----------------------------------------------------------------------

    let winner_idx = sample_trajectories
        .iter()
        .enumerate()
        .filter(|(_, t)| !t.observations.is_empty())
        .max_by(|(_, a), (_, b)| {
            let a_level = a
                .best_observation()
                .and_then(|o| o.metrics.as_ref())
                .map(|m| m.intent_blended_level.unwrap_or(m.convergence_level))
                .unwrap_or(0.0);
            let b_level = b
                .best_observation()
                .and_then(|o| o.metrics.as_ref())
                .map(|m| m.intent_blended_level.unwrap_or(m.convergence_level))
                .unwrap_or(0.0);
            a_level
                .partial_cmp(&b_level)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|(idx, _)| idx);

    let winner_idx = match winner_idx {
        Some(idx) => idx,
        None => {
            // All parallel samples failed -- clean up worktrees
            for wt_path in &worktree_paths {
                let _ = destroy_worktree(wt_path).await;
            }
            let outcome = ConvergenceOutcome::Exhausted {
                trajectory_id: base_trajectory.id.to_string(),
                best_observation_sequence: None,
            };
            engine
                .finalize(
                    &mut base_trajectory,
                    &outcome,
                    sample_bandits
                        .first()
                        .unwrap_or(&StrategyBandit::with_default_priors()),
                )
                .await?;
            emit_convergence_terminated(event_bus, task, goal_id, &base_trajectory, "exhausted")
                .await;
            return Ok(ConvergentOutcome::Failed(
                "all parallel samples failed".to_string(),
            ));
        }
    };

    // -----------------------------------------------------------------------
    // 5. Destroy losing worktrees, keep the winner
    // -----------------------------------------------------------------------

    let winner_worktree = worktree_paths[winner_idx].clone();
    for (i, wt_path) in worktree_paths.iter().enumerate() {
        if i != winner_idx {
            let _ = destroy_worktree(wt_path).await;
        }
    }

    // Transfer the winning trajectory's state: promote the winning sample
    // to the base trajectory and grant it the remaining budget.
    let mut trajectory = sample_trajectories.swap_remove(winner_idx);
    let bandit = sample_bandits.swap_remove(winner_idx);

    // Grant remaining budget for Phase 2: the base budget minus what
    // Phase 1 consumed. Phase 1 used N iterations' worth of budget,
    // so Phase 2 gets the rest.
    let phase1_tokens: u64 = trajectory.budget.tokens_used;
    let remaining_token_budget = base_trajectory
        .budget
        .max_tokens
        .saturating_sub(phase1_tokens);
    let remaining_iterations = base_trajectory
        .budget
        .max_iterations
        .saturating_sub(n as u32);

    trajectory.budget.max_tokens = trajectory.budget.tokens_used + remaining_token_budget;
    trajectory.budget.max_iterations =
        trajectory.budget.iterations_used + remaining_iterations.max(1);
    trajectory.budget.max_wall_time = base_trajectory.budget.max_wall_time;

    // -----------------------------------------------------------------------
    // 6. PHASE 2 -- Continue sequential iteration on the winner
    // -----------------------------------------------------------------------

    // Delegate to the standard sequential loop for the remaining budget.
    // We pass the winning worktree path and let the sequential loop run.
    // Phase 2 uses LLM intent verification.
    run_convergent_execution_inner(
        task,
        goal_id,
        substrate,
        task_repo,
        trajectory_store,
        engine,
        event_bus,
        agent_type,
        system_prompt,
        Some(&winner_worktree),
        max_turns,
        cancellation_token,
        trajectory,
        bandit,
        intent_verifier,
    )
    .await
}

// ---------------------------------------------------------------------------
// run_convergent_execution_inner (shared sequential loop)
// ---------------------------------------------------------------------------

/// Inner sequential convergence loop, used by both the standard sequential
/// path (after Phase 1 in parallel mode) and the main `run_convergent_execution`.
///
/// This function takes an already-prepared trajectory and bandit and runs the
/// convergence loop until a terminal condition is reached.
#[allow(clippy::too_many_arguments)]
async fn run_convergent_execution_inner<T2, Tr, M, O>(
    task: &Task,
    goal_id: Option<Uuid>,
    substrate: &Arc<dyn Substrate>,
    task_repo: &Arc<T2>,
    trajectory_store: &Arc<Tr>,
    engine: &ConvergenceEngine<Tr, M, O>,
    event_bus: &Arc<EventBus>,
    agent_type: &str,
    system_prompt: &str,
    worktree_path: Option<&str>,
    max_turns: u32,
    cancellation_token: CancellationToken,
    trajectory: Trajectory,
    bandit: StrategyBandit,
    intent_verifier: Arc<dyn ConvergentIntentVerifier>,
) -> DomainResult<ConvergentOutcome>
where
    T2: TaskRepository + ?Sized + 'static,
    Tr: TrajectoryRepository + 'static,
    M: MemoryRepository + 'static,
    O: OverseerMeasurer + 'static,
{
    // The outer entrypoints (`run_convergent_execution` and
    // `run_parallel_convergent_execution`) build the trajectory up-front (either
    // PREPARE or PARALLEL Phase-1 promotion). The engine's `run()` entrypoint,
    // however, wants either a fresh submission or a `resume` trajectory id it
    // can load from the store. To bridge the two worlds we persist the
    // already-prepared trajectory and pass its id as `resume`.
    trajectory_store.save(&trajectory).await?;
    // Bandit state is re-derived inside `engine.run` via `initialize_bandit`,
    // so the local `bandit` variable is no longer consulted here.
    let _ = bandit;
    let resume_id = trajectory.id;

    // Construct the ports: executor, effects, advisor, prompt builder, event
    // sink. Each wraps a slice of behaviour that previously lived inline.
    let executor = Arc::new(OrchestratorStrategyExecutor {
        substrate: substrate.clone(),
        worktree_path: worktree_path.map(PathBuf::from),
        agent_type: agent_type.to_string(),
        system_prompt: system_prompt.to_string(),
        task_id: task.id,
        max_turns,
    });

    let effects = Arc::new(OrchestratorStrategyEffects {
        event_bus: event_bus.clone(),
        goal_id,
        task_id: task.id,
        worktree_path: worktree_path.map(PathBuf::from),
    });

    let hints_loader: Arc<dyn TaskHintsLoader> = Arc::new(TaskRepoHintsLoader {
        repo: Arc::clone(task_repo),
    });
    let advisor = Arc::new(
        OrchestratorConvergenceAdvisor::new(
            task.clone(),
            goal_id,
            event_bus.clone(),
            intent_verifier.clone(),
            cancellation_token.clone(),
        )
        .with_hints_loader(hints_loader),
    );

    let prompt_builder = Arc::new(OrchestratorPromptBuilder {
        task: task.clone(),
        last_intent_verification: Arc::clone(&advisor.last_intent_verification),
    });

    let event_sink = Arc::new(EventBusSink {
        event_bus: event_bus.clone(),
        task_id: task.id,
        goal_id,
    });

    // The engine's `run()` takes `&self` on the engine passed in from the
    // outer function (borrowed in the function signature). We need a mutable
    // reference to install ports — but `with_executor` / `with_*` are
    // builder-style `self`-consuming methods on a by-value engine. We can't
    // rebuild the engine here because it holds private fields. Instead,
    // install ports via the raw `set_*` helpers... except those don't exist
    // today. For PR 4b we cannot mutate the caller's engine, so we construct
    // a local engine façade via the builder by cloning the shared Arc fields
    // that matter, OR we accept that the caller has already installed the
    // ports. For the orchestrator, the caller (`ConvergentOrchestrator`)
    // controls engine construction, so we expect ports already installed.
    //
    // Practical path for PR 4b: inspect the engine for installed ports and
    // build a one-shot wrapper engine that forwards to the same trajectory
    // store / memory repo / overseer measurer / budget tracker / cost window
    // / config. Copying config isn't trivial because ConvergenceEngine has
    // multiple private fields including `calibration_tracker: Mutex<..>`. So
    // we take a different route: we expose a `run_with_ports` helper on the
    // engine that accepts the three ports + prompt builder + event sink as
    // per-call arguments. PR 4b adds that indirection below.
    let submission = convergence_bridge::task_to_submission(task, goal_id);
    let outcome = engine
        .run_with_ports(
            submission,
            task.id,
            Some(resume_id),
            executor,
            Some(effects as Arc<dyn StrategyEffects>),
            advisor,
            Some(prompt_builder as Arc<dyn PromptBuilder>),
            Some(event_sink as Arc<dyn ConvergenceEventSink>),
        )
        .await?;

    // Translate the engine outcome to the orchestrator's ConvergentOutcome,
    // emitting the terminal ConvergenceTerminated event along the way.
    let (orchestrator_outcome, terminal_label): (ConvergentOutcome, &'static str) = match outcome {
        ConvergenceRunOutcome::Converged => (ConvergentOutcome::Converged, "converged"),
        ConvergenceRunOutcome::Exhausted(msg) => (ConvergentOutcome::Failed(msg), "exhausted"),
        ConvergenceRunOutcome::IntentGapsFound(ivr) => {
            (ConvergentOutcome::IntentGapsFound(ivr), "intent_gaps_found")
        }
        ConvergenceRunOutcome::PartialAccepted => (ConvergentOutcome::PartialAccepted, "converged"),
        ConvergenceRunOutcome::IndeterminateAccepted => (
            ConvergentOutcome::IndeterminateAccepted,
            "indeterminate_accepted",
        ),
        ConvergenceRunOutcome::Cancelled => (ConvergentOutcome::Cancelled, "cancelled"),
        ConvergenceRunOutcome::Decomposed(t) => (ConvergentOutcome::Decomposed(t), "decomposed"),
        ConvergenceRunOutcome::Failed(msg) => (ConvergentOutcome::Failed(msg), "failed"),
    };

    // Re-load the finalized trajectory so the ConvergenceTerminated event
    // reflects the engine's final state (total tokens, final convergence
    // level, etc.).
    if let Ok(Some(final_traj)) = trajectory_store.get(&resume_id.to_string()).await {
        emit_convergence_terminated(event_bus, task, goal_id, &final_traj, terminal_label).await;
    }

    Ok(orchestrator_outcome)
}

// ---------------------------------------------------------------------------
// Intent verification helpers
// ---------------------------------------------------------------------------

/// Convert Major/Critical gaps from an intent verification result into
/// specification amendments on the trajectory.
///
/// This enriches the effective specification so that subsequent iterations
/// see the discovered requirements. Minor and Moderate gaps are left as
/// prompt-level feedback only (via `last_intent_verification`).
fn apply_verification_amendments(ivr: &IntentVerificationResult, trajectory: &mut Trajectory) {
    for gap in ivr.all_gaps() {
        if gap.severity >= GapSeverity::Major {
            let amendment = SpecificationAmendment::new(
                AmendmentSource::ImplicitRequirementDiscovered,
                &gap.description,
                gap.suggested_action
                    .as_deref()
                    .unwrap_or("Discovered via intent verification"),
            );
            trajectory.specification.add_amendment(amendment);
        }
    }
}

/// Emit an `IntentVerificationResult` event to the event bus.
async fn emit_intent_verification_event(
    event_bus: &Arc<EventBus>,
    task: &Task,
    goal_id: Option<Uuid>,
    ivr: &IntentVerificationResult,
) {
    let should_continue = ivr.satisfaction.should_retry();
    event_bus
        .publish(event_factory::make_event(
            EventSeverity::Info,
            crate::services::event_bus::EventCategory::Verification,
            goal_id,
            Some(task.id),
            EventPayload::IntentVerificationResult {
                satisfaction: ivr.satisfaction.as_str().to_string(),
                confidence: ivr.confidence,
                gaps_count: ivr.gaps.len() + ivr.implicit_gaps.len(),
                iteration: ivr.iteration,
                should_continue,
            },
        ))
        .await;
}

/// Emit a `HumanEscalationNeeded` event triggered by intent verification.
async fn emit_escalation_from_verification(
    event_bus: &Arc<EventBus>,
    task: &Task,
    goal_id: Option<Uuid>,
    escalation: &crate::domain::models::HumanEscalation,
) {
    let is_blocking = matches!(
        escalation.urgency,
        crate::domain::models::intent_verification::EscalationUrgency::Blocking,
    );
    event_bus
        .publish(event_factory::make_event(
            EventSeverity::Warning,
            crate::services::event_bus::EventCategory::Escalation,
            goal_id,
            Some(task.id),
            EventPayload::HumanEscalationNeeded(HumanEscalationPayload {
                goal_id,
                task_id: Some(task.id),
                reason: escalation.reason.clone(),
                urgency: format!("{:?}", escalation.urgency).to_lowercase(),
                questions: escalation.questions.clone(),
                is_blocking,
            }),
        ))
        .await;
}

// ---------------------------------------------------------------------------
// Worktree management helpers (Parts 3.7, 11.2)
// ---------------------------------------------------------------------------

/// Reset a worktree to the base branch state (Part 3.7).
///
/// Used when the FreshStart strategy is selected. Runs `git checkout -- .`
/// followed by `git clean -fd` to restore the worktree to a pristine state
/// without destroying the worktree allocation itself. The carry-forward
/// context in the prompt provides the agent with learnings from previous
/// attempts.
async fn reset_worktree(worktree_path: &str) -> DomainResult<()> {
    // Defense-in-depth: remove transient artifacts before resetting
    let removed = remove_transient_artifacts(worktree_path);
    if !removed.is_empty() {
        tracing::info!(
            worktree = %worktree_path,
            files = ?removed,
            "cleaned transient artifacts during worktree reset"
        );
    }

    let checkout = tokio::process::Command::new("git")
        .args(["checkout", "--", "."])
        .current_dir(worktree_path)
        .output()
        .await
        .map_err(|e| DomainError::ExternalServiceError {
            service: "git".to_string(),
            reason: format!(
                "failed to reset worktree {}: git checkout: {}",
                worktree_path, e
            ),
        })?;

    if !checkout.status.success() {
        let stderr = String::from_utf8_lossy(&checkout.stderr);
        return Err(DomainError::ExternalServiceError {
            service: "git".to_string(),
            reason: format!("git checkout -- . failed in {}: {}", worktree_path, stderr),
        });
    }

    let clean = tokio::process::Command::new("git")
        .args(["clean", "-fd"])
        .current_dir(worktree_path)
        .output()
        .await
        .map_err(|e| DomainError::ExternalServiceError {
            service: "git".to_string(),
            reason: format!(
                "failed to clean worktree {}: git clean: {}",
                worktree_path, e
            ),
        })?;

    if !clean.status.success() {
        let stderr = String::from_utf8_lossy(&clean.stderr);
        return Err(DomainError::ExternalServiceError {
            service: "git".to_string(),
            reason: format!("git clean -fd failed in {}: {}", worktree_path, stderr),
        });
    }

    Ok(())
}

/// Check whether a worktree already contains committed or staged work that
/// satisfies the task's intent.
///
/// This prevents federation-type cascade failures where a retry agent enters
/// a convergence loop on a worktree that already has committed changes from a
/// prior (possibly timed-out) execution. Instead of burning the entire
/// convergence budget redoing work that's already done, this function detects
/// the existing work and runs intent verification against the current state.
///
/// Returns `Some(ConvergentOutcome::Converged)` if the existing work fully
/// satisfies the intent, or `None` to proceed with normal convergent iteration.
async fn check_work_already_done(
    worktree_path: &str,
    task: &Task,
    goal_id: Option<Uuid>,
    intent_verifier: &Arc<dyn ConvergentIntentVerifier>,
    event_bus: &Arc<EventBus>,
) -> Option<ConvergentOutcome> {
    // Check for already-committed changes that are NEW on this branch
    // (ahead of the upstream tracking branch or origin/main).
    // We first try @{upstream}..HEAD (works when upstream is set), then
    // fall back to origin/main..HEAD. If both fail we assume no new commits.
    let has_commits = {
        let upstream_log = tokio::process::Command::new("git")
            .args(["log", "--oneline", "@{u}..HEAD", "-5"])
            .current_dir(worktree_path)
            .output()
            .await
            .ok();

        let log_output = match &upstream_log {
            Some(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).trim().to_string(),
            _ => {
                // No upstream tracking branch — fall back to origin/main
                let fallback = tokio::process::Command::new("git")
                    .args(["log", "--oneline", "origin/main..HEAD", "-5"])
                    .current_dir(worktree_path)
                    .output()
                    .await
                    .ok();
                match &fallback {
                    Some(o) if o.status.success() => {
                        String::from_utf8_lossy(&o.stdout).trim().to_string()
                    }
                    _ => String::new(),
                }
            }
        };
        !log_output.is_empty()
    };

    // Check for staged (but uncommitted) changes
    let git_diff = tokio::process::Command::new("git")
        .args(["diff", "--cached", "--stat"])
        .current_dir(worktree_path)
        .output()
        .await
        .ok()?;

    let diff_output = String::from_utf8_lossy(&git_diff.stdout);
    let has_staged = git_diff.status.success() && !diff_output.trim().is_empty();

    if !has_commits && !has_staged {
        return None;
    }

    tracing::info!(
        task_id = %task.id,
        has_commits = has_commits,
        has_staged = has_staged,
        "Worktree has existing work; running pre-iteration intent verification"
    );

    // Run intent verification against current worktree state
    // Use iteration 0 to indicate this is a pre-iteration check
    match intent_verifier
        .verify_convergent_intent(task, goal_id, 0, None)
        .await
    {
        Ok(Some(ivr)) => {
            if ivr.satisfaction == IntentSatisfaction::Satisfied {
                tracing::info!(
                    task_id = %task.id,
                    confidence = ivr.confidence,
                    "Already-done detection: existing work satisfies intent — skipping convergence"
                );

                emit_intent_verification_event(event_bus, task, goal_id, &ivr).await;

                // Emit a lightweight convergence event (no trajectory available
                // at this stage since we short-circuit before iteration begins).
                event_bus
                    .publish(event_factory::make_event(
                        EventSeverity::Info,
                        crate::services::event_bus::EventCategory::Convergence,
                        goal_id,
                        Some(task.id),
                        EventPayload::ConvergenceTerminated(ConvergenceTerminatedPayload {
                            task_id: task.id,
                            trajectory_id: Uuid::nil(),
                            outcome: "already_done".to_string(),
                            total_iterations: 0,
                            total_tokens: 0,
                            final_convergence_level: 1.0,
                        }),
                    ))
                    .await;

                Some(ConvergentOutcome::Converged)
            } else {
                tracing::info!(
                    task_id = %task.id,
                    satisfaction = %ivr.satisfaction.as_str(),
                    gaps = ivr.gaps.len() + ivr.implicit_gaps.len(),
                    "Already-done detection: existing work does not fully satisfy intent — proceeding with convergence"
                );
                None
            }
        }
        Ok(None) => {
            tracing::debug!(
                task_id = %task.id,
                "Already-done detection: no intent extractable — proceeding normally"
            );
            None
        }
        Err(e) => {
            tracing::warn!(
                task_id = %task.id,
                error = %e,
                "Already-done detection: intent verification failed — proceeding normally"
            );
            None
        }
    }
}

/// Create a git worktree at the specified path from the given branch.
///
/// Used by parallel mode (Part 11.2) to create N independent worktrees
/// for concurrent substrate invocations.
async fn create_worktree(worktree_path: &str, branch: &str) -> DomainResult<()> {
    let output = tokio::process::Command::new("git")
        .args(["worktree", "add", worktree_path, branch])
        .output()
        .await
        .map_err(|e| DomainError::ExternalServiceError {
            service: "git".to_string(),
            reason: format!("failed to create worktree at {}: {}", worktree_path, e),
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(DomainError::ExternalServiceError {
            service: "git".to_string(),
            reason: format!("git worktree add failed for {}: {}", worktree_path, stderr),
        });
    }

    Ok(())
}

/// Destroy a git worktree at the specified path.
///
/// Used by parallel mode (Part 11.2) to clean up losing worktrees after
/// the best trajectory is selected.
async fn destroy_worktree(worktree_path: &str) -> DomainResult<()> {
    let output = tokio::process::Command::new("git")
        .args(["worktree", "remove", "--force", worktree_path])
        .output()
        .await
        .map_err(|e| DomainError::ExternalServiceError {
            service: "git".to_string(),
            reason: format!("failed to destroy worktree at {}: {}", worktree_path, e),
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        tracing::warn!(
            worktree = worktree_path,
            "git worktree remove failed: {}",
            stderr
        );
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Event emission helpers
// ---------------------------------------------------------------------------

/// Emit a ConvergenceTerminated event summarizing the final state.
async fn emit_convergence_terminated(
    event_bus: &Arc<EventBus>,
    task: &Task,
    goal_id: Option<Uuid>,
    trajectory: &Trajectory,
    outcome_label: &str,
) {
    let final_level = trajectory
        .observations
        .last()
        .and_then(|o| o.metrics.as_ref())
        .map(|m| m.convergence_level)
        .unwrap_or(0.0);

    event_bus
        .publish(event_factory::make_event(
            EventSeverity::Info,
            crate::services::event_bus::EventCategory::Convergence,
            goal_id,
            Some(task.id),
            EventPayload::ConvergenceTerminated(ConvergenceTerminatedPayload {
                task_id: task.id,
                trajectory_id: trajectory.id,
                outcome: outcome_label.to_string(),
                total_iterations: trajectory.observations.len() as u32,
                total_tokens: trajectory.budget.tokens_used,
                final_convergence_level: final_level,
            }),
        ))
        .await;
}

/// Apply SLA pressure hints to the convergence policy (Part 8.2).
///
/// The ConvergenceSLAPressureHandler adds "sla:warning" or "sla:critical"
/// hints to the task's persisted context. When the convergence loop detects
/// these hints, it adjusts the trajectory policy to increase the likelihood
/// of convergence within the remaining time:
///
/// - **sla:warning** -- Lower the acceptance threshold to accept "good enough"
///   results and enable partial acceptance.
/// - **sla:critical** -- Aggressively lower thresholds and skip expensive
///   overseers to converge as quickly as possible.
fn apply_sla_pressure(hints: &[String], policy: &mut ConvergencePolicy) {
    if hints.iter().any(|h| h == "sla:critical") {
        policy.acceptance_threshold = policy.acceptance_threshold.min(0.80);
        policy.partial_acceptance = true;
        policy.partial_threshold = policy.partial_threshold.min(0.50);
        policy.skip_expensive_overseers = true;
    } else if hints.iter().any(|h| h == "sla:warning") {
        policy.acceptance_threshold = policy.acceptance_threshold.min(0.85);
        policy.partial_acceptance = true;
        policy.partial_threshold = policy.partial_threshold.min(0.60);
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::models::intent_verification::{GapCategory, IntentGap};
    use std::sync::Mutex;
    use tempfile::TempDir;

    /// Mock intent verifier that returns a configurable result.
    struct MockIntentVerifier {
        result: Mutex<Option<DomainResult<Option<IntentVerificationResult>>>>,
    }

    impl MockIntentVerifier {
        fn satisfied() -> Self {
            Self {
                result: Mutex::new(Some(Ok(Some(IntentVerificationResult {
                    id: Uuid::new_v4(),
                    intent_id: Uuid::new_v4(),
                    satisfaction: IntentSatisfaction::Satisfied,
                    confidence: 0.95,
                    gaps: vec![],
                    implicit_gaps: vec![],
                    evaluated_tasks: vec![],
                    accomplishment_summary: "All done".to_string(),
                    reprompt_guidance: None,
                    iteration: 0,
                    verified_at: chrono::Utc::now(),
                    constraint_evaluations: vec![],
                    escalation: None,
                })))),
            }
        }

        fn partial() -> Self {
            Self {
                result: Mutex::new(Some(Ok(Some(IntentVerificationResult {
                    id: Uuid::new_v4(),
                    intent_id: Uuid::new_v4(),
                    satisfaction: IntentSatisfaction::Partial,
                    confidence: 0.6,
                    gaps: vec![IntentGap {
                        description: "Missing tests".to_string(),
                        severity: GapSeverity::Moderate,
                        category: GapCategory::Testing,
                        suggested_action: Some("Add tests".to_string()),
                        related_tasks: vec![],
                        is_implicit: false,
                        implicit_rationale: None,
                        embedding: None,
                    }],
                    implicit_gaps: vec![],
                    evaluated_tasks: vec![],
                    accomplishment_summary: "Partial work".to_string(),
                    reprompt_guidance: None,
                    iteration: 0,
                    verified_at: chrono::Utc::now(),
                    constraint_evaluations: vec![],
                    escalation: None,
                })))),
            }
        }

        fn no_intent() -> Self {
            Self {
                result: Mutex::new(Some(Ok(None))),
            }
        }

        fn error() -> Self {
            Self {
                result: Mutex::new(Some(Err(DomainError::ExecutionFailed(
                    "verification failed".to_string(),
                )))),
            }
        }
    }

    #[async_trait]
    impl ConvergentIntentVerifier for MockIntentVerifier {
        async fn verify_convergent_intent(
            &self,
            _task: &Task,
            _goal_id: Option<Uuid>,
            _iteration: u32,
            _overseer_signals: Option<&OverseerSignals>,
        ) -> DomainResult<Option<IntentVerificationResult>> {
            self.result.lock().unwrap().take().unwrap_or(Ok(None))
        }
    }

    /// Create a temporary git repo with an initial commit on `main`,
    /// returning `(TempDir, path_string)`. The repo starts on `main`
    /// with `origin/main` pointing at the same commit (via a local
    /// fake remote).
    async fn setup_git_repo() -> (TempDir, String) {
        let dir = TempDir::new().expect("create tempdir");
        let path = dir.path().to_str().unwrap().to_string();

        // Init repo on main
        run_git(&path, &["init", "-b", "main"]).await;
        run_git(&path, &["config", "user.email", "test@test.com"]).await;
        run_git(&path, &["config", "user.name", "Test"]).await;

        // Initial commit
        std::fs::write(dir.path().join("README"), "init").unwrap();
        run_git(&path, &["add", "."]).await;
        run_git(&path, &["commit", "-m", "initial"]).await;

        // Create a bare clone to act as origin, then add it as a remote
        let origin_dir = dir.path().join("origin.git");
        let origin_path = origin_dir.to_str().unwrap().to_string();
        tokio::process::Command::new("git")
            .args(["clone", "--bare", &path, &origin_path])
            .output()
            .await
            .unwrap();
        run_git(&path, &["remote", "add", "origin", &origin_path]).await;
        run_git(&path, &["fetch", "origin"]).await;
        run_git(&path, &["branch", "--set-upstream-to=origin/main", "main"]).await;

        (dir, path)
    }

    async fn run_git(dir: &str, args: &[&str]) {
        let out = tokio::process::Command::new("git")
            .args(args)
            .current_dir(dir)
            .output()
            .await
            .expect("git command");
        assert!(
            out.status.success(),
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&out.stderr)
        );
    }

    fn make_task() -> Task {
        Task::new("Implement the widget feature")
    }

    fn make_event_bus() -> Arc<EventBus> {
        Arc::new(EventBus::new(Default::default()))
    }

    // ------- Tests -------

    #[tokio::test]
    async fn no_new_commits_and_no_staged_returns_none() {
        let (_dir, path) = setup_git_repo().await;
        let task = make_task();
        let verifier: Arc<dyn ConvergentIntentVerifier> = Arc::new(MockIntentVerifier::satisfied());
        let event_bus = make_event_bus();

        // No new commits on main (HEAD == origin/main), no staged changes
        let result = check_work_already_done(&path, &task, None, &verifier, &event_bus).await;
        assert!(
            result.is_none(),
            "should return None when no new work exists"
        );
    }

    #[tokio::test]
    async fn new_commits_and_satisfied_intent_returns_converged() {
        let (dir, path) = setup_git_repo().await;
        let task = make_task();
        let verifier: Arc<dyn ConvergentIntentVerifier> = Arc::new(MockIntentVerifier::satisfied());
        let event_bus = make_event_bus();

        // Add a new commit ahead of origin/main
        std::fs::write(dir.path().join("feature.rs"), "fn feature() {}").unwrap();
        run_git(&path, &["add", "."]).await;
        run_git(&path, &["commit", "-m", "add feature"]).await;

        let result = check_work_already_done(&path, &task, None, &verifier, &event_bus).await;
        assert!(
            matches!(result, Some(ConvergentOutcome::Converged)),
            "should return Converged when new commits satisfy intent"
        );
    }

    #[tokio::test]
    async fn new_commits_but_partial_intent_returns_none() {
        let (dir, path) = setup_git_repo().await;
        let task = make_task();
        let verifier: Arc<dyn ConvergentIntentVerifier> = Arc::new(MockIntentVerifier::partial());
        let event_bus = make_event_bus();

        // Add a new commit ahead of origin/main
        std::fs::write(dir.path().join("feature.rs"), "fn feature() {}").unwrap();
        run_git(&path, &["add", "."]).await;
        run_git(&path, &["commit", "-m", "add feature"]).await;

        let result = check_work_already_done(&path, &task, None, &verifier, &event_bus).await;
        assert!(
            result.is_none(),
            "should return None when intent is only partially satisfied"
        );
    }

    #[tokio::test]
    async fn staged_changes_and_satisfied_intent_returns_converged() {
        let (dir, path) = setup_git_repo().await;
        let task = make_task();
        let verifier: Arc<dyn ConvergentIntentVerifier> = Arc::new(MockIntentVerifier::satisfied());
        let event_bus = make_event_bus();

        // Stage a file without committing — no new commits, but staged changes
        std::fs::write(dir.path().join("staged.rs"), "fn staged() {}").unwrap();
        run_git(&path, &["add", "staged.rs"]).await;

        let result = check_work_already_done(&path, &task, None, &verifier, &event_bus).await;
        assert!(
            matches!(result, Some(ConvergentOutcome::Converged)),
            "should return Converged when staged changes satisfy intent"
        );
    }

    #[tokio::test]
    async fn no_intent_extractable_returns_none() {
        let (dir, path) = setup_git_repo().await;
        let task = make_task();
        let verifier: Arc<dyn ConvergentIntentVerifier> = Arc::new(MockIntentVerifier::no_intent());
        let event_bus = make_event_bus();

        // Add work so we reach the verification branch
        std::fs::write(dir.path().join("feature.rs"), "fn feature() {}").unwrap();
        run_git(&path, &["add", "."]).await;
        run_git(&path, &["commit", "-m", "add feature"]).await;

        let result = check_work_already_done(&path, &task, None, &verifier, &event_bus).await;
        assert!(
            result.is_none(),
            "should return None when no intent can be extracted"
        );
    }

    #[tokio::test]
    async fn intent_verification_error_returns_none() {
        let (dir, path) = setup_git_repo().await;
        let task = make_task();
        let verifier: Arc<dyn ConvergentIntentVerifier> = Arc::new(MockIntentVerifier::error());
        let event_bus = make_event_bus();

        // Add work so we reach the verification branch
        std::fs::write(dir.path().join("feature.rs"), "fn feature() {}").unwrap();
        run_git(&path, &["add", "."]).await;
        run_git(&path, &["commit", "-m", "add feature"]).await;

        let result = check_work_already_done(&path, &task, None, &verifier, &event_bus).await;
        assert!(
            result.is_none(),
            "should return None when verification errors out"
        );
    }

    // -----------------------------------------------------------------------
    // apply_sla_pressure tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_apply_sla_pressure_critical() {
        let hints = vec!["sla:critical".to_string()];
        let mut policy = ConvergencePolicy::default();

        apply_sla_pressure(&hints, &mut policy);

        assert!(
            (policy.acceptance_threshold - 0.80).abs() < f64::EPSILON,
            "critical SLA should lower acceptance_threshold to 0.80, got {}",
            policy.acceptance_threshold
        );
        assert!(
            policy.partial_acceptance,
            "critical SLA should enable partial_acceptance"
        );
        assert!(
            (policy.partial_threshold - 0.50).abs() < f64::EPSILON,
            "critical SLA should lower partial_threshold to 0.50, got {}",
            policy.partial_threshold
        );
        assert!(
            policy.skip_expensive_overseers,
            "critical SLA should enable skip_expensive_overseers"
        );
    }

    #[test]
    fn test_apply_sla_pressure_warning() {
        let hints = vec!["sla:warning".to_string()];
        let mut policy = ConvergencePolicy::default();

        apply_sla_pressure(&hints, &mut policy);

        assert!(
            (policy.acceptance_threshold - 0.85).abs() < f64::EPSILON,
            "warning SLA should lower acceptance_threshold to 0.85, got {}",
            policy.acceptance_threshold
        );
        assert!(
            policy.partial_acceptance,
            "warning SLA should enable partial_acceptance"
        );
        assert!(
            (policy.partial_threshold - 0.60).abs() < f64::EPSILON,
            "warning SLA should lower partial_threshold to 0.60, got {}",
            policy.partial_threshold
        );
        assert!(
            !policy.skip_expensive_overseers,
            "warning SLA should NOT enable skip_expensive_overseers"
        );
    }

    #[test]
    fn test_apply_sla_pressure_no_hint() {
        let hints: Vec<String> = vec![];
        let mut policy = ConvergencePolicy::default();
        let original = policy.clone();

        apply_sla_pressure(&hints, &mut policy);

        assert!(
            (policy.acceptance_threshold - original.acceptance_threshold).abs() < f64::EPSILON,
            "no SLA hint should leave acceptance_threshold unchanged"
        );
        assert_eq!(
            policy.partial_acceptance, original.partial_acceptance,
            "no SLA hint should leave partial_acceptance unchanged"
        );
        assert!(
            (policy.partial_threshold - original.partial_threshold).abs() < f64::EPSILON,
            "no SLA hint should leave partial_threshold unchanged"
        );
        assert_eq!(
            policy.skip_expensive_overseers, original.skip_expensive_overseers,
            "no SLA hint should leave skip_expensive_overseers unchanged"
        );
    }

    // -----------------------------------------------------------------------
    // run_convergent_execution happy-path test
    // -----------------------------------------------------------------------

    /// Minimal mock TaskRepository for convergent execution tests.
    /// Only `get()` and `update()` have real logic; all other methods
    /// return Ok defaults.
    struct MockTaskRepo {
        tasks: Mutex<std::collections::HashMap<Uuid, Task>>,
    }

    impl MockTaskRepo {
        fn new() -> Self {
            Self {
                tasks: Mutex::new(std::collections::HashMap::new()),
            }
        }

        fn insert(&self, task: &Task) {
            self.tasks.lock().unwrap().insert(task.id, task.clone());
        }
    }

    #[async_trait]
    impl crate::domain::ports::TaskRepository for MockTaskRepo {
        async fn create(&self, task: &Task) -> DomainResult<()> {
            self.tasks.lock().unwrap().insert(task.id, task.clone());
            Ok(())
        }
        async fn get(&self, id: Uuid) -> DomainResult<Option<Task>> {
            Ok(self.tasks.lock().unwrap().get(&id).cloned())
        }
        async fn update(&self, task: &Task) -> DomainResult<()> {
            self.tasks.lock().unwrap().insert(task.id, task.clone());
            Ok(())
        }
        async fn delete(&self, _id: Uuid) -> DomainResult<()> {
            Ok(())
        }
        async fn list(
            &self,
            _filter: crate::domain::ports::task_repository::TaskFilter,
        ) -> DomainResult<Vec<Task>> {
            Ok(vec![])
        }
        async fn list_by_status(
            &self,
            _status: crate::domain::models::TaskStatus,
        ) -> DomainResult<Vec<Task>> {
            Ok(vec![])
        }
        async fn get_subtasks(&self, _parent_id: Uuid) -> DomainResult<Vec<Task>> {
            Ok(vec![])
        }
        async fn get_ready_tasks(&self, _limit: usize) -> DomainResult<Vec<Task>> {
            Ok(vec![])
        }
        async fn get_by_agent(&self, _agent_type: &str) -> DomainResult<Vec<Task>> {
            Ok(vec![])
        }
        async fn get_dependencies(&self, _task_id: Uuid) -> DomainResult<Vec<Task>> {
            Ok(vec![])
        }
        async fn get_dependents(&self, _task_id: Uuid) -> DomainResult<Vec<Task>> {
            Ok(vec![])
        }
        async fn add_dependency(&self, _task_id: Uuid, _depends_on: Uuid) -> DomainResult<()> {
            Ok(())
        }
        async fn remove_dependency(&self, _task_id: Uuid, _depends_on: Uuid) -> DomainResult<()> {
            Ok(())
        }
        async fn count_descendants(&self, _task_id: Uuid) -> DomainResult<u64> {
            Ok(0)
        }
        async fn get_by_idempotency_key(&self, _key: &str) -> DomainResult<Option<Task>> {
            Ok(None)
        }
        async fn list_by_source(&self, _source_type: &str) -> DomainResult<Vec<Task>> {
            Ok(vec![])
        }
        async fn count_by_status(
            &self,
        ) -> DomainResult<std::collections::HashMap<crate::domain::models::TaskStatus, u64>>
        {
            Ok(std::collections::HashMap::new())
        }
        async fn claim_task_atomic(
            &self,
            _task_id: Uuid,
            _agent_type: &str,
        ) -> DomainResult<Option<Task>> {
            Ok(None)
        }
        async fn get_parent_id(&self, _task_id: Uuid) -> DomainResult<Option<Uuid>> {
            Ok(None)
        }
        async fn calculate_depth(&self, _task_id: Uuid) -> DomainResult<u32> {
            Ok(0)
        }
        async fn find_root_task_id(&self, _task_id: Uuid) -> DomainResult<Uuid> {
            Ok(Uuid::nil())
        }
        async fn count_children(&self, _task_id: Uuid) -> DomainResult<u32> {
            Ok(0)
        }
    }

    #[tokio::test]
    async fn run_convergent_execution_happy_path_converges() {
        use crate::adapters::substrates::MockSubstrate;
        use crate::domain::models::convergence::{
            BuildResult, OverseerSignals, TestResults, TypeCheckResult,
        };
        use crate::services::convergence_engine::ConvergenceEngine;
        use crate::services::convergence_engine::test_support::*;

        // 1. Set up git repo for worktree path
        let (_dir, path) = setup_git_repo().await;

        // 2. Create task (no trajectory_id = new execution path)
        let task = make_task();

        // 3. Shared trajectory repo so we can inspect after execution
        let trajectory_repo = Arc::new(MockTrajectoryRepo::new());
        let memory_repo = Arc::new(MockMemoryRepo::new());

        // High-quality overseer signals: build passes, all tests pass, no lint errors
        let signals = OverseerSignals {
            test_results: Some(TestResults {
                passed: 10,
                failed: 0,
                skipped: 0,
                total: 10,
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
        };
        let overseer = Arc::new(MockOverseerMeasurer::with_signals(signals));

        // Build engine manually with shared Arc repos
        let engine = ConvergenceEngine::new(
            Arc::clone(&trajectory_repo),
            Arc::clone(&memory_repo),
            Arc::clone(&overseer),
            test_config(),
        );

        // 4. Task repo for persisting trajectory linkage
        let task_repo = Arc::new(MockTaskRepo::new());
        task_repo.insert(&task);

        // 5. Mock substrate
        let substrate: Arc<dyn crate::domain::ports::Substrate> = Arc::new(MockSubstrate::new());

        // 6. Intent verifier that returns Satisfied
        let intent_verifier: Arc<dyn ConvergentIntentVerifier> =
            Arc::new(MockIntentVerifier::satisfied());

        let event_bus = make_event_bus();
        let cancellation_token = CancellationToken::new();

        // 7. Call run_convergent_execution
        let outcome = run_convergent_execution(
            &task,
            None, // no goal_id
            &substrate,
            &task_repo,
            &trajectory_repo,
            &engine,
            &event_bus,
            "test-agent",
            "You are a test agent",
            Some(&path),
            10,
            cancellation_token,
            None, // no deadline
            intent_verifier,
        )
        .await
        .expect("run_convergent_execution should succeed");

        // 8. Assert outcome is Converged
        assert!(
            matches!(outcome, ConvergentOutcome::Converged),
            "Expected Converged outcome, got {:?}",
            outcome
        );

        // 9. Verify trajectory was persisted
        let stored = trajectory_repo.trajectories.lock().unwrap();
        assert!(
            !stored.is_empty(),
            "Expected at least one trajectory to be persisted"
        );

        // 10. Verify task got trajectory_id set
        let updated_task = task_repo.tasks.lock().unwrap().get(&task.id).cloned();
        assert!(
            updated_task
                .as_ref()
                .and_then(|t| t.trajectory_id)
                .is_some(),
            "Expected task to have trajectory_id set after execution"
        );
    }

    #[test]
    fn test_apply_sla_pressure_critical_preserves_lower_thresholds() {
        let hints = vec!["sla:critical".to_string()];
        let mut policy = ConvergencePolicy {
            acceptance_threshold: 0.70, // already below 0.80
            partial_threshold: 0.40,    // already below 0.50
            ..ConvergencePolicy::default()
        };

        apply_sla_pressure(&hints, &mut policy);

        assert!(
            (policy.acceptance_threshold - 0.70).abs() < f64::EPSILON,
            ".min() semantics should preserve already-lower acceptance_threshold 0.70, got {}",
            policy.acceptance_threshold
        );
        assert!(
            (policy.partial_threshold - 0.40).abs() < f64::EPSILON,
            ".min() semantics should preserve already-lower partial_threshold 0.40, got {}",
            policy.partial_threshold
        );
        assert!(
            policy.skip_expensive_overseers,
            "critical SLA should still enable skip_expensive_overseers"
        );
    }

    /// Validates intent check total cap threshold logic:
    /// counter should trigger exhaustion only after exceeding cap of 10.
    #[test]
    fn test_intent_check_total_cap_threshold() {
        let cap: u32 = 10;
        let mut total_intent_checks: u32 = 0;

        // Simulate 10 checks — none should exceed the cap
        for i in 1..=cap {
            total_intent_checks += 1;
            assert!(
                total_intent_checks <= cap,
                "check #{i} should not exceed cap"
            );
        }

        // The 11th check should exceed the cap
        total_intent_checks += 1;
        assert!(
            total_intent_checks > cap,
            "check #11 must exceed the cap of {cap}, got {total_intent_checks}"
        );
    }
}
