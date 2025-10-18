"""Unit tests for Database.delete_tasks() method.

Tests CASCADE deletion behavior, edge cases, and correct row count returns.
"""

import pytest
from datetime import datetime, timezone
from pathlib import Path
from uuid import uuid4

from abathur.domain.models import Task, TaskStatus, TaskSource, DependencyType, TaskDependency
from abathur.infrastructure.database import Database, DeleteResult


@pytest.fixture
async def db():
    """Create an in-memory database for testing."""
    database = Database(Path(":memory:"))
    await database.initialize()
    yield database
    await database.close()


@pytest.fixture
def sample_task():
    """Create a sample task for testing."""
    return Task(
        id=uuid4(),
        prompt="Test task",
        agent_type="test-agent",
        priority=5,
        status=TaskStatus.COMPLETED,
        input_data={"test": "data"},
        submitted_at=datetime.now(timezone.utc),
        last_updated_at=datetime.now(timezone.utc),
        source=TaskSource.HUMAN,
        dependency_type=DependencyType.SEQUENTIAL,
        summary="Test task summary",
    )


@pytest.mark.asyncio
async def test_delete_single_task(db: Database, sample_task: Task):
    """Test deleting a single task returns count=1."""
    # Insert task
    await db.insert_task(sample_task)

    # Verify task exists
    retrieved = await db.get_task(sample_task.id)
    assert retrieved is not None
    assert retrieved.id == sample_task.id

    # Delete task
    result = await db.delete_tasks([sample_task.id])

    # Verify deletion
    assert isinstance(result, DeleteResult)
    assert result.deleted_count == 1
    assert result.blocked_deletions == []
    assert result.errors == []

    # Verify task no longer exists
    retrieved_after = await db.get_task(sample_task.id)
    assert retrieved_after is None


@pytest.mark.asyncio
async def test_delete_multiple_tasks(db: Database):
    """Test deleting multiple tasks returns count=N."""
    # Create and insert 3 tasks
    tasks = [
        Task(
            id=uuid4(),
            prompt=f"Test task {i}",
            agent_type="test-agent",
            priority=5,
            status=TaskStatus.COMPLETED,
            input_data={"test": f"data{i}"},
            submitted_at=datetime.now(timezone.utc),
            last_updated_at=datetime.now(timezone.utc),
            source=TaskSource.HUMAN,
            dependency_type=DependencyType.SEQUENTIAL,
            summary=f"Test task {i} summary",
        )
        for i in range(3)
    ]

    for task in tasks:
        await db.insert_task(task)

    # Verify all tasks exist
    for task in tasks:
        assert await db.get_task(task.id) is not None

    # Delete all tasks
    task_ids = [task.id for task in tasks]
    result = await db.delete_tasks(task_ids)

    # Verify deletion count
    assert isinstance(result, DeleteResult)
    assert result.deleted_count == 3
    assert result.blocked_deletions == []
    assert result.errors == []

    # Verify all tasks are gone
    for task in tasks:
        assert await db.get_task(task.id) is None


@pytest.mark.asyncio
async def test_delete_nonexistent_task_returns_zero(db: Database):
    """Test deleting non-existent task returns count=0 without error."""
    # Generate a random UUID that doesn't exist in database
    nonexistent_id = uuid4()

    # Delete non-existent task
    result = await db.delete_tasks([nonexistent_id])

    # Verify count is 0
    assert isinstance(result, DeleteResult)
    assert result.deleted_count == 0
    assert result.blocked_deletions == []
    assert result.errors == []


@pytest.mark.asyncio
async def test_delete_mixed_existing_nonexistent(db: Database, sample_task: Task):
    """Test deleting mix of valid/invalid IDs returns partial count."""
    # Insert one task
    await db.insert_task(sample_task)

    # Create list with 1 existing + 2 non-existent IDs
    task_ids = [sample_task.id, uuid4(), uuid4()]

    # Delete all IDs
    result = await db.delete_tasks(task_ids)

    # Verify only 1 task was deleted
    assert isinstance(result, DeleteResult)
    assert result.deleted_count == 1
    assert result.blocked_deletions == []
    assert result.errors == []

    # Verify the real task is gone
    assert await db.get_task(sample_task.id) is None


