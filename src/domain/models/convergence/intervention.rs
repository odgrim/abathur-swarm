//! User interaction types for the convergence system (spec 7.1, 7.3).
//!
//! This module defines the types through which users interact with the
//! convergence engine. The engine works without the user knowing any of it
//! exists -- a task submission is just a sentence. Everything else (basin
//! estimation, budget allocation, strategy selection) is inferred.
//!
//! The surface area is progressive disclosure: users can enrich their
//! submissions to widen the attractor basin, but nothing is required.
//!
//! ## Key types
//!
//! - [`TaskSubmission`] -- the input a user provides to start a convergence
//!   trajectory (spec 7.1).
//! - [`InterventionPoint`] -- natural convergence boundaries where the engine
//!   pauses for user input (spec 7.3).
//! - [`ConvergenceMode`] -- sequential vs. parallel trajectory sampling
//!   (spec 6.6).

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::*;
use crate::domain::models::task::Complexity;

// ---------------------------------------------------------------------------
// TaskSubmission (spec 7.1)
// ---------------------------------------------------------------------------

/// A task submission that enters the convergence engine.
///
/// At minimum, only a `description` is required. Everything else is either
/// inferred by the system (complexity, discovered infrastructure) or
/// optionally provided by the user to improve convergence (priority hints,
/// constraints, references, anti-patterns).
///
/// # Examples
///
/// ```ignore
/// // Minimal submission -- everything else is inferred.
/// let submission = TaskSubmission::new("Implement user login".to_string());
///
/// // Enriched submission with constraints and references.
/// let submission = TaskSubmission::new("Implement user login".to_string())
///     .with_constraint("Must use bcrypt for password hashing")
///     .with_anti_pattern("Do not store passwords in plaintext");
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskSubmission {
    /// The natural-language description of the task to accomplish.
    pub description: String,
    /// Optional goal this task contributes to.
    pub goal_id: Option<Uuid>,
    /// Complexity inferred from the description and discovered infrastructure.
    pub inferred_complexity: Complexity,
    /// Infrastructure discovered from the project (test frameworks, build
    /// tools, linters, type checkers).
    pub discovered_infrastructure: DiscoveredInfrastructure,
    /// Optional priority hint that adjusts budget and policy.
    pub priority_hint: Option<PriorityHint>,
    /// User-provided invariants: "must X" constraints.
    pub constraints: Vec<String>,
    /// Code, test, or documentation file references the user wants the
    /// engine to consider.
    pub references: Vec<Reference>,
    /// Anti-patterns the user wants to avoid: "not Y".
    pub anti_patterns: Vec<String>,
    /// If set, overrides the default convergence mode to use parallel
    /// trajectory sampling with this many initial samples.
    pub parallel_samples: Option<u32>,
}

impl TaskSubmission {
    /// Create a new task submission with the given description.
    ///
    /// Defaults:
    /// - `inferred_complexity` is set to [`Complexity::Moderate`].
    /// - All other fields are empty or `None`.
    pub fn new(description: String) -> Self {
        Self {
            description,
            goal_id: None,
            inferred_complexity: Complexity::Moderate,
            discovered_infrastructure: DiscoveredInfrastructure::default(),
            priority_hint: None,
            constraints: Vec::new(),
            references: Vec::new(),
            anti_patterns: Vec::new(),
            parallel_samples: None,
        }
    }

    /// Add a constraint ("must X" invariant) to this submission.
    ///
    /// Constraints are folded into the specification as amendments during
    /// trajectory initialization and widen the attractor basin by making
    /// acceptance criteria explicit.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let submission = TaskSubmission::new("Implement user login".to_string())
    ///     .with_constraint("Must use bcrypt for password hashing".to_string());
    /// ```
    pub fn with_constraint(mut self, constraint: String) -> Self {
        self.constraints.push(constraint);
        self
    }

