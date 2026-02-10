//! Acceptance test overseer implementation.
//!
//! Runs acceptance tests (specific tests verifying task requirements) against
//! an artifact and produces a [`TestResults`] signal. Unlike the
//! [`TestSuiteOverseer`](super::test_suite::TestSuiteOverseer) which runs the
//! full test suite, this overseer targets specific acceptance tests that were
//! either discovered from the project or generated during the PREPARE phase.
//!
//! This is an **Expensive** overseer -- it runs in Phase 3 and can be skipped
//! via `policy.skip_expensive_overseers`.

use async_trait::async_trait;
use tokio::process::Command;

use crate::domain::models::convergence::Overseer;
use crate::domain::models::convergence::{
    ArtifactReference, OverseerCost, OverseerResult, OverseerSignalUpdate, TestResults,
};

// ---------------------------------------------------------------------------
// AcceptanceTestOverseer
// ---------------------------------------------------------------------------

/// Overseer that runs acceptance tests against the artifact.
///
/// Acceptance tests are specific tests that verify the task's requirements.
/// They can be:
/// - Discovered from the project (existing test files)
/// - Generated during the PREPARE phase from the task specification
/// - Provided by the user via [`TaskSubmission::references`]
///
/// The overseer takes the test definitions (file paths or test names) at
/// construction time and uses them to narrow the test execution scope.
pub struct AcceptanceTestOverseer {
    /// The program to execute (e.g. `"cargo"`, `"npm"`, `"pytest"`).
    program: String,
    /// Base arguments to pass to the program (e.g. `["test"]`).
    args: Vec<String>,
    /// Acceptance test definitions: file paths or test name patterns.
    ///
    /// These are appended to the command arguments to narrow test execution.
    /// For `cargo test`, these are test name filters. For `pytest`, these are
    /// test file paths.
    test_definitions: Vec<String>,
}

impl AcceptanceTestOverseer {
    /// Create a new acceptance test overseer with the given command and test
    /// definitions.
    ///
    /// # Arguments
    ///
    /// * `program` -- The executable to run.
    /// * `args` -- Base arguments to pass.
    /// * `test_definitions` -- Test names, file paths, or patterns that
    ///   identify the acceptance tests to run.
    pub fn new(
        program: impl Into<String>,
        args: Vec<String>,
        test_definitions: Vec<String>,
    ) -> Self {
        Self {
            program: program.into(),
            args,
            test_definitions,
        }
    }

    /// Create an acceptance test overseer using `cargo test` with the given
    /// test name filters.
    pub fn cargo_test(test_filters: Vec<String>) -> Self {
        Self::new("cargo", vec!["test".into()], test_filters)
    }

    /// Build the full command arguments by combining base args with test
    /// definitions.
    fn build_args(&self) -> Vec<String> {
        let mut args = self.args.clone();
        for definition in &self.test_definitions {
            args.push(definition.clone());
        }
        args
    }

    /// Parse test runner output to extract acceptance test results.
    ///
    /// Uses the same parsing logic as [`TestSuiteOverseer`] since the output
    /// format is the same -- only the scope of tests run differs.
    fn parse_output(stdout: &str, stderr: &str) -> TestResults {
        let mut passed: u32 = 0;
        let mut failed: u32 = 0;
        let mut skipped: u32 = 0;
        let mut failing_test_names = Vec::new();

        let combined = format!("{}\n{}", stdout, stderr);

        for line in combined.lines() {
            let trimmed = line.trim();

            if trimmed.starts_with("test ") && trimmed.ends_with("... ok") {
                passed += 1;
            } else if trimmed.starts_with("test ") && trimmed.ends_with("... FAILED") {
                failed += 1;
                if let Some(name) = trimmed
                    .strip_prefix("test ")
                    .and_then(|s| s.strip_suffix(" ... FAILED"))
                {
                    failing_test_names.push(name.trim().to_string());
                }
            } else if trimmed.starts_with("test ") && trimmed.ends_with("... ignored") {
                skipped += 1;
            }
        }

        // Parse the summary line.
        for line in combined.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("test result:") {
                if let Some(p) = Self::extract_count(trimmed, "passed") {
                    passed = p;
                }
                if let Some(f) = Self::extract_count(trimmed, "failed") {
                    failed = f;
                }
                if let Some(i) = Self::extract_count(trimmed, "ignored") {
                    skipped = i;
                }
            }
        }

