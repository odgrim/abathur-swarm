# Abathur Runtime Agents

This directory contains the 13 core runtime agents that power the Abathur swarm orchestration system.

## Agent Architecture

Abathur uses a three-tier agent architecture:

1. **Meta-Orchestration** (2 agents) - Swarm-level coordination
2. **Autonomous Specialists** (6 agents) - Core framework capabilities
3. **Hyperspecialized Workers** (5+ agents) - Task-specific micro-specialists

## Directory Structure

```
agents/
├── meta/              # Meta-orchestration tier (2 agents)
│   ├── swarm-coordinator.md
│   └── context-synthesizer.md
├── specialists/       # Autonomous specialist tier (6 agents)
│   ├── task-planner.md
│   ├── agent-creator.md
│   ├── resource-allocator.md
│   ├── conflict-resolver.md
│   ├── performance-monitor.md
│   └── learning-coordinator.md
└── workers/           # Hyperspecialized worker tier (dynamically created)
    └── custom/        # Agent-creator generates workers here
```

## Meta-Orchestration Agents

### swarm-coordinator
Manages swarm lifecycle, health monitoring, and agent pool coordination. Invoked proactively for swarm management tasks.

### context-synthesizer
Maintains cross-swarm state coherence and synthesizes distributed context across agents.

## Autonomous Specialists

### task-planner
Decomposes complex tasks into atomic, independently executable units with explicit dependencies. Central to the Abathur workflow.

### agent-creator
Dynamically generates hyperspecialized agents when task requirements exceed existing agent capabilities. Creates new agents in `workers/custom/`.

### resource-allocator
Manages computational resources, task priorities, and concurrency limits across the swarm.

### conflict-resolver
Resolves inter-agent state conflicts and coordination issues to maintain swarm coherence.

### performance-monitor
Tracks swarm efficiency metrics and identifies optimization opportunities proactively.

### learning-coordinator
Captures patterns, improves agent performance, and coordinates swarm learning over time.

## Hyperspecialized Workers

The `workers/` directory is where task-specific micro-specialists are created dynamically. The template starts with an empty `workers/custom/` directory.

### Dynamic Agent Creation

As you use Abathur, the `agent-creator` specialist will automatically generate new workers in `workers/custom/` based on your project's specific needs. For example:

```bash
abathur submit "Add GraphQL API to Django app"
# agent-creator generates: django-graphql-implementer.md
```

## Adding Custom Agents

Create your own agents in `workers/custom/`:

```markdown
---
name: my-specialist
description: What it does. Keywords: keyword1, keyword2
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

Agents are invoked automatically by the orchestrator based on task requirements. You can also invoke them directly:

```bash
# Let the orchestrator choose agents
abathur submit "Your task description"

# Check swarm status
abathur health

# View agent registry
abathur agents list
```

## Best Practices

- **Single Responsibility** - Each agent has one micro-domain
- **Exhaustive Knowledge** - Include all best practices for the domain
- **Clear Boundaries** - Define what IS and IS NOT in scope
- **Minimal Tools** - Only tools required for the specific domain
- **Structured Output** - Use JSON with execution_status and deliverables

## More Information

- [Agent Catalog](https://github.com/odgrim/abathur-swarm/blob/main/AGENT_CATALOG.md)
- [Architecture Guide](https://github.com/odgrim/abathur-swarm/blob/main/docs/architecture.md)
- [Creating Agents](https://github.com/odgrim/abathur-swarm/blob/main/docs/creating-agents.md)
