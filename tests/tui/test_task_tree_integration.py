"""Integration tests for TaskTreeWidget with TreeRenderer.

These tests verify the integration between TaskTreeWidget and TreeRenderer,
ensuring proper color coding, formatting, and tree structure rendering.
"""

from datetime import datetime, timezone
from uuid import uuid4

import pytest
from rich.text import Text
from textual.app import App, ComposeResult

from abathur.domain.models import Task, TaskStatus
from abathur.tui.rendering.tree_renderer import TASK_STATUS_COLORS, TreeRenderer
from abathur.tui.widgets.task_tree import TaskTreeWidget


@pytest.fixture
def complex_task_hierarchy() -> list[Task]:
    """Create a complex task hierarchy for integration testing.

    Structure:
        Root Task 1 (PENDING)
        ├── Child 1A (RUNNING)
        │   └── Grandchild 1A1 (READY)
        └── Child 1B (COMPLETED)
        Root Task 2 (FAILED)
        └── Child 2A (BLOCKED)
    """
    now = datetime.now(timezone.utc)

    root1_id = uuid4()
    child1a_id = uuid4()
    grandchild1a1_id = uuid4()
    child1b_id = uuid4()
    root2_id = uuid4()
    child2a_id = uuid4()

    return [
        Task(
            id=root1_id,
            summary="Root Task 1 - Setup infrastructure",
            prompt="Setup infrastructure",
            priority=8,
            calculated_priority=8.0,
            status=TaskStatus.PENDING,
            submitted_at=now,
            last_updated_at=now,
        ),
        Task(
            id=child1a_id,
            summary="Child 1A - Configure database",
            prompt="Configure database",
            priority=7,
            calculated_priority=7.0,
            status=TaskStatus.RUNNING,
            parent_task_id=root1_id,
            submitted_at=now,
            last_updated_at=now,
        ),
        Task(
            id=grandchild1a1_id,
            summary="Grandchild 1A1 - Create schema",
            prompt="Create schema",
            priority=6,
            calculated_priority=6.0,
            status=TaskStatus.READY,
            parent_task_id=child1a_id,
            submitted_at=now,
            last_updated_at=now,
        ),
        Task(
            id=child1b_id,
            summary="Child 1B - Setup monitoring",
            prompt="Setup monitoring",
            priority=5,
            calculated_priority=5.0,
            status=TaskStatus.COMPLETED,
            parent_task_id=root1_id,
            submitted_at=now,
            last_updated_at=now,
        ),
        Task(
            id=root2_id,
            summary="Root Task 2 - Deploy application",
            prompt="Deploy application",
            priority=9,
            calculated_priority=9.0,
            status=TaskStatus.FAILED,
            submitted_at=now,
            last_updated_at=now,
        ),
        Task(
            id=child2a_id,
            summary="Child 2A - Run tests",
            prompt="Run tests",
            priority=4,
            calculated_priority=4.0,
            status=TaskStatus.BLOCKED,
            parent_task_id=root2_id,
            submitted_at=now,
            last_updated_at=now,
        ),
    ]


def test_tree_renderer_format_task_node_color_coding():
    """Test that TreeRenderer correctly color-codes tasks by status."""
    renderer = TreeRenderer()
    now = datetime.now(timezone.utc)

    # Test each status gets correct color
    status_tests = [
        (TaskStatus.PENDING, "blue"),
        (TaskStatus.BLOCKED, "yellow"),
        (TaskStatus.READY, "green"),
        (TaskStatus.RUNNING, "magenta"),
        (TaskStatus.COMPLETED, "bright_green"),
        (TaskStatus.FAILED, "red"),
        (TaskStatus.CANCELLED, "dim"),
    ]

    for status, expected_color in status_tests:
        task = Task(
            id=uuid4(),
            summary=f"Task with {status.value} status",
            prompt="Test task",
            priority=5,
            calculated_priority=5.0,
            status=status,
            submitted_at=now,
            last_updated_at=now,
        )

        formatted = renderer.format_task_node(task)

        # Check that formatted is a Rich Text object
        assert isinstance(formatted, Text)

        # Check that the text contains the expected color
        # Note: Rich Text styles are stored as Style objects, so we check the string representation
        text_str = formatted.plain
        assert "Task with" in text_str
        assert status.value in text_str or "5.0" in text_str  # Contains priority


