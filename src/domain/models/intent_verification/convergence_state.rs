//! Convergence state machines: iteration state, gap fingerprints, embedding config.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::verification::{
    GapCategory, GapSeverity, IntentGap, IntentSatisfaction, IntentVerificationResult,
    OriginalIntent,
};

/// Configuration for the convergence loop.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConvergenceConfig {
    /// Maximum iterations before giving up
    pub max_iterations: u32,
    /// Minimum confidence to accept partial satisfaction
    pub min_confidence_threshold: f64,
    /// Whether to require explicit satisfaction (vs. partial)
    pub require_full_satisfaction: bool,
    /// Whether to automatically retry on partial satisfaction
    pub auto_retry_partial: bool,
    /// Timeout for the entire convergence loop (seconds)
    pub convergence_timeout_secs: u64,
}

impl Default for ConvergenceConfig {
    fn default() -> Self {
        Self {
            max_iterations: 3,
            min_confidence_threshold: 0.7,
            require_full_satisfaction: false,
            auto_retry_partial: true,
            convergence_timeout_secs: 7200, // 2 hours
        }
    }
}

impl ConvergenceConfig {
    /// Check if we should continue iterating.
    pub fn should_continue(&self, result: &IntentVerificationResult) -> bool {
        // Don't continue if we've hit max iterations
        if result.iteration >= self.max_iterations {
            return false;
        }

        // Don't continue if fully satisfied
        if result.satisfaction == IntentSatisfaction::Satisfied {
            return false;
        }

        // Don't continue if indeterminate (needs human)
        if result.satisfaction == IntentSatisfaction::Indeterminate {
            return false;
        }

        // For partial satisfaction, check config
        if result.satisfaction == IntentSatisfaction::Partial {
            if self.require_full_satisfaction {
                return true;
            }
            // Accept partial if confidence is high enough
            if result.confidence >= self.min_confidence_threshold {
                return false;
            }
            return self.auto_retry_partial;
        }

        // Unsatisfied - continue if we have guidance
        result.should_iterate()
    }
}

/// State of a convergence loop.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConvergenceState {
    /// The intent being converged on
    pub intent: OriginalIntent,
    /// History of verification results
    pub verification_history: Vec<IntentVerificationResult>,
    /// Current iteration number
    pub current_iteration: u32,
    /// Whether convergence has been achieved
    pub converged: bool,
    /// When the loop started
    pub started_at: DateTime<Utc>,
    /// When the loop ended (if done)
    pub ended_at: Option<DateTime<Utc>>,
    /// Detected semantic drift (same gaps recurring)
    pub drift_detected: bool,
    /// Gap fingerprints seen across iterations for drift detection
    pub gap_fingerprints: Vec<GapFingerprint>,
}

/// Fingerprint of a gap for semantic drift detection.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct GapFingerprint {
    /// Normalized description (lowercase, trimmed)
    pub normalized_description: String,
    /// Severity of the gap
    pub severity: GapSeverity,
    /// Which iteration this gap was first seen
    pub first_seen_iteration: u32,
    /// How many times this gap has appeared
    pub occurrence_count: u32,
}

impl ConvergenceState {
    pub fn new(intent: OriginalIntent) -> Self {
        Self {
            intent,
            verification_history: Vec::new(),
            current_iteration: 0,
            converged: false,
            started_at: Utc::now(),
            ended_at: None,
            drift_detected: false,
            gap_fingerprints: Vec::new(),
        }
    }

    /// Record a verification result and update drift detection.
    pub fn record_verification(&mut self, result: IntentVerificationResult) {
        self.current_iteration = result.iteration;
        if result.satisfaction == IntentSatisfaction::Satisfied {
            self.converged = true;
            self.ended_at = Some(Utc::now());
        }

        // Update gap fingerprints for drift detection
        self.update_gap_fingerprints(&result);

        self.verification_history.push(result);
    }

    /// Update gap fingerprints and detect semantic drift.
    fn update_gap_fingerprints(&mut self, result: &IntentVerificationResult) {
        for gap in &result.gaps {
            let normalized = Self::normalize_gap_description(&gap.description);

            // Check if we've seen a similar gap before
            let existing = self.gap_fingerprints.iter_mut().find(|fp| {
                Self::gaps_are_similar(&fp.normalized_description, &normalized)
            });

            if let Some(fingerprint) = existing {
                fingerprint.occurrence_count += 1;
                // If same gap appears 3+ times, we have drift
                if fingerprint.occurrence_count >= 3 {
                    self.drift_detected = true;
                }
            } else {
                self.gap_fingerprints.push(GapFingerprint {
                    normalized_description: normalized,
                    severity: gap.severity,
                    first_seen_iteration: result.iteration,
                    occurrence_count: 1,
                });
            }
        }
    }

