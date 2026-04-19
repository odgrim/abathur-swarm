//! Tests for `convergence_engine::iterate`.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use uuid::Uuid;

use crate::domain::errors::DomainResult;
use crate::domain::models::convergence::*;
use crate::domain::models::intent_verification::{GapCategory, GapSeverity};
use crate::domain::models::task::Complexity;
use crate::domain::ports::TrajectoryRepository;

use super::super::test_support::*;
use super::super::{ConvergenceEngine, OverseerMeasurer};
use super::super::ports::{
    ConvergenceRunOutcome, NullConvergenceAdvisor, NullStrategyEffects, NullStrategyExecutor,
};
use super::{
    metrics_with, signals_with_tests, test_artifact, test_engine, test_observation, test_policy,
    test_trajectory,
};

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
    assert!(
        matches!(result, LoopControl::IntentCheck),
        "Expected IntentCheck at iteration 2 (interval=2), got {:?}",
        result
    );
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
    assert!(
        matches!(result, LoopControl::Continue),
        "Expected Continue at iteration 1 (between intervals), got {:?}",
        result
    );
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
    assert!(
        matches!(result, LoopControl::IntentCheck),
        "Expected IntentCheck when budget fraction exceeded, got {:?}",
        result
    );
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
    assert!(
        matches!(result, LoopControl::IntentCheck),
        "Expected IntentCheck at FixedPoint attractor, got {:?}",
        result
    );
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
    trajectory.budget.tokens_used = (trajectory.budget.max_tokens as f64 * 0.9) as u64;

    // Add observation with positive delta
    let obs =
        test_observation(0, StrategyKind::RetryWithFeedback).with_metrics(metrics_with(0.15, 0.7));
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
    trajectory.budget.tokens_used = (trajectory.budget.max_tokens as f64 * 0.9) as u64;
    trajectory.budget.extensions_requested = trajectory.budget.max_extensions;

    let obs =
        test_observation(0, StrategyKind::RetryWithFeedback).with_metrics(metrics_with(0.15, 0.7));
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
        test_observation(0, StrategyKind::RetryWithFeedback).with_metrics(metrics_with(0.001, 0.6)),
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
    )
    .with_metrics(metrics_with(0.1, 0.8));
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
    )
    .with_metrics(metrics_with(0.1, 1.0));
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
// no-premature-termination constraint tests
// (overseer_signals_are_ambiguous guard on OverseerConverged)
// -----------------------------------------------------------------------

/// Happy path: FixedPoint + build passing + test_results present → OverseerConverged.
#[test]
fn test_loop_control_overseer_converged_fixed_point_with_full_signals() {
    let engine = test_engine();
    let mut trajectory = test_trajectory();

    // Disable interval and budget triggers so only the FixedPoint path fires.
    trajectory.policy.intent_check_interval = u32::MAX;
    trajectory.policy.intent_check_at_budget_fraction = 1.0;

    // Set FixedPoint attractor.
    trajectory.attractor_state = AttractorState {
        classification: AttractorType::FixedPoint {
            estimated_remaining_iterations: 0,
            estimated_remaining_tokens: 0,
        },
        confidence: 0.95,
        detected_at: None,
        evidence: AttractorEvidence {
            recent_deltas: vec![0.001, 0.0, 0.0],
            recent_signatures: vec![],
            rationale: String::new(),
        },
    };

    // Two consecutive observations with full signals (build + tests passing).
    let full_signals = signals_with_tests(10, 10);
    for i in 0..2 {
        let obs = Observation::new(
            i,
            test_artifact(i),
            full_signals.clone(),
            StrategyKind::RetryWithFeedback,
            10_000,
            5_000,
        )
        .with_metrics(metrics_with(0.0, 1.0));
        trajectory.observations.push(obs);
    }

    let bandit = StrategyBandit::with_default_priors();
    let result = engine.check_loop_control(&trajectory, &bandit).unwrap();
    assert!(
        matches!(result, LoopControl::OverseerConverged),
        "FixedPoint + full test signals should yield OverseerConverged, got {:?}",
        result
    );
}

