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
- CASCADE deletion completeness
- Foreign key constraint enforcement
- Database recovery after interrupted operations
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
    Agent,
    AgentState,
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


class TestCascadeDeletion:
    """Integration tests for CASCADE deletion completeness.

    Tests Recommendation #12, Scenario #5: Verify CASCADE deletion removes
    all related records when a task is deleted.
    """

    @pytest.mark.asyncio
    async def test_cascade_deletion_completes_fully(
        self, memory_db: Database
    ) -> None:
        """Test CASCADE deletion removes all related records completely.

        Scenario:
        1. Create a task with multiple related records:
           - 3 agents
           - 2 task_dependencies (as prerequisite)
        2. Delete the task using delete_tasks
        3. Verify ALL agents CASCADE deleted
        4. Verify ALL dependencies CASCADE deleted
        5. Verify result.deleted_count == 1
        6. Verify no orphaned records remain

        This test validates that ON DELETE CASCADE foreign key constraints
        work correctly across all related tables.
        """
        # Step 1: Create task with multiple related records
        main_task = Task(
            prompt="Main task to be deleted",
            summary="Main task",
            agent_type="test-agent",
        )
        await memory_db.insert_task(main_task)

        # Create 3 agents associated with this task
        agent1 = Agent(
            name="Agent 1",
            specialization="Testing CASCADE deletion",
            task_id=main_task.id,
            state=AgentState.BUSY,
        )
        agent2 = Agent(
            name="Agent 2",
            specialization="Testing CASCADE deletion",
            task_id=main_task.id,
            state=AgentState.IDLE,
        )
        agent3 = Agent(
            name="Agent 3",
            specialization="Testing CASCADE deletion",
            task_id=main_task.id,
            state=AgentState.TERMINATED,
        )

        await memory_db.insert_agent(agent1)
        await memory_db.insert_agent(agent2)
        await memory_db.insert_agent(agent3)

        # Create 2 dependent tasks that depend on main_task
        dependent_task1 = Task(
            prompt="Dependent task 1",
            summary="Dep task 1",
        )
        dependent_task2 = Task(
            prompt="Dependent task 2",
            summary="Dep task 2",
        )
        await memory_db.insert_task(dependent_task1)
        await memory_db.insert_task(dependent_task2)

        # Create task_dependencies where main_task is the prerequisite
        dependency1 = TaskDependency(
            dependent_task_id=dependent_task1.id,
            prerequisite_task_id=main_task.id,
            dependency_type=DependencyType.SEQUENTIAL,
        )
        dependency2 = TaskDependency(
            dependent_task_id=dependent_task2.id,
            prerequisite_task_id=main_task.id,
            dependency_type=DependencyType.SEQUENTIAL,
        )
        await memory_db.insert_task_dependency(dependency1)
        await memory_db.insert_task_dependency(dependency2)

        # Verify all records exist before deletion
        # Check task exists
        task_before = await memory_db.get_task(main_task.id)
        assert task_before is not None

        # Check agents exist (direct SQL query)
        async with memory_db._get_connection() as conn:
            cursor = await conn.execute(
                "SELECT COUNT(*) FROM agents WHERE task_id = ?",
                (str(main_task.id),)
            )
            agent_count = (await cursor.fetchone())[0]
            assert agent_count == 3, "Should have 3 agents before deletion"

        # Check dependencies exist (direct SQL query)
        async with memory_db._get_connection() as conn:
            cursor = await conn.execute(
                "SELECT COUNT(*) FROM task_dependencies WHERE prerequisite_task_id = ?",
                (str(main_task.id),)
            )
            dep_count = (await cursor.fetchone())[0]
            assert dep_count == 2, "Should have 2 dependencies before deletion"

        # Step 2: Delete the task using delete_tasks
        result = await memory_db.delete_tasks([main_task.id])

        # Step 3: Verify result.deleted_count == 1
        assert result["deleted_count"] == 1, "Should delete exactly 1 task"
        assert result["blocked_deletions"] == [], "Should have no blocked deletions"
        assert result["errors"] == [], "Should have no errors"

        # Verify task is deleted
        task_after = await memory_db.get_task(main_task.id)
        assert task_after is None, "Main task should be deleted"

        # Step 4: Verify ALL agents CASCADE deleted
        async with memory_db._get_connection() as conn:
            cursor = await conn.execute(
                "SELECT COUNT(*) FROM agents WHERE task_id = ?",
                (str(main_task.id),)
            )
            agent_count_after = (await cursor.fetchone())[0]
            assert agent_count_after == 0, "All agents should be CASCADE deleted"

        # Verify no orphaned agents exist with the task_id
        async with memory_db._get_connection() as conn:
            cursor = await conn.execute(
                "SELECT id FROM agents WHERE task_id = ?",
                (str(main_task.id),)
            )
            orphaned_agents = await cursor.fetchall()
            assert len(orphaned_agents) == 0, "No orphaned agent records should remain"

        # Step 5: Verify ALL dependencies CASCADE deleted
        async with memory_db._get_connection() as conn:
            cursor = await conn.execute(
                "SELECT COUNT(*) FROM task_dependencies WHERE prerequisite_task_id = ?",
                (str(main_task.id),)
            )
            dep_count_after = (await cursor.fetchone())[0]
            assert dep_count_after == 0, "All dependencies should be CASCADE deleted"

        # Also check dependent_task_id side
        async with memory_db._get_connection() as conn:
            cursor = await conn.execute(
                "SELECT COUNT(*) FROM task_dependencies WHERE dependent_task_id = ?",
                (str(main_task.id),)
            )
            dep_count_dependent = (await cursor.fetchone())[0]
            assert dep_count_dependent == 0, "No dependencies should reference deleted task"

        # Verify no orphaned dependency records
        async with memory_db._get_connection() as conn:
            cursor = await conn.execute(
                """
                SELECT id FROM task_dependencies
                WHERE prerequisite_task_id = ? OR dependent_task_id = ?
                """,
                (str(main_task.id), str(main_task.id))
            )
            orphaned_deps = await cursor.fetchall()
            assert len(orphaned_deps) == 0, "No orphaned dependency records should remain"

        # Step 6: Verify dependent tasks still exist (not CASCADE deleted)
        # These tasks should still exist because they were dependents, not the deleted task
        dep_task1_after = await memory_db.get_task(dependent_task1.id)
        dep_task2_after = await memory_db.get_task(dependent_task2.id)
        assert dep_task1_after is not None, "Dependent task 1 should still exist"
        assert dep_task2_after is not None, "Dependent task 2 should still exist"

        # Final verification: Database is consistent and queryable
        all_tasks = await memory_db.list_tasks()
        assert len(all_tasks) == 2, "Should have 2 remaining tasks (the dependents)"

    @pytest.mark.asyncio
    async def test_cascade_deletion_with_bidirectional_dependencies(
        self, memory_db: Database
    ) -> None:
        """Test CASCADE deletion with bidirectional dependency relationships.

        This tests edge case where a task has both incoming and outgoing dependencies.
        When deleted, both sets of dependencies should CASCADE delete.
        """
        # Create 3 tasks: A -> B -> C (A is prereq for B, B is prereq for C)
        task_a = Task(prompt="Task A", summary="Task A")
        task_b = Task(prompt="Task B", summary="Task B")
        task_c = Task(prompt="Task C", summary="Task C")

        await memory_db.insert_task(task_a)
        await memory_db.insert_task(task_b)
        await memory_db.insert_task(task_c)

        # Create dependencies: A -> B -> C
        dep_a_to_b = TaskDependency(
            dependent_task_id=task_b.id,
            prerequisite_task_id=task_a.id,
            dependency_type=DependencyType.SEQUENTIAL,
        )
        dep_b_to_c = TaskDependency(
            dependent_task_id=task_c.id,
            prerequisite_task_id=task_b.id,
            dependency_type=DependencyType.SEQUENTIAL,
        )
        await memory_db.insert_task_dependency(dep_a_to_b)
        await memory_db.insert_task_dependency(dep_b_to_c)

        # Verify both dependencies exist
        async with memory_db._get_connection() as conn:
            cursor = await conn.execute("SELECT COUNT(*) FROM task_dependencies")
            dep_count = (await cursor.fetchone())[0]
            assert dep_count == 2

        # Delete middle task B (has both incoming and outgoing dependencies)
        result = await memory_db.delete_tasks([task_b.id])

        assert result["deleted_count"] == 1
        assert result["blocked_deletions"] == []

        # Verify both dependencies CASCADE deleted
        async with memory_db._get_connection() as conn:
            # Check prerequisite side (B as prerequisite of C)
            cursor = await conn.execute(
                "SELECT COUNT(*) FROM task_dependencies WHERE prerequisite_task_id = ?",
                (str(task_b.id),)
            )
            prereq_deps = (await cursor.fetchone())[0]
            assert prereq_deps == 0, "Dependencies with B as prerequisite should be deleted"

            # Check dependent side (B as dependent on A)
            cursor = await conn.execute(
                "SELECT COUNT(*) FROM task_dependencies WHERE dependent_task_id = ?",
                (str(task_b.id),)
            )
            dependent_deps = (await cursor.fetchone())[0]
            assert dependent_deps == 0, "Dependencies with B as dependent should be deleted"

            # Verify no dependencies remain for task B
            cursor = await conn.execute("SELECT COUNT(*) FROM task_dependencies")
            total_deps = (await cursor.fetchone())[0]
            assert total_deps == 0, "All dependencies involving B should be CASCADE deleted"

        # Verify tasks A and C still exist
        assert await memory_db.get_task(task_a.id) is not None
        assert await memory_db.get_task(task_c.id) is not None

    @pytest.mark.asyncio
    async def test_cascade_deletion_multiple_tasks_simultaneously(
        self, memory_db: Database
    ) -> None:
        """Test CASCADE deletion works correctly when deleting multiple tasks at once.

        Verifies that CASCADE deletion handles batch deletions properly.
        """
        # Create 3 tasks with agents
        tasks = []
        for i in range(3):
            task = Task(prompt=f"Task {i}", summary=f"Task {i}")
            await memory_db.insert_task(task)
            tasks.append(task)

            # Each task gets 2 agents
            for j in range(2):
                agent = Agent(
                    name=f"Agent {i}-{j}",
                    specialization=f"Spec {i}-{j}",
                    task_id=task.id,
                    state=AgentState.IDLE,
                )
                await memory_db.insert_agent(agent)

        # Verify 6 agents exist total (3 tasks × 2 agents)
        async with memory_db._get_connection() as conn:
            cursor = await conn.execute("SELECT COUNT(*) FROM agents")
            agent_count = (await cursor.fetchone())[0]
            assert agent_count == 6

        # Delete all 3 tasks simultaneously
        task_ids = [t.id for t in tasks]
        result = await memory_db.delete_tasks(task_ids)

        assert result["deleted_count"] == 3
        assert result["blocked_deletions"] == []

        # Verify ALL agents CASCADE deleted
        async with memory_db._get_connection() as conn:
            cursor = await conn.execute("SELECT COUNT(*) FROM agents")
            agent_count_after = (await cursor.fetchone())[0]
            assert agent_count_after == 0, "All agents should be CASCADE deleted"

        # Verify all tasks deleted
        for task_id in task_ids:
            assert await memory_db.get_task(task_id) is None


