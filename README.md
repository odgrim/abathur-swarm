# Abathur

A self-evolving swarm orchestrator for AI agents, written in Rust.

Abathur coordinates multiple AI agents to work on complex tasks. It handles task decomposition, parallel execution, git worktree isolation, and learns from agent performance over time.

## What it does

- **Task orchestration**: Break down work into a DAG of subtasks, route them to specialized agents, run them in parallel waves
- **Git worktree isolation**: Each task runs in its own worktree so agents can work on different parts of the codebase simultaneously without stepping on each other
- **Three-tier memory**: Semantic, episodic, and procedural memory with configurable decay. Agents can query past context and learnings
- **Agent evolution**: Track success rates per agent template version. Underperforming agents get refined, regressions get reverted
- **Meta-planning**: A meta-planner agent analyzes incoming tasks, detects capability gaps, and can spawn new specialist agents when needed

## Building

```bash
cargo build --release
```

## Getting started

Initialize a project directory:

```bash
abathur init
```

This creates:
- `.abathur/` directory for the database, worktrees, and logs
- `.claude/` directory with baseline agent definitions

Set a goal (goals are convergent and guide work, they don't complete):

```bash
abathur goal set "Build a REST API for user management" --priority high
```

Submit a task:

```bash
abathur task submit "Implement user registration endpoint" --goal <goal-id>
```

Start the swarm:

```bash
abathur swarm start
```

## CLI commands

```
abathur init           Initialize project structure
abathur goal           Manage convergent goals
abathur task           Submit and track tasks  
abathur memory         Query the three-tier memory system
abathur agent          List and inspect agent templates
abathur worktree       Manage git worktrees for task isolation
abathur swarm          Start/stop the orchestrator
abathur mcp            Run MCP servers for agent access to infrastructure
```

All commands support `--json` for machine-readable output.

## Configuration

Create an `abathur.toml` in your project root:

```toml
[limits]
max_depth = 5
max_subtasks = 10
max_descendants = 50

[memory]
semantic_decay_rate = 0.01
episodic_decay_rate = 0.05
procedural_decay_rate = 0.02

[worktree]
base_path = ".abathur/worktrees"
```

Environment variables with the `ABATHUR_` prefix override config file values.

## Architecture

The codebase follows hexagonal architecture:

- `domain/models/` - Core types: tasks, goals, memory, agents, worktrees
- `domain/ports/` - Repository traits (interfaces)
- `adapters/sqlite/` - SQLite implementations
- `adapters/substrates/` - LLM backend integrations (Claude Code, Anthropic API)
- `services/` - Business logic: orchestrator, meta-planner, memory decay, merge queue
- `cli/` - Command handlers

## How agents work

Agents are markdown files in `.claude/agents/` with YAML frontmatter defining their capabilities, allowed tools, and constraints. The orchestrator copies the entire `.claude/` directory to each worktree so agents have access to definitions and MCP config.

Agent tiers:
- **Meta**: Analyzes tasks, designs execution topology, creates new agents
- **Strategic**: Product strategy, requirements, architecture, task decomposition
- **Execution**: Code implementation, tests, docs, refactoring  
- **Specialist**: Security auditing, performance optimization, merge conflicts, database work

## Two-stage merge queue

When agents complete work:
1. Agent branches merge into the task branch
2. Task branches merge into main after integration verification passes

Conflicts trigger retry-with-rebase. Persistent conflicts escalate to a merge conflict specialist agent.

## License

MIT
