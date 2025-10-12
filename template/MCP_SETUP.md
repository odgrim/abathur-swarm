# Abathur MCP Server Setup

This template includes configuration for two Abathur MCP servers:

## Memory MCP Server

Gives Claude agents access to:
- **Long-term memory** - Store and retrieve facts, preferences, and learnings
- **Session management** - Track conversation history and state
- **Semantic document search** - Find relevant documents using natural language

## Task Queue MCP Server

Gives Claude agents access to:
- **Task enqueueing** - Submit tasks with dependencies and priorities
- **Task monitoring** - List, filter, and track task progress
- **Dependency management** - Define task prerequisites and execution order
- **Queue statistics** - Monitor overall queue health and performance

## Quick Start

### 1. Configure Claude Desktop

Copy the MCP configuration to your Claude Desktop config:

**macOS/Linux:**
```bash
# Location: ~/Library/Application Support/Claude/claude_desktop_config.json
cat .claude/mcp_config.json
```

**Windows:**
```bash
# Location: %APPDATA%\Claude\claude_desktop_config.json
type .claude\mcp_config.json
```

Add the `abathur-memory` server to your `mcpServers` section.

### 2. Start the Servers

#### Option A: Auto-start with Swarm/Loop (Recommended)

Both servers start automatically when you run:

```bash
abathur swarm start
abathur loop start <task-id>
```

Use `--no-mcp` to disable auto-start if needed.

#### Option B: Standalone Servers

Start the servers manually:

**Memory Server:**
```bash
# Foreground (for testing)
abathur mcp start-memory --foreground

# Background
abathur mcp start-memory

# Check status
abathur mcp status-memory

# Stop
abathur mcp stop-memory
```

**Task Queue Server:**
```bash
# Foreground (for testing)
python -m abathur.mcp.task_queue_server --db-path ./abathur.db

# Background
python -m abathur.mcp.task_queue_server_manager start --db-path ./abathur.db

# Check status
python -m abathur.mcp.task_queue_server_manager status

# Stop
python -m abathur.mcp.task_queue_server_manager stop
```

#### Option C: Direct Python

**Memory Server:**
```bash
# Using Python module
python -m abathur.mcp.memory_server --db-path ./abathur.db

# Using CLI entry point (after installation)
abathur-mcp --db-path ./abathur.db
```

**Task Queue Server:**
```bash
# Using Python module
python -m abathur.mcp.task_queue_server --db-path ./abathur.db
```

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

Create an agent that uses memory tools in `.claude/agents/memory-aware-agent.yaml`:

```yaml
name: memory-aware-agent
description: Agent with long-term memory capabilities
system_prompt: |
  You are a memory-aware agent. You have access to tools for:
  - Storing and retrieving long-term memories
  - Managing conversation sessions
  - Searching documents semantically

  Use these tools to:
  1. Remember user preferences (memory_add with type='semantic')
  2. Recall past conversations (memory_search)
  3. Find relevant documentation (document_semantic_search)
  4. Track session state (session_get_state, session_set_state)

  Always check for existing memories before asking the user.
  Store important learnings for future conversations.

tools:
  - memory_add
  - memory_get
  - memory_search
  - session_get_state
  - session_set_state
  - document_semantic_search
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

**Semantic document search:**
```json
{
  "tool": "document_semantic_search",
  "arguments": {
    "query_text": "memory management patterns",
    "namespace": "docs:architecture",
    "limit": 5
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
    "description": "Analyze requirements for new feature",
    "source": "agent_planner",
    "agent_type": "requirements-specialist",
    "base_priority": 8,
    "prerequisites": [],
    "deadline": "2025-12-31T23:59:59Z"
  }
}
```

**Submit a dependent task:**
```json
{
  "tool": "task_enqueue",
  "arguments": {
    "description": "Implement feature based on analysis",
    "source": "agent_implementation",
    "agent_type": "implementation-specialist",
    "base_priority": 7,
    "prerequisites": ["task-uuid-from-previous-task"]
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
│   Claude Agent/Desktop  │
└───────────┬─────────────┘
            │ MCP Protocol (stdio)
            │
┌───────────▼─────────────┐
│  Abathur Memory Server  │
│  - 12 MCP Tools         │
│  - Error Handling       │
│  - Structured Logging   │
└───────────┬─────────────┘
            │
┌───────────▼─────────────┐
│  Service Layer          │
│  - MemoryService        │
│  - SessionService       │
│  - DocumentIndexService │
└───────────┬─────────────┘
            │
┌───────────▼─────────────┐
│  SQLite Database        │
│  - memory_entries       │
│  - sessions             │
│  - document_embeddings  │
└─────────────────────────┘
```

## Troubleshooting

### Server Not Starting

1. Check Python version: `python --version` (requires 3.10+)
2. Verify installation: `pip list | grep mcp`
3. Check logs: `tail -f ~/.abathur/logs/abathur.log`

### Tools Not Visible in Claude Desktop

1. Restart Claude Desktop
2. Check MCP config syntax: `cat ~/Library/Application\ Support/Claude/claude_desktop_config.json | jq`
3. Verify server is running: `abathur mcp status-memory`

### Database Errors

1. Check database path is correct
2. Ensure database is initialized: `abathur init`
3. Verify database permissions: `ls -l abathur.db`

## Security Notes

- **Database Access:** The MCP server has full read/write access to the database
- **Network Exposure:** Server uses stdio (local only), no network exposure
- **Memory Scope:** Tools can access all memories in the database
- **Audit Trail:** All operations are logged to the audit table

## Advanced Configuration

### Custom Database Path

```bash
abathur mcp start-memory --db-path /custom/path/abathur.db
```

### Multiple Projects

Use separate databases for different projects:

```json
{
  "mcpServers": {
    "abathur-project-a": {
      "command": "abathur-mcp",
      "args": ["--db-path", "/projects/a/abathur.db"]
    },
    "abathur-project-b": {
      "command": "abathur-mcp",
      "args": ["--db-path", "/projects/b/abathur.db"]
    }
  }
}
```

### Performance Tuning

For large databases (1000+ documents), consider:

1. **Index Optimization:** Ensure all indexes are created
2. **Embedding Cache:** Use consistent embedding model
3. **Query Limits:** Set reasonable limits (default: 50 for memory, 10 for docs)

## Next Steps

1. Configure Claude Desktop with the MCP server
2. Test with: `abathur mcp start-memory --foreground`
3. Create memory-aware agent definitions
4. Run swarm with memory: `abathur swarm start`
5. Review stored memories: Browse database or use `memory_search` tool

## Resources

- [MCP Documentation](https://github.com/anthropics/mcp)
- [Abathur MCP Server Source](/src/abathur/mcp/memory_server.py)
- [Example Agents](.claude/agents/)
- [Database Schema](/design_docs/phase2_tech_specs/ddl-memory-tables.sql)

---

**Generated by Abathur Template**