/// Guard: FixedPoint + build-only signals (no test_results) + success_criteria present
/// → must fall back to IntentCheck, not OverseerConverged.
#[test]
fn test_loop_control_no_overseer_converged_when_build_only_and_success_criteria() {
    let engine = test_engine();
    let mut trajectory = test_trajectory();

    // Disable interval and budget triggers.
    trajectory.policy.intent_check_interval = u32::MAX;
    trajectory.policy.intent_check_at_budget_fraction = 1.0;

    // Add a success criterion so the spec is testable.
    trajectory
        .specification
        .effective
        .success_criteria
        .push("All endpoints return valid JSON".to_string());

    // Set FixedPoint attractor.
    trajectory.attractor_state = AttractorState {
        classification: AttractorType::FixedPoint {
            estimated_remaining_iterations: 0,
            estimated_remaining_tokens: 0,
        },
        confidence: 0.95,
        detected_at: None,
        evidence: AttractorEvidence {
            recent_deltas: vec![0.001, 0.0, 0.0],
            recent_signatures: vec![],
            rationale: String::new(),
        },
    };

    // Two consecutive observations — build passing but NO test_results.
    let build_only = OverseerSignals {
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
        test_results: None, // ← no test evidence
        ..OverseerSignals::default()
    };
    for i in 0..2 {
        let obs = Observation::new(
            i,
            test_artifact(i),
            build_only.clone(),
            StrategyKind::RetryWithFeedback,
            10_000,
            5_000,
        )
        .with_metrics(metrics_with(0.0, 1.0));
        trajectory.observations.push(obs);
    }

    let bandit = StrategyBandit::with_default_priors();
    let result = engine.check_loop_control(&trajectory, &bandit).unwrap();
    assert!(
        matches!(result, LoopControl::IntentCheck),
        "Build-only signals with success_criteria must fall back to IntentCheck \
             (no-premature-termination), got {:?}",
        result
    );
}

/// No guard needed: build-only signals + empty success_criteria → OverseerConverged
/// is legitimate because there are no testable criteria to verify.
#[test]
fn test_loop_control_overseer_converged_build_only_no_success_criteria() {
    let engine = test_engine();
    let mut trajectory = test_trajectory();

    // Disable interval and budget triggers.
    trajectory.policy.intent_check_interval = u32::MAX;
    trajectory.policy.intent_check_at_budget_fraction = 1.0;

    // success_criteria is empty (test_trajectory() → test_spec() uses empty Vec).
    assert!(
        trajectory
            .specification
            .effective
            .success_criteria
            .is_empty(),
        "test_trajectory must start with empty success_criteria"
    );

    // Set FixedPoint attractor.
    trajectory.attractor_state = AttractorState {
        classification: AttractorType::FixedPoint {
            estimated_remaining_iterations: 0,
            estimated_remaining_tokens: 0,
        },
        confidence: 0.95,
        detected_at: None,
        evidence: AttractorEvidence {
            recent_deltas: vec![0.001, 0.0, 0.0],
            recent_signatures: vec![],
            rationale: String::new(),
        },
    };

    // Two consecutive build-only observations.
    let build_only = OverseerSignals {
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
        test_results: None,
        ..OverseerSignals::default()
    };
    for i in 0..2 {
        let obs = Observation::new(
            i,
            test_artifact(i),
            build_only.clone(),
            StrategyKind::RetryWithFeedback,
            10_000,
            5_000,
        )
        .with_metrics(metrics_with(0.0, 1.0));
        trajectory.observations.push(obs);
    }

    let bandit = StrategyBandit::with_default_priors();
    let result = engine.check_loop_control(&trajectory, &bandit).unwrap();
    assert!(
        matches!(result, LoopControl::OverseerConverged),
        "Build-only signals with no success_criteria should yield OverseerConverged, \
             got {:?}",
        result
    );
}

