#![allow(clippy::needless_borrows_for_generic_args)]

use assert_cmd::Command;
use predicates::prelude::*;

#[test]
fn test_task_submit() {
    let mut cmd = Command::cargo_bin("abathur-cli").unwrap();
    cmd.args(&[
        "task",
        "submit",
        "Test task description",
        "--agent-type",
        "rust-specialist",
        "--priority",
        "7",
    ])
    .assert()
    .success()
    .stdout(predicate::str::contains("Task submitted successfully"))
    .stdout(predicate::str::contains("Test task description"))
    .stdout(predicate::str::contains("rust-specialist"))
    .stdout(predicate::str::contains("Priority: 7"));
}

#[test]
fn test_task_submit_json() {
    let mut cmd = Command::cargo_bin("abathur-cli").unwrap();
    cmd.args(&["--json", "task", "submit", "JSON test task"])
        .assert()
        .success()
        .stdout(predicate::str::contains("{"))
        .stdout(predicate::str::contains("task_id"))
        .stdout(predicate::str::contains("JSON test task"));
}

#[test]
fn test_task_submit_with_dependencies() {
    let mut cmd = Command::cargo_bin("abathur-cli").unwrap();
    cmd.args(&[
        "task",
        "submit",
        "Dependent task",
        "--dependencies",
        "550e8400-e29b-41d4-a716-446655440000,6ba7b810-9dad-11d1-80b4-00c04fd430c8",
    ])
    .assert()
    .success()
    .stdout(predicate::str::contains("Dependencies: 2 task(s)"));
}

#[test]
fn test_task_submit_invalid_priority() {
    let mut cmd = Command::cargo_bin("abathur-cli").unwrap();
    cmd.args(&["task", "submit", "Test", "--priority", "15"])
        .assert()
        .failure();
}

#[test]
fn test_task_list() {
    let mut cmd = Command::cargo_bin("abathur-cli").unwrap();
    cmd.args(&["task", "list"]).assert().success();
    // Output will vary based on state, so just check it runs
}

#[test]
fn test_task_list_json() {
    let mut cmd = Command::cargo_bin("abathur-cli").unwrap();
    cmd.args(&["--json", "task", "list"])
        .assert()
        .success()
        .stdout(predicate::str::starts_with("["));
}

#[test]
fn test_task_list_with_status_filter() {
    let mut cmd = Command::cargo_bin("abathur-cli").unwrap();
    cmd.args(&["task", "list", "--status", "pending", "--limit", "10"])
        .assert()
        .success();
}

#[test]
fn test_task_status() {
    let mut cmd = Command::cargo_bin("abathur-cli").unwrap();
    cmd.args(&["task", "status"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Queue Status"));
}

#[test]
fn test_task_status_json() {
    let mut cmd = Command::cargo_bin("abathur-cli").unwrap();
    cmd.args(&["--json", "task", "status"])
        .assert()
        .success()
        .stdout(predicate::str::contains("{"))
        .stdout(predicate::str::contains("total"));
}

#[test]
fn test_global_verbose_flag() {
    let mut cmd = Command::cargo_bin("abathur-cli").unwrap();
    cmd.args(&["-v", "task", "status"]).assert().success();
}

#[test]
fn test_global_config_flag() {
    let mut cmd = Command::cargo_bin("abathur-cli").unwrap();
    cmd.args(&["--config", "/tmp/test-config.yaml", "task", "status"])
        .assert()
        .success();
}

#[test]
fn test_help_command() {
    let mut cmd = Command::cargo_bin("abathur-cli").unwrap();
    cmd.args(&["--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("AI agent orchestration system"));
}

#[test]
fn test_task_help() {
    let mut cmd = Command::cargo_bin("abathur-cli").unwrap();
    cmd.args(&["task", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Task management commands"));
}

#[test]
fn test_task_submit_help() {
    let mut cmd = Command::cargo_bin("abathur-cli").unwrap();
    cmd.args(&["task", "submit", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Submit a new task"))
        .stdout(predicate::str::contains("--agent-type"))
        .stdout(predicate::str::contains("--priority"));
}
