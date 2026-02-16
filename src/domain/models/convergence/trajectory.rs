//! Core trajectory types for attractor-driven task convergence.
//!
//! A **Trajectory** is the sequence of attempts to satisfy a task. Each attempt
//! produces an **Observation** -- a snapshot of where the implementation stands
//! relative to the specification. The trajectory tracks the full convergence
//! lifecycle: strategy selection, attractor classification, budget consumption,
//! context health degradation, and specification evolution.
//!
//! Every task execution is a trajectory through solution space. The goal is not
//! "did it pass?" but "where is this trajectory heading?" Attractors are the
//! destinations -- some are correct implementations (fixed points we want), some
//! are oscillating failure modes (limit cycles we must escape). The system's job
//! is to detect which attractor a trajectory is approaching and intervene
//! accordingly.
//!
//! ## Key Types
//!
//! - [`Trajectory`] -- The unit of convergence. Owns the full lifecycle of a
//!   task's convergence process.
//! - [`Observation`] -- A point in solution space. Each iteration produces one,
//!   measured by overseers (external verification signals), never self-assessed.
//! - [`StrategyEntry`] -- A record of which strategy was tried, what it achieved,
//!   and how much budget it consumed.
//! - [`ConvergencePhase`] -- The current phase in the convergence lifecycle.
//! - [`VerificationResult`] -- LLM-based verification of intent satisfaction.
//! - [`ArtifactReference`] -- A reference to an artifact produced by a strategy
//!   execution.

use std::collections::HashSet;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::domain::models::intent_verification::{IntentGap, IntentVerificationResult};

use super::*;

// ---------------------------------------------------------------------------
// Trajectory
// ---------------------------------------------------------------------------

/// The unit of convergence -- tracks the full lifecycle of a task's iterative
/// convergence toward a correct implementation.
///
/// A trajectory is the sequence of attempts to satisfy a task. Each attempt
/// produces an [`Observation`]. The trajectory manages strategy selection,
/// attractor classification, budget consumption, context health, and
/// specification evolution across all iterations including fresh starts.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Trajectory {
    /// Unique identifier for this trajectory.
    pub id: Uuid,

    /// The task being converged on.
    pub task_id: Uuid,

    /// Optional parent goal that this task belongs to.
    pub goal_id: Option<Uuid>,

    /// The specification being converged toward (evolves via amendments).
    pub specification: SpecificationEvolution,

    /// Ordered sequence of observations, one per iteration.
    pub observations: Vec<Observation>,

    /// Current attractor classification for this trajectory.
    pub attractor_state: AttractorState,

    /// Convergence budget (multi-dimensional resource envelope).
    pub budget: ConvergenceBudget,

    /// Active convergence policy governing behavior.
    pub policy: ConvergencePolicy,

    /// Strategy history -- what was tried and what happened.
    pub strategy_log: Vec<StrategyEntry>,

    /// Phase in the convergence lifecycle.
    pub phase: ConvergencePhase,

    /// Health of the LLM's working context (degrades over iterations).
    pub context_health: ContextHealth,

    /// User-provided trajectory hints (always carry forward across fresh starts).
    pub hints: Vec<String>,

    /// When set, this strategy is executed next, bypassing normal selection.
    pub forced_strategy: Option<StrategyKind>,

    /// Total fresh starts in this trajectory (guards against infinite reset loops).
    pub total_fresh_starts: u32,

    /// Confidence from the second-to-last LLM intent verification, if any.
    /// Used together with `last_intent_confidence` to compute intent-weighted
    /// convergence delta.
    pub prev_intent_confidence: Option<f64>,

    /// Confidence from the most recent LLM intent verification, if any.
    /// Updated by the orchestrator after each intent check.
    pub last_intent_confidence: Option<f64>,

    /// When this trajectory was created.
    pub created_at: DateTime<Utc>,

    /// When this trajectory was last updated.
    pub updated_at: DateTime<Utc>,
}

