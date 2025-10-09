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


# Helper to get database and services
async def _get_services() -> dict[str, Any]:
    """Get initialized services."""
    from abathur.application import (
        AgentExecutor,
        ClaudeClient,
        FailureRecovery,
        LoopExecutor,
        MCPManager,
        ResourceMonitor,
        SwarmOrchestrator,
        TaskCoordinator,
        TemplateManager,
    )
    from abathur.infrastructure import ConfigManager, Database

    config_manager = ConfigManager()
    database = Database(config_manager.get_database_path())
    await database.initialize()

    task_coordinator = TaskCoordinator(database)
    claude_client = ClaudeClient(api_key=config_manager.get_api_key())
    agent_executor = AgentExecutor(database, claude_client)
    swarm_orchestrator = SwarmOrchestrator(
        task_coordinator, agent_executor, max_concurrent_agents=10
    )
    template_manager = TemplateManager()
    mcp_manager = MCPManager()
    await mcp_manager.initialize()
    failure_recovery = FailureRecovery(task_coordinator, database)
    resource_monitor = ResourceMonitor()
    loop_executor = LoopExecutor(task_coordinator, agent_executor, database)

    return {
        "database": database,
        "task_coordinator": task_coordinator,
        "claude_client": claude_client,
        "agent_executor": agent_executor,
        "swarm_orchestrator": swarm_orchestrator,
        "template_manager": template_manager,
        "mcp_manager": mcp_manager,
        "failure_recovery": failure_recovery,
        "resource_monitor": resource_monitor,
        "loop_executor": loop_executor,
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
    template: str = typer.Argument(..., help="Template name"),
    input_file: Path | None = typer.Option(None, help="JSON file with input data"),  # noqa: B008
    priority: int = typer.Option(5, help="Task priority (0-10)"),  # noqa: B008
) -> None:
    """Submit a new task to the queue."""

    async def _submit() -> UUID:
        services = await _get_services()
        from abathur.domain.models import Task

        # Load input data
        input_data = {}
        if input_file and input_file.exists():
            with open(input_file) as f:
                input_data = json.load(f)

        task = Task(template_name=template, input_data=input_data, priority=priority)
        task_id: UUID = await services["task_coordinator"].submit_task(task)

        console.print(f"[green]✓[/green] Task submitted: [cyan]{task_id}[/cyan]")
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
        table.add_column("ID", style="cyan")
        table.add_column("Template", style="green")
        table.add_column("Priority", justify="center")
        table.add_column("Status", style="yellow")
        table.add_column("Submitted", style="blue")

        for task in tasks:
            table.add_row(
                str(task.id)[:8],
                task.template_name,
                str(task.priority),
                task.status.value,
                task.submitted_at.strftime("%Y-%m-%d %H:%M"),
            )

        console.print(table)

    asyncio.run(_list())


@task_app.command("status")
def task_status(task_id: str = typer.Argument(..., help="Task ID")) -> None:
    """Get detailed task status."""

    async def _status() -> None:
        services = await _get_services()
        task = await services["task_coordinator"].get_task(UUID(task_id))

        if not task:
            console.print(f"[red]Error:[/red] Task {task_id} not found")
            return

        console.print(f"[bold]Task {task.id}[/bold]")
        console.print(f"Template: {task.template_name}")
        console.print(f"Priority: {task.priority}")
        console.print(f"Status: {task.status.value}")
        console.print(f"Submitted: {task.submitted_at}")
        if task.started_at:
            console.print(f"Started: {task.started_at}")
        if task.completed_at:
            console.print(f"Completed: {task.completed_at}")
        if task.error_message:
            console.print(f"[red]Error:[/red] {task.error_message}")

    asyncio.run(_status())


@task_app.command("cancel")
def cancel(task_id: str = typer.Argument(..., help="Task ID to cancel")) -> None:
    """Cancel a pending/running task."""

    async def _cancel() -> None:
        services = await _get_services()

        success = await services["task_coordinator"].cancel_task(UUID(task_id))

        if success:
            console.print(f"[green]✓[/green] Task {task_id} cancelled")
        else:
            console.print(f"[red]Error:[/red] Failed to cancel task {task_id}")

    asyncio.run(_cancel())


