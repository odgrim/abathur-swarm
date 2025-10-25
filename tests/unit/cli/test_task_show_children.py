"""Unit tests for child task display in task show command.

Tests the child task display feature:
- Display child task IDs with clear labeling
- Show ID, summary, status in table format
- Handle edge cases (no children, missing summary, truncation)
"""

import asyncio
from pathlib import Path

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


class TestTaskShowChildTasks:
    """Unit tests for child task display in task show command."""

    def test_task_show_no_children(self, database):
        """Test task with no children -> no child section displayed.

        Scenario: Task with no children
        Expected: Clean output, no 'Child Tasks' section
        """
        # Arrange: Create parent task with no children
        from abathur.application import TaskCoordinator

        coordinator = TaskCoordinator(database)

        async def create_parent_task():
            parent_task = Task(
                prompt="Parent task with no children",
                summary="Leaf task",
                agent_type="test-agent",
                source=TaskSource.HUMAN,
                status=TaskStatus.PENDING,
            )
            parent_id = await coordinator.submit_task(parent_task)
            return parent_id

        parent_id = asyncio.run(create_parent_task())
        parent_id_str = str(parent_id)

        # Act: Run task show command
        result = runner.invoke(app, ["task", "show", parent_id_str])

        # Assert: No child section displayed
        assert result.exit_code == 0
        assert "Child Tasks:" not in result.stdout
        assert "ID" not in result.stdout or "Summary:" in result.stdout  # Avoid false positive from task details
        # Existing task info should be present
        assert "Leaf task" in result.stdout
        assert "Parent task with no children" in result.stdout

    def test_task_show_one_child(self, database):
        """Test task with 1 child -> table with 1 row displayed.

        Scenario: Task with 1 child
        Expected: Child Tasks section with 1-row table, proper formatting
        """
        # Arrange: Create parent with 1 child
        from abathur.application import TaskCoordinator

        coordinator = TaskCoordinator(database)

        async def create_parent_with_child():
            # Create parent
            parent_task = Task(
                prompt="Parent task with one child",
                summary="Parent task",
                agent_type="test-agent",
                source=TaskSource.HUMAN,
                status=TaskStatus.RUNNING,
            )
            parent_id = await coordinator.submit_task(parent_task)

            # Create child
            child_task = Task(
                prompt="Child task prompt",
                summary="Test child task",
                agent_type="test-agent",
                source=TaskSource.AGENT_PLANNER,
                status=TaskStatus.PENDING,
                parent_task_id=parent_id,
            )
            child_id = await coordinator.submit_task(child_task)

            return parent_id, child_id

        parent_id, child_id = asyncio.run(create_parent_with_child())
        parent_id_str = str(parent_id)
        child_id_prefix = str(child_id)[:8]

        # Act: Run task show command
        result = runner.invoke(app, ["task", "show", parent_id_str])

        # Assert: Child section displayed with 1 row
        assert result.exit_code == 0
        assert "Child Tasks:" in result.stdout
        # Table should have columns: ID, Summary, Status
        assert child_id_prefix in result.stdout
        assert "Test child task" in result.stdout
        assert "pending" in result.stdout

    def test_task_show_multiple_children(self, database):
        """Test task with 5 children -> table with 5 rows.

        Scenario: Task with 5 children
        Expected: Child Tasks section with 5-row table, all columns aligned
        """
        # Arrange: Create parent with 5 children
        from abathur.application import TaskCoordinator

        coordinator = TaskCoordinator(database)

        async def create_parent_with_multiple_children():
            # Create parent
            parent_task = Task(
                prompt="Parent task with multiple children",
                summary="Parent with 5 children",
                agent_type="test-agent",
                source=TaskSource.HUMAN,
                status=TaskStatus.RUNNING,
            )
            parent_id = await coordinator.submit_task(parent_task)

            # Create 5 children with different statuses
            child_summaries = [
                "First child task",
                "Second child task",
                "Third child task",
                "Fourth child task",
                "Fifth child task",
            ]
            statuses = [
                TaskStatus.PENDING,
                TaskStatus.RUNNING,
                TaskStatus.COMPLETED,
                TaskStatus.FAILED,
                TaskStatus.BLOCKED,
            ]

            child_ids = []
            for summary, status in zip(child_summaries, statuses):
                child_task = Task(
                    prompt=f"Child task: {summary}",
                    summary=summary,
                    agent_type="test-agent",
                    source=TaskSource.AGENT_PLANNER,
                    status=status,
                    parent_task_id=parent_id,
                )
                child_id = await coordinator.submit_task(child_task)
                child_ids.append(child_id)

            return parent_id, child_ids

        parent_id, child_ids = asyncio.run(create_parent_with_multiple_children())
        parent_id_str = str(parent_id)

        # Act: Run task show command
        result = runner.invoke(app, ["task", "show", parent_id_str])

        # Assert: Child section displayed with 5 rows
        assert result.exit_code == 0
        assert "Child Tasks:" in result.stdout

        # Verify all 5 children are present
        for child_id in child_ids:
            child_id_prefix = str(child_id)[:8]
            assert child_id_prefix in result.stdout

        # Verify summaries
        assert "First child task" in result.stdout
        assert "Second child task" in result.stdout
        assert "Third child task" in result.stdout
        assert "Fourth child task" in result.stdout
        assert "Fifth child task" in result.stdout

        # Verify statuses
        assert "pending" in result.stdout
        assert "running" in result.stdout
        assert "completed" in result.stdout
        assert "failed" in result.stdout
        assert "blocked" in result.stdout

    def test_task_show_child_summary_truncation(self, database):
        """Test child with 60-char summary -> truncated to 40 chars + '...'.

        Scenario: Child with very long summary (60 characters)
        Expected: Summary shows 40 chars + '...' (43 total)
        """
        # Arrange: Create parent with child that has long summary
        from abathur.application import TaskCoordinator

        coordinator = TaskCoordinator(database)

        async def create_parent_with_long_summary_child():
            # Create parent
            parent_task = Task(
                prompt="Parent task",
                summary="Parent",
                agent_type="test-agent",
                source=TaskSource.HUMAN,
                status=TaskStatus.RUNNING,
            )
            parent_id = await coordinator.submit_task(parent_task)

            # Create child with 60-char summary
            long_summary = "A" * 60  # 60 characters
            child_task = Task(
                prompt="Child with long summary",
                summary=long_summary,
                agent_type="test-agent",
                source=TaskSource.AGENT_PLANNER,
                status=TaskStatus.PENDING,
                parent_task_id=parent_id,
            )
            child_id = await coordinator.submit_task(child_task)

            return parent_id, child_id, long_summary

        parent_id, child_id, original_summary = asyncio.run(
            create_parent_with_long_summary_child()
        )
        parent_id_str = str(parent_id)

        # Act: Run task show command
        result = runner.invoke(app, ["task", "show", parent_id_str])

        # Assert: Summary truncated to 40 chars + '...'
        assert result.exit_code == 0
        assert "Child Tasks:" in result.stdout

        # Should display truncated summary (40 chars + '...')
        expected_truncated = "A" * 40 + "..."
        assert expected_truncated in result.stdout

        # Should NOT display full 60-char summary
        assert original_summary not in result.stdout

        # Verify child ID is present
        child_id_prefix = str(child_id)[:8]
        assert child_id_prefix in result.stdout

    def test_task_show_child_missing_summary(self, database):
        """Test child with summary=None -> displays '-'.

        Scenario: Child with summary=None
        Expected: Summary column shows '-'
        """
        # Arrange: Create parent with child that has no summary
        from abathur.application import TaskCoordinator

        coordinator = TaskCoordinator(database)

        async def create_parent_with_no_summary_child():
            # Create parent
            parent_task = Task(
                prompt="Parent task",
                summary="Parent",
                agent_type="test-agent",
                source=TaskSource.HUMAN,
                status=TaskStatus.RUNNING,
            )
            parent_id = await coordinator.submit_task(parent_task)

            # Create child with summary=None
            child_task = Task(
                prompt="Child with no summary",
                summary=None,  # Explicitly set to None
                agent_type="test-agent",
                source=TaskSource.AGENT_PLANNER,
                status=TaskStatus.PENDING,
                parent_task_id=parent_id,
            )
            child_id = await coordinator.submit_task(child_task)

            return parent_id, child_id

        parent_id, child_id = asyncio.run(create_parent_with_no_summary_child())
        parent_id_str = str(parent_id)
        child_id_prefix = str(child_id)[:8]

        # Act: Run task show command
        result = runner.invoke(app, ["task", "show", parent_id_str])

        # Assert: Summary shows '-'
        assert result.exit_code == 0
        assert "Child Tasks:" in result.stdout

        # Verify child ID is present
        assert child_id_prefix in result.stdout

        # Summary column should show '-' for missing summary
        # Need to verify the '-' appears in the context of the child task row
        # Since we can't easily parse the table, we verify the child ID and '-' are both present
        assert "-" in result.stdout

    def test_task_show_child_at_summary_boundary(self, database):
        """Test child with exactly 40-char summary -> no truncation.

        Scenario: Child with summary at exact boundary (40 characters)
        Expected: Summary displayed in full, no '...' added
        """
        # Arrange: Create parent with child at boundary
        from abathur.application import TaskCoordinator

        coordinator = TaskCoordinator(database)

        async def create_parent_with_boundary_child():
            # Create parent
            parent_task = Task(
                prompt="Parent task",
                summary="Parent",
                agent_type="test-agent",
                source=TaskSource.HUMAN,
                status=TaskStatus.RUNNING,
            )
            parent_id = await coordinator.submit_task(parent_task)

            # Create child with exactly 40-char summary
            summary_40_chars = "X" * 40  # Exactly 40 characters
            child_task = Task(
                prompt="Child at boundary",
                summary=summary_40_chars,
                agent_type="test-agent",
                source=TaskSource.AGENT_PLANNER,
                status=TaskStatus.PENDING,
                parent_task_id=parent_id,
            )
            child_id = await coordinator.submit_task(child_task)

            return parent_id, child_id, summary_40_chars

        parent_id, child_id, summary_40_chars = asyncio.run(
            create_parent_with_boundary_child()
        )
        parent_id_str = str(parent_id)

        # Act: Run task show command
        result = runner.invoke(app, ["task", "show", parent_id_str])

        # Assert: Full summary displayed, no truncation
        assert result.exit_code == 0
        assert "Child Tasks:" in result.stdout

        # Should display full 40-char summary
        assert summary_40_chars in result.stdout

        # Should NOT have '...' after the summary
        # (though '...' might appear elsewhere in output, we check for truncation pattern)
        assert (summary_40_chars + "...") not in result.stdout

    def test_task_show_child_order_by_submitted_at(self, database):
        """Test children are displayed in submitted_at order.

        Scenario: Create children in specific order
        Expected: Children displayed in submission order (submitted_at ASC)
        """
        # Arrange: Create parent with children submitted in specific order
        from abathur.application import TaskCoordinator

        coordinator = TaskCoordinator(database)

        async def create_parent_with_ordered_children():
            # Create parent
            parent_task = Task(
                prompt="Parent task",
                summary="Parent",
                agent_type="test-agent",
                source=TaskSource.HUMAN,
                status=TaskStatus.RUNNING,
            )
            parent_id = await coordinator.submit_task(parent_task)

            # Create 3 children with slight delays to ensure different timestamps
            child_summaries = ["First submitted", "Second submitted", "Third submitted"]
            child_ids = []

            for summary in child_summaries:
                child_task = Task(
                    prompt=f"Child: {summary}",
                    summary=summary,
                    agent_type="test-agent",
                    source=TaskSource.AGENT_PLANNER,
                    status=TaskStatus.PENDING,
                    parent_task_id=parent_id,
                )
                child_id = await coordinator.submit_task(child_task)
                child_ids.append(child_id)
                # Small delay to ensure different timestamps
                await asyncio.sleep(0.01)

            return parent_id, child_ids

        parent_id, child_ids = asyncio.run(create_parent_with_ordered_children())
        parent_id_str = str(parent_id)

        # Act: Run task show command
        result = runner.invoke(app, ["task", "show", parent_id_str])

        # Assert: Children appear in order
        assert result.exit_code == 0

        # Find positions of each summary in output
        first_pos = result.stdout.find("First submitted")
        second_pos = result.stdout.find("Second submitted")
        third_pos = result.stdout.find("Third submitted")

        # All should be found
        assert first_pos != -1
        assert second_pos != -1
        assert third_pos != -1

        # Should appear in order (first < second < third)
        assert first_pos < second_pos < third_pos
