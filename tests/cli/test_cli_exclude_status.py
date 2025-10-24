"""Unit tests for CLI --exclude-status parameter.

Tests the CLI layer handling of the --exclude-status flag for task list command.
Uses typer.testing.CliRunner to test CLI parameter parsing and validation.

Test Coverage:
- Valid status value acceptance
- Invalid status value error handling
- All TaskStatus enum values
- Combined with --limit flag
- Combined with --status flag
- Backward compatibility (None default)

**IMPORTANT NOTE**:
These tests are written for the CLI implementation that will be added in task
c45cc01d-c060-4fb3-b4fa-4a24352346df. Currently, 11/12 tests will fail with
"no such option: --exclude-status" because the CLI parameter hasn't been
implemented yet. This is expected behavior.

Once the CLI implementation is complete, all tests should pass. The one test
that currently passes (test_exclude_status_none_when_not_provided) validates
backward compatibility.

Expected State After CLI Implementation:
- All 12 tests should pass
- CLI should accept --exclude-status parameter
- Parameter should be validated against TaskStatus enum values
- Invalid values should show clear error messages
"""

from unittest.mock import AsyncMock, patch

import pytest
from abathur.cli.main import app
from abathur.domain.models import TaskStatus
from typer.testing import CliRunner

# Initialize CliRunner
runner = CliRunner()


