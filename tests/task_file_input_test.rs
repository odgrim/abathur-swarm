use clap::Parser;
use std::path::PathBuf;

// Import types from the main crate
use abathur_cli::cli::{Cli, Commands, TaskCommands};

#[test]
fn test_task_submit_with_inline_description() {
    let cli =
        Cli::try_parse_from(vec!["abathur", "task", "submit", "Test task description"]).unwrap();

    match cli.command {
        Commands::Task(task_cmd) => match task_cmd {
            TaskCommands::Submit {
                input,
                agent_type,
                summary,
                priority,
                dependencies,
                chain,
                feature_branch,
                needs_worktree,
            } => {
                assert_eq!(input.description, Some("Test task description".to_string()));
                assert!(input.file.is_none());
                assert_eq!(agent_type, "requirements-gatherer");
                assert!(summary.is_none());
                assert_eq!(priority, 5);
                assert!(dependencies.is_empty());
                assert!(chain.is_none());
                assert!(feature_branch.is_none());
                assert!(!needs_worktree);
            }
            _ => panic!("Wrong task command"),
        },
        _ => panic!("Wrong top-level command"),
    }
}

#[test]
fn test_task_submit_with_file_short_flag() {
    let cli = Cli::try_parse_from(vec![
        "abathur",
        "task",
        "submit",
        "-f",
        "/path/to/task.txt",
    ])
    .unwrap();

    match cli.command {
        Commands::Task(task_cmd) => match task_cmd {
            TaskCommands::Submit {
                input,
                agent_type,
                summary,
                priority,
                dependencies,
                chain,
                feature_branch,
                needs_worktree,
            } => {
                assert!(input.description.is_none());
                assert_eq!(input.file, Some(PathBuf::from("/path/to/task.txt")));
                assert_eq!(agent_type, "requirements-gatherer");
                assert!(summary.is_none());
                assert_eq!(priority, 5);
                assert!(dependencies.is_empty());
                assert!(chain.is_none());
                assert!(feature_branch.is_none());
                assert!(!needs_worktree);
            }
            _ => panic!("Wrong task command"),
        },
        _ => panic!("Wrong top-level command"),
    }
}

#[test]
fn test_task_submit_with_file_long_flag() {
    let cli = Cli::try_parse_from(vec![
        "abathur",
        "task",
        "submit",
        "--file",
        "/path/to/task.txt",
    ])
    .unwrap();

    match cli.command {
        Commands::Task(task_cmd) => match task_cmd {
            TaskCommands::Submit {
                input,
                agent_type,
                summary,
                priority,
                dependencies,
                chain,
                feature_branch,
                needs_worktree,
            } => {
                assert!(input.description.is_none());
                assert_eq!(input.file, Some(PathBuf::from("/path/to/task.txt")));
                assert_eq!(agent_type, "requirements-gatherer");
                assert!(summary.is_none());
                assert_eq!(priority, 5);
                assert!(dependencies.is_empty());
                assert!(chain.is_none());
                assert!(feature_branch.is_none());
                assert!(!needs_worktree);
            }
            _ => panic!("Wrong task command"),
        },
        _ => panic!("Wrong top-level command"),
    }
}

#[test]
fn test_task_submit_with_file_and_options() {
    let cli = Cli::try_parse_from(vec![
        "abathur",
        "task",
        "submit",
        "-f",
        "/path/to/task.txt",
        "--agent-type",
        "rust-specialist",
        "--priority",
        "8",
    ])
    .unwrap();

    match cli.command {
        Commands::Task(task_cmd) => match task_cmd {
            TaskCommands::Submit {
                input,
                agent_type,
                summary,
                priority,
                dependencies,
                chain,
                feature_branch,
                needs_worktree,
            } => {
                assert!(input.description.is_none());
                assert_eq!(input.file, Some(PathBuf::from("/path/to/task.txt")));
                assert_eq!(agent_type, "rust-specialist");
                assert!(summary.is_none());
                assert_eq!(priority, 8);
                assert!(dependencies.is_empty());
                assert!(chain.is_none());
                assert!(feature_branch.is_none());
                assert!(!needs_worktree);
            }
            _ => panic!("Wrong task command"),
        },
        _ => panic!("Wrong top-level command"),
    }
}

#[test]
fn test_task_submit_mutual_exclusivity_error() {
    // Test that providing both description and file produces an error
    let result = Cli::try_parse_from(vec![
        "abathur",
        "task",
        "submit",
        "Inline description",
        "--file",
        "/path/to/task.txt",
    ]);

    assert!(result.is_err(), "Should reject when both description and file are provided");
}

#[test]
fn test_task_submit_description_required_without_file() {
    // Test that missing both description and file produces an error
    let result = Cli::try_parse_from(vec![
        "abathur",
        "task",
        "submit",
        "--agent-type",
        "rust-specialist",
    ]);

    assert!(result.is_err(), "Should reject when neither description nor file is provided");
}

#[test]
fn test_task_submit_only_one_positional_allowed() {
    // Test that file flag allows positioning elsewhere while positional description must be at specific position
    let cli = Cli::try_parse_from(vec![
        "abathur",
        "task",
        "submit",
        "--priority",
        "8",
        "-f",
        "/path/to/task.txt",
    ])
    .unwrap();

    match cli.command {
        Commands::Task(task_cmd) => match task_cmd {
            TaskCommands::Submit { input, priority, .. } => {
                assert!(input.description.is_none());
                assert_eq!(input.file, Some(PathBuf::from("/path/to/task.txt")));
                assert_eq!(priority, 8);
            }
            _ => panic!("Wrong task command"),
        },
        _ => panic!("Wrong top-level command"),
    }
}