impl Trajectory {
    /// Create a new trajectory for the given task.
    ///
    /// Initializes the trajectory in the `Preparing` phase with the provided
    /// specification, budget, and policy. The attractor state starts as
    /// `Indeterminate` and context health starts fresh. These evolve as
    /// observations are recorded during the convergence loop.
    pub fn new(
        task_id: Uuid,
        goal_id: Option<Uuid>,
        specification: SpecificationEvolution,
        budget: ConvergenceBudget,
        policy: ConvergencePolicy,
    ) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            task_id,
            goal_id,
            specification,
            observations: Vec::new(),
            attractor_state: AttractorState::default(),
            budget,
            policy,
            strategy_log: Vec::new(),
            phase: ConvergencePhase::Preparing,
            context_health: ContextHealth::default(),
            hints: Vec::new(),
            forced_strategy: None,
            total_fresh_starts: 0,
            prev_intent_confidence: None,
            last_intent_confidence: None,
            created_at: now,
            updated_at: now,
        }
    }

    /// Returns a reference to the most recent artifact, if any observations exist.
    pub fn latest_artifact(&self) -> Option<&ArtifactReference> {
        self.observations.last().map(|o| &o.artifact)
    }

    /// Returns a reference to the most recent overseer signals, if any
    /// observations exist.
    pub fn latest_overseer_signals(&self) -> Option<&OverseerSignals> {
        self.observations.last().map(|o| &o.overseer_signals)
    }

    /// Returns the convergence delta from the most recent observation that has
    /// computed metrics.
    ///
    /// The first observation has no delta (no previous reference point), so this
    /// searches backwards from the most recent observation for one with metrics.
    /// Returns `0.0` if no observations have computed metrics.
    pub fn latest_convergence_delta(&self) -> f64 {
        self.observations
            .iter()
            .rev()
            .find_map(|o| o.metrics.as_ref().map(|m| m.convergence_delta))
            .unwrap_or(0.0)
    }

    /// Whether this trajectory has reached the `Converged` phase.
    pub fn is_converged(&self) -> bool {
        matches!(self.phase, ConvergencePhase::Converged)
    }

    /// Returns the ordered sequence of strategies that were actually used,
    /// deduplicated by consecutive runs of the same strategy kind.
    ///
    /// This is useful for convergence memory -- understanding *which path*
    /// led to convergence (or failure) without the noise of repeated strategies.
    /// Uses `kind_name()` to compare strategies since `StrategyKind` variants
    /// may carry data.
    pub fn effective_strategy_sequence(&self) -> Vec<StrategyKind> {
        let mut sequence: Vec<StrategyKind> = Vec::new();
        for entry in &self.strategy_log {
            let dominated = sequence
                .last()
                .map(|last| last.kind_name() == entry.strategy_kind.kind_name())
                .unwrap_or(false);
            if !dominated {
                sequence.push(entry.strategy_kind.clone());
            }
        }
        sequence
    }

    /// Returns the sequence of attractor classifications over time.
    ///
    /// Tracks how the attractor state evolved as observations were recorded.
    /// Useful for convergence memory and understanding trajectory dynamics.
    /// Each entry summarizes one observation's convergence delta, level, and
    /// overall trend direction.
    pub fn attractor_path(&self) -> Vec<String> {
        let mut path = Vec::new();
        for obs in &self.observations {
            if let Some(ref metrics) = obs.metrics {
                let label = if metrics.convergence_delta > 0.05 {
                    "improving"
                } else if metrics.convergence_delta < -0.05 {
                    "declining"
                } else {
                    "flat"
                };
                let entry = format!(
                    "seq={} delta={:.3} level={:.3} ({})",
                    obs.sequence, metrics.convergence_delta, metrics.convergence_level, label
                );
                if path.last() != Some(&entry) {
                    path.push(entry);
                }
            }
        }
        path
    }

    /// Returns the overseer signal changes that most influenced convergence
    /// decisions.
    ///
    /// Identifies transitions where overseer signals changed significantly
    /// between consecutive observations (e.g., test pass count jumps, build
    /// failures resolved, new vulnerabilities introduced).
    pub fn decisive_overseer_changes(&self) -> Vec<String> {
        let mut changes = Vec::new();

        for window in self.observations.windows(2) {
            let prev = &window[0];
            let curr = &window[1];

            // Track error count changes across build/type/lint
            let prev_errors = prev.overseer_signals.error_count();
            let curr_errors = curr.overseer_signals.error_count();
            if prev_errors != curr_errors {
                changes.push(format!(
                    "seq {}->{}: errors {} -> {}",
                    prev.sequence, curr.sequence, prev_errors, curr_errors
                ));
            }

            // Track test pass count changes
            let prev_pass = prev
                .overseer_signals
                .test_results
                .as_ref()
                .map(|t| t.passed)
                .unwrap_or(0);
            let curr_pass = curr
                .overseer_signals
                .test_results
                .as_ref()
                .map(|t| t.passed)
                .unwrap_or(0);
            if prev_pass != curr_pass {
                changes.push(format!(
                    "seq {}->{}: test passes {} -> {}",
                    prev.sequence, curr.sequence, prev_pass, curr_pass
                ));
            }

            // Track vulnerability regressions
            let prev_vulns = prev.overseer_signals.vulnerability_count();
            let curr_vulns = curr.overseer_signals.vulnerability_count();
            if curr_vulns > prev_vulns {
                changes.push(format!(
                    "seq {}->{}: vulnerabilities {} -> {} (regression!)",
                    prev.sequence, curr.sequence, prev_vulns, curr_vulns
                ));
            }
        }

        changes
    }

    /// Returns all distinct strategy kinds that have been used in this trajectory.
    ///
    /// Uses `kind_name()` for deduplication since `StrategyKind` variants may
    /// carry data (e.g., `RevertAndBranch` and `FreshStart`).
    pub fn all_strategies_used(&self) -> Vec<StrategyKind> {
        let mut seen_names: HashSet<&str> = HashSet::new();
        let mut strategies = Vec::new();
        for entry in &self.strategy_log {
            if seen_names.insert(entry.strategy_kind.kind_name()) {
                strategies.push(entry.strategy_kind.clone());
            }
        }
        strategies
    }

    /// Returns gaps that persist across multiple observations.
    ///
    /// A gap is considered persistent if it appears in at least two verification
    /// results (by normalized description). These are the gaps the system has
    /// been unable to resolve and are prime candidates for human escalation or
    /// specification amendment.
    pub fn persistent_gaps(&self) -> Vec<String> {
        let mut gap_counts: std::collections::HashMap<String, u32> =
            std::collections::HashMap::new();

        for obs in &self.observations {
            if let Some(ref verification) = obs.verification {
                for gap in &verification.gaps {
                    let normalized = gap
                        .description
                        .to_lowercase()
                        .split_whitespace()
                        .collect::<Vec<_>>()
                        .join(" ");
                    *gap_counts.entry(normalized).or_insert(0) += 1;
                }
            }
        }

        gap_counts
            .into_iter()
            .filter(|(_, count)| *count >= 2)
            .map(|(desc, _)| desc)
            .collect()
    }

    /// Returns the observation with the highest convergence level.
    ///
    /// If no observations have metrics, returns the most recent observation.
    /// Returns `None` if the trajectory has no observations.
    pub fn best_observation(&self) -> Option<&Observation> {
        if self.observations.is_empty() {
            return None;
        }

        let best_by_level = self
            .observations
            .iter()
            .filter(|o| o.metrics.is_some())
            .max_by(|a, b| {
                let a_m = a.metrics.as_ref().unwrap();
                let b_m = b.metrics.as_ref().unwrap();
                let a_level = a_m.intent_blended_level.unwrap_or(a_m.convergence_level);
                let b_level = b_m.intent_blended_level.unwrap_or(b_m.convergence_level);
                a_level
                    .partial_cmp(&b_level)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });

        best_by_level.or(self.observations.last())
    }
}

