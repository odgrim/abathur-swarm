//! Overseer cluster service implementation.
//!
//! Provides [`OverseerClusterService`], a service-layer wrapper around the
//! domain-layer [`OverseerCluster`] that adds timing, logging, and
//! measurement collection.
//!
//! The domain-layer [`OverseerCluster`] already implements the core phased
//! execution logic (spec 2.3). This service adds:
//! - Per-overseer timing via [`OverseerMeasurement`]
//! - Structured logging of cluster execution
//! - A convenience method that accepts [`ConvergencePolicy`] directly

use std::time::Instant;

use super::traits::{apply_signal_update, has_blocking_failures, OverseerMeasurement};
use crate::domain::models::convergence::{
    ArtifactReference, ConvergencePolicy, Overseer, OverseerCluster, OverseerCost,
    OverseerSignals,
};

// ---------------------------------------------------------------------------
// OverseerClusterService
// ---------------------------------------------------------------------------

/// Service-layer orchestrator for phased overseer execution.
///
/// Wraps a collection of [`Overseer`] implementations and executes them
/// in cost-ordered phases per the spec (2.3):
///
/// 1. **Phase 1 (Cheap)** -- Compilation, type check, build. Always runs.
///    If blocking failures are detected, remaining phases are skipped.
/// 2. **Phase 2 (Moderate)** -- Lint, security scan. Runs only if Phase 1
///    has no blocking failures.
/// 3. **Phase 3 (Expensive)** -- Full test suite, acceptance tests. Skipped
///    if `policy.skip_expensive_overseers` is `true` or Phase 1 has blocking
///    failures.
///
/// Unlike the domain-layer [`OverseerCluster`], this service collects
/// per-overseer [`OverseerMeasurement`]s with timing data for observability.
pub struct OverseerClusterService {
    overseers: Vec<Box<dyn Overseer>>,
}

impl OverseerClusterService {
    /// Create a new empty cluster service.
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
    /// Accepts a [`ConvergencePolicy`] and reads `skip_expensive_overseers`
    /// from it. Returns aggregated [`OverseerSignals`] from all phases that
    /// were executed.
    ///
    /// See the struct-level documentation for the phased execution strategy.
    pub async fn measure(
        &self,
        artifact: &ArtifactReference,
        policy: &ConvergencePolicy,
    ) -> OverseerSignals {
        tracing::info!(
            artifact_path = %artifact.path,
            overseer_count = self.overseers.len(),
            skip_expensive = policy.skip_expensive_overseers,
            "Starting overseer cluster measurement"
        );

        let cluster_start = Instant::now();

        // Phase 1: Cheap overseers -- always run.
        let (cheap_signals, cheap_measurements) =
            self.run_phase(OverseerCost::Cheap, artifact).await;

        tracing::info!(
            phase = "cheap",
            overseer_count = cheap_measurements.len(),
            elapsed_ms = cluster_start.elapsed().as_millis() as u64,
            "Phase 1 (cheap) complete"
        );

        if has_blocking_failures(&cheap_signals) {
            tracing::warn!(
                "Cheap overseers produced blocking failures; skipping moderate and expensive phases"
            );
            return OverseerSignals::from_partial(cheap_signals);
        }

        // Phase 2: Moderate overseers.
        let (moderate_signals, moderate_measurements) =
            self.run_phase(OverseerCost::Moderate, artifact).await;

        tracing::info!(
            phase = "moderate",
            overseer_count = moderate_measurements.len(),
            elapsed_ms = cluster_start.elapsed().as_millis() as u64,
            "Phase 2 (moderate) complete"
        );

        // Phase 3: Expensive overseers -- skippable by policy.
        if policy.skip_expensive_overseers {
            tracing::info!("Skipping expensive overseers per convergence policy");
            return OverseerSignals::merge(
                cheap_signals,
                moderate_signals,
                OverseerSignals::empty(),
            );
        }

        let (expensive_signals, expensive_measurements) =
            self.run_phase(OverseerCost::Expensive, artifact).await;

        tracing::info!(
            phase = "expensive",
            overseer_count = expensive_measurements.len(),
            total_elapsed_ms = cluster_start.elapsed().as_millis() as u64,
            "Phase 3 (expensive) complete"
        );

        OverseerSignals::merge(cheap_signals, moderate_signals, expensive_signals)
    }

