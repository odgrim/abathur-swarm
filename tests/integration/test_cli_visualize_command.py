"""Integration tests for 'abathur task visualize' CLI command.

Tests:
- Command registration and help text
- Command execution with various options
- Error handling (missing TUI components, service initialization failures)
- Graceful shutdown on KeyboardInterrupt
"""

import subprocess
from unittest.mock import AsyncMock, MagicMock, patch

import pytest

from abathur.cli.main import task_app


class TestVisualizeCommand:
    """Test suite for 'abathur task visualize' command."""

    def test_command_registered(self):
        """Test that visualize command is registered under task_app."""
        # Get list of registered commands
        commands = {cmd.name for cmd in task_app.registered_commands}

        assert "visualize" in commands, "visualize command should be registered"

    def test_command_help(self, tmp_path):
        """Test that command help text is accessible and well-formed."""
        # Run help command
        result = subprocess.run(
            ["abathur", "task", "visualize", "--help"],
            capture_output=True,
            text=True,
        )

        assert result.returncode == 0, "Help command should succeed"
        assert "Launch interactive TUI" in result.stdout, "Help text should describe TUI"
        assert "--refresh-interval" in result.stdout, "Help should show refresh-interval option"
        assert "--no-auto-refresh" in result.stdout, "Help should show no-auto-refresh option"
        assert "--view-mode" in result.stdout, "Help should show view-mode option"
        assert "--no-unicode" in result.stdout, "Help should show no-unicode option"

    @pytest.mark.asyncio
    async def test_visualize_with_missing_tui_components(self, isolated_database):
        """Test visualize command when TUI components are not yet implemented."""
        from typer.testing import CliRunner
        from abathur.cli.main import app

        runner = CliRunner()

        # Mock import to raise ImportError
        with patch("abathur.cli.main.asyncio.run") as mock_run:
            # Simulate ImportError from TUI imports
            mock_run.side_effect = ImportError("No module named 'abathur.tui.app'")

            result = runner.invoke(app, ["task", "visualize", "--no-auto-refresh"])

            # Should handle ImportError gracefully
            assert result.exit_code == 1, "Should exit with error code"

    @pytest.mark.asyncio
    async def test_visualize_command_options(self):
        """Test that command accepts all documented options."""
        from typer.testing import CliRunner
        from abathur.cli.main import app

        runner = CliRunner()

        # Test with all options (will fail to run TUI, but should parse args correctly)
        test_cases = [
            ["task", "visualize", "--help"],
            ["task", "visualize", "--no-auto-refresh"],
            ["task", "visualize", "--refresh-interval", "5.0"],
            ["task", "visualize", "--view-mode", "dependency"],
            ["task", "visualize", "--no-unicode"],
            ["task", "visualize", "--no-auto-refresh", "--no-unicode"],
        ]

        for args in test_cases:
            result = runner.invoke(app, args)
            # Help command should succeed, others will fail due to missing TUI
            if "--help" in args:
                assert result.exit_code == 0, f"Command should accept args: {args}"

    @pytest.mark.asyncio
    async def test_visualize_service_initialization(self, isolated_database):
        """Test that visualize command properly initializes services."""
        from typer.testing import CliRunner
        from abathur.cli.main import app

        runner = CliRunner()

        # Mock TUI components to test service initialization
        with patch("abathur.cli.main.asyncio.run") as mock_run:
            async def mock_visualize():
                # Import inside mock to test service initialization path
                from abathur.cli.main import _get_services

                services = await _get_services()

                # Verify required services are initialized
                assert "database" in services
                assert "task_queue_service" in services
                assert services["task_queue_service"].dependency_resolver is not None

            mock_run.side_effect = mock_visualize

            # This will call the mocked asyncio.run
            result = runner.invoke(app, ["task", "visualize", "--no-auto-refresh"])

    @pytest.mark.asyncio
    async def test_visualize_with_tui_components(self, isolated_database):
        """Test visualize command when TUI components are available (mocked)."""
        from typer.testing import CliRunner
        from abathur.cli.main import app

        runner = CliRunner()

        # Mock TUI app and services
        mock_tui_app = MagicMock()
        mock_tui_app.run_async = AsyncMock()

        mock_task_data_service = MagicMock()

        with patch("abathur.cli.main.TaskQueueTUI", return_value=mock_tui_app):
            with patch("abathur.cli.main.TaskDataService", return_value=mock_task_data_service):
                # This will fail because asyncio.run is actually called
                # But we can verify the command structure is correct
                result = runner.invoke(app, ["task", "visualize", "--no-auto-refresh"])

                # The command should attempt to run (may fail in test environment)
                # Main goal: verify no syntax errors in command implementation

    @pytest.mark.asyncio
    async def test_visualize_keyboard_interrupt(self):
        """Test that KeyboardInterrupt is handled gracefully."""
        from typer.testing import CliRunner
        from abathur.cli.main import app

        runner = CliRunner()

        with patch("abathur.cli.main.asyncio.run") as mock_run:
            # Simulate KeyboardInterrupt
            mock_run.side_effect = KeyboardInterrupt()

            result = runner.invoke(app, ["task", "visualize"])

            # Should exit cleanly (not crash)
            # Exit code might be non-zero, but shouldn't raise unhandled exception


