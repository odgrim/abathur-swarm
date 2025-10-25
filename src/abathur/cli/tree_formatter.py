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


def format_lineage_tree(tasks: list[Task], use_unicode: bool = True) -> RichTree:
    """Build hierarchical tree from task list using parent_task_id relationships.

    O(n) algorithm: Build task_map → build children map → recursively build tree.

    Tree displays lineage (spawning) relationships: tasks appear nested under their parent.
    For example, if Task A spawned Task B, the tree shows:
        Task A (parent)
        └── Task B (child)

    Args:
        tasks: List of Task objects to render
        use_unicode: Use Unicode box-drawing (│ ├ └ ─) vs ASCII (| + - \\)

    Returns:
        Rich Tree ready for console.print()

    Edge cases: Empty list → 'No tasks found', tasks without parent → root level,
    circular refs → break cycles with visited set.
    """
    # Configure box-drawing style
    guide_style = "tree.line" if use_unicode else "dim"

    # Create root tree
    root_tree = RichTree("Task Queue (Lineage)", guide_style=guide_style)

    # Handle empty task list
    if not tasks:
        root_tree.add(Text("No tasks found", style="dim"))
        return root_tree

    # Build task_map for O(1) lookup
    task_map: Dict[UUID, Task] = {task.id: task for task in tasks}

    # Build children map: which tasks were spawned by each task
    # children_map[task_id] = list of tasks spawned by task_id
    children_map: Dict[UUID, list[Task]] = defaultdict(list)
    for task in tasks:
        if task.parent_task_id and task.parent_task_id in task_map:
            children_map[task.parent_task_id].append(task)

    # Sort children by priority (highest first)
    for children_list in children_map.values():
        priority_attr = lambda t: getattr(t, 'calculated_priority', t.priority)
        children_list.sort(key=priority_attr, reverse=True)

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

        # Add children (tasks spawned by this task)
        if task.id in children_map:
            for child_task in children_map[task.id]:
                add_subtree(subtree, child_task)

    # Find root tasks: tasks with no parent_task_id or parent not in our list
    root_tasks = [
        task for task in tasks
        if not task.parent_task_id or task.parent_task_id not in task_map
    ]

    # Sort root tasks by priority (highest first)
    priority_attr = lambda t: getattr(t, 'calculated_priority', t.priority)
    root_tasks.sort(key=priority_attr, reverse=True)

    if not root_tasks:
        # All tasks have parents - might be circular references or missing parents
        # Just show all tasks at root level
        root_tree.add(Text("All tasks have parent tasks - showing flat list", style="dim yellow"))
        for task in sorted(tasks, key=priority_attr, reverse=True):
            if task.id not in visited:
                add_subtree(root_tree, task)
    else:
        # Normal case: add all root tasks
        for root_task in root_tasks:
            add_subtree(root_tree, root_task)

    return root_tree


def format_tree(tasks: list[Task], use_unicode: bool = True) -> RichTree:
    """Build hierarchical tree from task list using dependency relationships.

    O(n) algorithm: Build task_map → build reverse dependency map → recursively build tree.

    Tree displays dependency ordering: tasks appear nested under the tasks that depend on them.
    For example, if Task A depends on Task B, the tree shows:
        Task A
        └── Task B (dependency)

    Args:
        tasks: List of Task objects to render
        use_unicode: Use Unicode box-drawing (│ ├ └ ─) vs ASCII (| + - \\)

    Returns:
        Rich Tree ready for console.print()

    Edge cases: Empty list → 'No tasks found', tasks without dependencies → root level,
    circular refs → break cycles with visited set.
    """
    # Configure box-drawing style
    # Note: Rich Tree auto-detects terminal Unicode support.
    # The use_unicode parameter is primarily for documentation and consistency.
    # The guide_style controls the color of connecting lines.
    guide_style = "tree.line" if use_unicode else "dim"

    # Create root tree
    root_tree = RichTree("Task Queue (Dependencies)", guide_style=guide_style)

    # Handle empty task list
    if not tasks:
        root_tree.add(Text("No tasks found", style="dim"))
        return root_tree

    # Build task_map for O(1) lookup
    task_map: Dict[UUID, Task] = {task.id: task for task in tasks}

    # Build reverse dependency map: which tasks depend on each task
    # dependents_map[task_id] = list of tasks that depend on task_id
    dependents_map: Dict[UUID, list[Task]] = defaultdict(list)
    for task in tasks:
        for dep_id in task.dependencies:
            # Only add if the dependency exists in our task list
            if dep_id in task_map:
                dependents_map[dep_id].append(task)

    # Sort dependents by priority (highest first)
    for dependent_list in dependents_map.values():
        priority_attr = lambda t: getattr(t, 'calculated_priority', t.priority)
        dependent_list.sort(key=priority_attr, reverse=True)

    # Track visited nodes to prevent infinite loops from circular references
    visited: set[UUID] = set()

    def add_subtree(parent_widget: RichTree, task: Task) -> None:
        """Recursively add task and its dependencies to parent widget.

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

        # Add dependencies (tasks that this task depends on)
        for dep_id in task.dependencies:
            if dep_id in task_map:
                dep_task = task_map[dep_id]
                add_subtree(subtree, dep_task)

    # Find root tasks: tasks that no other task depends on
    tasks_depended_on = set(dependents_map.keys())
    root_tasks = [task for task in tasks if task.id not in tasks_depended_on]

    # Sort root tasks by priority (highest first)
    priority_attr = lambda t: getattr(t, 'calculated_priority', t.priority)
    root_tasks.sort(key=priority_attr, reverse=True)

    if not root_tasks:
        # All tasks are depended on by other tasks - might be circular dependencies
        # Just show all tasks at root level
        root_tree.add(Text("All tasks have dependencies - showing flat list", style="dim yellow"))
        for task in sorted(tasks, key=priority_attr, reverse=True):
            if task.id not in visited:
                add_subtree(root_tree, task)
    else:
        # Normal case: add all root tasks
        for root_task in root_tasks:
            add_subtree(root_tree, root_task)

    return root_tree