@pytest.mark.asyncio
async def test_delete_cascades_to_task_dependencies(db: Database):
    """Test CASCADE deletion removes task_dependencies records."""
    # Create two tasks: prerequisite and dependent
    prerequisite_task = Task(
        id=uuid4(),
        prompt="Prerequisite task",
        agent_type="test-agent",
        priority=5,
        status=TaskStatus.COMPLETED,
        input_data={},
        submitted_at=datetime.now(timezone.utc),
        last_updated_at=datetime.now(timezone.utc),
        source=TaskSource.HUMAN,
        dependency_type=DependencyType.SEQUENTIAL,
        summary="Prerequisite task",
    )

    dependent_task = Task(
        id=uuid4(),
        prompt="Dependent task",
        agent_type="test-agent",
        priority=5,
        status=TaskStatus.PENDING,
        input_data={},
        submitted_at=datetime.now(timezone.utc),
        last_updated_at=datetime.now(timezone.utc),
        source=TaskSource.HUMAN,
        dependency_type=DependencyType.SEQUENTIAL,
        summary="Dependent task",
    )

    await db.insert_task(prerequisite_task)
    await db.insert_task(dependent_task)

    # Create dependency relationship
    dependency = TaskDependency(
        id=uuid4(),
        dependent_task_id=dependent_task.id,
        prerequisite_task_id=prerequisite_task.id,
        dependency_type=DependencyType.SEQUENTIAL,
        created_at=datetime.now(timezone.utc),
    )
    await db.insert_task_dependency(dependency)

    # Verify dependency exists
    dependencies = await db.get_task_dependencies(dependent_task.id)
    assert len(dependencies) == 1
    assert dependencies[0].prerequisite_task_id == prerequisite_task.id

    # Delete the prerequisite task (should CASCADE delete dependency)
    result = await db.delete_tasks([prerequisite_task.id])
    assert isinstance(result, DeleteResult)
    assert result.deleted_count == 1
    assert result.blocked_deletions == []
    assert result.errors == []

    # Verify dependency was CASCADE deleted
    dependencies_after = await db.get_task_dependencies(dependent_task.id)
    assert len(dependencies_after) == 0


@pytest.mark.asyncio
async def test_delete_preserves_audit_records(db: Database, sample_task: Task):
    """Test that audit records are preserved when task is deleted."""
    # Insert task
    await db.insert_task(sample_task)

    # Create audit record (no FK constraint on task_id)
    await db.log_audit(
        task_id=sample_task.id,
        action_type="TEST_ACTION",
        action_data={"test": "data"},
        result="success",
    )

    # Verify audit record exists
    async with db._get_connection() as conn:
        cursor = await conn.execute(
            "SELECT COUNT(*) as count FROM audit WHERE task_id = ?",
            (str(sample_task.id),),
        )
        row = await cursor.fetchone()
        assert row["count"] == 1

    # Delete task
    result = await db.delete_tasks([sample_task.id])
    assert isinstance(result, DeleteResult)
    assert result.deleted_count == 1
    assert result.blocked_deletions == []
    assert result.errors == []

    # Verify audit record is still there (orphaned, but preserved)
    async with db._get_connection() as conn:
        cursor = await conn.execute(
            "SELECT COUNT(*) as count FROM audit WHERE task_id = ?",
            (str(sample_task.id),),
        )
        row = await cursor.fetchone()
        assert row["count"] == 1  # Audit record preserved


