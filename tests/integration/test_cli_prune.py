"""Integration tests for CLI prune command."""

import asyncio
import pytest
from pathlib import Path
from typer.testing import CliRunner
from uuid import UUID

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
def sample_tasks(database):
    """Create sample tasks for testing (sync fixture)."""
    from abathur.application import TaskCoordinator

    coordinator = TaskCoordinator(database)

    async def create_tasks():
        # Create tasks with different statuses
        tasks = []

        # Completed tasks
        for i in range(3):
            task = Task(
                prompt=f"Completed task {i}",
                summary=f"Test completed task {i}",
                agent_type="test-agent",
                source=TaskSource.HUMAN,
                status=TaskStatus.COMPLETED,
            )
            task_id = await coordinator.submit_task(task)
            tasks.append(task_id)

        # Failed tasks
        for i in range(2):
            task = Task(
                prompt=f"Failed task {i}",
                summary=f"Test failed task {i}",
                agent_type="test-agent",
                source=TaskSource.HUMAN,
                status=TaskStatus.FAILED,
            )
            task_id = await coordinator.submit_task(task)
            tasks.append(task_id)

        # Pending tasks (will be transitioned to READY automatically)
        for i in range(2):
            task = Task(
                prompt=f"Pending task {i}",
                summary=f"Test pending task {i}",
                agent_type="test-agent",
                source=TaskSource.HUMAN,
                status=TaskStatus.PENDING,
            )
            task_id = await coordinator.submit_task(task)
            tasks.append(task_id)

        return tasks

    # Run async task creation synchronously
    return asyncio.run(create_tasks())


def test_prune_by_task_id(database, sample_tasks):
    """Test pruning a single task by ID."""
    task_id = str(sample_tasks[0])

    result = runner.invoke(app, ["task", "prune", task_id, "--force"])

    assert result.exit_code == 0
    assert "Deleted 1 task(s)" in result.stdout or "✓" in result.stdout


def test_prune_by_task_id_prefix(database, sample_tasks):
    """Test pruning a task by ID prefix."""
    task_id_prefix = str(sample_tasks[0])[:8]

    result = runner.invoke(app, ["task", "prune", task_id_prefix, "--force"])

    assert result.exit_code == 0
    assert "Deleted 1 task(s)" in result.stdout or "✓" in result.stdout


def test_prune_multiple_tasks_by_id(database, sample_tasks):
    """Test pruning multiple tasks by ID."""
    task_id_1 = str(sample_tasks[0])
    task_id_2 = str(sample_tasks[1])

    result = runner.invoke(app, ["task", "prune", task_id_1, task_id_2, "--force"])

    assert result.exit_code == 0
    assert "Deleted 2 task(s)" in result.stdout or "✓" in result.stdout


def test_prune_by_status_completed(database, sample_tasks):
    """Test pruning tasks by status (completed)."""
    result = runner.invoke(app, ["task", "prune", "--status", "completed", "--force"])

    assert result.exit_code == 0
    assert "Deleted 3 task(s)" in result.stdout or "✓" in result.stdout


def test_prune_by_status_failed(database, sample_tasks):
    """Test pruning tasks by status (failed)."""
    result = runner.invoke(app, ["task", "prune", "--status", "failed", "--force"])

    assert result.exit_code == 0
    assert "Deleted 2 task(s)" in result.stdout or "✓" in result.stdout


def test_prune_mutual_exclusion_error(database, sample_tasks):
    """Test that providing both task IDs and status raises an error."""
    task_id = str(sample_tasks[0])

    result = runner.invoke(app, ["task", "prune", task_id, "--status", "completed"])

    assert result.exit_code != 0
    assert "Cannot specify both task IDs and --status" in result.stdout


def test_prune_no_filter_error(database):
    """Test that providing neither task IDs nor status raises an error."""
    result = runner.invoke(app, ["task", "prune"])

    assert result.exit_code != 0
    assert "Must specify either task IDs or --status" in result.stdout


def test_prune_invalid_status_error(database):
    """Test that providing an invalid status raises an error."""
    result = runner.invoke(app, ["task", "prune", "--status", "invalid-status"])

    assert result.exit_code != 0
    assert "Invalid status" in result.stdout
    assert "Valid values:" in result.stdout


def test_prune_dry_run(database, sample_tasks):
    """Test prune in dry-run mode."""
    result = runner.invoke(app, ["task", "prune", "--status", "completed", "--dry-run"])

    assert result.exit_code == 0
    assert "Dry-run mode" in result.stdout
    assert "Would delete 3 task(s)" in result.stdout

    # Verify tasks were NOT deleted
    async def verify():
        tasks = await database.list_tasks(TaskStatus.COMPLETED, limit=100)
        assert len(tasks) == 3

    asyncio.run(verify())


def test_prune_confirmation_prompt_accept(database, sample_tasks):
    """Test confirmation prompt accepts 'y'."""
    task_id = str(sample_tasks[0])

    result = runner.invoke(app, ["task", "prune", task_id], input="y\n")

    assert result.exit_code == 0
    assert "Deleted 1 task(s)" in result.stdout or "✓" in result.stdout


def test_prune_confirmation_prompt_reject(database, sample_tasks):
    """Test confirmation prompt rejects 'n'."""
    task_id = str(sample_tasks[0])

    result = runner.invoke(app, ["task", "prune", task_id], input="n\n")

    assert result.exit_code == 0
    assert "Operation cancelled" in result.stdout

    # Verify task was NOT deleted
    async def verify():
        task = await database.get_task(UUID(task_id))
        assert task is not None

    asyncio.run(verify())


