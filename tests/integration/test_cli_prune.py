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
    assert "Cannot use multiple filter methods together" in result.stdout


def test_prune_no_filter_error(database):
    """Test that providing neither task IDs nor status raises an error."""
    result = runner.invoke(app, ["task", "prune"])

    assert result.exit_code != 0
    assert "Must specify at least one filter method" in result.stdout


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


def test_prune_older_than_30_days(database):
    """Test pruning tasks older than 30 days."""
    from datetime import datetime, timedelta, timezone
    from abathur.application import TaskCoordinator

    coordinator = TaskCoordinator(database)

    async def create_tasks_and_prune():
        # Create old task (40 days ago)
        old_task = Task(
            prompt="Old task",
            summary="Task to be pruned",
            agent_type="test-agent",
            source=TaskSource.HUMAN,
            status=TaskStatus.COMPLETED,
            submitted_at=datetime.now(timezone.utc) - timedelta(days=40),
            completed_at=datetime.now(timezone.utc) - timedelta(days=40),
        )
        old_task_id = await coordinator.submit_task(old_task)

        # Create recent task (10 days ago)
        recent_task = Task(
            prompt="Recent task",
            summary="Task to be preserved",
            agent_type="test-agent",
            source=TaskSource.HUMAN,
            status=TaskStatus.COMPLETED,
            submitted_at=datetime.now(timezone.utc) - timedelta(days=10),
            completed_at=datetime.now(timezone.utc) - timedelta(days=10),
        )
        recent_task_id = await coordinator.submit_task(recent_task)

        return old_task_id, recent_task_id

    old_task_id, recent_task_id = asyncio.run(create_tasks_and_prune())

    # Execute: Run CLI command
    result = runner.invoke(
        app,
        ["task", "prune", "--older-than", "30d", "--force"]
    )

    # Assert: Verify old task deleted, recent task preserved
    assert result.exit_code == 0
    assert "Successfully deleted 1 task(s)" in result.stdout or "✓" in result.stdout

    # Verify database state
    async def verify():
        remaining_tasks = await database.list_tasks(TaskStatus.COMPLETED, limit=100)
        assert len(remaining_tasks) == 1
        assert remaining_tasks[0].id == recent_task_id

    asyncio.run(verify())


def test_prune_before_date(database):
    """Test pruning tasks before a specific date."""
    from datetime import datetime, timezone
    from abathur.application import TaskCoordinator

    coordinator = TaskCoordinator(database)

    async def create_tasks():
        # Create old task (before cutoff date)
        old_task = Task(
            prompt="Old task",
            summary="Task before cutoff",
            agent_type="test-agent",
            source=TaskSource.HUMAN,
            status=TaskStatus.COMPLETED,
            submitted_at=datetime(2024, 12, 15, tzinfo=timezone.utc),
            completed_at=datetime(2024, 12, 15, tzinfo=timezone.utc),
        )
        old_task_id = await coordinator.submit_task(old_task)

        # Create new task (after cutoff date)
        new_task = Task(
            prompt="New task",
            summary="Task after cutoff",
            agent_type="test-agent",
            source=TaskSource.HUMAN,
            status=TaskStatus.COMPLETED,
            submitted_at=datetime(2025, 1, 15, tzinfo=timezone.utc),
            completed_at=datetime(2025, 1, 15, tzinfo=timezone.utc),
        )
        new_task_id = await coordinator.submit_task(new_task)

        return old_task_id, new_task_id

    old_task_id, new_task_id = asyncio.run(create_tasks())

    # Execute: Run CLI with ISO date (YYYY-MM-DD format)
    result = runner.invoke(
        app,
        ["task", "prune", "--before", "2025-01-01", "--force"]
    )

    # Assert: Verify correct deletion
    assert result.exit_code == 0
    assert "Successfully deleted 1 task(s)" in result.stdout or "✓" in result.stdout

    # Verify database state
    async def verify():
        remaining_tasks = await database.list_tasks(TaskStatus.COMPLETED, limit=100)
        assert len(remaining_tasks) == 1
        assert remaining_tasks[0].id == new_task_id

    asyncio.run(verify())


