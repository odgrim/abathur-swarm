//! Tests for file input functionality in TaskCommands::Submit
//!
//! This module contains comprehensive tests for the file input feature added to
//! the task submit command, including:
//! - CLI argument parsing with both positional description and file input
//! - File reading with relative and absolute paths
//! - Error handling for missing files, permission issues, and invalid UTF-8
//! - Mutual exclusivity validation between description and file arguments

use clap::Parser;
use std::fs::{self, File};
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use tempfile::TempDir;

use abathur_cli::cli::{Cli, Commands, TaskCommands};

// ============================================================================
// CLI Argument Parsing Tests
// ============================================================================

#[test]
fn test_submit_with_positional_description() {
    let cli = Cli::try_parse_from(vec!["abathur", "task", "submit", "Test task description"]).unwrap();

    match cli.command {
        Commands::Task(TaskCommands::Submit {
            input,
            agent_type,
            summary,
            priority,
            dependencies,
            chain,
            feature_branch,
            needs_worktree,
        }) => {
            assert_eq!(input.description, Some("Test task description".to_string()));
            assert_eq!(input.file, None);
            assert_eq!(agent_type, "requirements-gatherer");
            assert_eq!(summary, None);
            assert_eq!(priority, 5);
            assert!(dependencies.is_empty());
            assert_eq!(chain, None);
            assert_eq!(feature_branch, None);
            assert!(!needs_worktree);
        }
        _ => panic!("Expected TaskCommands::Submit"),
    }
}

#[test]
fn test_submit_with_file_flag_short() {
    let cli = Cli::try_parse_from(vec!["abathur", "task", "submit", "-f", "task.txt"]).unwrap();

    match cli.command {
        Commands::Task(TaskCommands::Submit { input, .. }) => {
            assert_eq!(input.description, None);
            assert_eq!(input.file, Some(PathBuf::from("task.txt")));
        }
        _ => panic!("Expected TaskCommands::Submit"),
    }
}

#[test]
fn test_submit_with_file_flag_long() {
    let cli = Cli::try_parse_from(vec!["abathur", "task", "submit", "--file", "task.txt"]).unwrap();

    match cli.command {
        Commands::Task(TaskCommands::Submit { input, .. }) => {
            assert_eq!(input.description, None);
            assert_eq!(input.file, Some(PathBuf::from("task.txt")));
        }
        _ => panic!("Expected TaskCommands::Submit"),
    }
}

#[test]
fn test_submit_with_relative_file_path() {
    let cli = Cli::try_parse_from(vec![
        "abathur",
        "task",
        "submit",
        "-f",
        "tasks/feature_request.txt",
    ])
    .unwrap();

    match cli.command {
        Commands::Task(TaskCommands::Submit { input, .. }) => {
            assert_eq!(input.description, None);
            assert_eq!(input.file, Some(PathBuf::from("tasks/feature_request.txt")));
        }
        _ => panic!("Expected TaskCommands::Submit"),
    }
}

#[test]
fn test_submit_with_absolute_file_path() {
    let cli = Cli::try_parse_from(vec![
        "abathur",
        "task",
        "submit",
        "-f",
        "/tmp/task.txt",
    ])
    .unwrap();

    match cli.command {
        Commands::Task(TaskCommands::Submit { input, .. }) => {
            assert_eq!(input.description, None);
            assert_eq!(input.file, Some(PathBuf::from("/tmp/task.txt")));
        }
        _ => panic!("Expected TaskCommands::Submit"),
    }
}

#[test]
fn test_submit_with_file_and_other_options() {
    let cli = Cli::try_parse_from(vec![
        "abathur",
        "task",
        "submit",
        "-f",
        "task.txt",
        "--agent-type",
        "rust-specialist",
        "--summary",
        "Task summary",
        "--priority",
        "8",
        "--dependencies",
        "abc123,def456",
        "--chain",
        "custom_chain",
        "--feature-branch",
        "feature/test",
        "--needs-worktree",
    ])
    .unwrap();

    match cli.command {
        Commands::Task(TaskCommands::Submit {
            input,
            agent_type,
            summary,
            priority,
            dependencies,
            chain,
            feature_branch,
            needs_worktree,
        }) => {
            assert_eq!(input.file, Some(PathBuf::from("task.txt")));
            assert_eq!(agent_type, "rust-specialist");
            assert_eq!(summary, Some("Task summary".to_string()));
            assert_eq!(priority, 8);
            assert_eq!(dependencies, vec!["abc123", "def456"]);
            assert_eq!(chain, Some("custom_chain".to_string()));
            assert_eq!(feature_branch, Some("feature/test".to_string()));
            assert!(needs_worktree);
        }
        _ => panic!("Expected TaskCommands::Submit"),
    }
}

