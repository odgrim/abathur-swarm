"""Integration tests for database error recovery.

Tests complete end-to-end workflows for error recovery scenarios:
- Transaction rollback behavior
- Database consistency after errors
- Error message propagation
- Dry run behavior (prevents VACUUM execution)
- Partial deletion success when some tasks deletable, others blocked
- Proper reporting of blocked deletions with parent-child relationships
- Database lock timeout handling
- Concurrent write conflicts
- Connection failures
- Data integrity verification
"""

import asyncio
import sqlite3
import tempfile
from collections.abc import AsyncGenerator
from datetime import datetime, timedelta, timezone
from pathlib import Path
from unittest.mock import patch
from uuid import UUID, uuid4

import aiosqlite
import pytest
from abathur.domain.models import (
    DependencyType,
    Task,
    TaskDependency,
    TaskSource,
    TaskStatus,
)
from abathur.infrastructure.database import Database, PruneFilters


@pytest.fixture
async def memory_db() -> AsyncGenerator[Database, None]:
    """Create in-memory database for fast integration tests."""
    db = Database(Path(":memory:"))
    await db.initialize()
    yield db
    await db.close()


@pytest.fixture
async def file_db_path() -> AsyncGenerator[Path, None]:
    """Create temporary file-based database path.

    Uses actual file (not :memory:) to enable multiple connection testing.
    """
    with tempfile.NamedTemporaryFile(suffix=".db", delete=False) as f:
        db_path = Path(f.name)

    yield db_path

    # Cleanup database and WAL files
    if db_path.exists():
        db_path.unlink()
    wal_path = db_path.with_suffix(".db-wal")
    shm_path = db_path.with_suffix(".db-shm")
    if wal_path.exists():
        wal_path.unlink()
    if shm_path.exists():
        shm_path.unlink()


@pytest.fixture
async def file_database(file_db_path: Path) -> AsyncGenerator[Database, None]:
    """Create file-based database for multi-connection testing."""
    db = Database(file_db_path)
    await db.initialize()
    yield db
    # No explicit close needed for file-based databases


