//! Tests for `abathur swarm ...`.

use super::{AssertExt, abathur_cmd, init_project, json_str, run_json};
use predicates::prelude::*;
use serde_json::Value;
use tempfile::TempDir;

#[test]
fn swarm_config_shows_defaults() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    abathur_cmd(dir)
        .args(["swarm", "config"])
        .assert()
        .success_without_warnings()
        .stdout(
            predicates::str::contains("Swarm Configuration")
                .and(predicates::str::contains("Max agents")),
        );
}

#[test]
fn swarm_config_json_output() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    let json = run_json(dir, &["swarm", "config", "--json"]);

    assert!(json.get("max_agents").is_some());
    assert!(json["max_agents"].as_u64().is_some());
    assert!(json.get("default_max_turns").is_some());
    assert!(json["default_max_turns"].as_u64().is_some());
    assert!(json.get("use_worktrees").is_some());
    assert!(json["use_worktrees"].as_bool().is_some());
    assert!(json.get("auto_retry").is_some());
    assert!(json["auto_retry"].as_bool().is_some());
    assert!(json.get("max_task_retries").is_some());
    assert!(json.get("goal_timeout_secs").is_some());
}

#[test]
fn swarm_status_when_not_running() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    abathur_cmd(dir)
        .args(["swarm", "status"])
        .assert()
        .success_without_warnings()
        .stdout(predicates::str::contains("STOPPED"));
}

#[test]
fn swarm_status_json_output() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    let json = run_json(dir, &["swarm", "status", "--json"]);

    assert_eq!(
        json["status"].as_str().unwrap(),
        "stopped",
        "Swarm should report stopped when not running"
    );
    assert!(json.get("active_goals").is_some());
    assert!(json.get("pending_tasks").is_some());
    assert!(json.get("running_tasks").is_some());
    assert!(json.get("active_worktrees").is_some());
}

#[test]
fn swarm_active_when_empty() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    abathur_cmd(dir)
        .args(["swarm", "active"])
        .assert()
        .success_without_warnings()
        .stdout(predicates::str::contains("Active Goals"));
}

#[test]
fn swarm_active_json_output() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    let json = run_json(dir, &["swarm", "active", "--json"]);

    let active_goals = json["active_goals"]
        .as_array()
        .expect("active_goals should be an array");
    assert_eq!(active_goals.len(), 0);

    let running_tasks = json["running_tasks"]
        .as_array()
        .expect("running_tasks should be an array");
    assert_eq!(running_tasks.len(), 0);

    assert!(json.get("pending_tasks").is_some());
}

#[test]
fn swarm_help_shows_usage() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();

    abathur_cmd(dir)
        .args(["swarm", "--help"])
        .assert()
        .success_without_warnings()
        .stdout(predicates::str::contains("Usage").and(predicates::str::contains("swarm")));
}

#[test]
fn swarm_stop_when_not_running() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    abathur_cmd(dir)
        .args(["swarm", "stop"])
        .assert()
        .success_without_warnings()
        .stdout(predicates::str::contains("No swarm"));
}

#[test]
fn swarm_stop_json_when_not_running() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    let json = run_json(dir, &["swarm", "stop", "--json"]);

    assert_eq!(
        json["status"].as_str().unwrap(),
        "not_running",
        "Stop should report not_running when no swarm is active"
    );
    assert!(json["message"].as_str().is_some());
}

#[test]
fn swarm_tick_runs_successfully() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    abathur_cmd(dir)
        .args(["swarm", "tick"])
        .assert()
        .success_without_warnings()
        .stdout(predicates::str::contains("Tick completed"));
}

#[test]
fn swarm_tick_json_output() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    let json = run_json(dir, &["swarm", "tick", "--json"]);

    assert!(
        json.get("active_goals").is_some(),
        "tick should report active_goals"
    );
    assert!(
        json.get("pending_tasks").is_some(),
        "tick should report pending_tasks"
    );
    assert!(
        json.get("running_tasks").is_some(),
        "tick should report running_tasks"
    );
    assert!(
        json.get("completed_tasks").is_some(),
        "tick should report completed_tasks"
    );
    assert!(
        json.get("failed_tasks").is_some(),
        "tick should report failed_tasks"
    );
    assert!(
        json.get("active_agents").is_some(),
        "tick should report active_agents"
    );

    // All numeric fields should be parseable
    assert!(json["active_goals"].as_u64().is_some());
    assert!(json["pending_tasks"].as_u64().is_some());
    assert!(json["running_tasks"].as_u64().is_some());
    assert!(json["completed_tasks"].as_u64().is_some());
    assert!(json["failed_tasks"].as_u64().is_some());
    assert!(json["active_agents"].as_u64().is_some());
}