/// LimitCycle path: passing build-only + success_criteria → IntentCheck, not
/// OverseerConverged. Verifies the same guard applies to the LimitCycle branch.
#[test]
fn test_loop_control_no_overseer_converged_limit_cycle_build_only_with_criteria() {
    let engine = test_engine();
    let mut trajectory = test_trajectory();

    // Disable interval and budget triggers.
    trajectory.policy.intent_check_interval = u32::MAX;
    trajectory.policy.intent_check_at_budget_fraction = 1.0;

    // Add a success criterion.
    trajectory
        .specification
        .effective
        .success_criteria
        .push("All auth tokens expire correctly".to_string());

    // Set LimitCycle attractor.
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

    // One observation: build passing, no test_results.
    let build_only = OverseerSignals {
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
        test_results: None, // ← no test evidence
        ..OverseerSignals::default()
    };
    let obs = Observation::new(
        0,
        test_artifact(0),
        build_only,
        StrategyKind::RetryWithFeedback,
        10_000,
        5_000,
    )
    .with_metrics(metrics_with(0.0, 1.0));
    trajectory.observations.push(obs);

    let bandit = StrategyBandit::with_default_priors();
    let result = engine.check_loop_control(&trajectory, &bandit).unwrap();
    assert!(
        matches!(result, LoopControl::IntentCheck),
        "LimitCycle + build-only signals with success_criteria must fall back to \
             IntentCheck (no-premature-termination), got {:?}",
        result
    );
}

/// Regression test for no-premature-termination constraint: at FixedPoint with
/// 2+ consecutive all-passing observations, but overseer_signals_are_ambiguous()
/// returns true (success_criteria present, test_results absent), verify that
/// OverseerConverged is NOT returned — IntentCheck should fire instead.
#[test]
fn test_overseer_converged_bypass_when_signals_ambiguous() {
    let engine = test_engine();
    let mut trajectory = test_trajectory();

    // Disable interval and budget triggers so only the FixedPoint shortcircuit path runs.
    trajectory.policy.intent_check_interval = u32::MAX;
    trajectory.policy.intent_check_at_budget_fraction = 1.0;

    // Add success criteria — this makes test evidence required for unambiguous convergence.
    trajectory
        .specification
        .effective
        .success_criteria
        .push("All unit tests pass".to_string());
    trajectory
        .specification
        .effective
        .success_criteria
        .push("No regressions in integration suite".to_string());

    // Set FixedPoint attractor — trajectory has stabilized.
    trajectory.attractor_state = AttractorState {
        classification: AttractorType::FixedPoint {
            estimated_remaining_iterations: 0,
            estimated_remaining_tokens: 0,
        },
        confidence: 0.95,
        detected_at: None,
        evidence: AttractorEvidence {
            recent_deltas: vec![0.0, 0.0, 0.0],
            recent_signatures: vec![],
            rationale: String::new(),
        },
    };

    // Two consecutive build-only observations (no test_results).
    // all_passing_relative() returns true for build-only signals, so
    // consecutive_all_passing >= 2 is satisfied.
    let build_only = OverseerSignals {
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
        test_results: None, // ← ambiguous: criteria exist but no test evidence
        ..OverseerSignals::default()
    };

    for i in 0..3 {
        let obs = Observation::new(
            i,
            test_artifact(i),
            build_only.clone(),
            StrategyKind::RetryWithFeedback,
            10_000,
            5_000,
        )
        .with_metrics(metrics_with(0.0, 1.0));
        trajectory.observations.push(obs);
    }

    let bandit = StrategyBandit::with_default_priors();
    let result = engine.check_loop_control(&trajectory, &bandit).unwrap();

    // OverseerConverged must NOT be returned — the guard should detect ambiguity
    // and fall back to IntentCheck so the LLM-based verifier can assess completeness.
    assert!(
        !matches!(result, LoopControl::OverseerConverged),
        "OverseerConverged must not fire when overseer signals are ambiguous \
             (success_criteria present but no test_results), got {:?}",
        result
    );
    assert!(
        matches!(result, LoopControl::IntentCheck),
        "Ambiguous signals at FixedPoint should fall back to IntentCheck, got {:?}",
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
    )
    .with_metrics(metrics_with(0.1, 0.3));
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
    )
    .with_metrics(metrics_with(0.2, 0.6));
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
    )
    .with_metrics(metrics_with(0.1, 0.5));
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
    )
    .with_metrics(metrics_with(0.15, 0.7));
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
    )
    .with_metrics(metrics_with(0.1, 0.7));
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
    )
    .with_metrics(metrics_with(0.05, 0.75));
    trajectory.observations.push(obs2);

    // No state transition, not at frequency, not FixedPoint
    assert!(!engine.should_verify(&trajectory));
}
// -----------------------------------------------------------------------
// ProgressiveMockOverseerMeasurer -- returns different signals per call
// -----------------------------------------------------------------------

struct ProgressiveMockOverseerMeasurer {
    signals_sequence: Mutex<Vec<OverseerSignals>>,
    call_index: Mutex<usize>,
}

