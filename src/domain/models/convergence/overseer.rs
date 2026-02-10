//! Overseer types and traits (Spec Part 2).
//!
//! Overseers are external verification signals that measure implementation state
//! without self-bias. The convergence engine receives overseers as configured
//! inputs -- infrastructure discovery is handled upstream.
//!
//! The [`OverseerSignals`] struct aggregates the output of all overseers for a
//! single observation. Each field is `Option` because not all overseers apply to
//! every task.
//!
//! The [`Overseer`] trait defines the interface for individual overseers, and
//! [`OverseerCluster`] orchestrates phased execution: cheap overseers run first,
//! and expensive ones are skipped when cheap results already show blocking
//! failures or when the convergence policy says so (spec 2.3).

use serde::{Deserialize, Serialize};

use super::trajectory::ArtifactReference;

// ---------------------------------------------------------------------------
// TestResults
// ---------------------------------------------------------------------------

/// Results from running the test suite against an artifact.
///
/// Captures pass/fail/skip counts, regression detection, and the names of
/// individual failing tests so that downstream strategies (e.g. `FocusedRepair`)
/// can target specific failures.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestResults {
    /// Number of tests that passed.
    pub passed: u32,
    /// Number of tests that failed.
    pub failed: u32,
    /// Number of tests that were skipped.
    pub skipped: u32,
    /// Total number of tests (passed + failed + skipped).
    pub total: u32,
    /// Number of tests that previously passed but now fail.
    ///
    /// Regressions are weighted heavily in convergence delta computation
    /// because they indicate the agent is undoing prior progress.
    pub regression_count: u32,
    /// Names of the individual failing tests.
    ///
    /// Used by `FocusedRepair` and similar strategies to narrow context to
    /// only the relevant failures.
    pub failing_test_names: Vec<String>,
}

impl TestResults {
    /// Returns `true` when every test passes and there are no regressions.
    pub fn all_passing(&self) -> bool {
        self.failed == 0 && self.regression_count == 0
    }
}

// ---------------------------------------------------------------------------
// TypeCheckResult
// ---------------------------------------------------------------------------

/// Result of running the type checker (e.g. `cargo check`, `tsc`) against an
/// artifact.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TypeCheckResult {
    /// Whether the type check completed with zero errors.
    pub clean: bool,
    /// Number of type errors found.
    pub error_count: u32,
    /// Individual error messages from the type checker.
    pub errors: Vec<String>,
}

// ---------------------------------------------------------------------------
// LintResults
// ---------------------------------------------------------------------------

/// Result of running the linter (e.g. `clippy`, `eslint`) against an artifact.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LintResults {
    /// Number of lint errors (distinct from warnings).
    pub error_count: u32,
    /// Number of lint warnings.
    pub warning_count: u32,
    /// Individual error messages from the linter.
    pub errors: Vec<String>,
}

// ---------------------------------------------------------------------------
// BuildResult
// ---------------------------------------------------------------------------

/// Result of running the build (e.g. `cargo build`, `npm run build`) against
/// an artifact.
///
/// Build failures cap convergence level at 0.3 (see spec 1.4).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuildResult {
    /// Whether the build succeeded.
    pub success: bool,
    /// Number of build errors.
    pub error_count: u32,
    /// Individual error messages from the build.
    pub errors: Vec<String>,
}

// ---------------------------------------------------------------------------
// SecurityScanResult
// ---------------------------------------------------------------------------

/// Result of running a security scanner against an artifact.
///
/// Vulnerabilities accumulate non-linearly with iteration count. The security
/// veto in convergence delta computation (spec 1.4) ensures that strategies
/// introducing new vulnerabilities never receive credit for "progress."
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecurityScanResult {
    /// Number of critical-severity findings.
    pub critical_count: u32,
    /// Number of high-severity findings.
    pub high_count: u32,
    /// Number of medium-severity findings.
    pub medium_count: u32,
    /// Human-readable descriptions of individual findings.
    pub findings: Vec<String>,
}

