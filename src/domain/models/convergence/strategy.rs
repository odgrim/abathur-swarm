//! Convergence strategies and Thompson Sampling bandit selection.
//!
//! This module implements Part 4 of the convergence-attractors spec:
//! strategy types, eligibility filtering, bandit-based selection via
//! Thompson Sampling, outcome evaluation, and decay-aware rotation.
//!
//! ## Design
//!
//! A strategy is not just "retry" -- it is a specific approach to moving
//! a trajectory toward its attractor. Strategies are selected based on
//! attractor state, remaining budget, and historical effectiveness learned
//! through a contextual multi-armed bandit.
//!
//! The flow is:
//! 1. **Eligibility filter** (`eligible_strategies`) narrows candidates
//!    based on the current attractor classification (deterministic).
//! 2. **Thompson Sampling** (`StrategyBandit::select`) picks among
//!    eligible candidates using learned Beta distributions.
//! 3. **Outcome evaluation** (`evaluate_strategy_outcome`) maps an
//!    observation's convergence delta to a reward signal.
//! 4. **Bandit update** (`StrategyBandit::update`) adjusts the Beta
//!    distributions based on the observed outcome.
//! 5. **Decay-aware rotation** (`should_rotate_strategy`) detects when
//!    a strategy's marginal effectiveness has dropped below threshold.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

use super::*;
use crate::domain::models::intent_verification::IntentGap;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Minimum convergence delta to consider a strategy outcome a success.
pub const STRATEGY_SUCCESS_THRESHOLD: f64 = 0.1;

/// Minimum useful progress per iteration; below this, the strategy's
/// contribution is negligible and rotation should be considered.
pub const MINIMUM_USEFUL_PROGRESS: f64 = 0.05;

// ---------------------------------------------------------------------------
// StrategyKind
// ---------------------------------------------------------------------------

/// The set of convergence strategies available to the engine.
///
/// Strategies fall into four families:
/// - **Exploitation** (refine current approach): `RetryWithFeedback`,
///   `RetryAugmented`, `FocusedRepair`, `IncrementalRefinement`.
/// - **Exploration** (try different approach): `Reframe`, `Decompose`,
///   `AlternativeApproach`, `ArchitectReview`.
/// - **Rollback**: `RevertAndBranch`.
/// - **Reset**: `FreshStart`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StrategyKind {
    /// Re-run with overseer feedback from the last iteration appended.
    RetryWithFeedback,

    /// Re-run with additional context: related code, examples, documentation.
    RetryAugmented,

    /// Target specific failing tests with minimal context, maximum focus.
    FocusedRepair,

    /// Address one gap at a time rather than all at once.
    IncrementalRefinement,

    /// Restructure the prompt. Instead of "fix the failing tests," reframe
    /// as "implement X from scratch given these constraints."
    Reframe,

    /// Break the task into smaller sub-tasks. Transitions the trajectory
    /// to Coordinating phase.
    Decompose,

    /// Explicitly instruct a different implementation approach.
    AlternativeApproach,

    /// Escalate to the architect agent for re-planning. Returns a
    /// `SpecificationAmendment` and optionally restructures the DAG.
    ArchitectReview,

    /// Roll back to the best observation so far and branch from there.
    RevertAndBranch {
        /// The observation to revert to.
        target: Uuid,
    },

    /// Start a new LLM execution carrying forward curated knowledge.
    /// The filesystem persists code; the trajectory persists metadata;
    /// the LLM gets a clean context with only curated `CarryForward` signals.
    FreshStart {
        /// Knowledge to carry across the context boundary.
        carry_forward: CarryForward,
    },
}

impl StrategyKind {
    /// Estimated token cost of executing this strategy, used for budget
    /// checks and cost-aware bandit selection.
    pub fn estimated_cost(&self) -> u64 {
        match self {
            StrategyKind::FocusedRepair => 15_000,
            StrategyKind::RetryWithFeedback => 20_000,
            StrategyKind::RevertAndBranch { .. } => 20_000,
            StrategyKind::IncrementalRefinement => 25_000,
            StrategyKind::RetryAugmented
            | StrategyKind::ArchitectReview
            | StrategyKind::FreshStart { .. } => 30_000,
            StrategyKind::AlternativeApproach => 35_000,
            StrategyKind::Reframe => 40_000,
            StrategyKind::Decompose => 50_000,
        }
    }

