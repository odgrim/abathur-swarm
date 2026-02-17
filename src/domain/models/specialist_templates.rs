//! Baseline agent templates.
//!
//! The only pre-packaged agent is the Overmind. All other agents are
//! created dynamically by the Overmind at runtime via MCP tools.

use crate::domain::models::agent::{
    AgentConstraint, AgentTemplate, AgentTier, ToolCapability,
};
use crate::domain::models::workflow_template::{PhaseDependency, WorkflowTemplate};

/// Create all baseline agents.
///
/// Returns only the Overmind - the sole pre-packaged agent.
pub fn create_baseline_agents() -> Vec<AgentTemplate> {
    create_baseline_agents_with_workflow(None)
}

/// Create all baseline agents with an optional workflow template.
///
/// If `workflow` is `Some`, generates the Overmind prompt dynamically from
/// the workflow template. If `None`, uses the static `OVERMIND_SYSTEM_PROMPT`.
pub fn create_baseline_agents_with_workflow(
    workflow: Option<&WorkflowTemplate>,
) -> Vec<AgentTemplate> {
    vec![create_overmind_with_workflow(workflow)]
}

/// Overmind - The agentic orchestrator of the swarm.
///
/// The Overmind is the sole pre-packaged agent. It analyzes tasks,
/// creates whatever agents are needed dynamically via the `agent_create` MCP tool,
/// delegates work via the `task_submit` MCP tool, and tracks completion.
pub fn create_overmind() -> AgentTemplate {
    create_overmind_with_workflow(None)
}