    /// Run all overseers in the cluster that match the given cost tier.
    ///
    /// Returns both the aggregated signals and the individual measurements
    /// (with timing data). Overseers that return errors are logged and
    /// skipped.
    async fn run_phase(
        &self,
        cost: OverseerCost,
        artifact: &ArtifactReference,
    ) -> (OverseerSignals, Vec<OverseerMeasurement>) {
        let mut signals = OverseerSignals::empty();
        let mut measurements = Vec::new();

        for overseer in self.overseers.iter().filter(|o| o.cost() == cost) {
            let start = Instant::now();

            match overseer.measure(artifact).await {
                Ok(result) => {
                    let duration_ms = start.elapsed().as_millis() as u64;

                    tracing::debug!(
                        overseer = overseer.name(),
                        pass = result.pass,
                        duration_ms = duration_ms,
                        "Overseer measurement complete"
                    );

                    measurements.push(OverseerMeasurement {
                        overseer_name: overseer.name().to_string(),
                        result: result.clone(),
                        duration_ms,
                    });

                    apply_signal_update(&mut signals, result.signal);
                }
                Err(err) => {
                    let duration_ms = start.elapsed().as_millis() as u64;

                    tracing::warn!(
                        overseer = overseer.name(),
                        error = %err,
                        duration_ms = duration_ms,
                        "Overseer measurement failed; skipping"
                    );
                }
            }
        }

        (signals, measurements)
    }

    /// Run all overseers and return both the aggregated signals and the
    /// individual measurements.
    ///
    /// This is the same as [`measure`](Self::measure) but also returns
    /// the per-overseer timing data for observability and debugging.
    pub async fn measure_with_details(
        &self,
        artifact: &ArtifactReference,
        policy: &ConvergencePolicy,
    ) -> (OverseerSignals, Vec<OverseerMeasurement>) {
        let cluster_start = Instant::now();
        let mut all_measurements = Vec::new();

        // Phase 1: Cheap.
        let (cheap_signals, cheap_measurements) =
            self.run_phase(OverseerCost::Cheap, artifact).await;
        all_measurements.extend(cheap_measurements);

        if has_blocking_failures(&cheap_signals) {
            return (
                OverseerSignals::from_partial(cheap_signals),
                all_measurements,
            );
        }

        // Phase 2: Moderate.
        let (moderate_signals, moderate_measurements) =
            self.run_phase(OverseerCost::Moderate, artifact).await;
        all_measurements.extend(moderate_measurements);

        // Phase 3: Expensive.
        if policy.skip_expensive_overseers {
            let signals =
                OverseerSignals::merge(cheap_signals, moderate_signals, OverseerSignals::empty());
            return (signals, all_measurements);
        }

        let (expensive_signals, expensive_measurements) =
            self.run_phase(OverseerCost::Expensive, artifact).await;
        all_measurements.extend(expensive_measurements);

        let _ = cluster_start; // Used only for structured logging above.

        let signals = OverseerSignals::merge(cheap_signals, moderate_signals, expensive_signals);
        (signals, all_measurements)
    }
}

impl Default for OverseerClusterService {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Conversion from domain-layer OverseerCluster
// ---------------------------------------------------------------------------

impl From<OverseerCluster> for OverseerClusterService {
    /// Convert a domain-layer [`OverseerCluster`] into an
    /// [`OverseerClusterService`].
    ///
    /// Note: This conversion is not directly possible because
    /// `OverseerCluster` does not expose its internal overseer vector.
    /// This impl creates an empty service. Use [`configure`](Self::configure)
    /// to populate it.
    fn from(_cluster: OverseerCluster) -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::models::convergence::{
        BuildResult, OverseerResult, OverseerSignalUpdate, TypeCheckResult,
    };
    use async_trait::async_trait;

    // -- Mock overseers for testing ------------------------------------------

    struct MockOverseer {
        name: &'static str,
        cost: OverseerCost,
        result: OverseerResult,
    }

