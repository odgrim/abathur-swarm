"""Tests for task timeout and cancellation features."""

import asyncio
from datetime import datetime, timedelta, timezone
from pathlib import Path

import pytest
from abathur.application.task_coordinator import TaskCoordinator
from abathur.domain.models import Task, TaskStatus
from abathur.infrastructure.database import Database


@pytest.fixture
async def database(tmp_path: Path) -> Database:
    """Create a test database."""
    db_path = tmp_path / "test.db"
    db = Database(db_path)
    await db.initialize()
    return db


@pytest.fixture
async def task_coordinator(database: Database) -> TaskCoordinator:
    """Create a task coordinator."""
    return TaskCoordinator(database)


@pytest.mark.asyncio
async def test_task_has_last_updated_at(task_coordinator: TaskCoordinator) -> None:
    """Test that tasks have last_updated_at field."""
    task = Task(
        prompt="Test task",
        summary="Test task",
        agent_type="general",
        priority=5,
    )

    task_id = await task_coordinator.submit_task(task)
    retrieved_task = await task_coordinator.get_task(task_id)

    assert retrieved_task is not None
    assert retrieved_task.last_updated_at is not None
    assert isinstance(retrieved_task.last_updated_at, datetime)


@pytest.mark.asyncio
async def test_task_has_timeout_field(task_coordinator: TaskCoordinator) -> None:
    """Test that tasks have max_execution_timeout_seconds field."""
    task = Task(
        prompt="Test task",
        summary="Test task",
        agent_type="general",
        priority=5,
        max_execution_timeout_seconds=1800,  # 30 minutes
    )

    task_id = await task_coordinator.submit_task(task)
    retrieved_task = await task_coordinator.get_task(task_id)

    assert retrieved_task is not None
    assert retrieved_task.max_execution_timeout_seconds == 1800


@pytest.mark.asyncio
async def test_last_updated_at_updates_on_status_change(
    task_coordinator: TaskCoordinator,
) -> None:
    """Test that last_updated_at is updated when status changes."""
    task = Task(
        prompt="Test task",
        summary="Test task",
        agent_type="general",
        priority=5,
    )

    task_id = await task_coordinator.submit_task(task)
    initial_task = await task_coordinator.get_task(task_id)
    assert initial_task is not None
    initial_time = initial_task.last_updated_at

    # Wait a bit to ensure time difference
    await asyncio.sleep(0.1)

    # Update status
    await task_coordinator.update_task_status(task_id, TaskStatus.RUNNING)
    updated_task = await task_coordinator.get_task(task_id)

    assert updated_task is not None
    assert updated_task.last_updated_at > initial_time


@pytest.mark.asyncio
async def test_cancel_pending_task(task_coordinator: TaskCoordinator) -> None:
    """Test canceling a pending task."""
    task = Task(
        prompt="Test task",
        summary="Test task",
        agent_type="general",
        priority=5,
    )

    task_id = await task_coordinator.submit_task(task)

    # Cancel pending task using update_task_status
    await task_coordinator.update_task_status(task_id, TaskStatus.CANCELLED)

    # Verify task is cancelled
    cancelled_task = await task_coordinator.get_task(task_id)
    assert cancelled_task is not None
    assert cancelled_task.status == TaskStatus.CANCELLED


@pytest.mark.asyncio
async def test_cancel_running_task_with_force(task_coordinator: TaskCoordinator) -> None:
    """Test canceling a running task with force flag."""
    task = Task(
        prompt="Test task",
        summary="Test task",
        agent_type="general",
        priority=5,
    )

    task_id = await task_coordinator.submit_task(task)
    await task_coordinator.update_task_status(task_id, TaskStatus.RUNNING)

    # Cancel running task using update_task_status with error message
    await task_coordinator.update_task_status(task_id, TaskStatus.CANCELLED, error_message='Task cancelled by user')

    # Verify task is cancelled
    cancelled_task = await task_coordinator.get_task(task_id)
    assert cancelled_task is not None
    assert cancelled_task.status == TaskStatus.CANCELLED
    assert cancelled_task.error_message == "Task cancelled by user"


