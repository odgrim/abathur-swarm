//! Convergence metrics and context health estimation.
//!
//! This module implements the convergence measurement system from Parts 1.4 and 1.5
//! of the attractor-driven convergence specification. Two complementary metrics drive
//! all convergence decisions:
//!
//! - **`convergence_delta`** -- the *derivative* of progress. Measures the change between
//!   consecutive observations. Positive means the trajectory is approaching its attractor;
//!   negative means it is diverging. Drives attractor classification and strategy selection.
//!
//! - **`convergence_level`** -- the *absolute position*. Measures how close the current
//!   implementation state is to "done." Drives termination decisions.
//!
//! Context health tracks the degradation of the LLM's working context over successive
//! iterations. When context becomes noisy, convergence signals become unreliable and the
//! system must trigger a fresh start to restore productive iteration.
//!
//! # Security Invariant
//!
//! Strategies that introduce new vulnerabilities never receive positive convergence credit,
//! regardless of other progress. This trains the strategy bandit to avoid approaches that
//! trade functional progress for security regressions.
//!
//! # Context Degradation Guard
//!
//! Fresh starts are guarded by `total_fresh_starts` to prevent infinite reset loops.
//! Once the maximum is reached, context degradation detection returns `false` and the
//! trajectory must resolve through other means (escalation, partial acceptance, or failure).

use serde::{Deserialize, Serialize};

use super::*;

// ============================================================================
// Observation Metrics
// ============================================================================

/// Per-observation convergence metrics computed from overseer signals.
///
/// These metrics capture both the *derivative* (how much progress was made between
/// consecutive observations) and the *absolute position* (how close to done). The
/// first observation in a trajectory has `metrics: None` because there is no prior
/// observation to compute a delta against.
///
/// All delta values are relative to the previous observation. The composite
/// `convergence_delta` is a weighted combination of individual signal deltas,
/// subject to security vetoes and context degradation penalties.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObservationMetrics {
    /// Number of AST nodes changed between consecutive artifacts.
    /// Measures structural distance -- high values indicate large rewrites.
    pub ast_diff_nodes: u32,

    /// Change in passing test count from previous observation.
    /// Positive means more tests pass; negative means regressions.
    pub test_pass_delta: i32,

    /// Number of tests that previously passed but now fail.
    /// Always non-negative. Regressions are penalized heavily.
    pub test_regression_count: u32,

    /// Change in total error count (build + type check + lint) from previous observation.
    /// Negative values indicate error reduction (progress).
    pub error_count_delta: i32,

    /// Change in vulnerability count (critical + high severity) from previous observation.
    /// Positive values trigger the security veto on convergence delta.
    pub vulnerability_delta: i32,

    /// Composite progress score for this iteration.
    /// Negative means regressing, 0.0 means stalled, positive means converging.
    /// Subject to security veto (capped at 0.0 if vulnerabilities increased) and
    /// context degradation penalty (scaled down when signal-to-noise is low).
    pub convergence_delta: f64,

    /// Absolute convergence position: how close to "done."
    /// Range: 0.0 (nothing works) to 1.0 (fully converged).
    /// Subject to hard gates: build failure caps at 0.3, type failure caps at 0.6.
    pub convergence_level: f64,
}

impl Default for ObservationMetrics {
    /// Returns zeroed-out metrics representing no change and no convergence.
    ///
    /// This is primarily useful in tests and as a structural default for the
    /// `..ObservationMetrics::default()` pattern.
    fn default() -> Self {
        Self {
            ast_diff_nodes: 0,
            test_pass_delta: 0,
            test_regression_count: 0,
            error_count_delta: 0,
            vulnerability_delta: 0,
            convergence_delta: 0.0,
            convergence_level: 0.0,
        }
    }
}

// ============================================================================
// Context Health
// ============================================================================

/// Health metrics for the LLM's working context.
///
/// As iterations progress, the context window fills with iteration history, previous
/// failed attempts, and stale feedback. This degrades the signal-to-noise ratio and
/// causes the LLM to produce confused or repetitive output. ContextHealth tracks
/// three symptoms of degradation and is used to decide when a fresh start is needed.
///
/// Fresh starts discard accumulated context noise and re-initialize with only the
/// essential carry-forward: the effective specification, latest artifact, most recent
/// overseer signals, user hints, and curated convergence memory.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextHealth {
    /// Ratio of useful context (spec + current code + latest signals + hints)
    /// to total context (including old iteration history, failed attempts, stale feedback).
    /// Range: 0.0 (entirely noise) to 1.0 (entirely useful signal).
    /// A fresh context starts at 1.0 and degrades with each iteration.
    pub signal_to_noise: f64,

    /// Average AST diff nodes per iteration over a recent window (typically 3 observations).
    /// High churn with no functional progress indicates context confusion --
    /// the LLM is making large changes each iteration but not converging.
    pub structural_churn_rate: f64,

    /// Similarity between recent artifacts. Range: 0.0 (completely different) to 1.0 (identical).
    /// High self-similarity (> 0.9) with multiple observations suggests the LLM is
    /// re-generating the same code -- a sign of context saturation or prompt fixation.
    pub artifact_self_similarity: f64,
}

impl Default for ContextHealth {
    /// Returns a healthy initial context state.
    ///
    /// A fresh context has perfect signal-to-noise (1.0), no structural churn (0.0),
    /// and no self-similarity (0.0) because there are no prior artifacts to compare.
    fn default() -> Self {
        Self {
            signal_to_noise: 1.0,
            structural_churn_rate: 0.0,
            artifact_self_similarity: 0.0,
        }
    }
}

