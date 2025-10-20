#!/usr/bin/env python3
"""Manual demonstration of VACUUM progress indicator.

This script demonstrates the progress indicator during VACUUM operations.
Run it to see the spinner in action during database optimization.

Usage:
    python tests/manual/test_vacuum_progress_demo.py
"""

import asyncio
from datetime import datetime, timedelta, timezone
from pathlib import Path
from tempfile import NamedTemporaryFile

from rich.console import Console
from rich.progress import Progress, SpinnerColumn, TextColumn

from abathur.domain.models import Task, TaskSource, TaskStatus
from abathur.infrastructure.database import Database, PruneFilters

console = Console()


async def demo_vacuum_with_progress():
    """Demonstrate VACUUM with progress indicator."""
    console.print("[bold]VACUUM Progress Indicator Demo[/bold]\n")

    # Create temporary database
    with NamedTemporaryFile(suffix=".db", delete=False) as tmp_file:
        db_path = Path(tmp_file.name)

    try:
        # Initialize database
        console.print("[blue]Step 1: Creating database and inserting 150 tasks...[/blue]")
        db = Database(db_path)
        await db.initialize()

        # Create 150 tasks to trigger VACUUM threshold
        old_timestamp = datetime.now(timezone.utc) - timedelta(days=60)
        task_ids = []

        for i in range(150):
            task = Task(
                prompt=f"Demo task {i}",
                summary=f"Summary {i}",
                agent_type="test-agent",
                source=TaskSource.HUMAN,
                status=TaskStatus.COMPLETED,
                submitted_at=old_timestamp,
                completed_at=old_timestamp,
            )
            await db.insert_task(task)
            task_ids.append(task.id)

        console.print(f"[green]✓[/green] Created {len(task_ids)} tasks\n")

        # Demo 1: VACUUM with progress indicator (conditional mode, above threshold)
        console.print(
            "[bold]Demo 1: Deleting 150 tasks with vacuum_mode='conditional'[/bold]"
        )
        console.print("[dim]Expected: Progress indicator shown (>= 100 tasks)[/dim]\n")

        with Progress(
            SpinnerColumn(),
            TextColumn("[progress.description]{task.description}"),
            console=console,
        ) as progress:
            task_desc = progress.add_task(
                description="Deleting tasks and optimizing database...", total=None
            )
            filters = PruneFilters(task_ids=task_ids[:150], vacuum_mode="conditional")
            result = await db.prune_tasks(filters)

        console.print(f"[green]✓[/green] Deleted {result.deleted_tasks} tasks")
        if result.reclaimed_bytes is not None:
            reclaimed_mb = result.reclaimed_bytes / (1024 * 1024)
            console.print(
                f"[green]✓[/green] VACUUM completed: {reclaimed_mb:.2f} MB reclaimed\n"
            )

        # Create more tasks for demo 2
        console.print("[blue]Step 2: Creating 50 more tasks for next demo...[/blue]")
        task_ids_2 = []
        for i in range(50):
            task = Task(
                prompt=f"Demo task batch 2 - {i}",
                summary=f"Summary batch 2 - {i}",
                agent_type="test-agent",
                source=TaskSource.HUMAN,
                status=TaskStatus.COMPLETED,
                submitted_at=old_timestamp,
                completed_at=old_timestamp,
            )
            await db.insert_task(task)
            task_ids_2.append(task.id)

        console.print(f"[green]✓[/green] Created {len(task_ids_2)} tasks\n")

        # Demo 2: No progress indicator (below threshold)
        console.print(
            "[bold]Demo 2: Deleting 50 tasks with vacuum_mode='conditional'[/bold]"
        )
        console.print("[dim]Expected: No progress indicator (< 100 tasks)[/dim]\n")

        filters = PruneFilters(task_ids=task_ids_2, vacuum_mode="conditional")
        result = await db.prune_tasks(filters)

        console.print(f"[green]✓[/green] Deleted {result.deleted_tasks} tasks")
        console.print(
            f"[dim]VACUUM skipped (conditional mode, only {result.deleted_tasks} tasks deleted, threshold is 100)[/dim]\n"
        )

        # Create more tasks for demo 3
        console.print("[blue]Step 3: Creating 10 more tasks for final demo...[/blue]")
        task_ids_3 = []
        for i in range(10):
            task = Task(
                prompt=f"Demo task batch 3 - {i}",
                summary=f"Summary batch 3 - {i}",
                agent_type="test-agent",
                source=TaskSource.HUMAN,
                status=TaskStatus.COMPLETED,
                submitted_at=old_timestamp,
                completed_at=old_timestamp,
            )
            await db.insert_task(task)
            task_ids_3.append(task.id)

        console.print(f"[green]✓[/green] Created {len(task_ids_3)} tasks\n")

        # Demo 3: VACUUM with progress indicator (always mode)
        console.print("[bold]Demo 3: Deleting 10 tasks with vacuum_mode='always'[/bold]")
        console.print("[dim]Expected: Progress indicator shown (always mode)[/dim]\n")

        with Progress(
            SpinnerColumn(),
            TextColumn("[progress.description]{task.description}"),
            console=console,
        ) as progress:
            task_desc = progress.add_task(
                description="Deleting tasks and optimizing database...", total=None
            )
            filters = PruneFilters(task_ids=task_ids_3, vacuum_mode="always")
            result = await db.prune_tasks(filters)

        console.print(f"[green]✓[/green] Deleted {result.deleted_tasks} tasks")
        if result.reclaimed_bytes is not None:
            reclaimed_mb = result.reclaimed_bytes / (1024 * 1024)
            console.print(
                f"[green]✓[/green] VACUUM completed: {reclaimed_mb:.2f} MB reclaimed\n"
            )

        await db.close()

        console.print("[bold green]Demo completed successfully![/bold green]")

    finally:
        # Cleanup
        if db_path.exists():
            db_path.unlink()
            console.print(f"[dim]Cleaned up temporary database: {db_path}[/dim]")


if __name__ == "__main__":
    asyncio.run(demo_vacuum_with_progress())
