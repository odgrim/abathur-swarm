#!/usr/bin/env python3
"""Test script for Unicode/ASCII tree rendering.

This script creates a hierarchical task structure and tests
the TreeRenderer with both Unicode and ASCII modes.
"""

import sys
import os
from datetime import datetime, timezone
from uuid import uuid4, UUID

# Add src to path
sys.path.insert(0, '/Users/odgrim/dev/home/agentics/abathur/.abathur/worktrees/cli-tree-unicode-ascii/src')

from abathur.domain.models import Task, TaskStatus
from abathur.tui.rendering.tree_renderer import TreeRenderer
from rich.console import Console


def create_test_task(
    summary: str,
    status: TaskStatus,
    priority: float = 5.0,
    parent_id: UUID | None = None,
    depth: int = 0,
) -> Task:
    """Create a test task with synthetic data."""
    task_id = uuid4()
    now = datetime.now(timezone.utc)

    return Task(
        id=task_id,
        prompt=f"Test task: {summary}",
        summary=summary,
        agent_type="test-agent",
        source="human",
        status=status,
        parent_task_id=parent_id,
        dependency_depth=depth,
        base_priority=priority,
        calculated_priority=priority,
        created_at=now,
        updated_at=now,
    )


def test_unicode_rendering():
    """Test Unicode box-drawing mode."""
    console = Console()

    # Create hierarchical task structure
    parent = create_test_task("Implement authentication", TaskStatus.RUNNING, 9.0, depth=0)

    child1 = create_test_task("Design user model", TaskStatus.COMPLETED, 8.5, parent.id, depth=1)
    child2 = create_test_task("Implement login endpoint", TaskStatus.RUNNING, 8.0, parent.id, depth=1)
    child3 = create_test_task("Add JWT tokens", TaskStatus.PENDING, 7.5, parent.id, depth=1)

    grandchild1 = create_test_task("Add password hashing", TaskStatus.COMPLETED, 7.0, child1.id, depth=2)
    grandchild2 = create_test_task("Add email validation", TaskStatus.COMPLETED, 6.5, child1.id, depth=2)

    grandchild3 = create_test_task("Validate credentials", TaskStatus.RUNNING, 7.0, child2.id, depth=2)
    grandchild4 = create_test_task("Generate session", TaskStatus.PENDING, 6.5, child2.id, depth=2)

    tasks = [
        parent,
        child1, child2, child3,
        grandchild1, grandchild2, grandchild3, grandchild4,
    ]

    # Create renderer
    renderer = TreeRenderer()

    # Test Unicode mode
    console.print("\n[bold cyan]═══ Unicode Mode Test ═══[/bold cyan]")
    console.print(f"Encoding: {sys.stdout.encoding}")
    console.print(f"LANG: {os.environ.get('LANG', 'NOT SET')}")
    console.print(f"supports_unicode(): {TreeRenderer.supports_unicode()}\n")

    # Compute layout
    dependency_graph: dict[UUID, list[UUID]] = {}
    layout = renderer.compute_layout(tasks, dependency_graph)

    # Render tree (forced Unicode)
    tree = renderer.render_tree(layout, use_unicode=True)
    console.print(tree)

    # Show expected characters
    console.print("\n[dim]Expected Unicode characters:[/dim]")
    console.print("├── (U+251C U+2500 U+2500) - mid connector")
    console.print("└── (U+2514 U+2500 U+2500) - last connector")
    console.print("│   (U+2502) - vertical line")


def test_ascii_rendering():
    """Test ASCII fallback mode."""
    console = Console()

    # Create hierarchical task structure (same as Unicode test)
    parent = create_test_task("Implement authentication", TaskStatus.RUNNING, 9.0, depth=0)

    child1 = create_test_task("Design user model", TaskStatus.COMPLETED, 8.5, parent.id, depth=1)
    child2 = create_test_task("Implement login endpoint", TaskStatus.RUNNING, 8.0, parent.id, depth=1)
    child3 = create_test_task("Add JWT tokens", TaskStatus.PENDING, 7.5, parent.id, depth=1)

    grandchild1 = create_test_task("Add password hashing", TaskStatus.COMPLETED, 7.0, child1.id, depth=2)
    grandchild2 = create_test_task("Add email validation", TaskStatus.COMPLETED, 6.5, child1.id, depth=2)

    grandchild3 = create_test_task("Validate credentials", TaskStatus.RUNNING, 7.0, child2.id, depth=2)
    grandchild4 = create_test_task("Generate session", TaskStatus.PENDING, 6.5, child2.id, depth=2)

    tasks = [
        parent,
        child1, child2, child3,
        grandchild1, grandchild2, grandchild3, grandchild4,
    ]

    # Create renderer
    renderer = TreeRenderer()

    # Test ASCII mode
    console.print("\n[bold cyan]═══ ASCII Mode Test ═══[/bold cyan]")
    console.print(f"Encoding: {sys.stdout.encoding}")
    console.print(f"LANG: {os.environ.get('LANG', 'NOT SET')}")
    console.print(f"supports_unicode(): {TreeRenderer.supports_unicode()}\n")

    # Compute layout
    dependency_graph: dict[UUID, list[UUID]] = {}
    layout = renderer.compute_layout(tasks, dependency_graph)

    # Render tree (forced ASCII)
    tree = renderer.render_tree(layout, use_unicode=False)
    console.print(tree)

    # Show expected characters
    console.print("\n[dim]Expected ASCII characters:[/dim]")
    console.print("|-- - mid connector")
    console.print("`-- - last connector")
    console.print("|   - vertical line")


def test_auto_detection():
    """Test automatic Unicode detection."""
    console = Console()

    console.print("\n[bold cyan]═══ Auto-Detection Test ═══[/bold cyan]")

    # Test supports_unicode() logic
    console.print(f"\nSystem Info:")
    console.print(f"  stdout.encoding: {sys.stdout.encoding}")
    console.print(f"  LANG env var: {os.environ.get('LANG', 'NOT SET')}")

    supports = TreeRenderer.supports_unicode()
    console.print(f"\n  TreeRenderer.supports_unicode(): {supports}")

    if supports:
        console.print("  [green]✓[/green] Unicode box-drawing characters will be used")
    else:
        console.print("  [yellow]![/yellow] ASCII fallback characters will be used")

    # Show detection logic
    console.print("\n[dim]Detection logic:[/dim]")
    console.print("  1. Check stdout.encoding is 'utf-8' or 'utf8'")
    console.print("  2. Check LANG contains 'UTF-8' or 'utf8'")
    console.print("  3. Both must be true for Unicode mode")


if __name__ == "__main__":
    # Run tests based on environment
    if len(sys.argv) > 1:
        mode = sys.argv[1]
        if mode == "unicode":
            test_unicode_rendering()
        elif mode == "ascii":
            test_ascii_rendering()
        elif mode == "detect":
            test_auto_detection()
        else:
            print(f"Unknown mode: {mode}")
            print("Usage: test_tree_unicode.py [unicode|ascii|detect]")
            sys.exit(1)
    else:
        # Run all tests
        test_auto_detection()
        test_unicode_rendering()
        test_ascii_rendering()