// ============================================================================
// Convergence Weights
// ============================================================================

/// Configurable weights for the convergence delta computation.
///
/// These weights control the relative importance of each signal dimension when
/// computing the composite `convergence_delta`. They are configurable per task
/// complexity:
///
/// - For well-tested tasks, `w_test` should dominate because test pass/fail is
///   the strongest convergence signal.
/// - For exploratory tasks with few tests, structural stability (`w_structural`)
///   matters more as a proxy for progress.
/// - For security-sensitive tasks, the security veto provides a hard constraint
///   independent of these weights.
///
/// Weights should sum to 1.0 for normalized delta values, but this is not enforced --
/// non-unit sums produce scaled (not normalized) deltas, which is acceptable if
/// consistent within a trajectory.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConvergenceWeights {
    /// Weight for the test pass rate delta. Default: 0.4.
    /// Measures functional progress through test results.
    pub w_test: f64,

    /// Weight for the error count reduction. Default: 0.3.
    /// Measures build, type check, and lint error elimination.
    pub w_error: f64,

    /// Weight for the regression penalty (inverted: higher weight penalizes regressions more).
    /// Default: 0.2.
    pub w_regression: f64,

    /// Weight for structural stability (low AST churn). Default: 0.1.
    /// Measures whether changes are becoming smaller and more targeted.
    pub w_structural: f64,
}

impl Default for ConvergenceWeights {
    /// Returns the default weight configuration tuned for well-tested tasks.
    ///
    /// Default: w_test=0.4, w_error=0.3, w_regression=0.2, w_structural=0.1 (sum = 1.0).
    fn default() -> Self {
        Self {
            w_test: 0.4,
            w_error: 0.3,
            w_regression: 0.2,
            w_structural: 0.1,
        }
    }
}

// ============================================================================
// Convergence Delta Computation
// ============================================================================

/// Computes the convergence delta between two consecutive observations.
///
/// The convergence delta is the *derivative* of progress -- it measures how much
/// closer (or further) the current observation is from the goal compared to the
/// previous observation. This is the primary signal for attractor classification
/// and strategy bandit updates.
///
/// # Formula
///
/// ```text
/// test_delta     = (curr_pass - prev_pass) / total_tests
/// error_delta    = (prev_errors - curr_errors) / prev_errors
/// regression_pen = regression_count / total_tests
/// structural     = 1.0 - min(ast_diff / 200, 1.0)
///
/// delta = w_test * test_delta
///       + w_error * error_delta
///       + w_regression * (1.0 - regression_penalty)
///       + w_structural * structural
/// ```
///
/// # Security Veto
///
/// If the current observation introduces new vulnerabilities (critical or high severity)
/// compared to the previous observation, the delta is capped at 0.0. This prevents
/// the strategy bandit from learning that vulnerability-introducing approaches are
/// productive, even if they make progress on other dimensions.
///
/// # Context Degradation Penalty
///
/// When `context_health.signal_to_noise` drops below 0.5, the convergence signal itself
/// becomes unreliable (the LLM may be producing confused output). The delta is scaled
/// down proportionally: `delta *= signal_to_noise / 0.5`. At signal_to_noise = 0.25,
/// the delta is halved; at 0.0 it is zeroed out entirely.
///
/// # Arguments
///
/// * `prev` - The previous observation to compute the delta against.
/// * `current_signals` - The current overseer signals (from the observation being measured).
/// * `current_ast_diff` - Number of AST nodes changed between the previous and current artifact.
/// * `context_health` - Current context health metrics for degradation penalty.
/// * `weights` - Configurable weights for each signal dimension.
///
/// # Returns
///
/// The composite convergence delta. Positive means converging, negative means diverging,
/// zero means stalled.
pub fn compute_convergence_delta(
    prev: &Observation,
    current_signals: &OverseerSignals,
    current_ast_diff: u32,
    context_health: &ContextHealth,
    weights: &ConvergenceWeights,
) -> f64 {
    let prev_signals = &prev.overseer_signals;

    // --- Test pass rate delta ---
    // Measures functional progress: how many more (or fewer) tests pass now.
    let prev_pass = prev_signals
        .test_results
        .as_ref()
        .map(|t| t.passed)
        .unwrap_or(0);
    let curr_pass = current_signals
        .test_results
        .as_ref()
        .map(|t| t.passed)
        .unwrap_or(0);
    let total_tests = current_signals
        .test_results
        .as_ref()
        .map(|t| t.total.max(1))
        .unwrap_or(1);
    let test_delta = (curr_pass as f64 - prev_pass as f64) / total_tests as f64;

    // --- Error count reduction ---
    // Measures build/type/lint error elimination. Uses prev_errors.max(1) to avoid
    // division by zero when the previous observation had zero errors.
    let prev_errors = prev_signals.error_count().max(1);
    let error_delta =
        (prev_errors as f64 - current_signals.error_count() as f64) / prev_errors as f64;

    // --- Regression penalty ---
    // Regressions (tests that previously passed but now fail) are penalized as a
    // fraction of total tests. The penalty is inverted in the formula: we reward
    // (1.0 - penalty), so zero regressions contribute the full weight.
    let regression_penalty = current_signals
        .test_results
        .as_ref()
        .map(|t| t.regression_count as f64 / total_tests as f64)
        .unwrap_or(0.0);

    // --- Structural churn ---
    // Measures whether the changes are becoming smaller and more targeted. An AST
    // diff of 0 nodes gives structural = 1.0 (perfect stability); 200+ nodes gives
    // structural = 0.0 (massive rewrite). The 200-node threshold is a heuristic
    // calibrated to typical function-level changes.
    let structural_churn = 1.0 - (current_ast_diff as f64 / 200.0).min(1.0);

    // --- Weighted composite ---
    let mut delta = weights.w_test * test_delta
        + weights.w_error * error_delta
        + weights.w_regression * (1.0 - regression_penalty)
        + weights.w_structural * structural_churn;

    // --- Security veto ---
    // Strategies that introduce vulnerabilities never get credit for "progress."
    // This trains the bandit to avoid vulnerability-introducing approaches.
    let prev_vulns = prev_signals.vulnerability_count();
    let curr_vulns = current_signals.vulnerability_count();
    if curr_vulns > prev_vulns {
        delta = delta.min(0.0);
    }

    // --- Context degradation penalty ---
    // When context health degrades, the convergence signal itself becomes unreliable.
    // Scale down proportionally below the 0.5 threshold.
    if context_health.signal_to_noise < 0.5 {
        delta *= context_health.signal_to_noise / 0.5;
    }

    delta
}