/// Create the Overmind with an optional workflow template.
///
/// If `workflow` is `Some`, generates the system prompt dynamically from the
/// workflow template. If `None`, uses the static `OVERMIND_SYSTEM_PROMPT`.
pub fn create_overmind_with_workflow(workflow: Option<&WorkflowTemplate>) -> AgentTemplate {
    let prompt = match workflow {
        Some(wf) => generate_overmind_prompt(wf),
        None => OVERMIND_SYSTEM_PROMPT.to_string(),
    };

    let mut template = AgentTemplate::new("overmind", AgentTier::Architect)
        .with_description("Agentic orchestrator that analyzes tasks, dynamically creates agents, and delegates work through MCP tools")
        .with_prompt(prompt)
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

/// Generate a complete Overmind system prompt from a workflow template.
///
/// Concatenates the prompt prefix (core identity + MCP tools), a dynamically
/// generated workflow spine section, a concrete example, and the prompt suffix
/// (agent design principles + spawn limits + error handling).
pub fn generate_overmind_prompt(template: &WorkflowTemplate) -> String {
    let workflow_section = generate_workflow_prompt_section(template);
    let example_section = generate_workflow_example(template);
    format!(
        "{}\n{}\n{}\n{}",
        OVERMIND_PROMPT_PREFIX, workflow_section, example_section, OVERMIND_PROMPT_SUFFIX
    )
}

/// Generate the workflow spine section of the Overmind prompt.
fn generate_workflow_prompt_section(template: &WorkflowTemplate) -> String {
    let total_phases = template.phases.len() + 1; // +1 for implicit Memory Search phase
    let mut section = format!(
        "## Workflow Spine: {}\n\n{}\n\nEvery task MUST follow this {}-phase spine. Do NOT skip phases or jump straight to implementation.\n",
        capitalize(&template.name),
        if template.description.is_empty() {
            format!("Workflow with {} phases.", template.phases.len())
        } else {
            template.description.clone()
        },
        total_phases,
    );

    // Phase 1 is always Memory Search (implicit)
    section.push_str(
        "\n### Phase 1: Memory Search\nQuery swarm memory for similar past tasks, known patterns, and prior decisions via `memory_search`.\n",
    );

    // Each template phase becomes Phase N+1
    let has_review = template
        .phases
        .iter()
        .any(|p| p.name.to_lowercase() == "review");

    for (i, phase) in template.phases.iter().enumerate() {
        let phase_num = i + 2; // offset by 1 for memory search
        section.push_str(&format!(
            "\n### Phase {}: {}\n{}\n",
            phase_num,
            capitalize(&phase.name),
            phase.description,
        ));

        // Role description
        section.push_str(&format!("- **Role**: {}\n", phase.role));

        // Tools
        if !phase.tools.is_empty() {
            section.push_str(&format!(
                "- **Tools**: `{}`",
                phase.tools.join("`, `")
            ));
            if phase.read_only {
                section.push_str(" — read-only agent");
            }
            section.push('\n');
        }

        // Read-only flag
        if phase.read_only {
            section.push_str("- Set `read_only: true` when creating this agent.\n");
        }

        // Dependency instructions
        match phase.dependency {
            PhaseDependency::Root => {
                section.push_str(
                    "- This phase has no dependencies (it runs first).\n",
                );
            }
            PhaseDependency::Sequential => {
                if i == 0 {
                    section.push_str(
                        "- This phase has no task dependencies (memory search is done by the Overmind directly).\n",
                    );
                } else {
                    let prev_phase = &template.phases[i - 1];
                    section.push_str(&format!(
                        "- The {} task MUST `depends_on` the {} task UUID.\n",
                        phase.name,
                        prev_phase.name,
                    ));
                }
            }
            PhaseDependency::AllPrevious => {
                section.push_str(
                    "- This phase MUST `depends_on` ALL previous phase task UUIDs.\n",
                );
            }
        }

        // task_wait instructions
        if i < template.phases.len() - 1 {
            section.push_str(&format!(
                "- After submitting the {} task, call `task_wait` with the task UUID before proceeding to Phase {}.\n",
                phase.name,
                phase_num + 1,
            ));
        }
    }

    // Review loop instructions
    if has_review {
        section.push_str(
            "\nAfter review completion, the orchestrator's post-completion workflow automatically handles integration (PR creation or merge to main).\n\
            \n\
            When a review task fails because the implementation has issues, the system automatically loops back to create a new plan + implement + review cycle incorporating the review feedback. This loop is bounded by `max_review_iterations`. Ensure review tasks use `agent_type: \"code-reviewer\"` so the system can identify them.\n",
        );
    }

    section
}

/// Generate a concrete workflow example for the Overmind prompt.
fn generate_workflow_example(template: &WorkflowTemplate) -> String {
    let mut example = String::from("## Example: Full Workflow Spine\n\n```\n");

    // Phase 1: Memory Search
    example.push_str(
        "# Phase 1: Memory Search\n\
         tool: memory_search\n\
         arguments:\n\
         \x20 query: \"<relevant search terms>\"\n\n\
         # Check existing agents before creating any\n\
         tool: agent_list\n",
    );

    for (i, phase) in template.phases.iter().enumerate() {
        let phase_num = i + 2;
        let phase_name_cap = capitalize(&phase.name);
        let var_name = format!("{}_task_id", phase.name.replace('-', "_"));

        example.push_str(&format!(
            "\n# Phase {}: {}\n",
            phase_num, phase_name_cap,
        ));

        // agent_create example
        let tier_str = match phase.name.as_str() {
            "plan" | "review" => "specialist",
            _ => "worker",
        };

        example.push_str(&format!(
            "tool: agent_create\narguments:\n\
             \x20 name: \"{}-agent\"\n\
             \x20 description: \"{}\"\n\
             \x20 tier: \"{}\"\n\
             \x20 system_prompt: \"You are a {}. {}\"\n\
             \x20 tools:\n",
            phase.name, phase.role, tier_str, phase.role, phase.description,
        ));

        for tool in &phase.tools {
            example.push_str(&format!(
                "   - {{name: \"{}\", description: \"{}\" , required: true}}\n",
                tool,
                tool_description(tool),
            ));
        }
        example.push_str(
            "   - {name: \"task_status\", description: \"Mark task complete or failed\", required: true}\n",
        );

        if phase.read_only {
            example.push_str("  read_only: true\n");
        }

        example.push_str(&format!(
            "\ntool: task_submit\narguments:\n\
             \x20 title: \"{}: <specific task title>\"\n\
             \x20 description: \"<specific task instructions>\"\n\
             \x20 agent_type: \"{}-agent\"\n",
            phase_name_cap, phase.name,
        ));

        // depends_on
        if i > 0 {
            let prev_var = format!(
                "{}_task_id",
                template.phases[i - 1].name.replace('-', "_")
            );
            match phase.dependency {
                PhaseDependency::Root => {}
                PhaseDependency::Sequential => {
                    example.push_str(&format!(
                        "  depends_on: [\"<{}>\"]",
                        prev_var,
                    ));
                }
                PhaseDependency::AllPrevious => {
                    let all_prev: Vec<String> = template.phases[..i]
                        .iter()
                        .map(|p| format!("\"<{}_task_id>\"", p.name.replace('-', "_")))
                        .collect();
                    example.push_str(&format!(
                        "  depends_on: [{}]",
                        all_prev.join(", "),
                    ));
                }
            }
            example.push('\n');
        }

        example.push_str(&format!("# Returns {}\n", var_name));

        // task_wait
        if i < template.phases.len() - 1 {
            example.push_str(&format!(
                "\n# Wait for {} to complete\n\
                 tool: task_wait\narguments:\n\
                 \x20 id: \"<{}>\"\n",
                phase.name, var_name,
            ));
        }
    }

    example.push_str("```\n");
    example
}

/// Capitalize the first character of a string.
fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        None => String::new(),
        Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
    }
}

