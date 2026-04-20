//! Agent preparation service.
//!
//! Resolves an agent template, derives capability lists and CLI tool names,
//! and computes the read-only role flag used downstream by execution-mode
//! resolution and post-completion verification.
//!
//! Extracted from `goal_processing::spawn_task_agent` per spec T10
//! (`specs/T10-spawn-task-agent-extraction.md`).

use std::sync::Arc;

use crate::domain::errors::DomainResult;
use crate::domain::models::AgentTier;
use crate::domain::ports::AgentRepository;

/// Metadata derived from an agent template, used by downstream services.
#[derive(Debug, Clone)]
pub struct AgentMetadata {
    /// Template version (for evolution-loop tracking).
    pub version: u32,
    /// Raw capability/tool names from the template.
    pub capabilities: Vec<String>,
    /// CLI tool names (PascalCase / `mcp__abathur__*`) for `--allowedTools`.
    pub cli_tools: Vec<String>,
    /// True when the agent's tools include write/edit/shell.
    pub can_write: bool,
    /// Whether the template was explicitly marked `read_only`.
    #[allow(dead_code)]
    pub is_read_only: bool,
    /// Maximum turns set on the template (0 = unset, falls back to role default).
    pub max_turns: u32,
    /// Optional preferred model override.
    pub preferred_model: Option<String>,
    /// Agent tier (Architect / Specialist / Worker).
    pub tier: AgentTier,
    /// Effective read-only role: combines the template flag with the
    /// legacy name-based heuristic (overmind, aggregator, planner, etc.).
    pub is_read_only_role: bool,
}

/// Service that prepares an agent for execution.
///
/// Holds an `Arc<dyn AgentRepository>` so it can be cloned before
/// `tokio::spawn` to avoid deadlocks (see spec §6 Risk 1).
pub struct AgentPreparationService {
    agent_repo: Arc<dyn AgentRepository>,
}

impl AgentPreparationService {
    pub fn new(agent_repo: Arc<dyn AgentRepository>) -> Self {
        Self { agent_repo }
    }

    /// Resolve an agent template and derive the metadata needed by downstream
    /// services. When the template lookup fails or returns `None`, returns a
    /// safe default that requires commits (treated as a write-capable worker).
    pub async fn prepare_agent(&self, agent_type: &str) -> DomainResult<AgentMetadata> {
        let (
            version,
            capabilities,
            cli_tools,
            can_write,
            is_read_only,
            max_turns,
            preferred_model,
            tier,
        ) = match self.agent_repo.get_template_by_name(agent_type).await {
            Ok(Some(template)) => {
                let caps: Vec<String> = template.tools.iter().map(|t| t.name.clone()).collect();
                let tools = map_template_tools_to_cli(&caps);
                let can_write = caps.iter().any(|c| {
                    let lower = c.to_lowercase();
                    lower == "write" || lower == "edit" || lower == "shell"
                });
                (
                    template.version,
                    caps,
                    tools,
                    can_write,
                    template.read_only,
                    template.max_turns,
                    template.preferred_model.clone(),
                    template.tier,
                )
            }
            // Default to true when template lookup fails (safer to require
            // commits from unknown agents).
            _ => (
                1,
                vec!["task-execution".to_string()],
                vec![],
                true,
                false,
                0,
                None,
                AgentTier::Worker,
            ),
        };

        let is_read_only_role = determine_read_only_role(agent_type, is_read_only);

        Ok(AgentMetadata {
            version,
            capabilities,
            cli_tools,
            can_write,
            is_read_only,
            max_turns,
            preferred_model,
            tier,
            is_read_only_role,
        })
    }
}

