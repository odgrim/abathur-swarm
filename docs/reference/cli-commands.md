# CLI Command Reference

Complete reference for all Abathur command-line interface commands.

## Synopsis

```bash
abathur [GLOBAL_OPTIONS] <COMMAND> [SUBCOMMAND] [OPTIONS] [ARGUMENTS]
```

## Global Options

Global options can be used with any command:

| Option | Description |
|--------|-------------|
| `--json` | Output results in JSON format for scripting |
| `-h, --help` | Display help information |
| `-V, --version` | Display version information |

## Commands

### init

Initialize Abathur configuration and database.

**Usage**:
```bash
abathur init [OPTIONS]
```

**Options**:

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `-f, --force` | flag | `false` | Force reinitialization even if already initialized |
| `-t, --template-repo <URL>` | string | `https://github.com/odgrim/abathur-claude-template` | Custom template repository URL |
| `--skip-clone` | flag | `false` | Skip cloning template repository (use existing template/ directory) |

**Description**:

Initializes Abathur by creating the configuration directory, generating the config file, running database migrations, cloning the agent template repository, and setting up MCP server configuration.

**Examples**:

Initialize Abathur with default settings:
```bash
abathur init
```

Force reinitialize with custom template:
```bash
abathur init --force --template-repo https://github.com/myuser/my-template
```

Initialize without cloning templates:
```bash
abathur init --skip-clone
```

Get JSON output:
```bash
abathur --json init
```

**Output**:

Success:
```
Initializing Abathur...

✓ Created config directory: .abathur
✓ Created config file: .abathur/config.yaml
✓ Database initialized: .abathur/abathur.db
✓ Cloned template repository to template
✓ Copied agent templates
✓ Merged MCP server configuration

✓ Abathur initialized successfully!

Configuration: .abathur/config.yaml
Database: .abathur/abathur.db
Agents: .claude/agents

Next steps:
  1. Edit your config file to customize settings
  2. Set ANTHROPIC_API_KEY environment variable
  3. Run 'abathur swarm start' to start the orchestrator
```

## task

Task queue management commands.

### task submit

Submit a new task to the execution queue.

**Usage**:
```bash
abathur task submit <DESCRIPTION> [OPTIONS]
```

**Arguments**:

| Argument | Required | Description |
|----------|----------|-------------|
| `<DESCRIPTION>` | Yes | Task description (used as summary if --summary not provided) |

**Options**:

| Option | Short | Type | Default | Description |
|--------|-------|------|---------|-------------|
| `--agent-type` | `-a` | string | `requirements-gatherer` | Agent type to execute the task |
| `--summary` | `-s` | string | (auto-generated) | Brief task summary (max 140 characters) |
| `--priority` | `-p` | integer | `5` | Task priority (0-10, higher = more urgent) |
| `--dependencies` | `-D` | list | `[]` | Comma-separated task IDs (full UUID or prefix) |

**Description**:

Submits a new task to the queue for execution by the specified agent type. Tasks can have dependencies that must complete before the task becomes ready. If no summary is provided, the first 140 characters of the description are used.

**Examples**:

Submit a simple task:
```bash
abathur task submit "Implement user authentication"
```

Submit with specific agent and priority:
```bash
abathur task submit "Deploy to production" \
  --agent-type rust-deployment-specialist \
  --priority 9
```

Submit with dependencies (using task ID prefixes):
```bash
abathur task submit "Integration tests" \
  --dependencies a1b2c3,d4e5f6 \
  --priority 7
```

Submit with custom summary:
```bash
abathur task submit "This is a very long description that exceeds the summary limit..." \
  --summary "Implement feature X"
```

Get JSON output:
```bash
abathur --json task submit "Test task"
```

**Output**:

Standard output:
```
Task submitted successfully!
  Task ID: 550e8400-e29b-41d4-a716-446655440000
  Summary: Implement user authentication
  Description: Implement user authentication
  Agent type: requirements-gatherer
  Priority: 5
```

With dependencies:
```
Task submitted successfully!
  Task ID: 550e8400-e29b-41d4-a716-446655440000
  Summary: Integration tests
  Description: Integration tests
  Agent type: requirements-gatherer
  Priority: 7
  Dependencies: 2 task(s)
```

### task list

List tasks in the queue with optional filtering.

**Usage**:
```bash
abathur task list [OPTIONS]
```

**Options**:

| Option | Short | Type | Default | Description |
|--------|-------|------|---------|-------------|
| `--status` | `-s` | string | (none) | Filter by status: `pending`, `blocked`, `ready`, `running`, `completed`, `failed`, `cancelled` |
| `--limit` | `-l` | integer | `50` | Maximum number of tasks to display |