/// Get a short description for a tool name (used in examples).
fn tool_description(tool: &str) -> &'static str {
    match tool {
        "read" => "Read source files",
        "write" => "Write new files",
        "edit" => "Edit existing files",
        "shell" => "Run shell commands",
        "glob" => "Find files by pattern",
        "grep" => "Search code patterns",
        "memory" => "Query and store swarm memory",
        "task_status" => "Mark task complete or failed",
        "tasks" => "Interact with task queue",
        "agents" => "Create and manage agent templates",
        _ => "Tool",
    }
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
- **agent_create**: Create a new agent template. Required fields: `name`, `description`, `system_prompt`. Optional: `tier` (worker|specialist|architect, default: worker), `tools` (array of {name, description, required}), `constraints` (array of {name, description}), `max_turns` (default: 25), `read_only` (boolean, default: false — set to true for research/analysis/planning agents that produce findings via memory rather than code commits).
  - **Tool categories for agents**: `read`, `write`, `edit`, `shell`, `glob`, `grep`, `memory`, `task_status`, `tasks`, `agents`.
  - Use `task_status` for worker/specialist agents — it grants only `task_update_status` and `task_get` so agents can mark themselves complete and read their task details, but CANNOT create subtasks or list other tasks.
  - Use `tasks` only for orchestrator-level agents that need to create and manage subtasks.
  - Use `agents` only for agents that need to create other agent templates (almost never — only the Overmind needs this).

### Task Management
- **task_submit**: Create a subtask and delegate it to an agent. Required field: `description`. Optional: `title`, `agent_type` (name of agent template to execute this task), `depends_on` (array of task UUIDs that must complete first), `priority` (low|normal|high|critical, default: normal). The parent_id is set automatically from your current task context.
- **task_list**: List tasks, optionally filtered by `status` (pending|ready|running|complete|failed|blocked). Use this to track subtask progress.
- **task_get**: Get full task details by `id` (UUID). Use to check subtask results and failure reasons.
- **task_update_status**: Mark a task as `complete` or `failed`. Provide `error` message when failing a task.
- **task_wait**: Block until a task reaches a terminal state (complete, failed, or canceled). Pass `id` for a single task or `ids` for multiple tasks. Optional `timeout_seconds` (default: 600). Returns the final status. ALWAYS use this instead of polling with task_list + sleep loops — polling wastes your turn budget. For implementation tasks that use convergent execution, set `timeout_seconds` to at least 1800 (30 minutes) since convergent tasks may run multiple iterations.

### Memory
- **memory_search**: Search swarm memory by `query` string. Use before planning to find similar past tasks and known patterns.
- **memory_store**: Store a memory with `key` and `content`. Optional: `namespace`, `memory_type` (fact|code|decision|error|pattern|reference|context), `tier` (working|episodic|semantic).
- **memory_get**: Retrieve a specific memory by `id` (UUID).

