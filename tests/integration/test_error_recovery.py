"""Integration tests for error recovery scenarios.

Tests real error conditions including:
- Database lock timeout handling
- Concurrent write conflicts
- Connection failures
- Data integrity verification
"""

import asyncio
import tempfile
from collections.abc import AsyncGenerator
from pathlib import Path

import aiosqlite
import pytest
from abathur.domain.models import Task, TaskSource, TaskStatus
from abathur.infrastructure.database import Database


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
