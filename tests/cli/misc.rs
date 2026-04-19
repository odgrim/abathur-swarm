//! Miscellaneous top-level CLI tests (global flags, help, version, unknown
//! commands) that are not specific to any one subcommand.

use super::{AssertExt, abathur_cmd, init_project};
use predicates::prelude::*;
use tempfile::TempDir;

#[test]
fn nonexistent_command_fails() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();

    abathur_cmd(dir)
        .args(["nonexistent"])
        .assert()
        .failure()
        .stderr(predicates::str::contains("error"));
}

#[test]
fn help_flag_shows_help() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();

    abathur_cmd(dir)
        .args(["--help"])
        .assert()
        .success_without_warnings()
        .stdout(predicates::str::contains("Usage").and(predicates::str::contains("abathur")));
}

#[test]
fn version_flag_shows_version() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();

    abathur_cmd(dir)
        .args(["--version"])
        .assert()
        .success_without_warnings()
        .stdout(predicates::str::contains("abathur"));
}

#[test]
fn verbose_flag_accepted() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    abathur_cmd(dir)
        .args(["-v", "goal", "list"])
        .assert()
        .success_without_warnings();
}
