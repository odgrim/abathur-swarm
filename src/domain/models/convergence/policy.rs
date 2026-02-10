//! Convergence policy and priority hints.
//!
//! Implements the convergence policy (spec 1.7) and priority hint system (spec 7.2).
//!
//! The [`ConvergencePolicy`] governs convergence behavior for a trajectory. It is
//! assembled from basin width estimation, priority hints, and task complexity during
//! the SETUP phase. It is never configured directly by the user.
//!
//! [`PriorityHint`] allows users to express intent about the convergence tradeoff
//! space. Priority hints adjust both the budget and policy, composing with basin
//! width adjustments -- basin width runs first, then priority hints overlay.

use serde::{Deserialize, Serialize};

use super::budget::ConvergenceBudget;

/// Governs convergence behavior for a trajectory.
///
/// Assembled during SETUP from basin width estimation, priority hints, and
/// inferred complexity. Fields control the exploitation/exploration balance,
/// acceptance criteria, overseer selection, and fresh start limits.
///
/// This struct is never configured directly by the user. Instead, it is
/// composed from [`PriorityHint`], basin width classification, and task
/// complexity through their respective `apply` methods.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConvergencePolicy {
    /// Exploitation vs exploration balance for strategy selection.
    ///
    /// `0.0` = pure exploitation (always pick the best-known strategy),
    /// `1.0` = pure exploration (always try something new).
    /// Used as the exploration parameter in Thompson Sampling.
    pub exploration_weight: f64,

    /// Minimum convergence level to accept as "done."
    ///
    /// A trajectory must reach this level before the engine will classify it
    /// as having reached a fixed-point attractor and accept the result.
    pub acceptance_threshold: f64,

    /// Whether to accept the best result when the budget is exhausted,
    /// provided it meets [`partial_threshold`](Self::partial_threshold).
    ///
    /// When `true`, the engine will return the best observation rather than
    /// reporting failure, as long as it exceeds the partial threshold.
    pub partial_acceptance: bool,

    /// Minimum convergence level for partial acceptance.
    ///
    /// Only relevant when [`partial_acceptance`](Self::partial_acceptance) is `true`.
    /// If the best observation's convergence level is below this threshold,
    /// the trajectory is reported as failed even with partial acceptance enabled.
    pub partial_threshold: f64,

    /// Whether to skip expensive overseers (full test suite, integration tests).
    ///
    /// When `true`, the engine will only use cheap/fast overseers for evaluation.
    /// This trades accuracy for speed and cost savings.
    pub skip_expensive_overseers: bool,

    /// Whether to generate additional acceptance tests during the PREPARE phase.
    ///
    /// When `true`, the engine will use an LLM to generate acceptance tests from
    /// the task specification before entering the convergence loop. These tests
    /// serve as additional deterministic overseers.
    pub generate_acceptance_tests: bool,

    /// How often to run LLM-based intent verification (every Nth iteration).
    ///
    /// Intent verification checks whether the trajectory is still aligned with
    /// the original task specification. Higher values reduce cost but risk
    /// specification drift going undetected.
    pub intent_verification_frequency: u32,

    /// Whether to prefer cheaper strategies in bandit selection.
    ///
    /// When `true`, the strategy bandit applies a cost-weighted penalty to
    /// expensive strategies, biasing selection toward cheaper alternatives.
    pub prefer_cheap_strategies: bool,

    /// Priority hint that affects intervention behavior and convergence tuning.
    ///
    /// Set from the user's task submission. When present, the hint's [`apply`](PriorityHint::apply)
    /// method adjusts both the policy and budget during SETUP.
    pub priority_hint: Option<PriorityHint>,

    /// Maximum total fresh starts before escalating.
    ///
    /// A fresh start discards the current trajectory and begins again from
    /// scratch. This cap prevents unbounded restarts; once reached, the
    /// engine must either accept a partial result or escalate.
    pub max_fresh_starts: u32,
}