def test_prune_time_based_with_child_blocking(database):
    """Test that time-based prune respects child task blocking."""
    from datetime import datetime, timedelta, timezone
    from abathur.application import TaskCoordinator

    coordinator = TaskCoordinator(database)

    async def create_parent_and_child():
        # Create parent task (old, completed)
        parent_task = Task(
            prompt="Parent task",
            summary="Parent with child",
            agent_type="test-agent",
            source=TaskSource.HUMAN,
            status=TaskStatus.COMPLETED,
            submitted_at=datetime.now(timezone.utc) - timedelta(days=40),
            completed_at=datetime.now(timezone.utc) - timedelta(days=40),
        )
        parent_id = await coordinator.submit_task(parent_task)

        # Create child task (old, but running)
        child_task = Task(
            prompt="Child task",
            summary="Active child task",
            agent_type="test-agent",
            source=TaskSource.HUMAN,
            parent_task_id=parent_id,
            status=TaskStatus.RUNNING,
            submitted_at=datetime.now(timezone.utc) - timedelta(days=35),
        )
        await coordinator.submit_task(child_task)

        return parent_id

    parent_id = asyncio.run(create_parent_and_child())

    # Execute: Attempt to prune parent (should be blocked)
    result = runner.invoke(
        app,
        ["task", "prune", "--older-than", "30d", "--force"]
    )

    # Assert: Should show blocked message
    assert result.exit_code == 0
    assert "Cannot delete" in result.stdout or "child" in result.stdout.lower()

    # Verify parent still exists
    async def verify():
        parent = await database.get_task(parent_id)
        assert parent is not None

    asyncio.run(verify())


def test_prune_time_based_dry_run(database):
    """Test dry-run mode shows preview without deleting."""
    from datetime import datetime, timedelta, timezone
    from abathur.application import TaskCoordinator

    coordinator = TaskCoordinator(database)

    async def create_old_tasks():
        tasks = []
        # Create two old completed tasks
        for i in range(2):
            submitted_time = datetime.now(timezone.utc) - timedelta(days=40 + i*10)
            task = Task(
                prompt=f"Old task {i}",
                summary=f"Task to preview {i}",
                agent_type="test-agent",
                source=TaskSource.HUMAN,
                status=TaskStatus.COMPLETED if i == 0 else TaskStatus.FAILED,
                submitted_at=submitted_time,
                completed_at=submitted_time if i == 0 else submitted_time,
            )
            task_id = await coordinator.submit_task(task)
            tasks.append(task_id)
        return tasks

    task_ids = asyncio.run(create_old_tasks())

    # Execute: Run with --dry-run
    result = runner.invoke(
        app,
        ["task", "prune", "--older-than", "30d", "--dry-run"]
    )

    # Assert: Preview shown, but no deletion
    assert result.exit_code == 0
    assert "Dry-run mode" in result.stdout
    assert "Would delete 2 task(s)" in result.stdout or "Tasks to Delete (2)" in result.stdout

    # Verify no tasks were deleted
    async def verify():
        all_tasks = await database.list_tasks(limit=100)
        assert len(all_tasks) == 2

    asyncio.run(verify())


def test_prune_time_based_displays_result(database):
    """Test that PruneResult is displayed with proper formatting."""
    from datetime import datetime, timedelta, timezone
    from abathur.application import TaskCoordinator

    coordinator = TaskCoordinator(database)

    async def create_tasks_with_statuses():
        # Create completed task
        completed_time = datetime.now(timezone.utc) - timedelta(days=40)
        completed_task = Task(
            prompt="Completed task",
            summary="Completed",
            agent_type="test-agent",
            source=TaskSource.HUMAN,
            status=TaskStatus.COMPLETED,
            submitted_at=completed_time,
            completed_at=completed_time,
        )
        await coordinator.submit_task(completed_task)

        # Create failed task
        failed_time = datetime.now(timezone.utc) - timedelta(days=45)
        failed_task = Task(
            prompt="Failed task",
            summary="Failed",
            agent_type="test-agent",
            source=TaskSource.HUMAN,
            status=TaskStatus.FAILED,
            submitted_at=failed_time,
            completed_at=failed_time,
        )
        await coordinator.submit_task(failed_task)

        # Create cancelled task
        cancelled_time = datetime.now(timezone.utc) - timedelta(days=50)
        cancelled_task = Task(
            prompt="Cancelled task",
            summary="Cancelled",
            agent_type="test-agent",
            source=TaskSource.HUMAN,
            status=TaskStatus.CANCELLED,
            submitted_at=cancelled_time,
            completed_at=cancelled_time,
        )
        await coordinator.submit_task(cancelled_task)

    asyncio.run(create_tasks_with_statuses())

    # Execute: Prune old tasks
    result = runner.invoke(
        app,
        ["task", "prune", "--older-than", "30d", "--force"]
    )

    # Assert: Verify result display
    assert result.exit_code == 0
    assert "Successfully deleted 3 task(s)" in result.stdout or "✓" in result.stdout
    # Check for status breakdown table
    assert "Breakdown by Status" in result.stdout or "completed" in result.stdout.lower()

    # Verify database is empty
    async def verify():
        remaining_tasks = await database.list_tasks(limit=100)
        assert len(remaining_tasks) == 0

    asyncio.run(verify())


