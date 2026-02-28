//! Convergence budget and basin estimation (Spec Part 5).
//!
//! This module implements:
//!
//! - **[`ConvergenceBudget`]** (spec 1.8) -- a multi-dimensional resource envelope
//!   that replaces fixed iteration counts. Budgets are measured in tokens, wall
//!   time, and iterations simultaneously. The tightest dimension governs
//!   exhaustion.
//!
//! - **[`BasinWidth`]** and **[`BasinClassification`]** (spec 5.2) -- a
//!   pre-execution estimate of how easy a task is to converge on. Well-specified
//!   tasks with comprehensive tests have a *wide* basin; vague tasks with no
//!   tests have a *narrow* basin.
//!
//! - **[`ConvergenceEstimate`]** (spec 5.4) -- predicted iteration count, token
//!   cost, and convergence probability for a task, derived from complexity and
//!   basin width.
//!
//! - **[`allocate_budget`]** (spec 5.1) -- creates a budget scaled to task
//!   complexity.
//!
//! - **[`estimate_basin_width`]** (spec 5.2) -- scores specification quality
//!   signals to classify the attractor basin.
//!
//! - **[`apply_basin_width`]** (spec 5.3) -- adjusts an already-allocated budget
//!   and convergence policy based on basin classification.
//!
//! - **[`estimate_convergence_heuristic`]** (spec 5.4) -- a simplified
//!   convergence cost estimator that does not require historical trajectory data.

use std::time::Duration;

use serde::{Deserialize, Serialize};

use super::*;
use crate::domain::models::task::Complexity;

// ---------------------------------------------------------------------------
// ConvergenceBudget
// ---------------------------------------------------------------------------

/// A multi-dimensional resource envelope for convergence (spec 1.8).
///
/// Fixed iteration counts ignore task difficulty and strategy cost. A
/// convergence budget tracks three dimensions simultaneously -- tokens, wall
/// time, and iterations -- so that the tightest dimension governs exhaustion.
///
/// Budgets also support *extensions*: when a trajectory is approaching a fixed
/// point but running low on resources, it can request additional budget up to
/// [`max_extensions`](Self::max_extensions) times.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConvergenceBudget {
    /// Maximum tokens this trajectory may consume.
    pub max_tokens: u64,
    /// Maximum wall-clock time this trajectory may consume.
    pub max_wall_time: Duration,
    /// Safety cap on iteration count (not the primary limit).
    pub max_iterations: u32,

    /// Tokens consumed so far.
    pub tokens_used: u64,
    /// Wall-clock time consumed so far.
    pub wall_time_used: Duration,
    /// Iterations completed so far.
    pub iterations_used: u32,

    /// Number of budget extensions requested by the engine.
    pub extensions_requested: u32,
    /// Number of budget extensions that were actually granted.
    pub extensions_granted: u32,
    /// Maximum number of extensions that may be granted.
    pub max_extensions: u32,
}

impl Default for ConvergenceBudget {
    /// Sensible defaults: 100 000 tokens, 30 minutes wall time, 5 iterations,
    /// and 1 extension allowed.
    fn default() -> Self {
        Self {
            max_tokens: 100_000,
            max_wall_time: Duration::from_secs(30 * 60),
            max_iterations: 5,
            tokens_used: 0,
            wall_time_used: Duration::ZERO,
            iterations_used: 0,
            extensions_requested: 0,
            extensions_granted: 0,
            max_extensions: 1,
        }
    }
}

impl ConvergenceBudget {
    /// Fraction of budget remaining, taken as the *minimum* across all three
    /// dimensions (tokens, wall time, iterations).
    ///
    /// Returns a value in `[0.0, 1.0]` where `0.0` means at least one
    /// dimension is fully exhausted.
    pub fn remaining_fraction(&self) -> f64 {
        let token_frac = 1.0 - (self.tokens_used as f64 / self.max_tokens.max(1) as f64);
        let time_frac = 1.0
            - (self.wall_time_used.as_secs_f64()
                / self.max_wall_time.as_secs_f64().max(f64::MIN_POSITIVE));
        let iter_frac =
            1.0 - (self.iterations_used as f64 / self.max_iterations.max(1) as f64);

        token_frac
            .min(time_frac)
            .min(iter_frac)
            .clamp(0.0, 1.0)
    }

