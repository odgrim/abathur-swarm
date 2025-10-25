"""MCP server management commands."""

import asyncio

import typer
from rich.console import Console
from rich.table import Table

console = Console()

# Create Typer app for MCP commands
mcp_app = typer.Typer(help="MCP server management", no_args_is_help=True)


@mcp_app.command("list")
def mcp_list() -> None:
    """List all MCP servers (including built-in memory server)."""

    async def _list() -> None:
        from abathur.cli.main import _get_services
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
            from abathur.cli.main import _get_services

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
            from abathur.cli.main import _get_services

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
            from abathur.cli.main import _get_services

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
                from abathur.cli.main import _get_services

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
            from abathur.cli.main import _get_services
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
