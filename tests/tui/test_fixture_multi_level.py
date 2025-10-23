"""Verification tests for multi_level_hierarchy_tasks fixture.

This module tests that the fixture creates the expected structure and data.
"""

from abathur.domain.models import Task, TaskStatus


def test_multi_level_hierarchy_fixture_creates_four_tasks(
    multi_level_hierarchy_tasks: list[Task],
):
    """Test fixture creates exactly 4 tasks."""
    assert len(multi_level_hierarchy_tasks) == 4


def test_multi_level_hierarchy_fixture_has_one_root(
    multi_level_hierarchy_tasks: list[Task],
):
    """Test fixture has exactly 1 root task (parent_task_id=None)."""
    root_tasks = [t for t in multi_level_hierarchy_tasks if t.parent_task_id is None]
    assert len(root_tasks) == 1
    assert root_tasks[0].summary == "Root Task"


def test_multi_level_hierarchy_fixture_has_two_children(
    multi_level_hierarchy_tasks: list[Task],
):
    """Test fixture has exactly 2 direct children of root."""
    root_tasks = [t for t in multi_level_hierarchy_tasks if t.parent_task_id is None]
    root_id = root_tasks[0].id

    children = [t for t in multi_level_hierarchy_tasks if t.parent_task_id == root_id]
    assert len(children) == 2
    assert {c.summary for c in children} == {"Child 1A", "Child 1B"}


def test_multi_level_hierarchy_fixture_has_one_grandchild(
    multi_level_hierarchy_tasks: list[Task],
):
    """Test fixture has exactly 1 grandchild (depth 2)."""
    # Find Child 1A
    child_1a = [
        t for t in multi_level_hierarchy_tasks if t.summary == "Child 1A"
    ][0]

    # Find grandchildren of Child 1A
    grandchildren = [
        t for t in multi_level_hierarchy_tasks if t.parent_task_id == child_1a.id
    ]
    assert len(grandchildren) == 1
    assert grandchildren[0].summary == "Grandchild 1A1"


def test_multi_level_hierarchy_fixture_has_correct_parent_relationships(
    multi_level_hierarchy_tasks: list[Task],
):
    """Test parent_task_id relationships are correctly established."""
    # Create lookup by summary
    tasks_by_summary = {t.summary: t for t in multi_level_hierarchy_tasks}

    root = tasks_by_summary["Root Task"]
    child_1a = tasks_by_summary["Child 1A"]
    child_1b = tasks_by_summary["Child 1B"]
    grandchild = tasks_by_summary["Grandchild 1A1"]

    # Verify hierarchy
    assert root.parent_task_id is None
    assert child_1a.parent_task_id == root.id
    assert child_1b.parent_task_id == root.id
    assert grandchild.parent_task_id == child_1a.id


def test_multi_level_hierarchy_fixture_has_varied_statuses(
    multi_level_hierarchy_tasks: list[Task],
):
    """Test fixture includes tasks with different statuses."""
    statuses = {t.status for t in multi_level_hierarchy_tasks}
    assert TaskStatus.READY in statuses
    assert TaskStatus.COMPLETED in statuses
    assert TaskStatus.PENDING in statuses


def test_multi_level_hierarchy_fixture_has_unique_ids(
    multi_level_hierarchy_tasks: list[Task],
):
    """Test all tasks have unique IDs."""
    task_ids = [t.id for t in multi_level_hierarchy_tasks]
    assert len(task_ids) == len(set(task_ids))


def test_multi_level_hierarchy_fixture_structure(
    multi_level_hierarchy_tasks: list[Task],
):
    """Test the complete 3-level hierarchy structure.

    Structure should be:
        Root (depth 0)
        ├─ Child 1A (depth 1)
        │  └─ Grandchild 1A1 (depth 2)
        └─ Child 1B (depth 1)
    """
    tasks_by_summary = {t.summary: t for t in multi_level_hierarchy_tasks}

    root = tasks_by_summary["Root Task"]
    child_1a = tasks_by_summary["Child 1A"]
    child_1b = tasks_by_summary["Child 1B"]
    grandchild = tasks_by_summary["Grandchild 1A1"]

    # Level 0: Root
    assert root.parent_task_id is None

    # Level 1: Children of root
    assert child_1a.parent_task_id == root.id
    assert child_1b.parent_task_id == root.id

    # Level 2: Grandchild of root via Child 1A
    assert grandchild.parent_task_id == child_1a.id

    # Verify no other relationships
    assert len([t for t in multi_level_hierarchy_tasks if t.parent_task_id == child_1b.id]) == 0
    assert len([t for t in multi_level_hierarchy_tasks if t.parent_task_id == grandchild.id]) == 0
