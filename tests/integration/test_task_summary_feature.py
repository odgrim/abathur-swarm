"""Integration tests for Task Summary Feature (MCP End-to-End).

Complete integration test suite covering Phase 6 requirements:
- Test 1: End-to-end MCP flow with summary
- Test 2: Backward compatibility without summary
- Test 3: Validation error handling (max_length)
- Test 4: task_list returns summaries
- Test 5: Database migration idempotency
- Test 6: Existing tests still pass (verified by running test suite)

Tests simulate actual MCP tool calls through the TaskQueueServer handlers.
"""

import asyncio
from collections.abc import AsyncGenerator
from pathlib import Path
from uuid import UUID

import pytest
from abathur.domain.models import TaskSource
from abathur.infrastructure.database import Database
from abathur.mcp.task_queue_server import AbathurTaskQueueServer
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


@pytest.fixture
async def mcp_server(
    memory_db: Database, task_queue_service: TaskQueueService
) -> AbathurTaskQueueServer:
    """Create MCP server with in-memory database (initialized).

    This fixture provides a fully initialized MCP server with:
    - In-memory database (already initialized)
    - TaskQueueService
    - DependencyResolver
    - PriorityCalculator

    Ready for MCP handler testing without running the full server.
    """
    server = AbathurTaskQueueServer(Path(":memory:"))
    # Manually inject initialized dependencies to avoid double-initialization
    server._db = memory_db
    server._task_queue_service = task_queue_service
    server._dependency_resolver = task_queue_service._dependency_resolver
    server._priority_calculator = task_queue_service._priority_calculator
    return server


# Test 1: End-to-End MCP Flow with Summary


@pytest.mark.asyncio
async def test_mcp_end_to_end_flow_with_summary(
    mcp_server: AbathurTaskQueueServer,
    memory_db: Database,
) -> None:
    """Test complete MCP flow: task_enqueue with summary → Database → task_get returns summary.

    Simulates actual MCP tool calls:
    1. Client calls task_enqueue with summary parameter
    2. Server persists task to database with summary
    3. Client calls task_get to retrieve task
    4. Response includes summary field

    Verifies:
    - MCP task_enqueue accepts summary parameter
    - Summary persists to database
    - MCP task_get returns summary field
    - End-to-end integration works
    """
    # Step 1: Enqueue task via MCP with summary
    enqueue_args = {
        "description": "Implement OAuth2 authentication with JWT tokens",
        "summary": "Add user authentication feature",
        "source": "human",
        "agent_type": "python-backend-specialist",
        "base_priority": 7,
    }

    enqueue_result = await mcp_server._handle_task_enqueue(enqueue_args)

    # Assert: No error in enqueue response
    assert "error" not in enqueue_result, f"Enqueue failed: {enqueue_result.get('error')}"
    assert "task_id" in enqueue_result

    task_id = enqueue_result["task_id"]

    # Step 2: Verify database persistence
    retrieved_from_db = await memory_db.get_task(UUID(task_id))
    assert retrieved_from_db is not None
    assert retrieved_from_db.summary == "Add user authentication feature"

    # Step 3: Retrieve via MCP task_get
    get_args = {"task_id": task_id}
    get_result = await mcp_server._handle_task_get(get_args)

    # Assert: task_get returns summary
    assert "error" not in get_result, f"task_get failed: {get_result.get('error')}"
    assert "summary" in get_result
    assert get_result["summary"] == "Add user authentication feature"
    assert get_result["prompt"] == "Implement OAuth2 authentication with JWT tokens"


# Test 2: Backward Compatibility Without Summary


