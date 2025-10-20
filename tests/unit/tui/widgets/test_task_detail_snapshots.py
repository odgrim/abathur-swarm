"""Visual snapshot tests for TaskDetailPanel widget.

Tests visual rendering and layout using Textual's snapshot testing feature.
Snapshots capture the exact terminal output for regression testing.
"""

import pytest
from datetime import datetime, timezone, timedelta
from uuid import UUID

from textual.app import App, ComposeResult

from abathur.domain.models import Task, TaskStatus, TaskSource, DependencyType
from abathur.tui.widgets.task_detail import TaskDetailPanel


class SnapshotTestApp(App):
    """Test app for snapshot testing."""

    def compose(self) -> ComposeResult:
        yield TaskDetailPanel()


# Fixed UUIDs for consistent snapshots
TASK_ID = UUID("12345678-1234-5678-1234-567812345678")
PARENT_ID = UUID("87654321-4321-8765-4321-876543218765")
DEP_ID_1 = UUID("11111111-1111-1111-1111-111111111111")
DEP_ID_2 = UUID("22222222-2222-2222-2222-222222222222")

# Fixed datetime for consistent snapshots
FIXED_NOW = datetime(2025, 10, 20, 12, 0, 0, tzinfo=timezone.utc)


@pytest.fixture
def snapshot_task() -> Task:
    """Create a task with fixed values for snapshot testing."""
    return Task(
        id=TASK_ID,
        summary="Implement TaskDetailPanel widget with all 28 fields",
        prompt="Create a comprehensive Textual widget that displays all task metadata",
        agent_type="python-textual-widget-specialist",
        priority=7,
        status=TaskStatus.RUNNING,
        source=TaskSource.AGENT_PLANNER,
        dependency_type=DependencyType.SEQUENTIAL,
        calculated_priority=8.5,
        dependency_depth=2,
        feature_branch="feature/tui-implementation",
        task_branch="task/tui-detail-panel",
        worktree_path="/Users/dev/worktree/tui-detail-panel",
        submitted_at=FIXED_NOW - timedelta(hours=2),
        started_at=FIXED_NOW - timedelta(minutes=30),
        completed_at=None,
        last_updated_at=FIXED_NOW,
        parent_task_id=PARENT_ID,
        dependencies=[DEP_ID_1, DEP_ID_2],
        session_id="test-session-abc123",
        created_by="test-user",
        retry_count=1,
        max_retries=3,
        max_execution_timeout_seconds=3600,
        deadline=FIXED_NOW + timedelta(days=1),
        estimated_duration_seconds=1800,
        input_data={"config": "test", "mode": "debug"},
        result_data=None,
        error_message=None,
    )


@pytest.mark.asyncio
async def test_empty_state_snapshot(snap_compare):
    """Snapshot test for empty state rendering."""
    app = SnapshotTestApp()
    async with app.run_test() as pilot:
        # Capture snapshot of empty state
        assert await snap_compare(app, "empty_state.txt")


@pytest.mark.asyncio
async def test_running_task_snapshot(snap_compare, snapshot_task):
    """Snapshot test for running task rendering."""
    app = SnapshotTestApp()
    async with app.run_test() as pilot:
        panel = app.query_one(TaskDetailPanel)
        panel.task_data = snapshot_task
        await pilot.pause()

        # Capture snapshot of running task
        assert await snap_compare(app, "running_task.txt")


@pytest.mark.asyncio
async def test_completed_task_snapshot(snap_compare):
    """Snapshot test for completed task with results."""
    completed_task = Task(
        id=TASK_ID,
        summary="Completed implementation task",
        prompt="Task that completed successfully",
        status=TaskStatus.COMPLETED,
        completed_at=FIXED_NOW,
        result_data={
            "status": "success",
            "files_created": ["widget.py", "test_widget.py"],
            "test_results": {"passed": 25, "failed": 0},
        },
        submitted_at=FIXED_NOW - timedelta(hours=1),
        started_at=FIXED_NOW - timedelta(minutes=45),
        last_updated_at=FIXED_NOW,
    )

    app = SnapshotTestApp()
    async with app.run_test() as pilot:
        panel = app.query_one(TaskDetailPanel)
        panel.task_data = completed_task
        await pilot.pause()

        # Capture snapshot of completed task
        assert await snap_compare(app, "completed_task_with_results.txt")


@pytest.mark.asyncio
async def test_failed_task_snapshot(snap_compare):
    """Snapshot test for failed task with error message."""
    failed_task = Task(
        id=TASK_ID,
        summary="Failed implementation task",
        prompt="Task that encountered an error",
        status=TaskStatus.FAILED,
        completed_at=FIXED_NOW,
        error_message="ImportError: No module named 'nonexistent_module'. "
        "Task execution failed at line 42.",
        submitted_at=FIXED_NOW - timedelta(hours=1),
        started_at=FIXED_NOW - timedelta(minutes=30),
        last_updated_at=FIXED_NOW,
    )

    app = SnapshotTestApp()
    async with app.run_test() as pilot:
        panel = app.query_one(TaskDetailPanel)
        panel.task_data = failed_task
        await pilot.pause()

        # Capture snapshot of failed task
        assert await snap_compare(app, "failed_task_with_error.txt")