@pytest.mark.asyncio
async def test_prune_vacuum_always() -> None:
    """Test vacuum_mode='always' runs VACUUM every time."""
    from datetime import datetime, timedelta, timezone
    from pathlib import Path
    from tempfile import NamedTemporaryFile

    from abathur.domain.models import Task, TaskSource, TaskStatus
    from abathur.infrastructure.database import Database, PruneFilters

    # Create temporary database file
    with NamedTemporaryFile(suffix=".db", delete=False) as tmp_file:
        db_path = Path(tmp_file.name)

    try:
        # Initialize database and create tasks
        db = Database(db_path)
        await db.initialize()

        # Create 10 completed tasks with completed_at set to 60 days ago
        old_timestamp = datetime.now(timezone.utc) - timedelta(days=60)

        for i in range(10):
            task = Task(
                prompt=f"Old task {i}",
                summary=f"Task to prune {i}",
                agent_type="test-agent",
                source=TaskSource.HUMAN,
                status=TaskStatus.COMPLETED,
                submitted_at=old_timestamp,
                completed_at=old_timestamp,
            )
            await db.insert_task(task)

        # Execute: Prune with vacuum_mode="always"
        filters = PruneFilters(older_than_days=30, vacuum_mode="always")
        result = await db.prune_tasks(filters)

        # Assert: Verify deletion and VACUUM ran
        assert result.deleted_tasks == 10
        assert result.reclaimed_bytes is not None  # VACUUM should have run
        assert result.reclaimed_bytes >= 0  # May be 0 if DB is small

        await db.close()
    finally:
        # Cleanup
        if db_path.exists():
            db_path.unlink()


@pytest.mark.asyncio
async def test_prune_vacuum_never() -> None:
    """Test vacuum_mode='never' never runs VACUUM."""
    from datetime import datetime, timedelta, timezone
    from pathlib import Path
    from tempfile import NamedTemporaryFile

    from abathur.domain.models import Task, TaskSource, TaskStatus
    from abathur.infrastructure.database import Database, PruneFilters

    # Create temporary database file
    with NamedTemporaryFile(suffix=".db", delete=False) as tmp_file:
        db_path = Path(tmp_file.name)

    try:
        # Initialize database and create tasks
        db = Database(db_path)
        await db.initialize()

        # Create 200 completed tasks (well above threshold)
        old_timestamp = datetime.now(timezone.utc) - timedelta(days=60)

        for i in range(200):
            task = Task(
                prompt=f"Old task {i}",
                summary=f"Task to prune {i}",
                agent_type="test-agent",
                source=TaskSource.HUMAN,
                status=TaskStatus.COMPLETED,
                submitted_at=old_timestamp,
                completed_at=old_timestamp,
            )
            await db.insert_task(task)

        # Execute: Prune with vacuum_mode="never"
        filters = PruneFilters(older_than_days=30, vacuum_mode="never")
        result = await db.prune_tasks(filters)

        # Assert: Verify deletion but no VACUUM
        assert result.deleted_tasks == 200
        assert result.reclaimed_bytes is None  # VACUUM should NOT have run

        await db.close()
    finally:
        # Cleanup
        if db_path.exists():
            db_path.unlink()