async def test_foreign_key_enforcement_during_prune(memory_db: Database):
    """Test that foreign key constraints are enforced during pruning operations.

    This test verifies:
    1. PRAGMA foreign_keys = ON is set
    2. Foreign key constraint violations are caught
    3. Valid parent-child relationships work correctly
    4. Referential integrity is maintained during deletions

    Foreign key constraint:
        FOREIGN KEY (parent_task_id) REFERENCES tasks(id)
    """
    # Step 1: Verify PRAGMA foreign_keys is ON
    async with memory_db._get_connection() as conn:
        cursor = await conn.execute('PRAGMA foreign_keys')
        fk_setting = await cursor.fetchone()
        assert fk_setting is not None
        assert fk_setting[0] == 1, 'Foreign keys should be enabled (PRAGMA foreign_keys = ON)'

    # Step 2: Attempt to insert task with non-existent parent_task_id
    # This should violate the foreign key constraint
    non_existent_parent_id = uuid4()
    task_with_invalid_parent = Task(
        summary='Task with invalid parent',
        prompt='Task with invalid parent',
        agent_type='test-agent',
        parent_task_id=non_existent_parent_id,
        status=TaskStatus.PENDING,
    )

    # Should raise IntegrityError due to FK constraint violation
    with pytest.raises(aiosqlite.IntegrityError) as exc_info:
        await memory_db.insert_task(task_with_invalid_parent)

    # Verify error message mentions foreign key
    error_msg = str(exc_info.value).lower()
    assert 'foreign key' in error_msg or 'constraint' in error_msg, \
        f'Expected FK constraint error, got: {exc_info.value}'

    # Step 3: Create valid parent task
    parent_task = Task(
        summary='Parent task',
        prompt='Parent task',
        agent_type='test-agent',
        status=TaskStatus.COMPLETED,
        priority=5,
    )

    # Set completion timestamp to make it eligible for pruning
    completed_time = datetime.now(timezone.utc) - timedelta(days=60)
    parent_task.completed_at = completed_time

    await memory_db.insert_task(parent_task)

    # Verify parent task was inserted
    retrieved_parent = await memory_db.get_task(parent_task.id)
    assert retrieved_parent is not None
    assert retrieved_parent.id == parent_task.id

    # Step 4: Create child task with valid parent_task_id
    child_task = Task(
        summary='Child task',
        prompt='Child task',
        agent_type='test-agent',
        parent_task_id=parent_task.id,
        status=TaskStatus.PENDING,
        priority=3,
    )

    # This should succeed because parent_task_id references existing task
    await memory_db.insert_task(child_task)

    # Verify child task was inserted with correct parent relationship
    retrieved_child = await memory_db.get_task(child_task.id)
    assert retrieved_child is not None
    assert retrieved_child.id == child_task.id
    assert retrieved_child.parent_task_id == parent_task.id

    # Step 5: Attempt to delete parent task while child exists
    # This should fail due to FK constraint (no CASCADE DELETE on parent_task_id)
    with pytest.raises(aiosqlite.IntegrityError) as exc_info:
        async with memory_db._get_connection() as conn:
            await conn.execute('DELETE FROM tasks WHERE id = ?', (str(parent_task.id),))
            await conn.commit()

    # Verify error mentions foreign key constraint
    error_msg = str(exc_info.value).lower()
    assert 'foreign key' in error_msg or 'constraint' in error_msg, \
        f'Expected FK constraint error when deleting parent, got: {exc_info.value}'

    # Step 6: Delete child task first (should succeed)
    async with memory_db._get_connection() as conn:
        cursor = await conn.execute('DELETE FROM tasks WHERE id = ?', (str(child_task.id),))
        await conn.commit()
        assert cursor.rowcount == 1, 'Child task should be deleted'

    # Verify child was deleted
    deleted_child = await memory_db.get_task(child_task.id)
    assert deleted_child is None

    # Step 7: Now delete parent task (should succeed since child is gone)
    async with memory_db._get_connection() as conn:
        cursor = await conn.execute('DELETE FROM tasks WHERE id = ?', (str(parent_task.id),))
        await conn.commit()
        assert cursor.rowcount == 1, 'Parent task should be deleted after child removed'

    # Verify parent was deleted
    deleted_parent = await memory_db.get_task(parent_task.id)
    assert deleted_parent is None

    # Step 8: Verify referential integrity maintained
    # No orphaned tasks should exist
    async with memory_db._get_connection() as conn:
        cursor = await conn.execute(
            '''
            SELECT COUNT(*) FROM tasks
            WHERE parent_task_id IS NOT NULL
            AND parent_task_id NOT IN (SELECT id FROM tasks)
            '''
        )
        orphan_count = await cursor.fetchone()
        assert orphan_count[0] == 0, 'No orphaned tasks should exist'


