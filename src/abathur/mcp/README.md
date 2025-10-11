# Abathur MCP Memory Server

MCP (Model Context Protocol) server exposing Abathur's memory management system to Claude agents.

## Overview

The Abathur MCP Memory Server provides standardized access to:
- **Long-term memory** (semantic, episodic, procedural)
- **Session management** (conversation tracking with events and state)
- **Document search** (semantic search with vector embeddings)

## Installation

1. Install the MCP SDK:
```bash
pip install mcp
```

2. Install Abathur with MCP support:
```bash
cd /Users/odgrim/dev/home/agentics/abathur
pip install -e .
```

## Running the Server

### Standalone Mode
```bash
python -m abathur.mcp.memory_server --db-path /path/to/abathur.db
```

### Using the CLI Entry Point
```bash
abathur-mcp --db-path /path/to/abathur.db
```

### Default Database Location
If no `--db-path` is specified, the server uses `./abathur.db` in the current directory.

## Available Tools

### Memory Operations

#### `memory_add`
Add a new memory entry to long-term storage.

**Parameters:**
- `namespace` (string, required): Hierarchical namespace (e.g., "user:alice:preferences")
- `key` (string, required): Unique key within namespace
- `value` (object, required): Memory content as JSON object
- `memory_type` (string, required): Type of memory (semantic|episodic|procedural)
- `created_by` (string, required): Session or agent ID creating this memory
- `task_id` (string, optional): Task ID for audit logging
- `metadata` (object, optional): Optional metadata

**Returns:**
```json
{"memory_id": 123, "status": "created"}
```

#### `memory_get`
Retrieve a memory entry by namespace and key.

**Parameters:**
- `namespace` (string, required): Memory namespace
- `key` (string, required): Memory key
- `version` (integer, optional): Specific version (defaults to latest)

**Returns:**
```json
{
  "id": 123,
  "namespace": "user:alice:preferences",
  "key": "theme",
  "value": {"mode": "dark"},
  "memory_type": "semantic",
  "version": 1,
  "created_at": "2025-10-10T10:00:00Z",
  "updated_at": "2025-10-10T10:00:00Z"
}
```

#### `memory_search`
Search memories by namespace prefix and type.

**Parameters:**
- `namespace_prefix` (string, required): Namespace prefix (e.g., "user:alice")
- `memory_type` (string, optional): Filter by type (semantic|episodic|procedural)
- `limit` (integer, optional): Maximum results (default: 50)

**Returns:**
```json
[
  {
    "id": 123,
    "namespace": "user:alice:preferences",
    "key": "theme",
    "value": {"mode": "dark"},
    "memory_type": "semantic",
    "version": 1
  }
]
```

#### `memory_update`
Update a memory entry (creates new version).

**Parameters:**
- `namespace` (string, required): Memory namespace
- `key` (string, required): Memory key
- `value` (object, required): New memory content
- `updated_by` (string, required): Session or agent ID making the update
- `task_id` (string, optional): Task ID for audit logging

**Returns:**
```json
{"memory_id": 124, "status": "updated"}
```

#### `memory_delete`
Soft-delete a memory entry.

**Parameters:**
- `namespace` (string, required): Memory namespace
- `key` (string, required): Memory key
- `task_id` (string, optional): Task ID for audit logging

**Returns:**
```json
{"deleted": true}
```

### Session Operations

#### `session_create`
Create a new conversation session.

**Parameters:**
- `session_id` (string, required): Unique session identifier
- `app_name` (string, required): Application context (e.g., "abathur")
- `user_id` (string, required): User identifier
- `project_id` (string, optional): Project association for cross-agent collaboration
- `initial_state` (object, optional): Initial state dictionary

**Returns:**
```json
{"session_id": "abc123", "status": "created"}
```

#### `session_get`
Retrieve session by ID.

**Parameters:**
- `session_id` (string, required): Session identifier

