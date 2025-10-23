"""Test fixtures for TUI parent-child hierarchy tests.

This module provides reusable pytest fixtures for creating hierarchical task
test data following the patterns defined in the test_fixtures_architecture
technical specification.
"""

from datetime import datetime, timezone
from uuid import UUID, uuid4

import pytest

from abathur.domain.models import Task, TaskSource, TaskStatus


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