@pytest.mark.asyncio
async def test_get_stale_running_tasks(database: Database) -> None:
    """Test detecting stale running tasks."""
    # Create a task that started long ago
    old_time = datetime.now(timezone.utc) - timedelta(hours=2)
    stale_task = Task(
        prompt="Stale task",
        summary="Stale task",
        agent_type="general",
        priority=5,
        status=TaskStatus.RUNNING,
        started_at=old_time,
        last_updated_at=old_time,
        max_execution_timeout_seconds=3600,  # 1 hour timeout
    )

    await database.insert_task(stale_task)

    # Create a fresh running task
    fresh_task = Task(
        prompt="Fresh task",
        summary="Fresh task",
        agent_type="general",
        priority=5,
        status=TaskStatus.RUNNING,
        started_at=datetime.now(timezone.utc),
        last_updated_at=datetime.now(timezone.utc),
        max_execution_timeout_seconds=3600,
    )

    await database.insert_task(fresh_task)

    # Get stale tasks
    stale_tasks = await database.get_stale_running_tasks()

    # Only the stale task should be returned
    assert len(stale_tasks) == 1
    assert stale_tasks[0].id == stale_task.id


@pytest.mark.asyncio
async def test_handle_stale_tasks_retry(
    task_coordinator: TaskCoordinator, database: Database
) -> None:
    """Test handling stale tasks that should be retried."""
    # Create a stale task with retries available
    old_time = datetime.now(timezone.utc) - timedelta(hours=2)
    task = Task(
        prompt="Stale task",
        summary="Stale task",
        agent_type="general",
        priority=5,
        status=TaskStatus.RUNNING,
        started_at=old_time,
        last_updated_at=old_time,
        max_execution_timeout_seconds=3600,
        retry_count=0,
        max_retries=3,
    )

    await database.insert_task(task)

    # Handle stale tasks
    handled_ids = await task_coordinator.handle_stale_tasks()

    assert len(handled_ids) == 1
    assert task.id in handled_ids

    # Verify task was reset to pending
    updated_task = await task_coordinator.get_task(task.id)
    assert updated_task is not None
    assert updated_task.status == TaskStatus.PENDING
    assert updated_task.retry_count == 1


@pytest.mark.asyncio
async def test_handle_stale_tasks_max_retries(
    task_coordinator: TaskCoordinator, database: Database
) -> None:
    """Test handling stale tasks that have exceeded max retries."""
    # Create a stale task with max retries reached
    old_time = datetime.now(timezone.utc) - timedelta(hours=2)
    task = Task(
        prompt="Stale task",
        summary="Stale task",
        agent_type="general",
        priority=5,
        status=TaskStatus.RUNNING,
        started_at=old_time,
        last_updated_at=old_time,
        max_execution_timeout_seconds=3600,
        retry_count=2,
        max_retries=3,
    )

    await database.insert_task(task)

    # Handle stale tasks
    handled_ids = await task_coordinator.handle_stale_tasks()

    assert len(handled_ids) == 1
    assert task.id in handled_ids

    # Verify task was marked as failed
    updated_task = await task_coordinator.get_task(task.id)
    assert updated_task is not None
    assert updated_task.status == TaskStatus.FAILED
    assert updated_task.retry_count == 3
    assert updated_task.error_message is not None
    assert "timeout" in updated_task.error_message.lower()
    assert "max retries" in updated_task.error_message.lower()


@pytest.mark.asyncio
async def test_handle_stale_tasks_empty(task_coordinator: TaskCoordinator) -> None:
    """Test handling stale tasks when there are none."""
    # Don't create any stale tasks
    handled_ids = await task_coordinator.handle_stale_tasks()

    assert len(handled_ids) == 0


@pytest.mark.asyncio
async def test_increment_retry_count(database: Database) -> None:
    """Test incrementing task retry count."""
    task = Task(
        prompt="Test task",
        summary="Test task",
        agent_type="general",
        priority=5,
        retry_count=0,
    )

    await database.insert_task(task)

    # Increment retry count
    await database.increment_task_retry_count(task.id)

    # Verify retry count was incremented
    updated_task = await database.get_task(task.id)
    assert updated_task is not None
    assert updated_task.retry_count == 1

    # Increment again
    await database.increment_task_retry_count(task.id)
    updated_task = await database.get_task(task.id)
    assert updated_task is not None
    assert updated_task.retry_count == 2