    /// A stable string name for this strategy kind, suitable for use as
    /// a `HashMap` key. Variants with data are collapsed to their family
    /// name so that the bandit learns about the *kind* of strategy rather
    /// than each unique parameterisation.
    pub fn kind_name(&self) -> &'static str {
        match self {
            StrategyKind::RetryWithFeedback => "retry_with_feedback",
            StrategyKind::RetryAugmented => "retry_augmented",
            StrategyKind::FocusedRepair => "focused_repair",
            StrategyKind::IncrementalRefinement => "incremental_refinement",
            StrategyKind::Reframe => "reframe",
            StrategyKind::Decompose => "decompose",
            StrategyKind::AlternativeApproach => "alternative_approach",
            StrategyKind::ArchitectReview => "architect_review",
            StrategyKind::RevertAndBranch { .. } => "revert_and_branch",
            StrategyKind::FreshStart { .. } => "fresh_start",
        }
    }

    /// Whether this strategy is an exploitation strategy (refines the
    /// current approach) as opposed to exploration (tries something new).
    pub fn is_exploitation(&self) -> bool {
        matches!(
            self,
            StrategyKind::RetryWithFeedback
                | StrategyKind::RetryAugmented
                | StrategyKind::FocusedRepair
                | StrategyKind::IncrementalRefinement
        )
    }

    /// Whether this strategy is an exploration strategy.
    pub fn is_exploration(&self) -> bool {
        matches!(
            self,
            StrategyKind::Reframe
                | StrategyKind::Decompose
                | StrategyKind::AlternativeApproach
                | StrategyKind::ArchitectReview
        )
    }
}

// ---------------------------------------------------------------------------
// CarryForward
// ---------------------------------------------------------------------------

/// Knowledge carried across a fresh-start boundary.
///
/// The filesystem persists code; neural context is discarded; curated
/// signals are injected into the new LLM context. This struct captures
/// exactly what crosses that boundary.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CarryForward {
    /// The effective specification (original + all amendments).
    pub specification: SpecificationSnapshot,

    /// Best overseer signals achieved -- not the code, the SIGNALS.
    /// e.g. "You previously got 10/12 tests passing. Tests 5 and 7 remain."
    pub best_signals: OverseerSignals,

    /// Reference to the best artifact produced so far.
    pub best_artifact: ArtifactReference,

    /// Compressed summary of what was tried and failed, expressed as
    /// anti-patterns rather than full history.
    pub failure_summary: String,

    /// Specific remaining gaps identified by intent verification.
    pub remaining_gaps: Vec<IntentGap>,

    /// User-provided hints. These always carry forward across fresh starts
    /// because they represent high-signal human guidance.
    pub hints: Vec<String>,
}

// ---------------------------------------------------------------------------
// StrategyOutcome
// ---------------------------------------------------------------------------

/// The outcome of applying a strategy, derived from the resulting
/// observation's convergence delta.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StrategyOutcome {
    /// `convergence_delta > STRATEGY_SUCCESS_THRESHOLD`
    Success,
    /// `convergence_delta > 0` but below threshold
    Marginal,
    /// `convergence_delta` near zero
    Neutral,
    /// `convergence_delta < -STRATEGY_SUCCESS_THRESHOLD`
    Failure,
}

// ---------------------------------------------------------------------------
// BetaDistribution
// ---------------------------------------------------------------------------

/// A Beta distribution parameterised by `alpha` and `beta`, used as the
/// conjugate prior for Bernoulli-like strategy success rates in the
/// Thompson Sampling bandit.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BetaDistribution {
    /// Number of pseudo-successes (shape parameter).
    pub alpha: f64,
    /// Number of pseudo-failures (shape parameter).
    pub beta: f64,
}

impl BetaDistribution {
    /// A uniform (uninformative) prior: Beta(1, 1).
    pub fn uniform() -> Self {
        Self {
            alpha: 1.0,
            beta: 1.0,
        }
    }