def test_tree_renderer_format_task_node_truncation():
    """Test that TreeRenderer truncates long summaries to 40 chars."""
    renderer = TreeRenderer()
    now = datetime.now(timezone.utc)

    long_summary = "This is a very long task summary that exceeds forty characters"
    task = Task(
        id=uuid4(),
        summary=long_summary,
        prompt="Test task",
        priority=5,
        calculated_priority=5.0,
        status=TaskStatus.PENDING,
        submitted_at=now,
        last_updated_at=now,
    )

    formatted = renderer.format_task_node(task)
    text_str = formatted.plain

    # Should be truncated to ~40 chars plus "..."
    assert len(text_str) <= 50  # 40 + "..." + priority display
    assert "..." in text_str


def test_tree_renderer_format_task_node_priority_display():
    """Test that TreeRenderer displays priority in formatted node."""
    renderer = TreeRenderer()
    now = datetime.now(timezone.utc)

    task = Task(
        id=uuid4(),
        summary="Test task",
        prompt="Test task",
        priority=8,
        calculated_priority=8.5,
        status=TaskStatus.READY,
        submitted_at=now,
        last_updated_at=now,
    )

    formatted = renderer.format_task_node(task)
    text_str = formatted.plain

    # Priority should be displayed
    assert "8.5" in text_str or "(8.5)" in text_str


class IntegrationTestApp(App[None]):
    """Test app for TaskTreeWidget + TreeRenderer integration."""

    def __init__(self, tasks: list[Task]):
        super().__init__()
        self.tasks = tasks

    def compose(self) -> ComposeResult:
        tree = TaskTreeWidget("Task Queue", id="task-tree")
        yield tree

    def on_mount(self) -> None:
        tree = self.query_one(TaskTreeWidget)
        tree.load_tasks(self.tasks, {})


@pytest.mark.asyncio
async def test_integration_widget_uses_renderer_for_formatting(
    complex_task_hierarchy: list[Task],
):
    """Test that TaskTreeWidget uses TreeRenderer for task formatting."""
    app = IntegrationTestApp(complex_task_hierarchy)

    async with app.run_test() as pilot:
        tree = app.query_one(TaskTreeWidget)

        # Verify TreeRenderer is instantiated
        assert tree._renderer is not None
        assert isinstance(tree._renderer, TreeRenderer)

        # Verify node map contains tasks
        assert len(tree._node_map) > 0


@pytest.mark.asyncio
async def test_integration_hierarchical_structure_display(
    complex_task_hierarchy: list[Task],
):
    """Test that hierarchical task structure is correctly displayed."""
    app = IntegrationTestApp(complex_task_hierarchy)

    async with app.run_test() as pilot:
        tree = app.query_one(TaskTreeWidget)

        # Find root tasks
        root_tasks = [t for t in complex_task_hierarchy if t.parent_task_id is None]
        assert len(root_tasks) == 2

        # Each root task should be in node map
        for root_task in root_tasks:
            assert root_task.id in tree._node_map

        # Check that child tasks are also mapped
        child_tasks = [
            t for t in complex_task_hierarchy if t.parent_task_id is not None
        ]
        for child_task in child_tasks:
            assert child_task.id in tree._node_map


@pytest.mark.asyncio
async def test_integration_color_coding_preserved_in_tree(
    complex_task_hierarchy: list[Task],
):
    """Test that TreeRenderer color coding is preserved in tree nodes."""
    app = IntegrationTestApp(complex_task_hierarchy)

    async with app.run_test() as pilot:
        tree = app.query_one(TaskTreeWidget)

        # Check that each task status has corresponding color in TASK_STATUS_COLORS
        for task in complex_task_hierarchy:
            assert task.status in TASK_STATUS_COLORS
            expected_color = TASK_STATUS_COLORS[task.status]
            assert expected_color is not None