// ---------------------------------------------------------------------------
// CustomCheckResult
// ---------------------------------------------------------------------------

/// Result of a user-defined custom check.
///
/// Custom checks are extensible overseers defined via scripts with
/// success/failure pattern matching.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomCheckResult {
    /// Name identifying this custom check.
    pub name: String,
    /// Whether the check passed.
    pub passed: bool,
    /// Human-readable details about the check result.
    pub details: String,
}

// ---------------------------------------------------------------------------
// OverseerSignals
// ---------------------------------------------------------------------------

/// Aggregated output from all overseers for a single observation (spec 1.3).
///
/// Each field is `Option` because not all overseers apply to every task. For
/// example, a documentation-only task may have no test suite and no build step,
/// so `test_results` and `build_result` would both be `None`.
///
/// The convergence engine uses these signals to compute convergence delta and
/// convergence level (spec 1.4), classify attractors (spec Part 3), and select
/// strategies (spec Part 4).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OverseerSignals {
    /// Results from running the test suite.
    pub test_results: Option<TestResults>,
    /// Results from the type checker.
    pub type_check: Option<TypeCheckResult>,
    /// Results from the linter.
    pub lint_results: Option<LintResults>,
    /// Results from the build.
    pub build_result: Option<BuildResult>,
    /// Results from the security scanner.
    pub security_scan: Option<SecurityScanResult>,
    /// Results from user-defined custom checks.
    pub custom_checks: Vec<CustomCheckResult>,
}

impl Default for OverseerSignals {
    fn default() -> Self {
        Self::empty()
    }
}

impl OverseerSignals {
    /// Returns `true` if at least one signal source has reported results.
    ///
    /// When no signals are present, convergence level is 0.0 because the
    /// system has no basis for assessing convergence (spec 1.4).
    pub fn has_any_signal(&self) -> bool {
        self.test_results.is_some()
            || self.type_check.is_some()
            || self.lint_results.is_some()
            || self.build_result.is_some()
            || self.security_scan.is_some()
            || !self.custom_checks.is_empty()
    }

    /// Returns `true` when every present signal indicates success.
    ///
    /// Absent signals are treated as passing (they simply do not apply to
    /// the current task). This means `all_passing()` returns `true` for an
    /// empty `OverseerSignals`, but convergence level is still 0.0 because
    /// `has_any_signal()` returns `false`.
    pub fn all_passing(&self) -> bool {
        self.test_results
            .as_ref()
            .map(|t| t.all_passing())
            .unwrap_or(true)
            && self
                .type_check
                .as_ref()
                .map(|t| t.clean)
                .unwrap_or(true)
            && self
                .lint_results
                .as_ref()
                .map(|l| l.error_count == 0)
                .unwrap_or(true)
            && self
                .build_result
                .as_ref()
                .map(|b| b.success)
                .unwrap_or(true)
            && self
                .security_scan
                .as_ref()
                .map(|s| s.critical_count == 0)
                .unwrap_or(true)
            && self.custom_checks.iter().all(|c| c.passed)
    }

    /// Total error count across build, type check, and lint results.
    ///
    /// Used in convergence delta computation (spec 1.4) to measure the
    /// functional distance between consecutive observations.
    pub fn error_count(&self) -> u32 {
        let build = self
            .build_result
            .as_ref()
            .map(|b| b.error_count)
            .unwrap_or(0);
        let type_c = self
            .type_check
            .as_ref()
            .map(|t| t.error_count)
            .unwrap_or(0);
        let lint = self
            .lint_results
            .as_ref()
            .map(|l| l.error_count)
            .unwrap_or(0);
        build + type_c + lint
    }

