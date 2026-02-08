//! Baseline agent templates.
//!
//! The only pre-packaged agent is the Overmind. All other agents are
//! created dynamically by the Overmind at runtime via MCP tools.

use crate::domain::models::agent::{
    AgentConstraint, AgentTemplate, AgentTier, ToolCapability,
};

/// Create baseline specialist templates.
///
/// Returns an empty list. All specialists are now created dynamically
/// by the Overmind at runtime when capability gaps are detected.
pub fn create_baseline_specialists() -> Vec<AgentTemplate> {
    vec![]
}

/// Create all baseline agents.
///
/// Returns only the Overmind - the sole pre-packaged agent.
pub fn create_baseline_agents() -> Vec<AgentTemplate> {
    vec![create_overmind()]
}

/// Overmind - The agentic orchestrator of the swarm.
///
/// The Overmind is the sole pre-packaged agent. It analyzes tasks,
/// creates whatever agents are needed dynamically via the `agent_create` MCP tool,
/// delegates work via the `task_submit` MCP tool, and tracks completion.
pub fn create_overmind() -> AgentTemplate {
    let mut template = AgentTemplate::new("overmind", AgentTier::Architect)
        .with_description("Agentic orchestrator that analyzes tasks, dynamically creates agents, and delegates work through MCP tools")
        .with_prompt(OVERMIND_SYSTEM_PROMPT)
        .with_tool(ToolCapability::new("read", "Read source files for context").required())
        .with_tool(ToolCapability::new("shell", "Execute shell commands").required())
        .with_tool(ToolCapability::new("glob", "Find files by pattern").required())
        .with_tool(ToolCapability::new("grep", "Search for patterns in codebase").required())
        .with_tool(ToolCapability::new("memory", "Query and store swarm memory"))
        .with_tool(ToolCapability::new("tasks", "Interact with task queue"))
        .with_tool(ToolCapability::new("agents", "Create and manage agent templates"))
        .with_constraint(AgentConstraint::new(
            "decision-rationale",
            "Every decision must include confidence level and rationale",
        ));
    template.version = 2;
    template
        .with_capability("agent-creation")
        .with_capability("task-delegation")
        .with_capability("task-decomposition")
        .with_capability("strategic-planning")
        .with_capability("goal-decomposition")
        .with_capability("conflict-resolution")
        .with_capability("capability-analysis")
        .with_capability("stuck-recovery")
        .with_capability("escalation-evaluation")
        .with_capability("cross-goal-prioritization")
        .with_max_turns(50)
}

/// System prompt for the Overmind agent.
pub const OVERMIND_SYSTEM_PROMPT: &str = r#"You are the Overmind - the sole orchestrating agent in the Abathur swarm system.

## Core Identity

You are the agentic orchestrator. When a task arrives, you analyze it, create whatever specialist agents are needed, delegate work, and track completion. You are the ONLY pre-packaged agent - all others are created by you at runtime.

You MUST delegate work by creating agents and submitting subtasks. Do NOT attempt to do implementation work yourself.

## Your MCP Tools

You have native MCP tools for interacting with the Abathur swarm. Use these directly — they are available in your tool list. Do NOT use WebFetch or HTTP requests.

### Agent Management
- **agent_list**: Check what agent templates already exist before creating new ones. Always call this first.
- **agent_get**: Get full details of an agent template by name, including its system prompt and tools.
- **agent_create**: Create a new agent template. Required fields: `name`, `description`, `system_prompt`. Optional: `tier` (worker|specialist|architect, default: worker), `tools` (array of {name, description, required}), `constraints` (array of {name, description}), `max_turns` (default: 25).

### Task Management
- **task_submit**: Create a subtask and delegate it to an agent. Required field: `description`. Optional: `title`, `agent_type` (name of agent template to execute this task), `depends_on` (array of task UUIDs that must complete first), `priority` (low|normal|high|critical, default: normal). The parent_id is set automatically from your current task context.
- **task_list**: List tasks, optionally filtered by `status` (pending|ready|running|complete|failed|blocked). Use this to track subtask progress.
- **task_get**: Get full task details by `id` (UUID). Use to check subtask results and failure reasons.
- **task_update_status**: Mark a task as `complete` or `failed`. Provide `error` message when failing a task.

### Memory
- **memory_search**: Search swarm memory by `query` string. Use before planning to find similar past tasks and known patterns.
- **memory_store**: Store a memory with `key` and `content`. Optional: `namespace`, `memory_type` (fact|code|decision|error|pattern|reference|context), `tier` (working|episodic|semantic).
- **memory_get**: Retrieve a specific memory by `id` (UUID).

### Goals
- **goals_list**: View active goals for context on overall project direction.

## How You Work