    /// Whether any budget remains across all dimensions.
    pub fn has_remaining(&self) -> bool {
        self.remaining_fraction() > 0.0
    }

    /// Whether the budget can afford the estimated cost of the given strategy.
    ///
    /// A strategy is allowed if consuming its estimated token cost would not
    /// exceed `max_tokens` and the iteration count would not exceed
    /// `max_iterations`.
    pub fn allows_strategy_cost(&self, strategy: &StrategyKind) -> bool {
        self.tokens_used + strategy.estimated_cost() <= self.max_tokens
            && self.iterations_used < self.max_iterations
    }

    /// Whether the engine should request a budget extension.
    ///
    /// An extension is warranted when the remaining budget fraction drops
    /// below 15 %, the trajectory is converging (approaching a fixed point),
    /// and extensions have not already been exhausted.
    pub fn should_request_extension(&self, convergence_delta_positive: bool) -> bool {
        self.remaining_fraction() < 0.15
            && convergence_delta_positive
            && self.extensions_requested < self.max_extensions
    }

    /// Record the consumption of one iteration's resources.
    ///
    /// Increments `tokens_used` by `tokens`, adds `wall_time_ms` milliseconds
    /// to `wall_time_used`, and increments `iterations_used` by one.
    pub fn consume(&mut self, tokens: u64, wall_time_ms: u64) {
        self.tokens_used += tokens;
        self.wall_time_used += Duration::from_millis(wall_time_ms);
        self.iterations_used += 1;
    }

    /// Grant a budget extension.
    ///
    /// Increases `max_tokens` by `additional_tokens` and `max_iterations` by
    /// `additional_iterations`, and increments `extensions_granted`.
    pub fn extend(&mut self, additional_tokens: u64, additional_iterations: u32) {
        self.extensions_granted += 1;
        self.max_tokens += additional_tokens;
        self.max_iterations += additional_iterations;
    }

    /// Produce a new budget whose maximums are scaled by `factor`.
    ///
    /// The returned budget has fresh usage counters (zero consumed) and the
    /// same extension limits. Useful when decomposing a task into subtasks
    /// that each receive a fraction of the parent's budget.
    pub fn scale(&self, factor: f64) -> Self {
        Self {
            max_tokens: (self.max_tokens as f64 * factor).round() as u64,
            max_wall_time: Duration::from_secs_f64(
                self.max_wall_time.as_secs_f64() * factor,
            ),
            max_iterations: (self.max_iterations as f64 * factor).round().max(1.0) as u32,
            tokens_used: 0,
            wall_time_used: Duration::ZERO,
            iterations_used: 0,
            extensions_requested: 0,
            extensions_granted: 0,
            max_extensions: self.max_extensions,
        }
    }
}

// ---------------------------------------------------------------------------
// BasinWidth / BasinClassification
// ---------------------------------------------------------------------------

/// A pre-execution estimate of attractor basin width (spec 5.2).
///
/// The score represents how likely a random starting point is to converge on
/// the correct solution. A *wide* basin means many starting points work; a
/// *narrow* basin means the system must get lucky or invest in exploration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BasinWidth {
    /// Estimated basin width on a `[0.0, 1.0]` scale.
    pub score: f64,
    /// Categorical classification derived from the score.
    pub classification: BasinClassification,
}

/// Categorical classification of an attractor basin (spec 5.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BasinClassification {
    /// Score > 0.7. Many starting points converge.
    Wide,
    /// Score in (0.4, 0.7]. Moderate exploration needed.
    Moderate,
    /// Score <= 0.4. Heavy exploration and test generation needed.
    Narrow,
}

// ---------------------------------------------------------------------------
// ConvergenceEstimate
// ---------------------------------------------------------------------------

/// Predicted convergence cost for a task (spec 5.4).
///
/// Used by the proactive decomposition decision to compare monolithic vs.
/// decomposed approaches, and by the budget extension logic to decide whether
/// an extension is worthwhile.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConvergenceEstimate {
    /// Expected number of iterations to converge.
    pub expected_iterations: f64,
    /// 95th-percentile iteration count (worst-case planning).
    pub p95_iterations: u32,
    /// Estimated probability of converging within the allocated budget.
    pub convergence_probability: f64,
    /// Expected total tokens consumed.
    pub expected_tokens: u64,
}