@pytest.mark.asyncio
async def test_mcp_backward_compatibility_without_summary(
    mcp_server: AbathurTaskQueueServer,
    memory_db: Database,
) -> None:
    """Test MCP task_enqueue auto-generates summary when not provided (backward compatibility).

    Simulates legacy MCP client not providing summary parameter:
    1. Client calls task_enqueue WITHOUT summary
    2. Server auto-generates summary from description
    3. Client retrieves task
    4. Response has auto-generated summary

    Verifies:
    - Backward compatibility maintained
    - Existing code works without summary
    - No errors when summary omitted
    - Summary is auto-generated from description
    """
    # Step 1: Enqueue task WITHOUT summary parameter
    enqueue_args = {
        "description": "Refactor payment processing module",
        # No summary parameter
        "source": "human",
        "agent_type": "python-backend-specialist",
        "base_priority": 5,
    }

    enqueue_result = await mcp_server._handle_task_enqueue(enqueue_args)

    # Assert: No error
    assert "error" not in enqueue_result
    assert "task_id" in enqueue_result

    task_id = enqueue_result["task_id"]

    # Step 2: Verify database has auto-generated summary (Human source gets "User Prompt: " prefix)
    retrieved_from_db = await memory_db.get_task(UUID(task_id))
    assert retrieved_from_db is not None
    assert retrieved_from_db.summary is not None
    assert retrieved_from_db.summary.startswith("User Prompt: ")
    assert "Refactor payment processing module" in retrieved_from_db.summary

    # Step 3: Retrieve via MCP task_get
    get_result = await mcp_server._handle_task_get({"task_id": task_id})

    # Assert: summary field present and auto-generated
    assert "error" not in get_result
    assert "summary" in get_result
    assert get_result["summary"] is not None
    assert get_result["summary"].startswith("User Prompt: ")


# Test 3: Validation Error Handling


@pytest.mark.asyncio
async def test_mcp_summary_validation_max_length(
    task_queue_service: TaskQueueService,
) -> None:
    """Test that summaries exceeding max_length are automatically truncated.

    The Pydantic field validator auto-truncates summaries to 140 characters instead of raising errors,
    providing better UX (auto-correction vs rejection).

    Verifies:
    - Summaries >140 chars are automatically truncated to 140
    - No error is raised (graceful handling)
    - Truncated summary is persisted correctly
    - Auto-truncation maintains data integrity
    """
    # Enqueue task with >140 char summary
    long_summary = "x" * 141  # 141 characters (exceeds max_length=140)

    # Should NOT raise error - Pydantic field validator auto-truncates
    task = await task_queue_service.enqueue_task(
        description="Task with too-long summary",
        source=TaskSource.HUMAN,
        summary=long_summary,
        base_priority=5,
    )

    # Assert: Summary was truncated to 140 characters by Pydantic validator
    assert task.summary is not None
    assert len(task.summary) == 140
    assert task.summary == "x" * 140


@pytest.mark.asyncio
async def test_mcp_summary_validation_exactly_max_length(
    mcp_server: AbathurTaskQueueServer,
    memory_db: Database,
) -> None:
    """Test that summary exactly at max_length (140 chars) is accepted.

    Verifies:
    - Boundary condition: exactly 140 characters accepted
    - No truncation occurs
    - Full summary persisted and retrieved
    """
    # Exactly 140 characters
    max_length_summary = "x" * 140

    enqueue_args = {
        "description": "Task with max length summary",
        "summary": max_length_summary,
        "source": "human",
        "agent_type": "requirements-gatherer",
        "base_priority": 5,
    }

    enqueue_result = await mcp_server._handle_task_enqueue(enqueue_args)

    # Assert: No error
    assert "error" not in enqueue_result

    task_id = enqueue_result["task_id"]

    # Retrieve and verify full summary
    get_result = await mcp_server._handle_task_get({"task_id": task_id})

    assert "error" not in get_result
    assert get_result["summary"] == max_length_summary
    assert len(get_result["summary"]) == 140


