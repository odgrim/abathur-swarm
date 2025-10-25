"""Abathur CLI - Hivemind Swarm Management System."""

import asyncio
import json
import logging
import sqlite3
import sys
from datetime import datetime, timezone
from pathlib import Path
from typing import Any
from uuid import UUID

import aiosqlite
import typer
from pydantic import ValidationError
from rich.console import Console
from rich.progress import Progress, SpinnerColumn, TextColumn
from rich.table import Table
from rich.text import Text

from abathur import __version__
from abathur.cli.tree_formatter import format_lineage_tree, format_tree, supports_unicode
from abathur.cli.utils import parse_duration_to_days
from abathur.domain.models import TaskStatus
from abathur.infrastructure.database import PruneFilters

logger = logging.getLogger(__name__)

# Initialize Typer app
app = typer.Typer(
    name="abathur",
    help="Hivemind Swarm Management System - Orchestrate specialized Claude agents",
    no_args_is_help=True,
)

console = Console()


# Helper to resolve UUID prefix to full UUID
async def _resolve_task_id(task_id_prefix: str, services: dict[str, Any]) -> UUID:
    """Resolve a task ID prefix to a full UUID.

    Args:
        task_id_prefix: Full UUID or prefix (e.g., 'ebec23ad')
        services: Services dictionary with task_coordinator

    Returns:
        Full UUID if exactly one match found

    Raises:
        typer.Exit: If no matches or multiple matches found
    """
    from abathur.domain.models import TaskStatus

    # Try to parse as full UUID first
    try:
        return UUID(task_id_prefix)
    except ValueError:
        pass

    # Search for prefix match across all tasks
    from abathur.domain.models import Task

    all_tasks: list[Task] = []
    for status in TaskStatus:
        tasks = await services["task_coordinator"].list_tasks(status, limit=10000)
        all_tasks.extend(tasks)

    # Find matches
    matches = [task for task in all_tasks if str(task.id).startswith(task_id_prefix.lower())]

    if len(matches) == 0:
        console.print(f"[red]Error:[/red] No task found matching prefix '{task_id_prefix}'")
        raise typer.Exit(1)
    elif len(matches) > 1:
        console.print(f"[red]Error:[/red] Multiple tasks match prefix '{task_id_prefix}':")
        for task in matches:
            console.print(f"  - {task.id} ({task.status.value})")
        console.print(
            "\n[yellow]Please provide a longer prefix to uniquely identify the task[/yellow]"
        )
        raise typer.Exit(1)

    return matches[0].id


# Helper to render tree preview for recursive deletion
def _render_tree_preview(tasks: list[Any], max_depth: int = 5) -> None:
    """Render hierarchical tree preview of tasks to be deleted.

    Args:
        tasks: List of Task objects to preview
        max_depth: Maximum depth to display in tree (currently unused, tree shows all levels)
    """
    from abathur.cli.tree_formatter import format_tree, supports_unicode

    # Build and render tree using TreeFormatter
    tree = format_tree(tasks, use_unicode=supports_unicode())

    console.print("\n[bold cyan]Tasks to Delete (Tree View)[/bold cyan]")
    console.print(tree)
    console.print(f"\n[dim]Showing {len(tasks)} tasks[/dim]")


# Helper to get database and services
async def _get_services() -> dict[str, Any]:
    """Get initialized services with API key or Claude CLI authentication."""
    from abathur.application import (
        AgentExecutor,
        ClaudeClient,
        LoopExecutor,
        MCPManager,
        ResourceMonitor,
        SwarmOrchestrator,
        TaskCoordinator,
        TemplateManager,
    )
    from abathur.infrastructure import ConfigManager, Database
    from abathur.infrastructure.api_key_auth import APIKeyAuthProvider
    from abathur.infrastructure.claude_cli_auth import ClaudeCLIAuthProvider
    from abathur.infrastructure.logger import get_logger, setup_logging
    from abathur.services import DependencyResolver, PriorityCalculator, TaskQueueService

    # Initialize config manager
    config_manager = ConfigManager()
    config = config_manager.load_config()

    # Setup logging to both console and file
    setup_logging(log_level=config.log_level, log_dir=config_manager.get_log_dir())

    logger = get_logger(__name__)

    database = Database(config_manager.get_database_path())
    await database.initialize()

    # Detect and initialize authentication
    from abathur.domain.ports.auth_provider import AuthProvider

    auth_provider: AuthProvider | None = None

    try:
        # Try API key first (environment variable precedence)
        api_key = config_manager.get_api_key()
        auth_provider = APIKeyAuthProvider(api_key)
        logger.debug("auth_initialized", method="api_key")
    except ValueError:
        # API key not found, try Claude CLI
        try:
            auth_provider = ClaudeCLIAuthProvider()
            logger.debug("auth_initialized", method="claude_cli")
        except Exception as e:
            raise ValueError(
                "No authentication configured.\n"
                "Please either:\n"
                "  1. Set API key: export ANTHROPIC_API_KEY=<key>\n"
                "  2. Install Claude CLI and authenticate: https://docs.anthropic.com/claude/docs/quickstart"
            ) from e

    # Initialize task queue services
    dependency_resolver = DependencyResolver(database)
    priority_calculator = PriorityCalculator(dependency_resolver)
    task_queue_service = TaskQueueService(database, dependency_resolver, priority_calculator)

    # Initialize task coordinator (still used by some commands)
    task_coordinator = TaskCoordinator(database)

    claude_client = ClaudeClient(auth_provider=auth_provider)
    agent_executor = AgentExecutor(database, claude_client)
    swarm_orchestrator = SwarmOrchestrator(
        task_queue_service=task_queue_service,
        agent_executor=agent_executor,
        max_concurrent_agents=config.swarm.max_concurrent_agents,
        poll_interval=2.0,
    )
    template_manager = TemplateManager()
    mcp_manager = MCPManager()
    await mcp_manager.initialize()
    resource_monitor = ResourceMonitor()
    loop_executor = LoopExecutor(task_coordinator, agent_executor, database)

    return {
        "database": database,
        "task_coordinator": task_coordinator,
        "task_queue_service": task_queue_service,
        "claude_client": claude_client,
        "agent_executor": agent_executor,
        "swarm_orchestrator": swarm_orchestrator,
        "template_manager": template_manager,
        "mcp_manager": mcp_manager,
        "resource_monitor": resource_monitor,
        "loop_executor": loop_executor,
        "config_manager": config_manager,
    }