// ============================================================================
// Convergence Level Computation
// ============================================================================

/// Computes the absolute convergence level from overseer signals.
///
/// The convergence level measures how close the current implementation state is to
/// "done" on a scale of 0.0 (nothing works) to 1.0 (fully converged). Unlike
/// `convergence_delta` which measures change between observations, the level is a
/// snapshot of the current state.
///
/// # Weights
///
/// The level is a weighted combination of individual overseer dimensions:
///
/// | Dimension   | Weight | Source                                |
/// |-------------|--------|---------------------------------------|
/// | Tests       | 0.55   | Fraction of tests passing             |
/// | Build       | 0.20   | Binary: builds or doesn't             |
/// | Type check  | 0.10   | Binary: clean or has errors           |
/// | Custom      | 0.15   | Fraction of custom checks passing     |
///
/// Absent overseers default to 1.0 (assumed passing) to avoid penalizing tasks that
/// don't use all overseer types.
///
/// # Hard Gates
///
/// Regardless of the weighted score, certain failures impose hard caps:
///
/// - **Build failure**: caps the level at 0.3. Code that doesn't build cannot be
///   more than 30% converged, no matter how many tests are defined.
/// - **Type check failure**: caps the level at 0.6. Type errors indicate structural
///   problems that limit confidence in functional correctness.
///
/// # No-Signal Rule
///
/// If no overseers have produced any signals (`has_any_signal()` returns false),
/// the level is 0.0. The system must have at least one signal source to assess
/// convergence -- self-assessment is explicitly excluded.
///
/// # Arguments
///
/// * `signals` - The overseer signals from the current observation.
///
/// # Returns
///
/// The convergence level in the range [0.0, 1.0].
pub fn convergence_level(signals: &OverseerSignals) -> f64 {
    // No overseers configured means level is 0.0.
    // The system must have at least one signal source to assess convergence.
    if !signals.has_any_signal() {
        return 0.0;
    }

    // --- Individual dimension levels ---
    // Absent overseers default to 1.0 (assumed passing) to avoid penalizing tasks
    // that don't use all overseer types.

    let test_level = signals
        .test_results
        .as_ref()
        .map(|t| t.passed as f64 / t.total.max(1) as f64)
        .unwrap_or(1.0);

    let build_level = signals
        .build_result
        .as_ref()
        .map(|b| if b.success { 1.0 } else { 0.0 })
        .unwrap_or(1.0);

    let type_level = signals
        .type_check
        .as_ref()
        .map(|t| if t.clean { 1.0 } else { 0.0 })
        .unwrap_or(1.0);

    let custom_level = if signals.custom_checks.is_empty() {
        1.0
    } else {
        signals
            .custom_checks
            .iter()
            .filter(|c| c.passed)
            .count() as f64
            / signals.custom_checks.len() as f64
    };

    // --- Weighted composite ---
    let level =
        0.55 * test_level + 0.20 * build_level + 0.10 * type_level + 0.15 * custom_level;

    // --- Hard gates ---
    // Build failure caps at 0.3: code that doesn't compile cannot be more than 30% done.
    if build_level < 1.0 {
        return level.min(0.3);
    }
    // Type check failure caps at 0.6: type errors limit confidence in correctness.
    if type_level < 1.0 {
        return level.min(0.6);
    }

    level
}

// ============================================================================
// Context Health Estimation
// ============================================================================

