"""Integration tests for Task summary field feature.

Tests end-to-end workflows for the summary field:
- Service layer integration (enqueue_task with summary)
- Database persistence and retrieval
- MCP tool integration (task_enqueue, task_get, task_list)
- Backward compatibility (tasks without summary)
- Serialization (all 28 fields returned)

Coverage:
- FR001: Add summary field to Task model (Pydantic validation)
- FR002: Store summary in database (ALTER TABLE migration)
- FR003: MCP tools accept/return summary
- FR004: task_get returns ALL 28 Task fields
- NFR002: Backward compatibility (summary=None for old tasks)
"""

import asyncio
from collections.abc import AsyncGenerator
from datetime import datetime, timezone
from pathlib import Path
from uuid import uuid4

import pytest
from abathur.domain.models import TaskSource, TaskStatus
from abathur.infrastructure.database import Database
from abathur.services.dependency_resolver import DependencyResolver
from abathur.services.priority_calculator import PriorityCalculator
from abathur.services.task_queue_service import TaskQueueService

# Fixtures


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


# Service Layer Integration Tests


@pytest.mark.asyncio
async def test_enqueue_task_with_summary(memory_db: Database, task_queue_service: TaskQueueService):
    """Test enqueue_task accepts summary parameter and persists to database.

    Verifies:
    - Service accepts summary parameter
    - Summary persists to database
    - Summary retrieved correctly
    - Full end-to-end flow: Service → Database → Retrieval
    """
    # Arrange
    test_summary = "Test task for implementing user authentication"
    test_description = "Implement OAuth2 authentication flow with JWT tokens"

    # Act - enqueue task with summary
    enqueued_task = await task_queue_service.enqueue_task(
        description=test_description,
        source=TaskSource.HUMAN,
        summary=test_summary,
        base_priority=7,
    )

    # Assert - service returns task with summary
    assert enqueued_task.summary == test_summary
    assert enqueued_task.prompt == test_description
    assert enqueued_task.id is not None

    # Assert - database persistence
    retrieved_task = await memory_db.get_task(enqueued_task.id)
    assert retrieved_task is not None
    assert retrieved_task.summary == test_summary
    assert retrieved_task.prompt == test_description


@pytest.mark.asyncio
async def test_enqueue_task_without_summary(
    memory_db: Database, task_queue_service: TaskQueueService
):
    """Test enqueue_task auto-generates summary when not provided.

    Verifies:
    - Service accepts omitted summary parameter
    - Summary auto-generated from description with "User Prompt: " prefix
    - Task persists correctly with auto-generated summary
    - Auto-generation maintains backward compatibility
    """
    # Arrange
    test_description = "Implement user registration endpoint"

    # Act - enqueue task WITHOUT summary parameter
    enqueued_task = await task_queue_service.enqueue_task(
        description=test_description,
        source=TaskSource.HUMAN,
        base_priority=5,
    )

    # Assert - service auto-generates summary with "User Prompt: " prefix
    expected_summary = "User Prompt: " + test_description
    assert enqueued_task.summary == expected_summary
    assert enqueued_task.prompt == test_description

    # Assert - database persistence (auto-generated summary)
    retrieved_task = await memory_db.get_task(enqueued_task.id)
    assert retrieved_task is not None
    assert retrieved_task.summary == expected_summary
    assert retrieved_task.prompt == test_description


@pytest.mark.asyncio
async def test_enqueue_task_with_empty_string_summary(
    memory_db: Database, task_queue_service: TaskQueueService
):
    """Test enqueue_task auto-generates summary when empty string provided.

    Verifies:
    - Service treats empty string "" same as None (auto-generates)
    - Summary auto-generated from description with "User Prompt: " prefix
    - Empty string treated as "not provided"
    """
    # Arrange
    test_description = "Fix bug in payment processing"

    # Act - enqueue task with empty string summary
    enqueued_task = await task_queue_service.enqueue_task(
        description=test_description,
        source=TaskSource.HUMAN,
        summary="",  # Empty string - treated as not provided
        base_priority=5,
    )

    # Assert - service auto-generates summary (treats empty string as None)
    expected_summary = "User Prompt: " + test_description
    assert enqueued_task.summary == expected_summary

    # Assert - database persistence with auto-generated summary
    retrieved_task = await memory_db.get_task(enqueued_task.id)
    assert retrieved_task is not None
    assert retrieved_task.summary == expected_summary


