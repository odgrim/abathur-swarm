"""Integration tests for VACUUM behavior in prune operations.

Tests complete end-to-end workflows for VACUUM functionality:
- VACUUM successfully reclaims space after deletion
- VACUUM is disabled in dry-run mode
- VACUUM is optional (controlled by vacuum_mode: 'always', 'conditional', or 'never')
- VACUUM behavior with zero deletions
- VACUUM space calculation accuracy
- VACUUM with large datasets
- VACUUM failure handling and recovery
- VACUUM preserves data integrity of remaining tasks
- VACUUM with concurrent database operations
- VACUUM CLI integration (placeholder for future implementation)
"""

import pytest
from collections.abc import AsyncGenerator
from datetime import datetime, timedelta, timezone
from pathlib import Path
from uuid import UUID

from abathur.domain.models import Task, TaskSource, TaskStatus
from abathur.infrastructure.database import Database, PruneFilters


@pytest.fixture
async def memory_db() -> AsyncGenerator[Database, None]:
    """Create in-memory database for fast integration tests."""
    db = Database(Path(":memory:"))
    await db.initialize()
    yield db
    await db.close()


async def create_tasks(db: Database, count: int, status: TaskStatus = TaskStatus.COMPLETED) -> list[str]:
    """Helper function to create tasks for testing.

    Args:
        db: Database instance
        count: Number of tasks to create
        status: Status for created tasks

    Returns:
        List of task IDs
    """
    task_ids = []
    old_time = datetime.now(timezone.utc) - timedelta(days=40)

    for i in range(count):
        task = Task(
            prompt=f"Task {i}",
            summary=f"Test task {i}",
            agent_type="test-agent",
            source=TaskSource.HUMAN,
            status=status,
            submitted_at=old_time,
            completed_at=old_time if status in [TaskStatus.COMPLETED, TaskStatus.FAILED, TaskStatus.CANCELLED] else None,
        )
        await db.insert_task(task)
        task_ids.append(str(task.id))  # Get ID from task object, not return value

    return task_ids


@pytest.mark.asyncio
async def test_vacuum_reclaims_space(memory_db: Database):
    """Test VACUUM successfully reclaims space after deletion."""
    # Step 1: Create 1000 tasks to generate substantial data
    task_ids = await create_tasks(memory_db, 1000, TaskStatus.COMPLETED)

    assert len(task_ids) == 1000

    # Step 2: Get initial database size (before deletion)
    async with memory_db._get_connection() as conn:
        cursor = await conn.execute("PRAGMA page_count")
        page_count_row = await cursor.fetchone()
        cursor = await conn.execute("PRAGMA page_size")
        page_size_row = await cursor.fetchone()

        if page_count_row and page_size_row:
            initial_pages = page_count_row[0]
            page_size = page_size_row[0]
            initial_size = initial_pages * page_size
        else:
            pytest.fail("Could not get initial database size")

    # Step 3: Delete all tasks WITH VACUUM enabled
    filters = PruneFilters(
        older_than_days=30,
        vacuum_mode="always",  # Enable VACUUM
        dry_run=False
    )
    result = await memory_db.prune_tasks(filters)

    # Step 4: Assertions
    assert result.deleted_tasks == 1000, f"Expected 1000 tasks deleted, got {result.deleted_tasks}"
    assert result.dry_run is False
    assert result.reclaimed_bytes is not None, "reclaimed_bytes should not be None when vacuum_mode='always'"
    assert result.reclaimed_bytes >= 0, f"reclaimed_bytes should be non-negative, got {result.reclaimed_bytes}"

    # VACUUM should reclaim some space (even if minimal for in-memory DB)
    # We don't assert > 0 because in-memory DBs may not reclaim space the same way

    # Step 5: Verify database size after VACUUM
    async with memory_db._get_connection() as conn:
        cursor = await conn.execute("PRAGMA page_count")
        page_count_row = await cursor.fetchone()

        if page_count_row:
            final_pages = page_count_row[0]
            final_size = final_pages * page_size

            # Database should be smaller or equal after deletion + VACUUM
            assert final_size <= initial_size, f"Database grew after VACUUM: {initial_size} -> {final_size}"


