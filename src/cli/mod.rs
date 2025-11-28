//! CLI module for Abathur
//!
//! Contains command-line interface implementations and output formatting.

pub mod commands;
pub mod models;
pub mod output;
pub mod service;
pub mod tui;
pub mod types;

pub use output::TableFormatter;
pub use types::{
    BranchCommands, Cli, Commands, ConvergenceStrategy, DbCommands, LoopCommands, McpCommands,
    MemoryCommands, MemoryType, SwarmCommands, TaskCommands, TaskInputSource, TaskStatus,
    TemplateCommands,
};