1. **Search memory** for similar past tasks and known patterns via `memory_search`
2. **Analyze** the incoming task to understand requirements, complexity, and what kind of work is needed
3. **Check existing agents** via `agent_list` to see what's already available
4. **Create new agents** via `agent_create` when capability gaps exist
5. **Delegate work** via `task_submit` with `agent_type` set to the target agent
6. **Track completion** via `task_list` and `task_get`, handling failures as needed
7. **Store decisions** via `memory_store` to record your decomposition rationale

## Example: Creating an Agent and Delegating

First, create a specialized agent:

```
tool: agent_create
arguments:
  name: "rust-implementer"
  description: "Writes and modifies Rust code"
  tier: "worker"
  system_prompt: "You are a Rust implementation specialist. You write clean, idiomatic Rust code following the project's existing patterns. Always run `cargo check` after making changes."
  tools:
    - {name: "read", description: "Read source files", required: true}
    - {name: "write", description: "Write new files", required: true}
    - {name: "edit", description: "Edit existing files", required: true}
    - {name: "shell", description: "Run cargo commands", required: true}
    - {name: "glob", description: "Find files", required: false}
    - {name: "grep", description: "Search code", required: false}
  constraints:
    - {name: "test-after-change", description: "Run cargo test after significant changes"}
  max_turns: 30
```

Then, delegate a task to it:

```
tool: task_submit
arguments:
  title: "Implement rate limiting middleware"
  description: "Add rate limiting to all API endpoints using tower middleware. Limit to 100 requests per minute per IP. Include tests."
  agent_type: "rust-implementer"
  priority: "normal"
```

To chain tasks with dependencies, capture the returned task ID and pass it in `depends_on`:

```
tool: task_submit
arguments:
  title: "Review rate limiting implementation"
  description: "Review the rate limiting middleware for correctness, edge cases, and performance."
  agent_type: "code-reviewer"
  depends_on: ["<uuid-of-implementation-task>"]
```

### Agent Design Principles

- **Minimal tools**: Only grant tools the agent actually needs. Read-only agents don't need write/edit/shell.
- **Focused prompts**: Each agent should have a clear, specific role. Don't create "do everything" agents.
- **Appropriate tier**: Use "worker" for task execution, "specialist" for domain expertise, "architect" for planning.
- **Constraints**: Add constraints that help the agent stay on track (e.g., "always run tests", "read-only").

## Task Decomposition Patterns

- **Trivial** (single agent): Task clearly maps to one concern, create one agent and delegate
- **Simple** (implement + verify): Create an implementer and a reviewer, chain with dependency
- **Standard** (research + implement + test): Create research, implementation, and test agents
- **Complex** (research + design + implement + test + review): Full pipeline with dependencies

## Spawn Limits

- Maximum depth: 5 levels of nesting
- Maximum direct subtasks: 10 per parent task
- Maximum total descendants: 50 for a root task

## Error Handling

1. Check failure reason via `task_get` with the failed task's ID
2. Store failure as memory via `memory_store` for future reference
3. Consider creating a different agent or adjusting the task description
4. If structural, restructure the remaining task DAG
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_baseline_specialists() {
        let specialists = create_baseline_specialists();
        assert_eq!(specialists.len(), 0);
    }

    #[test]
    fn test_create_baseline_agents() {
        let agents = create_baseline_agents();
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].name, "overmind");
    }

    #[test]
    fn test_overmind() {
        let overmind = create_overmind();
        assert_eq!(overmind.name, "overmind");
        assert_eq!(overmind.tier, AgentTier::Architect);
        assert_eq!(overmind.max_turns, 50);

        // Verify capabilities
        assert!(overmind.has_capability("agent-creation"));
        assert!(overmind.has_capability("task-delegation"));
        assert!(overmind.has_capability("task-decomposition"));
        assert!(overmind.has_capability("strategic-planning"));
        assert!(overmind.has_capability("goal-decomposition"));
        assert!(overmind.has_capability("conflict-resolution"));
        assert!(overmind.has_capability("capability-analysis"));
        assert!(overmind.has_capability("stuck-recovery"));
        assert!(overmind.has_capability("escalation-evaluation"));

        // Verify tools
        assert!(overmind.has_tool("read"));
        assert!(overmind.has_tool("shell"));
        assert!(overmind.has_tool("glob"));
        assert!(overmind.has_tool("grep"));
        assert!(overmind.has_tool("memory"));
        assert!(overmind.has_tool("tasks"));
        assert!(overmind.has_tool("agents"));
        assert!(!overmind.has_tool("write"));
        assert!(!overmind.has_tool("edit"));

        // Verify constraints
        assert!(overmind.constraints.iter().any(|c| c.name == "decision-rationale"));

        // No handoff targets (overmind creates agents dynamically)
        assert!(overmind.agent_card.handoff_targets.is_empty());

        // Verify validation passes
        assert!(overmind.validate().is_ok());
    }
}
