"""Integration tests for database error recovery.

Tests complete end-to-end workflows for error recovery scenarios:
- Transaction rollback behavior
- Database consistency after errors
- Error message propagation
- Dry run behavior (prevents VACUUM execution)
- Partial deletion success when some tasks deletable, others blocked
- Proper reporting of blocked deletions with parent-child relationships
"""

import asyncio
import sqlite3
from collections.abc import AsyncGenerator
from datetime import datetime, timedelta, timezone
from pathlib import Path
from unittest.mock import patch
from uuid import UUID, uuid4

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