    /// Add an anti-pattern ("not Y") to this submission.
    ///
    /// Anti-patterns are injected into the agent prompt and used by overseer
    /// checks to penalize implementations that exhibit known bad patterns.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let submission = TaskSubmission::new("Implement user login".to_string())
    ///     .with_anti_pattern("Do not store passwords in plaintext".to_string());
    /// ```
    pub fn with_anti_pattern(mut self, pattern: String) -> Self {
        self.anti_patterns.push(pattern);
        self
    }

    /// Add a file or resource reference for the engine to consider.
    ///
    /// References are routed into the convergence infrastructure during
    /// the PREPARE phase: test files become acceptance tests, examples
    /// become prompt enrichment, and other types become context files.
    pub fn with_reference(mut self, reference: Reference) -> Self {
        self.references.push(reference);
        self
    }

    /// Set the priority hint for this submission.
    ///
    /// The priority hint adjusts both the convergence budget and policy
    /// during the SETUP phase, controlling the tradeoff between speed,
    /// cost, and thoroughness.
    pub fn with_priority_hint(mut self, hint: PriorityHint) -> Self {
        self.priority_hint = Some(hint);
        self
    }
}

// ---------------------------------------------------------------------------
// DiscoveredInfrastructure
// ---------------------------------------------------------------------------

/// Infrastructure discovered from the project environment.
///
/// This is populated upstream (by repository analysis, file scanning, etc.)
/// and provided as input to the convergence engine. It tells the engine what
/// external verification tools are available and what existing test/example
/// assets can be leveraged.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DiscoveredInfrastructure {
    /// Paths to existing acceptance or integration tests.
    pub acceptance_tests: Vec<String>,
    /// Paths to example files or usage demonstrations.
    pub examples: Vec<String>,
    /// Known project invariants discovered from existing code.
    pub invariants: Vec<String>,
    /// Known anti-examples discovered from existing code.
    pub anti_examples: Vec<String>,
    /// Paths to context files relevant to the task.
    pub context_files: Vec<String>,
    /// Detected test framework (e.g. "jest", "pytest", "cargo test").
    pub test_framework: Option<String>,
    /// Detected build tool (e.g. "cargo", "npm", "gradle").
    pub build_tool: Option<String>,
    /// Detected type checker (e.g. "tsc", "mypy", "rustc").
    pub type_checker: Option<String>,
    /// Detected linter (e.g. "clippy", "eslint", "ruff").
    pub linter: Option<String>,
}

// ---------------------------------------------------------------------------
// Reference
// ---------------------------------------------------------------------------

/// A user-provided reference to a file or resource that the convergence
/// engine should consider during preparation and iteration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Reference {
    /// Path to the referenced file or resource.
    pub path: String,
    /// Optional description of why this reference is relevant.
    pub description: Option<String>,
    /// The type of this reference (code, test, documentation, etc.).
    pub reference_type: ReferenceType,
}

// ---------------------------------------------------------------------------
// ReferenceType
// ---------------------------------------------------------------------------

/// Classifies the type of a user-provided [`Reference`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReferenceType {
    /// A source code file to use as context or a target for modification.
    CodeFile,
    /// A test file containing relevant test cases.
    TestFile,
    /// A documentation file with specification or design details.
    Documentation,
    /// An example file demonstrating expected behavior or patterns.
    Example,
    /// A configuration file (e.g. build config, CI config).
    Config,
}

// ---------------------------------------------------------------------------
// ConvergenceInfrastructure
// ---------------------------------------------------------------------------

/// The convergence-specific infrastructure assembled during the PREPARE phase.
///
/// Built from [`DiscoveredInfrastructure`] (project-level assets) combined
/// with user-provided [`Reference`]s, constraints, and anti-patterns. This
/// is the input to overseer configuration and acceptance test generation.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ConvergenceInfrastructure {
    /// Acceptance tests (discovered + generated) used by the
    /// AcceptanceTestOverseer.
    pub acceptance_tests: Vec<String>,
    /// Examples and demonstrations used for test generation and prompt
    /// enrichment.
    pub examples: Vec<String>,
    /// Invariants that must hold across all iterations.
    pub invariants: Vec<String>,
    /// Anti-patterns that must be avoided in all produced artifacts.
    pub anti_patterns: Vec<String>,
    /// Context files included in the agent prompt.
    pub context_files: Vec<String>,
}