    /// Combined count of critical and high severity vulnerabilities.
    ///
    /// Used in the security veto within convergence delta computation
    /// (spec 1.4). When this count increases between observations, the delta
    /// is capped at zero to prevent the bandit from rewarding
    /// vulnerability-introducing strategies.
    pub fn vulnerability_count(&self) -> u32 {
        self.security_scan
            .as_ref()
            .map(|s| s.critical_count + s.high_count)
            .unwrap_or(0)
    }

    /// Number of test regressions (tests that previously passed but now fail).
    ///
    /// Extracted from `test_results` if present, otherwise zero.
    pub fn regression_count(&self) -> u32 {
        self.test_results
            .as_ref()
            .map(|t| t.regression_count)
            .unwrap_or(0)
    }

    /// Create an empty `OverseerSignals` with all fields set to `None`
    /// and no custom checks.
    pub fn empty() -> Self {
        Self {
            test_results: None,
            type_check: None,
            lint_results: None,
            build_result: None,
            security_scan: None,
            custom_checks: Vec::new(),
        }
    }

    /// Create signals from a partial set of results.
    ///
    /// This is used when cheap overseers report blocking failures and
    /// expensive overseers are skipped (spec 2.3). The resulting signals
    /// contain only the results that were actually collected.
    pub fn from_partial(partial: OverseerSignals) -> Self {
        partial
    }

    /// Merge three sets of overseer signals (cheap, moderate, expensive)
    /// into a single aggregate.
    ///
    /// For each optional field, the first `Some` value encountered wins
    /// (cheap > moderate > expensive). Custom checks are concatenated from
    /// all three phases.
    ///
    /// This implements the phased merge described in spec 2.3.
    pub fn merge(
        cheap: OverseerSignals,
        moderate: OverseerSignals,
        expensive: OverseerSignals,
    ) -> Self {
        let mut custom_checks = cheap.custom_checks;
        custom_checks.extend(moderate.custom_checks);
        custom_checks.extend(expensive.custom_checks);

        Self {
            test_results: cheap
                .test_results
                .or(moderate.test_results)
                .or(expensive.test_results),
            type_check: cheap
                .type_check
                .or(moderate.type_check)
                .or(expensive.type_check),
            lint_results: cheap
                .lint_results
                .or(moderate.lint_results)
                .or(expensive.lint_results),
            build_result: cheap
                .build_result
                .or(moderate.build_result)
                .or(expensive.build_result),
            security_scan: cheap
                .security_scan
                .or(moderate.security_scan)
                .or(expensive.security_scan),
            custom_checks,
        }
    }
}

// ---------------------------------------------------------------------------
// OverseerCost
// ---------------------------------------------------------------------------

/// Cost classification for an overseer (spec 2.1).
///
/// Determines the execution phase in [`OverseerCluster::measure`]. Cheap
/// overseers run first; if they show blocking failures, more expensive
/// overseers are skipped to conserve budget.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OverseerCost {
    /// Fast checks like compilation and type checking.
    Cheap,
    /// Moderate-cost checks like linting, fast tests, and security scans.
    Moderate,
    /// Expensive checks like full test suites and integration tests.
    Expensive,
}

// ---------------------------------------------------------------------------
// OverseerSignalUpdate
// ---------------------------------------------------------------------------

/// Indicates which field of [`OverseerSignals`] an [`OverseerResult`] updates.
///
/// Each overseer produces a single signal type. The convergence engine uses
/// this variant to route the result into the correct field of the aggregated
/// [`OverseerSignals`].
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OverseerSignalUpdate {
    /// Update the `test_results` field.
    TestResults(TestResults),
    /// Update the `type_check` field.
    TypeCheck(TypeCheckResult),
    /// Update the `lint_results` field.
    LintResults(LintResults),
    /// Update the `build_result` field.
    BuildResult(BuildResult),
    /// Update the `security_scan` field.
    SecurityScan(SecurityScanResult),
    /// Append to the `custom_checks` vector.
    CustomCheck(CustomCheckResult),
}

// ---------------------------------------------------------------------------
// OverseerResult
// ---------------------------------------------------------------------------

