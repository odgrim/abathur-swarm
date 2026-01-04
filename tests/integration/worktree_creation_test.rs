//! Integration tests for git worktree creation
//!
//! Tests the git operations used by `create_branch_for_step()` in agent_executor.
//! These tests validate actual git behavior in temp directories.

use std::path::Path;
use std::process::Command;

use crate::common::{setup_test_git_repo, setup_test_git_repo_with_branch};

/// Helper to run git command and get output
fn git_output(repo_path: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(args)
        .current_dir(repo_path)
        .output()
        .expect("Failed to run git command");
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

/// Helper to check if git command succeeds
fn git_succeeds(repo_path: &Path, args: &[&str]) -> bool {
    Command::new("git")
        .args(args)
        .current_dir(repo_path)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Helper to create a worktree, mimicking create_branch_for_step logic
fn create_worktree_with_branch(
    repo_path: &Path,
    branch_name: &str,
    parent_ref: &str,
    worktree_subpath: &str,
) -> Result<std::path::PathBuf, String> {
    let worktree_path = repo_path.join(worktree_subpath);

    // Check if branch exists
    let branch_exists = git_succeeds(repo_path, &["rev-parse", "--verify", branch_name]);

    if branch_exists {
        // Check if branch is already in a worktree
        let worktree_list = git_output(repo_path, &["worktree", "list", "--porcelain"]);

        // Parse worktree list to find if this branch is checked out
        let mut current_wt_path: Option<String> = None;
        for line in worktree_list.lines() {
            if let Some(path) = line.strip_prefix("worktree ") {
                current_wt_path = Some(path.to_string());
            } else if let Some(branch_ref) = line.strip_prefix("branch refs/heads/") {
                if branch_ref == branch_name {
                    if let Some(ref existing_path) = current_wt_path {
                        return Ok(std::path::PathBuf::from(existing_path));
                    }
                }
            }
        }

        // Branch exists but not in worktree, create worktree from it
        let output = Command::new("git")
            .args(["worktree", "add", worktree_path.to_str().unwrap(), branch_name])
            .current_dir(repo_path)
            .output()
            .map_err(|e| e.to_string())?;

        if !output.status.success() {
            return Err(String::from_utf8_lossy(&output.stderr).to_string());
        }
    } else {
        // Create new branch and worktree atomically
        let output = Command::new("git")
            .args([
                "worktree",
                "add",
                "-b",
                branch_name,
                worktree_path.to_str().unwrap(),
                parent_ref,
            ])
            .current_dir(repo_path)
            .output()
            .map_err(|e| e.to_string())?;

        if !output.status.success() {
            return Err(String::from_utf8_lossy(&output.stderr).to_string());
        }
    }

    Ok(worktree_path)
}

#[test]
fn test_git_repo_helper_creates_valid_repo() {
    let (_dir, repo_path) = setup_test_git_repo();

    // Verify it's a valid git repo
    assert!(git_succeeds(&repo_path, &["status"]));

    // Verify we have at least one commit
    let log = git_output(&repo_path, &["log", "--oneline", "-1"]);
    assert!(!log.is_empty(), "Should have at least one commit");
}

#[test]
fn test_git_repo_helper_with_custom_branch() {
    let (_dir, repo_path) = setup_test_git_repo_with_branch("main");

    let branch = git_output(&repo_path, &["branch", "--show-current"]);
    assert_eq!(branch, "main");
}

#[test]
fn test_create_branch_from_main() {
    let (_dir, repo_path) = setup_test_git_repo_with_branch("main");

    // Create worktree directory
    std::fs::create_dir_all(repo_path.join(".abathur/worktrees")).unwrap();

    // Create a new branch with worktree
    let result = create_worktree_with_branch(
        &repo_path,
        "feature/test-feature",
        "main",
        ".abathur/worktrees/feature-test-feature",
    );

    assert!(result.is_ok(), "Failed to create worktree: {:?}", result.err());

    let worktree_path = result.unwrap();
    assert!(worktree_path.exists(), "Worktree directory should exist");

    // Verify the branch was created
    assert!(git_succeeds(&repo_path, &["rev-parse", "--verify", "feature/test-feature"]));

    // Verify the worktree is listed
    let worktree_list = git_output(&repo_path, &["worktree", "list"]);
    assert!(
        worktree_list.contains("feature-test-feature"),
        "Worktree should be listed"
    );
}

#[test]
fn test_worktree_reuse_existing_branch_in_worktree() {
    let (_dir, repo_path) = setup_test_git_repo_with_branch("main");

    std::fs::create_dir_all(repo_path.join(".abathur/worktrees")).unwrap();

    // Create first worktree
    let result1 = create_worktree_with_branch(
        &repo_path,
        "feature/shared",
        "main",
        ".abathur/worktrees/feature-shared-task1",
    );
    assert!(result1.is_ok());
    let path1 = result1.unwrap();

    // Try to create second worktree with same branch - should reuse existing
    let result2 = create_worktree_with_branch(
        &repo_path,
        "feature/shared",
        "main",
        ".abathur/worktrees/feature-shared-task2",
    );

    assert!(result2.is_ok());
    let path2 = result2.unwrap();

    // Should return the existing worktree path, not create a new one
    // Note: On macOS, /var is symlinked to /private/var, so we compare canonical paths
    let canonical1 = path1.canonicalize().unwrap_or(path1.clone());
    let canonical2 = path2.canonicalize().unwrap_or(path2.clone());
    assert_eq!(canonical1, canonical2, "Should reuse existing worktree for same branch");
}

#[test]
fn test_worktree_path_uniqueness_with_different_branches() {
    let (_dir, repo_path) = setup_test_git_repo_with_branch("main");

    std::fs::create_dir_all(repo_path.join(".abathur/worktrees")).unwrap();

    // Create first worktree
    let result1 = create_worktree_with_branch(
        &repo_path,
        "feature/task-a",
        "main",
        ".abathur/worktrees/feature-task-a",
    );
    assert!(result1.is_ok());

    // Create second worktree with different branch
    let result2 = create_worktree_with_branch(
        &repo_path,
        "feature/task-b",
        "main",
        ".abathur/worktrees/feature-task-b",
    );
    assert!(result2.is_ok());

    // Both should exist
    assert!(result1.unwrap().exists());
    assert!(result2.unwrap().exists());

    // Both branches should exist
    assert!(git_succeeds(&repo_path, &["rev-parse", "--verify", "feature/task-a"]));
    assert!(git_succeeds(&repo_path, &["rev-parse", "--verify", "feature/task-b"]));
}

#[test]
fn test_branch_name_sanitization() {
    // Test cases for branch name sanitization (mimics sanitize_branch_name)
    fn sanitize_branch_name(name: &str) -> String {
        name.chars()
            .map(|c| match c {
                ' ' | '\t' | '\n' | '\r' => '-',
                '~' | '^' | ':' | '?' | '*' | '[' | '\\' => '-',
                c if c.is_ascii_control() => '-',
                c => c,
            })
            .collect::<String>()
            .replace("..", "-")
            .replace("@{", "-")
            .trim_matches(|c| c == '.' || c == '/')
            .to_string()
    }

    assert_eq!(sanitize_branch_name("feature/my feature"), "feature/my-feature");
    assert_eq!(sanitize_branch_name("test~branch"), "test-branch");
    assert_eq!(sanitize_branch_name("branch:name"), "branch-name");
    assert_eq!(sanitize_branch_name("path/to/branch"), "path/to/branch");
}

#[test]
fn test_variable_substitution_in_branch_name() {
    // Test variable substitution logic (mimics substitute_branch_variables)
    fn substitute_template(template: &str, vars: &[(&str, &str)]) -> String {
        let mut result = template.to_string();
        for (key, value) in vars {
            result = result.replace(&format!("{{{}}}", key), value);
        }
        result
    }

    let vars = &[
        ("feature_name", "user-auth"),
        ("step_id", "design"),
        ("task_id", "abc123"),
    ];

    assert_eq!(
        substitute_template("feature/{feature_name}", vars),
        "feature/user-auth"
    );
    assert_eq!(
        substitute_template("task/{feature_name}/{step_id}", vars),
        "task/user-auth/design"
    );
    assert_eq!(
        substitute_template("{feature_name}-{task_id}", vars),
        "user-auth-abc123"
    );
}

#[test]
fn test_feature_name_extraction_priority() {
    // Tests the priority order for feature_name:
    // 1. From step output (previous step's JSON)
    // 2. From existing feature_branch on task
    // 3. From original_task_summary
    // 4. Fallback to sanitized current summary

    // Priority 1: Output has feature_name
    let output_json = serde_json::json!({
        "feature_name": "from-output",
        "other_data": "value"
    });
    let feature_name = output_json
        .get("feature_name")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    assert_eq!(feature_name, Some("from-output".to_string()));

    // Priority 2: No output feature_name, use feature_branch
    let empty_output = serde_json::json!({});
    let task_feature_branch = Some("feature/from-task".to_string());
    let feature_name = empty_output
        .get("feature_name")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .or_else(|| {
            task_feature_branch.as_ref().and_then(|fb| {
                fb.strip_prefix("feature/").map(|s| s.to_string())
            })
        });
    assert_eq!(feature_name, Some("from-task".to_string()));
}

#[test]
fn test_worktree_creation_from_existing_branch() {
    let (_dir, repo_path) = setup_test_git_repo_with_branch("main");

    std::fs::create_dir_all(repo_path.join(".abathur/worktrees")).unwrap();

    // First, create the branch without a worktree
    assert!(git_succeeds(&repo_path, &["branch", "feature/existing"]));

    // Now create a worktree for the existing branch
    let result = create_worktree_with_branch(
        &repo_path,
        "feature/existing",
        "main", // parent doesn't matter since branch exists
        ".abathur/worktrees/feature-existing",
    );

    assert!(result.is_ok(), "Should create worktree from existing branch");
    assert!(result.unwrap().exists());
}

#[test]
fn test_worktree_cleanup() {
    let (_dir, repo_path) = setup_test_git_repo_with_branch("main");

    std::fs::create_dir_all(repo_path.join(".abathur/worktrees")).unwrap();

    // Create a worktree
    let result = create_worktree_with_branch(
        &repo_path,
        "feature/cleanup-test",
        "main",
        ".abathur/worktrees/feature-cleanup-test",
    );
    assert!(result.is_ok());
    let worktree_path = result.unwrap();

    // Remove the worktree
    let remove_output = Command::new("git")
        .args(["worktree", "remove", worktree_path.to_str().unwrap()])
        .current_dir(&repo_path)
        .output()
        .expect("Failed to remove worktree");

    assert!(remove_output.status.success(), "Should remove worktree successfully");
    assert!(!worktree_path.exists(), "Worktree directory should be removed");

    // Branch should still exist
    assert!(git_succeeds(&repo_path, &["rev-parse", "--verify", "feature/cleanup-test"]));
}
