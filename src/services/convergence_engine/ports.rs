//! Observability ports for the convergence engine.
//!
//! Defines [`ConvergenceDomainEvent`] -- the full catalogue of engine-internal
//! domain events that are observability-relevant -- and the
//! [`ConvergenceEventSink`] trait used to route those events to a concrete
//! sink. Two sinks are provided out of the box:
//!
//! - [`TracingEventSink`] preserves the engine's original `tracing::{info,warn}`
//!   output, so wiring the sink is behaviour-preserving by default.
//! - [`NullEventSink`] is a no-op sink for tests that don't care about events.
//!
//! This module is the foundation for the engine-as-core refactor
//! (#13/#21): subsequent PRs plug `StrategyExecutor`, `StrategyEffects`, and
//! `ConvergenceAdvisor` into the same ports module. `ConvergenceDomainEvent`
//! is deliberately decoupled from the orchestrator's `EventPayload` bus enum
//! so the engine stays free of orchestrator imports.

use async_trait::async_trait;
use uuid::Uuid;

use crate::domain::errors::DomainResult;
use crate::domain::models::convergence::{
    ArtifactReference, AttractorType, StrategyKind, Trajectory,
};
use crate::domain::models::intent_verification::IntentVerificationResult;
use crate::services::event_bus::BudgetPressureLevel;

use super::StrategyContext;

// ---------------------------------------------------------------------------
// ConvergenceDomainEvent
// ---------------------------------------------------------------------------

/// Domain-level events emitted by the convergence engine's internal phases
/// for observability. These correspond to the structured `tracing::{info,warn}`
/// call sites previously scattered through `iterate.rs`, `decide.rs`, and
/// `resolve.rs`.
///
/// Variants carry the same structured fields the original log sites carried
/// (`trajectory_id`, `strategy`, `attractor`, etc.) so a `TracingEventSink`
/// can reproduce the original log lines verbatim.
#[derive(Debug)]
pub enum ConvergenceDomainEvent {
    // -- iterate.rs -----------------------------------------------------
    /// Acceptance tests were generated/discovered for the trajectory.
    AcceptanceTestsGenerated {
        count: usize,
    },
    /// Global budget tracker hit Critical pressure; loop is terminating early.
    BudgetCriticalTerminating {
        trajectory_id: String,
    },
    /// Decay-aware rotation filtered out the current exploitation strategy.
    StrategyRotationTriggered {
        strategy: &'static str,
        consecutive_uses: u32,
    },
    /// The attractor classification changed between iterations (spec 7.3
    /// intervention point).
    AttractorTransition {
        trajectory_id: String,
        from: &'static str,
        to: &'static str,
    },
    /// A strategy is about to be executed for a trajectory.
    StrategyExecutionStarted {
        strategy: &'static str,
        trajectory_id: String,
    },
    /// An `ArchitectReview` strategy produced a specification amendment.
    ArchitectReviewAmended {
        trajectory_id: String,
        total_amendments: usize,
    },
    /// A `FreshStart` strategy is resetting context with carry-forward data.
    FreshStartInitiated {
        trajectory_id: String,
        carry_forward_hints: usize,
        observation_count: usize,
    },
    /// A `RevertAndBranch` strategy is reverting to a target observation.
    RevertAndBranchInitiated {
        trajectory_id: String,
        target: String,
    },
    /// `RevertAndBranch` could not locate its target observation.
    RevertAndBranchTargetMissing {
        trajectory_id: String,
        target: String,
    },

    // -- decide.rs ------------------------------------------------------
    /// A child subtask failed to converge during decomposition coordination.
    DecompositionChildFailed {
        parent_trajectory_id: String,
        child_subtask: String,
    },
    /// The post-decomposition integration trajectory failed to converge.
    DecompositionIntegrationFailed {
        parent_trajectory_id: String,
    },

    // -- resolve.rs -----------------------------------------------------
    /// Persisted bandit state could not be deserialized; defaults used.
    BanditDeserializationFailed {
        error: String,
    },
    /// Bandit memory lookup failed; defaults used.
    BanditQueryFailed {
        error: String,
    },

