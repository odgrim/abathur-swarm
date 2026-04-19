//! Consolidated tests for the convergence engine, split per sibling module.
//!
//! The submodules mirror the phase layout of the source:
//! `prepare`, `decide`, `iterate`, `resolve`, and `integration` for
//! cross-cutting tests. Shared fixtures live in this module.

use uuid::Uuid;

use crate::domain::models::convergence::*;

use super::ConvergenceEngine;
use super::test_support::*;

pub mod decide;
pub mod integration;
pub mod iterate;
pub mod prepare;
pub mod resolve;

pub(super) fn test_engine() -> ConvergenceEngine<MockTrajectoryRepo, MockMemoryRepo, MockOverseerMeasurer> {
    build_test_engine()
}

pub(super) fn test_spec() -> SpecificationEvolution {
    SpecificationEvolution::new(SpecificationSnapshot::new(
        "Implement a REST API endpoint for user authentication".to_string(),
    ))
}

pub(super) fn test_budget() -> ConvergenceBudget {
    ConvergenceBudget::default()
}

pub(super) fn test_policy() -> ConvergencePolicy {
    ConvergencePolicy::default()
}

pub(super) fn test_trajectory() -> Trajectory {
    Trajectory::new(
        Uuid::new_v4(),
        None,
        test_spec(),
        test_budget(),
        test_policy(),
    )
}

pub(super) fn test_artifact(seq: u32) -> ArtifactReference {
    ArtifactReference::new(
        format!("/worktree/task/artifact_{}.rs", seq),
        format!("hash_{}", seq),
    )
}

pub(super) fn test_observation(seq: u32, strategy: StrategyKind) -> Observation {
    Observation::new(
        seq,
        test_artifact(seq),
        OverseerSignals::default(),
        strategy,
        10_000,
        5_000,
    )
}

pub(super) fn metrics_with(delta: f64, level: f64) -> ObservationMetrics {
    ObservationMetrics {
        convergence_delta: delta,
        convergence_level: level,
        ..ObservationMetrics::default()
    }
}

pub(super) fn signals_with_tests(passed: u32, total: u32) -> OverseerSignals {
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
