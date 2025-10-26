#![allow(clippy::needless_borrows_for_generic_args)]

use abathur_cli::cli::{Cli, Commands, TaskCommands, TaskStatus};
use clap::Parser;
use uuid::Uuid;

#[test]
fn test_parse_task_submit() {
    let cli = Cli::try_parse_from(vec![
        "abathur",
        "task",
        "submit",
        "Test task",
        "--agent-type",
        "rust-specialist",
        "--priority",
        "7",
    ])
    .unwrap();

    match cli.command {
        Commands::Task { command } => match command {
            TaskCommands::Submit {
                description,
                agent_type,
                priority,
                dependencies,
            } => {
                assert_eq!(description, "Test task");
                assert_eq!(agent_type, "rust-specialist");
                assert_eq!(priority, 7);
                assert!(dependencies.is_empty());
            }
            _ => panic!("Wrong task command"),
        },
        _ => panic!("Wrong top-level command"),
    }
}

#[test]
fn test_parse_task_submit_with_dependencies() {
    let dep1 = Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap();
    let dep2 = Uuid::parse_str("6ba7b810-9dad-11d1-80b4-00c04fd430c8").unwrap();

    let cli = Cli::try_parse_from(vec![
        "abathur",
        "task",
        "submit",
        "Dependent task",
        "--dependencies",
        "550e8400-e29b-41d4-a716-446655440000,6ba7b810-9dad-11d1-80b4-00c04fd430c8",
    ])
    .unwrap();

    match cli.command {
        Commands::Task { command } => match command {
            TaskCommands::Submit { dependencies, .. } => {
                assert_eq!(dependencies.len(), 2);
                assert_eq!(dependencies[0], dep1);
                assert_eq!(dependencies[1], dep2);
            }
            _ => panic!("Wrong task command"),
        },
        _ => panic!("Wrong top-level command"),
    }
}

#[test]
fn test_parse_task_submit_defaults() {
    let cli = Cli::try_parse_from(vec!["abathur", "task", "submit", "Simple task"]).unwrap();

    match cli.command {
        Commands::Task { command } => match command {
            TaskCommands::Submit {
                agent_type,
                priority,
                dependencies,
                ..
            } => {
                assert_eq!(agent_type, "general-purpose");
                assert_eq!(priority, 5);
                assert!(dependencies.is_empty());
            }
            _ => panic!("Wrong task command"),
        },
        _ => panic!("Wrong top-level command"),
    }
}

#[test]
fn test_parse_task_list() {
    let cli = Cli::try_parse_from(vec!["abathur", "task", "list"]).unwrap();

    match cli.command {
        Commands::Task { command } => match command {
            TaskCommands::List { status, limit } => {
                assert!(status.is_none());
                assert_eq!(limit, 50);
            }
            _ => panic!("Wrong task command"),
        },
        _ => panic!("Wrong top-level command"),
    }
}

#[test]
fn test_parse_task_list_with_filters() {
    let cli = Cli::try_parse_from(vec![
        "abathur", "task", "list", "--status", "pending", "--limit", "20",
    ])
    .unwrap();

    match cli.command {
        Commands::Task { command } => match command {
            TaskCommands::List { status, limit } => {
                assert!(matches!(status, Some(TaskStatus::Pending)));
                assert_eq!(limit, 20);
            }
            _ => panic!("Wrong task command"),
        },
        _ => panic!("Wrong top-level command"),
    }
}

#[test]
fn test_parse_task_show() {
    let task_id = Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap();

    let cli = Cli::try_parse_from(vec![
        "abathur",
        "task",
        "show",
        "550e8400-e29b-41d4-a716-446655440000",
    ])
    .unwrap();

    match cli.command {
        Commands::Task { command } => match command {
            TaskCommands::Show { task_id: parsed_id } => {
                assert_eq!(parsed_id, task_id);
            }
            _ => panic!("Wrong task command"),
        },
        _ => panic!("Wrong top-level command"),
    }
}

#[test]
fn test_parse_task_cancel() {
    let task_id = Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap();

    let cli = Cli::try_parse_from(vec![
        "abathur",
        "task",
        "cancel",
        "550e8400-e29b-41d4-a716-446655440000",
    ])
    .unwrap();

    match cli.command {
        Commands::Task { command } => match command {
            TaskCommands::Cancel { task_id: parsed_id } => {
                assert_eq!(parsed_id, task_id);
            }
            _ => panic!("Wrong task command"),
        },
        _ => panic!("Wrong top-level command"),
    }
}

#[test]
fn test_parse_task_retry() {
    let task_id = Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap();

    let cli = Cli::try_parse_from(vec![
        "abathur",
        "task",
        "retry",
        "550e8400-e29b-41d4-a716-446655440000",
    ])
    .unwrap();

    match cli.command {
        Commands::Task { command } => match command {
            TaskCommands::Retry { task_id: parsed_id } => {
                assert_eq!(parsed_id, task_id);
            }
            _ => panic!("Wrong task command"),
        },
        _ => panic!("Wrong top-level command"),
    }
}

#[test]
fn test_parse_task_status() {
    let cli = Cli::try_parse_from(vec!["abathur", "task", "status"]).unwrap();

    match cli.command {
        Commands::Task { command } => match command {
            TaskCommands::Status => {}
            _ => panic!("Wrong task command"),
        },
        _ => panic!("Wrong top-level command"),
    }
}

#[test]
fn test_global_options() {
    let cli = Cli::try_parse_from(vec![
        "abathur",
        "--config",
        "/custom/config.yaml",
        "-vvv",
        "--json",
        "task",
        "status",
    ])
    .unwrap();

    assert_eq!(cli.config, std::path::PathBuf::from("/custom/config.yaml"));
    assert_eq!(cli.verbose, 3);
    assert!(cli.json);
}

#[test]
fn test_priority_validation_min() {
    let cli = Cli::try_parse_from(vec!["abathur", "task", "submit", "Test", "--priority", "0"]);
    assert!(cli.is_ok());
}

#[test]
fn test_priority_validation_max() {
    let cli = Cli::try_parse_from(vec![
        "abathur",
        "task",
        "submit",
        "Test",
        "--priority",
        "10",
    ]);
    assert!(cli.is_ok());
}

#[test]
fn test_priority_validation_out_of_range() {
    let result = Cli::try_parse_from(vec![
        "abathur",
        "task",
        "submit",
        "Test",
        "--priority",
        "15",
    ]);
    assert!(result.is_err());
}

#[test]
fn test_invalid_uuid() {
    let result = Cli::try_parse_from(vec!["abathur", "task", "show", "not-a-uuid"]);
    assert!(result.is_err());
}
