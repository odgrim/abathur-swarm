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
from rich.tree import Tree

from abathur import __version__
from abathur.cli.utils import parse_duration_to_days
from abathur.domain.models import TaskStatus
from abathur.infrastructure.database import PruneFilters
from abathur.tui.models import TreeNode

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
        max_depth: Maximum depth to display in tree
    """
    from abathur.tui.rendering.tree_renderer import TreeRenderer

    # Build task hierarchy
    task_map = {task.id: task for task in tasks}
    root_tasks = [task for task in tasks if task.parent_task_id is None]

    # Create tree renderer
    renderer = TreeRenderer()

    # Compute layout
    dependency_graph: dict[UUID, list[UUID]] = {}  # Empty for now, used by renderer
    layout = renderer.compute_layout(tasks, dependency_graph)

    # Render tree with depth limit
    tree = renderer.render_tree(layout, use_unicode=TreeRenderer.supports_unicode())

    console.print("\n[bold cyan]Tasks to Delete (Tree View)[/bold cyan]")
    console.print(tree)
    console.print(f"\n[dim]Showing {len(tasks)} tasks (max depth: {max_depth})[/dim]")


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
def _get_status_color(status: TaskStatus) -> str:
    """Get Rich color name for task status.

    Uses semantic colors for accessibility:
    - Green tones: Success, completion, ready
    - Red tones: Failure, errors
    - Yellow/Orange: Warnings, blocked
    - Blue/Cyan: Information, pending
    - Magenta: Active, running
    - Dim/Gray: Cancelled, inactive

    Args:
        status: TaskStatus enum value

    Returns:
        Rich color name string (e.g., "green", "red", "blue")
    """
    color_map = {
        TaskStatus.PENDING: "blue",
        TaskStatus.BLOCKED: "yellow",
        TaskStatus.READY: "cyan",
        TaskStatus.RUNNING: "magenta",
        TaskStatus.COMPLETED: "green",
        TaskStatus.FAILED: "red",
        TaskStatus.CANCELLED: "dim",
    }
    return color_map.get(status, "white")


def _get_status_symbol(status: TaskStatus) -> str:
    """Get Unicode symbol for task status.

    Provides visual indicator in addition to color for accessibility.
    Supports users with color blindness or limited color terminals.

    Args:
        status: TaskStatus enum value

    Returns:
        Unicode status symbol character
    """
    symbol_map = {
        TaskStatus.PENDING: "○",      # Empty circle
        TaskStatus.BLOCKED: "⊗",      # Circled X
        TaskStatus.READY: "◎",        # Double circle
        TaskStatus.RUNNING: "◉",      # Filled circle
        TaskStatus.COMPLETED: "✓",    # Check mark
        TaskStatus.FAILED: "✗",       # X mark
        TaskStatus.CANCELLED: "⊘",    # Circle with slash
    }
    return symbol_map.get(status, "•")


def _render_tree_preview(
    tree_nodes: list[TreeNode],
    max_depth: int = 5,
    console: Console | None = None,
) -> None:
    """Render hierarchical tree structure using Rich Tree widget.

    Displays task hierarchy with color-coded status, truncating at max_depth
    to prevent overwhelming output for large trees.

    Args:
        tree_nodes: Nodes to visualize (from _discover_task_tree)
        max_depth: Maximum tree depth to display (default 5, shows "..." beyond)
        console: Rich console for output (uses global console if None)

    Example:
        >>> nodes = await database._discover_task_tree(conn, [root_id])
        >>> _render_tree_preview(nodes, max_depth=3, console=console)

    Output format:
        Task Tree (3 tasks)
        ├── ✓ Implement feature X (completed)
        │   ├── ✓ Write unit tests (completed)
        │   └── ✓ Update documentation (completed)
        └── ○ Follow-up task Y (pending)
    """
    if console is None:
        # Use global console instance
        console = globals()["console"]

    if not tree_nodes:
        console.print("[yellow]No tasks to display[/yellow]")
        return

    # Build node map and identify roots
    node_map = {node.task_id: node for node in tree_nodes}
    root_nodes = [
        node for node in tree_nodes
        if node.task.parent_task_id is None or node.task.parent_task_id not in node_map
    ]

    # Create root tree with title
    root_tree = Tree(
        Text(f"Task Tree ({len(tree_nodes)} tasks)", style="bold cyan"),
        guide_style="dim"  # Dim connecting lines
    )

    def add_subtree(
        parent_widget: Tree,
        node: TreeNode,
        current_depth: int
    ) -> None:
        """Recursively add nodes to tree widget with depth truncation."""

        # Check depth limit
        if current_depth >= max_depth:
            parent_widget.add(
                Text("... (more items)", style="dim italic")
            )
            return

        # Create node label with color coding and status symbol
        label = _format_node_label(node)

        # Add node to parent
        subtree = parent_widget.add(label)

        # Get and sort children by priority (highest first)
        children = [
            node_map[child_id]
            for child_id in node.children
            if child_id in node_map
        ]
        children.sort(
            key=lambda n: n.task.calculated_priority,
            reverse=True
        )

        # Add children recursively
        for child in children:
            add_subtree(subtree, child, current_depth + 1)

    # Build tree from root nodes
    for root_node in root_nodes:
        add_subtree(root_tree, root_node, 0)

    # Render to console
    console.print(root_tree)


def _format_node_label(node: TreeNode) -> Text:
    """Format tree node with color-coded status and summary.

    Returns Rich Text object with styling based on task status.

    Args:
        node: TreeNode to format

    Returns:
        Rich Text with status symbol, summary, and metadata
    """
    # Get status color and symbol
    status_color = _get_status_color(node.task.status)
    status_symbol = _get_status_symbol(node.task.status)

    # Truncate summary to 40 chars
    summary = node.task.summary[:40] if node.task.summary else "No summary"
    if node.task.summary and len(node.task.summary) > 40:
        summary += "..."

    # Get task ID prefix (first 8 chars)
    task_id_prefix = str(node.task.id)[:8]

    # Create styled text
    text = Text()

    # Add status symbol
    text.append(f"{status_symbol} ", style=status_color)

    # Add summary
    text.append(summary, style=status_color)

    # Add status label in parentheses
    text.append(f" ({node.task.status.value})", style="dim")

    # Add task ID prefix
    text.append(f" [{task_id_prefix}]", style="dim cyan")

    return text


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
    limit: int = typer.Option(100, help="Maximum number of tasks"),
) -> None:
    """List tasks in the queue."""

    async def _list() -> None:
        services = await _get_services()
        from abathur.domain.models import TaskStatus

        task_status = TaskStatus(status) if status else None
        tasks = await services["task_coordinator"].list_tasks(task_status, limit)

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
                child_table.add_row(
                    str(child.id)[:8],
                    summary_preview,
                    child.status.value
                )

            console.print(child_table)

    asyncio.run(_status())


@task_app.command("cancel")
def cancel(
    task_id: str = typer.Argument(..., help="Task ID or prefix"),
    force: bool = typer.Option(False, help="Force cancel running tasks"),
) -> None:
    """Cancel a pending/running task.

    Use --force to cancel running tasks. Without --force, only pending tasks can be cancelled.
    """

    async def _cancel() -> None:
        services = await _get_services()
        resolved_id = await _resolve_task_id(task_id, services)

        success = await services["task_coordinator"].cancel_task(resolved_id, force=force)

        if success:
            console.print(f"[green]✓[/green] Task {task_id} cancelled")
        else:
            if not force:
                console.print(
                    f"[red]Error:[/red] Failed to cancel task {task_id}. "
                    "Use --force to cancel running tasks."
                )
            else:
                console.print(f"[red]Error:[/red] Failed to cancel task {task_id}")

    asyncio.run(_cancel())


@task_app.command("retry")
def retry(task_id: str = typer.Argument(..., help="Task ID or prefix")) -> None:
    """Retry a failed or cancelled task."""

    async def _retry() -> None:
        services = await _get_services()
        resolved_id = await _resolve_task_id(task_id, services)
        success = await services["task_coordinator"].retry_task(resolved_id)

        if success:
            console.print(f"[green]✓[/green] Task {task_id} queued for retry")
        else:
            console.print(f"[red]Error:[/red] Failed to retry task {task_id}")

    asyncio.run(_retry())


@task_app.command("prune")
def prune(
    task_ids: list[str] = typer.Argument(None, help="Task IDs or prefixes to delete"),
    status: str | None = typer.Option(None, "--status", help="Delete all tasks with this status (pending|blocked|ready|running|completed|failed|cancelled)"),
    older_than: str | None = typer.Option(None, "--older-than", help="Delete tasks older than duration (e.g., 30d, 2w, 6m, 1y)"),
    before: str | None = typer.Option(None, "--before", help="Delete tasks before date (ISO 8601: YYYY-MM-DD)"),
    limit: int | None = typer.Option(None, "--limit", help="Maximum tasks to delete", min=1),
    force: bool = typer.Option(False, "--force", help="Skip confirmation prompt"),
    dry_run: bool = typer.Option(False, "--dry-run", help="Show what would be deleted without deleting"),
    vacuum: str = typer.Option(
        "conditional",
        "--vacuum",
        help="VACUUM strategy: 'always' (may be slow), 'conditional' (auto, default), or 'never' (fastest)"
    ),
    recursive: bool = typer.Option(
        False,
        "--recursive",
        "-r",
        help="Recursively delete task and all descendants. Validates entire descendant tree "
             "matches deletion criteria before deleting. Use --dry-run to preview what will be deleted."
    ),
    preview_depth: int = typer.Option(
        5,
        "--preview-depth",
        min=1,
        max=50,
        help="Maximum depth to display in tree preview when using --recursive (default: 5). "
             "Deeper levels show '...' indicator."
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
    filter_count = sum([
        bool(task_ids),
        bool(older_than or before),
        bool(status)
    ])

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
                        recursive=recursive
                    )
                else:
                    # No status specified - use default (COMPLETED, FAILED, CANCELLED)
                    filters = PruneFilters(
                        older_than_days=older_than_days,
                        before_date=before_date,
                        limit=limit,
                        dry_run=dry_run,
                        vacuum_mode=vacuum,
                        recursive=recursive
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
                console.print("\n[yellow]Delete child tasks first before deleting parent tasks.[/yellow]")
                return

            # Component 2: Preview Display (~25 lines)
            # Fetch full Task objects for preview
            tasks_to_delete = []
            for task_id in preview_task_ids:
                task = await services['task_coordinator'].get_task(task_id)
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
                    console.print(f"[dim]Would delete {len(tasks_to_delete)} task(s) in recursive mode[/dim]")
                else:
                    console.print(f"[dim]Would delete {len(tasks_to_delete)} task(s)[/dim]")
                return

            # Component 4: Confirmation Prompt (~10 lines)
            if not force:
                console.print(f"\n[yellow]About to permanently delete {len(tasks_to_delete)} task(s)[/yellow]")
                confirmed = typer.confirm("Are you sure you want to continue?")
                if not confirmed:
                    console.print("[dim]Operation cancelled[/dim]")
                    raise typer.Exit(0)

            # Component 5: Prune Execution (~10 lines)
            console.print("[blue]Deleting tasks...[/blue]")

            # Show progress indicator for VACUUM if expected to run
            show_vacuum_progress = (
                filters.vacuum_mode == "always" or
                (filters.vacuum_mode == "conditional" and len(preview_task_ids) >= 100)
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
                            description="Deleting tasks and optimizing database...",
                            total=None
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
                console.print(f"\n[green]✓[/green] Successfully deleted {result.deleted_tasks} task(s) in recursive mode")
            else:
                console.print(f"\n[green]✓[/green] Successfully deleted {result.deleted_tasks} task(s)")

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
                console.print(f"\n[yellow]⚠[/yellow]  VACUUM automatically skipped (deleting {result.deleted_tasks} tasks)")
                console.print("[dim]Large prune operations (>10,000 tasks) skip VACUUM to avoid long database locks.[/dim]")
                console.print("[dim]Run 'abathur task prune --older-than 0d --vacuum=always' to manually VACUUM if needed.[/dim]")
            elif result.reclaimed_bytes is not None:
                reclaimed_mb = result.reclaimed_bytes / (1024 * 1024)
                console.print(f"\n[green]VACUUM completed: {reclaimed_mb:.2f} MB reclaimed[/green]")
            elif filters.vacuum_mode == "never":
                console.print("\n[dim]VACUUM skipped (--vacuum=never)[/dim]")
            elif filters.vacuum_mode == "conditional" and result.deleted_tasks < 100:
                console.print(f"\n[dim]VACUUM skipped (conditional mode, only {result.deleted_tasks} tasks deleted, threshold is 100)[/dim]")

            # Display dependency count
            if result.deleted_dependencies:
                console.print(f"[cyan]Deleted {result.deleted_dependencies} task dependencies[/cyan]")

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
            console.print("\n[yellow]Delete child tasks first before deleting parent tasks.[/yellow]")
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
                console.print(f"[dim]Would delete {len(tasks_to_delete)} task(s) in recursive mode[/dim]")
            else:
                console.print(f"[dim]Would delete {len(tasks_to_delete)} task(s)[/dim]")
            return

        # Confirmation prompt (unless --force)
        if not force:
            console.print(f"\n[yellow]About to permanently delete {len(tasks_to_delete)} task(s)[/yellow]")
            confirmed = typer.confirm("Are you sure you want to continue?")
            if not confirmed:
                console.print("[dim]Operation cancelled[/dim]")
                raise typer.Exit(0)

        # Execute deletion using unified prune_tasks() interface
        console.print("[blue]Deleting tasks...[/blue]")

        # Show progress indicator for VACUUM if expected to run
        # Note: Don't show for large operations (>10,000) since VACUUM will be auto-skipped
        show_vacuum_progress = (
            len(selected_task_ids) < 10_000 and  # Auto-skip threshold
            (vacuum == "always" or (vacuum == "conditional" and len(selected_task_ids) >= 100))
        )

        try:
            filters = PruneFilters(
                task_ids=selected_task_ids,
                vacuum_mode=vacuum,
                recursive=recursive
            )

            if show_vacuum_progress:
                # Use progress indicator for operations that will VACUUM
                with Progress(
                    SpinnerColumn(),
                    TextColumn("[progress.description]{task.description}"),
                    console=console,
                ) as progress:
                    task_desc = progress.add_task(
                        description="Deleting tasks and optimizing database...",
                        total=None
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
            console.print(
                f"[green]✓[/green] Deleted {deleted_count} task(s) in recursive mode"
            )
        else:
            console.print(
                f"[green]✓[/green] Deleted {deleted_count} task(s)"
            )

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
            console.print(f"\n[yellow]⚠[/yellow]  VACUUM automatically skipped (deleting {result.deleted_tasks} tasks)")
            console.print("[dim]Large prune operations (>10,000 tasks) skip VACUUM to avoid long database locks.[/dim]")
            console.print("[dim]Run 'VACUUM;' manually in SQLite CLI if you need to reclaim disk space.[/dim]")

        # Display VACUUM information
        if result.reclaimed_bytes is not None:
            reclaimed_mb = result.reclaimed_bytes / (1024 * 1024)
            console.print(f"\n[green]VACUUM completed: {reclaimed_mb:.2f} MB reclaimed[/green]")
        elif vacuum == "never":
            console.print("\n[dim]VACUUM skipped (--vacuum=never)[/dim]")
        elif vacuum == "conditional" and result.deleted_tasks < 100:
            console.print(f"\n[dim]VACUUM skipped (conditional mode, only {result.deleted_tasks} tasks deleted, threshold is 100)[/dim]")

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
def visualize_queue(
    refresh_interval: float = typer.Option(2.0, "--refresh-interval", help="Auto-refresh interval in seconds"),
    no_auto_refresh: bool = typer.Option(False, "--no-auto-refresh", help="Disable auto-refresh"),
    view_mode: str = typer.Option("tree", "--view-mode", help="Initial view mode (tree, dependency, timeline, feature-branch, flat-list)"),
    no_unicode: bool = typer.Option(False, "--no-unicode", help="Use ASCII instead of Unicode box-drawing"),
) -> None:
    """Launch the Abathur Task Graph TUI.

    Examples:
        abathur task visualize                              # Launch with defaults (tree view, auto-refresh)
        abathur task visualize --no-auto-refresh            # Launch without auto-refresh
        abathur task visualize --view-mode dependency       # Start with dependency view
        abathur task visualize --refresh-interval 5.0       # Refresh every 5 seconds
        abathur task visualize --no-unicode                 # Use ASCII box-drawing
    """

    async def _visualize() -> None:
        try:
            from abathur.tui.app import TaskQueueTUI
            from abathur.tui.services.task_data_service import TaskDataService
        except ImportError as e:
            console.print(f"[red]Error:[/red] TUI components not yet implemented")
            console.print(f"[dim]Missing: {e}[/dim]")
            console.print("[yellow]The TUI is still under development. Use 'abathur task list' for now.[/yellow]")
            raise typer.Exit(1)

        try:
            # Initialize services using existing helper
            services = await _get_services()

            # Create TaskDataService
            task_data_service = TaskDataService(
                database=services["database"],
                task_queue_service=services["task_queue_service"],
                dependency_resolver=services["task_queue_service"].dependency_resolver,
            )

            # Create TUI app
            # Convert view_mode string to ViewMode enum
            from abathur.tui.models import ViewMode
            view_mode_enum = ViewMode(view_mode)

            # Handle refresh_interval: use default if auto-refresh enabled
            actual_refresh_interval = 2.0  # Default
            if no_auto_refresh:
                actual_refresh_interval = 0.0  # Disable
            elif refresh_interval is not None:
                actual_refresh_interval = refresh_interval

            tui_app = TaskQueueTUI(
                task_data_service=task_data_service,
                refresh_interval=actual_refresh_interval,
                initial_view_mode=view_mode_enum,
                use_unicode=not no_unicode,
            )

            # Run TUI
            await tui_app.run_async()

        except Exception as e:
            console.print(f"[red]Error:[/red] Failed to launch TUI: {e}")
            logger.exception("TUI launch failed")
            raise typer.Exit(1)

    try:
        asyncio.run(_visualize())
    except KeyboardInterrupt:
        console.print("\n[yellow]TUI closed[/yellow]")


# ===== Swarm Commands =====
swarm_app = typer.Typer(help="Agent swarm management", no_args_is_help=True)
app.add_typer(swarm_app, name="swarm")


@swarm_app.command("start")
def start_swarm(
    task_limit: int | None = typer.Option(None, help="Max tasks to process before stopping"),
    max_agents: int = typer.Option(10, help="Max concurrent agents"),
    no_mcp: bool = typer.Option(False, help="Disable auto-start of MCP memory server"),
    poll_interval: float = typer.Option(2.0, help="Polling interval in seconds"),
) -> None:
    """Start the swarm orchestrator in continuous mode.

    The swarm continuously polls the database for READY tasks and spawns agents
    up to the max_concurrent_agents limit. It runs until interrupted with Ctrl+C
    or until task_limit is reached (if specified).

    Automatically starts the MCP memory server for agent memory access.
    Use --no-mcp to disable auto-start of the memory server.

    Examples:
        abathur swarm start                         # Run continuously until Ctrl+C
        abathur swarm start --task-limit 5          # Stop after processing 5 tasks
        abathur swarm start --poll-interval 5.0     # Poll every 5 seconds
    """

    async def _start() -> None:
        import signal as sig

        from abathur.mcp.server_manager import MemoryServerManager

        services = await _get_services()

        # Update max_concurrent_agents if specified via CLI
        if max_agents != 10:  # 10 is the default value
            services["swarm_orchestrator"].max_concurrent_agents = max_agents
            # Also update the semaphore to match new limit
            import asyncio
            services["swarm_orchestrator"].semaphore = asyncio.Semaphore(max_agents)

        # Update poll interval if specified
        if poll_interval != 2.0:
            services["swarm_orchestrator"].poll_interval = poll_interval

        console.print("[blue]Starting swarm orchestrator in continuous mode...[/blue]")
        console.print("[dim]Press Ctrl+C to stop gracefully[/dim]")

        # Auto-start MCP memory server
        mcp_manager = None
        if not no_mcp:
            console.print("[dim]Starting MCP memory server...[/dim]")
            mcp_manager = MemoryServerManager(services["config_manager"].get_database_path())
            await mcp_manager.start()
            console.print("[dim]✓ MCP memory server running[/dim]")

        # Setup signal handlers for graceful shutdown
        shutdown_event = asyncio.Event()

        def signal_handler(signum: int, frame: Any) -> None:
            console.print("\n[yellow]Shutdown signal received, stopping gracefully...[/yellow]")
            shutdown_event.set()

        sig.signal(sig.SIGINT, signal_handler)
        sig.signal(sig.SIGTERM, signal_handler)

        # Start monitoring
        await services["resource_monitor"].start_monitoring()

        try:
            # Start swarm in a task so we can monitor shutdown signal
            swarm_task = asyncio.create_task(services["swarm_orchestrator"].start_swarm(task_limit))

            # Wait for either completion or shutdown signal
            done, pending = await asyncio.wait(
                [swarm_task, asyncio.create_task(shutdown_event.wait())],
                return_when=asyncio.FIRST_COMPLETED,
            )

            # If shutdown was signaled, cancel swarm and wait for graceful stop
            if shutdown_event.is_set():
                console.print("[dim]Initiating graceful shutdown...[/dim]")
                await services["swarm_orchestrator"].shutdown()
                # Wait for swarm task to complete
                try:
                    results = await asyncio.wait_for(swarm_task, timeout=30.0)
                except asyncio.TimeoutError:
                    console.print("[yellow]Warning: Swarm shutdown timed out[/yellow]")
                    swarm_task.cancel()
                    results = []
            else:
                # Swarm completed naturally
                results = await swarm_task

            console.print(f"[green]✓[/green] Swarm completed {len(results)} tasks")

        finally:
            # Stop monitoring
            await services["resource_monitor"].stop_monitoring()

            # Stop MCP memory server
            if mcp_manager:
                console.print("[dim]Stopping MCP memory server...[/dim]")
                await mcp_manager.stop()

    try:
        asyncio.run(_start())
    except KeyboardInterrupt:
        console.print("\n[yellow]Interrupted[/yellow]")
        pass


@swarm_app.command("status")
def swarm_status() -> None:
    """Get swarm status."""

    async def _status() -> None:
        services = await _get_services()
        status = await services["swarm_orchestrator"].get_swarm_status()

        console.print("[bold]Swarm Status[/bold]")
        console.print(f"Active tasks: {status.get('active_tasks', 0)}")
        console.print(f"Completed tasks: {status.get('completed_tasks', 0)}")
        console.print(f"Failed tasks: {status.get('failed_tasks', 0)}")

    asyncio.run(_status())


# ===== MCP Commands =====
mcp_app = typer.Typer(help="MCP server management", no_args_is_help=True)
app.add_typer(mcp_app, name="mcp")


@mcp_app.command("list")
def mcp_list() -> None:
    """List all MCP servers (including built-in memory server)."""

    async def _list() -> None:
        from abathur.infrastructure import ConfigManager
        from abathur.mcp.server_manager import MemoryServerManager

        services = await _get_services()
        config_manager = ConfigManager()

        table = Table(title="MCP Servers")
        table.add_column("Name", style="cyan")
        table.add_column("Command", style="green")
        table.add_column("State", style="yellow")
        table.add_column("PID", justify="center")

        # Add memory server
        memory_manager = MemoryServerManager(config_manager.get_database_path())
        memory_status = memory_manager.get_status()
        is_running = await memory_manager.is_running()
        table.add_row(
            "memory",
            "abathur-mcp (built-in)",
            "[green]running[/green]" if is_running else "[dim]stopped[/dim]",
            str(memory_status.get("pid", "N/A")),
        )

        # Add configured servers
        all_status = services["mcp_manager"].get_all_server_status()
        for name, server_status in all_status.items():
            table.add_row(
                name,
                server_status.get("command", ""),
                server_status.get("state", "unknown"),
                str(server_status.get("pid", "N/A")),
            )

        console.print(table)

    asyncio.run(_list())


@mcp_app.command("start")
def mcp_start(
    server: str = typer.Argument(..., help="Server name (e.g., 'memory' or configured server)"),
    foreground: bool = typer.Option(False, help="Run in foreground (memory server only)"),
) -> None:
    """Start an MCP server.

    Examples:
        abathur mcp start memory          # Start the built-in memory server
        abathur mcp start filesystem      # Start a configured MCP server
        abathur mcp start memory --foreground  # Run memory server in foreground
    """

    async def _start() -> None:
        # Special handling for built-in memory server
        if server == "memory":
            from abathur.infrastructure import ConfigManager
            from abathur.mcp.server_manager import MemoryServerManager

            config_manager = ConfigManager()
            db_path = config_manager.get_database_path()

            if foreground:
                # Run in foreground (blocking)
                console.print("[blue]Starting Memory MCP server in foreground...[/blue]")
                console.print(f"[dim]Database: {db_path}[/dim]")
                console.print("[dim]Press Ctrl+C to stop[/dim]\n")

                from abathur.mcp.memory_server import AbathurMemoryServer

                memory_server = AbathurMemoryServer(db_path)
                await memory_server.run()
            else:
                # Run in background
                manager = MemoryServerManager(db_path)
                success = await manager.start()

                if success:
                    console.print("[green]✓[/green] MCP server [cyan]memory[/cyan] started")
                    console.print(f"[dim]Database: {db_path}[/dim]")
                    console.print(
                        f"[dim]PID: {manager.process.pid if manager.process else 'N/A'}[/dim]"
                    )
                    console.print("\n[dim]Configure in Claude Desktop:[/dim]")
                    console.print(
                        f'[dim]  "abathur-memory": {{"command": "abathur-mcp", "args": ["--db-path", "{db_path}"]}}[/dim]'
                    )
                else:
                    console.print("[red]Error:[/red] Failed to start memory server")
        else:
            # Use generic MCPManager for configured servers
            services = await _get_services()
            success = await services["mcp_manager"].start_server(server)

            if success:
                console.print(f"[green]✓[/green] MCP server [cyan]{server}[/cyan] started")
            else:
                console.print(f"[red]Error:[/red] Failed to start MCP server {server}")

    try:
        asyncio.run(_start())
    except KeyboardInterrupt:
        console.print("\n[yellow]Server stopped[/yellow]")


@mcp_app.command("stop")
def mcp_stop(
    server: str = typer.Argument(..., help="Server name (e.g., 'memory' or configured server)"),
) -> None:
    """Stop an MCP server.

    Examples:
        abathur mcp stop memory      # Stop the built-in memory server
        abathur mcp stop filesystem  # Stop a configured MCP server
    """

    async def _stop() -> None:
        # Special handling for built-in memory server
        if server == "memory":
            from abathur.infrastructure import ConfigManager
            from abathur.mcp.server_manager import MemoryServerManager

            config_manager = ConfigManager()
            manager = MemoryServerManager(config_manager.get_database_path())

            success = await manager.stop()

            if success:
                console.print("[green]✓[/green] MCP server [cyan]memory[/cyan] stopped")
            else:
                console.print("[red]Error:[/red] Failed to stop memory server")
        else:
            # Use generic MCPManager for configured servers
            services = await _get_services()
            success = await services["mcp_manager"].stop_server(server)

            if success:
                console.print(f"[green]✓[/green] MCP server [cyan]{server}[/cyan] stopped")
            else:
                console.print(f"[red]Error:[/red] Failed to stop MCP server {server}")

    asyncio.run(_stop())


@mcp_app.command("restart")
def mcp_restart(
    server: str = typer.Argument(..., help="Server name (e.g., 'memory' or configured server)"),
) -> None:
    """Restart an MCP server.

    Examples:
        abathur mcp restart memory      # Restart the built-in memory server
        abathur mcp restart filesystem  # Restart a configured MCP server
    """

    async def _restart() -> None:
        # Special handling for built-in memory server
        if server == "memory":
            from abathur.infrastructure import ConfigManager
            from abathur.mcp.server_manager import MemoryServerManager

            config_manager = ConfigManager()
            manager = MemoryServerManager(config_manager.get_database_path())

            # Stop first
            await manager.stop()
            await asyncio.sleep(1.0)  # Brief pause

            # Then start
            success = await manager.start()

            if success:
                console.print("[green]✓[/green] MCP server [cyan]memory[/cyan] restarted")
            else:
                console.print("[red]Error:[/red] Failed to restart memory server")
        else:
            # Use generic MCPManager for configured servers
            services = await _get_services()
            success = await services["mcp_manager"].restart_server(server)

            if success:
                console.print(f"[green]✓[/green] MCP server [cyan]{server}[/cyan] restarted")
            else:
                console.print(f"[red]Error:[/red] Failed to restart MCP server {server}")

    asyncio.run(_restart())


@mcp_app.command("status")
def mcp_status(
    server: str | None = typer.Argument(None, help="Server name (omit to show all servers)"),
) -> None:
    """Show MCP server status.

    Examples:
        abathur mcp status           # Show status of all servers
        abathur mcp status memory    # Show status of memory server only
        abathur mcp status filesystem # Show status of a configured server
    """

    async def _status() -> None:
        # If a specific server is requested
        if server:
            # Special handling for built-in memory server
            if server == "memory":
                from abathur.infrastructure import ConfigManager
                from abathur.mcp.server_manager import MemoryServerManager

                config_manager = ConfigManager()
                manager = MemoryServerManager(config_manager.get_database_path())

                status_info = manager.get_status()
                is_running = await manager.is_running()

                console.print("[bold]Memory MCP Server Status[/bold]")
                console.print(f"Running: {'[green]Yes[/green]' if is_running else '[red]No[/red]'}")
                if status_info["pid"]:
                    console.print(f"PID: {status_info['pid']}")
                console.print(f"Database: {status_info['db_path']}")
            else:
                # Show status for a configured server
                services = await _get_services()
                status_info = services["mcp_manager"].get_server_status(server)

                if not status_info:
                    console.print(f"[red]Error:[/red] Server {server} not found")
                    return

                console.print(f"[bold]MCP Server Status: {server}[/bold]")
                console.print(f"Command: {status_info.get('command', 'N/A')}")
                console.print(f"State: {status_info.get('state', 'unknown')}")
                if status_info.get("pid"):
                    console.print(f"PID: {status_info['pid']}")
                if status_info.get("started_at"):
                    console.print(f"Started: {status_info['started_at']}")
                if status_info.get("error_message"):
                    console.print(f"[red]Error:[/red] {status_info['error_message']}")
        else:
            # Show all servers (including memory)
            from abathur.infrastructure import ConfigManager
            from abathur.mcp.server_manager import MemoryServerManager

            services = await _get_services()
            config_manager = ConfigManager()

            table = Table(title="MCP Servers")
            table.add_column("Name", style="cyan")
            table.add_column("Command", style="green")
            table.add_column("State", style="yellow")
            table.add_column("PID", justify="center")

            # Add memory server
            memory_manager = MemoryServerManager(config_manager.get_database_path())
            memory_status = memory_manager.get_status()
            is_running = await memory_manager.is_running()
            table.add_row(
                "memory",
                "abathur-mcp (built-in)",
                "[green]running[/green]" if is_running else "[dim]stopped[/dim]",
                str(memory_status.get("pid", "N/A")),
            )

            # Add configured servers
            all_status = services["mcp_manager"].get_all_server_status()
            for name, status_info in all_status.items():
                table.add_row(
                    name,
                    status_info.get("command", ""),
                    status_info.get("state", "unknown"),
                    str(status_info.get("pid", "N/A")),
                )

            console.print(table)

    asyncio.run(_status())


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
    memory_type: str | None = typer.Option(None, help="Filter by memory type (semantic, episodic, procedural)"),
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
                console.print(f"[red]Error:[/red] Invalid memory type '{memory_type}'. Valid values: semantic, episodic, procedural")
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
                (row["namespace"][:35] + "...")
                if len(row["namespace"]) > 35
                else row["namespace"]
            )
            key_display = (
                (row["key"][:20] + "...")
                if len(row["key"]) > 20
                else row["key"]
            )
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
                datetime.fromisoformat(row["updated_at"]).strftime("%Y-%m-%d %H:%M") if row["updated_at"] else "-",
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
    memory_type: str | None = typer.Option(None, help="Delete by memory type (semantic, episodic, procedural)"),
    older_than: str | None = typer.Option(None, help="Delete entries older than duration (e.g., 30d, 2w, 6m, 1y)"),
    task_status: str | None = typer.Option(None, help="Delete memories for tasks with status (completed, failed, cancelled)"),
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
            console.print(f"[red]Error:[/red] Invalid memory type '{memory_type}'. Valid values: semantic, episodic, procedural")
            raise typer.Exit(1)

        # Validate task_status if provided
        task_status_enum = None
        if task_status:
            try:
                task_status_enum = TaskStatus(task_status)
            except ValueError:
                valid_values = ", ".join([s.value for s in TaskStatus])
                console.print(f"[red]Error:[/red] Invalid status '{task_status}'. Valid values: {valid_values}")
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
            cutoff_date = datetime.now(timezone.utc) - __import__('datetime').timedelta(days=older_than_days)
            conditions.append("updated_at < ?")
            params.append(cutoff_date.isoformat())

        if task_status_enum:
            # Need to join with tasks table to filter by task status
            # Assuming namespace pattern is task:{task_id}:*
            conditions.append("""
                EXISTS (
                    SELECT 1 FROM tasks t
                    WHERE 'task:' || t.id || ':' = SUBSTR(memory_entries.namespace, 1, LENGTH('task:' || t.id || ':'))
                    AND t.status = ?
                )
            """)
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
                (row["namespace"][:40] + "...")
                if len(row["namespace"]) > 40
                else row["namespace"]
            )
            key_display = (
                (row["key"][:25] + "...")
                if len(row["key"]) > 25
                else row["key"]
            )

            table.add_row(
                str(row["id"])[:8],
                namespace_display,
                key_display,
                row["memory_type"],
                datetime.fromisoformat(row["updated_at"]).strftime("%Y-%m-%d %H:%M") if row["updated_at"] else "-",
            )

        console.print(table)

        # Dry-run mode
        if dry_run:
            console.print("\n[blue]Dry-run mode - no changes will be made[/blue]")
            console.print(f"[dim]Would delete {len(rows)} memory entries[/dim]")
            return

        # Confirmation prompt (unless --force)
        if not force:
            console.print(f"\n[yellow]About to permanently delete {len(rows)} memory entries[/yellow]")
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


# ===== Database Commands =====
@app.command()
def init(
    validate: bool = typer.Option(
        False, help="Run comprehensive database validation"
    ),  # noqa: B008
    db_path: Path
    | None = typer.Option(  # noqa: B008
        None, help="Custom database path (default: ~/.abathur/abathur.db)"
    ),
    report_output: Path
    | None = typer.Option(None, help="Save validation report as JSON"),  # noqa: B008
    skip_template: bool = typer.Option(False, help="Skip template installation"),  # noqa: B008
) -> None:
    """Initialize or update an Abathur project with latest templates.

    By default, pulls templates from the config file and installs/updates them
    to your project directory (.claude/ and related files).

    Template Update Behavior:
    - Core agent templates are always updated to latest version
    - MCP config is always updated
    - Custom agents (not in template) are preserved
    - Documentation files are updated

    Templates are configured in .abathur/config.yaml under the 'template_repos' field.
    Multiple templates can be specified, and they will be installed in order.

    Use --skip-template to only initialize the database without updating templates.
    Use --validate to run a comprehensive validation suite after initialization.
    This checks PRAGMA settings, foreign keys, indexes, and performance.

    Use --db-path to initialize a database at a custom location.
    Use --report-output to save the validation report as JSON (requires --validate).

    Examples:
        abathur init                                    # Init DB + update templates
        abathur init --skip-template                    # Only init database
        abathur init --validate
        abathur init --validate --report-output validation.json
        abathur init --db-path /tmp/test.db --validate
    """

    async def _init() -> None:
        import time

        from abathur.infrastructure import ConfigManager, Database, DatabaseValidator

        console.print("[blue]Initializing Abathur project...[/blue]")

        # Determine database path
        if db_path:
            database_path = db_path
            console.print(f"[dim]Using custom database path: {database_path}[/dim]")
        else:
            config_manager = ConfigManager()
            database_path = config_manager.get_database_path()

        # Initialize database
        database = Database(database_path)

        start_time = time.perf_counter()
        await database.initialize()
        init_duration = time.perf_counter() - start_time

        console.print(f"[green]✓[/green] Database initialized ({init_duration:.2f}s)")

        # Run validation if requested
        if validate:
            console.print("\n[blue]Running database validation...[/blue]")
            validator = DatabaseValidator(database)
            results = await validator.run_all_checks(verbose=True)

            # Add initialization metadata
            results["database_path"] = str(database_path)
            results["initialization_duration_seconds"] = round(init_duration, 2)

            # Save report if requested
            if report_output:
                report_output.parent.mkdir(parents=True, exist_ok=True)
                with open(report_output, "w") as f:
                    json.dump(results, f, indent=2)
                console.print(f"\n[green]✓[/green] Validation report saved to: {report_output}")

            if results["issues"]:
                console.print("\n[red]✗[/red] Validation failed - see issues above")
                raise typer.Exit(1)
            else:
                console.print("\n[green]✓[/green] Validation passed - database ready for use")
        elif report_output:
            console.print("[yellow]Warning:[/yellow] --report-output requires --validate flag")

        # Install templates (unless skipped)
        if not skip_template:
            # Load config to get template repos
            config_manager = ConfigManager()
            config = config_manager.load_config()

            if not config.template_repos:
                console.print("[yellow]Warning:[/yellow] No templates configured in config file")
                return

            console.print(f"\n[blue]Installing {len(config.template_repos)} template(s)...[/blue]")

            services = await _get_services()
            total_agents = 0

            for idx, template_repo in enumerate(config.template_repos, start=1):
                console.print(f"\n[dim]Template {idx}/{len(config.template_repos)}[/dim]")
                console.print(f"[dim]Repository: {template_repo.url}[/dim]")
                console.print(f"[dim]Version: {template_repo.version}[/dim]")

                with Progress(
                    SpinnerColumn(),
                    TextColumn("[progress.description]{task.description}"),
                    console=console,
                ) as progress:
                    progress.add_task(description="Pulling template into cache...", total=None)

                    # Pull template into cache
                    tmpl = await services["template_manager"].clone_template(
                        template_repo.url, template_repo.version
                    )

                console.print(f"[green]✓[/green] Template cached: [cyan]{tmpl.name}[/cyan]")

                with Progress(
                    SpinnerColumn(),
                    TextColumn("[progress.description]{task.description}"),
                    console=console,
                ) as progress:
                    progress.add_task(
                        description="Installing/updating template in project directory...",
                        total=None,
                    )

                    # Install template to project directory
                    await services["template_manager"].install_template(tmpl)

                console.print("[green]✓[/green] Template installed/updated in project directory")

                # Count agents
                if tmpl.agents:
                    total_agents += len(tmpl.agents)
                    console.print(f"[dim]  - {len(tmpl.agents)} agent(s) from this template[/dim]")

            console.print("\n[green]✓[/green] All templates installed successfully")
            console.print("[dim]  - Core agent templates updated from templates[/dim]")
            console.print("[dim]  - MCP config updated[/dim]")
            console.print("[dim]  - Custom agents preserved (if any)[/dim]")
            if total_agents > 0:
                console.print(
                    f"[dim]  - Total {total_agents} template agent(s) in .claude/agents/[/dim]"
                )

    asyncio.run(_init())


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
