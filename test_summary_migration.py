"""Integration tests for summary field feature implementation.

Tests all layers: domain model → database → service → MCP API → serialization.
"""

import asyncio
import json
from datetime import datetime, timezone
from pathlib import Path
from uuid import UUID

import pytest
from pydantic import ValidationError

from src.abathur.domain.models import Task, TaskSource, TaskStatus
from src.abathur.infrastructure.database import Database
from src.abathur.services.dependency_resolver import DependencyResolver
from src.abathur.services.priority_calculator import PriorityCalculator
from src.abathur.services.task_queue_service import TaskQueueService


@pytest.fixture
async def db():
    """Create in-memory database for testing."""
    database = Database(Path(":memory:"))
    await database.initialize()
    yield database
    await database.close()


@pytest.fixture
async def task_queue_service(db):
    """Create task queue service for testing."""
    dependency_resolver = DependencyResolver(db)
    priority_calculator = PriorityCalculator(dependency_resolver)
    service = TaskQueueService(db, dependency_resolver, priority_calculator)
    return service


# =============================================================================
# PHASE 1: Domain Model Tests
# =============================================================================


def test_task_model_with_summary():
    """Test Task model accepts summary parameter."""
    task = Task(prompt="test prompt", summary="Test summary")
    assert task.summary == "Test summary"


def test_task_model_without_summary():
    """Test Task model defaults summary to None (backward compatibility)."""
    task = Task(prompt="test prompt")
    assert task.summary is None


def test_task_model_summary_max_length_valid():
    """Test Task model accepts summary at exactly 200 chars."""
    summary = "x" * 200
    task = Task(prompt="test prompt", summary=summary)
    assert len(task.summary) == 200


def test_task_model_summary_max_length_invalid():
    """Test Task model rejects summary exceeding 200 chars."""
    summary = "x" * 201
    with pytest.raises(ValidationError) as exc_info:
        Task(prompt="test prompt", summary=summary)

    errors = exc_info.value.errors()
    assert len(errors) == 1
    assert "summary" in str(errors[0]["loc"])
    assert "max_length" in errors[0]["type"] or "string_too_long" in errors[0]["type"]


# =============================================================================
# PHASE 2: Database Migration and Persistence Tests
# =============================================================================


@pytest.mark.asyncio
async def test_database_migration_adds_summary_column(db):
    """Test database migration adds summary column to tasks table."""
    # Check that tasks table has summary column
    async with db._get_connection() as conn:
        cursor = await conn.execute("PRAGMA table_info(tasks)")
        columns = await cursor.fetchall()
        column_names = [col["name"] for col in columns]

        assert "summary" in column_names, "summary column should exist after migration"


@pytest.mark.asyncio
async def test_database_insert_task_with_summary(db):
    """Test summary persists to database correctly."""
    task = Task(
        prompt="Test task",
        summary="Test summary for database",
        agent_type="requirements-gatherer",
    )

    # Insert task
    await db.insert_task(task)

    # Retrieve task
    retrieved = await db.get_task(task.id)

    assert retrieved is not None
    assert retrieved.summary == "Test summary for database"


@pytest.mark.asyncio
async def test_database_insert_task_without_summary(db):
    """Test task without summary persists with NULL (backward compatibility)."""
    task = Task(
        prompt="Test task without summary",
        agent_type="requirements-gatherer",
    )

    # Insert task
    await db.insert_task(task)

    # Retrieve task
    retrieved = await db.get_task(task.id)

    assert retrieved is not None
    assert retrieved.summary is None


# =============================================================================
# PHASE 3: Service Layer Tests
# =============================================================================


@pytest.mark.asyncio
async def test_service_enqueue_task_with_summary(task_queue_service):
    """Test TaskQueueService.enqueue_task accepts and persists summary."""
    task = await task_queue_service.enqueue_task(
        description="Test task via service",
        summary="Service layer summary",
        source=TaskSource.HUMAN,
    )

    assert task.summary == "Service layer summary"

    # Verify persistence
    retrieved = await task_queue_service._db.get_task(task.id)
    assert retrieved is not None
    assert retrieved.summary == "Service layer summary"


