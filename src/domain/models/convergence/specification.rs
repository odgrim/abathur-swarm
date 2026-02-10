//! Specification evolution types (Spec 1.6).
//!
//! The specification co-evolves with the trajectory. Amending the specification
//! is the *primary mechanism* for escaping limit cycles that stem from
//! specification ambiguity -- a fixed prompt produces fixed periodic states.
//!
//! When an amendment is added, the `effective` snapshot is recomputed by merging
//! the original specification with all amendments. The effective specification is
//! what gets included in the LLM prompt for subsequent iterations and in
//! CarryForward for fresh starts.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ---------------------------------------------------------------------------
// SpecificationSnapshot
// ---------------------------------------------------------------------------

/// A point-in-time view of a specification's content and extracted facets.
///
/// Snapshots are immutable once created. The `effective` snapshot inside
/// [`SpecificationEvolution`] is recomputed whenever an amendment is added,
/// producing a new `SpecificationSnapshot` rather than mutating the original.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SpecificationSnapshot {
    /// The full textual content of the specification.
    pub content: String,
    /// Key requirements extracted from the specification.
    pub key_requirements: Vec<String>,
    /// Criteria that define success for this specification.
    pub success_criteria: Vec<String>,
    /// Constraints that bound acceptable solutions.
    pub constraints: Vec<String>,
    /// Known anti-patterns that must be avoided.
    pub anti_patterns: Vec<String>,
}

impl SpecificationSnapshot {
    /// Create a new snapshot from raw content.
    ///
    /// All facet vectors (`key_requirements`, `success_criteria`, `constraints`,
    /// `anti_patterns`) start empty and are populated later -- either by LLM
    /// extraction during the PREPARE phase or by amendment merging.
    pub fn new(content: String) -> Self {
        Self {
            content,
            key_requirements: Vec::new(),
            success_criteria: Vec::new(),
            constraints: Vec::new(),
            anti_patterns: Vec::new(),
        }
    }

    /// Produce a new snapshot that incorporates the given amendment.
    ///
    /// The merge appends the amendment description to the content and, depending
    /// on the amendment source, updates the appropriate facet vectors. The
    /// original snapshot is not modified.
    pub fn merge_amendment(&self, amendment: &SpecificationAmendment) -> Self {
        let mut merged = self.clone();

        // Append the amendment text to the specification content so that
        // subsequent LLM prompts include the full, evolved specification.
        merged.content = format!(
            "{}\n\n[Amendment ({:?})]: {}",
            merged.content, amendment.source, amendment.description
        );

        // Route the amendment into the appropriate facet based on its source.
        match amendment.source {
            AmendmentSource::UserHint => {
                // User hints may refine any facet; treat as a key requirement
                // since the user explicitly called it out.
                merged.key_requirements.push(amendment.description.clone());
            }
            AmendmentSource::ImplicitRequirementDiscovered => {
                // A test failure revealed a requirement not stated originally.
                merged.key_requirements.push(amendment.description.clone());
            }
            AmendmentSource::OverseerDiscovery => {
                // Overseer signals exposed a constraint (e.g. security, perf).
                merged.constraints.push(amendment.description.clone());
            }
            AmendmentSource::ArchitectAmendment => {
                // Architect review may touch requirements or constraints.
                merged.key_requirements.push(amendment.description.clone());
            }
            AmendmentSource::TestDisambiguation => {
                // Contradictory tests reveal success-criteria ambiguity.
                merged.success_criteria.push(amendment.description.clone());
            }
            AmendmentSource::SubmissionConstraint => {
                // Constraints and anti-patterns from the task submission.
                merged.constraints.push(amendment.description.clone());
            }
        }

        merged
    }
}

// ---------------------------------------------------------------------------
// SpecificationEvolution
// ---------------------------------------------------------------------------

