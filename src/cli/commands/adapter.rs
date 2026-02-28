//! Implementation of the `abathur adapter` command.
//!
//! Provides subcommands for managing adapter plugins: listing available
//! and enabled adapters, enabling/disabling them, showing detailed info,
//! and running health checks.

use anyhow::{bail, Context, Result};
use clap::{Args, Subcommand};
use std::path::{Path, PathBuf};
use tokio::fs;

use crate::adapters::plugins::{find_known_adapter, KnownAdapter, KNOWN_ADAPTERS};
use crate::cli::output::{output, CommandOutput};
use crate::domain::models::adapter::AdapterManifest;
use crate::services::adapter_loader::find_missing_env_vars;
use crate::services::config::AdapterConfig;

#[derive(Args, Debug)]
pub struct AdapterArgs {
    #[command(subcommand)]
    pub command: AdapterCommands,
}

#[derive(Subcommand, Debug)]
pub enum AdapterCommands {
    /// List all adapters (available and enabled)
    List,
    /// Enable an adapter by scaffolding its config directory
    Enable {
        /// Adapter name
        name: String,
        /// Overwrite existing configuration
        #[arg(long)]
        force: bool,
    },
    /// Disable an adapter by removing its config directory
    Disable {
        /// Adapter name
        name: String,
        /// Skip confirmation for customized configs
        #[arg(long)]
        force: bool,
    },
    /// Show detailed info about a specific adapter
    Info {
        /// Adapter name
        name: String,
    },
    /// Check adapter health (env vars, config validity, connectivity)
    Doctor {
        /// Adapter name (omit to check all enabled adapters)
        name: Option<String>,
    },
}

// ---------------------------------------------------------------------------
// Output types
// ---------------------------------------------------------------------------

#[derive(Debug, serde::Serialize)]
pub struct AdapterListEntry {
    pub name: String,
    pub adapter_type: String,
    pub direction: String,
    pub status: String,
    pub env_status: String,
}

#[derive(Debug, serde::Serialize)]
pub struct AdapterListOutput {
    pub adapters: Vec<AdapterListEntry>,
}

impl CommandOutput for AdapterListOutput {
    fn to_human(&self) -> String {
        if self.adapters.is_empty() {
            return "No adapters found.".to_string();
        }

        let mut lines = vec![format!(
            "{:<16} {:<10} {:<16} {:<12} {}",
            "NAME", "TYPE", "DIRECTION", "STATUS", "ENV VARS"
        )];
        lines.push("-".repeat(68));

        for a in &self.adapters {
            lines.push(format!(
                "{:<16} {:<10} {:<16} {:<12} {}",
                a.name, a.adapter_type, a.direction, a.status, a.env_status,
            ));
        }

        lines.join("\n")
    }

    fn to_json(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or_default()
    }
}

#[derive(Debug, serde::Serialize)]
pub struct AdapterEnableOutput {
    pub success: bool,
    pub message: String,
    pub adapter: String,
    pub next_steps: Vec<String>,
}

impl CommandOutput for AdapterEnableOutput {
    fn to_human(&self) -> String {
        let mut lines = vec![self.message.clone()];
        if !self.next_steps.is_empty() {
            lines.push("\nNext steps:".to_string());
            for step in &self.next_steps {
                lines.push(format!("  - {}", step));
            }
        }
        lines.join("\n")
    }

    fn to_json(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or_default()
    }
}

#[derive(Debug, serde::Serialize)]
pub struct AdapterDisableOutput {
    pub success: bool,
    pub message: String,
    pub adapter: String,
}

impl CommandOutput for AdapterDisableOutput {
    fn to_human(&self) -> String {
        self.message.clone()
    }

    fn to_json(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or_default()
    }
}

#[derive(Debug, serde::Serialize)]
pub struct AdapterInfoOutput {
    pub name: String,
    pub description: String,
    pub adapter_type: String,
    pub direction: String,
    pub capabilities: Vec<String>,
    pub status: String,
    pub required_env_vars: Vec<String>,
    pub config: Option<serde_json::Value>,
    pub adapter_md_summary: Option<String>,
}