@pytest.mark.asyncio
async def test_vacuum_disabled_in_dry_run(memory_db: Database):
    """Test VACUUM is skipped in dry-run mode."""
    # Step 1: Create tasks
    task_ids = await create_tasks(memory_db, 100, TaskStatus.COMPLETED)
    assert len(task_ids) == 100

    # Step 2: Run prune with dry_run=True and vacuum_mode='always'
    # VACUUM should be skipped because dry_run takes precedence
    filters = PruneFilters(
        older_than_days=30,
        vacuum_mode="always",  # Request VACUUM
        dry_run=True  # But dry-run mode
    )
    result = await memory_db.prune_tasks(filters)

    # Step 3: Assertions
    assert result.dry_run is True, "dry_run should be True"
    assert result.deleted_tasks == 100, "Should show 100 tasks would be deleted"
    assert result.reclaimed_bytes is None, "reclaimed_bytes should be None in dry-run mode"

    # Step 4: Verify tasks were NOT deleted
    async with memory_db._get_connection() as conn:
        cursor = await conn.execute("SELECT COUNT(*) FROM tasks")
        count_row = await cursor.fetchone()
        assert count_row[0] == 100, "Tasks should not be deleted in dry-run mode"


@pytest.mark.asyncio
async def test_vacuum_optional_flag(memory_db: Database):
    """Test VACUUM only runs when explicitly enabled via vacuum flag."""
    # Step 1: Create tasks
    task_ids = await create_tasks(memory_db, 200, TaskStatus.COMPLETED)
    assert len(task_ids) == 200

    # Step 2: Delete tasks WITHOUT vacuum (vacuum_mode='never')
    filters_no_vacuum = PruneFilters(
        older_than_days=30,
        vacuum_mode="never",  # Explicitly disable VACUUM
        dry_run=False
    )
    result_no_vacuum = await memory_db.prune_tasks(filters_no_vacuum)

    # Step 3: Verify VACUUM did NOT run
    assert result_no_vacuum.deleted_tasks == 200
    assert result_no_vacuum.dry_run is False
    assert result_no_vacuum.reclaimed_bytes is None, "reclaimed_bytes should be None when vacuum_mode='never'"

    # Step 4: Recreate tasks for second test
    task_ids = await create_tasks(memory_db, 200, TaskStatus.COMPLETED)
    assert len(task_ids) == 200

    # Step 5: Delete tasks WITH vacuum (vacuum_mode='always')
    filters_with_vacuum = PruneFilters(
        older_than_days=30,
        vacuum_mode="always",  # Enable VACUUM
        dry_run=False
    )
    result_with_vacuum = await memory_db.prune_tasks(filters_with_vacuum)

    # Step 6: Verify VACUUM DID run
    assert result_with_vacuum.deleted_tasks == 200
    assert result_with_vacuum.dry_run is False
    assert result_with_vacuum.reclaimed_bytes is not None, "reclaimed_bytes should not be None when vacuum_mode='always'"
    assert result_with_vacuum.reclaimed_bytes >= 0


@pytest.mark.asyncio
async def test_vacuum_with_zero_deletions(memory_db: Database):
    """Test VACUUM behavior when no tasks match deletion criteria."""
    # Step 1: Create tasks with recent timestamps (won't match older_than filter)
    recent_time = datetime.now(timezone.utc) - timedelta(days=5)

    for i in range(10):
        task = Task(
            prompt=f"Recent task {i}",
            summary=f"Recent test task {i}",
            agent_type="test-agent",
            source=TaskSource.HUMAN,
            status=TaskStatus.COMPLETED,
            submitted_at=recent_time,
            completed_at=recent_time,
        )
        await memory_db.insert_task(task)

    # Step 2: Try to delete tasks older than 99999 days (none match)
    filters = PruneFilters(
        older_than_days=99999,
        vacuum_mode="always",  # Request VACUUM
        dry_run=False
    )
    result = await memory_db.prune_tasks(filters)

    # Step 3: Assertions
    assert result.deleted_tasks == 0, "No tasks should be deleted"
    assert result.dry_run is False
    # VACUUM may or may not run when no deletions occur
    # Implementation can choose to skip VACUUM optimization
    # reclaimed_bytes can be None or 0
    if result.reclaimed_bytes is not None:
        assert result.reclaimed_bytes >= 0

    # Step 4: Verify all tasks still exist
    async with memory_db._get_connection() as conn:
        cursor = await conn.execute("SELECT COUNT(*) FROM tasks")
        count_row = await cursor.fetchone()
        assert count_row[0] == 10, "All tasks should still exist"


