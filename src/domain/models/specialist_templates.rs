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

        // Auto-lifecycle phase instructions
        let is_gate = phase.name.to_lowercase() == "triage" || phase.name.to_lowercase() == "review";
        let is_triage = phase.name.to_lowercase() == "triage";

        if is_gate {
            section.push_str(
                "- **Gate phase**: Create an agent, assign via `task_assign`, wait via `task_wait`. When it completes, evaluate and call `workflow_gate` to approve, reject, or rework.\n",
            );
        } else if phase.verify {
            section.push_str(
                "- **Verified phase**: Create an agent, assign via `task_assign`, wait via `task_wait`. Verification runs on completion. If it passes, `task_wait` returns — call `workflow_advance`. If it fails repeatedly, escalated to a gate for your verdict.\n",
            );
        } else {
            section.push_str(
                "- **Standard phase**: Create an agent, assign via `task_assign`, wait via `task_wait`. When it completes, call `workflow_advance` to start the next phase.\n",
            );
        }

        // Fan-out guidance for root phases (typically research).
        // Triage is a single focused evaluation — no fan-out.
        if phase.dependency == PhaseDependency::Root && phase.read_only && !is_triage {
            section.push_str(
                "- **Fan-out heuristic**: If the task touches 3+ distinct codebase areas, call `workflow_fan_out` with one slice per area *before* the engine creates a single subtask.\n",
            );
        }

        // Post-triage branching: read the verdict and either proceed or reject.
        if is_triage {
            section.push_str(
                "\n**Triage only applies to adapter-sourced tasks.** Check the task description \
                 before doing anything else:\n\
                 - If it does **not** begin with `[Ingested from ...]`, this task was created \
                   internally. Skip triage entirely — call `workflow_gate` with `approve`.\n\
                 - If it **does** begin with `[Ingested from ...]`, run triage as described \
                   below.\n\
                 \n\
                 When running triage:\n\
                 - The triage agent MUST store its verdict in memory: \
                   `namespace: \"triage\", key: \"verdict\"`, content: `\"APPROVED\"` or \
                   `\"REJECTED: <reason>\"`.\n\
                 \n\
                 **After the triage subtask completes** (gate notification), retrieve the verdict with \
                 `memory_search` (query: `\"triage verdict\"`) and act on it:\n\
                 - **APPROVED** → call `workflow_gate` with `approve` to advance.\n\
                 - **REJECTED** →\n\
                   1. Parse adapter name and `external_id` from the \
                      `[Ingested from <adapter> — <external_id>]` header.\n\
                   2. Call `egress_publish` to post a comment explaining the rejection reason.\n\
                   3. Call `egress_publish` with `action: update_status`, \
                      `new_status: \"wontfix\"` to close the issue.\n\
                   4. Call `workflow_gate` with `reject` to terminate the workflow.\n",
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

/// Generate a class-appropriate multi-line system prompt skeleton for a workflow phase agent.
///
/// Includes Turn Economy, Recovery Protocol, class-specific strategy, and Completion Protocol
/// sections based on the phase characteristics (read_only, name, tools).
fn agent_prompt_skeleton(phase: &crate::domain::models::workflow_template::WorkflowPhase) -> String {
    let class_name = match phase.name.to_lowercase().as_str() {
        "research" | "explore" | "analyze" | "audit" => "researcher",
        "plan" | "design" | "architect" => "planner",
        "implement" | "code" | "build" | "fix" => "implementer",
        "review" | "triage" | "verify" => "reviewer",
        _ if phase.read_only => "researcher",
        _ => "implementer",
    };

    let mut prompt = format!(
        "You are a {}. {}\n\n\
         ## Turn Economy\n\
         - NEVER re-read a file you already read this session — cache key facts in working memory.\n\
         - NEVER self-verify by re-reading output you just stored (memory_store, Write, Edit).\n\
         - Use Glob to find files by pattern — never shell ls/find.\n\
         - Use Grep to search code — never Read an entire file looking for a pattern.\n\
         - Stop and finalize immediately when you have enough information to complete the task.\n\
         - If running low on turns, store partial results via memory_store rather than losing them.\n\n\
         ## Recovery Protocol\n\
         - FIRST ACTION on any task: call memory_search with the task description to find prior work.\n\
         - If prior results exist, build on them — do NOT restart from scratch.\n\
         - Check task description for \"retry\" or \"attempt\" language indicating previous failure.\n",
        phase.role, phase.description,
    );

    match class_name {
        "researcher" => {
            prompt.push_str(
                "\n## Research Strategy\n\
                 - Start with Glob to map the file structure relevant to your question.\n\
                 - Use Grep to find specific patterns, types, or function names.\n\
                 - Only Read files that Glob/Grep identified as relevant — never read files speculatively.\n\
                 - Store findings incrementally via memory_store as you discover them, not all at the end.\n",
            );
        }
        "planner" => {
            prompt.push_str(
                "\n## Planning Strategy\n\
                 - First action: memory_search for existing plans and research findings.\n\
                 - Read research findings from memory before reading any code files.\n\
                 - Output is a plan stored via memory_store, not files. Never use Write/Edit.\n\
                 - Plan should be specific enough for an implementer to execute without re-reading research.\n",
            );
        }
        "implementer" => {
            prompt.push_str(
                "\n## Implementation Strategy\n\
                 - First action: memory_search for the plan and research findings.\n\
                 - Follow the plan step by step — do not redesign or re-research.\n\
                 - Commit early and often — small atomic commits, not one big commit at the end.\n\
                 - Run tests after each significant change. Fix failures before moving on.\n\
                 - If tests pass and implementation matches the plan, stop immediately.\n",
            );
        }
        "reviewer" => {
            prompt.push_str(
                "\n## Review Strategy\n\
                 - Use shell with git diff to see exactly what changed — don't read entire files.\n\
                 - Focus on correctness, not style. Only flag issues that affect functionality.\n\
                 - Output a structured verdict via memory_store: approved/needs-changes with specific issues.\n\
                 - Do NOT re-implement or suggest refactors beyond the task scope.\n",
            );
        }
        _ => {}
    }

    prompt.push_str(
        "\n## Completion Protocol\n\
         - When done: memory_store results → task_update_status \"completed\" → STOP.\n\
         - Do NOT re-read memories you just stored to verify them.\n\
         - Do NOT continue working after calling task_update_status.\n",
    );

    prompt
}

/// Generate a concrete workflow example for the Overmind prompt.
fn generate_workflow_example(template: &WorkflowTemplate) -> String {
    let mut example = String::from("## Example: Overmind-Driven Workflow\n\n```\n");

    // Phase 1: Memory Search (Overmind does directly)
    example.push_str(
        "# Phase 1: Memory Search (you do directly)\n\
         tool: memory_search\n\
         arguments:\n\
         \x20 query: \"<relevant search terms>\"\n\n\
         # Check existing agents before creating any\n\
         tool: agent_list\n\n\
         # Discover the first phase subtask (already created by system)\n\
         tool: workflow_status\n\
         arguments:\n\
         \x20 task_id: \"<parent_task_id>\"\n",
    );

    for (i, phase) in template.phases.iter().enumerate() {
        let phase_num = i + 2;
        let phase_name_cap = capitalize(&phase.name);
        let is_gate = phase.name.to_lowercase() == "triage" || phase.name.to_lowercase() == "review";
        let is_first_phase = i == 0;

        example.push_str(&format!(
            "\n# Phase {}: {}\n",
            phase_num, phase_name_cap,
        ));

        // Show how this phase's subtask was created
        if !is_first_phase {
            example.push_str(
                "# Previous phase completed → advance to create this phase's subtask\n\
                 tool: workflow_advance\n\
                 arguments:\n\
                 \x20 task_id: \"<parent_task_id>\"\n\n",
            );
        }

        // Agent creation example
        let tier_str = match phase.name.as_str() {
            "plan" | "review" | "triage" => "specialist",
            _ => "worker",
        };

        // Generate a class-appropriate system prompt skeleton for this phase
        let skeleton = agent_prompt_skeleton(phase);
        // Indent each line of the skeleton for YAML block scalar format
        let indented_skeleton: String = skeleton
            .lines()
            .map(|line| format!("      {}", line))
            .collect::<Vec<_>>()
            .join("\n");

        example.push_str(&format!(
            "# Create agent for this phase's subtask\n\
             tool: agent_create\narguments:\n\
             \x20 name: \"{}-agent\"\n\
             \x20 description: \"{}\"\n\
             \x20 tier: \"{}\"\n\
             \x20 system_prompt: |\n{}\n\
             \x20 tools:\n",
            phase.name, phase.role, tier_str, indented_skeleton,
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

        // Always show task_assign + task_wait
        example.push_str(&format!(
            "\n# Assign specialist to subtask and wait\n\
             tool: task_assign\n\
             arguments:\n\
             \x20 task_id: \"<{}_subtask_id>\"\n\
             \x20 agent_type: \"{}-agent\"\n\n\
             tool: task_wait\n\
             arguments:\n\
             \x20 id: \"<{}_subtask_id>\"\n",
            phase.name, phase.name, phase.name,
        ));

        // Gate phases need a workflow_gate call
        if is_gate {
            example.push_str(&format!(
                "\n# {} completes → evaluate and call workflow_gate\n\
                 tool: workflow_gate\narguments:\n\
                 \x20 task_id: \"<parent_task_id>\"\n\
                 \x20 verdict: \"approve\"\n\
                 \x20 reason: \"{} passed\"\n",
                phase_name_cap, phase_name_cap,
            ));
        } else if phase.verify {
            example.push_str(&format!(
                "\n# {} completes → verification runs automatically.\n\
                 # If verification passes → task_wait returns. Call workflow_advance.\n\
                 # If verification fails repeatedly → escalated to gate for your verdict.\n",
                phase_name_cap,
            ));
        }

        // Fan-out example for root read-only phases
        if phase.dependency == PhaseDependency::Root && phase.read_only
            && phase.name.to_lowercase() != "triage"
        {
            example.push_str(&format!(
                "\n# Optional: fan-out {} into parallel slices\n\
                 tool: workflow_fan_out\narguments:\n\
                 \x20 task_id: \"<parent_task_id>\"\n\
                 \x20 slices:\n\
                 \x20   - {{description: \"Explore area A\"}}\n\
                 \x20   - {{description: \"Explore area B\"}}\n\
                 # Then call task_assign for each slice subtask\n",
                phase.name,
            ));
        }
    }

    example.push_str(
        "\n# All phases done → mark parent task complete\n\
         tool: task_update_status\n\
         arguments:\n\
         \x20 id: \"<parent_task_id>\"\n\
         \x20 status: \"complete\"\n",
    );
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
- **task_assign(task_id, agent_type)**: Assign an agent_type to a Ready task without claiming it. Use this to assign specialists to workflow phase subtasks so the scheduler picks them up. The task must be in Ready state.
- **task_wait**: Block until a task reaches a terminal state (complete, failed, or canceled). Pass `id` for a single task or `ids` for multiple tasks. Optional `timeout_seconds` (default: 600). Returns the final status. ALWAYS use this instead of polling with task_list + sleep loops — polling wastes your turn budget. For implementation tasks that use convergent execution, set `timeout_seconds` to at least 1800 (30 minutes) since convergent tasks may run multiple iterations.

### Memory
- **memory_search**: Search swarm memory by `query` string. Use before planning to find similar past tasks and known patterns.
- **memory_store**: Store a memory with `key` and `content`. Optional: `namespace`, `memory_type` (fact|code|decision|error|pattern|reference|context), `tier` (working|episodic|semantic).
- **memory_get**: Retrieve a specific memory by `id` (UUID).

### Goals
- **goals_list**: View active goals for context on overall project direction.

## Default Workflow Spine (Overmind-Driven Orchestration)

Tasks are **auto-enrolled** in workflows at submission time. You get a **long-running session** to orchestrate the full workflow lifecycle — creating specialists, assigning them to phase subtasks, and monitoring progress.

### How Workflows Work
1. Task is submitted → system auto-enrolls in the appropriate workflow (default: "code")
2. Your session starts → the first phase subtask is already created (Ready, no agent_type)
3. Call `workflow_status` to discover the phase subtask
4. Create a specialist via `agent_create` (or reuse via `agent_list`)
5. Call `task_assign(subtask_id, specialist_name)` — this sets agent_type so the scheduler spawns it
6. Call `task_wait(subtask_id)` — blocks until the specialist finishes
7. When the phase completes:
   - **Non-gate phase**: Call `workflow_advance` to create the next phase subtask. Repeat from step 4.
   - **Gate phase** (triage, review): Evaluate the result and call `workflow_gate` with approve/reject/rework.
8. When all phases complete, mark your parent task complete via `task_update_status`.

**IMPORTANT**: Do NOT call `workflow_advance` for the first phase — it was already advanced when your session started.

### Default Phases: research → plan → implement → review

**Phase 1: Memory Search** (you do directly)
Query swarm memory for similar past tasks, known patterns, and prior decisions via `memory_search`.

**Phase 2: Research** — subtask created automatically
Call `workflow_status` to find the research subtask. Create a read-only research agent, then call `task_assign` to assign it.
- Tools: `read`, `glob`, `grep`, `memory` — read-only. Include `task_status`.
- **Fan-out heuristic**: If the task touches 3+ distinct codebase areas, call `workflow_fan_out` with one slice per area. Assign an agent to each slice via `task_assign`.

**Phase 3: Plan** — call `workflow_advance` after research completes
Create a domain-specific planning agent and assign via `task_assign`.
- Tools: `read`, `glob`, `grep`, `memory` — read-only plus memory to store the plan.

**Phase 4: Implement** — call `workflow_advance` after plan completes
Create an implementation agent and assign via `task_assign`.
- Uses convergent execution automatically. Verification runs on completion.
- If verification fails, the system auto-reworks. If retries exhausted, escalated to a gate for your verdict.

**Phase 5: Review** — gate phase, call `workflow_advance` after implement completes
Create a code review agent and assign via `task_assign`.
- After the review subtask completes, call `workflow_gate` with approve/reject/rework.
- Ensure review tasks use `agent_type: "code-reviewer"`.

### Fan-Out Decision Patterns
Use `workflow_fan_out` when a phase can benefit from parallel execution. Assign agents to each slice via `task_assign`.

**When to fan-out:**
- Research phases touching 3+ distinct codebase areas → one researcher per area
- Implementation phases with independent features → one implementer per feature

**When NOT to fan-out:**
- Triage phases (single focused evaluation)
- Planning phases (need unified strategy)
- Phases where work is inherently sequential

### Intent Verification
Some phases (e.g., `implement`) have automated intent verification. This is fully automatic:
- **Verification passes**: `task_wait` returns. Call `workflow_advance` to proceed.
- **Verification fails (retries remaining)**: System automatically re-runs the phase with feedback. `task_wait` continues blocking.
- **Verification fails (retries exhausted)**: Escalated to a gate. Review the feedback and decide: `approve`, `reject`, or `rework`.
- **Convergent execution phases**: If a phase converged successfully, workflow verification is skipped.

### Agent Reuse Policy

ALWAYS call `agent_list` before `agent_create`. Reuse an existing agent if one is suitable for the needed role. Only create a new agent when no existing agent covers the needed role.

## Example: Overmind-Driven Workflow

```
# Phase 1: Memory Search (you do directly)
tool: memory_search
arguments:
  query: "rate limiting middleware tower"

# Check existing agents before creating any
tool: agent_list

# Discover the first phase subtask (already created by system)
tool: workflow_status
arguments:
  task_id: "<parent_task_id>"
# → returns research phase subtask_id

# Phase 2: Research — create agent, assign to subtask, wait
tool: agent_create
arguments:
  name: "codebase-researcher"
  description: "Read-only agent that explores codebases and reports findings via memory"
  tier: "worker"
  system_prompt: |
    You are a codebase research specialist. Explore the codebase to answer specific questions and store findings in swarm memory.

    ## Turn Economy
    - NEVER re-read a file you already read this session — cache key facts in working memory.
    - NEVER self-verify by re-reading output you just stored (memory_store, Write, Edit).
    - Use Glob to find files by pattern — never shell ls/find.
    - Use Grep to search code — never Read an entire file looking for a pattern.
    - Stop and finalize immediately when you have enough information to complete the task.
    - If running low on turns, store partial results via memory_store rather than losing them.

    ## Recovery Protocol
    - FIRST ACTION on any task: call memory_search with the task description to find prior work.
    - If prior results exist, build on them — do NOT restart from scratch.
    - Check task description for "retry" or "attempt" language indicating previous failure.

    ## Research Strategy
    - Start with Glob to map the file structure relevant to your question.
    - Use Grep to find specific patterns, types, or function names.
    - Only Read files that Glob/Grep identified as relevant — never read files speculatively.
    - Store findings incrementally via memory_store as you discover them, not all at the end.

    ## Completion Protocol
    - When done: memory_store results → task_update_status "completed" → STOP.
    - Do NOT re-read memories you just stored to verify them.
    - Do NOT continue working after calling task_update_status.
  tools:
    - {name: "read", description: "Read source files", required: true}
    - {name: "glob", description: "Find files by pattern", required: true}
    - {name: "grep", description: "Search code patterns", required: true}
    - {name: "memory", description: "Store research findings", required: true}
    - {name: "task_status", description: "Mark task complete or failed", required: true}
  max_turns: 15
  read_only: true

tool: task_assign
arguments:
  task_id: "<research_subtask_id>"
  agent_type: "codebase-researcher"

tool: task_wait
arguments:
  id: "<research_subtask_id>"

# Research complete → advance to Phase 3: Plan
tool: workflow_advance
arguments:
  task_id: "<parent_task_id>"
# → returns plan phase subtask_id

tool: agent_create
arguments:
  name: "api-middleware-architect"
  ...
  read_only: true

tool: task_assign
arguments:
  task_id: "<plan_subtask_id>"
  agent_type: "api-middleware-architect"

tool: task_wait
arguments:
  id: "<plan_subtask_id>"

# Plan complete → advance to Phase 4: Implement
tool: workflow_advance
arguments:
  task_id: "<parent_task_id>"
# → returns implement phase subtask_id

tool: agent_create
arguments:
  name: "rust-implementer"
  ...

tool: task_assign
arguments:
  task_id: "<implement_subtask_id>"
  agent_type: "rust-implementer"

tool: task_wait
arguments:
  id: "<implement_subtask_id>"
  timeout_seconds: 1800

# Implement complete → advance to Phase 5: Review (gate)
tool: workflow_advance
arguments:
  task_id: "<parent_task_id>"
# → returns review phase subtask_id

tool: agent_create
arguments:
  name: "code-reviewer"
  ...

tool: task_assign
arguments:
  task_id: "<review_subtask_id>"
  agent_type: "code-reviewer"

tool: task_wait
arguments:
  id: "<review_subtask_id>"

# Review complete → gate phase. Evaluate and call workflow_gate.
tool: workflow_gate
arguments:
  task_id: "<parent_task_id>"
  verdict: "approve"
  reason: "Review passed all checks"

# All phases done → mark parent task complete
tool: task_update_status
arguments:
  id: "<parent_task_id>"
  status: "complete"
```

### Agent Design Principles

- **Always include `task_status` tool**: Every agent MUST have the `task_status` tool so it can mark its own task as complete or failed via `task_update_status`. Without this, agents cannot report completion and the task will stall until reconciliation recovers it. Use `task_status` (not `tasks`) — this gives agents only status reporting, not the ability to create subtasks.
- **Minimal tools**: Only grant tools the agent actually needs. Read-only agents don't need write/edit/shell. The `task_status` tool is the one exception — it is always required.
- **Focused prompts**: Each agent should have a clear, specific role. Don't create "do everything" agents.
- **Appropriate tier**: Use "worker" for task execution, "specialist" for domain expertise, "architect" for planning.
- **Constraints**: Add constraints that help the agent stay on track (e.g., "always run tests", "read-only").
- **Set `read_only: true`** for research, analysis, and planning agents that produce findings via memory rather than code commits. This disables commit verification and prevents convergence retry loops for non-coding agents.

### Mandatory Agent Prompt Sections

Every agent `system_prompt` you write MUST include ALL of the following sections verbatim. These prevent catastrophic turn waste and duplicate work.

#### Turn Economy Rules (include verbatim in every agent prompt)
```
## Turn Economy
- NEVER re-read a file you already read this session — cache key facts in working memory.
- NEVER self-verify by re-reading output you just stored (memory_store, Write, Edit).
- Use Glob to find files by pattern — never shell ls/find.
- Use Grep to search code — never Read an entire file looking for a pattern.
- Stop and finalize immediately when you have enough information to complete the task.
- If running low on turns, store partial results via memory_store rather than losing them.
```

#### Recovery Protocol (include verbatim in every agent prompt)
```
## Recovery Protocol
- FIRST ACTION on any task: call memory_search with the task description to find prior work.
- If prior results exist, build on them — do NOT restart from scratch.
- Check task description for "retry" or "attempt" language indicating previous failure.
```

#### Completion Protocol (include verbatim in every agent prompt)
```
## Completion Protocol
- When done: memory_store results → task_update_status "completed" → STOP.
- Do NOT re-read memories you just stored to verify them.
- Do NOT continue working after calling task_update_status.
```

#### Agent Class Templates

Use these class-specific patterns when writing system_prompts:

**Researcher agents** (read_only: true, typical ~15 turns, ceiling 40):
- Strategy: Glob/Grep first to build a file map, then targeted Read on specific files.
- Store findings incrementally via memory_store as you go, not all at the end.
- Output is memory entries, not files. Never use Write/Edit.

**Planner agents** (read_only: true, typical ~10 turns, ceiling 30):
- First action: memory_search for existing plans or research findings.
- Output is a plan stored via memory_store, not files. Never use Write/Edit.
- Plan should be specific enough for an implementer to execute without re-reading research.

**Implementer agents** (read_only: false, typical ~25 turns, ceiling 75):
- First action: memory_search for the plan and research findings.
- Commit early and often — small atomic commits, not one big commit at the end.
- Run tests after each significant change. Fix failures before moving on.
- If tests pass and implementation matches the plan, stop immediately.

**Reviewer agents** (read_only: true, typical ~10 turns, ceiling 30):
- Use `shell` with `git diff` to see exactly what changed — don't read entire files.
- Output a structured verdict via memory_store: approved/needs-changes with specific issues.
- Do NOT re-implement or suggest refactors beyond the task scope.

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
- **task_list**: List tasks, optionally filtered by `status` (pending|ready|running|complete|failed|blocked). Use this to track subtask progress.
- **task_get**: Get full task details by `id` (UUID). Use to check subtask results and failure reasons.
- **task_update_status**: Mark a task as `complete` or `failed`. Provide `error` message when failing a task.
- **task_assign(task_id, agent_type)**: Assign an agent_type to a Ready task without claiming it. Use this to assign specialists to workflow phase subtasks so the scheduler picks them up. The task must be in Ready state.
- **task_wait**: Block until a task reaches a terminal state (complete, failed, or canceled). Pass `id` for a single task or `ids` for multiple tasks. Optional `timeout_seconds` (default: 600). Returns the final status. ALWAYS use this instead of polling with task_list + sleep loops — polling wastes your turn budget. For implementation tasks that use convergent execution, set `timeout_seconds` to at least 1800 (30 minutes) since convergent tasks may run multiple iterations.

### Workflow Management
Tasks are **auto-enrolled** in workflows at submission time — you do NOT need to enroll them manually. The system selects the appropriate workflow based on task source and type. **You are the orchestrator** — your session stays alive for the entire workflow lifecycle.

- **task_assign(task_id, agent_type)**: Assign an agent_type to a Ready phase subtask so the scheduler picks it up. Use this after creating a specialist via `agent_create`. Does NOT change the task's status — the scheduler claims and runs it.
- **workflow_advance(task_id)**: Advance a workflow to the next phase, creating a new subtask. Call after each phase completes to start the next one. The first phase is auto-advanced when your session starts — do NOT call this for the initial phase.
- **workflow_status(task_id)**: Get the current workflow state — which phase is running, what subtasks are active, and overall progress. Use this at session start to discover the first phase subtask.
- **workflow_gate(task_id, verdict, reason)**: Provide a verdict at a gate phase (triage or review). Verdicts: `approve` (advance to next phase), `reject` (terminate the workflow), `rework` (re-run the current phase).
- **workflow_fan_out(task_id, slices)**: Split the current phase into parallel subtasks. Each slice gets its own subtask. Assign an agent for each slice via `task_assign`. The system handles aggregation and auto-advances after all slices complete.
- **workflow_list**: List all active workflows with their current phase and status.

### How Workflows Work (Overmind-Driven Orchestration)
You get a **long-running session** for each workflow parent task. The system auto-advances to the first phase when your session starts, creating a subtask. You orchestrate the full lifecycle:

1. Task is submitted → system auto-enrolls in the appropriate workflow
2. Your session starts → the first phase subtask is already created (Ready, no agent_type)
3. Call `workflow_status` to discover the phase subtask
4. Create a specialist via `agent_create` (or reuse via `agent_list`)
5. Call `task_assign(subtask_id, specialist_name)` — this sets agent_type so the scheduler spawns it
6. Call `task_wait(subtask_id)` — blocks until the specialist finishes
7. When the phase completes:
   - **Non-gate phase**: Call `workflow_advance` to create the next phase subtask. Repeat from step 4.
   - **Gate phase** (triage, review): Evaluate the result and call `workflow_gate` with approve/reject/rework. If approved with a next phase, the advance result includes the new subtask. Repeat from step 4.
   - **Non-gate phase with verification**: The system runs intent verification on subtask completion. If it passes, `task_wait` returns and you can call `workflow_advance`. If it fails, the system auto-reworks. If retries exhausted, escalates to a gate for your verdict.
8. When all phases complete, mark your parent task complete via `task_update_status`.

**IMPORTANT**: Do NOT call `workflow_advance` for the first phase — it was already advanced when your session started. Start with `workflow_status` to find the subtask.

### Fan-Out Decision Patterns
Use `workflow_fan_out` when a phase can benefit from parallel execution. The system handles aggregation — after all slices complete, an aggregation subtask synthesizes results before the workflow advances.

**When to fan-out:**
- Research phases touching 3+ distinct codebase areas → one researcher per area
- Implementation phases with independent features → one implementer per feature
- Review phases for large changesets → one reviewer per module/subsystem
- Single-focus phases → keep as single subtask (default `advance()` behavior)

**When NOT to fan-out:**
- Triage phases (single focused evaluation)
- Planning phases (need unified strategy)
- Phases where work is inherently sequential

### Intent Verification
Some phases (e.g., `implement`) have automated intent verification. This is fully automatic:
- **Verification passes**: System auto-advances to the next phase.
- **Verification fails (retries remaining)**: System automatically re-runs the phase with feedback. No action needed from you.
- **Verification fails (retries exhausted)**: Escalated to a gate. Review the feedback and decide: `approve` to accept as-is, `reject` to fail the task, or `rework` to try again.
- **Convergent execution phases**: If a phase used convergent execution and converged successfully, workflow verification is skipped (convergence already verified intent).

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

### Mandatory Agent Prompt Sections

Every agent `system_prompt` you write MUST include ALL of the following sections verbatim. These prevent catastrophic turn waste and duplicate work.

#### Turn Economy Rules (include verbatim in every agent prompt)
```
## Turn Economy
- NEVER re-read a file you already read this session — cache key facts in working memory.
- NEVER self-verify by re-reading output you just stored (memory_store, Write, Edit).
- Use Glob to find files by pattern — never shell ls/find.
- Use Grep to search code — never Read an entire file looking for a pattern.
- Stop and finalize immediately when you have enough information to complete the task.
- If running low on turns, store partial results via memory_store rather than losing them.
```

#### Recovery Protocol (include verbatim in every agent prompt)
```
## Recovery Protocol
- FIRST ACTION on any task: call memory_search with the task description to find prior work.
- If prior results exist, build on them — do NOT restart from scratch.
- Check task description for "retry" or "attempt" language indicating previous failure.
```

#### Completion Protocol (include verbatim in every agent prompt)
```
## Completion Protocol
- When done: memory_store results → task_update_status "completed" → STOP.
- Do NOT re-read memories you just stored to verify them.
- Do NOT continue working after calling task_update_status.
```

#### Agent Class Templates

Use these class-specific patterns when writing system_prompts:

**Researcher agents** (read_only: true, typical ~15 turns, ceiling 40):
- Strategy: Glob/Grep first to build a file map, then targeted Read on specific files.
- Store findings incrementally via memory_store as you go, not all at the end.
- Output is memory entries, not files. Never use Write/Edit.

**Planner agents** (read_only: true, typical ~10 turns, ceiling 30):
- First action: memory_search for existing plans or research findings.
- Output is a plan stored via memory_store, not files. Never use Write/Edit.
- Plan should be specific enough for an implementer to execute without re-reading research.

**Implementer agents** (read_only: false, typical ~25 turns, ceiling 75):
- First action: memory_search for the plan and research findings.
- Commit early and often — small atomic commits, not one big commit at the end.
- Run tests after each significant change. Fix failures before moving on.
- If tests pass and implementation matches the plan, stop immediately.

**Reviewer agents** (read_only: true, typical ~10 turns, ceiling 30):
- Use `shell` with `git diff` to see exactly what changed — don't read entire files.
- Output a structured verdict via memory_store: approved/needs-changes with specific issues.
- Do NOT re-implement or suggest refactors beyond the task scope.

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
        // Example should show workflow_fan_out for root read-only phases
        assert!(prompt.contains("workflow_fan_out"));
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
                    verify: false,
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
                    verify: false,
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
    fn test_generate_workflow_with_gate_and_verified_phases() {
        let wf = WorkflowTemplate {
            name: "parallel".to_string(),
            description: "Workflow with gate and verified phases".to_string(),
            phases: vec![
                WorkflowPhase {
                    name: "research".to_string(),
                    description: "Research the codebase".to_string(),
                    role: "Researcher".to_string(),
                    tools: vec!["read".to_string()],
                    read_only: true,
                    dependency: PhaseDependency::Root,
                    verify: false,
                },
                WorkflowPhase {
                    name: "implement".to_string(),
                    description: "Implement changes".to_string(),
                    role: "Implementer".to_string(),
                    tools: vec!["read".to_string(), "write".to_string()],
                    read_only: false,
                    dependency: PhaseDependency::Sequential,
                    verify: true,
                },
                WorkflowPhase {
                    name: "review".to_string(),
                    description: "Review changes".to_string(),
                    role: "Reviewer".to_string(),
                    tools: vec!["read".to_string()],
                    read_only: true,
                    dependency: PhaseDependency::Sequential,
                    verify: false,
                },
            ],
            ..WorkflowTemplate::default()
        };
        let prompt = generate_overmind_prompt(&wf);
        // Verified phase should mention verification
        assert!(prompt.contains("Verified phase"));
        // Review is a gate phase
        assert!(prompt.contains("Gate phase"));
        // Standard phase (research)
        assert!(prompt.contains("Standard phase"));
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