/// The result of a single overseer measurement (spec 2.1).
///
/// Combines a pass/fail verdict with the specific signal update to be applied
/// to the aggregated [`OverseerSignals`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OverseerResult {
    /// Whether this overseer considers the artifact acceptable.
    pub pass: bool,
    /// The specific signal data to merge into [`OverseerSignals`].
    pub signal: OverseerSignalUpdate,
}

// ---------------------------------------------------------------------------
// Overseer trait
// ---------------------------------------------------------------------------

/// External verification tool that measures implementation state (spec 2.1).
///
/// Overseers are deterministic, external measurement tools -- they never
/// self-assess. Each overseer runs against an [`ArtifactReference`] and
/// produces an [`OverseerResult`] containing both a pass/fail verdict and
/// the signal data to merge into the observation's [`OverseerSignals`].
///
/// # Cost Classification
///
/// Every overseer declares its [`OverseerCost`]. The [`OverseerCluster`]
/// uses this to implement phased execution (spec 2.3): cheap overseers run
/// first, and expensive overseers are skipped when cheap results already
/// show blocking failures.
///
/// # Built-in Implementations
///
/// The spec describes the following built-in overseers:
/// `CompilationOverseer`, `TypeCheckOverseer`, `LintOverseer`,
/// `BuildOverseer`, `TestSuiteOverseer`, `SecurityScanOverseer`,
/// `AcceptanceTestOverseer`.
///
/// User-extensible overseers can be defined via custom scripts with
/// success/failure pattern matching.
#[async_trait::async_trait]
pub trait Overseer: Send + Sync {
    /// Human-readable name identifying this overseer (e.g. `"security-scan"`).
    fn name(&self) -> &str;

    /// Measure the given artifact and produce an [`OverseerResult`].
    ///
    /// Implementations should be idempotent: measuring the same artifact
    /// twice should produce the same result.
    async fn measure(&self, artifact: &ArtifactReference) -> anyhow::Result<OverseerResult>;

    /// The cost classification of this overseer.
    ///
    /// Determines which phase of [`OverseerCluster::measure`] this overseer
    /// runs in.
    fn cost(&self) -> OverseerCost;
}

// ---------------------------------------------------------------------------
// OverseerCluster
// ---------------------------------------------------------------------------

/// A collection of overseers that execute in cost-ordered phases (spec 2.3).
///
/// The cluster implements the phased execution strategy described in the spec:
///
/// 1. **Phase 1 (Cheap)** -- Compilation, type check. Always runs.
/// 2. **Phase 2 (Moderate)** -- Lint, fast tests, security scan. Runs only
///    if cheap overseers have no blocking failures.
/// 3. **Phase 3 (Expensive)** -- Full test suite, integration tests. Skipped
///    if the convergence policy's `skip_expensive_overseers` flag is set, or
///    if cheap overseers have blocking failures.
///
/// This prioritization conserves budget: there is no point running the full
/// test suite if the code does not compile.
pub struct OverseerCluster {
    overseers: Vec<Box<dyn Overseer>>,
}

impl OverseerCluster {
    /// Create a new empty cluster.
    pub fn new() -> Self {
        Self {
            overseers: Vec::new(),
        }
    }

    /// Add an overseer to the cluster.
    pub fn add(&mut self, overseer: Box<dyn Overseer>) {
        self.overseers.push(overseer);
    }

    /// Configure the cluster with a complete set of overseers, replacing any
    /// previously registered overseers.
    pub fn configure(&mut self, overseers: Vec<Box<dyn Overseer>>) {
        self.overseers = overseers;
    }

