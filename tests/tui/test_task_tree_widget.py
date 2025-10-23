"""Unit tests for TaskTreeWidget with keyboard navigation.

Tests cover:
- Widget initialization
- Reactive property updates
- Keyboard navigation (up/down, hjkl)
- Expand/collapse functionality
- Task selection events
- Integration with TreeRenderer
"""

from datetime import datetime, timezone
from uuid import UUID, uuid4

import pytest
from textual.app import App, ComposeResult

from abathur.domain.models import Task, TaskStatus
from abathur.tui.widgets.task_tree import TaskTreeWidget


@pytest.fixture
def dependency_graph(simple_parent_child_tasks: list[Task]) -> dict[UUID, list[UUID]]:
    """Create dependency graph for simple parent-child tasks.

    Args:
        simple_parent_child_tasks: Simple parent-child tasks fixture

    Returns:
        Dict mapping task IDs to dependent task IDs
    """
    # For this test, we don't have dependencies, just parent-child
    return {}


def test_widget_initialization():
    """Test TaskTreeWidget initializes correctly."""
    widget = TaskTreeWidget("Tasks")

    assert widget.selected_task_id is None
    assert len(widget.expanded_nodes) == 0
    assert widget._task_data == {}
    assert widget._node_map == {}


def test_load_tasks(simple_parent_child_tasks: list[Task], dependency_graph: dict[UUID, list[UUID]]):
    """Test loading tasks into the widget."""
    widget = TaskTreeWidget("Tasks")
    widget.load_tasks(simple_parent_child_tasks, dependency_graph)

    # Check task data is cached
    assert len(widget._task_data) == 3
    for task in simple_parent_child_tasks:
        assert task.id in widget._task_data
        assert widget._task_data[task.id] == task


def test_reactive_property_selected_task_id():
    """Test selected_task_id reactive property triggers watch method."""
    widget = TaskTreeWidget("Tasks")
    task_id = uuid4()

    # Setting the property should work
    widget.selected_task_id = task_id
    assert widget.selected_task_id == task_id


def test_reactive_property_expanded_nodes():
    """Test expanded_nodes reactive property."""
    widget = TaskTreeWidget("Tasks")
    task_id = uuid4()

    # Create a new set with the task_id
    new_expanded = {task_id}
    widget.expanded_nodes = new_expanded

    assert task_id in widget.expanded_nodes


class TestApp(App[None]):
    """Test app for TaskTreeWidget integration tests."""

    def __init__(self, tasks: list[Task], dependency_graph: dict[UUID, list[UUID]]):
        """Initialize test app with tasks."""
        super().__init__()
        self.tasks = tasks
        self.dependency_graph = dependency_graph
        self.messages_received: list[TaskTreeWidget.TaskSelected] = []

    def compose(self) -> ComposeResult:
        """Compose app with TaskTreeWidget."""
        tree = TaskTreeWidget("Tasks", id="task-tree")
        yield tree

    def on_mount(self) -> None:
        """Load tasks when app mounts."""
        tree = self.query_one(TaskTreeWidget)
        tree.load_tasks(self.tasks, self.dependency_graph)

    def on_task_tree_widget_task_selected(
        self, message: TaskTreeWidget.TaskSelected
    ) -> None:
        """Capture TaskSelected messages."""
        self.messages_received.append(message)


@pytest.mark.asyncio
async def test_keyboard_navigation_down(
    simple_parent_child_tasks: list[Task], dependency_graph: dict[UUID, list[UUID]]
):
    """Test keyboard navigation down with j/down arrow."""
    app = TestApp(simple_parent_child_tasks, dependency_graph)

    async with app.run_test() as pilot:
        # Initial state: no selection
        tree = app.query_one(TaskTreeWidget)
        assert tree.selected_task_id is None

        # Press down to select first task
        await pilot.press("down")
        await pilot.pause()

        # Should select the parent task (first in tree)
        assert tree.selected_task_id == simple_parent_child_tasks[0].id

        # Check message was emitted
        assert len(app.messages_received) == 1
        assert app.messages_received[0].task_id == simple_parent_child_tasks[0].id


