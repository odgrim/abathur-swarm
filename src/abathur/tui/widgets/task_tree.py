"""Interactive tree widget for displaying task DAG with keyboard navigation.

This module provides the TaskTreeWidget, a Textual Tree widget extension
that displays hierarchical task structures with:
- Keyboard navigation (arrows, hjkl, vim-style)
- Expand/collapse state management
- Task selection with custom event emission
- Integration with TreeRenderer for visual formatting
"""

from dataclasses import dataclass
from typing import Any
from uuid import UUID

from textual.binding import Binding
from textual.message import Message
from textual.reactive import reactive
from textual.widgets import Tree
from textual.widgets.tree import TreeNode

from abathur.domain.models import Task
from abathur.tui.rendering.tree_renderer import TreeRenderer


class TaskTreeWidget(Tree[dict[str, Any]]):
    """Interactive tree widget for displaying task DAG with keyboard navigation.

    This widget extends Textual's Tree widget to provide specialized functionality
    for displaying and interacting with task hierarchies. It manages expansion state,
    emits custom selection events, and integrates with TreeRenderer for formatting.

    Reactive Properties:
        selected_task_id: UUID of currently selected task (or None)
        expanded_nodes: Set of task UUIDs that should be expanded

    Keyboard Bindings:
        up/k: Navigate to previous node
        down/j: Navigate to next node
        left/h: Collapse current node
        right/l: Expand current node
        enter/space: Toggle expansion of current node
        g: Jump to top of tree
        G: Jump to bottom of tree

    Custom Messages:
        TaskSelected: Emitted when a task is selected (via navigation or click)
        NodeExpanded: Emitted when a node is expanded or collapsed

    Example:
        >>> tree = TaskTreeWidget("Tasks")
        >>> tree.load_tasks(tasks, dependency_graph)
        >>> # In parent screen/app:
        >>> @on(TaskTreeWidget.TaskSelected)
        >>> def handle_task_selected(self, message: TaskTreeWidget.TaskSelected):
        >>>     print(f"Selected task: {message.task_id}")
    """

    # Reactive properties
    selected_task_id: reactive[UUID | None] = reactive(None)
    expanded_nodes: reactive[set[UUID]] = reactive(set, layout=False)

    # Keyboard bindings (show=False keeps them hidden from footer)
    BINDINGS = [
        Binding("up,k", "navigate_up", "Navigate up", show=False),
        Binding("down,j", "navigate_down", "Navigate down", show=False),
        Binding("left,h", "collapse_node", "Collapse", show=False),
        Binding("right,l", "expand_node", "Expand", show=False),
        Binding("enter,space", "toggle_expand", "Toggle", show=True),
        Binding("g", "goto_top", "Go to top", show=False),
        Binding("G", "goto_bottom", "Go to bottom", show=False),
    ]

    @dataclass
    class TaskSelected(Message):
        """Message emitted when a task is selected.

        This message is posted whenever the user navigates to or clicks on
        a task node in the tree. Parent widgets can listen for this message
        to update detail panels or trigger other actions.

        Attributes:
            task_id: UUID of the selected task
        """

        task_id: UUID

        @property
        def control(self) -> "TaskTreeWidget":
            """The TaskTreeWidget that sent this message."""
            return self.sender  # type: ignore

    @dataclass
    class NodeExpanded(Message):
        """Message emitted when a node is expanded or collapsed.

        Attributes:
            task_id: UUID of the task whose node was toggled
            is_expanded: True if node was expanded, False if collapsed
        """

        task_id: UUID
        is_expanded: bool

        @property
        def control(self) -> "TaskTreeWidget":
            """The TaskTreeWidget that sent this message."""
            return self.sender  # type: ignore

    def __init__(
        self,
        label: str,
        data: dict[str, Any] | None = None,
        *,
        name: str | None = None,
        id: str | None = None,  # noqa: A002
        classes: str | None = None,
        disabled: bool = False,
    ) -> None:
        """Initialize TaskTreeWidget.

        Args:
            label: Root label for the tree
            data: Optional data dict attached to root node
            name: Widget name
            id: Widget ID
            classes: CSS classes
            disabled: Whether widget starts disabled
        """
        super().__init__(
            label,
            data=data,
            name=name,
            id=id,
            classes=classes,
            disabled=disabled,
        )
        self._task_data: dict[UUID, Task] = {}  # Cache for quick task lookup
        self._node_map: dict[UUID, TreeNode[dict[str, Any]]] = (
            {}
        )  # Map task IDs to tree nodes
        self._renderer = TreeRenderer()

    def watch_selected_task_id(
        self, old_id: UUID | None, new_id: UUID | None
    ) -> None:
        """Watch method called when selected_task_id changes.

        This method is automatically invoked by Textual's reactive system
        whenever the selected_task_id property is modified.

        Args:
            old_id: Previously selected task ID (or None)
            new_id: Newly selected task ID (or None)
        """
        if new_id is not None:
            # Post message to notify parent widgets of selection change
            self.post_message(self.TaskSelected(new_id))

    def watch_expanded_nodes(
        self, old_nodes: set[UUID], new_nodes: set[UUID]
    ) -> None:
        """Watch method called when expanded_nodes set changes.

        Updates the visual expansion state of tree nodes to match the
        reactive property state.

        Args:
            old_nodes: Previous set of expanded task IDs
            new_nodes: New set of expanded task IDs
        """
        # Determine which nodes changed
        newly_expanded = new_nodes - old_nodes
        newly_collapsed = old_nodes - new_nodes

        # Update node expansion state
        for task_id in newly_expanded:
            if node := self._node_map.get(task_id):
                node.expand()
                self.post_message(self.NodeExpanded(task_id, is_expanded=True))

        for task_id in newly_collapsed:
            if node := self._node_map.get(task_id):
                node.collapse()
                self.post_message(self.NodeExpanded(task_id, is_expanded=False))

    def load_tasks(
        self, tasks: list[Task], dependency_graph: dict[UUID, list[UUID]]
    ) -> None:
        """Load tasks into the tree widget.

        Clears existing tree content and rebuilds it from the provided tasks
        and dependency graph. Tasks are organized hierarchically based on
        parent_task_id relationships.

        Args:
            tasks: List of Task objects to display
            dependency_graph: Dict mapping task IDs to lists of dependent task IDs
                            (used for visual hierarchy, not parent relationships)
        """
        # Clear existing tree
        self.clear()
        self._task_data.clear()
        self._node_map.clear()

        # Build task lookup dict
        for task in tasks:
            self._task_data[task.id] = task

        # Build parent-child relationships based on parent_task_id
        root_tasks = [t for t in tasks if t.parent_task_id is None]
        child_map: dict[UUID, list[Task]] = {}
        for task in tasks:
            if task.parent_task_id is not None:
                child_map.setdefault(task.parent_task_id, []).append(task)

        # Add root tasks to tree
        for task in sorted(
            root_tasks, key=lambda t: (-t.calculated_priority, t.submitted_at)
        ):
            self._add_task_node(self.root, task, child_map)

    def _add_task_node(
        self,
        parent_node: TreeNode[dict[str, Any]],
        task: Task,
        child_map: dict[UUID, list[Task]],
    ) -> TreeNode[dict[str, Any]]:
        """Add a task node to the tree recursively.

        Args:
            parent_node: Parent TreeNode to add this task under
            task: Task to add
            child_map: Dict mapping parent task IDs to child task lists

        Returns:
            The created TreeNode
        """
        # Format task label using TreeRenderer
        label = self._renderer.format_task_node(task)

        # Create node with task data
        node = parent_node.add(
            label,
            data={"task_id": task.id, "task": task},
            allow_expand=task.id in child_map,
        )

        # Store node mapping
        self._node_map[task.id] = node

        # Restore expansion state if previously expanded
        if task.id in self.expanded_nodes:
            node.expand()
        else:
            node.collapse()

        # Recursively add children
        if task.id in child_map:
            children = child_map[task.id]
            for child in sorted(
                children, key=lambda t: (-t.calculated_priority, t.submitted_at)
            ):
                self._add_task_node(node, child, child_map)

        return node

    def _get_cursor_task_id(self) -> UUID | None:
        """Get the task ID of the currently focused node.

        Returns:
            UUID of task at cursor position, or None if no task
        """
        if self.cursor_node is None:
            return None
        if self.cursor_node.data is None:
            return None
        return self.cursor_node.data.get("task_id")

    # Action methods for keyboard bindings

    def action_navigate_up(self) -> None:
        """Navigate to previous tree node (up arrow / k)."""
        # Call parent action which handles cursor movement
        self.action_cursor_up()
        # Update selected task immediately
        task_id = self._get_cursor_task_id()
        if task_id is not None:
            self.selected_task_id = task_id

    def action_navigate_down(self) -> None:
        """Navigate to next tree node (down arrow / j)."""
        # If cursor is not initialized, move to first child
        if self.cursor_node is None or self.cursor_node == self.root:
            if self.root.children:
                # Directly select first child instead of using cursor_down
                first_child = self.root.children[0]
                self.select_node(first_child)
                task_id = self._get_cursor_task_id()
                if task_id is not None:
                    self.selected_task_id = task_id
                return

        # Call parent action which handles cursor movement
        self.action_cursor_down()
        # Update selected task immediately
        task_id = self._get_cursor_task_id()
        if task_id is not None:
            self.selected_task_id = task_id

    def action_collapse_node(self) -> None:
        """Collapse current tree node (left arrow / h)."""
        if self.cursor_node is None:
            return

        task_id = self._get_cursor_task_id()
        if task_id is None:
            return

        # If node is expanded, collapse it
        if self.cursor_node.is_expanded:
            self.cursor_node.collapse()
            # Update reactive property
            new_expanded = self.expanded_nodes.copy()
            new_expanded.discard(task_id)
            self.expanded_nodes = new_expanded
        else:
            # If already collapsed, navigate to parent
            if self.cursor_node.parent and self.cursor_node.parent != self.root:
                self.select_node(self.cursor_node.parent)
                parent_task_id = self._get_cursor_task_id()
                if parent_task_id is not None:
                    self.selected_task_id = parent_task_id

    def action_expand_node(self) -> None:
        """Expand current tree node (right arrow / l)."""
        if self.cursor_node is None:
            return

        task_id = self._get_cursor_task_id()
        if task_id is None:
            return

        # Check if node has children (either allow_expand or actual children present)
        has_children = self.cursor_node.allow_expand or bool(self.cursor_node.children)

        # If node has children and is collapsed, expand it
        if has_children and not self.cursor_node.is_expanded:
            self.cursor_node.expand()
            # Update reactive property
            new_expanded = self.expanded_nodes.copy()
            new_expanded.add(task_id)
            self.expanded_nodes = new_expanded
        elif self.cursor_node.is_expanded and self.cursor_node.children:
            # If already expanded, navigate to first child
            self.select_node(self.cursor_node.children[0])
            child_task_id = self._get_cursor_task_id()
            if child_task_id is not None:
                self.selected_task_id = child_task_id

    def action_toggle_expand(self) -> None:
        """Toggle expansion of current node (enter / space)."""
        if self.cursor_node is None:
            return

        task_id = self._get_cursor_task_id()
        if task_id is None:
            return

        # Check if node has children
        has_children = self.cursor_node.allow_expand or bool(self.cursor_node.children)
        if not has_children:
            return

        # Toggle expansion
        if self.cursor_node.is_expanded:
            self.cursor_node.collapse()
            new_expanded = self.expanded_nodes.copy()
            new_expanded.discard(task_id)
            self.expanded_nodes = new_expanded
        else:
            self.cursor_node.expand()
            new_expanded = self.expanded_nodes.copy()
            new_expanded.add(task_id)
            self.expanded_nodes = new_expanded

    def action_goto_top(self) -> None:
        """Jump to top of tree (g key)."""
        # Move to first node
        if self.root.children:
            self.select_node(self.root.children[0])
            task_id = self._get_cursor_task_id()
            if task_id is not None:
                self.selected_task_id = task_id

    def action_goto_bottom(self) -> None:
        """Jump to bottom of tree (G key)."""
        # Find last visible node
        def find_last_visible(node: TreeNode[dict[str, Any]]) -> TreeNode[dict[str, Any]]:
            """Recursively find the last visible (expanded) node."""
            if node.is_expanded and node.children:
                return find_last_visible(node.children[-1])
            return node

        if self.root.children:
            last_node = find_last_visible(self.root.children[-1])
            self.select_node(last_node)
            task_id = self._get_cursor_task_id()
            if task_id is not None:
                self.selected_task_id = task_id

    def on_tree_node_selected(self, event: Tree.NodeSelected[dict[str, Any]]) -> None:
        """Handle tree node selection event (mouse click or enter key).

        Args:
            event: NodeSelected event from Textual Tree
        """
        if event.node.data:
            task_id = event.node.data.get("task_id")
            if task_id is not None:
                self.selected_task_id = task_id
