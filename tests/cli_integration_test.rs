//! CLI integration tests for the abathur binary.
//!
//! The suite is split by top-level subcommand into files under `tests/cli/`.
//! Cargo's `tests/` convention produces one binary per top-level `tests/*.rs`
//! file, so this thin entry keeps all CLI integration tests in a single binary
//! while the individual tests live in per-subcommand modules.

mod cli;
