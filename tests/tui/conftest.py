"""Test fixtures for TUI parent-child hierarchy tests.

This module provides reusable pytest fixtures for creating hierarchical task
test data following the patterns defined in the test_fixtures_architecture
technical specification.
"""

from datetime import datetime, timezone
from uuid import uuid4

import pytest

from abathur.domain.models import Task, TaskStatus


@pytest.fixture
def multi_level_hierarchy_tasks() -> list[Task]:
    """3-level task hierarchy for multi-level nesting tests.

    Creates a hierarchical structure with 4 tasks across 3 levels:
    - 1 root task (depth 0)
    - 2 child tasks (depth 1)
    - 1 grandchild task (depth 2)

    Structure:
        Root (depth 0, parent_task_id=None)
        ├─ Child 1A (depth 1, parent_task_id=root_id)
        │  └─ Grandchild 1A1 (depth 2, parent_task_id=child_1a_id)
        └─ Child 1B (depth 1, parent_task_id=root_id)

    Use cases:
        - FR003: Multi-level hierarchy display
        - FR003: Independent expand/collapse at each level
        - Navigation across 3 levels of nesting

    Returns:
        list[Task]: List of 4 tasks in hierarchical structure
    """
    # Create unique IDs for each task
    root_id = uuid4()
    child_1a_id = uuid4()
    child_1b_id = uuid4()
    grandchild_id = uuid4()

    # Common timestamp for all tasks
    now = datetime.now(timezone.utc)

    # Root task (depth 0)
    root = Task(
        id=root_id,
        summary="Root Task",
        prompt="Root task for multi-level hierarchy testing",
        parent_task_id=None,
        status=TaskStatus.READY,
        priority=5,
        submitted_at=now,
        last_updated_at=now,
    )

    # Child 1A (depth 1)
    child_1a = Task(
        id=child_1a_id,
        summary="Child 1A",
        prompt="First child of root task",
        parent_task_id=root_id,
        status=TaskStatus.READY,
        priority=5,
        submitted_at=now,
        last_updated_at=now,
    )

    # Child 1B (depth 1)
    child_1b = Task(
        id=child_1b_id,
        summary="Child 1B",
        prompt="Second child of root task",
        parent_task_id=root_id,
        status=TaskStatus.COMPLETED,
        priority=5,
        submitted_at=now,
        last_updated_at=now,
    )

    # Grandchild 1A1 (depth 2)
    grandchild = Task(
        id=grandchild_id,
        summary="Grandchild 1A1",
        prompt="Grandchild of root task, child of Child 1A",
        parent_task_id=child_1a_id,
        status=TaskStatus.PENDING,
        priority=5,
        submitted_at=now,
        last_updated_at=now,
    )

    return [root, child_1a, child_1b, grandchild]
