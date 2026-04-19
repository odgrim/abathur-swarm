//! Tests for `abathur goal ...`.

use super::{AssertExt, abathur_cmd, init_project, json_str, run_json};
use predicates::prelude::*;
use tempfile::TempDir;

#[test]
fn goal_set_creates_goal() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    abathur_cmd(dir)
        .args(["goal", "set", "Test Goal", "-d", "A test goal"])
        .assert()
        .success_without_warnings()
        .stdout(predicates::str::contains("Goal created"));
}

#[test]
fn goal_set_json_output() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    let json = run_json(dir, &["goal", "set", "Test Goal", "--json"]);

    assert_eq!(json["success"], true);
    let goal = &json["goal"];
    assert!(goal["id"].as_str().is_some(), "goal should have an id");
    assert_eq!(json_str(goal, "name"), "Test Goal");
    assert_eq!(json_str(goal, "status"), "active");
}

#[test]
fn goal_set_with_description_json() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    let json = run_json(
        dir,
        &[
            "goal",
            "set",
            "Described Goal",
            "-d",
            "Some description",
            "--json",
        ],
    );

    assert_eq!(json["success"], true);
    assert_eq!(json_str(&json["goal"], "name"), "Described Goal");
}

#[test]
fn goal_list_shows_goals() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    // Create a goal first
    abathur_cmd(dir)
        .args(["goal", "set", "Listed Goal"])
        .assert()
        .success_without_warnings();

    abathur_cmd(dir)
        .args(["goal", "list"])
        .assert()
        .success_without_warnings()
        .stdout(predicates::str::contains("Listed Goal"));
}

#[test]
fn goal_list_json_output() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    // Create two goals
    abathur_cmd(dir)
        .args(["goal", "set", "Goal A"])
        .assert()
        .success_without_warnings();
    abathur_cmd(dir)
        .args(["goal", "set", "Goal B"])
        .assert()
        .success_without_warnings();

    let json = run_json(dir, &["goal", "list", "--json"]);

    let goals = json["goals"].as_array().expect("goals should be an array");
    assert!(goals.len() >= 2);
    assert!(json["total"].as_u64().unwrap() >= 2);
}

#[test]
fn goal_list_empty() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    abathur_cmd(dir)
        .args(["goal", "list"])
        .assert()
        .success_without_warnings()
        .stdout(predicates::str::contains("No goals found"));
}

#[test]
fn goal_show_displays_details() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    let json = run_json(
        dir,
        &["goal", "set", "Show Goal", "-d", "Details here", "--json"],
    );
    let id = json_str(&json["goal"], "id");

    abathur_cmd(dir)
        .args(["goal", "show", &id])
        .assert()
        .success_without_warnings()
        .stdout(predicates::str::contains("Show Goal").and(predicates::str::contains(&id[..8])));
}

#[test]
fn goal_show_json_output() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    let create = run_json(dir, &["goal", "set", "Show JSON Goal", "--json"]);
    let id = json_str(&create["goal"], "id");

    let json = run_json(dir, &["goal", "show", &id, "--json"]);

    assert_eq!(json_str(&json["goal"], "id"), id);
    assert_eq!(json_str(&json["goal"], "name"), "Show JSON Goal");
    assert_eq!(json_str(&json["goal"], "status"), "active");
}

#[test]
fn goal_pause() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    let create = run_json(dir, &["goal", "set", "Pause Goal", "--json"]);
    let id = json_str(&create["goal"], "id");

    abathur_cmd(dir)
        .args(["goal", "pause", &id])
        .assert()
        .success_without_warnings()
        .stdout(predicates::str::contains("Goal paused"));
}

#[test]
fn goal_pause_json_output() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    let create = run_json(dir, &["goal", "set", "Pause JSON Goal", "--json"]);
    let id = json_str(&create["goal"], "id");

    let json = run_json(dir, &["goal", "pause", &id, "--json"]);

    assert_eq!(json["success"], true);
    assert_eq!(json_str(&json["goal"], "status"), "paused");
}

#[test]
fn goal_resume() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    let create = run_json(dir, &["goal", "set", "Resume Goal", "--json"]);
    let id = json_str(&create["goal"], "id");

    // Pause first, then resume
    abathur_cmd(dir)
        .args(["goal", "pause", &id])
        .assert()
        .success_without_warnings();

    abathur_cmd(dir)
        .args(["goal", "resume", &id])
        .assert()
        .success_without_warnings()
        .stdout(predicates::str::contains("Goal resumed"));
}