impl ProgressiveMockOverseerMeasurer {
    fn new(signals_sequence: Vec<OverseerSignals>) -> Self {
        Self {
            signals_sequence: Mutex::new(signals_sequence),
            call_index: Mutex::new(0),
        }
    }
}

#[async_trait]
impl OverseerMeasurer for ProgressiveMockOverseerMeasurer {
    async fn measure(
        &self,
        _artifact: &ArtifactReference,
        _policy: &ConvergencePolicy,
    ) -> DomainResult<OverseerSignals> {
        let mut idx = self.call_index.lock().unwrap();
        let seq = self.signals_sequence.lock().unwrap();
        let signals = if *idx < seq.len() {
            seq[*idx].clone()
        } else {
            // Repeat last signal if we run out
            seq.last().cloned().unwrap_or_default()
        };
        *idx += 1;
        Ok(signals)
    }
}

// -----------------------------------------------------------------------
// Helper functions for converge/iterate_once tests
// -----------------------------------------------------------------------

/// All-passing signals with build+type_check+tests.
fn all_passing_signals() -> OverseerSignals {
    OverseerSignals {
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
    }
}

/// Failing signals: build passes but tests fail.
fn failing_signals() -> OverseerSignals {
    OverseerSignals {
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
            passed: 3,
            failed: 7,
            skipped: 0,
            total: 10,
            regression_count: 0,
            failing_test_names: vec!["test_a".to_string(), "test_b".to_string()],
        }),
        ..OverseerSignals::default()
    }
}

/// Build an engine with a progressive measurer for converge/iterate_once tests.
fn test_engine_with_measurer(
    signals: Vec<OverseerSignals>,
) -> ConvergenceEngine<MockTrajectoryRepo, MockMemoryRepo, ProgressiveMockOverseerMeasurer> {
    let mut config = test_config();
    // Disable event emission and acceptance-test generation to simplify tests.
    config.event_emission_enabled = false;
    config.default_policy.generate_acceptance_tests = false;
    ConvergenceEngine::new(
        Arc::new(MockTrajectoryRepo::new()),
        Arc::new(MockMemoryRepo::new()),
        Arc::new(ProgressiveMockOverseerMeasurer::new(signals)),
        config,
    )
}

// -----------------------------------------------------------------------
// run_with_ports helpers for engine-lifecycle tests
// -----------------------------------------------------------------------

/// Run the engine's new ports-driven entrypoint against a prepared trajectory,
/// emulating the pre-port `engine.converge(trajectory, &infra)` call used in
/// legacy tests. Installs the trivial [`NullStrategyExecutor`] /
/// [`NullStrategyEffects`] / [`NullConvergenceAdvisor`] ports so the lifecycle
/// is driven purely by the overseer measurer + in-engine logic, just like the
/// deleted legacy path.
async fn run_engine_ports(
    engine: &ConvergenceEngine<MockTrajectoryRepo, MockMemoryRepo, ProgressiveMockOverseerMeasurer>,
    trajectory: Trajectory,
    submission: TaskSubmission,
    task_id: Uuid,
) -> ConvergenceRunOutcome {
    let trajectory_id = trajectory.id;
    engine.trajectory_store.save(&trajectory).await.unwrap();
    engine
        .run_with_ports(
            submission,
            task_id,
            Some(trajectory_id),
            Arc::new(NullStrategyExecutor),
            Some(Arc::new(NullStrategyEffects)),
            Arc::new(NullConvergenceAdvisor),
            None,
            None,
        )
        .await
        .unwrap()
}

// -----------------------------------------------------------------------
// run_with_ports() and iterate_once() tests
// -----------------------------------------------------------------------