impl Default for ConvergencePolicy {
    fn default() -> Self {
        Self {
            exploration_weight: 0.3,
            acceptance_threshold: 0.95,
            partial_acceptance: true,
            partial_threshold: 0.7,
            skip_expensive_overseers: false,
            generate_acceptance_tests: false,
            intent_verification_frequency: 2,
            prefer_cheap_strategies: false,
            priority_hint: None,
            max_fresh_starts: 3,
        }
    }
}

/// Priority hint expressing user intent about convergence tradeoffs (spec 7.2).
///
/// Priority hints adjust both the [`ConvergenceBudget`] and [`ConvergencePolicy`]
/// during the SETUP phase. They compose with basin width adjustments -- basin width
/// runs first, then priority hints overlay on top.
///
/// # Variants
///
/// - **Fast**: Minimize wall-clock time. Caps iterations, lowers acceptance
///   threshold, skips expensive overseers, and reduces exploration.
/// - **Thorough**: Maximize quality. Increases extensions, raises acceptance
///   threshold, enables acceptance test generation, and increases exploration.
/// - **Cheap**: Minimize token/cost spend. Reduces token budget, skips expensive
///   overseers, reduces verification frequency, and prefers cheap strategies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PriorityHint {
    /// Minimize wall-clock time at the expense of thoroughness.
    Fast,
    /// Maximize quality at the expense of time and cost.
    Thorough,
    /// Minimize token and cost spend at the expense of quality.
    Cheap,
}