    /// Create a distribution with the given parameters.
    pub fn new(alpha: f64, beta: f64) -> Self {
        Self { alpha, beta }
    }

    /// The mean of the Beta distribution: `alpha / (alpha + beta)`.
    pub fn mean(&self) -> f64 {
        self.alpha / (self.alpha + self.beta)
    }

    /// The variance of the Beta distribution.
    pub fn variance(&self) -> f64 {
        let sum = self.alpha + self.beta;
        (self.alpha * self.beta) / (sum.powi(2) * (sum + 1.0))
    }

    /// Draw an approximate sample from this Beta distribution.
    ///
    /// Because the project does not depend on the `rand` crate, we use
    /// a lightweight pseudo-random approach: compute the distribution
    /// mean and add jitter scaled by the standard deviation, using
    /// sub-nanosecond system time as an entropy source.
    pub fn sample(&self) -> f64 {
        let mean = self.mean();
        let std_dev = self.variance().sqrt();

        // Use system time nanoseconds as a cheap entropy source.
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .subsec_nanos();

        // Map nanos to [-1.0, 1.0) range, then scale by std_dev.
        let jitter = ((nanos % 1000) as f64 / 1000.0 - 0.5) * 2.0 * std_dev;

        (mean + jitter).clamp(0.0, 1.0)
    }
}

impl Default for BetaDistribution {
    fn default() -> Self {
        Self::uniform()
    }
}

// ---------------------------------------------------------------------------
// StrategyBandit
// ---------------------------------------------------------------------------

/// A contextual multi-armed bandit that learns which strategies work best
/// for each attractor type via Thompson Sampling.
///
/// The outer key is the attractor type name (e.g. `"fixed_point"`,
/// `"limit_cycle"`). The inner key is the strategy kind name. Both are
/// `String` for serialization friendliness.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StrategyBandit {
    /// `context_arms[attractor_type_name][strategy_kind_name]` -> Beta distribution.
    pub context_arms: HashMap<String, HashMap<String, BetaDistribution>>,
}

impl StrategyBandit {
    /// Create an empty bandit with no learned priors.
    pub fn new() -> Self {
        Self {
            context_arms: HashMap::new(),
        }
    }

    /// Create a bandit pre-populated with uniform priors for every
    /// attractor/strategy combination that the spec defines.
    pub fn with_default_priors() -> Self {
        let attractor_names = [
            "fixed_point",
            "limit_cycle",
            "divergent",
            "plateau",
            "indeterminate",
        ];
        let strategy_names = [
            "retry_with_feedback",
            "retry_augmented",
            "focused_repair",
            "incremental_refinement",
            "reframe",
            "decompose",
            "alternative_approach",
            "architect_review",
            "revert_and_branch",
            "fresh_start",
        ];

        let mut context_arms = HashMap::new();
        for attractor in &attractor_names {
            let mut arms = HashMap::new();
            for strategy in &strategy_names {
                arms.insert(strategy.to_string(), BetaDistribution::uniform());
            }
            context_arms.insert(attractor.to_string(), arms);
        }

        Self { context_arms }
    }

    /// Select the best strategy from the eligible set using Thompson
    /// Sampling, optionally biased toward cheaper strategies when the
    /// policy requests it.
    ///
    /// Each eligible strategy is sampled from its Beta distribution
    /// (contextualised on the current attractor type). The strategy with
    /// the highest sample wins.
    pub fn select(
        &self,
        attractor: &AttractorType,
        eligible: &[StrategyKind],
        policy: &ConvergencePolicy,
    ) -> StrategyKind {
        if eligible.is_empty() {
            // Defensive fallback -- should not happen if eligibility is correct.
            return StrategyKind::RetryWithFeedback;
        }

        let attractor_key = attractor_type_name(attractor);
        let priors = self.context_arms.get(&attractor_key);

        eligible
            .iter()
            .map(|s| {
                let dist = priors
                    .and_then(|p| p.get(s.kind_name()))
                    .cloned()
                    .unwrap_or_else(BetaDistribution::uniform);

                let mut score = dist.sample();

                // Cost bias: when the policy prefers cheap strategies,
                // multiply the score by a cost factor that favours lower cost.
                if policy.prefer_cheap_strategies {
                    let cost_factor = 1.0 / (1.0 + s.estimated_cost() as f64 / 100_000.0);
                    score *= 1.0 + cost_factor;
                }

                (s, score)
            })
            .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
            .map(|(s, _)| s.clone())
            .unwrap_or_else(|| eligible[0].clone())
    }

