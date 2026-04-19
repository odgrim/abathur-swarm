//! Tests for `abathur task ...`.

use super::{AssertExt, abathur_cmd, init_project, json_str, run_json};
use predicates::prelude::*;
use tempfile::TempDir;

#[test]
fn task_submit_creates_task() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    abathur_cmd(dir)
        .args(["task", "submit", "Do something", "-t", "Test task"])
        .assert()
        .success_without_warnings()
        .stdout(predicates::str::contains("Task created"));
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
    assert!(
        task["status"].as_str().is_some(),
        "task should have a status"
    );
}

#[test]
fn task_submit_with_title_json() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    let json = run_json(
        dir,
        &[
            "task",
            "submit",
            "Do something specific",
            "-t",
            "My Title",
            "--json",
        ],
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
        &[
            "task",
            "submit",
            "Show task prompt",
            "-t",
            "Show Task",
            "--json",
        ],
    );
    let id = json_str(&json["task"], "id");

    abathur_cmd(dir)
        .args(["task", "show", &id])
        .assert()
        .success_without_warnings()
        .stdout(predicates::str::contains("Show Task").and(predicates::str::contains(&id[..8])));
}

#[test]
fn task_show_json_output() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    let create = run_json(
        dir,
        &[
            "task",
            "submit",
            "Show JSON prompt",
            "-t",
            "Show JSON Task",
            "--json",
        ],
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
        &[
            "task",
            "submit",
            "Cancel me JSON",
            "-t",
            "Cancel JSON",
            "--json",
        ],
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
        &[
            "task",
            "submit",
            "Lifecycle prompt",
            "-t",
            "Lifecycle Task",
            "--json",
        ],
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

#[test]
fn task_submit_missing_prompt_fails() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    abathur_cmd(dir)
        .args(["task", "submit"])
        .assert()
        .failure()
        .stderr(predicates::str::contains(
            "provide either a prompt or --file",
        ));
}

#[test]
fn task_submit_file_flag_reads_prompt() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    let prompt_file = dir.join("prompt.txt");
    std::fs::write(&prompt_file, "Task from file").unwrap();

    let json = run_json(
        dir,
        &[
            "task",
            "submit",
            "-f",
            prompt_file.to_str().unwrap(),
            "--json",
        ],
    );
    assert_eq!(json["success"], true);
    assert!(json["task"]["id"].as_str().is_some());
}

#[test]
fn task_submit_file_flag_nonexistent_file_fails() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    abathur_cmd(dir)
        .args(["task", "submit", "-f", "nonexistent.txt"])
        .assert()
        .failure()
        .stderr(predicates::str::contains("failed to read file"));
}

#[test]
fn task_submit_file_flag_conflicts_with_prompt() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    let prompt_file = dir.join("prompt.txt");
    std::fs::write(&prompt_file, "Task from file").unwrap();

    abathur_cmd(dir)
        .args([
            "task",
            "submit",
            "inline prompt",
            "-f",
            prompt_file.to_str().unwrap(),
        ])
        .assert()
        .failure()
        .stderr(predicates::str::contains("cannot be used with"));
}

#[test]
fn task_submit_file_flag_empty_file_fails() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    let prompt_file = dir.join("empty.txt");
    std::fs::write(&prompt_file, "   ").unwrap();

    abathur_cmd(dir)
        .args(["task", "submit", "-f", prompt_file.to_str().unwrap()])
        .assert()
        .failure()
        .stderr(predicates::str::contains(
            "task description cannot be empty",
        ));
}

#[test]
fn task_submit_no_prompt_no_file_fails() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    abathur_cmd(dir)
        .args(["task", "submit"])
        .assert()
        .failure()
        .stderr(predicates::str::contains(
            "provide either a prompt or --file",
        ));
}

#[test]
fn task_help_shows_usage() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();

    abathur_cmd(dir)
        .args(["task", "--help"])
        .assert()
        .success_without_warnings()
        .stdout(predicates::str::contains("Usage").and(predicates::str::contains("task")));
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
fn task_show_missing_id_fails() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();

    abathur_cmd(dir)
        .args(["task", "show"])
        .assert()
        .failure()
        .stderr(predicates::str::is_match("required|Usage").unwrap());
}

