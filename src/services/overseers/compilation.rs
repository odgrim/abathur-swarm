//! Compilation overseer implementation.
//!
//! Runs a compilation command (e.g. `cargo check`) against an artifact and
//! produces a [`BuildResult`] signal indicating whether the code compiles
//! cleanly.
//!
//! This is a **Cheap** overseer -- it runs in Phase 1 of the overseer cluster
//! and its failure is considered blocking (no point running tests if the code
//! does not compile).

use async_trait::async_trait;
use tokio::process::Command;

use crate::domain::models::convergence::Overseer;
use crate::domain::models::convergence::{
    ArtifactReference, BuildResult, OverseerCost, OverseerResult, OverseerSignalUpdate,
};

// ---------------------------------------------------------------------------
// CompilationOverseer
// ---------------------------------------------------------------------------

/// Overseer that verifies the artifact compiles successfully.
///
/// Executes a configurable compilation command (defaulting to `cargo check`)
/// in the artifact's directory and parses stdout/stderr to extract error
/// counts and messages.
pub struct CompilationOverseer {
    /// The program to execute (e.g. `"cargo"`).
    program: String,
    /// Arguments to pass to the program (e.g. `["check"]`).
    args: Vec<String>,
}

impl CompilationOverseer {
    /// Create a new compilation overseer with the given command.
    ///
    /// # Arguments
    ///
    /// * `program` -- The executable to run (e.g. `"cargo"`).
    /// * `args` -- Arguments to pass (e.g. `["check"]`).
    pub fn new(program: impl Into<String>, args: Vec<String>) -> Self {
        Self {
            program: program.into(),
            args,
        }
    }

    /// Create a compilation overseer using `cargo check`.
    pub fn cargo_check() -> Self {
        Self::new("cargo", vec!["check".into()])
    }

    /// Parse compiler output to extract error count and error messages.
    fn parse_errors(stderr: &str) -> (u32, Vec<String>) {
        let mut errors = Vec::new();
        let mut error_count: u32 = 0;

        for line in stderr.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("error") {
                errors.push(trimmed.to_string());
                error_count += 1;
            }
        }

        // Try to extract the summary line: "error: could not compile" or
        // "error[E0xxx]" lines have already been counted. The final summary
        // "error: aborting due to N previous errors" gives the authoritative
        // count if present.
        for line in stderr.lines().rev() {
            let trimmed = line.trim();
            if let Some(rest) = trimmed.strip_prefix("error: aborting due to ")
                && let Some(count_str) = rest.split_whitespace().next()
                    && let Ok(count) = count_str.parse::<u32>() {
                        error_count = count;
                        break;
                    }
        }

        (error_count, errors)
    }
}

#[async_trait]
impl Overseer for CompilationOverseer {
    fn name(&self) -> &str {
        "compilation"
    }

    async fn measure(&self, artifact: &ArtifactReference) -> anyhow::Result<OverseerResult> {
        tracing::info!(
            overseer = self.name(),
            artifact_path = %artifact.path,
            "Running compilation check"
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
                    "Failed to spawn compilation command"
                );
                anyhow::anyhow!("Failed to spawn compilation command: {}", e)
            })?;

        let stderr = String::from_utf8_lossy(&output.stderr);
        let success = output.status.success();

        let (error_count, errors) = if success {
            (0, Vec::new())
        } else {
            Self::parse_errors(&stderr)
        };

        let build_result = BuildResult {
            success,
            error_count,
            errors,
        };

        tracing::info!(
            overseer = self.name(),
            success = success,
            error_count = error_count,
            "Compilation check complete"
        );

        Ok(OverseerResult {
            pass: success,
            signal: OverseerSignalUpdate::BuildResult(build_result),
        })
    }

    fn cost(&self) -> OverseerCost {
        OverseerCost::Cheap
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_errors_empty_output() {
        let (count, errors) = CompilationOverseer::parse_errors("");
        assert_eq!(count, 0);
        assert!(errors.is_empty());
    }

    #[test]
    fn parse_errors_with_summary_line() {
        let stderr = r#"error[E0308]: mismatched types
  --> src/main.rs:5:14
   |
5  |     let x: i32 = "hello";
   |            ---   ^^^^^^^ expected `i32`, found `&str`
   |            |
   |            expected due to this

error: aborting due to 1 previous error"#;

        let (count, errors) = CompilationOverseer::parse_errors(stderr);
        assert_eq!(count, 1);
        assert!(!errors.is_empty());
    }

    #[test]
    fn parse_errors_multiple_errors() {
        let stderr = r#"error[E0308]: mismatched types
error[E0425]: cannot find value `x` in this scope
error: aborting due to 2 previous errors"#;

        let (count, errors) = CompilationOverseer::parse_errors(stderr);
        assert_eq!(count, 2);
        assert_eq!(errors.len(), 3); // two E-codes + the summary
    }

    #[test]
    fn cargo_check_default() {
        let overseer = CompilationOverseer::cargo_check();
        assert_eq!(overseer.program, "cargo");
        assert_eq!(overseer.args, vec!["check"]);
        assert_eq!(overseer.name(), "compilation");
        assert_eq!(overseer.cost(), OverseerCost::Cheap);
    }
}