    #[async_trait]
    impl Overseer for MockOverseer {
        fn name(&self) -> &str {
            self.name
        }

        async fn measure(&self, _artifact: &ArtifactReference) -> anyhow::Result<OverseerResult> {
            Ok(self.result.clone())
        }

        fn cost(&self) -> OverseerCost {
            self.cost
        }
    }

    struct FailingOverseer {
        name: &'static str,
        cost: OverseerCost,
    }

    #[async_trait]
    impl Overseer for FailingOverseer {
        fn name(&self) -> &str {
            self.name
        }

        async fn measure(&self, _artifact: &ArtifactReference) -> anyhow::Result<OverseerResult> {
            Err(anyhow::anyhow!("overseer crashed"))
        }

        fn cost(&self) -> OverseerCost {
            self.cost
        }
    }

    fn test_artifact() -> ArtifactReference {
        ArtifactReference::new("/test/path", "hash123")
    }

    fn passing_build_result() -> OverseerResult {
        OverseerResult {
            pass: true,
            signal: OverseerSignalUpdate::BuildResult(BuildResult {
                success: true,
                error_count: 0,
                errors: vec![],
            }),
        }
    }

    fn failing_build_result() -> OverseerResult {
        OverseerResult {
            pass: false,
            signal: OverseerSignalUpdate::BuildResult(BuildResult {
                success: false,
                error_count: 1,
                errors: vec!["link error".into()],
            }),
        }
    }

    fn passing_type_check_result() -> OverseerResult {
        OverseerResult {
            pass: true,
            signal: OverseerSignalUpdate::TypeCheck(TypeCheckResult {
                clean: true,
                error_count: 0,
                errors: vec![],
            }),
        }
    }

    // -- Tests ---------------------------------------------------------------

    #[tokio::test]
    async fn empty_cluster_returns_empty_signals() {
        let cluster = OverseerClusterService::new();
        let policy = ConvergencePolicy::default();
        let signals = cluster.measure(&test_artifact(), &policy).await;

        assert!(!signals.has_any_signal());
    }

    #[tokio::test]
    async fn all_phases_run_when_cheap_passes() {
        let mut cluster = OverseerClusterService::new();

        cluster.add(Box::new(MockOverseer {
            name: "build",
            cost: OverseerCost::Cheap,
            result: passing_build_result(),
        }));
        cluster.add(Box::new(MockOverseer {
            name: "type-check",
            cost: OverseerCost::Cheap,
            result: passing_type_check_result(),
        }));
        cluster.add(Box::new(MockOverseer {
            name: "lint",
            cost: OverseerCost::Moderate,
            result: OverseerResult {
                pass: true,
                signal: OverseerSignalUpdate::LintResults(
                    crate::domain::models::convergence::LintResults {
                        error_count: 0,
                        warning_count: 2,
                        errors: vec![],
                    },
                ),
            },
        }));
        cluster.add(Box::new(MockOverseer {
            name: "test-suite",
            cost: OverseerCost::Expensive,
            result: OverseerResult {
                pass: true,
                signal: OverseerSignalUpdate::TestResults(
                    crate::domain::models::convergence::TestResults {
                        passed: 10,
                        failed: 0,
                        skipped: 0,
                        total: 10,
                        regression_count: 0,
                        failing_test_names: vec![],
                    },
                ),
            },
        }));

        let policy = ConvergencePolicy::default();
        let signals = cluster.measure(&test_artifact(), &policy).await;

        // All phases ran, so all signal types should be present.
        assert!(signals.build_result.is_some());
        assert!(signals.type_check.is_some());
        assert!(signals.lint_results.is_some());
        assert!(signals.test_results.is_some());
        assert!(signals.all_passing());
    }