impl PriorityHint {
    /// Apply this priority hint to a convergence policy and budget.
    ///
    /// This method implements the exact adjustments from spec 7.2. It is called
    /// during SETUP after basin width adjustments have already been applied, so
    /// the values it sets overlay on top of basin-adjusted defaults.
    ///
    /// # Adjustments by variant
    ///
    /// ## Fast
    /// - `budget.max_iterations` capped at 5 (takes the minimum of current and 5)
    /// - `policy.acceptance_threshold` set to 0.85
    /// - `policy.skip_expensive_overseers` set to `true`
    /// - `policy.partial_acceptance` set to `true`
    /// - `policy.exploration_weight` set to 0.1
    ///
    /// ## Thorough
    /// - `budget.max_extensions` increased by 2
    /// - `policy.acceptance_threshold` set to 0.98
    /// - `policy.skip_expensive_overseers` set to `false`
    /// - `policy.partial_acceptance` set to `false`
    /// - `policy.exploration_weight` set to 0.4
    /// - `policy.generate_acceptance_tests` set to `true`
    ///
    /// ## Cheap
    /// - `budget.max_tokens` scaled to 70% of current value
    /// - `policy.skip_expensive_overseers` set to `true`
    /// - `policy.intent_verification_frequency` set to 3
    /// - `policy.prefer_cheap_strategies` set to `true`
    pub fn apply(&self, policy: &mut ConvergencePolicy, budget: &mut ConvergenceBudget) {
        match self {
            PriorityHint::Fast => {
                budget.max_iterations = budget.max_iterations.min(5);
                policy.acceptance_threshold = 0.85;
                policy.skip_expensive_overseers = true;
                policy.partial_acceptance = true;
                policy.exploration_weight = 0.1;
            }
            PriorityHint::Thorough => {
                budget.max_extensions += 2;
                policy.acceptance_threshold = 0.98;
                policy.skip_expensive_overseers = false;
                policy.partial_acceptance = false;
                policy.exploration_weight = 0.4;
                policy.generate_acceptance_tests = true;
            }
            PriorityHint::Cheap => {
                budget.max_tokens = (budget.max_tokens as f64 * 0.7) as u64;
                policy.skip_expensive_overseers = true;
                policy.intent_verification_frequency = 3;
                policy.prefer_cheap_strategies = true;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_policy_values() {
        let policy = ConvergencePolicy::default();
        assert!((policy.exploration_weight - 0.3).abs() < f64::EPSILON);
        assert!((policy.acceptance_threshold - 0.95).abs() < f64::EPSILON);
        assert!(policy.partial_acceptance);
        assert!((policy.partial_threshold - 0.7).abs() < f64::EPSILON);
        assert!(!policy.skip_expensive_overseers);
        assert!(!policy.generate_acceptance_tests);
        assert_eq!(policy.intent_verification_frequency, 2);
        assert!(!policy.prefer_cheap_strategies);
        assert!(policy.priority_hint.is_none());
        assert_eq!(policy.max_fresh_starts, 3);
    }

    #[test]
    fn test_fast_hint_caps_iterations() {
        let mut policy = ConvergencePolicy::default();
        let mut budget = ConvergenceBudget::default();
        budget.max_iterations = 12;

        PriorityHint::Fast.apply(&mut policy, &mut budget);

        assert_eq!(budget.max_iterations, 5);
        assert!((policy.acceptance_threshold - 0.85).abs() < f64::EPSILON);
        assert!(policy.skip_expensive_overseers);
        assert!(policy.partial_acceptance);
        assert!((policy.exploration_weight - 0.1).abs() < f64::EPSILON);
    }

    #[test]
    fn test_fast_hint_preserves_lower_iteration_cap() {
        let mut policy = ConvergencePolicy::default();
        let mut budget = ConvergenceBudget::default();
        budget.max_iterations = 3;

        PriorityHint::Fast.apply(&mut policy, &mut budget);

        // min(3, 5) = 3; should not increase iterations
        assert_eq!(budget.max_iterations, 3);
    }

    #[test]
    fn test_thorough_hint_increases_extensions() {
        let mut policy = ConvergencePolicy::default();
        let mut budget = ConvergenceBudget::default();
        budget.max_extensions = 1;

        PriorityHint::Thorough.apply(&mut policy, &mut budget);

        assert_eq!(budget.max_extensions, 3);
        assert!((policy.acceptance_threshold - 0.98).abs() < f64::EPSILON);
        assert!(!policy.skip_expensive_overseers);
        assert!(!policy.partial_acceptance);
        assert!((policy.exploration_weight - 0.4).abs() < f64::EPSILON);
        assert!(policy.generate_acceptance_tests);
    }

    #[test]
    fn test_cheap_hint_reduces_tokens() {
        let mut policy = ConvergencePolicy::default();
        let mut budget = ConvergenceBudget::default();
        budget.max_tokens = 100_000;

        PriorityHint::Cheap.apply(&mut policy, &mut budget);

        assert_eq!(budget.max_tokens, 70_000);
        assert!(policy.skip_expensive_overseers);
        assert_eq!(policy.intent_verification_frequency, 3);
        assert!(policy.prefer_cheap_strategies);
    }

    #[test]
    fn test_priority_hint_serialization() {
        assert_eq!(
            serde_json::to_string(&PriorityHint::Fast).unwrap(),
            "\"fast\""
        );
        assert_eq!(
            serde_json::to_string(&PriorityHint::Thorough).unwrap(),
            "\"thorough\""
        );
        assert_eq!(
            serde_json::to_string(&PriorityHint::Cheap).unwrap(),
            "\"cheap\""
        );
    }

    #[test]
    fn test_priority_hint_deserialization() {
        let fast: PriorityHint = serde_json::from_str("\"fast\"").unwrap();
        assert_eq!(fast, PriorityHint::Fast);

        let thorough: PriorityHint = serde_json::from_str("\"thorough\"").unwrap();
        assert_eq!(thorough, PriorityHint::Thorough);

        let cheap: PriorityHint = serde_json::from_str("\"cheap\"").unwrap();
        assert_eq!(cheap, PriorityHint::Cheap);
    }

    #[test]
    fn test_policy_serialization_roundtrip() {
        let policy = ConvergencePolicy {
            priority_hint: Some(PriorityHint::Thorough),
            ..Default::default()
        };
        let json = serde_json::to_string(&policy).unwrap();
        let deserialized: ConvergencePolicy = serde_json::from_str(&json).unwrap();

        assert!((deserialized.exploration_weight - policy.exploration_weight).abs() < f64::EPSILON);
        assert_eq!(deserialized.priority_hint, Some(PriorityHint::Thorough));
        assert_eq!(deserialized.max_fresh_starts, policy.max_fresh_starts);
    }
}