**Description**:

Lists tasks from the queue with optional status filtering. Results are displayed in a formatted table showing task ID (first 8 characters), status, priority, agent type, and summary.

**Examples**:

List all tasks (up to 50):
```bash
abathur task list
```

List only ready tasks:
```bash
abathur task list --status ready
```

List failed tasks with higher limit:
```bash
abathur task list --status failed --limit 100
```

Get JSON output:
```bash
abathur --json task list
```

**Output**:

Standard output (table format):
```
Tasks:
┌──────────┬──────────┬──────────┬─────────────────────────┬───────────────────────────────┐
│ ID       │ Status   │ Priority │ Agent Type              │ Summary                       │
├──────────┼──────────┼──────────┼─────────────────────────┼───────────────────────────────┤
│ 550e8400 │ Ready    │ 7        │ rust-specialist         │ Implement authentication      │
│ 661f9511 │ Running  │ 9        │ technical-architect     │ Design system architecture    │
│ 772fa622 │ Pending  │ 5        │ requirements-gatherer   │ Gather requirements           │
└──────────┴──────────┴──────────┴─────────────────────────┴───────────────────────────────┘

Showing 3 task(s)
```

Empty result:
```
No tasks found.
```

### task show

Display detailed information for a specific task.

**Usage**:
```bash
abathur task show <TASK_ID>
```

**Arguments**:

| Argument | Required | Description |
|----------|----------|-------------|
| `<TASK_ID>` | Yes | Task ID (full UUID or prefix - minimum 4 characters) |

**Description**:

Shows complete details for a specific task including ID, status, summary, description, agent type, priorities (base and computed), timestamps, and dependencies.

**Examples**:

Show task by full UUID:
```bash
abathur task show 550e8400-e29b-41d4-a716-446655440000
```

Show task by prefix:
```bash
abathur task show 550e8400
```

Get JSON output:
```bash
abathur --json task show 550e
```

**Output**:

Standard output:
```
Task Details:
  ID: 550e8400-e29b-41d4-a716-446655440000
  Status: Running
  Summary: Implement authentication
  Description: Implement user authentication with JWT tokens
  Agent type: rust-specialist
  Priority: 7 (computed: 7.2)
  Created at: 2025-10-29 14:30:00 UTC
  Updated at: 2025-10-29 14:35:00 UTC
  Started at: 2025-10-29 14:35:00 UTC
  Dependencies:
    - 661f9511-e29b-41d4-a716-446655440001
    - 772fa622-e29b-41d4-a716-446655440002
```

Error (task not found):
```
Error: Task 550e not found. Use 'abathur task list' to see available tasks.
```

### task update

Update one or more tasks.

**Usage**:
```bash
abathur task update <TASK_IDS> [OPTIONS]
```

**Arguments**:

| Argument | Required | Description |
|----------|----------|-------------|
| `<TASK_IDS>` | Yes | Comma-separated task IDs (full UUID or prefix) |

**Options**:

| Option | Short | Type | Description |
|--------|-------|------|-------------|
| `--status` | `-s` | string | Update task status |
| `--priority` | `-p` | integer | Update base priority (0-10) |
| `--agent-type` | `-a` | string | Update agent type |
| `--add-dependency` | | list | Add dependencies (comma-separated UUIDs or prefixes) |
| `--remove-dependency` | | list | Remove dependencies (comma-separated UUIDs or prefixes) |
| `--retry` | | flag | Increment retry count and reset to pending (for failed tasks) |
| `--cancel` | | flag | Cancel task and cascade to dependents |

**Description**:

Updates one or more tasks with specified changes. At least one update operation must be specified. When updating multiple tasks, results show which succeeded and which failed.

!!! warning "Cascading Cancellation"
    Using `--cancel` will cancel the specified task(s) and all tasks that depend on them. This operation cannot be undone.

**Examples**:

Update task status:
```bash
abathur task update 550e8400 --status completed
```

Update priority:
```bash
abathur task update 550e --priority 9
```

Add dependencies:
```bash
abathur task update 661f9511 --add-dependency a1b2c3,d4e5f6
```

Remove dependencies:
```bash
abathur task update 661f --remove-dependency a1b2
```

Retry failed task:
```bash
abathur task update 772fa622 --retry
```

Cancel task:
```bash
abathur task update 550e --cancel
```

Update multiple tasks:
```bash
abathur task update 550e,661f,772f --priority 8
```

**Output**:

Success:
```
Successfully updated 1 task(s):
  - 550e8400-e29b-41d4-a716-446655440000
```

