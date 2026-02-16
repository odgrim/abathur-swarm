//! CLI integration tests for the abathur binary.
//! Tests each CLI command to validate interfaces function correctly.

use assert_cmd::Command;
use predicates::prelude::*;
use serde_json::Value;
use std::path::Path;
use tempfile::TempDir;

// ============================================================
// Helper functions
// ============================================================

/// Extension trait to assert command succeeded without warnings on stderr.
trait AssertExt {
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
fn abathur_cmd(dir: &Path) -> Command {
    let mut cmd = assert_cmd::cargo_bin_cmd!("abathur");
    cmd.current_dir(dir);
    cmd
}

/// Run `abathur init` in the given directory so that subsequent
/// commands have a database and project structure to work with.
fn init_project(dir: &Path) {
    abathur_cmd(dir)
        .args(["init"])
        .assert()
        .success_without_warnings();
}

/// Run a command with `--json`, assert success, and return the parsed
/// JSON value from stdout.
fn run_json(dir: &Path, args: &[&str]) -> Value {
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
fn json_str(val: &Value, key: &str) -> String {
    val.get(key)
        .and_then(|v| v.as_str())
        .unwrap_or_else(|| panic!("Missing string field '{}' in {}", key, val))
        .to_string()
}

// ============================================================
// Init command tests
// ============================================================

#[test]
fn init_creates_project_structure() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();

    abathur_cmd(dir)
        .args(["init"])
        .assert()
        .success_without_warnings()
        .stdout(predicates::str::contains("initialized successfully"));

    // Verify the expected directories and files were created
    assert!(dir.join(".abathur").is_dir());
    assert!(dir.join(".abathur/abathur.db").exists());
    assert!(dir.join(".claude").is_dir());
}

#[test]
fn init_already_initialized_without_force() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();

    // First init
    init_project(dir);

    // Second init without --force
    abathur_cmd(dir)
        .args(["init"])
        .assert()
        .success_without_warnings()
        .stdout(predicates::str::contains("already initialized"));
}

#[test]
fn init_force_reinitializes() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();

    init_project(dir);

    abathur_cmd(dir)
        .args(["init", "--force"])
        .assert()
        .success_without_warnings()
        .stdout(predicates::str::contains("reinitialized successfully"));

    // Structure should still be intact
    assert!(dir.join(".abathur/abathur.db").exists());
}

#[test]
fn init_json_output() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();

    let json = run_json(dir, &["init", "--json"]);

    assert_eq!(json["success"], true);
    assert!(json["message"].as_str().unwrap().contains("initialized"));
    assert_eq!(json["database_initialized"], true);
}

#[test]
fn init_json_already_initialized() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();

    init_project(dir);

    let json = run_json(dir, &["init", "--json"]);

    assert_eq!(json["success"], false);
    assert!(json["message"].as_str().unwrap().contains("already initialized"));
}

// ============================================================
// Goal command tests
// ============================================================

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
        &["goal", "set", "Described Goal", "-d", "Some description", "--json"],
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

    let json = run_json(dir, &["goal", "set", "Show Goal", "-d", "Details here", "--json"]);
    let id = json_str(&json["goal"], "id");

    abathur_cmd(dir)
        .args(["goal", "show", &id])
        .assert()
        .success_without_warnings()
        .stdout(
            predicates::str::contains("Show Goal")
                .and(predicates::str::contains(&id[..8])),
        );
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
    let create = run_json(dir, &["goal", "set", "Lifecycle Goal", "-d", "Full test", "--json"]);
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

// ============================================================
// Task command tests
// ============================================================

#[test]
fn task_submit_creates_task() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    abathur_cmd(dir)
        .args(["task", "submit", "Do something", "-t", "Test task"])
        .assert()
        .success_without_warnings()
        .stdout(predicates::str::contains("Task submitted"));
}

#[test]
fn task_submit_json_output() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    let json = run_json(dir, &["task", "submit", "Do something", "--json"]);

    assert_eq!(json["success"], true);
    let task = &json["task"];
    assert!(task["id"].as_str().is_some(), "task should have an id");
    assert!(task["status"].as_str().is_some(), "task should have a status");
}

#[test]
fn task_submit_with_title_json() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    let json = run_json(
        dir,
        &["task", "submit", "Do something specific", "-t", "My Title", "--json"],
    );

    assert_eq!(json["success"], true);
    assert_eq!(json_str(&json["task"], "title"), "My Title");
}

#[test]
fn task_list_shows_tasks() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    abathur_cmd(dir)
        .args(["task", "submit", "Listed task prompt", "-t", "Listed Task"])
        .assert()
        .success_without_warnings();

    abathur_cmd(dir)
        .args(["task", "list"])
        .assert()
        .success_without_warnings()
        .stdout(predicates::str::contains("Listed Task"));
}

#[test]
fn task_list_json_output() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    abathur_cmd(dir)
        .args(["task", "submit", "Task A prompt", "-t", "Task A"])
        .assert()
        .success_without_warnings();
    abathur_cmd(dir)
        .args(["task", "submit", "Task B prompt", "-t", "Task B"])
        .assert()
        .success_without_warnings();

    let json = run_json(dir, &["task", "list", "--json"]);

    let tasks = json["tasks"].as_array().expect("tasks should be an array");
    assert!(tasks.len() >= 2);
    assert!(json["total"].as_u64().unwrap() >= 2);
}

#[test]
fn task_list_empty() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    abathur_cmd(dir)
        .args(["task", "list"])
        .assert()
        .success_without_warnings()
        .stdout(predicates::str::contains("No tasks found"));
}

#[test]
fn task_show_displays_details() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    let json = run_json(
        dir,
        &["task", "submit", "Show task prompt", "-t", "Show Task", "--json"],
    );
    let id = json_str(&json["task"], "id");

    abathur_cmd(dir)
        .args(["task", "show", &id])
        .assert()
        .success_without_warnings()
        .stdout(
            predicates::str::contains("Show Task")
                .and(predicates::str::contains(&id[..8])),
        );
}

#[test]
fn task_show_json_output() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    let create = run_json(
        dir,
        &["task", "submit", "Show JSON prompt", "-t", "Show JSON Task", "--json"],
    );
    let id = json_str(&create["task"], "id");

    let json = run_json(dir, &["task", "show", &id, "--json"]);

    assert_eq!(json_str(&json["task"], "id"), id);
    assert_eq!(json_str(&json["task"], "title"), "Show JSON Task");
}

#[test]
fn task_cancel() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    let create = run_json(
        dir,
        &["task", "submit", "Cancel me", "-t", "Cancel Task", "--json"],
    );
    let id = json_str(&create["task"], "id");

    abathur_cmd(dir)
        .args(["task", "cancel", &id])
        .assert()
        .success_without_warnings()
        .stdout(predicates::str::contains("Task canceled"));
}

#[test]
fn task_cancel_json_output() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    let create = run_json(
        dir,
        &["task", "submit", "Cancel me JSON", "-t", "Cancel JSON", "--json"],
    );
    let id = json_str(&create["task"], "id");

    let json = run_json(dir, &["task", "cancel", &id, "--json"]);

    assert_eq!(json["success"], true);
    assert_eq!(json_str(&json["task"], "status"), "canceled");
}

#[test]
fn task_status_summary() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    abathur_cmd(dir)
        .args(["task", "status"])
        .assert()
        .success_without_warnings()
        .stdout(predicates::str::contains("Task Status Summary"));
}

#[test]
fn task_status_json_output() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    // Submit a task so counts are nonzero
    abathur_cmd(dir)
        .args(["task", "submit", "Status check", "-t", "Status Task"])
        .assert()
        .success_without_warnings();

    let json = run_json(dir, &["task", "status", "--json"]);

    // The status summary should have numeric fields
    assert!(json["total"].as_u64().is_some());
    assert!(json["total"].as_u64().unwrap() >= 1);
    assert!(json.get("pending").is_some() || json.get("ready").is_some());
}

#[test]
fn task_status_empty_json() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    let json = run_json(dir, &["task", "status", "--json"]);

    assert_eq!(json["total"].as_u64().unwrap(), 0);
}

#[test]
fn task_full_lifecycle() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    // 1. Submit a task
    let create = run_json(
        dir,
        &["task", "submit", "Lifecycle prompt", "-t", "Lifecycle Task", "--json"],
    );
    assert_eq!(create["success"], true);
    let id = json_str(&create["task"], "id");

    // 2. List should include it
    let list = run_json(dir, &["task", "list", "--json"]);
    let tasks = list["tasks"].as_array().unwrap();
    assert!(tasks.iter().any(|t| json_str(t, "id") == id));

    // 3. Show should return its details
    let show = run_json(dir, &["task", "show", &id, "--json"]);
    assert_eq!(json_str(&show["task"], "id"), id);
    assert_eq!(json_str(&show["task"], "title"), "Lifecycle Task");
    assert_eq!(show["description"].as_str().unwrap(), "Lifecycle prompt");

    // 4. Cancel the task
    let cancel = run_json(dir, &["task", "cancel", &id, "--json"]);
    assert_eq!(cancel["success"], true);
    assert_eq!(json_str(&cancel["task"], "status"), "canceled");

    // 5. Status should reflect the canceled task
    let status = run_json(dir, &["task", "status", "--json"]);
    assert!(status["canceled"].as_u64().unwrap() >= 1);
}

// ============================================================
// Worktree command tests
// ============================================================

#[test]
fn worktree_list_empty() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    abathur_cmd(dir)
        .args(["worktree", "list"])
        .assert()
        .success_without_warnings()
        .stdout(predicates::str::contains("No worktrees found"));
}

#[test]
fn worktree_list_json_output() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    let json = run_json(dir, &["worktree", "list", "--json"]);

    let worktrees = json["worktrees"]
        .as_array()
        .expect("worktrees should be an array");
    assert_eq!(worktrees.len(), 0);
    assert_eq!(json["total"].as_u64().unwrap(), 0);
}

#[test]
fn worktree_stats_shows_statistics() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    abathur_cmd(dir)
        .args(["worktree", "stats"])
        .assert()
        .success_without_warnings()
        .stdout(predicates::str::contains("Worktree Statistics"));
}

#[test]
fn worktree_stats_json_output() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    let json = run_json(dir, &["worktree", "stats", "--json"]);

    assert_eq!(json["total"].as_u64().unwrap(), 0);
    assert_eq!(json["active"].as_u64().unwrap(), 0);
    assert_eq!(json["creating"].as_u64().unwrap(), 0);
    assert_eq!(json["completed"].as_u64().unwrap(), 0);
    assert_eq!(json["merging"].as_u64().unwrap(), 0);
    assert_eq!(json["merged"].as_u64().unwrap(), 0);
    assert_eq!(json["failed"].as_u64().unwrap(), 0);
    assert_eq!(json["removed"].as_u64().unwrap(), 0);
}

#[test]
fn worktree_sync_completes() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    // worktree sync requires a git repo
    std::process::Command::new("git")
        .args(["init"])
        .current_dir(dir)
        .output()
        .expect("git init failed");
    init_project(dir);

    abathur_cmd(dir)
        .args(["worktree", "sync"])
        .assert()
        .success_without_warnings()
        .stdout(predicates::str::contains("Sync complete"));
}

#[test]
fn worktree_sync_json_output() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    // worktree sync requires a git repo
    std::process::Command::new("git")
        .args(["init"])
        .current_dir(dir)
        .output()
        .expect("git init failed");
    init_project(dir);

    let json = run_json(dir, &["worktree", "sync", "--json"]);

    assert!(json.get("activated").is_some());
    assert!(json.get("marked_removed").is_some());
    assert_eq!(json["activated"].as_u64().unwrap(), 0);
    assert_eq!(json["marked_removed"].as_u64().unwrap(), 0);
}

#[test]
fn worktree_cleanup_all_when_empty() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    abathur_cmd(dir)
        .args(["worktree", "cleanup-all"])
        .assert()
        .success_without_warnings()
        .stdout(predicates::str::contains("Cleaned up 0 worktree(s)"));
}

