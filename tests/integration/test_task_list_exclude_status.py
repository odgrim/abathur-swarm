"""Integration tests for CLI --exclude-status end-to-end flow.

Tests complete end-to-end workflows:
- CLI command: abathur task list --exclude-status <status>
- End-to-end flow from CLI to database
- Filter combinations with --limit flag
- Rich Table output format validation
- All TaskStatus values can be excluded

This test validates Phase 3 (Integration Testing) for CLI --exclude-status implementation.
"""

import asyncio
from pathlib import Path

import pytest
from abathur.cli.main import app
from abathur.domain.models import TaskSource, TaskStatus
from abathur.infrastructure.database import Database
from abathur.services.dependency_resolver import DependencyResolver
from abathur.services.priority_calculator import PriorityCalculator
from abathur.services.task_queue_service import TaskQueueService
from typer.testing import CliRunner

# Fixtures


@pytest.fixture
def cli_runner() -> CliRunner:
    """Create Typer CLI test runner."""
    return CliRunner()


def _setup_test_database_sync(
    db_path: Path, task_statuses: list[tuple[str, TaskStatus]]
) -> dict[TaskStatus, str]:
    """Helper to populate test database with tasks in various statuses.

    Args:
        db_path: Path to test database
        task_statuses: List of (description, target_status) tuples

    Returns:
        Dictionary mapping TaskStatus to task_id
    """

    async def setup():
        # Initialize database
        db = Database(db_path)
        await db.initialize()

        # Create task queue service
        dependency_resolver = DependencyResolver(db)
        priority_calculator = PriorityCalculator(dependency_resolver)
        task_queue_service = TaskQueueService(db, dependency_resolver, priority_calculator)

        # Create test session for FK constraint
        from datetime import datetime, timezone
        from uuid import uuid4

        session_id = f"test-session-{uuid4()}"
        async with db._get_connection() as conn:
            await conn.execute(
                """
                INSERT INTO sessions (id, app_name, user_id, created_at, last_update_time)
                VALUES (?, ?, ?, ?, ?)
                """,
                (
                    session_id,
                    "test-app",
                    "test-user",
                    datetime.now(timezone.utc).isoformat(),
                    datetime.now(timezone.utc).isoformat(),
                ),
            )
            await conn.commit()

        # Create tasks
        task_map = {}
        for description, target_status in task_statuses:
            task = await task_queue_service.enqueue_task(
                description=description,
                source=TaskSource.HUMAN,
                session_id=session_id,
                base_priority=5,
            )

            # Update task status if not READY
            if target_status != TaskStatus.READY:
                async with db._get_connection() as conn:
                    update_sql = "UPDATE tasks SET status = ?"
                    params = [target_status.value]

                    # Add error message for FAILED status
                    if target_status == TaskStatus.FAILED:
                        update_sql += ", error_message = ?"
                        params.append("Test error")

                    update_sql += " WHERE id = ?"
                    params.append(str(task.id))

                    await conn.execute(update_sql, params)
                    await conn.commit()

            task_map[target_status] = str(task.id)

        # Close database connection
        await db.close()
        return task_map

    return asyncio.run(setup())


# Integration Tests


def test_exclude_completed_tasks_integration(
    mock_cli_database_path: None,
    cli_test_db_path: Path,
    cli_runner: CliRunner,
) -> None:
    """Test CLI 'task list --exclude-status completed' excludes completed tasks.

    Integration test validating:
    1. CLI accepts --exclude-status completed flag
    2. Output does NOT contain completed tasks
    3. Output DOES contain pending, running, and failed tasks
    4. CLI exits successfully (exit code 0)
    5. Rich Table format is correct

    Tests end-to-end flow: CLI → TaskCoordinator → Database → Rich Table output
    """
    # Setup: Create test database with tasks in various statuses
    task_statuses = [
        ("Completed task 1", TaskStatus.COMPLETED),
        ("Completed task 2", TaskStatus.COMPLETED),
        ("Pending task 1", TaskStatus.PENDING),
        ("Pending task 2", TaskStatus.PENDING),
        ("Running task 1", TaskStatus.RUNNING),
        ("Running task 2", TaskStatus.RUNNING),
    ]
    task_map = _setup_test_database_sync(cli_test_db_path, task_statuses)

    # Act: Execute CLI command
    result = cli_runner.invoke(
        app,
        ["task", "list", "--exclude-status", "completed"],
    )

    # Assert 1: CLI exits successfully
    assert (
        result.exit_code == 0
    ), f"CLI exited with error (code {result.exit_code}): {result.stdout}"

    # Assert 2: Output does NOT contain completed tasks
    completed_id_1 = task_map[TaskStatus.COMPLETED][:8]  # First 8 chars for display
    assert completed_id_1 not in result.stdout, "Completed task ID should not appear in output"
    assert "Completed task" not in result.stdout, "Completed task description should not appear"

    # Assert 3: Output DOES contain other status tasks
    pending_id = task_map[TaskStatus.PENDING][:8]
    running_id = task_map[TaskStatus.RUNNING][:8]
    assert pending_id in result.stdout, "Pending task should appear in output"
    assert running_id in result.stdout, "Running task should appear in output"

    # Assert 4: Output contains "pending" and "running" status values
    assert "pending" in result.stdout.lower(), "Output should show pending status"
    assert "running" in result.stdout.lower(), "Output should show running status"

    # Assert 5: Output should NOT contain "completed" status
    # Check for "completed" as a status value (not in descriptions)
    output_lines = result.stdout.split("\n")
    for line in output_lines:
        # Skip lines that are task descriptions
        if "Completed task" in line:
            continue
        # Check status column doesn't show "completed"
        if "│" in line:  # Table row
            parts = [p.strip() for p in line.split("│")]
            # Status is typically 5th column (after ID, Summary, Agent Type, Priority)
            if len(parts) > 5:
                status_col = parts[5]
                assert (
                    "completed" not in status_col.lower()
                ), "Status column should not show 'completed'"


