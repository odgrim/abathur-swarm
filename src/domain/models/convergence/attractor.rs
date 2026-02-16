//! Attractor classification for convergence trajectories.
//!
//! This module implements the attractor detection and classification system described
//! in Part 3 of the convergence specification. Every task execution is a trajectory
//! through solution space, and attractors are the destinations those trajectories
//! approach. The system classifies trajectories in real-time to determine whether
//! the current approach is converging (FixedPoint), oscillating (LimitCycle),
//! regressing (Divergent), stalled (Plateau), or not yet classifiable (Indeterminate).
//!
//! ## Classification Algorithm
//!
//! After each observation, the classifier examines a sliding window of recent
//! observations and their convergence deltas:
//!
//! 1. **LimitCycle** -- Detected first via overseer fingerprint repetition. If the
//!    same pattern of pass/fail signatures repeats with period 2, 3, or 4, the
//!    trajectory is trapped in a cycle.
//!
//! 2. **Plateau** -- Average absolute delta falls below `PLATEAU_EPSILON`. The
//!    trajectory is making no meaningful progress in either direction.
//!
//! 3. **Divergent** -- More than 70% of recent deltas are negative. The trajectory
//!    is moving away from the target, and the system infers a probable cause.
//!
//! 4. **FixedPoint** -- More than 60% of recent deltas are positive. The trajectory
//!    is converging toward a correct solution, and remaining iterations are estimated.
//!
//! 5. **Indeterminate** -- Fewer than 3 observations with metrics, or no pattern
//!    matches the above criteria. A tendency (Improving, Declining, Flat) is reported.
//!
//! ## Cycle Detection
//!
//! Cycle detection uses fuzzy fingerprint matching. Each observation's overseer signals
//! are reduced to a string signature, and the system checks whether the most recent
//! `2 * period` signatures form two matching halves for periods 2, 3, and 4.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::*;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Threshold below which average absolute convergence delta is considered a plateau.
///
/// When the mean of `|delta|` over the sliding window falls below this value,
/// the trajectory is classified as `Plateau` -- the system is making no meaningful
/// progress in either direction.
pub const PLATEAU_EPSILON: f64 = 0.02;

/// Minimum fuzzy similarity score (Jaccard on character bigrams) for two overseer
/// signatures to be considered "the same" during cycle detection.
///
/// A value of 0.85 allows minor formatting or ordering differences while still
/// catching substantively identical pass/fail patterns.
pub const CYCLE_SIMILARITY_THRESHOLD: f64 = 0.85;

// ---------------------------------------------------------------------------
// AttractorState
// ---------------------------------------------------------------------------

/// The current attractor classification for a trajectory.
///
/// Updated after every observation. The `classification` field determines which
/// convergence strategies are eligible (see Part 4 of the spec), while `confidence`
/// indicates how certain the classifier is about the diagnosis. A low confidence
/// with a strong classification may indicate a recent regime change.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttractorState {
    /// The classified attractor type.
    pub classification: AttractorType,

    /// Confidence in this classification (0.0 -- 1.0).
    ///
    /// Higher values indicate more evidence supporting the classification.
    /// Influenced by the number of observations in the window, the consistency
    /// of deltas, and the strength of the signal.
    pub confidence: f64,

    /// The observation at which this classification was detected.
    ///
    /// `None` only for the default (initial) state before any classification
    /// has been performed.
    pub detected_at: Option<Uuid>,

    /// Supporting evidence for the classification.
    pub evidence: AttractorEvidence,
}

impl Default for AttractorState {
    /// Returns an `Indeterminate` state with `Flat` tendency, zero confidence,
    /// no detection observation, and empty evidence.
    ///
    /// This is the starting state for every new trajectory before any observations
    /// have been classified.
    fn default() -> Self {
        Self {
            classification: AttractorType::Indeterminate {
                tendency: ConvergenceTendency::Flat,
            },
            confidence: 0.0,
            detected_at: None,
            evidence: AttractorEvidence {
                recent_deltas: Vec::new(),
                recent_signatures: Vec::new(),
                rationale: String::from("No observations yet"),
            },
        }
    }
}

// ---------------------------------------------------------------------------
// AttractorEvidence
// ---------------------------------------------------------------------------

/// Supporting evidence for an attractor classification.
///
/// Captures the raw data that drove the classification decision, enabling both
/// human inspection and downstream services to understand *why* a particular
/// attractor type was assigned.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttractorEvidence {
    /// The convergence deltas from the sliding window used for classification.
    ///
    /// These are the `convergence_delta` values from each observation's metrics,
    /// filtered to only include observations that have computed metrics (i.e.,
    /// the first observation is excluded since it has no predecessor to diff against).
    pub recent_deltas: Vec<f64>,

    /// Overseer fingerprint signatures from the sliding window.
    ///
    /// Each string is a compact representation of which overseers passed and failed
    /// for a given observation, used primarily for limit cycle detection.
    pub recent_signatures: Vec<String>,

    /// Human-readable rationale explaining the classification.
    ///
    /// Includes the specific thresholds and measurements that led to the decision,
    /// suitable for logging and debugging.
    pub rationale: String,
}

