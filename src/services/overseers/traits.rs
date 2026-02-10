//! Core overseer service traits and supporting types.
//!
//! This module provides the [`OverseerMeasurement`] type that wraps an
//! [`OverseerResult`] with timing information. The actual [`Overseer`] trait
//! and [`OverseerCost`] enum live in the domain layer
//! (`domain::models::convergence::overseer`) -- this module re-exports them
//! for convenience and adds service-layer concerns like timing.

use crate::domain::models::convergence::{OverseerResult, OverseerSignals};

// ---------------------------------------------------------------------------
// OverseerMeasurement
// ---------------------------------------------------------------------------

/// The result of a single overseer execution with timing metadata.
///
/// Wraps the domain-layer [`OverseerResult`] with service-level concerns:
/// the overseer's name and wall-clock duration. This is used by the
/// [`OverseerCluster`](super::cluster::OverseerClusterService) to collect
/// and report individual overseer results alongside their performance data.
#[derive(Debug, Clone)]
pub struct OverseerMeasurement {
    /// Human-readable name of the overseer that produced this measurement.
    pub overseer_name: String,
    /// The domain-layer result (pass/fail verdict + signal update).
    pub result: OverseerResult,
    /// Wall-clock duration of the overseer execution in milliseconds.
    pub duration_ms: u64,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Check whether an [`OverseerSignals`] contains blocking failures.
///
/// A blocking failure is a build failure or type-check failure. When cheap
/// overseers produce blocking failures, there is no point running more
/// expensive overseers (spec 2.3).
pub fn has_blocking_failures(signals: &OverseerSignals) -> bool {
    let build_failed = signals
        .build_result
        .as_ref()
        .map(|b| !b.success)
        .unwrap_or(false);

    let type_check_failed = signals
        .type_check
        .as_ref()
        .map(|t| !t.clean)
        .unwrap_or(false);

    build_failed || type_check_failed
}

/// Apply an [`OverseerResult`]'s signal update to an [`OverseerSignals`].
pub fn apply_signal_update(
    signals: &mut OverseerSignals,
    update: crate::domain::models::convergence::OverseerSignalUpdate,
) {
    use crate::domain::models::convergence::OverseerSignalUpdate;
    match update {
        OverseerSignalUpdate::TestResults(r) => signals.test_results = Some(r),
        OverseerSignalUpdate::TypeCheck(r) => signals.type_check = Some(r),
        OverseerSignalUpdate::LintResults(r) => signals.lint_results = Some(r),
        OverseerSignalUpdate::BuildResult(r) => signals.build_result = Some(r),
        OverseerSignalUpdate::SecurityScan(r) => signals.security_scan = Some(r),
        OverseerSignalUpdate::CustomCheck(r) => signals.custom_checks.push(r),
    }
}

// ---------------------------------------------------------------------------
// Re-exports for convenience
// ---------------------------------------------------------------------------

pub use crate::domain::models::convergence::{Overseer, OverseerCluster, OverseerSignalUpdate};