# ===== Tree Visualization Helpers =====
# Tree visualization now handled by tree_formatter module


# ===== Import and Register Command Modules =====
# Import command modules after _get_services is defined to avoid circular imports
from abathur.cli.db_commands import db_app
from abathur.cli.mcp_commands import mcp_app
from abathur.cli.swarm_commands import swarm_app

app.add_typer(db_app, name="db")
app.add_typer(mcp_app, name="mcp")
app.add_typer(swarm_app, name="swarm")


# ===== Version =====
@app.command()
def version() -> None:
    """Show Abathur version."""
    console.print(f"[bold]Abathur[/bold] version [cyan]{__version__}[/cyan]")


# ===== Task Commands =====
task_app = typer.Typer(help="Task queue management", no_args_is_help=True)
app.add_typer(task_app, name="task")


@task_app.command("submit")
def submit(
    prompt: str = typer.Argument(..., help="Task prompt/instruction"),
    agent_type: str = typer.Option("requirements-gatherer", help="Agent type to use"),  # noqa: B008
    summary: str
    | None = typer.Option(
        None, help="Custom summary (max 140 chars, auto-generated if not provided)"
    ),  # noqa: B008
    input_file: Path
    | None = typer.Option(None, help="JSON file with additional context data"),  # noqa: B008
    input_json: str
    | None = typer.Option(None, help="JSON string with additional context data"),  # noqa: B008
    priority: int = typer.Option(5, help="Task priority (0-10)"),  # noqa: B008
) -> None:
    """Submit a new task to the queue.

    Examples:
        abathur task submit "Review the code in src/main.py"
        abathur task submit "Fix the authentication bug" --agent-type code-reviewer
        abathur task submit "Analyze performance" --input-file context.json
        abathur task submit "Generate report" --input-json '{"format": "pdf"}'
        abathur task submit "Complex task" --summary "Custom summary for this task"
    """

    async def _submit() -> UUID:
        services = await _get_services()
        from abathur.domain.models import Task, TaskSource

        # Load additional context data
        input_data = {}
        if input_file and input_file.exists():
            with open(input_file) as f:
                input_data = json.load(f)
        elif input_json:
            input_data = json.loads(input_json)

        # Auto-generate summary if not provided
        # Format: "User Prompt: " + first 126 chars of prompt
        task_summary = summary
        if task_summary is None:
            prefix = "User Prompt: "
            task_summary = prefix + prompt[:126].strip()

        task = Task(
            prompt=prompt,
            summary=task_summary,
            agent_type=agent_type,
            input_data=input_data,
            priority=priority,
            source=TaskSource.HUMAN,
        )
        task_id: UUID = await services["task_coordinator"].submit_task(task)

        console.print(f"[green]✓[/green] Task submitted: [cyan]{task_id}[/cyan]")
        console.print(f"[dim]Agent type: {agent_type}[/dim]")
        return task_id

    asyncio.run(_submit())


@task_app.command("list")
def list_tasks(
    status: str | None = typer.Option(None, help="Filter by status"),
    exclude_status: str | None = typer.Option(None, help="Exclude tasks with this status"),
    limit: int = typer.Option(100, help="Maximum number of tasks"),
    deps: bool = typer.Option(False, "--deps", help="Display as dependency tree (default: table)"),
    lineage: bool = typer.Option(False, "--lineage", help="Display as lineage tree showing task spawning relationships"),
    unicode_override: bool | None = typer.Option(
        None, "--unicode/--ascii", help="Force Unicode or ASCII box-drawing"
    ),
) -> None:
    """List tasks in the queue."""

    async def _list() -> None:
        services = await _get_services()
        from abathur.domain.models import TaskStatus

        # Validate and convert status
        task_status = None
        if status:
            try:
                task_status = TaskStatus(status)
            except ValueError:
                valid_values = ", ".join([s.value for s in TaskStatus])
                raise typer.BadParameter(
                    f"Invalid status '{status}'. Valid values: {valid_values}"
                ) from None

        # Validate and convert exclude_status
        task_exclude_status = None
        if exclude_status:
            try:
                task_exclude_status = TaskStatus(exclude_status)
            except ValueError:
                valid_values = ", ".join([s.value for s in TaskStatus])
                raise typer.BadParameter(
                    f"Invalid exclude_status '{exclude_status}'. Valid values: {valid_values}"
                ) from None

        tasks = await services["task_coordinator"].list_tasks(
            status=task_status, exclude_status=task_exclude_status, limit=limit
        )

        # Validate mutually exclusive options
        if deps and lineage:
            raise typer.BadParameter("Cannot use both --deps and --lineage at the same time")

        # Dependency tree view rendering
        if deps:
            use_unicode = unicode_override if unicode_override is not None else supports_unicode()
            tree_widget = format_tree(tasks, use_unicode=use_unicode)
            console.print(tree_widget)
            return

        # Lineage tree view rendering
        if lineage:
            from abathur.cli.tree_formatter import format_lineage_tree
            use_unicode = unicode_override if unicode_override is not None else supports_unicode()
            tree_widget = format_lineage_tree(tasks, use_unicode=use_unicode)
            console.print(tree_widget)
            return

        # Table view rendering (default)
        table = Table(title="Tasks")
        table.add_column("ID", style="cyan", no_wrap=True)
        table.add_column("Summary", style="magenta")
        table.add_column("Agent Type", style="green")
        table.add_column("Priority", justify="center")
        table.add_column("Status", style="yellow")
        table.add_column("Submitted", style="blue")

        for task in tasks:
            # Truncate summary and ID for display
            summary_preview = (
                (task.summary[:40] + "...")
                if task.summary and len(task.summary) > 40
                else (task.summary or "-")
            )
            table.add_row(
                str(task.id)[:8],
                summary_preview,
                task.agent_type,
                str(task.priority),
                task.status.value,
                task.submitted_at.strftime("%Y-%m-%d %H:%M"),
            )

        console.print(table)

    asyncio.run(_list())


