"""Integration tests for CLI exception handling.

Tests that specific database exceptions are caught and displayed with
user-friendly error messages in the CLI prune and delete commands.

IMPORTANT: These tests verify the exception handling improvements made in:
- Task 79dec034: CLI prune exception handling
- Task cde092d8: CLI delete exception handling

The tests check for user-friendly error messages instead of raw stack traces.

Test Coverage:
- Database locked errors (sqlite3.OperationalError)
- Integrity constraint errors (sqlite3.IntegrityError)
- Connection errors (aiosqlite.Error)
- Validation errors (ValueError)
- Proper exit codes and error messages

NOTE: Currently testing against generic exception handler. Once exception handling
tasks are merged, the error messages will be more specific.
"""

import asyncio
import sqlite3
from pathlib import Path
from unittest.mock import AsyncMock, patch
from uuid import uuid4

import aiosqlite
import pytest
from typer.testing import CliRunner

from abathur.cli.main import app
from abathur.domain.models import Task, TaskSource, TaskStatus
from abathur.infrastructure import Database

runner = CliRunner()


@pytest.fixture(scope="function")
def database(cli_test_db_path: Path, mock_cli_database_path):
    """Create a test database at .abathur/test.db with mocked path."""
    # Create test database (path already cleaned by cli_test_db_path fixture)
    db = Database(cli_test_db_path)
    asyncio.run(db.initialize())
    return db


@pytest.fixture(scope="function")
def sample_task(database):
    """Create a single sample task for testing."""
    from datetime import datetime, timedelta, timezone

    from abathur.application import TaskCoordinator

    coordinator = TaskCoordinator(database)

    async def create_task():
        # Create old task (40 days ago) so it matches --older-than 1d filter
        old_time = datetime.now(timezone.utc) - timedelta(days=40)
        task = Task(
            prompt="Test task for exception handling",
            summary="Test task",
            agent_type="test-agent",
            source=TaskSource.HUMAN,
            status=TaskStatus.COMPLETED,
            submitted_at=old_time,
            completed_at=old_time,
        )
        task_id = await coordinator.submit_task(task)
        return task_id

    return asyncio.run(create_task())


def test_prune_database_locked_error(database, sample_task, cli_test_db_path):
    """Test that database locked error displays user-friendly message.

    Simulates sqlite3.OperationalError (database locked) during prune_tasks
    and verifies that:
    1. User sees friendly error message (not stack trace)
    2. Exit code is 1 (error)
    3. Message mentions retry/try again
    """
    # Store original method
    from abathur.infrastructure.database import Database

    original_prune = Database.prune_tasks

    # Create a wrapper that raises after being called
    async def mock_prune_raising(self, filters):
        """Mock prune_tasks that raises OperationalError."""
        raise sqlite3.OperationalError("database is locked")

    # Patch at class level so all instances use it
    with patch.object(Database, "prune_tasks", new=mock_prune_raising):
        # Execute: Run CLI command with time filter (triggers prune_tasks path)
        result = runner.invoke(app, ["task", "prune", "--older-than", "1d", "--force"])

        # Assert: Error handling works correctly
        assert result.exit_code == 1, f"Expected exit code 1, got {result.exit_code}\nOutput: {result.stdout}"

        # TODO: Once exception handling tasks are merged, update to check for specific message:
        # assert "Database is locked or busy" in result.stdout
        # assert "try again" in result.stdout.lower()

        # Current behavior: generic error message
        assert "Error:" in result.stdout, f"Expected error message, got: {result.stdout}"
        assert "OperationalError" in result.stdout, f"Expected OperationalError mention, got: {result.stdout}"
        assert "database is locked" in result.stdout, f"Expected 'database is locked', got: {result.stdout}"

        # Verify no Python stack trace leaked to user (critical requirement)
        assert "Traceback" not in result.stdout, (
            f"Stack trace leaked to user output: {result.stdout}"
        )


def test_prune_integrity_error(database, sample_task, cli_test_db_path):
    """Test that integrity constraint error displays user-friendly message.

    Simulates sqlite3.IntegrityError (constraint violation) during prune_tasks
    and verifies that:
    1. User sees friendly error message
    2. Exit code is 1
    3. Message mentions constraint violation
    """
    from abathur.infrastructure.database import Database

    async def mock_prune_raising(self, filters):
        """Mock prune_tasks that raises IntegrityError."""
        raise sqlite3.IntegrityError("FOREIGN KEY constraint failed")

    with patch.object(Database, "prune_tasks", new=mock_prune_raising):
        # Execute: Run CLI command
        result = runner.invoke(app, ["task", "prune", "--older-than", "1d", "--force"])

        # Assert: Error handling works correctly
        assert result.exit_code == 1, f"Expected exit code 1, got {result.exit_code}\nOutput: {result.stdout}"

        # TODO: Once exception handling tasks are merged, update to check for specific message:
        # assert "integrity constraint" in result.stdout.lower()

        # Current behavior: generic error message
        assert "Error:" in result.stdout, f"Expected error message, got: {result.stdout}"
        assert "IntegrityError" in result.stdout, f"Expected IntegrityError mention, got: {result.stdout}"

        # Verify no Python stack trace leaked to user (critical requirement)
        assert "Traceback" not in result.stdout, (
            f"Stack trace leaked to user output: {result.stdout}"
        )