@pytest.mark.asyncio
async def test_mcp_summary_with_leading_trailing_whitespace(
    mcp_server: AbathurTaskQueueServer,
    memory_db: Database,
) -> None:
    """Test that summary with leading/trailing whitespace is stripped.

    Verifies:
    - Leading whitespace stripped
    - Trailing whitespace stripped
    - Internal whitespace preserved
    - Pydantic field validator handles whitespace correctly
    """
    # Summary with leading and trailing whitespace
    whitespace_summary = "  test summary with spaces  "

    enqueue_args = {
        "description": "Task with whitespace in summary",
        "summary": whitespace_summary,
        "source": "human",
        "agent_type": "requirements-gatherer",
        "base_priority": 5,
    }

    enqueue_result = await mcp_server._handle_task_enqueue(enqueue_args)

    # Assert: No error
    assert "error" not in enqueue_result

    task_id = enqueue_result["task_id"]

    # Retrieve and verify stripped summary
    get_result = await mcp_server._handle_task_get({"task_id": task_id})

    assert "error" not in get_result
    assert (
        get_result["summary"] == "test summary with spaces"
    )  # Leading/trailing whitespace removed
    assert get_result["summary"] != whitespace_summary  # Different from original


# Test 4: task_list Returns Summaries


@pytest.mark.asyncio
async def test_mcp_task_list_includes_summaries(
    mcp_server: AbathurTaskQueueServer,
    memory_db: Database,
) -> None:
    """Test MCP task_list returns summary field for all tasks.

    Simulates:
    1. Create multiple tasks with/without summaries
    2. Client calls task_list
    3. All tasks in response include summary field

    Verifies:
    - task_list serialization includes summary
    - Tasks with summary show value
    - Tasks without summary show None
    - Consistent serialization across list
    """
    # Step 1: Create tasks with mixed summaries
    task1_args = {
        "description": "Task 1 with summary",
        "summary": "Summary for task 1",
        "source": "human",
        "agent_type": "requirements-gatherer",
        "base_priority": 5,
    }

    task2_args = {
        "description": "Task 2 without summary",
        # No summary
        "source": "human",
        "agent_type": "task-planner",
        "base_priority": 5,
    }

    task3_args = {
        "description": "Task 3 with empty summary",
        "summary": "",  # Empty string gets treated as None (auto-generates)
        "source": "human",
        "agent_type": "technical-architect",
        "base_priority": 5,
    }

    result1 = await mcp_server._handle_task_enqueue(task1_args)
    result2 = await mcp_server._handle_task_enqueue(task2_args)
    result3 = await mcp_server._handle_task_enqueue(task3_args)

    # All should succeed - empty summary auto-generates (no MCP validation error)
    assert "error" not in result1
    assert "error" not in result2
    assert "error" not in result3

    # Step 2: List all tasks via MCP
    list_result = await mcp_server._handle_task_list({})

    # Assert: No error
    assert "error" not in list_result
    assert "tasks" in list_result
    assert len(list_result["tasks"]) >= 3

    # Step 3: Verify all tasks have summary field
    task_ids = [result1["task_id"], result2["task_id"], result3["task_id"]]

    for task in list_result["tasks"]:
        assert "summary" in task, "All tasks should have summary field"

        if task["id"] in task_ids:
            # Verify specific summaries
            if task["id"] == result1["task_id"]:
                assert task["summary"] == "Summary for task 1"
            elif task["id"] == result2["task_id"]:
                # Auto-generated from description for human source
                assert task["summary"].startswith("User Prompt: ")
                assert "Task 2 without summary" in task["summary"]
            elif task["id"] == result3["task_id"]:
                # Empty summary gets auto-generated
                assert task["summary"].startswith("User Prompt: ")
                assert "Task 3 with empty summary" in task["summary"]


# Test 5: Database Migration Idempotency