Multiple tasks with partial failure:
```
Successfully updated 2 task(s):
  - 550e8400-e29b-41d4-a716-446655440000
  - 661f9511-e29b-41d4-a716-446655440001

Failed to update 1 task(s):
  - 772fa622-e29b-41d4-a716-446655440002: Task not found
```

### task status

Display queue status and statistics.

**Usage**:
```bash
abathur task status
```

**Description**:

Shows comprehensive statistics about the task queue including total tasks and breakdown by status (pending, blocked, ready, running, completed, failed, cancelled).

**Examples**:

Get queue status:
```bash
abathur task status
```

Get JSON output:
```bash
abathur --json task status
```

**Output**:

Standard output (table format):
```
Queue Status:
┌───────────┬───────┐
│ Status    │ Count │
├───────────┼───────┤
│ Total     │ 127   │
│ Pending   │ 12    │
│ Blocked   │ 5     │
│ Ready     │ 8     │
│ Running   │ 3     │
│ Completed │ 95    │
│ Failed    │ 3     │
│ Cancelled │ 1     │
└───────────┴───────┘
```

### task resolve

Resolve task dependencies and update statuses.

**Usage**:
```bash
abathur task resolve
```

**Description**:

Checks all Pending and Blocked tasks and updates them to Ready status if their dependencies are satisfied. This command is useful after tasks complete to automatically unblock dependent tasks.

!!! tip "Automatic Resolution"
    The swarm orchestrator automatically resolves dependencies. This command is primarily useful for manual queue management or troubleshooting.

**Examples**:

Resolve dependencies:
```bash
abathur task resolve
```

Get JSON output:
```bash
abathur --json task resolve
```

**Output**:

Success with tasks updated:
```
Task Dependency Resolution
=========================
Tasks updated to Ready: 5

Run 'abathur task list --status ready' to view ready tasks.
```

No tasks updated:
```
Task Dependency Resolution
=========================
Tasks updated to Ready: 0

No tasks were ready to be updated.
Check 'abathur task list --status pending' or '--status blocked' for pending tasks.
```

## memory

Memory management commands.

### memory list

List memories with optional filtering.

**Usage**:
```bash
abathur memory list [OPTIONS]
```

**Options**:

| Option | Short | Type | Default | Description |
|--------|-------|------|---------|-------------|
| `--namespace` | `-n` | string | (none) | Filter by namespace prefix |
| `--memory-type` | `-t` | string | (none) | Filter by memory type: `semantic`, `episodic`, `procedural` |
| `--limit` | `-l` | integer | `50` | Maximum number of memories to display |

**Description**:

Lists memories from the hierarchical memory system with optional filtering by namespace prefix and memory type. Results are displayed in a formatted table.

**Examples**:

List all memories:
```bash
abathur memory list
```

List memories in specific namespace:
```bash
abathur memory list --namespace task:550e8400
```

List only semantic memories:
```bash
abathur memory list --memory-type semantic
```

Combine filters:
```bash
abathur memory list --namespace agent: --memory-type episodic --limit 100
```

Get JSON output:
```bash
abathur --json memory list
```

**Output**:

Standard output (table format):
```
Memories:
┌─────────────────────────────┬──────────────────┬───────────┬──────────────┐
│ Namespace                   │ Key              │ Type      │ Created By   │
├─────────────────────────────┼──────────────────┼───────────┼──────────────┤
│ task:550e8400:specs         │ requirements     │ Semantic  │ gatherer     │
│ task:550e8400:implementation│ phase1_complete  │ Episodic  │ architect    │
│ agent:rust-specialist       │ patterns         │ Procedural│ system       │
└─────────────────────────────┴──────────────────┴───────────┴──────────────┘

Showing 3 memories
```

Empty result:
```
No memories found.
```

### memory show

Display detailed information for a specific memory entry.

**Usage**:
```bash
abathur memory show <NAMESPACE> <KEY>
```

**Arguments**:

| Argument | Required | Description |
|----------|----------|-------------|
| `<NAMESPACE>` | Yes | Memory namespace |
| `<KEY>` | Yes | Memory key |

**Description**:

Shows complete details for a specific memory entry including namespace, key, type, value (as formatted JSON), metadata, and timestamps.

**Examples**:

Show specific memory:
```bash
abathur memory show task:550e8400:specs requirements
```

Get JSON output:
```bash
abathur --json memory show task:550e8400:specs requirements
```

**Output**:

