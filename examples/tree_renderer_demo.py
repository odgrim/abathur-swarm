#!/usr/bin/env python3
"""Visual demonstration of TreeRenderer with color-coded task statuses.

Run this script to see the tree renderer in action with all task statuses.
This is a manual/visual test to validate rendering output in the terminal.

Usage:
    PYTHONPATH=src python examples/tree_renderer_demo.py
"""

from uuid import uuid4
from datetime import datetime, timezone

from rich.console import Console

from abathur.domain.models import Task, TaskStatus, TaskSource
from abathur.tui.rendering.tree_renderer import TreeRenderer


def create_sample_task_hierarchy():
    """Create sample task hierarchy showcasing all task statuses."""

    # Root tasks (different statuses)
    root_completed = Task(
        id=uuid4(),
        prompt="Implement user authentication system",
        summary="User authentication",
        agent_type="technical-architect",
        status=TaskStatus.COMPLETED,
        calculated_priority=10.0,
        dependency_depth=0,
        submitted_at=datetime.now(timezone.utc),
        source=TaskSource.HUMAN,
        parent_task_id=None,
    )

    root_running = Task(
        id=uuid4(),
        prompt="Build task queue visualization TUI",
        summary="Task queue TUI",
        agent_type="python-textual-tui-foundation-specialist",
        status=TaskStatus.RUNNING,
        calculated_priority=9.0,
        dependency_depth=0,
        submitted_at=datetime.now(timezone.utc),
        source=TaskSource.HUMAN,
        parent_task_id=None,
    )

    root_pending = Task(
        id=uuid4(),
        prompt="Refactor database query layer",
        summary="Database refactoring",
        agent_type="requirements-gatherer",
        status=TaskStatus.PENDING,
        calculated_priority=7.0,
        dependency_depth=0,
        submitted_at=datetime.now(timezone.utc),
        source=TaskSource.HUMAN,
        parent_task_id=None,
    )

    # Children of first root (COMPLETED parent)
    child_auth_1 = Task(
        id=uuid4(),
        prompt="Implement JWT token generation",
        summary="JWT token generation",
        agent_type="python-backend-specialist",
        status=TaskStatus.COMPLETED,
        calculated_priority=8.5,
        dependency_depth=1,
        submitted_at=datetime.now(timezone.utc),
        source=TaskSource.AGENT_PLANNER,
        parent_task_id=root_completed.id,
    )

    child_auth_2 = Task(
        id=uuid4(),
        prompt="Add OAuth2 provider integration",
        summary="OAuth2 integration",
        agent_type="python-backend-specialist",
        status=TaskStatus.COMPLETED,
        calculated_priority=8.0,
        dependency_depth=1,
        submitted_at=datetime.now(timezone.utc),
        source=TaskSource.AGENT_PLANNER,
        parent_task_id=root_completed.id,
    )

    # Children of second root (RUNNING parent)
    child_tui_1 = Task(
        id=uuid4(),
        prompt="Implement TreeRenderer with hierarchical layout",
        summary="TreeRenderer implementation",
        agent_type="python-tree-dag-rendering-specialist",
        status=TaskStatus.COMPLETED,
        calculated_priority=8.5,
        dependency_depth=1,
        submitted_at=datetime.now(timezone.utc),
        source=TaskSource.AGENT_PLANNER,
        parent_task_id=root_running.id,
    )

    child_tui_2 = Task(
        id=uuid4(),
        prompt="Create TaskTreeWidget with keyboard navigation",
        summary="TaskTreeWidget",
        agent_type="python-textual-widget-specialist",
        status=TaskStatus.RUNNING,
        calculated_priority=8.0,
        dependency_depth=1,
        submitted_at=datetime.now(timezone.utc),
        source=TaskSource.AGENT_PLANNER,
        parent_task_id=root_running.id,
    )

    child_tui_3 = Task(
        id=uuid4(),
        prompt="Implement filter modal with search",
        summary="Filter modal",
        agent_type="python-filtering-and-search-specialist",
        status=TaskStatus.READY,
        calculated_priority=7.5,
        dependency_depth=1,
        submitted_at=datetime.now(timezone.utc),
        source=TaskSource.AGENT_PLANNER,
        parent_task_id=root_running.id,
    )

    child_tui_4 = Task(
        id=uuid4(),
        prompt="Write comprehensive TUI integration tests",
        summary="TUI integration tests",
        agent_type="python-tui-testing-specialist",
        status=TaskStatus.BLOCKED,
        calculated_priority=7.0,
        dependency_depth=1,
        submitted_at=datetime.now(timezone.utc),
        source=TaskSource.AGENT_PLANNER,
        parent_task_id=root_running.id,
    )

    # Grandchildren (depth 2)
    grandchild_1 = Task(
        id=uuid4(),
        prompt="Write unit tests for TreeRenderer.compute_layout()",
        summary="TreeRenderer unit tests",
        agent_type="python-testing-specialist",
        status=TaskStatus.COMPLETED,
        calculated_priority=7.5,
        dependency_depth=2,
        submitted_at=datetime.now(timezone.utc),
        source=TaskSource.AGENT_IMPLEMENTATION,
        parent_task_id=child_tui_1.id,
    )

    grandchild_2 = Task(
        id=uuid4(),
        prompt="Fix color contrast for dark terminals",
        summary="Color contrast fix",
        agent_type="python-code-editor-specialist",
        status=TaskStatus.FAILED,
        calculated_priority=6.0,
        dependency_depth=2,
        submitted_at=datetime.now(timezone.utc),
        source=TaskSource.AGENT_IMPLEMENTATION,
        parent_task_id=child_tui_1.id,
    )

    grandchild_3 = Task(
        id=uuid4(),
        prompt="Update documentation for TreeLayout API",
        summary="Documentation update",
        agent_type="technical-documentation-writer-specialist",
        status=TaskStatus.CANCELLED,
        calculated_priority=5.0,
        dependency_depth=2,
        submitted_at=datetime.now(timezone.utc),
        source=TaskSource.AGENT_IMPLEMENTATION,
        parent_task_id=child_tui_1.id,
    )

    return [
        root_completed, root_running, root_pending,
        child_auth_1, child_auth_2,
        child_tui_1, child_tui_2, child_tui_3, child_tui_4,
        grandchild_1, grandchild_2, grandchild_3,
    ]