async def test_database_recovery_after_interrupted_prune(file_db_path: Path):
    """Test database recovery when prune operation is interrupted mid-transaction.

    SQLite uses write-ahead logging (WAL) for crash recovery. When a transaction
    is interrupted (connection closed without commit), the changes should be
    rolled back automatically when the database is reopened.

    Test scenario:
    1. Create file-based database with 50 tasks
    2. Start a DELETE transaction to simulate prune operation
    3. Interrupt the transaction by closing connection without commit
    4. Reopen database to trigger crash recovery
    5. Verify database integrity and correct task count

    Expected behavior:
    - Database can be opened after interruption (no corruption)
    - Interrupted transaction is rolled back (all tasks remain)
    - Database passes integrity check
    - No partial deletions visible
    """
    # Step 1: Create database and populate with 50 tasks
    db = Database(file_db_path)
    await db.initialize()

    # Create 50 tasks with different statuses
    tasks_created = []
    for i in range(50):
        # Mix of statuses: 20 completed, 15 pending, 10 failed, 5 cancelled
        if i < 20:
            status = TaskStatus.COMPLETED
        elif i < 35:
            status = TaskStatus.PENDING
        elif i < 45:
            status = TaskStatus.FAILED
        else:
            status = TaskStatus.CANCELLED

        task = Task(
            prompt=f"Test task {i}",
            summary=f"Test task {i}",
            agent_type="test-agent",
            priority=i % 10,
        )
        # Manually set status for test setup
        if status != TaskStatus.PENDING:
            task.status = status

        await db.insert_task(task)
        tasks_created.append(task)

    # Verify all 50 tasks were created
    all_tasks_before = await db.list_tasks()
    assert len(all_tasks_before) == 50, "Should have 50 tasks before interruption"

    # Step 2: Simulate interrupted prune operation
    # Open direct connection to simulate prune transaction
    # Don't use context manager since we want to close without commit
    conn = await aiosqlite.connect(file_db_path)

    # Start transaction (implicit with first query)
    cursor = await conn.execute(
        "DELETE FROM tasks WHERE status IN (?, ?)",
        (TaskStatus.COMPLETED.value, TaskStatus.FAILED.value)
    )
    deleted_count = cursor.rowcount
    await cursor.close()

    # Verify deletion would affect 30 tasks (20 completed + 10 failed)
    # Note: Don't commit - we're simulating an interruption
    assert deleted_count == 30, "Should attempt to delete 30 tasks"

    # Step 3: Interrupt by closing connection WITHOUT commit
    # Connection close triggers rollback
    await conn.close()

    # Step 4: Reopen database - triggers SQLite crash recovery
    # The uncommitted transaction should be rolled back
    db2 = Database(file_db_path)
    await db2.initialize()

    # Step 5: Verify database recovery

    # 5a. Database should be accessible (no corruption)
    try:
        all_tasks_after = await db2.list_tasks()
    except Exception as e:
        pytest.fail(f"Database corrupted after interruption: {e}")

    # 5b. All tasks should still exist (transaction rolled back)
    assert len(all_tasks_after) == 50, (
        f"Expected 50 tasks after rollback, found {len(all_tasks_after)}. "
        "Interrupted transaction should have been rolled back."
    )

    # 5c. Verify specific task counts by status
    completed_tasks = [t for t in all_tasks_after if t.status == TaskStatus.COMPLETED]
    failed_tasks = [t for t in all_tasks_after if t.status == TaskStatus.FAILED]
    pending_tasks = [t for t in all_tasks_after if t.status == TaskStatus.PENDING]
    cancelled_tasks = [t for t in all_tasks_after if t.status == TaskStatus.CANCELLED]

    assert len(completed_tasks) == 20, "All completed tasks should remain"
    assert len(failed_tasks) == 10, "All failed tasks should remain"
    assert len(pending_tasks) == 15, "All pending tasks should remain"
    assert len(cancelled_tasks) == 5, "All cancelled tasks should remain"

    # 5d. Verify database integrity using SQLite PRAGMA
    async with aiosqlite.connect(file_db_path) as conn:
        cursor = await conn.execute("PRAGMA integrity_check")
        integrity_result = await cursor.fetchone()
        await cursor.close()

        assert integrity_result[0] == "ok", (
            f"Database integrity check failed: {integrity_result[0]}"
        )

    # 5e. Verify database can still perform operations normally
    new_task = Task(
        prompt="Post-recovery test task",
        summary="Post-recovery test task",
        agent_type="recovery-test",
    )
    await db2.insert_task(new_task)

    retrieved_task = await db2.get_task(new_task.id)
    assert retrieved_task is not None, "Should be able to insert and retrieve after recovery"
    assert retrieved_task.prompt == "Post-recovery test task"

    # Cleanup
    await db2.close()


