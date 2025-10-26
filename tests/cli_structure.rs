use clap::Parser;
use std::path::PathBuf;
use uuid::Uuid;

// Import types from the main crate
use abathur_cli::cli::{
    BranchCommands, Cli, Commands, ConvergenceStrategy, DbCommands, LoopCommands, McpCommands,
    MemoryCommands, MemoryType, SwarmCommands, TaskCommands, TaskStatus, TemplateCommands,
};

#[test]
fn test_cli_help() {
    let result = Cli::try_parse_from(vec!["abathur", "--help"]);
    assert!(result.is_err()); // --help causes early exit with error
}

#[test]
fn test_cli_version() {
    let result = Cli::try_parse_from(vec!["abathur", "--version"]);
    assert!(result.is_err()); // --version causes early exit with error
}

// ============================================================================
// Global Options Tests
// ============================================================================

#[test]
fn test_global_config_option() {
    let cli = Cli::try_parse_from(vec![
        "abathur",
        "--config",
        "/custom/config.yaml",
        "task",
        "status",
    ])
    .unwrap();

    assert_eq!(cli.config, PathBuf::from("/custom/config.yaml"));
}

#[test]
fn test_global_config_default() {
    let cli = Cli::try_parse_from(vec!["abathur", "task", "status"]).unwrap();

    assert_eq!(cli.config, PathBuf::from(".abathur/config.yaml"));
}

#[test]
fn test_global_verbose_single() {
    let cli = Cli::try_parse_from(vec!["abathur", "-v", "task", "status"]).unwrap();

    assert_eq!(cli.verbose, 1);
}

#[test]
fn test_global_verbose_multiple() {
    let cli = Cli::try_parse_from(vec!["abathur", "-vvv", "task", "status"]).unwrap();

    assert_eq!(cli.verbose, 3);
}

#[test]
fn test_global_json_flag() {
    let cli = Cli::try_parse_from(vec!["abathur", "--json", "task", "status"]).unwrap();

    assert!(cli.json);
}

#[test]
fn test_global_options_combined() {
    let cli = Cli::try_parse_from(vec![
        "abathur",
        "--config",
        "/tmp/config.yaml",
        "-vv",
        "--json",
        "task",
        "list",
    ])
    .unwrap();

    assert_eq!(cli.config, PathBuf::from("/tmp/config.yaml"));
    assert_eq!(cli.verbose, 2);
    assert!(cli.json);
}

// ============================================================================
// Task Command Tests
// ============================================================================