/// Tracks how a specification evolves over the lifetime of a trajectory.
///
/// The `original` snapshot is captured at trajectory creation time. As
/// amendments are applied (user hints, overseer discoveries, architect
/// reviews, etc.) they are appended to `amendments` and the `effective`
/// snapshot is recomputed so that subsequent iterations and CarryForward
/// always see the most up-to-date specification.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpecificationEvolution {
    /// The specification as it was when the trajectory started.
    pub original: SpecificationSnapshot,
    /// Ordered list of amendments applied so far.
    pub amendments: Vec<SpecificationAmendment>,
    /// The result of merging the original with all amendments.
    /// Recomputed on every call to [`add_amendment`](Self::add_amendment).
    pub effective: SpecificationSnapshot,
}

impl SpecificationEvolution {
    /// Start tracking evolution of the given specification.
    ///
    /// The effective snapshot initially equals the original.
    pub fn new(original: SpecificationSnapshot) -> Self {
        let effective = original.clone();
        Self {
            original,
            amendments: Vec::new(),
            effective,
        }
    }

    /// Apply a new amendment and recompute the effective specification.
    ///
    /// Amendments are ordered; the effective snapshot is rebuilt from the
    /// original by folding over all amendments in sequence.
    pub fn add_amendment(&mut self, amendment: SpecificationAmendment) {
        self.amendments.push(amendment);
        self.recompute_effective();
    }

    /// Rebuild the effective specification from the original and all
    /// accumulated amendments.
    ///
    /// This is called automatically by [`add_amendment`](Self::add_amendment)
    /// but is also exposed publicly so that callers can force a recompute
    /// after bulk-loading amendments.
    pub fn recompute_effective(&mut self) {
        self.effective = self
            .amendments
            .iter()
            .fold(self.original.clone(), |snapshot, amendment| {
                snapshot.merge_amendment(amendment)
            });
    }
}

// ---------------------------------------------------------------------------
// SpecificationAmendment
// ---------------------------------------------------------------------------

/// A single amendment to a specification.
///
/// Amendments are the primary mechanism for escaping limit cycles caused by
/// specification ambiguity. Each amendment records *what* changed, *why* it
/// changed, and optionally *which observation* triggered the change.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpecificationAmendment {
    /// Unique identifier for this amendment.
    pub id: Uuid,
    /// Where this amendment originated.
    pub source: AmendmentSource,
    /// Human-readable description of the amendment.
    pub description: String,
    /// Why this amendment was necessary.
    pub rationale: String,
    /// The observation that triggered this amendment, if any.
    pub observation_trigger: Option<Uuid>,
    /// When this amendment was created.
    pub timestamp: DateTime<Utc>,
}

impl SpecificationAmendment {
    /// Create a new amendment with the given source, description, and rationale.
    pub fn new(
        source: AmendmentSource,
        description: impl Into<String>,
        rationale: impl Into<String>,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            source,
            description: description.into(),
            rationale: rationale.into(),
            observation_trigger: None,
            timestamp: Utc::now(),
        }
    }
}

// ---------------------------------------------------------------------------
// AmendmentSource
// ---------------------------------------------------------------------------

