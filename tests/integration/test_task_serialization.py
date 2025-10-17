"""Integration tests for Task serialization in MCP server.

Tests that _serialize_task includes ALL 28 Task model fields:
1. Core identification (5): id, prompt, agent_type, priority, status
2. Data fields (3): input_data, result_data, error_message
3. Retry and timeout (3): retry_count, max_retries, max_execution_timeout_seconds
4. Timestamp fields (4): submitted_at, started_at, completed_at, last_updated_at
5. Relationship fields (4): created_by, parent_task_id, dependencies, session_id
6. Summary field (1): summary
7. Enhanced task queue (6): source, dependency_type, calculated_priority, deadline, estimated_duration_seconds, dependency_depth
8. Branch tracking (2): feature_branch, task_branch

Coverage Target: Complete serialization validation
"""

from collections.abc import AsyncGenerator
from datetime import datetime, timedelta, timezone
from pathlib import Path
from uuid import uuid4

import pytest
from abathur.domain.models import TaskSource, TaskStatus
from abathur.infrastructure.database import Database
from abathur.mcp.task_queue_server import AbathurTaskQueueServer
from abathur.services.dependency_resolver import DependencyResolver
from abathur.services.priority_calculator import PriorityCalculator
from abathur.services.task_queue_service import TaskQueueService


@pytest.fixture
async def memory_db() -> AsyncGenerator[Database, None]:
    """Create in-memory database for fast integration tests."""
    db = Database(Path(":memory:"))
    await db.initialize()
    yield db
    await db.close()


@pytest.fixture
async def task_queue_service(memory_db: Database) -> TaskQueueService:
    """Create TaskQueueService with in-memory database."""
    dependency_resolver = DependencyResolver(memory_db)
    priority_calculator = PriorityCalculator(dependency_resolver)
    return TaskQueueService(memory_db, dependency_resolver, priority_calculator)


@pytest.fixture
def mcp_server(memory_db: Database) -> AbathurTaskQueueServer:
    """Create MCP server instance for testing serialization."""
    server = AbathurTaskQueueServer(Path(":memory:"))
    server._db = memory_db
    return server


