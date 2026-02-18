//! Command-line interface module for Abathur.

pub mod command_dispatcher;
pub mod commands;
pub mod event_helpers;
pub mod id_resolver;
mod output;

use clap::{Parser, Subcommand};
use std::path::PathBuf;

/// Abathur - Self-evolving agentic swarm orchestrator
#[derive(Parser, Debug)]
#[command(name = "abathur")]
#[command(author, version, about, long_about = None)]
pub struct Cli {
    /// Output results as JSON
    #[arg(long, global = true)]
    pub json: bool,

    /// Configuration file path
    #[arg(long, global = true, default_value = "abathur.toml")]
    pub config: PathBuf,

    /// Verbosity level (-v, -vv, -vvv)
    #[arg(short, long, action = clap::ArgAction::Count, global = true)]
    pub verbose: u8,

    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Initialize a new Abathur project
    Init(commands::init::InitArgs),
    /// Manage convergent goals
    Goal(commands::goal::GoalArgs),
    /// Manage tasks
    Task(commands::task::TaskArgs),
    /// Manage memory (three-tier system)
    Memory(commands::memory::MemoryArgs),
    /// Manage agent templates
    Agent(commands::agent::AgentArgs),
    /// Manage git worktrees for task isolation
    Worktree(commands::worktree::WorktreeArgs),
    /// Control the swarm orchestrator
    Swarm(commands::swarm::SwarmArgs),
    /// Start and manage MCP (Model Context Protocol) HTTP servers
    Mcp(commands::mcp::McpArgs),
    /// Manage trigger rules for event-driven automation
    Trigger(commands::trigger::TriggerArgs),
    /// Manage periodic task schedules
    Schedule(commands::schedule::ScheduleArgs),
    /// Query and inspect the event store
    Event(commands::event::EventArgs),
    /// Manage workflow templates
    Workflow(commands::workflow::WorkflowArgs),
}

pub fn handle_error(err: anyhow::Error, json_mode: bool) -> ! {
    if json_mode {
        let output = serde_json::json!({
            "error": true,
            "message": err.to_string(),
            "chain": err.chain().skip(1).map(|e| e.to_string()).collect::<Vec<_>>()
        });
        eprintln!("{}", serde_json::to_string_pretty(&output).unwrap());
    } else {
        eprintln!("Error: {}", err);
        for cause in err.chain().skip(1) {
            eprintln!("  Caused by: {}", cause);
        }
    }
    std::process::exit(1);
}