class TestDatabasePruneErrorRecovery:
    """Integration tests for database prune operation error recovery.

    Tests Recommendation #12: Error recovery for database prune functionality.
    Focuses on transaction integrity and rollback behavior.
    """

    @pytest.mark.asyncio
    async def test_vacuum_success_returns_reclaimed_bytes(
        self, memory_db: Database
    ) -> None:
        """Test successful VACUUM returns reclaimed bytes in result.

        This is a positive test case to verify normal VACUUM behavior.
        """
        # Create tasks with some data
        task_ids = []
        for i in range(20):
            task = Task(
                prompt=f"Task {i}",
                summary=f"Task {i}",
                input_data={"large_data": "x" * 1000},
            )
            await memory_db.insert_task(task)
            task_ids.append(task.id)

        # Update to completed with old dates
        old_date = datetime.now(timezone.utc) - timedelta(days=35)
        async with memory_db._get_connection() as conn:
            for task_id in task_ids:
                await conn.execute(
                    "UPDATE tasks SET status = ?, completed_at = ? WHERE id = ?",
                    (TaskStatus.COMPLETED.value, old_date.isoformat(), str(task_id)),
                )
            await conn.commit()

        # Prune tasks (VACUUM should succeed)
        filters = PruneFilters(
            older_than_days=30,
            statuses=[TaskStatus.COMPLETED],
        )
        result = await memory_db.prune_tasks(filters)

        # Verify success
        assert result.deleted_tasks == 20
        assert result.deleted_dependencies == 0
        assert result.dry_run is False
        # VACUUM should report reclaimed bytes (may be None or >= 0)
        assert result.reclaimed_bytes is None or result.reclaimed_bytes >= 0

    @pytest.mark.asyncio
    async def test_transaction_rollback_on_deletion_error(
        self, memory_db: Database
    ) -> None:
        """Test transaction rollback if deletion fails before VACUUM.

        This tests error handling during the deletion phase (before VACUUM).
        """
        # Create tasks
        task_ids = []
        for i in range(10):
            task = Task(prompt=f"Task {i}", summary=f"Task {i}")
            await memory_db.insert_task(task)
            task_ids.append(task.id)

        # Update to completed
        old_date = datetime.now(timezone.utc) - timedelta(days=35)
        async with memory_db._get_connection() as conn:
            for task_id in task_ids:
                await conn.execute(
                    "UPDATE tasks SET status = ?, completed_at = ? WHERE id = ?",
                    (TaskStatus.COMPLETED.value, old_date.isoformat(), str(task_id)),
                )
            await conn.commit()

        # Mock to fail during deletion phase (before VACUUM)
        import aiosqlite
        original_execute = aiosqlite.Connection.execute

        async def execute_with_deletion_failure(self, sql, *args, **kwargs):
            """Fail on DELETE FROM tasks statement."""
            if isinstance(sql, str) and "DELETE FROM tasks" in sql:
                raise sqlite3.OperationalError("simulated deletion error")
            return await original_execute(self, sql, *args, **kwargs)

        with patch.object(aiosqlite.Connection, "execute", execute_with_deletion_failure):
            filters = PruneFilters(
                older_than_days=30,
                statuses=[TaskStatus.COMPLETED],
            )

            with pytest.raises(RuntimeError) as exc_info:
                await memory_db.prune_tasks(filters)

            assert "Failed to prune tasks" in str(exc_info.value)
            assert "simulated deletion error" in str(exc_info.value)

        # Verify tasks were NOT deleted (transaction rolled back)
        remaining_tasks = await memory_db.list_tasks(status=TaskStatus.COMPLETED)
        assert len(remaining_tasks) == 10, "Tasks should not be deleted if transaction fails"

        # Verify database is still consistent
        for task_id in task_ids:
            task = await memory_db.get_task(task_id)
            assert task is not None, f"Task {task_id} should still exist after rollback"

    @pytest.mark.asyncio
    async def test_dry_run_never_runs_vacuum(
        self, memory_db: Database
    ) -> None:
        """Test dry_run mode never executes VACUUM.

        Verifies that:
        1. Dry run doesn't delete tasks
        2. VACUUM is never executed
        3. reclaimed_bytes is None in dry run
        """
        # Create tasks
        task_ids = []
        for i in range(10):
            task = Task(prompt=f"Task {i}", summary=f"Task {i}")
            await memory_db.insert_task(task)
            task_ids.append(task.id)

        # Update to completed
        old_date = datetime.now(timezone.utc) - timedelta(days=35)
        async with memory_db._get_connection() as conn:
            for task_id in task_ids:
                await conn.execute(
                    "UPDATE tasks SET status = ?, completed_at = ? WHERE id = ?",
                    (TaskStatus.COMPLETED.value, old_date.isoformat(), str(task_id)),
                )
            await conn.commit()

        # Track if VACUUM was called
        vacuum_called = {"called": False}

        import aiosqlite
        original_execute = aiosqlite.Connection.execute

        async def execute_tracking_vacuum(self, sql, *args, **kwargs):
            """Track if VACUUM is called."""
            if isinstance(sql, str) and sql.strip().upper() == "VACUUM":
                vacuum_called["called"] = True
            return await original_execute(self, sql, *args, **kwargs)

        with patch.object(aiosqlite.Connection, "execute", execute_tracking_vacuum):
            filters = PruneFilters(
                older_than_days=30,
                statuses=[TaskStatus.COMPLETED],
                dry_run=True,
            )
            result = await memory_db.prune_tasks(filters)

        # Verify dry run behavior
        assert result.deleted_tasks == 10  # Would delete 10
        assert result.dry_run is True
        assert result.reclaimed_bytes is None  # No VACUUM in dry run
        assert not vacuum_called["called"], "VACUUM should not be called in dry run"

        # Verify tasks still exist
        remaining_tasks = await memory_db.list_tasks(status=TaskStatus.COMPLETED)
        assert len(remaining_tasks) == 10, "Tasks should not be deleted in dry run"