@pytest.mark.asyncio
async def test_pending_task_snapshot(snap_compare):
    """Snapshot test for pending task (minimal fields)."""
    pending_task = Task(
        id=TASK_ID,
        summary="Pending task awaiting execution",
        prompt="Task that has not started yet",
        status=TaskStatus.PENDING,
        submitted_at=FIXED_NOW,
        last_updated_at=FIXED_NOW,
    )

    app = SnapshotTestApp()
    async with app.run_test() as pilot:
        panel = app.query_one(TaskDetailPanel)
        panel.task_data = pending_task
        await pilot.pause()

        # Capture snapshot of pending task
        assert await snap_compare(app, "pending_task.txt")


@pytest.mark.asyncio
async def test_blocked_task_snapshot(snap_compare):
    """Snapshot test for blocked task with dependencies."""
    blocked_task = Task(
        id=TASK_ID,
        summary="Blocked task waiting on dependencies",
        prompt="Task blocked by prerequisites",
        status=TaskStatus.BLOCKED,
        dependencies=[DEP_ID_1, DEP_ID_2],
        dependency_type=DependencyType.PARALLEL,
        dependency_depth=3,
        submitted_at=FIXED_NOW - timedelta(minutes=15),
        last_updated_at=FIXED_NOW,
    )

    app = SnapshotTestApp()
    async with app.run_test() as pilot:
        panel = app.query_one(TaskDetailPanel)
        panel.task_data = blocked_task
        await pilot.pause()

        # Capture snapshot of blocked task
        assert await snap_compare(app, "blocked_task.txt")


@pytest.mark.asyncio
async def test_cancelled_task_snapshot(snap_compare):
    """Snapshot test for cancelled task."""
    cancelled_task = Task(
        id=TASK_ID,
        summary="Cancelled task",
        prompt="Task that was cancelled",
        status=TaskStatus.CANCELLED,
        submitted_at=FIXED_NOW - timedelta(minutes=10),
        last_updated_at=FIXED_NOW,
    )

    app = SnapshotTestApp()
    async with app.run_test() as pilot:
        panel = app.query_one(TaskDetailPanel)
        panel.task_data = cancelled_task
        await pilot.pause()

        # Capture snapshot of cancelled task
        assert await snap_compare(app, "cancelled_task.txt")


@pytest.mark.asyncio
async def test_task_with_long_summary_snapshot(snap_compare):
    """Snapshot test for task with maximum-length summary."""
    long_summary = "A" * 140  # Maximum 140 characters

    long_summary_task = Task(
        id=TASK_ID,
        summary=long_summary,
        prompt="Task with very long summary",
        status=TaskStatus.READY,
        submitted_at=FIXED_NOW,
        last_updated_at=FIXED_NOW,
    )

    app = SnapshotTestApp()
    async with app.run_test() as pilot:
        panel = app.query_one(TaskDetailPanel)
        panel.task_data = long_summary_task
        await pilot.pause()

        # Capture snapshot of task with long summary
        assert await snap_compare(app, "task_long_summary.txt")


@pytest.mark.asyncio
async def test_task_with_all_branches_snapshot(snap_compare):
    """Snapshot test for task with all branch fields populated."""
    branched_task = Task(
        id=TASK_ID,
        summary="Task with full branch configuration",
        prompt="Task using git worktrees",
        status=TaskStatus.RUNNING,
        feature_branch="feature/major-refactor",
        task_branch="task/refactor-database-layer",
        worktree_path="/home/user/worktrees/refactor-db",
        submitted_at=FIXED_NOW - timedelta(minutes=5),
        started_at=FIXED_NOW - timedelta(minutes=2),
        last_updated_at=FIXED_NOW,
    )

    app = SnapshotTestApp()
    async with app.run_test() as pilot:
        panel = app.query_one(TaskDetailPanel)
        panel.task_data = branched_task
        await pilot.pause()

        # Capture snapshot of task with branches
        assert await snap_compare(app, "task_with_branches.txt")


@pytest.mark.asyncio
async def test_task_with_complex_result_data_snapshot(snap_compare):
    """Snapshot test for task with complex nested result data."""
    complex_result_task = Task(
        id=TASK_ID,
        summary="Task with complex result data",
        prompt="Task that produced nested results",
        status=TaskStatus.COMPLETED,
        completed_at=FIXED_NOW,
        result_data={
            "execution_status": "success",
            "metrics": {
                "duration_seconds": 125.5,
                "memory_mb": 256,
                "cpu_percent": 45.2,
            },
            "output": {
                "files_modified": ["file1.py", "file2.py", "file3.py"],
                "lines_changed": {"added": 150, "removed": 75},
            },
            "nested": {"deeply": {"nested": {"data": "value"}}},
        },
        submitted_at=FIXED_NOW - timedelta(minutes=10),
        started_at=FIXED_NOW - timedelta(minutes=8),
        last_updated_at=FIXED_NOW,
    )

    app = SnapshotTestApp()
    async with app.run_test() as pilot:
        panel = app.query_one(TaskDetailPanel)
        panel.task_data = complex_result_task
        await pilot.pause()

        # Capture snapshot of complex result data
        assert await snap_compare(app, "task_complex_result_data.txt")
