//! Abathur - Self-evolving agentic swarm orchestrator.

pub mod adapters;
pub mod cli;
pub mod domain;
pub mod services;

pub use domain::{DomainError, DomainResult};
pub use services::{Config, ConfigError};

/// Tool names that should be pre-approved in `.claude/settings.json` for
/// abathur-managed Claude Code sessions (both the main project and worktrees).
pub const ABATHUR_ALLOWED_TOOLS: &[&str] = &[
    "mcp__abathur__task_submit",
    "mcp__abathur__task_list",
    "mcp__abathur__task_get",
    "mcp__abathur__task_update_status",
    "mcp__abathur__task_assign",
    "mcp__abathur__task_wait",
    "mcp__abathur__agent_create",
    "mcp__abathur__agent_list",
    "mcp__abathur__agent_get",
    "mcp__abathur__memory_search",
    "mcp__abathur__memory_store",
    "mcp__abathur__memory_get",
    "mcp__abathur__goals_list",
    "mcp__abathur__adapter_list",
    "mcp__abathur__egress_publish",
];
