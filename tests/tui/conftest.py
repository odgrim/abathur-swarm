"""Test fixtures for TUI parent-child relationship tests.

This module provides reusable pytest fixtures for creating hierarchical task test data.
Following the test_fixtures_architecture design from technical specifications.
"""

from datetime import datetime, timezone
from uuid import UUID, uuid4

import pytest

from abathur.domain.models import Task, TaskStatus


@pytest.fixture
def orphaned_tasks() -> list[Task]:
    """Tasks with parent_task_id=None (root tasks).

    Creates 3 independent root tasks with no parent relationships.
    Useful for testing edge cases where all tasks are at the root level.

    Structure:
        Task 1 (parent_task_id=None)
        Task 2 (parent_task_id=None)
        Task 3 (parent_task_id=None)

    Use Cases:
        - FR004: Orphaned task display
        - Edge case: All tasks are roots
        - Testing tree widget with no hierarchy

    Returns:
        list[Task]: List of 3 root tasks with no parent relationships
    """
    now = datetime.now(timezone.utc)

    return [
        Task(
            id=uuid4(),
            summary="Orphan 1",
            prompt="First orphaned task with no parent",
            parent_task_id=None,
            status=TaskStatus.PENDING,
            priority=5,
            submitted_at=now,
            last_updated_at=now
        ),
        Task(
            id=uuid4(),
            summary="Orphan 2",
            prompt="Second orphaned task with no parent",
            parent_task_id=None,
            status=TaskStatus.READY,
            priority=3,
            submitted_at=now,
            last_updated_at=now
        ),
        Task(
            id=uuid4(),
            summary="Orphan 3",
            prompt="Third orphaned task with no parent",
            parent_task_id=None,
            status=TaskStatus.COMPLETED,
            priority=7,
            submitted_at=now,
            last_updated_at=now
        )
    ]


@pytest.fixture
def mixed_hierarchy_tasks() -> list[Task]:
    """Complex scenario: multiple trees, orphans, various statuses.

    Creates a complex multi-tree structure with different task statuses
    to simulate real-world complexity and test edge cases.

    Structure:
        Root 1 (PENDING)
        ├─ Child 1A (COMPLETED)
        └─ Child 1B (RUNNING)

        Root 2 (PENDING)
        └─ Child 2A (FAILED)

        Orphan Task (PENDING)

    Use Cases:
        - Integration tests with complex hierarchies
        - Real-world complexity simulation
        - Testing multiple independent task trees
        - Testing various task status combinations

    Returns:
        list[Task]: List of tasks forming 2 complete trees plus 1 orphan task
    """
    now = datetime.now(timezone.utc)

    # Create root task IDs
    root1_id = uuid4()
    root2_id = uuid4()

    return [
        # First tree: Root 1 with 2 children
        Task(
            id=root1_id,
            summary="Root 1",
            prompt="First root task with children",
            parent_task_id=None,
            status=TaskStatus.PENDING,
            priority=5,
            submitted_at=now,
            last_updated_at=now
        ),
        Task(
            id=uuid4(),
            summary="Child 1A",
            prompt="First child of Root 1 - completed",
            parent_task_id=root1_id,
            status=TaskStatus.COMPLETED,
            priority=5,
            submitted_at=now,
            last_updated_at=now,
            completed_at=now
        ),
        Task(
            id=uuid4(),
            summary="Child 1B",
            prompt="Second child of Root 1 - running",
            parent_task_id=root1_id,
            status=TaskStatus.RUNNING,
            priority=5,
            submitted_at=now,
            last_updated_at=now,
            started_at=now
        ),

        # Second tree: Root 2 with 1 child
        Task(
            id=root2_id,
            summary="Root 2",
            prompt="Second root task with one child",
            parent_task_id=None,
            status=TaskStatus.PENDING,
            priority=3,
            submitted_at=now,
            last_updated_at=now
        ),
        Task(
            id=uuid4(),
            summary="Child 2A",
            prompt="Child of Root 2 - failed",
            parent_task_id=root2_id,
            status=TaskStatus.FAILED,
            priority=3,
            submitted_at=now,
            last_updated_at=now,
            started_at=now,
            error_message="Task failed during execution"
        ),

        # Orphan task (no parent)
        Task(
            id=uuid4(),
            summary="Orphan",
            prompt="Standalone orphan task with no parent",
            parent_task_id=None,
            status=TaskStatus.PENDING,
            priority=7,
            submitted_at=now,
            last_updated_at=now
        )
    ]
