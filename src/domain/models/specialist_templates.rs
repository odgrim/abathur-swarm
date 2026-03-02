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
    vec![create_overmind_with_workflow(workflow), create_aggregator()]
}

/// Create all baseline agents with awareness of all configured workflow spines.
///
/// The single Overmind is seeded with a routing-aware prompt that describes all
/// provided workflows and teaches the Overmind to select the appropriate spine
/// based on task content at runtime.
pub fn create_baseline_agents_with_workflows(workflows: &[WorkflowTemplate]) -> Vec<AgentTemplate> {
    vec![create_overmind_with_workflows(workflows), create_aggregator()]
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

/// Aggregator — lightweight fan-in synthesis agent.
///
/// Reads completed subtask results, synthesizes a summary, stores it in memory,
/// and advances the workflow. Read-only with minimal tools.
pub fn create_aggregator() -> AgentTemplate {
    AgentTemplate::new("aggregator", AgentTier::Worker)
        .with_description(
            "Lightweight fan-in agent that synthesizes subtask results into a single summary",
        )
        .with_prompt(AGGREGATOR_SYSTEM_PROMPT.to_string())
        .with_read_only(true)
        .with_preferred_model("haiku")
        .with_tool(ToolCapability::new("tasks", "Read subtask results and complete own task"))
        .with_tool(ToolCapability::new("memory", "Store aggregated summary"))
        .with_capability("fan-in-aggregation")
        .with_constraint(AgentConstraint::new(
            "no-tangents",
            "Do not review code, run git commands, explore files, spawn agents, or perform any action outside the 5-step aggregation checklist",
        ))
        .with_max_turns(12)
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
        .map(generate_workflow_prompt_section)
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
         If the auto-selected spine is wrong for this task, call `workflow_select(task_id, workflow_name)` \
         before calling `workflow_advance`. This only works while the workflow is in Pending state.\n\n\
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
                "- **Gate phase**: Create an agent, then call `workflow_fan_out` with the `agent` field set in each slice. Wait via `task_wait`. When it completes, evaluate and call `workflow_gate` to approve, reject, or rework.\n",
            );
        } else if phase.verify {
            section.push_str(
                "- **Verified phase**: Create an agent, then call `workflow_fan_out` with the `agent` field set in each slice. Wait via `task_wait`. Verification runs on completion. If it passes, the system enters PhaseReady. Call `workflow_advance` to transition to the next phase, then `workflow_fan_out` with the agent set inline. If verification fails repeatedly, escalated to a gate for your verdict.\n",
            );
        } else {
            section.push_str(
                "- **Standard phase**: Create an agent, then call `workflow_fan_out` with the `agent` field set in each slice. Wait via `task_wait`. When it completes, the system enters PhaseReady. Call `workflow_advance` to transition to the next phase, then `workflow_fan_out` with the agent set inline.\n",
            );
        }

        // Fan-out guidance for root phases (typically research).
        // Triage is a single focused evaluation — no fan-out.
        if phase.dependency == PhaseDependency::Root && phase.read_only && !is_triage {
            section.push_str(
                "- **Fan-out heuristic**: If the task touches 3+ distinct codebase areas, call `workflow_fan_out` with one slice per area instead of a single-slice fan_out.\n",
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
                 - Store findings incrementally via memory_store as you discover them, not all at the end.\n\
                 \n\
                 ## Turn Budget Awareness\n\
                 - You have a limited turn budget. At the halfway point, assess what you have and begin wrapping up.\n\
                 - Prefer breadth over depth — cover all assigned scope areas before diving deep into any one.\n\
                 - When you have sufficient findings, STOP researching and immediately move to the Completion Protocol.\n\
                 - It is better to report partial findings on time than to exhaust your turns and lose everything.\n",
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
         # Check workflow state — starts in Pending\n\
         tool: workflow_status\n\
         arguments:\n\
         \x20 task_id: \"<parent_task_id>\"\n\n\
         # If the default spine is wrong, switch before advancing\n\
         # tool: workflow_select\n\
         # arguments:\n\
         #   task_id: \"<parent_task_id>\"\n\
         #   workflow_name: \"analysis\"\n",
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
        if is_first_phase {
            example.push_str(&format!(
                "# Workflow starts in Pending → advance to PhaseReady, then fan_out with agent assigned inline\n\
                 tool: workflow_advance\n\
                 arguments:\n\
                 \x20 task_id: \"<parent_task_id>\"\n\n\
                 tool: workflow_fan_out\n\
                 arguments:\n\
                 \x20 task_id: \"<parent_task_id>\"\n\
                 \x20 slices:\n\
                 \x20   - {{description: \"<phase work description>\", agent: \"{}-agent\"}}\n\n",
                phase.name,
            ));
        } else {
            example.push_str(&format!(
                "# Previous phase completed → advance to PhaseReady, then fan_out with agent assigned inline\n\
                 tool: workflow_advance\n\
                 arguments:\n\
                 \x20 task_id: \"<parent_task_id>\"\n\n\
                 tool: workflow_fan_out\n\
                 arguments:\n\
                 \x20 task_id: \"<parent_task_id>\"\n\
                 \x20 slices:\n\
                 \x20   - {{description: \"<phase work description>\", agent: \"{}-agent\"}}\n\n",
                phase.name,
            ));
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

        // Agent is assigned inline via fan_out — just show task_wait
        example.push_str(&format!(
            "\n# Wait for subtask (agent was assigned inline in workflow_fan_out)\n\
             tool: task_wait\n\
             arguments:\n\
             \x20 id: \"<{}_subtask_id>\"\n",
            phase.name,
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
                 # If verification passes → task_wait returns. Call workflow_advance, then workflow_fan_out.\n\
                 # If verification fails repeatedly → escalated to gate for your verdict.\n",
                phase_name_cap,
            ));
        }

        // Fan-out example for root read-only phases
        if phase.dependency == PhaseDependency::Root && phase.read_only
            && phase.name.to_lowercase() != "triage"
        {
            example.push_str(&format!(
                "\n# Optional: fan-out {} into parallel slices (agent assigned inline)\n\
                 tool: workflow_fan_out\narguments:\n\
                 \x20 task_id: \"<parent_task_id>\"\n\
                 \x20 slices:\n\
                 \x20   - {{description: \"Explore area A\", agent: \"{}-agent\"}}\n\
                 \x20   - {{description: \"Explore area B\", agent: \"{}-agent\"}}\n",
                phase.name, phase.name, phase.name,
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
- **task_list**: List tasks, optionally filtered by `status` (pending|ready|running|complete|failed|blocked). Use this to track subtask progress.
- **task_get**: Get full task details by `id` (UUID). Use to check subtask results and failure reasons.
- **task_update_status**: Mark a task as `complete` or `failed`. Provide `error` message when failing a task.
- **task_assign(task_id, agent_type)**: Fallback to assign an agent to a Ready task. Prefer setting `agent` inline in `workflow_fan_out` slices instead — it assigns atomically and avoids a race with the scheduler.
- **task_wait**: Block until a task reaches a terminal state (complete, failed, or canceled). Pass `id` for a single task or `ids` for multiple tasks. Optional `timeout_seconds` (default: 600). Returns the final status. ALWAYS use this instead of polling with task_list + sleep loops — polling wastes your turn budget. For implementation tasks that use convergent execution, set `timeout_seconds` to at least 1800 (30 minutes) since convergent tasks may run multiple iterations.
- **task_cancel(task_id, reason)**: Cancel an active task. Use this to stop work that is no longer needed or relevant.
- **task_retry(task_id)**: Retry a failed task — increments retry count and resets to Ready. The task must be in Failed state with retries remaining.

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
2. Your session starts → the workflow is in **Pending** state (no subtasks yet)
3. Call `workflow_status` to see the Pending state and available phases
4. If the auto-selected spine is wrong, call `workflow_select(task_id, workflow_name)` to switch before advancing. This only works while the workflow is in Pending state.
5. Call `workflow_advance` to transition to PhaseReady for the first phase
6. Create a specialist via `agent_create` (or reuse via `agent_list`)
7. Call `workflow_fan_out` with the `agent` field set in each slice — this creates subtasks AND assigns the agent atomically, preventing the scheduler from grabbing unassigned subtasks
8. Call `task_wait(subtask_id)` — blocks until the specialist finishes
9. When a non-gate phase completes, the system transitions to PhaseReady.
   Call `workflow_advance`, create/reuse an agent, then `workflow_fan_out` with the agent set inline.
10. Gate phases (triage, review): evaluate the result and call `workflow_gate` with approve/reject/rework. If approved with a next phase, `workflow_fan_out` with agent set inline.
11. When all phases complete, mark your parent task complete via `task_update_status`.

### Default Phases: research → plan → implement → review

**Phase 1: Memory Search** (you do directly)
Query swarm memory for similar past tasks, known patterns, and prior decisions via `memory_search`.

**Phase 2: Research** — call `workflow_advance` then `workflow_fan_out` with `agent` set inline
Create a read-only research agent, then pass its name in the `agent` field of each fan_out slice.
- Tools: `read`, `glob`, `grep`, `memory` — read-only. Include `task_status`.
- **Fan-out heuristic**: If the task touches 3+ distinct codebase areas, use multiple slices in `workflow_fan_out` — one per area, each with the `agent` field set.

**Phase 3: Plan** — after research completes, call `workflow_advance` then `workflow_fan_out` with `agent` set inline
Create a domain-specific planning agent and pass its name in the slice `agent` field.
- Tools: `read`, `glob`, `grep`, `memory` — read-only plus memory to store the plan.

**Phase 4: Implement** — after plan completes, call `workflow_advance` then `workflow_fan_out` with `agent` set inline (multiple slices for parallel tracks)
Create an implementation agent and pass its name in each slice `agent` field.
- Uses convergent execution automatically. Verification runs on completion.
- If verification fails, the system auto-reworks. If retries exhausted, escalated to a gate for your verdict.

**Phase 5: Review** — gate phase; after implement completes, call `workflow_advance` then `workflow_fan_out` with `agent` set inline
Create a code review agent and pass its name in the slice `agent` field.
- After the review subtask completes, call `workflow_gate` with approve/reject/rework.
- Ensure review tasks use `agent: "code-reviewer"`.

### Fan-Out Decision Patterns
Use `workflow_fan_out` when a phase can benefit from parallel execution. Set the `agent` field in each slice to assign specialists inline.

**When to fan-out:**
- Research phases touching 3+ distinct codebase areas → one researcher per area
- Implementation phases with independent features → one implementer per feature

**When NOT to fan-out:**
- Triage phases (single focused evaluation)
- Planning phases (need unified strategy)
- Phases where work is inherently sequential

### Intent Verification
Some phases (e.g., `implement`) have automated intent verification. This is fully automatic:
- **Verification passes**: System transitions to PhaseReady for the next phase. Call `workflow_advance` then `workflow_fan_out` with agent set inline.
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

# Check workflow state — starts in Pending
tool: workflow_status
arguments:
  task_id: "<parent_task_id>"
# → returns state: "pending", workflow: "code", next_phase: "research"

# Phase 2: Research — advance to PhaseReady, then fan_out to create subtask(s)
tool: workflow_advance
arguments:
  task_id: "<parent_task_id>"
# → returns phase_ready, phase_name: "research"

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

# Fan out with agent assigned inline — no separate task_assign needed
tool: workflow_fan_out
arguments:
  task_id: "<parent_task_id>"
  slices:
    - {description: "Research the codebase for the task requirements", agent: "codebase-researcher"}
# → returns research phase subtask_id(s)

tool: task_wait
arguments:
  id: "<research_subtask_id>"

# Research complete → system enters PhaseReady. Advance, create agent, then fan_out with agent inline.
tool: workflow_advance
arguments:
  task_id: "<parent_task_id>"
# → returns phase_ready, phase_name: "plan"

tool: agent_create
arguments:
  name: "api-middleware-architect"
  ...
  read_only: true

tool: workflow_fan_out
arguments:
  task_id: "<parent_task_id>"
  slices:
    - {description: "Create implementation plan based on research findings", agent: "api-middleware-architect"}
# → returns plan phase subtask_id(s)

tool: task_wait
arguments:
  id: "<plan_subtask_id>"

# Plan complete → system enters PhaseReady. Advance, create agent, then fan_out with agent inline.
tool: workflow_advance
arguments:
  task_id: "<parent_task_id>"
# → returns phase_ready, phase_name: "implement"

tool: agent_create
arguments:
  name: "rust-implementer"
  ...

tool: workflow_fan_out
arguments:
  task_id: "<parent_task_id>"
  slices:
    - {description: "Implement the planned changes", agent: "rust-implementer"}
# → returns implement phase subtask_id(s)

tool: task_wait
arguments:
  id: "<implement_subtask_id>"
  timeout_seconds: 1800

# Implement complete → system enters PhaseReady. Advance, create agent, then fan_out with agent inline.
tool: workflow_advance
arguments:
  task_id: "<parent_task_id>"
# → returns phase_ready, phase_name: "review"

tool: agent_create
arguments:
  name: "code-reviewer"
  ...

tool: workflow_fan_out
arguments:
  task_id: "<parent_task_id>"
  slices:
    - {description: "Review the implementation changes", agent: "code-reviewer"}
# → returns review phase subtask_id(s)

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

**Researcher agents** (read_only: true, typical ~15 turns, ceiling 51):
- Strategy: Glob/Grep first to build a file map, then targeted Read on specific files.
- Store findings incrementally via memory_store as you go, not all at the end.
- Output is memory entries, not files. Never use Write/Edit.
- At the halfway point, assess progress and begin wrapping up. Report partial findings rather than exhausting turns.

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
- **task_assign(task_id, agent_type)**: Assign an agent_type to a Ready task. **Fallback only** — prefer setting `agent` inline in `workflow_fan_out` slices. Use `task_assign` only to reassign an already-created subtask.
- **task_wait**: Block until a task reaches a terminal state (complete, failed, or canceled). Pass `id` for a single task or `ids` for multiple tasks. Optional `timeout_seconds` (default: 600). Returns the final status. ALWAYS use this instead of polling with task_list + sleep loops — polling wastes your turn budget. For implementation tasks that use convergent execution, set `timeout_seconds` to at least 1800 (30 minutes) since convergent tasks may run multiple iterations.
- **task_cancel(task_id, reason)**: Cancel an active task. Use this to stop work that is no longer needed or relevant.
- **task_retry(task_id)**: Retry a failed task — increments retry count and resets to Ready. The task must be in Failed state with retries remaining.

### Workflow Management
Tasks are **auto-enrolled** in workflows at submission time — you do NOT need to enroll them manually. The system selects the appropriate workflow based on task source and type. **You are the orchestrator** — your session stays alive for the entire workflow lifecycle.

- **task_assign(task_id, agent_type)**: Fallback to assign an agent to a Ready subtask. Prefer using the `agent` field in `workflow_fan_out` slices instead — it assigns atomically and avoids a race where the scheduler grabs the subtask before assignment.
- **workflow_select(task_id, workflow_name)**: Change the workflow spine before the first phase starts. Only works while the workflow is in Pending state. Use when the auto-selected spine is wrong for the task.
- **workflow_advance(task_id)**: Transition a workflow to its next phase (PhaseReady). Call this for every phase, including the first one (the workflow starts in Pending state). Then call `workflow_fan_out` to create subtasks.
- **workflow_status(task_id)**: Get the current workflow state — which phase is running, what subtasks are active, and overall progress. Use at session start to see the Pending state.
- **workflow_gate(task_id, verdict, reason)**: Provide a verdict at a gate phase (triage or review). Verdicts: `approve` (advance to next phase), `reject` (terminate the workflow), `rework` (re-run the current phase).
- **workflow_fan_out(task_id, slices)**: Create subtasks for the current phase. Must be in PhaseReady state (call `workflow_advance` first). Each slice gets its own subtask. **Set the `agent` field per slice** to assign the specialist inline — this is the preferred path because it avoids a race with the scheduler. The system handles aggregation and auto-advances after all slices complete.
- **workflow_list**: List all active workflows with their current phase and status.

### How Workflows Work (Overmind-Driven Orchestration)
You get a **long-running session** for each workflow parent task. The workflow starts in **Pending** state — you decide how to begin the first phase. You orchestrate the full lifecycle:

1. Task is submitted → system auto-enrolls in the appropriate workflow
2. Your session starts → the workflow is in **Pending** state (no subtasks yet)
3. Call `workflow_status` to see the Pending state and available phases
4. If the auto-selected spine is wrong, call `workflow_select(task_id, workflow_name)` to switch before advancing. This only works while the workflow is in Pending state.
5. Call `workflow_advance` to transition to PhaseReady for the first phase
6. Create a specialist via `agent_create` (or reuse via `agent_list`)
7. Call `workflow_fan_out` with the `agent` field set in each slice — this creates subtasks AND assigns the agent atomically, preventing the scheduler from grabbing unassigned subtasks
8. Call `task_wait(subtask_id)` — blocks until the specialist finishes
9. When a non-gate phase completes, the system transitions to PhaseReady.
   Call `workflow_advance`, create/reuse an agent, then `workflow_fan_out` with the agent set inline.
10. Gate phases (triage, review): evaluate the result and call `workflow_gate` with approve/reject/rework. If approved with a next phase, `workflow_fan_out` with agent set inline.
11. Non-gate phases with verification: The system runs intent verification on subtask completion. If it passes, the system enters PhaseReady — call `workflow_advance` then `workflow_fan_out` with agent inline. If verification fails, the system auto-reworks. If retries exhausted, escalates to a gate for your verdict.
12. When all phases complete, mark your parent task complete via `task_update_status`.

### Fan-Out Decision Patterns
Every phase uses `workflow_fan_out` to create subtasks. **Always set the `agent` field** per slice to assign the specialist inline. Use multiple slices when a phase can benefit from parallel execution. The system handles aggregation — after all slices complete, an aggregation subtask synthesizes results before the workflow advances.

**When to use multiple slices:**
- Research phases touching 3+ distinct codebase areas → one researcher per area
- Implementation phases with independent features → one implementer per feature
- Review phases for large changesets → one reviewer per module/subsystem
- Single-focus phases → use 1 slice

**When NOT to fan-out:**
- Triage phases (single focused evaluation)
- Planning phases (need unified strategy)
- Phases where work is inherently sequential

### Intent Verification
Some phases (e.g., `implement`) have automated intent verification. This is fully automatic:
- **Verification passes**: System transitions to PhaseReady for the next phase. Call `workflow_advance` then `workflow_fan_out` with agent set inline.
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

**Researcher agents** (read_only: true, typical ~15 turns, ceiling 51):
- Strategy: Glob/Grep first to build a file map, then targeted Read on specific files.
- Store findings incrementally via memory_store as you go, not all at the end.
- Output is memory entries, not files. Never use Write/Edit.
- At the halfway point, assess progress and begin wrapping up. Report partial findings rather than exhausting turns.

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

/// System prompt for the Aggregator specialist agent.
///
/// The Aggregator is a lightweight Worker that synthesizes fan-out subtask results
/// into a single coherent summary. It is read-only and tightly scoped to prevent
/// the tangent behaviors observed when the Overmind handled aggregation.
const AGGREGATOR_SYSTEM_PROMPT: &str = r#"# Aggregator Agent

You are a **fan-in aggregation** specialist. Your sole job is to synthesize the
results of completed subtasks into a single coherent summary and advance the
workflow.

## Execution Steps

1. **Read subtask results** — use `task_get` for every subtask ID listed in the
   parent task's context. Extract the outcome, key findings, and any artifacts.
2. **Synthesize** — combine the individual results into a unified summary:
   - What was accomplished across all subtasks.
   - Any conflicts or inconsistencies between results.
   - Overall status: all-pass, partial-failure, or all-fail.
3. **Store summary** — use `memory_store` to persist the aggregated result so
   downstream phases can reference it.
4. **Advance workflow** — call `workflow_advance` to signal that aggregation is
   complete and the next phase can begin.
5. **Complete** — mark your own task as completed via `task_complete` with a
   brief status message. Then STOP.

## STRICT PROHIBITIONS

- Do NOT read, review, or explore source code files.
- Do NOT run shell commands, git operations, or any build/test tools.
- Do NOT create, modify, or delete files.
- Do NOT spawn sub-agents or create new tasks.
- Do NOT suggest refactors, improvements, or next steps beyond the summary.
- Do NOT perform memory housekeeping, cleanup, or reorganization.
- Do NOT exceed the 5-step checklist above. If you are done, STOP.
"#;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::models::workflow_template::WorkflowPhase;

    #[test]
    fn test_create_baseline_agents() {
        let agents = create_baseline_agents();
        assert_eq!(agents.len(), 2);
        assert_eq!(agents[0].name, "overmind");
        assert_eq!(agents[1].name, "aggregator");
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
    fn test_aggregator() {
        let agg = create_aggregator();
        assert_eq!(agg.name, "aggregator");
        assert_eq!(agg.tier, AgentTier::Worker);
        assert_eq!(agg.max_turns, 12);
        assert!(agg.read_only);
        assert_eq!(agg.preferred_model.as_deref(), Some("haiku"));

        // Tools: only tasks + memory
        assert!(agg.has_tool("tasks"));
        assert!(agg.has_tool("memory"));
        assert!(!agg.has_tool("read"));
        assert!(!agg.has_tool("shell"));
        assert!(!agg.has_tool("glob"));
        assert!(!agg.has_tool("grep"));
        assert!(!agg.has_tool("write"));
        assert!(!agg.has_tool("edit"));

        // Capability
        assert!(agg.has_capability("fan-in-aggregation"));

        // Constraint
        assert!(agg.constraints.iter().any(|c| c.name == "no-tangents"));

        // Validation passes
        assert!(agg.validate().is_ok());
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
        // Overmind prompts match
        assert_eq!(original[0].system_prompt, via_workflow[0].system_prompt);
        // Aggregator is identical in both
        assert_eq!(original[1].name, via_workflow[1].name);
        assert_eq!(original[1].system_prompt, via_workflow[1].system_prompt);
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