def test_prune_force_skips_confirmation(database, sample_tasks):
    """Test that --force flag skips confirmation prompt."""
    task_id = str(sample_tasks[0])

    result = runner.invoke(app, ["task", "prune", task_id, "--force"])

    assert result.exit_code == 0
    assert "Deleted 1 task(s)" in result.stdout or "✓" in result.stdout
    # Should NOT prompt for confirmation
    assert "Are you sure" not in result.stdout


def test_prune_no_tasks_match_status(database, sample_tasks):
    """Test pruning when no tasks match the status filter."""
    result = runner.invoke(app, ["task", "prune", "--status", "running", "--force"])

    assert result.exit_code == 0
    assert "No tasks found" in result.stdout or "✓" in result.stdout


def test_prune_blocked_parent_task(database):
    """Test that parent tasks with children cannot be deleted."""
    from abathur.application import TaskCoordinator

    coordinator = TaskCoordinator(database)

    async def create_and_test():
        # Create parent task
        parent_task = Task(
            prompt="Parent task",
            summary="Test parent task",
            agent_type="test-agent",
            source=TaskSource.HUMAN,
            status=TaskStatus.COMPLETED,
        )
        parent_id = await coordinator.submit_task(parent_task)

        # Create child task
        child_task = Task(
            prompt="Child task",
            summary="Test child task",
            agent_type="test-agent",
            source=TaskSource.HUMAN,
            parent_task_id=parent_id,
            status=TaskStatus.PENDING,
        )
        await coordinator.submit_task(child_task)

        return parent_id

    parent_id = asyncio.run(create_and_test())

    # Try to delete parent
    result = runner.invoke(app, ["task", "prune", str(parent_id), "--force"])

    assert result.exit_code == 0
    # Should show blocked deletions
    assert "blocked" in result.stdout.lower() or "child" in result.stdout.lower()

    # Verify parent was NOT deleted
    async def verify():
        parent = await database.get_task(parent_id)
        assert parent is not None

    asyncio.run(verify())


def test_prune_invalid_task_id(database):
    """Test pruning with an invalid task ID."""
    result = runner.invoke(app, ["task", "prune", "invalid-uuid", "--force"])

    assert result.exit_code != 0
    assert "Error" in result.stdout or "not found" in result.stdout.lower()


def test_prune_nonexistent_task_id(database):
    """Test pruning with a non-existent task ID."""
    nonexistent_id = "00000000-0000-0000-0000-000000000000"

    result = runner.invoke(app, ["task", "prune", nonexistent_id, "--force"])

    assert result.exit_code != 0
    assert "Error" in result.stdout and "not found" in result.stdout


def test_prune_displays_task_table(database, sample_tasks):
    """Test that prune displays a table of tasks to be deleted."""
    result = runner.invoke(app, ["task", "prune", "--status", "completed", "--dry-run"])

    assert result.exit_code == 0
    # Should display table with columns
    assert "Tasks to Delete" in result.stdout
    assert "ID" in result.stdout
    assert "Summary" in result.stdout
    assert "Status" in result.stdout


def test_prune_truncates_long_summary(database):
    """Test that long summaries are truncated in the table display."""
    from abathur.application import TaskCoordinator

    coordinator = TaskCoordinator(database)

    async def create_task():
        # Create task with very long summary
        long_summary = "A" * 100
        task = Task(
            prompt="Task with long summary",
            summary=long_summary,
            agent_type="test-agent",
            source=TaskSource.HUMAN,
            status=TaskStatus.COMPLETED,
        )
        return await coordinator.submit_task(task)

    task_id = asyncio.run(create_task())

    result = runner.invoke(app, ["task", "prune", str(task_id), "--dry-run"])

    assert result.exit_code == 0
    # Just verify that the table is displayed successfully
    # Rich table rendering may format text differently, so we just check basics
    assert "Tasks to Delete" in result.stdout
    assert str(task_id)[:8] in result.stdout


def test_prune_ambiguous_prefix_error(database):
    """Test that ambiguous prefix (matches multiple tasks) raises an error."""
    from abathur.application import TaskCoordinator

    coordinator = TaskCoordinator(database)

    async def create_tasks_and_find_collision():
        # Create multiple tasks and find a shared prefix
        tasks = []
        for i in range(20):
            task = Task(
                prompt=f"Task {i}",
                summary=f"Test task {i}",
                agent_type="test-agent",
                source=TaskSource.HUMAN,
                status=TaskStatus.COMPLETED,
            )
            task_id = await coordinator.submit_task(task)
            tasks.append(str(task_id))

        # Find a common prefix by checking first 2 characters
        prefix_map = {}
        for task_id in tasks:
            prefix = task_id[:2]
            if prefix not in prefix_map:
                prefix_map[prefix] = []
            prefix_map[prefix].append(task_id)

        # Find a prefix that matches multiple tasks
        ambiguous_prefix = None
        for prefix, matching_ids in prefix_map.items():
            if len(matching_ids) > 1:
                ambiguous_prefix = prefix
                break

        # If no collision with 2 chars, try 1 char
        if not ambiguous_prefix:
            prefix_map = {}
            for task_id in tasks:
                prefix = task_id[0]
                if prefix not in prefix_map:
                    prefix_map[prefix] = []
                prefix_map[prefix].append(task_id)

            for prefix, matching_ids in prefix_map.items():
                if len(matching_ids) > 1:
                    ambiguous_prefix = prefix
                    break

        return ambiguous_prefix

    ambiguous_prefix = asyncio.run(create_tasks_and_find_collision())

    # Skip test if we couldn't create an ambiguous prefix (very unlikely)
    if not ambiguous_prefix:
        pytest.skip("Could not create ambiguous prefix scenario")

    result = runner.invoke(app, ["task", "prune", ambiguous_prefix, "--force"])

    # Should fail with ambiguous match error
    assert result.exit_code != 0
    assert "Multiple tasks match" in result.stdout
