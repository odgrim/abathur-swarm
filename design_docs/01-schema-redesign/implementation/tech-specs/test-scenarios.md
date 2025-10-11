# Test Scenarios - Comprehensive Testing Strategy

## Overview

This document specifies unit tests, integration tests, and performance benchmarks for validating the memory management schema implementation.

**Testing Framework:** pytest + pytest-asyncio

**Coverage Target:** 95%+ for database layer, 85%+ for service layer

---

## 1. Unit Tests - Session Management

### 1.1 Session CRUD Operations

```python
import pytest
from abathur.infrastructure.database import Database
from pathlib import Path
import uuid

@pytest.mark.asyncio
async def test_create_session():
    """Test session creation with initial state."""
    db = Database(Path(":memory:"))
    await db.initialize()

    session_id = str(uuid.uuid4())
    await db.sessions.create_session(
        session_id=session_id,
        app_name="test_app",
        user_id="test_user",
        initial_state={"user:test_user:theme": "dark"}
    )

    # Verify session exists
    session = await db.sessions.get_session(session_id)
    assert session is not None
    assert session['app_name'] == 'test_app'
    assert session['user_id'] == 'test_user'
    assert session['status'] == 'created'
    assert session['state']['user:test_user:theme'] == 'dark'

@pytest.mark.asyncio
async def test_duplicate_session_raises_error():
    """Test that duplicate session_id raises error."""
    db = Database(Path(":memory:"))
    await db.initialize()

    session_id = str(uuid.uuid4())
    await db.sessions.create_session(session_id, "app", "user")

    # Attempt duplicate creation
    with pytest.raises(ValueError, match="already exists"):
        await db.sessions.create_session(session_id, "app", "user")

@pytest.mark.asyncio
async def test_append_event():
    """Test event appending with state delta."""
    db = Database(Path(":memory:"))
    await db.initialize()

    session_id = str(uuid.uuid4())
    await db.sessions.create_session(session_id, "app", "user")

    # Append event
    event = {
        "event_id": "evt_001",
        "timestamp": "2025-10-10T10:00:00Z",
        "event_type": "message",
        "actor": "user",
        "content": {"message": "Hello"},
        "is_final_response": False
    }
    state_delta = {"session:current_task": "greeting"}

    await db.sessions.append_event(session_id, event, state_delta)

    # Verify event and state
    session = await db.sessions.get_session(session_id)
    assert len(session['events']) == 1
    assert session['events'][0]['event_type'] == 'message'
    assert session['state']['session:current_task'] == 'greeting'

@pytest.mark.asyncio
async def test_update_session_status():
    """Test session status transitions."""
    db = Database(Path(":memory:"))
    await db.initialize()

    session_id = str(uuid.uuid4())
    await db.sessions.create_session(session_id, "app", "user")

    # Transition to active
    await db.sessions.update_status(session_id, "active")
    session = await db.sessions.get_session(session_id)
    assert session['status'] == 'active'

    # Transition to terminated
    await db.sessions.update_status(session_id, "terminated")
    session = await db.sessions.get_session(session_id)
    assert session['status'] == 'terminated'
    assert session['terminated_at'] is not None

@pytest.mark.asyncio
async def test_invalid_status_raises_error():
    """Test that invalid status raises ValueError."""
    db = Database(Path(":memory:"))
    await db.initialize()

    session_id = str(uuid.uuid4())
    await db.sessions.create_session(session_id, "app", "user")

    with pytest.raises(ValueError, match="Invalid status"):
        await db.sessions.update_status(session_id, "invalid_status")
```

---

## 2. Unit Tests - Memory Management

### 2.1 Memory CRUD with Versioning