    // -- run.rs (PR 4b) -------------------------------------------------
    /// Emitted once per iteration of `engine.run()` with the post-iteration
    /// metrics + bookkeeping. Orchestrator sinks translate this to
    /// `EventPayload::ConvergenceIteration`.
    IterationCompleted {
        trajectory_id: uuid::Uuid,
        iteration: u32,
        strategy: String,
        convergence_delta: f64,
        convergence_level: f64,
        attractor_type: String,
        budget_remaining_fraction: f64,
    },
    /// Emitted when the engine detects an attractor-classification change
    /// between two successive iterations. Orchestrator sinks translate this
    /// to `EventPayload::ConvergenceAttractorTransition`.
    AttractorTransitionChanged {
        trajectory_id: uuid::Uuid,
        from: String,
        to: String,
        confidence: f64,
    },
}

impl ConvergenceDomainEvent {
    /// Helper so `TracingEventSink` can render an `AttractorType`-derived
    /// transition where the engine uses a static short name.
    pub fn attractor_name(attractor: &AttractorType) -> &'static str {
        match attractor {
            AttractorType::FixedPoint { .. } => "fixed_point",
            AttractorType::LimitCycle { .. } => "limit_cycle",
            AttractorType::Divergent { .. } => "divergent",
            AttractorType::Plateau { .. } => "plateau",
            AttractorType::Indeterminate { .. } => "indeterminate",
        }
    }
}

// ---------------------------------------------------------------------------
// ConvergenceEventSink
// ---------------------------------------------------------------------------

/// Port the engine uses to emit [`ConvergenceDomainEvent`]s. Implementors
/// receive every domain-relevant observability signal the engine produces.
#[async_trait]
pub trait ConvergenceEventSink: Send + Sync {
    async fn emit(&self, event: ConvergenceDomainEvent);
}

// ---------------------------------------------------------------------------
// TracingEventSink -- preserves pre-port tracing output
// ---------------------------------------------------------------------------

/// Default sink that forwards every [`ConvergenceDomainEvent`] to
/// `tracing::{info,warn}` with the same structured fields and message the
/// engine emitted before this port existed. Wiring this sink is
/// behaviour-preserving.
pub struct TracingEventSink;

