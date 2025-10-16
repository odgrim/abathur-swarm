#!/usr/bin/env python3
"""Test script to verify summary column migration idempotency and functionality."""

import asyncio
import json
from datetime import datetime, timezone
from pathlib import Path
from uuid import uuid4

from abathur.domain.models import Task, TaskStatus, TaskSource, DependencyType
from abathur.infrastructure.database import Database


async def test_summary_migration():
    """Test summary column migration."""
    # Use temporary in-memory database
    db_path = Path(":memory:")
    db = Database(db_path)

    print("=" * 80)
    print("Test 1: Initialize database and run migration")
    print("=" * 80)
    await db.initialize()
    print("✓ Database initialized successfully\n")

    # Verify summary column exists
    async with db._get_connection() as conn:
        cursor = await conn.execute("PRAGMA table_info(tasks)")
        columns = await cursor.fetchall()
        column_names = [col["name"] for col in columns]

        if "summary" in column_names:
            print("✓ Summary column exists in tasks table")
        else:
            print("✗ FAILED: Summary column not found!")
            return False

    print("\n" + "=" * 80)
    print("Test 2: Test migration idempotency (run twice)")
    print("=" * 80)
    try:
        # Run migration again by re-initializing (should be idempotent)
        await db._run_migrations(db._shared_conn)
        print("✓ Migration is idempotent (no errors on second run)\n")
    except Exception as e:
        print(f"✗ FAILED: Migration not idempotent: {e}")
        return False

    print("=" * 80)
    print("Test 3: Insert task WITH summary")
    print("=" * 80)
    task_with_summary = Task(
        id=uuid4(),
        prompt="Test task with summary",
        agent_type="test-agent",
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
        created_by="test",
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
        summary="This is a test summary",
    )

    await db.insert_task(task_with_summary)
    print(f"✓ Inserted task with summary: '{task_with_summary.summary}'")

    # Retrieve and verify
    retrieved_task = await db.get_task(task_with_summary.id)
    if retrieved_task and retrieved_task.summary == "This is a test summary":
        print(f"✓ Retrieved task has correct summary: '{retrieved_task.summary}'\n")
    else:
        print(f"✗ FAILED: Summary mismatch or not retrieved. Got: {retrieved_task.summary if retrieved_task else 'None'}")
        return False

    print("=" * 80)
    print("Test 4: Insert task WITHOUT summary (backward compatibility)")
    print("=" * 80)
    task_without_summary = Task(
        id=uuid4(),
        prompt="Test task without summary",
        agent_type="test-agent",
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
        created_by="test",
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
        summary=None,
    )

    await db.insert_task(task_without_summary)
    print("✓ Inserted task with summary=None")

    # Retrieve and verify
    retrieved_task = await db.get_task(task_without_summary.id)
    if retrieved_task and retrieved_task.summary is None:
        print("✓ Retrieved task has summary=None (backward compatible)\n")
    else:
        print(f"✗ FAILED: Expected summary=None, got: {retrieved_task.summary if retrieved_task else 'None'}")
        return False

    print("=" * 80)
    print("Test 5: List tasks and verify summary field")
    print("=" * 80)
    tasks = await db.list_tasks(limit=10)
    print(f"✓ Retrieved {len(tasks)} tasks")
    for task in tasks:
        print(f"  - Task {task.id}: summary='{task.summary}'")
    print()

    print("=" * 80)
    print("ALL TESTS PASSED! ✓")
    print("=" * 80)
    print("\nMigration Summary:")
    print("- Migration is idempotent (can run multiple times)")
    print("- Tasks can be inserted with summary field")
    print("- Tasks can be inserted without summary (backward compatible)")
    print("- Summary field is correctly persisted and retrieved")
    print("- Ready for service layer integration\n")

    await db.close()
    return True


if __name__ == "__main__":
    success = asyncio.run(test_summary_migration())
    exit(0 if success else 1)
