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

use crate::domain::errors::DomainResult;
use crate::domain::models::convergence::{
    ArtifactReference, AttractorType, StrategyKind, Trajectory,
};

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
    /// Parallel convergence path dropped a trajectory that became divergent.
    ParallelDivergentFiltered {
        trajectory_id: String,
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
            ConvergenceDomainEvent::ParallelDivergentFiltered { trajectory_id } => {
                tracing::info!(
                    trajectory_id = %trajectory_id,
                    "Parallel convergence: filtering out divergent \
                     trajectory",
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
/// logic. This trait is staged for PR 4 of the engine-as-core refactor
/// chain (#13/#21): PR 2 introduces the port and wires an optional executor
/// field onto `ConvergenceEngine`, but the engine's internal `execute_strategy`
/// placeholder is not yet migrated to call it.
#[async_trait]
pub trait StrategyExecutor: Send + Sync {
    async fn execute(
        &self,
        ctx: &StrategyExecutionContext<'_>,
    ) -> DomainResult<StrategyExecutionOutput>;
}