@task_app.command("show")
def task_show(task_id: str = typer.Argument(..., help="Task ID or prefix")) -> None:
    """Get detailed task information."""

    async def _status() -> None:
        from datetime import datetime, timezone

        services = await _get_services()
        resolved_id = await _resolve_task_id(task_id, services)
        task = await services["task_coordinator"].get_task(resolved_id)

        if not task:
            console.print(f"[red]Error:[/red] Task {task_id} not found")
            return

        console.print(f"[bold]Task {task.id}[/bold]")
        if task.summary:
            console.print(f"Summary: [magenta]{task.summary}[/magenta]")
        console.print(f"Prompt: {task.prompt}")
        console.print(f"Agent Type: {task.agent_type}")
        console.print(f"Priority: {task.priority}")
        console.print(f"Status: {task.status.value}")
        console.print(f"Retry Count: {task.retry_count}/{task.max_retries}")
        console.print(f"Timeout: {task.max_execution_timeout_seconds}s")
        console.print(f"Submitted: {task.submitted_at}")
        if task.started_at:
            console.print(f"Started: {task.started_at}")
        if task.completed_at:
            console.print(f"Completed: {task.completed_at}")
        console.print(f"Last Updated: {task.last_updated_at}")

        # Show time since last update for running tasks
        if task.status.value == "running":
            now = datetime.now(timezone.utc)
            time_since_update = (now - task.last_updated_at).total_seconds()
            console.print(f"Time Since Update: {int(time_since_update)}s")

            # Warn if approaching timeout
            if time_since_update > task.max_execution_timeout_seconds * 0.8:
                console.print(
                    f"[yellow]⚠[/yellow]  Task approaching timeout "
                    f"({int(time_since_update)}s / {task.max_execution_timeout_seconds}s)"
                )

        if task.input_data:
            console.print("\n[dim]Additional Context:[/dim]")
            console.print(json.dumps(task.input_data, indent=2))
        if task.error_message:
            console.print(f"\n[red]Error:[/red] {task.error_message}")

        # Retrieve child tasks
        children = await services["database"].get_child_tasks([resolved_id])

        if children:
            console.print("\n[bold]Child Tasks:[/bold]")
            child_table = Table()
            child_table.add_column("ID", style="cyan", no_wrap=True)
            child_table.add_column("Summary", style="magenta")
            child_table.add_column("Status", style="yellow")

            for child in children:
                # Truncate summary to 40 chars (matches task list pattern)
                summary_preview = (
                    (child.summary[:40] + "...")
                    if child.summary and len(child.summary) > 40
                    else (child.summary or "-")
                )
                # Add row: 8-char ID prefix, truncated summary, status
                child_table.add_row(str(child.id)[:8], summary_preview, child.status.value)

            console.print(child_table)

    asyncio.run(_status())


@task_app.command("update")
def update_task(
    task_id: str = typer.Argument(..., help="Task ID or prefix"),
    status: str
    | None = typer.Option(
        None, help="New status (pending|blocked|ready|running|completed|failed|cancelled)"
    ),
    priority: int | None = typer.Option(None, help="New priority (0-10)", min=0, max=10),
    agent_type: str | None = typer.Option(None, help="New agent type"),
    dry_run: bool = typer.Option(False, help="Preview changes without applying"),
) -> None:
    """Update task attributes.

    Examples:
        abathur task update abc123 --status ready
        abathur task update abc123 --status completed --priority 10
        abathur task update abc123 --agent-type requirements-gatherer --dry-run
        abathur task update abc123 --priority 8
    """

    async def _update() -> None:
        from abathur.domain.models import TaskStatus

        services = await _get_services()
        resolved_id = await _resolve_task_id(task_id, services)

        # Validate at least one field is being updated
        if not any([status, priority is not None, agent_type]):
            console.print("[red]Error:[/red] At least one field must be specified")
            console.print("Use --status, --priority, or --agent-type")
            raise typer.Exit(1)

        # Get current task
        task = await services["task_coordinator"].get_task(resolved_id)
        if not task:
            console.print(f"[red]Error:[/red] Task {task_id} not found")
            raise typer.Exit(1)

        # Validate status if provided
        new_status = None
        if status:
            try:
                new_status = TaskStatus(status)
            except ValueError:
                valid_values = ", ".join([s.value for s in TaskStatus])
                console.print(f"[red]Error:[/red] Invalid status '{status}'")
                console.print(f"Valid values: {valid_values}")
                raise typer.Exit(1)

        # Validate agent type change (only for PENDING/READY tasks)
        # Use new_status if being updated, otherwise use current status
        effective_status = new_status if new_status else task.status
        if agent_type and effective_status not in [TaskStatus.PENDING, TaskStatus.READY]:
            console.print(
                f"[red]Error:[/red] Cannot update agent type for task in {effective_status.value} status"
            )
            console.print("Agent type can only be changed for PENDING or READY tasks")
            raise typer.Exit(1)

        # Display preview
        console.print(f"\n[bold]Task {resolved_id}[/bold]")
        console.print(f"Summary: {task.summary or 'No summary'}\n")

        table = Table(title="Proposed Changes")
        table.add_column("Field", style="cyan")
        table.add_column("Current", style="yellow")
        table.add_column("New", style="green")

        updated_fields = []

        if new_status:
            table.add_row("Status", task.status.value, new_status.value)
            updated_fields.append("status")

        if priority is not None:
            table.add_row("Priority", str(task.priority), str(priority))
            updated_fields.append("priority")

        if agent_type:
            table.add_row("Agent Type", task.agent_type, agent_type)
            updated_fields.append("agent_type")

        console.print(table)

        if dry_run:
            console.print("\n[blue]Dry-run mode - no changes will be made[/blue]")
            console.print(
                f"[dim]Would update {len(updated_fields)} field(s): {', '.join(updated_fields)}[/dim]"
            )
            return

        # Apply updates
        try:
            from datetime import datetime, timezone

            if new_status:
                await services["task_coordinator"].update_task_status(resolved_id, new_status)

            if priority is not None:
                # Update priority directly in database
                async with services["database"]._get_connection() as conn:
                    now = datetime.now(timezone.utc).isoformat()
                    await conn.execute(
                        "UPDATE tasks SET priority = ?, last_updated_at = ? WHERE id = ?",
                        (priority, now, str(resolved_id)),
                    )
                    await conn.commit()
                # Log audit
                await services["database"].log_audit(
                    task_id=resolved_id,
                    action_type="task_priority_updated",
                    action_data={"old_priority": task.priority, "new_priority": priority},
                    result="success",
                )

            if agent_type:
                # Update agent type directly in database
                async with services["database"]._get_connection() as conn:
                    now = datetime.now(timezone.utc).isoformat()
                    await conn.execute(
                        "UPDATE tasks SET agent_type = ?, last_updated_at = ? WHERE id = ?",
                        (agent_type, now, str(resolved_id)),
                    )
                    await conn.commit()
                # Log audit
                await services["database"].log_audit(
                    task_id=resolved_id,
                    action_type="task_agent_type_updated",
                    action_data={"old_agent_type": task.agent_type, "new_agent_type": agent_type},
                    result="success",
                )

            console.print(f"\n[green]✓[/green] Task {task_id} updated successfully")
            console.print(
                f"[dim]Updated {len(updated_fields)} field(s): {', '.join(updated_fields)}[/dim]"
            )

        except Exception as e:
            console.print(f"\n[red]Error:[/red] Failed to update task")
            console.print(f"[dim]{e}[/dim]")
            raise typer.Exit(1)

    asyncio.run(_update())