### Goals
- **goals_list**: View active goals for context on overall project direction.

## Default Workflow Spine

Every task MUST follow this 5-phase spine. Do NOT skip phases or jump straight to implementation.

### Phase 1: Memory Search
Query swarm memory for similar past tasks, known patterns, and prior decisions via `memory_search`.

### Phase 2: Research
Create a **read-only research agent** to explore the codebase, understand existing patterns, identify files that need to change, and report findings back via task completion.
- Tools: `read`, `glob`, `grep` only — NO write, edit, or shell.
- The research task has no dependencies (it runs first).
- You MUST always create a research agent first. NEVER create an implementation agent without a preceding research task.
- After submitting the research task, call `task_wait` with the research task UUID before proceeding to Phase 3.

### Phase 3: Plan
Create a **domain-specific planning agent** to draft a concrete implementation plan based on the research findings.
- Pick the planner's domain based on what the research revealed (e.g., "database-schema-architect" for DB changes, "api-designer" for new endpoints, "systems-architect" for infrastructure changes, "rust-module-planner" for Rust refactoring).
- Tools: `read`, `glob`, `grep`, `memory` — read-only plus memory to store the plan.
- The planning task MUST `depends_on` the research task UUID.
- Do NOT use a generic "planner" agent. The planner must be a domain specialist.
- After submitting the plan task, call `task_wait` with the plan task UUID before proceeding to Phase 4.

### Phase 4: Implement
Create **implementation agents** with specific instructions derived from the planning phase.
- Implementation tasks MUST `depends_on` the planning task UUID.
- Split large implementations into parallel tasks where possible.
- After submitting implementation tasks, call `task_wait` with all implementation task UUIDs before proceeding to Phase 5.

### Phase 5: Review
Create a **code review agent** that reviews for correctness, edge cases, test coverage, and adherence to the plan.
- The review task MUST `depends_on` all implementation task UUIDs.

After review completion, the orchestrator's post-completion workflow automatically handles integration (PR creation or merge to main).

When a review task fails because the implementation has issues, the system automatically loops back to create a new plan + implement + review cycle incorporating the review feedback. This loop is bounded by `max_review_iterations`. Ensure review tasks use `agent_type: "code-reviewer"` so the system can identify them.

### Agent Reuse Policy

ALWAYS call `agent_list` before `agent_create`. Reuse an existing agent if one is suitable for the needed role. Only create a new agent when no existing agent covers the needed role. For example, if a "database-schema-architect" already exists from a previous task, reuse it for subsequent database planning tasks rather than creating a duplicate.

## Example: Full Workflow Spine