// ============================================================================
// Mutual Exclusivity Tests
// ============================================================================

#[test]
fn test_submit_mutual_exclusivity_both_fails() {
    // Using both description and file should fail
    let result = Cli::try_parse_from(vec![
        "abathur",
        "task",
        "submit",
        "Task description",
        "-f",
        "task.txt",
    ]);

    assert!(
        result.is_err(),
        "Should fail when both description and file are provided"
    );

    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("cannot be used with") || err_msg.contains("group"),
        "Error message should indicate mutual exclusivity, got: {}",
        err_msg
    );
}

#[test]
fn test_submit_mutual_exclusivity_neither_fails() {
    // Using neither description nor file should fail
    let result = Cli::try_parse_from(vec!["abathur", "task", "submit"]);

    assert!(
        result.is_err(),
        "Should fail when neither description nor file is provided"
    );

    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("required") || err_msg.contains("argument"),
        "Error message should indicate missing required argument, got: {}",
        err_msg
    );
}

#[test]
fn test_submit_mutual_exclusivity_file_before_other_args() {
    // File flag should work when placed before other arguments
    let cli = Cli::try_parse_from(vec![
        "abathur",
        "task",
        "submit",
        "-f",
        "task.txt",
        "--priority",
        "7",
    ])
    .unwrap();

    match cli.command {
        Commands::Task(TaskCommands::Submit { input, priority, .. }) => {
            assert_eq!(input.file, Some(PathBuf::from("task.txt")));
            assert_eq!(priority, 7);
        }
        _ => panic!("Expected TaskCommands::Submit"),
    }
}

#[test]
fn test_submit_mutual_exclusivity_file_after_other_args() {
    // File flag should work when placed after other arguments
    let cli = Cli::try_parse_from(vec![
        "abathur",
        "task",
        "submit",
        "--priority",
        "7",
        "-f",
        "task.txt",
    ])
    .unwrap();

    match cli.command {
        Commands::Task(TaskCommands::Submit { input, priority, .. }) => {
            assert_eq!(input.file, Some(PathBuf::from("task.txt")));
            assert_eq!(priority, 7);
        }
        _ => panic!("Expected TaskCommands::Submit"),
    }
}

// ============================================================================
// Backward Compatibility Tests
// ============================================================================

#[test]
fn test_submit_positional_description_still_works() {
    // Ensure the original positional description argument still works
    let cli = Cli::try_parse_from(vec![
        "abathur",
        "task",
        "submit",
        "Implement user authentication",
    ])
    .unwrap();

    match cli.command {
        Commands::Task(TaskCommands::Submit { input, .. }) => {
            assert_eq!(
                input.description,
                Some("Implement user authentication".to_string())
            );
            assert_eq!(input.file, None);
        }
        _ => panic!("Expected TaskCommands::Submit"),
    }
}

#[test]
fn test_submit_description_with_spaces() {
    let cli = Cli::try_parse_from(vec![
        "abathur",
        "task",
        "submit",
        "This is a task description with multiple words",
    ])
    .unwrap();

    match cli.command {
        Commands::Task(TaskCommands::Submit { input, .. }) => {
            assert_eq!(
                input.description,
                Some("This is a task description with multiple words".to_string())
            );
        }
        _ => panic!("Expected TaskCommands::Submit"),
    }
}