    /// Run all overseers against the given artifact using phased execution.
    ///
    /// Implements the prioritization strategy from spec 2.3:
    ///
    /// 1. Run cheap overseers. If any produce blocking failures (build or
    ///    type check failures), return early with only cheap results.
    /// 2. Run moderate overseers.
    /// 3. If `skip_expensive` is `true`, skip expensive overseers and merge
    ///    cheap + moderate results. Otherwise, run expensive overseers and
    ///    merge all three phases.
    ///
    /// # Arguments
    ///
    /// * `artifact` -- Reference to the artifact to measure.
    /// * `skip_expensive` -- When `true`, phase 3 (expensive) overseers are
    ///   skipped entirely. This corresponds to the convergence policy's
    ///   `skip_expensive_overseers` flag.
    pub async fn measure(
        &self,
        artifact: &ArtifactReference,
        skip_expensive: bool,
    ) -> OverseerSignals {
        // Phase 1: Cheap overseers -- always run.
        let cheap_signals = self
            .run_overseers_by_cost(OverseerCost::Cheap, artifact)
            .await;

        if Self::has_blocking_failures(&cheap_signals) {
            return OverseerSignals::from_partial(cheap_signals);
        }

        // Phase 2: Moderate overseers.
        let moderate_signals = self
            .run_overseers_by_cost(OverseerCost::Moderate, artifact)
            .await;

        // Phase 3: Expensive overseers -- skippable by policy.
        if skip_expensive {
            return OverseerSignals::merge(cheap_signals, moderate_signals, OverseerSignals::empty());
        }

        let expensive_signals = self
            .run_overseers_by_cost(OverseerCost::Expensive, artifact)
            .await;

        OverseerSignals::merge(cheap_signals, moderate_signals, expensive_signals)
    }

    /// Run all overseers in the cluster that match the given cost tier.
    ///
    /// Results are collected into an [`OverseerSignals`] by routing each
    /// overseer's [`OverseerSignalUpdate`] into the appropriate field.
    /// Overseers that return errors are logged and skipped.
    async fn run_overseers_by_cost(
        &self,
        cost: OverseerCost,
        artifact: &ArtifactReference,
    ) -> OverseerSignals {
        let mut signals = OverseerSignals::empty();

        for overseer in self.overseers.iter().filter(|o| o.cost() == cost) {
            match overseer.measure(artifact).await {
                Ok(result) => {
                    Self::apply_signal_update(&mut signals, result.signal);
                }
                Err(err) => {
                    tracing::warn!(
                        overseer = overseer.name(),
                        error = %err,
                        "Overseer measurement failed; skipping"
                    );
                }
            }
        }

        signals
    }

    /// Route an [`OverseerSignalUpdate`] into the appropriate field of the
    /// given [`OverseerSignals`].
    fn apply_signal_update(signals: &mut OverseerSignals, update: OverseerSignalUpdate) {
        match update {
            OverseerSignalUpdate::TestResults(r) => signals.test_results = Some(r),
            OverseerSignalUpdate::TypeCheck(r) => signals.type_check = Some(r),
            OverseerSignalUpdate::LintResults(r) => signals.lint_results = Some(r),
            OverseerSignalUpdate::BuildResult(r) => signals.build_result = Some(r),
            OverseerSignalUpdate::SecurityScan(r) => signals.security_scan = Some(r),
            OverseerSignalUpdate::CustomCheck(r) => signals.custom_checks.push(r),
        }
    }

    /// Determine whether the given signals contain blocking failures.
    ///
    /// A blocking failure is a build failure or type check failure -- there is
    /// no point running expensive overseers (full test suite, integration tests)
    /// when the code does not compile or has type errors.
    fn has_blocking_failures(signals: &OverseerSignals) -> bool {
        let build_failed = signals
            .build_result
            .as_ref()
            .map(|b| !b.success)
            .unwrap_or(false);

        let type_check_failed = signals
            .type_check
            .as_ref()
            .map(|t| !t.clean)
            .unwrap_or(false);

        build_failed || type_check_failed
    }
}

impl Default for OverseerCluster {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -- TestResults ---------------------------------------------------------