#[test]
fn goal_resume_json_output() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    let create = run_json(dir, &["goal", "set", "Resume JSON Goal", "--json"]);
    let id = json_str(&create["goal"], "id");

    // Pause then resume
    run_json(dir, &["goal", "pause", &id, "--json"]);

    let json = run_json(dir, &["goal", "resume", &id, "--json"]);

    assert_eq!(json["success"], true);
    assert_eq!(json_str(&json["goal"], "status"), "active");
}

#[test]
fn goal_retire() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    let create = run_json(dir, &["goal", "set", "Retire Goal", "--json"]);
    let id = json_str(&create["goal"], "id");

    abathur_cmd(dir)
        .args(["goal", "retire", &id])
        .assert()
        .success_without_warnings()
        .stdout(predicates::str::contains("Goal retired"));
}

#[test]
fn goal_retire_json_output() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    let create = run_json(dir, &["goal", "set", "Retire JSON Goal", "--json"]);
    let id = json_str(&create["goal"], "id");

    let json = run_json(dir, &["goal", "retire", &id, "--json"]);

    assert_eq!(json["success"], true);
    assert_eq!(json_str(&json["goal"], "status"), "retired");
}

#[test]
fn goal_full_lifecycle() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    // 1. Create the goal
    let create = run_json(
        dir,
        &["goal", "set", "Lifecycle Goal", "-d", "Full test", "--json"],
    );
    assert_eq!(create["success"], true);
    let id = json_str(&create["goal"], "id");
    assert_eq!(json_str(&create["goal"], "status"), "active");

    // 2. List should include it
    let list = run_json(dir, &["goal", "list", "--json"]);
    let goals = list["goals"].as_array().unwrap();
    assert!(goals.iter().any(|g| json_str(g, "id") == id));

    // 3. Show should return its details
    let show = run_json(dir, &["goal", "show", &id, "--json"]);
    assert_eq!(json_str(&show["goal"], "name"), "Lifecycle Goal");

    // 4. Pause the goal
    let pause = run_json(dir, &["goal", "pause", &id, "--json"]);
    assert_eq!(json_str(&pause["goal"], "status"), "paused");

    // 5. Resume the goal
    let resume = run_json(dir, &["goal", "resume", &id, "--json"]);
    assert_eq!(json_str(&resume["goal"], "status"), "active");

    // 6. Retire the goal
    let retire = run_json(dir, &["goal", "retire", &id, "--json"]);
    assert_eq!(json_str(&retire["goal"], "status"), "retired");
}

#[test]
fn goal_missing_subcommand_fails() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();

    abathur_cmd(dir)
        .args(["goal"])
        .assert()
        .failure()
        .stderr(predicates::str::is_match("subcommand|Usage").unwrap());
}

#[test]
fn goal_help_shows_goal_help() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();

    abathur_cmd(dir)
        .args(["goal", "--help"])
        .assert()
        .success_without_warnings()
        .stdout(predicates::str::contains("Usage").and(predicates::str::contains("goal")));
}

#[test]
fn goal_without_init_auto_creates_db() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();

    // No init_project(dir) call
    assert!(!dir.join(".abathur").exists());

    abathur_cmd(dir)
        .args(["goal", "list"])
        .assert()
        .success_without_warnings()
        .stdout(predicates::str::contains("No goals found"));

    // Database should have been auto-created
    assert!(dir.join(".abathur/abathur.db").exists());
}

#[test]
fn goal_list_invalid_status() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    // GoalStatus::from_str returns None for invalid values, so the filter
    // becomes None and all goals are returned (succeeds silently).
    let json = run_json(dir, &["goal", "list", "--status", "bogus", "--json"]);

    // Should succeed with an empty or full list (filter is null)
    assert!(json["goals"].as_array().is_some());
}

#[test]
fn goal_set_missing_name_fails() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();

    abathur_cmd(dir)
        .args(["goal", "set"])
        .assert()
        .failure()
        .stderr(predicates::str::is_match("required|Usage").unwrap());
}

#[test]
fn goal_show_missing_id_fails() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();

    abathur_cmd(dir)
        .args(["goal", "show"])
        .assert()
        .failure()
        .stderr(predicates::str::is_match("required|Usage").unwrap());
}

