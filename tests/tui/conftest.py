"""Pytest fixtures for TUI tests."""

from datetime import datetime, timezone
from uuid import UUID, uuid4

import pytest
from abathur.domain.models import Task, TaskSource, TaskStatus


@pytest.fixture(params=[0, 1, 10, 50, 100])
def large_child_count_tasks(request) -> list[Task]:
    """Parent task with parametrized child count.

    Params: 0, 1, 10, 50, 100 children
    Used for performance and edge case testing.

    Returns:
        List containing parent task followed by child tasks.
        The parent task is always at index 0.
        All children have parent_task_id set to parent's ID.

    Example:
        def test_rendering(large_child_count_tasks):
            tasks = large_child_count_tasks
            parent = tasks[0]
            children = tasks[1:]
            assert len(children) == request.param
    """
    n = request.param
    parent_id: UUID = uuid4()

    # Create parent task
    parent = Task(
        id=parent_id,
        summary=f"Parent with {n} children",
        prompt=f"Parent task with {n} child tasks",
        parent_task_id=None,
        status=TaskStatus.PENDING,
        source=TaskSource.HUMAN,
        agent_type="requirements-gatherer",
        priority=5,
        submitted_at=datetime.now(timezone.utc),
        last_updated_at=datetime.now(timezone.utc),
    )

    # Create child tasks
    children = [
        Task(
            id=uuid4(),
            summary=f"Child {i+1}/{n}",
            prompt=f"Child task {i+1} of {n}",
            parent_task_id=parent_id,
            status=TaskStatus.PENDING,
            source=TaskSource.AGENT_PLANNER,
            agent_type="requirements-gatherer",
            priority=5,
            submitted_at=datetime.now(timezone.utc),
            last_updated_at=datetime.now(timezone.utc),
        )
        for i in range(n)
    ]

    return [parent] + children
