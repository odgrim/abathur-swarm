# Abathur

A self-evolving swarm orchestrator for AI agents, written in Rust.

Abathur coordinates multiple AI agents to work on complex tasks. It handles task decomposition, parallel execution, git worktree isolation, and learns from agent performance over time.

## What it does

- **Task orchestration**: Break down work into a DAG of subtasks, route them to specialized agents, run them in parallel waves
- **Git worktree isolation**: Each task runs in its own worktree so agents can work on different parts of the codebase simultaneously without stepping on each other
- **Three-tier memory**: Working, episodic, and semantic memory with configurable decay. Agents can query past context and learnings
- **Agent evolution**: Track success rates per agent template version. Underperforming agents get refined, regressions get reverted
- **Meta-planning**: A meta-planner agent analyzes incoming tasks, detects capability gaps, and can spawn new specialist agents when needed

## Prerequisites

- **Rust 1.85+** — install via [rustup](https://rustup.rs)
- **ANTHROPIC_API_KEY** — set in your environment; agents run on Claude
- **Git 2.5+** — worktree isolation requires Git worktree support
- No system SQLite required — SQLite is bundled in the binary

## Installation

**Quick install** (Linux and macOS):

```bash
curl -fsSL https://raw.githubusercontent.com/odgrim/abathur-swarm/main/install.sh | bash
```

**With cargo**:

```bash
cargo install --git https://github.com/odgrim/abathur-swarm.git
```

**Build from source**:

```bash
git clone https://github.com/odgrim/abathur-swarm.git
cd abathur-swarm
cargo build --release
# Binary is at target/release/abathur
```

## Getting started

Initialize a project directory:

```bash
abathur init
```

This creates:
- `.abathur/` directory for the database, worktrees, and logs
- `.claude/` directory with MCP server configuration

Set a goal (goals are convergent and guide work, they don't complete):

```bash
abathur goal set "Maintain a clean, well-tested codebase with comprehensive documentation" --priority high
```

Submit a task:

```bash
abathur task submit "Add user registration endpoint with input validation" --goal <goal-id>
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
abathur trigger        Manage trigger rules for event-driven automation
abathur schedule       Manage periodic task schedules
abathur event          Query and inspect the event store
abathur workflow       Manage workflow templates
abathur adapter        Manage adapter plugins
```

All commands support `--json` for machine-readable output and `--config <path>` to override the default `abathur.toml`.

## Configuration

Create an `abathur.toml` in your project root (see `examples/abathur.toml` for a fully annotated reference):

```toml
[limits]
max_depth = 5
max_subtasks = 10
max_descendants = 100
max_concurrent_tasks = 5

[memory]
decay_rate = 0.05
prune_threshold = 0.1
maintenance_interval_secs = 3600

[worktrees]
base_path = ".abathur/worktrees"
auto_cleanup = true

[polling]
goal_convergence_check_interval_secs = 28800
```

Environment variables with the `ABATHUR_` prefix override config file values (e.g. `ABATHUR_LIMITS_MAX_DEPTH`, `ABATHUR_DATABASE_PATH`, `ABATHUR_LOG_LEVEL`).

## Architecture

Abathur follows hexagonal architecture — domain logic is isolated from infrastructure through port interfaces:

```
┌─────────────────────────────────────────────────────────────┐
│                        CLI / MCP                            │
│              (clap commands, HTTP JSON handlers)            │
└───────────────────────┬─────────────────────────────────────┘
                        │
┌───────────────────────▼─────────────────────────────────────┐
│                      Services                               │
│  orchestrator · meta-planner · memory-decay · merge-queue   │
│  convergence-engine · evolution-loop · adapter-manager      │
└───────┬──────────────────────────────────────┬──────────────┘
        │  domain/ports (traits)               │
┌───────▼──────────┐               ┌───────────▼──────────────┐
│   Domain Models  │               │       Adapters           │
│  tasks · goals   │               │  SQLite repositories     │
│  memory · agents │               │  Claude / Anthropic API  │
│  worktrees       │               │  External plugins        │
│  events          │               │  (GitHub, ClickUp, …)    │
└──────────────────┘               └──────────────────────────┘
```

Source layout:

- `src/domain/models/` — Core types: tasks, goals, memory, agents, worktrees, events
- `src/domain/ports/` — Repository and service traits (interfaces)
- `src/adapters/sqlite/` — SQLite implementations of all repository ports
- `src/adapters/substrates/` — LLM backend integrations (Claude Code, Anthropic API)
- `src/adapters/plugins/` — External adapter plugins (GitHub Issues, ClickUp)
- `src/services/` — Business logic: orchestrator, meta-planner, memory decay, merge queue
- `src/cli/` — Command handlers

## How agents work

Agent templates are stored in the database as the sole source of truth. The only pre-packaged agent is the **Overmind** (hardcoded in Rust as bootstrap); all other agents are created dynamically by the Overmind at runtime via the Agents MCP API.

Agent tiers:
- **Architect**: Analyzes tasks, designs execution topology, creates new agents (Overmind)
- **Specialist**: Domain expertise (security, performance, databases)
- **Worker**: Task execution (code implementation, tests, docs, refactoring)

## Two-stage merge queue

When agents complete work:
1. Agent branches merge into the task branch
2. Task branches merge into main after integration verification passes

Conflicts trigger retry-with-rebase. Persistent conflicts escalate to a merge conflict specialist agent.

## External adapters

Adapters connect Abathur to external project management tools, feeding work items in as tasks and syncing status back out. Adapter definitions live in `.abathur/adapters/<name>/adapter.toml`.

### GitHub Issues

Polls GitHub repository issues and creates Abathur tasks. Lifecycle events (task completed, failed) close or reopen the corresponding issue.

```toml
# .abathur/adapters/github-issues/adapter.toml
[config]
owner         = "your-org"
repo          = "your-repo"
state         = "open"
filter_labels = "swarm-ingestible"
```

Requires `GITHUB_TOKEN` in the environment.

### ClickUp

Polls a ClickUp list for tasks and syncs status updates back. Tag-based filtering controls which tasks are ingested.

```toml
# .abathur/adapters/clickup/adapter.toml
[config]
list_id            = "901711146339"
filter_tag         = ""
status_pending     = "PENDING"
status_in_progress = "IN PROGRESS"
status_done        = "COMPLETED"
```

Requires `CLICKUP_API_TOKEN` in the environment.

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for development setup, coding conventions, and the PR process.

## License

MIT
