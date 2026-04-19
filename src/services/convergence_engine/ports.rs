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

use crate::domain::models::convergence::AttractorType;

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
