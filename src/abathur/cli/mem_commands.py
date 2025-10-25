"""Memory management CLI commands."""

import asyncio
import json
from datetime import datetime, timezone
from typing import Any

import typer
from rich.console import Console
from rich.table import Table

from abathur.cli.utils import parse_duration_to_days
from abathur.domain.models import TaskStatus

console = Console()

# Create memory sub-app
mem_app = typer.Typer(help="Memory management", no_args_is_help=True)


# Import helper at module level to avoid circular imports
# The _get_services and _resolve_task_id helpers are in main.py
# We'll import them when needed inside async functions


@mem_app.command("list")
def mem_list(
    namespace: str | None = typer.Option(None, help="Filter by namespace prefix"),
    memory_type: str
    | None = typer.Option(None, help="Filter by memory type (semantic, episodic, procedural)"),
    created_by: str | None = typer.Option(None, help="Filter by creator (agent or session ID)"),
    limit: int = typer.Option(100, help="Maximum number of entries"),
) -> None:
    """List memory entries with optional filtering.

    Examples:
        abathur mem list                           # List all memories (up to 100)
        abathur mem list --namespace task:         # List all task-related memories
        abathur mem list --type semantic           # List semantic memories only
        abathur mem list --created-by requirements-gatherer  # List by creator
        abathur mem list --namespace user: --limit 50        # List first 50 user memories
    """

    async def _list() -> None:
        # Import here to avoid circular imports
        from abathur.cli.main import _get_services

        services = await _get_services()

        # Build query based on filters
        conditions = ["is_deleted = 0"]
        params: list[Any] = []

        if namespace:
            conditions.append("namespace LIKE ?")
            params.append(f"{namespace}%")

        if memory_type:
            if memory_type not in ["semantic", "episodic", "procedural"]:
                console.print(
                    f"[red]Error:[/red] Invalid memory type '{memory_type}'. Valid values: semantic, episodic, procedural"
                )
                raise typer.Exit(1)
            conditions.append("memory_type = ?")
            params.append(memory_type)

        if created_by:
            conditions.append("created_by = ?")
            params.append(created_by)

        where_clause = " AND ".join(conditions)
        query = f"""
            SELECT id, namespace, key, memory_type, created_by, updated_at, version
            FROM memory_entries
            WHERE {where_clause}
            ORDER BY updated_at DESC
            LIMIT ?
        """
        params.append(limit)

        # Execute query
        async with services["database"]._get_connection() as conn:
            cursor = await conn.execute(query, tuple(params))
            rows = await cursor.fetchall()

        # Display results
        table = Table(title=f"Memory Entries ({len(rows)})")
        table.add_column("ID", style="cyan", no_wrap=True)
        table.add_column("Namespace", style="magenta")
        table.add_column("Key", style="green")
        table.add_column("Type", style="yellow")
        table.add_column("Created By", style="blue")
        table.add_column("Updated", style="dim")

        for row in rows:
            # Truncate long values for display
            namespace_display = (
                (row["namespace"][:35] + "...") if len(row["namespace"]) > 35 else row["namespace"]
            )
            key_display = (row["key"][:20] + "...") if len(row["key"]) > 20 else row["key"]
            created_by_display = (
                (row["created_by"][:20] + "...")
                if row["created_by"] and len(row["created_by"]) > 20
                else (row["created_by"] or "-")
            )

            table.add_row(
                str(row["id"])[:8],
                namespace_display,
                key_display,
                row["memory_type"],
                created_by_display,
                datetime.fromisoformat(row["updated_at"]).strftime("%Y-%m-%d %H:%M")
                if row["updated_at"]
                else "-",
            )

        console.print(table)

        if len(rows) == limit:
            console.print(f"\n[dim]Showing first {limit} entries. Use --limit to see more.[/dim]")

    asyncio.run(_list())