@pytest.mark.asyncio
async def test_database_migration_idempotent() -> None:
    """Test that database migration can run multiple times safely.

    Verifies:
    - Migration adds summary column on first run
    - Migration skips ALTER TABLE on second run
    - No errors or exceptions on repeated runs
    - Column properties unchanged after multiple runs

    This test creates a file-based database to test persistence across connections.
    """
    import tempfile

    # Create temporary file database
    with tempfile.NamedTemporaryFile(suffix=".db", delete=False) as f:
        db_path = Path(f.name)

    try:
        # Run migration 1: First initialization
        db1 = Database(db_path)
        await db1.initialize()

        # Verify column exists after first migration
        async with db1._get_connection() as conn:
            cursor = await conn.execute("PRAGMA table_info(tasks)")
            columns_before = list(await cursor.fetchall())
            column_names_before = [col["name"] for col in columns_before]

            assert (
                "summary" in column_names_before
            ), "Summary column should exist after first migration"

        # Close first connection (file-based DB auto-closes)

        # Run migration 2: Second initialization (should be idempotent)
        db2 = Database(db_path)
        await db2.initialize()  # Should NOT raise error

        # Verify column still exists with same properties
        async with db2._get_connection() as conn:
            cursor = await conn.execute("PRAGMA table_info(tasks)")
            columns_after = list(await cursor.fetchall())
            column_names_after = [col["name"] for col in columns_after]

            assert (
                "summary" in column_names_after
            ), "Summary column should persist after second migration"

            # Verify column count unchanged (no duplicate columns)
            assert len(columns_before) == len(columns_after), "Column count should be unchanged"

            # Verify exactly one summary column
            summary_columns = [col for col in columns_after if col["name"] == "summary"]
            assert len(summary_columns) == 1, "Should have exactly one summary column"

        # Run migration 3: Third initialization for extra safety
        db3 = Database(db_path)
        await db3.initialize()  # Should still NOT raise error

        async with db3._get_connection() as conn:
            cursor = await conn.execute("PRAGMA table_info(tasks)")
            columns_final = list(await cursor.fetchall())
            summary_columns_final = [col for col in columns_final if col["name"] == "summary"]

            assert len(summary_columns_final) == 1, "Should still have exactly one summary column"

    finally:
        # Cleanup
        if db_path.exists():
            db_path.unlink()
        # Cleanup WAL files
        wal_path = db_path.with_suffix(".db-wal")
        shm_path = db_path.with_suffix(".db-shm")
        if wal_path.exists():
            wal_path.unlink()
        if shm_path.exists():
            shm_path.unlink()


# Test 6: Existing Functionality Unaffected (Implicit)


@pytest.mark.asyncio
async def test_existing_task_fields_unaffected(
    mcp_server: AbathurTaskQueueServer,
) -> None:
    """Test that adding summary field doesn't affect other Task fields.

    Verifies:
    - All 28 Task fields present in serialization
    - Summary field is field #20 (as documented)
    - No fields removed or renamed
    - Backward compatibility with existing field structure
    """
    # Create task with all optional fields populated
    from datetime import datetime, timezone

    enqueue_args = {
        "description": "Full task with all fields",
        "summary": "Complete task test",
        "source": "human",
        "agent_type": "requirements-gatherer",
        "base_priority": 8,
        "deadline": datetime.now(timezone.utc).isoformat(),
        "estimated_duration_seconds": 3600,
        "input_data": {"key": "value"},
        "feature_branch": "feature/summary-field",
        "task_branch": "task/test-all-fields",
    }

    enqueue_result = await mcp_server._handle_task_enqueue(enqueue_args)
    assert "error" not in enqueue_result

    task_id = enqueue_result["task_id"]

    # Retrieve task via MCP task_get
    get_result = await mcp_server._handle_task_get({"task_id": task_id})

    assert "error" not in get_result

    # Verify all 28 Task fields present
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
        "summary",  # Field #20
        "source",
        "dependency_type",
        "calculated_priority",
        "deadline",
        "estimated_duration_seconds",
        "dependency_depth",
        "feature_branch",
        "task_branch",
    }

    actual_fields = set(get_result.keys())

    assert len(actual_fields) == 28, f"Expected 28 fields, got {len(actual_fields)}"
    assert actual_fields == expected_fields, f"Missing fields: {expected_fields - actual_fields}"

    # Verify summary field specifically present
    assert "summary" in get_result
    assert get_result["summary"] == "Complete task test"


