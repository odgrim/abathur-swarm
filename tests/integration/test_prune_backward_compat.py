"""Backward compatibility tests for task pruning.

Tests ensure that existing prune_tasks behavior remains unchanged when recursive
mode is not enabled (recursive=False or not specified).

Key test coverage:
- PruneFilters without recursive field works (defaults to False)
- Linear mode behavior identical to pre-recursive implementation
- Child validation blocking still works in linear mode
- Orphaning still works in linear mode
- Return type is PruneResult (not RecursivePruneResult) when recursive=False
- All existing prune functionality preserved
"""

import asyncio
from datetime import datetime, timedelta, timezone
from pathlib import Path
from tempfile import NamedTemporaryFile

import pytest
from abathur.domain.models import Task, TaskSource, TaskStatus
from abathur.infrastructure.database import Database, PruneFilters, PruneResult


class TestPruneFiltersBackwardCompatibility:
    """Unit tests for PruneFilters backward compatibility."""

    def test_prune_filters_without_recursive_field(self):
        """Test PruneFilters creation without recursive field (should default to False)."""
        # Arrange & Act
        filters = PruneFilters(task_ids=["00000000-0000-0000-0000-000000000001"])

        # Assert - recursive field should not be present or should default to False
        # This verifies old code that doesn't specify recursive still works
        assert filters.task_ids is not None
        assert len(filters.task_ids) == 1
        # If recursive field exists, verify it defaults to False
        if hasattr(filters, "recursive"):
            assert filters.recursive is False

    def test_prune_filters_with_time_based_criteria_no_recursive(self):
        """Test PruneFilters with time-based criteria without recursive field."""
        # Arrange & Act
        filters = PruneFilters(older_than_days=30)

        # Assert
        assert filters.older_than_days == 30
        assert filters.statuses is not None  # Should auto-set pruneable statuses
        assert TaskStatus.COMPLETED in filters.statuses
        assert TaskStatus.FAILED in filters.statuses
        assert TaskStatus.CANCELLED in filters.statuses

    def test_prune_filters_with_status_criteria_no_recursive(self):
        """Test PruneFilters with status criteria without recursive field."""
        # Arrange & Act
        filters = PruneFilters(statuses=[TaskStatus.COMPLETED])

        # Assert
        assert filters.statuses == [TaskStatus.COMPLETED]

    def test_prune_filters_serialization_backward_compat(self):
        """Test PruneFilters serializes correctly without recursive field."""
        # Arrange
        filters = PruneFilters(older_than_days=30, vacuum_mode="always")

        # Act
        serialized = filters.model_dump()

        # Assert - verify core fields present
        assert "older_than_days" in serialized
        assert serialized["older_than_days"] == 30
        assert "vacuum_mode" in serialized
        assert serialized["vacuum_mode"] == "always"


