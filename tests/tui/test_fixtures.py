"""Verification tests for TUI test fixtures.

These tests verify that the fixtures defined in conftest.py are properly
importable and create the expected data structures.
"""

import pytest

from abathur.domain.models import Task, TaskStatus


def test_orphaned_tasks_fixture(orphaned_tasks: list[Task]) -> None:
    """Verify orphaned_tasks fixture creates 3 root tasks."""
    # Verify count
    assert len(orphaned_tasks) == 3, "Should create exactly 3 orphaned tasks"

    # Verify all are root tasks (no parent)
    for task in orphaned_tasks:
        assert task.parent_task_id is None, f"Task {task.summary} should have no parent"

    # Verify all have summaries
    summaries = [task.summary for task in orphaned_tasks]
    assert "Orphan 1" in summaries
    assert "Orphan 2" in summaries
    assert "Orphan 3" in summaries

    # Verify different statuses
    statuses = {task.status for task in orphaned_tasks}
    assert len(statuses) >= 2, "Should have variety of statuses"


def test_mixed_hierarchy_tasks_fixture(mixed_hierarchy_tasks: list[Task]) -> None:
    """Verify mixed_hierarchy_tasks fixture creates complex multi-tree scenario."""
    # Verify total count (2 roots with 3 children total + 1 orphan = 6 tasks)
    assert len(mixed_hierarchy_tasks) == 6, "Should create exactly 6 tasks"

    # Count root tasks (parent_task_id=None)
    root_tasks = [task for task in mixed_hierarchy_tasks if task.parent_task_id is None]
    assert len(root_tasks) == 3, "Should have 3 root tasks (Root 1, Root 2, Orphan)"

    # Count child tasks (have parent_task_id)
    child_tasks = [task for task in mixed_hierarchy_tasks if task.parent_task_id is not None]
    assert len(child_tasks) == 3, "Should have 3 child tasks"

    # Verify Root 1 has 2 children
    root1 = next((t for t in mixed_hierarchy_tasks if t.summary == "Root 1"), None)
    assert root1 is not None, "Root 1 should exist"
    root1_children = [t for t in mixed_hierarchy_tasks if t.parent_task_id == root1.id]
    assert len(root1_children) == 2, "Root 1 should have 2 children"

    # Verify Root 2 has 1 child
    root2 = next((t for t in mixed_hierarchy_tasks if t.summary == "Root 2"), None)
    assert root2 is not None, "Root 2 should exist"
    root2_children = [t for t in mixed_hierarchy_tasks if t.parent_task_id == root2.id]
    assert len(root2_children) == 1, "Root 2 should have 1 child"

    # Verify various statuses present
    statuses = {task.status for task in mixed_hierarchy_tasks}
    expected_statuses = {TaskStatus.PENDING, TaskStatus.COMPLETED, TaskStatus.RUNNING, TaskStatus.FAILED}
    assert expected_statuses.issubset(statuses), f"Should have various statuses: {expected_statuses}"

    # Verify specific task scenarios
    child_1a = next((t for t in mixed_hierarchy_tasks if t.summary == "Child 1A"), None)
    assert child_1a is not None, "Child 1A should exist"
    assert child_1a.status == TaskStatus.COMPLETED, "Child 1A should be completed"
    assert child_1a.completed_at is not None, "Completed task should have completed_at timestamp"

    child_1b = next((t for t in mixed_hierarchy_tasks if t.summary == "Child 1B"), None)
    assert child_1b is not None, "Child 1B should exist"
    assert child_1b.status == TaskStatus.RUNNING, "Child 1B should be running"
    assert child_1b.started_at is not None, "Running task should have started_at timestamp"

    child_2a = next((t for t in mixed_hierarchy_tasks if t.summary == "Child 2A"), None)
    assert child_2a is not None, "Child 2A should exist"
    assert child_2a.status == TaskStatus.FAILED, "Child 2A should be failed"
    assert child_2a.error_message is not None, "Failed task should have error_message"

    orphan = next((t for t in mixed_hierarchy_tasks if t.summary == "Orphan"), None)
    assert orphan is not None, "Orphan task should exist"
    assert orphan.parent_task_id is None, "Orphan should have no parent"
