"""Tree Renderer for Hierarchical Task Visualization.

This module provides tree layout computation and Rich text rendering
with TaskStatus-based color coding for terminal display.
"""

from typing import Dict, List
from uuid import UUID
from collections import defaultdict

from rich.text import Text
from rich.tree import Tree as RichTree

from abathur.domain.models import Task, TaskStatus
from abathur.tui.models import TreeNode, TreeLayout, ViewMode


# TaskStatus color mapping constants
# Colors are matched to existing task_visualizer.py GraphViz colors
# for visual consistency across visualization formats
TASK_STATUS_COLORS: Dict[TaskStatus, str] = {
    TaskStatus.PENDING: "blue",  # GraphViz: lightblue
    TaskStatus.BLOCKED: "yellow",  # GraphViz: yellow
    TaskStatus.READY: "green",  # GraphViz: lightgreen
    TaskStatus.RUNNING: "magenta",  # GraphViz: orange (magenta chosen for better contrast)
    TaskStatus.COMPLETED: "bright_green",  # GraphViz: green (bright_green for emphasis)
    TaskStatus.FAILED: "red",  # GraphViz: red
    TaskStatus.CANCELLED: "dim",  # GraphViz: gray (dim style for de-emphasis)
}


def get_status_color(status: TaskStatus) -> str:
    """Get Rich color string for a TaskStatus value.

    This helper provides graceful fallback for unknown status values.

    Args:
        status: TaskStatus enum value

    Returns:
        Rich color name string (e.g., "blue", "bright_green", "dim")
        Returns "white" for unknown status values.

    Examples:
        >>> get_status_color(TaskStatus.COMPLETED)
        'bright_green'
        >>> get_status_color(TaskStatus.FAILED)
        'red'
        >>> get_status_color(TaskStatus.PENDING)
        'blue'
    """
    return TASK_STATUS_COLORS.get(status, "white")


