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

use std::sync::{Arc, Mutex};

use async_trait::async_trait;

use crate::domain::errors::DomainResult;
use crate::domain::models::convergence::*;
use crate::domain::ports::{MemoryRepository, TrajectoryRepository};
use crate::services::budget_tracker::BudgetTracker;

mod decide;
mod iterate;
pub mod ports;
mod prepare;
mod resolve;

pub use ports::{
    ConvergenceDomainEvent, ConvergenceEventSink, NullEventSink, StrategyExecutionContext,
    StrategyExecutionOutput, StrategyExecutor, TracingEventSink,
};

#[cfg(test)]
pub(crate) mod test_support;

#[cfg(test)]
mod tests;

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
    pub(super) trajectory_store: Arc<T>,
    pub(super) memory_repository: Arc<M>,
    pub(super) overseer_measurer: Arc<O>,
    pub(super) config: ConvergenceEngineConfig,
    /// Optional global budget tracker for pressure-aware convergence.
    ///
    /// When set, the convergence loop checks global budget pressure at the top
    /// of each iteration and terminates early with `BudgetDenied` if the
    /// pressure level is Critical (>95% consumed).
    pub(super) budget_tracker: Option<Arc<BudgetTracker>>,
    /// Optional cost-window service for quiet-hours scheduling.
    ///
    /// When set, the convergence loop checks whether we are inside a quiet window
    /// at the start of each iteration and terminates early to avoid dispatching work.
    #[allow(dead_code)]
    pub(super) cost_window_service:
        Option<Arc<crate::services::cost_window_service::CostWindowService>>,
    /// Tracks actual token usage per complexity tier for budget calibration.
    pub(super) calibration_tracker: Mutex<BudgetCalibrationTracker>,
    /// Sink for domain-level observability events emitted by the engine.
    ///
    /// Defaults to [`TracingEventSink`] which preserves the pre-port
    /// `tracing::{info,warn}` output verbatim. Tests that want to silence
    /// events can swap in [`NullEventSink`] via [`Self::with_event_sink`].
    pub(super) event_sink: Arc<dyn ConvergenceEventSink>,
    /// Optional substrate-invocation port.
    ///
    /// Staged for PR 4 of the engine-as-core refactor chain (#13/#21): the
    /// field and builder exist in PR 2 so callers can begin wiring a concrete
    /// [`StrategyExecutor`] implementation, but the engine's own
    /// `execute_strategy` placeholder does not yet dispatch through this
    /// field. Today the orchestrator invokes the executor out-of-band.
    #[allow(dead_code)]
    pub(super) executor: Option<Arc<dyn StrategyExecutor>>,
}

impl<T: TrajectoryRepository, M: MemoryRepository, O: OverseerMeasurer> ConvergenceEngine<T, M, O> {
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
            budget_tracker: None,
            cost_window_service: None,
            calibration_tracker: Mutex::new(BudgetCalibrationTracker::default()),
            event_sink: Arc::new(TracingEventSink),
            executor: None,
        }
    }

    /// Override the domain event sink (builder-style).
    ///
    /// Defaults to [`TracingEventSink`] so callers that don't override get
    /// the pre-port `tracing::{info,warn}` output automatically.
    pub fn with_event_sink(mut self, sink: Arc<dyn ConvergenceEventSink>) -> Self {
        self.event_sink = sink;
        self
    }

    /// Attach a [`StrategyExecutor`] (builder-style).
    ///
    /// Staged for PR 4 of the engine-as-core refactor chain (#13/#21): today
    /// the engine stores the executor but its internal loop does not yet
    /// dispatch through it. PR 4 will migrate `execute_strategy` to call
    /// `self.executor.as_ref().unwrap().execute(...)` once every caller has
    /// been updated to install one.
    pub fn with_executor<E: StrategyExecutor + 'static>(mut self, e: Arc<E>) -> Self {
        self.executor = Some(e);
        self
    }

    /// Set the global budget tracker for pressure-aware convergence.
    ///
    /// When configured, the convergence loop will check global budget pressure
    /// at the start of each iteration and terminate early if the pressure level
    /// is Critical.
    pub fn set_budget_tracker(&mut self, tracker: Arc<BudgetTracker>) {
        self.budget_tracker = Some(tracker);
    }

    /// Set the cost-window service for quiet-hours dispatch gating.
    pub fn set_cost_window_service(
        &mut self,
        service: Arc<crate::services::cost_window_service::CostWindowService>,
    ) {
        self.cost_window_service = Some(service);
    }

    /// Returns any calibration alerts where P95 token usage exceeds the
    /// allocated budget by more than 20 % for a complexity tier.
    pub fn calibration_alerts(&self) -> Vec<CalibrationAlert> {
        self.calibration_tracker
            .lock()
            .map(|t| t.calibration_alerts())
            .unwrap_or_default()
    }

    // -----------------------------------------------------------------------
    // Internal helpers (shared across phase files)
    // -----------------------------------------------------------------------

    /// Emit a convergence event if event emission is enabled.
    pub(super) fn emit_event(&self, event: ConvergenceEvent) {
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
    pub(super) fn attractor_type_name(&self, attractor: &AttractorType) -> &'static str {
        match attractor {
            AttractorType::FixedPoint { .. } => "fixed_point",
            AttractorType::LimitCycle { .. } => "limit_cycle",
            AttractorType::Divergent { .. } => "divergent",
            AttractorType::Plateau { .. } => "plateau",
            AttractorType::Indeterminate { .. } => "indeterminate",
        }
    }
}