/// Identifies where a [`SpecificationAmendment`] originated.
///
/// Each variant corresponds to a distinct feedback channel in the convergence
/// system (see spec 1.6):
///
/// - **`UserHint`** -- The user provided a hint or explicit feedback. Hints
///   always carry forward across fresh starts and are prepended to the agent's
///   prompt as high-priority context.
///
/// - **`ImplicitRequirementDiscovered`** -- A test failure revealed a
///   requirement not stated in the original specification. Detected during LLM
///   verification of test results.
///
/// - **`OverseerDiscovery`** -- An overseer measurement exposed a constraint
///   that was absent from the specification (e.g. a security scan finds the
///   spec did not mention authentication requirements).
///
/// - **`ArchitectAmendment`** -- The `ArchitectReview` strategy returned with
///   a modified specification after identifying a gap during review.
///
/// - **`TestDisambiguation`** -- During the PREPARE phase, contradictory or
///   ambiguous generated tests revealed that the specification itself is
///   ambiguous and needs clarification.
///
/// - **`SubmissionConstraint`** -- At trajectory initialization, constraints
///   and anti-patterns from the task submission are folded into the
///   specification as amendments.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AmendmentSource {
    /// User hints or explicit feedback.
    UserHint,
    /// A test failure reveals a missing requirement.
    ImplicitRequirementDiscovered,
    /// Overseer signals expose a constraint not in the spec.
    OverseerDiscovery,
    /// Architect review identifies a spec gap.
    ArchitectAmendment,
    /// Generated tests reveal ambiguity.
    TestDisambiguation,
    /// Constraints and anti-patterns from the task submission.
    SubmissionConstraint,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_snapshot_new_starts_empty() {
        let snap = SpecificationSnapshot::new("build a CLI".into());
        assert_eq!(snap.content, "build a CLI");
        assert!(snap.key_requirements.is_empty());
        assert!(snap.success_criteria.is_empty());
        assert!(snap.constraints.is_empty());
        assert!(snap.anti_patterns.is_empty());
    }

    #[test]
    fn test_merge_amendment_appends_content() {
        let snap = SpecificationSnapshot::new("original spec".into());
        let amendment = SpecificationAmendment::new(
            AmendmentSource::UserHint,
            "must support JSON output",
            "user requested JSON support",
        );

        let merged = snap.merge_amendment(&amendment);
        assert!(merged.content.contains("original spec"));
        assert!(merged.content.contains("must support JSON output"));
        assert!(merged.content.contains("[Amendment"));
    }

    #[test]
    fn test_merge_routes_user_hint_to_key_requirements() {
        let snap = SpecificationSnapshot::new("spec".into());
        let amendment = SpecificationAmendment::new(
            AmendmentSource::UserHint,
            "use async/await",
            "user prefers async",
        );

        let merged = snap.merge_amendment(&amendment);
        assert_eq!(merged.key_requirements, vec!["use async/await"]);
    }

    #[test]
    fn test_merge_routes_overseer_to_constraints() {
        let snap = SpecificationSnapshot::new("spec".into());
        let amendment = SpecificationAmendment::new(
            AmendmentSource::OverseerDiscovery,
            "requires authentication",
            "security scan found unauthenticated endpoints",
        );

        let merged = snap.merge_amendment(&amendment);
        assert_eq!(merged.constraints, vec!["requires authentication"]);
    }

    #[test]
    fn test_merge_routes_test_disambiguation_to_success_criteria() {
        let snap = SpecificationSnapshot::new("spec".into());
        let amendment = SpecificationAmendment::new(
            AmendmentSource::TestDisambiguation,
            "empty input returns 400 not 200",
            "contradictory tests clarified",
        );

        let merged = snap.merge_amendment(&amendment);
        assert_eq!(
            merged.success_criteria,
            vec!["empty input returns 400 not 200"]
        );
    }

    #[test]
    fn test_merge_routes_submission_constraint_to_constraints() {
        let snap = SpecificationSnapshot::new("spec".into());
        let amendment = SpecificationAmendment::new(
            AmendmentSource::SubmissionConstraint,
            "no unsafe code",
            "submission constraint from task",
        );

        let merged = snap.merge_amendment(&amendment);
        assert_eq!(merged.constraints, vec!["no unsafe code"]);
    }

    #[test]
    fn test_evolution_new_effective_equals_original() {
        let original = SpecificationSnapshot::new("original".into());
        let evo = SpecificationEvolution::new(original.clone());
        assert_eq!(evo.effective.content, "original");
        assert!(evo.amendments.is_empty());
    }

    #[test]
    fn test_evolution_add_amendment_updates_effective() {
        let original = SpecificationSnapshot::new("original".into());
        let mut evo = SpecificationEvolution::new(original);

        let amendment = SpecificationAmendment::new(
            AmendmentSource::ImplicitRequirementDiscovered,
            "handle timeout errors",
            "test revealed missing timeout handling",
        );
        evo.add_amendment(amendment);

        assert_eq!(evo.amendments.len(), 1);
        assert!(evo.effective.content.contains("handle timeout errors"));
        assert!(evo
            .effective
            .key_requirements
            .contains(&"handle timeout errors".to_string()));
    }

    #[test]
    fn test_evolution_multiple_amendments_compose() {
        let original = SpecificationSnapshot::new("base spec".into());
        let mut evo = SpecificationEvolution::new(original);

        evo.add_amendment(SpecificationAmendment::new(
            AmendmentSource::UserHint,
            "support pagination",
            "user requested",
        ));
        evo.add_amendment(SpecificationAmendment::new(
            AmendmentSource::OverseerDiscovery,
            "rate limit to 100 req/s",
            "load test overseer discovered",
        ));
        evo.add_amendment(SpecificationAmendment::new(
            AmendmentSource::TestDisambiguation,
            "404 for missing resources",
            "tests disagreed on missing-resource behavior",
        ));

        assert_eq!(evo.amendments.len(), 3);
        assert_eq!(evo.effective.key_requirements, vec!["support pagination"]);
        assert_eq!(
            evo.effective.constraints,
            vec!["rate limit to 100 req/s"]
        );
        assert_eq!(
            evo.effective.success_criteria,
            vec!["404 for missing resources"]
        );
        // Original is unchanged.
        assert_eq!(evo.original.content, "base spec");
        assert!(evo.original.key_requirements.is_empty());
    }

    #[test]
    fn test_evolution_recompute_effective_is_idempotent() {
        let original = SpecificationSnapshot::new("spec".into());
        let mut evo = SpecificationEvolution::new(original);

        evo.add_amendment(SpecificationAmendment::new(
            AmendmentSource::UserHint,
            "add logging",
            "user asked",
        ));

        let first = evo.effective.clone();
        evo.recompute_effective();
        assert_eq!(evo.effective.content, first.content);
        assert_eq!(evo.effective.key_requirements, first.key_requirements);
    }

    #[test]
    fn test_amendment_new_sets_fields() {
        let amendment = SpecificationAmendment::new(
            AmendmentSource::ArchitectAmendment,
            "refactor to use repository pattern",
            "architect identified coupling",
        );

        assert_eq!(amendment.source, AmendmentSource::ArchitectAmendment);
        assert_eq!(amendment.description, "refactor to use repository pattern");
        assert_eq!(amendment.rationale, "architect identified coupling");
        assert!(amendment.observation_trigger.is_none());
    }

    #[test]
    fn test_amendment_source_serde_roundtrip() {
        let sources = vec![
            AmendmentSource::UserHint,
            AmendmentSource::ImplicitRequirementDiscovered,
            AmendmentSource::OverseerDiscovery,
            AmendmentSource::ArchitectAmendment,
            AmendmentSource::TestDisambiguation,
            AmendmentSource::SubmissionConstraint,
        ];

        for source in sources {
            let json = serde_json::to_string(&source).unwrap();
            let deserialized: AmendmentSource = serde_json::from_str(&json).unwrap();
            assert_eq!(source, deserialized);
        }
    }

    #[test]
    fn test_amendment_source_snake_case_serialization() {
        assert_eq!(
            serde_json::to_string(&AmendmentSource::UserHint).unwrap(),
            "\"user_hint\""
        );
        assert_eq!(
            serde_json::to_string(&AmendmentSource::ImplicitRequirementDiscovered).unwrap(),
            "\"implicit_requirement_discovered\""
        );
        assert_eq!(
            serde_json::to_string(&AmendmentSource::OverseerDiscovery).unwrap(),
            "\"overseer_discovery\""
        );
        assert_eq!(
            serde_json::to_string(&AmendmentSource::ArchitectAmendment).unwrap(),
            "\"architect_amendment\""
        );
        assert_eq!(
            serde_json::to_string(&AmendmentSource::TestDisambiguation).unwrap(),
            "\"test_disambiguation\""
        );
        assert_eq!(
            serde_json::to_string(&AmendmentSource::SubmissionConstraint).unwrap(),
            "\"submission_constraint\""
        );
    }
}