Standard output:
```
Memory Details:
─────────────────────────────────────────
Namespace:   task:550e8400:specs
Key:         requirements
Type:        Semantic
Created by:  requirements-gatherer
Updated by:  requirements-gatherer
Created at:  2025-10-29 14:30:00 UTC
Updated at:  2025-10-29 14:35:00 UTC

Value:
{
  "functional_requirements": [
    "User authentication with JWT",
    "Role-based access control"
  ],
  "non_functional_requirements": [
    "99.9% uptime",
    "Sub-100ms response time"
  ]
}

Metadata:
{
  "version": "1.2",
  "confidence": "high"
}
```

Error (not found):
```
Error: Memory not found at task:550e8400:specs:requirements
```

### memory count

Count memories matching specified criteria.

**Usage**:
```bash
abathur memory count [OPTIONS]
```

**Options**:

| Option | Short | Type | Default | Description |
|--------|-------|------|---------|-------------|
| `--namespace` | `-n` | string | `""` | Namespace prefix to count (empty = all) |
| `--memory-type` | `-t` | string | (none) | Filter by memory type: `semantic`, `episodic`, `procedural` |

**Description**:

Counts the number of memories matching the specified criteria. Useful for understanding memory usage and organization.

**Examples**:

Count all memories:
```bash
abathur memory count
```

Count memories in namespace:
```bash
abathur memory count --namespace task:550e8400
```

Count semantic memories:
```bash
abathur memory count --memory-type semantic
```

Combine filters:
```bash
abathur memory count --namespace agent: --memory-type procedural
```

Get JSON output:
```bash
abathur --json memory count --namespace task:
```

**Output**:

Standard output:
```
Found 47 memories matching prefix 'task:550e8400'
```

With type filter:
```
Found 23 semantic memories matching prefix 'agent:'
```

All memories:
```
Found 312 memories matching prefix ''
```

## swarm

Swarm orchestration commands.

### swarm start

Start the swarm orchestrator.

**Usage**:
```bash
abathur swarm start [OPTIONS]
```

**Options**:

| Option | Short | Type | Default | Description |
|--------|-------|------|---------|-------------|
| `--max-agents` | `-m` | integer | `10` | Maximum number of concurrent agents |

**Description**:

Starts the swarm orchestrator as a background daemon process. The orchestrator automatically picks up tasks from the queue and assigns them to agents. HTTP MCP servers are started on ports 45678 (memory) and 45679 (tasks) for external client access.

!!! info "Prerequisites"
    The swarm requires:
    - Initialized database (run `abathur init`)
    - `ANTHROPIC_API_KEY` environment variable set
    - Or Claude CLI installed and authenticated

**Examples**:

Start with default settings:
```bash
abathur swarm start
```

Start with custom max agents:
```bash
abathur swarm start --max-agents 20
```

Get JSON output:
```bash
abathur --json swarm start
```

**Output**:

Success:
```
Starting swarm orchestrator with 10 max agents...
Swarm orchestrator started successfully

Daemon logs are written to: .abathur/swarm_daemon.log
```

Without initialization:
```
Starting swarm orchestrator with 10 max agents...
Swarm orchestrator started successfully

Daemon logs are written to: .abathur/swarm_daemon.log

Note: Full orchestration requires database setup.
Run 'abathur init' to initialize Abathur.
```

Error:
```
Failed to start swarm orchestrator: No healthy substrates available

Check logs at: .abathur/swarm_daemon.log

To enable full swarm functionality:
  1. Run 'abathur init' to initialize Abathur
  2. Ensure ANTHROPIC_API_KEY environment variable is set
```

### swarm stop

Stop the swarm orchestrator.

**Usage**:
```bash
abathur swarm stop
```

**Description**:

Gracefully stops the swarm orchestrator daemon. Running agents complete their current tasks before shutdown. HTTP MCP servers are also stopped.

**Examples**:

Stop the swarm:
```bash
abathur swarm stop
```

Get JSON output:
```bash
abathur --json swarm stop
```

**Output**:

Success:
```
Stopping swarm orchestrator...
Swarm orchestrator stopped successfully
```

Error:
```
Failed to stop swarm orchestrator: No running swarm found
```

### swarm status

Show swarm orchestrator status and statistics.

**Usage**:
```bash
abathur swarm status
```

**Description**:

Displays the current state of the swarm orchestrator including active/idle agents, queue statistics, and performance metrics.

**Examples**:

Get swarm status:
```bash
abathur swarm status
```

Get JSON output:
```bash
abathur --json swarm status
```

**Output**:

Standard output:
```
Swarm Orchestrator Status
========================
State: Running
Active Agents: 7
Idle Agents: 3
Max Agents: 10
Tasks Processed: 142
Tasks Failed: 3

Queue Statistics:
  Total Tasks: 87
  Pending: 12
  Blocked: 5
  Ready: 8
  Running: 7
  Completed: 52
  Failed: 2
  Cancelled: 1
```