#[test]
fn test_submit_description_with_special_characters() {
    let cli = Cli::try_parse_from(vec![
        "abathur",
        "task",
        "submit",
        "Fix bug: API returns 500 on /users?limit=100",
    ])
    .unwrap();

    match cli.command {
        Commands::Task(TaskCommands::Submit { input, .. }) => {
            assert_eq!(
                input.description,
                Some("Fix bug: API returns 500 on /users?limit=100".to_string())
            );
        }
        _ => panic!("Expected TaskCommands::Submit"),
    }
}

// ============================================================================
// File Reading Tests (Integration Tests)
// ============================================================================

#[cfg(test)]
mod file_reading_tests {
    use super::*;

    #[test]
    fn test_read_file_contents_basic() {
        // Create a temporary directory and file
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("task.txt");

        // Write test content
        let mut file = File::create(&file_path).unwrap();
        writeln!(file, "This is a test task description").unwrap();
        file.flush().unwrap();

        // Verify file can be read
        let contents = fs::read_to_string(&file_path).unwrap();
        assert_eq!(contents.trim(), "This is a test task description");
    }

    #[test]
    fn test_read_file_contents_multiline() {
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("task.txt");

        let mut file = File::create(&file_path).unwrap();
        writeln!(file, "Line 1: Task summary").unwrap();
        writeln!(file, "Line 2: Task details").unwrap();
        writeln!(file, "Line 3: Expected behavior").unwrap();
        file.flush().unwrap();

        let contents = fs::read_to_string(&file_path).unwrap();
        let lines: Vec<&str> = contents.lines().collect();
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0], "Line 1: Task summary");
        assert_eq!(lines[1], "Line 2: Task details");
        assert_eq!(lines[2], "Line 3: Expected behavior");
    }

    #[test]
    fn test_read_file_contents_with_unicode() {
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("task.txt");

        let mut file = File::create(&file_path).unwrap();
        writeln!(file, "Task with unicode: 你好 🚀 café").unwrap();
        file.flush().unwrap();

        let contents = fs::read_to_string(&file_path).unwrap();
        assert_eq!(contents.trim(), "Task with unicode: 你好 🚀 café");
    }

    #[test]
    fn test_read_file_empty_file() {
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("empty.txt");

        File::create(&file_path).unwrap();

        let contents = fs::read_to_string(&file_path).unwrap();
        assert_eq!(contents.trim(), "");
    }

    #[test]
    fn test_read_file_whitespace_only() {
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("whitespace.txt");

        let mut file = File::create(&file_path).unwrap();
        writeln!(file, "   ").unwrap();
        writeln!(file).unwrap();
        writeln!(file, "\t\t").unwrap();
        file.flush().unwrap();

        let contents = fs::read_to_string(&file_path).unwrap();
        assert!(contents.trim().is_empty());
    }

    // ============================================================================
    // Error Case Tests
    // ============================================================================

    #[test]
    fn test_file_not_found_error() {
        let non_existent_path = PathBuf::from("/tmp/non_existent_file_12345.txt");

        let result = fs::read_to_string(&non_existent_path);
        assert!(result.is_err());

        let err = result.unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::NotFound);
    }

    #[test]
    fn test_file_permission_denied_error() {
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("no_read_permission.txt");

        // Create file with no read permissions (write-only)
        let mut file = File::create(&file_path).unwrap();
        writeln!(file, "Secret content").unwrap();
        file.flush().unwrap();
        drop(file);

        // Remove read permissions (keep write only)
        let mut perms = fs::metadata(&file_path).unwrap().permissions();
        perms.set_mode(0o200); // write-only
        fs::set_permissions(&file_path, perms).unwrap();

        // Attempt to read should fail with permission denied
        let result = fs::read_to_string(&file_path);
        assert!(result.is_err());

        let err = result.unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::PermissionDenied);
    }

    #[test]
    fn test_file_invalid_utf8_error() {
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("invalid_utf8.txt");

        // Write invalid UTF-8 bytes
        let invalid_utf8: Vec<u8> = vec![0xFF, 0xFE, 0xFD];
        fs::write(&file_path, invalid_utf8).unwrap();

        // Attempt to read as UTF-8 string should fail
        let result = fs::read_to_string(&file_path);
        assert!(result.is_err());

        let err = result.unwrap_err();
        // The error kind should indicate an invalid data error
        assert!(
            matches!(err.kind(), std::io::ErrorKind::InvalidData),
            "Expected InvalidData error, got: {:?}",
            err.kind()
        );
    }

    #[test]
    fn test_file_is_directory_error() {
        let temp_dir = TempDir::new().unwrap();
        let dir_path = temp_dir.path().join("subdir");
        fs::create_dir(&dir_path).unwrap();

        // Attempt to read directory as file should fail
        let result = fs::read_to_string(&dir_path);
        assert!(result.is_err());

        let err = result.unwrap_err();
        // On Unix, reading a directory typically returns InvalidInput or Other
        assert!(
            matches!(
                err.kind(),
                std::io::ErrorKind::InvalidInput
                    | std::io::ErrorKind::Other
                    | std::io::ErrorKind::IsADirectory
            ),
            "Expected directory read error, got: {:?}",
            err.kind()
        );
    }

    #[test]
    fn test_relative_path_resolution() {
        // Create a temporary directory with nested structure
        let temp_dir = TempDir::new().unwrap();
        let subdir = temp_dir.path().join("tasks");
        fs::create_dir(&subdir).unwrap();

        let file_path = subdir.join("feature.txt");
        let mut file = File::create(&file_path).unwrap();
        writeln!(file, "Feature task description").unwrap();
        file.flush().unwrap();

        // Change to temp directory
        let original_dir = std::env::current_dir().unwrap();
        std::env::set_current_dir(temp_dir.path()).unwrap();

        // Read using relative path
        let contents = fs::read_to_string("tasks/feature.txt").unwrap();
        assert_eq!(contents.trim(), "Feature task description");

        // Restore original directory
        std::env::set_current_dir(original_dir).unwrap();
    }

    #[test]
    fn test_large_file_reading() {
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("large_task.txt");

        let mut file = File::create(&file_path).unwrap();
        // Write a large task description (10KB)
        for i in 0..1000 {
            writeln!(file, "Line {}: This is part of a large task description", i).unwrap();
        }
        file.flush().unwrap();

        let contents = fs::read_to_string(&file_path).unwrap();
        let lines: Vec<&str> = contents.lines().collect();
        assert_eq!(lines.len(), 1000);
        assert!(lines[0].contains("Line 0"));
        assert!(lines[999].contains("Line 999"));
    }
}

