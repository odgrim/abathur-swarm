//! Tests for `abathur worktree ...`.

use super::{AssertExt, abathur_cmd, init_project, json_str, run_json};
use predicates::prelude::*;
use tempfile::TempDir;

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
        &[
            "task",
            "submit",
            "Worktree test task",
            "-t",
            "WT Task",
            "--json",
        ],
    );
    json_str(&json["task"], "id")
}

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

#[test]
fn worktree_help_shows_usage() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();

    abathur_cmd(dir)
        .args(["worktree", "--help"])
        .assert()
        .success_without_warnings()
        .stdout(predicates::str::contains("Usage").and(predicates::str::contains("worktree")));
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
    assert!(wt["path"].as_str().is_some(), "worktree should have a path");
    assert!(
        wt["branch"].as_str().is_some(),
        "worktree should have a branch"
    );
}

#[test]
fn worktree_create_nonexistent_task_rejects_missing_task() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    setup_git_repo(dir);
    init_project(dir);

    // With FK constraints on worktrees.task_id, creating a worktree for a
    // nonexistent task should fail with a foreign key constraint error.
    abathur_cmd(dir)
        .args(["worktree", "create", "00000000-0000-0000-0000-000000000000"])
        .assert()
        .failure();
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
            predicates::str::contains("Worktree")
                .and(predicates::str::contains("Task ID"))
                .and(predicates::str::contains("Status"))
                .and(predicates::str::contains("Path"))
                .and(predicates::str::contains("Branch")),
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
        .args([
            "worktree",
            "cleanup",
            "00000000-0000-0000-0000-000000000000",
        ])
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