@pytest.mark.asyncio
async def test_serialize_task_includes_all_28_fields(
    memory_db: Database, task_queue_service: TaskQueueService, mcp_server: AbathurTaskQueueServer
) -> None:
    """Test that _serialize_task includes all 28 Task model fields."""
    # Create prerequisite task first (no dependencies)
    prereq_task = await task_queue_service.enqueue_task(
        description="Prerequisite task",
        source=TaskSource.HUMAN,
        base_priority=5,
    )

    # Create task with many fields populated (avoid foreign key constraints in test)
    deadline = datetime.now(timezone.utc) + timedelta(hours=2)
    task = await task_queue_service.enqueue_task(
        description="Test task with all fields",
        source=TaskSource.AGENT_IMPLEMENTATION,
        summary="Test task summary for serialization",
        agent_type="python-backend-specialist",
        base_priority=7,
        prerequisites=[prereq_task.id],
        deadline=deadline,
        estimated_duration_seconds=1800,
        input_data={"key": "value"},
        feature_branch="feature/test-branch",
        task_branch="task/test-implementation",
        # Note: session_id omitted to avoid FK constraint (would need to create session first)
    )

    # Retrieve task (it already has many fields populated from enqueue)
    retrieved_task = await memory_db.get_task(task.id)
    assert retrieved_task is not None

    # Serialize task using MCP server's _serialize_task method
    serialized = mcp_server._serialize_task(retrieved_task)

    # Verify all 28 fields are present
    expected_fields = {
        # Core identification (5)
        "id",
        "prompt",
        "agent_type",
        "priority",
        "status",
        # Data fields (3)
        "input_data",
        "result_data",
        "error_message",
        # Retry and timeout fields (3)
        "retry_count",
        "max_retries",
        "max_execution_timeout_seconds",
        # Timestamp fields (4)
        "submitted_at",
        "started_at",
        "completed_at",
        "last_updated_at",
        # Relationship fields (4)
        "created_by",
        "parent_task_id",
        "dependencies",
        "session_id",
        # Summary field (1)
        "summary",
        # Enhanced task queue fields (6)
        "source",
        "dependency_type",
        "calculated_priority",
        "deadline",
        "estimated_duration_seconds",
        "dependency_depth",
        # Branch tracking fields (2)
        "feature_branch",
        "task_branch",
    }

    actual_fields = set(serialized.keys())
    assert (
        actual_fields == expected_fields
    ), f"Missing fields: {expected_fields - actual_fields}, Extra fields: {actual_fields - expected_fields}"

    # Verify key field values (sampling from each category)
    # Core identification
    assert serialized["id"] == str(retrieved_task.id)
    assert serialized["prompt"] == "Test task with all fields"
    assert serialized["agent_type"] == "python-backend-specialist"
    assert serialized["priority"] == 7
    assert serialized["status"] == TaskStatus.BLOCKED.value

    # Data fields (verify type/presence, values may vary based on task state)
    assert isinstance(serialized["input_data"], dict)
    assert serialized["result_data"] is None or isinstance(serialized["result_data"], dict)
    assert serialized["error_message"] is None or isinstance(serialized["error_message"], str)

    # Retry and timeout fields
    assert isinstance(serialized["retry_count"], int)
    assert isinstance(serialized["max_retries"], int)
    assert isinstance(serialized["max_execution_timeout_seconds"], int)

    # Timestamp fields
    assert isinstance(serialized["submitted_at"], str)
    assert serialized["started_at"] is None or isinstance(serialized["started_at"], str)
    assert serialized["completed_at"] is None or isinstance(serialized["completed_at"], str)
    assert isinstance(serialized["last_updated_at"], str)

    # Relationship fields
    assert serialized["dependencies"] == [str(prereq_task.id)]

    # Summary field
    assert serialized["summary"] == "Test task summary for serialization"

    # Enhanced task queue fields
    assert serialized["source"] == TaskSource.AGENT_IMPLEMENTATION.value
    assert serialized["dependency_type"] == "sequential"
    assert isinstance(serialized["calculated_priority"], float)
    assert isinstance(serialized["deadline"], str)
    assert serialized["estimated_duration_seconds"] == 1800
    assert serialized["dependency_depth"] == 1  # Depends on prereq_task

    # Branch tracking fields
    assert serialized["feature_branch"] == "feature/test-branch"
    assert serialized["task_branch"] == "task/test-implementation"


@pytest.mark.asyncio
async def test_serialize_task_with_null_optional_fields(
    memory_db: Database, task_queue_service: TaskQueueService, mcp_server: AbathurTaskQueueServer
) -> None:
    """Test serialization with NULL/None optional fields."""
    # Create minimal task
    task = await task_queue_service.enqueue_task(
        description="Minimal task",
        source=TaskSource.HUMAN,
    )

    # Serialize
    serialized = mcp_server._serialize_task(task)

    # Verify optional fields are None
    assert serialized["parent_task_id"] is None
    assert serialized["session_id"] is None
    assert serialized["summary"] is None
    assert serialized["deadline"] is None
    assert serialized["estimated_duration_seconds"] is None
    assert serialized["started_at"] is None
    assert serialized["completed_at"] is None
    assert serialized["result_data"] is None
    assert serialized["error_message"] is None
    assert serialized["created_by"] is None
    assert serialized["feature_branch"] is None
    assert serialized["task_branch"] is None

    # Verify required fields are present
    assert serialized["id"] is not None
    assert serialized["prompt"] == "Minimal task"
    assert serialized["status"] == TaskStatus.READY.value
    assert serialized["dependencies"] == []