// ---------------------------------------------------------------------------
// AttractorType
// ---------------------------------------------------------------------------

/// The type of attractor a trajectory is approaching.
///
/// Each variant corresponds to a qualitatively different convergence regime and
/// drives different strategy eligibility rules (Part 4 of the spec).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttractorType {
    /// Approaching a stable correct solution.
    ///
    /// Convergence delta is positive and increasing or stable. The trajectory is
    /// making consistent forward progress. Only exploitation strategies (refine the
    /// current approach) are eligible.
    FixedPoint {
        /// Estimated number of additional iterations to reach convergence,
        /// based on the current rate of progress and distance from level 1.0.
        estimated_remaining_iterations: u32,

        /// Estimated additional tokens needed, computed from the per-iteration
        /// average token consumption and estimated remaining iterations.
        estimated_remaining_tokens: u64,
    },

    /// Oscillating between N states.
    ///
    /// The overseer fingerprints or test result signatures form a repeating cycle.
    /// This typically indicates the LLM is toggling between two "fixes" that each
    /// break the other. Only exploration strategies (try something fundamentally
    /// different) are eligible.
    LimitCycle {
        /// The detected cycle period (2, 3, or 4).
        period: u32,

        /// The fingerprint signatures that form the cycle, for diagnostic purposes.
        cycle_signatures: Vec<String>,
    },

    /// Moving away from the target.
    ///
    /// Convergence delta is consistently negative -- the test pass rate is declining,
    /// errors are accumulating, or regressions dominate. The `probable_cause` drives
    /// strategy selection: specification ambiguity triggers architect review, wrong
    /// approach triggers reframing, accumulated regression triggers revert-and-branch.
    Divergent {
        /// The average rate of divergence (negative value).
        divergence_rate: f64,

        /// The inferred root cause of divergence.
        probable_cause: DivergenceCause,
    },

    /// Not enough observations to classify.
    ///
    /// Fewer than 3 observations with metrics are available, or the pattern does not
    /// match any known attractor type. The `tendency` field provides a directional
    /// hint based on the most recent delta.
    Indeterminate {
        /// The directional tendency of the most recent observation(s).
        tendency: ConvergenceTendency,
    },

    /// Stalled -- convergence delta near zero for multiple iterations.
    ///
    /// The trajectory is neither converging nor diverging; it is stuck at a fixed
    /// quality level. Extended plateaus trigger fresh starts or decomposition
    /// depending on the plateau level and remaining fresh start budget.
    Plateau {
        /// Number of consecutive iterations at the plateau.
        stall_duration: u32,

        /// The convergence level at which the trajectory is stalled (0.0 -- 1.0).
        plateau_level: f64,
    },
}

// ---------------------------------------------------------------------------
// DivergenceCause
// ---------------------------------------------------------------------------

/// Inferred root cause of a diverging trajectory.
///
/// Used to select the appropriate recovery strategy when the trajectory is
/// classified as `Divergent`. Each cause maps to a different set of eligible
/// strategies in the strategy eligibility filter (Part 4 of the spec).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DivergenceCause {
    /// The specification is ambiguous, causing the LLM to oscillate between
    /// conflicting interpretations. Triggers architect review or reframing.
    SpecificationAmbiguity,

    /// The current implementation approach is fundamentally unsuitable.
    /// Triggers alternative approach or reframing strategies.
    WrongApproach,

    /// Fixes are introducing regressions faster than they resolve issues.
    /// Triggers revert-and-branch to the best prior observation.
    AccumulatedRegression,

    /// No specific cause could be inferred from the available evidence.
    Unknown,
}

// ---------------------------------------------------------------------------
// ConvergenceTendency
// ---------------------------------------------------------------------------

/// Directional tendency for an `Indeterminate` attractor classification.
///
/// Provides a coarse signal about which direction the trajectory appears to
/// be heading, even when there is insufficient data for full classification.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConvergenceTendency {
    /// The most recent delta is positive -- progress is being made.
    Improving,

    /// The most recent delta is negative -- quality is declining.
    Declining,

    /// The most recent delta is approximately zero, or no delta is available.
    Flat,
}

// ---------------------------------------------------------------------------
// classify_attractor
// ---------------------------------------------------------------------------