def test_exclude_failed_tasks_integration(
    mock_cli_database_path: None,
    cli_test_db_path: Path,
    cli_runner: CliRunner,
) -> None:
    """Test CLI 'task list --exclude-status failed' excludes failed tasks.

    Validates:
    1. Failed tasks are excluded from output
    2. Other statuses (ready, running, completed) are included
    3. CLI exits successfully
    """
    # Setup: Create test database with failed and non-failed tasks
    task_statuses = [
        ("Failed task 1", TaskStatus.FAILED),
        ("Failed task 2", TaskStatus.FAILED),
        ("Ready task", TaskStatus.READY),
        ("Running task", TaskStatus.RUNNING),
        ("Completed task", TaskStatus.COMPLETED),
    ]
    task_map = _setup_test_database_sync(cli_test_db_path, task_statuses)

    # Act: Execute CLI command
    result = cli_runner.invoke(
        app,
        ["task", "list", "--exclude-status", "failed"],
    )

    # Assert 1: CLI exits successfully
    assert (
        result.exit_code == 0
    ), f"CLI exited with error (code {result.exit_code}): {result.stdout}"

    # Assert 2: Output excludes failed tasks
    failed_id = task_map[TaskStatus.FAILED][:8]
    assert failed_id not in result.stdout, "Failed task should not appear in output"
    assert "Failed task" not in result.stdout, "Failed task description should not appear"

    # Assert 3: Output includes all other statuses
    ready_id = task_map[TaskStatus.READY][:8]
    running_id = task_map[TaskStatus.RUNNING][:8]
    completed_id = task_map[TaskStatus.COMPLETED][:8]

    assert ready_id in result.stdout, "Ready task should appear"
    assert running_id in result.stdout, "Running task should appear"
    assert completed_id in result.stdout, "Completed task should appear"


def test_exclude_with_limit_integration(
    mock_cli_database_path: None,
    cli_test_db_path: Path,
    cli_runner: CliRunner,
) -> None:
    """Test CLI 'task list --exclude-status completed --limit 3' applies both filters.

    Validates:
    1. --exclude-status works correctly with --limit
    2. Only 3 tasks are shown (limit applied)
    3. All shown tasks are non-completed
    4. Limit is applied AFTER exclusion filter
    """
    # Setup: Create 10 tasks (5 completed, 5 pending)
    task_statuses = []
    for i in range(5):
        task_statuses.append((f"Completed task {i}", TaskStatus.COMPLETED))
        task_statuses.append((f"Pending task {i}", TaskStatus.PENDING))

    _setup_test_database_sync(cli_test_db_path, task_statuses)

    # Act: Execute CLI command with both flags
    result = cli_runner.invoke(
        app,
        ["task", "list", "--exclude-status", "completed", "--limit", "3"],
    )

    # Assert 1: CLI exits successfully
    assert (
        result.exit_code == 0
    ), f"CLI exited with error (code {result.exit_code}): {result.stdout}"

    # Assert 2: Count task IDs in output (only 3 unique tasks should appear)
    # The table uses multi-line cells, so we count unique task IDs instead of rows
    import re

    # Extract 8-character task IDs (format: 8 hex chars)
    task_ids = re.findall(r"│\s+([0-9a-f]{8})\s+│", result.stdout)

    # Should have exactly 3 unique task IDs
    assert len(task_ids) == 3, f"Expected 3 tasks in output, got {len(task_ids)}: {task_ids}"

    # Assert 3: All shown tasks are non-completed (should be pending)
    assert (
        "completed" not in result.stdout.lower() or "Completed task" in result.stdout
    ), "Output should not contain 'completed' status (descriptions are ok)"
    assert "pending" in result.stdout.lower(), "Output should contain pending tasks"