@task_app.command("retry")
def retry(task_id: str = typer.Argument(..., help="Task ID to retry")) -> None:
    """Retry a failed task."""

    async def _retry() -> None:
        services = await _get_services()
        success = await services["task_coordinator"].retry_task(UUID(task_id))

        if success:
            console.print(f"[green]✓[/green] Task {task_id} queued for retry")
        else:
            console.print(f"[red]Error:[/red] Failed to retry task {task_id}")

    asyncio.run(_retry())


# ===== Swarm Commands =====
swarm_app = typer.Typer(help="Agent swarm management", no_args_is_help=True)
app.add_typer(swarm_app, name="swarm")


@swarm_app.command("start")
def start_swarm(
    task_limit: int | None = typer.Option(None, help="Max tasks to process"),
    max_agents: int = typer.Option(10, help="Max concurrent agents"),
) -> None:
    """Start the swarm orchestrator."""

    async def _start() -> None:
        services = await _get_services()

        console.print("[blue]Starting swarm orchestrator...[/blue]")

        # Start monitoring
        await services["resource_monitor"].start_monitoring()
        await services["failure_recovery"].start_recovery_monitor()

        # Start swarm
        results = await services["swarm_orchestrator"].start_swarm(task_limit)

        console.print(f"[green]✓[/green] Swarm completed {len(results)} tasks")

        # Stop monitoring
        await services["resource_monitor"].stop_monitoring()
        await services["failure_recovery"].stop_recovery_monitor()

    asyncio.run(_start())


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


# ===== Template Commands =====
template_app = typer.Typer(help="Template management", no_args_is_help=True)
app.add_typer(template_app, name="template")


@template_app.command("list")
def template_list() -> None:
    """List installed templates."""

    async def _list() -> None:
        services = await _get_services()
        templates = services["template_manager"].list_templates()

        if not templates:
            console.print("No templates installed")
            return

        table = Table(title="Templates")
        table.add_column("Name", style="cyan")
        table.add_column("Path", style="green")

        for template in templates:
            table.add_row(template.name, str(template.path))

        console.print(table)

    asyncio.run(_list())


@template_app.command("install")
def template_install(
    repo_url: str = typer.Argument(..., help="Git repository URL"),
    version: str = typer.Option("main", help="Git branch/tag"),
) -> None:
    """Install a template from Git repository."""

    async def _install() -> None:
        services = await _get_services()

        with Progress(
            SpinnerColumn(),
            TextColumn("[progress.description]{task.description}"),
            console=console,
        ) as progress:
            progress.add_task(description=f"Cloning template from {repo_url}...", total=None)

            template = await services["template_manager"].clone_template(repo_url, version)

        console.print(f"[green]✓[/green] Template installed: [cyan]{template.name}[/cyan]")

    asyncio.run(_install())


@template_app.command("validate")
def template_validate(name: str = typer.Argument(..., help="Template name")) -> None:
    """Validate a template."""

    async def _validate() -> None:
        services = await _get_services()
        template = services["template_manager"].get_template(name)

        if not template:
            console.print(f"[red]Error:[/red] Template {name} not found")
            return

        result = services["template_manager"].validate_template(template)

        if result.is_valid:
            console.print(f"[green]✓[/green] Template {name} is valid")
        else:
            console.print(f"[red]✗[/red] Template {name} validation failed:")
            for error in result.errors:
                console.print(f"  - {error}")

    asyncio.run(_validate())


# ===== MCP Commands =====
mcp_app = typer.Typer(help="MCP server management", no_args_is_help=True)
app.add_typer(mcp_app, name="mcp")


@mcp_app.command("list")
def mcp_list() -> None:
    """List MCP servers."""

    async def _list() -> None:
        services = await _get_services()
        status = services["mcp_manager"].get_all_server_status()

        if not status:
            console.print("No MCP servers configured")
            return

        table = Table(title="MCP Servers")
        table.add_column("Name", style="cyan")
        table.add_column("Command", style="green")
        table.add_column("State", style="yellow")
        table.add_column("PID", justify="center")

        for name, server_status in status.items():
            table.add_row(
                name,
                server_status.get("command", ""),
                server_status.get("state", "unknown"),
                str(server_status.get("pid", "N/A")),
            )

        console.print(table)

    asyncio.run(_list())


