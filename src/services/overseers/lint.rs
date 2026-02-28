//! Lint overseer implementation.
//!
//! Runs a linting command (e.g. `cargo clippy`, `eslint`) against an artifact
//! and produces a [`LintResults`] signal with error and warning counts.
//!
//! This is a **Moderate** overseer -- it runs in Phase 2 of the overseer
//! cluster, after cheap checks (compilation, type check) have passed.

use async_trait::async_trait;
use tokio::process::Command;

use crate::domain::models::convergence::Overseer;
use crate::domain::models::convergence::{
    ArtifactReference, LintResults, OverseerCost, OverseerResult, OverseerSignalUpdate,
};

// ---------------------------------------------------------------------------
// LintOverseer
// ---------------------------------------------------------------------------

/// Overseer that runs a linter against the artifact.
///
/// Executes a configurable lint command and parses output to extract error
/// counts, warning counts, and individual error messages.
pub struct LintOverseer {
    /// The program to execute (e.g. `"cargo"`, `"eslint"`).
    program: String,
    /// Arguments to pass to the program (e.g. `["clippy", "--", "-D", "warnings"]`).
    args: Vec<String>,
}

impl LintOverseer {
    /// Create a new lint overseer with the given command.
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

    /// Create a lint overseer using `cargo clippy`.
    pub fn cargo_clippy() -> Self {
        Self::new(
            "cargo",
            vec!["clippy".into(), "--".into(), "-D".into(), "warnings".into()],
        )
    }

    /// Parse linter output to extract error count, warning count, and messages.
    fn parse_output(stderr: &str, stdout: &str) -> (u32, u32, Vec<String>) {
        let mut errors = Vec::new();
        let mut error_count: u32 = 0;
        let mut warning_count: u32 = 0;

        // Parse both streams -- clippy writes to stderr, eslint to stdout.
        for line in stderr.lines().chain(stdout.lines()) {
            let trimmed = line.trim();
            if trimmed.starts_with("error") {
                errors.push(trimmed.to_string());
                error_count += 1;
            } else if trimmed.starts_with("warning") {
                warning_count += 1;
            }
        }

        // Look for Rust-style summary lines.
        for line in stderr.lines().rev() {
            let trimmed = line.trim();
            if let Some(rest) = trimmed.strip_prefix("error: aborting due to ")
                && let Some(count_str) = rest.split_whitespace().next()
                    && let Ok(count) = count_str.parse::<u32>() {
                        error_count = count;
                        break;
                    }
        }

        // Look for clippy-style warning summary: "warning: N warnings emitted"
        for line in stderr.lines().rev() {
            let trimmed = line.trim();
            if let Some(rest) = trimmed.strip_prefix("warning: ") {
                // "N warnings emitted" or "N warning emitted"
                if rest.contains("warning") && rest.contains("emitted")
                    && let Some(count_str) = rest.split_whitespace().next()
                        && let Ok(count) = count_str.parse::<u32>() {
                            warning_count = count;
                            break;
                        }
            }
        }

        (error_count, warning_count, errors)
    }
}

#[async_trait]
impl Overseer for LintOverseer {
    fn name(&self) -> &str {
        "lint"
    }

    async fn measure(&self, artifact: &ArtifactReference) -> anyhow::Result<OverseerResult> {
        tracing::info!(
            overseer = self.name(),
            artifact_path = %artifact.path,
            "Running linter"
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
                    "Failed to spawn lint command"
                );
                anyhow::anyhow!("Failed to spawn lint command: {}", e)
            })?;

        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let pass = output.status.success();

        let (error_count, warning_count, errors) = if pass {
            // Even on success, parse warnings from output.
            Self::parse_output(&stderr, &stdout)
        } else {
            Self::parse_output(&stderr, &stdout)
        };

        let lint_results = LintResults {
            error_count,
            warning_count,
            errors,
        };

        tracing::info!(
            overseer = self.name(),
            pass = pass,
            error_count = error_count,
            warning_count = warning_count,
            "Lint check complete"
        );

        Ok(OverseerResult {
            pass,
            signal: OverseerSignalUpdate::LintResults(lint_results),
        })
    }

    fn cost(&self) -> OverseerCost {
        OverseerCost::Moderate
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
        let (errors, warnings, messages) = LintOverseer::parse_output("", "");
        assert_eq!(errors, 0);
        assert_eq!(warnings, 0);
        assert!(messages.is_empty());
    }

    #[test]
    fn parse_output_warnings_only() {
        let stderr = "warning: unused variable `x`\nwarning: 1 warning emitted";
        let (errors, warnings, _messages) = LintOverseer::parse_output(stderr, "");
        assert_eq!(errors, 0);
        assert_eq!(warnings, 1);
    }

    #[test]
    fn parse_output_errors_and_warnings() {
        let stderr = r#"error: this could be simplified
warning: unused import
error: aborting due to 1 previous error"#;

        let (errors, _warnings, messages) = LintOverseer::parse_output(stderr, "");
        assert_eq!(errors, 1);
        assert!(!messages.is_empty());
    }

    #[test]
    fn cargo_clippy_default() {
        let overseer = LintOverseer::cargo_clippy();
        assert_eq!(overseer.program, "cargo");
        assert_eq!(overseer.name(), "lint");
        assert_eq!(overseer.cost(), OverseerCost::Moderate);
    }
}