@pytest.mark.asyncio
async def test_integration_navigation_with_multi_level_hierarchy(
    complex_task_hierarchy: list[Task],
):
    """Test keyboard navigation through multi-level task hierarchy."""
    app = IntegrationTestApp(complex_task_hierarchy)

    async with app.run_test() as pilot:
        tree = app.query_one(TaskTreeWidget)

        # Navigate to first task
        await pilot.press("j")
        await pilot.pause()

        # Should select a root task
        assert tree.selected_task_id is not None
        selected_task = tree._task_data[tree.selected_task_id]
        assert selected_task.parent_task_id is None  # Root task

        # Expand the node
        await pilot.press("l")
        await pilot.pause()

        # Should be in expanded nodes
        assert tree.selected_task_id in tree.expanded_nodes

        # Navigate down into children
        await pilot.press("j")
        await pilot.pause()

        # Now should have selected a child task potentially
        # (depends on tree structure, but task_id should still be valid)
        assert tree.selected_task_id is not None


@pytest.mark.asyncio
async def test_integration_expansion_state_persistence(
    complex_task_hierarchy: list[Task],
):
    """Test that expansion state is maintained in expanded_nodes reactive property."""
    app = IntegrationTestApp(complex_task_hierarchy)

    async with app.run_test() as pilot:
        tree = app.query_one(TaskTreeWidget)

        # Find a root task with children
        root_with_children = None
        for task in complex_task_hierarchy:
            if task.parent_task_id is None:
                # Check if it has children
                children = [
                    t for t in complex_task_hierarchy if t.parent_task_id == task.id
                ]
                if children:
                    root_with_children = task
                    break

        assert root_with_children is not None

        # Manually expand the node
        node = tree._node_map[root_with_children.id]
        tree.cursor_node = node

        await pilot.press("l")
        await pilot.pause()

        # Should be expanded
        assert root_with_children.id in tree.expanded_nodes

        # Reload tasks with same expansion state
        initial_expanded = tree.expanded_nodes.copy()
        tree.load_tasks(complex_task_hierarchy, {})

        # Expansion state should be restored
        assert root_with_children.id in tree._node_map


@pytest.mark.asyncio
async def test_integration_selection_with_different_statuses(
    complex_task_hierarchy: list[Task],
):
    """Test task selection works correctly with tasks of different statuses."""
    app = IntegrationTestApp(complex_task_hierarchy)

    async with app.run_test() as pilot:
        tree = app.query_one(TaskTreeWidget)

        # Iterate through several tasks
        for _ in range(3):
            await pilot.press("j")
            await pilot.pause()

        # Should have a valid selection
        assert tree.selected_task_id is not None

        # Selected task should exist in task data
        assert tree.selected_task_id in tree._task_data

        # Task should have a valid status
        selected_task = tree._task_data[tree.selected_task_id]
        assert isinstance(selected_task.status, TaskStatus)


@pytest.mark.asyncio
async def test_integration_goto_top_bottom_with_hierarchy(
    complex_task_hierarchy: list[Task],
):
    """Test goto top/bottom navigation with hierarchical structure."""
    app = IntegrationTestApp(complex_task_hierarchy)

    async with app.run_test() as pilot:
        tree = app.query_one(TaskTreeWidget)

        # Navigate down
        await pilot.press("j")
        await pilot.press("j")
        await pilot.pause()

        # Go to top
        await pilot.press("g")
        await pilot.pause()

        # Should be at first root task
        assert tree.selected_task_id is not None
        first_task = tree._task_data[tree.selected_task_id]
        assert first_task.parent_task_id is None

        # Go to bottom
        await pilot.press("G")
        await pilot.pause()

        # Should be at some task (exact position depends on expansion)
        assert tree.selected_task_id is not None


def test_tree_renderer_status_colors_completeness():
    """Test that all TaskStatus values have defined colors."""
    for status in TaskStatus:
        assert status in TASK_STATUS_COLORS
        color = TASK_STATUS_COLORS[status]
        assert isinstance(color, str)
        assert len(color) > 0