// ============================================================
// Trigger command tests
// ============================================================

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
        .stdout(predicates::str::contains("trigger rule(s)"));
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

    let rules = json["rules"]
        .as_array()
        .expect("rules should be an array");
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

    let rules = json["rules"]
        .as_array()
        .expect("rules should be an array");
    // All built-in rules start enabled
    assert!(!rules.is_empty());
    for rule in rules {
        assert_eq!(
            rule["enabled"].as_bool().unwrap(),
            true,
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

    assert_eq!(
        json_str(&json["rule"], "name"),
        "semantic-memory-goal-eval"
    );
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
    assert_eq!(show["rule"]["enabled"].as_bool().unwrap(), false);

    // Re-enable the rule
    abathur_cmd(dir)
        .args(["trigger", "enable", rule_name])
        .assert()
        .success_without_warnings()
        .stdout(predicates::str::contains("enabled"));

    // Verify it shows as enabled again
    let show = run_json(dir, &["trigger", "show", rule_name, "--json"]);
    assert_eq!(show["rule"]["enabled"].as_bool().unwrap(), true);
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
    assert_eq!(show_disabled["rule"]["enabled"].as_bool().unwrap(), false);

    // 5. Enable the rule
    abathur_cmd(dir)
        .args(["trigger", "enable", &first_name])
        .assert()
        .success_without_warnings();

    let show_enabled = run_json(dir, &["trigger", "show", &first_name, "--json"]);
    assert_eq!(show_enabled["rule"]["enabled"].as_bool().unwrap(), true);

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

// ============================================================
// Event command tests
// ============================================================

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

// ============================================================
// Swarm command tests
// ============================================================

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

// ============================================================
// MCP command tests
// ============================================================

#[test]
fn mcp_status_shows_servers() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    abathur_cmd(dir)
        .args(["mcp", "status"])
        .assert()
        .success_without_warnings()
        .stdout(
            predicates::str::contains("MCP Server Status")
                .and(predicates::str::contains("STOPPED")),
        );
}

#[test]
fn mcp_status_json_output() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    let json = run_json(dir, &["mcp", "status", "--json"]);

    let servers = json["servers"]
        .as_array()
        .expect("servers should be an array");
    assert!(!servers.is_empty(), "Should list at least one server");

    // Each server should have required fields
    for server in servers {
        assert!(server["name"].as_str().is_some());
        assert!(server["port"].as_u64().is_some());
        assert!(server["running"].as_bool().is_some());
        // All servers should be stopped in a fresh project
        assert_eq!(
            server["running"].as_bool().unwrap(),
            false,
            "Server {} should be stopped",
            server["name"]
        );
    }
}

// ============================================================
// Clap verification tests
// ============================================================

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
fn task_submit_missing_prompt_fails() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    // No init needed - clap rejects the command before db access

    abathur_cmd(dir)
        .args(["task", "submit"])
        .assert()
        .failure()
        .stderr(predicates::str::is_match("required|Usage").unwrap());
}

#[test]
fn help_flag_shows_help() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();

    abathur_cmd(dir)
        .args(["--help"])
        .assert()
        .success_without_warnings()
        .stdout(
            predicates::str::contains("Usage")
                .and(predicates::str::contains("abathur")),
        );
}

#[test]
fn goal_help_shows_goal_help() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();

    abathur_cmd(dir)
        .args(["goal", "--help"])
        .assert()
        .success_without_warnings()
        .stdout(
            predicates::str::contains("Usage")
                .and(predicates::str::contains("goal")),
        );
}

// ============================================================
// Memory command tests
// ============================================================

#[test]
fn memory_store_creates_memory() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    abathur_cmd(dir)
        .args(["memory", "store", "test-key", "test content"])
        .assert()
        .success_without_warnings()
        .stdout(predicates::str::contains("Memory stored"));
}

#[test]
fn memory_store_json_output() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    let json = run_json(dir, &["memory", "store", "test-key", "test content", "--json"]);

    assert_eq!(json["success"], true);
    let memory = &json["memory"];
    assert!(memory["id"].as_str().is_some(), "memory should have an id");
    assert_eq!(json_str(memory, "key"), "test-key");
    assert_eq!(json_str(memory, "namespace"), "default");
    assert_eq!(json_str(memory, "tier"), "working");
    assert_eq!(json_str(memory, "memory_type"), "fact");
}

#[test]
fn memory_recall_by_id() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    // Store a memory and get its ID
    let store = run_json(dir, &["memory", "store", "recall-key", "recall content", "--json"]);
    let id = json_str(&store["memory"], "id");

    // Recall by ID
    let recall = run_json(dir, &["memory", "recall", &id, "--json"]);

    assert_eq!(json_str(&recall["memory"], "id"), id);
    assert_eq!(json_str(&recall["memory"], "key"), "recall-key");
    assert_eq!(recall["content"].as_str().unwrap(), "recall content");
}

#[test]
fn memory_recall_by_key() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    // Store a memory
    let store = run_json(dir, &["memory", "store", "key-recall", "key recall content", "--json"]);
    let id = json_str(&store["memory"], "id");

    // Recall by key with namespace
    let recall = run_json(dir, &["memory", "recall", "key-recall", "-n", "default", "--json"]);

    assert_eq!(json_str(&recall["memory"], "id"), id);
    assert_eq!(json_str(&recall["memory"], "key"), "key-recall");
    assert_eq!(recall["content"].as_str().unwrap(), "key recall content");
}

#[test]
fn memory_search_finds_memories() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    // Store a memory with distinctive content
    abathur_cmd(dir)
        .args(["memory", "store", "searchable-key", "unique searchable content"])
        .assert()
        .success_without_warnings();

    // Search for it
    let json = run_json(dir, &["memory", "search", "searchable", "--json"]);

    let memories = json["memories"].as_array().expect("memories should be an array");
    assert!(!memories.is_empty(), "Search should find at least one memory");
    assert!(
        memories.iter().any(|m| json_str(m, "key") == "searchable-key"),
        "Search results should include the stored memory"
    );
}

#[test]
fn memory_list_shows_memories() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    // Store a memory
    abathur_cmd(dir)
        .args(["memory", "store", "listed-key", "listed content"])
        .assert()
        .success_without_warnings();

    abathur_cmd(dir)
        .args(["memory", "list"])
        .assert()
        .success_without_warnings()
        .stdout(predicates::str::contains("listed-key"));
}

#[test]
fn memory_list_json_output() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    // Store two memories
    abathur_cmd(dir)
        .args(["memory", "store", "list-key-a", "content a"])
        .assert()
        .success_without_warnings();
    abathur_cmd(dir)
        .args(["memory", "store", "list-key-b", "content b"])
        .assert()
        .success_without_warnings();

    let json = run_json(dir, &["memory", "list", "--json"]);

    let memories = json["memories"].as_array().expect("memories should be an array");
    assert!(memories.len() >= 2);
    assert!(json["total"].as_u64().unwrap() >= 2);
}

#[test]
fn memory_list_filter_by_namespace() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    // Store memories in different namespaces
    abathur_cmd(dir)
        .args(["memory", "store", "ns-key", "ns content", "-n", "custom-ns"])
        .assert()
        .success_without_warnings();
    abathur_cmd(dir)
        .args(["memory", "store", "default-key", "default content"])
        .assert()
        .success_without_warnings();

    // Filter by custom namespace
    let json = run_json(dir, &["memory", "list", "-n", "custom-ns", "--json"]);

    let memories = json["memories"].as_array().expect("memories should be an array");
    assert_eq!(memories.len(), 1, "Should only find memory in custom-ns");
    assert_eq!(json_str(&memories[0], "namespace"), "custom-ns");
}

#[test]
fn memory_list_empty() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    abathur_cmd(dir)
        .args(["memory", "list"])
        .assert()
        .success_without_warnings()
        .stdout(predicates::str::contains("No memories found"));
}

#[test]
fn memory_forget_deletes_memory() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    // Store a memory
    let store = run_json(dir, &["memory", "store", "forget-key", "forget content", "--json"]);
    let id = json_str(&store["memory"], "id");

    // Forget it
    let forget = run_json(dir, &["memory", "forget", &id, "--json"]);

    assert_eq!(forget["success"], true);
    assert!(
        forget["message"].as_str().unwrap().contains("deleted"),
        "Message should confirm deletion"
    );

    // Verify it is gone from the list
    let list = run_json(dir, &["memory", "list", "--json"]);
    let memories = list["memories"].as_array().unwrap();
    assert!(
        !memories.iter().any(|m| json_str(m, "id") == id),
        "Deleted memory should not appear in list"
    );
}

#[test]
fn memory_prune_runs_maintenance() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    abathur_cmd(dir)
        .args(["memory", "prune"])
        .assert()
        .success_without_warnings()
        .stdout(predicates::str::contains("Maintenance complete"));
}

#[test]
fn memory_prune_json_output() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    let json = run_json(dir, &["memory", "prune", "--json"]);

    assert!(json.get("expired_pruned").is_some());
    assert!(json.get("decayed_pruned").is_some());
    assert!(json.get("promoted").is_some());
    assert!(json.get("conflicts_resolved").is_some());
}

#[test]
fn memory_prune_expired_only() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    let json = run_json(dir, &["memory", "prune", "--expired-only", "--json"]);

    assert!(json.get("expired_pruned").is_some());
    assert_eq!(json["decayed_pruned"].as_u64().unwrap(), 0,
        "expired-only mode should not report decayed pruning");
}

#[test]
fn memory_stats_shows_statistics() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    abathur_cmd(dir)
        .args(["memory", "stats"])
        .assert()
        .success_without_warnings()
        .stdout(predicates::str::contains("Memory Statistics"));
}

#[test]
fn memory_stats_json_output() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    // Store one memory so stats are nonzero
    abathur_cmd(dir)
        .args(["memory", "store", "stats-key", "stats content"])
        .assert()
        .success_without_warnings();

    let json = run_json(dir, &["memory", "stats", "--json"]);

    assert!(json.get("working").is_some());
    assert!(json.get("episodic").is_some());
    assert!(json.get("semantic").is_some());
    assert!(json.get("total").is_some());
    assert!(json["working"].as_u64().unwrap() >= 1,
        "Should have at least one working memory");
    assert_eq!(json["total"].as_u64().unwrap(),
        json["working"].as_u64().unwrap()
            + json["episodic"].as_u64().unwrap()
            + json["semantic"].as_u64().unwrap(),
        "Total should equal sum of tiers");
}

#[test]
fn memory_stats_json_empty() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    let json = run_json(dir, &["memory", "stats", "--json"]);

    assert_eq!(json["working"].as_u64().unwrap(), 0);
    assert_eq!(json["episodic"].as_u64().unwrap(), 0);
    assert_eq!(json["semantic"].as_u64().unwrap(), 0);
    assert_eq!(json["total"].as_u64().unwrap(), 0);
}

#[test]
fn memory_full_lifecycle() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    // 1. Store a memory
    let store = run_json(dir, &["memory", "store", "lifecycle-key", "lifecycle content", "--json"]);
    assert_eq!(store["success"], true);
    let id = json_str(&store["memory"], "id");
    assert_eq!(json_str(&store["memory"], "key"), "lifecycle-key");

    // 2. List should include it
    let list = run_json(dir, &["memory", "list", "--json"]);
    let memories = list["memories"].as_array().unwrap();
    assert!(memories.iter().any(|m| json_str(m, "id") == id));

    // 3. Recall by ID
    let recall_id = run_json(dir, &["memory", "recall", &id, "--json"]);
    assert_eq!(json_str(&recall_id["memory"], "id"), id);
    assert_eq!(recall_id["content"].as_str().unwrap(), "lifecycle content");

    // 4. Recall by key
    let recall_key = run_json(dir, &["memory", "recall", "lifecycle-key", "-n", "default", "--json"]);
    assert_eq!(json_str(&recall_key["memory"], "id"), id);
    assert_eq!(recall_key["content"].as_str().unwrap(), "lifecycle content");

    // 5. Search
    let search = run_json(dir, &["memory", "search", "lifecycle", "--json"]);
    let search_results = search["memories"].as_array().unwrap();
    assert!(search_results.iter().any(|m| json_str(m, "id") == id));

    // 6. Stats should show one working memory
    let stats = run_json(dir, &["memory", "stats", "--json"]);
    assert!(stats["working"].as_u64().unwrap() >= 1);

    // 7. Forget the memory
    let forget = run_json(dir, &["memory", "forget", &id, "--json"]);
    assert_eq!(forget["success"], true);

    // 8. Stats should reflect deletion
    let stats_after = run_json(dir, &["memory", "stats", "--json"]);
    assert!(
        stats_after["total"].as_u64().unwrap() < stats["total"].as_u64().unwrap(),
        "Total should decrease after forgetting"
    );
}