/// Estimates context health from a slice of observations.
///
/// This is a simplified estimator that uses observation-level proxies rather than
/// full token counting. In production, the full estimator would count useful vs total
/// tokens in the LLM context window. This version uses observation count as a proxy
/// for context noise accumulation.
///
/// # Signal-to-Noise Estimation
///
/// Uses the observation count as a proxy: each observation adds context noise while
/// the useful signal (spec + current code + latest signals) stays roughly constant.
/// The formula is `1.0 / (1.0 + 0.1 * observation_count)`, which gives:
/// - 0 observations: 1.0 (fresh context)
/// - 5 observations: ~0.67
/// - 10 observations: ~0.50
/// - 20 observations: ~0.33
///
/// # Structural Churn Rate
///
/// Computed from the most recent 3 observations' `ast_diff_nodes`. High churn with
/// no functional progress indicates context confusion -- the LLM is making large
/// rewrites each iteration without converging.
///
/// # Artifact Self-Similarity
///
/// A simplified check based on recent convergence deltas: if the last few deltas are
/// all near zero (|delta| < 0.01), the artifacts are likely very similar (the LLM is
/// reproducing similar output each iteration). A more sophisticated implementation
/// would compute actual AST similarity between artifacts.
///
/// # Arguments
///
/// * `observations` - The complete sequence of observations in the trajectory.
///
/// # Returns
///
/// A `ContextHealth` struct with estimated health metrics.
pub fn estimate_context_health(observations: &[Observation]) -> ContextHealth {
    if observations.is_empty() {
        return ContextHealth::default();
    }

    // --- Signal-to-noise estimation ---
    // Use observation count as a proxy for context noise accumulation.
    // Each observation adds iteration history to the context while the useful signal
    // (spec + current code + latest signals + hints) stays roughly constant.
    let observation_count = observations.len() as f64;
    let signal_to_noise = 1.0 / (1.0 + 0.1 * observation_count);

    // --- Structural churn rate ---
    // Average ast_diff_nodes over the most recent 3 observations.
    let recent_start = observations.len().saturating_sub(3);
    let recent = &observations[recent_start..];
    let churn_rate = recent
        .iter()
        .filter_map(|o| o.metrics.as_ref())
        .map(|m| m.ast_diff_nodes as f64)
        .sum::<f64>()
        / recent.len().max(1) as f64;

    // --- Artifact self-similarity ---
    // Simplified: check if recent convergence deltas are near zero.
    // Near-zero deltas over multiple observations suggest the LLM is producing
    // nearly identical artifacts each iteration.
    let recent_deltas: Vec<f64> = recent
        .iter()
        .filter_map(|o| o.metrics.as_ref())
        .map(|m| m.convergence_delta)
        .collect();

    let artifact_self_similarity = if recent_deltas.len() >= 2 {
        // Compute similarity as the fraction of near-zero deltas.
        // All near-zero means high self-similarity (the LLM is regenerating the same code).
        let near_zero_count = recent_deltas.iter().filter(|d| d.abs() < 0.01).count();
        (near_zero_count as f64 / recent_deltas.len() as f64).min(1.0)
    } else {
        // Fewer than 2 deltas -- cannot meaningfully assess self-similarity.
        0.0
    };

    ContextHealth {
        signal_to_noise,
        structural_churn_rate: churn_rate,
        artifact_self_similarity,
    }
}

// ============================================================================
// Context Degradation Detection
// ============================================================================