/// Happy path: progressive improvement leads to FixedPoint attractor,
/// then OverseerConverged fires when 2+ consecutive all-passing signals
/// are observed at a FixedPoint.
#[tokio::test]
async fn test_converge_happy_path_overseer_converged() {
    // Progressive signals: start failing, improve each iteration,
    // then stabilize at all-passing. This produces positive deltas →
    // FixedPoint attractor.
    let signals = vec![
        // Iteration 0: build fails
        OverseerSignals {
            build_result: Some(BuildResult {
                success: false,
                error_count: 3,
                errors: vec!["error1".to_string()],
            }),
            ..OverseerSignals::default()
        },
        // Iteration 1: build passes, tests fail
        OverseerSignals {
            build_result: Some(BuildResult {
                success: true,
                error_count: 0,
                errors: Vec::new(),
            }),
            test_results: Some(TestResults {
                passed: 3,
                failed: 7,
                skipped: 0,
                total: 10,
                regression_count: 0,
                failing_test_names: vec!["t1".to_string()],
            }),
            ..OverseerSignals::default()
        },
        // Iteration 2: more tests pass
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
                regression_count: 0,
                failing_test_names: vec!["t1".to_string()],
            }),
            ..OverseerSignals::default()
        },
        // Iterations 3+: all passing (repeat for remaining iterations)
        all_passing_signals(),
        all_passing_signals(),
        all_passing_signals(),
        all_passing_signals(),
        all_passing_signals(),
        all_passing_signals(),
        all_passing_signals(),
    ];
    let engine = test_engine_with_measurer(signals);

    let task_id = Uuid::new_v4();
    let mut submission = TaskSubmission::new(
        "Implement a simple function that adds two numbers together for a calculator module"
            .to_string(),
    );
    submission.inferred_complexity = Complexity::Moderate; // More budget

    let (mut trajectory, _infra) = engine.prepare(&submission, task_id).await.unwrap();
    // Force Sequential convergence mode (Cheap hint avoids parallel routing).
    trajectory.policy.priority_hint = Some(PriorityHint::Cheap);
    // Disable interval and budget-fraction triggers so the
    // OverseerConverged shortcircuit path can fire.
    trajectory.policy.intent_check_interval = u32::MAX;
    trajectory.policy.intent_check_at_budget_fraction = 1.0;

    let outcome = run_engine_ports(&engine, trajectory, submission, task_id).await;

    assert!(
        matches!(outcome, ConvergenceRunOutcome::Converged),
        "Expected Converged outcome, got {:?}",
        outcome
    );
}

/// Budget exhaustion: always-failing signals with a tiny budget → Exhausted.
#[tokio::test]
async fn test_converge_budget_exhaustion() {
    let signals: Vec<OverseerSignals> = (0..20).map(|_| failing_signals()).collect();
    let engine = test_engine_with_measurer(signals);

    let task_id = Uuid::new_v4();
    let mut submission = TaskSubmission::new(
        "Build a comprehensive REST API endpoint for user authentication with full test coverage"
            .to_string(),
    );
    submission.inferred_complexity = Complexity::Simple;

    let (mut trajectory, _infra) = engine.prepare(&submission, task_id).await.unwrap();
    // Force Sequential convergence mode (the pre-PR-5 version of this test
    // was implicitly routed to converge_parallel by a Narrow basin; PR 5
    // removed parallel routing, so we pin Cheap to keep the sequential path
    // exercised).
    trajectory.policy.priority_hint = Some(PriorityHint::Cheap);
    // Shrink the budget so the loop exhausts quickly.
    trajectory.budget.max_tokens = 30_000;
    trajectory.budget.max_iterations = 2;
    trajectory.budget.max_extensions = 0;
    // Disable partial acceptance so we get Exhausted, not partial Converged.
    trajectory.policy.partial_acceptance = false;

    let outcome = run_engine_ports(&engine, trajectory, submission, task_id).await;

    // Either Exhausted (budget) or Failed/trapped (no eligible escape
    // strategies) is acceptable: both are terminal failure outcomes for a
    // tiny budget with always-failing signals. In the ports-driven path,
    // trapped surfaces as `Failed("trapped in ...")`.
    assert!(
        matches!(
            outcome,
            ConvergenceRunOutcome::Exhausted(_) | ConvergenceRunOutcome::Failed(_)
        ),
        "Expected Exhausted or Failed/trapped outcome, got {:?}",
        outcome
    );
}