// ============================================================
// Agent command tests
// ============================================================

#[test]
fn agent_register_creates_worker() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    abathur_cmd(dir)
        .args(["agent", "register", "test-worker", "-p", "You are a test agent"])
        .assert()
        .success_without_warnings()
        .stdout(predicates::str::contains("Agent registered"));
}

#[test]
fn agent_register_json_output() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    let json = run_json(
        dir,
        &["agent", "register", "test-worker", "-p", "You are a test agent", "--json"],
    );

    assert_eq!(json["success"], true);
    let agent = &json["agent"];
    assert!(agent["id"].as_str().is_some(), "agent should have an id");
    assert_eq!(json_str(agent, "name"), "test-worker");
    assert_eq!(json_str(agent, "tier"), "worker");
    assert_eq!(json_str(agent, "status"), "active");
    assert_eq!(agent["version"].as_u64().unwrap(), 1);
}

#[test]
fn agent_register_specialist_json() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    let json = run_json(
        dir,
        &["agent", "register", "test-specialist", "-p", "You are a specialist", "-t", "specialist", "--json"],
    );

    assert_eq!(json["success"], true);
    assert_eq!(json_str(&json["agent"], "name"), "test-specialist");
    assert_eq!(json_str(&json["agent"], "tier"), "specialist");
}

#[test]
fn agent_register_with_tools() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    let json = run_json(
        dir,
        &[
            "agent", "register", "with-tools",
            "-p", "Agent with tools",
            "--tool", "bash:Run shell commands",
            "--tool", "read:Read files",
            "--json",
        ],
    );

    assert_eq!(json["success"], true);
    assert_eq!(json_str(&json["agent"], "name"), "with-tools");
    assert_eq!(json["agent"]["tools_count"].as_u64().unwrap(), 2);
}

#[test]
fn agent_list_shows_agents() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    // Register an agent first
    abathur_cmd(dir)
        .args(["agent", "register", "listed-worker", "-p", "Listed agent"])
        .assert()
        .success_without_warnings();

    abathur_cmd(dir)
        .args(["agent", "list"])
        .assert()
        .success_without_warnings()
        .stdout(predicates::str::contains("listed-worker"));
}

#[test]
fn agent_list_json_output() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    // Register two agents
    abathur_cmd(dir)
        .args(["agent", "register", "agent-a", "-p", "Agent A"])
        .assert()
        .success_without_warnings();
    abathur_cmd(dir)
        .args(["agent", "register", "agent-b", "-p", "Agent B", "-t", "specialist"])
        .assert()
        .success_without_warnings();

    let json = run_json(dir, &["agent", "list", "--json"]);

    let agents = json["agents"].as_array().expect("agents should be an array");
    assert!(agents.len() >= 2);
    assert!(json["total"].as_u64().unwrap() >= 2);
}

#[test]
fn agent_list_filter_by_tier() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    // Register agents with different tiers
    abathur_cmd(dir)
        .args(["agent", "register", "tier-worker", "-p", "A worker", "-t", "worker"])
        .assert()
        .success_without_warnings();
    abathur_cmd(dir)
        .args(["agent", "register", "tier-specialist", "-p", "A specialist", "-t", "specialist"])
        .assert()
        .success_without_warnings();

    // Filter by worker tier
    let json = run_json(dir, &["agent", "list", "-t", "worker", "--json"]);

    let agents = json["agents"].as_array().expect("agents should be an array");
    assert!(!agents.is_empty(), "Should find worker agents");
    for agent in agents {
        assert_eq!(json_str(agent, "tier"), "worker",
            "All listed agents should be workers");
    }
}

#[test]
fn agent_list_empty() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    abathur_cmd(dir)
        .args(["agent", "list"])
        .assert()
        .success_without_warnings()
        .stdout(predicates::str::contains("No agents found"));
}

#[test]
fn agent_show_displays_agent() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    abathur_cmd(dir)
        .args(["agent", "register", "show-worker", "-p", "Show agent"])
        .assert()
        .success_without_warnings();

    abathur_cmd(dir)
        .args(["agent", "show", "show-worker"])
        .assert()
        .success_without_warnings()
        .stdout(predicates::str::contains("show-worker"));
}

#[test]
fn agent_show_json_output() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    abathur_cmd(dir)
        .args(["agent", "register", "show-json-worker", "-p", "Show JSON agent"])
        .assert()
        .success_without_warnings();

    let json = run_json(dir, &["agent", "show", "show-json-worker", "--json"]);

    assert_eq!(json_str(&json["agent"], "name"), "show-json-worker");
    assert_eq!(json_str(&json["agent"], "status"), "active");
    assert_eq!(json_str(&json["agent"], "tier"), "worker");
}

#[test]
fn agent_disable_disables_agent() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    abathur_cmd(dir)
        .args(["agent", "register", "disable-worker", "-p", "Disable me"])
        .assert()
        .success_without_warnings();

    abathur_cmd(dir)
        .args(["agent", "disable", "disable-worker"])
        .assert()
        .success_without_warnings()
        .stdout(predicates::str::contains("Agent disabled"));
}

#[test]
fn agent_disable_json_output() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    abathur_cmd(dir)
        .args(["agent", "register", "disable-json-worker", "-p", "Disable me"])
        .assert()
        .success_without_warnings();

    let json = run_json(dir, &["agent", "disable", "disable-json-worker", "--json"]);

    assert_eq!(json["success"], true);
    assert_eq!(json_str(&json["agent"], "status"), "disabled");
    assert_eq!(json_str(&json["agent"], "name"), "disable-json-worker");
}

#[test]
fn agent_enable_reenables_agent() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    abathur_cmd(dir)
        .args(["agent", "register", "enable-worker", "-p", "Enable me"])
        .assert()
        .success_without_warnings();

    // Disable first
    abathur_cmd(dir)
        .args(["agent", "disable", "enable-worker"])
        .assert()
        .success_without_warnings();

    // Then re-enable
    abathur_cmd(dir)
        .args(["agent", "enable", "enable-worker"])
        .assert()
        .success_without_warnings()
        .stdout(predicates::str::contains("Agent enabled"));
}

#[test]
fn agent_enable_json_output() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    abathur_cmd(dir)
        .args(["agent", "register", "enable-json-worker", "-p", "Enable me"])
        .assert()
        .success_without_warnings();

    // Disable then enable
    abathur_cmd(dir)
        .args(["agent", "disable", "enable-json-worker"])
        .assert()
        .success_without_warnings();

    let json = run_json(dir, &["agent", "enable", "enable-json-worker", "--json"]);

    assert_eq!(json["success"], true);
    assert_eq!(json_str(&json["agent"], "status"), "active");
    assert_eq!(json_str(&json["agent"], "name"), "enable-json-worker");
}

#[test]
fn agent_instances_empty() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    abathur_cmd(dir)
        .args(["agent", "instances"])
        .assert()
        .success_without_warnings()
        .stdout(predicates::str::contains("No running instances"));
}

#[test]
fn agent_instances_json_output() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    let json = run_json(dir, &["agent", "instances", "--json"]);

    let instances = json["instances"]
        .as_array()
        .expect("instances should be an array");
    assert_eq!(instances.len(), 0);
    assert_eq!(json["total"].as_u64().unwrap(), 0);
}

#[test]
fn agent_stats_shows_statistics() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    abathur_cmd(dir)
        .args(["agent", "stats"])
        .assert()
        .success_without_warnings()
        .stdout(predicates::str::contains("Agent Statistics"));
}

#[test]
fn agent_stats_json_output() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    // Register agents of different tiers
    abathur_cmd(dir)
        .args(["agent", "register", "stats-worker", "-p", "Worker", "-t", "worker"])
        .assert()
        .success_without_warnings();
    abathur_cmd(dir)
        .args(["agent", "register", "stats-specialist", "-p", "Specialist", "-t", "specialist"])
        .assert()
        .success_without_warnings();

    let json = run_json(dir, &["agent", "stats", "--json"]);

    assert!(json.get("architect_count").is_some());
    assert!(json.get("specialist_count").is_some());
    assert!(json.get("worker_count").is_some());
    assert!(json.get("total").is_some());
    assert!(json.get("running_instances").is_some());

    assert!(json["worker_count"].as_u64().unwrap() >= 1);
    assert!(json["specialist_count"].as_u64().unwrap() >= 1);
    assert!(json["total"].as_u64().unwrap() >= 2);
    assert_eq!(json["running_instances"].as_u64().unwrap(), 0);
}

#[test]
fn agent_stats_json_empty() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    let json = run_json(dir, &["agent", "stats", "--json"]);

    assert_eq!(json["architect_count"].as_u64().unwrap(), 0);
    assert_eq!(json["specialist_count"].as_u64().unwrap(), 0);
    assert_eq!(json["worker_count"].as_u64().unwrap(), 0);
    assert_eq!(json["total"].as_u64().unwrap(), 0);
    assert_eq!(json["running_instances"].as_u64().unwrap(), 0);
}

#[test]
fn agent_gateway_status_graceful_when_not_running() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    // The gateway-status command should succeed even when no gateway is running.
    // It reports the status as NOT RUNNING rather than failing.
    abathur_cmd(dir)
        .args(["agent", "gateway-status"])
        .assert()
        .success_without_warnings()
        .stdout(predicates::str::contains("NOT RUNNING"));
}

#[test]
fn agent_gateway_status_json_output() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    let json = run_json(dir, &["agent", "gateway-status", "--json"]);

    assert_eq!(json["running"].as_bool().unwrap(), false);
    assert!(json["url"].as_str().is_some());
    assert!(json["message"].as_str().is_some());
    assert_eq!(json["agents"].as_u64().unwrap(), 0);
}

#[test]
fn agent_full_lifecycle() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    // 1. Register an agent
    let register = run_json(
        dir,
        &["agent", "register", "lifecycle-worker", "-p", "Lifecycle agent", "--json"],
    );
    assert_eq!(register["success"], true);
    let name = json_str(&register["agent"], "name");
    assert_eq!(name, "lifecycle-worker");
    assert_eq!(json_str(&register["agent"], "status"), "active");
    assert_eq!(json_str(&register["agent"], "tier"), "worker");

    // 2. List should include it
    let list = run_json(dir, &["agent", "list", "--json"]);
    let agents = list["agents"].as_array().unwrap();
    assert!(agents.iter().any(|a| json_str(a, "name") == "lifecycle-worker"));

    // 3. Disable the agent
    let disable = run_json(dir, &["agent", "disable", "lifecycle-worker", "--json"]);
    assert_eq!(disable["success"], true);
    assert_eq!(json_str(&disable["agent"], "status"), "disabled");

    // 4. Enable the agent
    let enable = run_json(dir, &["agent", "enable", "lifecycle-worker", "--json"]);
    assert_eq!(enable["success"], true);
    assert_eq!(json_str(&enable["agent"], "status"), "active");

    // 5. Stats should reflect the registered agent
    let stats = run_json(dir, &["agent", "stats", "--json"]);
    assert!(stats["worker_count"].as_u64().unwrap() >= 1);
    assert!(stats["total"].as_u64().unwrap() >= 1);
}

// ============================================================
// Agent command gap tests
// ============================================================