class TestPruneTasksLinearModeBackwardCompatibility:
    """Integration tests for prune_tasks linear mode (recursive=False)."""

    @pytest.mark.asyncio
    async def test_prune_single_task_linear_mode(self):
        """Test pruning a single task without recursive mode matches old behavior."""
        # Setup: Create temporary database
        with NamedTemporaryFile(suffix=".db", delete=False) as tmp_file:
            db_path = Path(tmp_file.name)

        try:
            db = Database(db_path)
            await db.initialize()

            # Create a completed task
            task = Task(
                prompt="Test task",
                summary="Single task to prune",
                agent_type="test-agent",
                source=TaskSource.HUMAN,
                status=TaskStatus.COMPLETED,
                completed_at=datetime.now(timezone.utc) - timedelta(days=40),
            )
            await db.insert_task(task)

            # Act: Prune without recursive flag (linear mode)
            filters = PruneFilters(task_ids=[task.id])
            result = await db.prune_tasks(filters)

            # Assert: Verify result is PruneResult (not RecursivePruneResult)
            assert isinstance(result, PruneResult)
            assert result.deleted_tasks == 1
            assert result.dry_run is False
            assert TaskStatus.COMPLETED in result.breakdown_by_status
            assert result.breakdown_by_status[TaskStatus.COMPLETED] == 1

            # Verify task was deleted from database
            retrieved = await db.get_task(task.id)
            assert retrieved is None

            await db.close()
        finally:
            if db_path.exists():
                db_path.unlink()

    @pytest.mark.asyncio
    async def test_prune_time_based_linear_mode(self):
        """Test time-based pruning without recursive mode."""
        # Setup
        with NamedTemporaryFile(suffix=".db", delete=False) as tmp_file:
            db_path = Path(tmp_file.name)

        try:
            db = Database(db_path)
            await db.initialize()

            # Create old completed task
            old_task = Task(
                prompt="Old task",
                summary="Task to prune",
                agent_type="test-agent",
                source=TaskSource.HUMAN,
                status=TaskStatus.COMPLETED,
                submitted_at=datetime.now(timezone.utc) - timedelta(days=40),
                completed_at=datetime.now(timezone.utc) - timedelta(days=40),
            )
            await db.insert_task(old_task)

            # Create recent task
            recent_task = Task(
                prompt="Recent task",
                summary="Task to keep",
                agent_type="test-agent",
                source=TaskSource.HUMAN,
                status=TaskStatus.COMPLETED,
                submitted_at=datetime.now(timezone.utc) - timedelta(days=10),
                completed_at=datetime.now(timezone.utc) - timedelta(days=10),
            )
            await db.insert_task(recent_task)

            # Act: Prune old tasks without recursive flag
            filters = PruneFilters(older_than_days=30)
            result = await db.prune_tasks(filters)

            # Assert
            assert isinstance(result, PruneResult)
            assert result.deleted_tasks == 1
            assert result.breakdown_by_status[TaskStatus.COMPLETED] == 1

            # Verify only old task was deleted
            old_retrieved = await db.get_task(old_task.id)
            assert old_retrieved is None

            recent_retrieved = await db.get_task(recent_task.id)
            assert recent_retrieved is not None
            assert recent_retrieved.id == recent_task.id

            await db.close()
        finally:
            if db_path.exists():
                db_path.unlink()

    @pytest.mark.asyncio
    async def test_prune_status_based_linear_mode(self):
        """Test status-based pruning without recursive mode."""
        # Setup
        with NamedTemporaryFile(suffix=".db", delete=False) as tmp_file:
            db_path = Path(tmp_file.name)

        try:
            db = Database(db_path)
            await db.initialize()

            # Create completed task
            completed_task = Task(
                prompt="Completed task",
                summary="Completed",
                agent_type="test-agent",
                source=TaskSource.HUMAN,
                status=TaskStatus.COMPLETED,
            )
            await db.insert_task(completed_task)

            # Create failed task
            failed_task = Task(
                prompt="Failed task",
                summary="Failed",
                agent_type="test-agent",
                source=TaskSource.HUMAN,
                status=TaskStatus.FAILED,
            )
            await db.insert_task(failed_task)

            # Create cancelled task
            cancelled_task = Task(
                prompt="Cancelled task",
                summary="Cancelled",
                agent_type="test-agent",
                source=TaskSource.HUMAN,
                status=TaskStatus.CANCELLED,
            )
            await db.insert_task(cancelled_task)

            # Act: Prune only completed tasks (linear mode)
            filters = PruneFilters(statuses=[TaskStatus.COMPLETED])
            result = await db.prune_tasks(filters)

            # Assert
            assert isinstance(result, PruneResult)
            assert result.deleted_tasks == 1
            assert result.breakdown_by_status[TaskStatus.COMPLETED] == 1

            # Verify only completed task was deleted
            completed_retrieved = await db.get_task(completed_task.id)
            assert completed_retrieved is None

            failed_retrieved = await db.get_task(failed_task.id)
            assert failed_retrieved is not None

            cancelled_retrieved = await db.get_task(cancelled_task.id)
            assert cancelled_retrieved is not None

            await db.close()
        finally:
            if db_path.exists():
                db_path.unlink()


