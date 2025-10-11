"""Integration tests for session-task-memory workflows."""

from datetime import datetime, timezone
from uuid import uuid4

import pytest
from abathur.domain.models import Task, TaskStatus
from abathur.infrastructure.database import Database
from abathur.services import MemoryService, SessionService


class TestSessionMemoryWorkflow:
    """Test complete workflows integrating sessions, tasks, and memories."""

    @pytest.mark.asyncio
    async def test_complete_task_execution_workflow(self, memory_db: Database) -> None:
        """Test full workflow: session → task → memory → audit."""
        session_service = SessionService(memory_db)
        memory_service = MemoryService(memory_db)

        # Step 1: Create session
        session_id = str(uuid4())
        await session_service.create_session(
            session_id=session_id, app_name="abathur", user_id="alice", project_id="test_project"
        )

        # Step 2: Create task linked to session
        task_id = uuid4()
        task = Task(
            id=task_id,
            prompt="Test task execution",
            agent_type="general",
            priority=5,
            status=TaskStatus.PENDING,
            input_data={"param": "value"},
            submitted_at=datetime.now(timezone.utc),
            last_updated_at=datetime.now(timezone.utc),
            session_id=session_id,
            dependencies=[],
        )
        await memory_db.insert_task(task)

        # Step 3: Execute task (append event)
        event = {
            "event_id": "evt_001",
            "timestamp": datetime.now(timezone.utc).isoformat(),
            "event_type": "action",
            "actor": "agent:test",
            "content": {"action": "execute_task", "task_id": str(task_id)},
            "is_final_response": False,
        }
        state_delta = {"session:current_task_id": str(task_id)}
        await session_service.append_event(session_id, event, state_delta)

        # Step 4: Store memory from task execution
        memory_id = await memory_service.add_memory(
            namespace="user:alice:task_history",
            key=f"task_{task_id}",
            value={"status": "success", "duration_ms": 1500},
            memory_type="episodic",
            created_by=session_id,
            task_id=str(task_id),
        )

        # Step 5: Verify audit trail
        async with memory_db._get_connection() as conn:
            cursor = await conn.execute(
                "SELECT * FROM audit WHERE memory_entry_id = ?", (memory_id,)
            )
            audit_entries = list(await cursor.fetchall())

        assert len(audit_entries) == 1
        assert audit_entries[0]["action_type"] == "memory_create"
        assert audit_entries[0]["memory_operation_type"] == "create"

        # Step 6: Verify task-session linkage
        retrieved_task = await memory_db.get_task(task_id)
        assert retrieved_task is not None
        assert str(retrieved_task.session_id) == session_id

        # Step 7: Verify session state
        session = await session_service.get_session(session_id)
        assert session is not None
        assert session["state"]["session:current_task_id"] == str(task_id)
        assert len(session["events"]) == 1

    @pytest.mark.asyncio
    async def test_memory_versioning_workflow(self, memory_db: Database) -> None:
        """Test memory versioning workflow: create → update 5 times → verify history."""
        memory_service = MemoryService(memory_db)

        # Create v1
        await memory_service.add_memory(
            "user:alice:counter", "value", {"count": 0}, "semantic", "session:abc", "task:xyz"
        )

        # Update 5 times
        for i in range(1, 6):
            await memory_service.update_memory(
                "user:alice:counter", "value", {"count": i}, f"session:update_{i}", "task:xyz"
            )

        # Get history
        history = await memory_service.get_memory_history("user:alice:counter", "value")

        # Verify 6 versions (1 create + 5 updates)
        assert len(history) == 6
        for i, mem in enumerate(history):
            expected_version = 6 - i  # Reverse order
            assert mem["version"] == expected_version
            assert mem["value"]["count"] == expected_version - 1

    @pytest.mark.asyncio
    async def test_namespace_hierarchy_workflow(self, memory_db: Database) -> None:
        """Test hierarchical namespace organization and search."""
        memory_service = MemoryService(memory_db)

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
            "project:abathur:rules",
            "timeout",
            {"seconds": 3600},
            "procedural",
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
            "user:alice:settings",
            "language",
            {"code": "python"},
            "semantic",
            "session:abc",
            "task:xyz",
        )

        # Search project: namespace
        project_memories = await memory_service.search_memories("project:abathur")
        assert len(project_memories) == 2

        # Search user:alice namespace
        alice_memories = await memory_service.search_memories("user:alice")
        assert len(alice_memories) == 2

        # Verify hierarchical organization
        project_namespaces = {mem["namespace"] for mem in project_memories}
        assert "project:abathur:config" in project_namespaces
        assert "project:abathur:rules" in project_namespaces

    @pytest.mark.asyncio
    async def test_session_task_cascade_delete(self, memory_db: Database) -> None:
        """Test ON DELETE SET NULL cascade for tasks.session_id."""
        session_service = SessionService(memory_db)

        # Create session and task
        session_id = str(uuid4())
        await session_service.create_session(session_id, "app", "user")

        task_id = uuid4()
        task = Task(
            id=task_id,
            prompt="Test task",
            agent_type="general",
            priority=5,
            status=TaskStatus.PENDING,
            input_data={},
            submitted_at=datetime.now(timezone.utc),
            last_updated_at=datetime.now(timezone.utc),
            session_id=session_id,
            dependencies=[],
        )
        await memory_db.insert_task(task)

        # Verify task has session_id
        retrieved_task = await memory_db.get_task(task_id)
        assert retrieved_task is not None
        assert str(retrieved_task.session_id) == session_id

        # Delete session
        async with memory_db._get_connection() as conn:
            await conn.execute("DELETE FROM sessions WHERE id = ?", (session_id,))
            await conn.commit()

        # Verify task.session_id is set to NULL (ON DELETE SET NULL)
        retrieved_task = await memory_db.get_task(task_id)
        assert retrieved_task is not None
        assert retrieved_task.session_id is None

    @pytest.mark.asyncio
    async def test_multi_session_project_collaboration(self, memory_db: Database) -> None:
        """Test multiple sessions collaborating on same project."""
        session_service = SessionService(memory_db)
        memory_service = MemoryService(memory_db)

        # Create 3 sessions in same project
        session_ids = []
        for i, user in enumerate(["alice", "bob", "charlie"]):
            session_id = str(uuid4())
            await session_service.create_session(
                session_id, "abathur", user, project_id="schema_redesign"
            )
            session_ids.append(session_id)

            # Each session creates a memory
            await memory_service.add_memory(
                f"user:{user}:contributions",
                "schema_task",
                {"completed": True},
                "episodic",
                session_id,
                f"task:{i}",
            )

        # Query sessions by project
        project_sessions = await session_service.list_sessions(project_id="schema_redesign")
        assert len(project_sessions) == 3

        # Verify memories from all users
        alice_memories = await memory_service.search_memories("user:alice")
        bob_memories = await memory_service.search_memories("user:bob")
        charlie_memories = await memory_service.search_memories("user:charlie")

        assert len(alice_memories) == 1
        assert len(bob_memories) == 1
        assert len(charlie_memories) == 1

    @pytest.mark.asyncio
    async def test_session_state_merge_workflow(self, memory_db: Database) -> None:
        """Test session state merging with multiple updates."""
        session_service = SessionService(memory_db)

        session_id = str(uuid4())
        await session_service.create_session(
            session_id, "app", "user", initial_state={"user:theme": "dark"}
        )

        # Append events with state deltas
        for i in range(5):
            event = {
                "event_id": f"evt_{i}",
                "timestamp": datetime.now(timezone.utc).isoformat(),
                "event_type": "message",
                "actor": "user",
                "content": {"message": f"Message {i}"},
                "is_final_response": False,
            }
            state_delta = {f"session:msg_{i}": True, "session:msg_count": i + 1}
            await session_service.append_event(session_id, event, state_delta)

        # Verify final state
        session = await session_service.get_session(session_id)
        assert session is not None
        assert session["state"]["user:theme"] == "dark"  # Initial state preserved
        assert session["state"]["session:msg_count"] == 5  # Updated 5 times
        assert all(session["state"][f"session:msg_{i}"] for i in range(5))
        assert len(session["events"]) == 5

    @pytest.mark.asyncio
    async def test_memory_audit_trail_integrity(self, memory_db: Database) -> None:
        """Test audit trail integrity for memory operations."""
        memory_service = MemoryService(memory_db)

        # Create memory
        _memory_id = await memory_service.add_memory(
            "user:alice:pref", "theme", {"mode": "dark"}, "semantic", "session:abc", "task:create"
        )

        # Update memory twice
        await memory_service.update_memory(
            "user:alice:pref", "theme", {"mode": "light"}, "session:abc", "task:update_1"
        )
        await memory_service.update_memory(
            "user:alice:pref", "theme", {"mode": "auto"}, "session:abc", "task:update_2"
        )

        # Delete memory
        await memory_service.delete_memory("user:alice:pref", "theme", "task:delete")

        # Verify audit trail
        async with memory_db._get_connection() as conn:
            cursor = await conn.execute(
                """
                SELECT * FROM audit
                WHERE memory_namespace = 'user:alice:pref'
                ORDER BY timestamp ASC
                """
            )
            audit_entries = list(await cursor.fetchall())

        # Should have 4 entries: 1 create + 2 updates + 1 delete
        assert len(audit_entries) >= 4

        # Verify operation types
        operation_types = [entry["memory_operation_type"] for entry in audit_entries]
        assert "create" in operation_types
        assert "update" in operation_types
        assert "delete" in operation_types