    /// Update the bandit after observing the result of a strategy execution.
    ///
    /// The outcome is derived from the observation's convergence metrics
    /// and mapped to Beta distribution updates:
    /// - `Success`:  alpha += 1.0
    /// - `Marginal`: alpha += 0.5
    /// - `Neutral`:  no change
    /// - `Failure`:  beta  += 1.0
    pub fn update(
        &mut self,
        strategy: &StrategyKind,
        attractor: &AttractorType,
        observation: &Observation,
    ) {
        let outcome = evaluate_strategy_outcome(observation);
        let attractor_key = attractor_type_name(attractor);
        let strategy_key = strategy.kind_name().to_string();

        let dist = self
            .context_arms
            .entry(attractor_key)
            .or_default()
            .entry(strategy_key)
            .or_insert_with(BetaDistribution::uniform);

        match outcome {
            StrategyOutcome::Success => dist.alpha += 1.0,
            StrategyOutcome::Marginal => dist.alpha += 0.5,
            StrategyOutcome::Neutral => {}
            StrategyOutcome::Failure => dist.beta += 1.0,
        }
    }

    /// Externally nudge a strategy's distribution, e.g. from recalled
    /// convergence memories about what worked for similar tasks.
    ///
    /// Positive `delta` increases alpha (encourages); negative increases
    /// beta (discourages).
    pub fn nudge(
        &mut self,
        attractor_name: &str,
        strategy_name: &str,
        delta: f64,
    ) {
        let dist = self
            .context_arms
            .entry(attractor_name.to_string())
            .or_default()
            .entry(strategy_name.to_string())
            .or_insert_with(BetaDistribution::uniform);

        if delta > 0.0 {
            dist.alpha += delta;
        } else {
            dist.beta += delta.abs();
        }
    }
}