#[test]
fn agent_register_architect_json() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    let json = run_json(
        dir,
        &["agent", "register", "test-architect", "-p", "You are an architect", "-t", "architect", "--json"],
    );

    assert_eq!(json["success"], true);
    assert_eq!(json_str(&json["agent"], "name"), "test-architect");
    assert_eq!(json_str(&json["agent"], "tier"), "architect");
    assert_eq!(json["agent"]["version"].as_u64().unwrap(), 1);
}

#[test]
fn agent_register_with_description_json() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    let json = run_json(
        dir,
        &["agent", "register", "described-agent", "-p", "A prompt", "-d", "A helper agent", "--json"],
    );

    assert_eq!(json["success"], true);
    assert_eq!(json_str(&json["agent"], "name"), "described-agent");

    // Verify description appears in agent show output
    let show = run_json(dir, &["agent", "show", "described-agent", "--json"]);
    assert_eq!(show["description"].as_str().unwrap(), "A helper agent");
}

#[test]
fn agent_register_with_max_turns_json() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    let json = run_json(
        dir,
        &["agent", "register", "capped-agent", "-p", "A prompt", "--max-turns", "15", "--json"],
    );

    assert_eq!(json["success"], true);
    assert_eq!(json["agent"]["max_turns"].as_u64().unwrap(), 15);
}

#[test]
fn agent_list_active_only() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    // Register two agents
    abathur_cmd(dir)
        .args(["agent", "register", "active-agent", "-p", "I stay active"])
        .assert()
        .success_without_warnings();
    abathur_cmd(dir)
        .args(["agent", "register", "disabled-agent", "-p", "I get disabled"])
        .assert()
        .success_without_warnings();

    // Disable one
    abathur_cmd(dir)
        .args(["agent", "disable", "disabled-agent"])
        .assert()
        .success_without_warnings();

    // List with --active-only
    let json = run_json(dir, &["agent", "list", "--active-only", "--json"]);

    let agents = json["agents"].as_array().expect("agents should be an array");
    assert!(!agents.is_empty(), "Should have at least one active agent");
    for agent in agents {
        assert_eq!(
            json_str(agent, "status"),
            "active",
            "All listed agents should be active"
        );
    }
    // The disabled agent should not appear
    assert!(
        !agents.iter().any(|a| json_str(a, "name") == "disabled-agent"),
        "Disabled agent should not appear in active-only list"
    );
}

#[test]
fn agent_register_versioning() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    // Register the same agent name twice
    let v1 = run_json(
        dir,
        &["agent", "register", "versioned-agent", "-p", "Version one prompt", "--json"],
    );
    assert_eq!(v1["success"], true);
    assert_eq!(v1["agent"]["version"].as_u64().unwrap(), 1);

    let v2 = run_json(
        dir,
        &["agent", "register", "versioned-agent", "-p", "Version two prompt", "--json"],
    );
    assert_eq!(v2["success"], true);
    assert_eq!(v2["agent"]["version"].as_u64().unwrap(), 2);
}

#[test]
fn agent_show_specific_version() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    // Register the same agent name twice to create v1 and v2
    abathur_cmd(dir)
        .args(["agent", "register", "multi-ver", "-p", "First version"])
        .assert()
        .success_without_warnings();
    abathur_cmd(dir)
        .args(["agent", "register", "multi-ver", "-p", "Second version"])
        .assert()
        .success_without_warnings();

    // Show with --version 1, verify version is 1
    let json = run_json(dir, &["agent", "show", "multi-ver", "--version", "1", "--json"]);
    assert_eq!(json["agent"]["version"].as_u64().unwrap(), 1);
}

#[test]
fn agent_show_nonexistent() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    let json = run_json(dir, &["agent", "show", "no-such-agent", "--json"]);

    assert_eq!(json["success"], false);
    assert!(
        json["message"].as_str().unwrap().to_lowercase().contains("not found"),
        "Message should indicate agent was not found"
    );
}

#[test]
fn agent_disable_nonexistent_fails() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    abathur_cmd(dir)
        .args(["agent", "disable", "ghost-agent"])
        .assert()
        .failure();
}

#[test]
fn agent_enable_nonexistent_fails() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    abathur_cmd(dir)
        .args(["agent", "enable", "ghost-agent"])
        .assert()
        .failure();
}

#[test]
fn agent_register_invalid_tier_fails() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    abathur_cmd(dir)
        .args(["agent", "register", "bad-tier-agent", "-p", "A prompt", "-t", "bogus"])
        .assert()
        .failure();
}

#[test]
fn agent_list_filter_specialist_json() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    // Register a specialist and a worker
    abathur_cmd(dir)
        .args(["agent", "register", "filter-specialist", "-p", "Specialist prompt", "-t", "specialist"])
        .assert()
        .success_without_warnings();
    abathur_cmd(dir)
        .args(["agent", "register", "filter-worker", "-p", "Worker prompt", "-t", "worker"])
        .assert()
        .success_without_warnings();

    // List filtered to specialist tier only
    let json = run_json(dir, &["agent", "list", "-t", "specialist", "--json"]);

    let agents = json["agents"].as_array().expect("agents should be an array");
    assert!(!agents.is_empty(), "Should find at least one specialist");
    for agent in agents {
        assert_eq!(
            json_str(agent, "tier"),
            "specialist",
            "All listed agents should be specialists"
        );
    }
    assert!(
        !agents.iter().any(|a| json_str(a, "name") == "filter-worker"),
        "Worker agent should not appear in specialist-filtered list"
    );
}

#[test]
fn agent_register_with_all_options_json() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    let json = run_json(
        dir,
        &[
            "agent", "register", "full-options-agent",
            "-d", "desc",
            "-t", "specialist",
            "-p", "prompt",
            "--tool", "bash:Run commands",
            "--max-turns", "20",
            "--json",
        ],
    );

    assert_eq!(json["success"], true);
    let agent = &json["agent"];
    assert_eq!(json_str(agent, "name"), "full-options-agent");
    assert_eq!(json_str(agent, "tier"), "specialist");
    assert_eq!(agent["version"].as_u64().unwrap(), 1);
    assert_eq!(json_str(agent, "status"), "active");
    assert_eq!(agent["tools_count"].as_u64().unwrap(), 1);
    assert_eq!(agent["max_turns"].as_u64().unwrap(), 20);

    // Verify description and prompt appear in show output
    let show = run_json(dir, &["agent", "show", "full-options-agent", "--json"]);
    assert_eq!(show["description"].as_str().unwrap(), "desc");
    assert!(
        show["prompt_preview"].as_str().unwrap().contains("prompt"),
        "Show output should include the system prompt"
    );
    let tools = show["tools"].as_array().expect("tools should be an array");
    assert_eq!(tools.len(), 1);
    assert!(
        tools[0].as_str().unwrap().contains("bash"),
        "Tool entry should reference bash"
    );
}

// ============================================================
// Memory command gap tests
// ============================================================

#[test]
fn memory_store_with_tier_episodic() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    let json = run_json(
        dir,
        &["memory", "store", "epi-key", "episodic content", "--tier", "episodic", "--json"],
    );

    assert_eq!(json["success"], true);
    let memory = &json["memory"];
    assert_eq!(json_str(memory, "tier"), "episodic");
    assert_eq!(json_str(memory, "key"), "epi-key");
}

#[test]
fn memory_store_with_tier_semantic() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    let json = run_json(
        dir,
        &["memory", "store", "sem-key", "semantic content", "--tier", "semantic", "--json"],
    );

    assert_eq!(json["success"], true);
    let memory = &json["memory"];
    assert_eq!(json_str(memory, "tier"), "semantic");
    assert_eq!(json_str(memory, "key"), "sem-key");
}

#[test]
fn memory_store_with_type_code() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    let json = run_json(
        dir,
        &["memory", "store", "code-key", "fn main() {}", "--memory-type", "code", "--json"],
    );

    assert_eq!(json["success"], true);
    assert_eq!(json_str(&json["memory"], "memory_type"), "code");
}

#[test]
fn memory_store_with_type_decision() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    let json = run_json(
        dir,
        &["memory", "store", "decision-key", "chose option A", "--memory-type", "decision", "--json"],
    );

    assert_eq!(json["success"], true);
    assert_eq!(json_str(&json["memory"], "memory_type"), "decision");
}

#[test]
fn memory_store_with_type_error() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    let json = run_json(
        dir,
        &["memory", "store", "error-key", "panicked at line 42", "--memory-type", "error", "--json"],
    );

    assert_eq!(json["success"], true);
    assert_eq!(json_str(&json["memory"], "memory_type"), "error");
}

#[test]
fn memory_store_with_type_pattern() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    let json = run_json(
        dir,
        &["memory", "store", "pattern-key", "retry with backoff", "--memory-type", "pattern", "--json"],
    );

    assert_eq!(json["success"], true);
    assert_eq!(json_str(&json["memory"], "memory_type"), "pattern");
}

#[test]
fn memory_store_with_type_reference() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    let json = run_json(
        dir,
        &["memory", "store", "ref-key", "see RFC 1234", "--memory-type", "reference", "--json"],
    );

    assert_eq!(json["success"], true);
    assert_eq!(json_str(&json["memory"], "memory_type"), "reference");
}

#[test]
fn memory_store_with_type_context() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    let json = run_json(
        dir,
        &["memory", "store", "ctx-key", "running on linux", "--memory-type", "context", "--json"],
    );

    assert_eq!(json["success"], true);
    assert_eq!(json_str(&json["memory"], "memory_type"), "context");
}

#[test]
fn memory_search_with_namespace_filter() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    // Store a memory in a custom namespace
    abathur_cmd(dir)
        .args(["memory", "store", "ns-search-key", "findme in custom", "-n", "custom-ns"])
        .assert()
        .success_without_warnings();

    // Store a memory in the default namespace with similar content
    abathur_cmd(dir)
        .args(["memory", "store", "default-search-key", "findme in default"])
        .assert()
        .success_without_warnings();

    // Search with namespace filter
    let json = run_json(dir, &["memory", "search", "findme", "-n", "custom-ns", "--json"]);

    let memories = json["memories"].as_array().expect("memories should be an array");
    assert!(!memories.is_empty(), "Should find at least one memory in custom-ns");
    for mem in memories {
        assert_eq!(json_str(mem, "namespace"), "custom-ns",
            "All search results should be in custom-ns");
    }
}

#[test]
fn memory_search_with_limit() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    // Store multiple memories with a common keyword
    abathur_cmd(dir)
        .args(["memory", "store", "limit-a", "limitword alpha"])
        .assert()
        .success_without_warnings();
    abathur_cmd(dir)
        .args(["memory", "store", "limit-b", "limitword beta"])
        .assert()
        .success_without_warnings();
    abathur_cmd(dir)
        .args(["memory", "store", "limit-c", "limitword gamma"])
        .assert()
        .success_without_warnings();

    // Search with limit 1
    let json = run_json(dir, &["memory", "search", "limitword", "--limit", "1", "--json"]);

    let memories = json["memories"].as_array().expect("memories should be an array");
    assert!(memories.len() <= 1, "Should return at most 1 result when limit is 1");
}

#[test]
fn memory_search_empty_results() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    let json = run_json(dir, &["memory", "search", "zzz_nonexistent_term_zzz", "--json"]);

    let memories = json["memories"].as_array().expect("memories should be an array");
    assert_eq!(memories.len(), 0, "Search for nonexistent term should return empty results");
    assert_eq!(json["total"].as_u64().unwrap(), 0);
}

#[test]
fn memory_list_filter_by_tier_json() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    // Store memories in different tiers
    abathur_cmd(dir)
        .args(["memory", "store", "working-key", "working content", "--tier", "working"])
        .assert()
        .success_without_warnings();
    abathur_cmd(dir)
        .args(["memory", "store", "episodic-key", "episodic content", "--tier", "episodic"])
        .assert()
        .success_without_warnings();

    // Filter by working tier
    let json = run_json(dir, &["memory", "list", "--tier", "working", "--json"]);

    let memories = json["memories"].as_array().expect("memories should be an array");
    assert!(!memories.is_empty(), "Should find at least one working memory");
    for mem in memories {
        assert_eq!(json_str(mem, "tier"), "working",
            "All listed memories should be in working tier");
    }
}

