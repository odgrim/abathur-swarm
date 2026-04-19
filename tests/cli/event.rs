//! Tests for `abathur event ...`.

use super::{AssertExt, abathur_cmd, init_project, run_json};
use predicates::prelude::*;
use tempfile::TempDir;

#[test]
fn event_stats_shows_statistics() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    abathur_cmd(dir)
        .args(["event", "stats"])
        .assert()
        .success_without_warnings()
        .stdout(predicates::str::contains("Event Store Statistics"));
}

#[test]
fn event_stats_json_output() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    let json = run_json(dir, &["event", "stats", "--json"]);

    assert!(json.get("total_events").is_some());
    assert!(json["total_events"].as_u64().is_some());
}

#[test]
fn event_list_empty() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    abathur_cmd(dir)
        .args(["event", "list"])
        .assert()
        .success_without_warnings();
}

#[test]
fn event_list_json_output() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    let json = run_json(dir, &["event", "list", "--json"]);

    assert!(json.get("events").is_some());
    let events = json["events"]
        .as_array()
        .expect("events should be an array");
    // Freshly initialized project may have zero events
    assert!(json["total"].as_u64().is_some());
    let _ = events; // used for type assertion above
}

#[test]
fn event_list_with_limit() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    // Create some tasks to generate events
    abathur_cmd(dir)
        .args(["task", "submit", "Event test 1"])
        .assert()
        .success_without_warnings();
    abathur_cmd(dir)
        .args(["task", "submit", "Event test 2"])
        .assert()
        .success_without_warnings();

    let json = run_json(dir, &["event", "list", "-l", "5", "--json"]);

    let events = json["events"]
        .as_array()
        .expect("events should be an array");
    assert!(events.len() <= 5, "Should respect the limit");
}

#[test]
fn event_gaps_runs_successfully() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    // On a fresh DB the gaps command may report a gap at sequence 0 or find none;
    // we just verify it executes successfully.
    abathur_cmd(dir)
        .args(["event", "gaps"])
        .assert()
        .success_without_warnings();
}

#[test]
fn event_dlq_list_empty() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    abathur_cmd(dir)
        .args(["event", "dlq", "list"])
        .assert()
        .success_without_warnings()
        .stdout(predicates::str::contains("No dead letter entries found"));
}

#[test]
fn event_dlq_list_json_output() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    let json = run_json(dir, &["event", "dlq", "list", "--json"]);

    let entries = json["entries"]
        .as_array()
        .expect("entries should be an array");
    assert_eq!(entries.len(), 0);
    assert_eq!(json["total"].as_u64().unwrap(), 0);
}

#[test]
fn event_dlq_purge_with_defaults() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    abathur_cmd(dir)
        .args(["event", "dlq", "purge"])
        .assert()
        .success_without_warnings()
        .stdout(predicates::str::contains("Purged"));
}

#[test]
fn event_dlq_purge_json_output() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    let json = run_json(dir, &["event", "dlq", "purge", "--json"]);

    assert!(json.get("message").is_some());
    assert!(json.get("count").is_some());
    assert_eq!(json["count"].as_u64().unwrap(), 0);
}

#[test]
fn event_help_shows_usage() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();

    abathur_cmd(dir)
        .args(["event", "--help"])
        .assert()
        .success_without_warnings()
        .stdout(predicates::str::contains("Usage").and(predicates::str::contains("event")));
}

#[test]
fn event_without_init_auto_creates_db() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();

    assert!(!dir.join(".abathur").exists());

    abathur_cmd(dir)
        .args(["event", "stats"])
        .assert()
        .success_without_warnings()
        .stdout(predicates::str::contains("Event Store Statistics"));

    assert!(dir.join(".abathur/abathur.db").exists());
}

#[test]
fn event_list_with_category_task() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    // Create some tasks to potentially generate task events
    abathur_cmd(dir)
        .args([
            "task",
            "submit",
            "Category filter test 1",
            "-t",
            "Cat Task 1",
        ])
        .assert()
        .success_without_warnings();
    abathur_cmd(dir)
        .args([
            "task",
            "submit",
            "Category filter test 2",
            "-t",
            "Cat Task 2",
        ])
        .assert()
        .success_without_warnings();

    let json = run_json(dir, &["event", "list", "-c", "task", "--json"]);

    // The response should have the expected structure regardless of event count
    let events = json["events"]
        .as_array()
        .expect("events should be an array");
    assert!(json["total"].as_u64().is_some());

    // If any events were returned, verify they have the correct category
    for event in events {
        assert_eq!(
            event["category"].as_str().unwrap().to_lowercase(),
            "task",
            "Filtered events should all be in the task category"
        );
    }
}

#[test]
fn event_gaps_json_output() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    let json = run_json(dir, &["event", "gaps", "--json"]);

    assert!(json.get("gaps").is_some(), "gaps field should exist");
    assert!(
        json.get("total_gaps").is_some(),
        "total_gaps field should exist"
    );
    assert!(
        json.get("scan_from").is_some(),
        "scan_from field should exist"
    );
    assert!(json.get("scan_to").is_some(), "scan_to field should exist");

    // gaps should be an array
    let gaps = json["gaps"].as_array().expect("gaps should be an array");
    let _ = gaps; // type assertion

    // total_gaps should be numeric
    assert!(json["total_gaps"].as_u64().is_some());
    assert!(json["scan_from"].as_u64().is_some());
    assert!(json["scan_to"].as_u64().is_some());
}

#[test]
fn event_gaps_with_window() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    abathur_cmd(dir)
        .args(["event", "gaps", "--window", "100"])
        .assert()
        .success_without_warnings();
}