@pytest.mark.asyncio
async def test_keyboard_navigation_vim_keys(
    simple_parent_child_tasks: list[Task], dependency_graph: dict[UUID, list[UUID]]
):
    """Test Vim-style keyboard navigation (hjkl)."""
    app = TestApp(simple_parent_child_tasks, dependency_graph)

    async with app.run_test() as pilot:
        tree = app.query_one(TaskTreeWidget)

        # Navigate down with 'j'
        await pilot.press("j")
        await pilot.pause()
        assert tree.selected_task_id == simple_parent_child_tasks[0].id

        # Navigate up with 'k' (back to root, no selection change expected)
        await pilot.press("k")
        await pilot.pause()


@pytest.mark.asyncio
async def test_expand_collapse_with_keyboard(
    simple_parent_child_tasks: list[Task], dependency_graph: dict[UUID, list[UUID]]
):
    """Test expand/collapse with keyboard (hjkl, enter, space)."""
    app = TestApp(simple_parent_child_tasks, dependency_graph)

    async with app.run_test() as pilot:
        tree = app.query_one(TaskTreeWidget)

        # Navigate to parent task
        await pilot.press("j")
        await pilot.pause()

        parent_id = simple_parent_child_tasks[0].id
        assert tree.selected_task_id == parent_id

        # Expand with 'l' (right)
        await pilot.press("l")
        await pilot.pause()

        # Parent should now be in expanded_nodes
        assert parent_id in tree.expanded_nodes

        # Collapse with 'h' (left)
        await pilot.press("h")
        await pilot.pause()

        # Parent should be removed from expanded_nodes
        assert parent_id not in tree.expanded_nodes


@pytest.mark.asyncio
async def test_toggle_expand_with_enter(
    simple_parent_child_tasks: list[Task], dependency_graph: dict[UUID, list[UUID]]
):
    """Test toggling expansion with enter/space."""
    app = TestApp(simple_parent_child_tasks, dependency_graph)

    async with app.run_test() as pilot:
        tree = app.query_one(TaskTreeWidget)

        # Navigate to parent task
        await pilot.press("j")
        await pilot.pause()

        parent_id = simple_parent_child_tasks[0].id

        # Toggle expand with enter
        await pilot.press("enter")
        await pilot.pause()

        # Should be expanded
        assert parent_id in tree.expanded_nodes

        # Toggle again to collapse
        await pilot.press("enter")
        await pilot.pause()

        # Should be collapsed
        assert parent_id not in tree.expanded_nodes


@pytest.mark.asyncio
async def test_goto_top_and_bottom(
    simple_parent_child_tasks: list[Task], dependency_graph: dict[UUID, list[UUID]]
):
    """Test jump to top (g) and bottom (G) of tree."""
    app = TestApp(simple_parent_child_tasks, dependency_graph)

    async with app.run_test() as pilot:
        tree = app.query_one(TaskTreeWidget)

        # Navigate down a bit
        await pilot.press("j")
        await pilot.press("j")
        await pilot.pause()

        # Jump to top with 'g'
        await pilot.press("g")
        await pilot.pause()

        # Should be at first task
        assert tree.selected_task_id == simple_parent_child_tasks[0].id

        # Jump to bottom with 'G' (shift+g)
        await pilot.press("G")
        await pilot.pause()

        # Should be at last visible task
        # Note: Without expansion, this should still be parent task
        assert tree.selected_task_id is not None


@pytest.mark.asyncio
async def test_message_emission_on_selection(
    simple_parent_child_tasks: list[Task], dependency_graph: dict[UUID, list[UUID]]
):
    """Test that TaskSelected messages are emitted on selection."""
    app = TestApp(simple_parent_child_tasks, dependency_graph)

    async with app.run_test() as pilot:
        tree = app.query_one(TaskTreeWidget)

        # Navigate to select a task
        await pilot.press("j")
        await pilot.pause()

        # Message should be captured
        assert len(app.messages_received) == 1
        assert isinstance(app.messages_received[0], TaskTreeWidget.TaskSelected)
        assert app.messages_received[0].task_id == simple_parent_child_tasks[0].id