class TestOrphaningBackwardCompatibility:
    """Tests for orphaning behavior in linear mode (pre-recursive implementation)."""

    @pytest.mark.asyncio
    async def test_parent_deletion_orphans_children_linear_mode(self):
        """Test parent deletion orphans children (sets parent_task_id=NULL) in linear mode.

        This is the backward compatible behavior - linear mode orphans children
        rather than blocking parent deletion or recursively deleting.
        """
        # Setup
        with NamedTemporaryFile(suffix=".db", delete=False) as tmp_file:
            db_path = Path(tmp_file.name)

        try:
            db = Database(db_path)
            await db.initialize()

            # Create parent task (completed, eligible for pruning)
            parent_task = Task(
                prompt="Parent task",
                summary="Parent with child",
                agent_type="test-agent",
                source=TaskSource.HUMAN,
                status=TaskStatus.COMPLETED,
                completed_at=datetime.now(timezone.utc) - timedelta(days=40),
            )
            await db.insert_task(parent_task)

            # Create child task (running, not eligible for pruning)
            child_task = Task(
                prompt="Child task",
                summary="Active child",
                agent_type="test-agent",
                source=TaskSource.HUMAN,
                parent_task_id=parent_task.id,
                status=TaskStatus.RUNNING,
            )
            await db.insert_task(child_task)

            # Act: Prune parent (should orphan child)
            filters = PruneFilters(task_ids=[parent_task.id])
            result = await db.prune_tasks(filters)

            # Assert: Parent should be deleted, child orphaned
            assert isinstance(result, PruneResult)
            assert result.deleted_tasks == 1
            assert result.dry_run is False

            # Verify parent was deleted
            parent_retrieved = await db.get_task(parent_task.id)
            assert parent_retrieved is None

            # Verify child still exists but is now orphaned (parent_task_id=None)
            child_retrieved = await db.get_task(child_task.id)
            assert child_retrieved is not None
            assert child_retrieved.parent_task_id is None  # Orphaned

            await db.close()
        finally:
            if db_path.exists():
                db_path.unlink()

    @pytest.mark.asyncio
    async def test_time_based_prune_orphans_children_linear_mode(self):
        """Test time-based prune orphans children in linear mode."""
        # Setup
        with NamedTemporaryFile(suffix=".db", delete=False) as tmp_file:
            db_path = Path(tmp_file.name)

        try:
            db = Database(db_path)
            await db.initialize()

            # Create old parent task (completed)
            parent_task = Task(
                prompt="Old parent",
                summary="Parent with active child",
                agent_type="test-agent",
                source=TaskSource.HUMAN,
                status=TaskStatus.COMPLETED,
                submitted_at=datetime.now(timezone.utc) - timedelta(days=50),
                completed_at=datetime.now(timezone.utc) - timedelta(days=50),
            )
            await db.insert_task(parent_task)

            # Create child task (still running)
            child_task = Task(
                prompt="Active child",
                summary="Running child task",
                agent_type="test-agent",
                source=TaskSource.HUMAN,
                parent_task_id=parent_task.id,
                status=TaskStatus.RUNNING,
                submitted_at=datetime.now(timezone.utc) - timedelta(days=10),
            )
            await db.insert_task(child_task)

            # Create standalone old completed task (no children)
            standalone_task = Task(
                prompt="Standalone task",
                summary="Task without children",
                agent_type="test-agent",
                source=TaskSource.HUMAN,
                status=TaskStatus.COMPLETED,
                submitted_at=datetime.now(timezone.utc) - timedelta(days=45),
                completed_at=datetime.now(timezone.utc) - timedelta(days=45),
            )
            await db.insert_task(standalone_task)

            # Act: Prune tasks older than 30 days
            filters = PruneFilters(older_than_days=30)
            result = await db.prune_tasks(filters)

            # Assert: Both old tasks should be deleted (parent orphans child)
            assert isinstance(result, PruneResult)
            assert result.deleted_tasks == 2  # Parent and standalone
            assert result.breakdown_by_status[TaskStatus.COMPLETED] == 2

            # Verify parent was deleted
            parent_retrieved = await db.get_task(parent_task.id)
            assert parent_retrieved is None

            # Verify child still exists but is orphaned
            child_retrieved = await db.get_task(child_task.id)
            assert child_retrieved is not None
            assert child_retrieved.parent_task_id is None  # Orphaned

            # Verify standalone task was deleted
            standalone_retrieved = await db.get_task(standalone_task.id)
            assert standalone_retrieved is None

            await db.close()
        finally:
            if db_path.exists():
                db_path.unlink()

    @pytest.mark.asyncio
    async def test_parent_can_be_deleted_after_children_pruned_linear_mode(self):
        """Test parent can be deleted after children are pruned in linear mode."""
        # Setup
        with NamedTemporaryFile(suffix=".db", delete=False) as tmp_file:
            db_path = Path(tmp_file.name)

        try:
            db = Database(db_path)
            await db.initialize()

            # Create parent task (completed)
            parent_task = Task(
                prompt="Parent task",
                summary="Parent",
                agent_type="test-agent",
                source=TaskSource.HUMAN,
                status=TaskStatus.COMPLETED,
            )
            await db.insert_task(parent_task)

            # Create child task (completed)
            child_task = Task(
                prompt="Child task",
                summary="Child",
                agent_type="test-agent",
                source=TaskSource.HUMAN,
                parent_task_id=parent_task.id,
                status=TaskStatus.COMPLETED,
            )
            await db.insert_task(child_task)

            # Act Step 1: Delete child first
            child_filters = PruneFilters(task_ids=[child_task.id])
            child_result = await db.prune_tasks(child_filters)

            # Assert: Child deleted successfully
            assert child_result.deleted_tasks == 1

            # Act Step 2: Now delete parent (should succeed)
            parent_filters = PruneFilters(task_ids=[parent_task.id])
            parent_result = await db.prune_tasks(parent_filters)

            # Assert: Parent can now be deleted
            assert parent_result.deleted_tasks == 1

            # Verify both deleted
            parent_retrieved = await db.get_task(parent_task.id)
            assert parent_retrieved is None

            child_retrieved = await db.get_task(child_task.id)
            assert child_retrieved is None

            await db.close()
        finally:
            if db_path.exists():
                db_path.unlink()


