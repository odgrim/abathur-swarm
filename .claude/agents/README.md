# Abathur Workflow Agents

This directory contains the core agents that power the Abathur workflow philosophy.

## Workflow Philosophy

Abathur follows a structured 5-phase workflow for every task:

1. **Requirements Gathering** → Understand what needs to be built
2. **Technical Specification** → Define how it will be built
3. **Agent Provisioning** → Create or identify specialists needed
4. **Task Planning** → Break down work into executable tasks
5. **Execution** → Complete the tasks

Each phase has validation gates that must pass before proceeding to the next phase.

## Agent Architecture

Abathur uses a two-tier agent architecture:

1. **Meta-Orchestration** (1 agent) - Workflow coordination
2. **Workflow Specialists** (4 agents) - Phase-specific expertise
3. **Hyperspecialized Workers** (dynamic) - Task-specific implementations

## Directory Structure

```
agents/
├── meta/                    # Meta-orchestration tier (1 agent)
│   └── workflow-orchestrator.md
├── specialists/             # Workflow specialist tier (4 agents)
│   ├── requirements-gatherer.md
│   ├── technical-requirements-specialist.md
│   ├── task-planner.md
│   └── agent-creator.md
└── workers/                 # Hyperspecialized worker tier (dynamically created)
    └── custom/              # Agent-creator generates workers here
```

## Meta-Orchestration Agent

### workflow-orchestrator
The central coordinator that manages the entire workflow pipeline. Enforces phase gates, makes go/no-go decisions, and ensures each phase completes successfully before proceeding. Invoked proactively for all new tasks.

## Workflow Specialists

### requirements-gatherer (Phase 1)
Gathers and analyzes user requirements, clarifies objectives, identifies constraints, and prepares structured requirements. First step in every workflow.

### technical-requirements-specialist (Phase 2)
Translates requirements into detailed technical specifications, makes architecture decisions, researches best practices, and creates implementation plans.

### agent-creator (Phase 3)
Dynamically generates hyperspecialized agents when task requirements exceed existing agent capabilities. Creates new agents in `workers/custom/`.

### task-planner (Phase 4)
Decomposes technical specifications into atomic, independently executable tasks with explicit dependencies. Creates task queue for execution.

## Hyperspecialized Workers

The `workers/` directory is where task-specific micro-specialists are created dynamically during Phase 3. The template starts with an empty `workers/custom/` directory.

### Dynamic Agent Creation

As you use Abathur, the `agent-creator` specialist will automatically generate new workers in `workers/custom/` based on your project's specific needs. For example:

```bash
# User request triggers workflow
"Add GraphQL API to Django app"

# Phase 1: requirements-gatherer collects requirements
# Phase 2: technical-requirements-specialist defines architecture
# Phase 3: agent-creator generates specialized agents:
#   - django-graphql-implementer.md
#   - graphql-schema-designer.md
#   - api-endpoint-tester.md
# Phase 4: task-planner creates atomic tasks
# Phase 5: Execution with specialized workers
```

## Workflow Example

Here's how a typical task flows through the system:

```
User: "Build a REST API for user authentication"
  ↓
workflow-orchestrator (initiates pipeline)
  ↓
Phase 1: requirements-gatherer
  → Gathers requirements
  → Identifies: login, signup, password reset, JWT tokens
  → Gate: Requirements complete ✓
  ↓
Phase 2: technical-requirements-specialist
  → Designs architecture (database schema, endpoints, security)
  → Specifies JWT implementation, password hashing
  → Creates implementation plan
  → Gate: Technical specs complete ✓
  ↓
Phase 3: agent-creator
  → Checks for existing agents
  → Creates: auth-api-implementer, jwt-security-specialist
  → Gate: All required agents available ✓
  ↓
Phase 4: task-planner
  → Breaks down into atomic tasks
  → Creates dependency graph
  → Gate: Task plan validated ✓
  ↓
Phase 5: Execution
  → Tasks execute in dependency order
  → Specialized workers implement each piece
  → Complete ✓
```

## Phase Gates

Each phase has specific criteria that must be met:

**Phase 1 → 2:** Requirements complete, constraints identified, no unanswered questions
**Phase 2 → 3:** Architecture documented, implementation plan complete, agent needs identified
**Phase 3 → 4:** All required agents available, no capability gaps
**Phase 4 → 5:** Tasks atomic and testable, valid dependency graph, agents assigned

The workflow-orchestrator enforces these gates and blocks progression if criteria aren't met.

## Adding Custom Agents

Create your own agents in `workers/custom/`:

```markdown
---
name: my-specialist
description: "What it does. Keywords: keyword1, keyword2"
model: thinking
color: Blue
tools: Read, Edit, Bash
---

## Purpose
What this agent does

## Instructions
Detailed instructions

## Deliverable Output
```json
{
  "deliverables": [],
  "execution_status": "success"
}
```
```

## Agent Invocation

Agents are invoked automatically by the workflow-orchestrator based on the current phase:

```bash
# Let the orchestrator manage the workflow
abathur submit "Your task description"

# Check workflow status
abathur status

# View agent registry
abathur agents list
```

## Best Practices

- **Workflow Discipline** - Always follow the 5-phase workflow
- **Phase Gates** - Never skip validation gates
- **Single Responsibility** - Each agent has one micro-domain
- **Exhaustive Knowledge** - Include all best practices for the domain
- **Clear Boundaries** - Define what IS and IS NOT in scope
- **Minimal Tools** - Only tools required for the specific domain
- **Structured Output** - Use JSON with execution_status and deliverables

## More Information

- [Workflow Philosophy](https://github.com/odgrim/abathur-swarm/blob/main/docs/workflow.md)
- [Agent Catalog](https://github.com/odgrim/abathur-swarm/blob/main/AGENT_CATALOG.md)
- [Architecture Guide](https://github.com/odgrim/abathur-swarm/blob/main/docs/architecture.md)
- [Creating Agents](https://github.com/odgrim/abathur-swarm/blob/main/docs/creating-agents.md)