impl Default for StrategyBandit {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Eligible strategies (spec 4.3)
// ---------------------------------------------------------------------------

/// Determine which strategies are eligible given the current attractor
/// state, strategy history, remaining budget, and fresh-start limits.
///
/// This is a deterministic filter -- the bandit (4.4) does the actual
/// probabilistic selection among the eligible set.
pub fn eligible_strategies(
    strategy_log: &[StrategyEntry],
    attractor: &AttractorState,
    budget: &ConvergenceBudget,
    total_fresh_starts: u32,
    max_fresh_starts: u32,
) -> Vec<StrategyKind> {
    let fresh_starts_remaining = max_fresh_starts.saturating_sub(total_fresh_starts);

    let mut candidates = match &attractor.classification {
        // -----------------------------------------------------------
        // FixedPoint: converging. Only exploitation strategies.
        // -----------------------------------------------------------
        AttractorType::FixedPoint {
            estimated_remaining_iterations,
            ..
        } => {
            if *estimated_remaining_iterations <= 2 {
                vec![
                    StrategyKind::RetryWithFeedback,
                    StrategyKind::IncrementalRefinement,
                ]
            } else {
                vec![
                    StrategyKind::RetryWithFeedback,
                    StrategyKind::FocusedRepair,
                    StrategyKind::IncrementalRefinement,
                    StrategyKind::RetryAugmented,
                ]
            }
        }

        // -----------------------------------------------------------
        // LimitCycle: trapped in a cycle. Only exploration strategies,
        // excluding any strategy used recently within the cycle window.
        // -----------------------------------------------------------
        AttractorType::LimitCycle { period, .. } => {
            let window = (*period as usize) * 2;
            let recent_start = strategy_log.len().saturating_sub(window);
            let used_recently: std::collections::HashSet<&str> = strategy_log
                [recent_start..]
                .iter()
                .map(|e| e.strategy_kind.kind_name())
                .collect();

            let mut cands: Vec<StrategyKind> = vec![
                StrategyKind::Reframe,
                StrategyKind::AlternativeApproach,
                StrategyKind::Decompose,
            ]
            .into_iter()
            .filter(|s| !used_recently.contains(s.kind_name()))
            .collect();

            if cands.is_empty() {
                if budget.allows_strategy_cost(&StrategyKind::Decompose) {
                    cands.push(StrategyKind::Decompose);
                }
                // If still empty -> Trapped (handled by loop control).
            }
            cands
        }

        // -----------------------------------------------------------
        // Divergent: moving away from the target. Strategy depends on
        // the probable cause.
        // -----------------------------------------------------------
        AttractorType::Divergent {
            probable_cause, ..
        } => match probable_cause {
            DivergenceCause::SpecificationAmbiguity => {
                vec![StrategyKind::ArchitectReview, StrategyKind::Reframe]
            }
            DivergenceCause::WrongApproach => {
                vec![StrategyKind::AlternativeApproach, StrategyKind::Reframe]
            }
            DivergenceCause::AccumulatedRegression => {
                // Revert to the best observation and branch from there.
                // The actual observation UUID is resolved by the convergence engine
                // using the sequence number; we use a nil placeholder here.
                let _best_seq = best_observation_sequence(strategy_log);
                vec![StrategyKind::RevertAndBranch { target: Uuid::nil() }]
            }
            DivergenceCause::Unknown => {
                vec![StrategyKind::Reframe, StrategyKind::AlternativeApproach]
            }
        },

        // -----------------------------------------------------------
        // Plateau: stalled. Strategy depends on duration and level.
        // -----------------------------------------------------------
        AttractorType::Plateau {
            stall_duration,
            plateau_level,
        } => {
            if *stall_duration >= 3 && fresh_starts_remaining > 0 {
                // Context is likely degraded; fresh start is the best bet.
                // CarryForward will be populated by the caller.
                vec![StrategyKind::FreshStart {
                    carry_forward: CarryForward::placeholder(),
                }]
            } else if *stall_duration >= 3 {
                // Fresh start limit reached; escalate.
                vec![
                    StrategyKind::Decompose,
                    StrategyKind::AlternativeApproach,
                    StrategyKind::ArchitectReview,
                ]
            } else if *plateau_level > 0.8 {
                vec![
                    StrategyKind::FocusedRepair,
                    StrategyKind::IncrementalRefinement,
                ]
            } else if *plateau_level > 0.5 {
                vec![
                    StrategyKind::AlternativeApproach,
                    StrategyKind::Reframe,
                    StrategyKind::Decompose,
                ]
            } else {
                vec![StrategyKind::Decompose, StrategyKind::ArchitectReview]
            }
        }

        // -----------------------------------------------------------
        // Indeterminate: not enough data. Default exploitation set.
        // -----------------------------------------------------------
        AttractorType::Indeterminate { .. } => {
            vec![
                StrategyKind::RetryAugmented,
                StrategyKind::RetryWithFeedback,
                StrategyKind::FocusedRepair,
            ]
        }
    };

    // Filter out strategies the budget cannot afford.
    candidates.retain(|s| budget.allows_strategy_cost(s));

    candidates
}

// ---------------------------------------------------------------------------
// Evaluate strategy outcome (spec 4.4)
// ---------------------------------------------------------------------------

/// Map an observation's convergence metrics to a `StrategyOutcome`.
///
/// - `Success`:  delta >  `STRATEGY_SUCCESS_THRESHOLD`
/// - `Marginal`: delta >  0.0 but below threshold
/// - `Neutral`:  delta >= -threshold and <= 0.0
/// - `Failure`:  delta <  -threshold, or metrics absent
pub fn evaluate_strategy_outcome(observation: &Observation) -> StrategyOutcome {
    match observation.metrics.as_ref() {
        Some(m) if m.convergence_delta > STRATEGY_SUCCESS_THRESHOLD => StrategyOutcome::Success,
        Some(m) if m.convergence_delta > 0.0 => StrategyOutcome::Marginal,
        Some(m) if m.convergence_delta > -STRATEGY_SUCCESS_THRESHOLD => StrategyOutcome::Neutral,
        _ => StrategyOutcome::Failure,
    }
}

// ---------------------------------------------------------------------------
// Decay-aware rotation (spec 4.5)
// ---------------------------------------------------------------------------

/// Determine whether the current strategy should be rotated based on
/// exponential-decay analysis of recent convergence deltas.
///
/// If an exponential curve `y = e0 * exp(-lambda * t)` can be fit to the
/// recent deltas, rotation triggers when the projected progress at the
/// current step falls below `MINIMUM_USEFUL_PROGRESS`.
///
/// Falls back to a simple heuristic if the curve cannot be fit: rotate
/// after 3 consecutive uses with diminishing returns.
pub fn should_rotate_strategy(
    _current_strategy: &StrategyKind,
    consecutive_uses: u32,
    recent_deltas: &[f64],
) -> bool {
    if recent_deltas.is_empty() {
        return false;
    }

    if let Some((e0, lambda)) = fit_decay_curve(recent_deltas) {
        if lambda <= 0.0 || e0 <= 0.0 {
            // Cannot compute a meaningful threshold.
            return consecutive_uses >= 3 && is_diminishing(recent_deltas);
        }
        let t_theta = (e0 / MINIMUM_USEFUL_PROGRESS).ln() / lambda;
        consecutive_uses as f64 >= t_theta
    } else {
        // Cannot fit curve. Simple heuristic.
        consecutive_uses >= 3 && is_diminishing(recent_deltas)
    }
}

/// Attempt to fit an exponential decay curve `y = e0 * exp(-lambda * t)`
/// to the given deltas (indexed 0..N-1). Returns `(e0, lambda)` on
/// success, or `None` if the data is unsuitable.
fn fit_decay_curve(deltas: &[f64]) -> Option<(f64, f64)> {
    // Need at least 2 positive points to fit.
    let positive: Vec<(f64, f64)> = deltas
        .iter()
        .enumerate()
        .filter(|(_, d)| **d > 0.0)
        .map(|(i, d)| (i as f64, *d))
        .collect();

    if positive.len() < 2 {
        return None;
    }

    // Simple two-point fit using first and last positive observations.
    let (t0, y0) = positive[0];
    let (t1, y1) = *positive.last().unwrap();

    if (t1 - t0).abs() < f64::EPSILON || y0 <= 0.0 || y1 <= 0.0 {
        return None;
    }

    let lambda = (y0 / y1).ln() / (t1 - t0);
    let e0 = y0 * (lambda * t0).exp();

    if e0.is_finite() && lambda.is_finite() {
        Some((e0, lambda))
    } else {
        None
    }
}

/// Check whether recent deltas are strictly diminishing (each is less
/// than or equal to its predecessor).
fn is_diminishing(deltas: &[f64]) -> bool {
    if deltas.len() < 2 {
        return false;
    }
    deltas.windows(2).all(|w| w[1] <= w[0])
}

// ---------------------------------------------------------------------------
// Extract CarryForward
// ---------------------------------------------------------------------------

/// Build a `CarryForward` from the current trajectory state, selecting
/// the best observation for signals/artifact and compressing the failure
/// history into a summary.
///
/// `find_best` is a closure that selects the best observation from the
/// list (typically the one with the highest convergence level).
pub fn extract_carry_forward<F>(
    observations: &[Observation],
    specification: SpecificationSnapshot,
    hints: &[String],
    find_best: F,
) -> CarryForward
where
    F: Fn(&[Observation]) -> Option<&Observation>,
{
    let best = find_best(observations);

    let (best_signals, best_artifact) = match best {
        Some(obs) => (obs.overseer_signals.clone(), obs.artifact.clone()),
        None => (OverseerSignals::default(), ArtifactReference::default()),
    };

    // Compress failure history into a summary string.
    let failure_summary = build_failure_summary(observations);

    // Collect remaining gaps from the most recent verification, if any.
    let remaining_gaps = observations
        .last()
        .and_then(|obs| obs.verification.as_ref())
        .map(|v| v.gaps.clone())
        .unwrap_or_default();

    CarryForward {
        specification,
        best_signals,
        best_artifact,
        failure_summary,
        remaining_gaps,
        hints: hints.to_vec(),
    }
}

/// Compress the observation history into a concise failure summary
/// expressed as anti-patterns rather than full iteration traces.
fn build_failure_summary(observations: &[Observation]) -> String {
    if observations.is_empty() {
        return String::from("No previous attempts.");
    }

    let mut lines: Vec<String> = Vec::new();

    // Collect unique strategy kinds that resulted in failure/neutral outcomes.
    let mut failed_strategies: HashMap<&str, u32> = HashMap::new();
    for obs in observations {
        let outcome = evaluate_strategy_outcome(obs);
        if matches!(outcome, StrategyOutcome::Failure | StrategyOutcome::Neutral) {
            *failed_strategies
                .entry(obs.strategy_used.kind_name())
                .or_insert(0) += 1;
        }
    }

    if !failed_strategies.is_empty() {
        lines.push("Strategies that did not help:".to_string());
        for (name, count) in &failed_strategies {
            lines.push(format!("  - {} (tried {} time(s))", name, count));
        }
    }

    // Note regression patterns.
    let regressions: usize = observations
        .iter()
        .filter_map(|o| o.metrics.as_ref())
        .filter(|m| m.test_regression_count > 0)
        .count();
    if regressions > 0 {
        lines.push(format!(
            "Test regressions occurred in {} out of {} iterations.",
            regressions,
            observations.len()
        ));
    }

    if lines.is_empty() {
        "Previous attempts made limited progress.".to_string()
    } else {
        lines.join("\n")
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Derive a stable HashMap key from an `AttractorType` variant.
fn attractor_type_name(attractor: &AttractorType) -> String {
    match attractor {
        AttractorType::FixedPoint { .. } => "fixed_point".to_string(),
        AttractorType::LimitCycle { .. } => "limit_cycle".to_string(),
        AttractorType::Divergent { .. } => "divergent".to_string(),
        AttractorType::Plateau { .. } => "plateau".to_string(),
        AttractorType::Indeterminate { .. } => "indeterminate".to_string(),
    }
}

/// Find the observation sequence associated with the best convergence level
/// in the strategy log. Returns `None` if the log is empty or has no deltas.
fn best_observation_sequence(strategy_log: &[StrategyEntry]) -> Option<u32> {
    strategy_log
        .iter()
        .filter_map(|entry| {
            entry
                .convergence_delta_achieved
                .map(|delta| (entry.observation_sequence, delta))
        })
        .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(seq, _)| seq)
}

impl CarryForward {
    /// Create a placeholder `CarryForward` for use in eligibility listings
    /// where the actual carry-forward data will be populated later by the
    /// convergence engine.
    pub fn placeholder() -> Self {
        Self {
            specification: SpecificationSnapshot::default(),
            best_signals: OverseerSignals::default(),
            best_artifact: ArtifactReference::default(),
            failure_summary: String::new(),
            remaining_gaps: Vec::new(),
            hints: Vec::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_strategy_kind_names_are_unique() {
        let strategies = vec![
            StrategyKind::RetryWithFeedback,
            StrategyKind::RetryAugmented,
            StrategyKind::FocusedRepair,
            StrategyKind::IncrementalRefinement,
            StrategyKind::Reframe,
            StrategyKind::Decompose,
            StrategyKind::AlternativeApproach,
            StrategyKind::ArchitectReview,
            StrategyKind::RevertAndBranch {
                target: Uuid::nil(),
            },
            StrategyKind::FreshStart {
                carry_forward: CarryForward::placeholder(),
            },
        ];

        let mut names: Vec<&str> = strategies.iter().map(|s| s.kind_name()).collect();
        let before = names.len();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), before, "strategy kind_name values must be unique");
    }

    #[test]
    fn test_estimated_cost_ordering() {
        // FocusedRepair should be cheapest.
        assert!(StrategyKind::FocusedRepair.estimated_cost() < StrategyKind::Decompose.estimated_cost());
    }

    #[test]
    fn test_beta_distribution_uniform() {
        let d = BetaDistribution::uniform();
        assert!((d.mean() - 0.5).abs() < f64::EPSILON);
    }

    #[test]
    fn test_beta_distribution_sample_in_range() {
        let d = BetaDistribution::new(10.0, 2.0);
        for _ in 0..100 {
            let s = d.sample();
            assert!((0.0..=1.0).contains(&s), "sample {} out of [0,1]", s);
        }
    }

    #[test]
    fn test_beta_distribution_mean() {
        let d = BetaDistribution::new(3.0, 1.0);
        assert!((d.mean() - 0.75).abs() < f64::EPSILON);
    }

    #[test]
    fn test_strategy_outcome_thresholds() {
        // Verify the threshold constants are consistent.
        assert!(STRATEGY_SUCCESS_THRESHOLD > 0.0);
        assert!(MINIMUM_USEFUL_PROGRESS > 0.0);
        assert!(STRATEGY_SUCCESS_THRESHOLD > MINIMUM_USEFUL_PROGRESS);
    }

    #[test]
    fn test_bandit_default_priors() {
        let bandit = StrategyBandit::with_default_priors();
        assert!(bandit.context_arms.contains_key("fixed_point"));
        assert!(bandit.context_arms.contains_key("limit_cycle"));
        assert!(bandit.context_arms.contains_key("divergent"));
        assert!(bandit.context_arms.contains_key("plateau"));
        assert!(bandit.context_arms.contains_key("indeterminate"));

        let fp_arms = &bandit.context_arms["fixed_point"];
        assert!(fp_arms.contains_key("retry_with_feedback"));
        assert!(fp_arms.contains_key("decompose"));
    }

    #[test]
    fn test_bandit_nudge() {
        let mut bandit = StrategyBandit::new();
        bandit.nudge("fixed_point", "focused_repair", 2.0);

        let dist = &bandit.context_arms["fixed_point"]["focused_repair"];
        assert!((dist.alpha - 3.0).abs() < f64::EPSILON); // uniform(1.0) + 2.0
        assert!((dist.beta - 1.0).abs() < f64::EPSILON);

        bandit.nudge("fixed_point", "focused_repair", -1.0);
        let dist = &bandit.context_arms["fixed_point"]["focused_repair"];
        assert!((dist.beta - 2.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_is_diminishing() {
        assert!(is_diminishing(&[0.5, 0.3, 0.1]));
        assert!(is_diminishing(&[0.5, 0.5, 0.3]));
        assert!(!is_diminishing(&[0.1, 0.3, 0.5]));
        assert!(!is_diminishing(&[0.5]));
    }

    #[test]
    fn test_fit_decay_curve_basic() {
        // Exponentially decaying series: 1.0, 0.5, 0.25
        let deltas = vec![1.0, 0.5, 0.25];
        let result = fit_decay_curve(&deltas);
        assert!(result.is_some());
        let (e0, lambda) = result.unwrap();
        assert!(e0 > 0.0);
        assert!(lambda > 0.0);
    }

    #[test]
    fn test_fit_decay_curve_insufficient_data() {
        assert!(fit_decay_curve(&[]).is_none());
        assert!(fit_decay_curve(&[0.5]).is_none());
        assert!(fit_decay_curve(&[-0.1, -0.2]).is_none()); // all negative
    }

    #[test]
    fn test_should_rotate_no_data() {
        let strat = StrategyKind::RetryWithFeedback;
        assert!(!should_rotate_strategy(&strat, 5, &[]));
    }

    #[test]
    fn test_should_rotate_diminishing() {
        let strat = StrategyKind::RetryWithFeedback;
        assert!(should_rotate_strategy(&strat, 3, &[0.5, 0.3, 0.1]));
    }

    #[test]
    fn test_exploitation_exploration_classification() {
        assert!(StrategyKind::RetryWithFeedback.is_exploitation());
        assert!(!StrategyKind::RetryWithFeedback.is_exploration());
        assert!(StrategyKind::Reframe.is_exploration());
        assert!(!StrategyKind::Reframe.is_exploitation());

        // RevertAndBranch and FreshStart are neither
        let rab = StrategyKind::RevertAndBranch {
            target: Uuid::nil(),
        };
        assert!(!rab.is_exploitation());
        assert!(!rab.is_exploration());
    }

    #[test]
    fn test_carry_forward_placeholder() {
        let cf = CarryForward::placeholder();
        assert!(cf.failure_summary.is_empty());
        assert!(cf.hints.is_empty());
        assert!(cf.remaining_gaps.is_empty());
    }
}