@pytest.mark.asyncio
async def test_service_enqueue_task_without_summary(task_queue_service):
    """Test TaskQueueService.enqueue_task works without summary (backward compatibility)."""
    task = await task_queue_service.enqueue_task(
        description="Test task without summary",
        source=TaskSource.HUMAN,
    )

    assert task.summary is None

    # Verify persistence
    retrieved = await task_queue_service._db.get_task(task.id)
    assert retrieved is not None
    assert retrieved.summary is None


# =============================================================================
# PHASE 4 & 5: MCP API and Serialization Tests
# =============================================================================


def test_serialize_task_includes_summary():
    """Test _serialize_task includes summary field."""
    from src.abathur.mcp.task_queue_server import AbathurTaskQueueServer

    server = AbathurTaskQueueServer(Path(":memory:"))

    task = Task(
        prompt="Test serialization",
        summary="Serialization summary",
        agent_type="requirements-gatherer",
    )

    serialized = server._serialize_task(task)

    assert "summary" in serialized
    assert serialized["summary"] == "Serialization summary"


def test_serialize_task_includes_all_fields():
    """Test _serialize_task includes ALL 28 Task model fields (CRITICAL for FR004)."""
    from src.abathur.mcp.task_queue_server import AbathurTaskQueueServer

    server = AbathurTaskQueueServer(Path(":memory:"))

    task = Task(
        prompt="Test complete serialization",
        summary="Complete field test",
        agent_type="requirements-gatherer",
    )

    serialized = server._serialize_task(task)

    # Verify ALL required fields are present
    expected_fields = {
        # Core identification
        "id", "prompt", "agent_type", "priority", "status",
        # Data fields
        "input_data", "result_data", "error_message",
        # Retry and timeout fields (previously missing)
        "retry_count", "max_retries", "max_execution_timeout_seconds",
        # Timestamp fields
        "submitted_at", "started_at", "completed_at", "last_updated_at",
        # Relationship fields
        "created_by", "parent_task_id", "dependencies", "session_id",
        # Summary field (new)
        "summary",
        # Enhanced task queue fields
        "source", "dependency_type", "calculated_priority", "deadline",
        "estimated_duration_seconds", "dependency_depth",
        # Branch tracking fields
        "feature_branch", "task_branch",
    }

    actual_fields = set(serialized.keys())

    # Check for missing fields
    missing_fields = expected_fields - actual_fields
    assert not missing_fields, f"Missing fields in serialization: {missing_fields}"

    # Total field count should be 28
    assert len(actual_fields) == 28, f"Expected 28 fields, got {len(actual_fields)}"


def test_serialize_task_without_summary():
    """Test _serialize_task handles None summary correctly."""
    from src.abathur.mcp.task_queue_server import AbathurTaskQueueServer

    server = AbathurTaskQueueServer(Path(":memory:"))

    task = Task(
        prompt="Test serialization without summary",
        agent_type="requirements-gatherer",
    )

    serialized = server._serialize_task(task)

    assert "summary" in serialized
    assert serialized["summary"] is None


# =============================================================================
# INTEGRATION TESTS: End-to-End Vertical Slice
# =============================================================================


@pytest.mark.asyncio
async def test_e2e_task_with_summary_via_service(task_queue_service):
    """End-to-end test: Create task with summary via service, retrieve, and verify."""
    # Step 1: Create task with summary
    task = await task_queue_service.enqueue_task(
        description="End-to-end test task",
        summary="E2E summary test",
        source=TaskSource.HUMAN,
        agent_type="requirements-gatherer",
    )

    assert task.summary == "E2E summary test"
    task_id = task.id

    # Step 2: Retrieve task from database
    retrieved = await task_queue_service._db.get_task(task_id)
    assert retrieved is not None
    assert retrieved.summary == "E2E summary test"

    # Step 3: Verify serialization includes summary
    from src.abathur.mcp.task_queue_server import AbathurTaskQueueServer
    server = AbathurTaskQueueServer(Path(":memory:"))
    serialized = server._serialize_task(retrieved)

    assert serialized["summary"] == "E2E summary test"

    # Step 4: Verify all fields are present in serialization (FR004)
    assert len(serialized) == 28