    /// Normalize a gap description for comparison.
    fn normalize_gap_description(description: &str) -> String {
        description
            .to_lowercase()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// Check if two gap descriptions are semantically similar.
    /// Uses simple word overlap heuristic.
    fn gaps_are_similar(a: &str, b: &str) -> bool {
        let words_a: std::collections::HashSet<_> = a.split_whitespace().collect();
        let words_b: std::collections::HashSet<_> = b.split_whitespace().collect();

        if words_a.is_empty() || words_b.is_empty() {
            return false;
        }

        let intersection = words_a.intersection(&words_b).count();
        let union = words_a.union(&words_b).count();

        // Jaccard similarity > 0.5 means similar
        (intersection as f64 / union as f64) > 0.5
    }

    /// Get the latest verification result.
    pub fn latest_result(&self) -> Option<&IntentVerificationResult> {
        self.verification_history.last()
    }

    /// Mark the loop as ended (even if not converged).
    pub fn end(&mut self) {
        if self.ended_at.is_none() {
            self.ended_at = Some(Utc::now());
        }
    }

    /// Check if we've made progress across iterations.
    pub fn is_making_progress(&self) -> bool {
        // If drift detected, we're not making progress
        if self.drift_detected {
            return false;
        }

        if self.verification_history.len() < 2 {
            return true; // Not enough data
        }

        let recent: Vec<_> = self.verification_history.iter().rev().take(2).collect();
        if recent.len() < 2 {
            return true;
        }

        // Check if gaps are decreasing
        let current_gaps = recent[0].gaps.len();
        let previous_gaps = recent[1].gaps.len();

        // Check if confidence is increasing
        let current_conf = recent[0].confidence;
        let previous_conf = recent[1].confidence;

        // Check if we're seeing different gaps (progress even if count same)
        let current_gap_set: std::collections::HashSet<_> = recent[0]
            .gaps
            .iter()
            .map(|g| Self::normalize_gap_description(&g.description))
            .collect();
        let previous_gap_set: std::collections::HashSet<_> = recent[1]
            .gaps
            .iter()
            .map(|g| Self::normalize_gap_description(&g.description))
            .collect();
        let gaps_changed = current_gap_set != previous_gap_set;

        current_gaps < previous_gaps || current_conf > previous_conf || gaps_changed
    }

    /// Get recurring gaps (those that have appeared multiple times).
    pub fn recurring_gaps(&self) -> Vec<&GapFingerprint> {
        self.gap_fingerprints
            .iter()
            .filter(|fp| fp.occurrence_count > 1)
            .collect()
    }

    /// Build context about the convergence state for agent prompts.
    ///
    /// Uses progressive history pruning: the last 2 iterations get full
    /// gap details, older iterations only get one-line summaries. This
    /// keeps the iteration context compact for longer convergence loops.
    pub fn build_iteration_context(&self) -> IterationContext {
        let recurring = self.recurring_gaps();
        let recurring_descriptions: Vec<String> = recurring
            .iter()
            .map(|fp| format!(
                "- {} (seen {} times, severity: {})",
                fp.normalized_description,
                fp.occurrence_count,
                fp.severity.as_str()
            ))
            .collect();

        let history_len = self.verification_history.len();
        let detail_cutoff = history_len.saturating_sub(2); // Last 2 get full details

        // Older iterations: one-line summaries only
        let previous_attempts: Vec<String> = self.verification_history
            .iter()
            .enumerate()
            .map(|(i, r)| {
                if i < detail_cutoff {
                    // Compact summary for older iterations
                    format!(
                        "Iteration {}: {} (confidence: {:.0}%, {} gaps)",
                        r.iteration,
                        r.satisfaction.as_str(),
                        r.confidence * 100.0,
                        r.gaps.len()
                    )
                } else {
                    // Detailed summary for recent iterations (last 2)
                    let mut detail = format!(
                        "Iteration {}: {} (confidence: {:.0}%, {} gaps)",
                        r.iteration,
                        r.satisfaction.as_str(),
                        r.confidence * 100.0,
                        r.gaps.len()
                    );
                    if !r.gaps.is_empty() {
                        detail.push_str("\n  Gaps:");
                        for gap in &r.gaps {
                            detail.push_str(&format!(
                                "\n  - [{}] {}",
                                gap.severity.as_str(),
                                gap.description
                            ));
                            if let Some(ref action) = gap.suggested_action {
                                detail.push_str(&format!(" (action: {})", action));
                            }
                        }
                    }
                    detail
                }
            })
            .collect();

        IterationContext {
            current_iteration: self.current_iteration + 1, // Next iteration
            total_iterations_so_far: self.current_iteration,
            drift_detected: self.drift_detected,
            recurring_gap_descriptions: recurring_descriptions,
            previous_attempt_summaries: previous_attempts,
            focus_areas: self.latest_result()
                .and_then(|r| r.reprompt_guidance.as_ref())
                .map(|g| g.focus_areas.clone())
                .unwrap_or_default(),
        }
    }
}

/// Context about the current convergence iteration for agent prompts.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IterationContext {
    /// Which iteration this is (1-indexed)
    pub current_iteration: u32,
    /// How many iterations have been attempted
    pub total_iterations_so_far: u32,
    /// Whether semantic drift has been detected
    pub drift_detected: bool,
    /// Descriptions of gaps that keep recurring
    pub recurring_gap_descriptions: Vec<String>,
    /// Summaries of previous attempts
    pub previous_attempt_summaries: Vec<String>,
    /// Focus areas from latest verification
    pub focus_areas: Vec<String>,
}

