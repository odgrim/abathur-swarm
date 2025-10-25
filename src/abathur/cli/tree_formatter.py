"""Tree Formatter - Stdout tree display for hierarchical task visualization.

Provides Rich library-based tree formatting matching Unix 'tree' command style.
Features: parent_task_id hierarchy, status color coding, Unicode/ASCII box-drawing.
"""

import sys
import locale
import os
from typing import Dict
from uuid import UUID
from collections import defaultdict

from rich.tree import Tree as RichTree
from rich.text import Text

from abathur.domain.models import Task, TaskStatus


# TaskStatus color mapping constants
# Colors matched to existing tree_renderer.py for visual consistency
TASK_STATUS_COLORS: Dict[TaskStatus, str] = {
    TaskStatus.PENDING: "blue",
    TaskStatus.BLOCKED: "yellow",
    TaskStatus.READY: "green",
    TaskStatus.RUNNING: "magenta",
    TaskStatus.COMPLETED: "bright_green",
    TaskStatus.FAILED: "red",
    TaskStatus.CANCELLED: "dim",
}


def get_status_color(status: TaskStatus) -> str:
    """Map TaskStatus to Rich color name.

    Args:
        status: TaskStatus enum value

    Returns:
        Rich color name ('blue', 'bright_green', 'red', etc.)
        Defaults to 'white' for unknown status.
    """
    return TASK_STATUS_COLORS.get(status, "white")


def supports_unicode() -> bool:
    """Detect if terminal supports Unicode box-drawing characters.

    Returns:
        True if UTF-8 encoding detected, False for ASCII fallback.
        Returns False on any exception (safe fallback).
    """
    try:
        # Check encoding
        encoding = sys.stdout.encoding or locale.getpreferredencoding()
        if encoding.lower() not in ("utf-8", "utf8"):
            return False

        # Check LANG environment variable
        lang = os.environ.get("LANG", "")
        if "UTF-8" not in lang and "utf8" not in lang.lower():
            return False

        return True

    except Exception:
        # Safe fallback to ASCII on any detection error
        return False


def format_task_line(task: Task) -> Text:
    """Format single task as Rich Text: '<id-prefix> <summary> (<priority>)'.

    Args:
        task: Task object to format

    Returns:
        Rich Text with status color, ID prefix (8 chars), summary (max 60 chars), priority.
        Summary fallback: task.prompt if summary is None, 'Untitled Task' if both None.
    """
    # Extract task ID prefix (first 8 chars of UUID)
    id_prefix = str(task.id)[:8]

    # Get summary with fallback to prompt
    summary = task.summary if task.summary is not None else task.prompt
    if summary is None:
        summary = "Untitled Task"

    # Truncate summary to 60 chars
    if len(summary) > 60:
        summary = summary[:60] + "..."

    # Get status color
    color = get_status_color(task.status)

    # Format: '<id-prefix> <summary> (<priority>)'
    priority = getattr(task, 'calculated_priority', task.priority)
    formatted_text = f"{id_prefix} {summary} ({priority:.1f})"

    # Create Rich Text with color
    text = Text(formatted_text, style=color)

    return text


def format_tree(tasks: list[Task], use_unicode: bool = True) -> RichTree:
    """Build hierarchical tree from task list using parent_task_id relationships.

    O(n) algorithm: Build task_map → group by parent_task_id → recursively build tree.

    Args:
        tasks: List of Task objects to render
        use_unicode: Use Unicode box-drawing (│ ├ └ ─) vs ASCII (| + - \\)

    Returns:
        Rich Tree ready for console.print()

    Edge cases: Empty list → 'No tasks found', orphaned tasks → root level,
    circular refs → break cycles with visited set.
    """
    # Configure box-drawing style
    # Note: Rich Tree auto-detects terminal Unicode support.
    # The use_unicode parameter is primarily for documentation and consistency.
    # The guide_style controls the color of connecting lines.
    guide_style = "tree.line" if use_unicode else "dim"

    # Create root tree
    root_tree = RichTree("Task Queue", guide_style=guide_style)

    # Handle empty task list
    if not tasks:
        root_tree.add(Text("No tasks found", style="dim"))
        return root_tree

    # Build task_map for O(1) lookup
    task_map: Dict[UUID, Task] = {task.id: task for task in tasks}

    # Group tasks by parent_task_id for efficient tree construction
    children_map: Dict[UUID | None, list[Task]] = defaultdict(list)
    for task in tasks:
        children_map[task.parent_task_id].append(task)

    # Sort children by priority (highest first)
    for child_list in children_map.values():
        priority_attr = lambda t: getattr(t, 'calculated_priority', t.priority)
        child_list.sort(key=priority_attr, reverse=True)

    # Track visited nodes to prevent infinite loops from circular references
    visited: set[UUID] = set()

    def add_subtree(parent_widget: RichTree, task: Task) -> None:
        """Recursively add task and its children to parent widget.

        Handles circular reference detection via visited set.
        """
        # Check for circular reference
        if task.id in visited:
            # Break cycle - don't recurse further
            return

        visited.add(task.id)

        # Format task line and add to parent widget
        label = format_task_line(task)
        subtree = parent_widget.add(label)

        # Add children if they exist
        if task.id in children_map:
            for child in children_map[task.id]:
                add_subtree(subtree, child)

    # Build tree from root tasks (parent_task_id=None)
    root_tasks = children_map[None]

    if not root_tasks:
        # All tasks have parent_task_id set - find orphaned tasks
        # (tasks whose parent_task_id doesn't exist in task_map)
        for task in tasks:
            if task.parent_task_id is not None and task.parent_task_id not in task_map:
                # Orphaned task - add to root level
                add_subtree(root_tree, task)

        # If still no tasks added, all tasks reference valid parents but no roots
        if not visited:
            root_tree.add(Text("No root tasks found (all tasks have parent_task_id set)", style="dim yellow"))
    else:
        # Normal case: add all root tasks
        for root_task in root_tasks:
            add_subtree(root_tree, root_task)

    return root_tree