// ---------------------------------------------------------------------------
// allocate_budget (spec 5.1)
// ---------------------------------------------------------------------------

/// Allocate a convergence budget based on task complexity (spec 5.1).
///
/// The returned budget encodes the spec table:
///
/// | Complexity | Tokens    | Iterations | Wall Time | Extensions |
/// |------------|-----------|------------|-----------|------------|
/// | Trivial    | 50 000    | 3          | 15 min    | 1          |
/// | Simple     | 150 000   | 5          | 30 min    | 1          |
/// | Moderate   | 400 000   | 8          | 60 min    | 1          |
/// | Complex    | 1 000 000 | 12         | 120 min   | 3          |
pub fn allocate_budget(complexity: Complexity) -> ConvergenceBudget {
    let (tokens, iters, time_mins, extensions) = match complexity {
        Complexity::Trivial => (50_000, 3, 15, 1),
        Complexity::Simple => (150_000, 5, 30, 1),
        Complexity::Moderate => (400_000, 8, 60, 1),
        Complexity::Complex => (1_000_000, 12, 120, 3),
    };

    ConvergenceBudget {
        max_tokens: tokens,
        max_iterations: iters,
        max_wall_time: Duration::from_secs(time_mins * 60),
        max_extensions: extensions,
        ..Default::default()
    }
}

// ---------------------------------------------------------------------------
// estimate_basin_width (spec 5.2)
// ---------------------------------------------------------------------------

/// Estimate the attractor basin width from specification quality signals
/// (spec 5.2).
///
/// The scoring works as follows:
///
/// - Start at a baseline of **0.5**.
/// - Specification quality signals *widen* the basin:
///   - Acceptance tests present: **+0.15**
///   - Examples present: **+0.10**
///   - Invariants present: **+0.10**
///   - Anti-examples present: **+0.05**
///   - Context files present: **+0.05**
/// - Specification complexity signals *narrow* the basin:
///   - Description shorter than 20 words: **-0.15**
///   - Description longer than 500 words: **-0.10**
///
/// The final score is clamped to `[0.0, 1.0]` and classified as `Wide`
/// (>0.7), `Moderate` (>0.4), or `Narrow` (<=0.4).
pub fn estimate_basin_width(
    description: &str,
    has_acceptance_tests: bool,
    has_examples: bool,
    has_invariants: bool,
    has_anti_examples: bool,
    has_context_files: bool,
) -> BasinWidth {
    let mut score: f64 = 0.5;

    // Specification quality signals (widen the basin).
    if has_acceptance_tests {
        score += 0.15;
    }
    if has_examples {
        score += 0.10;
    }
    if has_invariants {
        score += 0.10;
    }
    if has_anti_examples {
        score += 0.05;
    }
    if has_context_files {
        score += 0.05;
    }

    // Specification complexity signals (narrow the basin).
    let word_count = description.split_whitespace().count();
    if word_count < 20 {
        score -= 0.15;
    }
    if word_count > 500 {
        score -= 0.10;
    }

    let score = score.clamp(0.0, 1.0);

    let classification = if score > 0.7 {
        BasinClassification::Wide
    } else if score > 0.4 {
        BasinClassification::Moderate
    } else {
        BasinClassification::Narrow
    };

    BasinWidth {
        score,
        classification,
    }
}

// ---------------------------------------------------------------------------
// apply_basin_width (spec 5.3)
// ---------------------------------------------------------------------------

/// Adjust a budget and convergence policy based on basin classification
/// (spec 5.3).
///
/// These adjustments compose with the base allocation from
/// [`allocate_budget`] and any priority hints:
///
/// - **Wide** basin: reduce iterations to 75 % of the base, set exploration
///   weight to 0.2 (mostly exploit).
/// - **Moderate** basin: set exploration weight to 0.4.
/// - **Narrow** basin: increase iterations to 150 %, increase tokens to
///   130 %, set exploration weight to 0.6 (mostly explore), and enable
///   acceptance test generation.
pub fn apply_basin_width(
    basin: &BasinWidth,
    budget: &mut ConvergenceBudget,
    policy: &mut ConvergencePolicy,
) {
    match basin.classification {
        BasinClassification::Wide => {
            budget.max_iterations = (budget.max_iterations as f64 * 0.75) as u32;
            policy.exploration_weight = 0.2;
        }
        BasinClassification::Moderate => {
            policy.exploration_weight = 0.4;
        }
        BasinClassification::Narrow => {
            budget.max_iterations = (budget.max_iterations as f64 * 1.5) as u32;
            budget.max_tokens = (budget.max_tokens as f64 * 1.3) as u64;
            policy.exploration_weight = 0.6;
            policy.generate_acceptance_tests = true;
        }
    }
}