@pytest.mark.asyncio
async def test_delete_orphans_agents_acceptably(db: Database, sample_task: Task):
    """Test that agents are orphaned when task is deleted (expected behavior)."""
    from abathur.domain.models import Agent, AgentState

    # Insert task
    await db.insert_task(sample_task)

    # Create agent linked to task
    agent = Agent(
        id=uuid4(),
        name="Test Agent",
        specialization="testing",
        task_id=sample_task.id,
        state=AgentState.BUSY,
        model="test-model",
        spawned_at=datetime.now(timezone.utc),
        resource_usage={},
    )
    await db.insert_agent(agent)

    # Verify agent exists
    async with db._get_connection() as conn:
        cursor = await conn.execute(
            "SELECT COUNT(*) as count FROM agents WHERE task_id = ?",
            (str(sample_task.id),),
        )
        row = await cursor.fetchone()
        assert row["count"] == 1

    # Delete task (should CASCADE delete agent due to FK with ON DELETE CASCADE)
    result = await db.delete_tasks([sample_task.id])
    assert isinstance(result, DeleteResult)
    assert result.deleted_count == 1
    assert result.blocked_deletions == []
    assert result.errors == []

    # Verify agent was CASCADE deleted (updated behavior after migration)
    async with db._get_connection() as conn:
        cursor = await conn.execute(
            "SELECT COUNT(*) as count FROM agents WHERE task_id = ?",
            (str(sample_task.id),),
        )
        row = await cursor.fetchone()
        # After the migration adding CASCADE DELETE to agents.task_id FK,
        # agents should be deleted (not orphaned)
        assert row["count"] == 0  # Agent CASCADE deleted


@pytest.mark.asyncio
async def test_delete_empty_list_behavior(db: Database):
    """Test that deleting with empty list raises ValueError."""
    # The delete_tasks method now validates input and raises ValueError
    # for empty task_ids list to prevent accidental no-op calls
    with pytest.raises(ValueError, match="task_ids cannot be empty"):
        await db.delete_tasks([])


@pytest.mark.asyncio
async def test_delete_duplicate_ids(db: Database, sample_task: Task):
    """Test deleting same ID multiple times in list only counts once."""
    # Insert task
    await db.insert_task(sample_task)

    # Delete with duplicate IDs in list
    result = await db.delete_tasks([sample_task.id, sample_task.id, sample_task.id])

    # Should only delete once (SQL IN clause deduplicates)
    assert isinstance(result, DeleteResult)
    assert result.deleted_count == 1
    assert result.blocked_deletions == []
    assert result.errors == []

    # Verify task is gone
    assert await db.get_task(sample_task.id) is None


@pytest.mark.asyncio
async def test_delete_tasks_returns_delete_result_model(db: Database, sample_task: Task):
    """Test that delete_tasks returns properly typed DeleteResult."""
    await db.insert_task(sample_task)

    result = await db.delete_tasks([sample_task.id])

    # Verify type
    assert isinstance(result, DeleteResult)

    # Verify all required fields exist
    assert hasattr(result, "deleted_count")
    assert hasattr(result, "blocked_deletions")
    assert hasattr(result, "errors")

    # Verify field types and values
    assert isinstance(result.deleted_count, int)
    assert isinstance(result.blocked_deletions, list)
    assert isinstance(result.errors, list)
    assert result.deleted_count == 1
    assert result.blocked_deletions == []
    assert result.errors == []


# ============================================================================
# Unit Tests for delete_tasks_by_status() Validation (Issue #5)
# ============================================================================