#[test]
fn memory_list_with_limit() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    // Store several memories
    abathur_cmd(dir)
        .args(["memory", "store", "lim-a", "content a"])
        .assert()
        .success_without_warnings();
    abathur_cmd(dir)
        .args(["memory", "store", "lim-b", "content b"])
        .assert()
        .success_without_warnings();
    abathur_cmd(dir)
        .args(["memory", "store", "lim-c", "content c"])
        .assert()
        .success_without_warnings();

    // List with limit 2
    let json = run_json(dir, &["memory", "list", "--limit", "2", "--json"]);

    let memories = json["memories"].as_array().expect("memories should be an array");
    assert!(memories.len() <= 2, "Should return at most 2 memories when limit is 2");
}

#[test]
fn memory_recall_nonexistent() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    let fake_uuid = "00000000-0000-0000-0000-000000000000";

    // Recall with a nonexistent UUID should either fail or return a not-found message
    let output = abathur_cmd(dir)
        .args(["memory", "recall", fake_uuid, "--json"])
        .output()
        .unwrap();

    if output.status.success() {
        // If it succeeds, the JSON should indicate not found
        let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(json["success"], false,
            "Recalling nonexistent memory should report success=false");
        assert!(json["message"].as_str().unwrap().contains("not found"),
            "Message should indicate memory not found");
    }
    // If it fails (non-zero exit), that is also acceptable behavior
}

#[test]
fn memory_forget_nonexistent_fails() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    let fake_uuid = "00000000-0000-0000-0000-000000000000";

    abathur_cmd(dir)
        .args(["memory", "forget", fake_uuid])
        .assert()
        .failure();
}

#[test]
fn memory_store_invalid_tier_fails() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    abathur_cmd(dir)
        .args(["memory", "store", "bad-tier-key", "content", "--tier", "bogus"])
        .assert()
        .failure();
}

#[test]
fn memory_store_invalid_type_fails() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    abathur_cmd(dir)
        .args(["memory", "store", "bad-type-key", "content", "--memory-type", "bogus"])
        .assert()
        .failure();
}

#[test]
fn memory_store_all_options_json() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    let json = run_json(
        dir,
        &[
            "memory", "store", "all-opts-key", "all options content",
            "-n", "myns",
            "--tier", "episodic",
            "--memory-type", "decision",
            "--json",
        ],
    );

    assert_eq!(json["success"], true);
    let memory = &json["memory"];
    assert_eq!(json_str(memory, "key"), "all-opts-key");
    assert_eq!(json_str(memory, "namespace"), "myns");
    assert_eq!(json_str(memory, "tier"), "episodic");
    assert_eq!(json_str(memory, "memory_type"), "decision");
}

// ============================================================
// Per-command help flag tests
// ============================================================

#[test]
fn task_help_shows_usage() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();

    abathur_cmd(dir)
        .args(["task", "--help"])
        .assert()
        .success_without_warnings()
        .stdout(
            predicates::str::contains("Usage")
                .and(predicates::str::contains("task")),
        );
}

#[test]
fn memory_help_shows_usage() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();

    abathur_cmd(dir)
        .args(["memory", "--help"])
        .assert()
        .success_without_warnings()
        .stdout(
            predicates::str::contains("Usage")
                .and(predicates::str::contains("memory")),
        );
}

#[test]
fn agent_help_shows_usage() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();

    abathur_cmd(dir)
        .args(["agent", "--help"])
        .assert()
        .success_without_warnings()
        .stdout(
            predicates::str::contains("Usage")
                .and(predicates::str::contains("agent")),
        );
}

#[test]
fn worktree_help_shows_usage() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();

    abathur_cmd(dir)
        .args(["worktree", "--help"])
        .assert()
        .success_without_warnings()
        .stdout(
            predicates::str::contains("Usage")
                .and(predicates::str::contains("worktree")),
        );
}

#[test]
fn trigger_help_shows_usage() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();

    abathur_cmd(dir)
        .args(["trigger", "--help"])
        .assert()
        .success_without_warnings()
        .stdout(
            predicates::str::contains("Usage")
                .and(predicates::str::contains("trigger")),
        );
}

#[test]
fn event_help_shows_usage() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();

    abathur_cmd(dir)
        .args(["event", "--help"])
        .assert()
        .success_without_warnings()
        .stdout(
            predicates::str::contains("Usage")
                .and(predicates::str::contains("event")),
        );
}

#[test]
fn swarm_help_shows_usage() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();

    abathur_cmd(dir)
        .args(["swarm", "--help"])
        .assert()
        .success_without_warnings()
        .stdout(
            predicates::str::contains("Usage")
                .and(predicates::str::contains("swarm")),
        );
}

#[test]
fn mcp_help_shows_usage() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();

    abathur_cmd(dir)
        .args(["mcp", "--help"])
        .assert()
        .success_without_warnings()
        .stdout(
            predicates::str::contains("Usage")
                .and(predicates::str::contains("mcp")),
        );
}

// ============================================================
// Version flag test
// ============================================================

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

// ============================================================
// Commands without init (auto-initialize behavior)
// ============================================================
//
// The CLI auto-creates the .abathur directory and database when
// commands are run without explicit init. These tests verify that
// commands succeed gracefully with empty results even when init
// has not been explicitly called.

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
fn task_without_init_auto_creates_db() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();

    assert!(!dir.join(".abathur").exists());

    abathur_cmd(dir)
        .args(["task", "list"])
        .assert()
        .success_without_warnings()
        .stdout(predicates::str::contains("No tasks found"));

    assert!(dir.join(".abathur/abathur.db").exists());
}

#[test]
fn memory_without_init_auto_creates_db() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();

    assert!(!dir.join(".abathur").exists());

    abathur_cmd(dir)
        .args(["memory", "list"])
        .assert()
        .success_without_warnings()
        .stdout(predicates::str::contains("No memories found"));

    assert!(dir.join(".abathur/abathur.db").exists());
}

#[test]
fn agent_without_init_auto_creates_db() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();

    assert!(!dir.join(".abathur").exists());

    abathur_cmd(dir)
        .args(["agent", "list"])
        .assert()
        .success_without_warnings()
        .stdout(predicates::str::contains("No agents found"));

    assert!(dir.join(".abathur/abathur.db").exists());
}

#[test]
fn worktree_without_init_auto_creates_db() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();

    assert!(!dir.join(".abathur").exists());

    abathur_cmd(dir)
        .args(["worktree", "list"])
        .assert()
        .success_without_warnings()
        .stdout(predicates::str::contains("No worktrees found"));

    assert!(dir.join(".abathur/abathur.db").exists());
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

// ============================================================
// Invalid enum values
// ============================================================

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
fn task_list_invalid_status() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    // TaskStatus::from_str returns None for invalid values, so the filter
    // becomes None and all tasks are returned (succeeds silently).
    let json = run_json(dir, &["task", "list", "--status", "bogus", "--json"]);

    // Should succeed with an empty or full list (filter is null)
    assert!(json["tasks"].as_array().is_some());
}

#[test]
fn worktree_list_invalid_status() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    // WorktreeStatus::from_str returns None, which triggers an explicit
    // error: "Invalid status: bogus"
    abathur_cmd(dir)
        .args(["worktree", "list", "--status", "bogus"])
        .assert()
        .failure()
        .stderr(predicates::str::contains("Invalid status"));
}

// ============================================================
// Missing required args
// ============================================================

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
fn memory_store_missing_key_fails() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();

    abathur_cmd(dir)
        .args(["memory", "store"])
        .assert()
        .failure()
        .stderr(predicates::str::is_match("required|Usage").unwrap());
}

#[test]
fn memory_store_missing_content_fails() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();

    abathur_cmd(dir)
        .args(["memory", "store", "mykey"])
        .assert()
        .failure()
        .stderr(predicates::str::is_match("required|Usage").unwrap());
}

#[test]
fn agent_register_missing_prompt_fails() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();

    abathur_cmd(dir)
        .args(["agent", "register", "myname"])
        .assert()
        .failure()
        .stderr(predicates::str::is_match("required|--prompt").unwrap());
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
fn task_show_missing_id_fails() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();

    abathur_cmd(dir)
        .args(["task", "show"])
        .assert()
        .failure()
        .stderr(predicates::str::is_match("required|Usage").unwrap());
}

// ============================================================
// UUID prefix resolution
// ============================================================

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
fn task_show_by_prefix() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    let create = run_json(
        dir,
        &["task", "submit", "Prefix task prompt", "-t", "Prefix Task", "--json"],
    );
    let full_id = json_str(&create["task"], "id");
    let prefix = &full_id[..8];

    let show = run_json(dir, &["task", "show", prefix, "--json"]);

    assert_eq!(json_str(&show["task"], "id"), full_id);
    assert_eq!(json_str(&show["task"], "title"), "Prefix Task");
}

// ============================================================
// Init subpath
// ============================================================

#[test]
fn init_custom_path() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();

    abathur_cmd(dir)
        .args(["init", "subdir"])
        .assert()
        .success_without_warnings()
        .stdout(predicates::str::contains("initialized successfully"));

    assert!(dir.join("subdir/.abathur").is_dir());
    assert!(dir.join("subdir/.abathur/abathur.db").exists());
}

// ============================================================
// Verbose flag
// ============================================================

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

// ============================================================
// Swarm command gap tests
// ============================================================

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

    assert!(json.get("active_goals").is_some(), "tick should report active_goals");
    assert!(json.get("pending_tasks").is_some(), "tick should report pending_tasks");
    assert!(json.get("running_tasks").is_some(), "tick should report running_tasks");
    assert!(json.get("completed_tasks").is_some(), "tick should report completed_tasks");
    assert!(json.get("failed_tasks").is_some(), "tick should report failed_tasks");
    assert!(json.get("active_agents").is_some(), "tick should report active_agents");

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
    assert_eq!(escalations.len(), 0, "Fresh project should have no pending escalations");
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
        active_goals.len() >= 1,
        "Should have at least one active goal"
    );
    assert!(
        active_goals.iter().any(|g| json_str(g, "id") == goal_id),
        "Active goals should include the newly created goal"
    );
}

// ============================================================
// Event command gap tests
// ============================================================

#[test]
fn event_list_with_category_task() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    // Create some tasks to potentially generate task events
    abathur_cmd(dir)
        .args(["task", "submit", "Category filter test 1", "-t", "Cat Task 1"])
        .assert()
        .success_without_warnings();
    abathur_cmd(dir)
        .args(["task", "submit", "Category filter test 2", "-t", "Cat Task 2"])
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
    assert!(json.get("total_gaps").is_some(), "total_gaps field should exist");
    assert!(json.get("scan_from").is_some(), "scan_from field should exist");
    assert!(json.get("scan_to").is_some(), "scan_to field should exist");

    // gaps should be an array
    let gaps = json["gaps"]
        .as_array()
        .expect("gaps should be an array");
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

    let json = run_json(dir, &["event", "dlq", "purge", "--older-than", "24h", "--json"]);

    assert!(json.get("count").is_some(), "purge should report a count");
    assert_eq!(
        json["count"].as_u64().unwrap(),
        0,
        "purge on empty DLQ should report 0"
    );
    assert!(json.get("message").is_some(), "purge should include a message");
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
    assert_eq!(entries.len(), 0, "Filtering by nonexistent handler should return empty");
    assert_eq!(json["total"].as_u64().unwrap(), 0);
}

#[test]
fn event_dlq_list_with_limit() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    let json = run_json(dir, &["event", "dlq", "list", "--limit", "5", "--json"]);

    assert!(json.get("total").is_some(), "DLQ list with limit should include total field");
    assert!(json["total"].as_u64().is_some());

    let entries = json["entries"]
        .as_array()
        .expect("entries should be an array");
    assert!(entries.len() <= 5, "Should respect the limit of 5");
}