impl IterationContext {
    /// Format as a section for agent system prompts.
    pub fn format_for_prompt(&self) -> String {
        if self.total_iterations_so_far == 0 {
            return String::new();
        }

        let mut context = String::from("\n\n## Convergence Loop Context\n\n");
        context.push_str(&format!(
            "**This is iteration {} of a convergence loop.**\n\n",
            self.current_iteration
        ));

        if !self.previous_attempt_summaries.is_empty() {
            context.push_str("### Previous Attempts\n");
            for summary in &self.previous_attempt_summaries {
                context.push_str(&format!("- {}\n", summary));
            }
            context.push('\n');
        }

        if self.drift_detected {
            context.push_str("**WARNING: Semantic drift detected.** The same gaps keep appearing across iterations.\n");
            context.push_str("Please carefully review whether you are truly addressing the root cause.\n\n");
        }

        if !self.recurring_gap_descriptions.is_empty() {
            context.push_str("### Recurring Gaps (NOT YET RESOLVED)\n");
            context.push_str("These issues have appeared multiple times and MUST be addressed:\n");
            for gap in &self.recurring_gap_descriptions {
                context.push_str(&format!("{}\n", gap));
            }
            context.push('\n');
        }

        if !self.focus_areas.is_empty() {
            context.push_str("### Required Focus Areas\n");
            context.push_str("Based on previous verification, focus on:\n");
            for area in &self.focus_areas {
                context.push_str(&format!("- {}\n", area));
            }
            context.push('\n');
        }

        context.push_str("---\n");
        context
    }
}

// ============================================================================
// Embedding-Based Similarity
// ============================================================================

/// Configuration for embedding-based gap similarity.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbeddingSimilarityConfig {
    /// Minimum cosine similarity to consider gaps as the same (0.0-1.0)
    pub similarity_threshold: f64,
    /// Whether to fall back to Jaccard if embeddings unavailable
    pub fallback_to_jaccard: bool,
    /// Model to use for embeddings (if using an embedding service)
    pub embedding_model: String,
    /// Embedding dimension (depends on model)
    pub embedding_dimension: usize,
}

impl Default for EmbeddingSimilarityConfig {
    fn default() -> Self {
        Self {
            similarity_threshold: 0.85,
            fallback_to_jaccard: true,
            embedding_model: "text-embedding-3-small".to_string(),
            embedding_dimension: 1536,
        }
    }
}

/// Enhanced gap fingerprint with embedding support.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbeddedGapFingerprint {
    /// Normalized description
    pub normalized_description: String,
    /// Embedding vector (if computed)
    pub embedding: Option<Vec<f32>>,
    /// Severity
    pub severity: GapSeverity,
    /// Category
    pub category: GapCategory,
    /// First seen iteration
    pub first_seen_iteration: u32,
    /// Occurrence count
    pub occurrence_count: u32,
    /// IDs of similar gaps that were merged into this fingerprint
    pub merged_gap_ids: Vec<Uuid>,
}

