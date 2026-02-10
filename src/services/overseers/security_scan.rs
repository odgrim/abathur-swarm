//! Security scan overseer implementation.
//!
//! Runs a security scanning command (e.g. `cargo audit`, `npm audit`) against
//! an artifact and produces a [`SecurityScanResult`] signal with vulnerability
//! counts by severity.
//!
//! This is a **Moderate** overseer -- it runs in Phase 2 of the overseer
//! cluster. Vulnerability findings feed into the security veto in convergence
//! delta computation (spec 1.4), ensuring that strategies introducing new
//! vulnerabilities never receive credit for "progress."

use async_trait::async_trait;
use tokio::process::Command;

use crate::domain::models::convergence::Overseer;
use crate::domain::models::convergence::{
    ArtifactReference, OverseerCost, OverseerResult, OverseerSignalUpdate, SecurityScanResult,
};

// ---------------------------------------------------------------------------
// SecurityScanOverseer
// ---------------------------------------------------------------------------

/// Overseer that runs a security scanner against the artifact.
///
/// Executes a configurable security scanning command and parses output to
/// extract vulnerability counts by severity (critical, high, medium) and
/// individual finding descriptions.
pub struct SecurityScanOverseer {
    /// The program to execute (e.g. `"cargo"`, `"npm"`).
    program: String,
    /// Arguments to pass to the program (e.g. `["audit"]`).
    args: Vec<String>,
}

impl SecurityScanOverseer {
    /// Create a new security scan overseer with the given command.
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

    /// Create a security scan overseer using `cargo audit`.
    pub fn cargo_audit() -> Self {
        Self::new("cargo", vec!["audit".into()])
    }

    /// Create a security scan overseer using `npm audit`.
    pub fn npm_audit() -> Self {
        Self::new("npm", vec!["audit".into()])
    }

    /// Parse security scanner output to extract vulnerability counts and findings.
    ///
    /// This parser recognizes common patterns from `cargo audit` and `npm audit`
    /// output. For unknown scanner output, it falls back to counting lines that
    /// mention severity keywords.
    fn parse_output(stdout: &str, stderr: &str) -> SecurityScanResult {
        let mut critical_count: u32 = 0;
        let mut high_count: u32 = 0;
        let mut medium_count: u32 = 0;
        let mut findings = Vec::new();

        let combined = format!("{}\n{}", stdout, stderr);

        for line in combined.lines() {
            let lower = line.to_lowercase();

            // Detect severity from output lines.
            if lower.contains("critical") {
                critical_count += 1;
                findings.push(line.trim().to_string());
            } else if lower.contains("high")
                && (lower.contains("severity") || lower.contains("vulnerability"))
            {
                high_count += 1;
                findings.push(line.trim().to_string());
            } else if lower.contains("medium")
                && (lower.contains("severity") || lower.contains("vulnerability"))
            {
                medium_count += 1;
                findings.push(line.trim().to_string());
            }
        }

        // Try to parse structured summary lines from npm audit:
        // "N critical", "N high", "N moderate"
        for line in combined.lines() {
            let trimmed = line.trim().to_lowercase();
            let parts: Vec<&str> = trimmed.split_whitespace().collect();
            if parts.len() >= 2 {
                if let Ok(count) = parts[0].parse::<u32>() {
                    match parts[1] {
                        "critical" => critical_count = count,
                        "high" => high_count = count,
                        "moderate" | "medium" => medium_count = count,
                        _ => {}
                    }
                }
            }
        }

        SecurityScanResult {
            critical_count,
            high_count,
            medium_count,
            findings,
        }
    }
}

#[async_trait]
impl Overseer for SecurityScanOverseer {
    fn name(&self) -> &str {
        "security-scan"
    }

    async fn measure(&self, artifact: &ArtifactReference) -> anyhow::Result<OverseerResult> {
        tracing::info!(
            overseer = self.name(),
            artifact_path = %artifact.path,
            "Running security scan"
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
                    "Failed to spawn security scan command"
                );
                anyhow::anyhow!("Failed to spawn security scan command: {}", e)
            })?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);

        let scan_result = Self::parse_output(&stdout, &stderr);

        // Pass if no critical vulnerabilities (spec 2.2).
        let pass = scan_result.critical_count == 0;

        tracing::info!(
            overseer = self.name(),
            pass = pass,
            critical = scan_result.critical_count,
            high = scan_result.high_count,
            medium = scan_result.medium_count,
            "Security scan complete"
        );

        Ok(OverseerResult {
            pass,
            signal: OverseerSignalUpdate::SecurityScan(scan_result),
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
        let result = SecurityScanOverseer::parse_output("", "");
        assert_eq!(result.critical_count, 0);
        assert_eq!(result.high_count, 0);
        assert_eq!(result.medium_count, 0);
        assert!(result.findings.is_empty());
    }

    #[test]
    fn parse_output_npm_style_summary() {
        let stdout = "found 3 vulnerabilities (1 moderate, 1 high, 1 critical)\n\
                       1 critical\n\
                       1 high\n\
                       1 moderate";
        let result = SecurityScanOverseer::parse_output(stdout, "");
        assert_eq!(result.critical_count, 1);
        assert_eq!(result.high_count, 1);
        assert_eq!(result.medium_count, 1);
    }

    #[test]
    fn parse_output_no_vulnerabilities() {
        let stdout = "0 vulnerabilities found";
        let result = SecurityScanOverseer::parse_output(stdout, "");
        assert_eq!(result.critical_count, 0);
        assert_eq!(result.high_count, 0);
        assert_eq!(result.medium_count, 0);
    }

    #[test]
    fn cargo_audit_default() {
        let overseer = SecurityScanOverseer::cargo_audit();
        assert_eq!(overseer.program, "cargo");
        assert_eq!(overseer.args, vec!["audit"]);
        assert_eq!(overseer.name(), "security-scan");
        assert_eq!(overseer.cost(), OverseerCost::Moderate);
    }
}