// ============================================================
// Goal command gap tests (priority, constraints, parent, filters)
// ============================================================

#[test]
fn goal_set_with_priority_json() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    let json = run_json(
        dir,
        &["goal", "set", "High Priority Goal", "--priority", "high", "--json"],
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
        &["goal", "set", "Critical Goal", "--priority", "critical", "--json"],
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
        &["goal", "set", "Constrained Goal", "-c", "perf:Must be fast", "--json"],
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
        &["goal", "set", "Child Goal", "--parent", &parent_id, "--json"],
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
    assert!(!goals.is_empty(), "Should find at least one high-priority goal");
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

// ============================================================
// Task command gap tests (priority, agent, input, idempotency,
// deadline, parent, depends-on, filters, limit, error cases)
// ============================================================

#[test]
fn task_submit_with_priority_json() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    let json = run_json(
        dir,
        &["task", "submit", "High priority work", "--priority", "high", "--json"],
    );

    assert_eq!(json["success"], true);
    assert_eq!(json_str(&json["task"], "priority"), "high");
}

#[test]
fn task_submit_with_priority_critical() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    let json = run_json(
        dir,
        &["task", "submit", "Critical work", "--priority", "critical", "--json"],
    );

    assert_eq!(json["success"], true);
    assert_eq!(json_str(&json["task"], "priority"), "critical");
}

#[test]
fn task_submit_with_agent_json() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    let json = run_json(
        dir,
        &["task", "submit", "Agent work", "--agent", "my-worker", "--json"],
    );

    assert_eq!(json["success"], true);
    assert_eq!(
        json["task"]["agent_type"].as_str().unwrap(),
        "my-worker",
        "Task should be assigned to my-worker"
    );
}

#[test]
fn task_submit_with_input_json() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    let create = run_json(
        dir,
        &["task", "submit", "Input work", "--input", "some context", "--json"],
    );
    assert_eq!(create["success"], true);
    let id = json_str(&create["task"], "id");

    // Use task show to verify context_input is persisted
    let show = run_json(dir, &["task", "show", &id, "--json"]);

    assert_eq!(
        show["context_input"].as_str().unwrap(),
        "some context",
        "Task show should return the context_input"
    );
}

#[test]
fn task_submit_with_idempotency_key() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    // Submit first task with idempotency key
    let first = run_json(
        dir,
        &["task", "submit", "Dedup task", "--idempotency-key", "dup", "--json"],
    );
    assert_eq!(first["success"], true);
    let first_id = json_str(&first["task"], "id");

    // Submit second task with the same idempotency key
    let second = run_json(
        dir,
        &["task", "submit", "Dedup task again", "--idempotency-key", "dup", "--json"],
    );
    assert_eq!(second["success"], true);
    let second_id = json_str(&second["task"], "id");

    // Both calls should return the same task
    assert_eq!(
        first_id, second_id,
        "Duplicate idempotency key should return existing task"
    );

    // Only one task should exist in the list
    let list = run_json(dir, &["task", "list", "--json"]);
    let tasks = list["tasks"].as_array().unwrap();
    assert_eq!(
        tasks.len(),
        1,
        "Only one task should be created despite two submits"
    );
}

#[test]
fn task_submit_with_deadline_json() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    let json = run_json(
        dir,
        &["task", "submit", "Deadline work", "--deadline", "2030-12-31T23:59:59Z", "--json"],
    );

    assert_eq!(json["success"], true);
    assert!(
        json["task"]["id"].as_str().is_some(),
        "Task should be created with deadline"
    );
}

#[test]
fn task_submit_with_parent_json() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    // Create parent task
    let parent = run_json(
        dir,
        &["task", "submit", "Parent task prompt", "-t", "Parent Task", "--json"],
    );
    assert_eq!(parent["success"], true);
    let parent_id = json_str(&parent["task"], "id");

    // Create child task with --parent
    let child = run_json(
        dir,
        &[
            "task", "submit", "Child task prompt",
            "-t", "Child Task",
            "--parent", &parent_id,
            "--json",
        ],
    );

    assert_eq!(child["success"], true);
    let child_id = json_str(&child["task"], "id");

    // Verify the child was created successfully
    let show = run_json(dir, &["task", "show", &child_id, "--json"]);
    assert_eq!(json_str(&show["task"], "title"), "Child Task");
}

#[test]
fn task_submit_with_depends_on() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    // Create task A
    let task_a = run_json(
        dir,
        &["task", "submit", "Task A prompt", "-t", "Task A", "--json"],
    );
    let a_id = json_str(&task_a["task"], "id");

    // Create task B depending on A
    let task_b = run_json(
        dir,
        &[
            "task", "submit", "Task B prompt",
            "-t", "Task B",
            "--depends-on", &a_id,
            "--json",
        ],
    );

    assert_eq!(task_b["success"], true);
    let b_deps = task_b["task"]["depends_on"]
        .as_array()
        .expect("depends_on should be an array");
    assert!(
        b_deps.iter().any(|d| d.as_str().unwrap() == a_id),
        "Task B should depend on Task A"
    );
}

#[test]
fn task_list_filter_by_status_json() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    // Submit a task then cancel it
    let create = run_json(
        dir,
        &["task", "submit", "Cancel me", "-t", "Cancelable Task", "--json"],
    );
    let id = json_str(&create["task"], "id");

    run_json(dir, &["task", "cancel", &id, "--json"]);

    // Submit another task that stays active
    abathur_cmd(dir)
        .args(["task", "submit", "Active task", "-t", "Active Task"])
        .assert()
        .success_without_warnings();

    // List with --status canceled
    let list = run_json(dir, &["task", "list", "--status", "canceled", "--json"]);

    let tasks = list["tasks"].as_array().expect("tasks should be an array");
    assert!(!tasks.is_empty(), "Should find at least one canceled task");
    for task in tasks {
        assert_eq!(
            json_str(task, "status"),
            "canceled",
            "All listed tasks should be canceled"
        );
    }
}

#[test]
fn task_list_filter_by_priority_json() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    // Submit tasks with different priorities
    abathur_cmd(dir)
        .args(["task", "submit", "Low work", "--priority", "low"])
        .assert()
        .success_without_warnings();
    abathur_cmd(dir)
        .args(["task", "submit", "High work", "--priority", "high"])
        .assert()
        .success_without_warnings();

    // List with --priority high
    let list = run_json(dir, &["task", "list", "--priority", "high", "--json"]);

    let tasks = list["tasks"].as_array().expect("tasks should be an array");
    assert!(!tasks.is_empty(), "Should find at least one high-priority task");
    for task in tasks {
        assert_eq!(
            json_str(task, "priority"),
            "high",
            "All listed tasks should have high priority"
        );
    }
}

#[test]
fn task_list_with_limit() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    // Submit 5 tasks (no dependencies, so they become ready)
    for i in 1..=5 {
        abathur_cmd(dir)
            .args(["task", "submit", &format!("Limit task {}", i)])
            .assert()
            .success_without_warnings();
    }

    // List ready tasks with --limit 2
    let list = run_json(
        dir,
        &["task", "list", "--ready", "--limit", "2", "--json"],
    );

    let tasks = list["tasks"].as_array().expect("tasks should be an array");
    assert!(
        tasks.len() <= 2,
        "Should return at most 2 tasks, got {}",
        tasks.len()
    );
}

#[test]
fn task_show_nonexistent_fails() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    abathur_cmd(dir)
        .args(["task", "show", "00000000-0000-0000-0000-000000000000"])
        .assert()
        .failure();
}

#[test]
fn task_cancel_nonexistent_fails() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    abathur_cmd(dir)
        .args(["task", "cancel", "00000000-0000-0000-0000-000000000000"])
        .assert()
        .failure();
}

#[test]
fn task_submit_invalid_priority_fails() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    abathur_cmd(dir)
        .args(["task", "submit", "x", "--priority", "bogus"])
        .assert()
        .failure();
}

#[test]
fn task_submit_invalid_deadline_fails() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    abathur_cmd(dir)
        .args(["task", "submit", "x", "--deadline", "not-a-date"])
        .assert()
        .failure();
}
// ============================================================
// Task retry tests
// ============================================================

#[test]
fn task_retry_pending_task_fails() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    // Submit a task (no dependencies -> it starts in "ready" state)
    let create = run_json(
        dir,
        &["task", "submit", "Retry test prompt", "-t", "Retry Pending", "--json"],
    );
    let id = json_str(&create["task"], "id");

    // Attempting to retry a ready (non-failed) task should fail
    abathur_cmd(dir)
        .args(["task", "retry", &id])
        .assert()
        .failure()
        .stderr(predicates::str::contains("cannot be retried").or(predicates::str::contains("not failed")));
}

#[test]
fn task_retry_canceled_task_fails() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    // Submit and then cancel a task
    let create = run_json(
        dir,
        &["task", "submit", "Cancel then retry", "-t", "Retry Canceled", "--json"],
    );
    let id = json_str(&create["task"], "id");

    abathur_cmd(dir)
        .args(["task", "cancel", &id])
        .assert()
        .success_without_warnings();

    // Attempting to retry a canceled task should fail
    abathur_cmd(dir)
        .args(["task", "retry", &id])
        .assert()
        .failure()
        .stderr(predicates::str::contains("cannot be retried").or(predicates::str::contains("not failed")));
}

#[test]
fn task_retry_nonexistent_fails() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    // Use a valid UUID format that does not correspond to any task
    abathur_cmd(dir)
        .args(["task", "retry", "00000000-0000-0000-0000-000000000000"])
        .assert()
        .failure()
        .stderr(predicates::str::contains("not found").or(predicates::str::contains("Not found")));
}

#[test]
fn task_retry_missing_id_fails() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();

    // Clap should reject the command before any database access
    abathur_cmd(dir)
        .args(["task", "retry"])
        .assert()
        .failure()
        .stderr(predicates::str::is_match("required|Usage").unwrap());
}

// ============================================================
// Task list --ready tests
// ============================================================

#[test]
fn task_list_ready_json_output() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    // Submit a task with no dependencies (transitions to ready)
    let create = run_json(
        dir,
        &["task", "submit", "Ready task prompt", "-t", "Ready Task", "--json"],
    );
    let id = json_str(&create["task"], "id");

    // List with --ready should include it
    let json = run_json(dir, &["task", "list", "--ready", "--json"]);

    let tasks = json["tasks"].as_array().expect("tasks should be an array");
    assert!(tasks.len() >= 1, "Should have at least one ready task");
    assert!(json["total"].as_u64().unwrap() >= 1);

    // The submitted task (with no deps) should appear in the ready list
    assert!(
        tasks.iter().any(|t| json_str(t, "id") == id),
        "Ready list should include the no-dependency task"
    );

    // Every task in the ready list should have status "ready"
    for task in tasks {
        assert_eq!(
            json_str(task, "status"),
            "ready",
            "All tasks from --ready should have status 'ready'"
        );
    }
}

#[test]
fn task_list_ready_empty() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    // No tasks submitted at all
    let json = run_json(dir, &["task", "list", "--ready", "--json"]);

    let tasks = json["tasks"].as_array().expect("tasks should be an array");
    assert_eq!(tasks.len(), 0, "Should have no ready tasks");
    assert_eq!(json["total"].as_u64().unwrap(), 0);
}

#[test]
fn task_list_ready_excludes_canceled() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    // Submit a task and then cancel it
    let create = run_json(
        dir,
        &["task", "submit", "Cancel for ready test", "-t", "Canceled Ready", "--json"],
    );
    let id = json_str(&create["task"], "id");

    // Verify it was ready before canceling
    let before = run_json(dir, &["task", "list", "--ready", "--json"]);
    let before_tasks = before["tasks"].as_array().unwrap();
    assert!(
        before_tasks.iter().any(|t| json_str(t, "id") == id),
        "Task should initially be in the ready list"
    );

    // Cancel the task
    abathur_cmd(dir)
        .args(["task", "cancel", &id])
        .assert()
        .success_without_warnings();

    // The canceled task should no longer appear in --ready
    let after = run_json(dir, &["task", "list", "--ready", "--json"]);
    let after_tasks = after["tasks"].as_array().unwrap();
    assert!(
        !after_tasks.iter().any(|t| json_str(t, "id") == id),
        "Canceled task should not appear in --ready list"
    );
}

