# Abathur MCP Server Setup

This template includes configuration for two Abathur MCP servers implemented in Rust:

## Memory MCP Server

Gives Claude agents access to:
- **Long-term memory** - Store and retrieve facts, preferences, and learnings

## Task Queue MCP Server

Gives Claude agents access to:
- **Task enqueueing** - Submit tasks with dependencies and priorities
- **Task monitoring** - List, filter, and track task progress
- **Dependency management** - Define task prerequisites and execution order
- **Queue statistics** - Monitor overall queue health and performance

## Quick Start

### 1. Build the MCP Servers

The MCP servers are implemented in Rust for performance and reliability:

```bash
cargo build --release --bin abathur-mcp-memory --bin abathur-mcp-tasks
```

This creates two binaries:
- `target/release/abathur-mcp-memory` - Memory management server
- `target/release/abathur-mcp-tasks` - Task queue management server

### 2. Project Configuration

The project's `.mcp.json` file is already configured to use the Rust binaries:

```json
{
  "mcpServers": {
    "abathur-memory": {
      "command": "./target/release/abathur-mcp-memory",
      "args": ["--db-path", ".abathur/abathur.db"]
    },
    "abathur-task-queue": {
      "command": "./target/release/abathur-mcp-tasks",
      "args": ["--db-path", ".abathur/abathur.db"]
    }
  }
}
```

Claude Code automatically picks up this configuration when running in the project directory.

### 3. Manual Testing (Optional)

Test the servers directly:

```bash
# Memory server (runs in foreground, expects stdio MCP protocol)
./target/release/abathur-mcp-memory --db-path .abathur/abathur.db

# Task queue server (runs in foreground, expects stdio MCP protocol)
./target/release/abathur-mcp-tasks --db-path .abathur/abathur.db
```

**Note:** The servers communicate via stdio using the MCP protocol. They will wait for JSON-RPC messages on stdin and respond on stdout. Claude Code handles this automatically.

## Available Tools

### Memory Operations

- `memory_add` - Add new memory entry to long-term storage
- `memory_get` - Retrieve memory by namespace and key
- `memory_search` - Search memories by namespace prefix and type
- `memory_update` - Update memory (creates new version)
- `memory_delete` - Soft-delete memory entry

### Session Operations

- `session_create` - Create new conversation session
- `session_get` - Retrieve session by ID
- `session_append_event` - Append event to session history
- `session_get_state` - Get specific state value
- `session_set_state` - Set specific state value

### Document Operations

- `document_semantic_search` - Search documents using natural language
- `document_index` - Index document for semantic search

### Task Queue Operations

- `task_enqueue` - Submit new task with dependencies and priorities
- `task_list` - List and filter tasks by status, source, or agent
- `task_get` - Retrieve specific task details by ID
- `task_queue_status` - Get overall queue statistics
- `task_cancel` - Cancel task with cascade to dependents
- `task_execution_plan` - Calculate execution order and dependencies

## Example Usage

### Agent Definition with Memory

Create an agent that uses memory tools in `.claude/agents/memory-aware-agent.md`:

```markdown
You are a memory-aware agent. You have access to tools for:
- Storing and retrieving long-term memories

Use these tools to:
1. Remember user preferences (memory_add with type='semantic')
2. Recall past conversations (memory_search)

Always check for existing memories before asking the user.
Store important learnings for future conversations.
```

### Memory Storage Pattern

Agents should use hierarchical namespaces:

- `user:<user_id>:preferences` - User-specific preferences
- `user:<user_id>:context` - User context and history
- `project:<project_id>:specs` - Project specifications
- `app:abathur:config` - Application configuration
- `session:<session_id>:state` - Session-specific state

### Example Tool Calls

**Store user preference:**
```json
{
  "tool": "memory_add",
  "arguments": {
    "namespace": "user:alice:preferences",
    "key": "coding_style",
    "value": {"language": "concise", "technical_level": "expert"},
    "memory_type": "semantic",
    "created_by": "agent:memory-aware-agent"
  }
}
```

**Search for memories:**
```json
{
  "tool": "memory_search",
  "arguments": {
    "namespace_prefix": "user:alice",
    "memory_type": "semantic",
    "limit": 10
  }
}
```

