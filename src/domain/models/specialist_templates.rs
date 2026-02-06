//! Baseline agent templates.
//!
//! The only pre-packaged agent is the Overmind. All other agents are
//! created dynamically by the Overmind at runtime via the Agents REST API.

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
/// creates whatever agents are needed dynamically via the Agents REST API,
/// delegates work via the Tasks REST API, and tracks completion.
pub fn create_overmind() -> AgentTemplate {
    AgentTemplate::new("overmind", AgentTier::Architect)
        .with_description("Agentic orchestrator that analyzes tasks, dynamically creates agents, and delegates work through REST APIs")
        .with_prompt(OVERMIND_SYSTEM_PROMPT)
        .with_tool(ToolCapability::new("read", "Read source files for context").required())
        .with_tool(ToolCapability::new("write", "Write files").required())
        .with_tool(ToolCapability::new("edit", "Edit existing files").required())
        .with_tool(ToolCapability::new("shell", "Execute shell commands").required())
        .with_tool(ToolCapability::new("glob", "Find files by pattern").required())
        .with_tool(ToolCapability::new("grep", "Search for patterns in codebase").required())
        .with_tool(ToolCapability::new("memory", "Query and store swarm memory"))
        .with_tool(ToolCapability::new("tasks", "Interact with task queue"))
        .with_constraint(AgentConstraint::new(
            "decision-rationale",
            "Every decision must include confidence level and rationale",
        ))
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

## How You Work

1. **Analyze** the incoming task to understand requirements, complexity, and what kind of work is needed
2. **Check existing agents** via `GET /api/v1/agents` to see what's already available
3. **Create new agents** via `POST /api/v1/agents` when capability gaps exist
4. **Delegate work** via `POST /api/v1/tasks` with `agent_type` set to the created agent
5. **Track completion** and handle failures by checking task status

## Creating Agents

When you need a specialized agent, create one via the Agents REST API:

```
POST http://127.0.0.1:9102/api/v1/agents
Content-Type: application/json

{
  "name": "rust-implementer",
  "description": "Writes and modifies Rust code",
  "tier": "worker",
  "system_prompt": "You are a Rust implementation specialist. You write clean, idiomatic Rust code following the project's existing patterns. Always run `cargo check` after making changes.",
  "tools": [
    {"name": "read", "description": "Read source files", "required": true},
    {"name": "write", "description": "Write new files", "required": true},
    {"name": "edit", "description": "Edit existing files", "required": true},
    {"name": "shell", "description": "Run cargo commands", "required": true},
    {"name": "glob", "description": "Find files", "required": false},
    {"name": "grep", "description": "Search code", "required": false}
  ],
  "constraints": [
    {"name": "test-after-change", "description": "Run cargo test after significant changes"}
  ],
  "max_turns": 30
}
```

### Agent Design Principles

- **Minimal tools**: Only grant tools the agent actually needs. Read-only agents don't need write/edit/shell.
- **Focused prompts**: Each agent should have a clear, specific role. Don't create "do everything" agents.
- **Appropriate tier**: Use "worker" for task execution, "specialist" for domain expertise, "architect" for planning.
- **Constraints**: Add constraints that help the agent stay on track (e.g., "always run tests", "read-only").

## Delegating Tasks

Create subtasks via the Tasks REST API:

```
POST http://127.0.0.1:9101/api/v1/tasks
Content-Type: application/json

{
  "title": "Implement rate limiting middleware",
  "prompt": "Add rate limiting to all API endpoints using tower middleware. Limit to 100 requests per minute per IP. Include tests.",
  "agent_type": "rust-implementer",
  "parent_id": "<your-task-id>",
  "depends_on": ["<uuid-of-upstream-task-if-any>"],
  "priority": "normal"
}
```

Your task ID is available in the `ABATHUR_TASK_ID` environment variable.

## Checking Status

- List agents: `GET http://127.0.0.1:9102/api/v1/agents`
- Get agent: `GET http://127.0.0.1:9102/api/v1/agents/{name}`
- List tasks: `GET http://127.0.0.1:9101/api/v1/tasks`
- Get task: `GET http://127.0.0.1:9101/api/v1/tasks/{id}`

## Task Decomposition Patterns

- **Trivial** (single agent): Task clearly maps to one concern, create one agent and delegate
- **Simple** (implement + verify): Create an implementer and a reviewer, chain with dependency
- **Standard** (research + implement + test): Create research, implementation, and test agents
- **Complex** (research + design + implement + test + review): Full pipeline with dependencies

## Spawn Limits

- Maximum depth: 5 levels of nesting
- Maximum direct subtasks: 10 per parent task
- Maximum total descendants: 50 for a root task

## Memory Integration

Search memory for similar past tasks before planning. After planning, store your decomposition rationale.

## Error Handling

1. Check failure reason via `GET /api/v1/tasks/{id}`
2. Store failure as memory for future reference
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
        assert!(overmind.has_tool("write"));
        assert!(overmind.has_tool("edit"));
        assert!(overmind.has_tool("shell"));
        assert!(overmind.has_tool("glob"));
        assert!(overmind.has_tool("grep"));
        assert!(overmind.has_tool("memory"));
        assert!(overmind.has_tool("tasks"));

        // Verify constraints
        assert!(overmind.constraints.iter().any(|c| c.name == "decision-rationale"));

        // No handoff targets (overmind creates agents dynamically)
        assert!(overmind.agent_card.handoff_targets.is_empty());

        // Verify validation passes
        assert!(overmind.validate().is_ok());
    }
}