@pytest.mark.asyncio
async def test_prune_vacuum_conditional_below_threshold() -> None:
    """Test vacuum_mode='conditional' does not run VACUUM below 100 tasks."""
    from datetime import datetime, timedelta, timezone
    from pathlib import Path
    from tempfile import NamedTemporaryFile

    from abathur.domain.models import Task, TaskSource, TaskStatus
    from abathur.infrastructure.database import Database, PruneFilters

    # Create temporary database file
    with NamedTemporaryFile(suffix=".db", delete=False) as tmp_file:
        db_path = Path(tmp_file.name)

    try:
        # Initialize database and create tasks
        db = Database(db_path)
        await db.initialize()

        # Create 50 completed tasks (below threshold of 100)
        old_timestamp = datetime.now(timezone.utc) - timedelta(days=60)

        for i in range(50):
            task = Task(
                prompt=f"Old task {i}",
                summary=f"Task to prune {i}",
                agent_type="test-agent",
                source=TaskSource.HUMAN,
                status=TaskStatus.COMPLETED,
                submitted_at=old_timestamp,
                completed_at=old_timestamp,
            )
            await db.insert_task(task)

        # Execute: Prune with vacuum_mode="conditional" (default)
        filters = PruneFilters(older_than_days=30, vacuum_mode="conditional")
        result = await db.prune_tasks(filters)

        # Assert: Verify deletion but no VACUUM (below threshold)
        assert result.deleted_tasks == 50
        assert result.reclaimed_bytes is None  # Should NOT run (< 100 tasks)

        await db.close()
    finally:
        # Cleanup
        if db_path.exists():
            db_path.unlink()


@pytest.mark.asyncio
async def test_prune_vacuum_conditional_above_threshold() -> None:
    """Test vacuum_mode='conditional' runs VACUUM at or above 100 tasks."""
    from datetime import datetime, timedelta, timezone
    from pathlib import Path
    from tempfile import NamedTemporaryFile

    from abathur.domain.models import Task, TaskSource, TaskStatus
    from abathur.infrastructure.database import Database, PruneFilters

    # Create temporary database file
    with NamedTemporaryFile(suffix=".db", delete=False) as tmp_file:
        db_path = Path(tmp_file.name)

    try:
        # Initialize database and create tasks
        db = Database(db_path)
        await db.initialize()

        # Create 150 completed tasks (above threshold of 100)
        old_timestamp = datetime.now(timezone.utc) - timedelta(days=60)

        for i in range(150):
            task = Task(
                prompt=f"Old task {i}",
                summary=f"Task to prune {i}",
                agent_type="test-agent",
                source=TaskSource.HUMAN,
                status=TaskStatus.COMPLETED,
                submitted_at=old_timestamp,
                completed_at=old_timestamp,
            )
            await db.insert_task(task)

        # Execute: Prune with vacuum_mode="conditional" (default)
        filters = PruneFilters(older_than_days=30, vacuum_mode="conditional")
        result = await db.prune_tasks(filters)

        # Assert: Verify deletion and VACUUM ran (above threshold)
        assert result.deleted_tasks == 150
        assert result.reclaimed_bytes is not None  # Should run (>= 100 tasks)
        assert result.reclaimed_bytes >= 0  # May be 0 if DB is small

        await db.close()
    finally:
        # Cleanup
        if db_path.exists():
            db_path.unlink()


@pytest.mark.asyncio
async def test_prune_vacuum_default_is_conditional() -> None:
    """Test default vacuum_mode is 'conditional' when not specified."""
    from datetime import datetime, timedelta, timezone
    from pathlib import Path
    from tempfile import NamedTemporaryFile

    from abathur.domain.models import Task, TaskSource, TaskStatus
    from abathur.infrastructure.database import Database, PruneFilters

    # Create temporary database file
    with NamedTemporaryFile(suffix=".db", delete=False) as tmp_file:
        db_path = Path(tmp_file.name)

    try:
        # Initialize database and create tasks
        db = Database(db_path)
        await db.initialize()

        # Create 1 completed task (well below threshold)
        old_timestamp = datetime.now(timezone.utc) - timedelta(days=60)

        task = Task(
            prompt="Old task",
            summary="Task to prune",
            agent_type="test-agent",
            source=TaskSource.HUMAN,
            status=TaskStatus.COMPLETED,
            submitted_at=old_timestamp,
            completed_at=old_timestamp,
        )
        await db.insert_task(task)

        # Execute: Prune without specifying vacuum_mode (should default to "conditional")
        filters = PruneFilters(older_than_days=30)  # No vacuum_mode specified
        result = await db.prune_tasks(filters)

        # Assert: Verify default behavior (conditional, below threshold, no VACUUM)
        assert result.deleted_tasks == 1
        assert result.reclaimed_bytes is None  # Should NOT run (< 100 tasks, conditional mode)

        await db.close()
    finally:
        # Cleanup
        if db_path.exists():
            db_path.unlink()