#[test]
fn task_list_ready_human_output() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    // Submit a task with no dependencies
    abathur_cmd(dir)
        .args(["task", "submit", "Human ready prompt", "-t", "Human Ready Task"])
        .assert()
        .success_without_warnings();

    // List with --ready in human mode should show the task
    abathur_cmd(dir)
        .args(["task", "list", "--ready"])
        .assert()
        .success_without_warnings()
        .stdout(predicates::str::contains("Human Ready Task"));
}

#[test]
fn task_list_ready_empty_human_output() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    // No tasks submitted; --ready should show "No tasks found"
    abathur_cmd(dir)
        .args(["task", "list", "--ready"])
        .assert()
        .success_without_warnings()
        .stdout(predicates::str::contains("No tasks found"));
}

// ============================================================
// Task list --agent filter tests
// ============================================================

#[test]
fn task_list_filter_by_agent_json() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    // Submit tasks with different agent types
    let create_a = run_json(
        dir,
        &["task", "submit", "Agent A prompt", "-t", "Agent A Task", "--agent", "coder", "--json"],
    );
    let id_a = json_str(&create_a["task"], "id");

    let create_b = run_json(
        dir,
        &["task", "submit", "Agent B prompt", "-t", "Agent B Task", "--agent", "reviewer", "--json"],
    );
    let id_b = json_str(&create_b["task"], "id");

    // Filter by agent "coder"
    let json = run_json(dir, &["task", "list", "--agent", "coder", "--json"]);

    let tasks = json["tasks"].as_array().expect("tasks should be an array");
    assert!(
        tasks.iter().any(|t| json_str(t, "id") == id_a),
        "Agent filter should include the 'coder' task"
    );
    assert!(
        !tasks.iter().any(|t| json_str(t, "id") == id_b),
        "Agent filter should exclude the 'reviewer' task"
    );

    // Verify all returned tasks have the correct agent type
    for task in tasks {
        assert_eq!(
            task["agent_type"].as_str().unwrap(),
            "coder",
            "All tasks should have agent_type 'coder'"
        );
    }
}

#[test]
fn task_list_filter_by_agent_no_match() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    // Submit a task with a specific agent
    abathur_cmd(dir)
        .args(["task", "submit", "Agent prompt", "-t", "Specific Agent", "--agent", "coder"])
        .assert()
        .success_without_warnings();

    // Filter by a non-existent agent type
    let json = run_json(dir, &["task", "list", "--agent", "nonexistent-agent", "--json"]);

    let tasks = json["tasks"].as_array().expect("tasks should be an array");
    assert_eq!(tasks.len(), 0, "No tasks should match a nonexistent agent");
    assert_eq!(json["total"].as_u64().unwrap(), 0);
}
// ============================================================
// Worktree command tests (git-dependent operations)
// ============================================================

/// Helper: initialize a git repo with an initial commit so that
/// git worktree operations have a valid HEAD to branch from.
fn setup_git_repo(dir: &std::path::Path) {
    std::process::Command::new("git")
        .args(["init"])
        .current_dir(dir)
        .output()
        .expect("git init failed");
    std::process::Command::new("git")
        .args(["config", "user.email", "test@test.com"])
        .current_dir(dir)
        .output()
        .unwrap();
    std::process::Command::new("git")
        .args(["config", "user.name", "Test"])
        .current_dir(dir)
        .output()
        .unwrap();
    std::fs::write(dir.join("README.md"), "test").unwrap();
    std::process::Command::new("git")
        .args(["add", "."])
        .current_dir(dir)
        .output()
        .unwrap();
    std::process::Command::new("git")
        .args(["commit", "-m", "initial"])
        .current_dir(dir)
        .output()
        .unwrap();
}

/// Helper: set up git repo + abathur init + submit a task, returning
/// the task ID.
fn setup_with_task(dir: &std::path::Path) -> String {
    setup_git_repo(dir);
    init_project(dir);
    let json = run_json(
        dir,
        &["task", "submit", "Worktree test task", "-t", "WT Task", "--json"],
    );
    json_str(&json["task"], "id")
}

#[test]
fn worktree_create_for_task() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    let task_id = setup_with_task(dir);

    abathur_cmd(dir)
        .args(["worktree", "create", &task_id])
        .assert()
        .success_without_warnings()
        .stdout(predicates::str::contains("Worktree created at:"));
}

#[test]
fn worktree_create_json_output() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    let task_id = setup_with_task(dir);

    let json = run_json(dir, &["worktree", "create", &task_id, "--json"]);

    assert_eq!(json["success"], true);
    let wt = &json["worktree"];
    assert!(wt["id"].as_str().is_some(), "worktree should have an id");
    assert_eq!(json_str(wt, "task_id"), task_id);
    assert!(
        wt["status"].as_str().is_some(),
        "worktree should have a status"
    );
    assert!(
        wt["path"].as_str().is_some(),
        "worktree should have a path"
    );
    assert!(
        wt["branch"].as_str().is_some(),
        "worktree should have a branch"
    );
}

#[test]
fn worktree_create_nonexistent_task_accepts_any_uuid() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    setup_git_repo(dir);
    init_project(dir);

    // The worktree system does not validate task existence before creating;
    // it simply creates a worktree record with the given task_id.
    abathur_cmd(dir)
        .args(["worktree", "create", "00000000-0000-0000-0000-000000000000"])
        .assert()
        .success_without_warnings()
        .stdout(predicates::str::contains("Worktree created at:"));
}

#[test]
fn worktree_show_displays_worktree() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    let task_id = setup_with_task(dir);

    // Create the worktree first
    abathur_cmd(dir)
        .args(["worktree", "create", &task_id])
        .assert()
        .success_without_warnings();

    // Show by task_id (resolve_worktree_id searches task_id column too)
    abathur_cmd(dir)
        .args(["worktree", "show", &task_id])
        .assert()
        .success_without_warnings()
        .stdout(
            predicates::str::contains("Worktree:")
                .and(predicates::str::contains("Task ID:"))
                .and(predicates::str::contains("Status:"))
                .and(predicates::str::contains("Path:"))
                .and(predicates::str::contains("Branch:")),
        );
}

#[test]
fn worktree_show_json_output() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    let task_id = setup_with_task(dir);

    // Create the worktree
    let create = run_json(dir, &["worktree", "create", &task_id, "--json"]);
    let wt_id = json_str(&create["worktree"], "id");

    // Show by task_id
    let json = run_json(dir, &["worktree", "show", &task_id, "--json"]);

    let wt = &json["worktree"];
    assert_eq!(json_str(wt, "id"), wt_id);
    assert_eq!(json_str(wt, "task_id"), task_id);
    assert!(wt["status"].as_str().is_some());
    assert!(wt["path"].as_str().is_some());
    assert!(wt["branch"].as_str().is_some());
    assert!(wt["base_ref"].as_str().is_some());
    assert!(wt["created_at"].as_str().is_some());
}

#[test]
fn worktree_show_nonexistent_reports_not_found() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    setup_git_repo(dir);
    init_project(dir);

    // Show for a nonexistent UUID exits 0 but prints "not found" message
    abathur_cmd(dir)
        .args(["worktree", "show", "00000000-0000-0000-0000-000000000000"])
        .assert()
        .success_without_warnings()
        .stdout(predicates::str::contains("not found"));
}

#[test]
fn worktree_complete_marks_completed() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    let task_id = setup_with_task(dir);

    // Create the worktree
    abathur_cmd(dir)
        .args(["worktree", "create", &task_id])
        .assert()
        .success_without_warnings();

    // Complete the worktree
    abathur_cmd(dir)
        .args(["worktree", "complete", &task_id])
        .assert()
        .success_without_warnings()
        .stdout(predicates::str::contains("Worktree marked as completed"));
}

#[test]
fn worktree_complete_json_output() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    let task_id = setup_with_task(dir);

    // Create the worktree
    abathur_cmd(dir)
        .args(["worktree", "create", &task_id])
        .assert()
        .success_without_warnings();

    // Complete the worktree with JSON output
    let json = run_json(dir, &["worktree", "complete", &task_id, "--json"]);

    assert_eq!(json["success"], true);
    let wt = &json["worktree"];
    assert_eq!(json_str(wt, "status"), "completed");
    assert_eq!(json_str(wt, "task_id"), task_id);
}

#[test]
fn worktree_cleanup_single() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    let task_id = setup_with_task(dir);

    // Create and complete the worktree (cleanup requires completed/merged status)
    let create = run_json(dir, &["worktree", "create", &task_id, "--json"]);
    let wt_id = json_str(&create["worktree"], "id");

    abathur_cmd(dir)
        .args(["worktree", "complete", &task_id])
        .assert()
        .success_without_warnings();

    // Cleanup by worktree ID
    abathur_cmd(dir)
        .args(["worktree", "cleanup", &wt_id])
        .assert()
        .success_without_warnings()
        .stdout(predicates::str::contains("Worktree cleaned up"));
}

#[test]
fn worktree_cleanup_nonexistent_fails() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    setup_git_repo(dir);
    init_project(dir);

    // A valid UUID that does not match any worktree
    abathur_cmd(dir)
        .args(["worktree", "cleanup", "00000000-0000-0000-0000-000000000000"])
        .assert()
        .failure();
}

#[test]
fn worktree_list_active_flag() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    let task_id = setup_with_task(dir);

    // Create a worktree so there is an active one
    abathur_cmd(dir)
        .args(["worktree", "create", &task_id])
        .assert()
        .success_without_warnings();

    // List with --active
    let json = run_json(dir, &["worktree", "list", "--active", "--json"]);

    let worktrees = json["worktrees"]
        .as_array()
        .expect("worktrees should be an array");
    assert!(
        json["total"].as_u64().unwrap() >= 1,
        "Should have at least one active worktree"
    );
    // Verify the worktree for our task appears in the list
    assert!(
        worktrees
            .iter()
            .any(|wt| json_str(wt, "task_id") == task_id),
        "Active list should contain the worktree for our task"
    );
}

#[test]
fn worktree_create_missing_task_id_fails() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    setup_git_repo(dir);
    init_project(dir);

    // Missing required positional argument: clap should reject this
    abathur_cmd(dir)
        .args(["worktree", "create"])
        .assert()
        .failure()
        .stderr(predicates::str::contains("Usage"));
}

#[test]
fn worktree_help_subcommands() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();

    // Top-level worktree help
    abathur_cmd(dir)
        .args(["worktree", "--help"])
        .assert()
        .success_without_warnings()
        .stdout(
            predicates::str::contains("create")
                .and(predicates::str::contains("list"))
                .and(predicates::str::contains("show"))
                .and(predicates::str::contains("complete"))
                .and(predicates::str::contains("merge"))
                .and(predicates::str::contains("cleanup")),
        );

    // Subcommand-level help
    abathur_cmd(dir)
        .args(["worktree", "create", "--help"])
        .assert()
        .success_without_warnings()
        .stdout(predicates::str::contains("TASK_ID"));

    abathur_cmd(dir)
        .args(["worktree", "show", "--help"])
        .assert()
        .success_without_warnings()
        .stdout(predicates::str::contains("ID"));

    abathur_cmd(dir)
        .args(["worktree", "complete", "--help"])
        .assert()
        .success_without_warnings()
        .stdout(predicates::str::contains("TASK_ID"));

    abathur_cmd(dir)
        .args(["worktree", "cleanup", "--help"])
        .assert()
        .success_without_warnings()
        .stdout(predicates::str::contains("ID"));
}
// ============================================================
// Agent Send command tests
// ============================================================

