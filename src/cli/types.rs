//! CLI type definitions
//!
//! This module contains clap command structures that define the CLI interface.

use clap::{Parser, Subcommand};
use uuid::Uuid;

#[derive(Parser)]
#[command(name = "abathur")]
#[command(about = "Abathur - Agentic Swarm Orchestrator", long_about = None)]
#[command(version)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,

    /// Output in JSON format
    #[arg(short, long, global = true)]
    pub json: bool,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Initialize Abathur configuration and database
    Init {
        /// Force reinitialization even if already initialized
        #[arg(short, long)]
        force: bool,
    },

    /// Task management commands
    #[command(subcommand)]
    Task(TaskCommands),

    /// Memory management commands
    #[command(subcommand)]
    Memory(MemoryCommands),

    /// Swarm orchestration commands
    #[command(subcommand)]
    Swarm(SwarmCommands),
}

#[derive(Subcommand)]
pub enum TaskCommands {
    /// Submit a new task to the queue
    Submit {
        /// Task description (positional argument)
        description: String,

        /// Agent type to execute the task
        #[arg(short, long, default_value = "requirements-gatherer")]
        agent_type: String,

        /// Optional summary of the task
        #[arg(short, long)]
        summary: Option<String>,

        /// Task priority (0-10, higher = more urgent)
        #[arg(short, long, default_value = "5")]
        priority: u8,

        /// Task dependencies (comma-separated UUIDs)
        #[arg(short = 'D', long, value_delimiter = ',')]
        dependencies: Vec<Uuid>,
    },

    /// List tasks in the queue
    List {
        /// Filter by status
        #[arg(short, long)]
        status: Option<String>,

        /// Maximum number of tasks to display
        #[arg(short, long, default_value = "50")]
        limit: usize,
    },

    /// Show details for a specific task
    Show {
        /// Task ID
        task_id: Uuid,
    },

    /// Update one or more tasks
    Update {
        /// Task ID(s) to update (comma-separated)
        #[arg(value_delimiter = ',')]
        task_ids: Vec<Uuid>,

        /// Update task status
        #[arg(short, long)]
        status: Option<String>,

        /// Update base priority (0-10)
        #[arg(short, long)]
        priority: Option<u8>,

        /// Update agent type
        #[arg(short, long)]
        agent_type: Option<String>,

        /// Add dependencies (comma-separated UUIDs)
        #[arg(long, value_delimiter = ',')]
        add_dependency: Vec<Uuid>,

        /// Remove dependencies (comma-separated UUIDs)
        #[arg(long, value_delimiter = ',')]
        remove_dependency: Vec<Uuid>,

        /// Increment retry count and reset to pending (for failed tasks)
        #[arg(long)]
        retry: bool,

        /// Cancel task and cascade to dependents
        #[arg(long)]
        cancel: bool,
    },

    /// Show queue status and statistics
    Status,
}

#[derive(Subcommand)]
pub enum MemoryCommands {
    /// List memories
    List {
        /// Filter by namespace prefix
        #[arg(short, long)]
        namespace: Option<String>,

        /// Filter by memory type (semantic, episodic, procedural)
        #[arg(short = 't', long)]
        memory_type: Option<String>,

        /// Maximum number of memories to display
        #[arg(short, long, default_value = "50")]
        limit: usize,
    },

    /// Show details for a specific memory
    Show {
        /// Memory namespace
        namespace: String,

        /// Memory key
        key: String,
    },

    /// Count memories matching criteria
    Count {
        /// Namespace prefix to count
        #[arg(short, long, default_value = "")]
        namespace: String,

        /// Filter by memory type (semantic, episodic, procedural)
        #[arg(short = 't', long)]
        memory_type: Option<String>,
    },
}

#[derive(Subcommand)]
pub enum SwarmCommands {
    /// Start the swarm orchestrator
    Start {
        /// Maximum number of concurrent agents
        #[arg(short, long, default_value = "10")]
        max_agents: usize,

        /// Hidden flag for daemon mode (internal use only)
        #[arg(long, hide = true)]
        __daemon: bool,
    },

    /// Stop the swarm orchestrator
    Stop,

    /// Show swarm orchestrator status
    Status,
}

// Re-export types that tests expect
pub use crate::cli::models::TaskStatus;
pub use crate::domain::models::MemoryType;

// Placeholder types for other commands referenced in tests
// These will be implemented later
#[derive(Subcommand)]
pub enum BranchCommands {
    /// Create a new branch
    Create {
        /// Branch name
        name: String,

        /// Base branch
        #[arg(short, long)]
        from: Option<String>,
    },

    /// List branches
    List,
}

#[derive(Subcommand)]
pub enum LoopCommands {
    /// Start an execution loop
    Start {
        /// Loop description
        #[arg(short, long)]
        description: String,

        /// Convergence strategy
        #[arg(short, long)]
        strategy: String,

        /// Maximum iterations
        #[arg(short, long)]
        max_iterations: Option<usize>,
    },

    /// Show loop execution history
    History {
        /// Loop ID
        loop_id: Uuid,
    },
}

#[derive(Subcommand)]
pub enum McpCommands {
    /// List MCP servers
    List,

    /// Start an MCP server
    Start {
        /// Server name
        server_name: String,
    },

    /// Stop an MCP server
    Stop {
        /// Server name
        server_name: String,
    },

    /// Restart an MCP server
    Restart {
        /// Server name
        server_name: String,
    },
}

#[derive(Subcommand)]
pub enum DbCommands {
    /// Run database migrations
    Migrate,

    /// Show database status
    Status,

    /// Backup database
    Backup {
        /// Output path for backup
        #[arg(short, long)]
        output: String,
    },
}

#[derive(Subcommand)]
pub enum TemplateCommands {
    /// Initialize from template
    Init {
        /// Template name
        template: String,

        /// Output directory
        #[arg(short, long)]
        output: Option<String>,
    },
}

/// Convergence strategy placeholder
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConvergenceStrategy {
    /// Fixed iterations
    Fixed,
    /// Converge on criteria
    Criteria,
}