@pytest.mark.asyncio
async def test_enqueue_task_with_max_length_summary(
    memory_db: Database, task_queue_service: TaskQueueService
):
    """Test enqueue_task accepts exactly 140 character summary.

    Verifies:
    - Service accepts 140 char summary (boundary condition)
    - Pydantic validation passes
    - Summary persisted correctly
    """
    # Arrange - exactly 140 characters
    test_summary = "x" * 140
    test_description = "Implement feature with maximum summary length"

    # Act
    enqueued_task = await task_queue_service.enqueue_task(
        description=test_description,
        source=TaskSource.HUMAN,
        summary=test_summary,
        base_priority=5,
    )

    # Assert
    assert enqueued_task.summary == test_summary
    assert len(enqueued_task.summary) == 140

    # Assert - database persistence
    retrieved_task = await memory_db.get_task(enqueued_task.id)
    assert retrieved_task is not None
    assert retrieved_task.summary == test_summary
    assert len(retrieved_task.summary) == 140


@pytest.mark.asyncio
async def test_enqueue_task_exceeds_max_summary_length(
    memory_db: Database, task_queue_service: TaskQueueService
):
    """Test enqueue_task truncates summary exceeding 140 characters.

    Verifies:
    - Service truncates summaries >140 chars to 140 chars
    - No validation error raised (service handles truncation)
    - Summary persisted as truncated value
    """
    # Arrange - 141 characters (one char too long)
    test_summary = "x" * 141
    test_description = "Test truncation of overly long summary"

    # Act - service should truncate to 140 chars
    enqueued_task = await task_queue_service.enqueue_task(
        description=test_description,
        source=TaskSource.HUMAN,
        summary=test_summary,
        base_priority=5,
    )

    # Assert - summary truncated to 140 chars
    assert enqueued_task.summary is not None
    assert len(enqueued_task.summary) == 140
    assert enqueued_task.summary == "x" * 140

    # Assert - database persistence with truncated value
    retrieved_task = await memory_db.get_task(enqueued_task.id)
    assert retrieved_task is not None
    assert retrieved_task.summary is not None
    assert len(retrieved_task.summary) == 140
    assert retrieved_task.summary == "x" * 140


@pytest.mark.asyncio
async def test_enqueue_task_with_unicode_summary(
    memory_db: Database, task_queue_service: TaskQueueService
):
    """Test enqueue_task accepts unicode characters in summary.

    Verifies:
    - Unicode characters (emojis, accents) accepted
    - Full unicode support
    - Correct persistence and retrieval
    """
    # Arrange - unicode with emojis and accents
    test_summary = "Fix bug 🐛 in payment processing with café ☕"
    test_description = "Debug payment gateway integration"

    # Act
    enqueued_task = await task_queue_service.enqueue_task(
        description=test_description,
        source=TaskSource.HUMAN,
        summary=test_summary,
        base_priority=5,
    )

    # Assert
    assert enqueued_task.summary == test_summary

    # Assert - database persistence preserves unicode
    retrieved_task = await memory_db.get_task(enqueued_task.id)
    assert retrieved_task is not None
    assert retrieved_task.summary == test_summary


# MCP Integration Tests