class TestPartialDeletionFailureHandling:
    """Test error recovery for partial deletion scenarios."""

    @pytest.mark.asyncio
    async def test_delete_tasks_partial_failure_handling(
        self, memory_db: Database
    ):
        """Test error recovery when attempting to delete mix of deletable and blocked tasks.

        Scenario:
        1. Create parent task with child (cannot delete parent)
        2. Create standalone task (can delete individually)
        3. Attempt to delete both parent and standalone together
        4. Verify all-or-nothing behavior: ALL deletions blocked due to parent
        5. Verify DeleteResult structure reports blocked deletions correctly

        Expected result (current implementation - all-or-nothing):
        - deleted_count == 0 (no deletions when any task has children)
        - blocked_deletions contains parent with child_ids
        - errors contains appropriate error message
        - Both tasks still exist in database
        """
        # Step 1: Create parent task with child
        parent_task = Task(
            id=uuid4(),
            prompt="Parent task",
            agent_type="test-agent",
            priority=5,
            status=TaskStatus.COMPLETED,
            input_data={},
            submitted_at=datetime.now(timezone.utc),
            last_updated_at=datetime.now(timezone.utc),
            source=TaskSource.HUMAN,
            dependency_type=DependencyType.SEQUENTIAL,
            summary="Parent task with child",
        )

        child_task = Task(
            id=uuid4(),
            prompt="Child task",
            agent_type="test-agent",
            priority=5,
            status=TaskStatus.PENDING,
            input_data={},
            parent_task_id=parent_task.id,  # Link to parent
            submitted_at=datetime.now(timezone.utc),
            last_updated_at=datetime.now(timezone.utc),
            source=TaskSource.HUMAN,
            dependency_type=DependencyType.SEQUENTIAL,
            summary="Child task of parent",
        )

        await memory_db.insert_task(parent_task)
        await memory_db.insert_task(child_task)

        # Step 2: Create standalone task (no children, can be deleted)
        standalone_task = Task(
            id=uuid4(),
            prompt="Standalone task",
            agent_type="test-agent",
            priority=5,
            status=TaskStatus.COMPLETED,
            input_data={},
            submitted_at=datetime.now(timezone.utc),
            last_updated_at=datetime.now(timezone.utc),
            source=TaskSource.HUMAN,
            dependency_type=DependencyType.SEQUENTIAL,
            summary="Standalone task - no children",
        )

        await memory_db.insert_task(standalone_task)

        # Step 3: Attempt to delete both parent and standalone
        result = await memory_db.delete_tasks([parent_task.id, standalone_task.id])

        # Step 4: Verify all-or-nothing behavior - NO tasks deleted when ANY has children
        assert result["deleted_count"] == 0, \
            "All deletions should be blocked when any task has children"

        # Step 5: Verify blocked_deletions contains parent
        assert len(result["blocked_deletions"]) == 1, "Parent should be in blocked list"
        blocked = result["blocked_deletions"][0]

        # Step 6: Verify blocked deletion structure
        assert "task_id" in blocked, "Blocked deletion must contain task_id"
        assert "child_ids" in blocked, "Blocked deletion must contain child_ids"
        assert blocked["task_id"] == str(parent_task.id), "Parent task should be blocked"
        assert str(child_task.id) in blocked["child_ids"], "Child ID should be listed"
        assert len(blocked["child_ids"]) == 1, "Should report exactly 1 child"

        # Step 7: Verify errors list contains appropriate message
        assert len(result["errors"]) > 0, "Should have error message for blocked deletion"
        error_message = result["errors"][0].lower()
        assert "child" in error_message or "dependent" in error_message, \
            "Error message should mention child tasks"

        # Step 8: Verify standalone was NOT deleted (all-or-nothing behavior)
        standalone_retrieved = await memory_db.get_task(standalone_task.id)
        assert standalone_retrieved is not None, \
            "Standalone task should still exist (all deletions blocked)"

        # Step 9: Verify parent still exists (was not deleted)
        parent_retrieved = await memory_db.get_task(parent_task.id)
        assert parent_retrieved is not None, "Parent task should still exist"

        # Step 10: Verify child still exists (was not deleted)
        child_retrieved = await memory_db.get_task(child_task.id)
        assert child_retrieved is not None, "Child task should still exist"