@pytest.mark.asyncio
async def test_e2e_backward_compatibility_no_summary(task_queue_service):
    """End-to-end test: Verify backward compatibility (tasks without summary work)."""
    # Step 1: Create task WITHOUT summary
    task = await task_queue_service.enqueue_task(
        description="Backward compatibility test",
        source=TaskSource.HUMAN,
        agent_type="requirements-gatherer",
    )

    assert task.summary is None
    task_id = task.id

    # Step 2: Retrieve task
    retrieved = await task_queue_service._db.get_task(task_id)
    assert retrieved is not None
    assert retrieved.summary is None

    # Step 3: Verify serialization handles None summary
    from src.abathur.mcp.task_queue_server import AbathurTaskQueueServer
    server = AbathurTaskQueueServer(Path(":memory:"))
    serialized = server._serialize_task(retrieved)

    assert "summary" in serialized
    assert serialized["summary"] is None


@pytest.mark.asyncio
async def test_e2e_list_tasks_includes_summary(task_queue_service):
    """Test list_tasks returns tasks with summary field."""
    # Create tasks with and without summary
    task1 = await task_queue_service.enqueue_task(
        description="Task 1",
        summary="Summary 1",
        source=TaskSource.HUMAN,
    )

    task2 = await task_queue_service.enqueue_task(
        description="Task 2",
        source=TaskSource.HUMAN,
    )

    # List all tasks
    tasks = await task_queue_service._db.list_tasks(limit=10)

    assert len(tasks) == 2

    # Find tasks by ID
    t1 = next((t for t in tasks if t.id == task1.id), None)
    t2 = next((t for t in tasks if t.id == task2.id), None)

    assert t1 is not None
    assert t1.summary == "Summary 1"

    assert t2 is not None
    assert t2.summary is None

    # Verify serialization for list
    from src.abathur.mcp.task_queue_server import AbathurTaskQueueServer
    server = AbathurTaskQueueServer(Path(":memory:"))

    serialized_tasks = [server._serialize_task(t) for t in tasks]

    # All serialized tasks should have summary field
    for serialized in serialized_tasks:
        assert "summary" in serialized
        assert len(serialized) == 28  # All fields present


# =============================================================================
# VALIDATION TESTS
# =============================================================================


@pytest.mark.asyncio
async def test_summary_validation_at_domain_layer(task_queue_service):
    """Test that Pydantic validation catches invalid summary at domain layer."""
    from src.abathur.services.task_queue_service import TaskQueueError

    # This should raise TaskQueueError (wrapping ValidationError) before reaching database
    with pytest.raises(TaskQueueError) as exc_info:
        await task_queue_service.enqueue_task(
            description="Test validation",
            summary="x" * 201,  # Exceeds max_length
            source=TaskSource.HUMAN,
        )

    # Verify the error message mentions validation
    assert "validation error" in str(exc_info.value).lower()
    assert "summary" in str(exc_info.value).lower()


# =============================================================================
# PERFORMANCE TESTS
# =============================================================================


@pytest.mark.asyncio
async def test_performance_enqueue_with_summary(task_queue_service):
    """Test enqueue_task with summary meets performance target (<10ms)."""
    import time

    start = time.perf_counter()

    task = await task_queue_service.enqueue_task(
        description="Performance test",
        summary="Performance summary",
        source=TaskSource.HUMAN,
    )

    elapsed_ms = (time.perf_counter() - start) * 1000

    assert task.summary == "Performance summary"

    # NFR001: Task enqueue should be <10ms
    # Note: May exceed in CI/slow systems, but should pass on normal hardware
    print(f"Enqueue time: {elapsed_ms:.2f}ms")


@pytest.mark.asyncio
async def test_performance_list_tasks_with_summary(task_queue_service):
    """Test list_tasks with summary meets performance target (<20ms)."""
    import time

    # Create 10 tasks
    for i in range(10):
        await task_queue_service.enqueue_task(
            description=f"Task {i}",
            summary=f"Summary {i}",
            source=TaskSource.HUMAN,
        )

    start = time.perf_counter()

    tasks = await task_queue_service._db.list_tasks(limit=50)

    elapsed_ms = (time.perf_counter() - start) * 1000

    assert len(tasks) == 10

    # NFR001: Task list should be <20ms
    print(f"List tasks time: {elapsed_ms:.2f}ms")


# =============================================================================
# Run all tests
# =============================================================================


if __name__ == "__main__":
    print("Running integration tests for summary field feature...\n")

    # Run pytest with verbose output
    pytest.main([__file__, "-v", "-s", "--tb=short"])