class TreeRenderer:
    """Computes tree layout and generates Rich renderables for DAG visualization.

    This class handles:
    - Hierarchical layout computation from task dependencies
    - TaskStatus-based color coding
    - Rich Text formatting with Unicode/ASCII box-drawing
    - Tree structure rendering for terminal display
    """

    # Color mapping exposed as class constant for easy access
    STATUS_COLORS = TASK_STATUS_COLORS

    def format_task_node(self, task: Task) -> Text:
        """Format task as Rich Text with status-based color-coding.

        Format: [status_color]{summary[:40]}[/] [dim]({priority})[/]

        Args:
            task: Task object to format

        Returns:
            Rich Text object with color formatting and priority display

        Examples:
            Task with summary "Implement feature X" and priority 8.5
            renders as colored text: "Implement feature X (8.5)"
        """
        color = get_status_color(task.status)

        # Truncate summary to 40 chars
        summary = task.summary[:40] if task.summary else task.prompt[:40]
        if len(task.summary or task.prompt or "") > 40:
            summary += "..."

        # Format: colored summary + priority in dim
        text = Text()
        text.append(summary, style=color)
        text.append(f" ({task.calculated_priority:.1f})", style="dim")

        return text

    def _get_status_icon(self, status: TaskStatus) -> str:
        """Get Unicode icon for task status.

        Provides both color AND symbol for accessibility.
        Supports users with color blindness or limited color terminals.

        Args:
            status: TaskStatus enum value

        Returns:
            Unicode status icon character

        Status Icons:
            - PENDING: ○ (empty circle)
            - BLOCKED: ⊗ (circled times)
            - READY: ◎ (circled dot)
            - RUNNING: ◉ (filled circle)
            - COMPLETED: ✓ (check mark)
            - FAILED: ✗ (ballot X)
            - CANCELLED: ⊘ (circled slash)
        """
        icons = {
            TaskStatus.PENDING: "○",
            TaskStatus.BLOCKED: "⊗",
            TaskStatus.READY: "◎",
            TaskStatus.RUNNING: "◉",
            TaskStatus.COMPLETED: "✓",
            TaskStatus.FAILED: "✗",
            TaskStatus.CANCELLED: "⊘",
        }
        return icons.get(status, "○")

    def render_flat_list(self, tasks: list[Task], max_width: int = 80) -> list[Text]:
        """Render tasks as flat list with color-coding (for flat view mode).

        Args:
            tasks: Tasks to render
            max_width: Maximum line width (currently unused, reserved for future wrapping)

        Returns:
            List of Rich Text lines with status icons and colors
        """
        lines = []

        for task in tasks:
            text = self.format_task_node(task)

            # Add status indicator
            status_icon = self._get_status_icon(task.status)
            line = Text()
            line.append(status_icon + " ", style=get_status_color(task.status))
            line.append(text)

            lines.append(line)

        return lines

    def compute_layout(
        self,
        tasks: list[Task],
        dependency_graph: dict[UUID, list[UUID]],
    ) -> TreeLayout:
        """Compute hierarchical tree layout from task list and dependency graph.

        Implements hierarchical layout algorithm:
        1. Group tasks by dependency_depth (hierarchical levels)
        2. Sort within each level by calculated_priority (descending)
        3. Build parent-child relationships using parent_task_id
        4. Assign position numbers within each level

        Args:
            tasks: List of Task objects to layout
            dependency_graph: Dict mapping task_id -> list of prerequisite task_ids
                             (Currently unused, reserved for future DAG rendering)

        Returns:
            TreeLayout with computed node positions and hierarchy

        Example:
            >>> renderer = TreeRenderer()
            >>> tasks = [parent_task, child1_task, child2_task]
            >>> layout = renderer.compute_layout(tasks, {})
            >>> print(f"Max depth: {layout.max_depth}")
            Max depth: 1
            >>> print(f"Root nodes: {len(layout.root_nodes)}")
            Root nodes: 1
        """
        layout = TreeLayout()

        # Group by dependency depth (hierarchical levels)
        levels: Dict[int, List[Task]] = defaultdict(list)
        for task in tasks:
            levels[task.dependency_depth].append(task)

        # Sort each level by priority (descending - highest priority first)
        for level in levels.values():
            level.sort(key=lambda t: t.calculated_priority, reverse=True)

        # Build nodes with position assignments
        nodes: Dict[UUID, TreeNode] = {}
        for depth, tasks_at_level in sorted(levels.items()):
            for position, task in enumerate(tasks_at_level):
                nodes[task.id] = TreeNode(
                    task_id=task.id,
                    task=task,
                    children=[],
                    level=depth,
                    is_expanded=True,
                    position=position,
                )

        # Build parent-child relationships using parent_task_id
        root_nodes = []
        for task_id, node in nodes.items():
            task = node.task

            if task.parent_task_id is None:
                # Root node (no parent)
                root_nodes.append(task_id)
            else:
                # Add as child to parent
                if task.parent_task_id in nodes:
                    nodes[task.parent_task_id].children.append(task_id)
                else:
                    # Parent not in tree - treat as orphan root
                    root_nodes.append(task_id)

        # Finalize layout
        layout.nodes = nodes
        layout.root_nodes = root_nodes
        layout.max_depth = max(levels.keys()) if levels else 0
        layout.total_nodes = len(nodes)

        return layout

    def render_tree(
        self,
        layout: TreeLayout,
        expanded_nodes: set[UUID] | None = None,
        use_unicode: bool = True,
    ) -> RichTree:
        """Render tree structure using Rich Tree widget with box-drawing.

        Args:
            layout: TreeLayout with node hierarchy
            expanded_nodes: Set of expanded node IDs. If None, uses each node's
                          is_expanded property. If provided (even if empty),
                          it's authoritative - nodes not in set are collapsed.
            use_unicode: Use Unicode box-drawing (│ ├ └ ─) vs ASCII (| + - \\)

        Returns:
            Rich Tree widget ready for console rendering

        Example:
            >>> layout = renderer.compute_layout(tasks, {})
            >>> expanded = set(layout.nodes.keys())  # All expanded
            >>> tree = renderer.render_tree(layout, expanded, use_unicode=True)
            >>> from rich.console import Console
            >>> console = Console()
            >>> console.print(tree)
        """
        # Configure box-drawing style
        # Rich Tree uses Unicode by default if supported
        # The use_unicode parameter is currently ignored as Rich handles this automatically
        # Future enhancement: support forced ASCII mode via Console configuration
        guide_style = "tree.line"

        # Create root tree with title
        root_tree = RichTree("Task Queue", guide_style=guide_style)

        # Recursively build tree from layout
        def add_subtree(parent_widget: RichTree, node_id: UUID) -> None:
            """Recursively add node and its children to parent widget."""
            if node_id not in layout.nodes:
                return

            node = layout.nodes[node_id]
            label = self.format_task_node(node.task)

            # Add node to parent widget
            subtree = parent_widget.add(label)

            # Determine if children should be shown
            # If expanded_nodes is None, use is_expanded property
            # Otherwise, expanded_nodes is authoritative
            if expanded_nodes is None:
                show_children = node.is_expanded
            else:
                show_children = node_id in expanded_nodes

            # Add children if this node is expanded
            if show_children:
                # Sort children by position for deterministic rendering
                children = sorted(
                    node.children,
                    key=lambda cid: (
                        layout.nodes[cid].position if cid in layout.nodes else 0
                    ),
                )

                for child_id in children:
                    add_subtree(subtree, child_id)

        # Build tree from all root nodes
        for root_id in layout.root_nodes:
            add_subtree(root_tree, root_id)

        return root_tree

    @staticmethod
    def supports_unicode() -> bool:
        """Detect if terminal supports Unicode box-drawing characters.

        Checks terminal encoding and LANG environment variable.

        Returns:
            True if Unicode is supported, False for ASCII fallback

        Usage:
            use_unicode = TreeRenderer.supports_unicode()
            renderer.render_tree(layout, expanded, use_unicode=use_unicode)
        """
        import sys
        import locale
        import os

        # Check encoding
        encoding = sys.stdout.encoding or locale.getpreferredencoding()
        if encoding.lower() not in ("utf-8", "utf8"):
            return False

        # Check LANG environment variable
        lang = os.environ.get("LANG", "")
        if "UTF-8" not in lang and "utf8" not in lang:
            return False

        return True