```
# Phase 1: Memory Search
tool: memory_search
arguments:
  query: "rate limiting middleware tower"

# Check existing agents before creating any
tool: agent_list

# Phase 2: Research - create read-only researcher (if none exists)
tool: agent_create
arguments:
  name: "codebase-researcher"
  description: "Read-only agent that explores codebases and reports findings"
  tier: "worker"
  system_prompt: "You are a codebase research specialist. Explore the code, identify patterns, relevant files, and dependencies. Report your findings clearly. You are read-only — do NOT attempt to modify any files."
  tools:
    - {name: "read", description: "Read source files", required: true}
    - {name: "glob", description: "Find files by pattern", required: true}
    - {name: "grep", description: "Search code patterns", required: true}
    - {name: "task_status", description: "Mark task complete or failed", required: true}
  max_turns: 15
  read_only: true

tool: task_submit
arguments:
  title: "Research: rate limiting patterns and middleware stack"
  description: "Explore the codebase to find: 1) existing middleware patterns, 2) tower service usage, 3) configuration patterns, 4) test patterns for middleware. Report all findings."
  agent_type: "codebase-researcher"
  priority: "normal"
# Returns research_task_id

# Wait for research to complete before planning
tool: task_wait
arguments:
  id: "<research_task_id>"
# → Returns when research completes, then proceed to Phase 3

# Phase 3: Plan - create domain-specific planner
tool: agent_create
arguments:
  name: "api-middleware-architect"
  description: "Plans API middleware implementations"
  tier: "specialist"
  system_prompt: "You are an API middleware architect. Based on research findings, draft concrete implementation plans with specific file changes, function signatures, and test strategies. Store your plan via memory_store."
  tools:
    - {name: "read", description: "Read source files", required: true}
    - {name: "glob", description: "Find files", required: true}
    - {name: "grep", description: "Search code", required: true}
    - {name: "memory", description: "Store implementation plan", required: true}
    - {name: "task_status", description: "Mark task complete or failed", required: true}
  max_turns: 15
  read_only: true

tool: task_submit
arguments:
  title: "Plan: rate limiting middleware design"
  description: "Based on research findings, design the rate limiting middleware. Specify: files to create/modify, data structures, configuration, error handling, and test plan. Store the plan in memory."
  agent_type: "api-middleware-architect"
  depends_on: ["<research_task_id>"]
  priority: "normal"
# Returns plan_task_id

# Wait for planning to complete before implementation
tool: task_wait
arguments:
  id: "<plan_task_id>"
# → Returns when planning completes, then proceed to Phase 4

# Phase 4: Implement
tool: agent_create
arguments:
  name: "rust-implementer"
  description: "Writes and modifies Rust code"
  tier: "worker"
  system_prompt: "You are a Rust implementation specialist. Follow the implementation plan exactly. Write clean, idiomatic Rust code following existing patterns. Run cargo check after changes."
  tools:
    - {name: "read", description: "Read source files", required: true}
    - {name: "write", description: "Write new files", required: true}
    - {name: "edit", description: "Edit existing files", required: true}
    - {name: "shell", description: "Run cargo commands", required: true}
    - {name: "glob", description: "Find files", required: false}
    - {name: "grep", description: "Search code", required: false}
    - {name: "memory", description: "Read implementation plan", required: false}
    - {name: "task_status", description: "Mark task complete or failed", required: true}
  constraints:
    - {name: "test-after-change", description: "Run cargo check after significant changes"}
  max_turns: 30

tool: task_submit
arguments:
  title: "Implement rate limiting middleware"
  description: "Follow the stored implementation plan. Add rate limiting to all API endpoints using tower middleware. Limit to 100 req/min per IP. Include tests."
  agent_type: "rust-implementer"
  depends_on: ["<plan_task_id>"]
  priority: "normal"
# Returns impl_task_id

# Wait for implementation to complete before review
tool: task_wait
arguments:
  id: "<impl_task_id>"
# → Returns when implementation completes, then proceed to Phase 5

# Phase 5: Review
tool: agent_create
arguments:
  name: "code-reviewer"
  description: "Reviews code for correctness and quality"
  tier: "specialist"
  system_prompt: "You are a code review specialist. Review changes for correctness, edge cases, error handling, test coverage, and adherence to the implementation plan. Report issues clearly."
  tools:
    - {name: "read", description: "Read source files", required: true}
    - {name: "glob", description: "Find files", required: true}
    - {name: "grep", description: "Search code", required: true}
    - {name: "shell", description: "Run tests", required: true}
    - {name: "memory", description: "Read implementation plan", required: false}
    - {name: "task_status", description: "Mark task complete or failed", required: true}
  max_turns: 15

tool: task_submit
arguments:
  title: "Review rate limiting implementation"
  description: "Review the rate limiting middleware for correctness, edge cases, performance, and test coverage. Verify it matches the implementation plan."
  agent_type: "code-reviewer"
  depends_on: ["<impl_task_id>"]
```

### Agent Design Principles

