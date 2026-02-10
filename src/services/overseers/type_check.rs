//! Type check overseer implementation.
//!
//! Runs a type checking command (e.g. `cargo check`, `tsc --noEmit`) against
//! an artifact and produces a [`TypeCheckResult`] signal.
//!
//! This is a **Cheap** overseer -- it runs in Phase 1 of the overseer cluster
//! and its failure is considered blocking.

use async_trait::async_trait;
use tokio::process::Command;

use crate::domain::models::convergence::Overseer;
use crate::domain::models::convergence::{
    ArtifactReference, OverseerCost, OverseerResult, OverseerSignalUpdate, TypeCheckResult,
};

// ---------------------------------------------------------------------------
// TypeCheckOverseer
// ---------------------------------------------------------------------------

/// Overseer that verifies the artifact passes type checking.
///
/// Executes a configurable type check command and parses stderr to extract
/// type error counts and messages. For Rust projects, type checking is
/// effectively the same as compilation (`cargo check`), but for polyglot
/// projects (TypeScript, Python with mypy, etc.) this is a separate step.
pub struct TypeCheckOverseer {
    /// The program to execute (e.g. `"tsc"`, `"mypy"`, `"cargo"`).
    program: String,
    /// Arguments to pass to the program (e.g. `["--noEmit"]`).
    args: Vec<String>,
}

impl TypeCheckOverseer {
    /// Create a new type check overseer with the given command.
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

    /// Create a type check overseer using `cargo check` (for Rust projects).
    pub fn cargo_check() -> Self {
        Self::new("cargo", vec!["check".into()])
    }

    /// Create a type check overseer using `tsc --noEmit` (for TypeScript projects).
    pub fn typescript() -> Self {
        Self::new("tsc", vec!["--noEmit".into()])
    }

    /// Parse type checker output to extract error count and messages.
    fn parse_errors(stderr: &str, stdout: &str) -> (u32, Vec<String>) {
        let mut errors = Vec::new();
        let mut error_count: u32 = 0;

        // Parse both stderr and stdout since different type checkers write
        // to different streams (rustc -> stderr, tsc -> stdout).
        for line in stderr.lines().chain(stdout.lines()) {
            let trimmed = line.trim();
            if trimmed.starts_with("error") || trimmed.contains(": error ") {
                errors.push(trimmed.to_string());
                error_count += 1;
            }
        }

        // Look for the Rust-style summary line.
        for line in stderr.lines().rev() {
            let trimmed = line.trim();
            if let Some(rest) = trimmed.strip_prefix("error: aborting due to ") {
                if let Some(count_str) = rest.split_whitespace().next() {
                    if let Ok(count) = count_str.parse::<u32>() {
                        error_count = count;
                        break;
                    }
                }
            }
        }

        (error_count, errors)
    }
}

#[async_trait]
impl Overseer for TypeCheckOverseer {
    fn name(&self) -> &str {
        "type-check"
    }

    async fn measure(&self, artifact: &ArtifactReference) -> anyhow::Result<OverseerResult> {
        tracing::info!(
            overseer = self.name(),
            artifact_path = %artifact.path,
            "Running type check"
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
                    "Failed to spawn type check command"
                );
                anyhow::anyhow!("Failed to spawn type check command: {}", e)
            })?;

        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let clean = output.status.success();

        let (error_count, errors) = if clean {
            (0, Vec::new())
        } else {
            Self::parse_errors(&stderr, &stdout)
        };

        let type_check_result = TypeCheckResult {
            clean,
            error_count,
            errors,
        };

        tracing::info!(
            overseer = self.name(),
            clean = clean,
            error_count = error_count,
            "Type check complete"
        );

        Ok(OverseerResult {
            pass: clean,
            signal: OverseerSignalUpdate::TypeCheck(type_check_result),
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
    fn parse_errors_empty() {
        let (count, errors) = TypeCheckOverseer::parse_errors("", "");
        assert_eq!(count, 0);
        assert!(errors.is_empty());
    }

    #[test]
    fn parse_errors_rust_style() {
        let stderr = "error[E0308]: mismatched types\nerror: aborting due to 1 previous error";
        let (count, _errors) = TypeCheckOverseer::parse_errors(stderr, "");
        assert_eq!(count, 1);
    }

    #[test]
    fn parse_errors_typescript_style() {
        let stdout = "src/index.ts(5,3): error TS2322: Type 'string' is not assignable";
        let (count, errors) = TypeCheckOverseer::parse_errors("", stdout);
        assert_eq!(count, 1);
        assert!(!errors.is_empty());
    }

    #[test]
    fn cargo_check_default() {
        let overseer = TypeCheckOverseer::cargo_check();
        assert_eq!(overseer.name(), "type-check");
        assert_eq!(overseer.cost(), OverseerCost::Cheap);
    }

    #[test]
    fn typescript_default() {
        let overseer = TypeCheckOverseer::typescript();
        assert_eq!(overseer.program, "tsc");
        assert_eq!(overseer.args, vec!["--noEmit"]);
    }
}