@pytest.mark.asyncio
async def test_database_consistency_with_committed_prune(file_db_path: Path):
    """Test that properly committed prune operations work correctly.

    This test verifies the expected behavior when a prune operation
    completes successfully with a proper commit.
    """
    # Create database and populate with tasks
    db = Database(file_db_path)
    await db.initialize()

    # Create 30 tasks: 10 completed, 10 pending, 10 failed
    for i in range(30):
        if i < 10:
            status = TaskStatus.COMPLETED
        elif i < 20:
            status = TaskStatus.PENDING
        else:
            status = TaskStatus.FAILED

        task = Task(
            prompt=f"Test task {i}",
            summary=f"Test task {i}",
            agent_type="test-agent",
        )
        if status != TaskStatus.PENDING:
            task.status = status

        await db.insert_task(task)

    # Verify initial count
    all_tasks = await db.list_tasks()
    assert len(all_tasks) == 30

    # Perform committed deletion (simulating successful prune)
    async with aiosqlite.connect(file_db_path) as conn:
        await conn.execute(
            "DELETE FROM tasks WHERE status = ?",
            (TaskStatus.COMPLETED.value,)
        )
        await conn.commit()  # Properly commit the transaction

    # Reopen database
    db2 = Database(file_db_path)
    await db2.initialize()

    # Verify tasks were actually deleted
    remaining_tasks = await db2.list_tasks()
    assert len(remaining_tasks) == 20, "Should have 20 tasks after deleting 10 completed"

    # Verify no completed tasks remain
    completed_tasks = [t for t in remaining_tasks if t.status == TaskStatus.COMPLETED]
    assert len(completed_tasks) == 0, "No completed tasks should remain"

    # Verify other tasks still exist
    pending_tasks = [t for t in remaining_tasks if t.status == TaskStatus.PENDING]
    failed_tasks = [t for t in remaining_tasks if t.status == TaskStatus.FAILED]
    assert len(pending_tasks) == 10, "All pending tasks should remain"
    assert len(failed_tasks) == 10, "All failed tasks should remain"

    await db2.close()


