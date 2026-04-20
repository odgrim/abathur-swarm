//! Workspace provisioning service.
//!
//! Writes the per-worktree agent configuration files (`CLAUDE.md` and
//! `.claude/settings.json`) that Claude Code reads as project-level
//! instructions and tool permissions.
//!
//! Worktree creation itself is left on the orchestrator
//! ([`SwarmOrchestrator::provision_workspace_for_task`]) because it depends on
//! several orchestrator-internal services (worktree repo, event bus, parent
//! resolution). This module owns the bounded I/O that follows provisioning.
//!
//! Extracted from `goal_processing::spawn_task_agent` per spec T10
//! (`specs/T10-spawn-task-agent-extraction.md`).
#![allow(dead_code)]

const CLAUDE_MD_CONTENT: &str = "\
# Abathur Agent Rules

IMPORTANT: You are running inside the Abathur swarm orchestration system.

## Prohibited Tools
NEVER use these Claude Code built-in tools — they bypass Abathur's orchestration:
- Task (subagent spawner)
- TodoWrite / TodoRead
- TaskCreate, TaskUpdate, TaskList, TaskGet, TaskStop, TaskOutput
- TeamCreate, TeamDelete, SendMessage
- EnterPlanMode, ExitPlanMode
- Skill
- NotebookEdit

## How to manage work
- Advance workflow: Use `workflow_advance` or `workflow_fan_out` to create phase subtasks
- Change spine: Use `workflow_select` before first advance (if auto-selected spine is wrong)
- Cancel tasks: Use `task_cancel` to stop work that is no longer needed
- Retry failed tasks: Use `task_retry` to reset a failed task to Ready
- Create agents: Use the `agent_create` tool directly
- Track progress: Use `task_list` and `task_get` tools
- Store learnings: Use the `memory_store` tool directly

## Efficiency Rules
- Use Glob for file discovery — never shell ls or find.
- Use Grep to search code — never Read entire files looking for a pattern.
- NEVER re-read a file you already read this session.
- Store findings incrementally via memory_store as you go, not all at the end.
- When done, call task_update_status immediately — no self-verification reads.
- If retrying a task, call memory_search FIRST to find prior work and build on it.
";

/// Writes the per-worktree agent configuration files.
#[derive(Debug, Default, Clone, Copy)]
pub struct WorkspaceProvisioningService;

impl WorkspaceProvisioningService {
    pub fn new() -> Self {
        Self
    }

    /// Write `CLAUDE.md` and `.claude/settings.json` into the given worktree
    /// path. Failures are logged at warn level — they are best-effort and
    /// shouldn't abort task spawning.
    pub fn write_agent_config(&self, worktree_path: &str) {
        write_claude_md(worktree_path);
        write_settings_json(worktree_path);
    }
}

fn write_claude_md(worktree_path: &str) {
    let claude_md_path = std::path::Path::new(worktree_path).join("CLAUDE.md");
    if let Err(e) = std::fs::write(&claude_md_path, CLAUDE_MD_CONTENT) {
        tracing::warn!("Failed to write CLAUDE.md to worktree: {}", e);
    } else {
        tracing::debug!(
            "Wrote CLAUDE.md with tool restrictions to {:?}",
            claude_md_path
        );
    }
}

fn write_settings_json(worktree_path: &str) {
    let claude_dir = std::path::Path::new(worktree_path).join(".claude");
    let _ = std::fs::create_dir_all(&claude_dir);
    let tools: Vec<serde_json::Value> = crate::ABATHUR_ALLOWED_TOOLS
        .iter()
        .map(|t| serde_json::Value::String(t.to_string()))
        .collect();
    let settings_content = serde_json::json!({
        "permissions": {
            "allowedTools": tools
        }
    });
    if let Ok(pretty) = serde_json::to_string_pretty(&settings_content)
        && let Err(e) = std::fs::write(claude_dir.join("settings.json"), format!("{pretty}\n"))
    {
        tracing::warn!("Failed to write .claude/settings.json to worktree: {}", e);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_workspace_writes_claude_md_with_tool_restrictions() {
        let dir = tempfile::tempdir().unwrap();
        let svc = WorkspaceProvisioningService::new();
        svc.write_agent_config(dir.path().to_str().unwrap());

        let claude_md = std::fs::read_to_string(dir.path().join("CLAUDE.md")).unwrap();
        assert!(claude_md.contains("Abathur Agent Rules"));
        assert!(claude_md.contains("Prohibited Tools"));
        assert!(claude_md.contains("TodoWrite"));
    }

    #[test]
    fn test_workspace_writes_settings_json_with_allowed_tools() {
        let dir = tempfile::tempdir().unwrap();
        let svc = WorkspaceProvisioningService::new();
        svc.write_agent_config(dir.path().to_str().unwrap());

        let settings_path = dir.path().join(".claude").join("settings.json");
        assert!(settings_path.exists(), ".claude/settings.json should exist");
        let raw = std::fs::read_to_string(&settings_path).unwrap();
        let json: serde_json::Value = serde_json::from_str(&raw).unwrap();
        let tools = json["permissions"]["allowedTools"]
            .as_array()
            .expect("allowedTools array");
        assert!(!tools.is_empty(), "allowedTools should not be empty");
    }

    #[test]
    fn test_workspace_provision_creates_claude_dir() {
        let dir = tempfile::tempdir().unwrap();
        let svc = WorkspaceProvisioningService::new();
        svc.write_agent_config(dir.path().to_str().unwrap());
        assert!(dir.path().join(".claude").is_dir());
    }
}