@task_app.command("prune")
def prune(
    task_ids: list[str] = typer.Argument(None, help="Task IDs or prefixes to delete"),
    status: str
    | None = typer.Option(
        None,
        "--status",
        help="Delete all tasks with this status (pending|blocked|ready|running|completed|failed|cancelled)",
    ),
    older_than: str
    | None = typer.Option(
        None, "--older-than", help="Delete tasks older than duration (e.g., 30d, 2w, 6m, 1y)"
    ),
    before: str
    | None = typer.Option(None, "--before", help="Delete tasks before date (ISO 8601: YYYY-MM-DD)"),
    limit: int | None = typer.Option(None, "--limit", help="Maximum tasks to delete", min=1),
    force: bool = typer.Option(False, "--force", help="Skip confirmation prompt"),
    dry_run: bool = typer.Option(
        False, "--dry-run", help="Show what would be deleted without deleting"
    ),
    vacuum: str = typer.Option(
        "conditional",
        "--vacuum",
        help="VACUUM strategy: 'always' (may be slow), 'conditional' (auto, default), or 'never' (fastest)",
    ),
    recursive: bool = typer.Option(
        False,
        "--recursive",
        "-r",
        help="Recursively delete task and all descendants. Validates entire descendant tree "
        "matches deletion criteria before deleting. Use --dry-run to preview what will be deleted.",
    ),
    preview_depth: int = typer.Option(
        5,
        "--preview-depth",
        min=1,
        max=50,
        help="Maximum depth to display in tree preview when using --recursive (default: 5). "
        "Deeper levels show '...' indicator.",
    ),
) -> None:
    """Delete tasks by ID or status.

    By default, deletes only tasks directly matching the criteria.
    Use --recursive to delete entire task trees.

    Examples:
        # Delete single task (non-recursive)
        abathur task prune ebec23ad

        # Delete multiple tasks
        abathur task prune ebec23ad-1234-5678-90ab-cdef12345678 fbec23ad-5678-1234-90ab-cdef12345678

        # Delete by status (non-recursive)
        abathur task prune --status completed
        abathur task prune --status failed --force
        abathur task prune --status pending --dry-run

        # Delete by time
        abathur task prune --older-than 30d
        abathur task prune --older-than 30d --vacuum=always
        abathur task prune --older-than 30d --vacuum=never

        # Recursive deletion (entire task tree)
        abathur task prune --task-id ebec23ad --recursive
        abathur task prune --status completed --recursive

        # Preview recursive deletion
        abathur task prune --task-id ebec23ad --recursive --dry-run

        # Custom preview depth
        abathur task prune --status completed --recursive --preview-depth 10
    """
    from abathur.domain.models import TaskStatus

    # Parameter validation (fail fast - before async)
    # Mutual exclusion: task_ids XOR time-based filters XOR status
    filter_count = sum([bool(task_ids), bool(older_than or before), bool(status)])

    if filter_count == 0:
        raise typer.BadParameter(
            "Must specify at least one filter method.\n"
            "Options:\n"
            "  - Task IDs: abathur task prune <task-id-1> <task-id-2>\n"
            "  - Time-based: abathur task prune --older-than 30d\n"
            "  - Status: abathur task prune --status completed"
        )

    if filter_count > 1:
        filters_used = []
        if task_ids:
            filters_used.append("task IDs")
        if older_than or before:
            filters_used.append("time-based filters (--older-than or --before)")
        if status:
            filters_used.append("--status")

        raise typer.BadParameter(
            f"Cannot use multiple filter methods together: {', '.join(filters_used)}.\n"
            "Choose one filter method:\n"
            "  - Task IDs only\n"
            "  - Time-based filters only (--older-than or --before)\n"
            "  - Status only (--status)"
        )

    # Validate incompatible option combinations
    if recursive and limit:
        raise typer.BadParameter(
            "Cannot use --recursive with --limit.\n"
            "Recursive deletion operates on entire task trees, making limit semantics unclear.\n"
            "Remove --limit to proceed with recursive deletion."
        )

    # Validate status enum value
    task_status = None
    if status:
        try:
            task_status = TaskStatus(status)
        except ValueError:
            valid_values = ", ".join([s.value for s in TaskStatus])
            raise typer.BadParameter(
                f"Invalid status '{status}'. Valid values: {valid_values}"
            ) from None

    # Parse --older-than duration
    older_than_days = None
    if older_than:
        try:
            older_than_days = parse_duration_to_days(older_than)
        except ValueError as e:
            raise typer.BadParameter(
                f"Invalid duration format: {older_than}. "
                f"Use format <number><unit> (e.g., 30d, 2w, 6m, 1y). "
                f"Error: {e}"
            ) from None

    # Parse --before date
    before_date = None
    if before:
        try:
            before_date = datetime.fromisoformat(before)
            if before_date.tzinfo is None:
                before_date = before_date.replace(tzinfo=timezone.utc)
        except ValueError as e:
            raise typer.BadParameter(
                f"Invalid date format: {before}. "
                f"Use ISO 8601 format (YYYY-MM-DD or YYYY-MM-DDTHH:MM:SS). "
                f"Examples: 2025-01-01, 2025-01-01T12:00:00. "
                f"Error: {e}"
            ) from None

    async def _prune() -> None:
        services = await _get_services()

        # Routing decision: time filters -> prune_tasks(), else -> delete_tasks()
        # This enables advanced time-based filtering while preserving backward compatibility
        has_time_filters = older_than is not None or before is not None

        if has_time_filters:
            # Phase 2: PruneFilters construction and child validation
            # Construct PruneFilters from parsed CLI parameters
            try:
                # Construct with explicit parameters to satisfy type checker
                if task_status is not None:
                    # Status specified - use single-status list
                    filters = PruneFilters(
                        older_than_days=older_than_days,
                        before_date=before_date,
                        statuses=[task_status],
                        limit=limit,
                        dry_run=dry_run,
                        vacuum_mode=vacuum,
                        recursive=recursive,
                    )
                else:
                    # No status specified - use default (COMPLETED, FAILED, CANCELLED)
                    filters = PruneFilters(
                        older_than_days=older_than_days,
                        before_date=before_date,
                        limit=limit,
                        dry_run=dry_run,
                        vacuum_mode=vacuum,
                        recursive=recursive,
                    )
            except ValidationError as e:
                raise typer.BadParameter(f"Invalid filter parameters: {e}") from None

            # CLI-007: Task ID preview query for child validation
            # Uses shared PruneFilters.build_where_clause() method to ensure
            # preview query matches prune_tasks() deletion query exactly

            # Build WHERE clause from PruneFilters (use shared method)
            where_sql, params = filters.build_where_clause()

            # Build complete preview query
            limit_sql = f" LIMIT {filters.limit}" if filters.limit else ""
            preview_query = f"""
                SELECT id FROM tasks
                WHERE {where_sql}
                ORDER BY submitted_at ASC
                {limit_sql}
            """

            # Execute preview query to get task IDs
            async with services["database"]._get_connection() as conn:
                cursor = await conn.execute(preview_query, tuple(params))
                rows = await cursor.fetchall()
                preview_task_ids = [UUID(row["id"]) for row in rows]

            # Early return if no tasks match
            if not preview_task_ids:
                console.print("[yellow]No tasks match the specified filters.[/yellow]")
                return

            # Phase 3: prune_tasks() execution and result display

            # Component 1: Child Task Validation (~30 lines)
            child_tasks = await services["database"].get_child_tasks(preview_task_ids)

            if child_tasks:
                console.print(
                    f"\n[yellow]![/yellow] Cannot delete {len(preview_task_ids)} task(s) - "
                    f"{len(child_tasks)} have child tasks:"
                )

                blocked_table = Table()
                blocked_table.add_column("Parent ID", style="cyan", no_wrap=True)
                blocked_table.add_column("Child ID", style="yellow", no_wrap=True)
                blocked_table.add_column("Child Summary", style="magenta")

                for child in child_tasks:
                    parent_id_str = (
                        str(child.parent_task_id)[:8] if child.parent_task_id else "unknown"
                    )
                    child_id_str = str(child.id)[:8]
                    summary_preview = (
                        (child.summary[:40] + "...")
                        if child.summary and len(child.summary) > 40
                        else (child.summary or "-")
                    )
                    blocked_table.add_row(
                        parent_id_str,
                        child_id_str,
                        summary_preview,
                    )

                console.print(blocked_table)
                console.print(
                    "\n[yellow]Delete child tasks first before deleting parent tasks.[/yellow]"
                )
                return

            # Component 2: Preview Display (~25 lines)
            # Fetch full Task objects for preview
            tasks_to_delete = []
            for task_id in preview_task_ids:
                task = await services["task_coordinator"].get_task(task_id)
                if task:
                    tasks_to_delete.append(task)

            # Display preview table
            preview_table = Table(title=f"Tasks to Delete ({len(tasks_to_delete)})")
            preview_table.add_column("ID", style="cyan", no_wrap=True)
            preview_table.add_column("Summary", style="magenta")
            preview_table.add_column("Status", style="yellow")
            preview_table.add_column("Agent Type", style="green")

            for task in tasks_to_delete:
                summary_preview = (
                    (task.summary[:40] + "...")
                    if task.summary and len(task.summary) > 40
                    else (task.summary or "-")
                )
                preview_table.add_row(
                    str(task.id)[:8],
                    summary_preview,
                    task.status.value,
                    task.agent_type,
                )

            console.print(preview_table)

            # Component 3: Dry-Run Check (~5 lines)
            if dry_run:
                # Show tree preview if recursive mode
                if recursive:
                    _render_tree_preview(tasks_to_delete, max_depth=preview_depth)

                console.print("\n[blue]Dry-run mode - no changes will be made[/blue]")
                if recursive:
                    console.print(
                        f"[dim]Would delete {len(tasks_to_delete)} task(s) in recursive mode[/dim]"
                    )
                else:
                    console.print(f"[dim]Would delete {len(tasks_to_delete)} task(s)[/dim]")
                return

            # Component 4: Confirmation Prompt (~10 lines)
            if not force:
                console.print(
                    f"\n[yellow]About to permanently delete {len(tasks_to_delete)} task(s)[/yellow]"
                )
                confirmed = typer.confirm("Are you sure you want to continue?")
                if not confirmed:
                    console.print("[dim]Operation cancelled[/dim]")
                    raise typer.Exit(0)

            # Component 5: Prune Execution (~10 lines)
            console.print("[blue]Deleting tasks...[/blue]")

            # Show progress indicator for VACUUM if expected to run
            show_vacuum_progress = filters.vacuum_mode == "always" or (
                filters.vacuum_mode == "conditional" and len(preview_task_ids) >= 100
            )

            try:
                if show_vacuum_progress:
                    # Use progress indicator for operations that will VACUUM
                    with Progress(
                        SpinnerColumn(),
                        TextColumn("[progress.description]{task.description}"),
                        console=console,
                    ) as progress:
                        task_desc = progress.add_task(
                            description="Deleting tasks and optimizing database...", total=None
                        )
                        result = await services["database"].prune_tasks(filters)
                else:
                    # No VACUUM expected, run without progress indicator
                    result = await services["database"].prune_tasks(filters)
            except sqlite3.OperationalError as e:
                # Database locked, busy, or permission issues
                console.print(
                    "[red]Error:[/red] Database is locked or busy.\n"
                    "This can happen if another process is using the database.\n"
                    "Try again in a few moments."
                )
                logger.error(f"Database operational error: {e}")
                raise typer.Exit(1)
            except sqlite3.IntegrityError as e:
                # Foreign key violations, constraint failures
                console.print(
                    "[red]Error:[/red] Database integrity constraint violated.\n"
                    "This may indicate data corruption or concurrent modifications.\n"
                    f"Details: {e}"
                )
                logger.error(f"Database integrity error: {e}")
                raise typer.Exit(1)
            except aiosqlite.Error as e:
                # General aiosqlite errors (connection, protocol, etc.)
                console.print(
                    "[red]Error:[/red] Database connection or protocol error.\n"
                    f"Details: {e}\n"
                    "Check database file permissions and disk space."
                )
                logger.error(f"Aiosqlite error: {e}")
                raise typer.Exit(1)
            except ValueError as e:
                # Validation errors from our code
                console.print(
                    f"[red]Error:[/red] Invalid parameters: {e}\n"
                    "Check your command arguments and try again."
                )
                logger.error(f"Validation error: {e}")
                raise typer.Exit(1)
            except Exception as e:
                # Unexpected errors - still catch for safety
                console.print(
                    f"[red]Error:[/red] Unexpected error during task deletion.\n"
                    f"Type: {type(e).__name__}\n"
                    f"Details: {e}\n"
                    "Please report this issue if it persists."
                )
                logger.exception("Unexpected error in prune command")
                raise typer.Exit(1)

            # Component 6: PruneResult Display (~25 lines)
            # Display result summary
            if recursive:
                console.print(
                    f"\n[green]✓[/green] Successfully deleted {result.deleted_tasks} task(s) in recursive mode"
                )
            else:
                console.print(
                    f"\n[green]✓[/green] Successfully deleted {result.deleted_tasks} task(s)"
                )

            # Display breakdown by status
            if result.breakdown_by_status:
                breakdown_table = Table(title="Breakdown by Status")
                breakdown_table.add_column("Status", style="cyan")
                breakdown_table.add_column("Count", style="yellow", justify="right")

                for status, count in result.breakdown_by_status.items():
                    breakdown_table.add_row(status.value, str(count))

                console.print(breakdown_table)

            # Display VACUUM information
            if result.vacuum_auto_skipped:
                # Auto-skipped for large prune operation
                console.print(
                    f"\n[yellow]⚠[/yellow]  VACUUM automatically skipped (deleting {result.deleted_tasks} tasks)"
                )
                console.print(
                    "[dim]Large prune operations (>10,000 tasks) skip VACUUM to avoid long database locks.[/dim]"
                )
                console.print(
                    "[dim]Run 'abathur task prune --older-than 0d --vacuum=always' to manually VACUUM if needed.[/dim]"
                )
            elif result.reclaimed_bytes is not None:
                reclaimed_mb = result.reclaimed_bytes / (1024 * 1024)
                console.print(f"\n[green]VACUUM completed: {reclaimed_mb:.2f} MB reclaimed[/green]")
            elif filters.vacuum_mode == "never":
                console.print("\n[dim]VACUUM skipped (--vacuum=never)[/dim]")
            elif filters.vacuum_mode == "conditional" and result.deleted_tasks < 100:
                console.print(
                    f"\n[dim]VACUUM skipped (conditional mode, only {result.deleted_tasks} tasks deleted, threshold is 100)[/dim]"
                )

            # Display dependency count
            if result.deleted_dependencies:
                console.print(
                    f"[cyan]Deleted {result.deleted_dependencies} task dependencies[/cyan]"
                )

            return

        # Unified prune path - uses prune_tasks() for all selection strategies
        # Task selection logic
        selected_task_ids: list[UUID] = []

        if task_ids:
            # Resolve task ID prefixes
            for task_id_prefix in task_ids:
                resolved_id = await _resolve_task_id(task_id_prefix, services)
                selected_task_ids.append(resolved_id)
        elif task_status:
            # Filter by status
            # Use the CLI limit if specified, otherwise default to 10000
            task_limit = limit if limit is not None else 10000
            tasks = await services["database"].list_tasks(task_status, limit=task_limit)
            selected_task_ids = [task.id for task in tasks]

            if not selected_task_ids:
                console.print(f"[green]✓[/green] No tasks found with status '{task_status.value}'")
                return

        # Apply limit to selected task IDs if specified (for task-ID based deletion)
        if limit is not None and len(selected_task_ids) > limit:
            selected_task_ids = selected_task_ids[:limit]

        # Fetch full task details for display
        tasks_to_delete = []
        for task_id in selected_task_ids:
            task = await services["task_coordinator"].get_task(task_id)
            if task:
                tasks_to_delete.append(task)
            else:
                # Task ID was resolved but doesn't exist in database
                console.print(f"[red]Error:[/red] Task {task_id} not found")
                raise typer.Exit(1)

        if not tasks_to_delete:
            console.print("[green]✓[/green] No tasks to delete")
            return

        # Child Task Validation - check if any selected tasks have children
        child_tasks = await services["database"].get_child_tasks(selected_task_ids)

        if child_tasks:
            console.print(
                f"\n[yellow]![/yellow] Cannot delete {len(selected_task_ids)} task(s) - "
                f"{len(child_tasks)} have child tasks:"
            )

            blocked_table = Table()
            blocked_table.add_column("Parent ID", style="cyan", no_wrap=True)
            blocked_table.add_column("Child ID", style="yellow", no_wrap=True)
            blocked_table.add_column("Child Summary", style="magenta")

            for child in child_tasks:
                parent_id_str = str(child.parent_task_id)[:8] if child.parent_task_id else "unknown"
                child_id_str = str(child.id)[:8]
                summary_preview = (
                    (child.summary[:40] + "...")
                    if child.summary and len(child.summary) > 40
                    else (child.summary or "-")
                )
                blocked_table.add_row(
                    parent_id_str,
                    child_id_str,
                    summary_preview,
                )

            console.print(blocked_table)
            console.print(
                "\n[yellow]Delete child tasks first before deleting parent tasks.[/yellow]"
            )
            return

        # Display preview table
        table = Table(title=f"Tasks to Delete ({len(tasks_to_delete)})")
        table.add_column("ID", style="cyan", no_wrap=True)
        table.add_column("Summary", style="magenta")
        table.add_column("Status", style="yellow")
        table.add_column("Agent Type", style="green")

        for task in tasks_to_delete:
            summary_preview = (
                (task.summary[:40] + "...")
                if task.summary and len(task.summary) > 40
                else (task.summary or "-")
            )
            table.add_row(
                str(task.id)[:8],
                summary_preview,
                task.status.value,
                task.agent_type,
            )

        console.print(table)

        # Dry-run mode
        if dry_run:
            # Show tree preview if recursive mode
            if recursive:
                _render_tree_preview(tasks_to_delete, max_depth=preview_depth)

            console.print("\n[blue]Dry-run mode - no changes will be made[/blue]")
            if recursive:
                console.print(
                    f"[dim]Would delete {len(tasks_to_delete)} task(s) in recursive mode[/dim]"
                )
            else:
                console.print(f"[dim]Would delete {len(tasks_to_delete)} task(s)[/dim]")
            return

        # Confirmation prompt (unless --force)
        if not force:
            console.print(
                f"\n[yellow]About to permanently delete {len(tasks_to_delete)} task(s)[/yellow]"
            )
            confirmed = typer.confirm("Are you sure you want to continue?")
            if not confirmed:
                console.print("[dim]Operation cancelled[/dim]")
                raise typer.Exit(0)

        # Execute deletion using unified prune_tasks() interface
        console.print("[blue]Deleting tasks...[/blue]")

        # Show progress indicator for VACUUM if expected to run
        # Note: Don't show for large operations (>10,000) since VACUUM will be auto-skipped
        show_vacuum_progress = len(selected_task_ids) < 10_000 and (  # Auto-skip threshold
            vacuum == "always" or (vacuum == "conditional" and len(selected_task_ids) >= 100)
        )

        try:
            filters = PruneFilters(
                task_ids=selected_task_ids, vacuum_mode=vacuum, recursive=recursive
            )

            if show_vacuum_progress:
                # Use progress indicator for operations that will VACUUM
                with Progress(
                    SpinnerColumn(),
                    TextColumn("[progress.description]{task.description}"),
                    console=console,
                ) as progress:
                    task_desc = progress.add_task(
                        description="Deleting tasks and optimizing database...", total=None
                    )
                    result = await services["database"].prune_tasks(filters)
            else:
                # No VACUUM expected, run without progress indicator
                result = await services["database"].prune_tasks(filters)

            deleted_count = result.deleted_tasks
        except sqlite3.OperationalError as e:
            console.print(
                "[red]Error:[/red] Database is locked or busy.\n"
                "This can happen if another process is using the database.\n"
                "Try again in a few moments."
            )
            logger.error(f"Database operational error: {e}")
            raise typer.Exit(1)
        except sqlite3.IntegrityError as e:
            console.print(
                "[red]Error:[/red] Database integrity constraint violated.\n"
                "This may indicate data corruption or concurrent modifications.\n"
                f"Details: {e}"
            )
            logger.error(f"Database integrity error: {e}")
            raise typer.Exit(1)
        except aiosqlite.Error as e:
            console.print(
                "[red]Error:[/red] Database connection or protocol error.\n"
                f"Details: {e}\n"
                "Check database file permissions and disk space."
            )
            logger.error(f"Aiosqlite error: {e}")
            raise typer.Exit(1)
        except ValueError as e:
            console.print(
                f"[red]Error:[/red] Invalid parameters: {e}\n"
                "Check your command arguments and try again."
            )
            logger.error(f"Validation error: {e}")
            raise typer.Exit(1)
        except Exception as e:
            console.print(
                f"[red]Error:[/red] Unexpected error during task deletion.\n"
                f"Type: {type(e).__name__}\n"
                f"Details: {e}\n"
                "Please report this issue if it persists."
            )
            logger.exception("Unexpected error in delete command")
            raise typer.Exit(1)

        # Display results
        if recursive:
            console.print(f"[green]✓[/green] Deleted {deleted_count} task(s) in recursive mode")
        else:
            console.print(f"[green]✓[/green] Deleted {deleted_count} task(s)")

        # Show breakdown if available
        if result.breakdown_by_status:
            breakdown_table = Table(title="Breakdown by Status")
            breakdown_table.add_column("Status", style="cyan")
            breakdown_table.add_column("Count", style="yellow", justify="right")

            for status, count in result.breakdown_by_status.items():
                breakdown_table.add_row(status.value, str(count))

            console.print(breakdown_table)

        # Display VACUUM auto-skip warning if applicable
        if result.vacuum_auto_skipped:
            console.print(
                f"\n[yellow]⚠[/yellow]  VACUUM automatically skipped (deleting {result.deleted_tasks} tasks)"
            )
            console.print(
                "[dim]Large prune operations (>10,000 tasks) skip VACUUM to avoid long database locks.[/dim]"
            )
            console.print(
                "[dim]Run 'VACUUM;' manually in SQLite CLI if you need to reclaim disk space.[/dim]"
            )

        # Display VACUUM information
        if result.reclaimed_bytes is not None:
            reclaimed_mb = result.reclaimed_bytes / (1024 * 1024)
            console.print(f"\n[green]VACUUM completed: {reclaimed_mb:.2f} MB reclaimed[/green]")
        elif vacuum == "never":
            console.print("\n[dim]VACUUM skipped (--vacuum=never)[/dim]")
        elif vacuum == "conditional" and result.deleted_tasks < 100:
            console.print(
                f"\n[dim]VACUUM skipped (conditional mode, only {result.deleted_tasks} tasks deleted, threshold is 100)[/dim]"
            )

    asyncio.run(_prune())