#[test]
fn goal_show_by_prefix() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    let create = run_json(dir, &["goal", "set", "Prefix Goal", "--json"]);
    let full_id = json_str(&create["goal"], "id");
    let prefix = &full_id[..8];

    let show = run_json(dir, &["goal", "show", prefix, "--json"]);

    assert_eq!(json_str(&show["goal"], "id"), full_id);
    assert_eq!(json_str(&show["goal"], "name"), "Prefix Goal");
}

#[test]
fn goal_set_with_priority_json() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    let json = run_json(
        dir,
        &[
            "goal",
            "set",
            "High Priority Goal",
            "--priority",
            "high",
            "--json",
        ],
    );

    assert_eq!(json["success"], true);
    assert_eq!(json_str(&json["goal"], "priority"), "high");
    assert_eq!(json_str(&json["goal"], "name"), "High Priority Goal");
}

#[test]
fn goal_set_with_priority_critical() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    let json = run_json(
        dir,
        &[
            "goal",
            "set",
            "Critical Goal",
            "--priority",
            "critical",
            "--json",
        ],
    );

    assert_eq!(json["success"], true);
    assert_eq!(json_str(&json["goal"], "priority"), "critical");
    assert_eq!(json_str(&json["goal"], "name"), "Critical Goal");
}

#[test]
fn goal_set_with_constraint_json() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    let json = run_json(
        dir,
        &[
            "goal",
            "set",
            "Constrained Goal",
            "-c",
            "perf:Must be fast",
            "--json",
        ],
    );

    assert_eq!(json["success"], true);
    let goal = &json["goal"];
    assert!(
        goal["constraints_count"].as_u64().unwrap() > 0,
        "Goal should have at least one constraint"
    );
}

#[test]
fn goal_set_with_parent_json() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    // Create parent goal
    let parent = run_json(dir, &["goal", "set", "Parent Goal", "--json"]);
    let parent_id = json_str(&parent["goal"], "id");

    // Create child goal with --parent
    let child = run_json(
        dir,
        &[
            "goal",
            "set",
            "Child Goal",
            "--parent",
            &parent_id,
            "--json",
        ],
    );

    assert_eq!(child["success"], true);
    assert_eq!(
        child["goal"]["parent_id"].as_str().unwrap(),
        parent_id,
        "Child goal should reference the parent"
    );
}

#[test]
fn goal_list_filter_by_status() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    // Create a goal, then pause it
    let create = run_json(dir, &["goal", "set", "Filter Status Goal", "--json"]);
    let id = json_str(&create["goal"], "id");

    run_json(dir, &["goal", "pause", &id, "--json"]);

    // Create an active goal that should NOT appear in filtered list
    abathur_cmd(dir)
        .args(["goal", "set", "Active Goal"])
        .assert()
        .success_without_warnings();

    // List with --status paused
    let list = run_json(dir, &["goal", "list", "--status", "paused", "--json"]);

    let goals = list["goals"].as_array().expect("goals should be an array");
    assert!(!goals.is_empty(), "Should find at least one paused goal");
    for goal in goals {
        assert_eq!(
            json_str(goal, "status"),
            "paused",
            "All listed goals should be paused"
        );
    }
}

#[test]
fn goal_list_filter_by_priority() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    // Create goals with different priorities
    abathur_cmd(dir)
        .args(["goal", "set", "Low Goal", "--priority", "low"])
        .assert()
        .success_without_warnings();
    abathur_cmd(dir)
        .args(["goal", "set", "High Goal", "--priority", "high"])
        .assert()
        .success_without_warnings();

    // List with --priority high
    let list = run_json(dir, &["goal", "list", "--priority", "high", "--json"]);

    let goals = list["goals"].as_array().expect("goals should be an array");
    assert!(
        !goals.is_empty(),
        "Should find at least one high-priority goal"
    );
    for goal in goals {
        assert_eq!(
            json_str(goal, "priority"),
            "high",
            "All listed goals should have high priority"
        );
    }
}

#[test]
fn goal_show_nonexistent_fails() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    abathur_cmd(dir)
        .args(["goal", "show", "00000000-0000-0000-0000-000000000000"])
        .assert()
        .failure();
}

#[test]
fn goal_set_invalid_priority_fails() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    abathur_cmd(dir)
        .args(["goal", "set", "Test", "--priority", "bogus"])
        .assert()
        .failure();
}
