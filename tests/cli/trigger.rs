//! Tests for `abathur trigger ...`.

use super::{AssertExt, abathur_cmd, init_project, json_str, run_json};
use predicates::prelude::*;
use tempfile::TempDir;

#[test]
fn trigger_seed_creates_builtin_rules() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    abathur_cmd(dir)
        .args(["trigger", "seed"])
        .assert()
        .success_without_warnings()
        .stdout(predicates::str::contains("Seeded"));
}

#[test]
fn trigger_list_after_seed() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    // Seed first
    abathur_cmd(dir)
        .args(["trigger", "seed"])
        .assert()
        .success_without_warnings();

    abathur_cmd(dir)
        .args(["trigger", "list"])
        .assert()
        .success_without_warnings()
        .stdout(predicates::str::contains("trigger rule"));
}

#[test]
fn trigger_list_json_output() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    // Seed first
    abathur_cmd(dir)
        .args(["trigger", "seed"])
        .assert()
        .success_without_warnings();

    let json = run_json(dir, &["trigger", "list", "--json"]);

    let rules = json["rules"].as_array().expect("rules should be an array");
    assert!(!rules.is_empty(), "Should have seeded rules");
    assert!(json["total"].as_u64().unwrap() > 0);

    // Verify each rule has required fields
    let first = &rules[0];
    assert!(first["id"].as_str().is_some());
    assert!(first["name"].as_str().is_some());
    assert!(first["enabled"].as_bool().is_some());
}

#[test]
fn trigger_list_enabled_only() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    abathur_cmd(dir)
        .args(["trigger", "seed"])
        .assert()
        .success_without_warnings();

    let json = run_json(dir, &["trigger", "list", "--enabled-only", "--json"]);

    let rules = json["rules"].as_array().expect("rules should be an array");
    // All built-in rules start enabled
    assert!(!rules.is_empty());
    for rule in rules {
        assert!(
            rule["enabled"].as_bool().unwrap(),
            "enabled-only filter should return only enabled rules"
        );
    }
}

#[test]
fn trigger_show_by_name() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    abathur_cmd(dir)
        .args(["trigger", "seed"])
        .assert()
        .success_without_warnings();

    // The first built-in rule is "semantic-memory-goal-eval"
    abathur_cmd(dir)
        .args(["trigger", "show", "semantic-memory-goal-eval"])
        .assert()
        .success_without_warnings()
        .stdout(predicates::str::contains("semantic-memory-goal-eval"));
}

#[test]
fn trigger_show_json_output() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    abathur_cmd(dir)
        .args(["trigger", "seed"])
        .assert()
        .success_without_warnings();

    let json = run_json(
        dir,
        &["trigger", "show", "semantic-memory-goal-eval", "--json"],
    );

    assert_eq!(json_str(&json["rule"], "name"), "semantic-memory-goal-eval");
    assert!(json["rule"]["enabled"].as_bool().is_some());
    assert!(json.get("filter").is_some());
    assert!(json.get("action").is_some());
}

#[test]
fn trigger_disable_and_enable() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    abathur_cmd(dir)
        .args(["trigger", "seed"])
        .assert()
        .success_without_warnings();

    let rule_name = "semantic-memory-goal-eval";

    // Disable the rule
    abathur_cmd(dir)
        .args(["trigger", "disable", rule_name])
        .assert()
        .success_without_warnings()
        .stdout(predicates::str::contains("disabled"));

    // Verify it shows as disabled
    let show = run_json(dir, &["trigger", "show", rule_name, "--json"]);
    assert!(!show["rule"]["enabled"].as_bool().unwrap());

    // Re-enable the rule
    abathur_cmd(dir)
        .args(["trigger", "enable", rule_name])
        .assert()
        .success_without_warnings()
        .stdout(predicates::str::contains("enabled"));

    // Verify it shows as enabled again
    let show = run_json(dir, &["trigger", "show", rule_name, "--json"]);
    assert!(show["rule"]["enabled"].as_bool().unwrap());
}

#[test]
fn trigger_delete() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    abathur_cmd(dir)
        .args(["trigger", "seed"])
        .assert()
        .success_without_warnings();

    // Get initial count
    let before = run_json(dir, &["trigger", "list", "--json"]);
    let before_count = before["total"].as_u64().unwrap();

    // Delete a rule by name
    abathur_cmd(dir)
        .args(["trigger", "delete", "semantic-memory-goal-eval"])
        .assert()
        .success_without_warnings()
        .stdout(predicates::str::contains("deleted"));

    // Verify count decreased
    let after = run_json(dir, &["trigger", "list", "--json"]);
    let after_count = after["total"].as_u64().unwrap();
    assert_eq!(after_count, before_count - 1);
}

#[test]
fn trigger_full_lifecycle() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    // 1. Seed built-in rules
    abathur_cmd(dir)
        .args(["trigger", "seed"])
        .assert()
        .success_without_warnings();

    // 2. List all rules and grab the first name
    let list = run_json(dir, &["trigger", "list", "--json"]);
    let rules = list["rules"].as_array().unwrap();
    assert!(!rules.is_empty());
    let first_name = json_str(&rules[0], "name");

    // 3. Show rule details by name
    let show = run_json(dir, &["trigger", "show", &first_name, "--json"]);
    assert_eq!(json_str(&show["rule"], "name"), first_name);

    // 4. Disable the rule
    abathur_cmd(dir)
        .args(["trigger", "disable", &first_name])
        .assert()
        .success_without_warnings();

    let show_disabled = run_json(dir, &["trigger", "show", &first_name, "--json"]);
    assert!(!show_disabled["rule"]["enabled"].as_bool().unwrap());

    // 5. Enable the rule
    abathur_cmd(dir)
        .args(["trigger", "enable", &first_name])
        .assert()
        .success_without_warnings();

    let show_enabled = run_json(dir, &["trigger", "show", &first_name, "--json"]);
    assert!(show_enabled["rule"]["enabled"].as_bool().unwrap());

    // 6. Delete the rule
    abathur_cmd(dir)
        .args(["trigger", "delete", &first_name])
        .assert()
        .success_without_warnings();

    // 7. Verify it was deleted - show should fail
    abathur_cmd(dir)
        .args(["trigger", "show", &first_name])
        .assert()
        .failure();
}

#[test]
fn trigger_seed_is_idempotent() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    // First seed
    abathur_cmd(dir)
        .args(["trigger", "seed"])
        .assert()
        .success_without_warnings();

    let first = run_json(dir, &["trigger", "list", "--json"]);
    let first_count = first["total"].as_u64().unwrap();

    // Second seed should not duplicate
    abathur_cmd(dir)
        .args(["trigger", "seed"])
        .assert()
        .success_without_warnings()
        .stdout(predicates::str::contains("Seeded 0"));

    let second = run_json(dir, &["trigger", "list", "--json"]);
    let second_count = second["total"].as_u64().unwrap();
    assert_eq!(first_count, second_count, "Seed should be idempotent");
}

#[test]
fn trigger_help_shows_usage() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();

    abathur_cmd(dir)
        .args(["trigger", "--help"])
        .assert()
        .success_without_warnings()
        .stdout(predicates::str::contains("Usage").and(predicates::str::contains("trigger")));
}

#[test]
fn trigger_without_init_auto_creates_db() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();

    assert!(!dir.join(".abathur").exists());

    abathur_cmd(dir)
        .args(["trigger", "list"])
        .assert()
        .success_without_warnings()
        .stdout(predicates::str::contains("No trigger rules found"));

    assert!(dir.join(".abathur/abathur.db").exists());
}