// ============================================================================
// Edge Case Tests
// ============================================================================

#[test]
fn test_file_path_with_spaces() {
    let cli = Cli::try_parse_from(vec![
        "abathur",
        "task",
        "submit",
        "-f",
        "my task file.txt",
    ])
    .unwrap();

    match cli.command {
        Commands::Task(TaskCommands::Submit { input, .. }) => {
            assert_eq!(input.file, Some(PathBuf::from("my task file.txt")));
        }
        _ => panic!("Expected TaskCommands::Submit"),
    }
}

#[test]
fn test_file_path_with_tilde() {
    let cli = Cli::try_parse_from(vec!["abathur", "task", "submit", "-f", "~/tasks/task.txt"]).unwrap();

    match cli.command {
        Commands::Task(TaskCommands::Submit { input, .. }) => {
            assert_eq!(input.file, Some(PathBuf::from("~/tasks/task.txt")));
        }
        _ => panic!("Expected TaskCommands::Submit"),
    }
}

#[test]
fn test_priority_validation_with_file() {
    // Priority must be 0-10
    let result = Cli::try_parse_from(vec![
        "abathur",
        "task",
        "submit",
        "-f",
        "task.txt",
        "--priority",
        "15",
    ]);

    assert!(result.is_err(), "Should fail with priority > 10");
}

#[test]
fn test_json_flag_with_file() {
    let cli = Cli::try_parse_from(vec![
        "abathur",
        "--json",
        "task",
        "submit",
        "-f",
        "task.txt",
    ])
    .unwrap();

    assert!(cli.json);
    match cli.command {
        Commands::Task(TaskCommands::Submit { input, .. }) => {
            assert_eq!(input.file, Some(PathBuf::from("task.txt")));
        }
        _ => panic!("Expected TaskCommands::Submit"),
    }
}