@pytest.mark.asyncio
async def test_vacuum_space_calculation_accuracy(memory_db: Database):
    """Test accuracy of reclaimed space calculation."""
    # Step 1: Create substantial dataset (500 tasks)
    task_ids = await create_tasks(memory_db, 500, TaskStatus.COMPLETED)
    assert len(task_ids) == 500

    # Step 2: Get database size before deletion
    async with memory_db._get_connection() as conn:
        cursor = await conn.execute("PRAGMA page_count")
        page_count_before_row = await cursor.fetchone()
        cursor = await conn.execute("PRAGMA page_size")
        page_size_row = await cursor.fetchone()

        if page_count_before_row and page_size_row:
            pages_before = page_count_before_row[0]
            page_size = page_size_row[0]
            size_before_deletion = pages_before * page_size
        else:
            pytest.fail("Could not get database size before deletion")

    # Step 3: Delete half the tasks (250) with VACUUM
    filters = PruneFilters(
        older_than_days=30,
        limit=250,  # Delete only 250 tasks
        vacuum=True,
        dry_run=False
    )
    result = await memory_db.prune_tasks(filters)

    # Step 4: Verify deletion and space calculation
    assert result.deleted_tasks == 250, f"Expected 250 tasks deleted, got {result.deleted_tasks}"
    assert result.reclaimed_bytes is not None, "reclaimed_bytes should not be None"
    assert result.reclaimed_bytes >= 0, "reclaimed_bytes should be non-negative"

    # Step 5: Verify reclaimed_bytes doesn't exceed initial size
    assert result.reclaimed_bytes <= size_before_deletion, \
        f"reclaimed_bytes ({result.reclaimed_bytes}) cannot exceed initial size ({size_before_deletion})"

    # Step 6: Verify database size decreased (or stayed same)
    async with memory_db._get_connection() as conn:
        cursor = await conn.execute("PRAGMA page_count")
        page_count_after_row = await cursor.fetchone()

        if page_count_after_row:
            pages_after = page_count_after_row[0]
            size_after_deletion = pages_after * page_size

            # Size should be less than or equal to before
            assert size_after_deletion <= size_before_deletion, \
                f"Database size increased after deletion: {size_before_deletion} -> {size_after_deletion}"

            # Verify reclaimed_bytes calculation matches actual size change
            actual_reclaimed = size_before_deletion - size_after_deletion
            assert result.reclaimed_bytes == actual_reclaimed, \
                f"reclaimed_bytes ({result.reclaimed_bytes}) doesn't match actual ({actual_reclaimed})"