#[test]
fn task_show_by_prefix() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    let create = run_json(
        dir,
        &[
            "task",
            "submit",
            "Prefix task prompt",
            "-t",
            "Prefix Task",
            "--json",
        ],
    );
    let full_id = json_str(&create["task"], "id");
    let prefix = &full_id[..8];

    let show = run_json(dir, &["task", "show", prefix, "--json"]);

    assert_eq!(json_str(&show["task"], "id"), full_id);
    assert_eq!(json_str(&show["task"], "title"), "Prefix Task");
}

#[test]
fn task_submit_with_priority_json() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    let json = run_json(
        dir,
        &[
            "task",
            "submit",
            "High priority work",
            "--priority",
            "high",
            "--json",
        ],
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
        &[
            "task",
            "submit",
            "Critical work",
            "--priority",
            "critical",
            "--json",
        ],
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
        &[
            "task",
            "submit",
            "Agent work",
            "--agent",
            "my-worker",
            "--json",
        ],
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
        &[
            "task",
            "submit",
            "Input work",
            "--input",
            "some context",
            "--json",
        ],
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
        &[
            "task",
            "submit",
            "Dedup task",
            "--idempotency-key",
            "dup",
            "--json",
        ],
    );
    assert_eq!(first["success"], true);
    let first_id = json_str(&first["task"], "id");

    // Submit second task with the same idempotency key
    let second = run_json(
        dir,
        &[
            "task",
            "submit",
            "Dedup task again",
            "--idempotency-key",
            "dup",
            "--json",
        ],
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
        &[
            "task",
            "submit",
            "Deadline work",
            "--deadline",
            "2030-12-31T23:59:59Z",
            "--json",
        ],
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
        &[
            "task",
            "submit",
            "Parent task prompt",
            "-t",
            "Parent Task",
            "--json",
        ],
    );
    assert_eq!(parent["success"], true);
    let parent_id = json_str(&parent["task"], "id");

    // Create child task with --parent
    let child = run_json(
        dir,
        &[
            "task",
            "submit",
            "Child task prompt",
            "-t",
            "Child Task",
            "--parent",
            &parent_id,
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
            "task",
            "submit",
            "Task B prompt",
            "-t",
            "Task B",
            "--depends-on",
            &a_id,
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
        &[
            "task",
            "submit",
            "Cancel me",
            "-t",
            "Cancelable Task",
            "--json",
        ],
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
    assert!(
        !tasks.is_empty(),
        "Should find at least one high-priority task"
    );
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
    let list = run_json(dir, &["task", "list", "--ready", "--limit", "2", "--json"]);

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

#[test]
fn task_retry_pending_task_fails() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    // Submit a task (no dependencies -> it starts in "ready" state)
    let create = run_json(
        dir,
        &[
            "task",
            "submit",
            "Retry test prompt",
            "-t",
            "Retry Pending",
            "--json",
        ],
    );
    let id = json_str(&create["task"], "id");

    // Attempting to retry a ready (non-failed) task should fail
    abathur_cmd(dir)
        .args(["task", "retry", &id])
        .assert()
        .failure()
        .stderr(
            predicates::str::contains("cannot be retried")
                .or(predicates::str::contains("not failed")),
        );
}

#[test]
fn task_retry_canceled_task_fails() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    // Submit and then cancel a task
    let create = run_json(
        dir,
        &[
            "task",
            "submit",
            "Cancel then retry",
            "-t",
            "Retry Canceled",
            "--json",
        ],
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
        .stderr(
            predicates::str::contains("cannot be retried")
                .or(predicates::str::contains("not failed")),
        );
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

#[test]
fn task_list_ready_json_output() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    // Submit a task with no dependencies (transitions to ready)
    let create = run_json(
        dir,
        &[
            "task",
            "submit",
            "Ready task prompt",
            "-t",
            "Ready Task",
            "--json",
        ],
    );
    let id = json_str(&create["task"], "id");

    // List with --ready should include it
    let json = run_json(dir, &["task", "list", "--ready", "--json"]);

    let tasks = json["tasks"].as_array().expect("tasks should be an array");
    assert!(!tasks.is_empty(), "Should have at least one ready task");
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
        &[
            "task",
            "submit",
            "Cancel for ready test",
            "-t",
            "Canceled Ready",
            "--json",
        ],
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
        .args([
            "task",
            "submit",
            "Human ready prompt",
            "-t",
            "Human Ready Task",
        ])
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

#[test]
fn task_list_filter_by_agent_json() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    // Submit tasks with different agent types
    let create_a = run_json(
        dir,
        &[
            "task",
            "submit",
            "Agent A prompt",
            "-t",
            "Agent A Task",
            "--agent",
            "coder",
            "--json",
        ],
    );
    let id_a = json_str(&create_a["task"], "id");

    let create_b = run_json(
        dir,
        &[
            "task",
            "submit",
            "Agent B prompt",
            "-t",
            "Agent B Task",
            "--agent",
            "reviewer",
            "--json",
        ],
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
        .args([
            "task",
            "submit",
            "Agent prompt",
            "-t",
            "Specific Agent",
            "--agent",
            "coder",
        ])
        .assert()
        .success_without_warnings();

    // Filter by a non-existent agent type
    let json = run_json(
        dir,
        &["task", "list", "--agent", "nonexistent-agent", "--json"],
    );

    let tasks = json["tasks"].as_array().expect("tasks should be an array");
    assert_eq!(tasks.len(), 0, "No tasks should match a nonexistent agent");
    assert_eq!(json["total"].as_u64().unwrap(), 0);
}

#[test]
fn task_list_limit_without_ready_flag() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    // Submit 5 tasks
    for i in 1..=5 {
        abathur_cmd(dir)
            .args(["task", "submit", &format!("Limit test {}", i)])
            .assert()
            .success_without_warnings();
    }

    // List with -l 3 (short flag, no --ready)
    let list = run_json(dir, &["task", "list", "-l", "3", "--json"]);
    let tasks = list["tasks"].as_array().expect("tasks should be an array");
    assert_eq!(
        tasks.len(),
        3,
        "Should return exactly 3 tasks, got {}",
        tasks.len()
    );

    // Verify total reflects the limited count
    assert_eq!(list["total"].as_u64().unwrap(), 3);
}

#[test]
fn task_list_limit_with_status_filter() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    // Submit 4 tasks, cancel 2 of them
    let mut ids = Vec::new();
    for i in 1..=4 {
        let create = run_json(
            dir,
            &["task", "submit", &format!("Status limit {}", i), "--json"],
        );
        ids.push(json_str(&create["task"], "id"));
    }
    // Cancel first two
    run_json(dir, &["task", "cancel", &ids[0], "--json"]);
    run_json(dir, &["task", "cancel", &ids[1], "--json"]);

    // List canceled tasks with limit 1
    let list = run_json(
        dir,
        &["task", "list", "--status", "canceled", "-l", "1", "--json"],
    );
    let tasks = list["tasks"].as_array().expect("tasks should be an array");
    assert_eq!(tasks.len(), 1, "Should return exactly 1 canceled task");
    assert_eq!(json_str(&tasks[0], "status"), "canceled");
}