    #[test]
    fn test_results_all_passing_when_no_failures() {
        let results = TestResults {
            passed: 10,
            failed: 0,
            skipped: 2,
            total: 12,
            regression_count: 0,
            failing_test_names: vec![],
        };
        assert!(results.all_passing());
    }

    #[test]
    fn test_results_not_all_passing_when_failures() {
        let results = TestResults {
            passed: 8,
            failed: 2,
            skipped: 0,
            total: 10,
            regression_count: 0,
            failing_test_names: vec!["test_a".into(), "test_b".into()],
        };
        assert!(!results.all_passing());
    }

    #[test]
    fn test_results_not_all_passing_when_regressions() {
        let results = TestResults {
            passed: 10,
            failed: 0,
            skipped: 0,
            total: 10,
            regression_count: 1,
            failing_test_names: vec![],
        };
        assert!(!results.all_passing());
    }

    // -- OverseerSignals -----------------------------------------------------

    #[test]
    fn empty_signals_has_no_signal() {
        let signals = OverseerSignals::empty();
        assert!(!signals.has_any_signal());
    }

    #[test]
    fn signals_with_test_results_has_signal() {
        let mut signals = OverseerSignals::empty();
        signals.test_results = Some(TestResults {
            passed: 5,
            failed: 0,
            skipped: 0,
            total: 5,
            regression_count: 0,
            failing_test_names: vec![],
        });
        assert!(signals.has_any_signal());
    }

    #[test]
    fn signals_with_custom_checks_has_signal() {
        let mut signals = OverseerSignals::empty();
        signals.custom_checks.push(CustomCheckResult {
            name: "format".into(),
            passed: true,
            details: "all files formatted".into(),
        });
        assert!(signals.has_any_signal());
    }

    #[test]
    fn empty_signals_all_passing() {
        let signals = OverseerSignals::empty();
        assert!(signals.all_passing());
    }

    #[test]
    fn all_passing_with_successful_signals() {
        let signals = OverseerSignals {
            test_results: Some(TestResults {
                passed: 10,
                failed: 0,
                skipped: 0,
                total: 10,
                regression_count: 0,
                failing_test_names: vec![],
            }),
            type_check: Some(TypeCheckResult {
                clean: true,
                error_count: 0,
                errors: vec![],
            }),
            lint_results: Some(LintResults {
                error_count: 0,
                warning_count: 3,
                errors: vec![],
            }),
            build_result: Some(BuildResult {
                success: true,
                error_count: 0,
                errors: vec![],
            }),
            security_scan: Some(SecurityScanResult {
                critical_count: 0,
                high_count: 0,
                medium_count: 1,
                findings: vec!["medium finding".into()],
            }),
            custom_checks: vec![CustomCheckResult {
                name: "fmt".into(),
                passed: true,
                details: "ok".into(),
            }],
        };
        assert!(signals.all_passing());
    }

    #[test]
    fn not_all_passing_with_build_failure() {
        let mut signals = OverseerSignals::empty();
        signals.build_result = Some(BuildResult {
            success: false,
            error_count: 1,
            errors: vec!["link error".into()],
        });
        assert!(!signals.all_passing());
    }

    #[test]
    fn not_all_passing_with_critical_vulnerability() {
        let mut signals = OverseerSignals::empty();
        signals.security_scan = Some(SecurityScanResult {
            critical_count: 1,
            high_count: 0,
            medium_count: 0,
            findings: vec!["SQL injection".into()],
        });
        assert!(!signals.all_passing());
    }

    #[test]
    fn not_all_passing_with_failing_custom_check() {
        let mut signals = OverseerSignals::empty();
        signals.custom_checks.push(CustomCheckResult {
            name: "coverage".into(),
            passed: false,
            details: "coverage below threshold".into(),
        });
        assert!(!signals.all_passing());
    }