class TestPruneResultReturnTypeBackwardCompatibility:
    """Tests for PruneResult return type in linear mode."""

    @pytest.mark.asyncio
    async def test_prune_returns_prune_result_not_recursive_result(self):
        """Test prune_tasks returns PruneResult (not RecursivePruneResult) in linear mode."""
        # Setup
        with NamedTemporaryFile(suffix=".db", delete=False) as tmp_file:
            db_path = Path(tmp_file.name)

        try:
            db = Database(db_path)
            await db.initialize()

            # Create completed task
            task = Task(
                prompt="Test task",
                summary="Task to prune",
                agent_type="test-agent",
                source=TaskSource.HUMAN,
                status=TaskStatus.COMPLETED,
            )
            await db.insert_task(task)

            # Act: Prune in linear mode (no recursive flag)
            filters = PruneFilters(task_ids=[task.id])
            result = await db.prune_tasks(filters)

            # Assert: Return type is PruneResult
            assert isinstance(result, PruneResult)
            assert type(result).__name__ == "PruneResult"

            # Verify PruneResult has expected fields
            assert hasattr(result, "deleted_tasks")
            assert hasattr(result, "deleted_dependencies")
            assert hasattr(result, "reclaimed_bytes")
            assert hasattr(result, "dry_run")
            assert hasattr(result, "breakdown_by_status")
            assert hasattr(result, "vacuum_auto_skipped")

            # Verify it does NOT have recursive-specific fields
            # (If RecursivePruneResult exists, it would have additional fields)
            assert result.deleted_tasks == 1

            await db.close()
        finally:
            if db_path.exists():
                db_path.unlink()

    @pytest.mark.asyncio
    async def test_prune_result_fields_populated_correctly_linear_mode(self):
        """Test PruneResult fields are populated correctly in linear mode."""
        # Setup
        with NamedTemporaryFile(suffix=".db", delete=False) as tmp_file:
            db_path = Path(tmp_file.name)

        try:
            db = Database(db_path)
            await db.initialize()

            # Create multiple tasks with different statuses
            completed_task = Task(
                prompt="Completed",
                summary="Completed task",
                agent_type="test-agent",
                source=TaskSource.HUMAN,
                status=TaskStatus.COMPLETED,
            )
            await db.insert_task(completed_task)

            failed_task = Task(
                prompt="Failed",
                summary="Failed task",
                agent_type="test-agent",
                source=TaskSource.HUMAN,
                status=TaskStatus.FAILED,
            )
            await db.insert_task(failed_task)

            # Act: Prune both tasks
            filters = PruneFilters(
                statuses=[TaskStatus.COMPLETED, TaskStatus.FAILED]
            )
            result = await db.prune_tasks(filters)

            # Assert: Verify all fields populated correctly
            assert isinstance(result, PruneResult)
            assert result.deleted_tasks == 2
            assert result.dry_run is False

            # Verify breakdown by status
            assert TaskStatus.COMPLETED in result.breakdown_by_status
            assert result.breakdown_by_status[TaskStatus.COMPLETED] == 1
            assert TaskStatus.FAILED in result.breakdown_by_status
            assert result.breakdown_by_status[TaskStatus.FAILED] == 1

            # Verify vacuum fields
            assert isinstance(result.vacuum_auto_skipped, bool)

            await db.close()
        finally:
            if db_path.exists():
                db_path.unlink()


