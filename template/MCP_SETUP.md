# Abathur MCP Server Setup

This template includes configuration for two Abathur MCP servers implemented in Rust using HTTP transport:

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

The MCP servers are implemented in Rust with HTTP transport for concurrent access:

```bash
cargo build --release --bin abathur-mcp-memory-http --bin abathur-mcp-tasks-http
```

This creates two binaries:
- `target/release/abathur-mcp-memory-http` - Memory management server (HTTP)
- `target/release/abathur-mcp-tasks-http` - Task queue management server (HTTP)

### 2. Server Architecture

The servers use HTTP transport for concurrent access by multiple agents:

- **Memory Server**: Runs on port 45678
- **Task Queue Server**: Runs on port 45679

These servers are automatically started by the swarm orchestrator when you run `abathur swarm`.

### 3. Manual Testing (Optional)

Test the servers directly:

```bash
# Memory server (HTTP on port 45678)
./target/release/abathur-mcp-memory-http --db-path .abathur/abathur.db --port 45678

# Task queue server (HTTP on port 45679)
./target/release/abathur-mcp-tasks-http --db-path .abathur/abathur.db --port 45679
```

**Note:** The servers provide HTTP endpoints for MCP operations, allowing concurrent access from multiple agents without process/connection overhead.

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
│   Swarm Orchestrator    │
└───────────┬─────────────┘
            │ HTTP (concurrent)
            │
┌───────────▼─────────────┐
│  Rust MCP Servers       │
│  - memory-http :45678   │
│  - tasks-http :45679    │
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

1. Build the binaries: `cargo build --release --bin abathur-mcp-memory-http --bin abathur-mcp-tasks-http`
2. Verify binaries exist: `ls -la target/release/abathur-mcp-*-http`
3. Check server logs in the swarm orchestrator output
4. Verify ports 45678 and 45679 are not in use: `lsof -i :45678 -i :45679`

### Connection Issues

1. Ensure servers are running (started by swarm orchestrator)
2. Check that the HTTP servers are listening: `curl http://localhost:45678/health`
3. Rebuild if binaries are missing: `cargo build --release`
4. Check for port conflicts

### Database Errors

1. Verify database exists: `ls -la .abathur/abathur.db`
2. Check database permissions: `ls -l .abathur/abathur.db`
3. Reinitialize if needed: The servers will run migrations automatically on startup

## Security Notes

- **Database Access:** The MCP servers have full read/write access to the database
- **Network Exposure:** Servers listen on localhost only (127.0.0.1:45678, 127.0.0.1:45679)
- **Memory Scope:** Tools can access all memories in the database
- **Audit Trail:** All operations are logged to the audit table
- **Concurrent Access:** HTTP transport allows safe concurrent access from multiple agents

## Advanced Configuration

### Custom Ports and Database Path

The HTTP servers support custom configuration via command-line arguments:

```bash
# Custom database path
./target/release/abathur-mcp-memory-http --db-path /custom/path/abathur.db --port 45678

# Custom port
./target/release/abathur-mcp-memory-http --db-path .abathur/abathur.db --port 8080
```

### Environment Variables

Set environment variables for debugging:

```bash
RUST_LOG=debug ./target/release/abathur-mcp-memory-http --db-path .abathur/abathur.db --port 45678
```

## Next Steps

1. Build the MCP servers: `cargo build --release --bin abathur-mcp-memory-http --bin abathur-mcp-tasks-http`
2. Run the swarm orchestrator: `abathur swarm` (automatically starts HTTP servers)
3. The servers will be available on ports 45678 (memory) and 45679 (tasks)
4. Use the MCP tools in your agents and workflows

## Resources

- [MCP Documentation](https://github.com/anthropics/mcp)
- [Abathur MCP Memory Server Source](/src/bin/abathur-mcp-memory-http.rs)
- [Abathur MCP Tasks Server Source](/src/bin/abathur-mcp-tasks-http.rs)
- [Example Agents](.claude/agents/)

---

**Abathur Swarm - Rust Implementation**
