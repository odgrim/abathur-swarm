# CLI Command Reference

Complete reference for all Abathur command-line interface commands.

## Global Syntax

```bash
abathur [global-options] <command> [subcommand] [options] [arguments]
```

## Global Options

| Option | Short | Type | Default | Description |
|--------|-------|------|---------|-------------|
| `--json` | `-j` | flag | `false` | Output results in JSON format |
| `--help` | `-h` | flag | - | Display help information |
| `--version` | `-V` | flag | - | Display version information |

---

## Commands Overview

- [`init`](#init) - Initialize Abathur configuration and database
- [`task`](#task-commands) - Task management commands
- [`memory`](#memory-commands) - Memory management commands
- [`swarm`](#swarm-commands) - Swarm orchestration commands
- [`mcp`](#mcp-commands) - MCP server commands (internal use)

---

## `init`

Initialize Abathur configuration and database.

### Syntax

```bash
abathur init [options]
```

### Description

Sets up the Abathur environment by:
- Creating the `.abathur/` directory structure
- Initializing the SQLite database
- Running database migrations
- Cloning the default agent template repository

### Options

| Option | Short | Type | Default | Description |
|--------|-------|------|---------|-------------|
| `--force` | `-f` | flag | `false` | Force reinitialization even if already initialized |
| `--template-repo` | `-t` | string | `https://github.com/odgrim/abathur-claude-template` | Custom template repository URL |
| `--skip-clone` | | flag | `false` | Skip cloning template repository (use existing template/ directory) |

### Examples

**Basic initialization**:
```bash
abathur init
```

**Force reinitialize with custom template**:
```bash
abathur init --force --template-repo https://github.com/example/custom-template
```

**Initialize without cloning template**:
```bash
abathur init --skip-clone
```

### Exit Codes

- `0`: Success
- `1`: Initialization failed
- `2`: Database error

### See Also

- [Installation Guide](../getting-started/installation.md)
- [Configuration Reference](configuration.md)

---

## Task Commands

Manage tasks in the execution queue.

### `task submit`

Submit a new task to the queue.

#### Syntax

```bash
abathur task submit <description> [options]
```

#### Required Arguments

- `<description>`: Task description (positional argument)

#### Options

| Option | Short | Type | Default | Description |
|--------|-------|------|---------|-------------|
| `--agent-type` | `-a` | string | `requirements-gatherer` | Agent type to execute the task |
| `--summary` | `-s` | string | - | Optional brief summary of the task |
| `--priority` | `-p` | integer (0-10) | `5` | Task priority (higher = more urgent) |
| `--dependencies` | `-D` | list | `[]` | Comma-separated task IDs or prefixes |

#### Examples

**Basic task submission**:
```bash
abathur task submit "Implement user authentication feature"
```

**Output**:
```
Task submitted successfully
ID: 550e8400-e29b-41d4-a716-446655440000
Status: pending
Priority: 5
Agent Type: requirements-gatherer
```

**High-priority task with specific agent**:
```bash
abathur task submit "Fix critical login bug" \
  --priority 9 \
  --agent-type rust-debugging-specialist
```

**Task with dependencies**:
```bash
abathur task submit "Deploy to production" \
  --summary "Production deployment" \
  --priority 8 \
  --dependencies "550e8400,a3b4c5d6"
```

**JSON output**:
```bash
abathur --json task submit "Create API endpoint"
```

**Output**:
```json
{
  "task_id": "550e8400-e29b-41d4-a716-446655440000",
  "status": "pending",
  "priority": 5,
  "agent_type": "requirements-gatherer"
}
```

#### Exit Codes

- `0`: Success
- `1`: Invalid arguments
- `2`: Task creation failed
- `3`: Database error

---

### `task list`

List tasks in the queue.

#### Syntax

```bash
abathur task list [options]
```

#### Options

| Option | Short | Type | Default | Description |
|--------|-------|------|---------|-------------|
| `--status` | `-s` | string | - | Filter by status (pending, ready, running, completed, failed, cancelled, blocked) |
| `--limit` | `-l` | integer | `50` | Maximum number of tasks to display |

#### Examples

**List all tasks**:
```bash
abathur task list
```

**Output**:
```
ID       Status    Priority  Agent Type              Summary
550e8400 pending   5         requirements-gatherer   Implement authentication
a3b4c5d6 running   8         rust-specialist         Fix login bug
7f8e9d0c completed 5         requirements-gatherer   Create API endpoint
```

**Filter by status**:
```bash
abathur task list --status pending
```

**Limit results**:
```bash
abathur task list --limit 10
```

**JSON output**:
```bash
abathur --json task list --status running
```

**Output**:
```json
{
  "tasks": [
    {
      "id": "a3b4c5d6-e29b-41d4-a716-446655440001",
      "status": "running",
      "priority": 8,
      "agent_type": "rust-specialist",
      "summary": "Fix login bug"
    }
  ]
}
```

#### Exit Codes

- `0`: Success
- `1`: Invalid arguments
- `3`: Database error

---

### `task show`

Show details for a specific task.

#### Syntax

```bash
abathur task show <task_id>
```

#### Required Arguments

- `<task_id>`: Task ID (full UUID or unique prefix)

#### Examples

**Show full task details**:
```bash
abathur task show 550e8400
```

**Output**:
```
Task ID: 550e8400-e29b-41d4-a716-446655440000
Status: pending
Priority: 5
Agent Type: requirements-gatherer
Created: 2025-10-29 14:30:00 UTC
Updated: 2025-10-29 14:30:00 UTC

Description:
Implement user authentication feature

Dependencies:
- None

Dependents:
- 7f8e9d0c: Deploy to production
```

**JSON output**:
```bash
abathur --json task show 550e8400
```

**Output**:
```json
{
  "id": "550e8400-e29b-41d4-a716-446655440000",
  "status": "pending",
  "priority": 5,
  "agent_type": "requirements-gatherer",
  "description": "Implement user authentication feature",
  "created_at": "2025-10-29T14:30:00Z",
  "updated_at": "2025-10-29T14:30:00Z",
  "dependencies": [],
  "dependents": ["7f8e9d0c-e29b-41d4-a716-446655440002"]
}
```

#### Exit Codes

- `0`: Success
- `1`: Task not found
- `3`: Database error

---

### `task update`

Update one or more tasks.

#### Syntax

```bash
abathur task update <task_ids> [options]
```

#### Required Arguments

- `<task_ids>`: Task ID(s) to update (comma-separated, full UUIDs or prefixes)

#### Options

| Option | Short | Type | Default | Description |
|--------|-------|------|---------|-------------|
| `--status` | `-s` | string | - | Update task status |
| `--priority` | `-p` | integer (0-10) | - | Update base priority |
| `--agent-type` | `-a` | string | - | Update agent type |
| `--add-dependency` | | list | - | Add dependencies (comma-separated UUIDs or prefixes) |
| `--remove-dependency` | | list | - | Remove dependencies (comma-separated UUIDs or prefixes) |
| `--retry` | | flag | `false` | Increment retry count and reset to pending (for failed tasks) |
| `--cancel` | | flag | `false` | Cancel task and cascade to dependents |

#### Examples

**Update task status**:
```bash
abathur task update 550e8400 --status ready
```

**Update priority**:
```bash
abathur task update 550e8400 --priority 9
```

**Change agent type**:
```bash
abathur task update 550e8400 --agent-type rust-specialist
```

**Add dependencies**:
```bash
abathur task update 7f8e9d0c --add-dependency 550e8400,a3b4c5d6
```

**Remove dependencies**:
```bash
abathur task update 7f8e9d0c --remove-dependency 550e8400
```

**Retry failed task**:
```bash
abathur task update 550e8400 --retry
```

**Cancel task and dependents**:
```bash
abathur task update 550e8400 --cancel
```

**Update multiple tasks**:
```bash
abathur task update 550e8400,a3b4c5d6 --priority 8
```

#### Exit Codes

- `0`: Success
- `1`: Invalid arguments or task not found
- `2`: Update failed
- `3`: Database error

---

### `task status`

Show queue status and statistics.

#### Syntax

```bash
abathur task status
```

#### Examples

**Show queue status**:
```bash
abathur task status
```

**Output**:
```
Task Queue Status

Total Tasks: 15
  Pending:   3
  Ready:     2
  Running:   1
  Completed: 7
  Failed:    1
  Cancelled: 1
  Blocked:   0

Active Agents: 1/10
```

**JSON output**:
```bash
abathur --json task status
```

**Output**:
```json
{
  "total_tasks": 15,
  "by_status": {
    "pending": 3,
    "ready": 2,
    "running": 1,
    "completed": 7,
    "failed": 1,
    "cancelled": 1,
    "blocked": 0
  },
  "active_agents": 1,
  "max_agents": 10
}
```

#### Exit Codes

- `0`: Success
- `3`: Database error

---

### `task resolve`

Resolve task dependencies and update statuses.

#### Syntax

```bash
abathur task resolve
```

#### Description

Checks all Pending/Blocked tasks and updates them to Ready if their dependencies are satisfied. This is useful after completing tasks to automatically unblock dependent tasks.

#### Examples

**Resolve dependencies**:
```bash
abathur task resolve
```

**Output**:
```
Resolving task dependencies...

Updated 3 tasks:
  550e8400: blocked -> ready
  a3b4c5d6: pending -> ready
  7f8e9d0c: blocked -> ready
```

**JSON output**:
```bash
abathur --json task resolve
```

**Output**:
```json
{
  "updated_tasks": [
    {
      "id": "550e8400-e29b-41d4-a716-446655440000",
      "old_status": "blocked",
      "new_status": "ready"
    }
  ]
}
```

#### Exit Codes

- `0`: Success
- `2`: Resolution failed
- `3`: Database error

---

## Memory Commands

Manage memories in the memory system.

### `memory list`

List memories in the system.

#### Syntax

```bash
abathur memory list [options]
```

#### Options

| Option | Short | Type | Default | Description |
|--------|-------|------|---------|-------------|
| `--namespace` | `-n` | string | - | Filter by namespace prefix |
| `--memory-type` | `-t` | string | - | Filter by memory type (semantic, episodic, procedural) |
| `--limit` | `-l` | integer | `50` | Maximum number of memories to display |

#### Examples

**List all memories**:
```bash
abathur memory list
```

**Output**:
```
Namespace                           Key               Type        Updated
task:550e8400:technical_specs       requirements      semantic    2025-10-29 14:30:00
task:550e8400:technical_specs       architecture      semantic    2025-10-29 14:31:00
agent:rust-specialist:experience    bug_fixes         episodic    2025-10-29 14:32:00
```

**Filter by namespace**:
```bash
abathur memory list --namespace task:550e8400
```

**Filter by type**:
```bash
abathur memory list --memory-type semantic
```

**Combined filters with limit**:
```bash
abathur memory list --namespace agent: --memory-type episodic --limit 10
```

**JSON output**:
```bash
abathur --json memory list --namespace task:550e8400
```

**Output**:
```json
{
  "memories": [
    {
      "namespace": "task:550e8400:technical_specs",
      "key": "requirements",
      "memory_type": "semantic",
      "updated_at": "2025-10-29T14:30:00Z"
    }
  ]
}
```

#### Exit Codes

- `0`: Success
- `3`: Database error

---

### `memory show`

Show details for a specific memory.

#### Syntax

```bash
abathur memory show <namespace> <key>
```

#### Required Arguments

- `<namespace>`: Memory namespace
- `<key>`: Memory key

#### Examples

**Show memory details**:
```bash
abathur memory show task:550e8400:technical_specs requirements
```

**Output**:
```
Namespace: task:550e8400:technical_specs
Key: requirements
Type: semantic
Created: 2025-10-29 14:30:00 UTC
Updated: 2025-10-29 14:30:00 UTC
Created By: requirements-gatherer

Value:
{
  "functional": [
    "User authentication",
    "Session management",
    "Role-based access control"
  ],
  "non_functional": [
    "Response time < 200ms",
    "99.9% uptime"
  ]
}
```

**JSON output**:
```bash
abathur --json memory show task:550e8400:technical_specs requirements
```

**Output**:
```json
{
  "namespace": "task:550e8400:technical_specs",
  "key": "requirements",
  "memory_type": "semantic",
  "value": {
    "functional": ["User authentication", "Session management"],
    "non_functional": ["Response time < 200ms"]
  },
  "created_at": "2025-10-29T14:30:00Z",
  "updated_at": "2025-10-29T14:30:00Z",
  "created_by": "requirements-gatherer"
}
```

#### Exit Codes

- `0`: Success
- `1`: Memory not found
- `3`: Database error

---

### `memory count`

Count memories matching criteria.

#### Syntax

```bash
abathur memory count [options]
```

#### Options

| Option | Short | Type | Default | Description |
|--------|-------|------|---------|-------------|
| `--namespace` | `-n` | string | `""` | Namespace prefix to count |
| `--memory-type` | `-t` | string | - | Filter by memory type (semantic, episodic, procedural) |

#### Examples

**Count all memories**:
```bash
abathur memory count
```

**Output**:
```
Total memories: 42
```

**Count by namespace**:
```bash
abathur memory count --namespace task:550e8400
```

**Output**:
```
Total memories in namespace 'task:550e8400': 8
```

**Count by type**:
```bash
abathur memory count --memory-type semantic
```

**Output**:
```
Total semantic memories: 15
```

**JSON output**:
```bash
abathur --json memory count --namespace task:
```

**Output**:
```json
{
  "namespace": "task:",
  "count": 35
}
```

#### Exit Codes

- `0`: Success
- `3`: Database error

---

## Swarm Commands

Manage the swarm orchestrator.

### `swarm start`

Start the swarm orchestrator.

#### Syntax

```bash
abathur swarm start [options]
```

#### Options

| Option | Short | Type | Default | Description |
|--------|-------|------|---------|-------------|
| `--max-agents` | `-m` | integer | `10` | Maximum number of concurrent agents |

#### Examples

**Start swarm with default settings**:
```bash
abathur swarm start
```

**Output**:
```
Starting swarm orchestrator...
Max concurrent agents: 10
Status: running

Processing tasks...
```

**Start with custom agent limit**:
```bash
abathur swarm start --max-agents 5
```

**JSON output**:
```bash
abathur --json swarm start
```

**Output**:
```json
{
  "status": "running",
  "max_agents": 10,
  "active_agents": 0
}
```

#### Exit Codes

- `0`: Success (orchestrator stopped normally)
- `1`: Failed to start
- `2`: Already running

---

### `swarm stop`

Stop the swarm orchestrator.

#### Syntax

```bash
abathur swarm stop
```

#### Examples

**Stop swarm**:
```bash
abathur swarm stop
```

**Output**:
```
Stopping swarm orchestrator...
Status: stopped
```

**JSON output**:
```bash
abathur --json swarm stop
```

**Output**:
```json
{
  "status": "stopped",
  "message": "Swarm orchestrator stopped successfully"
}
```

#### Exit Codes

- `0`: Success
- `1`: Not running
- `2`: Failed to stop

---

### `swarm status`

Show swarm orchestrator status.

#### Syntax

```bash
abathur swarm status
```

#### Examples

**Check swarm status**:
```bash
abathur swarm status
```

**Output**:
```
Swarm Orchestrator Status

Status: running
Active Agents: 3/10
Tasks in Queue: 12
  Ready: 5
  Running: 3
  Pending: 4
```

**JSON output**:
```bash
abathur --json swarm status
```

**Output**:
```json
{
  "status": "running",
  "active_agents": 3,
  "max_agents": 10,
  "queue_stats": {
    "total": 12,
    "ready": 5,
    "running": 3,
    "pending": 4
  }
}
```

#### Exit Codes

- `0`: Success
- `3`: Database error

---

## MCP Commands

Model Context Protocol server commands (for internal use).

!!! warning "Internal Use Only"
    These commands are primarily for internal use by the Claude Code MCP integration. Most users will not need to run these directly.

### `mcp memory-http`

Run HTTP MCP server for memory management.

#### Syntax

```bash
abathur mcp memory-http [options]
```

#### Options

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `--db-path` | string | `.abathur/abathur.db` | Path to SQLite database file |
| `--port` | integer | `45678` | Port to listen on |

#### Examples

**Start memory MCP server**:
```bash
abathur mcp memory-http
```

**Custom database and port**:
```bash
abathur mcp memory-http --db-path /var/db/abathur.db --port 8080
```

#### Exit Codes

- `0`: Server stopped normally
- `1`: Failed to start
- `2`: Port already in use

---

### `mcp tasks-http`

Run HTTP MCP server for task queue management.

#### Syntax

```bash
abathur mcp tasks-http [options]
```

#### Options

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `--db-path` | string | `.abathur/abathur.db` | Path to SQLite database file |
| `--port` | integer | `45679` | Port to listen on |

#### Examples

**Start task MCP server**:
```bash
abathur mcp tasks-http
```

**Custom database and port**:
```bash
abathur mcp tasks-http --db-path /var/db/abathur.db --port 8081
```

#### Exit Codes

- `0`: Server stopped normally
- `1`: Failed to start
- `2`: Port already in use

---

## Common Patterns

### Task Workflow

**1. Submit a task**:
```bash
abathur task submit "Implement feature X" --priority 7
```

**2. Check queue status**:
```bash
abathur task status
```

**3. View task details**:
```bash
abathur task show <task-id>
```

**4. Start swarm to process tasks**:
```bash
abathur swarm start
```

**5. Monitor progress**:
```bash
abathur task list --status running
```

### Dependency Management

**Create dependent tasks**:
```bash
# Create parent task
abathur task submit "Design API" --summary "API Design"

# Create dependent task (use ID from previous command)
abathur task submit "Implement API" --dependencies <parent-id>
```

**Resolve dependencies after completion**:
```bash
abathur task resolve
```

### Memory Inspection

**View task-specific memories**:
```bash
abathur memory list --namespace task:<task-id>
```

**Examine technical specifications**:
```bash
abathur memory show task:<task-id>:technical_specs requirements
```

### JSON Integration

**Parse task list in scripts**:
```bash
abathur --json task list | jq '.tasks[] | select(.status=="ready")'
```

**Automated task submission**:
```bash
for desc in "Task 1" "Task 2" "Task 3"; do
  abathur --json task submit "$desc" | jq -r '.task_id'
done
```

---

## Environment Variables

| Variable | Description | Default |
|----------|-------------|---------|
| `ABATHUR_CONFIG` | Path to configuration file | `.abathur/config.yaml` |
| `ABATHUR_DB` | Path to SQLite database | `.abathur/abathur.db` |
| `RUST_LOG` | Logging level (error, warn, info, debug, trace) | `info` |

---

## Exit Codes Summary

| Code | Meaning |
|------|---------|
| `0` | Success |
| `1` | General error (invalid arguments, not found, etc.) |
| `2` | Operation failed (creation, update, start, etc.) |
| `3` | Database error |

---

## Related Documentation

- [Getting Started Guide](../getting-started/quickstart.md)
- [How-To: Task Management](../how-to/task-management.md)
- [Configuration Reference](configuration.md)
- [Understanding Task Queue](../explanation/task-queue.md)
- [Understanding Memory System](../explanation/memory-system.md)
