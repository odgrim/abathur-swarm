//! Build overseer implementation.
//!
//! Runs a full build command (e.g. `cargo build`) against an artifact and
//! produces a [`BuildResult`] signal. Unlike the compilation overseer (which
//! runs `cargo check` for fast verification), this overseer runs a full build
//! including linking.
//!
//! This is a **Cheap** overseer -- build failures are blocking and there is
//! no point running tests or lints if the build does not succeed.

use async_trait::async_trait;
use tokio::process::Command;

use crate::domain::models::convergence::Overseer;
use crate::domain::models::convergence::{
    ArtifactReference, BuildResult, OverseerCost, OverseerResult, OverseerSignalUpdate,
};

// ---------------------------------------------------------------------------
// BuildOverseer
// ---------------------------------------------------------------------------

/// Overseer that verifies the artifact builds successfully.
///
/// Executes a configurable build command (defaulting to `cargo build`) in
/// the artifact's directory and parses output to extract error information.
pub struct BuildOverseer {
    /// The program to execute (e.g. `"cargo"`, `"npm"`).
    program: String,
    /// Arguments to pass to the program (e.g. `["build"]`).
    args: Vec<String>,
}

impl BuildOverseer {
    /// Create a new build overseer with the given command.
    ///
    /// # Arguments
    ///
    /// * `program` -- The executable to run (e.g. `"cargo"`).
    /// * `args` -- Arguments to pass (e.g. `["build"]`).
    pub fn new(program: impl Into<String>, args: Vec<String>) -> Self {
        Self {
            program: program.into(),
            args,
        }
    }

    /// Create a build overseer using `cargo build`.
    pub fn cargo_build() -> Self {
        Self::new("cargo", vec!["build".into()])
    }

    /// Create a build overseer using `npm run build`.
    pub fn npm_build() -> Self {
        Self::new("npm", vec!["run".into(), "build".into()])
    }

    /// Parse build output to extract error count and error messages.
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

        // Look for the Rust-style summary: "error: aborting due to N previous errors"
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
impl Overseer for BuildOverseer {
    fn name(&self) -> &str {
        "build"
    }

    async fn measure(&self, artifact: &ArtifactReference) -> anyhow::Result<OverseerResult> {
        tracing::info!(
            overseer = self.name(),
            artifact_path = %artifact.path,
            "Running full build"
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
                    "Failed to spawn build command"
                );
                anyhow::anyhow!("Failed to spawn build command: {}", e)
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
            "Build complete"
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
    fn parse_errors_empty() {
        let (count, errors) = BuildOverseer::parse_errors("");
        assert_eq!(count, 0);
        assert!(errors.is_empty());
    }

    #[test]
    fn parse_errors_with_summary() {
        let stderr = r#"error[E0463]: can't find crate for `missing`
error: aborting due to 1 previous error"#;

        let (count, errors) = BuildOverseer::parse_errors(stderr);
        assert_eq!(count, 1);
        assert!(!errors.is_empty());
    }

    #[test]
    fn cargo_build_default() {
        let overseer = BuildOverseer::cargo_build();
        assert_eq!(overseer.program, "cargo");
        assert_eq!(overseer.args, vec!["build"]);
        assert_eq!(overseer.name(), "build");
        assert_eq!(overseer.cost(), OverseerCost::Cheap);
    }

    #[test]
    fn npm_build_default() {
        let overseer = BuildOverseer::npm_build();
        assert_eq!(overseer.program, "npm");
        assert_eq!(overseer.args, vec!["run", "build"]);
    }
}