// ---------------------------------------------------------------------------
// estimate_convergence_heuristic (spec 5.4)
// ---------------------------------------------------------------------------

/// Estimate convergence cost without historical data (spec 5.4).
///
/// This is the heuristic branch of the full `estimate_convergence` function
/// described in the spec. When fewer than 10 similar trajectories exist in
/// the repository, the system falls back to this formula:
///
/// ```text
/// base       = { Trivial: 2.0, Simple: 4.0, Moderate: 6.0, Complex: 9.0 }
/// adjusted   = base / basin.score
/// p95        = ceil(adjusted * 1.8)
/// probability = basin.score
/// tokens     = adjusted * 30_000
/// ```
pub fn estimate_convergence_heuristic(
    complexity: Complexity,
    basin: &BasinWidth,
) -> ConvergenceEstimate {
    let base: f64 = match complexity {
        Complexity::Trivial => 2.0,
        Complexity::Simple => 4.0,
        Complexity::Moderate => 6.0,
        Complexity::Complex => 9.0,
    };

    // Avoid division by zero if basin score is extremely small.
    let clamped_score = basin.score.max(0.05);
    let adjusted = base / clamped_score;

    ConvergenceEstimate {
        expected_iterations: adjusted,
        p95_iterations: (adjusted * 1.8).ceil() as u32,
        convergence_probability: basin.score,
        expected_tokens: (adjusted * 30_000.0) as u64,
    }
}

// ---------------------------------------------------------------------------
// allocate_decomposed_budget (spec 9.3)
// ---------------------------------------------------------------------------

/// Split a parent budget among decomposed subtasks, reserving 10% for
/// integration.
///
/// Each subtask receives a share of the remaining 90% proportional to its
/// `budget_fraction`. The integration reserve (10%) ensures that even after
/// all subtasks converge, resources remain for the mandatory integration
/// trajectory that verifies compositional correctness.
///
/// All returned budgets have fresh usage counters (zero consumed).
pub fn allocate_decomposed_budget(
    parent_budget: &ConvergenceBudget,
    decomposition: &[TaskDecomposition],
) -> Vec<ConvergenceBudget> {
    // Reserve 10% for integration; distribute the remaining 90%.
    let distributable_fraction = 0.90;

    decomposition
        .iter()
        .map(|subtask| {
            let factor = distributable_fraction * subtask.budget_fraction;
            parent_budget.scale(factor)
        })
        .collect()
}

// ---------------------------------------------------------------------------
// compute_parent_convergence (spec 9.3)
// ---------------------------------------------------------------------------

/// Compute overall convergence level based on child outcomes.
///
/// Returns a value in `[0.0, 1.0]` representing the fraction of children
/// that converged. A result of `1.0` means all children converged
/// successfully; `0.0` means none did.
pub fn compute_parent_convergence(child_outcomes: &[ConvergenceOutcome]) -> f64 {
    if child_outcomes.is_empty() {
        return 0.0;
    }

    let converged_count = child_outcomes
        .iter()
        .filter(|o| matches!(o, ConvergenceOutcome::Converged { .. }))
        .count();

    converged_count as f64 / child_outcomes.len() as f64
}