```python
@pytest.mark.asyncio
async def test_add_memory():
    """Test creating new memory entry."""
    db = Database(Path(":memory:"))
    await db.initialize()

    memory_id = await db.memory.add_memory(
        namespace="user:alice:preferences",
        key="theme",
        value={"mode": "dark"},
        memory_type="semantic",
        created_by="session:abc123",
        task_id="task:xyz789"
    )

    assert memory_id > 0

    # Verify memory
    memory = await db.memory.get_memory("user:alice:preferences", "theme")
    assert memory is not None
    assert memory['value'] == {"mode": "dark"}
    assert memory['version'] == 1
    assert memory['memory_type'] == 'semantic'

@pytest.mark.asyncio
async def test_update_memory_creates_new_version():
    """Test memory update increments version."""
    db = Database(Path(":memory:"))
    await db.initialize()

    # Create v1
    await db.memory.add_memory(
        namespace="user:alice:preferences",
        key="theme",
        value={"mode": "dark"},
        memory_type="semantic",
        created_by="session:abc123",
        task_id="task:xyz789"
    )

    # Update to v2
    new_id = await db.memory.update_memory(
        namespace="user:alice:preferences",
        key="theme",
        value={"mode": "light"},
        updated_by="session:abc123",
        task_id="task:xyz789"
    )

    # Verify new version
    memory = await db.memory.get_memory("user:alice:preferences", "theme")
    assert memory['version'] == 2
    assert memory['value'] == {"mode": "light"}

    # Verify old version still exists
    old_memory = await db.memory.get_memory("user:alice:preferences", "theme", version=1)
    assert old_memory['value'] == {"mode": "dark"}

@pytest.mark.asyncio
async def test_search_memories_by_namespace():
    """Test namespace prefix search."""
    db = Database(Path(":memory:"))
    await db.initialize()

    # Create multiple memories
    await db.memory.add_memory(
        "user:alice:preferences", "theme", {"mode": "dark"},
        "semantic", "session:abc", "task:xyz"
    )
    await db.memory.add_memory(
        "user:alice:preferences", "language", {"code": "python"},
        "semantic", "session:abc", "task:xyz"
    )
    await db.memory.add_memory(
        "user:bob:preferences", "theme", {"mode": "light"},
        "semantic", "session:def", "task:uvw"
    )

    # Search user:alice namespace
    alice_memories = await db.memory.search_memories("user:alice")
    assert len(alice_memories) == 2

    # Search user:bob namespace
    bob_memories = await db.memory.search_memories("user:bob")
    assert len(bob_memories) == 1

@pytest.mark.asyncio
async def test_memory_type_filter():
    """Test filtering memories by type."""
    db = Database(Path(":memory:"))
    await db.initialize()

    # Create different types
    await db.memory.add_memory(
        "project:test:data", "semantic_fact", {"fact": "value"},
        "semantic", "session:abc", "task:xyz"
    )
    await db.memory.add_memory(
        "project:test:data", "episodic_event", {"event": "happened"},
        "episodic", "session:abc", "task:xyz"
    )

    # Filter by type
    semantic = await db.memory.search_memories("project:test", memory_type="semantic")
    assert len(semantic) == 1
    assert semantic[0]['memory_type'] == 'semantic'

    episodic = await db.memory.search_memories("project:test", memory_type="episodic")
    assert len(episodic) == 1
    assert episodic[0]['memory_type'] == 'episodic'
```

---

## 3. Integration Tests - Workflows

### 3.1 Complete Session-Task-Memory Workflow

