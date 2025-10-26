"""MCP server exposing Abathur memory management tools."""

import asyncio
import json
import sys
from pathlib import Path
from typing import Any

# Add MCP SDK - will be installed via: pip install mcp
try:
    from mcp.server import Server
    from mcp.server.stdio import stdio_server
    from mcp.types import TextContent, Tool
except ImportError:
    print("ERROR: mcp package not installed. Run: pip install mcp", file=sys.stderr)
    sys.exit(1)

# Import abathur modules using package-relative imports
from ..infrastructure.database import Database
from ..infrastructure.logger import get_logger

logger = get_logger(__name__)


class AbathurMemoryServer:
    """MCP server for Abathur memory management.

    Exposes tools for:
    - Memory operations (add, get, search, update, delete)
    - Session operations (create, get, append_event, get_state, set_state)
    - Document search (semantic_search, index_document)
    """

    def __init__(self, db_path: Path) -> None:
        """Initialize memory server.

        Args:
            db_path: Path to SQLite database
        """
        self.db_path = db_path
        self.db: Database | None = None
        self.server = Server("abathur-memory")

        # Register tools
        self._register_tools()

    def _register_tools(self) -> None:
        """Register all MCP tools."""

        # Memory tools
        @self.server.list_tools()  # type: ignore[no-untyped-call, unused-ignore]
        async def list_tools() -> list[Tool]:
            """List available tools."""
            return [
                # Memory operations
                Tool(
                    name="memory_add",
                    description="Add a new memory entry to long-term storage",
                    inputSchema={
                        "type": "object",
                        "properties": {
                            "namespace": {
                                "type": "string",
                                "description": "Hierarchical namespace (e.g., 'user:alice:preferences')",
                            },
                            "key": {
                                "type": "string",
                                "description": "Unique key within namespace",
                            },
                            "value": {
                                "type": "object",
                                "description": "Memory content as JSON object",
                            },
                            "memory_type": {
                                "type": "string",
                                "enum": ["semantic", "episodic", "procedural"],
                                "description": "Type of memory",
                            },
                            "created_by": {
                                "type": "string",
                                "description": "Session or agent ID creating this memory",
                            },
                            "task_id": {
                                "type": "string",
                                "description": "Optional task ID for audit logging",
                            },
                            "metadata": {
                                "type": "object",
                                "description": "Optional metadata",
                            },
                        },
                        "required": ["namespace", "key", "value", "memory_type", "created_by"],
                    },
                ),
                Tool(
                    name="memory_get",
                    description="Retrieve a memory entry by namespace and key",
                    inputSchema={
                        "type": "object",
                        "properties": {
                            "namespace": {"type": "string"},
                            "key": {"type": "string"},
                            "version": {
                                "type": "integer",
                                "description": "Optional specific version (defaults to latest)",
                            },
                        },
                        "required": ["namespace", "key"],
                    },
                ),
                Tool(
                    name="memory_search",
                    description="Search memories by namespace prefix and type",
                    inputSchema={
                        "type": "object",
                        "properties": {
                            "namespace_prefix": {
                                "type": "string",
                                "description": "Namespace prefix (e.g., 'user:alice' matches 'user:alice:*')",
                            },
                            "memory_type": {
                                "type": "string",
                                "enum": ["semantic", "episodic", "procedural"],
                                "description": "Optional filter by type",
                            },
                            "limit": {
                                "type": "integer",
                                "default": 50,
                                "description": "Maximum results",
                            },
                        },
                        "required": ["namespace_prefix"],
                    },
                ),
                Tool(
                    name="memory_update",
                    description="Update a memory entry (creates new version)",
                    inputSchema={
                        "type": "object",
                        "properties": {
                            "namespace": {"type": "string"},
                            "key": {"type": "string"},
                            "value": {"type": "object"},
                            "updated_by": {"type": "string"},
                            "task_id": {
                                "type": "string",
                                "description": "Optional task ID for audit logging",
                            },
                        },
                        "required": ["namespace", "key", "value", "updated_by"],
                    },
                ),
                Tool(
                    name="memory_delete",
                    description="Soft-delete a memory entry",
                    inputSchema={
                        "type": "object",
                        "properties": {
                            "namespace": {"type": "string"},
                            "key": {"type": "string"},
                            "task_id": {
                                "type": "string",
                                "description": "Optional task ID for audit logging",
                            },
                        },
                        "required": ["namespace", "key"],
                    },
                ),
                # Session operations
                Tool(
                    name="session_create",
                    description="Create a new conversation session",
                    inputSchema={
                        "type": "object",
                        "properties": {
                            "session_id": {"type": "string"},
                            "app_name": {"type": "string"},
                            "user_id": {"type": "string"},
                            "project_id": {"type": "string"},
                            "initial_state": {"type": "object"},
                        },
                        "required": ["session_id", "app_name", "user_id"],
                    },
                ),
                Tool(
                    name="session_get",
                    description="Retrieve session by ID",
                    inputSchema={
                        "type": "object",
                        "properties": {"session_id": {"type": "string"}},
                        "required": ["session_id"],
                    },
                ),
                Tool(
                    name="session_append_event",
                    description="Append event to session history",
                    inputSchema={
                        "type": "object",
                        "properties": {
                            "session_id": {"type": "string"},
                            "event": {
                                "type": "object",
                                "description": "Event with event_id, timestamp, event_type, actor, content",
                            },
                            "state_delta": {
                                "type": "object",
                                "description": "Optional state changes to merge",
                            },
                        },
                        "required": ["session_id", "event"],
                    },
                ),
                Tool(
                    name="session_get_state",
                    description="Get specific state value from session",
                    inputSchema={
                        "type": "object",
                        "properties": {
                            "session_id": {"type": "string"},
                            "key": {"type": "string"},
                        },
                        "required": ["session_id", "key"],
                    },
                ),
                Tool(
                    name="session_set_state",
                    description="Set specific state value in session",
                    inputSchema={
                        "type": "object",
                        "properties": {
                            "session_id": {"type": "string"},
                            "key": {"type": "string"},
                            "value": {"description": "State value (any JSON type)"},
                        },
                        "required": ["session_id", "key", "value"],
                    },
                ),
                # Document search
                Tool(
                    name="document_semantic_search",
                    description="Search documents using natural language semantic search",
                    inputSchema={
                        "type": "object",
                        "properties": {
                            "query_text": {
                                "type": "string",
                                "description": "Natural language search query",
                            },
                            "namespace": {
                                "type": "string",
                                "description": "Optional namespace filter",
                            },
                            "limit": {
                                "type": "integer",
                                "default": 10,
                                "description": "Maximum results",
                            },
                        },
                        "required": ["query_text"],
                    },
                ),
                Tool(
                    name="document_index",
                    description="Index a document for semantic search",
                    inputSchema={
                        "type": "object",
                        "properties": {
                            "namespace": {"type": "string"},
                            "file_path": {"type": "string"},
                            "content": {"type": "string"},
                        },
                        "required": ["namespace", "file_path", "content"],
                    },
                ),
            ]

        @self.server.call_tool()
        async def call_tool(name: str, arguments: dict[str, Any]) -> list[TextContent]:
            """Handle tool calls."""
            if self.db is None:
                return [TextContent(type="text", text="Error: Database not initialized")]

            try:
                # Memory operations
                if name == "memory_add":
                    memory_id = await self.db.memory.add_memory(
                        namespace=arguments["namespace"],
                        key=arguments["key"],
                        value=arguments["value"],
                        memory_type=arguments["memory_type"],
                        created_by=arguments["created_by"],
                        task_id=arguments.get("task_id"),
                        metadata=arguments.get("metadata"),
                    )
                    return [
                        TextContent(
                            type="text",
                            text=json.dumps({"memory_id": memory_id, "status": "created"}),
                        )
                    ]

                elif name == "memory_get":
                    memory = await self.db.memory.get_memory(
                        namespace=arguments["namespace"],
                        key=arguments["key"],
                        version=arguments.get("version"),
                    )
                    if memory is None:
                        return [
                            TextContent(type="text", text=json.dumps({"error": "Memory not found"}))
                        ]
                    return [TextContent(type="text", text=json.dumps(memory, default=str))]

                elif name == "memory_search":
                    memories = await self.db.memory.search_memories(
                        namespace_prefix=arguments["namespace_prefix"],
                        memory_type=arguments.get("memory_type"),
                        limit=arguments.get("limit", 50),
                    )
                    return [TextContent(type="text", text=json.dumps(memories, default=str))]

                elif name == "memory_update":
                    memory_id = await self.db.memory.update_memory(
                        namespace=arguments["namespace"],
                        key=arguments["key"],
                        value=arguments["value"],
                        updated_by=arguments["updated_by"],
                        task_id=arguments.get("task_id"),
                    )
                    return [
                        TextContent(
                            type="text",
                            text=json.dumps({"memory_id": memory_id, "status": "updated"}),
                        )
                    ]

                elif name == "memory_delete":
                    deleted = await self.db.memory.delete_memory(
                        namespace=arguments["namespace"],
                        key=arguments["key"],
                        task_id=arguments.get("task_id"),
                    )
                    return [TextContent(type="text", text=json.dumps({"deleted": deleted}))]

                # Session operations
                elif name == "session_create":
                    await self.db.sessions.create_session(
                        session_id=arguments["session_id"],
                        app_name=arguments["app_name"],
                        user_id=arguments["user_id"],
                        project_id=arguments.get("project_id"),
                        initial_state=arguments.get("initial_state"),
                    )
                    return [
                        TextContent(
                            type="text",
                            text=json.dumps(
                                {"session_id": arguments["session_id"], "status": "created"}
                            ),
                        )
                    ]

                elif name == "session_get":
                    session = await self.db.sessions.get_session(arguments["session_id"])
                    if session is None:
                        return [
                            TextContent(
                                type="text", text=json.dumps({"error": "Session not found"})
                            )
                        ]
                    return [TextContent(type="text", text=json.dumps(session, default=str))]

                elif name == "session_append_event":
                    await self.db.sessions.append_event(
                        session_id=arguments["session_id"],
                        event=arguments["event"],
                        state_delta=arguments.get("state_delta"),
                    )
                    return [TextContent(type="text", text=json.dumps({"status": "event_appended"}))]

                elif name == "session_get_state":
                    value = await self.db.sessions.get_state(
                        session_id=arguments["session_id"], key=arguments["key"]
                    )
                    return [TextContent(type="text", text=json.dumps({"value": value}))]

                elif name == "session_set_state":
                    await self.db.sessions.set_state(
                        session_id=arguments["session_id"],
                        key=arguments["key"],
                        value=arguments["value"],
                    )
                    return [TextContent(type="text", text=json.dumps({"status": "state_updated"}))]

                # Document operations
                elif name == "document_semantic_search":
                    results = await self.db.documents.semantic_search(
                        query_text=arguments["query_text"],
                        namespace=arguments.get("namespace"),
                        limit=arguments.get("limit", 10),
                    )
                    return [TextContent(type="text", text=json.dumps(results, default=str))]

                elif name == "document_index":
                    result = await self.db.documents.sync_document_to_vector_db(
                        namespace=arguments["namespace"],
                        file_path=arguments["file_path"],
                        content=arguments["content"],
                    )
                    return [TextContent(type="text", text=json.dumps(result))]

                else:
                    return [TextContent(type="text", text=f"Unknown tool: {name}")]

            except Exception as e:
                logger.error("mcp_tool_error", tool=name, error=str(e))
                return [TextContent(type="text", text=json.dumps({"error": str(e), "tool": name}))]

    async def run(self) -> None:
        """Run the MCP server."""
        # Initialize database
        self.db = Database(self.db_path)
        await self.db.initialize()

        logger.info("abathur_mcp_server_started", db_path=str(self.db_path))

        # Run stdio server
        async with stdio_server() as (read_stream, write_stream):
            await self.server.run(
                read_stream, write_stream, self.server.create_initialization_options()
            )


async def main() -> None:
    """Main entry point."""
    import argparse
    from ..infrastructure.logger import setup_logging

    # Setup logging to stderr (stdout reserved for MCP JSON-RPC protocol)
    setup_logging(log_level="INFO")

    parser = argparse.ArgumentParser(description="Abathur Memory MCP Server")
    parser.add_argument(
        "--db-path",
        type=Path,
        default=Path.cwd() / "abathur.db",
        help="Path to SQLite database (default: ./abathur.db)",
    )

    args = parser.parse_args()

    server = AbathurMemoryServer(args.db_path)
    await server.run()


if __name__ == "__main__":
    asyncio.run(main())