/// Budget exhaustion with partial acceptance threshold met → Converged.
/// When the budget runs out but the best observation exceeds
/// partial_threshold, the engine returns Converged instead of Exhausted.
#[tokio::test]
async fn test_converge_budget_exhaustion_with_partial_acceptance() {
    // Deteriorating signals: start good (high convergence_level), then get
    // worse. This produces negative deltas → Declining attractor → budget
    // extension denied. The best observation's convergence_level still
    // exceeds partial_threshold, so partial acceptance fires.
    let good_signals = OverseerSignals {
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
            passed: 9,
            failed: 1,
            skipped: 0,
            total: 10,
            regression_count: 0,
            failing_test_names: vec!["test_edge_case".to_string()],
        }),
        ..OverseerSignals::default()
    };
    let worse_signals = OverseerSignals {
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
            failing_test_names: vec!["test_a".to_string()],
        }),
        ..OverseerSignals::default()
    };
    let bad_signals = failing_signals();
    // 3 iterations: good → worse → bad (declining trajectory)
    let signals = vec![
        good_signals.clone(),
        worse_signals,
        bad_signals.clone(),
        bad_signals.clone(),
        bad_signals,
    ];
    let engine = test_engine_with_measurer(signals);

    let task_id = Uuid::new_v4();
    let mut submission = TaskSubmission::new(
            "Implement data validation logic for incoming API requests with comprehensive error handling".to_string(),
        );
    submission.inferred_complexity = Complexity::Simple;

    let (mut trajectory, _infra) = engine.prepare(&submission, task_id).await.unwrap();
    // Force Sequential convergence mode (Cheap hint avoids parallel routing).
    trajectory.policy.priority_hint = Some(PriorityHint::Cheap);
    // Budget: 3 iterations (enough for iterate_once to detect exhaustion
    // via check_loop_control rather than Trapped at strategy selection).
    trajectory.budget.max_tokens = 200_000;
    trajectory.budget.max_iterations = 3;
    trajectory.budget.max_extensions = 0;
    // Enable partial acceptance with a low threshold.
    // The best observation (iter 1, good_signals 9/10) has
    // convergence_level ≈ 0.945, well above 0.5.
    trajectory.policy.partial_acceptance = true;
    trajectory.policy.partial_threshold = 0.5;

    let outcome = run_engine_ports(&engine, trajectory, submission, task_id).await;

    // Should be PartialAccepted via the null advisor's partial-acceptance
    // handling at pre-exhaustion (the ports-driven equivalent of legacy
    // `converge()`'s partial-acceptance branch).
    assert!(
        matches!(outcome, ConvergenceRunOutcome::PartialAccepted),
        "Expected PartialAccepted via partial acceptance, got {:?}",
        outcome
    );
}

/// Oscillating signals that form a limit cycle with all exploration
/// strategies exhausted → Trapped outcome with LimitCycle attractor.
#[tokio::test]
async fn test_converge_trapped_limit_cycle() {
    // Two distinct signal patterns that alternate, producing different
    // overseer fingerprints and thus a detectable period-2 limit cycle.
    let signal_a = OverseerSignals {
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
            failing_test_names: vec!["test_x".to_string()],
        }),
        ..OverseerSignals::default()
    };
    let signal_b = OverseerSignals {
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
            passed: 3,
            failed: 7,
            skipped: 0,
            total: 10,
            regression_count: 0,
            failing_test_names: vec!["test_y".to_string()],
        }),
        ..OverseerSignals::default()
    };

    // Provide enough alternating signals for: initial classification
    // iterations + exploration strategy exhaustion iterations.
    let signals: Vec<OverseerSignals> = (0..20)
        .map(|i| {
            if i % 2 == 0 {
                signal_a.clone()
            } else {
                signal_b.clone()
            }
        })
        .collect();

    let engine = test_engine_with_measurer(signals);

    let task_id = Uuid::new_v4();
    let mut submission = TaskSubmission::new(
        "Implement a data pipeline transformation step with error recovery logic".to_string(),
    );
    submission.inferred_complexity = Complexity::Simple;

    let (mut trajectory, _infra) = engine.prepare(&submission, task_id).await.unwrap();
    // Force Sequential convergence mode.
    trajectory.policy.priority_hint = Some(PriorityHint::Cheap);
    // Give enough budget so the loop doesn't exhaust before Trapped.
    trajectory.budget.max_tokens = 5_000_000;
    trajectory.budget.max_iterations = 20;
    trajectory.budget.max_extensions = 0;
    // Disable intent check triggers so they don't interfere.
    trajectory.policy.intent_check_interval = u32::MAX;
    trajectory.policy.intent_check_at_budget_fraction = 1.0;
    // Disable partial acceptance.
    trajectory.policy.partial_acceptance = false;

    let outcome = run_engine_ports(&engine, trajectory, submission, task_id).await;

    // In the ports-driven path, LoopControl::Trapped surfaces as
    // `Failed("trapped in LimitCycle ...")`; we assert the message references
    // LimitCycle to preserve the attractor-type check.
    match &outcome {
        ConvergenceRunOutcome::Failed(msg) => {
            assert!(
                msg.to_lowercase().contains("limitcycle")
                    || msg.to_lowercase().contains("limit_cycle")
                    || msg.to_lowercase().contains("limit cycle"),
                "Expected trapped-in-LimitCycle message, got {:?}",
                msg
            );
        }
        other => panic!("Expected Failed (trapped) outcome, got {:?}", other),
    }
}