@pytest.mark.asyncio
class TestDeleteTasksByStatusValidation:
    """Unit tests for validation logic in delete_tasks_by_status().

    These tests verify that the method properly validates status parameters
    and prevents deletion of active tasks (PENDING, BLOCKED, READY, RUNNING).
    Only terminal statuses (COMPLETED, FAILED, CANCELLED) should be allowed.

    Related to Issue #5: Missing Validation in delete_tasks_by_status()
    """

    async def test_rejects_pending_status(self, db: Database):
        """Should raise ValueError when attempting to delete PENDING tasks.

        PENDING tasks are actively queued and should not be deleted.
        Users should cancel tasks first before deleting.
        """
        # Arrange - no setup needed, testing validation before query

        # Act & Assert
        with pytest.raises(ValueError, match=r"Cannot delete tasks with status pending"):
            await db.delete_tasks_by_status(TaskStatus.PENDING)

    async def test_rejects_blocked_status(self, db: Database):
        """Should raise ValueError when attempting to delete BLOCKED tasks.

        BLOCKED tasks are waiting for dependencies to complete.
        They should not be deleted while active in the queue.
        """
        # Arrange - no setup needed, testing validation before query

        # Act & Assert
        with pytest.raises(ValueError, match=r"Cannot delete tasks with status blocked"):
            await db.delete_tasks_by_status(TaskStatus.BLOCKED)

    async def test_rejects_ready_status(self, db: Database):
        """Should raise ValueError when attempting to delete READY tasks.

        READY tasks have met dependencies and are queued for execution.
        They should not be deleted while waiting to run.
        """
        # Arrange - no setup needed, testing validation before query

        # Act & Assert
        with pytest.raises(ValueError, match=r"Cannot delete tasks with status ready"):
            await db.delete_tasks_by_status(TaskStatus.READY)

    async def test_rejects_running_status(self, db: Database):
        """Should raise ValueError when attempting to delete RUNNING tasks.

        RUNNING tasks are actively executing and must not be deleted.
        Users should wait for completion or cancel first.
        """
        # Arrange - no setup needed, testing validation before query

        # Act & Assert
        with pytest.raises(ValueError, match=r"Cannot delete tasks with status running"):
            await db.delete_tasks_by_status(TaskStatus.RUNNING)

    async def test_allows_completed_status(self, db: Database):
        """Should successfully delete COMPLETED tasks without raising ValueError.

        COMPLETED tasks are terminal state and safe to delete.
        """
        # Arrange - create a completed task to delete
        completed_task = Task(
            id=uuid4(),
            prompt="Completed task for deletion test",
            agent_type="test-agent",
            priority=5,
            status=TaskStatus.COMPLETED,
            input_data={"test": "data"},
            submitted_at=datetime.now(timezone.utc),
            last_updated_at=datetime.now(timezone.utc),
            source=TaskSource.HUMAN,
            dependency_type=DependencyType.SEQUENTIAL,
            summary="Completed task",
        )
        await db.insert_task(completed_task)

        # Act - should not raise ValueError
        result = await db.delete_tasks_by_status(TaskStatus.COMPLETED)

        # Assert - returns integer rowcount (1 task deleted)
        assert isinstance(result, int)
        assert result == 1

        # Verify task was actually deleted
        retrieved = await db.get_task(completed_task.id)
        assert retrieved is None

    async def test_allows_failed_status(self, db: Database):
        """Should successfully delete FAILED tasks without raising ValueError.

        FAILED tasks are terminal state and safe to delete.
        """
        # Arrange - create a failed task to delete
        failed_task = Task(
            id=uuid4(),
            prompt="Failed task for deletion test",
            agent_type="test-agent",
            priority=5,
            status=TaskStatus.FAILED,
            input_data={"test": "data"},
            submitted_at=datetime.now(timezone.utc),
            last_updated_at=datetime.now(timezone.utc),
            source=TaskSource.HUMAN,
            dependency_type=DependencyType.SEQUENTIAL,
            summary="Failed task",
            error_message="Task failed during execution",
        )
        await db.insert_task(failed_task)

        # Act - should not raise ValueError
        result = await db.delete_tasks_by_status(TaskStatus.FAILED)

        # Assert - returns integer rowcount (1 task deleted)
        assert isinstance(result, int)
        assert result == 1

        # Verify task was actually deleted
        retrieved = await db.get_task(failed_task.id)
        assert retrieved is None

    async def test_allows_cancelled_status(self, db: Database):
        """Should successfully delete CANCELLED tasks without raising ValueError.

        CANCELLED tasks are terminal state and safe to delete.
        """
        # Arrange - create a cancelled task to delete
        cancelled_task = Task(
            id=uuid4(),
            prompt="Cancelled task for deletion test",
            agent_type="test-agent",
            priority=5,
            status=TaskStatus.CANCELLED,
            input_data={"test": "data"},
            submitted_at=datetime.now(timezone.utc),
            last_updated_at=datetime.now(timezone.utc),
            source=TaskSource.HUMAN,
            dependency_type=DependencyType.SEQUENTIAL,
            summary="Cancelled task",
        )
        await db.insert_task(cancelled_task)

        # Act - should not raise ValueError
        result = await db.delete_tasks_by_status(TaskStatus.CANCELLED)

        # Assert - returns integer rowcount (1 task deleted)
        assert isinstance(result, int)
        assert result == 1

        # Verify task was actually deleted
        retrieved = await db.get_task(cancelled_task.id)
        assert retrieved is None

    async def test_validation_error_message_includes_forbidden_status(self, db: Database):
        """Verify error message clearly states which status was rejected.

        Good error messages help users understand what went wrong and how to fix it.
        """
        # Test that error message includes the specific forbidden status
        with pytest.raises(ValueError) as exc_info:
            await db.delete_tasks_by_status(TaskStatus.PENDING)

        error_message = str(exc_info.value)
        assert "pending" in error_message.lower()
        assert "cannot delete" in error_message.lower()

    async def test_validation_error_message_suggests_allowed_statuses(self, db: Database):
        """Verify error message mentions which statuses ARE allowed.

        Helpful error messages guide users toward the correct solution.
        """
        # Test that error message mentions allowed statuses
        with pytest.raises(ValueError) as exc_info:
            await db.delete_tasks_by_status(TaskStatus.RUNNING)

        error_message = str(exc_info.value)
        # Error should mention at least one allowed status
        assert any(status in error_message.lower() for status in ["completed", "failed", "cancelled"])

    async def test_validation_happens_before_database_query(self, db: Database):
        """Verify validation occurs before any database operations.

        This ensures no partial state changes if validation fails.
        Fast-fail validation improves performance and safety.
        """
        # Arrange - count initial queries (approximate check)
        # We can't directly count queries, but we can verify no deletion occurred

        # Create a PENDING task
        pending_task = Task(
            id=uuid4(),
            prompt="Task that should not be deleted",
            agent_type="test-agent",
            priority=5,
            status=TaskStatus.PENDING,
            input_data={},
            submitted_at=datetime.now(timezone.utc),
            last_updated_at=datetime.now(timezone.utc),
            source=TaskSource.HUMAN,
            dependency_type=DependencyType.SEQUENTIAL,
            summary="Pending task",
        )
        await db.insert_task(pending_task)

        # Act - attempt to delete PENDING status (should fail validation)
        with pytest.raises(ValueError):
            await db.delete_tasks_by_status(TaskStatus.PENDING)

        # Assert - task should still exist (no deletion occurred)
        retrieved = await db.get_task(pending_task.id)
        assert retrieved is not None
        assert retrieved.status == TaskStatus.PENDING

    async def test_deletes_multiple_tasks_with_same_allowed_status(self, db: Database):
        """Verify bulk deletion works correctly for allowed statuses.

        Tests that validation doesn't interfere with normal bulk delete operations.
        """
        # Arrange - create 3 completed tasks
        completed_tasks = [
            Task(
                id=uuid4(),
                prompt=f"Completed task {i}",
                agent_type="test-agent",
                priority=5,
                status=TaskStatus.COMPLETED,
                input_data={},
                submitted_at=datetime.now(timezone.utc),
                last_updated_at=datetime.now(timezone.utc),
                source=TaskSource.HUMAN,
                dependency_type=DependencyType.SEQUENTIAL,
                summary=f"Completed task {i}",
            )
            for i in range(3)
        ]

        for task in completed_tasks:
            await db.insert_task(task)

        # Act - delete all completed tasks
        result = await db.delete_tasks_by_status(TaskStatus.COMPLETED)

        # Assert - all 3 tasks deleted
        assert result == 3

        # Verify all tasks are gone
        for task in completed_tasks:
            retrieved = await db.get_task(task.id)
            assert retrieved is None

    async def test_returns_zero_when_no_tasks_match_allowed_status(self, db: Database):
        """Verify method returns 0 when no tasks match the allowed status.

        Edge case: deleting COMPLETED tasks when none exist should return 0, not error.
        """
        # Arrange - create tasks with different statuses, but no COMPLETED tasks
        await db.insert_task(Task(
            id=uuid4(),
            prompt="Pending task",
            agent_type="test-agent",
            priority=5,
            status=TaskStatus.PENDING,
            input_data={},
            submitted_at=datetime.now(timezone.utc),
            last_updated_at=datetime.now(timezone.utc),
            source=TaskSource.HUMAN,
            dependency_type=DependencyType.SEQUENTIAL,
            summary="Pending task",
        ))

        # Act - try to delete COMPLETED tasks (none exist)
        result = await db.delete_tasks_by_status(TaskStatus.COMPLETED)

        # Assert - returns 0 (no tasks deleted)
        assert result == 0