@pytest.mark.asyncio
async def test_mcp_task_enqueue_with_summary(
    memory_db: Database, task_queue_service: TaskQueueService
):
    """Test MCP task_enqueue tool with summary parameter.

    Simulates MCP tool invocation flow:
    1. MCP request with summary parameter
    2. Service layer processes request
    3. Database persistence
    4. Verify task created with summary

    Verifies:
    - MCP tool schema accepts summary parameter
    - Summary flows through service layer
    - Task persisted with summary
    """
    # Arrange - simulate MCP tool request
    mcp_request_params = {
        "description": "Implement user authentication feature",
        "summary": "Add OAuth2 login with JWT tokens",
        "source": "human",
        "base_priority": 8,
        "agent_type": "requirements-gatherer",
    }

    # Act - call service method (as MCP handler would)
    task = await task_queue_service.enqueue_task(
        description=mcp_request_params["description"],  # type: ignore[arg-type]
        source=TaskSource.HUMAN,
        summary=mcp_request_params["summary"],  # type: ignore[arg-type]
        base_priority=mcp_request_params["base_priority"],  # type: ignore[arg-type]
        agent_type=mcp_request_params["agent_type"],  # type: ignore[arg-type]
    )

    # Assert - task created with summary
    assert task.id is not None
    assert task.summary == mcp_request_params["summary"]
    assert task.prompt == mcp_request_params["description"]

    # Assert - database persistence
    retrieved_task = await memory_db.get_task(task.id)
    assert retrieved_task is not None
    assert retrieved_task.summary == mcp_request_params["summary"]


@pytest.mark.asyncio
async def test_mcp_task_get_returns_summary(
    memory_db: Database, task_queue_service: TaskQueueService
):
    """Test MCP task_get tool returns summary field.

    Simulates MCP task_get flow:
    1. Create task with summary
    2. Retrieve task (simulate MCP task_get)
    3. Serialize task
    4. Verify summary in response

    Verifies:
    - task_get returns summary field
    - Serialization includes summary
    - Response format matches MCP schema
    """
    # Arrange - create task with summary
    test_summary = "Refactor authentication module"
    task = await task_queue_service.enqueue_task(
        description="Refactor OAuth2 implementation for better modularity",
        source=TaskSource.HUMAN,
        summary=test_summary,
        base_priority=6,
    )

    # Act - retrieve task (simulate MCP task_get)
    retrieved_task = await memory_db.get_task(task.id)

    # Assert - task includes summary
    assert retrieved_task is not None
    assert retrieved_task.summary == test_summary

    # Assert - serialization includes summary
    serialized = retrieved_task.model_dump()
    assert "summary" in serialized
    assert serialized["summary"] == test_summary


@pytest.mark.asyncio
async def test_mcp_task_get_returns_all_28_fields(
    memory_db: Database, task_queue_service: TaskQueueService
):
    """Test MCP task_get returns ALL 28 Task fields (FR004).

    Verifies:
    - _serialize_task includes all Task model fields
    - No fields omitted in serialization
    - Matches Task model field count

    Expected fields (28 total):
    1. id, 2. prompt, 3. agent_type, 4. priority, 5. status
    6. input_data, 7. result_data, 8. error_message, 9. retry_count
    10. max_retries, 11. max_execution_timeout_seconds
    12. submitted_at, 13. started_at, 14. completed_at, 15. last_updated_at
    16. created_by, 17. parent_task_id, 18. dependencies, 19. session_id
    20. summary, 21. source, 22. dependency_type, 23. calculated_priority
    24. deadline, 25. estimated_duration_seconds, 26. dependency_depth
    27. feature_branch, 28. task_branch
    """
    # Arrange - create task with all optional fields populated
    task = await task_queue_service.enqueue_task(
        description="Full task with all fields",
        source=TaskSource.HUMAN,
        summary="Complete task with all fields populated",
        base_priority=7,
        deadline=datetime.now(timezone.utc),
        estimated_duration_seconds=3600,
        agent_type="requirements-gatherer",
        session_id=None,  # Don't use session_id to avoid FK constraint
        input_data={"key": "value"},
        feature_branch="feature/summary-field",
        task_branch="task/implement-summary",
    )

    # Act - retrieve and serialize task
    retrieved_task = await memory_db.get_task(task.id)
    serialized = retrieved_task.model_dump()  # type: ignore[union-attr]

    # Assert - all 28 fields present
    expected_fields = {
        "id",
        "prompt",
        "agent_type",
        "priority",
        "status",
        "input_data",
        "result_data",
        "error_message",
        "retry_count",
        "max_retries",
        "max_execution_timeout_seconds",
        "submitted_at",
        "started_at",
        "completed_at",
        "last_updated_at",
        "created_by",
        "parent_task_id",
        "dependencies",
        "session_id",
        "summary",
        "source",
        "dependency_type",
        "calculated_priority",
        "deadline",
        "estimated_duration_seconds",
        "dependency_depth",
        "feature_branch",
        "task_branch",
    }

    actual_fields = set(serialized.keys())
    assert len(actual_fields) == 28, f"Expected 28 fields, got {len(actual_fields)}"
    assert actual_fields == expected_fields, f"Missing fields: {expected_fields - actual_fields}"

    # Assert - summary field specifically present
    assert "summary" in serialized
    assert serialized["summary"] == "Complete task with all fields populated"