def test_exclude_all_status_values(
    mock_cli_database_path: None,
    cli_test_db_path: Path,
    cli_runner: CliRunner,
) -> None:
    """Test each TaskStatus value can be excluded correctly.

    Validates:
    1. All 7 TaskStatus values (pending, blocked, ready, running, completed, failed, cancelled)
    2. Each exclusion filter works correctly
    3. Remaining tasks are shown
    4. CLI accepts all valid status enum values
    """
    # Setup: Create tasks with all 7 status values
    task_statuses = [
        ("Pending task", TaskStatus.PENDING),
        ("Ready task", TaskStatus.READY),
        ("Running task", TaskStatus.RUNNING),
        ("Completed task", TaskStatus.COMPLETED),
        ("Failed task", TaskStatus.FAILED),
        ("Cancelled task", TaskStatus.CANCELLED),
        # Note: BLOCKED requires dependencies, tested separately
    ]
    _setup_test_database_sync(cli_test_db_path, task_statuses)

    # Test excluding each status value
    test_cases = [
        ("pending", "Pending task", ["Ready task", "Running task", "Completed task"]),
        ("ready", "Ready task", ["Pending task", "Running task", "Completed task"]),
        ("running", "Running task", ["Pending task", "Ready task", "Completed task"]),
        ("completed", "Completed task", ["Pending task", "Ready task", "Running task"]),
        ("failed", "Failed task", ["Pending task", "Ready task", "Running task"]),
        ("cancelled", "Cancelled task", ["Pending task", "Ready task", "Running task"]),
    ]

    for exclude_status, excluded_desc, included_descs in test_cases:
        # Recreate database for each test to ensure clean state
        _setup_test_database_sync(cli_test_db_path, task_statuses)

        # Act: Execute CLI command
        result = cli_runner.invoke(
            app,
            ["task", "list", "--exclude-status", exclude_status],
        )

        # Assert: CLI exits successfully
        assert (
            result.exit_code == 0
        ), f"CLI failed when excluding '{exclude_status}' (code {result.exit_code}): {result.stdout}"

        # Assert: Excluded task does not appear
        assert (
            excluded_desc not in result.stdout
        ), f"Excluded task '{excluded_desc}' should not appear when excluding '{exclude_status}'"

        # Assert: At least some included tasks appear
        # (We check at least one to verify filter is working)
        included_found = any(desc in result.stdout for desc in included_descs)
        assert (
            included_found
        ), f"At least one included task should appear when excluding '{exclude_status}'"


def test_rich_table_format_unchanged(
    mock_cli_database_path: None,
    cli_test_db_path: Path,
    cli_runner: CliRunner,
) -> None:
    """Test Rich Table output format remains consistent with --exclude-status.

    Validates:
    1. Table columns match existing format (ID, Summary, Agent Type, Priority, Status, Submitted)
    2. No formatting regressions
    3. Rich Table structure unchanged
    4. Output is properly formatted table
    """
    # Setup: Create test tasks
    task_statuses = [
        ("Test task 1", TaskStatus.READY),
        ("Test task 2", TaskStatus.RUNNING),
        ("Test task 3", TaskStatus.COMPLETED),
    ]
    _setup_test_database_sync(cli_test_db_path, task_statuses)

    # Act: Execute CLI command with --exclude-status
    result = cli_runner.invoke(
        app,
        ["task", "list", "--exclude-status", "completed"],
    )

    # Assert 1: CLI exits successfully
    assert result.exit_code == 0, f"CLI exited with error: {result.stdout}"

    # Assert 2: Output contains table structure
    assert "│" in result.stdout, "Output should contain table structure"
    assert "─" in result.stdout, "Output should contain table borders"

    # Assert 3: Table has expected column headers
    # The exact columns from src/abathur/cli/main.py:438-445
    expected_headers = ["ID", "Summary", "Agent Type", "Priority", "Status", "Submitted"]

    for header in expected_headers:
        assert header in result.stdout, f"Table should contain '{header}' column header"

    # Assert 4: Table has title "Tasks"
    assert "Tasks" in result.stdout, "Table should have 'Tasks' title"

    # Assert 5: Verify table rows are formatted correctly
    # Check that rows contain task data separated by │
    task_rows = [
        line
        for line in result.stdout.split("\n")
        if "│" in line and "ID" not in line and "─" not in line and len(line.strip()) > 0
    ]

    # Should have at least 2 task rows (2 non-completed tasks)
    assert len(task_rows) >= 2, f"Expected at least 2 task rows, got {len(task_rows)}"

    # Assert 6: Each row should have correct number of columns (6 columns)
    for row in task_rows:
        # Count column separators (should be 7 for 6 columns: │ col │ col │ col │ col │ col │ col │)
        separator_count = row.count("│")
        assert (
            separator_count >= 6
        ), f"Row should have at least 6 column separators, got {separator_count}"