@mem_app.command("show")
def mem_show(
    namespace_prefix: str = typer.Argument(..., help="Namespace prefix to filter memories")
) -> None:
    """Show all memory entries matching a namespace prefix.

    Examples:
        abathur mem show task:535a8666
        abathur mem show project:my-project
        abathur mem show user:alice
    """

    async def _show() -> None:
        # Import here to avoid circular imports
        from abathur.cli.main import _get_services, _resolve_task_id

        services = await _get_services()

        # If the prefix looks like a task ID without "task:" prefix, add it
        if namespace_prefix and ":" not in namespace_prefix:
            # Might be a bare task ID, try to resolve it
            try:
                resolved_id = await _resolve_task_id(namespace_prefix, services)
                task = await services["task_coordinator"].get_task(resolved_id)
                if task:
                    # Display task context
                    console.print(f"[bold]Task {resolved_id}[/bold]")
                    if task.summary:
                        console.print(f"Summary: [magenta]{task.summary}[/magenta]\n")
                    final_prefix = f"task:{resolved_id}"
                else:
                    final_prefix = namespace_prefix
            except Exception:
                # Not a valid task ID, use as-is
                final_prefix = namespace_prefix
        else:
            final_prefix = namespace_prefix

        console.print(f"[dim]Searching for memories with prefix: {final_prefix}[/dim]\n")

        # Query memories with the given prefix
        query = """
            SELECT id, namespace, key, value, memory_type, version, created_by, updated_by, created_at, updated_at
            FROM memory_entries
            WHERE namespace LIKE ? AND is_deleted = 0
            ORDER BY namespace, key, version
        """

        async with services["database"]._get_connection() as conn:
            cursor = await conn.execute(query, (f"{final_prefix}%",))
            rows = await cursor.fetchall()

        if not rows:
            console.print(f"[yellow]No memories found with prefix '{final_prefix}'[/yellow]")
            return

        console.print(f"[green]Found {len(rows)} memory entries[/green]\n")

        # Group by namespace and key
        current_namespace_key = None
        for row in rows:
            namespace_key = f"{row['namespace']}:{row['key']}"

            if namespace_key != current_namespace_key:
                current_namespace_key = namespace_key
                console.print(f"\n[cyan]━━ {row['namespace']} / {row['key']} ━━[/cyan]")
                console.print(f"[dim]Type: {row['memory_type']}[/dim]")

            console.print(f"\n[bold]Version {row['version']}:[/bold]")
            console.print(f"Created By: {row['created_by'] or '-'}")
            console.print(f"Updated By: {row['updated_by'] or '-'}")
            console.print(f"Created At: {row['created_at']}")
            console.print(f"Updated At: {row['updated_at']}")
            console.print(f"\n[dim]Value:[/dim]")

            # Pretty print JSON value
            try:
                value_obj = json.loads(row["value"])
                console.print(json.dumps(value_obj, indent=2))
            except json.JSONDecodeError:
                console.print(row["value"])

    asyncio.run(_show())