// ---------------------------------------------------------------------------
// Observation
// ---------------------------------------------------------------------------

/// A point in solution space -- a snapshot of where the implementation stands
/// after a single iteration.
///
/// Each iteration produces an observation. Observations are *measured by
/// overseers* (external verification signals), never self-assessed. The
/// observation captures the artifact produced, the overseer measurements,
/// optional LLM-based verification, computed convergence metrics, cost
/// tracking, and which strategy produced it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Observation {
    /// Unique identifier for this observation.
    pub id: Uuid,

    /// Sequence number within the trajectory (0-indexed).
    pub sequence: u32,

    /// When this observation was recorded.
    pub timestamp: DateTime<Utc>,

    /// What the agent produced in this iteration.
    pub artifact: ArtifactReference,

    /// Overseer measurements (external, deterministic).
    pub overseer_signals: OverseerSignals,

    /// LLM-based verification (calibrated, periodic). Not run on every
    /// iteration -- controlled by `policy.intent_verification_frequency`.
    pub verification: Option<VerificationResult>,

    /// Convergence metrics computed from this observation. `None` for the
    /// first observation since there is no previous reference point for
    /// computing deltas.
    pub metrics: Option<ObservationMetrics>,

    /// Tokens consumed by the strategy execution that produced this observation.
    pub tokens_used: u64,

    /// Wall-clock time in milliseconds for the strategy execution.
    pub wall_time_ms: u64,

    /// The strategy that produced this observation.
    pub strategy_used: StrategyKind,
}