/// Classify the attractor type for a trajectory based on recent observations.
///
/// This is the primary classification function, implementing the algorithm from
/// Section 3.2 of the convergence specification. It examines a sliding window of
/// the most recent observations and their convergence metrics to determine which
/// attractor type best describes the trajectory's current behavior.
///
/// # Arguments
///
/// * `observations` -- The full ordered sequence of observations for the trajectory.
/// * `window` -- The size of the sliding window (number of most recent observations
///   to consider). Typical values are 5--8.
///
/// # Algorithm
///
/// 1. Extract the most recent `window` observations.
/// 2. If fewer than 3 observations exist, return `Indeterminate` with a tendency
///    derived from the last observation's delta.
/// 3. Extract convergence deltas from observations that have computed metrics.
///    If fewer than 2 deltas, return `Indeterminate`.
/// 4. Generate overseer fingerprint signatures for cycle detection.
/// 5. Check for **LimitCycle** -- repeating fingerprint patterns.
/// 6. Check for **Plateau** -- average absolute delta below `PLATEAU_EPSILON`.
/// 7. Check for **Divergent** -- more than 70% of deltas are negative.
/// 8. Check for **FixedPoint** -- more than 60% of deltas are positive.
/// 9. Fall back to **Indeterminate** with computed tendency.
///
/// # Returns
///
/// An `AttractorState` with the classification, confidence, detection observation ID,
/// and supporting evidence.
pub fn classify_attractor(observations: &[Observation], window: usize) -> AttractorState {
    let start = observations.len().saturating_sub(window);
    let recent = &observations[start..];

    // Not enough observations for meaningful classification.
    if recent.len() < 3 {
        let tendency = recent
            .last()
            .and_then(|o| o.metrics.as_ref())
            .map(|m| match m.convergence_delta {
                d if d > 0.0 => ConvergenceTendency::Improving,
                d if d < 0.0 => ConvergenceTendency::Declining,
                _ => ConvergenceTendency::Flat,
            })
            .unwrap_or(ConvergenceTendency::Flat);

        return AttractorState {
            classification: AttractorType::Indeterminate { tendency },
            confidence: 0.2,
            detected_at: recent.last().map(|o| o.id),
            evidence: AttractorEvidence {
                recent_deltas: recent
                    .iter()
                    .filter_map(|o| o.metrics.as_ref())
                    .map(|m| m.convergence_delta)
                    .collect(),
                recent_signatures: Vec::new(),
                rationale: format!(
                    "Only {} observations available (minimum 3 required for classification)",
                    recent.len()
                ),
            },
        };
    }

    // Extract convergence deltas from observations that have computed metrics.
    // The first observation has no predecessor, so its metrics field is None.
    let deltas: Vec<f64> = recent
        .iter()
        .filter_map(|o| o.metrics.as_ref())
        .map(|m| m.convergence_delta)
        .collect();

    if deltas.len() < 2 {
        return AttractorState {
            classification: AttractorType::Indeterminate {
                tendency: ConvergenceTendency::Flat,
            },
            confidence: 0.15,
            detected_at: recent.last().map(|o| o.id),
            evidence: AttractorEvidence {
                recent_deltas: deltas,
                recent_signatures: Vec::new(),
                rationale: String::from(
                    "Fewer than 2 observations with computed metrics; cannot classify",
                ),
            },
        };
    }

    // Generate overseer fingerprint signatures for cycle detection.
    let test_signatures: Vec<String> = recent
        .iter()
        .map(|o| fingerprint_overseer_results(&o.overseer_signals))
        .collect();

    let detected_at = recent.last().map(|o| o.id);

    // --- Check for limit cycle: repeating signatures ---
    if let Some(period) = detect_cycle(&test_signatures) {
        return AttractorState {
            classification: AttractorType::LimitCycle {
                period,
                cycle_signatures: test_signatures.clone(),
            },
            confidence: 0.85,
            detected_at,
            evidence: AttractorEvidence {
                recent_deltas: deltas,
                recent_signatures: test_signatures,
                rationale: format!(
                    "Detected repeating overseer fingerprint cycle with period {}",
                    period
                ),
            },
        };
    }

    // --- Check for plateau: deltas near zero ---
    let avg_abs_delta = deltas.iter().map(|d| d.abs()).sum::<f64>() / deltas.len() as f64;
    if avg_abs_delta < PLATEAU_EPSILON {
        let plateau_level = recent
            .last()
            .and_then(|o| o.metrics.as_ref())
            .map(|m| m.intent_blended_level.unwrap_or(m.convergence_level))
            .unwrap_or(0.0);

        return AttractorState {
            classification: AttractorType::Plateau {
                stall_duration: deltas.len() as u32,
                plateau_level,
            },
            confidence: 0.75,
            detected_at,
            evidence: AttractorEvidence {
                recent_deltas: deltas,
                recent_signatures: test_signatures,
                rationale: format!(
                    "Average absolute delta {:.4} is below plateau threshold {:.4}; \
                     stalled at level {:.3}",
                    avg_abs_delta, PLATEAU_EPSILON, plateau_level
                ),
            },
        };
    }

    // --- Check for divergence: deltas consistently negative ---
    let negative_count = deltas.iter().filter(|d| **d < 0.0).count();
    let negative_ratio = negative_count as f64 / deltas.len() as f64;
    if negative_ratio > 0.7 {
        let rate = deltas.iter().sum::<f64>() / deltas.len() as f64;
        let cause = infer_divergence_cause(recent);

        return AttractorState {
            classification: AttractorType::Divergent {
                divergence_rate: rate,
                probable_cause: cause,
            },
            confidence: 0.7 + (negative_ratio - 0.7) * 0.5,
            detected_at,
            evidence: AttractorEvidence {
                recent_deltas: deltas,
                recent_signatures: test_signatures,
                rationale: format!(
                    "{:.0}% of deltas are negative (threshold: 70%); average rate {:.4}",
                    negative_ratio * 100.0,
                    rate
                ),
            },
        };
    }

    // --- Check for fixed point: deltas consistently positive ---
    let positive_count = deltas.iter().filter(|d| **d > 0.0).count();
    let positive_ratio = positive_count as f64 / deltas.len() as f64;
    if positive_ratio > 0.6 {
        let rate = deltas.iter().sum::<f64>() / deltas.len() as f64;
        let level = recent
            .last()
            .and_then(|o| o.metrics.as_ref())
            .map(|m| m.intent_blended_level.unwrap_or(m.convergence_level))
            .unwrap_or(0.0);
        let remaining = estimate_remaining_iterations(rate, level);
        let avg_tokens_per_iter = recent
            .iter()
            .map(|o| o.tokens_used)
            .sum::<u64>()
            .checked_div(recent.len() as u64)
            .unwrap_or(20_000);
        let estimated_remaining_tokens = remaining as u64 * avg_tokens_per_iter;

        return AttractorState {
            classification: AttractorType::FixedPoint {
                estimated_remaining_iterations: remaining,
                estimated_remaining_tokens,
            },
            confidence: 0.6 + (positive_ratio - 0.6) * 0.75,
            detected_at,
            evidence: AttractorEvidence {
                recent_deltas: deltas,
                recent_signatures: test_signatures,
                rationale: format!(
                    "{:.0}% of deltas are positive (threshold: 60%); \
                     average rate {:.4}, current level {:.3}, \
                     estimated {} iterations remaining",
                    positive_ratio * 100.0,
                    rate,
                    level,
                    remaining
                ),
            },
        };
    }

    // --- Fallback: Indeterminate with computed tendency ---
    let tendency = compute_tendency(&deltas);
    AttractorState {
        classification: AttractorType::Indeterminate { tendency },
        confidence: 0.3,
        detected_at,
        evidence: AttractorEvidence {
            recent_deltas: deltas,
            recent_signatures: test_signatures,
            rationale: String::from(
                "No attractor pattern matched with sufficient confidence; \
                 trajectory behavior is mixed",
            ),
        },
    }
}

