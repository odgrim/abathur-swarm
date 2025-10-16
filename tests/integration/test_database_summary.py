"""Integration tests for database summary field."""
import uuid
from datetime import datetime, timezone
from pathlib import Path

import pytest

from abathur.domain.models import Task, TaskSource, TaskStatus, DependencyType
from abathur.infrastructure.database import Database


@pytest.fixture
async def db():
    """Create in-memory database for testing."""
    db = Database(Path(":memory:"))
    await db.initialize()
    yield db
    await db.close()


@pytest.mark.asyncio
async def test_insert_task_with_summary(db):
    """Test inserting a task with a summary."""
    task = Task(
        id=uuid.uuid4(),
        prompt="Detailed instructions for implementing feature X with specific requirements",
        summary="Implement feature X",
        agent_type="python-backend-specialist",
        status=TaskStatus.PENDING,
        source=TaskSource.HUMAN,
        dependency_type=DependencyType.SEQUENTIAL,
        submitted_at=datetime.now(timezone.utc),
        last_updated_at=datetime.now(timezone.utc),
    )

    # Insert task
    await db.insert_task(task)

    # Retrieve task
    retrieved = await db.get_task(task.id)

    # Verify task was stored and retrieved correctly
    assert retrieved is not None
    assert retrieved.id == task.id
    assert retrieved.prompt == task.prompt
    assert retrieved.summary == "Implement feature X"
    assert retrieved.agent_type == task.agent_type


@pytest.mark.asyncio
async def test_insert_task_without_summary(db):
    """Test inserting a task without a summary (None)."""
    task = Task(
        id=uuid.uuid4(),
        prompt="Detailed instructions without a summary",
        summary=None,
        agent_type="general-purpose",
        status=TaskStatus.PENDING,
        source=TaskSource.HUMAN,
        dependency_type=DependencyType.SEQUENTIAL,
        submitted_at=datetime.now(timezone.utc),
        last_updated_at=datetime.now(timezone.utc),
    )

    # Insert task
    await db.insert_task(task)

    # Retrieve task
    retrieved = await db.get_task(task.id)

    # Verify task was stored and retrieved correctly
    assert retrieved is not None
    assert retrieved.id == task.id
    assert retrieved.prompt == task.prompt
    assert retrieved.summary is None
    assert retrieved.agent_type == task.agent_type


@pytest.mark.asyncio
async def test_summary_roundtrip(db):
    """Test that summary values roundtrip correctly through database."""
    tasks = [
        Task(
            id=uuid.uuid4(),
            prompt="Long prompt 1",
            summary="Short summary 1",
            agent_type="test-agent",
            status=TaskStatus.PENDING,
            source=TaskSource.HUMAN,
            dependency_type=DependencyType.SEQUENTIAL,
            submitted_at=datetime.now(timezone.utc),
            last_updated_at=datetime.now(timezone.utc),
        ),
        Task(
            id=uuid.uuid4(),
            prompt="Long prompt 2",
            summary=None,
            agent_type="test-agent",
            status=TaskStatus.PENDING,
            source=TaskSource.HUMAN,
            dependency_type=DependencyType.SEQUENTIAL,
            submitted_at=datetime.now(timezone.utc),
            last_updated_at=datetime.now(timezone.utc),
        ),
    ]

    # Insert both tasks
    for task in tasks:
        await db.insert_task(task)

    # Retrieve and verify
    retrieved_1 = await db.get_task(tasks[0].id)
    assert retrieved_1.summary == "Short summary 1"

    retrieved_2 = await db.get_task(tasks[1].id)
    assert retrieved_2.summary is None


@pytest.mark.asyncio
async def test_list_tasks_with_summary(db):
    """Test listing tasks preserves summary field."""
    task = Task(
        id=uuid.uuid4(),
        prompt="Task with summary",
        summary="My summary",
        agent_type="test-agent",
        status=TaskStatus.PENDING,
        source=TaskSource.HUMAN,
        dependency_type=DependencyType.SEQUENTIAL,
        submitted_at=datetime.now(timezone.utc),
        last_updated_at=datetime.now(timezone.utc),
    )

    await db.insert_task(task)

    # List tasks
    tasks = await db.list_tasks(status=TaskStatus.PENDING, limit=10)

    # Verify summary is preserved
    assert len(tasks) == 1
    assert tasks[0].summary == "My summary"
