"""Integration tests for MCP Memory Server."""

from collections.abc import AsyncGenerator
from pathlib import Path

import pytest
from abathur.infrastructure.database import Database
from abathur.mcp.memory_server import AbathurMemoryServer


@pytest.fixture
async def db() -> AsyncGenerator[Database, None]:
    """Create test database."""
    db = Database(Path(":memory:"))
    await db.initialize()
    yield db
    await db.close()


@pytest.fixture
async def mcp_server(db: Database) -> AbathurMemoryServer:
    """Create MCP server with test database."""
    server = AbathurMemoryServer(Path(":memory:"))
    server.db = db  # Use our test database
    return server


@pytest.mark.asyncio
async def test_server_initialization(mcp_server: AbathurMemoryServer) -> None:
    """Test that server initializes correctly."""
    assert mcp_server.server.name == "abathur-memory"
    assert mcp_server.db is not None


@pytest.mark.asyncio
async def test_memory_add_and_get(db: Database) -> None:
    """Test memory add and get operations through service layer."""
    # Add memory
    memory_id = await db.memory.add_memory(
        namespace="test:user:alice",
        key="preference",
        value={"theme": "dark"},
        memory_type="semantic",
        created_by="test_session",
    )

    assert memory_id > 0

    # Get memory
    memory = await db.memory.get_memory(namespace="test:user:alice", key="preference")

    assert memory is not None
    assert memory["namespace"] == "test:user:alice"
    assert memory["key"] == "preference"
    assert memory["value"] == {"theme": "dark"}
    assert memory["memory_type"] == "semantic"


@pytest.mark.asyncio
async def test_memory_search(db: Database) -> None:
    """Test memory search by namespace prefix."""
    # Add multiple memories
    await db.memory.add_memory(
        namespace="test:user:alice:prefs",
        key="theme",
        value={"mode": "dark"},
        memory_type="semantic",
        created_by="test",
    )

    await db.memory.add_memory(
        namespace="test:user:alice:settings",
        key="notifications",
        value={"enabled": True},
        memory_type="semantic",
        created_by="test",
    )

    await db.memory.add_memory(
        namespace="test:user:bob:prefs",
        key="theme",
        value={"mode": "light"},
        memory_type="semantic",
        created_by="test",
    )

    # Search for alice's memories
    results = await db.memory.search_memories(namespace_prefix="test:user:alice")

    assert len(results) == 2
    namespaces = [r["namespace"] for r in results]
    assert "test:user:alice:prefs" in namespaces
    assert "test:user:alice:settings" in namespaces


@pytest.mark.asyncio
async def test_session_create_and_get(db: Database) -> None:
    """Test session create and get operations."""
    # Create session
    await db.sessions.create_session(
        session_id="test_session_123",
        app_name="test_app",
        user_id="alice",
        initial_state={"test_key": "test_value"},
    )

    # Get session
    session = await db.sessions.get_session("test_session_123")

    assert session is not None
    assert session["id"] == "test_session_123"
    assert session["app_name"] == "test_app"
    assert session["user_id"] == "alice"
    assert session["state"] == {"test_key": "test_value"}


@pytest.mark.asyncio
async def test_session_append_event(db: Database) -> None:
    """Test appending events to session."""
    # Create session
    await db.sessions.create_session(
        session_id="test_session_456", app_name="test_app", user_id="alice"
    )

    # Append event
    event = {
        "event_id": "evt_001",
        "timestamp": "2025-10-10T10:00:00Z",
        "event_type": "message",
        "actor": "user",
        "content": {"message": "Hello"},
        "is_final_response": False,
    }

    await db.sessions.append_event(session_id="test_session_456", event=event)

    # Verify event was added
    session = await db.sessions.get_session("test_session_456")
    assert session is not None
    assert len(session["events"]) == 1
    assert session["events"][0]["event_id"] == "evt_001"


@pytest.mark.asyncio
async def test_session_state_operations(db: Database) -> None:
    """Test session state get/set operations."""
    # Create session
    await db.sessions.create_session(
        session_id="test_session_789",
        app_name="test_app",
        user_id="alice",
        initial_state={"key1": "value1"},
    )

    # Get state value
    value = await db.sessions.get_state("test_session_789", "key1")
    assert value == "value1"

    # Set state value
    await db.sessions.set_state("test_session_789", "key2", "value2")

    # Verify state was updated
    value = await db.sessions.get_state("test_session_789", "key2")
    assert value == "value2"


@pytest.mark.asyncio
async def test_memory_update_creates_version(db: Database) -> None:
    """Test that memory update creates new version."""
    # Add initial memory
    await db.memory.add_memory(
        namespace="test:versioning",
        key="counter",
        value={"count": 1},
        memory_type="semantic",
        created_by="test",
    )

    # Update memory
    _memory_id = await db.memory.update_memory(
        namespace="test:versioning",
        key="counter",
        value={"count": 2},
        updated_by="test",
    )

    # Verify new version exists
    memory = await db.memory.get_memory(namespace="test:versioning", key="counter")
    assert memory is not None
    assert memory["value"] == {"count": 2}
    assert memory["version"] == 2

    # Verify old version still exists
    old_memory = await db.memory.get_memory(namespace="test:versioning", key="counter", version=1)
    assert old_memory is not None
    assert old_memory["value"] == {"count": 1}
    assert old_memory["version"] == 1


@pytest.mark.asyncio
async def test_memory_delete(db: Database) -> None:
    """Test soft-delete of memory."""
    # Add memory
    await db.memory.add_memory(
        namespace="test:delete",
        key="temp",
        value={"data": "test"},
        memory_type="semantic",
        created_by="test",
    )

    # Delete memory
    deleted = await db.memory.delete_memory(namespace="test:delete", key="temp")
    assert deleted is True

    # Verify memory is not returned
    memory = await db.memory.get_memory(namespace="test:delete", key="temp")
    assert memory is None


if __name__ == "__main__":
    pytest.main([__file__, "-v"])