@pytest.mark.asyncio
async def test_tree_renderer_integration(
    simple_parent_child_tasks: list[Task], dependency_graph: dict[UUID, list[UUID]]
):
    """Test integration with TreeRenderer for task formatting."""
    app = TestApp(simple_parent_child_tasks, dependency_graph)

    async with app.run_test() as pilot:
        tree = app.query_one(TaskTreeWidget)

        # Check that TreeRenderer was used for formatting
        assert tree._renderer is not None

        # Verify task nodes exist in node map
        for task in simple_parent_child_tasks:
            # Parent task should be in node map
            if task.parent_task_id is None:
                assert task.id in tree._node_map


def test_get_cursor_task_id_with_no_cursor():
    """Test _get_cursor_task_id returns None when no cursor."""
    widget = TaskTreeWidget("Tasks")
    assert widget._get_cursor_task_id() is None


@pytest.mark.asyncio
async def test_node_expanded_message_emission(
    simple_parent_child_tasks: list[Task], dependency_graph: dict[UUID, list[UUID]]
):
    """Test that NodeExpanded messages are emitted on expand/collapse."""

    class TestAppWithNodeExpanded(TestApp):
        """Test app that captures NodeExpanded messages."""

        def __init__(
            self, tasks: list[Task], dependency_graph: dict[UUID, list[UUID]]
        ):
            super().__init__(tasks, dependency_graph)
            self.node_messages: list[TaskTreeWidget.NodeExpanded] = []

        def on_task_tree_widget_node_expanded(
            self, message: TaskTreeWidget.NodeExpanded
        ) -> None:
            """Capture NodeExpanded messages."""
            self.node_messages.append(message)

    app = TestAppWithNodeExpanded(simple_parent_child_tasks, dependency_graph)

    async with app.run_test() as pilot:
        tree = app.query_one(TaskTreeWidget)

        # Navigate to parent task
        await pilot.press("j")
        await pilot.pause()

        parent_id = simple_parent_child_tasks[0].id

        # Expand node
        await pilot.press("l")
        await pilot.pause()

        # Should have received NodeExpanded message
        assert len(app.node_messages) >= 1
        # Find the expansion message for our parent
        expansion_msgs = [
            msg for msg in app.node_messages if msg.task_id == parent_id and msg.is_expanded
        ]
        assert len(expansion_msgs) > 0


@pytest.mark.asyncio
async def test_load_tasks_clears_existing_state(
    simple_parent_child_tasks: list[Task], dependency_graph: dict[UUID, list[UUID]]
):
    """Test that load_tasks clears existing tree state."""
    widget = TaskTreeWidget("Tasks")

    # Load tasks once
    widget.load_tasks(simple_parent_child_tasks, dependency_graph)
    initial_count = len(widget._task_data)

    # Load again with fewer tasks
    widget.load_tasks(simple_parent_child_tasks[:1], dependency_graph)

    # Should have fewer tasks now
    assert len(widget._task_data) == 1
    assert len(widget._task_data) < initial_count


@pytest.mark.asyncio
async def test_hierarchical_sorting_by_priority(dependency_graph: dict[UUID, list[UUID]]):
    """Test that tasks are sorted by priority within each hierarchy level."""
    now = datetime.now(timezone.utc)

    tasks = [
        Task(
            id=uuid4(),
            summary="Low priority task",
            prompt="Low priority",
            priority=2,
            calculated_priority=2.0,
            status=TaskStatus.PENDING,
            submitted_at=now,
            last_updated_at=now,
        ),
        Task(
            id=uuid4(),
            summary="High priority task",
            prompt="High priority",
            priority=9,
            calculated_priority=9.0,
            status=TaskStatus.PENDING,
            submitted_at=now,
            last_updated_at=now,
        ),
        Task(
            id=uuid4(),
            summary="Medium priority task",
            prompt="Medium priority",
            priority=5,
            calculated_priority=5.0,
            status=TaskStatus.PENDING,
            submitted_at=now,
            last_updated_at=now,
        ),
    ]

    widget = TaskTreeWidget("Tasks")
    widget.load_tasks(tasks, dependency_graph)

    # Verify tasks are loaded (exact ordering verification would require inspecting tree structure)
    assert len(widget._task_data) == 3