    #[test]
    fn error_count_sums_build_type_lint() {
        let signals = OverseerSignals {
            test_results: None,
            type_check: Some(TypeCheckResult {
                clean: false,
                error_count: 3,
                errors: vec![],
            }),
            lint_results: Some(LintResults {
                error_count: 2,
                warning_count: 5,
                errors: vec![],
            }),
            build_result: Some(BuildResult {
                success: false,
                error_count: 1,
                errors: vec![],
            }),
            security_scan: None,
            custom_checks: vec![],
        };
        assert_eq!(signals.error_count(), 6);
    }

    #[test]
    fn error_count_zero_when_all_none() {
        let signals = OverseerSignals::empty();
        assert_eq!(signals.error_count(), 0);
    }

    #[test]
    fn vulnerability_count_sums_critical_and_high() {
        let mut signals = OverseerSignals::empty();
        signals.security_scan = Some(SecurityScanResult {
            critical_count: 2,
            high_count: 3,
            medium_count: 10,
            findings: vec![],
        });
        assert_eq!(signals.vulnerability_count(), 5);
    }

    #[test]
    fn vulnerability_count_zero_without_scan() {
        let signals = OverseerSignals::empty();
        assert_eq!(signals.vulnerability_count(), 0);
    }

    #[test]
    fn regression_count_from_test_results() {
        let mut signals = OverseerSignals::empty();
        signals.test_results = Some(TestResults {
            passed: 8,
            failed: 2,
            skipped: 0,
            total: 10,
            regression_count: 2,
            failing_test_names: vec![],
        });
        assert_eq!(signals.regression_count(), 2);
    }

    #[test]
    fn regression_count_zero_without_tests() {
        let signals = OverseerSignals::empty();
        assert_eq!(signals.regression_count(), 0);
    }

    #[test]
    fn default_is_empty() {
        let signals = OverseerSignals::default();
        assert!(!signals.has_any_signal());
        assert!(signals.test_results.is_none());
        assert!(signals.type_check.is_none());
        assert!(signals.lint_results.is_none());
        assert!(signals.build_result.is_none());
        assert!(signals.security_scan.is_none());
        assert!(signals.custom_checks.is_empty());
    }

    // -- OverseerSignals::merge ----------------------------------------------

    #[test]
    fn merge_cheap_wins_for_optional_fields() {
        let cheap = OverseerSignals {
            test_results: None,
            type_check: Some(TypeCheckResult {
                clean: true,
                error_count: 0,
                errors: vec![],
            }),
            lint_results: None,
            build_result: Some(BuildResult {
                success: true,
                error_count: 0,
                errors: vec![],
            }),
            security_scan: None,
            custom_checks: vec![],
        };

        let moderate = OverseerSignals {
            test_results: None,
            type_check: Some(TypeCheckResult {
                clean: false,
                error_count: 5,
                errors: vec!["should not appear".into()],
            }),
            lint_results: Some(LintResults {
                error_count: 1,
                warning_count: 0,
                errors: vec![],
            }),
            build_result: None,
            security_scan: None,
            custom_checks: vec![],
        };

        let expensive = OverseerSignals {
            test_results: Some(TestResults {
                passed: 20,
                failed: 0,
                skipped: 0,
                total: 20,
                regression_count: 0,
                failing_test_names: vec![],
            }),
            type_check: None,
            lint_results: None,
            build_result: None,
            security_scan: None,
            custom_checks: vec![],
        };

        let merged = OverseerSignals::merge(cheap, moderate, expensive);

        // type_check from cheap wins (clean: true), not moderate (clean: false)
        assert!(merged.type_check.as_ref().unwrap().clean);
        // build_result from cheap
        assert!(merged.build_result.as_ref().unwrap().success);
        // lint_results from moderate (cheap had None)
        assert_eq!(merged.lint_results.as_ref().unwrap().error_count, 1);
        // test_results from expensive (cheap and moderate had None)
        assert_eq!(merged.test_results.as_ref().unwrap().passed, 20);
    }

