//! CLI integration tests for the abathur binary.
//! Tests each CLI command to validate interfaces function correctly.
//!
//! Submodules are organized by top-level subcommand. Shared helpers live in
//! this module and are imported via `use super::*;` from the submodule files.

use assert_cmd::Command;
use predicates::prelude::*;
use serde_json::Value;
use std::path::Path;

pub mod agent;
pub mod event;
pub mod goal;
pub mod init;
pub mod mcp;
pub mod memory;
pub mod misc;
pub mod swarm;
pub mod task;
pub mod trigger;
pub mod worktree;

// ============================================================
// Helper functions
// ============================================================

/// Extension trait to assert command succeeded without warnings on stderr.
pub trait AssertExt {
    fn success_without_warnings(self) -> Self;
}

impl AssertExt for assert_cmd::assert::Assert {
    fn success_without_warnings(self) -> Self {
        self.success()
            .stderr(predicates::str::contains("WARN").not())
    }
}

/// Build an `assert_cmd::Command` pointing at the `abathur` binary,
/// with its working directory set to `dir`.
pub fn abathur_cmd(dir: &Path) -> Command {
    let mut cmd = assert_cmd::cargo_bin_cmd!("abathur");
    cmd.current_dir(dir);
    cmd
}

/// Run `abathur init` in the given directory so that subsequent
/// commands have a database and project structure to work with.
pub fn init_project(dir: &Path) {
    abathur_cmd(dir)
        .args(["init"])
        .assert()
        .success_without_warnings();
}

/// Run a command with `--json`, assert success, and return the parsed
/// JSON value from stdout.
pub fn run_json(dir: &Path, args: &[&str]) -> Value {
    let output = abathur_cmd(dir)
        .args(args)
        .assert()
        .success_without_warnings()
        .get_output()
        .stdout
        .clone();
    serde_json::from_slice(&output)
        .unwrap_or_else(|e| panic!("Failed to parse JSON from {:?}: {}", args, e))
}

/// Extract a string field from a nested JSON path like `obj.field`.
pub fn json_str(val: &Value, key: &str) -> String {
    val.get(key)
        .and_then(|v| v.as_str())
        .unwrap_or_else(|| panic!("Missing string field '{}' in {}", key, val))
        .to_string()
}
