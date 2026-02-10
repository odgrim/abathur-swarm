//! Test suite overseer implementation.
//!
//! Runs the project's test suite (e.g. `cargo test`, `npm test`) against an
//! artifact and produces a [`TestResults`] signal with pass/fail/skip counts
//! and failing test names.
//!
//! This is an **Expensive** overseer -- it runs in Phase 3 of the overseer
//! cluster and can be skipped via `policy.skip_expensive_overseers`.

use async_trait::async_trait;
use tokio::process::Command;

use crate::domain::models::convergence::Overseer;
use crate::domain::models::convergence::{
    ArtifactReference, OverseerCost, OverseerResult, OverseerSignalUpdate, TestResults,
};

// ---------------------------------------------------------------------------
// TestSuiteOverseer
// ---------------------------------------------------------------------------

/// Overseer that runs the project's test suite against the artifact.
///
/// Executes a configurable test command and parses output to extract test
/// counts (passed, failed, skipped) and the names of failing tests. Failing
/// test names are used by downstream strategies (e.g. `FocusedRepair`) to
/// narrow context to specific failures.
pub struct TestSuiteOverseer {
    /// The program to execute (e.g. `"cargo"`, `"npm"`, `"pytest"`).
    program: String,
    /// Arguments to pass to the program (e.g. `["test"]`).
    args: Vec<String>,
}

impl TestSuiteOverseer {
    /// Create a new test suite overseer with the given command.
    ///
    /// # Arguments
    ///
    /// * `program` -- The executable to run.
    /// * `args` -- Arguments to pass.
    pub fn new(program: impl Into<String>, args: Vec<String>) -> Self {
        Self {
            program: program.into(),
            args,
        }
    }

    /// Create a test suite overseer using `cargo test`.
    pub fn cargo_test() -> Self {
        Self::new("cargo", vec!["test".into()])
    }

    /// Create a test suite overseer using `npm test`.
    pub fn npm_test() -> Self {
        Self::new("npm", vec!["test".into()])
    }

    /// Parse test runner output to extract test results.
    ///
    /// Recognizes patterns from `cargo test` output:
    /// - `test result: ok. N passed; N failed; N ignored;`
    /// - `test some::test_name ... FAILED`
    ///
    /// For unknown test runners, falls back to heuristic line counting.
    fn parse_output(stdout: &str, stderr: &str) -> TestResults {
        let mut passed: u32 = 0;
        let mut failed: u32 = 0;
        let mut skipped: u32 = 0;
        let mut failing_test_names = Vec::new();

        let combined = format!("{}\n{}", stdout, stderr);

        // Look for Rust test output: "test some::path ... ok" / "... FAILED"
        for line in combined.lines() {
            let trimmed = line.trim();

            if trimmed.starts_with("test ") && trimmed.ends_with("... ok") {
                passed += 1;
            } else if trimmed.starts_with("test ") && trimmed.ends_with("... FAILED") {
                failed += 1;
                // Extract test name: "test some::path ... FAILED" -> "some::path"
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

        // Try to parse the Rust test summary line:
        // "test result: ok. 10 passed; 0 failed; 2 ignored; 0 measured; 0 filtered out"
        for line in combined.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("test result:") {
                // Parse "N passed"
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
            regression_count: 0, // Regressions require comparison with prior signals.
            failing_test_names,
        }
    }

    /// Extract a numeric count preceding a keyword from a test summary line.
    ///
    /// E.g., from "10 passed; 0 failed" extract 10 for keyword "passed".
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
impl Overseer for TestSuiteOverseer {
    fn name(&self) -> &str {
        "test-suite"
    }

    async fn measure(&self, artifact: &ArtifactReference) -> anyhow::Result<OverseerResult> {
        tracing::info!(
            overseer = self.name(),
            artifact_path = %artifact.path,
            "Running test suite"
        );

        let output = Command::new(&self.program)
            .args(&self.args)
            .current_dir(&artifact.path)
            .output()
            .await
            .map_err(|e| {
                tracing::error!(
                    overseer = self.name(),
                    error = %e,
                    "Failed to spawn test command"
                );
                anyhow::anyhow!("Failed to spawn test command: {}", e)
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
            "Test suite complete"
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
        let result = TestSuiteOverseer::parse_output("", "");
        assert_eq!(result.passed, 0);
        assert_eq!(result.failed, 0);
        assert_eq!(result.skipped, 0);
        assert_eq!(result.total, 0);
        assert!(result.failing_test_names.is_empty());
    }

    #[test]
    fn parse_output_rust_style() {
        let stdout = r#"
running 3 tests
test tests::test_a ... ok
test tests::test_b ... FAILED
test tests::test_c ... ignored

test result: FAILED. 1 passed; 1 failed; 1 ignored; 0 measured; 0 filtered out
"#;

        let result = TestSuiteOverseer::parse_output(stdout, "");
        assert_eq!(result.passed, 1);
        assert_eq!(result.failed, 1);
        assert_eq!(result.skipped, 1);
        assert_eq!(result.total, 3);
        assert_eq!(result.failing_test_names, vec!["tests::test_b"]);
    }

    #[test]
    fn parse_output_all_passing() {
        let stdout = r#"
running 5 tests
test tests::a ... ok
test tests::b ... ok
test tests::c ... ok
test tests::d ... ok
test tests::e ... ok

test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
"#;

        let result = TestSuiteOverseer::parse_output(stdout, "");
        assert_eq!(result.passed, 5);
        assert_eq!(result.failed, 0);
        assert!(result.all_passing());
    }

    #[test]
    fn extract_count_from_summary() {
        let line = "test result: ok. 10 passed; 2 failed; 1 ignored; 0 measured";
        assert_eq!(TestSuiteOverseer::extract_count(line, "passed"), Some(10));
        assert_eq!(TestSuiteOverseer::extract_count(line, "failed"), Some(2));
        assert_eq!(TestSuiteOverseer::extract_count(line, "ignored"), Some(1));
        assert_eq!(TestSuiteOverseer::extract_count(line, "missing"), None);
    }

    #[test]
    fn cargo_test_default() {
        let overseer = TestSuiteOverseer::cargo_test();
        assert_eq!(overseer.program, "cargo");
        assert_eq!(overseer.args, vec!["test"]);
        assert_eq!(overseer.name(), "test-suite");
        assert_eq!(overseer.cost(), OverseerCost::Expensive);
    }
}
