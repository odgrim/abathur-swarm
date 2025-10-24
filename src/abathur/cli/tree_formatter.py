"""Tree Formatter for CLI - Wrapper around TUI TreeRenderer.

This module provides a simplified interface for CLI tree display,
wrapping the TUI TreeRenderer for command-line use.
"""

from typing import Any
from uuid import UUID

from rich.console import Console
from rich.tree import Tree as RichTree

from abathur.domain.models import Task
from abathur.tui.rendering.tree_renderer import TreeRenderer


def format_tree(
    tasks: list[Task],
    use_unicode: bool = True,
    console: Console | None = None,
) -> tuple[RichTree, Console]:
    """Format tasks as tree structure for CLI display.

    Args:
        tasks: List of Task objects to display
        use_unicode: Use Unicode box-drawing (│ ├ └ ─) vs ASCII (| + - \\)
        console: Optional Console instance. If None, creates one with appropriate settings.

    Returns:
        Tuple of (Rich Tree widget, Console instance to use for rendering)

    Example:
        >>> from rich.console import Console
        >>> tree_widget, console = format_tree(tasks, use_unicode=True)
        >>> console.print(tree_widget)
    """
    # Create console with appropriate settings for ASCII/Unicode
    if console is None:
        # Force ASCII mode by setting legacy_windows=True
        # This makes Rich use +, |, - instead of Unicode box-drawing
        console = Console(legacy_windows=not use_unicode)

    renderer = TreeRenderer()

    # Build dependency graph (empty for parent-based tree)
    dependency_graph: dict[UUID, list[UUID]] = {}

    # Compute tree layout
    layout = renderer.compute_layout(tasks, dependency_graph)

    # Render tree with all nodes expanded
    expanded_nodes = set(layout.nodes.keys())
    tree = renderer.render_tree(layout, expanded_nodes, use_unicode=use_unicode)

    return tree, console


def supports_unicode() -> bool:
    """Detect if terminal supports Unicode box-drawing characters.

    Checks terminal encoding and LANG environment variable.

    Returns:
        True if Unicode is supported, False for ASCII fallback

    Usage:
        >>> if supports_unicode():
        ...     tree = format_tree(tasks, use_unicode=True)
        ... else:
        ...     tree = format_tree(tasks, use_unicode=False)
    """
    return TreeRenderer.supports_unicode()