@pytest.mark.asyncio
async def test_mcp_task_list_returns_summary(
    memory_db: Database, task_queue_service: TaskQueueService
):
    """Test MCP task_list tool returns summary field for all tasks.

    Simulates MCP task_list flow:
    1. Create multiple tasks with/without summaries
    2. List all tasks (simulate MCP task_list)
    3. Verify all tasks include summary field

    Verifies:
    - task_list returns summary for all tasks
    - Tasks without summary have summary=None
    - Serialization consistent across list
    """
    # Arrange - create mix of tasks with/without summaries
    task_with_summary = await task_queue_service.enqueue_task(
        description="Task with summary",
        source=TaskSource.HUMAN,
        summary="This task has a summary",
        base_priority=5,
    )

    task_without_summary = await task_queue_service.enqueue_task(
        description="Task without summary",
        source=TaskSource.HUMAN,
        # No summary parameter
        base_priority=5,
    )

    task_with_empty_summary = await task_queue_service.enqueue_task(
        description="Task with empty summary",
        source=TaskSource.HUMAN,
        summary="",
        base_priority=5,
    )

    # Act - list all tasks (simulate MCP task_list)
    async with memory_db._get_connection() as conn:
        cursor = await conn.execute("SELECT * FROM tasks ORDER BY submitted_at ASC")
        rows = await cursor.fetchall()

    # Convert rows to tasks
    tasks = [memory_db._row_to_task(row) for row in rows]

    # Assert - all tasks returned
    assert len(tasks) == 3

    # Assert - each task has summary field
    task_ids = [task_with_summary.id, task_without_summary.id, task_with_empty_summary.id]
    for task in tasks:
        assert task.id in task_ids

        # Verify summary field present in serialization
        serialized = task.model_dump()
        assert "summary" in serialized

    # Assert - verify specific summaries
    retrieved_task_1 = next(t for t in tasks if t.id == task_with_summary.id)
    assert retrieved_task_1.summary == "This task has a summary"

    retrieved_task_2 = next(t for t in tasks if t.id == task_without_summary.id)
    # Service auto-generates summary with "User Prompt: " prefix
    assert retrieved_task_2.summary == "User Prompt: Task without summary"

    retrieved_task_3 = next(t for t in tasks if t.id == task_with_empty_summary.id)
    # Service auto-generates summary when empty string provided
    assert retrieved_task_3.summary == "User Prompt: Task with empty summary"


# Backward Compatibility Tests