```python
@pytest.mark.asyncio
async def test_complete_task_execution_workflow():
    """Test full workflow: session → task → memory → audit."""
    db = Database(Path(":memory:"))
    await db.initialize()

    # Step 1: Create session
    session_id = str(uuid.uuid4())
    await db.sessions.create_session(
        session_id=session_id,
        app_name="abathur",
        user_id="alice",
        project_id="test_project"
    )

    # Step 2: Create task linked to session
    task_id = str(uuid.uuid4())
    from abathur.domain.models import Task, TaskStatus
    from datetime import datetime, timezone

    task = Task(
        id=uuid.UUID(task_id),
        prompt="Test task",
        agent_type="general",
        priority=5,
        status=TaskStatus.PENDING,
        input_data={},
        submitted_at=datetime.now(timezone.utc),
        last_updated_at=datetime.now(timezone.utc),
        dependencies=[],
        session_id=session_id
    )
    await db.insert_task(task)

    # Step 3: Execute task (simulated) - append event
    await db.sessions.append_event(
        session_id=session_id,
        event={
            "event_id": "evt_001",
            "timestamp": datetime.now(timezone.utc).isoformat(),
            "event_type": "action",
            "actor": f"agent:test_agent",
            "content": {"action": "execute_task", "task_id": task_id},
            "is_final_response": False
        },
        state_delta={"session:current_task_id": task_id}
    )

    # Step 4: Store learned memory
    memory_id = await db.memory.add_memory(
        namespace=f"user:alice:task_history",
        key=f"task_{task_id}",
        value={"status": "success", "duration_ms": 1500},
        memory_type="episodic",
        created_by=f"session:{session_id}",
        task_id=task_id
    )

    # Step 5: Verify audit trail
    async with db._get_connection() as conn:
        cursor = await conn.execute(
            "SELECT * FROM audit WHERE memory_entry_id = ?",
            (memory_id,)
        )
        audit_entries = await cursor.fetchall()

    assert len(audit_entries) == 1
    assert audit_entries[0]['action_type'] == 'memory_create'
    assert audit_entries[0]['memory_operation_type'] == 'create'

    # Step 6: Verify task-session linkage
    retrieved_task = await db.get_task(uuid.UUID(task_id))
    assert str(retrieved_task.session_id) == session_id

    # Step 7: Verify session state
    session = await db.sessions.get_session(session_id)
    assert session['state']['session:current_task_id'] == task_id
    assert len(session['events']) == 1
```

---

## 4. Constraint Violation Tests

### 4.1 Foreign Key Constraints

```python
@pytest.mark.asyncio
async def test_task_invalid_session_fk_allows_null():
    """Test that task with invalid session_id can have NULL."""
    db = Database(Path(":memory:"))
    await db.initialize()

    # Create task with NULL session_id (should succeed)
    task_id = str(uuid.uuid4())
    task = Task(
        id=uuid.UUID(task_id),
        prompt="Test",
        status=TaskStatus.PENDING,
        input_data={},
        submitted_at=datetime.now(timezone.utc),
        last_updated_at=datetime.now(timezone.utc),
        dependencies=[],
        session_id=None  # NULL is allowed
    )
    await db.insert_task(task)

    retrieved = await db.get_task(uuid.UUID(task_id))
    assert retrieved.session_id is None

@pytest.mark.asyncio
async def test_unique_constraint_memory_version():
    """Test UNIQUE(namespace, key, version) constraint."""
    db = Database(Path(":memory:"))
    await db.initialize()

    # Create memory v1
    await db.memory.add_memory(
        "user:test:data", "key1", {"v": 1},
        "semantic", "session:abc", "task:xyz"
    )

    # Attempt to create duplicate v1 (should fail)
    async with db._get_connection() as conn:
        with pytest.raises(Exception):  # IntegrityError
            await conn.execute(
                """
                INSERT INTO memory_entries (namespace, key, value, memory_type, version, created_by, updated_by)
                VALUES (?, ?, ?, ?, ?, ?, ?)
                """,
                ("user:test:data", "key1", '{"v": 2}', "semantic", 1, "session:abc", "session:abc")
            )
```

### 4.2 JSON Validation Constraints