@mem_app.command("prune")
def mem_prune(
    namespace: str | None = typer.Option(None, help="Delete by namespace prefix"),
    memory_type: str
    | None = typer.Option(None, help="Delete by memory type (semantic, episodic, procedural)"),
    older_than: str
    | None = typer.Option(None, help="Delete entries older than duration (e.g., 30d, 2w, 6m, 1y)"),
    task_status: str
    | None = typer.Option(
        None, help="Delete memories for tasks with status (completed, failed, cancelled)"
    ),
    dry_run: bool = typer.Option(False, help="Preview what would be deleted without deleting"),
    force: bool = typer.Option(False, help="Skip confirmation prompt"),
    limit: int | None = typer.Option(None, help="Maximum entries to delete"),
) -> None:
    """Delete memory entries matching filters.

    Examples:
        abathur mem prune --namespace task: --dry-run    # Preview deletion of task memories
        abathur mem prune --namespace task: --force      # Delete all task memories
        abathur mem prune --type episodic --older-than 30d  # Delete old episodic memories
        abathur mem prune --task-status completed        # Delete memories for completed tasks
        abathur mem prune --namespace temp: --limit 100  # Delete first 100 temp memories
    """

    async def _prune() -> None:
        # Import here to avoid circular imports
        from abathur.cli.main import _get_services

        # Validate at least one filter is provided
        if not any([namespace, memory_type, older_than, task_status]):
            console.print("[red]Error:[/red] At least one filter must be specified")
            console.print("Use --namespace, --memory-type, --older-than, or --task-status")
            raise typer.Exit(1)

        # Validate memory_type if provided
        if memory_type and memory_type not in ["semantic", "episodic", "procedural"]:
            console.print(
                f"[red]Error:[/red] Invalid memory type '{memory_type}'. Valid values: semantic, episodic, procedural"
            )
            raise typer.Exit(1)

        # Validate task_status if provided
        task_status_enum = None
        if task_status:
            try:
                task_status_enum = TaskStatus(task_status)
            except ValueError:
                valid_values = ", ".join([s.value for s in TaskStatus])
                console.print(
                    f"[red]Error:[/red] Invalid status '{task_status}'. Valid values: {valid_values}"
                )
                raise typer.Exit(1)

        # Parse --older-than duration
        older_than_days = None
        if older_than:
            try:
                older_than_days = parse_duration_to_days(older_than)
            except ValueError as e:
                console.print(f"[red]Error:[/red] Invalid duration format: {older_than}")
                console.print(f"Use format <number><unit> (e.g., 30d, 2w, 6m, 1y)")
                console.print(f"Error: {e}")
                raise typer.Exit(1)

        services = await _get_services()

        # Build query to find matching memories
        conditions = ["is_deleted = 0"]
        params: list[Any] = []

        if namespace:
            conditions.append("namespace LIKE ?")
            params.append(f"{namespace}%")

        if memory_type:
            conditions.append("memory_type = ?")
            params.append(memory_type)

        if older_than_days:
            cutoff_date = datetime.now(timezone.utc) - __import__("datetime").timedelta(
                days=older_than_days
            )
            conditions.append("updated_at < ?")
            params.append(cutoff_date.isoformat())

        if task_status_enum:
            # Need to join with tasks table to filter by task status
            # Assuming namespace pattern is task:{task_id}:*
            conditions.append(
                """
                EXISTS (
                    SELECT 1 FROM tasks t
                    WHERE 'task:' || t.id || ':' = SUBSTR(memory_entries.namespace, 1, LENGTH('task:' || t.id || ':'))
                    AND t.status = ?
                )
            """
            )
            params.append(task_status_enum.value)

        where_clause = " AND ".join(conditions)
        preview_query = f"""
            SELECT id, namespace, key, memory_type, updated_at
            FROM memory_entries
            WHERE {where_clause}
            ORDER BY updated_at ASC
        """

        if limit:
            preview_query += f" LIMIT {limit}"

        # Execute preview query
        async with services["database"]._get_connection() as conn:
            cursor = await conn.execute(preview_query, tuple(params))
            rows = await cursor.fetchall()

        if not rows:
            console.print("[yellow]No memories match the specified filters[/yellow]")
            return

        # Display preview
        table = Table(title=f"Memories to Delete ({len(rows)})")
        table.add_column("ID", style="cyan", no_wrap=True)
        table.add_column("Namespace", style="magenta")
        table.add_column("Key", style="green")
        table.add_column("Type", style="yellow")
        table.add_column("Updated", style="dim")

        for row in rows:
            namespace_display = (
                (row["namespace"][:40] + "...") if len(row["namespace"]) > 40 else row["namespace"]
            )
            key_display = (row["key"][:25] + "...") if len(row["key"]) > 25 else row["key"]

            table.add_row(
                str(row["id"])[:8],
                namespace_display,
                key_display,
                row["memory_type"],
                datetime.fromisoformat(row["updated_at"]).strftime("%Y-%m-%d %H:%M")
                if row["updated_at"]
                else "-",
            )

        console.print(table)

        # Dry-run mode
        if dry_run:
            console.print("\n[blue]Dry-run mode - no changes will be made[/blue]")
            console.print(f"[dim]Would delete {len(rows)} memory entries[/dim]")
            return

        # Confirmation prompt (unless --force)
        if not force:
            console.print(
                f"\n[yellow]About to permanently delete {len(rows)} memory entries[/yellow]"
            )
            confirmed = typer.confirm("Are you sure you want to continue?")
            if not confirmed:
                console.print("[dim]Operation cancelled[/dim]")
                raise typer.Exit(0)

        # Execute deletion (soft delete)
        console.print("[blue]Deleting memory entries...[/blue]")

        memory_ids = [row["id"] for row in rows]
        placeholders = ",".join(["?"] * len(memory_ids))
        delete_query = f"""
            UPDATE memory_entries
            SET is_deleted = 1
            WHERE id IN ({placeholders})
        """

        async with services["database"]._get_connection() as conn:
            await conn.execute(delete_query, tuple(memory_ids))
            await conn.commit()

        console.print(f"[green]✓[/green] Deleted {len(rows)} memory entries")

    asyncio.run(_prune())