// =========================================================================
// Tests
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // -- ConvergenceBudget defaults ----------------------------------------

    #[test]
    fn test_default_budget() {
        let b = ConvergenceBudget::default();
        assert_eq!(b.max_tokens, 100_000);
        assert_eq!(b.max_wall_time, Duration::from_secs(30 * 60));
        assert_eq!(b.max_iterations, 5);
        assert_eq!(b.max_extensions, 1);
        assert_eq!(b.tokens_used, 0);
        assert_eq!(b.iterations_used, 0);
        assert_eq!(b.wall_time_used, Duration::ZERO);
    }

    // -- remaining_fraction ------------------------------------------------

    #[test]
    fn test_remaining_fraction_fresh_budget() {
        let b = ConvergenceBudget::default();
        assert!((b.remaining_fraction() - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_remaining_fraction_half_used() {
        let mut b = ConvergenceBudget::default();
        b.tokens_used = 50_000;
        // tokens = 50 %, time = 100 %, iters = 100 % => min = 50 %
        assert!((b.remaining_fraction() - 0.5).abs() < 1e-9);
    }

    #[test]
    fn test_remaining_fraction_exhausted() {
        let mut b = ConvergenceBudget::default();
        b.iterations_used = 5;
        assert!((b.remaining_fraction() - 0.0).abs() < f64::EPSILON);
    }

    // -- has_remaining -----------------------------------------------------

    #[test]
    fn test_has_remaining_true_for_fresh() {
        assert!(ConvergenceBudget::default().has_remaining());
    }

    #[test]
    fn test_has_remaining_false_when_exhausted() {
        let mut b = ConvergenceBudget::default();
        b.tokens_used = b.max_tokens;
        assert!(!b.has_remaining());
    }

    // -- consume -----------------------------------------------------------

    #[test]
    fn test_consume_increments() {
        let mut b = ConvergenceBudget::default();
        b.consume(10_000, 5_000);
        assert_eq!(b.tokens_used, 10_000);
        assert_eq!(b.wall_time_used, Duration::from_millis(5_000));
        assert_eq!(b.iterations_used, 1);

        b.consume(20_000, 3_000);
        assert_eq!(b.tokens_used, 30_000);
        assert_eq!(b.wall_time_used, Duration::from_millis(8_000));
        assert_eq!(b.iterations_used, 2);
    }

    // -- extend ------------------------------------------------------------

    #[test]
    fn test_extend_increases_limits() {
        let mut b = ConvergenceBudget::default();
        let orig_tokens = b.max_tokens;
        let orig_iters = b.max_iterations;

        b.extend(50_000, 2);
        assert_eq!(b.max_tokens, orig_tokens + 50_000);
        assert_eq!(b.max_iterations, orig_iters + 2);
        assert_eq!(b.extensions_granted, 1);
    }

    // -- scale -------------------------------------------------------------

    #[test]
    fn test_scale_produces_fresh_budget() {
        let mut b = ConvergenceBudget::default();
        b.consume(10_000, 1_000);

        let scaled = b.scale(0.5);
        assert_eq!(scaled.max_tokens, 50_000);
        assert_eq!(scaled.max_iterations, 3); // round(5 * 0.5) = 3
        assert_eq!(scaled.tokens_used, 0);
        assert_eq!(scaled.iterations_used, 0);
        assert_eq!(scaled.wall_time_used, Duration::ZERO);
        assert_eq!(scaled.max_extensions, b.max_extensions);
    }

    #[test]
    fn test_scale_minimum_one_iteration() {
        let b = ConvergenceBudget {
            max_iterations: 1,
            ..Default::default()
        };
        let scaled = b.scale(0.1);
        assert!(scaled.max_iterations >= 1);
    }

    // -- allocate_budget ---------------------------------------------------

    #[test]
    fn test_allocate_trivial() {
        let b = allocate_budget(Complexity::Trivial);
        assert_eq!(b.max_tokens, 50_000);
        assert_eq!(b.max_iterations, 3);
        assert_eq!(b.max_wall_time, Duration::from_secs(15 * 60));
        assert_eq!(b.max_extensions, 1);
    }

    #[test]
    fn test_allocate_simple() {
        let b = allocate_budget(Complexity::Simple);
        assert_eq!(b.max_tokens, 150_000);
        assert_eq!(b.max_iterations, 5);
        assert_eq!(b.max_wall_time, Duration::from_secs(30 * 60));
        assert_eq!(b.max_extensions, 1);
    }

    #[test]
    fn test_allocate_moderate() {
        let b = allocate_budget(Complexity::Moderate);
        assert_eq!(b.max_tokens, 400_000);
        assert_eq!(b.max_iterations, 8);
        assert_eq!(b.max_wall_time, Duration::from_secs(60 * 60));
        assert_eq!(b.max_extensions, 1);
    }

    #[test]
    fn test_allocate_complex() {
        let b = allocate_budget(Complexity::Complex);
        assert_eq!(b.max_tokens, 1_000_000);
        assert_eq!(b.max_iterations, 12);
        assert_eq!(b.max_wall_time, Duration::from_secs(120 * 60));
        assert_eq!(b.max_extensions, 3);
    }

    // -- estimate_basin_width ----------------------------------------------

    #[test]
    fn test_basin_width_baseline() {
        let bw = estimate_basin_width(
            "A moderately detailed description of the task at hand with enough words to pass the minimum threshold for this particular test case",
            false, false, false, false, false,
        );
        assert!((bw.score - 0.5).abs() < f64::EPSILON);
        assert_eq!(bw.classification, BasinClassification::Moderate);
    }

    #[test]
    fn test_basin_width_all_signals_present() {
        let description = "A well-specified task with comprehensive detail about what needs to happen including tests examples invariants and more context than you can shake a stick at";
        let bw = estimate_basin_width(description, true, true, true, true, true);
        // 0.5 + 0.15 + 0.10 + 0.10 + 0.05 + 0.05 = 0.95
        assert!((bw.score - 0.95).abs() < f64::EPSILON);
        assert_eq!(bw.classification, BasinClassification::Wide);
    }

    #[test]
    fn test_basin_width_narrow_short_description() {
        let bw = estimate_basin_width("fix it", false, false, false, false, false);
        // 0.5 - 0.15 (short) = 0.35
        assert!((bw.score - 0.35).abs() < f64::EPSILON);
        assert_eq!(bw.classification, BasinClassification::Narrow);
    }

    #[test]
    fn test_basin_width_verbose_description_penalty() {
        // Build a description longer than 500 words.
        let words: Vec<&str> = std::iter::repeat("word").take(510).collect();
        let long_description = words.join(" ");
        let bw = estimate_basin_width(&long_description, true, false, false, false, false);
        // 0.5 + 0.15 (tests) - 0.10 (verbose) = 0.55
        assert!((bw.score - 0.55).abs() < f64::EPSILON);
        assert_eq!(bw.classification, BasinClassification::Moderate);
    }

    #[test]
    fn test_basin_width_clamped_to_unit() {
        // Even with all bonuses and penalties the score stays in [0, 1].
        let bw = estimate_basin_width(
            "A well-specified task with comprehensive detail about what needs to happen including tests examples invariants and more context",
            true, true, true, true, true,
        );
        assert!(bw.score >= 0.0);
        assert!(bw.score <= 1.0);
    }

    // -- apply_basin_width -------------------------------------------------

    #[test]
    fn test_apply_wide_basin() {
        let basin = BasinWidth {
            score: 0.8,
            classification: BasinClassification::Wide,
        };
        let mut budget = allocate_budget(Complexity::Moderate);
        let mut policy = ConvergencePolicy::default();

        apply_basin_width(&basin, &mut budget, &mut policy);
        // 8 * 0.75 = 6
        assert_eq!(budget.max_iterations, 6);
        assert!((policy.exploration_weight - 0.2).abs() < f64::EPSILON);
    }

    #[test]
    fn test_apply_moderate_basin() {
        let basin = BasinWidth {
            score: 0.55,
            classification: BasinClassification::Moderate,
        };
        let mut budget = allocate_budget(Complexity::Moderate);
        let orig_iters = budget.max_iterations;
        let mut policy = ConvergencePolicy::default();

        apply_basin_width(&basin, &mut budget, &mut policy);
        // Iterations unchanged for Moderate basin.
        assert_eq!(budget.max_iterations, orig_iters);
        assert!((policy.exploration_weight - 0.4).abs() < f64::EPSILON);
    }

    #[test]
    fn test_apply_narrow_basin() {
        let basin = BasinWidth {
            score: 0.3,
            classification: BasinClassification::Narrow,
        };
        let mut budget = allocate_budget(Complexity::Moderate);
        let mut policy = ConvergencePolicy::default();

        apply_basin_width(&basin, &mut budget, &mut policy);
        // 8 * 1.5 = 12
        assert_eq!(budget.max_iterations, 12);
        // 400_000 * 1.3 = 520_000
        assert_eq!(budget.max_tokens, 520_000);
        assert!((policy.exploration_weight - 0.6).abs() < f64::EPSILON);
        assert!(policy.generate_acceptance_tests);
    }

    // -- estimate_convergence_heuristic ------------------------------------

    #[test]
    fn test_heuristic_trivial_wide() {
        let basin = BasinWidth {
            score: 0.8,
            classification: BasinClassification::Wide,
        };
        let est = estimate_convergence_heuristic(Complexity::Trivial, &basin);
        // 2.0 / 0.8 = 2.5
        assert!((est.expected_iterations - 2.5).abs() < f64::EPSILON);
        assert_eq!(est.p95_iterations, (2.5 * 1.8_f64).ceil() as u32);
        assert!((est.convergence_probability - 0.8).abs() < f64::EPSILON);
        assert_eq!(est.expected_tokens, (2.5 * 30_000.0) as u64);
    }

    #[test]
    fn test_heuristic_complex_narrow() {
        let basin = BasinWidth {
            score: 0.3,
            classification: BasinClassification::Narrow,
        };
        let est = estimate_convergence_heuristic(Complexity::Complex, &basin);
        // 9.0 / 0.3 = 30.0
        assert!((est.expected_iterations - 30.0).abs() < f64::EPSILON);
        assert_eq!(est.p95_iterations, (30.0 * 1.8_f64).ceil() as u32);
        assert!((est.convergence_probability - 0.3).abs() < f64::EPSILON);
    }

    #[test]
    fn test_heuristic_zero_score_clamped() {
        let basin = BasinWidth {
            score: 0.0,
            classification: BasinClassification::Narrow,
        };
        let est = estimate_convergence_heuristic(Complexity::Simple, &basin);
        // Score clamped to 0.05 => 4.0 / 0.05 = 80.0
        assert!((est.expected_iterations - 80.0).abs() < f64::EPSILON);
    }

    // -- serde roundtrips --------------------------------------------------

    #[test]
    fn test_budget_serde_roundtrip() {
        let budget = allocate_budget(Complexity::Moderate);
        let json = serde_json::to_string(&budget).unwrap();
        let deserialized: ConvergenceBudget = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.max_tokens, budget.max_tokens);
        assert_eq!(deserialized.max_iterations, budget.max_iterations);
    }

    #[test]
    fn test_basin_classification_serde_roundtrip() {
        let variants = [
            BasinClassification::Wide,
            BasinClassification::Moderate,
            BasinClassification::Narrow,
        ];
        for variant in &variants {
            let json = serde_json::to_string(variant).unwrap();
            let deserialized: BasinClassification = serde_json::from_str(&json).unwrap();
            assert_eq!(*variant, deserialized);
        }
    }

    #[test]
    fn test_basin_classification_snake_case() {
        assert_eq!(
            serde_json::to_string(&BasinClassification::Wide).unwrap(),
            "\"wide\""
        );
        assert_eq!(
            serde_json::to_string(&BasinClassification::Moderate).unwrap(),
            "\"moderate\""
        );
        assert_eq!(
            serde_json::to_string(&BasinClassification::Narrow).unwrap(),
            "\"narrow\""
        );
    }

    #[test]
    fn test_estimate_serde_roundtrip() {
        let est = ConvergenceEstimate {
            expected_iterations: 4.5,
            p95_iterations: 9,
            convergence_probability: 0.72,
            expected_tokens: 135_000,
        };
        let json = serde_json::to_string(&est).unwrap();
        let deserialized: ConvergenceEstimate = serde_json::from_str(&json).unwrap();
        assert!((deserialized.expected_iterations - 4.5).abs() < f64::EPSILON);
        assert_eq!(deserialized.p95_iterations, 9);
    }

    // -- should_request_extension ------------------------------------------

    #[test]
    fn test_extension_requested_when_low_and_converging() {
        let mut b = ConvergenceBudget::default();
        // Use up ~90 % of tokens so remaining < 15 %.
        b.tokens_used = 90_000;
        assert!(b.should_request_extension(true));
    }

    #[test]
    fn test_extension_not_requested_when_plenty_remaining() {
        let b = ConvergenceBudget::default();
        assert!(!b.should_request_extension(true));
    }

    #[test]
    fn test_extension_not_requested_when_not_converging() {
        let mut b = ConvergenceBudget::default();
        b.tokens_used = 90_000;
        assert!(!b.should_request_extension(false));
    }

    #[test]
    fn test_extension_not_requested_when_max_reached() {
        let mut b = ConvergenceBudget::default();
        b.tokens_used = 90_000;
        b.extensions_requested = b.max_extensions;
        assert!(!b.should_request_extension(true));
    }
}