impl EmbeddedGapFingerprint {
    pub fn from_gap(gap: &IntentGap, iteration: u32) -> Self {
        Self {
            normalized_description: gap.description.to_lowercase().split_whitespace().collect::<Vec<_>>().join(" "),
            embedding: gap.embedding.clone(),
            severity: gap.severity,
            category: gap.category,
            first_seen_iteration: iteration,
            occurrence_count: 1,
            merged_gap_ids: Vec::new(),
        }
    }

    /// Check if another gap is similar to this fingerprint.
    pub fn is_similar_to(&self, other: &IntentGap, config: &EmbeddingSimilarityConfig) -> bool {
        // Try embedding similarity first
        if let (Some(self_emb), Some(other_emb)) = (&self.embedding, &other.embedding) {
            let similarity = cosine_similarity(self_emb, other_emb);
            return similarity >= config.similarity_threshold;
        }

        // Fall back to Jaccard if configured
        if config.fallback_to_jaccard {
            let other_normalized = other.description.to_lowercase()
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ");
            return jaccard_similarity(&self.normalized_description, &other_normalized) > 0.5;
        }

        false
    }

    /// Merge another similar gap into this fingerprint.
    ///
    /// Updates the embedding via weighted average `E' = (E*n + I) / (n+1)`
    /// so that repeated merges converge the fingerprint toward the centroid
    /// of the gaps it represents. The result is always renormalized to unit
    /// length so cosine similarity stays stable across merges.
    ///
    /// This relies on the invariant that embeddings enter the pipeline
    /// already L2-normalized — see `src/adapters/embeddings/mod.rs`
    /// (`normalize_unit`). All production providers enforce it; tests that
    /// synthesize `IntentGap::embedding` directly should supply unit vectors.
    pub fn merge(&mut self, gap: &IntentGap) {
        match (self.embedding.as_mut(), gap.embedding.as_ref()) {
            (Some(existing), Some(incoming)) => {
                if existing.len() != incoming.len() {
                    tracing::warn!(
                        existing_dim = existing.len(),
                        incoming_dim = incoming.len(),
                        "gap fingerprint embedding dimension mismatch; skipping embedding merge"
                    );
                } else {
                    let n = self.occurrence_count as f32;
                    if n == 0.0 {
                        // First occurrence: adopt incoming directly.
                        for (e, i) in existing.iter_mut().zip(incoming.iter()) {
                            *e = *i;
                        }
                    } else {
                        // Weighted average: E' = (E*n + I) / (n+1)
                        for (e, i) in existing.iter_mut().zip(incoming.iter()) {
                            *e = (*e * n + *i) / (n + 1.0);
                        }
                        // Unconditionally renormalize. Adapter-level
                        // normalize_unit guarantees unit-length inputs, so the
                        // centroid is always a well-defined direction.
                        let norm_sq: f32 = existing.iter().map(|x| x * x).sum();
                        let norm = norm_sq.sqrt();
                        if norm > 0.0 && norm.is_finite() {
                            for e in existing.iter_mut() {
                                *e /= norm;
                            }
                        }
                    }
                }
            }
            (None, Some(incoming)) => {
                // Existing had no embedding; adopt incoming wholesale.
                self.embedding = Some(incoming.clone());
            }
            _ => {
                // Either both None or incoming is None: nothing to update.
            }
        }
        self.occurrence_count += 1;
    }
}

/// Calculate cosine similarity between two embedding vectors.
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f64 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }

    let dot_product: f64 = a.iter().zip(b.iter()).map(|(x, y)| (*x as f64) * (*y as f64)).sum();
    let norm_a: f64 = a.iter().map(|x| (*x as f64).powi(2)).sum::<f64>().sqrt();
    let norm_b: f64 = b.iter().map(|x| (*x as f64).powi(2)).sum::<f64>().sqrt();

    if norm_a == 0.0 || norm_b == 0.0 {
        return 0.0;
    }

    dot_product / (norm_a * norm_b)
}

/// Calculate Jaccard similarity between two normalized strings.
pub fn jaccard_similarity(a: &str, b: &str) -> f64 {
    let words_a: std::collections::HashSet<_> = a.split_whitespace().collect();
    let words_b: std::collections::HashSet<_> = b.split_whitespace().collect();

    if words_a.is_empty() || words_b.is_empty() {
        return 0.0;
    }

    let intersection = words_a.intersection(&words_b).count();
    let union = words_a.union(&words_b).count();

    intersection as f64 / union as f64
}