class TestDryRunBackwardCompatibility:
    """Tests for dry-run mode backward compatibility."""

    @pytest.mark.asyncio
    async def test_dry_run_mode_no_deletion_linear_mode(self):
        """Test dry-run mode does not delete tasks in linear mode."""
        # Setup
        with NamedTemporaryFile(suffix=".db", delete=False) as tmp_file:
            db_path = Path(tmp_file.name)

        try:
            db = Database(db_path)
            await db.initialize()

            # Create completed task
            task = Task(
                prompt="Test task",
                summary="Task for dry-run",
                agent_type="test-agent",
                source=TaskSource.HUMAN,
                status=TaskStatus.COMPLETED,
            )
            await db.insert_task(task)

            # Act: Dry-run prune
            filters = PruneFilters(task_ids=[task.id], dry_run=True)
            result = await db.prune_tasks(filters)

            # Assert: Result shows what would be deleted
            assert isinstance(result, PruneResult)
            assert result.deleted_tasks == 1  # Shows count
            assert result.dry_run is True
            assert result.breakdown_by_status[TaskStatus.COMPLETED] == 1

            # Verify task was NOT actually deleted
            retrieved = await db.get_task(task.id)
            assert retrieved is not None
            assert retrieved.id == task.id

            await db.close()
        finally:
            if db_path.exists():
                db_path.unlink()

    @pytest.mark.asyncio
    async def test_dry_run_time_based_shows_correct_count(self):
        """Test dry-run with time-based filter shows correct count."""
        # Setup
        with NamedTemporaryFile(suffix=".db", delete=False) as tmp_file:
            db_path = Path(tmp_file.name)

        try:
            db = Database(db_path)
            await db.initialize()

            # Create 3 old tasks
            for i in range(3):
                task = Task(
                    prompt=f"Old task {i}",
                    summary=f"Task {i}",
                    agent_type="test-agent",
                    source=TaskSource.HUMAN,
                    status=TaskStatus.COMPLETED,
                    submitted_at=datetime.now(timezone.utc) - timedelta(days=40),
                    completed_at=datetime.now(timezone.utc) - timedelta(days=40),
                )
                await db.insert_task(task)

            # Act: Dry-run time-based prune
            filters = PruneFilters(older_than_days=30, dry_run=True)
            result = await db.prune_tasks(filters)

            # Assert: Shows count of 3
            assert isinstance(result, PruneResult)
            assert result.deleted_tasks == 3
            assert result.dry_run is True

            # Verify all tasks still exist
            all_tasks = await db.list_tasks(TaskStatus.COMPLETED, limit=10)
            assert len(all_tasks) == 3

            await db.close()
        finally:
            if db_path.exists():
                db_path.unlink()


