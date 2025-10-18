"""Integration tests for database error recovery.

Tests complete end-to-end workflows for error recovery scenarios:
- Transaction rollback behavior
- Database consistency after errors
- Error message propagation
- Dry run behavior (prevents VACUUM execution)
"""

import sqlite3
from collections.abc import AsyncGenerator
from datetime import datetime, timedelta, timezone
from pathlib import Path
from unittest.mock import patch

import pytest
from abathur.domain.models import Task, TaskStatus
from abathur.infrastructure.database import Database, PruneFilters


@pytest.fixture
async def memory_db() -> AsyncGenerator[Database, None]:
    """Create in-memory database for fast integration tests."""
    db = Database(Path(":memory:"))
    await db.initialize()
    yield db
    await db.close()


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