/// Enhanced convergence state with embedding-based drift detection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnhancedConvergenceState {
    /// Base convergence state
    pub base: ConvergenceState,
    /// Embedded gap fingerprints for better similarity matching
    pub embedded_fingerprints: Vec<EmbeddedGapFingerprint>,
    /// Configuration for similarity matching
    pub similarity_config: EmbeddingSimilarityConfig,
    /// Clusters of related gaps (gaps that address similar issues)
    pub gap_clusters: Vec<GapCluster>,
}

impl EnhancedConvergenceState {
    pub fn new(intent: OriginalIntent) -> Self {
        Self {
            base: ConvergenceState::new(intent),
            embedded_fingerprints: Vec::new(),
            similarity_config: EmbeddingSimilarityConfig::default(),
            gap_clusters: Vec::new(),
        }
    }

    pub fn with_similarity_config(mut self, config: EmbeddingSimilarityConfig) -> Self {
        self.similarity_config = config;
        self
    }

    /// Record a verification result with embedding-based similarity.
    pub fn record_verification(&mut self, result: IntentVerificationResult) {
        let iteration = result.iteration;

        // Update embedded fingerprints
        for gap in result.all_gaps() {
            self.update_embedded_fingerprint(gap, iteration);
        }

        // Check for drift using embedded fingerprints
        self.check_drift();

        // Update base state
        self.base.record_verification(result);
    }

    fn update_embedded_fingerprint(&mut self, gap: &IntentGap, iteration: u32) {
        // Find similar existing fingerprint
        let similar_idx = self.embedded_fingerprints.iter()
            .position(|fp| fp.is_similar_to(gap, &self.similarity_config));

        if let Some(idx) = similar_idx {
            self.embedded_fingerprints[idx].merge(gap);
        } else {
            self.embedded_fingerprints.push(EmbeddedGapFingerprint::from_gap(gap, iteration));
        }
    }

    fn check_drift(&mut self) {
        // Drift if any fingerprint has 3+ occurrences
        self.base.drift_detected = self.embedded_fingerprints.iter()
            .any(|fp| fp.occurrence_count >= 3);
    }

    /// Get recurring gaps with their full context.
    pub fn recurring_gaps_detailed(&self) -> Vec<&EmbeddedGapFingerprint> {
        self.embedded_fingerprints.iter()
            .filter(|fp| fp.occurrence_count > 1)
            .collect()
    }

    /// Delegate to base state
    pub fn converged(&self) -> bool {
        self.base.converged
    }

    pub fn drift_detected(&self) -> bool {
        self.base.drift_detected
    }

    pub fn is_making_progress(&self) -> bool {
        self.base.is_making_progress()
    }

    pub fn end(&mut self) {
        self.base.end();
    }
}

/// A cluster of related gaps.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GapCluster {
    /// Unique identifier
    pub id: Uuid,
    /// Representative description for this cluster
    pub representative_description: String,
    /// Gap IDs in this cluster
    pub gap_ids: Vec<Uuid>,
    /// Centroid embedding (average of all gaps)
    pub centroid: Option<Vec<f32>>,
    /// Dominant category
    pub dominant_category: GapCategory,
    /// Maximum severity in cluster
    pub max_severity: GapSeverity,
}

impl GapCluster {
    pub fn new(representative: impl Into<String>, category: GapCategory, severity: GapSeverity) -> Self {
        Self {
            id: Uuid::new_v4(),
            representative_description: representative.into(),
            gap_ids: Vec::new(),
            centroid: None,
            dominant_category: category,
            max_severity: severity,
        }
    }

