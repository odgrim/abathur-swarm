"""Unit tests for MemoryService."""

import pytest
from abathur.services import MemoryService


class TestMemoryService:
    """Test MemoryService CRUD operations and memory management."""

    @pytest.mark.asyncio
    async def test_add_memory_success(self, memory_service: MemoryService) -> None:
        """Test successful memory entry creation."""
        memory_id = await memory_service.add_memory(
            namespace="user:alice:preferences",
            key="theme",
            value={"mode": "dark"},
            memory_type="semantic",
            created_by="session:abc123",
            task_id="task:xyz789",
        )

        assert memory_id > 0, "Memory ID should be positive"

        # Verify memory was created
        memory = await memory_service.get_memory("user:alice:preferences", "theme")
        assert memory is not None
        assert memory["value"] == {"mode": "dark"}
        assert memory["version"] == 1
        assert memory["memory_type"] == "semantic"
        assert memory["is_deleted"] == 0

    @pytest.mark.asyncio
    async def test_add_memory_with_metadata(self, memory_service: MemoryService) -> None:
        """Test memory creation with metadata."""
        metadata = {"source": "user_input", "confidence": 0.95}

        _memory_id = await memory_service.add_memory(
            namespace="user:alice:preferences",
            key="language",
            value={"code": "python"},
            memory_type="semantic",
            created_by="session:abc",
            task_id="task:xyz",
            metadata=metadata,
        )

        memory = await memory_service.get_memory("user:alice:preferences", "language")
        assert memory is not None
        assert memory["metadata"] == metadata

    @pytest.mark.asyncio
    async def test_add_memory_invalid_type_raises_error(
        self, memory_service: MemoryService
    ) -> None:
        """Test that invalid memory_type raises ValueError."""
        with pytest.raises(ValueError, match="Invalid memory_type"):
            await memory_service.add_memory(
                namespace="user:alice:pref",
                key="theme",
                value={"mode": "dark"},
                memory_type="invalid_type",  # Invalid
                created_by="session:abc",
                task_id="task:xyz",
            )

    @pytest.mark.asyncio
    async def test_add_memory_invalid_namespace_raises_error(
        self, memory_service: MemoryService
    ) -> None:
        """Test that invalid namespace format raises ValueError."""
        with pytest.raises(ValueError, match="Invalid namespace format"):
            await memory_service.add_memory(
                namespace="invalid_namespace",  # Missing ':'
                key="theme",
                value={"mode": "dark"},
                memory_type="semantic",
                created_by="session:abc",
                task_id="task:xyz",
            )

    @pytest.mark.asyncio
    async def test_update_memory_creates_version_2(self, memory_service: MemoryService) -> None:
        """Test memory update creates new version."""
        # Create v1
        await memory_service.add_memory(
            namespace="user:alice:preferences",
            key="theme",
            value={"mode": "dark"},
            memory_type="semantic",
            created_by="session:abc",
            task_id="task:xyz",
        )

        # Update to v2
        _new_id = await memory_service.update_memory(
            namespace="user:alice:preferences",
            key="theme",
            value={"mode": "light"},
            updated_by="session:abc",
            task_id="task:xyz",
        )

        # Verify v2 is current
        memory = await memory_service.get_memory("user:alice:preferences", "theme")
        assert memory is not None
        assert memory["version"] == 2
        assert memory["value"] == {"mode": "light"}

        # Verify v1 still exists
        memory_v1 = await memory_service.get_memory("user:alice:preferences", "theme", version=1)
        assert memory_v1 is not None
        assert memory_v1["value"] == {"mode": "dark"}
        assert memory_v1["version"] == 1

    @pytest.mark.asyncio
    async def test_update_memory_multiple_versions(self, memory_service: MemoryService) -> None:
        """Test creating multiple memory versions."""
        await memory_service.add_memory(
            namespace="user:alice:counter",
            key="count",
            value={"count": 0},
            memory_type="semantic",
            created_by="session:abc",
            task_id="task:xyz",
        )

        # Update 4 times
        for i in range(1, 5):
            await memory_service.update_memory(
                namespace="user:alice:counter",
                key="count",
                value={"count": i},
                updated_by=f"session:update_{i}",
                task_id="task:xyz",
            )

        # Verify latest version
        memory = await memory_service.get_memory("user:alice:counter", "count")
        assert memory is not None
        assert memory["version"] == 5, f"Expected version 5, got {memory['version']}"
        assert memory["value"] == {"count": 4}

    @pytest.mark.asyncio
    async def test_update_nonexistent_memory_raises_error(
        self, memory_service: MemoryService
    ) -> None:
        """Test updating nonexistent memory raises ValueError."""
        with pytest.raises(ValueError, match="Memory not found"):
            await memory_service.update_memory(
                namespace="user:alice:pref",
                key="nonexistent",
                value={"test": "value"},
                updated_by="session:abc",
                task_id="task:xyz",
            )

    @pytest.mark.asyncio
    async def test_delete_memory_soft_delete(self, memory_service: MemoryService) -> None:
        """Test soft delete marks memory as deleted."""
        await memory_service.add_memory(
            namespace="user:alice:temp",
            key="temporary",
            value={"data": "value"},
            memory_type="episodic",
            created_by="session:abc",
            task_id="task:xyz",
        )

        # Soft delete
        deleted = await memory_service.delete_memory(
            namespace="user:alice:temp", key="temporary", task_id="task:xyz"
        )
        assert deleted is True

        # Verify memory is marked deleted (not returned by get_memory)
        memory = await memory_service.get_memory("user:alice:temp", "temporary")
        assert memory is None, "Deleted memory should not be returned"

    @pytest.mark.asyncio
    async def test_search_memories_by_namespace_prefix(self, memory_service: MemoryService) -> None:
        """Test namespace prefix search returns correct results."""
        # Create memories in different namespaces
        await memory_service.add_memory(
            "user:alice:preferences",
            "theme",
            {"mode": "dark"},
            "semantic",
            "session:abc",
            "task:xyz",
        )
        await memory_service.add_memory(
            "user:alice:settings", "lang", {"code": "python"}, "semantic", "session:abc", "task:xyz"
        )
        await memory_service.add_memory(
            "user:bob:preferences",
            "theme",
            {"mode": "light"},
            "semantic",
            "session:def",
            "task:uvw",
        )

        # Search user:alice namespace
        alice_memories = await memory_service.search_memories("user:alice")
        assert len(alice_memories) == 2

        # Search user:bob namespace
        bob_memories = await memory_service.search_memories("user:bob")
        assert len(bob_memories) == 1

    @pytest.mark.asyncio
    async def test_search_memories_by_type(self, memory_service: MemoryService) -> None:
        """Test filtering memories by type."""
        await memory_service.add_memory(
            "project:test:data",
            "fact",
            {"value": "semantic"},
            "semantic",
            "session:abc",
            "task:xyz",
        )
        await memory_service.add_memory(
            "project:test:data",
            "event",
            {"value": "episodic"},
            "episodic",
            "session:abc",
            "task:xyz",
        )
        await memory_service.add_memory(
            "project:test:data",
            "rule",
            {"value": "procedural"},
            "procedural",
            "session:abc",
            "task:xyz",
        )

        # Filter by semantic
        semantic = await memory_service.search_memories("project:test", memory_type="semantic")
        assert len(semantic) == 1
        assert semantic[0]["memory_type"] == "semantic"

        # Filter by episodic
        episodic = await memory_service.search_memories("project:test", memory_type="episodic")
        assert len(episodic) == 1
        assert episodic[0]["memory_type"] == "episodic"

    @pytest.mark.asyncio
    async def test_search_memories_limit(self, memory_service: MemoryService) -> None:
        """Test search limit parameter works."""
        # Create 10 memories
        for i in range(10):
            await memory_service.add_memory(
                f"user:alice:test_{i}",
                f"key_{i}",
                {"data": i},
                "semantic",
                "session:abc",
                "task:xyz",
            )

        # Search with limit=5
        results = await memory_service.search_memories("user:alice", limit=5)
        assert len(results) == 5

    @pytest.mark.asyncio
    async def test_list_namespaces(self, memory_service: MemoryService) -> None:
        """Test listing unique namespaces."""
        await memory_service.add_memory(
            "user:alice:preferences",
            "theme",
            {"mode": "dark"},
            "semantic",
            "session:abc",
            "task:xyz",
        )
        await memory_service.add_memory(
            "user:alice:settings", "lang", {"code": "python"}, "semantic", "session:abc", "task:xyz"
        )
        await memory_service.add_memory(
            "user:bob:preferences",
            "theme",
            {"mode": "light"},
            "semantic",
            "session:def",
            "task:uvw",
        )

        namespaces = await memory_service.list_namespaces()

        # Should have 3 unique namespaces
        assert len(namespaces) == 3
        assert "user:alice:preferences" in namespaces
        assert "user:alice:settings" in namespaces
        assert "user:bob:preferences" in namespaces

    @pytest.mark.asyncio
    async def test_get_memory_history(self, memory_service: MemoryService) -> None:
        """Test getting all versions of a memory entry."""
        # Create v1
        await memory_service.add_memory(
            "user:alice:pref", "theme", {"mode": "dark"}, "semantic", "session:abc", "task:xyz"
        )

        # Create v2 and v3
        await memory_service.update_memory(
            "user:alice:pref", "theme", {"mode": "light"}, "session:abc", "task:xyz"
        )
        await memory_service.update_memory(
            "user:alice:pref", "theme", {"mode": "auto"}, "session:abc", "task:xyz"
        )

        # Get history
        history = await memory_service.get_memory_history("user:alice:pref", "theme")

        assert len(history) == 3
        assert history[0]["version"] == 3  # Most recent first
        assert history[0]["value"] == {"mode": "auto"}
        assert history[1]["version"] == 2
        assert history[1]["value"] == {"mode": "light"}
        assert history[2]["version"] == 1
        assert history[2]["value"] == {"mode": "dark"}

    @pytest.mark.asyncio
    async def test_cleanup_expired_memories(self, memory_service: MemoryService) -> None:
        """Test cleanup of expired episodic memories."""
        # Create episodic memory
        await memory_service.add_memory(
            "temp:test:data", "key1", {"value": "test"}, "episodic", "session:abc", "task:xyz"
        )

        # Manually update created_at to simulate old memory
        async with memory_service.db._get_connection() as conn:
            await conn.execute(
                """
                UPDATE memory_entries
                SET created_at = datetime('now', '-100 days')
                WHERE namespace = 'temp:test:data' AND key = 'key1'
                """
            )
            await conn.commit()

        # Cleanup memories older than 90 days
        deleted_count = await memory_service.cleanup_expired_memories(ttl_days=90)
        assert deleted_count == 1, f"Expected 1 deletion, got {deleted_count}"

        # Verify memory is marked as deleted
        memory = await memory_service.get_memory("temp:test:data", "key1")
        assert memory is None

    @pytest.mark.asyncio
    async def test_cleanup_does_not_affect_semantic_memories(
        self, memory_service: MemoryService
    ) -> None:
        """Test cleanup only affects episodic memories."""
        # Create semantic memory
        await memory_service.add_memory(
            "user:alice:pref", "theme", {"mode": "dark"}, "semantic", "session:abc", "task:xyz"
        )

        # Manually set created_at to old date
        async with memory_service.db._get_connection() as conn:
            await conn.execute(
                """
                UPDATE memory_entries
                SET created_at = datetime('now', '-100 days')
                WHERE namespace = 'user:alice:pref' AND key = 'theme'
                """
            )
            await conn.commit()

        # Cleanup should not affect semantic memories
        deleted_count = await memory_service.cleanup_expired_memories(ttl_days=90)
        assert deleted_count == 0

        # Verify semantic memory still exists
        memory = await memory_service.get_memory("user:alice:pref", "theme")
        assert memory is not None

    @pytest.mark.asyncio
    async def test_memory_type_classification(self, memory_service: MemoryService) -> None:
        """Test all three memory types can be created."""
        # Semantic memory (facts)
        await memory_service.add_memory(
            "user:alice:knowledge",
            "python_expert",
            {"level": "expert"},
            "semantic",
            "session:abc",
            "task:xyz",
        )

        # Episodic memory (experiences)
        await memory_service.add_memory(
            "user:alice:history",
            "task_123",
            {"outcome": "success"},
            "episodic",
            "session:abc",
            "task:xyz",
        )

        # Procedural memory (rules)
        await memory_service.add_memory(
            "app:abathur:rules",
            "error_handling",
            {"strategy": "retry_3x"},
            "procedural",
            "session:abc",
            "task:xyz",
        )

        # Verify all types exist
        semantic = await memory_service.search_memories(
            "user:alice:knowledge", memory_type="semantic"
        )
        episodic = await memory_service.search_memories(
            "user:alice:history", memory_type="episodic"
        )
        procedural = await memory_service.search_memories(
            "app:abathur:rules", memory_type="procedural"
        )

        assert len(semantic) == 1
        assert len(episodic) == 1
        assert len(procedural) == 1

    @pytest.mark.asyncio
    async def test_hierarchical_namespace_organization(self, memory_service: MemoryService) -> None:
        """Test hierarchical namespace structure (project:, app:, user:, session:, temp:)."""
        # Create memories at different hierarchy levels
        await memory_service.add_memory(
            "project:abathur:config",
            "max_agents",
            {"value": 50},
            "semantic",
            "session:abc",
            "task:xyz",
        )
        await memory_service.add_memory(
            "app:abathur:settings",
            "log_level",
            {"value": "INFO"},
            "semantic",
            "session:abc",
            "task:xyz",
        )
        await memory_service.add_memory(
            "user:alice:preferences",
            "theme",
            {"mode": "dark"},
            "semantic",
            "session:abc",
            "task:xyz",
        )
        await memory_service.add_memory(
            "session:abc123:state",
            "current_task",
            {"id": "task_1"},
            "episodic",
            "session:abc",
            "task:xyz",
        )
        await memory_service.add_memory(
            "temp:cache:data",
            "temp_value",
            {"data": "cached"},
            "episodic",
            "session:abc",
            "task:xyz",
        )

        # Verify we can retrieve at each level
        project_memories = await memory_service.search_memories("project:")
        app_memories = await memory_service.search_memories("app:")
        user_memories = await memory_service.search_memories("user:")
        session_memories = await memory_service.search_memories("session:")
        temp_memories = await memory_service.search_memories("temp:")

        assert len(project_memories) >= 1
        assert len(app_memories) >= 1
        assert len(user_memories) >= 1
        assert len(session_memories) >= 1
        assert len(temp_memories) >= 1
