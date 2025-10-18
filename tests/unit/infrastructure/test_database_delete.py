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