/// Read-only role heuristic.
///
/// Combines the template's explicit `read_only` flag with a legacy
/// name-based fallback for templates created before the field existed.
pub(crate) fn determine_read_only_role(agent_type: &str, is_template_read_only: bool) -> bool {
    if is_template_read_only {
        return true;
    }
    let lower = agent_type.to_lowercase();
    lower == "overmind"
        || lower == "aggregator"
        || lower.contains("researcher")
        || lower.contains("planner")
        || lower.contains("analyst")
        || lower.contains("architect")
}

/// Map agent template tool names (lowercase YAML) to Claude Code CLI tool names.
///
/// Template tools like "read", "shell", "memory" need to be translated to
/// the PascalCase names that `claude --allowedTools` expects.
/// Tools like "memory" and "tasks" are Abathur MCP tools, mapped to specific
/// `mcp__abathur__*` tool names. Use "task_status" for worker agents
/// (only task_update_status + task_get) and "tasks" for orchestrators.
pub(crate) fn map_template_tools_to_cli(template_tool_names: &[String]) -> Vec<String> {
    let mut cli_tools = Vec::new();

    for tool in template_tool_names {
        match tool.as_str() {
            "read" => cli_tools.push("Read".to_string()),
            "write" => {
                cli_tools.push("Write".to_string());
            }
            "edit" => {
                cli_tools.push("Edit".to_string());
                cli_tools.push("MultiEdit".to_string());
            }
            "shell" => cli_tools.push("Bash".to_string()),
            "glob" => cli_tools.push("Glob".to_string()),
            "grep" => cli_tools.push("Grep".to_string()),
            // Abathur APIs are provided via MCP stdio server as native tools.
            // Claude Code still needs these in --allowedTools to use them in headless mode.
            "memory" => {
                cli_tools.push("mcp__abathur__memory_search".to_string());
                cli_tools.push("mcp__abathur__memory_store".to_string());
                cli_tools.push("mcp__abathur__memory_get".to_string());
            }
            "tasks" => {
                cli_tools.push("mcp__abathur__task_submit".to_string());
                cli_tools.push("mcp__abathur__task_list".to_string());
                cli_tools.push("mcp__abathur__task_get".to_string());
                cli_tools.push("mcp__abathur__task_update_status".to_string());
                cli_tools.push("mcp__abathur__task_assign".to_string());
                cli_tools.push("mcp__abathur__task_wait".to_string());
                cli_tools.push("mcp__abathur__goals_list".to_string());
                cli_tools.push("mcp__abathur__workflow_select".to_string());
                cli_tools.push("mcp__abathur__workflow_advance".to_string());
                cli_tools.push("mcp__abathur__workflow_fan_out".to_string());
                cli_tools.push("mcp__abathur__workflow_gate".to_string());
                cli_tools.push("mcp__abathur__workflow_status".to_string());
                cli_tools.push("mcp__abathur__task_cancel".to_string());
                cli_tools.push("mcp__abathur__task_retry".to_string());
            }
            "task_status" => {
                cli_tools.push("mcp__abathur__task_update_status".to_string());
                cli_tools.push("mcp__abathur__task_get".to_string());
            }
            "agents" => {
                cli_tools.push("mcp__abathur__agent_create".to_string());
                cli_tools.push("mcp__abathur__agent_list".to_string());
                cli_tools.push("mcp__abathur__agent_get".to_string());
            }
            // Pass through any already-PascalCase tool names, but reject blocked tools
            other => {
                const BLOCKED: &[&str] = &[
                    "task",
                    "todowrite",
                    "todoread",
                    "taskcreate",
                    "taskupdate",
                    "tasklist",
                    "taskget",
                    "taskstop",
                    "taskoutput",
                    "teamcreate",
                    "teamdelete",
                    "sendmessage",
                    "enterplanmode",
                    "exitplanmode",
                    "skill",
                    "notebookedit",
                ];
                if BLOCKED.contains(&other.to_lowercase().as_str()) {
                    tracing::warn!(
                        "Agent template requested blocked tool '{}' - skipping",
                        other
                    );
                } else {
                    cli_tools.push(other.to_string());
                }
            }
        }
    }

    // Inject baseline read-only tools for agents that interact with code.
    // Orchestration-only agents (overmind, aggregator) should NOT get these —
    // they delegate to workers instead of exploring the codebase themselves.
    let is_orchestration_only = template_tool_names.iter().all(|t| {
        matches!(
            t.as_str(),
            "memory" | "tasks" | "agents" | "task_status" | "egress_publish"
        )
    });
    if !is_orchestration_only {
        for baseline in &["Read", "Glob", "Grep"] {
            if !cli_tools.contains(&baseline.to_string()) {
                cli_tools.push(baseline.to_string());
            }
        }
    }

    cli_tools.sort();
    cli_tools.dedup();
    cli_tools
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::sqlite::test_support;
    use crate::domain::models::{AgentTemplate, AgentTier, ToolCapability};

    #[tokio::test]
    async fn test_prepare_agent_resolves_template_metadata() {
        let agent_repo = test_support::setup_agent_repo().await;
        let mut tmpl = AgentTemplate::new("code-writer", AgentTier::Worker);
        tmpl.tools = vec![
            ToolCapability::new("read", "Read files"),
            ToolCapability::new("write", "Write files"),
            ToolCapability::new("edit", "Edit files"),
        ];
        tmpl.max_turns = 42;
        agent_repo.create_template(&tmpl).await.unwrap();

        let svc =
            AgentPreparationService::new(agent_repo as Arc<dyn AgentRepository>);
        let meta = svc.prepare_agent("code-writer").await.unwrap();

        assert_eq!(meta.max_turns, 42);
        assert_eq!(meta.tier, AgentTier::Worker);
        assert!(meta.can_write, "write/edit tools imply can_write");
        assert!(!meta.is_read_only, "template flag is false");
    }

    #[tokio::test]
    async fn test_prepare_agent_maps_capabilities_to_cli_tools() {
        let agent_repo = test_support::setup_agent_repo().await;
        let mut tmpl = AgentTemplate::new("worker", AgentTier::Worker);
        tmpl.tools = vec![
            ToolCapability::new("read", "Read files"),
            ToolCapability::new("shell", "Shell"),
            ToolCapability::new("memory", "Memory"),
        ];
        agent_repo.create_template(&tmpl).await.unwrap();

        let svc =
            AgentPreparationService::new(agent_repo as Arc<dyn AgentRepository>);
        let meta = svc.prepare_agent("worker").await.unwrap();

        assert!(meta.cli_tools.contains(&"Read".to_string()));
        assert!(meta.cli_tools.contains(&"Bash".to_string()));
        assert!(
            meta.cli_tools
                .contains(&"mcp__abathur__memory_store".to_string())
        );
        // Baseline injection: non-orchestration-only agents always get
        // Read/Glob/Grep.
        assert!(meta.cli_tools.contains(&"Glob".to_string()));
        assert!(meta.cli_tools.contains(&"Grep".to_string()));
    }

    #[tokio::test]
    async fn test_prepare_agent_detects_read_only_roles() {
        let agent_repo = test_support::setup_agent_repo().await;
        // Template flag wins, name doesn't matter
        let mut tmpl = AgentTemplate::new("custom-name", AgentTier::Worker);
        tmpl.read_only = true;
        agent_repo.create_template(&tmpl).await.unwrap();

        let svc = AgentPreparationService::new(agent_repo as Arc<dyn AgentRepository>);
        let meta = svc.prepare_agent("custom-name").await.unwrap();
        assert!(meta.is_read_only_role);

        // Name-based heuristic for legacy templates without the flag
        assert!(determine_read_only_role("overmind", false));
        assert!(determine_read_only_role("aggregator", false));
        assert!(determine_read_only_role("foo-researcher", false));
        assert!(determine_read_only_role("foo-planner", false));
        assert!(!determine_read_only_role("code-writer", false));
    }
}