@task_app.command("check-stale")
def check_stale() -> None:
    """Check for and handle stale running tasks that have exceeded their timeout."""

    async def _check_stale() -> None:
        services = await _get_services()

        console.print("[blue]Checking for stale running tasks...[/blue]")
        handled_task_ids = await services["task_coordinator"].handle_stale_tasks()

        if not handled_task_ids:
            console.print("[green]✓[/green] No stale tasks found")
        else:
            console.print(f"[yellow]![/yellow] Handled {len(handled_task_ids)} stale task(s):")
            for task_id in handled_task_ids:
                console.print(f"  - {task_id}")

    asyncio.run(_check_stale())


@task_app.command("status")
def task_status(watch: bool = typer.Option(False, help="Watch mode (live updates)")) -> None:
    """Show task queue status and statistics."""

    async def _status() -> None:
        services = await _get_services()
        from abathur.domain.models import TaskStatus

        # Count tasks by status
        pending = len(await services["database"].list_tasks(TaskStatus.PENDING, 1000))
        running = len(await services["database"].list_tasks(TaskStatus.RUNNING, 1000))
        completed = len(await services["database"].list_tasks(TaskStatus.COMPLETED, 1000))
        failed = len(await services["database"].list_tasks(TaskStatus.FAILED, 1000))

        console.print("[bold]Task Queue Status[/bold]")
        console.print(f"Pending tasks: {pending}")
        console.print(f"Running tasks: {running}")
        console.print(f"Completed tasks: {completed}")
        console.print(f"Failed tasks: {failed}")
        console.print(f"Total tasks: {pending + running + completed + failed}")

    asyncio.run(_status())