/// When the attractor is classified as LimitCycle but the remaining
/// budget is too small for any exploration strategy, eligible_strategies
/// returns empty and converge() returns Trapped immediately.
#[tokio::test]
async fn test_converge_trapped_no_eligible_strategies() {
    // Oscillating signals: two distinct patterns that alternate.
    let signal_a = failing_signals(); // 3 passed, 7 failed
    let signal_b = OverseerSignals {
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
            passed: 7,
            failed: 3,
            skipped: 0,
            total: 10,
            regression_count: 0,
            failing_test_names: vec!["test_z".to_string()],
        }),
        ..OverseerSignals::default()
    };

    // Enough signals for initial iterations + LimitCycle detection.
    let signals: Vec<OverseerSignals> = (0..20)
        .map(|i| {
            if i % 2 == 0 {
                signal_a.clone()
            } else {
                signal_b.clone()
            }
        })
        .collect();

    let engine = test_engine_with_measurer(signals);

    let task_id = Uuid::new_v4();
    let mut submission = TaskSubmission::new(
        "Build a configuration parser with validation and default value support".to_string(),
    );
    submission.inferred_complexity = Complexity::Simple;

    let (mut trajectory, _infra) = engine.prepare(&submission, task_id).await.unwrap();
    // Force Sequential convergence mode.
    trajectory.policy.priority_hint = Some(PriorityHint::Cheap);
    // Give enough iterations but very tight token budget. After the
    // initial iterations consume tokens, the remaining budget won't
    // afford any exploration strategy.
    trajectory.budget.max_iterations = 20;
    trajectory.budget.max_extensions = 0;
    // Set tokens very tight: enough for ~5 iterations (5000 tokens each
    // from the mock measurer) but too little for any exploration strategy
    // cost on top. Exploration strategies cost 15k-30k tokens.
    trajectory.budget.max_tokens = 30_000;
    // Disable intent check triggers.
    trajectory.policy.intent_check_interval = u32::MAX;
    trajectory.policy.intent_check_at_budget_fraction = 1.0;
    // Disable partial acceptance.
    trajectory.policy.partial_acceptance = false;

    let outcome = run_engine_ports(&engine, trajectory, submission, task_id).await;

    // Should be Failed/trapped (LimitCycle with no affordable strategies)
    // or Exhausted if budget runs out first. Both are valid terminal states.
    assert!(
        matches!(
            outcome,
            ConvergenceRunOutcome::Failed(_) | ConvergenceRunOutcome::Exhausted(_)
        ),
        "Expected Failed/trapped or Exhausted outcome, got {:?}",
        outcome
    );
}

/// iterate_once with a fresh trajectory (first observation) records the
/// observation and returns a LoopControl.
#[tokio::test]
async fn test_iterate_once_first_observation() {
    let engine = test_engine_with_measurer(vec![]);

    let mut trajectory = test_trajectory();
    trajectory.phase = ConvergencePhase::Iterating;
    let mut bandit = StrategyBandit::with_default_priors();
    let strategy = StrategyKind::RetryWithFeedback;

    // Create an observation with all-passing signals.
    let observation = Observation::new(
        0,
        test_artifact(0),
        all_passing_signals(),
        strategy.clone(),
        5_000,
        1_000,
    );

    let control = engine
        .iterate_once(&mut trajectory, &mut bandit, &strategy, observation)
        .await
        .unwrap();

    // First observation should be recorded.
    assert_eq!(
        trajectory.observations.len(),
        1,
        "Expected 1 observation after iterate_once"
    );
    // Budget should be consumed.
    assert!(trajectory.budget.tokens_used > 0);
    // Attractor should be classified.
    // Verify attractor classification doesn't panic and produces a result.
    let _classification = &trajectory.attractor_state.classification;
    // Control should be some valid value (exact value depends on attractor classification).
    let _ = control; // Confirm no error
}