// ---------------------------------------------------------------------------
// detect_cycle
// ---------------------------------------------------------------------------

/// Detect repeating cycles in overseer fingerprint signatures.
///
/// Tries periods 2, 3, and 4 (the most common cycle lengths observed in LLM
/// convergence loops per research). For each candidate period, extracts the most
/// recent `2 * period` signatures and checks whether the first half fuzzy-matches
/// the second half.
///
/// # Arguments
///
/// * `signatures` -- Overseer fingerprint strings, one per observation, in
///   chronological order.
///
/// # Returns
///
/// `Some(period)` if a cycle is detected, where `period` is the shortest
/// matching cycle length. `None` if no cycle is detected.
pub fn detect_cycle(signatures: &[String]) -> Option<u32> {
    // If all signatures are identical, there is no oscillation -- this is a
    // plateau or uniform state, not a limit cycle.  A true cycle requires at
    // least two distinct signature values within the period.
    if let Some(first) = signatures.first() {
        if signatures.iter().all(|s| s == first) {
            return None;
        }
    }

    for period in 2..=4usize {
        if signatures.len() < period * 2 {
            continue;
        }

        let recent = &signatures[signatures.len() - period * 2..];
        let first_half = &recent[..period];
        let second_half = &recent[period..];

        if fuzzy_sequence_match(first_half, second_half, CYCLE_SIMILARITY_THRESHOLD) {
            return Some(period as u32);
        }
    }
    None
}

// ---------------------------------------------------------------------------
// fuzzy_sequence_match
// ---------------------------------------------------------------------------

/// Compare two signature sequences for fuzzy equality.
///
/// Each pair of corresponding signatures is compared using character-bigram
/// Jaccard similarity. The sequences match if *every* corresponding pair exceeds
/// the similarity threshold. This accommodates minor formatting differences in
/// overseer output while still requiring substantive equivalence.
///
/// # Arguments
///
/// * `a` -- First sequence of signatures.
/// * `b` -- Second sequence of signatures (must be the same length as `a`).
/// * `threshold` -- Minimum Jaccard similarity (0.0 -- 1.0) for each pair.
///
/// # Returns
///
/// `true` if all corresponding pairs meet or exceed the threshold.
pub fn fuzzy_sequence_match(a: &[String], b: &[String], threshold: f64) -> bool {
    if a.len() != b.len() {
        return false;
    }
    if a.is_empty() {
        return false;
    }

    a.iter().zip(b.iter()).all(|(sa, sb)| {
        let similarity = bigram_jaccard_similarity(sa, sb);
        similarity >= threshold
    })
}