        let total = passed + failed + skipped;

        TestResults {
            passed,
            failed,
            skipped,
            total,
            regression_count: 0,
            failing_test_names,
        }
    }

    /// Extract a numeric count preceding a keyword from a test summary line.
    fn extract_count(line: &str, keyword: &str) -> Option<u32> {
        let parts: Vec<&str> = line.split(';').collect();
        for part in parts {
            let trimmed = part.trim();
            if trimmed.contains(keyword) {
                for word in trimmed.split_whitespace() {
                    if let Ok(n) = word.parse::<u32>() {
                        return Some(n);
                    }
                }
            }
        }
        None
    }
}

#[async_trait]
impl Overseer for AcceptanceTestOverseer {
    fn name(&self) -> &str {
        "acceptance-test"
    }

    async fn measure(&self, artifact: &ArtifactReference) -> anyhow::Result<OverseerResult> {
        tracing::info!(
            overseer = self.name(),
            artifact_path = %artifact.path,
            test_count = self.test_definitions.len(),
            "Running acceptance tests"
        );

        if self.test_definitions.is_empty() {
            tracing::warn!(
                overseer = self.name(),
                "No acceptance test definitions configured; returning empty passing result"
            );
            let empty_results = TestResults {
                passed: 0,
                failed: 0,
                skipped: 0,
                total: 0,
                regression_count: 0,
                failing_test_names: Vec::new(),
            };
            return Ok(OverseerResult {
                pass: true,
                signal: OverseerSignalUpdate::TestResults(empty_results),
            });
        }

        let full_args = self.build_args();

        let output = Command::new(&self.program)
            .args(&full_args)
            .current_dir(&artifact.path)
            .output()
            .await
            .map_err(|e| {
                tracing::error!(
                    overseer = self.name(),
                    error = %e,
                    "Failed to spawn acceptance test command"
                );
                anyhow::anyhow!("Failed to spawn acceptance test command: {}", e)
            })?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);

        let test_results = Self::parse_output(&stdout, &stderr);
        let pass = test_results.all_passing() && output.status.success();

        tracing::info!(
            overseer = self.name(),
            pass = pass,
            passed = test_results.passed,
            failed = test_results.failed,
            skipped = test_results.skipped,
            "Acceptance tests complete"
        );

        Ok(OverseerResult {
            pass,
            signal: OverseerSignalUpdate::TestResults(test_results),
        })
    }

    fn cost(&self) -> OverseerCost {
        OverseerCost::Expensive
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_output_empty() {
        let result = AcceptanceTestOverseer::parse_output("", "");
        assert_eq!(result.passed, 0);
        assert_eq!(result.failed, 0);
        assert_eq!(result.total, 0);
        assert!(result.failing_test_names.is_empty());
    }

    #[test]
    fn parse_output_rust_style() {
        let stdout = r#"
running 2 tests
test acceptance::login_works ... ok
test acceptance::register_fails_without_email ... FAILED

test result: FAILED. 1 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out
"#;

        let result = AcceptanceTestOverseer::parse_output(stdout, "");
        assert_eq!(result.passed, 1);
        assert_eq!(result.failed, 1);
        assert_eq!(result.total, 2);
        assert_eq!(
            result.failing_test_names,
            vec!["acceptance::register_fails_without_email"]
        );
    }

    #[test]
    fn build_args_combines_base_and_definitions() {
        let overseer = AcceptanceTestOverseer::new(
            "cargo",
            vec!["test".into()],
            vec!["acceptance::".into(), "--".into(), "--nocapture".into()],
        );
        let args = overseer.build_args();
        assert_eq!(args, vec!["test", "acceptance::", "--", "--nocapture"]);
    }

    #[test]
    fn cargo_test_default() {
        let overseer = AcceptanceTestOverseer::cargo_test(vec!["acceptance".into()]);
        assert_eq!(overseer.program, "cargo");
        assert_eq!(overseer.args, vec!["test"]);
        assert_eq!(overseer.test_definitions, vec!["acceptance"]);
        assert_eq!(overseer.name(), "acceptance-test");
        assert_eq!(overseer.cost(), OverseerCost::Expensive);
    }

    #[test]
    fn empty_definitions_produces_passing_result() {
        let overseer = AcceptanceTestOverseer::new("cargo", vec!["test".into()], vec![]);
        assert!(overseer.test_definitions.is_empty());
    }
}