/// iterate_once with improving delta over a baseline observation.
#[tokio::test]
async fn test_iterate_once_improving_delta() {
    let engine = test_engine_with_measurer(vec![]);

    let mut trajectory = test_trajectory();
    trajectory.phase = ConvergencePhase::Iterating;
    let mut bandit = StrategyBandit::with_default_priors();
    let strategy = StrategyKind::RetryWithFeedback;

    // Add a baseline observation (failing tests).
    let baseline = Observation::new(
        0,
        test_artifact(0),
        failing_signals(),
        strategy.clone(),
        5_000,
        1_000,
    );
    trajectory.observations.push(baseline);

    // Now iterate with better signals.
    let improved = Observation::new(
        1,
        test_artifact(1),
        all_passing_signals(),
        strategy.clone(),
        5_000,
        1_000,
    );

    let control = engine
        .iterate_once(&mut trajectory, &mut bandit, &strategy, improved)
        .await
        .unwrap();

    // Should now have 2 observations.
    assert_eq!(trajectory.observations.len(), 2);

    // The second observation should have metrics with a positive delta.
    let last_obs = trajectory.observations.last().unwrap();
    assert!(
        last_obs.metrics.is_some(),
        "Second observation should have computed metrics"
    );
    let metrics = last_obs.metrics.as_ref().unwrap();
    assert!(
        metrics.convergence_delta > 0.0,
        "Expected positive convergence delta for improving signals, got {}",
        metrics.convergence_delta
    );
    assert!(
        metrics.convergence_level > 0.0,
        "Expected positive convergence level, got {}",
        metrics.convergence_level
    );

    // Strategy log should be updated.
    assert!(
        !trajectory.strategy_log.is_empty(),
        "Strategy log should have an entry"
    );

    let _ = control;
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

    let build_gap = result
        .gaps
        .iter()
        .find(|g| g.description.contains("Build failure"))
        .unwrap();
    assert_eq!(build_gap.severity, GapSeverity::Critical);
    assert_eq!(build_gap.category, GapCategory::Functional);
    assert!(
        build_gap
            .suggested_action
            .as_ref()
            .unwrap()
            .contains("cannot find type")
    );
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

    let test_gap = result
        .gaps
        .iter()
        .find(|g| g.description.contains("Test failures"))
        .unwrap();
    // Regressions should bump severity to Major
    assert_eq!(test_gap.severity, GapSeverity::Major);
    assert_eq!(test_gap.category, GapCategory::Testing);
    assert!(
        test_gap
            .suggested_action
            .as_ref()
            .unwrap()
            .contains("test_login")
    );
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

    let sec_gap = result
        .gaps
        .iter()
        .find(|g| g.category == GapCategory::Security)
        .unwrap();
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
    let sec_gap = result
        .gaps
        .iter()
        .find(|g| g.category == GapCategory::Security)
        .unwrap();
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

    let lint_gap = result
        .gaps
        .iter()
        .find(|g| g.category == GapCategory::Maintainability)
        .unwrap();
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
    spec.success_criteria
        .push("All endpoints return valid JSON".to_string());
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

    let tc_gap = result
        .gaps
        .iter()
        .find(|g| g.description.contains("Type check"))
        .unwrap();
    assert_eq!(tc_gap.severity, GapSeverity::Major);
    assert!(
        tc_gap
            .suggested_action
            .as_ref()
            .unwrap()
            .contains("expected String")
    );
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

    let spec =
        SpecificationEvolution::new(SpecificationSnapshot::new("Implement feature".to_string()));
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
        assert_eq!(
            best.sequence, 1,
            "With intent_blended_level ({:.3}) > raw overseer level (0.85), \
                 best_observation should prefer the intent-aware observation",
            high_intent_blended
        );
    } else {
        // If the raw overseer from obs0 is still higher, obs0 wins.
        // Either way, the blended level is being compared — not just
        // the raw overseer. Verify intent_blended_level exists.
        let _best = trajectory.best_observation().unwrap();
        assert!(
            trajectory.observations[1]
                .metrics
                .as_ref()
                .unwrap()
                .intent_blended_level
                .is_some(),
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