class TestCLIExcludeStatus:
    """Unit tests for CLI --exclude-status parameter."""

    def test_exclude_status_valid_value(self):
        """Test valid status values are accepted and passed to TaskCoordinator.

        Verifies that:
        - CLI accepts valid TaskStatus enum values
        - Parameter is correctly converted to TaskStatus.COMPLETED
        - TaskCoordinator.list_tasks() is called with exclude_status parameter
        """
        # Arrange
        with patch('abathur.cli.main._get_services') as mock_get_services:
            # Create mock services
            mock_coordinator = AsyncMock()
            mock_coordinator.list_tasks = AsyncMock(return_value=[])
            mock_services = {
                'task_coordinator': mock_coordinator,
                'database': AsyncMock()
            }
            mock_get_services.return_value = mock_services

            # Act
            result = runner.invoke(app, ["task", "list", "--exclude-status", "completed"])

            # Assert
            assert result.exit_code == 0
            # Verify TaskCoordinator.list_tasks was called with exclude_status
            mock_coordinator.list_tasks.assert_called_once()
            call_args = mock_coordinator.list_tasks.call_args
            # Should be called with positional args: (status, limit, exclude_status)
            # or keyword args
            if call_args.kwargs:
                assert 'exclude_status' in call_args.kwargs
                assert call_args.kwargs['exclude_status'] == TaskStatus.COMPLETED
            else:
                # Positional args: (status, limit, exclude_status)
                assert len(call_args.args) >= 3
                assert call_args.args[2] == TaskStatus.COMPLETED

    def test_exclude_status_invalid_value(self):
        """Test invalid status values raise clear error.

        Verifies that:
        - Invalid status value triggers typer.BadParameter
        - Error message lists all valid status values
        - Error message includes the invalid value provided
        """
        # Arrange - no need to mock services since validation happens before service call

        # Act
        result = runner.invoke(app, ["task", "list", "--exclude-status", "invalid_value"])

        # Assert
        assert result.exit_code != 0
        # Check error message contains expected text
        error_output = result.output
        assert "invalid_value" in error_output.lower()
        # Should mention valid values
        assert any(status in error_output.lower() for status in [
            "pending", "blocked", "ready", "running", "completed", "failed", "cancelled"
        ])

    @pytest.mark.parametrize("status_value,expected_enum", [
        ("pending", TaskStatus.PENDING),
        ("blocked", TaskStatus.BLOCKED),
        ("ready", TaskStatus.READY),
        ("running", TaskStatus.RUNNING),
        ("completed", TaskStatus.COMPLETED),
        ("failed", TaskStatus.FAILED),
        ("cancelled", TaskStatus.CANCELLED),
    ])
    def test_exclude_status_all_enum_values(self, status_value, expected_enum):
        """Test all 7 TaskStatus enum values parse correctly.

        Verifies that:
        - Each TaskStatus enum value is accepted
        - String values are correctly converted to TaskStatus enum
        - TaskCoordinator receives the correct enum value

        Args:
            status_value: String status value from CLI
            expected_enum: Expected TaskStatus enum value
        """
        # Arrange
        with patch('abathur.cli.main._get_services') as mock_get_services:
            mock_coordinator = AsyncMock()
            mock_coordinator.list_tasks = AsyncMock(return_value=[])
            mock_services = {
                'task_coordinator': mock_coordinator,
                'database': AsyncMock()
            }
            mock_get_services.return_value = mock_services

            # Act
            result = runner.invoke(app, ["task", "list", "--exclude-status", status_value])

            # Assert
            assert result.exit_code == 0, f"Failed for status: {status_value}, output: {result.output}"
            mock_coordinator.list_tasks.assert_called_once()
            call_args = mock_coordinator.list_tasks.call_args

            # Verify correct enum value passed
            if call_args.kwargs:
                assert call_args.kwargs['exclude_status'] == expected_enum
            else:
                assert call_args.args[2] == expected_enum

    def test_exclude_status_combined_with_limit(self):
        """Test --exclude-status works with --limit flag.

        Verifies that:
        - Both --exclude-status and --limit can be used together
        - Both parameters are correctly passed to TaskCoordinator
        - Parameter order doesn't matter
        """
        # Arrange
        with patch('abathur.cli.main._get_services') as mock_get_services:
            mock_coordinator = AsyncMock()
            mock_coordinator.list_tasks = AsyncMock(return_value=[])
            mock_services = {
                'task_coordinator': mock_coordinator,
                'database': AsyncMock()
            }
            mock_get_services.return_value = mock_services

            # Act
            result = runner.invoke(app, [
                "task", "list",
                "--exclude-status", "completed",
                "--limit", "10"
            ])

            # Assert
            assert result.exit_code == 0
            mock_coordinator.list_tasks.assert_called_once()
            call_args = mock_coordinator.list_tasks.call_args

            # Verify both parameters passed
            if call_args.kwargs:
                assert call_args.kwargs.get('exclude_status') == TaskStatus.COMPLETED
                assert call_args.kwargs.get('limit') == 10 or call_args.args[1] == 10
            else:
                # Positional: (status, limit, exclude_status)
                assert call_args.args[1] == 10  # limit
                assert call_args.args[2] == TaskStatus.COMPLETED  # exclude_status

    def test_exclude_status_combined_with_status(self):
        """Test --exclude-status works with --status flag (advanced filtering).

        Verifies that:
        - Both --status and --exclude-status can be used together
        - Allows filtering for status=ready while excluding status=blocked
        - Both parameters are correctly passed to TaskCoordinator
        """
        # Arrange
        with patch('abathur.cli.main._get_services') as mock_get_services:
            mock_coordinator = AsyncMock()
            mock_coordinator.list_tasks = AsyncMock(return_value=[])
            mock_services = {
                'task_coordinator': mock_coordinator,
                'database': AsyncMock()
            }
            mock_get_services.return_value = mock_services

            # Act
            result = runner.invoke(app, [
                "task", "list",
                "--status", "ready",
                "--exclude-status", "blocked"
            ])

            # Assert
            assert result.exit_code == 0
            mock_coordinator.list_tasks.assert_called_once()
            call_args = mock_coordinator.list_tasks.call_args

            # Verify both parameters passed
            if call_args.kwargs:
                # Keyword args
                assert call_args.kwargs.get('exclude_status') == TaskStatus.BLOCKED
                # Status is positional or keyword
                if 'status' in call_args.kwargs:
                    assert call_args.kwargs['status'] == TaskStatus.READY
                else:
                    assert call_args.args[0] == TaskStatus.READY
            else:
                # Positional: (status, limit, exclude_status)
                assert call_args.args[0] == TaskStatus.READY  # status
                assert call_args.args[2] == TaskStatus.BLOCKED  # exclude_status

    def test_exclude_status_none_when_not_provided(self):
        """Test backward compatibility - exclude_status=None when flag not provided.

        Verifies that:
        - When --exclude-status is not provided, parameter is None
        - Existing behavior is preserved (backward compatibility)
        - TaskCoordinator is called correctly without exclude_status
        """
        # Arrange
        with patch('abathur.cli.main._get_services') as mock_get_services:
            mock_coordinator = AsyncMock()
            mock_coordinator.list_tasks = AsyncMock(return_value=[])
            mock_services = {
                'task_coordinator': mock_coordinator,
                'database': AsyncMock()
            }
            mock_get_services.return_value = mock_services

            # Act - invoke without --exclude-status flag
            result = runner.invoke(app, ["task", "list"])

            # Assert
            assert result.exit_code == 0
            mock_coordinator.list_tasks.assert_called_once()
            call_args = mock_coordinator.list_tasks.call_args

            # Verify exclude_status is None (backward compatibility)
            if call_args.kwargs:
                # If using kwargs, exclude_status should be None or not present
                exclude_status = call_args.kwargs.get('exclude_status')
                assert exclude_status is None
            else:
                # If using positional args, should have 3 args with last being None
                if len(call_args.args) >= 3:
                    assert call_args.args[2] is None