**Returns:**
```json
{
  "id": "abc123",
  "app_name": "abathur",
  "user_id": "alice",
  "project_id": "schema_redesign",
  "status": "active",
  "events": [...],
  "state": {...},
  "created_at": "2025-10-10T10:00:00Z"
}
```

#### `session_append_event`
Append event to session history.

**Parameters:**
- `session_id` (string, required): Session identifier
- `event` (object, required): Event with event_id, timestamp, event_type, actor, content
- `state_delta` (object, optional): Optional state changes to merge

**Returns:**
```json
{"status": "event_appended"}
```

#### `session_get_state`
Get specific state value from session.

**Parameters:**
- `session_id` (string, required): Session identifier
- `key` (string, required): State key

**Returns:**
```json
{"value": "dark"}
```

#### `session_set_state`
Set specific state value in session.

**Parameters:**
- `session_id` (string, required): Session identifier
- `key` (string, required): State key
- `value` (any): State value (any JSON type)

**Returns:**
```json
{"status": "state_updated"}
```

### Document Search Operations

#### `document_semantic_search`
Search documents using natural language semantic search.

**Parameters:**
- `query_text` (string, required): Natural language search query
- `namespace` (string, optional): Optional namespace filter
- `limit` (integer, optional): Maximum results (default: 10)

**Returns:**
```json
[
  {
    "document_id": 1,
    "namespace": "docs:architecture",
    "file_path": "/docs/memory-architecture.md",
    "distance": 0.42,
    "created_at": "2025-10-10T10:00:00Z"
  }
]
```

#### `document_index`
Index a document for semantic search.

**Parameters:**
- `namespace` (string, required): Document namespace
- `file_path` (string, required): Document file path
- `content` (string, required): Document content

**Returns:**
```json
{
  "document_id": 1,
  "embedding_rowid": 1
}
```

## Testing with MCP Inspector

Use the MCP Inspector tool to test the server interactively:

```bash
npx @modelcontextprotocol/inspector python -m abathur.mcp.memory_server
```

This opens a web UI where you can:
- Browse available tools
- Test tool calls with sample inputs
- View responses in real-time

## Claude Desktop Configuration

To use the Abathur MCP server with Claude Desktop, add this to your Claude Desktop config:

**macOS:** `~/Library/Application Support/Claude/claude_desktop_config.json`

```json
{
  "mcpServers": {
    "abathur-memory": {
      "command": "python",
      "args": [
        "-m",
        "abathur.mcp.memory_server",
        "--db-path",
        "/path/to/your/abathur.db"
      ]
    }
  }
}
```

## Architecture

The MCP server acts as a bridge between Claude agents and Abathur's memory infrastructure:

```
┌─────────────────┐
│  Claude Agent   │
└────────┬────────┘
         │ MCP Protocol (stdio)
         │
┌────────▼────────┐
│  MCP Server     │
│  (memory_server)│
└────────┬────────┘
         │
┌────────▼────────┐
│  Database       │
│  (MemoryService,│
│   SessionService,│
│   DocumentIndex)│
└─────────────────┘
```

## Error Handling

All tools return JSON responses. Errors are returned in this format:

```json
{
  "error": "Error message here",
  "tool": "tool_name"
}
```

Common errors:
- `"Memory not found"`: Memory entry doesn't exist
- `"Session not found"`: Session doesn't exist
- `"Invalid memory_type"`: Must be semantic|episodic|procedural
- `"Database not initialized"`: Server startup failed

## Logging

The server uses structured logging via `structlog`. All tool calls are logged with:
- Tool name
- Arguments
- Success/failure status
- Error details (if applicable)

To enable debug logging:
```python
from abathur.infrastructure.logger import setup_logging
setup_logging(log_level="DEBUG")
```

## Development

### Running Tests
```bash
pytest tests/mcp/
```

### Type Checking
```bash
mypy src/abathur/mcp/
```

### Code Formatting
```bash
black src/abathur/mcp/
ruff check src/abathur/mcp/
```

## License

MIT License - See main Abathur LICENSE file.