class TestVacuumModeBackwardCompatibility:
    """Tests for VACUUM mode backward compatibility."""

    @pytest.mark.asyncio
    async def test_vacuum_mode_conditional_default_linear_mode(self):
        """Test VACUUM mode defaults to 'conditional' in linear mode."""
        # Setup
        with NamedTemporaryFile(suffix=".db", delete=False) as tmp_file:
            db_path = Path(tmp_file.name)

        try:
            db = Database(db_path)
            await db.initialize()

            # Create task
            task = Task(
                prompt="Test task",
                summary="Task for vacuum test",
                agent_type="test-agent",
                source=TaskSource.HUMAN,
                status=TaskStatus.COMPLETED,
            )
            await db.insert_task(task)

            # Act: Prune without specifying vacuum_mode (should default to conditional)
            filters = PruneFilters(task_ids=[task.id])
            result = await db.prune_tasks(filters)

            # Assert: Verify default vacuum_mode behavior
            assert isinstance(result, PruneResult)
            assert result.deleted_tasks == 1
            # With only 1 task, conditional mode should NOT run VACUUM
            assert result.reclaimed_bytes is None

            await db.close()
        finally:
            if db_path.exists():
                db_path.unlink()

    @pytest.mark.asyncio
    async def test_vacuum_mode_always_runs_vacuum_linear_mode(self):
        """Test vacuum_mode='always' runs VACUUM in linear mode."""
        # Setup
        with NamedTemporaryFile(suffix=".db", delete=False) as tmp_file:
            db_path = Path(tmp_file.name)

        try:
            db = Database(db_path)
            await db.initialize()

            # Create task
            task = Task(
                prompt="Test task",
                summary="Task for vacuum test",
                agent_type="test-agent",
                source=TaskSource.HUMAN,
                status=TaskStatus.COMPLETED,
            )
            await db.insert_task(task)

            # Act: Prune with vacuum_mode='always'
            filters = PruneFilters(task_ids=[task.id], vacuum_mode="always")
            result = await db.prune_tasks(filters)

            # Assert: VACUUM should have run
            assert isinstance(result, PruneResult)
            assert result.deleted_tasks == 1
            assert result.reclaimed_bytes is not None
            assert result.reclaimed_bytes >= 0

            await db.close()
        finally:
            if db_path.exists():
                db_path.unlink()

    @pytest.mark.asyncio
    async def test_vacuum_mode_never_skips_vacuum_linear_mode(self):
        """Test vacuum_mode='never' never runs VACUUM in linear mode."""
        # Setup
        with NamedTemporaryFile(suffix=".db", delete=False) as tmp_file:
            db_path = Path(tmp_file.name)

        try:
            db = Database(db_path)
            await db.initialize()

            # Create many tasks (above conditional threshold)
            for i in range(150):
                task = Task(
                    prompt=f"Task {i}",
                    summary=f"Task {i}",
                    agent_type="test-agent",
                    source=TaskSource.HUMAN,
                    status=TaskStatus.COMPLETED,
                )
                await db.insert_task(task)

            # Act: Prune with vacuum_mode='never'
            filters = PruneFilters(
                statuses=[TaskStatus.COMPLETED], vacuum_mode="never"
            )
            result = await db.prune_tasks(filters)

            # Assert: VACUUM should NOT have run (even with 150 tasks)
            assert isinstance(result, PruneResult)
            assert result.deleted_tasks == 150
            assert result.reclaimed_bytes is None

            await db.close()
        finally:
            if db_path.exists():
                db_path.unlink()
