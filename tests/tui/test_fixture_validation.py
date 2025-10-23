"""Validation tests for TUI fixtures.

This test file validates that the parametrized large_child_count_tasks
fixture works correctly across all parameter values.
"""

import pytest
from abathur.domain.models import Task


def test_large_child_count_tasks_structure(large_child_count_tasks: list[Task], request):
    """Verify fixture creates correct parent-child structure."""
    tasks = large_child_count_tasks
    expected_child_count = request.node.callspec.params["large_child_count_tasks"]

    # Verify we have parent + children
    assert len(tasks) == expected_child_count + 1

    # Verify parent is first
    parent = tasks[0]
    assert parent.parent_task_id is None
    assert f"Parent with {expected_child_count} children" in parent.summary

    # Verify children reference parent
    children = tasks[1:]
    assert len(children) == expected_child_count

    for i, child in enumerate(children, start=1):
        assert child.parent_task_id == parent.id
        assert f"Child {i}/{expected_child_count}" == child.summary


def test_large_child_count_tasks_parent_properties(large_child_count_tasks: list[Task]):
    """Verify parent task has correct properties."""
    parent = large_child_count_tasks[0]

    # Verify required fields
    assert parent.id is not None
    assert parent.summary is not None
    assert parent.prompt is not None
    assert parent.parent_task_id is None

    # Verify status and source
    assert parent.status is not None
    assert parent.source is not None


def test_large_child_count_tasks_child_properties(large_child_count_tasks: list[Task]):
    """Verify child tasks have correct properties."""
    children = large_child_count_tasks[1:]

    for child in children:
        # Verify required fields
        assert child.id is not None
        assert child.summary is not None
        assert child.prompt is not None

        # Verify child links to parent
        parent_id = large_child_count_tasks[0].id
        assert child.parent_task_id == parent_id

        # Verify status and source
        assert child.status is not None
        assert child.source is not None


def test_large_child_count_tasks_unique_ids(large_child_count_tasks: list[Task]):
    """Verify all tasks have unique IDs."""
    task_ids = [task.id for task in large_child_count_tasks]
    assert len(task_ids) == len(set(task_ids))


def test_large_child_count_tasks_edge_case_zero_children(large_child_count_tasks: list[Task], request):
    """Verify fixture handles zero children correctly."""
    expected_child_count = request.node.callspec.params["large_child_count_tasks"]

    if expected_child_count == 0:
        # Should have only parent
        assert len(large_child_count_tasks) == 1
        parent = large_child_count_tasks[0]
        assert parent.parent_task_id is None