#[test]
fn agent_send_missing_required_args_fails() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();

    // Missing body, --to, and --subject should fail at clap validation
    abathur_cmd(dir)
        .args(["agent", "send"])
        .assert()
        .failure()
        .stderr(predicates::str::is_match("required|Usage").unwrap());
}

#[test]
fn agent_send_missing_to_fails() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();

    // Provide body and --subject but omit --to
    abathur_cmd(dir)
        .args(["agent", "send", "Hello world", "--subject", "test-subject"])
        .assert()
        .failure()
        .stderr(predicates::str::is_match("required|--to").unwrap());
}

#[test]
fn agent_send_missing_subject_fails() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();

    // Provide body and --to but omit --subject
    abathur_cmd(dir)
        .args(["agent", "send", "Hello world", "--to", "agent-1"])
        .assert()
        .failure()
        .stderr(predicates::str::is_match("required|--subject").unwrap());
}

#[test]
fn agent_send_missing_body_fails() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();

    // Provide --to and --subject but omit the positional body argument
    abathur_cmd(dir)
        .args(["agent", "send", "--to", "agent-1", "--subject", "test-subject"])
        .assert()
        .failure()
        .stderr(predicates::str::is_match("required|Usage").unwrap());
}

#[test]
fn agent_send_help_shows_usage() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();

    abathur_cmd(dir)
        .args(["agent", "send", "--help"])
        .assert()
        .success_without_warnings()
        .stdout(
            predicates::str::contains("Usage")
                .and(predicates::str::contains("--to"))
                .and(predicates::str::contains("--subject"))
                .and(predicates::str::contains("send")),
        );
}

#[test]
fn agent_send_gateway_unavailable_succeeds_with_failure_message() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    // Send a message to a gateway that is not running.
    // The command should succeed (exit 0) but report success: false in output.
    let json = run_json(
        dir,
        &[
            "agent", "send", "Hello world",
            "--to", "agent-1",
            "--subject", "test-subject",
            "--gateway", "http://127.0.0.1:19999",
            "--json",
        ],
    );

    assert_eq!(
        json["success"].as_bool().unwrap(),
        false,
        "send should report success: false when gateway is unreachable"
    );
    assert!(json["message_id"].as_str().is_some(), "should include a message_id");
    let msg = json["message"].as_str().unwrap();
    assert!(
        msg.contains("connect") || msg.contains("gateway") || msg.contains("Failed"),
        "message should indicate a connection failure, got: {}",
        msg
    );
}

#[test]
fn agent_send_gateway_unavailable_human_output() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    // Without --json, the human-readable output should mention the failure
    abathur_cmd(dir)
        .args([
            "agent", "send", "Hello world",
            "--to", "agent-1",
            "--subject", "test-subject",
            "--gateway", "http://127.0.0.1:19999",
        ])
        .assert()
        .success_without_warnings()
        .stdout(predicates::str::is_match("(?i)fail|connect|gateway").unwrap());
}

#[test]
fn agent_send_with_message_type_json() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    // Verify custom --message-type is accepted by clap even when gateway is down
    let json = run_json(
        dir,
        &[
            "agent", "send", "Error details",
            "--to", "agent-1",
            "--subject", "error-report",
            "--message-type", "error",
            "--gateway", "http://127.0.0.1:19999",
            "--json",
        ],
    );

    assert_eq!(json["success"].as_bool().unwrap(), false);
    assert!(json["message_id"].as_str().is_some());
}

#[test]
fn agent_send_with_from_and_task_id_json() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    // Use --from and --task-id options. task-id resolves to the UUID directly.
    // The send fails because the gateway is unreachable but exits 0 with success: false.
    let json = run_json(
        dir,
        &[
            "agent", "send", "Progress update",
            "--to", "agent-2",
            "--subject", "progress",
            "--from", "agent-1",
            "--task-id", "00000000-0000-0000-0000-000000000000",
            "--gateway", "http://127.0.0.1:19999",
            "--json",
        ],
    );

    assert_eq!(json["success"].as_bool().unwrap(), false);
    assert!(json["message_id"].as_str().is_some());
}

// ============================================================
// Agent Cards command tests
// ============================================================

#[test]
fn agent_cards_list_help() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();

    abathur_cmd(dir)
        .args(["agent", "cards", "list", "--help"])
        .assert()
        .success_without_warnings()
        .stdout(
            predicates::str::contains("Usage")
                .and(predicates::str::contains("list")),
        );
}

#[test]
fn agent_cards_export_help() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();

    abathur_cmd(dir)
        .args(["agent", "cards", "export", "--help"])
        .assert()
        .success_without_warnings()
        .stdout(
            predicates::str::contains("Usage")
                .and(predicates::str::contains("export")),
        );
}

#[test]
fn agent_cards_show_help() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();

    abathur_cmd(dir)
        .args(["agent", "cards", "show", "--help"])
        .assert()
        .success_without_warnings()
        .stdout(
            predicates::str::contains("Usage")
                .and(predicates::str::contains("show"))
                .and(predicates::str::contains("AGENT_ID")),
        );
}

#[test]
fn agent_cards_show_missing_id_fails() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();

    // Missing positional agent_id argument
    abathur_cmd(dir)
        .args(["agent", "cards", "show"])
        .assert()
        .failure()
        .stderr(predicates::str::is_match("required|Usage").unwrap());
}

#[test]
fn agent_cards_list_gateway_unavailable_json() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    // cards list should succeed (exit 0) but report failure when gateway is down
    let json = run_json(
        dir,
        &["agent", "cards", "list", "--gateway", "http://127.0.0.1:19999", "--json"],
    );

    assert_eq!(
        json["success"].as_bool().unwrap(),
        false,
        "cards list should report success: false when gateway is unreachable"
    );
    let msg = json["message"].as_str().unwrap();
    assert!(
        msg.contains("connect") || msg.contains("gateway") || msg.contains("Cannot"),
        "message should indicate a connection failure, got: {}",
        msg
    );
}

#[test]
fn agent_cards_list_gateway_unavailable_human() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    abathur_cmd(dir)
        .args(["agent", "cards", "list", "--gateway", "http://127.0.0.1:19999"])
        .assert()
        .success_without_warnings()
        .stdout(predicates::str::is_match("(?i)cannot connect|gateway|fail").unwrap());
}

#[test]
fn agent_cards_export_gateway_unavailable_json() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    let json = run_json(
        dir,
        &["agent", "cards", "export", "--gateway", "http://127.0.0.1:19999", "--json"],
    );

    assert_eq!(
        json["success"].as_bool().unwrap(),
        false,
        "cards export should report success: false when gateway is unreachable"
    );
    assert!(json["message"].as_str().is_some());
}

#[test]
fn agent_cards_show_gateway_unavailable_json() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    let json = run_json(
        dir,
        &[
            "agent", "cards", "show", "some-agent-id",
            "--gateway", "http://127.0.0.1:19999",
            "--json",
        ],
    );

    assert_eq!(
        json["success"].as_bool().unwrap(),
        false,
        "cards show should report success: false when gateway is unreachable"
    );
    let msg = json["message"].as_str().unwrap();
    assert!(
        msg.contains("connect") || msg.contains("gateway") || msg.contains("Cannot"),
        "message should indicate a connection failure, got: {}",
        msg
    );
}

#[test]
fn agent_cards_help_shows_subcommands() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();

    abathur_cmd(dir)
        .args(["agent", "cards", "--help"])
        .assert()
        .success_without_warnings()
        .stdout(
            predicates::str::contains("list")
                .and(predicates::str::contains("export"))
                .and(predicates::str::contains("show")),
        );
}

// ============================================================
// Event DLQ Retry command tests
// ============================================================

#[test]
fn event_dlq_retry_nonexistent_succeeds_silently() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    // The DLQ retry command resolves (marks as resolved) an entry by ID.
    // When the ID doesn't exist, the SQL update affects 0 rows but doesn't error.
    abathur_cmd(dir)
        .args(["event", "dlq", "retry", "00000000-0000-0000-0000-000000000000"])
        .assert()
        .success_without_warnings()
        .stdout(predicates::str::contains("Resolved DLQ entry"));
}

#[test]
fn event_dlq_retry_json_output() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    let json = run_json(dir, &["event", "dlq", "retry", "00000000-0000-0000-0000-000000000000", "--json"]);

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
    let msg = json["message"].as_str().expect("should include a message field");
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
        &["event", "dlq", "retry-all", "--handler", "some-handler", "--json"],
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
        .stdout(
            predicates::str::contains("Usage")
                .and(predicates::str::contains("retry")),
        );
}

#[test]
fn event_dlq_retry_all_help_shows_usage() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();

    abathur_cmd(dir)
        .args(["event", "dlq", "retry-all", "--help"])
        .assert()
        .success_without_warnings()
        .stdout(
            predicates::str::contains("Usage")
                .and(predicates::str::contains("retry-all")),
        );
}
// ============================================================
// Swarm start command tests
// ============================================================

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

// ============================================================
// Swarm respond command tests
// ============================================================

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
        .args(["swarm", "respond", "--id", "00000000-0000-0000-0000-000000000000"])
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

// ============================================================
// MCP server help tests (validates arg parsing without
// starting long-running servers)
// ============================================================

#[test]
fn mcp_memory_http_help() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();

    abathur_cmd(dir)
        .args(["mcp", "memory-http", "--help"])
        .assert()
        .success_without_warnings()
        .stdout(
            predicates::str::contains("Usage")
                .and(predicates::str::contains("memory-http"))
                .and(predicates::str::contains("--port"))
                .and(predicates::str::contains("--host")),
        );
}

#[test]
fn mcp_tasks_http_help() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();

    abathur_cmd(dir)
        .args(["mcp", "tasks-http", "--help"])
        .assert()
        .success_without_warnings()
        .stdout(
            predicates::str::contains("Usage")
                .and(predicates::str::contains("tasks-http"))
                .and(predicates::str::contains("--port"))
                .and(predicates::str::contains("--host")),
        );
}

#[test]
fn mcp_agents_http_help() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();

    abathur_cmd(dir)
        .args(["mcp", "agents-http", "--help"])
        .assert()
        .success_without_warnings()
        .stdout(
            predicates::str::contains("Usage")
                .and(predicates::str::contains("agents-http"))
                .and(predicates::str::contains("--port"))
                .and(predicates::str::contains("--host")),
        );
}

#[test]
fn mcp_a2a_http_help() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();

    abathur_cmd(dir)
        .args(["mcp", "a2a-http", "--help"])
        .assert()
        .success_without_warnings()
        .stdout(
            predicates::str::contains("Usage")
                .and(predicates::str::contains("a2a-http"))
                .and(predicates::str::contains("--port"))
                .and(predicates::str::contains("--host"))
                .and(predicates::str::contains("--no-streaming"))
                .and(predicates::str::contains("--heartbeat-ms")),
        );
}

#[test]
fn mcp_all_help() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();

    abathur_cmd(dir)
        .args(["mcp", "all", "--help"])
        .assert()
        .success_without_warnings()
        .stdout(
            predicates::str::contains("Usage")
                .and(predicates::str::contains("all"))
                .and(predicates::str::contains("--memory-port"))
                .and(predicates::str::contains("--tasks-port"))
                .and(predicates::str::contains("--agents-port"))
                .and(predicates::str::contains("--a2a-port")),
        );
}

#[test]
fn mcp_stdio_help() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();

    abathur_cmd(dir)
        .args(["mcp", "stdio", "--help"])
        .assert()
        .success_without_warnings()
        .stdout(
            predicates::str::contains("Usage")
                .and(predicates::str::contains("stdio"))
                .and(predicates::str::contains("--db-path"))
                .and(predicates::str::contains("--task-id")),
        );
}

#[test]
fn mcp_stdio_missing_db_path_fails() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();

    // mcp stdio requires --db-path; clap should reject without it
    abathur_cmd(dir)
        .args(["mcp", "stdio"])
        .assert()
        .failure()
        .stderr(predicates::str::is_match("required|--db-path").unwrap());
}