    #[test]
    fn merge_concatenates_custom_checks() {
        let cheap = OverseerSignals {
            custom_checks: vec![CustomCheckResult {
                name: "fmt".into(),
                passed: true,
                details: "ok".into(),
            }],
            ..OverseerSignals::empty()
        };

        let moderate = OverseerSignals {
            custom_checks: vec![CustomCheckResult {
                name: "coverage".into(),
                passed: false,
                details: "below threshold".into(),
            }],
            ..OverseerSignals::empty()
        };

        let expensive = OverseerSignals {
            custom_checks: vec![CustomCheckResult {
                name: "integration".into(),
                passed: true,
                details: "passed".into(),
            }],
            ..OverseerSignals::empty()
        };

        let merged = OverseerSignals::merge(cheap, moderate, expensive);
        assert_eq!(merged.custom_checks.len(), 3);
        assert_eq!(merged.custom_checks[0].name, "fmt");
        assert_eq!(merged.custom_checks[1].name, "coverage");
        assert_eq!(merged.custom_checks[2].name, "integration");
    }

    // -- OverseerCost serde --------------------------------------------------

    #[test]
    fn overseer_cost_serde_roundtrip() {
        let costs = vec![OverseerCost::Cheap, OverseerCost::Moderate, OverseerCost::Expensive];
        for cost in costs {
            let json = serde_json::to_string(&cost).unwrap();
            let deserialized: OverseerCost = serde_json::from_str(&json).unwrap();
            assert_eq!(cost, deserialized);
        }
    }

    #[test]
    fn overseer_cost_snake_case_serialization() {
        assert_eq!(
            serde_json::to_string(&OverseerCost::Cheap).unwrap(),
            "\"cheap\""
        );
        assert_eq!(
            serde_json::to_string(&OverseerCost::Moderate).unwrap(),
            "\"moderate\""
        );
        assert_eq!(
            serde_json::to_string(&OverseerCost::Expensive).unwrap(),
            "\"expensive\""
        );
    }

    // -- OverseerSignalUpdate serde ------------------------------------------

    #[test]
    fn signal_update_serde_roundtrip() {
        let update = OverseerSignalUpdate::BuildResult(BuildResult {
            success: true,
            error_count: 0,
            errors: vec![],
        });
        let json = serde_json::to_string(&update).unwrap();
        let deserialized: OverseerSignalUpdate = serde_json::from_str(&json).unwrap();
        if let OverseerSignalUpdate::BuildResult(r) = deserialized {
            assert!(r.success);
        } else {
            panic!("expected BuildResult variant");
        }
    }

    // -- OverseerCluster -----------------------------------------------------

    #[test]
    fn cluster_default_is_empty() {
        let cluster = OverseerCluster::default();
        assert!(cluster.overseers.is_empty());
    }

    // -- OverseerCluster::has_blocking_failures ------------------------------

    #[test]
    fn no_blocking_failures_on_empty() {
        let signals = OverseerSignals::empty();
        assert!(!OverseerCluster::has_blocking_failures(&signals));
    }

    #[test]
    fn blocking_failure_on_build_failure() {
        let mut signals = OverseerSignals::empty();
        signals.build_result = Some(BuildResult {
            success: false,
            error_count: 1,
            errors: vec!["error".into()],
        });
        assert!(OverseerCluster::has_blocking_failures(&signals));
    }

    #[test]
    fn blocking_failure_on_type_check_failure() {
        let mut signals = OverseerSignals::empty();
        signals.type_check = Some(TypeCheckResult {
            clean: false,
            error_count: 3,
            errors: vec![],
        });
        assert!(OverseerCluster::has_blocking_failures(&signals));
    }

    #[test]
    fn no_blocking_failure_on_test_failure_only() {
        let mut signals = OverseerSignals::empty();
        signals.test_results = Some(TestResults {
            passed: 5,
            failed: 3,
            skipped: 0,
            total: 8,
            regression_count: 0,
            failing_test_names: vec![],
        });
        assert!(!OverseerCluster::has_blocking_failures(&signals));
    }
}