@task_app.command("visualize")
def visualize_queue() -> None:
    """[DEPRECATED] The TUI has been removed. Use 'abathur task list --tree' instead.

    The interactive TUI has been replaced with a simpler tree view in the list command.

    Examples:
        abathur task list --tree                    # Show tasks as a tree
        abathur task list --tree --status pending   # Show pending tasks as a tree
        abathur feature-branch summary <branch>     # View feature branch progress
    """
    console.print("[yellow]The TUI has been deprecated and removed.[/yellow]")
    console.print("\nUse the following commands instead:")
    console.print("  [cyan]abathur task list --tree[/cyan]                    # Show tasks as a tree")
    console.print("  [cyan]abathur task list --tree --status pending[/cyan]   # Show pending tasks as a tree")
    console.print("  [cyan]abathur feature-branch summary <branch>[/cyan]     # View feature branch progress")
    raise typer.Exit(0)


# ===== Loop Commands =====
loop_app = typer.Typer(help="Iterative loop execution", no_args_is_help=True)
app.add_typer(loop_app, name="loop")


@loop_app.command("start")
def loop_start(
    task_id: str = typer.Argument(..., help="Task ID or prefix"),
    max_iterations: int = typer.Option(10, help="Maximum iterations"),
    convergence_threshold: float = typer.Option(0.95, help="Convergence threshold"),
    no_mcp: bool = typer.Option(False, help="Disable auto-start of MCP memory server"),
) -> None:
    """Start an iterative refinement loop.

    Automatically starts the MCP memory server for agent memory access.
    Use --no-mcp to disable auto-start of the memory server.
    """

    async def _start() -> None:
        from abathur.application import ConvergenceCriteria, ConvergenceType
        from abathur.mcp.server_manager import MemoryServerManager

        services = await _get_services()

        resolved_id = await _resolve_task_id(task_id, services)
        task = await services["task_coordinator"].get_task(resolved_id)
        if not task:
            console.print(f"[red]Error:[/red] Task {task_id} not found")
            return

        criteria = ConvergenceCriteria(
            type=ConvergenceType.THRESHOLD,
            threshold=convergence_threshold,
        )

        console.print(f"[blue]Starting loop execution for task {task_id}...[/blue]")

        # Auto-start MCP memory server
        mcp_manager = None
        if not no_mcp:
            console.print("[dim]Starting MCP memory server...[/dim]")
            mcp_manager = MemoryServerManager(services["config_manager"].get_database_path())
            await mcp_manager.start()
            console.print("[dim]✓ MCP memory server running[/dim]")

        try:
            result = await services["loop_executor"].execute_loop(task, criteria, max_iterations)

            if result.converged:
                console.print(f"[green]✓[/green] Converged after {result.iterations} iterations")
            else:
                console.print(
                    f"[yellow]![/yellow] Did not converge ({result.reason}) after {result.iterations} iterations"
                )
        finally:
            # Stop MCP memory server
            if mcp_manager:
                console.print("[dim]Stopping MCP memory server...[/dim]")
                await mcp_manager.stop()

    asyncio.run(_start())


# ===== Memory Commands =====
mem_app = typer.Typer(help="Memory management", no_args_is_help=True)
app.add_typer(mem_app, name="mem")


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
    from abathur.domain.models import TaskStatus

    async def _prune() -> None:
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


# ===== Main Entry Point =====
def main() -> None:
    """Main entry point."""
    try:
        app()
    except KeyboardInterrupt:
        console.print("\n[yellow]Interrupted[/yellow]")
        sys.exit(130)
    except Exception as e:
        console.print(f"[red]Error:[/red] {e}")
        sys.exit(1)


if __name__ == "__main__":
    main()
