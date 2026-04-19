//! Tests for `convergence_engine::decide`.

use uuid::Uuid;

use crate::domain::models::convergence::*;

use super::{test_budget, test_engine, test_policy};

// -----------------------------------------------------------------------
// maybe_decompose_proactively tests
// -----------------------------------------------------------------------

#[tokio::test]
async fn test_maybe_decompose_proactively_wide_basin_returns_none() {
    // Wide basin: content >= 20 words, success_criteria + constraints + anti_patterns populated
    // Score: 0.5 + 0.15 (criteria) + 0.10 (constraints) + 0.05 (anti_patterns) = 0.80 → Wide
    let engine = test_engine();
    let mut spec_snap = SpecificationSnapshot::new(
        "Implement a REST API endpoint for user authentication with proper \
             error handling validation middleware logging and comprehensive tests \
             covering all edge cases including rate limiting and token expiration"
            .to_string(),
    );
    spec_snap.success_criteria = vec!["All tests pass".to_string()];
    spec_snap.constraints = vec!["Must use async/await".to_string()];
    spec_snap.anti_patterns = vec!["No unwrap in production code".to_string()];

    let spec = SpecificationEvolution::new(spec_snap);
    let mut trajectory = Trajectory::new(Uuid::new_v4(), None, spec, test_budget(), test_policy());

    let result = engine.maybe_decompose_proactively(&mut trajectory).await;
    assert!(result.is_ok());
    assert!(
        result.unwrap().is_none(),
        "Wide basin should not trigger proactive decomposition"
    );
}

#[tokio::test]
async fn test_maybe_decompose_proactively_narrow_basin_low_probability_decomposes() {
    // Narrow basin: short content (< 20 words), no criteria/constraints/anti_patterns
    // Score: 0.5 - 0.15 (short) = 0.35 → Narrow
    // convergence_probability = 0.35 < 0.4 → triggers decomposition
    let engine = test_engine();
    let spec_snap = SpecificationSnapshot::new("fix the bug".to_string());
    let spec = SpecificationEvolution::new(spec_snap);

    // Use a small budget so decomposition is triggered on both conditions
    let mut budget = test_budget();
    budget.max_tokens = 5_000;

    let mut trajectory = Trajectory::new(Uuid::new_v4(), None, spec, budget, test_policy());

    let result = engine.maybe_decompose_proactively(&mut trajectory).await;
    assert!(result.is_ok());
    assert!(
        result.unwrap().is_some(),
        "Narrow basin with low convergence probability should trigger decomposition"
    );
}

#[tokio::test]
async fn test_maybe_decompose_proactively_narrow_basin_sufficient_budget_returns_none() {
    // Narrow basin at the boundary: score = 0.40 exactly
    // Content < 20 words (-0.15) but anti_patterns populated (+0.05) → 0.5 - 0.15 + 0.05 = 0.40
    // classification: score <= 0.4 → Narrow
    // convergence_probability = 0.40, which is NOT < 0.4
    // expected_tokens = (9.0 / 0.40) * 30_000 = 675_000
    // max_tokens = 1_000_000 > 675_000 → budget sufficient
    // Both conditions false → returns Ok(None)
    let engine = test_engine();
    let mut spec_snap = SpecificationSnapshot::new("fix the bug".to_string());
    spec_snap.anti_patterns = vec!["No panics".to_string()];

    let spec = SpecificationEvolution::new(spec_snap);

    let mut budget = test_budget();
    budget.max_tokens = 1_000_000;

    let mut trajectory = Trajectory::new(Uuid::new_v4(), None, spec, budget, test_policy());

    let result = engine.maybe_decompose_proactively(&mut trajectory).await;
    assert!(result.is_ok());
    assert!(
        result.unwrap().is_none(),
        "Narrow basin with sufficient budget and borderline probability should not decompose"
    );
}
