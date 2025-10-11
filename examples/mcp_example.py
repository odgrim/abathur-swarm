"""Example usage of Abathur MCP Memory Server.

This demonstrates how the MCP server can be configured and used
with Claude Desktop or other MCP clients.
"""

import asyncio
import json
from pathlib import Path

from abathur.infrastructure.database import Database


async def example_memory_operations():
    """Demonstrate memory operations through the database service layer."""
    print("=== Abathur Memory Operations Example ===\n")

    # Initialize database
    db = Database(Path(":memory:"))
    await db.initialize()

    # 1. Add semantic memory (facts)
    print("1. Adding semantic memory (user preferences)...")
    memory_id = await db.memory.add_memory(
        namespace="user:alice:preferences",
        key="theme",
        value={"mode": "dark", "font_size": 14},
        memory_type="semantic",
        created_by="session:example_001",
    )
    print(f"   Created memory ID: {memory_id}\n")

    # 2. Get memory
    print("2. Retrieving memory...")
    memory = await db.memory.get_memory(namespace="user:alice:preferences", key="theme")
    print(f"   Memory: {json.dumps(memory, indent=2, default=str)}\n")

    # 3. Update memory (creates new version)
    print("3. Updating memory (creates version 2)...")
    new_id = await db.memory.update_memory(
        namespace="user:alice:preferences",
        key="theme",
        value={"mode": "light", "font_size": 16},
        updated_by="session:example_001",
    )
    print(f"   Updated memory ID: {new_id}\n")

    # 4. Search memories
    print("4. Searching memories by namespace prefix...")
    memories = await db.memory.search_memories(
        namespace_prefix="user:alice", memory_type="semantic", limit=10
    )
    print(f"   Found {len(memories)} memories")
    for mem in memories:
        print(f"   - {mem['namespace']}:{mem['key']} (v{mem['version']})")
    print()

    # Clean up
    await db.close()


async def example_session_operations():
    """Demonstrate session operations through the database service layer."""
    print("=== Abathur Session Operations Example ===\n")

    # Initialize database
    db = Database(Path(":memory:"))
    await db.initialize()

    # 1. Create session
    print("1. Creating conversation session...")
    await db.sessions.create_session(
        session_id="session_example_001",
        app_name="abathur",
        user_id="alice",
        project_id="schema_redesign",
        initial_state={"current_task": "design", "mode": "interactive"},
    )
    print("   Session created\n")

    # 2. Append events
    print("2. Appending events to session...")
    events = [
        {
            "event_id": "evt_001",
            "timestamp": "2025-10-10T10:00:00Z",
            "event_type": "message",
            "actor": "user",
            "content": {"message": "Design the memory schema"},
            "is_final_response": False,
        },
        {
            "event_id": "evt_002",
            "timestamp": "2025-10-10T10:01:00Z",
            "event_type": "message",
            "actor": "agent:schema_designer",
            "content": {"message": "I'll create a hierarchical memory structure..."},
            "is_final_response": True,
        },
    ]

    for event in events:
        await db.sessions.append_event(
            session_id="session_example_001",
            event=event,
            state_delta={"last_actor": event["actor"]},
        )
        print(f"   Added event: {event['event_id']} from {event['actor']}")
    print()

    # 3. Get session with events
    print("3. Retrieving session with full history...")
    session = await db.sessions.get_session("session_example_001")
    print(f"   Session ID: {session['id']}")
    print(f"   Status: {session['status']}")
    print(f"   Events: {len(session['events'])}")
    print(f"   State: {json.dumps(session['state'], indent=2)}\n")

    # 4. Session state operations
    print("4. Session state get/set operations...")
    await db.sessions.set_state("session_example_001", "progress", 50)
    progress = await db.sessions.get_state("session_example_001", "progress")
    print(f"   Progress: {progress}%\n")

    # Clean up
    await db.close()


async def example_mcp_configuration():
    """Show example MCP configuration for Claude Desktop."""
    print("=== Claude Desktop MCP Configuration ===\n")

    config = {
        "mcpServers": {
            "abathur-memory": {
                "command": "python",
                "args": [
                    "-m",
                    "abathur.mcp.memory_server",
                    "--db-path",
                    str(Path.home() / "abathur" / "memory.db"),
                ],
            }
        }
    }

    print("Add this to your Claude Desktop config:")
    print("Location: ~/Library/Application Support/Claude/claude_desktop_config.json\n")
    print(json.dumps(config, indent=2))
    print()


async def main():
    """Run all examples."""
    await example_memory_operations()
    print("\n" + "=" * 60 + "\n")

    await example_session_operations()
    print("\n" + "=" * 60 + "\n")

    await example_mcp_configuration()


if __name__ == "__main__":
    asyncio.run(main())