/// Compute the Jaccard similarity of character bigrams between two strings.
///
/// Bigram Jaccard similarity is more robust than exact string matching for
/// overseer fingerprints, which may have minor ordering or formatting differences
/// across otherwise equivalent results.
///
/// Returns 1.0 for identical strings, 0.0 for completely disjoint bigram sets,
/// and values in between for partial overlap.
fn bigram_jaccard_similarity(a: &str, b: &str) -> f64 {
    if a == b {
        return 1.0;
    }
    if a.len() < 2 || b.len() < 2 {
        return if a == b { 1.0 } else { 0.0 };
    }

    let bigrams_a: std::collections::HashSet<(char, char)> = a
        .chars()
        .collect::<Vec<_>>()
        .windows(2)
        .map(|w| (w[0], w[1]))
        .collect();

    let bigrams_b: std::collections::HashSet<(char, char)> = b
        .chars()
        .collect::<Vec<_>>()
        .windows(2)
        .map(|w| (w[0], w[1]))
        .collect();

    let intersection = bigrams_a.intersection(&bigrams_b).count();
    let union = bigrams_a.union(&bigrams_b).count();

    if union == 0 {
        return 0.0;
    }

    intersection as f64 / union as f64
}

// ---------------------------------------------------------------------------
// fingerprint_overseer_results
// ---------------------------------------------------------------------------

/// Create a string fingerprint from overseer signals for cycle detection.
///
/// The fingerprint is a deterministic, compact representation of which overseers
/// passed and failed, including key numeric counts. Two observations with
/// substantively identical overseer results will produce identical (or
/// near-identical) fingerprints.
///
/// # Fingerprint Format
///
/// The fingerprint is a pipe-separated list of fields:
///
/// ```text
/// build:<pass|fail>|types:<clean|N_errors>|tests:<passed>/<total>r<regressions>|
/// lint:<N_errors>|sec:<crit>c<high>h|custom:<passed>/<total>
/// ```
///
/// # Arguments
///
/// * `signals` -- The overseer signals from a single observation.
///
/// # Returns
///
/// A deterministic string fingerprint suitable for cycle detection via
/// fuzzy matching.
pub fn fingerprint_overseer_results(signals: &OverseerSignals) -> String {
    let mut parts = Vec::new();

    // Build result
    if let Some(ref build) = signals.build_result {
        parts.push(format!(
            "build:{}",
            if build.success { "pass" } else { "fail" }
        ));
    }

    // Type check
    if let Some(ref tc) = signals.type_check {
        if tc.clean {
            parts.push(String::from("types:clean"));
        } else {
            parts.push(format!("types:{}_errors", tc.error_count));
        }
    }

    // Test results
    if let Some(ref tests) = signals.test_results {
        parts.push(format!(
            "tests:{}/{}r{}",
            tests.passed, tests.total, tests.regression_count
        ));
    }

    // Lint results
    if let Some(ref lint) = signals.lint_results {
        parts.push(format!("lint:{}_errors", lint.error_count));
    }

    // Security scan
    if let Some(ref sec) = signals.security_scan {
        parts.push(format!("sec:{}c{}h", sec.critical_count, sec.high_count));
    }

    // Custom checks
    if !signals.custom_checks.is_empty() {
        let passed = signals.custom_checks.iter().filter(|c| c.passed).count();
        parts.push(format!(
            "custom:{}/{}",
            passed,
            signals.custom_checks.len()
        ));
    }

    if parts.is_empty() {
        String::from("no_signals")
    } else {
        parts.join("|")
    }
}

// ---------------------------------------------------------------------------
// infer_divergence_cause
// ---------------------------------------------------------------------------

/// Infer the probable cause of a diverging trajectory.
///
/// Examines the recent observations for patterns that indicate *why* the
/// trajectory is moving away from the target. The inference priority is:
///
/// 1. **AccumulatedRegression** -- Any observation with `test_regression_count > 0`
///    indicates that fixes are breaking previously passing tests.
///
/// 2. **SpecificationAmbiguity** -- Any observation with a verification result
///    containing ambiguity gaps suggests the spec is unclear, causing oscillation
///    between conflicting interpretations.
///
/// 3. **WrongApproach** -- All consecutive signature pairs are different, meaning
///    each iteration produces a fundamentally different failure mode. The current
///    approach is not converging on any stable state.
///
/// 4. **Unknown** -- None of the above patterns are present.
///
/// # Arguments
///
/// * `recent` -- The windowed slice of recent observations.
///
/// # Returns
///
/// The inferred `DivergenceCause`.
pub fn infer_divergence_cause(recent: &[Observation]) -> DivergenceCause {
    // Check for regressions in any recent observation.
    let has_regressions = recent
        .iter()
        .filter_map(|o| o.metrics.as_ref())
        .any(|m| m.test_regression_count > 0);

    // Check for ambiguity signals in verification results.
    let has_ambiguity = recent.iter().any(|o| {
        o.verification
            .as_ref()
            .map(|v| v.has_ambiguity_gaps())
            .unwrap_or(false)
    });

    // Check if overseer signatures are all different (no stability).
    let signatures_vary = {
        let sigs: Vec<String> = recent
            .iter()
            .map(|o| fingerprint_overseer_results(&o.overseer_signals))
            .collect();
        sigs.len() >= 2 && sigs.windows(2).all(|w| w[0] != w[1])
    };

    if has_regressions {
        DivergenceCause::AccumulatedRegression
    } else if has_ambiguity {
        DivergenceCause::SpecificationAmbiguity
    } else if signatures_vary {
        DivergenceCause::WrongApproach
    } else {
        DivergenceCause::Unknown
    }
}

