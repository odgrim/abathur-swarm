//! Baseline agent templates.
//!
//! A single `overmind` agent is seeded at startup. It receives all tasks regardless of
//! source and selects the appropriate workflow spine at runtime based on task content.
//! All other agents are created dynamically by the Overmind at runtime via MCP tools.

use crate::domain::models::agent::{
    AgentConstraint, AgentTemplate, AgentTier, ToolCapability,
};
use crate::domain::models::workflow_template::{
    OutputDelivery, PhaseDependency, WorkspaceKind, WorkflowTemplate,
};

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

/// Create all baseline agents with awareness of all configured workflow spines.
///
/// The single Overmind is seeded with a routing-aware prompt that describes all
/// provided workflows and teaches the Overmind to select the appropriate spine
/// based on task content at runtime.
pub fn create_baseline_agents_with_workflows(workflows: &[WorkflowTemplate]) -> Vec<AgentTemplate> {
    vec![create_overmind_with_workflows(workflows)]
}

/// Overmind - The agentic orchestrator of the swarm.
///
/// Receives all tasks and selects the appropriate workflow spine at runtime.
pub fn create_overmind() -> AgentTemplate {
    create_overmind_with_workflow(None)
}

/// Create the Overmind with an optional workflow template.
///
/// If `workflow` is `Some`, generates the system prompt dynamically from the
/// workflow template. If `None`, uses the static `OVERMIND_SYSTEM_PROMPT`.
pub fn create_overmind_with_workflow(workflow: Option<&WorkflowTemplate>) -> AgentTemplate {
    let has_triage = workflow
        .map(|w| w.phases.iter().any(|p| p.name.to_lowercase() == "triage"))
        .unwrap_or(false);
    let prompt = match workflow {
        Some(wf) => generate_overmind_prompt(wf),
        None => OVERMIND_SYSTEM_PROMPT.to_string(),
    };
    build_overmind_template(prompt, has_triage)
}

/// Create the Overmind with awareness of all configured workflow spines.
///
/// Generates a routing-aware prompt that describes each workflow and teaches the
/// Overmind to select the appropriate spine based on task content at runtime.
pub fn create_overmind_with_workflows(workflows: &[WorkflowTemplate]) -> AgentTemplate {
    let has_triage = workflows
        .iter()
        .any(|w| w.phases.iter().any(|p| p.name.to_lowercase() == "triage"));
    let prompt = generate_overmind_prompt_multi(workflows);
    build_overmind_template(prompt, has_triage)
}