## mcp

MCP (Model Context Protocol) server commands for internal use.

!!! warning "Internal Use Only"
    These commands are typically used internally by the swarm orchestrator. Manual use is only needed for debugging or custom integrations.

### mcp memory-http

Run HTTP MCP server for memory management.

**Usage**:
```bash
abathur mcp memory-http [OPTIONS]
```

**Options**:

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `--db-path` | string | `.abathur/abathur.db` | Path to SQLite database file |
| `--port` | integer | `45678` | Port to listen on |

**Description**:

Starts an HTTP MCP server that exposes memory management operations. This allows external MCP clients to interact with the memory system.

**Examples**:

Start with default settings:
```bash
abathur mcp memory-http
```

Use custom database and port:
```bash
abathur mcp memory-http --db-path /path/to/db.sqlite --port 8080
```

### mcp tasks-http

Run HTTP MCP server for task queue management.

**Usage**:
```bash
abathur mcp tasks-http [OPTIONS]
```

**Options**:

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `--db-path` | string | `.abathur/abathur.db` | Path to SQLite database file |
| `--port` | integer | `45679` | Port to listen on |

**Description**:

Starts an HTTP MCP server that exposes task queue operations. This allows external MCP clients to interact with the task system.

**Examples**:

Start with default settings:
```bash
abathur mcp tasks-http
```

Use custom database and port:
```bash
abathur mcp tasks-http --db-path /path/to/db.sqlite --port 8081
```

## Exit Codes

Abathur uses standard exit codes:

| Code | Meaning |
|------|---------|
| `0` | Success |
| `1` | General error (invalid arguments, operation failed) |
| `2` | Database error |
| `3` | Configuration error |
| `101` | Not initialized (run `abathur init`) |

## Environment Variables

| Variable | Description | Required |
|----------|-------------|----------|
| `ANTHROPIC_API_KEY` | Anthropic API key for Claude access | Yes (or use Claude CLI) |
| `ABATHUR_CONFIG` | Custom path to config file | No |
| `ABATHUR_DB` | Custom path to database file | No |
| `RUST_LOG` | Log level (error, warn, info, debug, trace) | No |

## JSON Output Format

All commands support `--json` flag for machine-readable output. JSON responses follow this structure:

**Success Response**:
```json
{
  "status": "success",
  "data": {
    // Command-specific data
  }
}
```

**Error Response**:
```json
{
  "status": "error",
  "message": "Error description",
  "details": {
    // Optional error details
  }
}
```

## Common Patterns

### Working with Task Prefixes

Task IDs can be specified using prefixes for convenience:

```bash
# Full UUID
abathur task show 550e8400-e29b-41d4-a716-446655440000

# Short prefix (minimum 4 characters)
abathur task show 550e

# Longer prefix for disambiguation
abathur task show 550e8400
```

!!! tip "Prefix Length"
    Use longer prefixes if multiple tasks share the same starting characters. Abathur will error if a prefix matches multiple tasks.

### Scripting with JSON Output

Use `--json` for reliable parsing in scripts:

```bash
# Get task ID from submission
TASK_ID=$(abathur --json task submit "Build project" | jq -r '.task_id')

# Check if swarm is running
STATUS=$(abathur --json swarm status | jq -r '.swarm.state')
if [ "$STATUS" = "Running" ]; then
  echo "Swarm is active"
fi

# Count failed tasks
FAILED=$(abathur --json task status | jq -r '.queue.failed')
echo "Failed tasks: $FAILED"
```

### Chaining Tasks with Dependencies

Create task pipelines using dependencies:

```bash
# Submit first task
TASK1=$(abathur --json task submit "Design architecture" | jq -r '.task_id')

# Submit dependent task
TASK2=$(abathur --json task submit "Implement design" \
  --dependencies $TASK1 | jq -r '.task_id')

# Submit final task depending on both
abathur task submit "Deploy system" \
  --dependencies $TASK1,$TASK2 \
  --priority 9
```

### Monitoring Queue

Monitor queue in real-time:

```bash
# Watch queue status (Linux/macOS)
watch -n 5 'abathur task status'

# Check for ready tasks
abathur task list --status ready

# View running tasks
abathur task list --status running
```

## See Also

- [Configuration Reference](configuration.md) - Complete configuration options
- [Getting Started: Quickstart](../getting-started/quickstart.md) - Quick introduction tutorial
- [How-To: Task Management](../how-to/task-management.md) - Task queue recipes
- [Explanation: Task Queue](../explanation/task-queue.md) - Understanding task execution