impl ConvergenceInfrastructure {
    /// Create convergence infrastructure from discovered project
    /// infrastructure.
    ///
    /// Copies over acceptance tests, examples, invariants, and context
    /// files. Anti-examples from discovery are mapped to anti-patterns.
    pub fn from_discovered(discovered: &DiscoveredInfrastructure) -> Self {
        Self {
            acceptance_tests: discovered.acceptance_tests.clone(),
            examples: discovered.examples.clone(),
            invariants: discovered.invariants.clone(),
            anti_patterns: discovered.anti_examples.clone(),
            context_files: discovered.context_files.clone(),
        }
    }

    /// Merge user-provided references into the infrastructure.
    ///
    /// References are routed to the appropriate collection based on their
    /// [`ReferenceType`]:
    /// - [`ReferenceType::TestFile`] references are added to
    ///   `acceptance_tests`.
    /// - [`ReferenceType::Example`] references are added to `examples`.
    /// - All other types are added to `context_files`.
    pub fn merge_user_references(&mut self, references: &[Reference]) {
        for reference in references {
            match reference.reference_type {
                ReferenceType::TestFile => {
                    self.acceptance_tests.push(reference.path.clone());
                }
                ReferenceType::Example => {
                    self.examples.push(reference.path.clone());
                }
                ReferenceType::CodeFile
                | ReferenceType::Documentation
                | ReferenceType::Config => {
                    self.context_files.push(reference.path.clone());
                }
            }
        }
    }

    /// Add user-provided constraints as invariants.
    pub fn add_invariants(&mut self, constraints: &[String]) {
        self.invariants.extend(constraints.iter().cloned());
    }

    /// Add user-provided anti-patterns.
    pub fn add_anti_patterns(&mut self, anti_patterns: &[String]) {
        self.anti_patterns.extend(anti_patterns.iter().cloned());
    }
}

// ---------------------------------------------------------------------------
// InterventionPoint (spec 7.3)
// ---------------------------------------------------------------------------

/// A natural convergence boundary where the engine pauses for user input.
///
/// The convergence engine emits an intervention event and waits for a
/// response (approval, rejection, or user-provided context). Priority hints
/// control which interventions pause vs. auto-decide:
///
/// - **Fast**: Only `PartialResult` triggers. Everything else is auto-decided.
/// - **Thorough**: All intervention points pause for user input.
/// - **No hint**: `AttractorTransition` and `BudgetExtension` notify;
///   `StrategyEscalation`, `AmbiguityDetected`, and `HumanEscalation` pause.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InterventionPoint {
    /// The trajectory's attractor classification has changed (e.g. from
    /// Indeterminate to LimitCycle). The user can intervene to steer
    /// the trajectory.
    AttractorTransition {
        /// The previous attractor type.
        from: AttractorType,
        /// The new attractor type.
        to: AttractorType,
    },

    /// The engine wants to escalate to a more expensive or disruptive
    /// strategy. The user can approve, reject, or provide guidance.
    StrategyEscalation {
        /// The strategy being proposed.
        proposed_strategy: String,
        /// Why this escalation is being proposed.
        reason: String,
    },

    /// The convergence budget is running low and the engine is requesting
    /// an extension to continue iterating.
    BudgetExtension {
        /// Fraction of the original budget remaining (0.0 to 1.0).
        current_remaining_fraction: f64,
        /// Additional tokens requested.
        proposed_additional_tokens: u64,
        /// Additional iterations requested.
        proposed_additional_iterations: u32,
    },

    /// The engine detected contradictions or ambiguity in the specification
    /// or generated tests. The user should clarify.
    AmbiguityDetected {
        /// Descriptions of the contradictions found.
        contradictions: Vec<String>,
    },

    /// The engine has a partial result that meets some but not all criteria.
    /// The user can accept the partial result or request continued iteration.
    PartialResult {
        /// Current convergence level (0.0 to 1.0).
        convergence_level: f64,
        /// Descriptions of remaining gaps preventing full convergence.
        remaining_gaps: Vec<String>,
    },

    /// Terminal intervention: all escape strategies for a limit cycle are
    /// exhausted and the budget allows no further exploration. The engine
    /// pauses with full convergence context and waits for human guidance
    /// before either continuing or accepting the Trapped outcome.
    HumanEscalation {
        /// Why the engine is escalating to a human.
        reason: String,
        /// Summary of the convergence context to help the human decide.
        context_summary: String,
    },
}