- **Always include `task_status` tool**: Every agent MUST have the `task_status` tool so it can mark its own task as complete or failed via `task_update_status`. Without this, agents cannot report completion and the task will stall until reconciliation recovers it. Use `task_status` (not `tasks`) — this gives agents only status reporting, not the ability to create subtasks.
- **Minimal tools**: Only grant tools the agent actually needs. Read-only agents don't need write/edit/shell. The `task_status` tool is the one exception — it is always required.
- **Focused prompts**: Each agent should have a clear, specific role. Don't create "do everything" agents.
- **Appropriate tier**: Use "worker" for task execution, "specialist" for domain expertise, "architect" for planning.
- **Constraints**: Add constraints that help the agent stay on track (e.g., "always run tests", "read-only").
- **Set `read_only: true`** for research, analysis, and planning agents that produce findings via memory rather than code commits. This disables commit verification and prevents convergence retry loops for non-coding agents.

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

/// Prompt prefix: core identity and MCP tools section.
///
/// Everything from the start of the Overmind prompt up to (but not including)
/// the "## Default Workflow Spine" section.
pub const OVERMIND_PROMPT_PREFIX: &str = r#"You are the Overmind - the sole orchestrating agent in the Abathur swarm system.

## Core Identity

You are the agentic orchestrator. When a task arrives, you analyze it, create whatever specialist agents are needed, delegate work, and track completion. You are the ONLY pre-packaged agent - all others are created by you at runtime.

You MUST delegate work by creating agents and submitting subtasks. Do NOT attempt to do implementation work yourself.

## Your MCP Tools

You have native MCP tools for interacting with the Abathur swarm. Use these directly — they are available in your tool list. Do NOT use WebFetch or HTTP requests.

### Agent Management
- **agent_list**: Check what agent templates already exist before creating new ones. Always call this first.
- **agent_get**: Get full details of an agent template by name, including its system prompt and tools.
- **agent_create**: Create a new agent template. Required fields: `name`, `description`, `system_prompt`. Optional: `tier` (worker|specialist|architect, default: worker), `tools` (array of {name, description, required}), `constraints` (array of {name, description}), `max_turns` (default: 25), `read_only` (boolean, default: false — set to true for research/analysis/planning agents that produce findings via memory rather than code commits).
  - **Tool categories for agents**: `read`, `write`, `edit`, `shell`, `glob`, `grep`, `memory`, `task_status`, `tasks`, `agents`.
  - Use `task_status` for worker/specialist agents — it grants only `task_update_status` and `task_get` so agents can mark themselves complete and read their task details, but CANNOT create subtasks or list other tasks.
  - Use `tasks` only for orchestrator-level agents that need to create and manage subtasks.
  - Use `agents` only for agents that need to create other agent templates (almost never — only the Overmind needs this).

### Task Management
- **task_submit**: Create a subtask and delegate it to an agent. Required field: `description`. Optional: `title`, `agent_type` (name of agent template to execute this task), `depends_on` (array of task UUIDs that must complete first), `priority` (low|normal|high|critical, default: normal). The parent_id is set automatically from your current task context.
- **task_list**: List tasks, optionally filtered by `status` (pending|ready|running|complete|failed|blocked). Use this to track subtask progress.
- **task_get**: Get full task details by `id` (UUID). Use to check subtask results and failure reasons.
- **task_update_status**: Mark a task as `complete` or `failed`. Provide `error` message when failing a task.
- **task_wait**: Block until a task reaches a terminal state (complete, failed, or canceled). Pass `id` for a single task or `ids` for multiple tasks. Optional `timeout_seconds` (default: 600). Returns the final status. ALWAYS use this instead of polling with task_list + sleep loops — polling wastes your turn budget. For implementation tasks that use convergent execution, set `timeout_seconds` to at least 1800 (30 minutes) since convergent tasks may run multiple iterations.

### Memory
- **memory_search**: Search swarm memory by `query` string. Use before planning to find similar past tasks and known patterns.
- **memory_store**: Store a memory with `key` and `content`. Optional: `namespace`, `memory_type` (fact|code|decision|error|pattern|reference|context), `tier` (working|episodic|semantic).
- **memory_get**: Retrieve a specific memory by `id` (UUID).

### Goals
- **goals_list**: View active goals for context on overall project direction.
"#;

/// Prompt suffix: agent design principles, spawn limits, and error handling.
///
/// Everything from "### Agent Reuse Policy" to the end of the Overmind prompt.
pub const OVERMIND_PROMPT_SUFFIX: &str = r#"### Agent Reuse Policy

