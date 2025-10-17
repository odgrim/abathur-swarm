"""Abathur CLI - Hivemind Swarm Management System."""

import asyncio
import json
import sys
from pathlib import Path
from typing import Any
from uuid import UUID

import typer
from rich.console import Console
from rich.progress import Progress, SpinnerColumn, TextColumn
from rich.table import Table

from abathur import __version__

# Initialize Typer app
app = typer.Typer(
    name="abathur",
    help="Hivemind Swarm Management System - Orchestrate specialized Claude agents",
    no_args_is_help=True,
)

console = Console()


# Helper to resolve UUID prefix to full UUID
async def _resolve_task_id(task_id_prefix: str, services: dict[str, Any]) -> UUID | None:
    """Resolve a task ID prefix to a full UUID.

    Args:
        task_id_prefix: Full UUID or prefix (e.g., 'ebec23ad')
        services: Services dictionary with task_coordinator

    Returns:
        Full UUID if exactly one match found, None otherwise

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
        logger.info("auth_initialized", method="api_key")
    except ValueError:
        # API key not found, try Claude CLI
        try:
            auth_provider = ClaudeCLIAuthProvider()
            logger.info("auth_initialized", method="claude_cli")
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


@app.command()
def status(watch: bool = typer.Option(False, help="Watch mode (live updates)")) -> None:
    """Show system status."""

    async def _status() -> None:
        services = await _get_services()
        from abathur.domain.models import TaskStatus

        # Count tasks by status
        pending = len(await services["database"].list_tasks(TaskStatus.PENDING, 1000))
        running = len(await services["database"].list_tasks(TaskStatus.RUNNING, 1000))
        completed = len(await services["database"].list_tasks(TaskStatus.COMPLETED, 1000))
        failed = len(await services["database"].list_tasks(TaskStatus.FAILED, 1000))

        console.print("[bold]Abathur System Status[/bold]")
        console.print(f"Pending tasks: {pending}")
        console.print(f"Running tasks: {running}")
        console.print(f"Completed tasks: {completed}")
        console.print(f"Failed tasks: {failed}")
        console.print(f"Total tasks: {pending + running + completed + failed}")

    asyncio.run(_status())


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