impl Observation {
    /// Create a new observation with the given parameters.
    ///
    /// The `verification` and `metrics` fields are initialized to `None`;
    /// use the builder methods [`with_verification`](Self::with_verification)
    /// and [`with_metrics`](Self::with_metrics) to attach them.
    pub fn new(
        sequence: u32,
        artifact: ArtifactReference,
        overseer_signals: OverseerSignals,
        strategy_used: StrategyKind,
        tokens_used: u64,
        wall_time_ms: u64,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            sequence,
            timestamp: Utc::now(),
            artifact,
            overseer_signals,
            verification: None,
            metrics: None,
            tokens_used,
            wall_time_ms,
            strategy_used,
        }
    }

    /// Attach a verification result to this observation.
    pub fn with_verification(mut self, verification: VerificationResult) -> Self {
        self.verification = Some(verification);
        self
    }

    /// Attach computed metrics to this observation.
    pub fn with_metrics(mut self, metrics: ObservationMetrics) -> Self {
        self.metrics = Some(metrics);
        self
    }
}

// ---------------------------------------------------------------------------
// StrategyEntry
// ---------------------------------------------------------------------------

/// A record of a strategy execution within a trajectory.
///
/// Captures what strategy was used, which observation it produced, the
/// convergence delta it achieved, its token cost, and whether it was forced
/// (bypassing normal bandit selection).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StrategyEntry {
    /// The strategy that was executed.
    pub strategy_kind: StrategyKind,

    /// The sequence number of the observation this strategy produced.
    pub observation_sequence: u32,

    /// The convergence delta achieved by this strategy execution.
    /// `None` if this was the first observation (no delta to compute).
    pub convergence_delta_achieved: Option<f64>,

    /// Tokens consumed by this strategy execution.
    pub tokens_used: u64,

    /// Whether this strategy was forced (set via `trajectory.forced_strategy`),
    /// bypassing normal bandit selection.
    pub was_forced: bool,

    /// When this strategy was executed.
    pub timestamp: DateTime<Utc>,
}

impl StrategyEntry {
    /// Create a new strategy entry.
    pub fn new(
        strategy_kind: StrategyKind,
        observation_sequence: u32,
        tokens_used: u64,
        was_forced: bool,
    ) -> Self {
        Self {
            strategy_kind,
            observation_sequence,
            convergence_delta_achieved: None,
            tokens_used,
            was_forced,
            timestamp: Utc::now(),
        }
    }

    /// Set the convergence delta achieved by this strategy.
    pub fn with_delta(mut self, delta: f64) -> Self {
        self.convergence_delta_achieved = Some(delta);
        self
    }
}

// ---------------------------------------------------------------------------
// ConvergencePhase
// ---------------------------------------------------------------------------

/// The current phase in the convergence lifecycle.
///
/// Trajectories progress through phases from initial preparation through
/// iteration to a terminal state. The `Coordinating` phase is entered when
/// a task is decomposed into subtasks via the `Decompose` strategy.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ConvergencePhase {
    /// Initial phase: generating acceptance tests, detecting ambiguity,
    /// recalling relevant memories.
    Preparing,

    /// Main convergence loop: selecting strategies, executing, measuring,
    /// classifying attractors.
    Iterating,

    /// Task has been decomposed into subtasks. The trajectory coordinates
    /// child trajectories and waits for their convergence.
    Coordinating {
        /// Trajectory IDs of the child trajectories being coordinated.
        children: Vec<Uuid>,
    },

    /// Terminal: trajectory has converged to a satisfactory result.
    Converged,

    /// Terminal: convergence budget has been exhausted without reaching
    /// the acceptance threshold.
    Exhausted,

    /// Terminal: all escape strategies have been exhausted. The trajectory
    /// is trapped in a limit cycle or divergent state with no remaining
    /// options.
    Trapped,
}