ALWAYS call `agent_list` before `agent_create`. Reuse an existing agent if one is suitable for the needed role. Only create a new agent when no existing agent covers the needed role. For example, if a "database-schema-architect" already exists from a previous task, reuse it for subsequent database planning tasks rather than creating a duplicate.

### Agent Design Principles

- **Always include `task_status` tool**: Every agent MUST have the `task_status` tool so it can mark its own task as complete or failed via `task_update_status`. Without this, agents cannot report completion and the task will stall until reconciliation recovers it. Use `task_status` (not `tasks`) — this gives agents only status reporting, not the ability to create subtasks.
- **Minimal tools**: Only grant tools the agent actually needs. Read-only agents don't need write/edit/shell. The `task_status` tool is the one exception — it is always required.
- **Focused prompts**: Each agent should have a clear, specific role. Don't create "do everything" agents.
- **Appropriate tier**: Use "worker" for task execution, "specialist" for domain expertise, "architect" for planning.
- **Constraints**: Add constraints that help the agent stay on track (e.g., "always run tests", "read-only").
- **Set `read_only: true`** for research, analysis, and planning agents that produce findings via memory rather than code commits. This disables commit verification and prevents convergence retry loops for non-coding agents.

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
    use crate::domain::models::workflow_template::WorkflowPhase;

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

    #[test]
    fn test_create_overmind_with_no_workflow_matches_original() {
        let original = create_overmind();
        let via_workflow = create_overmind_with_workflow(None);

        // Both should use the same static prompt
        assert_eq!(original.system_prompt, via_workflow.system_prompt);
        assert_eq!(original.name, via_workflow.name);
        assert_eq!(original.tier, via_workflow.tier);
        assert_eq!(original.max_turns, via_workflow.max_turns);
    }

    #[test]
    fn test_create_baseline_agents_with_no_workflow_matches_original() {
        let original = create_baseline_agents();
        let via_workflow = create_baseline_agents_with_workflow(None);

        assert_eq!(original.len(), via_workflow.len());
        assert_eq!(original[0].system_prompt, via_workflow[0].system_prompt);
    }

    #[test]
    fn test_generate_overmind_prompt_with_default_workflow() {
        let wf = WorkflowTemplate::default_code_workflow();
        let prompt = generate_overmind_prompt(&wf);

        // Should contain prefix content
        assert!(prompt.contains("You are the Overmind"));
        assert!(prompt.contains("## Core Identity"));
        assert!(prompt.contains("## Your MCP Tools"));

        // Should contain workflow spine
        assert!(prompt.contains("Workflow Spine: Code"));
        assert!(prompt.contains("Phase 1: Memory Search"));
        assert!(prompt.contains("Phase 2: Research"));
        assert!(prompt.contains("Phase 3: Plan"));
        assert!(prompt.contains("Phase 4: Implement"));
        assert!(prompt.contains("Phase 5: Review"));

        // Should contain suffix content
        assert!(prompt.contains("Agent Reuse Policy"));
        assert!(prompt.contains("Agent Design Principles"));
        assert!(prompt.contains("Spawn Limits"));
        assert!(prompt.contains("Error Handling"));
    }

    #[test]
    fn test_generate_overmind_prompt_with_custom_workflow() {
        let wf = WorkflowTemplate {
            name: "docs".to_string(),
            description: "Documentation workflow".to_string(),
            phases: vec![
                WorkflowPhase {
                    name: "research".to_string(),
                    description: "Research the codebase".to_string(),
                    role: "Codebase researcher".to_string(),
                    tools: vec!["read".to_string(), "glob".to_string(), "grep".to_string()],
                    read_only: true,
                    dependency: PhaseDependency::Root,
                },
                WorkflowPhase {
                    name: "write-docs".to_string(),
                    description: "Write documentation".to_string(),
                    role: "Documentation writer".to_string(),
                    tools: vec![
                        "read".to_string(),
                        "write".to_string(),
                        "edit".to_string(),
                    ],
                    read_only: false,
                    dependency: PhaseDependency::Sequential,
                },
            ],
        };
        let prompt = generate_overmind_prompt(&wf);

        // Should have the correct phase count
        assert!(prompt.contains("3-phase spine")); // 2 template phases + 1 memory search
        assert!(prompt.contains("Phase 1: Memory Search"));
        assert!(prompt.contains("Phase 2: Research"));
        assert!(prompt.contains("Phase 3: Write-docs"));

        // Should NOT contain review loop instructions (no review phase)
        assert!(!prompt.contains("max_review_iterations"));
    }

    #[test]
    fn test_generate_workflow_with_all_previous_dependency() {
        let wf = WorkflowTemplate {
            name: "parallel".to_string(),
            description: "Workflow with all-previous dependency".to_string(),
            phases: vec![
                WorkflowPhase {
                    name: "task-a".to_string(),
                    description: "First task".to_string(),
                    role: "Worker A".to_string(),
                    tools: vec!["read".to_string()],
                    read_only: true,
                    dependency: PhaseDependency::Root,
                },
                WorkflowPhase {
                    name: "task-b".to_string(),
                    description: "Second task".to_string(),
                    role: "Worker B".to_string(),
                    tools: vec!["read".to_string()],
                    read_only: true,
                    dependency: PhaseDependency::Root,
                },
                WorkflowPhase {
                    name: "merge".to_string(),
                    description: "Merge results".to_string(),
                    role: "Merger".to_string(),
                    tools: vec!["read".to_string(), "write".to_string()],
                    read_only: false,
                    dependency: PhaseDependency::AllPrevious,
                },
            ],
        };
        let prompt = generate_overmind_prompt(&wf);
        assert!(prompt.contains("MUST `depends_on` ALL previous phase task UUIDs"));
    }

    #[test]
    fn test_create_overmind_with_workflow_uses_dynamic_prompt() {
        let wf = WorkflowTemplate::default_code_workflow();
        let overmind = create_overmind_with_workflow(Some(&wf));

        // Dynamic prompt should differ from static prompt (different formatting)
        // but should contain the same key sections
        assert!(overmind.system_prompt.contains("You are the Overmind"));
        assert!(overmind.system_prompt.contains("Phase 1: Memory Search"));
        assert!(overmind.system_prompt.contains("Research"));
        assert!(overmind.system_prompt.contains("Plan"));
        assert!(overmind.system_prompt.contains("Implement"));
        assert!(overmind.system_prompt.contains("Review"));

        // Should still have all the same tools and capabilities
        assert!(overmind.has_tool("read"));
        assert!(overmind.has_tool("shell"));
        assert!(overmind.has_tool("memory"));
        assert!(overmind.has_tool("tasks"));
        assert!(overmind.has_tool("agents"));
        assert!(overmind.has_capability("agent-creation"));
        assert!(overmind.has_capability("task-delegation"));
    }

    #[test]
    fn test_capitalize() {
        assert_eq!(capitalize("hello"), "Hello");
        assert_eq!(capitalize(""), "");
        assert_eq!(capitalize("a"), "A");
        assert_eq!(capitalize("Hello"), "Hello");
        assert_eq!(capitalize("code"), "Code");
    }

    #[test]
    fn test_prompt_prefix_is_subset_of_full_prompt() {
        // The prefix content should be present in the static OVERMIND_SYSTEM_PROMPT
        assert!(OVERMIND_SYSTEM_PROMPT.contains("## Core Identity"));
        assert!(OVERMIND_SYSTEM_PROMPT.contains("## Your MCP Tools"));
        assert!(OVERMIND_SYSTEM_PROMPT.contains("### Goals"));
    }

    #[test]
    fn test_prompt_suffix_is_subset_of_full_prompt() {
        // The suffix content should be present in the static OVERMIND_SYSTEM_PROMPT
        assert!(OVERMIND_SYSTEM_PROMPT.contains("Agent Reuse Policy"));
        assert!(OVERMIND_SYSTEM_PROMPT.contains("## Spawn Limits"));
        assert!(OVERMIND_SYSTEM_PROMPT.contains("## Error Handling"));
    }
}