    #[tokio::test]
    async fn blocking_failure_skips_later_phases() {
        let mut cluster = OverseerClusterService::new();

        cluster.add(Box::new(MockOverseer {
            name: "build",
            cost: OverseerCost::Cheap,
            result: failing_build_result(),
        }));
        cluster.add(Box::new(MockOverseer {
            name: "lint",
            cost: OverseerCost::Moderate,
            result: OverseerResult {
                pass: true,
                signal: OverseerSignalUpdate::LintResults(
                    crate::domain::models::convergence::LintResults {
                        error_count: 0,
                        warning_count: 0,
                        errors: vec![],
                    },
                ),
            },
        }));
        cluster.add(Box::new(MockOverseer {
            name: "test-suite",
            cost: OverseerCost::Expensive,
            result: OverseerResult {
                pass: true,
                signal: OverseerSignalUpdate::TestResults(
                    crate::domain::models::convergence::TestResults {
                        passed: 10,
                        failed: 0,
                        skipped: 0,
                        total: 10,
                        regression_count: 0,
                        failing_test_names: vec![],
                    },
                ),
            },
        }));

        let policy = ConvergencePolicy::default();
        let signals = cluster.measure(&test_artifact(), &policy).await;

        // Build ran (cheap), but lint (moderate) and tests (expensive) were skipped.
        assert!(signals.build_result.is_some());
        assert!(!signals.build_result.as_ref().unwrap().success);
        assert!(signals.lint_results.is_none());
        assert!(signals.test_results.is_none());
    }

    #[tokio::test]
    async fn skip_expensive_policy_skips_phase_3() {
        let mut cluster = OverseerClusterService::new();

        cluster.add(Box::new(MockOverseer {
            name: "build",
            cost: OverseerCost::Cheap,
            result: passing_build_result(),
        }));
        cluster.add(Box::new(MockOverseer {
            name: "test-suite",
            cost: OverseerCost::Expensive,
            result: OverseerResult {
                pass: true,
                signal: OverseerSignalUpdate::TestResults(
                    crate::domain::models::convergence::TestResults {
                        passed: 10,
                        failed: 0,
                        skipped: 0,
                        total: 10,
                        regression_count: 0,
                        failing_test_names: vec![],
                    },
                ),
            },
        }));

        let policy = ConvergencePolicy {
            skip_expensive_overseers: true,
            ..Default::default()
        };
        let signals = cluster.measure(&test_artifact(), &policy).await;

        // Build ran, but tests were skipped.
        assert!(signals.build_result.is_some());
        assert!(signals.test_results.is_none());
    }

    #[tokio::test]
    async fn failing_overseer_is_skipped_gracefully() {
        let mut cluster = OverseerClusterService::new();

        cluster.add(Box::new(MockOverseer {
            name: "build",
            cost: OverseerCost::Cheap,
            result: passing_build_result(),
        }));
        cluster.add(Box::new(FailingOverseer {
            name: "broken-check",
            cost: OverseerCost::Cheap,
        }));

        let policy = ConvergencePolicy::default();
        let signals = cluster.measure(&test_artifact(), &policy).await;

        // Build succeeded, broken check was skipped.
        assert!(signals.build_result.is_some());
        assert!(signals.build_result.as_ref().unwrap().success);
    }

    #[tokio::test]
    async fn measure_with_details_returns_measurements() {
        let mut cluster = OverseerClusterService::new();

        cluster.add(Box::new(MockOverseer {
            name: "build",
            cost: OverseerCost::Cheap,
            result: passing_build_result(),
        }));
        cluster.add(Box::new(MockOverseer {
            name: "lint",
            cost: OverseerCost::Moderate,
            result: OverseerResult {
                pass: true,
                signal: OverseerSignalUpdate::LintResults(
                    crate::domain::models::convergence::LintResults {
                        error_count: 0,
                        warning_count: 0,
                        errors: vec![],
                    },
                ),
            },
        }));

        let policy = ConvergencePolicy::default();
        let (signals, measurements) = cluster
            .measure_with_details(&test_artifact(), &policy)
            .await;

        assert!(signals.build_result.is_some());
        assert!(signals.lint_results.is_some());
        assert_eq!(measurements.len(), 2);
        assert_eq!(measurements[0].overseer_name, "build");
        assert_eq!(measurements[1].overseer_name, "lint");
    }

    #[test]
    fn default_cluster_is_empty() {
        let cluster = OverseerClusterService::default();
        assert!(cluster.overseers.is_empty());
    }
}