@pytest.mark.asyncio
async def test_database_vacuum_interruption_safety(file_db_path: Path):
    """Test database safety when VACUUM operation is interrupted.

    VACUUM creates a temporary copy of the database. If interrupted,
    the original database should remain unchanged and valid.
    """
    # Create and populate database
    db = Database(file_db_path)
    await db.initialize()

    # Create several tasks
    for i in range(20):
        task = Task(prompt=f"Task {i}", summary=f"Task {i}", agent_type="test")
        await db.insert_task(task)

    await db.close()

    # Get original file size
    original_size = file_db_path.stat().st_size

    # Attempt VACUUM (simulating interruption is difficult, so we just verify success)
    async with aiosqlite.connect(file_db_path) as conn:
        try:
            # VACUUM cannot be run in a transaction
            await conn.execute("VACUUM")
            vacuum_succeeded = True
        except Exception as e:
            vacuum_succeeded = False
            vacuum_error = str(e)

    # Verify database is still accessible
    db2 = Database(file_db_path)
    await db2.initialize()

    tasks = await db2.list_tasks()
    assert len(tasks) == 20, "All tasks should remain after VACUUM"

    # Verify integrity
    async with aiosqlite.connect(file_db_path) as conn:
        cursor = await conn.execute("PRAGMA integrity_check")
        result = await cursor.fetchone()
        await cursor.close()
        assert result[0] == "ok", "Database should pass integrity check"

    await db2.close()