def main():
    """Run visual test of TreeRenderer with color-coded statuses."""
    console = Console()
    renderer = TreeRenderer()

    # Create sample tasks
    tasks = create_sample_task_hierarchy()

    # Compute layout
    layout = renderer.compute_layout(tasks, {})

    # Render tree (all expanded)
    expanded = set(layout.nodes.keys())
    use_unicode = TreeRenderer.supports_unicode()

    tree = renderer.render_tree(layout, expanded, use_unicode=use_unicode)

    # Print header
    console.print("\n[bold cyan]TreeRenderer Visual Demo - Color-Coded Task Statuses[/bold cyan]\n")
    console.print(f"Total nodes: {layout.total_nodes}")
    console.print(f"Max depth: {layout.max_depth}")
    console.print(f"Root nodes: {len(layout.root_nodes)}")
    console.print(f"Unicode support: {use_unicode}")
    console.print("\n[bold]Status Legend:[/bold]")
    console.print("  [blue]● PENDING[/blue] - Task submitted, waiting")
    console.print("  [yellow]● BLOCKED[/yellow] - Waiting for dependencies")
    console.print("  [green]● READY[/green] - Ready for execution")
    console.print("  [magenta]● RUNNING[/magenta] - Currently executing")
    console.print("  [bright_green]● COMPLETED[/bright_green] - Successfully finished")
    console.print("  [red]● FAILED[/red] - Execution failed")
    console.print("  [dim]● CANCELLED[/dim] - Task cancelled")
    console.print("\n[bold]Tree Structure:[/bold]\n")

    # Render tree
    console.print(tree)

    # Test flat list rendering
    console.print("\n[bold]Flat List View (First 5 tasks):[/bold]\n")
    flat_lines = renderer.render_flat_list(tasks[:5])
    for line in flat_lines:
        console.print(line)

    # Test collapsed rendering
    console.print("\n[bold]Collapsed Tree (Only Roots Expanded):[/bold]\n")
    root_ids = layout.root_nodes
    tree_collapsed = renderer.render_tree(layout, set(root_ids), use_unicode=use_unicode)
    console.print(tree_collapsed)

    console.print("\n[bold green]✓ Visual demo complete![/bold green]\n")


if __name__ == "__main__":
    main()