@mcp_app.command("start")
def mcp_start(server: str = typer.Argument(..., help="Server name")) -> None:
    """Start an MCP server."""

    async def _start() -> None:
        services = await _get_services()
        success = await services["mcp_manager"].start_server(server)

        if success:
            console.print(f"[green]✓[/green] MCP server [cyan]{server}[/cyan] started")
        else:
            console.print(f"[red]Error:[/red] Failed to start MCP server {server}")

    asyncio.run(_start())


@mcp_app.command("stop")
def mcp_stop(server: str = typer.Argument(..., help="Server name")) -> None:
    """Stop an MCP server."""

    async def _stop() -> None:
        services = await _get_services()
        success = await services["mcp_manager"].stop_server(server)

        if success:
            console.print(f"[green]✓[/green] MCP server [cyan]{server}[/cyan] stopped")
        else:
            console.print(f"[red]Error:[/red] Failed to stop MCP server {server}")

    asyncio.run(_stop())


@mcp_app.command("restart")
def mcp_restart(server: str = typer.Argument(..., help="Server name")) -> None:
    """Restart an MCP server."""

    async def _restart() -> None:
        services = await _get_services()
        success = await services["mcp_manager"].restart_server(server)

        if success:
            console.print(f"[green]✓[/green] MCP server [cyan]{server}[/cyan] restarted")
        else:
            console.print(f"[red]Error:[/red] Failed to restart MCP server {server}")

    asyncio.run(_restart())


# ===== Loop Commands =====
loop_app = typer.Typer(help="Iterative loop execution", no_args_is_help=True)
app.add_typer(loop_app, name="loop")


@loop_app.command("start")
def loop_start(
    task_id: str = typer.Argument(..., help="Task ID to execute in loop"),
    max_iterations: int = typer.Option(10, help="Maximum iterations"),
    convergence_threshold: float = typer.Option(0.95, help="Convergence threshold"),
) -> None:
    """Start an iterative refinement loop."""

    async def _start() -> None:
        services = await _get_services()
        from abathur.application import ConvergenceCriteria, ConvergenceType

        task = await services["task_coordinator"].get_task(UUID(task_id))
        if not task:
            console.print(f"[red]Error:[/red] Task {task_id} not found")
            return

        criteria = ConvergenceCriteria(
            type=ConvergenceType.THRESHOLD,
            threshold=convergence_threshold,
        )

        console.print(f"[blue]Starting loop execution for task {task_id}...[/blue]")

        result = await services["loop_executor"].execute_loop(task, criteria, max_iterations)

        if result.converged:
            console.print(f"[green]✓[/green] Converged after {result.iterations} iterations")
        else:
            console.print(
                f"[yellow]![/yellow] Did not converge ({result.reason}) after {result.iterations} iterations"
            )

    asyncio.run(_start())


# ===== Resource Commands =====
@app.command()
def resources() -> None:
    """Show resource usage statistics."""

    async def _resources() -> None:
        services = await _get_services()
        await services["resource_monitor"].start_monitoring()
        await asyncio.sleep(1)  # Let it collect data
        stats = services["resource_monitor"].get_stats()
        await services["resource_monitor"].stop_monitoring()

        console.print("[bold]Resource Usage[/bold]")
        if stats.get("current"):
            current = stats["current"]
            console.print(f"CPU: {current.get('cpu_percent', 0):.1f}%")
            console.print(
                f"Memory: {current.get('memory_mb', 0):.1f} MB ({current.get('memory_percent', 0):.1f}%)"
            )
            console.print(f"Available: {current.get('available_memory_mb', 0):.1f} MB")
            console.print(f"Active agents: {current.get('agent_count', 0)}")

    asyncio.run(_resources())


# ===== DLQ Commands =====
dlq_app = typer.Typer(help="Dead letter queue management", no_args_is_help=True)
app.add_typer(dlq_app, name="dlq")


@dlq_app.command("list")
def dlq_list() -> None:
    """List tasks in dead letter queue."""

    async def _list() -> None:
        services = await _get_services()
        dlq_tasks = services["failure_recovery"].get_dlq_tasks()

        if not dlq_tasks:
            console.print("Dead letter queue is empty")
            return

        table = Table(title="Dead Letter Queue")
        table.add_column("Task ID", style="cyan")

        for task_id in dlq_tasks:
            table.add_row(str(task_id))

        console.print(table)

    asyncio.run(_list())


