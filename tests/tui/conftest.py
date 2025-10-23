"""Shared fixtures for TUI tests.

This module provides reusable test fixtures for testing Textual TUI components.
"""

from datetime import datetime, timezone
from uuid import UUID, uuid4

import pytest

from abathur.domain.models import Task, TaskStatus


@pytest.fixture
def simple_parent_child_tasks() -> list[Task]:
    """Create simple parent-child task hierarchy for testing.

    Structure: 1 parent with 2 children
    - Parent task (PENDING, priority 8)
      - Child task 1 (READY, priority 5)
      - Child task 2 (COMPLETED, priority 3)

    Returns:
        List of Task objects with hierarchical relationships
    """
    parent_id = uuid4()
    child1_id = uuid4()
    child2_id = uuid4()

    now = datetime.now(timezone.utc)

    return [
        Task(
            id=parent_id,
            summary="Parent task",
            prompt="Parent task description",
            priority=8,
            calculated_priority=8.0,
            status=TaskStatus.PENDING,
            submitted_at=now,
            last_updated_at=now,
        ),
        Task(
            id=child1_id,
            summary="Child task 1",
            prompt="Child task 1 description",
            priority=5,
            calculated_priority=5.0,
            status=TaskStatus.READY,
            parent_task_id=parent_id,
            submitted_at=now,
            last_updated_at=now,
        ),
        Task(
            id=child2_id,
            summary="Child task 2",
            prompt="Child task 2 description",
            priority=3,
            calculated_priority=3.0,
            status=TaskStatus.COMPLETED,
            parent_task_id=parent_id,
            submitted_at=now,
            last_updated_at=now,
        ),
    ]