// ---------------------------------------------------------------------------
// VerificationResult
// ---------------------------------------------------------------------------

/// Result of LLM-based intent verification for an observation.
///
/// This is a lightweight verification result embedded within an observation,
/// distinct from the full `IntentVerificationResult` used by the intent
/// verification service. It captures whether the current artifact satisfies
/// the specification's intent, the model's confidence, and any identified gaps.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerificationResult {
    /// Satisfaction assessment (e.g., "satisfied", "partial", "unsatisfied").
    pub satisfaction: String,

    /// Confidence in this assessment (0.0 to 1.0).
    pub confidence: f64,

    /// Gaps identified between the artifact and the specification intent.
    pub gaps: Vec<IntentGap>,
}

impl VerificationResult {
    /// Create a new verification result.
    pub fn new(satisfaction: impl Into<String>, confidence: f64, gaps: Vec<IntentGap>) -> Self {
        Self {
            satisfaction: satisfaction.into(),
            confidence: confidence.clamp(0.0, 1.0),
            gaps,
        }
    }

    /// Whether the verification indicates full satisfaction.
    pub fn satisfied(&self) -> bool {
        self.satisfaction == "satisfied"
    }

    /// Whether any of the identified gaps are ambiguity-related.
    ///
    /// Ambiguity gaps indicate that the specification itself is unclear or
    /// contradictory, which is a signal for specification amendment rather
    /// than further iteration on the current approach. Gaps marked as
    /// implicit (unstated requirements) are treated as ambiguity indicators.
    pub fn has_ambiguity_gaps(&self) -> bool {
        self.gaps.iter().any(|g| g.is_implicit)
    }
}

impl From<&IntentVerificationResult> for VerificationResult {
    /// Lossy conversion from the rich `IntentVerificationResult` into the
    /// lightweight `VerificationResult` stored on observations.
    ///
    /// Combines explicit and implicit gaps into a single list. Rich data
    /// (reprompt guidance, escalation info) is consumed at the orchestrator
    /// layer before this conversion happens for storage.
    fn from(ivr: &IntentVerificationResult) -> Self {
        let gaps: Vec<IntentGap> = ivr.all_gaps().cloned().collect();
        Self {
            satisfaction: ivr.satisfaction.as_str().to_string(),
            confidence: ivr.confidence,
            gaps,
        }
    }
}

// ---------------------------------------------------------------------------
// ArtifactReference
// ---------------------------------------------------------------------------

/// A reference to an artifact produced by a strategy execution.
///
/// Artifacts are the outputs of each iteration -- the code, configuration,
/// or other files that the agent produced. The reference includes a content
/// hash for detecting structural self-similarity across iterations.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArtifactReference {
    /// Unique identifier for this artifact.
    pub id: Uuid,

    /// Path to the artifact (e.g., worktree path or file URI).
    pub path: String,

    /// Content hash for deduplication and self-similarity detection.
    pub content_hash: String,
}

impl Default for ArtifactReference {
    fn default() -> Self {
        Self {
            id: Uuid::nil(),
            path: String::new(),
            content_hash: String::new(),
        }
    }
}