#[test]
fn swarm_escalations_empty() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    abathur_cmd(dir)
        .args(["swarm", "escalations"])
        .assert()
        .success_without_warnings()
        .stdout(predicates::str::contains("No pending escalations"));
}

#[test]
fn swarm_escalations_json_output() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    let json = run_json(dir, &["swarm", "escalations", "--json"]);

    // The escalations command returns a JSON array directly
    let escalations = json.as_array().expect("escalations should be a JSON array");
    assert_eq!(
        escalations.len(),
        0,
        "Fresh project should have no pending escalations"
    );
}

#[test]
fn swarm_active_with_goals() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    // Create a goal first
    let create = run_json(dir, &["goal", "set", "Active Swarm Goal", "--json"]);
    assert_eq!(create["success"], true);
    let goal_id = json_str(&create["goal"], "id");

    // Now check swarm active --json
    let json = run_json(dir, &["swarm", "active", "--json"]);

    let active_goals = json["active_goals"]
        .as_array()
        .expect("active_goals should be an array");
    assert!(
        !active_goals.is_empty(),
        "Should have at least one active goal"
    );
    assert!(
        active_goals.iter().any(|g| json_str(g, "id") == goal_id),
        "Active goals should include the newly created goal"
    );
}

#[test]
fn swarm_start_dry_run_and_stop() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    // Start the swarm in background with --dry-run
    abathur_cmd(dir)
        .args(["swarm", "start", "--dry-run"])
        .assert()
        .success_without_warnings()
        .stdout(predicates::str::contains("started"));

    // Give the background process a moment to write its PID file
    std::thread::sleep(std::time::Duration::from_millis(500));

    // Verify status reports the swarm is running
    abathur_cmd(dir)
        .args(["swarm", "status"])
        .assert()
        .success_without_warnings();

    // Stop the swarm
    abathur_cmd(dir)
        .args(["swarm", "stop"])
        .assert()
        .success_without_warnings();
}

#[test]
fn swarm_start_json_dry_run() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    // Start with --json --dry-run; the background launcher prints JSON immediately
    let output = abathur_cmd(dir)
        .args(["--json", "swarm", "start", "--dry-run"])
        .assert()
        .success_without_warnings()
        .get_output()
        .stdout
        .clone();

    let json: Value =
        serde_json::from_slice(&output).expect("swarm start --json should produce valid JSON");

    // The JSON response must contain "status" and "pid" fields
    assert!(
        json.get("status").is_some(),
        "JSON output should contain a 'status' field: {}",
        json
    );
    assert!(
        json.get("pid").is_some(),
        "JSON output should contain a 'pid' field: {}",
        json
    );
    assert_eq!(
        json["status"].as_str().unwrap(),
        "started",
        "status should be 'started'"
    );

    // Give the background process a moment to start
    std::thread::sleep(std::time::Duration::from_millis(500));

    // Clean up: stop the swarm
    abathur_cmd(dir)
        .args(["swarm", "stop"])
        .assert()
        .success_without_warnings();
}

#[test]
fn swarm_start_help() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();

    abathur_cmd(dir)
        .args(["swarm", "start", "--help"])
        .assert()
        .success_without_warnings()
        .stdout(
            predicates::str::contains("Usage")
                .and(predicates::str::contains("start"))
                .and(predicates::str::contains("--dry-run"))
                .and(predicates::str::contains("--max-agents")),
        );
}

#[test]
fn swarm_respond_missing_args_fails() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    // No init needed -- clap rejects the command before db access

    abathur_cmd(dir)
        .args(["swarm", "respond"])
        .assert()
        .failure()
        .stderr(predicates::str::is_match("required|Usage").unwrap());
}

#[test]
fn swarm_respond_missing_id_fails() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();

    abathur_cmd(dir)
        .args(["swarm", "respond", "--decision", "accept"])
        .assert()
        .failure()
        .stderr(predicates::str::is_match("required|--id").unwrap());
}

#[test]
fn swarm_respond_missing_decision_fails() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();

    abathur_cmd(dir)
        .args([
            "swarm",
            "respond",
            "--id",
            "00000000-0000-0000-0000-000000000000",
        ])
        .assert()
        .failure()
        .stderr(predicates::str::is_match("required|--decision").unwrap());
}

#[test]
fn swarm_respond_help() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();

    abathur_cmd(dir)
        .args(["swarm", "respond", "--help"])
        .assert()
        .success_without_warnings()
        .stdout(
            predicates::str::contains("Usage")
                .and(predicates::str::contains("respond"))
                .and(predicates::str::contains("--id"))
                .and(predicates::str::contains("--decision")),
        );
}