    pub fn add_gap(&mut self, gap_id: Uuid, severity: GapSeverity) {
        self.gap_ids.push(gap_id);
        if severity > self.max_severity {
            self.max_severity = severity;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::guidance::{RepromptApproach, RepromptGuidance};

    #[test]
    fn test_convergence_config_should_continue() {
        let config = ConvergenceConfig::default();
        let intent_id = Uuid::new_v4();

        // Satisfied - don't continue
        let result = IntentVerificationResult::new(intent_id, IntentSatisfaction::Satisfied);
        assert!(!config.should_continue(&result));

        // Max iterations - don't continue
        let result = IntentVerificationResult::new(intent_id, IntentSatisfaction::Partial)
            .with_iteration(3);
        assert!(!config.should_continue(&result));

        // Partial with low confidence - continue
        let result = IntentVerificationResult::new(intent_id, IntentSatisfaction::Partial)
            .with_confidence(0.5)
            .with_reprompt_guidance(RepromptGuidance::new(RepromptApproach::RetryWithContext));
        assert!(config.should_continue(&result));
    }

    #[test]
    fn test_convergence_state_progress() {
        let intent = OriginalIntent::from_goal(Uuid::new_v4(), "Test goal");
        let mut state = ConvergenceState::new(intent);

        // Add first result with 3 gaps
        let result1 = IntentVerificationResult::new(Uuid::new_v4(), IntentSatisfaction::Partial)
            .with_iteration(1)
            .with_confidence(0.4)
            .with_gap(IntentGap::new("Gap 1", GapSeverity::Major))
            .with_gap(IntentGap::new("Gap 2", GapSeverity::Moderate))
            .with_gap(IntentGap::new("Gap 3", GapSeverity::Minor));
        state.record_verification(result1);

        // Add second result with 2 gaps (progress!)
        let result2 = IntentVerificationResult::new(Uuid::new_v4(), IntentSatisfaction::Partial)
            .with_iteration(2)
            .with_confidence(0.6)
            .with_gap(IntentGap::new("Gap 1", GapSeverity::Moderate))
            .with_gap(IntentGap::new("Gap 2", GapSeverity::Minor));
        state.record_verification(result2);

        assert!(state.is_making_progress());
        assert_eq!(state.current_iteration, 2);
        assert!(!state.converged);
    }

    #[test]
    fn test_semantic_drift_detection() {
        let intent = OriginalIntent::from_goal(Uuid::new_v4(), "Test goal");
        let mut state = ConvergenceState::new(intent);

        // Add same gap across 3 iterations - should trigger drift
        for i in 1..=3 {
            let result = IntentVerificationResult::new(Uuid::new_v4(), IntentSatisfaction::Partial)
                .with_iteration(i)
                .with_confidence(0.5)
                .with_gap(IntentGap::new("Missing error handling", GapSeverity::Major));
            state.record_verification(result);
        }

        assert!(state.drift_detected);
        assert!(!state.is_making_progress());

        let recurring = state.recurring_gaps();
        assert_eq!(recurring.len(), 1);
        assert_eq!(recurring[0].occurrence_count, 3);
    }

    #[test]
    fn test_gap_similarity_detection() {
        let intent = OriginalIntent::from_goal(Uuid::new_v4(), "Test goal");
        let mut state = ConvergenceState::new(intent);

        // Add similar gaps with slight variations - should be detected as same
        let result1 = IntentVerificationResult::new(Uuid::new_v4(), IntentSatisfaction::Partial)
            .with_iteration(1)
            .with_gap(IntentGap::new("Missing error handling in API", GapSeverity::Major));
        state.record_verification(result1);

        let result2 = IntentVerificationResult::new(Uuid::new_v4(), IntentSatisfaction::Partial)
            .with_iteration(2)
            .with_gap(IntentGap::new("error handling missing in API", GapSeverity::Major));
        state.record_verification(result2);

        let result3 = IntentVerificationResult::new(Uuid::new_v4(), IntentSatisfaction::Partial)
            .with_iteration(3)
            .with_gap(IntentGap::new("API missing error handling", GapSeverity::Major));
        state.record_verification(result3);

        // Should detect drift because the gaps are semantically similar
        assert!(state.drift_detected);
    }

    #[test]
    fn test_iteration_context_formatting() {
        let intent = OriginalIntent::from_goal(Uuid::new_v4(), "Test goal");
        let mut state = ConvergenceState::new(intent);

        // Add a verification result
        let result = IntentVerificationResult::new(Uuid::new_v4(), IntentSatisfaction::Partial)
            .with_iteration(1)
            .with_confidence(0.6)
            .with_gap(IntentGap::new("Missing tests", GapSeverity::Major))
            .with_reprompt_guidance(
                RepromptGuidance::new(RepromptApproach::RetryWithContext)
                    .with_focus("Unit tests")
            );
        state.record_verification(result);

        let context = state.build_iteration_context();
        assert_eq!(context.current_iteration, 2); // Next iteration
        assert_eq!(context.total_iterations_so_far, 1);
        assert!(!context.previous_attempt_summaries.is_empty());

        let formatted = context.format_for_prompt();
        assert!(formatted.contains("iteration 2"));
        assert!(formatted.contains("### Previous Attempts"));
    }

    #[test]
    fn test_iteration_context_with_drift() {
        let intent = OriginalIntent::from_goal(Uuid::new_v4(), "Test goal");
        let mut state = ConvergenceState::new(intent);

        // Force drift detection
        for i in 1..=3 {
            let result = IntentVerificationResult::new(Uuid::new_v4(), IntentSatisfaction::Partial)
                .with_iteration(i)
                .with_gap(IntentGap::new("Same recurring gap", GapSeverity::Major));
            state.record_verification(result);
        }

        let context = state.build_iteration_context();
        assert!(context.drift_detected);
        assert!(!context.recurring_gap_descriptions.is_empty());

        let formatted = context.format_for_prompt();
        assert!(formatted.contains("WARNING: Semantic drift detected"));
        assert!(formatted.contains("### Recurring Gaps"));
    }

    fn make_gap_with_embedding(embedding: Vec<f32>) -> IntentGap {
        let mut gap = IntentGap::new("test gap", GapSeverity::Moderate);
        gap.embedding = Some(embedding);
        gap
    }

    #[test]
    fn test_merge_identical_vectors_yields_same_vector() {
        let initial = vec![1.0_f32, 0.0, 0.0];
        let mut fp = EmbeddedGapFingerprint::from_gap(
            &make_gap_with_embedding(initial.clone()),
            0,
        );
        fp.merge(&make_gap_with_embedding(initial.clone()));
        let merged = fp.embedding.as_ref().expect("embedding present");
        for (a, b) in merged.iter().zip(initial.iter()) {
            assert!(
                (a - b).abs() < 1e-6,
                "expected identical vectors after merge, got {:?}",
                merged
            );
        }
        assert_eq!(fp.occurrence_count, 2);
    }

    #[test]
    fn test_merge_divergent_vectors_shifts_proportionally() {
        let mut fp = EmbeddedGapFingerprint::from_gap(
            &make_gap_with_embedding(vec![1.0_f32, 0.0]),
            0,
        );
        fp.merge(&make_gap_with_embedding(vec![0.0_f32, 1.0]));
        let merged = fp.embedding.as_ref().expect("embedding present");
        let expected = 1.0_f32 / 2.0_f32.sqrt();
        assert!((merged[0] - expected).abs() < 1e-5, "got {:?}", merged);
        assert!((merged[1] - expected).abs() < 1e-5, "got {:?}", merged);
        let norm: f32 = merged.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 1e-5);
        assert_eq!(fp.occurrence_count, 2);
    }

    #[test]
    fn test_merge_preserves_unit_normalization() {
        let mut fp = EmbeddedGapFingerprint::from_gap(
            &make_gap_with_embedding(vec![1.0_f32, 0.0, 0.0]),
            0,
        );
        let inv_sqrt2 = 1.0_f32 / 2.0_f32.sqrt();
        fp.merge(&make_gap_with_embedding(vec![0.0_f32, 1.0, 0.0]));
        fp.merge(&make_gap_with_embedding(vec![inv_sqrt2, inv_sqrt2, 0.0]));
        fp.merge(&make_gap_with_embedding(vec![0.0_f32, 0.0, 1.0]));
        let merged = fp.embedding.as_ref().expect("embedding present");
        let norm: f32 = merged.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!(
            (norm - 1.0).abs() < 1e-3,
            "expected ~unit norm after 3 merges, got norm={} vec={:?}",
            norm,
            merged
        );
        assert_eq!(fp.occurrence_count, 4);
    }

    #[test]
    fn test_merge_dimension_mismatch_is_skipped() {
        let initial = vec![1.0_f32, 0.0];
        let mut fp = EmbeddedGapFingerprint::from_gap(
            &make_gap_with_embedding(initial.clone()),
            0,
        );
        fp.merge(&make_gap_with_embedding(vec![0.0_f32, 1.0, 0.0]));
        let merged = fp.embedding.as_ref().expect("embedding present");
        assert_eq!(
            merged, &initial,
            "embedding should be unchanged on dim mismatch"
        );
        assert_eq!(fp.occurrence_count, 2);
    }
}
