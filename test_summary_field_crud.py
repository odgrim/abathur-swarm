"""Test summary field CRUD operations in database.

This test verifies:
1. insert_task includes summary field
2. _row_to_task retrieves summary correctly
3. Summary field is properly stored and retrieved (round-trip test)
"""

import asyncio
from datetime import datetime, timezone
from pathlib import Path
from uuid import uuid4

from abathur.domain.models import Task, TaskSource, TaskStatus, DependencyType
from abathur.infrastructure.database import Database


async def test_summary_field_crud():
    """Test that summary field works in all CRUD operations."""
    # Create in-memory database
    db = Database(Path(":memory:"))
    await db.initialize()

    # Test 1: Insert task with summary
    print("Test 1: Insert task with summary...")
    task_id = uuid4()
    task = Task(
        id=task_id,
        prompt="Implement feature X",
        summary="Add user authentication to the API",
        agent_type="python-backend-specialist",
        priority=5,
        status=TaskStatus.PENDING,
        input_data={"key": "value"},
        result_data=None,
        error_message=None,
        retry_count=0,
        max_retries=3,
        max_execution_timeout_seconds=3600,
        submitted_at=datetime.now(timezone.utc),
        started_at=None,
        completed_at=None,
        last_updated_at=datetime.now(timezone.utc),
        created_by="test_user",
        parent_task_id=None,
        dependencies=[],
        session_id=None,  # No session_id to avoid FK constraint
        source=TaskSource.HUMAN,
        dependency_type=DependencyType.SEQUENTIAL,
        calculated_priority=5.0,
        deadline=None,
        estimated_duration_seconds=None,
        dependency_depth=0,
        feature_branch="feature/auth",
        task_branch=None,
    )

    await db.insert_task(task)
    print("✅ Task inserted successfully")

    # Test 2: Retrieve task and verify summary
    print("\nTest 2: Retrieve task and verify summary...")
    retrieved_task = await db.get_task(task_id)
    assert retrieved_task is not None, "Task should be retrievable"
    assert retrieved_task.summary == "Add user authentication to the API", (
        f"Summary mismatch: expected 'Add user authentication to the API', "
        f"got '{retrieved_task.summary}'"
    )
    print(f"✅ Summary retrieved correctly: '{retrieved_task.summary}'")

    # Test 3: Insert task without summary (None)
    print("\nTest 3: Insert task without summary (None)...")
    task_id_2 = uuid4()
    task_2 = Task(
        id=task_id_2,
        prompt="Another task",
        summary=None,  # No summary
        agent_type="general",
        priority=5,
        status=TaskStatus.PENDING,
        input_data={},
        result_data=None,
        error_message=None,
        retry_count=0,
        max_retries=3,
        max_execution_timeout_seconds=3600,
        submitted_at=datetime.now(timezone.utc),
        started_at=None,
        completed_at=None,
        last_updated_at=datetime.now(timezone.utc),
        created_by="test_user",
        parent_task_id=None,
        dependencies=[],
        session_id=None,
        source=TaskSource.HUMAN,
        dependency_type=DependencyType.SEQUENTIAL,
        calculated_priority=5.0,
        deadline=None,
        estimated_duration_seconds=None,
        dependency_depth=0,
        feature_branch=None,
        task_branch=None,
    )

    await db.insert_task(task_2)
    print("✅ Task without summary inserted successfully")

    # Test 4: Retrieve task without summary
    print("\nTest 4: Retrieve task without summary...")
    retrieved_task_2 = await db.get_task(task_id_2)
    assert retrieved_task_2 is not None, "Task should be retrievable"
    assert retrieved_task_2.summary is None, (
        f"Summary should be None, got '{retrieved_task_2.summary}'"
    )
    print("✅ Task without summary retrieved correctly (summary=None)")

    # Test 5: Verify _create_core_tables includes summary column
    print("\nTest 5: Verify summary column exists in database schema...")
    async with db._get_connection() as conn:
        cursor = await conn.execute("PRAGMA table_info(tasks)")
        columns = await cursor.fetchall()
        column_names = [col["name"] for col in columns]
        assert "summary" in column_names, "summary column should exist in tasks table"
    print("✅ Summary column exists in database schema")

    # Test 6: Round-trip test with special characters
    print("\nTest 6: Round-trip test with special characters in summary...")
    task_id_3 = uuid4()
    special_summary = "Fix bug: API returns 500 when user's name contains 'quotes' and \"double quotes\""
    task_3 = Task(
        id=task_id_3,
        prompt="Fix API bug",
        summary=special_summary,
        agent_type="general",
        priority=5,
        status=TaskStatus.PENDING,
        input_data={},
        result_data=None,
        error_message=None,
        retry_count=0,
        max_retries=3,
        max_execution_timeout_seconds=3600,
        submitted_at=datetime.now(timezone.utc),
        started_at=None,
        completed_at=None,
        last_updated_at=datetime.now(timezone.utc),
        created_by="test_user",
        parent_task_id=None,
        dependencies=[],
        session_id=None,
        source=TaskSource.HUMAN,
        dependency_type=DependencyType.SEQUENTIAL,
        calculated_priority=5.0,
        deadline=None,
        estimated_duration_seconds=None,
        dependency_depth=0,
        feature_branch=None,
        task_branch=None,
    )

    await db.insert_task(task_3)
    retrieved_task_3 = await db.get_task(task_id_3)
    assert retrieved_task_3 is not None
    assert retrieved_task_3.summary == special_summary, (
        f"Summary with special chars mismatch: "
        f"expected '{special_summary}', got '{retrieved_task_3.summary}'"
    )
    print("✅ Special characters preserved correctly in round-trip")

    await db.close()
    print("\n🎉 All tests passed! Summary field CRUD operations work correctly.")


if __name__ == "__main__":
    asyncio.run(test_summary_field_crud())