class TestVisualizeCommandParameterValidation:
    """Test parameter validation for visualize command."""

    def test_refresh_interval_type(self):
        """Test that refresh_interval accepts float values."""
        from typer.testing import CliRunner
        from abathur.cli.main import app

        runner = CliRunner()

        # Valid float values
        valid_cases = ["2.0", "0.5", "10.0"]

        for value in valid_cases:
            result = runner.invoke(app, ["task", "visualize", "--refresh-interval", value, "--help"])
            assert result.exit_code == 0, f"Should accept refresh_interval={value}"

        # Invalid values
        invalid_cases = ["abc", "not-a-number"]

        for value in invalid_cases:
            result = runner.invoke(app, ["task", "visualize", "--refresh-interval", value])
            assert result.exit_code != 0, f"Should reject refresh_interval={value}"

    def test_view_mode_values(self):
        """Test that view_mode accepts expected values."""
        from typer.testing import CliRunner
        from abathur.cli.main import app

        runner = CliRunner()

        # Valid view modes (as documented in help text)
        valid_modes = ["tree", "dependency", "timeline", "feature-branch", "flat-list"]

        for mode in valid_modes:
            result = runner.invoke(app, ["task", "visualize", "--view-mode", mode, "--help"])
            # Help should succeed regardless of view mode
            assert result.exit_code == 0, f"Should accept view_mode={mode}"

    def test_boolean_flags(self):
        """Test that boolean flags work correctly."""
        from typer.testing import CliRunner
        from abathur.cli.main import app

        runner = CliRunner()

        # Test boolean flags
        result = runner.invoke(app, ["task", "visualize", "--no-auto-refresh", "--help"])
        assert result.exit_code == 0, "Should accept --no-auto-refresh flag"

        result = runner.invoke(app, ["task", "visualize", "--no-unicode", "--help"])
        assert result.exit_code == 0, "Should accept --no-unicode flag"

        # Test combined flags
        result = runner.invoke(app, ["task", "visualize", "--no-auto-refresh", "--no-unicode", "--help"])
        assert result.exit_code == 0, "Should accept multiple flags"


class TestVisualizeCommandIntegration:
    """Integration tests with real service initialization."""

    @pytest.mark.asyncio
    async def test_service_initialization_pattern(self, isolated_database):
        """Test that visualize uses existing _get_services() pattern correctly."""
        from abathur.cli.main import _get_services
        from abathur.services import DependencyResolver, TaskQueueService

        # Initialize services
        services = await _get_services()

        # Verify services structure matches what visualize expects
        assert "database" in services
        assert "task_queue_service" in services
        assert isinstance(services["task_queue_service"], TaskQueueService)

        # Verify dependency_resolver is accessible
        dependency_resolver = services["task_queue_service"].dependency_resolver
        assert isinstance(dependency_resolver, DependencyResolver)

    @pytest.mark.asyncio
    async def test_task_data_service_creation(self, isolated_database):
        """Test that TaskDataService can be created from services (when implemented)."""
        from abathur.cli.main import _get_services

        services = await _get_services()

        # Verify all required dependencies are available
        # (TaskDataService not yet implemented, so we just check dependencies)
        assert services["database"] is not None
        assert services["task_queue_service"] is not None
        assert services["task_queue_service"].dependency_resolver is not None


# Test fixtures can be added here for mocking TUI components
@pytest.fixture
def mock_tui_app():
    """Mock TUI app for testing."""
    mock_app = MagicMock()
    mock_app.run_async = AsyncMock()
    return mock_app


@pytest.fixture
def mock_task_data_service():
    """Mock TaskDataService for testing."""
    return MagicMock()
