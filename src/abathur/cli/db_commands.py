"""Database management commands."""

import asyncio
import json
import time
from pathlib import Path

import typer
from rich.console import Console
from rich.progress import Progress, SpinnerColumn, TextColumn

from abathur.infrastructure import ConfigManager, Database, DatabaseValidator

console = Console()

# Create Typer app for db commands
db_app = typer.Typer(help="Database management", no_args_is_help=True)


@db_app.command("init")
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
        abathur db init                                    # Init DB + update templates
        abathur db init --skip-template                    # Only init database
        abathur db init --validate
        abathur db init --validate --report-output validation.json
        abathur db init --db-path /tmp/test.db --validate
    """

    async def _init() -> None:
        # Import _get_services from main
        from abathur.cli.main import _get_services

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