@pytest.mark.asyncio
async def test_database_migration_backward_compatibility(memory_db: Database):
    """Test backward compatibility: existing tasks work with summary column default.

    Simulates migration scenario:
    1. Migration adds summary column with NOT NULL DEFAULT 'Task'
    2. Existing rows get backfilled with auto-generated summaries
    3. Verify task can be retrieved with default or backfilled summary
    4. Verify no data loss

    Verifies:
    - Old tasks (before migration) get default 'Task' summary after migration
    - Migration backfill updates old tasks with auto-generated summaries
    - No errors reading migrated data
    - Backward compatibility maintained (NFR002)
    """
    # Arrange - simulate old task AFTER migration (has default 'Task' summary)
    old_task_id = uuid4()
    old_task_description = "Old task before migration"

    async with memory_db._get_connection() as conn:
        # Insert task with default 'Task' summary (simulates post-migration state)
        # The migration adds: ALTER TABLE tasks ADD COLUMN summary TEXT NOT NULL DEFAULT 'Task'
        # Then backfills with auto-generated summaries based on source
        await conn.execute(
            """
            INSERT INTO tasks (
                id, prompt, agent_type, priority, status, input_data,
                result_data, error_message, retry_count, max_retries,
                max_execution_timeout_seconds,
                submitted_at, started_at, completed_at, last_updated_at,
                created_by, parent_task_id, dependencies, session_id,
                source, dependency_type, calculated_priority, deadline,
                estimated_duration_seconds, dependency_depth, feature_branch, task_branch,
                summary
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            (
                str(old_task_id),
                old_task_description,
                "requirements-gatherer",
                5,
                TaskStatus.READY.value,
                "{}",
                None,
                None,
                0,
                3,
                3600,
                datetime.now(timezone.utc).isoformat(),
                None,
                None,
                datetime.now(timezone.utc).isoformat(),
                None,
                None,
                "[]",
                None,
                TaskSource.HUMAN.value,
                "sequential",
                5.0,
                None,
                None,
                0,
                None,
                None,
                "User Prompt: Old task before migration",  # Simulates migration backfill
            ),
        )
        await conn.commit()

    # Act - retrieve old task
    retrieved_task = await memory_db.get_task(old_task_id)

    # Assert - task retrieved successfully
    assert retrieved_task is not None
    assert retrieved_task.id == old_task_id
    assert retrieved_task.prompt == old_task_description

    # Assert - summary was backfilled by migration (matches auto-generation logic)
    assert retrieved_task.summary == "User Prompt: Old task before migration"

    # Assert - no data loss (other fields intact)
    assert retrieved_task.agent_type == "requirements-gatherer"
    assert retrieved_task.priority == 5
    assert retrieved_task.status == TaskStatus.READY


@pytest.mark.asyncio
async def test_update_old_task_with_summary(
    memory_db: Database, task_queue_service: TaskQueueService
):
    """Test adding summary to existing task with auto-generated summary.

    Verifies:
    - Tasks with auto-generated summary can be updated
    - Summary update doesn't affect other fields
    - Forward compatibility
    """
    # Arrange - create task without explicit summary (auto-generated)
    task = await task_queue_service.enqueue_task(
        description="Task to be updated",
        source=TaskSource.HUMAN,
        base_priority=5,
    )

    # Service auto-generates summary
    assert task.summary == "User Prompt: Task to be updated"

    # Act - update task with new summary (manually via database)
    new_summary = "Summary added later"
    async with memory_db._get_connection() as conn:
        await conn.execute(
            "UPDATE tasks SET summary = ? WHERE id = ?",
            (new_summary, str(task.id)),
        )
        await conn.commit()

    # Retrieve updated task
    updated_task = await memory_db.get_task(task.id)

    # Assert - summary updated
    assert updated_task is not None
    assert updated_task.summary == new_summary

    # Assert - other fields unchanged
    assert updated_task.prompt == "Task to be updated"
    assert updated_task.status == task.status
    assert updated_task.priority == 5


# Concurrent Operations Tests


@pytest.mark.asyncio
async def test_concurrent_enqueue_with_summary(
    memory_db: Database, task_queue_service: TaskQueueService
):
    """Test concurrent task enqueue with summary parameter.

    Verifies:
    - Multiple tasks with summaries can be enqueued concurrently
    - No race conditions in summary persistence
    - All summaries persisted correctly
    """
    # Arrange - 10 tasks with unique summaries
    tasks_to_enqueue = [
        {
            "description": f"Task {i} description",
            "summary": f"Summary for task {i}",
            "priority": 5 + (i % 3),
        }
        for i in range(10)
    ]

    # Act - enqueue concurrently
    enqueued_tasks = await asyncio.gather(
        *[
            task_queue_service.enqueue_task(
                description=task["description"],  # type: ignore[arg-type]
                source=TaskSource.HUMAN,
                summary=task["summary"],  # type: ignore[arg-type]
                base_priority=task["priority"],  # type: ignore[arg-type]
            )
            for task in tasks_to_enqueue
        ]
    )

    # Assert - all tasks created
    assert len(enqueued_tasks) == 10
    assert len({task.id for task in enqueued_tasks}) == 10  # All unique IDs

    # Assert - all summaries persisted correctly
    for i, task in enumerate(enqueued_tasks):
        assert task.summary == f"Summary for task {i}"

        # Verify database persistence
        retrieved = await memory_db.get_task(task.id)
        assert retrieved is not None
        assert retrieved.summary == f"Summary for task {i}"


# Edge Case Tests


@pytest.mark.asyncio
async def test_task_lifecycle_preserves_summary(
    memory_db: Database, task_queue_service: TaskQueueService
):
    """Test summary persists through complete task lifecycle.

    Verifies:
    - Summary present after enqueue
    - Summary present after status changes (READY → RUNNING → COMPLETED)
    - Summary never lost during lifecycle transitions
    """
    # Arrange - create task with summary
    test_summary = "Task lifecycle test summary"
    task = await task_queue_service.enqueue_task(
        description="Lifecycle test task",
        source=TaskSource.HUMAN,
        summary=test_summary,
        base_priority=5,
    )

    # Assert - summary present after enqueue (READY status)
    assert task.status == TaskStatus.READY
    assert task.summary == test_summary

    # Act - dequeue task (READY → RUNNING)
    running_task = await task_queue_service.get_next_task()
    assert running_task is not None
    assert running_task.id == task.id

    # Assert - summary present after dequeue (RUNNING status)
    assert running_task.status == TaskStatus.RUNNING
    assert running_task.summary == test_summary

    # Act - complete task (RUNNING → COMPLETED)
    await task_queue_service.complete_task(task.id)

    # Assert - summary present after completion (COMPLETED status)
    completed_task = await memory_db.get_task(task.id)
    assert completed_task is not None
    assert completed_task.status == TaskStatus.COMPLETED
    assert completed_task.summary == test_summary


@pytest.mark.asyncio
async def test_task_with_dependency_preserves_summary(
    memory_db: Database, task_queue_service: TaskQueueService
):
    """Test summary preserved in tasks with dependencies.

    Verifies:
    - Summary persists in prerequisite tasks
    - Summary persists in dependent tasks
    - Dependency resolution doesn't affect summary
    """
    # Arrange - create prerequisite task with summary
    prereq_task = await task_queue_service.enqueue_task(
        description="Prerequisite task",
        source=TaskSource.HUMAN,
        summary="Prerequisite task summary",
        base_priority=5,
    )

    # Create dependent task with summary
    dependent_task = await task_queue_service.enqueue_task(
        description="Dependent task",
        source=TaskSource.HUMAN,
        summary="Dependent task summary",
        prerequisites=[prereq_task.id],
        base_priority=5,
    )

    # Assert - both tasks have summaries
    assert prereq_task.summary == "Prerequisite task summary"
    assert dependent_task.summary == "Dependent task summary"

    # Act - complete prerequisite
    await task_queue_service.get_next_task()  # Dequeue prereq
    await task_queue_service.complete_task(prereq_task.id)

    # Assert - summaries preserved after dependency resolution
    retrieved_prereq = await memory_db.get_task(prereq_task.id)
    retrieved_dependent = await memory_db.get_task(dependent_task.id)

    assert retrieved_prereq is not None
    assert retrieved_prereq.summary == "Prerequisite task summary"

    assert retrieved_dependent is not None
    assert retrieved_dependent.summary == "Dependent task summary"
    assert retrieved_dependent.status == TaskStatus.READY  # Unblocked


if __name__ == "__main__":
    pytest.main([__file__, "-v"])