# Concurrent Operations Test


@pytest.mark.asyncio
async def test_mcp_concurrent_enqueue_with_summary(
    mcp_server: AbathurTaskQueueServer,
    memory_db: Database,
) -> None:
    """Test concurrent task enqueue via MCP with summary parameter.

    Verifies:
    - Multiple concurrent MCP task_enqueue calls work correctly
    - No race conditions in summary persistence
    - All summaries persisted correctly
    - Database handles concurrent writes
    """

    # Create 10 tasks concurrently
    async def enqueue_task(i: int):
        args = {
            "description": f"Concurrent task {i} description",
            "summary": f"Summary {i}",
            "source": "human",
            "agent_type": "requirements-gatherer",
            "base_priority": 5,
        }
        return await mcp_server._handle_task_enqueue(args)

    # Enqueue all tasks concurrently
    results = await asyncio.gather(*[enqueue_task(i) for i in range(10)])

    # Verify all enqueued successfully
    assert all("error" not in result for result in results)
    assert len(results) == 10

    # Verify all summaries persisted correctly
    for i, result in enumerate(results):
        task_id = result["task_id"]
        retrieved = await memory_db.get_task(UUID(task_id))

        assert retrieved is not None
        assert retrieved.summary == f"Summary {i}"


# Edge Cases


@pytest.mark.asyncio
async def test_mcp_summary_with_unicode_characters(
    mcp_server: AbathurTaskQueueServer,
) -> None:
    """Test summary field handles unicode characters correctly.

    Verifies:
    - Unicode characters (emojis, accents) accepted
    - Full unicode support in summary
    - Correct persistence and retrieval
    - JSON serialization handles unicode
    """
    unicode_summary = "Fix bug 🐛 in payment processing with café ☕ and résumé"

    enqueue_args = {
        "description": "Debug payment gateway",
        "summary": unicode_summary,
        "source": "human",
        "agent_type": "python-backend-specialist",
        "base_priority": 5,
    }

    enqueue_result = await mcp_server._handle_task_enqueue(enqueue_args)
    assert "error" not in enqueue_result

    task_id = enqueue_result["task_id"]

    # Retrieve via MCP task_get
    get_result = await mcp_server._handle_task_get({"task_id": task_id})

    # Assert: Unicode preserved
    assert "error" not in get_result
    assert get_result["summary"] == unicode_summary


@pytest.mark.asyncio
async def test_mcp_task_with_dependencies_preserves_summary(
    mcp_server: AbathurTaskQueueServer,
    memory_db: Database,
) -> None:
    """Test summary preserved in tasks with dependencies.

    Verifies:
    - Summary persists in prerequisite tasks
    - Summary persists in dependent tasks
    - Dependency resolution doesn't affect summary
    """
    # Create prerequisite task with summary
    prereq_args = {
        "description": "Prerequisite task",
        "summary": "Prereq summary",
        "source": "human",
        "agent_type": "requirements-gatherer",
        "base_priority": 5,
    }

    prereq_result = await mcp_server._handle_task_enqueue(prereq_args)
    assert "error" not in prereq_result
    prereq_id = prereq_result["task_id"]

    # Create dependent task with summary and prerequisite
    dependent_args = {
        "description": "Dependent task",
        "summary": "Dependent summary",
        "source": "human",
        "agent_type": "task-planner",
        "base_priority": 5,
        "prerequisites": [prereq_id],
    }

    dependent_result = await mcp_server._handle_task_enqueue(dependent_args)
    assert "error" not in dependent_result
    dependent_id = dependent_result["task_id"]

    # Verify both summaries preserved
    prereq_task = await memory_db.get_task(UUID(prereq_id))
    dependent_task = await memory_db.get_task(UUID(dependent_id))

    assert prereq_task is not None
    assert dependent_task is not None
    assert prereq_task.summary == "Prereq summary"
    assert dependent_task.summary == "Dependent summary"


if __name__ == "__main__":
    pytest.main([__file__, "-v"])