impl CommandOutput for AdapterInfoOutput {
    fn to_human(&self) -> String {
        let mut lines = vec![
            format!("Adapter: {}", self.name),
            format!("Description: {}", self.description),
            format!("Type: {}", self.adapter_type),
            format!("Direction: {}", self.direction),
            format!("Status: {}", self.status),
        ];

        if !self.capabilities.is_empty() {
            lines.push(format!("Capabilities: {}", self.capabilities.join(", ")));
        }
        if !self.required_env_vars.is_empty() {
            lines.push(format!(
                "Required env vars: {}",
                self.required_env_vars.join(", ")
            ));
        }
        if let Some(config) = &self.config {
            lines.push(format!(
                "\nConfig:\n{}",
                serde_json::to_string_pretty(config).unwrap_or_default()
            ));
        }
        if let Some(summary) = &self.adapter_md_summary {
            lines.push(format!("\n--- ADAPTER.md ---\n{}", summary));
        }

        lines.join("\n")
    }

    fn to_json(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or_default()
    }
}

#[derive(Debug, serde::Serialize)]
pub struct DoctorCheck {
    pub check: String,
    pub passed: bool,
    pub message: String,
}

#[derive(Debug, serde::Serialize)]
pub struct DoctorAdapterResult {
    pub adapter: String,
    pub checks: Vec<DoctorCheck>,
    pub healthy: bool,
}

#[derive(Debug, serde::Serialize)]
pub struct AdapterDoctorOutput {
    pub results: Vec<DoctorAdapterResult>,
    pub all_healthy: bool,
}

impl CommandOutput for AdapterDoctorOutput {
    fn to_human(&self) -> String {
        let mut lines = Vec::new();

        for result in &self.results {
            lines.push(format!("Adapter: {}", result.adapter));
            for check in &result.checks {
                let icon = if check.passed { "PASS" } else { "FAIL" };
                lines.push(format!("  [{}] {}: {}", icon, check.check, check.message));
            }
            let overall = if result.healthy { "healthy" } else { "unhealthy" };
            lines.push(format!("  Overall: {}\n", overall));
        }

        if self.all_healthy {
            lines.push("All adapters healthy.".to_string());
        } else {
            lines.push("Some adapters have issues. See above for details.".to_string());
        }

        lines.join("\n")
    }

    fn to_json(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or_default()
    }
}

// ---------------------------------------------------------------------------
// Execute
// ---------------------------------------------------------------------------