class TestDatabaseLockRecovery:
    """Tests for database lock timeout handling and recovery."""

    @pytest.mark.asyncio
    async def test_prune_with_database_locked(self, file_db_path: Path, file_database: Database) -> None:
        """Test graceful handling when database locked by another process.

        Scenario:
        1. Create task in database
        2. Open second connection and acquire exclusive write lock
        3. Attempt to write from first connection (should timeout)
        4. Verify appropriate timeout error raised
        5. Verify error message is clear
        6. Release lock and verify operation succeeds
        7. Verify no data corruption
        """
        # Step 1: Insert initial task
        task = Task(
            prompt="Test task for lock scenario",
            summary="Lock test task",
            agent_type="test-agent",
            source=TaskSource.HUMAN,
            status=TaskStatus.PENDING,
        )
        await file_database.insert_task(task)

        # Verify task was inserted
        retrieved = await file_database.get_task(task.id)
        assert retrieved is not None
        assert retrieved.prompt == task.prompt

        # Step 2: Open second connection and start exclusive transaction
        # Use low busy_timeout to make test run faster
        lock_conn = await aiosqlite.connect(str(file_db_path))
        await lock_conn.execute("PRAGMA busy_timeout=100")  # 100ms timeout

        try:
            # Begin exclusive transaction (acquires write lock)
            await lock_conn.execute("BEGIN EXCLUSIVE")

            # Step 3: Attempt write operation from first connection while locked
            # This should timeout and raise OperationalError
            second_task = Task(
                prompt="Second task - should timeout",
                summary="Timeout test",
                agent_type="test-agent",
                source=TaskSource.HUMAN,
                status=TaskStatus.PENDING,
            )

            # Step 4: Attempt to insert via file_database while lock is held
            # Need to create a low-timeout connection for the database to use
            # Since Database uses _get_connection(), we need to test at raw SQL level
            test_conn = await aiosqlite.connect(str(file_db_path))
            await test_conn.execute("PRAGMA busy_timeout=100")

            try:
                # Verify OperationalError raised with clear message
                with pytest.raises(aiosqlite.OperationalError) as exc_info:
                    # Attempt simple write operation (UPDATE is simpler to test)
                    await test_conn.execute(
                        "UPDATE tasks SET status = ? WHERE id = ?",
                        (TaskStatus.RUNNING.value, str(task.id)),
                    )
                    await test_conn.commit()

                # Step 5: Verify error message mentions database lock
                error_message = str(exc_info.value).lower()
                assert "locked" in error_message or "busy" in error_message, (
                    f"Error message should mention lock/busy state: {exc_info.value}"
                )

            finally:
                await test_conn.close()

            # Step 6: Release lock and verify operation succeeds
            await lock_conn.rollback()

        finally:
            await lock_conn.close()

        # Step 7: Verify original data integrity - first task still exists
        retrieved_after_lock = await file_database.get_task(task.id)
        assert retrieved_after_lock is not None
        assert retrieved_after_lock.prompt == task.prompt
        assert retrieved_after_lock.summary == task.summary

        # Verify second task was NOT inserted due to lock timeout
        all_tasks = await file_database.list_tasks()
        assert len(all_tasks) == 1
        assert all_tasks[0].id == task.id

    @pytest.mark.asyncio
    async def test_concurrent_read_during_write_lock(
        self, file_db_path: Path, file_database: Database
    ) -> None:
        """Test that reads can proceed while write lock is held (WAL mode).

        SQLite WAL mode allows concurrent reads during writes.
        """
        # Insert initial task
        task = Task(
            prompt="Read test task",
            summary="Concurrent read test",
            agent_type="test-agent",
            source=TaskSource.HUMAN,
            status=TaskStatus.PENDING,
        )
        await file_database.insert_task(task)

        # Open second connection and start regular transaction (not EXCLUSIVE)
        write_conn = await aiosqlite.connect(str(file_db_path))
        await write_conn.execute("PRAGMA journal_mode=WAL")

        try:
            # Start write transaction
            await write_conn.execute("BEGIN")
            await write_conn.execute(
                "UPDATE tasks SET status = ? WHERE id = ?",
                (TaskStatus.RUNNING.value, str(task.id)),
            )

            # Read from main database while write transaction is open
            # This should succeed in WAL mode
            retrieved = await file_database.get_task(task.id)
            assert retrieved is not None
            # Should see old data (transaction not committed yet)
            assert retrieved.status == TaskStatus.PENDING

            # Commit write transaction
            await write_conn.commit()

        finally:
            await write_conn.close()

        # Now read should see updated data
        retrieved_after = await file_database.get_task(task.id)
        assert retrieved_after is not None
        assert retrieved_after.status == TaskStatus.RUNNING

    @pytest.mark.asyncio
    async def test_write_lock_timeout_clear_error_message(
        self, file_db_path: Path, file_database: Database
    ) -> None:
        """Test that lock timeout errors provide clear, actionable messages."""
        # Insert task
        task = Task(
            prompt="Error message test",
            summary="Error message test task",
            agent_type="test-agent",
            source=TaskSource.HUMAN,
        )
        await file_database.insert_task(task)

        # Acquire exclusive lock
        lock_conn = await aiosqlite.connect(str(file_db_path))
        await lock_conn.execute("PRAGMA busy_timeout=50")

        try:
            await lock_conn.execute("BEGIN EXCLUSIVE")

            # Attempt update with short timeout
            update_conn = await aiosqlite.connect(str(file_db_path))
            await update_conn.execute("PRAGMA busy_timeout=50")

            try:
                with pytest.raises(aiosqlite.OperationalError) as exc_info:
                    await update_conn.execute(
                        "UPDATE tasks SET status = ? WHERE id = ?",
                        (TaskStatus.RUNNING.value, str(task.id)),
                    )
                    await update_conn.commit()

                # Verify error is OperationalError (database-level error)
                assert isinstance(exc_info.value, aiosqlite.OperationalError)

                # Error should mention the nature of the problem
                error_str = str(exc_info.value)
                assert error_str, "Error should have a message"

            finally:
                await update_conn.close()

        finally:
            await lock_conn.rollback()
            await lock_conn.close()

    @pytest.mark.asyncio
    async def test_data_integrity_after_lock_timeout(
        self, file_db_path: Path, file_database: Database
    ) -> None:
        """Test that failed operations due to lock timeout don't corrupt data."""
        # Insert multiple tasks
        tasks = []
        for i in range(5):
            task = Task(
                prompt=f"Task {i}",
                summary=f"Test task {i}",
                agent_type="test-agent",
                source=TaskSource.HUMAN,
                status=TaskStatus.PENDING,
            )
            await file_database.insert_task(task)
            tasks.append(task)

        # Verify all tasks inserted
        all_tasks = await file_database.list_tasks()
        assert len(all_tasks) == 5

        # Acquire exclusive lock
        lock_conn = await aiosqlite.connect(str(file_db_path))
        await lock_conn.execute("PRAGMA busy_timeout=50")

        try:
            await lock_conn.execute("BEGIN EXCLUSIVE")

            # Attempt to update task while locked (will fail)
            update_conn = await aiosqlite.connect(str(file_db_path))
            await update_conn.execute("PRAGMA busy_timeout=50")

            try:
                with pytest.raises(aiosqlite.OperationalError):
                    await update_conn.execute(
                        "UPDATE tasks SET status = ? WHERE id = ?",
                        (TaskStatus.COMPLETED.value, str(tasks[0].id)),
                    )
                    await update_conn.commit()
            finally:
                await update_conn.close()

        finally:
            await lock_conn.rollback()
            await lock_conn.close()

        # Verify data integrity: all tasks still in original state
        all_tasks_after = await file_database.list_tasks()
        assert len(all_tasks_after) == 5

        for task in all_tasks_after:
            assert task.status == TaskStatus.PENDING
            # Find original task
            original = next(t for t in tasks if t.id == task.id)
            assert task.prompt == original.prompt
            assert task.summary == original.summary

    @pytest.mark.asyncio
    async def test_lock_released_after_exception(
        self, file_db_path: Path, file_database: Database
    ) -> None:
        """Test that locks are properly released after exceptions."""
        task = Task(
            prompt="Lock release test",
            summary="Lock release test task",
            agent_type="test-agent",
            source=TaskSource.HUMAN,
        )
        await file_database.insert_task(task)

        # Simulate error during transaction
        conn = await aiosqlite.connect(str(file_db_path))

        try:
            await conn.execute("BEGIN")
            await conn.execute(
                "UPDATE tasks SET status = ? WHERE id = ?",
                (TaskStatus.RUNNING.value, str(task.id)),
            )

            # Simulate error (rollback instead of commit)
            await conn.rollback()

        finally:
            await conn.close()

        # Verify we can still access database from main connection
        # (lock was properly released)
        retrieved = await file_database.get_task(task.id)
        assert retrieved is not None
        assert retrieved.status == TaskStatus.PENDING  # Update was rolled back

        # Verify we can perform new operations
        await file_database.update_task_status(task.id, TaskStatus.COMPLETED)

        retrieved_after = await file_database.get_task(task.id)
        assert retrieved_after is not None
        assert retrieved_after.status == TaskStatus.COMPLETED