#[test]
fn task_list_limit_larger_than_result_set() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    // Submit 2 tasks
    for i in 1..=2 {
        abathur_cmd(dir)
            .args(["task", "submit", &format!("Few tasks {}", i)])
            .assert()
            .success_without_warnings();
    }

    // List with limit 100 — should return all 2
    let list = run_json(dir, &["task", "list", "-l", "100", "--json"]);
    let tasks = list["tasks"].as_array().expect("tasks should be an array");
    assert_eq!(
        tasks.len(),
        2,
        "Should return all 2 tasks when limit exceeds count"
    );
}

#[test]
fn task_list_filter_by_type_json() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    // Submit tasks (they default to "standard" type)
    abathur_cmd(dir)
        .args(["task", "submit", "Standard task one"])
        .assert()
        .success_without_warnings();
    abathur_cmd(dir)
        .args(["task", "submit", "Standard task two"])
        .assert()
        .success_without_warnings();

    // Filter by --type standard
    let list = run_json(dir, &["task", "list", "--type", "standard", "--json"]);
    let tasks = list["tasks"].as_array().expect("tasks should be an array");
    assert!(
        tasks.len() >= 2,
        "Should find at least 2 standard-type tasks"
    );
    for task in tasks {
        assert_eq!(json_str(task, "task_type"), "standard");
    }
}