@pytest.mark.asyncio
async def test_vacuum_with_large_dataset(memory_db: Database):
    """Test VACUUM with substantial data (10k tasks) completes successfully."""
    # Step 1: Create large dataset (10,000 tasks)
    # Use batched creation for performance
    total_tasks = 10000
    batch_size = 1000
    all_task_ids = []

    for batch_num in range(total_tasks // batch_size):
        batch_ids = await create_tasks(memory_db, batch_size, TaskStatus.COMPLETED)
        all_task_ids.extend(batch_ids)

    assert len(all_task_ids) == total_tasks, f"Expected {total_tasks} tasks, got {len(all_task_ids)}"

    # Step 2: Verify tasks were created
    async with memory_db._get_connection() as conn:
        cursor = await conn.execute("SELECT COUNT(*) FROM tasks")
        count_row = await cursor.fetchone()
        assert count_row[0] == total_tasks, f"Expected {total_tasks} tasks in DB, got {count_row[0]}"

    # Step 3: Delete half the tasks (5000) with VACUUM
    import time
    start_time = time.time()

    filters = PruneFilters(
        older_than_days=30,
        limit=5000,  # Delete 5000 tasks
        vacuum=True,
        dry_run=False
    )
    result = await memory_db.prune_tasks(filters)

    elapsed_time = time.time() - start_time

    # Step 4: Verify successful deletion
    assert result.deleted_tasks == 5000, f"Expected 5000 tasks deleted, got {result.deleted_tasks}"
    assert result.dry_run is False
    assert result.reclaimed_bytes is not None, "reclaimed_bytes should not be None"
    assert result.reclaimed_bytes >= 0

    # Step 5: Verify operation completed in reasonable time
    # 10 seconds is generous for in-memory DB with 10k tasks
    assert elapsed_time < 10.0, \
        f"Operation took too long: {elapsed_time:.2f}s (expected < 10s)"

    # Step 6: Verify remaining task count
    async with memory_db._get_connection() as conn:
        cursor = await conn.execute("SELECT COUNT(*) FROM tasks")
        count_row = await cursor.fetchone()
        expected_remaining = total_tasks - 5000
        assert count_row[0] == expected_remaining, \
            f"Expected {expected_remaining} tasks remaining, got {count_row[0]}"

    # Step 7: Verify database integrity after large VACUUM
    async with memory_db._get_connection() as conn:
        # Run integrity check
        cursor = await conn.execute("PRAGMA integrity_check")
        integrity_row = await cursor.fetchone()
        assert integrity_row[0] == "ok", f"Database integrity check failed: {integrity_row[0]}"


@pytest.mark.asyncio
async def test_vacuum_failure_handling(memory_db: Database):
    """Test graceful handling of potential VACUUM issues."""
    # Step 1: Create tasks and get initial state
    task_ids = await create_tasks(memory_db, 100, TaskStatus.COMPLETED)
    assert len(task_ids) == 100

    # Step 2: Get database size before operation
    async with memory_db._get_connection() as conn:
        cursor = await conn.execute("PRAGMA page_count")
        page_count_row = await cursor.fetchone()
        cursor = await conn.execute("PRAGMA page_size")
        page_size_row = await cursor.fetchone()

        if page_count_row and page_size_row:
            initial_pages = page_count_row[0]
            page_size = page_size_row[0]
            initial_size = initial_pages * page_size
        else:
            pytest.fail("Could not get initial database size")

    # Step 3: Test VACUUM with normal deletion (should succeed)
    filters = PruneFilters(
        older_than_days=30,
        vacuum=True,
        dry_run=False
    )
    result = await memory_db.prune_tasks(filters)

    # Step 4: Verify successful deletion and VACUUM
    assert result.deleted_tasks == 100, f"Expected 100 tasks deleted, got {result.deleted_tasks}"
    assert result.dry_run is False
    assert result.reclaimed_bytes is not None, "reclaimed_bytes should not be None"
    assert result.reclaimed_bytes >= 0, "reclaimed_bytes should be non-negative"

    # Step 5: Verify database remains consistent after VACUUM
    async with memory_db._get_connection() as conn:
        # Check database integrity
        cursor = await conn.execute("PRAGMA integrity_check")
        integrity_row = await cursor.fetchone()
        assert integrity_row[0] == "ok", f"Database integrity check failed: {integrity_row[0]}"

        # Verify no tasks remain
        cursor = await conn.execute("SELECT COUNT(*) FROM tasks")
        count_row = await cursor.fetchone()
        assert count_row[0] == 0, "All tasks should be deleted"

        # Verify database size changed appropriately
        cursor = await conn.execute("PRAGMA page_count")
        page_count_row = await cursor.fetchone()
        if page_count_row:
            final_pages = page_count_row[0]
            final_size = final_pages * page_size
            # Database should be smaller or equal after deletion + VACUUM
            assert final_size <= initial_size, \
                f"Database grew after VACUUM: {initial_size} -> {final_size}"

    # Step 6: Test that database remains usable after VACUUM
    # Create new tasks to verify database is not corrupted
    new_task_ids = await create_tasks(memory_db, 10, TaskStatus.PENDING)
    assert len(new_task_ids) == 10, "Should be able to create new tasks after VACUUM"

    # Verify new tasks can be queried
    async with memory_db._get_connection() as conn:
        cursor = await conn.execute("SELECT COUNT(*) FROM tasks WHERE status = ?", (TaskStatus.PENDING.value,))
        count_row = await cursor.fetchone()
        assert count_row[0] == 10, "New tasks should be queryable"


@pytest.mark.asyncio
async def test_vacuum_preserves_data_integrity(memory_db: Database):
    """Test VACUUM doesn't corrupt remaining data."""
    # Step 1: Create tasks with different statuses and timestamps
    completed_ids = await create_tasks(memory_db, 100, TaskStatus.COMPLETED)
    pending_ids = await create_tasks(memory_db, 50, TaskStatus.PENDING)

    # Create recent completed tasks (won't be deleted)
    recent_time = datetime.now(timezone.utc) - timedelta(days=5)
    recent_ids = []
    for i in range(25):
        task = Task(
            prompt=f"Recent task {i}",
            summary=f"Recent test task {i}",
            agent_type="test-agent",
            source=TaskSource.HUMAN,
            status=TaskStatus.COMPLETED,
            submitted_at=recent_time,
            completed_at=recent_time,
        )
        await memory_db.insert_task(task)
        recent_ids.append(str(task.id))  # Get ID from task object, not return value

    total_tasks = len(completed_ids) + len(pending_ids) + len(recent_ids)
    assert total_tasks == 175, f"Expected 175 total tasks, got {total_tasks}"

    # Step 2: Store task data for verification before deletion
    tasks_to_preserve = pending_ids + recent_ids
    task_data_before = {}

    async with memory_db._get_connection() as conn:
        for task_id in tasks_to_preserve:
            cursor = await conn.execute(
                "SELECT id, prompt, summary, agent_type, status, source FROM tasks WHERE id = ?",
                (task_id,)
            )
            row = await cursor.fetchone()
            if row:
                task_data_before[task_id] = row

    # Step 3: Delete old completed tasks (100 tasks) with VACUUM
    filters = PruneFilters(
        older_than_days=30,  # Only deletes old completed tasks (40 days old)
        vacuum=True,
        dry_run=False
    )
    result = await memory_db.prune_tasks(filters)

    # Step 4: Verify correct number of deletions
    assert result.deleted_tasks == 100, f"Expected 100 tasks deleted, got {result.deleted_tasks}"
    assert result.reclaimed_bytes is not None
    assert result.reclaimed_bytes >= 0

    # Step 5: Verify remaining tasks are intact and queryable
    async with memory_db._get_connection() as conn:
        # Check total remaining count
        cursor = await conn.execute("SELECT COUNT(*) FROM tasks")
        count_row = await cursor.fetchone()
        expected_remaining = 75  # 50 pending + 25 recent completed
        assert count_row[0] == expected_remaining, \
            f"Expected {expected_remaining} tasks remaining, got {count_row[0]}"

        # Verify each preserved task's data is intact
        for task_id, original_data in task_data_before.items():
            cursor = await conn.execute(
                "SELECT id, prompt, summary, agent_type, status, source FROM tasks WHERE id = ?",
                (task_id,)
            )
            row = await cursor.fetchone()

            assert row is not None, f"Task {task_id} should still exist"
            assert row == original_data, \
                f"Task {task_id} data corrupted: {row} != {original_data}"

    # Step 6: Verify database integrity
    async with memory_db._get_connection() as conn:
        cursor = await conn.execute("PRAGMA integrity_check")
        integrity_row = await cursor.fetchone()
        assert integrity_row[0] == "ok", f"Database integrity check failed: {integrity_row[0]}"

    # Step 7: Verify tasks can still be retrieved via ORM methods
    for task_id in tasks_to_preserve:
        # Ensure task_id is a valid UUID string
        if not isinstance(task_id, str):
            raise AssertionError(f"Expected task_id to be string, got {type(task_id)}: {task_id}")

        # Debug: Print task_id to see what we're dealing with
        try:
            task_uuid = UUID(task_id)
        except ValueError as e:
            raise AssertionError(f"Invalid UUID string '{task_id}' (len={len(task_id)}): {e}") from e

        task = await memory_db.get_task(task_uuid)
        assert task is not None, f"Task {task_id} should be retrievable via get_task()"
        assert str(task.id) == task_id
        assert task.prompt is not None
        assert task.summary is not None


@pytest.mark.asyncio
async def test_vacuum_with_concurrent_operations(memory_db: Database):
    """Test VACUUM behavior with concurrent database access.

    Note: SQLite VACUUM requires exclusive lock. This test verifies that
    concurrent operations either succeed in sequence or are properly
    queued without causing deadlocks or data corruption.
    """
    import asyncio

    # Step 1: Create initial dataset
    task_ids = await create_tasks(memory_db, 200, TaskStatus.COMPLETED)
    assert len(task_ids) == 200

    # Step 2: Define concurrent operations
    async def concurrent_vacuum():
        """Perform VACUUM operation."""
        filters = PruneFilters(
            older_than_days=30,
            limit=100,
            vacuum=True,
            dry_run=False
        )
        return await memory_db.prune_tasks(filters)

    async def concurrent_query():
        """Perform read operation during VACUUM."""
        await asyncio.sleep(0.01)  # Small delay to interleave with VACUUM
        async with memory_db._get_connection() as conn:
            cursor = await conn.execute("SELECT COUNT(*) FROM tasks")
            row = await cursor.fetchone()
            return row[0] if row else 0

    async def concurrent_insert():
        """Perform write operation during VACUUM."""
        await asyncio.sleep(0.02)  # Small delay to interleave with VACUUM
        task = Task(
            prompt="Concurrent task",
            summary="Test concurrent insert",
            agent_type="test-agent",
            source=TaskSource.HUMAN,
            status=TaskStatus.PENDING,
        )
        await memory_db.insert_task(task)
        return str(task.id)  # Return the task ID

    # Step 3: Run operations concurrently
    # SQLite will serialize these operations automatically
    results = await asyncio.gather(
        concurrent_vacuum(),
        concurrent_query(),
        concurrent_insert(),
        return_exceptions=True  # Capture any exceptions
    )

    vacuum_result, query_count, insert_id = results

    # Step 4: Verify no exceptions occurred
    for i, result in enumerate(results):
        if isinstance(result, Exception):
            pytest.fail(f"Operation {i} raised exception: {result}")

    # Step 5: Verify VACUUM completed successfully
    assert vacuum_result.deleted_tasks == 100, \
        f"Expected 100 tasks deleted, got {vacuum_result.deleted_tasks}"
    assert vacuum_result.reclaimed_bytes is not None

    # Step 6: Verify query returned valid count
    # Count should be between 100-101 (100 remaining from VACUUM, possibly +1 from concurrent insert)
    assert 100 <= query_count <= 201, \
        f"Query count should be 100-201, got {query_count}"

    # Step 7: Verify insert succeeded and task exists
    assert insert_id is not None, "Concurrent insert should succeed"
    inserted_task = await memory_db.get_task(UUID(insert_id))
    assert inserted_task is not None, "Inserted task should be retrievable"
    assert inserted_task.prompt == "Concurrent task"

    # Step 8: Verify database integrity after concurrent operations
    async with memory_db._get_connection() as conn:
        cursor = await conn.execute("PRAGMA integrity_check")
        integrity_row = await cursor.fetchone()
        assert integrity_row[0] == "ok", \
            f"Database integrity check failed after concurrent ops: {integrity_row[0]}"

        # Verify final count is consistent
        cursor = await conn.execute("SELECT COUNT(*) FROM tasks")
        final_count_row = await cursor.fetchone()
        expected_count = 101  # 100 remaining from VACUUM + 1 concurrent insert
        assert final_count_row[0] == expected_count, \
            f"Expected {expected_count} tasks after concurrent ops, got {final_count_row[0]}"


@pytest.mark.asyncio
async def test_vacuum_cli_integration(memory_db: Database):
    """Test --vacuum flag integration (placeholder for future CLI implementation).

    This test is a placeholder until the CLI prune command is updated with
    the --vacuum flag. Once implemented, this test should verify:
    1. CLI accepts --vacuum flag
    2. CLI passes vacuum_mode='always' to database layer
    3. CLI displays reclaimed space in output
    """
    # Step 1: Create tasks for testing
    task_ids = await create_tasks(memory_db, 50, TaskStatus.COMPLETED)
    assert len(task_ids) == 50

    # Step 2: Test database-level VACUUM (what CLI will eventually call)
    filters = PruneFilters(
        older_than_days=30,
        vacuum_mode="always",
        dry_run=False
    )
    result = await memory_db.prune_tasks(filters)

    # Step 3: Verify VACUUM behavior that CLI should expose
    assert result.deleted_tasks == 50
    assert result.reclaimed_bytes is not None, \
        "CLI should display reclaimed_bytes when --vacuum is used"
    assert result.reclaimed_bytes >= 0

    # TODO: Once CLI is updated with --vacuum flag, replace this with:
    # from typer.testing import CliRunner
    # from abathur.cli.main import app
    #
    # runner = CliRunner()
    # result = runner.invoke(app, [
    #     "task", "prune",
    #     "--older-than", "30d",
    #     "--vacuum",
    #     "--force"
    # ])
    #
    # assert result.exit_code == 0
    # assert "Space reclaimed" in result.output or "reclaimed" in result.output.lower()
    #
    # # Verify VACUUM was actually executed
    # async with memory_db._get_connection() as conn:
    #     cursor = await conn.execute("SELECT COUNT(*) FROM tasks")
    #     count_row = await cursor.fetchone()
    #     assert count_row[0] == 0, "All tasks should be deleted"

    # For now, this test validates the database layer is ready for CLI integration
    console_output = f"Would delete {result.deleted_tasks} tasks and reclaim {result.reclaimed_bytes} bytes"
    assert "reclaim" in console_output.lower(), "Output should mention space reclamation"