#[async_trait]
impl ConvergenceEventSink for TracingEventSink {
    async fn emit(&self, event: ConvergenceDomainEvent) {
        match event {
            ConvergenceDomainEvent::AcceptanceTestsGenerated { count } => {
                tracing::info!("Generated {} acceptance tests", count);
            }
            ConvergenceDomainEvent::BudgetCriticalTerminating { trajectory_id } => {
                tracing::warn!(
                    trajectory_id = %trajectory_id,
                    "Global budget critical — terminating convergence early",
                );
            }
            ConvergenceDomainEvent::StrategyRotationTriggered {
                strategy,
                consecutive_uses,
            } => {
                tracing::info!(
                    strategy = strategy,
                    consecutive_uses = consecutive_uses,
                    "Strategy rotation triggered: filtering out {}",
                    strategy
                );
            }
            ConvergenceDomainEvent::AttractorTransition {
                trajectory_id,
                from,
                to,
            } => {
                tracing::info!(
                    trajectory_id = %trajectory_id,
                    from = from,
                    to = to,
                    "AttractorTransition intervention point: attractor changed from {} to {}",
                    from,
                    to
                );
            }
            ConvergenceDomainEvent::StrategyExecutionStarted {
                strategy,
                trajectory_id,
            } => {
                tracing::info!(
                    strategy = strategy,
                    trajectory_id = %trajectory_id,
                    "Executing convergence strategy"
                );
            }
            ConvergenceDomainEvent::ArchitectReviewAmended {
                trajectory_id,
                total_amendments,
            } => {
                tracing::info!(
                    trajectory_id = %trajectory_id,
                    "ArchitectReview: specification amended, {} total amendments",
                    total_amendments,
                );
            }
            ConvergenceDomainEvent::FreshStartInitiated {
                trajectory_id,
                carry_forward_hints,
                observation_count,
            } => {
                tracing::info!(
                    trajectory_id = %trajectory_id,
                    "Fresh start: carrying forward {} hints, best level \
                     from {} observations",
                    carry_forward_hints,
                    observation_count,
                );
            }
            ConvergenceDomainEvent::RevertAndBranchInitiated {
                trajectory_id,
                target,
            } => {
                tracing::info!(
                    trajectory_id = %trajectory_id,
                    target = %target,
                    "Reverting to observation {} and branching",
                    target,
                );
            }
            ConvergenceDomainEvent::RevertAndBranchTargetMissing {
                trajectory_id,
                target,
            } => {
                tracing::warn!(
                    trajectory_id = %trajectory_id,
                    target = %target,
                    "RevertAndBranch target observation not found; \
                     using latest artifact",
                );
            }
            ConvergenceDomainEvent::DecompositionChildFailed {
                parent_trajectory_id,
                child_subtask,
            } => {
                tracing::warn!(
                    parent_trajectory_id = %parent_trajectory_id,
                    child_subtask = %child_subtask,
                    "Decomposition: child subtask did not converge, aborting coordination",
                );
            }
            ConvergenceDomainEvent::DecompositionIntegrationFailed {
                parent_trajectory_id,
            } => {
                tracing::warn!(
                    parent_trajectory_id = %parent_trajectory_id,
                    "Decomposition: integration trajectory did not converge",
                );
            }
            ConvergenceDomainEvent::BanditDeserializationFailed { error } => {
                tracing::warn!(
                    "Failed to deserialize bandit state: {}; using defaults",
                    error
                );
            }
            ConvergenceDomainEvent::BanditQueryFailed { error } => {
                tracing::warn!("Failed to query bandit memory: {}; using defaults", error);
            }
            ConvergenceDomainEvent::IterationCompleted {
                trajectory_id,
                iteration,
                strategy,
                convergence_delta,
                convergence_level,
                attractor_type,
                budget_remaining_fraction,
            } => {
                tracing::info!(
                    trajectory_id = %trajectory_id,
                    iteration = iteration,
                    strategy = %strategy,
                    convergence_delta = convergence_delta,
                    convergence_level = convergence_level,
                    attractor_type = %attractor_type,
                    budget_remaining_fraction = budget_remaining_fraction,
                    "Convergence iteration completed",
                );
            }
            ConvergenceDomainEvent::AttractorTransitionChanged {
                trajectory_id,
                from,
                to,
                confidence,
            } => {
                tracing::info!(
                    trajectory_id = %trajectory_id,
                    from = %from,
                    to = %to,
                    confidence = confidence,
                    "Attractor classification changed",
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------
// NullEventSink -- no-op for tests
// ---------------------------------------------------------------------------

/// No-op sink for tests that don't care about domain events.
pub struct NullEventSink;

#[async_trait]
impl ConvergenceEventSink for NullEventSink {
    async fn emit(&self, _event: ConvergenceDomainEvent) {}
}

// ---------------------------------------------------------------------------
// StrategyExecutor
// ---------------------------------------------------------------------------

/// Context assembled by the engine for a single strategy execution.
///
/// A `StrategyExecutor` implementation receives this context and must return
/// a [`StrategyExecutionOutput`] describing the produced artifact and the
/// resources consumed. The context is borrowed for the duration of the call
/// so the engine retains ownership of the trajectory and strategy state.
pub struct StrategyExecutionContext<'a> {
    pub trajectory: &'a Trajectory,
    pub strategy: &'a StrategyKind,
    pub strategy_context: &'a StrategyContext,
    pub iteration_seq: u32,
    /// Prompt to send to the substrate for this iteration.
    ///
    /// The orchestrator currently builds the convergent prompt itself (using
    /// task context, latest intent-verification feedback, and other outer-loop
    /// state that is not yet modeled inside the trajectory). Passing the
    /// prompt through the context keeps the executor pure with respect to
    /// that state until PR 4 can push prompt construction into the engine.
    pub prompt: &'a str,
}

/// Output of a single [`StrategyExecutor::execute`] call.
///
/// Captures the produced artifact plus the cost information the engine needs
/// to record an [`Observation`](crate::domain::models::convergence::Observation).
#[derive(Debug, Clone)]
pub struct StrategyExecutionOutput {
    pub artifact: ArtifactReference,
    pub tokens_used: u64,
    pub wall_time_ms: u64,
}

/// Port the engine uses to invoke a substrate / agent runtime.
///
/// The engine itself does not depend on any particular substrate --
/// implementations (e.g. `OrchestratorStrategyExecutor` in the swarm
/// orchestrator) wrap the concrete substrate call and artifact-collection
/// logic. [`ConvergenceEngine::run`] dispatches every strategy execution
/// through this port.
#[async_trait]
pub trait StrategyExecutor: Send + Sync {
    async fn execute(
        &self,
        ctx: &StrategyExecutionContext<'_>,
    ) -> DomainResult<StrategyExecutionOutput>;
}

// ---------------------------------------------------------------------------
// StrategyEffects
// ---------------------------------------------------------------------------

/// Port the engine uses to request side-effectful strategy work.
///
/// Strategies like `FreshStart` and `RevertAndBranch` have side effects that
/// touch the filesystem (worktree reset) or external event buses (fresh start
/// notifications). The engine itself has no notion of worktrees or event
/// payloads, so it delegates these effects to an implementation provided by
/// the orchestrator (see `OrchestratorStrategyEffects`).
/// [`ConvergenceEngine::run`] calls `on_fresh_start` / `on_revert` at the
/// top of each iteration whose selected strategy carries those variants.
#[async_trait]
pub trait StrategyEffects: Send + Sync {
    /// Invoked when a `FreshStart` strategy is selected. Implementations
    /// typically reset the trajectory's worktree to the base branch state
    /// and emit a `ConvergenceFreshStart` event.
    async fn on_fresh_start(&self, trajectory: &Trajectory) -> DomainResult<()>;

    /// Invoked when a `RevertAndBranch` strategy is selected. Implementations
    /// typically roll the worktree back to the filesystem state associated
    /// with the target observation.
    async fn on_revert(&self, trajectory: &Trajectory, target: &Uuid) -> DomainResult<()>;
}

// ---------------------------------------------------------------------------
// ConvergenceAdvisor
// ---------------------------------------------------------------------------

/// Runtime-adjustable policy overlay passed back from a
/// [`ConvergenceAdvisor`]. The engine applies the fields of this overlay to
/// the in-flight `ConvergencePolicy` before continuing. All fields are
/// optional; `None` means "leave the current policy alone".
#[derive(Debug, Clone, Default)]
pub struct PolicyOverlay {
    /// Delta applied to `max_iterations`. Positive values extend; negative
    /// values contract. The engine saturates at 0.
    pub max_iterations_delta: Option<i32>,
    /// Forced budget pressure level. Today this is advisory only — the engine
    /// uses the real [`BudgetTracker`](crate::services::budget_tracker::BudgetTracker)
    /// for termination decisions, but an advisor can surface higher pressure
    /// to trigger earlier termination via `on_iteration_start`.
    pub budget_pressure: Option<BudgetPressureLevel>,
}

/// Gate returned from [`ConvergenceAdvisor::on_iteration_start`]. Controls
/// whether the engine proceeds with the next iteration.
#[derive(Debug, Clone)]
pub enum IterationGate {
    /// Proceed with the iteration unchanged.
    Continue,
    /// Cancel the run. The engine finalizes the trajectory as cancelled and
    /// returns [`ConvergenceRunOutcome::Cancelled`].
    Cancel,
    /// Adjust the in-flight policy via the supplied overlay, then continue.
    AdjustPolicy(PolicyOverlay),
}

/// Directive returned from the intent-check / overseer-converged /
/// pre-exhaustion advisor hooks. Covers every outcome the orchestrator's
/// `run_convergent_execution_inner` post-processing currently produces.
#[derive(Debug)]
#[allow(clippy::large_enum_variant)]
pub enum AdvisorDirective {
    /// Finalize the trajectory as converged. Engine maps this to
    /// [`ConvergenceRunOutcome::Converged`].
    FinalizeConverged,
    /// Finalize the trajectory as exhausted with the given human-readable
    /// reason. Engine maps to [`ConvergenceRunOutcome::Exhausted`].
    FinalizeExhausted(String),
    /// Finalize with overseer convergence but intent gaps found. Engine maps
    /// to [`ConvergenceRunOutcome::IntentGapsFound`].
    FinalizeIntentGaps(IntentVerificationResult),
    /// Finalize on partial satisfaction at high confidence. Engine maps to
    /// [`ConvergenceRunOutcome::PartialAccepted`].
    FinalizePartialAccepted,
    /// Finalize accepting the 3-strike indeterminate fallback. Engine maps to
    /// [`ConvergenceRunOutcome::IndeterminateAccepted`].
    FinalizeIndeterminateAccepted,
    /// Finalize as cancelled. Engine maps to
    /// [`ConvergenceRunOutcome::Cancelled`].
    FinalizeCancelled,
    /// Continue iterating, optionally adjusting the policy.
    Continue { policy_overlay: Option<PolicyOverlay> },
}

/// Port used by the engine to delegate intent-verification and
/// finality-gate decisions back to an implementor. The orchestrator's
/// `OrchestratorConvergenceAdvisor` is the production impl; tests can
/// install trivial `Continue`-only advisors.
#[async_trait]
pub trait ConvergenceAdvisor: Send + Sync {
    /// Called at the top of each iteration, before strategy selection. The
    /// advisor may inspect (and amend) the trajectory, check a cancellation
    /// token, apply SLA-pressure hints, and return an [`IterationGate`].
    async fn on_iteration_start(
        &self,
        trajectory: &mut Trajectory,
    ) -> DomainResult<IterationGate>;

    /// Called when the engine's [`LoopControl::IntentCheck`] fires. The
    /// advisor runs its LLM intent verifier (or equivalent) and returns an
    /// [`AdvisorDirective`].
    ///
    /// The trajectory is passed `&mut` so the advisor can apply
    /// specification amendments (e.g. from intent-verification results)
    /// before returning the directive — subsequent iterations will then see
    /// the enriched specification. Advisors that don't need to mutate the
    /// trajectory may simply read it.
    async fn on_intent_check(
        &self,
        trajectory: &mut Trajectory,
        iteration: u32,
    ) -> DomainResult<AdvisorDirective>;

    /// Called when the engine's [`LoopControl::OverseerConverged`] fires.
    async fn on_overseer_converged(
        &self,
        trajectory: &Trajectory,
    ) -> DomainResult<AdvisorDirective>;

    /// Called when the engine's [`LoopControl::Exhausted`] fires. Implementors
    /// typically run a pre-exhaustion intent check; returning
    /// [`AdvisorDirective::Continue`] resumes iteration, otherwise the
    /// directive's finalize semantics apply.
    async fn on_pre_exhaustion(
        &self,
        trajectory: &Trajectory,
    ) -> DomainResult<AdvisorDirective>;
}

// ---------------------------------------------------------------------------
// ConvergenceRunOutcome -- engine.run() return type
// ---------------------------------------------------------------------------

/// Outcome of a full [`ConvergenceEngine::run`] invocation.
///
/// Mirrors the orchestrator's `ConvergentOutcome` variant-for-variant so the
/// orchestrator's wrapper in PR 4b can translate directly.
#[derive(Debug)]
#[allow(clippy::large_enum_variant)]
pub enum ConvergenceRunOutcome {
    /// The trajectory converged normally.
    Converged,
    /// Convergence budget / iterations exhausted; see message for details.
    Exhausted(String),
    /// Overseers confirmed convergence but intent verification found gaps.
    IntentGapsFound(IntentVerificationResult),
    /// Partial satisfaction accepted at high confidence.
    PartialAccepted,
    /// Overseer-strength acceptance after the 3-strike indeterminate fallback.
    IndeterminateAccepted,
    /// Cancellation token fired mid-run.
    Cancelled,
    /// The engine decomposed the trajectory into subtasks; the caller must
    /// coordinate the children.
    Decomposed(Trajectory),
    /// A terminal failure distinct from `Exhausted` -- trapped, budget denied,
    /// etc. Message describes the condition.
    Failed(String),
}

// ---------------------------------------------------------------------------
// PromptBuilder (PR 4b)
// ---------------------------------------------------------------------------

/// Port the engine uses to build the per-iteration substrate prompt.
///
/// The orchestrator's prompt construction depends on outer-loop state that
/// does not live inside the `Trajectory` (the `Task`, the most-recent intent
/// verification result, etc.), so the engine delegates to an implementor.
/// Implementations typically wrap `convergence_bridge::build_convergent_prompt`.
///
/// The engine calls `build(...)` at the top of each iteration (after strategy
/// selection, before strategy execution) and passes the returned string as the
/// `StrategyExecutionContext::prompt` field.
#[async_trait]
pub trait PromptBuilder: Send + Sync {
    async fn build(
        &self,
        trajectory: &Trajectory,
        strategy: &StrategyKind,
        iteration: u32,
    ) -> DomainResult<String>;
}

// ---------------------------------------------------------------------------
// Null port impls -- trivial defaults for engine-only tests
// ---------------------------------------------------------------------------

/// No-op [`StrategyExecutor`]. Produces an `ArtifactReference` derived from the
/// trajectory's latest observation, or a placeholder artifact when the
/// trajectory has no observations yet. Mirrors the pre-port
/// `ConvergenceEngine::execute_strategy` placeholder so tests that previously
/// called the legacy `converge()` entrypoint can migrate to `run_with_ports`
/// without an LLM substrate wired up.
///
/// For `StrategyKind::RevertAndBranch { target }` the executor returns the
/// target observation's artifact when found (matching the legacy revert
/// semantics), otherwise falls through to the latest-artifact behaviour.
///
/// The reported `tokens_used` mirrors `StrategyKind::estimated_cost()` so
/// budget consumption in tests tracks the legacy path closely enough for
/// attractor classification and exhaustion checks to reproduce.
pub struct NullStrategyExecutor;

#[async_trait]
impl StrategyExecutor for NullStrategyExecutor {
    async fn execute(
        &self,
        ctx: &StrategyExecutionContext<'_>,
    ) -> DomainResult<StrategyExecutionOutput> {
        // RevertAndBranch routes to the target observation's artifact when
        // present, preserving legacy revert semantics in tests.
        if let StrategyKind::RevertAndBranch { target } = ctx.strategy
            && let Some(obs) = ctx.trajectory.observations.iter().find(|o| o.id == *target)
        {
            return Ok(StrategyExecutionOutput {
                artifact: obs.artifact.clone(),
                tokens_used: ctx.strategy.estimated_cost(),
                wall_time_ms: 0,
            });
        }

        let artifact = ctx.trajectory.latest_artifact().cloned().unwrap_or_else(|| {
            ArtifactReference::new(
                format!("/worktree/{}/artifact", ctx.trajectory.task_id),
                format!("pending-{}", ctx.trajectory.observations.len()),
            )
        });

        Ok(StrategyExecutionOutput {
            artifact,
            tokens_used: ctx.strategy.estimated_cost(),
            wall_time_ms: 0,
        })
    }
}

/// No-op [`StrategyEffects`]. Used by tests that exercise `run_with_ports`
/// without needing filesystem worktree resets or orchestrator event emissions.
pub struct NullStrategyEffects;

#[async_trait]
impl StrategyEffects for NullStrategyEffects {
    async fn on_fresh_start(&self, _trajectory: &Trajectory) -> DomainResult<()> {
        Ok(())
    }
    async fn on_revert(&self, _trajectory: &Trajectory, _target: &Uuid) -> DomainResult<()> {
        Ok(())
    }
}

/// Trivial [`ConvergenceAdvisor`] that mirrors the policy the legacy
/// `converge()` method implemented inline:
///
/// - `on_iteration_start` -> always [`IterationGate::Continue`].
/// - `on_intent_check` -> [`AdvisorDirective::Continue`] (legacy treated
///   `LoopControl::IntentCheck` as a no-op continue).
/// - `on_overseer_converged` -> [`AdvisorDirective::FinalizeConverged`] (legacy
///   mapped `LoopControl::OverseerConverged` directly to Converged).
/// - `on_pre_exhaustion` -> [`AdvisorDirective::FinalizePartialAccepted`] when
///   the trajectory has partial acceptance enabled and its best observation
///   crosses the policy threshold; otherwise [`AdvisorDirective::Continue`] so
///   the engine's extension retry + Exhausted fall-through fires.
///
/// This makes `NullConvergenceAdvisor` a drop-in replacement for the legacy
/// converge() finality rules in engine-only tests.
pub struct NullConvergenceAdvisor;

#[async_trait]
impl ConvergenceAdvisor for NullConvergenceAdvisor {
    async fn on_iteration_start(
        &self,
        _trajectory: &mut Trajectory,
    ) -> DomainResult<IterationGate> {
        Ok(IterationGate::Continue)
    }

    async fn on_intent_check(
        &self,
        _trajectory: &mut Trajectory,
        _iteration: u32,
    ) -> DomainResult<AdvisorDirective> {
        Ok(AdvisorDirective::Continue { policy_overlay: None })
    }

    async fn on_overseer_converged(
        &self,
        _trajectory: &Trajectory,
    ) -> DomainResult<AdvisorDirective> {
        Ok(AdvisorDirective::FinalizeConverged)
    }

    async fn on_pre_exhaustion(
        &self,
        trajectory: &Trajectory,
    ) -> DomainResult<AdvisorDirective> {
        if trajectory.policy.partial_acceptance
            && let Some(best) = trajectory.best_observation()
        {
            let level = best
                .metrics
                .as_ref()
                .map(|m| m.intent_blended_level.unwrap_or(m.convergence_level))
                .unwrap_or(0.0);
            if level >= trajectory.policy.partial_threshold {
                return Ok(AdvisorDirective::FinalizePartialAccepted);
            }
        }
        Ok(AdvisorDirective::Continue { policy_overlay: None })
    }
}

/// No-op [`PromptBuilder`] that returns an empty string. Tests that don't care
/// about prompt content can install this so the engine's prompt step is a
/// trivial no-op.
pub struct NullPromptBuilder;

#[async_trait]
impl PromptBuilder for NullPromptBuilder {
    async fn build(
        &self,
        _trajectory: &Trajectory,
        _strategy: &StrategyKind,
        _iteration: u32,
    ) -> DomainResult<String> {
        Ok(String::new())
    }
}