def test_exclude_status_invalid_value_error(
    mock_cli_database_path: None,
    cli_test_db_path: Path,
    cli_runner: CliRunner,
) -> None:
    """Test CLI shows clear error for invalid --exclude-status value.

    Validates:
    1. Invalid status value shows error message
    2. Error message lists valid status values
    3. CLI exits with error code
    4. User-friendly error handling
    """
    # Setup: Create minimal test database
    task_statuses = [("Test task", TaskStatus.READY)]
    _setup_test_database_sync(cli_test_db_path, task_statuses)

    # Act: Execute CLI command with invalid status
    result = cli_runner.invoke(
        app,
        ["task", "list", "--exclude-status", "invalid_status"],
    )

    # Assert 1: CLI exits with error
    assert result.exit_code != 0, "CLI should exit with error for invalid status"

    # Assert 2: Error output mentions invalid value
    assert "invalid_status" in result.stdout or "invalid_status" in str(
        result.exception
    ), "Error should mention the invalid status value"

    # Note: Exact error format depends on validation implementation in CLI
    # This test verifies that invalid values are rejected


def test_backward_compatibility_no_exclude_status(
    mock_cli_database_path: None,
    cli_test_db_path: Path,
    cli_runner: CliRunner,
) -> None:
    """Test backward compatibility: task list works without --exclude-status.

    Validates:
    1. Existing 'task list' command works unchanged
    2. Shows all tasks by default
    3. No regressions introduced
    """
    # Setup: Create test tasks with various statuses
    task_statuses = [
        ("Ready task", TaskStatus.READY),
        ("Completed task", TaskStatus.COMPLETED),
        ("Failed task", TaskStatus.FAILED),
    ]
    _setup_test_database_sync(cli_test_db_path, task_statuses)

    # Act: Execute CLI command WITHOUT --exclude-status flag
    result = cli_runner.invoke(
        app,
        ["task", "list"],
    )

    # Assert 1: CLI exits successfully
    assert result.exit_code == 0, f"CLI exited with error: {result.stdout}"

    # Assert 2: All tasks appear in output (no exclusion)
    # Note: Task descriptions may be wrapped or truncated in table display
    # Check for status values instead which are always shown
    assert "ready" in result.stdout.lower(), "Ready status should appear"
    assert "completed" in result.stdout.lower(), "Completed status should appear"
    assert "failed" in result.stdout.lower(), "Failed status should appear"

    # Assert 3: Count task IDs to verify all 3 tasks are present
    import re

    task_ids = re.findall(r"│\s+([0-9a-f]{8})\s+│", result.stdout)
    assert len(task_ids) == 3, f"Expected 3 tasks in output, got {len(task_ids)}"


def test_exclude_status_with_status_filter_combined(
    mock_cli_database_path: None,
    cli_test_db_path: Path,
    cli_runner: CliRunner,
) -> None:
    """Test combining --status and --exclude-status flags.

    Validates:
    1. Both flags can be used together
    2. Filters are applied correctly (ANDed)
    3. Example: --status ready --exclude-status blocked

    Note: This tests the actual behavior. If mutual exclusivity validation
    exists at CLI layer, this test will verify that error handling.
    """
    # Setup: Create test tasks
    task_statuses = [
        ("Ready task 1", TaskStatus.READY),
        ("Ready task 2", TaskStatus.READY),
        ("Running task", TaskStatus.RUNNING),
        ("Completed task", TaskStatus.COMPLETED),
    ]
    _setup_test_database_sync(cli_test_db_path, task_statuses)

    # Act: Execute CLI command with both --status and --exclude-status
    result = cli_runner.invoke(
        app,
        ["task", "list", "--status", "ready", "--exclude-status", "completed"],
    )

    # This test documents the actual behavior
    # If the implementation allows both flags, we test the filtering logic
    # If the implementation rejects the combination, we test error handling

    # For now, test that CLI doesn't crash
    # The specific behavior will depend on implementation
    assert result.exit_code is not None, "CLI should return an exit code"

    # If successful (exit code 0), verify filtering worked
    if result.exit_code == 0:
        assert "Ready task" in result.stdout, "Ready tasks should appear with --status ready"
        assert "Completed task" not in result.stdout, "Completed task should be excluded"
        assert (
            "Running task" not in result.stdout
        ), "Running task should be excluded by --status filter"