/// Build the Overmind `AgentTemplate` from a pre-generated prompt.
///
/// Shared by `create_overmind_with_workflow` and `create_overmind_with_workflows`
/// so the tool list, constraints, and capabilities are defined in one place.
fn build_overmind_template(prompt: String, has_triage: bool) -> AgentTemplate {
    let mut template = AgentTemplate::new("overmind", AgentTier::Architect)
        .with_description("Agentic orchestrator that analyzes tasks, selects the appropriate workflow spine, dynamically creates agents, and delegates work through MCP tools")
        .with_prompt(prompt)
        .with_tool(ToolCapability::new("read", "Read source files for context").required())
        .with_tool(ToolCapability::new("shell", "Execute shell commands").required())
        .with_tool(ToolCapability::new("glob", "Find files by pattern").required())
        .with_tool(ToolCapability::new("grep", "Search for patterns in codebase").required())
        .with_tool(ToolCapability::new("memory", "Query and store swarm memory"))
        .with_tool(ToolCapability::new("tasks", "Interact with task queue"))
        .with_tool(ToolCapability::new("agents", "Create and manage agent templates"));

    if has_triage {
        template = template.with_tool(ToolCapability::new(
            "egress_publish",
            "Execute egress actions against external adapters — close issues, post comments, \
             create pull requests. Required for acting on triage rejections.",
        ));
    }

    let mut template = template.with_constraint(AgentConstraint::new(
        "decision-rationale",
        "Every decision must include confidence level and rationale",
    ));
    template.version = 3;
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

/// Generate a complete Overmind system prompt from a single workflow template.
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

/// Generate a routing-aware Overmind system prompt from all configured workflow templates.
///
/// Produces a prompt with a routing section (how to pick a spine) followed by the full
/// phase details for each workflow. When `workflows` is empty, falls back to the static
/// prompt. When only one workflow is provided, delegates to `generate_overmind_prompt`.
pub fn generate_overmind_prompt_multi(workflows: &[WorkflowTemplate]) -> String {
    if workflows.is_empty() {
        return OVERMIND_SYSTEM_PROMPT.to_string();
    }
    if workflows.len() == 1 {
        return generate_overmind_prompt(&workflows[0]);
    }

    let routing_section = generate_workflow_routing_section(workflows);

    // Full phase details for every workflow, clearly separated
    let workflow_sections: String = workflows
        .iter()
        .map(|wf| generate_workflow_prompt_section(wf))
        .collect::<Vec<_>>()
        .join("\n---\n\n");

    // Use the most instructive workflow as the example:
    // prefer one with a triage phase (shows the full flow), then "code", then the first.
    let example_wf = workflows
        .iter()
        .find(|w| w.phases.iter().any(|p| p.name.to_lowercase() == "triage"))
        .or_else(|| workflows.iter().find(|w| w.name == "code"))
        .unwrap_or(&workflows[0]);
    let example_section = generate_workflow_example(example_wf);

    format!(
        "{}\n{}\n{}\n{}\n{}",
        OVERMIND_PROMPT_PREFIX, routing_section, workflow_sections, example_section, OVERMIND_PROMPT_SUFFIX
    )
}

/// Generate the workflow selection / routing section of the Overmind prompt.
///
/// Produces a brief decision table and priority-ordered routing rules so the
/// Overmind knows which spine to choose before starting any task.
fn generate_workflow_routing_section(workflows: &[WorkflowTemplate]) -> String {
    let has_triage = workflows
        .iter()
        .any(|w| w.phases.iter().any(|p| p.name.to_lowercase() == "triage"));

    let mut section = String::from(
        "## Workflow Selection\n\n\
         Before starting any task, inspect the task description and select the \
         appropriate workflow spine. Each spine defines a mandatory phase sequence — \
         follow it completely from Phase 1 (Memory Search).\n\n\
         ### Available Spines\n\n",
    );

    for wf in workflows {
        let phases = std::iter::once("Memory Search".to_string())
            .chain(wf.phases.iter().map(|p| capitalize(&p.name)))
            .collect::<Vec<_>>()
            .join(" → ");
        let desc = if wf.description.is_empty() {
            format!("{} phases", wf.phases.len())
        } else {
            wf.description.clone()
        };
        section.push_str(&format!("- **{}** — {} _({})\n", wf.name, desc, phases));
    }

    section.push_str("\n### Routing Rules (apply in order)\n\n");
    if has_triage {
        section.push_str(
            "1. Task description begins with `[Ingested from ...]` → **external** \
             (adapter-sourced; triage before code spine)\n",
        );
    }
    section.push_str(
        "2. Task is investigation or analysis only, no code deliverable → **analysis**\n\
         3. Task is documentation only → **docs**\n\
         4. Task is a code review request only → **review**\n\
         5. Everything else → **code**\n\n\
         After selecting a spine, scroll to its `## Workflow Spine: <Name>` section below \
         and follow the phases in order. Do **not** mix phases from different spines.\n\n\
         ---\n\n",
    );

    section
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

    let has_triage = template
        .phases
        .iter()
        .any(|p| p.name.to_lowercase() == "triage");

    // Document egress_publish for triage workflows so the Overmind knows how
    // to close rejected issues.
    if has_triage {
        section.push_str(
            "\n### Egress Actions (Triage Workflows)\n\
             You have access to **`egress_publish`** in this workflow. Use it during triage to \
             act on rejected issues:\n\
             - Post a comment explaining the rejection: \
               `adapter: \"<adapter-name>\", action: {action: \"post_comment\", external_id: \"<id>\", body: \"<reason>\"}`\n\
             - Close the issue as out-of-scope: \
               `adapter: \"<adapter-name>\", action: {action: \"update_status\", external_id: \"<id>\", new_status: \"wontfix\"}`\n\
             Parse the adapter name and external_id from the `[Ingested from <adapter> — <external_id>]` \
             header in this task's description.\n",
        );
    }

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
                // Fan-out guidance for root phases (typically research).
                // Triage is a single focused evaluation — no fan-out.
                if phase.read_only && phase.name.to_lowercase() != "triage" {
                    section.push_str(
                        "- **Fan-out heuristic**: If the task touches 3+ distinct codebase areas, create one subtask per area running in parallel. Each stores findings via `memory_store` with a shared namespace and unique key. Use `task_wait` with `ids` array to wait for all.\n",
                    );
                }
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

        // Post-triage branching: read the verdict and either proceed or reject.
        if phase.name.to_lowercase() == "triage" {
            section.push_str(
                "\n**Triage only applies to adapter-sourced tasks.** Check the task description \
                 before doing anything else:\n\
                 - If it does **not** begin with `[Ingested from ...]`, this task was created \
                   internally. Skip triage entirely and proceed directly to the next phase.\n\
                 - If it **does** begin with `[Ingested from ...]`, run triage as described \
                   below.\n\
                 \n\
                 When running triage:\n\
                 - Use `execution_mode: \"direct\"` — triage is a single-pass evaluation, not \
                   iterative.\n\
                 - The triage agent MUST store its verdict in memory: \
                   `namespace: \"triage\", key: \"verdict\"`, content: `\"APPROVED\"` or \
                   `\"REJECTED: <reason>\"`.\n\
                 \n\
                 **After `task_wait` returns for triage**, retrieve the verdict with \
                 `memory_search` (query: `\"triage verdict\"`) and act on it:\n\
                 - **APPROVED** → proceed to the next phase. Triage succeeded.\n\
                 - **REJECTED** →\n\
                   1. Parse adapter name and `external_id` from the \
                      `[Ingested from <adapter> — <external_id>]` header.\n\
                   2. Call `egress_publish` to post a comment explaining the rejection reason.\n\
                   3. Call `egress_publish` with `action: update_status`, \
                      `new_status: \"wontfix\"` to close the issue.\n\
                   4. Call `task_update_status` to mark **this** task as `complete` — \
                      triage succeeded; rejecting an invalid issue is the correct outcome.\n\
                   5. **STOP** — do not proceed to the next phase.\n",
            );
        }
    }

    // Review loop instructions
    if has_review {
        section.push_str(
            "\nAfter review completion, the orchestrator's post-completion workflow automatically handles integration (PR creation or merge to main).\n\
            \n\
            When a review task fails because the implementation has issues, the system automatically loops back to create a new plan + implement + review cycle incorporating the review feedback. This loop is bounded by `max_review_iterations`. Ensure review tasks use `agent_type: \"code-reviewer\"` so the system can identify them.\n\
            \n\
            IMPORTANT — Do NOT spawn your own fix when a review task fails. After `task_wait` returns with a failed review task, call `task_get(review_task_id)` and inspect context.custom:\n\
            - If `review_loop_successor` is present, the system has already created the next review cycle. Call `task_wait(review_loop_successor)` to wait for it. Follow the chain if that also fails.\n\
            - If `review_loop_active` is present without a successor, the system is handling it — wait briefly then re-check before taking action.\n\
            Never independently spawn a new plan→implement→review cycle when the review loop is active; doing so creates conflicting parallel work tracks.\n",
        );
    }

    // Execution Environment section
    section.push_str("\n## Execution Environment\n\n");

    // Workspace
    match template.workspace_kind {
        WorkspaceKind::Worktree => {
            section.push_str(
                "**Workspace**: A dedicated git worktree is provisioned for this workflow. \
                 All agents run in an isolated branch. Work is committed to the branch and \
                 reviewed before merging.\n",
            );
        }
        WorkspaceKind::TempDir => {
            section.push_str(
                "**Workspace**: A temporary directory is provisioned for this workflow. \
                 Agents can write files but the directory is not a git repository. \
                 Artifacts must be stored via `memory_store` or delivered via the configured \
                 output delivery mechanism.\n",
            );
        }
        WorkspaceKind::None => {
            section.push_str(
                "**Workspace**: No workspace is provisioned for this workflow. \
                 Agents operate in read-only mode. All findings must be stored \
                 via `memory_store` for downstream consumption.\n",
            );
        }
    }

    // Output delivery
    match template.output_delivery {
        OutputDelivery::PullRequest => {
            section.push_str(
                "**Output Delivery**: When the workflow completes, a pull request is \
                 automatically created for the feature branch. Include a clear commit \
                 history — commits become the PR description.\n",
            );
        }
        OutputDelivery::DirectMerge => {
            section.push_str(
                "**Output Delivery**: When the workflow completes, the feature branch is \
                 merged directly without a PR. Ensure all work is committed before \
                 the final phase completes.\n",
            );
        }
        OutputDelivery::MemoryOnly => {
            section.push_str(
                "**Output Delivery**: This workflow produces no git artifacts. All output \
                 MUST be stored in swarm memory via `memory_store` with a meaningful \
                 namespace and key. The task is considered complete when findings are \
                 persisted to memory.\n",
            );
        }
    }

    // Template-level tool grants
    if !template.tool_grants.is_empty() {
        section.push_str(&format!(
            "**Additional Tools**: The following tools are granted to all phases in this \
             workflow in addition to phase-level grants: `{}`.\n",
            template.tool_grants.join("`, `"),
        ));
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
            "plan" | "review" | "triage" => "specialist",
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
            // Root phases may fan-out; show ids array pattern
            if phase.dependency == PhaseDependency::Root && phase.read_only {
                example.push_str(&format!(
                    "\n# Wait for all {} tasks to complete\n\
                     tool: task_wait\narguments:\n\
                     \x20 ids: [\"<{}_1>\", \"<{}_2>\"]\n",
                    phase.name, var_name, var_name,
                ));
            } else {
                example.push_str(&format!(
                    "\n# Wait for {} to complete\n\
                     tool: task_wait\narguments:\n\
                     \x20 id: \"<{}>\"\n",
                    phase.name, var_name,
                ));
            }
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
- **task_submit**: Create a subtask and delegate it to an agent. Required field: `description`. Optional: `title`, `agent_type` (name of agent template to execute this task), `depends_on` (array of task UUIDs that must complete first), `priority` (low|normal|high|critical, default: normal), `execution_mode` ("direct" or "convergent" — convergent uses iterative refinement with intent verification; recommended for implementation tasks; if omitted the system selects automatically). The parent_id is set automatically from your current task context.
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
Create **read-only research agents** to explore the codebase. Decompose the research into parallel, domain-scoped subtasks.
- Tools: `read`, `glob`, `grep`, `memory` — read-only. Include `task_status` so agents can mark completion.
- Research tasks have no dependencies (they run first).
- **Fan-out heuristic**: If the task touches 3+ distinct codebase areas (e.g., config system, CLI, domain models), create one research subtask per area. Each subtask should store its findings via `memory_store` with a shared namespace and a unique key. If the task is narrow (1-2 areas), a single research task is fine.
- Include `memory` in the researcher's tools so findings survive even if the agent hits its turn limit.
- After submitting all research tasks, call `task_wait` with ALL research task UUIDs (using the `ids` array parameter) before proceeding to Phase 3.
- You MUST always create research tasks first. NEVER create an implementation agent without preceding research.

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

IMPORTANT — Do NOT spawn your own fix when a review task fails. After `task_wait` returns with a failed review task, call `task_get(review_task_id)` and inspect context.custom:
- If `review_loop_successor` is present, the system has already created the next review cycle. Call `task_wait(review_loop_successor)` to wait for it. Follow the chain if that also fails.
- If `review_loop_active` is present without a successor, the system is handling it — wait briefly then re-check before taking action.
Never independently spawn a new plan→implement→review cycle when the review loop is active; doing so creates conflicting parallel work tracks.

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
  description: "Read-only agent that explores codebases and reports findings via memory"
  tier: "worker"
  system_prompt: "You are a codebase research specialist. Explore the code, identify patterns, relevant files, and dependencies. Store your findings via memory_store with the namespace given in your task description. You are read-only — do NOT attempt to modify any files."
  tools:
    - {name: "read", description: "Read source files", required: true}
    - {name: "glob", description: "Find files by pattern", required: true}
    - {name: "grep", description: "Search code patterns", required: true}
    - {name: "memory", description: "Store research findings", required: true}
    - {name: "task_status", description: "Mark task complete or failed", required: true}
  max_turns: 15
  read_only: true

# Fan-out: one task per research domain
tool: task_submit
arguments:
  title: "Research: middleware stack and tower service usage"
  description: "Explore the codebase to find existing middleware patterns and tower service usage. Store findings via memory_store with namespace 'rate-limiting-research' and key 'middleware-stack'."
  agent_type: "codebase-researcher"
  priority: "normal"
# Returns research_task_id_1

tool: task_submit
arguments:
  title: "Research: configuration and test patterns"
  description: "Explore configuration patterns and test patterns for middleware. Store findings via memory_store with namespace 'rate-limiting-research' and key 'config-and-tests'."
  agent_type: "codebase-researcher"
  priority: "normal"
# Returns research_task_id_2

# Wait for ALL research to complete before planning
tool: task_wait
arguments:
  ids: ["<research_task_id_1>", "<research_task_id_2>"]
# → Returns when all research completes, then proceed to Phase 3

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
  execution_mode: "convergent"
# Returns impl_task_id

# Wait for implementation to complete before review (convergent tasks need longer timeout)
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
- **task_submit**: Create a subtask and delegate it to an agent. Required field: `description`. Optional: `title`, `agent_type` (name of agent template to execute this task), `depends_on` (array of task UUIDs that must complete first), `priority` (low|normal|high|critical, default: normal), `execution_mode` ("direct" or "convergent" — convergent uses iterative refinement with intent verification; recommended for implementation tasks; if omitted the system selects automatically). The parent_id is set automatically from your current task context.
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
    fn test_generate_overmind_prompt_fan_out_heuristic() {
        let wf = WorkflowTemplate::default_code_workflow();
        let prompt = generate_overmind_prompt(&wf);

        // Dynamic path should contain fan-out heuristic for read-only root phases
        assert!(prompt.contains("Fan-out heuristic"));
        assert!(prompt.contains("memory_store"));
        // Example should use ids array for root read-only phases
        assert!(prompt.contains("ids: ["));
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
            ..WorkflowTemplate::default()
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
            ..WorkflowTemplate::default()
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