@dlq_app.command("reprocess")
def dlq_reprocess(task_id: str = typer.Argument(..., help="Task ID")) -> None:
    """Reprocess a task from DLQ."""

    async def _reprocess() -> None:
        services = await _get_services()
        success = await services["failure_recovery"].reprocess_dlq_task(UUID(task_id))

        if success:
            console.print(f"[green]✓[/green] Task {task_id} requeued from DLQ")
        else:
            console.print(f"[red]Error:[/red] Failed to reprocess task {task_id}")

    asyncio.run(_reprocess())


# ===== Config Commands =====
config_app = typer.Typer(help="Configuration management", no_args_is_help=True)
app.add_typer(config_app, name="config")


@config_app.command("show")
def config_show() -> None:
    """Show current configuration."""

    async def _show() -> None:
        from abathur.infrastructure import ConfigManager

        config_manager = ConfigManager()
        config = config_manager.load_config()

        console.print("[bold]Configuration[/bold]")
        console.print(f"Database path: {config_manager.get_database_path()}")
        console.print(f"Log level: {config.log_level}")
        console.print(f"Max concurrent agents: {config.swarm.max_concurrent_agents}")
        console.print(f"Max queue size: {config.queue.max_size}")
        console.print(f"Max loop iterations: {config.loop.max_iterations}")

    asyncio.run(_show())


@config_app.command("validate")
def config_validate() -> None:
    """Validate configuration files."""
    try:
        from abathur.infrastructure.config import ConfigManager

        config_manager = ConfigManager()
        config = config_manager.load_config()
        console.print("[green]✓[/green] Configuration is valid")
        console.print(f"Version: {config.version}")
        console.print(f"Log level: {config.log_level}")
        console.print(f"Max concurrent agents: {config.swarm.max_concurrent_agents}")
    except Exception as e:
        console.print(f"[red]✗[/red] Configuration error: {e}")
        raise typer.Exit(1) from e


@config_app.command("set-key")
def config_set_key(
    api_key: str = typer.Argument(..., help="Anthropic API key"),
    use_keychain: bool = typer.Option(True, help="Store in system keychain"),
) -> None:
    """Set Anthropic API key."""
    try:
        from abathur.infrastructure.config import ConfigManager

        config_manager = ConfigManager()
        config_manager.set_api_key(api_key, use_keychain=use_keychain)
        storage = "keychain" if use_keychain else ".env file"
        console.print(f"[green]✓[/green] API key stored in {storage}")
    except Exception as e:
        console.print(f"[red]✗[/red] Failed to store API key: {e}")
        raise typer.Exit(1) from e


# ===== Database Commands =====
@app.command()
def init(
    template: str | None = typer.Option(None, help="Template repository URL or name"),
    version_tag: str | None = typer.Option("main", help="Template version (tag or branch)"),
) -> None:
    """Initialize a new Abathur project with template."""

    async def _init() -> None:
        from abathur.infrastructure import ConfigManager, Database

        console.print("[blue]Initializing Abathur project...[/blue]")

        # Initialize database
        config_manager = ConfigManager()
        database = Database(config_manager.get_database_path())
        await database.initialize()

        console.print("[green]✓[/green] Database initialized")

        if template:
            services = await _get_services()
            console.print(f"[blue]Cloning template: {template}...[/blue]")
            tmpl = await services["template_manager"].clone_template(template, version_tag)
            console.print(f"[green]✓[/green] Template installed: {tmpl.name}")

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


# ===== Recovery Stats =====
@app.command()
def recovery() -> None:
    """Show failure recovery statistics."""

    async def _stats() -> None:
        services = await _get_services()
        stats = services["failure_recovery"].get_stats()

        console.print("[bold]Failure Recovery Statistics[/bold]")
        console.print(f"Total failures: {stats.get('total_failures', 0)}")
        console.print(f"Permanent failures: {stats.get('permanent_failures', 0)}")
        console.print(f"Transient failures: {stats.get('transient_failures', 0)}")
        console.print(f"Retried tasks: {stats.get('retried_tasks', 0)}")
        console.print(f"Recovered tasks: {stats.get('recovered_tasks', 0)}")
        console.print(f"DLQ count: {stats.get('dlq_count', 0)}")

    asyncio.run(_stats())


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