// ---------------------------------------------------------------------------
// BudgetExtension
// ---------------------------------------------------------------------------

/// A request to extend the convergence budget.
///
/// Used when the engine determines that additional resources could allow
/// the trajectory to converge, but the current budget is insufficient.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BudgetExtension {
    /// Additional tokens to add to the budget.
    pub additional_tokens: u64,
    /// Additional iterations to add to the budget.
    pub additional_iterations: u32,
}

// ---------------------------------------------------------------------------
// ConvergenceMode (spec 6.6)
// ---------------------------------------------------------------------------

/// The convergence mode determines whether the engine iterates sequentially
/// or runs parallel trajectory samples.
///
/// Sequential mode is the default and works well for wide or moderate basins.
/// Parallel mode spawns multiple independent trajectories and selects the
/// best, which can outperform sequential iteration for narrow basins.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConvergenceMode {
    /// Standard sequential iteration through the convergence loop.
    Sequential,
    /// Parallel trajectory sampling with the given number of initial samples.
    Parallel {
        /// Number of independent trajectories to spawn initially.
        initial_samples: u32,
    },
}

// ---------------------------------------------------------------------------
// select_convergence_mode (spec 6.6)
// ---------------------------------------------------------------------------

/// Select the convergence mode based on basin width, policy, and user
/// override.
///
/// If the user explicitly requested parallel samples, that takes precedence.
/// Otherwise, the mode is selected based on the combination of basin width
/// classification and priority hint:
///
/// | Basin | Priority | Mode |
/// |-------|----------|------|
/// | Wide | any | Sequential |
/// | Narrow | Thorough | Parallel(3) |
/// | Narrow | Fast | Parallel(2) |
/// | Narrow | Cheap | Sequential |
/// | Narrow | None | Parallel(2) |
/// | Moderate | any | Sequential |
pub fn select_convergence_mode(
    basin: &BasinWidth,
    policy: &ConvergencePolicy,
    parallel_samples: Option<u32>,
) -> ConvergenceMode {
    // User-requested parallel samples always win.
    if let Some(samples) = parallel_samples {
        return ConvergenceMode::Parallel {
            initial_samples: samples,
        };
    }

    match (&basin.classification, &policy.priority_hint) {
        (BasinClassification::Wide, _) => ConvergenceMode::Sequential,
        (BasinClassification::Narrow, Some(PriorityHint::Thorough)) => {
            ConvergenceMode::Parallel {
                initial_samples: 3,
            }
        }
        (BasinClassification::Narrow, Some(PriorityHint::Fast)) => ConvergenceMode::Parallel {
            initial_samples: 2,
        },
        (BasinClassification::Narrow, Some(PriorityHint::Cheap)) => ConvergenceMode::Sequential,
        (BasinClassification::Narrow, None) => ConvergenceMode::Parallel {
            initial_samples: 2,
        },
        (BasinClassification::Moderate, _) => ConvergenceMode::Sequential,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // -----------------------------------------------------------------------
    // TaskSubmission
    // -----------------------------------------------------------------------

    #[test]
    fn test_task_submission_new_defaults() {
        let submission = TaskSubmission::new("Implement login".to_string());

        assert_eq!(submission.description, "Implement login");
        assert!(submission.goal_id.is_none());
        assert_eq!(submission.inferred_complexity, Complexity::Moderate);
        assert!(submission.priority_hint.is_none());
        assert!(submission.constraints.is_empty());
        assert!(submission.references.is_empty());
        assert!(submission.anti_patterns.is_empty());
        assert!(submission.parallel_samples.is_none());
    }

    #[test]
    fn test_task_submission_serde_roundtrip() {
        let submission = TaskSubmission::new("Build a REST API".to_string());
        let json = serde_json::to_string(&submission).unwrap();
        let deserialized: TaskSubmission = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.description, "Build a REST API");
        assert_eq!(deserialized.inferred_complexity, Complexity::Moderate);
    }

    // -----------------------------------------------------------------------
    // DiscoveredInfrastructure
    // -----------------------------------------------------------------------

    #[test]
    fn test_discovered_infrastructure_default() {
        let infra = DiscoveredInfrastructure::default();

        assert!(infra.acceptance_tests.is_empty());
        assert!(infra.examples.is_empty());
        assert!(infra.invariants.is_empty());
        assert!(infra.anti_examples.is_empty());
        assert!(infra.context_files.is_empty());
        assert!(infra.test_framework.is_none());
        assert!(infra.build_tool.is_none());
        assert!(infra.type_checker.is_none());
        assert!(infra.linter.is_none());
    }

    // -----------------------------------------------------------------------
    // ReferenceType
    // -----------------------------------------------------------------------

    #[test]
    fn test_reference_type_serde_roundtrip() {
        let types = vec![
            ReferenceType::CodeFile,
            ReferenceType::TestFile,
            ReferenceType::Documentation,
            ReferenceType::Example,
            ReferenceType::Config,
        ];

        for rt in types {
            let json = serde_json::to_string(&rt).unwrap();
            let deserialized: ReferenceType = serde_json::from_str(&json).unwrap();
            assert_eq!(rt, deserialized);
        }
    }

    #[test]
    fn test_reference_type_snake_case_serialization() {
        assert_eq!(
            serde_json::to_string(&ReferenceType::CodeFile).unwrap(),
            "\"code_file\""
        );
        assert_eq!(
            serde_json::to_string(&ReferenceType::TestFile).unwrap(),
            "\"test_file\""
        );
        assert_eq!(
            serde_json::to_string(&ReferenceType::Documentation).unwrap(),
            "\"documentation\""
        );
        assert_eq!(
            serde_json::to_string(&ReferenceType::Example).unwrap(),
            "\"example\""
        );
        assert_eq!(
            serde_json::to_string(&ReferenceType::Config).unwrap(),
            "\"config\""
        );
    }

    // -----------------------------------------------------------------------
    // ConvergenceInfrastructure
    // -----------------------------------------------------------------------

    #[test]
    fn test_convergence_infra_from_discovered() {
        let discovered = DiscoveredInfrastructure {
            acceptance_tests: vec!["tests/login.rs".to_string()],
            examples: vec!["examples/auth.rs".to_string()],
            invariants: vec!["passwords must be hashed".to_string()],
            anti_examples: vec!["plaintext passwords".to_string()],
            context_files: vec!["src/auth.rs".to_string()],
            test_framework: Some("cargo test".to_string()),
            build_tool: Some("cargo".to_string()),
            type_checker: None,
            linter: Some("clippy".to_string()),
        };

        let infra = ConvergenceInfrastructure::from_discovered(&discovered);

        assert_eq!(infra.acceptance_tests, vec!["tests/login.rs"]);
        assert_eq!(infra.examples, vec!["examples/auth.rs"]);
        assert_eq!(infra.invariants, vec!["passwords must be hashed"]);
        assert_eq!(infra.anti_patterns, vec!["plaintext passwords"]);
        assert_eq!(infra.context_files, vec!["src/auth.rs"]);
    }

    #[test]
    fn test_convergence_infra_merge_user_references() {
        let mut infra = ConvergenceInfrastructure::default();

        let references = vec![
            Reference {
                path: "tests/api_test.rs".to_string(),
                description: Some("API integration tests".to_string()),
                reference_type: ReferenceType::TestFile,
            },
            Reference {
                path: "examples/usage.rs".to_string(),
                description: None,
                reference_type: ReferenceType::Example,
            },
            Reference {
                path: "src/lib.rs".to_string(),
                description: Some("Main library".to_string()),
                reference_type: ReferenceType::CodeFile,
            },
            Reference {
                path: "docs/design.md".to_string(),
                description: None,
                reference_type: ReferenceType::Documentation,
            },
            Reference {
                path: "config.toml".to_string(),
                description: None,
                reference_type: ReferenceType::Config,
            },
        ];

        infra.merge_user_references(&references);

        assert_eq!(infra.acceptance_tests, vec!["tests/api_test.rs"]);
        assert_eq!(infra.examples, vec!["examples/usage.rs"]);
        assert_eq!(
            infra.context_files,
            vec!["src/lib.rs", "docs/design.md", "config.toml"]
        );
    }

    #[test]
    fn test_convergence_infra_add_invariants() {
        let mut infra = ConvergenceInfrastructure::default();
        infra.add_invariants(&[
            "must use TLS".to_string(),
            "must validate input".to_string(),
        ]);
        assert_eq!(infra.invariants.len(), 2);
        assert_eq!(infra.invariants[0], "must use TLS");
        assert_eq!(infra.invariants[1], "must validate input");
    }

    #[test]
    fn test_convergence_infra_add_anti_patterns() {
        let mut infra = ConvergenceInfrastructure::default();
        infra.add_anti_patterns(&[
            "no unwrap in production code".to_string(),
            "no SQL injection".to_string(),
        ]);
        assert_eq!(infra.anti_patterns.len(), 2);
        assert_eq!(infra.anti_patterns[0], "no unwrap in production code");
        assert_eq!(infra.anti_patterns[1], "no SQL injection");
    }

    // -----------------------------------------------------------------------
    // InterventionPoint
    // -----------------------------------------------------------------------

    #[test]
    fn test_intervention_point_serde_roundtrip() {
        let points = vec![
            InterventionPoint::AmbiguityDetected {
                contradictions: vec!["test A says 200, test B says 400".to_string()],
            },
            InterventionPoint::StrategyEscalation {
                proposed_strategy: "ArchitectReview".to_string(),
                reason: "limit cycle detected".to_string(),
            },
            InterventionPoint::BudgetExtension {
                current_remaining_fraction: 0.1,
                proposed_additional_tokens: 50_000,
                proposed_additional_iterations: 3,
            },
            InterventionPoint::PartialResult {
                convergence_level: 0.85,
                remaining_gaps: vec!["missing error handling".to_string()],
            },
            InterventionPoint::HumanEscalation {
                reason: "all escape strategies exhausted".to_string(),
                context_summary: "3 iterations, stuck on test failures".to_string(),
            },
        ];

        for point in points {
            let json = serde_json::to_string(&point).unwrap();
            let deserialized: InterventionPoint = serde_json::from_str(&json).unwrap();
            // Verify round-trip by re-serializing.
            let json2 = serde_json::to_string(&deserialized).unwrap();
            assert_eq!(json, json2);
        }
    }

    // -----------------------------------------------------------------------
    // BudgetExtension
    // -----------------------------------------------------------------------

    #[test]
    fn test_budget_extension_serde_roundtrip() {
        let ext = BudgetExtension {
            additional_tokens: 100_000,
            additional_iterations: 5,
        };
        let json = serde_json::to_string(&ext).unwrap();
        let deserialized: BudgetExtension = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.additional_tokens, 100_000);
        assert_eq!(deserialized.additional_iterations, 5);
    }

    // -----------------------------------------------------------------------
    // ConvergenceMode
    // -----------------------------------------------------------------------

    #[test]
    fn test_convergence_mode_sequential_serde() {
        let mode = ConvergenceMode::Sequential;
        let json = serde_json::to_string(&mode).unwrap();
        assert_eq!(json, "\"sequential\"");
        let deserialized: ConvergenceMode = serde_json::from_str(&json).unwrap();
        assert!(matches!(deserialized, ConvergenceMode::Sequential));
    }

    #[test]
    fn test_convergence_mode_parallel_serde() {
        let mode = ConvergenceMode::Parallel {
            initial_samples: 3,
        };
        let json = serde_json::to_string(&mode).unwrap();
        let deserialized: ConvergenceMode = serde_json::from_str(&json).unwrap();
        match deserialized {
            ConvergenceMode::Parallel { initial_samples } => {
                assert_eq!(initial_samples, 3);
            }
            _ => panic!("expected Parallel variant"),
        }
    }

    // -----------------------------------------------------------------------
    // select_convergence_mode
    // -----------------------------------------------------------------------

    /// Helper to construct a [`BasinWidth`] for tests.
    fn basin(classification: BasinClassification) -> BasinWidth {
        let score = match classification {
            BasinClassification::Wide => 0.8,
            BasinClassification::Moderate => 0.55,
            BasinClassification::Narrow => 0.3,
        };
        BasinWidth {
            score,
            classification,
        }
    }

    #[test]
    fn test_select_mode_user_override() {
        let b = basin(BasinClassification::Wide);
        let policy = ConvergencePolicy {
            priority_hint: None,
            ..Default::default()
        };
        let mode = select_convergence_mode(&b, &policy, Some(5));
        match mode {
            ConvergenceMode::Parallel { initial_samples } => {
                assert_eq!(initial_samples, 5);
            }
            _ => panic!("expected Parallel when user overrides"),
        }
    }

    #[test]
    fn test_select_mode_wide_basin_is_sequential() {
        let b = basin(BasinClassification::Wide);
        let policy = ConvergencePolicy {
            priority_hint: Some(PriorityHint::Thorough),
            ..Default::default()
        };
        assert!(matches!(
            select_convergence_mode(&b, &policy, None),
            ConvergenceMode::Sequential
        ));
    }

    #[test]
    fn test_select_mode_narrow_thorough_is_parallel_3() {
        let b = basin(BasinClassification::Narrow);
        let policy = ConvergencePolicy {
            priority_hint: Some(PriorityHint::Thorough),
            ..Default::default()
        };
        match select_convergence_mode(&b, &policy, None) {
            ConvergenceMode::Parallel { initial_samples } => {
                assert_eq!(initial_samples, 3);
            }
            _ => panic!("expected Parallel(3) for narrow+thorough"),
        }
    }

    #[test]
    fn test_select_mode_narrow_fast_is_parallel_2() {
        let b = basin(BasinClassification::Narrow);
        let policy = ConvergencePolicy {
            priority_hint: Some(PriorityHint::Fast),
            ..Default::default()
        };
        match select_convergence_mode(&b, &policy, None) {
            ConvergenceMode::Parallel { initial_samples } => {
                assert_eq!(initial_samples, 2);
            }
            _ => panic!("expected Parallel(2) for narrow+fast"),
        }
    }

    #[test]
    fn test_select_mode_narrow_cheap_is_sequential() {
        let b = basin(BasinClassification::Narrow);
        let policy = ConvergencePolicy {
            priority_hint: Some(PriorityHint::Cheap),
            ..Default::default()
        };
        assert!(matches!(
            select_convergence_mode(&b, &policy, None),
            ConvergenceMode::Sequential
        ));
    }

    #[test]
    fn test_select_mode_narrow_no_hint_is_parallel_2() {
        let b = basin(BasinClassification::Narrow);
        let policy = ConvergencePolicy {
            priority_hint: None,
            ..Default::default()
        };
        match select_convergence_mode(&b, &policy, None) {
            ConvergenceMode::Parallel { initial_samples } => {
                assert_eq!(initial_samples, 2);
            }
            _ => panic!("expected Parallel(2) for narrow+no hint"),
        }
    }

    #[test]
    fn test_select_mode_moderate_is_sequential() {
        let b = basin(BasinClassification::Moderate);
        let policy = ConvergencePolicy {
            priority_hint: Some(PriorityHint::Thorough),
            ..Default::default()
        };
        assert!(matches!(
            select_convergence_mode(&b, &policy, None),
            ConvergenceMode::Sequential
        ));
    }
}