// ---------------------------------------------------------------------------
// estimate_remaining_iterations
// ---------------------------------------------------------------------------

/// Estimate how many additional iterations are needed to reach convergence.
///
/// Uses a simple linear projection: given the current convergence rate (average
/// positive delta per iteration) and the current convergence level, estimates how
/// many more iterations at the current rate would be needed to reach level 1.0.
///
/// The estimate is clamped to the range `[1, 20]` to avoid degenerate predictions
/// from very small or very large rates.
///
/// # Arguments
///
/// * `rate` -- The average convergence delta (should be positive for a FixedPoint
///   trajectory).
/// * `level` -- The current convergence level (0.0 -- 1.0).
///
/// # Returns
///
/// Estimated remaining iterations as a `u32`, clamped to `[1, 20]`.
pub fn estimate_remaining_iterations(rate: f64, level: f64) -> u32 {
    if rate <= 0.0 {
        return 20; // No positive progress; return max estimate.
    }

    let remaining_distance = (1.0 - level).max(0.0);
    let raw_estimate = (remaining_distance / rate).ceil() as u32;

    raw_estimate.clamp(1, 20)
}

// ---------------------------------------------------------------------------
// compute_tendency (internal helper)
// ---------------------------------------------------------------------------

/// Compute the convergence tendency from a slice of deltas.
///
/// Uses the average of the most recent deltas (up to 3) to determine the
/// overall direction. This smooths out single-observation noise while still
/// being responsive to recent changes.
fn compute_tendency(deltas: &[f64]) -> ConvergenceTendency {
    if deltas.is_empty() {
        return ConvergenceTendency::Flat;
    }

    let recent_count = deltas.len().min(3);
    let recent_avg: f64 =
        deltas[deltas.len() - recent_count..].iter().sum::<f64>() / recent_count as f64;

    if recent_avg > PLATEAU_EPSILON {
        ConvergenceTendency::Improving
    } else if recent_avg < -PLATEAU_EPSILON {
        ConvergenceTendency::Declining
    } else {
        ConvergenceTendency::Flat
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: create a test artifact reference.
    fn test_artifact(seq: u32) -> ArtifactReference {
        ArtifactReference::new(
            format!("/worktree/task/artifact_{}.rs", seq),
            format!("hash_{}", seq),
        )
    }

    /// Build a minimal `Observation` with the given metrics for testing.
    fn make_observation(
        seq: u32,
        delta: Option<f64>,
        level: f64,
        regression_count: u32,
    ) -> Observation {
        Observation {
            id: Uuid::new_v4(),
            sequence: seq,
            timestamp: chrono::Utc::now(),
            artifact: test_artifact(seq),
            overseer_signals: OverseerSignals::default(),
            verification: None,
            metrics: delta.map(|d| ObservationMetrics {
                ast_diff_nodes: 10,
                test_pass_delta: 0,
                test_regression_count: regression_count,
                error_count_delta: 0,
                vulnerability_delta: 0,
                convergence_delta: d,
                convergence_level: level,
                intent_blended_level: None,
            }),
            tokens_used: 20_000,
            wall_time_ms: 5_000,
            strategy_used: StrategyKind::RetryWithFeedback,
        }
    }

    /// Build an observation with specific overseer signals for fingerprinting tests.
    #[allow(dead_code)]
    fn make_observation_with_signals(
        seq: u32,
        delta: Option<f64>,
        level: f64,
        signals: OverseerSignals,
    ) -> Observation {
        Observation {
            id: Uuid::new_v4(),
            sequence: seq,
            timestamp: chrono::Utc::now(),
            artifact: test_artifact(seq),
            overseer_signals: signals,
            verification: None,
            metrics: delta.map(|d| ObservationMetrics {
                ast_diff_nodes: 10,
                test_pass_delta: 0,
                test_regression_count: 0,
                error_count_delta: 0,
                vulnerability_delta: 0,
                convergence_delta: d,
                convergence_level: level,
                intent_blended_level: None,
            }),
            tokens_used: 20_000,
            wall_time_ms: 5_000,
            strategy_used: StrategyKind::RetryWithFeedback,
        }
    }

    // --- AttractorState Default ---

    #[test]
    fn test_default_attractor_state() {
        let state = AttractorState::default();
        assert!(matches!(
            state.classification,
            AttractorType::Indeterminate {
                tendency: ConvergenceTendency::Flat
            }
        ));
        assert_eq!(state.confidence, 0.0);
        assert!(state.detected_at.is_none());
    }

    // --- classify_attractor: Indeterminate (insufficient data) ---

    #[test]
    fn test_classify_too_few_observations() {
        let obs = vec![
            make_observation(0, None, 0.0, 0),
            make_observation(1, Some(0.1), 0.1, 0),
        ];
        let result = classify_attractor(&obs, 5);
        assert!(matches!(
            result.classification,
            AttractorType::Indeterminate { .. }
        ));
    }

    #[test]
    fn test_classify_indeterminate_tendency_improving() {
        let obs = vec![
            make_observation(0, None, 0.0, 0),
            make_observation(1, Some(0.5), 0.3, 0),
        ];
        let result = classify_attractor(&obs, 5);
        assert!(matches!(
            result.classification,
            AttractorType::Indeterminate {
                tendency: ConvergenceTendency::Improving
            }
        ));
    }

    #[test]
    fn test_classify_indeterminate_tendency_declining() {
        let obs = vec![
            make_observation(0, None, 0.0, 0),
            make_observation(1, Some(-0.5), 0.3, 0),
        ];
        let result = classify_attractor(&obs, 5);
        assert!(matches!(
            result.classification,
            AttractorType::Indeterminate {
                tendency: ConvergenceTendency::Declining
            }
        ));
    }

    // --- classify_attractor: FixedPoint ---

    #[test]
    fn test_classify_fixed_point() {
        let obs = vec![
            make_observation(0, None, 0.0, 0),
            make_observation(1, Some(0.15), 0.15, 0),
            make_observation(2, Some(0.12), 0.27, 0),
            make_observation(3, Some(0.10), 0.37, 0),
            make_observation(4, Some(0.08), 0.45, 0),
        ];
        let result = classify_attractor(&obs, 5);
        assert!(
            matches!(result.classification, AttractorType::FixedPoint { .. }),
            "Expected FixedPoint, got {:?}",
            result.classification
        );
    }

    // --- classify_attractor: Divergent ---

    #[test]
    fn test_classify_divergent() {
        let obs = vec![
            make_observation(0, None, 0.5, 0),
            make_observation(1, Some(-0.10), 0.40, 0),
            make_observation(2, Some(-0.08), 0.32, 0),
            make_observation(3, Some(-0.12), 0.20, 0),
            make_observation(4, Some(-0.05), 0.15, 0),
        ];
        let result = classify_attractor(&obs, 5);
        assert!(
            matches!(result.classification, AttractorType::Divergent { .. }),
            "Expected Divergent, got {:?}",
            result.classification
        );
        if let AttractorType::Divergent {
            divergence_rate, ..
        } = result.classification
        {
            assert!(divergence_rate < 0.0);
        }
    }

    // --- classify_attractor: Plateau ---

    #[test]
    fn test_classify_plateau() {
        let obs = vec![
            make_observation(0, None, 0.5, 0),
            make_observation(1, Some(0.005), 0.505, 0),
            make_observation(2, Some(-0.003), 0.502, 0),
            make_observation(3, Some(0.002), 0.504, 0),
            make_observation(4, Some(-0.001), 0.503, 0),
        ];
        let result = classify_attractor(&obs, 5);
        assert!(
            matches!(result.classification, AttractorType::Plateau { .. }),
            "Expected Plateau, got {:?}",
            result.classification
        );
        if let AttractorType::Plateau {
            plateau_level,
            stall_duration,
            ..
        } = result.classification
        {
            assert!(plateau_level > 0.4);
            assert!(stall_duration >= 2);
        }
    }

    // --- detect_cycle ---

    #[test]
    fn test_detect_cycle_period_2() {
        let sigs: Vec<String> = vec![
            "build:pass|tests:5/10r0",
            "build:pass|tests:8/10r2",
            "build:pass|tests:5/10r0",
            "build:pass|tests:8/10r2",
        ]
        .into_iter()
        .map(String::from)
        .collect();

        assert_eq!(detect_cycle(&sigs), Some(2));
    }

    #[test]
    fn test_detect_cycle_no_cycle() {
        let sigs: Vec<String> = vec![
            "build:pass|tests:5/10r0",
            "build:pass|tests:6/10r0",
            "build:pass|tests:7/10r0",
            "build:pass|tests:8/10r0",
        ]
        .into_iter()
        .map(String::from)
        .collect();

        assert_eq!(detect_cycle(&sigs), None);
    }

    #[test]
    fn test_detect_cycle_insufficient_data() {
        let sigs: Vec<String> = vec!["build:pass|tests:5/10r0"]
            .into_iter()
            .map(String::from)
            .collect();

        assert_eq!(detect_cycle(&sigs), None);
    }

    // --- fuzzy_sequence_match ---

    #[test]
    fn test_fuzzy_sequence_match_identical() {
        let a = vec![String::from("abc"), String::from("def")];
        let b = vec![String::from("abc"), String::from("def")];
        assert!(fuzzy_sequence_match(&a, &b, 0.85));
    }

    #[test]
    fn test_fuzzy_sequence_match_different() {
        let a = vec![String::from("abc"), String::from("def")];
        let b = vec![String::from("xyz"), String::from("uvw")];
        assert!(!fuzzy_sequence_match(&a, &b, 0.85));
    }

    #[test]
    fn test_fuzzy_sequence_match_empty() {
        let a: Vec<String> = vec![];
        let b: Vec<String> = vec![];
        assert!(!fuzzy_sequence_match(&a, &b, 0.85));
    }

    #[test]
    fn test_fuzzy_sequence_match_length_mismatch() {
        let a = vec![String::from("abc")];
        let b = vec![String::from("abc"), String::from("def")];
        assert!(!fuzzy_sequence_match(&a, &b, 0.85));
    }

    // --- fingerprint_overseer_results ---

    #[test]
    fn test_fingerprint_empty_signals() {
        let signals = OverseerSignals::default();
        let fp = fingerprint_overseer_results(&signals);
        assert_eq!(fp, "no_signals");
    }

    #[test]
    fn test_fingerprint_with_test_results() {
        let signals = OverseerSignals {
            test_results: Some(TestResults {
                passed: 8,
                failed: 2,
                skipped: 0,
                total: 10,
                regression_count: 1,
                failing_test_names: vec!["test_a".into(), "test_b".into()],
            }),
            ..OverseerSignals::default()
        };
        let fp = fingerprint_overseer_results(&signals);
        assert!(fp.contains("tests:8/10r1"));
    }

    #[test]
    fn test_fingerprint_deterministic() {
        let signals = OverseerSignals {
            build_result: Some(BuildResult {
                success: true,
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
        let fp1 = fingerprint_overseer_results(&signals);
        let fp2 = fingerprint_overseer_results(&signals);
        assert_eq!(fp1, fp2);
    }

    // --- estimate_remaining_iterations ---

    #[test]
    fn test_estimate_remaining_basic() {
        // Rate 0.1 per iteration, currently at 0.5 => need 5 more
        let remaining = estimate_remaining_iterations(0.1, 0.5);
        assert_eq!(remaining, 5);
    }

    #[test]
    fn test_estimate_remaining_zero_rate() {
        let remaining = estimate_remaining_iterations(0.0, 0.5);
        assert_eq!(remaining, 20); // max
    }

    #[test]
    fn test_estimate_remaining_negative_rate() {
        let remaining = estimate_remaining_iterations(-0.1, 0.5);
        assert_eq!(remaining, 20); // max for non-positive rates
    }

    #[test]
    fn test_estimate_remaining_near_done() {
        let remaining = estimate_remaining_iterations(0.1, 0.95);
        assert_eq!(remaining, 1); // clamped to minimum 1
    }

    #[test]
    fn test_estimate_remaining_clamp_high() {
        // Rate 0.001 per iteration, currently at 0.0 => 1000 iterations => clamped to 20
        let remaining = estimate_remaining_iterations(0.001, 0.0);
        assert_eq!(remaining, 20);
    }

    // --- infer_divergence_cause ---

    #[test]
    fn test_infer_regression_cause() {
        let obs = vec![
            make_observation(0, Some(-0.1), 0.4, 3),
            make_observation(1, Some(-0.1), 0.3, 2),
        ];
        let cause = infer_divergence_cause(&obs);
        assert!(matches!(cause, DivergenceCause::AccumulatedRegression));
    }

    #[test]
    fn test_infer_unknown_cause() {
        // No regressions, no ambiguity, same signatures (default signals)
        let obs = vec![
            make_observation(0, Some(-0.1), 0.4, 0),
            make_observation(1, Some(-0.1), 0.3, 0),
        ];
        let cause = infer_divergence_cause(&obs);
        // With default signals, signatures are identical ("no_signals"), so
        // signatures_vary is false => Unknown.
        assert!(matches!(cause, DivergenceCause::Unknown));
    }

    // --- compute_tendency ---

    #[test]
    fn test_compute_tendency_improving() {
        let deltas = vec![0.05, 0.10, 0.08];
        let tendency = compute_tendency(&deltas);
        assert!(matches!(tendency, ConvergenceTendency::Improving));
    }

    #[test]
    fn test_compute_tendency_declining() {
        let deltas = vec![-0.05, -0.10, -0.08];
        let tendency = compute_tendency(&deltas);
        assert!(matches!(tendency, ConvergenceTendency::Declining));
    }

    #[test]
    fn test_compute_tendency_flat() {
        let deltas = vec![0.001, -0.001, 0.0];
        let tendency = compute_tendency(&deltas);
        assert!(matches!(tendency, ConvergenceTendency::Flat));
    }

    #[test]
    fn test_compute_tendency_empty() {
        let deltas: Vec<f64> = vec![];
        let tendency = compute_tendency(&deltas);
        assert!(matches!(tendency, ConvergenceTendency::Flat));
    }

    // --- bigram_jaccard_similarity ---

    #[test]
    fn test_bigram_similarity_identical() {
        assert_eq!(bigram_jaccard_similarity("hello", "hello"), 1.0);
    }

    #[test]
    fn test_bigram_similarity_disjoint() {
        let sim = bigram_jaccard_similarity("ab", "yz");
        assert_eq!(sim, 0.0);
    }

    #[test]
    fn test_bigram_similarity_partial() {
        let sim = bigram_jaccard_similarity("abc", "abd");
        // bigrams: {(a,b),(b,c)} vs {(a,b),(b,d)} => intersection=1, union=3
        assert!((sim - 1.0 / 3.0).abs() < 0.01);
    }
}