#[test]
fn event_stats_after_operations() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    // Create a goal and a task to generate events
    abathur_cmd(dir)
        .args(["goal", "set", "Stats Goal", "-d", "For event stats"])
        .assert()
        .success_without_warnings();
    abathur_cmd(dir)
        .args(["task", "submit", "Stats task prompt", "-t", "Stats Task"])
        .assert()
        .success_without_warnings();

    let json = run_json(dir, &["event", "stats", "--json"]);

    assert!(json.get("total_events").is_some());
    assert!(json["total_events"].as_u64().is_some());
    // After operations, total_events should be at least 0 (events may or may not
    // be emitted inline depending on configuration, but the field must exist)
}

#[test]
fn event_dlq_retry_all_empty() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    let json = run_json(dir, &["event", "dlq", "retry-all", "--json"]);

    assert_eq!(
        json["count"].as_u64().unwrap(),
        0,
        "retry-all on empty DLQ should report 0 resolved entries"
    );
    assert!(json["message"].as_str().is_some());
}

#[test]
fn event_dlq_purge_with_older_than() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    let json = run_json(
        dir,
        &["event", "dlq", "purge", "--older-than", "24h", "--json"],
    );

    assert!(json.get("count").is_some(), "purge should report a count");
    assert_eq!(
        json["count"].as_u64().unwrap(),
        0,
        "purge on empty DLQ should report 0"
    );
    assert!(
        json.get("message").is_some(),
        "purge should include a message"
    );
}

#[test]
fn event_dlq_list_with_handler_filter() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    let json = run_json(
        dir,
        &["event", "dlq", "list", "--handler", "nonexistent", "--json"],
    );

    let entries = json["entries"]
        .as_array()
        .expect("entries should be an array");
    assert_eq!(
        entries.len(),
        0,
        "Filtering by nonexistent handler should return empty"
    );
    assert_eq!(json["total"].as_u64().unwrap(), 0);
}

#[test]
fn event_dlq_list_with_limit() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    let json = run_json(dir, &["event", "dlq", "list", "--limit", "5", "--json"]);

    assert!(
        json.get("total").is_some(),
        "DLQ list with limit should include total field"
    );
    assert!(json["total"].as_u64().is_some());

    let entries = json["entries"]
        .as_array()
        .expect("entries should be an array");
    assert!(entries.len() <= 5, "Should respect the limit of 5");
}

#[test]
fn event_dlq_retry_nonexistent_succeeds_silently() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    // The DLQ retry command resolves (marks as resolved) an entry by ID.
    // When the ID doesn't exist, the SQL update affects 0 rows but doesn't error.
    abathur_cmd(dir)
        .args([
            "event",
            "dlq",
            "retry",
            "00000000-0000-0000-0000-000000000000",
        ])
        .assert()
        .success_without_warnings()
        .stdout(predicates::str::contains("Resolved DLQ entry"));
}

#[test]
fn event_dlq_retry_json_output() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    let json = run_json(
        dir,
        &[
            "event",
            "dlq",
            "retry",
            "00000000-0000-0000-0000-000000000000",
            "--json",
        ],
    );

    assert_eq!(json["count"].as_u64().unwrap(), 1);
    assert!(json["message"].as_str().unwrap().contains("Resolved"));
}

#[test]
fn event_dlq_retry_all_json_output() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    // On empty DLQ, retry-all should succeed and output valid JSON
    let json = run_json(dir, &["event", "dlq", "retry-all", "--json"]);

    assert_eq!(
        json["count"].as_u64().unwrap(),
        0,
        "retry-all on empty DLQ should report count: 0"
    );
    let msg = json["message"]
        .as_str()
        .expect("should include a message field");
    assert!(
        msg.contains("0"),
        "message should mention 0 resolved entries, got: {}",
        msg
    );
}

#[test]
fn event_dlq_retry_all_with_handler_filter() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    // retry-all with a handler filter on empty DLQ should succeed with count=0
    let json = run_json(
        dir,
        &[
            "event",
            "dlq",
            "retry-all",
            "--handler",
            "some-handler",
            "--json",
        ],
    );

    assert_eq!(
        json["count"].as_u64().unwrap(),
        0,
        "retry-all with handler filter on empty DLQ should report count: 0"
    );
    assert!(json["message"].as_str().is_some());
}

#[test]
fn event_dlq_retry_all_human_output() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    // Human-readable output should mention resolved entries
    abathur_cmd(dir)
        .args(["event", "dlq", "retry-all"])
        .assert()
        .success_without_warnings()
        .stdout(predicates::str::contains("Resolved 0 DLQ entries"));
}

#[test]
fn event_dlq_retry_missing_id_fails() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();

    // Missing the positional id argument should be rejected by clap
    abathur_cmd(dir)
        .args(["event", "dlq", "retry"])
        .assert()
        .failure()
        .stderr(predicates::str::is_match("required|Usage").unwrap());
}

#[test]
fn event_dlq_retry_help_shows_usage() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();

    abathur_cmd(dir)
        .args(["event", "dlq", "retry", "--help"])
        .assert()
        .success_without_warnings()
        .stdout(predicates::str::contains("Usage").and(predicates::str::contains("retry")));
}

#[test]
fn event_dlq_retry_all_help_shows_usage() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();

    abathur_cmd(dir)
        .args(["event", "dlq", "retry-all", "--help"])
        .assert()
        .success_without_warnings()
        .stdout(predicates::str::contains("Usage").and(predicates::str::contains("retry-all")));
}