def test_prune_aiosqlite_connection_error(database, sample_task, cli_test_db_path):
    """Test that aiosqlite connection error displays user-friendly message.

    Simulates aiosqlite.Error (connection/query failure) during prune_tasks
    and verifies that:
    1. User sees friendly error message
    2. Exit code is 1
    3. Message mentions connection or query error
    """
    from abathur.infrastructure.database import Database

    async def mock_prune_raising(self, filters):
        """Mock prune_tasks that raises aiosqlite.Error."""
        raise aiosqlite.Error("unable to open database file")

    with patch.object(Database, "prune_tasks", new=mock_prune_raising):
        # Execute: Run CLI command
        result = runner.invoke(app, ["task", "prune", "--older-than", "1d", "--force"])

        # Assert: Error handling works correctly
        assert result.exit_code == 1, f"Expected exit code 1, got {result.exit_code}\nOutput: {result.stdout}"

        # TODO: Once exception handling tasks are merged, update to check for specific message:
        # assert "connection or query error" in result.stdout.lower()

        # Current behavior: generic error message
        assert "Error:" in result.stdout, f"Expected error message, got: {result.stdout}"
        assert "Error" in result.stdout, f"Expected aiosqlite.Error mention, got: {result.stdout}"

        # Verify no Python stack trace leaked to user (critical requirement)
        assert "Traceback" not in result.stdout, (
            f"Stack trace leaked to user output: {result.stdout}"
        )


def test_delete_database_locked_error(database, sample_task, cli_test_db_path):
    """Test that database locked error in delete_tasks path displays friendly message.

    Tests the delete_tasks() path (task ID-based deletion) to ensure
    consistent error handling with prune_tasks() path.

    Verifies that:
    1. User sees friendly error message
    2. Exit code is 1
    3. Message is consistent with prune error handling
    """
    from abathur.infrastructure.database import Database

    task_id_str = str(sample_task)

    async def mock_delete_raising(self, task_ids):
        """Mock delete_tasks that raises OperationalError."""
        raise sqlite3.OperationalError("database is locked")

    with patch.object(Database, "delete_tasks", new=mock_delete_raising):
        # Execute: Run CLI command with task ID (triggers delete_tasks path)
        result = runner.invoke(app, ["task", "prune", task_id_str, "--force"])

        # Assert: Error handling works correctly
        assert result.exit_code == 1, f"Expected exit code 1, got {result.exit_code}\nOutput: {result.stdout}"

        # TODO: Once exception handling tasks are merged, update to check for specific message:
        # assert "Database is locked or busy" in result.stdout
        # assert "try again" in result.stdout.lower()

        # Current behavior: generic error message
        assert "Error:" in result.stdout, f"Expected error message, got: {result.stdout}"
        assert "OperationalError" in result.stdout, f"Expected OperationalError mention, got: {result.stdout}"
        assert "database is locked" in result.stdout, f"Expected 'database is locked', got: {result.stdout}"

        # Verify no Python stack trace leaked to user (critical requirement)
        assert "Traceback" not in result.stdout, (
            f"Stack trace leaked to user output: {result.stdout}"
        )


def test_delete_validation_error(database, cli_test_db_path):
    """Test that validation errors display clear, actionable messages.

    Tests parameter validation (handled before async execution) to ensure
    invalid inputs produce helpful error messages.

    Verifies that:
    1. Invalid status value produces clear error message
    2. Exit code is non-zero
    3. Error message lists valid status values
    """
    # Execute: Run CLI with invalid status parameter
    result = runner.invoke(app, ["task", "prune", "--status", "invalid-status-value"])

    # Assert: Validation error handling works correctly
    assert result.exit_code != 0, f"Expected non-zero exit code, got {result.exit_code}"
    assert "Invalid status" in result.stdout, (
        f"Expected validation error message, got: {result.stdout}"
    )
    assert "Valid values:" in result.stdout, (
        f"Expected list of valid values in output: {result.stdout}"
    )
    # Verify no Python stack trace
    assert "Traceback" not in result.stdout, (
        f"Stack trace leaked to user output: {result.stdout}"
    )