#[test]
fn test_task_submit() {
    let cli =
        Cli::try_parse_from(vec!["abathur", "task", "submit", "Test task description"]).unwrap();

    match cli.command {
        Commands::Task { command } => match command {
            TaskCommands::Submit {
                description,
                agent_type,
                priority,
                dependencies,
            } => {
                assert_eq!(description, "Test task description");
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
fn test_task_submit_with_options() {
    let uuid1 = Uuid::new_v4();
    let uuid2 = Uuid::new_v4();

    let cli = Cli::try_parse_from(vec![
        "abathur",
        "task",
        "submit",
        "Test task",
        "--agent-type",
        "rust-specialist",
        "--priority",
        "8",
        "--dependencies",
        &format!("{},{}", uuid1, uuid2),
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
                assert_eq!(priority, 8);
                assert_eq!(dependencies.len(), 2);
                assert_eq!(dependencies[0], uuid1);
                assert_eq!(dependencies[1], uuid2);
            }
            _ => panic!("Wrong task command"),
        },
        _ => panic!("Wrong top-level command"),
    }
}

#[test]
fn test_task_submit_priority_validation() {
    let result = Cli::try_parse_from(vec![
        "abathur",
        "task",
        "submit",
        "Test",
        "--priority",
        "15",
    ]);
    assert!(result.is_err()); // Priority out of range (0-10)
}

#[test]
fn test_task_list() {
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
fn test_task_list_with_status() {
    let cli = Cli::try_parse_from(vec!["abathur", "task", "list", "--status", "running"]).unwrap();

    match cli.command {
        Commands::Task { command } => match command {
            TaskCommands::List { status, limit } => {
                assert!(matches!(status, Some(TaskStatus::Running)));
                assert_eq!(limit, 50);
            }
            _ => panic!("Wrong task command"),
        },
        _ => panic!("Wrong top-level command"),
    }
}

#[test]
fn test_task_list_with_limit() {
    let cli = Cli::try_parse_from(vec!["abathur", "task", "list", "--limit", "100"]).unwrap();

    match cli.command {
        Commands::Task { command } => match command {
            TaskCommands::List { status, limit } => {
                assert!(status.is_none());
                assert_eq!(limit, 100);
            }
            _ => panic!("Wrong task command"),
        },
        _ => panic!("Wrong top-level command"),
    }
}

#[test]
fn test_task_show() {
    let test_uuid = Uuid::new_v4();
    let cli = Cli::try_parse_from(vec!["abathur", "task", "show", &test_uuid.to_string()]).unwrap();

    match cli.command {
        Commands::Task { command } => match command {
            TaskCommands::Show { task_id } => {
                assert_eq!(task_id, test_uuid);
            }
            _ => panic!("Wrong task command"),
        },
        _ => panic!("Wrong top-level command"),
    }
}

#[test]
fn test_task_cancel() {
    let test_uuid = Uuid::new_v4();
    let cli =
        Cli::try_parse_from(vec!["abathur", "task", "cancel", &test_uuid.to_string()]).unwrap();

    match cli.command {
        Commands::Task { command } => match command {
            TaskCommands::Cancel { task_id } => {
                assert_eq!(task_id, test_uuid);
            }
            _ => panic!("Wrong task command"),
        },
        _ => panic!("Wrong top-level command"),
    }
}

#[test]
fn test_task_retry() {
    let test_uuid = Uuid::new_v4();
    let cli =
        Cli::try_parse_from(vec!["abathur", "task", "retry", &test_uuid.to_string()]).unwrap();

    match cli.command {
        Commands::Task { command } => match command {
            TaskCommands::Retry { task_id } => {
                assert_eq!(task_id, test_uuid);
            }
            _ => panic!("Wrong task command"),
        },
        _ => panic!("Wrong top-level command"),
    }
}

#[test]
fn test_task_status() {
    let cli = Cli::try_parse_from(vec!["abathur", "task", "status"]).unwrap();

    match cli.command {
        Commands::Task { command } => match command {
            TaskCommands::Status => {
                // Success - command parsed correctly
            }
            _ => panic!("Wrong task command"),
        },
        _ => panic!("Wrong top-level command"),
    }
}

// ============================================================================
// Swarm Command Tests
// ============================================================================

#[test]
fn test_swarm_start() {
    let cli = Cli::try_parse_from(vec!["abathur", "swarm", "start"]).unwrap();

    match cli.command {
        Commands::Swarm { command } => match command {
            SwarmCommands::Start { max_agents } => {
                assert_eq!(max_agents, 10);
            }
            _ => panic!("Wrong swarm command"),
        },
        _ => panic!("Wrong top-level command"),
    }
}

#[test]
fn test_swarm_start_with_max_agents() {
    let cli = Cli::try_parse_from(vec!["abathur", "swarm", "start", "--max-agents", "20"]).unwrap();

    match cli.command {
        Commands::Swarm { command } => match command {
            SwarmCommands::Start { max_agents } => {
                assert_eq!(max_agents, 20);
            }
            _ => panic!("Wrong swarm command"),
        },
        _ => panic!("Wrong top-level command"),
    }
}

#[test]
fn test_swarm_status() {
    let cli = Cli::try_parse_from(vec!["abathur", "swarm", "status"]).unwrap();

    match cli.command {
        Commands::Swarm { command } => match command {
            SwarmCommands::Status => {
                // Success
            }
            _ => panic!("Wrong swarm command"),
        },
        _ => panic!("Wrong top-level command"),
    }
}

// ============================================================================
// Loop Command Tests
// ============================================================================

#[test]
fn test_loop_start() {
    let test_uuid = Uuid::new_v4();
    let cli =
        Cli::try_parse_from(vec!["abathur", "loop", "start", &test_uuid.to_string()]).unwrap();

    match cli.command {
        Commands::Loop { command } => match command {
            LoopCommands::Start {
                task_id,
                max_iterations,
                convergence_strategy,
            } => {
                assert_eq!(task_id, test_uuid);
                assert_eq!(max_iterations, 10);
                assert!(matches!(
                    convergence_strategy,
                    ConvergenceStrategy::Adaptive
                ));
            }
            _ => panic!("Wrong loop command"),
        },
        _ => panic!("Wrong top-level command"),
    }
}

#[test]
fn test_loop_start_with_options() {
    let test_uuid = Uuid::new_v4();
    let cli = Cli::try_parse_from(vec![
        "abathur",
        "loop",
        "start",
        &test_uuid.to_string(),
        "--max-iterations",
        "20",
        "--convergence-strategy",
        "threshold",
    ])
    .unwrap();

    match cli.command {
        Commands::Loop { command } => match command {
            LoopCommands::Start {
                task_id,
                max_iterations,
                convergence_strategy,
            } => {
                assert_eq!(task_id, test_uuid);
                assert_eq!(max_iterations, 20);
                assert!(matches!(
                    convergence_strategy,
                    ConvergenceStrategy::Threshold
                ));
            }
            _ => panic!("Wrong loop command"),
        },
        _ => panic!("Wrong top-level command"),
    }
}

#[test]
fn test_loop_history() {
    let test_uuid = Uuid::new_v4();
    let cli =
        Cli::try_parse_from(vec!["abathur", "loop", "history", &test_uuid.to_string()]).unwrap();

    match cli.command {
        Commands::Loop { command } => match command {
            LoopCommands::History { loop_id } => {
                assert_eq!(loop_id, test_uuid);
            }
            _ => panic!("Wrong loop command"),
        },
        _ => panic!("Wrong top-level command"),
    }
}

// ============================================================================
// MCP Command Tests
// ============================================================================

#[test]
fn test_mcp_list() {
    let cli = Cli::try_parse_from(vec!["abathur", "mcp", "list"]).unwrap();

    match cli.command {
        Commands::Mcp { command } => match command {
            McpCommands::List => {
                // Success
            }
            _ => panic!("Wrong mcp command"),
        },
        _ => panic!("Wrong top-level command"),
    }
}

#[test]
fn test_mcp_start() {
    let cli = Cli::try_parse_from(vec!["abathur", "mcp", "start", "test-server"]).unwrap();

    match cli.command {
        Commands::Mcp { command } => match command {
            McpCommands::Start { server_name } => {
                assert_eq!(server_name, "test-server");
            }
            _ => panic!("Wrong mcp command"),
        },
        _ => panic!("Wrong top-level command"),
    }
}

#[test]
fn test_mcp_stop() {
    let cli = Cli::try_parse_from(vec!["abathur", "mcp", "stop", "test-server"]).unwrap();

    match cli.command {
        Commands::Mcp { command } => match command {
            McpCommands::Stop { server_name } => {
                assert_eq!(server_name, "test-server");
            }
            _ => panic!("Wrong mcp command"),
        },
        _ => panic!("Wrong top-level command"),
    }
}

#[test]
fn test_mcp_restart() {
    let cli = Cli::try_parse_from(vec!["abathur", "mcp", "restart", "test-server"]).unwrap();

    match cli.command {
        Commands::Mcp { command } => match command {
            McpCommands::Restart { server_name } => {
                assert_eq!(server_name, "test-server");
            }
            _ => panic!("Wrong mcp command"),
        },
        _ => panic!("Wrong top-level command"),
    }
}

// ============================================================================
// Database Command Tests
// ============================================================================

#[test]
fn test_db_migrate() {
    let cli = Cli::try_parse_from(vec!["abathur", "db", "migrate"]).unwrap();

    match cli.command {
        Commands::Db { command } => match command {
            DbCommands::Migrate => {
                // Success
            }
            _ => panic!("Wrong db command"),
        },
        _ => panic!("Wrong top-level command"),
    }
}

#[test]
fn test_db_status() {
    let cli = Cli::try_parse_from(vec!["abathur", "db", "status"]).unwrap();

    match cli.command {
        Commands::Db { command } => match command {
            DbCommands::Status => {
                // Success
            }
            _ => panic!("Wrong db command"),
        },
        _ => panic!("Wrong top-level command"),
    }
}

#[test]
fn test_db_backup() {
    let cli = Cli::try_parse_from(vec!["abathur", "db", "backup", "/tmp/backup.db"]).unwrap();

    match cli.command {
        Commands::Db { command } => match command {
            DbCommands::Backup { output } => {
                assert_eq!(output, PathBuf::from("/tmp/backup.db"));
            }
            _ => panic!("Wrong db command"),
        },
        _ => panic!("Wrong top-level command"),
    }
}

// ============================================================================
// Memory Command Tests
// ============================================================================

#[test]
fn test_memory_add() {
    let cli = Cli::try_parse_from(vec![
        "abathur",
        "memory",
        "add",
        "test-namespace",
        "test-key",
        "test-value",
    ])
    .unwrap();

    match cli.command {
        Commands::Memory { command } => match command {
            MemoryCommands::Add {
                namespace,
                key,
                value,
                memory_type,
            } => {
                assert_eq!(namespace, "test-namespace");
                assert_eq!(key, "test-key");
                assert_eq!(value, "test-value");
                assert!(matches!(memory_type, MemoryType::Semantic));
            }
            _ => panic!("Wrong memory command"),
        },
        _ => panic!("Wrong top-level command"),
    }
}

#[test]
fn test_memory_add_with_type() {
    let cli = Cli::try_parse_from(vec![
        "abathur",
        "memory",
        "add",
        "test-namespace",
        "test-key",
        "test-value",
        "--memory-type",
        "episodic",
    ])
    .unwrap();

    match cli.command {
        Commands::Memory { command } => match command {
            MemoryCommands::Add {
                namespace,
                key,
                value,
                memory_type,
            } => {
                assert_eq!(namespace, "test-namespace");
                assert_eq!(key, "test-key");
                assert_eq!(value, "test-value");
                assert!(matches!(memory_type, MemoryType::Episodic));
            }
            _ => panic!("Wrong memory command"),
        },
        _ => panic!("Wrong top-level command"),
    }
}

#[test]
fn test_memory_get() {
    let cli = Cli::try_parse_from(vec![
        "abathur",
        "memory",
        "get",
        "test-namespace",
        "test-key",
    ])
    .unwrap();

    match cli.command {
        Commands::Memory { command } => match command {
            MemoryCommands::Get { namespace, key } => {
                assert_eq!(namespace, "test-namespace");
                assert_eq!(key, "test-key");
            }
            _ => panic!("Wrong memory command"),
        },
        _ => panic!("Wrong top-level command"),
    }
}

#[test]
fn test_memory_search() {
    let cli = Cli::try_parse_from(vec!["abathur", "memory", "search", "test-prefix"]).unwrap();

    match cli.command {
        Commands::Memory { command } => match command {
            MemoryCommands::Search {
                namespace_prefix,
                memory_type,
            } => {
                assert_eq!(namespace_prefix, "test-prefix");
                assert!(memory_type.is_none());
            }
            _ => panic!("Wrong memory command"),
        },
        _ => panic!("Wrong top-level command"),
    }
}

#[test]
fn test_memory_search_with_type() {
    let cli = Cli::try_parse_from(vec![
        "abathur",
        "memory",
        "search",
        "test-prefix",
        "--memory-type",
        "procedural",
    ])
    .unwrap();

    match cli.command {
        Commands::Memory { command } => match command {
            MemoryCommands::Search {
                namespace_prefix,
                memory_type,
            } => {
                assert_eq!(namespace_prefix, "test-prefix");
                assert!(matches!(memory_type, Some(MemoryType::Procedural)));
            }
            _ => panic!("Wrong memory command"),
        },
        _ => panic!("Wrong top-level command"),
    }
}

// ============================================================================
// Template Command Tests
// ============================================================================

#[test]
fn test_template_init() {
    let cli = Cli::try_parse_from(vec!["abathur", "template", "init", "rust-project"]).unwrap();

    match cli.command {
        Commands::Template { command } => match command {
            TemplateCommands::Init { template, output } => {
                assert_eq!(template, "rust-project");
                assert_eq!(output, PathBuf::from("."));
            }
        },
        _ => panic!("Wrong top-level command"),
    }
}

#[test]
fn test_template_init_with_output() {
    let cli = Cli::try_parse_from(vec![
        "abathur",
        "template",
        "init",
        "rust-project",
        "--output",
        "/tmp/new-project",
    ])
    .unwrap();

    match cli.command {
        Commands::Template { command } => match command {
            TemplateCommands::Init { template, output } => {
                assert_eq!(template, "rust-project");
                assert_eq!(output, PathBuf::from("/tmp/new-project"));
            }
        },
        _ => panic!("Wrong top-level command"),
    }
}

// ============================================================================
// Branch Command Tests
// ============================================================================

#[test]
fn test_branch_create() {
    let cli =
        Cli::try_parse_from(vec!["abathur", "branch", "create", "feature/new-feature"]).unwrap();

    match cli.command {
        Commands::Branch { command } => match command {
            BranchCommands::Create { name, from } => {
                assert_eq!(name, "feature/new-feature");
                assert_eq!(from, "main");
            }
            _ => panic!("Wrong branch command"),
        },
        _ => panic!("Wrong top-level command"),
    }
}

#[test]
fn test_branch_create_with_from() {
    let cli = Cli::try_parse_from(vec![
        "abathur",
        "branch",
        "create",
        "feature/new-feature",
        "--from",
        "develop",
    ])
    .unwrap();

    match cli.command {
        Commands::Branch { command } => match command {
            BranchCommands::Create { name, from } => {
                assert_eq!(name, "feature/new-feature");
                assert_eq!(from, "develop");
            }
            _ => panic!("Wrong branch command"),
        },
        _ => panic!("Wrong top-level command"),
    }
}

#[test]
fn test_branch_list() {
    let cli = Cli::try_parse_from(vec!["abathur", "branch", "list"]).unwrap();

    match cli.command {
        Commands::Branch { command } => match command {
            BranchCommands::List => {
                // Success
            }
            _ => panic!("Wrong branch command"),
        },
        _ => panic!("Wrong top-level command"),
    }
}

// ============================================================================
// Error Cases
// ============================================================================

#[test]
fn test_invalid_command() {
    let result = Cli::try_parse_from(vec!["abathur", "invalid-command"]);
    assert!(result.is_err());
}

#[test]
fn test_missing_required_argument() {
    let result = Cli::try_parse_from(vec!["abathur", "task", "submit"]);
    assert!(result.is_err()); // Missing description
}

#[test]
fn test_invalid_uuid() {
    let result = Cli::try_parse_from(vec!["abathur", "task", "show", "not-a-uuid"]);
    assert!(result.is_err());
}

#[test]
fn test_invalid_enum_value() {
    let result = Cli::try_parse_from(vec!["abathur", "task", "list", "--status", "invalid"]);
    assert!(result.is_err());
}