```python
@pytest.mark.asyncio
async def test_invalid_json_in_session_events_fails():
    """Test that invalid JSON in events column fails CHECK constraint."""
    db = Database(Path(":memory:"))
    await db.initialize()

    async with db._get_connection() as conn:
        with pytest.raises(Exception):  # CHECK constraint failed
            await conn.execute(
                """
                INSERT INTO sessions (id, app_name, user_id, events)
                VALUES (?, ?, ?, ?)
                """,
                ("test_id", "app", "user", "invalid json")
            )
```

---

## 5. Performance Tests

### 5.1 Concurrent Session Access

```python
import asyncio
import time

@pytest.mark.asyncio
async def test_concurrent_session_reads():
    """Test 50+ concurrent session reads (WAL mode performance)."""
    db = Database(Path("test_concurrent.db"))
    await db.initialize()

    # Create 50 sessions
    session_ids = [str(uuid.uuid4()) for _ in range(50)]
    for sid in session_ids:
        await db.sessions.create_session(sid, "app", f"user_{sid[:8]}")

    # Concurrent reads
    start_time = time.time()
    tasks = [db.sessions.get_session(sid) for sid in session_ids]
    results = await asyncio.gather(*tasks)
    duration = time.time() - start_time

    assert len(results) == 50
    assert all(r is not None for r in results)
    assert duration < 1.0  # All 50 reads in <1 second

@pytest.mark.asyncio
async def test_memory_query_latency():
    """Test memory retrieval performance."""
    db = Database(Path("test_perf.db"))
    await db.initialize()

    # Insert 1000 memories
    for i in range(1000):
        await db.memory.add_memory(
            f"user:test:memory_{i}",
            f"key_{i}",
            {"data": f"value_{i}"},
            "semantic",
            "session:abc",
            "task:xyz"
        )

    # Query performance
    start_time = time.time()
    results = await db.memory.search_memories("user:test", limit=100)
    duration = time.time() - start_time

    assert len(results) == 100
    assert duration < 0.05  # <50ms for 100 results from 1000 entries
```

### 5.2 Index Usage Verification

```python
@pytest.mark.asyncio
async def test_query_uses_index():
    """Verify EXPLAIN QUERY PLAN shows index usage."""
    db = Database(Path(":memory:"))
    await db.initialize()

    async with db._get_connection() as conn:
        # Test memory namespace query
        cursor = await conn.execute(
            """
            EXPLAIN QUERY PLAN
            SELECT * FROM memory_entries
            WHERE namespace = ? AND key = ? AND is_deleted = 0
            ORDER BY version DESC LIMIT 1
            """,
            ("user:test:pref", "theme")
        )
        plan = await cursor.fetchall()

        # Verify index is used (not table scan)
        plan_text = " ".join(str(row) for row in plan)
        assert "idx_memory_namespace_key_version" in plan_text or "USING INDEX" in plan_text
        assert "SCAN TABLE" not in plan_text
```

---

## 6. Test Fixtures and Utilities

### Common Fixtures

```python
import pytest
from pathlib import Path

@pytest.fixture
async def memory_db():
    """In-memory database for fast tests."""
    db = Database(Path(":memory:"))
    await db.initialize()
    yield db

@pytest.fixture
async def populated_db():
    """Database with sample data."""
    db = Database(Path(":memory:"))
    await db.initialize()

    # Seed data
    await db.sessions.create_session("sess_1", "app", "alice")
    await db.memory.add_memory(
        "user:alice:pref", "theme", {"mode": "dark"},
        "semantic", "sess_1", "task_1"
    )

    yield db
```

---

## 7. Test Execution

### Run All Tests

```bash
# Install dependencies
pip install pytest pytest-asyncio

# Run all tests
pytest design_docs/phase2_tech_specs/test_scenarios.md -v

# Run with coverage
pytest --cov=abathur.infrastructure.database --cov-report=html

# Run performance tests only
pytest -k "test_concurrent" -v
```

---

**Document Version:** 1.0
**Author:** technical-specifications-writer
**Date:** 2025-10-10
**Related Files:** `api-specifications.md`, `implementation-guide.md`