@pytest.mark.asyncio
async def test_serialize_task_with_multiple_dependencies(
    memory_db: Database, task_queue_service: TaskQueueService, mcp_server: AbathurTaskQueueServer
) -> None:
    """Test serialization of task with multiple dependencies."""
    # Create prerequisite tasks
    prereq1 = await task_queue_service.enqueue_task(description="Prereq 1", source=TaskSource.HUMAN)
    prereq2 = await task_queue_service.enqueue_task(description="Prereq 2", source=TaskSource.HUMAN)
    prereq3 = await task_queue_service.enqueue_task(description="Prereq 3", source=TaskSource.HUMAN)

    # Create task with multiple prerequisites
    task = await task_queue_service.enqueue_task(
        description="Task with multiple prereqs",
        source=TaskSource.HUMAN,
        prerequisites=[prereq1.id, prereq2.id, prereq3.id],
    )

    # Retrieve task from database to get populated dependencies field
    retrieved_task = await memory_db.get_task(task.id)
    assert retrieved_task is not None

    # Serialize
    serialized = mcp_server._serialize_task(retrieved_task)

    # Verify dependencies array
    assert len(serialized["dependencies"]) == 3
    assert str(prereq1.id) in serialized["dependencies"]
    assert str(prereq2.id) in serialized["dependencies"]
    assert str(prereq3.id) in serialized["dependencies"]


@pytest.mark.asyncio
async def test_serialize_task_datetime_formatting(
    memory_db: Database, task_queue_service: TaskQueueService, mcp_server: AbathurTaskQueueServer
) -> None:
    """Test that datetime fields are properly formatted as ISO 8601 strings."""
    task = await task_queue_service.enqueue_task(
        description="Task for datetime test",
        source=TaskSource.HUMAN,
        deadline=datetime.now(timezone.utc) + timedelta(hours=1),
    )

    # Start and complete task to populate all datetime fields
    await task_queue_service.get_next_task()
    await task_queue_service.complete_task(task.id)

    # Retrieve completed task
    completed_task = await memory_db.get_task(task.id)

    # Serialize
    serialized = mcp_server._serialize_task(completed_task)

    # Verify datetime fields are ISO 8601 strings
    assert isinstance(serialized["submitted_at"], str)
    assert isinstance(serialized["started_at"], str)
    assert isinstance(serialized["completed_at"], str)
    assert isinstance(serialized["last_updated_at"], str)
    assert isinstance(serialized["deadline"], str)

    # Verify they can be parsed back to datetime
    datetime.fromisoformat(serialized["submitted_at"])
    datetime.fromisoformat(serialized["started_at"])
    datetime.fromisoformat(serialized["completed_at"])
    datetime.fromisoformat(serialized["last_updated_at"])
    datetime.fromisoformat(serialized["deadline"])


@pytest.mark.asyncio
async def test_serialize_task_enum_values(
    memory_db: Database, task_queue_service: TaskQueueService, mcp_server: AbathurTaskQueueServer
) -> None:
    """Test that enum fields are serialized as string values."""
    task = await task_queue_service.enqueue_task(
        description="Task for enum test",
        source=TaskSource.AGENT_PLANNER,
    )

    # Serialize
    serialized = mcp_server._serialize_task(task)

    # Verify enums are serialized as strings
    assert serialized["status"] == "ready"  # TaskStatus.READY.value
    assert serialized["source"] == "agent_planner"  # TaskSource.AGENT_PLANNER.value
    assert serialized["dependency_type"] == "sequential"  # Default


@pytest.mark.asyncio
async def test_serialize_task_field_count_matches_model(
    memory_db: Database, mcp_server: AbathurTaskQueueServer
) -> None:
    """Test that serialized dict has exactly 28 fields matching Task model."""
    # Create a simple task directly in database
    task_id = uuid4()
    async with memory_db._get_connection() as conn:
        await conn.execute(
            """
            INSERT INTO tasks (
                id, prompt, agent_type, priority, status, source,
                dependency_type, calculated_priority, dependency_depth,
                input_data, submitted_at, last_updated_at
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            (
                str(task_id),
                "Test task",
                "test-agent",
                5,
                "ready",
                "human",
                "sequential",
                5.0,
                0,
                "{}",
                datetime.now(timezone.utc).isoformat(),
                datetime.now(timezone.utc).isoformat(),
            ),
        )
        await conn.commit()

    # Retrieve and serialize
    task = await memory_db.get_task(task_id)
    serialized = mcp_server._serialize_task(task)

    # Count Task model fields
    from abathur.domain.models import Task as TaskModel

    model_field_count = len(TaskModel.model_fields)

    # Verify serialized dict has same number of fields
    assert len(serialized) == model_field_count == 28, f"Expected 28 fields, got {len(serialized)}"


if __name__ == "__main__":
    pytest.main([__file__, "-v"])
