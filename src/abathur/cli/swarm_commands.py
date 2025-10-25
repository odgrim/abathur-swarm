"""Swarm orchestration commands."""

import asyncio
import signal as sig
from typing import Any

import typer
from rich.console import Console

console = Console()

# Create Typer app for swarm commands
swarm_app = typer.Typer(help="Swarm orchestration", no_args_is_help=True)


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
        from abathur.cli.main import _get_services
        from abathur.mcp.server_manager import MemoryServerManager

        services = await _get_services()

        # Update max_concurrent_agents if specified via CLI
        if max_agents != 10:  # 10 is the default value
            services["swarm_orchestrator"].max_concurrent_agents = max_agents
            # Also update the semaphore to match new limit
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
        from abathur.cli.main import _get_services

        services = await _get_services()
        status = await services["swarm_orchestrator"].get_swarm_status()

        console.print("[bold]Swarm Status[/bold]")
        console.print(f"Active tasks: {status.get('active_tasks', 0)}")
        console.print(f"Completed tasks: {status.get('completed_tasks', 0)}")
        console.print(f"Failed tasks: {status.get('failed_tasks', 0)}")

    asyncio.run(_status())