impl ArtifactReference {
    /// Create a new artifact reference with an auto-generated ID.
    pub fn new(path: impl Into<String>, content_hash: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            path: path.into(),
            content_hash: content_hash.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::models::intent_verification::{GapSeverity, IntentGap};

    /// Helper to create a minimal specification evolution for testing.
    fn test_spec() -> SpecificationEvolution {
        SpecificationEvolution::new(SpecificationSnapshot::new("test spec".into()))
    }

    /// Helper to create a minimal convergence budget for testing.
    fn test_budget() -> ConvergenceBudget {
        ConvergenceBudget::default()
    }

    /// Helper to create a minimal convergence policy for testing.
    fn test_policy() -> ConvergencePolicy {
        ConvergencePolicy::default()
    }

    /// Helper to create a test artifact reference.
    fn test_artifact(seq: u32) -> ArtifactReference {
        ArtifactReference::new(
            format!("/worktree/task/artifact_{}.rs", seq),
            format!("hash_{}", seq),
        )
    }

    /// Helper to create a test observation with default overseer signals.
    fn test_observation(seq: u32, strategy: StrategyKind) -> Observation {
        Observation::new(
            seq,
            test_artifact(seq),
            OverseerSignals::default(),
            strategy,
            10_000,
            5_000,
        )
    }

    /// Helper to create `ObservationMetrics` with specified delta and level,
    /// zeroing out all other fields.
    fn metrics_with(convergence_delta: f64, convergence_level: f64) -> ObservationMetrics {
        ObservationMetrics {
            ast_diff_nodes: 0,
            test_pass_delta: 0,
            test_regression_count: 0,
            error_count_delta: 0,
            vulnerability_delta: 0,
            convergence_delta,
            convergence_level,
            intent_blended_level: None,
        }
    }

    #[test]
    fn test_trajectory_new() {
        let task_id = Uuid::new_v4();
        let goal_id = Some(Uuid::new_v4());
        let trajectory = Trajectory::new(
            task_id,
            goal_id,
            test_spec(),
            test_budget(),
            test_policy(),
        );

        assert_eq!(trajectory.task_id, task_id);
        assert_eq!(trajectory.goal_id, goal_id);
        assert!(trajectory.observations.is_empty());
        assert!(trajectory.strategy_log.is_empty());
        assert_eq!(trajectory.phase, ConvergencePhase::Preparing);
        assert_eq!(trajectory.total_fresh_starts, 0);
        assert!(!trajectory.is_converged());
    }

    #[test]
    fn test_trajectory_latest_artifact() {
        let task_id = Uuid::new_v4();
        let mut trajectory =
            Trajectory::new(task_id, None, test_spec(), test_budget(), test_policy());

        // No observations yet
        assert!(trajectory.latest_artifact().is_none());

        // Add an observation
        trajectory
            .observations
            .push(test_observation(0, StrategyKind::RetryWithFeedback));

        let artifact = trajectory.latest_artifact().unwrap();
        assert!(artifact.path.contains("artifact_0"));
    }

    #[test]
    fn test_trajectory_is_converged() {
        let task_id = Uuid::new_v4();
        let mut trajectory =
            Trajectory::new(task_id, None, test_spec(), test_budget(), test_policy());

        assert!(!trajectory.is_converged());

        trajectory.phase = ConvergencePhase::Converged;
        assert!(trajectory.is_converged());

        trajectory.phase = ConvergencePhase::Exhausted;
        assert!(!trajectory.is_converged());
    }

    #[test]
    fn test_effective_strategy_sequence() {
        let task_id = Uuid::new_v4();
        let mut trajectory =
            Trajectory::new(task_id, None, test_spec(), test_budget(), test_policy());

        // Add strategy entries with some repeats
        trajectory.strategy_log.push(StrategyEntry::new(
            StrategyKind::RetryWithFeedback,
            0,
            10_000,
            false,
        ));
        trajectory.strategy_log.push(StrategyEntry::new(
            StrategyKind::RetryWithFeedback,
            1,
            10_000,
            false,
        ));
        trajectory.strategy_log.push(StrategyEntry::new(
            StrategyKind::FocusedRepair,
            2,
            15_000,
            false,
        ));
        trajectory.strategy_log.push(StrategyEntry::new(
            StrategyKind::Reframe,
            3,
            40_000,
            false,
        ));

        let seq = trajectory.effective_strategy_sequence();
        assert_eq!(seq.len(), 3);
        // Verify by kind_name since StrategyKind does not implement PartialEq
        assert_eq!(seq[0].kind_name(), "retry_with_feedback");
        assert_eq!(seq[1].kind_name(), "focused_repair");
        assert_eq!(seq[2].kind_name(), "reframe");
    }

    #[test]
    fn test_all_strategies_used() {
        let task_id = Uuid::new_v4();
        let mut trajectory =
            Trajectory::new(task_id, None, test_spec(), test_budget(), test_policy());

        trajectory.strategy_log.push(StrategyEntry::new(
            StrategyKind::RetryWithFeedback,
            0,
            10_000,
            false,
        ));
        trajectory.strategy_log.push(StrategyEntry::new(
            StrategyKind::FocusedRepair,
            1,
            15_000,
            false,
        ));
        trajectory.strategy_log.push(StrategyEntry::new(
            StrategyKind::RetryWithFeedback,
            2,
            10_000,
            false,
        ));

        let used = trajectory.all_strategies_used();
        assert_eq!(used.len(), 2);
        let used_names: HashSet<&str> = used.iter().map(|s| s.kind_name()).collect();
        assert!(used_names.contains("retry_with_feedback"));
        assert!(used_names.contains("focused_repair"));
    }

    #[test]
    fn test_best_observation() {
        let task_id = Uuid::new_v4();
        let mut trajectory =
            Trajectory::new(task_id, None, test_spec(), test_budget(), test_policy());

        // No observations
        assert!(trajectory.best_observation().is_none());

        // Add observations with varying convergence levels
        let obs1 = test_observation(0, StrategyKind::RetryWithFeedback)
            .with_metrics(metrics_with(0.1, 0.3));

        let obs2 = test_observation(1, StrategyKind::FocusedRepair)
            .with_metrics(metrics_with(0.2, 0.7));

        let obs3 = test_observation(2, StrategyKind::RetryWithFeedback)
            .with_metrics(metrics_with(-0.1, 0.5));

        trajectory.observations.push(obs1);
        trajectory.observations.push(obs2);
        trajectory.observations.push(obs3);

        let best = trajectory.best_observation().unwrap();
        assert_eq!(best.sequence, 1); // obs2 had highest level (0.7)
    }

    #[test]
    fn test_best_observation_fallback_to_latest() {
        let task_id = Uuid::new_v4();
        let mut trajectory =
            Trajectory::new(task_id, None, test_spec(), test_budget(), test_policy());

        // Add observations without metrics
        trajectory
            .observations
            .push(test_observation(0, StrategyKind::RetryWithFeedback));
        trajectory
            .observations
            .push(test_observation(1, StrategyKind::FocusedRepair));

        // Should fall back to the latest observation
        let best = trajectory.best_observation().unwrap();
        assert_eq!(best.sequence, 1);
    }

    #[test]
    fn test_latest_convergence_delta() {
        let task_id = Uuid::new_v4();
        let mut trajectory =
            Trajectory::new(task_id, None, test_spec(), test_budget(), test_policy());

        // No observations
        assert_eq!(trajectory.latest_convergence_delta(), 0.0);

        // First observation without metrics
        trajectory
            .observations
            .push(test_observation(0, StrategyKind::RetryWithFeedback));
        assert_eq!(trajectory.latest_convergence_delta(), 0.0);

        // Add observation with metrics
        let obs = test_observation(1, StrategyKind::FocusedRepair)
            .with_metrics(metrics_with(0.42, 0.6));
        trajectory.observations.push(obs);

        assert!((trajectory.latest_convergence_delta() - 0.42).abs() < f64::EPSILON);
    }

    #[test]
    fn test_persistent_gaps() {
        let task_id = Uuid::new_v4();
        let mut trajectory =
            Trajectory::new(task_id, None, test_spec(), test_budget(), test_policy());

        // Two observations with overlapping gaps
        let gap_a = IntentGap::new("Missing error handling", GapSeverity::Major);
        let gap_b = IntentGap::new("No unit tests", GapSeverity::Moderate);
        let gap_c = IntentGap::new("Missing error handling", GapSeverity::Major);

        let obs1 = test_observation(0, StrategyKind::RetryWithFeedback).with_verification(
            VerificationResult::new("partial", 0.5, vec![gap_a, gap_b]),
        );
        let obs2 = test_observation(1, StrategyKind::FocusedRepair).with_verification(
            VerificationResult::new("partial", 0.6, vec![gap_c]),
        );

        trajectory.observations.push(obs1);
        trajectory.observations.push(obs2);

        let persistent = trajectory.persistent_gaps();
        assert_eq!(persistent.len(), 1);
        assert!(persistent[0].contains("missing error handling"));
    }

    #[test]
    fn test_verification_result_satisfied() {
        let vr = VerificationResult::new("satisfied", 0.95, vec![]);
        assert!(vr.satisfied());

        let vr = VerificationResult::new("partial", 0.6, vec![]);
        assert!(!vr.satisfied());
    }

    #[test]
    fn test_verification_result_ambiguity_gaps() {
        let gap = IntentGap::new("Missing feature", GapSeverity::Major);
        let vr = VerificationResult::new("partial", 0.5, vec![gap]);
        assert!(!vr.has_ambiguity_gaps());

        let implicit_gap =
            IntentGap::new("Unclear requirement", GapSeverity::Moderate).as_implicit("ambiguous");
        let vr = VerificationResult::new("partial", 0.5, vec![implicit_gap]);
        assert!(vr.has_ambiguity_gaps());
    }

    #[test]
    fn test_verification_result_confidence_clamped() {
        let vr = VerificationResult::new("partial", 1.5, vec![]);
        assert!((vr.confidence - 1.0).abs() < f64::EPSILON);

        let vr = VerificationResult::new("partial", -0.5, vec![]);
        assert!((vr.confidence - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_artifact_reference_new() {
        let artifact = ArtifactReference::new("/path/to/file.rs", "abc123");
        assert_eq!(artifact.path, "/path/to/file.rs");
        assert_eq!(artifact.content_hash, "abc123");
    }

    #[test]
    fn test_convergence_phase_equality() {
        assert_eq!(ConvergencePhase::Preparing, ConvergencePhase::Preparing);
        assert_eq!(ConvergencePhase::Converged, ConvergencePhase::Converged);
        assert_ne!(ConvergencePhase::Preparing, ConvergencePhase::Iterating);

        let children = vec![Uuid::new_v4()];
        assert_eq!(
            ConvergencePhase::Coordinating {
                children: children.clone()
            },
            ConvergencePhase::Coordinating { children }
        );
    }

    #[test]
    fn test_strategy_entry_with_delta() {
        let entry = StrategyEntry::new(StrategyKind::RetryWithFeedback, 0, 10_000, false)
            .with_delta(0.15);

        assert_eq!(entry.strategy_kind.kind_name(), "retry_with_feedback");
        assert_eq!(entry.observation_sequence, 0);
        assert_eq!(entry.tokens_used, 10_000);
        assert!(!entry.was_forced);
        assert_eq!(entry.convergence_delta_achieved, Some(0.15));
    }

    #[test]
    fn test_observation_new() {
        let obs = Observation::new(
            3,
            ArtifactReference::new("/test", "hash"),
            OverseerSignals::default(),
            StrategyKind::Reframe,
            25_000,
            12_000,
        );

        assert_eq!(obs.sequence, 3);
        assert_eq!(obs.tokens_used, 25_000);
        assert_eq!(obs.wall_time_ms, 12_000);
        assert_eq!(obs.strategy_used.kind_name(), "reframe");
        assert!(obs.verification.is_none());
        assert!(obs.metrics.is_none());
    }

    #[test]
    fn test_decisive_overseer_changes_empty() {
        let task_id = Uuid::new_v4();
        let trajectory =
            Trajectory::new(task_id, None, test_spec(), test_budget(), test_policy());

        // No observations, no changes
        assert!(trajectory.decisive_overseer_changes().is_empty());
    }

    #[test]
    fn test_attractor_path_empty() {
        let task_id = Uuid::new_v4();
        let trajectory =
            Trajectory::new(task_id, None, test_spec(), test_budget(), test_policy());

        assert!(trajectory.attractor_path().is_empty());
    }

    #[test]
    fn test_attractor_path_with_observations() {
        let task_id = Uuid::new_v4();
        let mut trajectory =
            Trajectory::new(task_id, None, test_spec(), test_budget(), test_policy());

        trajectory
            .observations
            .push(test_observation(0, StrategyKind::RetryWithFeedback).with_metrics(
                metrics_with(0.15, 0.3),
            ));
        trajectory
            .observations
            .push(test_observation(1, StrategyKind::FocusedRepair).with_metrics(
                metrics_with(-0.1, 0.25),
            ));
        trajectory
            .observations
            .push(test_observation(2, StrategyKind::Reframe).with_metrics(
                metrics_with(0.01, 0.26),
            ));

        let path = trajectory.attractor_path();
        assert_eq!(path.len(), 3);
        assert!(path[0].contains("improving"));
        assert!(path[1].contains("declining"));
        assert!(path[2].contains("flat"));
    }

    #[test]
    fn test_latest_overseer_signals() {
        let task_id = Uuid::new_v4();
        let mut trajectory =
            Trajectory::new(task_id, None, test_spec(), test_budget(), test_policy());

        assert!(trajectory.latest_overseer_signals().is_none());

        trajectory
            .observations
            .push(test_observation(0, StrategyKind::RetryWithFeedback));

        assert!(trajectory.latest_overseer_signals().is_some());
    }
}