#[test]
fn task_list_filter_by_type_no_match() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    abathur_cmd(dir)
        .args(["task", "submit", "A normal task"])
        .assert()
        .success_without_warnings();

    // Filter by type "verification" — should return none (submitted tasks are "standard")
    let list = run_json(dir, &["task", "list", "--type", "verification", "--json"]);
    let tasks = list["tasks"].as_array().expect("tasks should be an array");
    assert_eq!(tasks.len(), 0, "Should find no verification tasks");
}

#[test]
fn task_list_combined_status_limit_json() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    // Submit 4 tasks — all will be "ready" (no deps)
    for i in 1..=4 {
        abathur_cmd(dir)
            .args(["task", "submit", &format!("Combo task {}", i)])
            .assert()
            .success_without_warnings();
    }

    // Combine --status ready --limit 2 --json
    let list = run_json(
        dir,
        &["task", "list", "--status", "ready", "-l", "2", "--json"],
    );
    let tasks = list["tasks"].as_array().expect("tasks should be an array");
    assert_eq!(tasks.len(), 2, "Should return exactly 2 tasks");
    for task in tasks {
        assert_eq!(json_str(task, "status"), "ready");
    }
}

#[test]
fn task_list_combined_priority_limit_json() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    // Submit tasks with different priorities
    for i in 1..=3 {
        abathur_cmd(dir)
            .args([
                "task",
                "submit",
                &format!("High task {}", i),
                "--priority",
                "high",
            ])
            .assert()
            .success_without_warnings();
    }
    abathur_cmd(dir)
        .args(["task", "submit", "Low task", "--priority", "low"])
        .assert()
        .success_without_warnings();

    // Filter by priority high + limit 2
    let list = run_json(
        dir,
        &["task", "list", "--priority", "high", "-l", "2", "--json"],
    );
    let tasks = list["tasks"].as_array().expect("tasks should be an array");
    assert_eq!(
        tasks.len(),
        2,
        "Should return exactly 2 high-priority tasks"
    );
    for task in tasks {
        assert_eq!(json_str(task, "priority"), "high");
    }
}

#[test]
fn task_list_combined_agent_limit_json() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    // Submit tasks with a specific agent
    for i in 1..=3 {
        abathur_cmd(dir)
            .args([
                "task",
                "submit",
                &format!("Agent task {}", i),
                "--agent",
                "test-agent",
            ])
            .assert()
            .success_without_warnings();
    }
    // And one without agent
    abathur_cmd(dir)
        .args(["task", "submit", "No agent task"])
        .assert()
        .success_without_warnings();

    // Filter by agent + limit
    let list = run_json(
        dir,
        &["task", "list", "--agent", "test-agent", "-l", "2", "--json"],
    );
    let tasks = list["tasks"].as_array().expect("tasks should be an array");
    assert_eq!(
        tasks.len(),
        2,
        "Should return exactly 2 agent-filtered tasks"
    );
    for task in tasks {
        assert_eq!(json_str(task, "agent_type"), "test-agent");
    }
}

#[test]
fn task_list_human_output_with_limit() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    for i in 1..=5 {
        abathur_cmd(dir)
            .args(["task", "submit", &format!("Human output {}", i)])
            .assert()
            .success_without_warnings();
    }

    // Non-JSON output with limit — verify it succeeds and shows limited results
    let output = abathur_cmd(dir)
        .args(["task", "list", "-l", "2"])
        .assert()
        .success_without_warnings()
        .get_output()
        .stdout
        .clone();

    let stdout = String::from_utf8(output).unwrap();
    // The table header says "Showing X of Y" — verify we see "2"
    assert!(
        stdout.contains("2 tasks"),
        "Human output should reflect the limit; got: {}",
        stdout
    );
}