/// Determines whether the LLM's context has degraded to the point where a fresh
/// start is warranted.
///
/// A fresh start discards accumulated context noise and re-initializes with only
/// essential carry-forward: the effective specification, latest artifact, most recent
/// overseer signals, user hints, and curated convergence memory.
///
/// # Fresh Start Guard
///
/// The first check guards against infinite reset loops: if `total_fresh_starts` has
/// reached `max_fresh_starts`, degradation detection returns `false` regardless of
/// context health. The trajectory must resolve through other means.
///
/// # Degradation Conditions
///
/// Context is considered degraded if any of the following conditions are met:
///
/// 1. **High churn with no progress**: `structural_churn_rate > 50.0` AND the most
///    recent 3 observations all have `|convergence_delta| < 0.03`. The LLM is making
///    large structural changes each iteration but not converging -- a sign of context
///    confusion.
///
/// 2. **Noisy context**: `signal_to_noise < 0.4`. The context has accumulated enough
///    noise that convergence signals are unreliable.
///
/// 3. **Security regression**: Any of the most recent 3 observations has
///    `vulnerability_delta > 2`. The LLM is introducing vulnerabilities at an
///    accelerating rate, suggesting it has lost track of security constraints.
///
/// 4. **Duplicating artifacts**: `artifact_self_similarity > 0.9` with at least 2
///    observations. The LLM is re-generating essentially the same code, indicating
///    prompt fixation or context saturation.
///
/// # Arguments
///
/// * `observations` - The complete sequence of observations in the trajectory.
/// * `total_fresh_starts` - How many fresh starts have already occurred in this trajectory.
/// * `max_fresh_starts` - Maximum allowed fresh starts (from convergence policy).
///
/// # Returns
///
/// `true` if a fresh start should be triggered, `false` otherwise.
pub fn context_is_degraded(
    observations: &[Observation],
    total_fresh_starts: u32,
    max_fresh_starts: u32,
) -> bool {
    // Guard against infinite reset loops.
    // Once max fresh starts are exhausted, the trajectory must resolve through
    // other means (partial acceptance, escalation, or failure).
    if total_fresh_starts >= max_fresh_starts {
        return false;
    }

    let health = estimate_context_health(observations);

    // --- Condition 1: High structural churn with no functional progress ---
    // The LLM is making large changes each iteration but convergence deltas are
    // near zero -- classic context confusion.
    let high_churn_no_progress = health.structural_churn_rate > 50.0 && {
        let recent_start = observations.len().saturating_sub(3);
        let recent = &observations[recent_start..];
        recent.iter().all(|o| {
            o.metrics
                .as_ref()
                .map(|m| m.convergence_delta.abs() < 0.03)
                .unwrap_or(true)
        })
    };

    // --- Condition 2: Context signal-to-noise degraded below threshold ---
    let context_noisy = health.signal_to_noise < 0.4;

    // --- Condition 3: Security regression accelerating ---
    // Any recent observation introduced more than 2 new vulnerabilities.
    let security_regressing = {
        let recent_start = observations.len().saturating_sub(3);
        let recent = &observations[recent_start..];
        recent.iter().any(|o| {
            o.metrics
                .as_ref()
                .map(|m| m.vulnerability_delta > 2)
                .unwrap_or(false)
        })
    };

    // --- Condition 4: Re-generating the same code ---
    // High artifact self-similarity with enough observations to be meaningful.
    let duplicating = health.artifact_self_similarity > 0.9 && observations.len() >= 2;

    high_churn_no_progress || context_noisy || security_regressing || duplicating
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper to create a minimal `OverseerSignals` with test results.
    fn signals_with_tests(passed: u32, total: u32, regression_count: u32) -> OverseerSignals {
        OverseerSignals {
            test_results: Some(TestResults {
                passed,
                failed: total - passed,
                skipped: 0,
                total,
                regression_count,
                failing_test_names: Vec::new(),
            }),
            type_check: None,
            lint_results: None,
            build_result: None,
            security_scan: None,
            custom_checks: vec![],
        }
    }

    /// Helper to create a minimal `Observation`.
    fn make_observation(
        sequence: u32,
        signals: OverseerSignals,
        metrics: Option<ObservationMetrics>,
    ) -> Observation {
        Observation {
            id: uuid::Uuid::new_v4(),
            sequence,
            timestamp: chrono::Utc::now(),
            artifact: ArtifactReference::default(),
            overseer_signals: signals,
            verification: None,
            metrics,
            tokens_used: 1000,
            wall_time_ms: 5000,
            strategy_used: StrategyKind::RetryWithFeedback,
        }
    }

    // --- ConvergenceWeights tests ---

    #[test]
    fn test_default_weights_sum_to_one() {
        let w = ConvergenceWeights::default();
        let sum = w.w_test + w.w_error + w.w_regression + w.w_structural;
        assert!(
            (sum - 1.0).abs() < f64::EPSILON,
            "Default weights should sum to 1.0, got {sum}"
        );
    }

    // --- ContextHealth tests ---

    #[test]
    fn test_context_health_default() {
        let health = ContextHealth::default();
        assert_eq!(health.signal_to_noise, 1.0);
        assert_eq!(health.structural_churn_rate, 0.0);
        assert_eq!(health.artifact_self_similarity, 0.0);
    }

    // --- ObservationMetrics default ---

    #[test]
    fn test_observation_metrics_default() {
        let metrics = ObservationMetrics::default();
        assert_eq!(metrics.ast_diff_nodes, 0);
        assert_eq!(metrics.test_pass_delta, 0);
        assert_eq!(metrics.test_regression_count, 0);
        assert_eq!(metrics.error_count_delta, 0);
        assert_eq!(metrics.vulnerability_delta, 0);
        assert_eq!(metrics.convergence_delta, 0.0);
        assert_eq!(metrics.convergence_level, 0.0);
    }

    // --- convergence_level tests ---

    #[test]
    fn test_convergence_level_no_signals() {
        let signals = OverseerSignals::empty();
        assert_eq!(convergence_level(&signals), 0.0);
    }

    #[test]
    fn test_convergence_level_all_tests_pass() {
        let signals = signals_with_tests(10, 10, 0);
        let level = convergence_level(&signals);
        // With only tests, absent overseers default to 1.0:
        // 0.55 * 1.0 + 0.20 * 1.0 + 0.10 * 1.0 + 0.15 * 1.0 = 1.0
        assert!((level - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_convergence_level_half_tests_pass() {
        let signals = signals_with_tests(5, 10, 0);
        let level = convergence_level(&signals);
        // 0.55 * 0.5 + 0.20 * 1.0 + 0.10 * 1.0 + 0.15 * 1.0 = 0.275 + 0.45 = 0.725
        assert!((level - 0.725).abs() < 1e-10);
    }

    #[test]
    fn test_convergence_level_build_failure_cap() {
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
                success: false,
                error_count: 3,
                errors: Vec::new(),
            }),
            type_check: None,
            lint_results: None,
            security_scan: None,
            custom_checks: vec![],
        };
        let level = convergence_level(&signals);
        // Build failure caps at 0.3
        assert!(
            level <= 0.3,
            "Build failure should cap level at 0.3, got {level}"
        );
    }

    #[test]
    fn test_convergence_level_type_failure_cap() {
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
                clean: false,
                error_count: 2,
                errors: Vec::new(),
            }),
            lint_results: None,
            security_scan: None,
            custom_checks: vec![],
        };
        let level = convergence_level(&signals);
        // Type failure caps at 0.6
        assert!(
            level <= 0.6,
            "Type failure should cap level at 0.6, got {level}"
        );
    }

    #[test]
    fn test_convergence_level_custom_checks() {
        let signals = OverseerSignals {
            test_results: None,
            type_check: None,
            lint_results: None,
            build_result: None,
            security_scan: None,
            custom_checks: vec![
                CustomCheckResult {
                    name: "format".into(),
                    passed: true,
                    details: "ok".into(),
                },
                CustomCheckResult {
                    name: "coverage".into(),
                    passed: false,
                    details: "below threshold".into(),
                },
            ],
        };
        let level = convergence_level(&signals);
        // custom_level = 1/2 = 0.5
        // 0.55 * 1.0 + 0.20 * 1.0 + 0.10 * 1.0 + 0.15 * 0.5 = 0.925
        assert!((level - 0.925).abs() < 1e-10);
    }

    // --- compute_convergence_delta tests ---

    #[test]
    fn test_delta_positive_progress() {
        let prev_signals = signals_with_tests(5, 10, 0);
        let prev = make_observation(1, prev_signals, None);

        let current_signals = signals_with_tests(8, 10, 0);
        let health = ContextHealth::default();
        let weights = ConvergenceWeights::default();

        let delta = compute_convergence_delta(&prev, &current_signals, 20, &health, &weights);
        assert!(
            delta > 0.0,
            "Progress should produce positive delta, got {delta}"
        );
    }

    #[test]
    fn test_delta_negative_regression() {
        let prev_signals = signals_with_tests(8, 10, 0);
        let prev = make_observation(1, prev_signals, None);

        let current_signals = signals_with_tests(3, 10, 5);
        let health = ContextHealth::default();
        let weights = ConvergenceWeights::default();

        let delta = compute_convergence_delta(&prev, &current_signals, 150, &health, &weights);
        // Tests went from 8 to 3, high regressions, high churn -- should be lower
        // than a progress case.
        let prev_signals2 = signals_with_tests(8, 10, 0);
        let prev2 = make_observation(1, prev_signals2, None);
        let current_signals2 = signals_with_tests(9, 10, 0);
        let delta_progress =
            compute_convergence_delta(&prev2, &current_signals2, 10, &health, &weights);
        assert!(
            delta < delta_progress,
            "Regression delta ({delta}) should be less than progress delta ({delta_progress})"
        );
    }

    #[test]
    fn test_delta_security_veto() {
        let prev_signals = OverseerSignals {
            test_results: Some(TestResults {
                passed: 5,
                failed: 5,
                skipped: 0,
                total: 10,
                regression_count: 0,
                failing_test_names: Vec::new(),
            }),
            security_scan: Some(SecurityScanResult {
                critical_count: 0,
                high_count: 0,
                medium_count: 0,
                findings: Vec::new(),
            }),
            type_check: None,
            lint_results: None,
            build_result: None,
            custom_checks: vec![],
        };
        let prev = make_observation(1, prev_signals, None);

        // More tests pass, but vulnerabilities introduced.
        let current_signals = OverseerSignals {
            test_results: Some(TestResults {
                passed: 8,
                failed: 2,
                skipped: 0,
                total: 10,
                regression_count: 0,
                failing_test_names: Vec::new(),
            }),
            security_scan: Some(SecurityScanResult {
                critical_count: 2,
                high_count: 0,
                medium_count: 0,
                findings: Vec::new(),
            }),
            type_check: None,
            lint_results: None,
            build_result: None,
            custom_checks: vec![],
        };
        let health = ContextHealth::default();
        let weights = ConvergenceWeights::default();

        let delta = compute_convergence_delta(&prev, &current_signals, 10, &health, &weights);
        assert!(
            delta <= 0.0,
            "Security veto should cap delta at 0.0, got {delta}"
        );
    }

    #[test]
    fn test_delta_no_security_veto_when_vulns_decrease() {
        let prev_signals = OverseerSignals {
            test_results: Some(TestResults {
                passed: 5,
                failed: 5,
                skipped: 0,
                total: 10,
                regression_count: 0,
                failing_test_names: Vec::new(),
            }),
            security_scan: Some(SecurityScanResult {
                critical_count: 3,
                high_count: 2,
                medium_count: 0,
                findings: Vec::new(),
            }),
            type_check: None,
            lint_results: None,
            build_result: None,
            custom_checks: vec![],
        };
        let prev = make_observation(1, prev_signals, None);

        // Tests improve and vulnerabilities decrease -- no veto.
        let current_signals = OverseerSignals {
            test_results: Some(TestResults {
                passed: 8,
                failed: 2,
                skipped: 0,
                total: 10,
                regression_count: 0,
                failing_test_names: Vec::new(),
            }),
            security_scan: Some(SecurityScanResult {
                critical_count: 1,
                high_count: 0,
                medium_count: 0,
                findings: Vec::new(),
            }),
            type_check: None,
            lint_results: None,
            build_result: None,
            custom_checks: vec![],
        };
        let health = ContextHealth::default();
        let weights = ConvergenceWeights::default();

        let delta = compute_convergence_delta(&prev, &current_signals, 10, &health, &weights);
        assert!(
            delta > 0.0,
            "No security veto when vulns decrease, delta should be positive, got {delta}"
        );
    }

    #[test]
    fn test_delta_context_degradation_penalty() {
        let prev_signals = signals_with_tests(5, 10, 0);
        let prev = make_observation(1, prev_signals, None);

        let current_signals = signals_with_tests(8, 10, 0);
        let weights = ConvergenceWeights::default();

        // Healthy context
        let healthy = ContextHealth {
            signal_to_noise: 1.0,
            ..Default::default()
        };
        let delta_healthy =
            compute_convergence_delta(&prev, &current_signals, 20, &healthy, &weights);

        // Degraded context (signal_to_noise = 0.25, below 0.5 threshold)
        let degraded = ContextHealth {
            signal_to_noise: 0.25,
            ..Default::default()
        };
        let delta_degraded =
            compute_convergence_delta(&prev, &current_signals, 20, &degraded, &weights);

        // Degraded delta should be scaled down: multiplied by 0.25 / 0.5 = 0.5
        assert!(
            delta_degraded < delta_healthy,
            "Degraded context should reduce delta magnitude"
        );
        let expected_ratio = 0.25 / 0.5;
        let actual_ratio = delta_degraded / delta_healthy;
        assert!(
            (actual_ratio - expected_ratio).abs() < 1e-10,
            "Expected ratio {expected_ratio}, got {actual_ratio}"
        );
    }

    #[test]
    fn test_delta_no_penalty_above_threshold() {
        let prev_signals = signals_with_tests(5, 10, 0);
        let prev = make_observation(1, prev_signals, None);

        let current_signals = signals_with_tests(8, 10, 0);
        let weights = ConvergenceWeights::default();

        // Context at exactly 0.5 -- no penalty should apply.
        let borderline = ContextHealth {
            signal_to_noise: 0.5,
            ..Default::default()
        };
        let delta_borderline =
            compute_convergence_delta(&prev, &current_signals, 20, &borderline, &weights);

        let healthy = ContextHealth {
            signal_to_noise: 0.8,
            ..Default::default()
        };
        let delta_healthy =
            compute_convergence_delta(&prev, &current_signals, 20, &healthy, &weights);

        assert!(
            (delta_borderline - delta_healthy).abs() < 1e-10,
            "No penalty should apply at or above 0.5 threshold"
        );
    }

    // --- estimate_context_health tests ---

    #[test]
    fn test_estimate_context_health_empty() {
        let health = estimate_context_health(&[]);
        assert_eq!(health.signal_to_noise, 1.0);
        assert_eq!(health.structural_churn_rate, 0.0);
        assert_eq!(health.artifact_self_similarity, 0.0);
    }

    #[test]
    fn test_estimate_context_health_degrades_with_observations() {
        let signals = signals_with_tests(5, 10, 0);
        let obs: Vec<Observation> = (0..10)
            .map(|i| {
                make_observation(
                    i,
                    signals.clone(),
                    Some(ObservationMetrics {
                        ast_diff_nodes: 100,
                        convergence_delta: 0.1,
                        convergence_level: 0.5,
                        ..ObservationMetrics::default()
                    }),
                )
            })
            .collect();

        let health = estimate_context_health(&obs);
        // 1.0 / (1.0 + 0.1 * 10) = 1.0 / 2.0 = 0.5
        assert!(
            (health.signal_to_noise - 0.5).abs() < 1e-10,
            "Expected signal_to_noise ~0.5, got {}",
            health.signal_to_noise
        );
        assert!(
            (health.structural_churn_rate - 100.0).abs() < 1e-10,
            "Expected churn rate 100.0, got {}",
            health.structural_churn_rate
        );
    }

    #[test]
    fn test_estimate_context_health_self_similarity_near_zero_deltas() {
        let signals = signals_with_tests(5, 10, 0);
        let obs: Vec<Observation> = (0..5)
            .map(|i| {
                make_observation(
                    i,
                    signals.clone(),
                    Some(ObservationMetrics {
                        ast_diff_nodes: 10,
                        convergence_delta: 0.005, // near zero
                        convergence_level: 0.5,
                        ..ObservationMetrics::default()
                    }),
                )
            })
            .collect();

        let health = estimate_context_health(&obs);
        // All 3 recent deltas are near zero -> self-similarity = 3/3 = 1.0
        assert!(
            health.artifact_self_similarity > 0.9,
            "Expected high self-similarity for near-zero deltas, got {}",
            health.artifact_self_similarity
        );
    }

    #[test]
    fn test_estimate_context_health_self_similarity_active_deltas() {
        let signals = signals_with_tests(5, 10, 0);
        let obs: Vec<Observation> = (0..5)
            .map(|i| {
                make_observation(
                    i,
                    signals.clone(),
                    Some(ObservationMetrics {
                        ast_diff_nodes: 10,
                        convergence_delta: 0.15, // well above 0.01
                        convergence_level: 0.5,
                        ..ObservationMetrics::default()
                    }),
                )
            })
            .collect();

        let health = estimate_context_health(&obs);
        // All deltas are far from zero -> self-similarity = 0/3 = 0.0
        assert!(
            health.artifact_self_similarity < 0.01,
            "Expected low self-similarity for active deltas, got {}",
            health.artifact_self_similarity
        );
    }

    // --- context_is_degraded tests ---

    #[test]
    fn test_context_is_degraded_guard_against_infinite_resets() {
        // Even with terrible health, if max fresh starts reached, return false.
        let signals = signals_with_tests(5, 10, 0);
        let obs: Vec<Observation> = (0..20)
            .map(|i| {
                make_observation(
                    i,
                    signals.clone(),
                    Some(ObservationMetrics {
                        ast_diff_nodes: 100,
                        convergence_delta: 0.0,
                        convergence_level: 0.5,
                        ..ObservationMetrics::default()
                    }),
                )
            })
            .collect();

        // max_fresh_starts = 3, total_fresh_starts = 3 -> guard triggers
        assert!(
            !context_is_degraded(&obs, 3, 3),
            "Should guard against infinite resets"
        );
        // Same observations but with fresh starts remaining
        assert!(
            context_is_degraded(&obs, 0, 3),
            "Should detect degradation when fresh starts available"
        );
    }

    #[test]
    fn test_context_is_degraded_security_regression() {
        let signals = signals_with_tests(5, 10, 0);
        let obs: Vec<Observation> = (0..3)
            .map(|i| {
                make_observation(
                    i,
                    signals.clone(),
                    Some(ObservationMetrics {
                        ast_diff_nodes: 10,
                        test_pass_delta: 1,
                        vulnerability_delta: 3, // > 2, triggers security regression
                        convergence_delta: 0.2,
                        convergence_level: 0.5,
                        ..ObservationMetrics::default()
                    }),
                )
            })
            .collect();

        assert!(
            context_is_degraded(&obs, 0, 3),
            "Security regression should trigger degradation"
        );
    }

    #[test]
    fn test_context_is_degraded_high_churn_no_progress() {
        let signals = signals_with_tests(5, 10, 0);
        let obs: Vec<Observation> = (0..5)
            .map(|i| {
                make_observation(
                    i,
                    signals.clone(),
                    Some(ObservationMetrics {
                        ast_diff_nodes: 150, // High churn
                        convergence_delta: 0.01, // Near-zero progress
                        convergence_level: 0.5,
                        ..ObservationMetrics::default()
                    }),
                )
            })
            .collect();

        assert!(
            context_is_degraded(&obs, 0, 3),
            "High churn with no progress should trigger degradation"
        );
    }

    #[test]
    fn test_context_is_degraded_noisy_context() {
        // With 20 observations, signal_to_noise = 1 / (1 + 0.1*20) = 1/3 = 0.33 < 0.4
        let signals = signals_with_tests(8, 10, 0);
        let obs: Vec<Observation> = (0..20)
            .map(|i| {
                make_observation(
                    i,
                    signals.clone(),
                    Some(ObservationMetrics {
                        ast_diff_nodes: 20,
                        convergence_delta: 0.15,
                        convergence_level: 0.8,
                        ..ObservationMetrics::default()
                    }),
                )
            })
            .collect();

        assert!(
            context_is_degraded(&obs, 0, 3),
            "Noisy context (signal_to_noise < 0.4) should trigger degradation"
        );
    }

    #[test]
    fn test_healthy_context_not_degraded() {
        let signals = signals_with_tests(8, 10, 0);
        let obs: Vec<Observation> = (0..3)
            .map(|i| {
                make_observation(
                    i,
                    signals.clone(),
                    Some(ObservationMetrics {
                        ast_diff_nodes: 20,
                        test_pass_delta: 2,
                        error_count_delta: -1,
                        convergence_delta: 0.15,
                        convergence_level: 0.8,
                        ..ObservationMetrics::default()
                    }),
                )
            })
            .collect();

        assert!(
            !context_is_degraded(&obs, 0, 3),
            "Healthy context should not be flagged as degraded"
        );
    }

    #[test]
    fn test_context_is_degraded_duplicating_artifacts() {
        let signals = signals_with_tests(5, 10, 0);
        // Create observations with near-zero deltas to trigger high self-similarity.
        let obs: Vec<Observation> = (0..4)
            .map(|i| {
                make_observation(
                    i,
                    signals.clone(),
                    Some(ObservationMetrics {
                        ast_diff_nodes: 5,
                        convergence_delta: 0.005, // near zero -> high self-similarity
                        convergence_level: 0.5,
                        ..ObservationMetrics::default()
                    }),
                )
            })
            .collect();

        assert!(
            context_is_degraded(&obs, 0, 3),
            "Duplicating artifacts (self_similarity > 0.9) should trigger degradation"
        );
    }

    // --- Serialization roundtrip tests ---

    #[test]
    fn test_observation_metrics_serde_roundtrip() {
        let metrics = ObservationMetrics {
            ast_diff_nodes: 42,
            test_pass_delta: 3,
            test_regression_count: 1,
            error_count_delta: -2,
            vulnerability_delta: 0,
            convergence_delta: 0.15,
            convergence_level: 0.72,
        };
        let json = serde_json::to_string(&metrics).unwrap();
        let deserialized: ObservationMetrics = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.ast_diff_nodes, 42);
        assert_eq!(deserialized.test_pass_delta, 3);
        assert!((deserialized.convergence_delta - 0.15).abs() < f64::EPSILON);
    }

    #[test]
    fn test_context_health_serde_roundtrip() {
        let health = ContextHealth {
            signal_to_noise: 0.65,
            structural_churn_rate: 45.0,
            artifact_self_similarity: 0.3,
        };
        let json = serde_json::to_string(&health).unwrap();
        let deserialized: ContextHealth = serde_json::from_str(&json).unwrap();
        assert!((deserialized.signal_to_noise - 0.65).abs() < f64::EPSILON);
        assert!((deserialized.structural_churn_rate - 45.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_convergence_weights_serde_roundtrip() {
        let weights = ConvergenceWeights {
            w_test: 0.5,
            w_error: 0.2,
            w_regression: 0.2,
            w_structural: 0.1,
        };
        let json = serde_json::to_string(&weights).unwrap();
        let deserialized: ConvergenceWeights = serde_json::from_str(&json).unwrap();
        assert!((deserialized.w_test - 0.5).abs() < f64::EPSILON);
    }
}