### Task Queue Pattern

Agents use the task queue for managing complex workflows:

**Submit a task:**
```json
{
  "tool": "task_enqueue",
  "arguments": {
    "summary": "Analyze requirements",
    "description": "Analyze requirements for new feature",
    "agent_type": "requirements-gatherer",
    "priority": 8,
    "dependencies": null
  }
}
```

**Submit a dependent task:**
```json
{
  "tool": "task_enqueue",
  "arguments": {
    "summary": "Implement feature",
    "description": "Implement feature based on analysis",
    "agent_type": "implementation-specialist",
    "priority": 7,
    "dependencies": ["task-uuid-from-previous-task"]
  }
}
```

**Monitor task progress:**
```json
{
  "tool": "task_list",
  "arguments": {
    "status": "ready",
    "source": "agent_planner",
    "limit": 50
  }
}
```

**Get queue statistics:**
```json
{
  "tool": "task_queue_status",
  "arguments": {}
}
```

## Architecture

```
┌─────────────────────────┐
│     Claude Code         │
└───────────┬─────────────┘
            │ MCP Protocol (stdio)
            │
┌───────────▼─────────────┐
│  Rust MCP Servers       │
│  - abathur-mcp-memory   │
│  - abathur-mcp-tasks    │
│  - Error Handling       │
│  - Structured Logging   │
└───────────┬─────────────┘
            │
┌───────────▼─────────────┐
│  Service Layer          │
│  - MemoryService        │
│  - TaskQueueService     │
└───────────┬─────────────┘
            │
┌───────────▼─────────────┐
│  SQLite Database        │
│  - memory_entries       │
│  - tasks                │
│  - task_dependencies    │
└─────────────────────────┘
```

## Troubleshooting

### Server Not Starting

1. Build the binaries: `cargo build --release --bin abathur-mcp-memory --bin abathur-mcp-tasks`
2. Verify binaries exist: `ls -la target/release/abathur-mcp-*`
3. Check logs: Set `RUST_LOG=debug` in `.mcp.json` and check stderr

### Tools Not Visible in Claude Code

1. Verify `.mcp.json` exists in project root
2. Check config syntax: `cat .mcp.json | jq`
3. Rebuild if binaries are missing: `cargo build --release`
4. Restart Claude Code session

### Database Errors

1. Verify database exists: `ls -la .abathur/abathur.db`
2. Check database permissions: `ls -l .abathur/abathur.db`
3. Reinitialize if needed: The servers will run migrations automatically on startup

## Security Notes

- **Database Access:** The MCP server has full read/write access to the database
- **Network Exposure:** Server uses stdio (local only), no network exposure
- **Memory Scope:** Tools can access all memories in the database
- **Audit Trail:** All operations are logged to the audit table

## Advanced Configuration

### Custom Database Path

Edit `.mcp.json` to use a custom database path:

```json
{
  "mcpServers": {
    "abathur-memory": {
      "command": "./target/release/abathur-mcp-memory",
      "args": ["--db-path", "/custom/path/abathur.db"]
    }
  }
}
```

### Environment Variables

Add environment variables to `.mcp.json`:

```json
{
  "mcpServers": {
    "abathur-memory": {
      "command": "./target/release/abathur-mcp-memory",
      "args": ["--db-path", ".abathur/abathur.db"],
      "env": {
        "RUST_LOG": "debug",
        "RUST_BACKTRACE": "1"
      }
    }
  }
}
```

## Next Steps

1. Build the MCP servers: `cargo build --release --bin abathur-mcp-memory --bin abathur-mcp-tasks`
2. The `.mcp.json` configuration is already set up
3. Claude Code will automatically connect to the servers
4. Use the MCP tools in your agents and workflows

## Resources

- [MCP Documentation](https://github.com/anthropics/mcp)
- [Abathur MCP Memory Server Source](/src/bin/abathur-mcp-memory.rs)
- [Abathur MCP Tasks Server Source](/src/bin/abathur-mcp-tasks.rs)
- [Example Agents](.claude/agents/)

---

**Abathur Swarm - Rust Implementation**