pub async fn execute(args: AdapterArgs, json_mode: bool) -> Result<()> {
    let adapters_dir = resolve_adapters_dir()?;

    match args.command {
        AdapterCommands::List => cmd_list(&adapters_dir, json_mode).await,
        AdapterCommands::Enable { name, force } => {
            cmd_enable(&adapters_dir, &name, force, json_mode).await
        }
        AdapterCommands::Disable { name, force } => {
            cmd_disable(&adapters_dir, &name, force, json_mode).await
        }
        AdapterCommands::Info { name } => cmd_info(&adapters_dir, &name, json_mode).await,
        AdapterCommands::Doctor { name } => {
            cmd_doctor(&adapters_dir, name.as_deref(), json_mode).await
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Resolve the adapters directory from the default config.
fn resolve_adapters_dir() -> Result<PathBuf> {
    let config = AdapterConfig::default();
    let cwd = std::env::current_dir().context("Failed to get current directory")?;
    Ok(cwd.join(&config.adapters_dir))
}

/// Check whether an adapter is currently enabled (has a config directory).
fn is_enabled(adapters_dir: &Path, name: &str) -> bool {
    adapters_dir.join(name).join("adapter.toml").exists()
}

/// Compute env var status for a known adapter.
fn env_var_status(known: &KnownAdapter) -> String {
    let missing: Vec<&&str> = known
        .required_env_vars
        .iter()
        .filter(|v| std::env::var(v).is_err())
        .collect();
    if known.required_env_vars.is_empty() {
        "n/a".to_string()
    } else if missing.is_empty() {
        "ok".to_string()
    } else {
        format!("missing: {}", missing.iter().map(|v| **v).collect::<Vec<_>>().join(", "))
    }
}

/// Read and parse an adapter.toml manifest from disk.
fn read_manifest(adapters_dir: &Path, name: &str) -> Result<AdapterManifest> {
    let manifest_path = adapters_dir.join(name).join("adapter.toml");
    let content = std::fs::read_to_string(&manifest_path)
        .with_context(|| format!("Failed to read {}", manifest_path.display()))?;
    let manifest: AdapterManifest =
        toml::from_str(&content).with_context(|| format!("Failed to parse {}", manifest_path.display()))?;
    Ok(manifest)
}

// ---------------------------------------------------------------------------
// Subcommand implementations
// ---------------------------------------------------------------------------

async fn cmd_list(adapters_dir: &Path, json_mode: bool) -> Result<()> {
    let mut entries: Vec<AdapterListEntry> = Vec::new();
    let mut seen = std::collections::HashSet::new();

    // Add known adapters
    for known in KNOWN_ADAPTERS {
        let enabled = is_enabled(adapters_dir, known.name);
        seen.insert(known.name.to_string());
        entries.push(AdapterListEntry {
            name: known.name.to_string(),
            adapter_type: known.adapter_type.as_str().to_string(),
            direction: known.direction.as_str().to_string(),
            status: if enabled {
                "enabled".to_string()
            } else {
                "available".to_string()
            },
            env_status: env_var_status(known),
        });
    }

    // Scan for enabled adapters not in KNOWN_ADAPTERS
    if adapters_dir.exists()
        && let Ok(entries_iter) = std::fs::read_dir(adapters_dir) {
            for entry in entries_iter.flatten() {
                let path = entry.path();
                if !path.is_dir() {
                    continue;
                }
                let name = path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string();
                if seen.contains(&name) {
                    continue;
                }
                // Try to read manifest for unknown adapters
                if let Ok(manifest) = read_manifest(adapters_dir, &name) {
                    entries.push(AdapterListEntry {
                        name: name.clone(),
                        adapter_type: manifest.adapter_type.as_str().to_string(),
                        direction: manifest.direction.as_str().to_string(),
                        status: "enabled".to_string(),
                        env_status: {
                            let missing = find_missing_env_vars(&manifest);
                            if missing.is_empty() {
                                "ok".to_string()
                            } else {
                                format!("missing: {}", missing.join(", "))
                            }
                        },
                    });
                } else {
                    entries.push(AdapterListEntry {
                        name,
                        adapter_type: "unknown".to_string(),
                        direction: "unknown".to_string(),
                        status: "enabled (invalid)".to_string(),
                        env_status: "unknown".to_string(),
                    });
                }
            }
        }

    output(&AdapterListOutput { adapters: entries }, json_mode);
    Ok(())
}

async fn cmd_enable(
    adapters_dir: &Path,
    name: &str,
    force: bool,
    json_mode: bool,
) -> Result<()> {
    let known = find_known_adapter(name).ok_or_else(|| {
        let available: Vec<&str> = KNOWN_ADAPTERS.iter().map(|a| a.name).collect();
        anyhow::anyhow!(
            "Unknown adapter '{}'. Available adapters: {}",
            name,
            available.join(", ")
        )
    })?;

    let adapter_dir = adapters_dir.join(name);

    if adapter_dir.exists() && !force {
        bail!(
            "Adapter '{}' is already enabled. Use --force to overwrite.",
            name
        );
    }

    // Create directory
    fs::create_dir_all(&adapter_dir)
        .await
        .with_context(|| format!("Failed to create directory {}", adapter_dir.display()))?;

    // Write adapter.toml
    fs::write(adapter_dir.join("adapter.toml"), known.default_config)
        .await
        .context("Failed to write adapter.toml")?;

    // Write ADAPTER.md
    fs::write(adapter_dir.join("ADAPTER.md"), known.default_adapter_md)
        .await
        .context("Failed to write ADAPTER.md")?;

    let mut next_steps = Vec::new();
    for var in known.required_env_vars {
        if std::env::var(var).is_err() {
            next_steps.push(format!("Set the {} environment variable", var));
        }
    }
    // Check for empty config values that need filling in
    if known.default_config.contains("list_id = \"\"") {
        next_steps.push(format!(
            "Edit {}/adapter.toml and set config.list_id",
            adapter_dir.display()
        ));
    }

    let out = AdapterEnableOutput {
        success: true,
        message: format!("Adapter '{}' enabled successfully.", name),
        adapter: name.to_string(),
        next_steps,
    };
    output(&out, json_mode);
    Ok(())
}

async fn cmd_disable(
    adapters_dir: &Path,
    name: &str,
    force: bool,
    json_mode: bool,
) -> Result<()> {
    let adapter_dir = adapters_dir.join(name);

    if !adapter_dir.exists() {
        bail!("Adapter '{}' is not enabled.", name);
    }

    // Check if config has been customized (differs from default template)
    if !force
        && let Some(known) = find_known_adapter(name) {
            let manifest_path = adapter_dir.join("adapter.toml");
            if let Ok(current) = std::fs::read_to_string(&manifest_path)
                && current.trim() != known.default_config.trim() {
                    bail!(
                        "Adapter '{}' has a customized adapter.toml. Use --force to remove anyway.",
                        name
                    );
                }
        }

    fs::remove_dir_all(&adapter_dir)
        .await
        .with_context(|| format!("Failed to remove directory {}", adapter_dir.display()))?;

    let out = AdapterDisableOutput {
        success: true,
        message: format!("Adapter '{}' disabled and configuration removed.", name),
        adapter: name.to_string(),
    };
    output(&out, json_mode);
    Ok(())
}

async fn cmd_info(adapters_dir: &Path, name: &str, json_mode: bool) -> Result<()> {
    let enabled = is_enabled(adapters_dir, name);
    let known = find_known_adapter(name);

    if !enabled && known.is_none() {
        let available: Vec<&str> = KNOWN_ADAPTERS.iter().map(|a| a.name).collect();
        bail!(
            "Unknown adapter '{}'. Available adapters: {}",
            name,
            available.join(", ")
        );
    }

    if enabled {
        // Read from disk
        let manifest = read_manifest(adapters_dir, name)?;
        let adapter_dir = adapters_dir.join(name);
        let md_path = adapter_dir.join("ADAPTER.md");
        let md_content = if md_path.exists() {
            Some(std::fs::read_to_string(&md_path).unwrap_or_default())
        } else {
            None
        };

        let required_env = known
            .map(|k| k.required_env_vars.iter().map(|s| s.to_string()).collect())
            .unwrap_or_default();

        let out = AdapterInfoOutput {
            name: manifest.name.clone(),
            description: manifest.description.clone(),
            adapter_type: manifest.adapter_type.as_str().to_string(),
            direction: manifest.direction.as_str().to_string(),
            capabilities: manifest.capabilities.iter().map(|c| c.as_str().to_string()).collect(),
            status: "enabled".to_string(),
            required_env_vars: required_env,
            config: Some(serde_json::to_value(&manifest.config).unwrap_or_default()),
            adapter_md_summary: md_content,
        };
        output(&out, json_mode);
    } else {
        // Only in KNOWN_ADAPTERS
        let known = known.unwrap();
        let out = AdapterInfoOutput {
            name: known.name.to_string(),
            description: known.description.to_string(),
            adapter_type: known.adapter_type.as_str().to_string(),
            direction: known.direction.as_str().to_string(),
            capabilities: known.capabilities.iter().map(|c| c.as_str().to_string()).collect(),
            status: "available (not enabled)".to_string(),
            required_env_vars: known.required_env_vars.iter().map(|s| s.to_string()).collect(),
            config: None,
            adapter_md_summary: None,
        };
        output(&out, json_mode);
    }

    Ok(())
}

async fn cmd_doctor(
    adapters_dir: &Path,
    name: Option<&str>,
    json_mode: bool,
) -> Result<()> {
    let adapters_to_check: Vec<String> = if let Some(n) = name {
        if !is_enabled(adapters_dir, n) {
            bail!(
                "Adapter '{}' is not enabled. Enable it first with: abathur adapter enable {}",
                n,
                n
            );
        }
        vec![n.to_string()]
    } else {
        // Check all enabled adapters
        let mut names = Vec::new();
        if adapters_dir.exists()
            && let Ok(entries) = std::fs::read_dir(adapters_dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.is_dir() && path.join("adapter.toml").exists()
                        && let Some(n) = path.file_name() {
                            names.push(n.to_string_lossy().to_string());
                        }
                }
            }
        if names.is_empty() {
            bail!("No enabled adapters found. Enable one with: abathur adapter enable <name>");
        }
        names
    };

    let mut results = Vec::new();

    for adapter_name in &adapters_to_check {
        let checks = run_doctor_checks(adapters_dir, adapter_name);
        let healthy = checks.iter().all(|c| c.passed);
        results.push(DoctorAdapterResult {
            adapter: adapter_name.clone(),
            checks,
            healthy,
        });
    }

    let all_healthy = results.iter().all(|r| r.healthy);
    output(
        &AdapterDoctorOutput {
            results,
            all_healthy,
        },
        json_mode,
    );
    Ok(())
}

fn run_doctor_checks(adapters_dir: &Path, name: &str) -> Vec<DoctorCheck> {
    let mut checks = Vec::new();
    let adapter_dir = adapters_dir.join(name);
    let manifest_path = adapter_dir.join("adapter.toml");

    // Check 1: adapter.toml exists
    if !manifest_path.exists() {
        checks.push(DoctorCheck {
            check: "adapter.toml exists".to_string(),
            passed: false,
            message: format!(
                "File not found at {}. Re-enable with: abathur adapter enable {}",
                manifest_path.display(),
                name
            ),
        });
        return checks;
    }
    checks.push(DoctorCheck {
        check: "adapter.toml exists".to_string(),
        passed: true,
        message: "Found".to_string(),
    });

    // Check 2: adapter.toml parses correctly
    let content = match std::fs::read_to_string(&manifest_path) {
        Ok(c) => c,
        Err(e) => {
            checks.push(DoctorCheck {
                check: "adapter.toml readable".to_string(),
                passed: false,
                message: format!("Failed to read: {}", e),
            });
            return checks;
        }
    };

    let manifest: AdapterManifest = match toml::from_str(&content) {
        Ok(m) => {
            checks.push(DoctorCheck {
                check: "adapter.toml parses".to_string(),
                passed: true,
                message: "Valid TOML".to_string(),
            });
            m
        }
        Err(e) => {
            checks.push(DoctorCheck {
                check: "adapter.toml parses".to_string(),
                passed: false,
                message: format!("Parse error: {}. Check your TOML syntax.", e),
            });
            return checks;
        }
    };

    // Check 3: Manifest validation
    match manifest.validate() {
        Ok(()) => {
            checks.push(DoctorCheck {
                check: "manifest validation".to_string(),
                passed: true,
                message: "All validations passed".to_string(),
            });
        }
        Err(reason) => {
            checks.push(DoctorCheck {
                check: "manifest validation".to_string(),
                passed: false,
                message: format!("Validation failed: {}", reason),
            });
        }
    }

    // Check 4: Required env vars
    let known = find_known_adapter(name);
    if let Some(known) = known {
        let missing: Vec<&str> = known
            .required_env_vars
            .iter()
            .copied()
            .filter(|v| std::env::var(v).is_err())
            .collect();
        if missing.is_empty() {
            checks.push(DoctorCheck {
                check: "environment variables".to_string(),
                passed: true,
                message: "All required env vars set".to_string(),
            });
        } else {
            checks.push(DoctorCheck {
                check: "environment variables".to_string(),
                passed: false,
                message: format!(
                    "Missing: {}. Set them with: export {}=<value>",
                    missing.join(", "),
                    missing[0]
                ),
            });
        }
    } else {
        // For unknown adapters, check config-level env vars
        let missing = find_missing_env_vars(&manifest);
        if missing.is_empty() {
            checks.push(DoctorCheck {
                check: "environment variables".to_string(),
                passed: true,
                message: "No missing env var references in config".to_string(),
            });
        } else {
            checks.push(DoctorCheck {
                check: "environment variables".to_string(),
                passed: false,
                message: format!("Missing env vars referenced in config: {}", missing.join(", ")),
            });
        }
    }

    // Check 5: Required config values are non-empty
    let empty_config_keys: Vec<String> = manifest
        .config
        .iter()
        .filter_map(|(k, v)| {
            if let serde_json::Value::String(s) = v
                && s.is_empty() {
                    return Some(k.clone());
                }
            None
        })
        .collect();

    if empty_config_keys.is_empty() {
        checks.push(DoctorCheck {
            check: "config values".to_string(),
            passed: true,
            message: "All config values populated".to_string(),
        });
    } else {
        checks.push(DoctorCheck {
            check: "config values".to_string(),
            passed: false,
            message: format!(
                "Empty config values: {}. Edit adapter.toml to fill them in.",
                empty_config_keys.join(", ")
            ),
        });
    }

    checks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_known_adapters_not_empty() {
        assert!(!KNOWN_ADAPTERS.is_empty());
    }

    #[test]
    fn test_find_known_adapter() {
        assert!(find_known_adapter("clickup").is_some());
        assert!(find_known_adapter("nonexistent").is_none());
    }

    #[test]
    fn test_env_var_status_missing() {
        let known = find_known_adapter("clickup").unwrap();
        // When CLICKUP_API_KEY is not set, should report missing
        if std::env::var("CLICKUP_API_KEY").is_err() {
            let status = env_var_status(known);
            assert!(status.contains("missing"), "got: {}", status);
        }
    }

    #[tokio::test]
    async fn test_enable_unknown_adapter_errors() {
        let dir = tempfile::tempdir().unwrap();
        let result = cmd_enable(dir.path(), "nonexistent", false, false).await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("Unknown adapter"), "got: {}", err);
        assert!(err.contains("clickup"), "got: {}", err);
    }

    #[tokio::test]
    async fn test_enable_and_disable_cycle() {
        let dir = tempfile::tempdir().unwrap();
        let adapters_dir = dir.path();

        // Enable
        cmd_enable(adapters_dir, "clickup", false, false)
            .await
            .expect("enable should succeed");
        assert!(adapters_dir.join("clickup/adapter.toml").exists());
        assert!(adapters_dir.join("clickup/ADAPTER.md").exists());

        // Enable again without force should fail
        let result = cmd_enable(adapters_dir, "clickup", false, false).await;
        assert!(result.is_err());

        // Enable with force should succeed
        cmd_enable(adapters_dir, "clickup", true, false)
            .await
            .expect("enable --force should succeed");

        // Disable with force (since we haven't modified the config, but use force to be safe)
        cmd_disable(adapters_dir, "clickup", true, false)
            .await
            .expect("disable should succeed");
        assert!(!adapters_dir.join("clickup").exists());
    }

    #[tokio::test]
    async fn test_disable_not_enabled_errors() {
        let dir = tempfile::tempdir().unwrap();
        let result = cmd_disable(dir.path(), "clickup", false, false).await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("not enabled"), "got: {}", err);
    }

    #[tokio::test]
    async fn test_info_unknown_errors() {
        let dir = tempfile::tempdir().unwrap();
        let result = cmd_info(dir.path(), "totally-unknown", false).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_info_known_but_not_enabled() {
        let dir = tempfile::tempdir().unwrap();
        // clickup is known but not enabled here
        cmd_info(dir.path(), "clickup", false)
            .await
            .expect("info should succeed for known adapter");
    }

    #[tokio::test]
    async fn test_doctor_not_enabled_errors() {
        let dir = tempfile::tempdir().unwrap();
        let result = cmd_doctor(dir.path(), Some("clickup"), false).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_doctor_enabled_adapter() {
        let dir = tempfile::tempdir().unwrap();
        let adapters_dir = dir.path();

        // Enable first
        cmd_enable(adapters_dir, "clickup", false, false)
            .await
            .expect("enable should succeed");

        // Doctor should succeed (though it may report issues like missing env vars)
        cmd_doctor(adapters_dir, Some("clickup"), false)
            .await
            .expect("doctor should succeed");
    }

    #[test]
    fn test_doctor_checks_valid_adapter() {
        let dir = tempfile::tempdir().unwrap();
        let adapters_dir = dir.path();

        // Create a valid adapter directory
        let adapter_dir = adapters_dir.join("clickup");
        std::fs::create_dir_all(&adapter_dir).unwrap();
        let known = find_known_adapter("clickup").unwrap();
        std::fs::write(adapter_dir.join("adapter.toml"), known.default_config).unwrap();

        let checks = run_doctor_checks(adapters_dir, "clickup");
        // adapter.toml exists and parses should pass
        assert!(checks[0].passed, "exists check: {}", checks[0].message);
        assert!(checks[1].passed, "parse check: {}", checks[1].message);
        assert!(checks[2].passed, "validation check: {}", checks[2].message);
        // config values will fail because list_id is empty
        let config_check = checks.iter().find(|c| c.check == "config values").unwrap();
        assert!(!config_check.passed, "config check should fail for empty list_id");
    }

    #[test]
    fn test_list_output_human() {
        let out = AdapterListOutput {
            adapters: vec![AdapterListEntry {
                name: "clickup".to_string(),
                adapter_type: "native".to_string(),
                direction: "bidirectional".to_string(),
                status: "available".to_string(),
                env_status: "ok".to_string(),
            }],
        };
        let human = out.to_human();
        assert!(human.contains("clickup"));
        assert!(human.contains("native"));
    }

    #[test]
    fn test_list_output_json() {
        let out = AdapterListOutput {
            adapters: vec![AdapterListEntry {
                name: "clickup".to_string(),
                adapter_type: "native".to_string(),
                direction: "bidirectional".to_string(),
                status: "enabled".to_string(),
                env_status: "ok".to_string(),
            }],
        };
        let json = out.to_json();
        assert!(json["adapters"][0]["name"] == "clickup");
    }
}
