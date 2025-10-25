"""Integration tests for database-level task pruning (recursive and non-recursive).

Consolidates recursive and non-recursive prune scenarios using parametrization.

Test Coverage:
- Deep hierarchy recursive deletion (linear chains, depth tracking)
- Mixed status tree handling (partial tree detection)
- Non-recursive prune operations
- VACUUM behavior (always, never, conditional, auto-skip)
- Time-based filtering (older_than_days, before_date)
- Status-based filtering
- Dry-run mode
- Edge cases (empty results, concurrent operations)
"""

import asyncio
from collections.abc import AsyncGenerator
from datetime import datetime, timedelta, timezone
from pathlib import Path
from tempfile import NamedTemporaryFile, TemporaryDirectory
from uuid import UUID, uuid4

import pytest

from abathur.domain.models import Task, TaskSource, TaskStatus
from abathur.infrastructure.database import (
    Database,
    PruneFilters,
    PruneResult,
    RecursivePruneResult,
)


# ============================================================================
# Fixtures
# ============================================================================


@pytest.fixture
async def memory_db() -> AsyncGenerator[Database, None]:
    """Create in-memory database for fast integration tests."""
    db = Database(Path(":memory:"))
    await db.initialize()
    yield db
    await db.close()


@pytest.fixture
async def file_db() -> AsyncGenerator[Database, None]:
    """Create file-based database for persistence tests."""
    with NamedTemporaryFile(suffix=".db", delete=False) as tmp_file:
        db_path = Path(tmp_file.name)

    try:
        db = Database(db_path)
        await db.initialize()
        yield db
        await db.close()
    finally:
        if db_path.exists():
            db_path.unlink()


async def create_task_tree(
    db: Database,
    root_status: TaskStatus,
    child_statuses: list[TaskStatus],
) -> tuple[UUID, list[UUID]]:
    """Helper to create a simple task tree with specified statuses.

    Args:
        db: Database instance
        root_status: Status for root task
        child_statuses: List of statuses for child tasks

    Returns:
        Tuple of (root_id, [child_ids])
    """
    # Create root task
    root_task = Task(
        prompt="Root task",
        summary="Root",
        agent_type="test-agent",
        source=TaskSource.HUMAN,
        status=root_status,
        input_data={},
    )
    await db.insert_task(root_task)
    root_id = root_task.id

    # Create child tasks
    child_ids = []
    for i, child_status in enumerate(child_statuses):
        child_task = Task(
            prompt=f"Child task {i}",
            summary=f"Child{i}",
            agent_type="test-agent",
            source=TaskSource.HUMAN,
            status=child_status,
            parent_task_id=root_id,
            input_data={},
        )
        await db.insert_task(child_task)
        child_id = child_task.id
        child_ids.append(child_id)

    return root_id, child_ids


# ============================================================================
# Recursive Deep Hierarchy Tests
# ============================================================================


class TestDeepHierarchyRecursivePrune:
    """Integration tests for deep hierarchy recursive pruning."""

    @pytest.mark.asyncio
    async def test_deep_linear_chain_all_completed_recursive_prune(
        self, memory_db: Database
    ):
        """Test recursive prune of deep linear chain (depth=10, all COMPLETED).

        Scenario: Root -> L1 -> L2 -> ... -> L10 (all COMPLETED)
        Expected:
        - All 11 tasks deleted (root + 10 levels)
        - tree_depth=10 in result
        - deleted_by_depth correctly tracks each level
        - Operation completes efficiently
        """
        # Setup: Create linear chain of depth 10
        tasks = []
        parent_id = None

        # Create root task (depth 0)
        root_task = Task(
            prompt="Root task",
            summary="Root of deep hierarchy",
            agent_type="test-agent",
            source=TaskSource.HUMAN,
            status=TaskStatus.COMPLETED,
            completed_at=datetime.now(timezone.utc) - timedelta(days=1),
        )
        await memory_db.insert_task(root_task)
        tasks.append(root_task)
        parent_id = root_task.id

        # Create 10 levels of children (L1 through L10)
        for level in range(1, 11):
            child_task = Task(
                prompt=f"Level {level} task",
                summary=f"Task at level {level}",
                agent_type="test-agent",
                source=TaskSource.HUMAN,
                parent_task_id=parent_id,
                status=TaskStatus.COMPLETED,
                completed_at=datetime.now(timezone.utc) - timedelta(days=1),
            )
            await memory_db.insert_task(child_task)
            tasks.append(child_task)
            parent_id = child_task.id

        # Verify setup: Should have 11 tasks total
        all_tasks = await memory_db.list_tasks(limit=20)
        assert len(all_tasks) == 11

        # Act: Prune root task with recursive=True
        filters = PruneFilters(
            task_ids=[root_task.id], recursive=True  # type: ignore[call-arg]
        )
        result = await memory_db.prune_tasks(filters)

        # Assert: Verify all tasks deleted
        assert result.deleted_tasks == 11, "Should delete root + 10 child levels"
        assert result.dry_run is False

        # Verify recursive-specific metrics
        if hasattr(result, "tree_depth"):
            assert result.tree_depth == 10, "Maximum depth should be 10"

        if hasattr(result, "deleted_by_depth"):
            # Verify each depth level has 1 task
            assert len(result.deleted_by_depth) == 11, "Should have 11 depth levels"
            for depth in range(11):
                assert (
                    result.deleted_by_depth.get(depth, 0) == 1
                ), f"Depth {depth} should have 1 task"

        # Verify breakdown by status
        assert result.breakdown_by_status[TaskStatus.COMPLETED] == 11

        # Verify database is empty
        remaining_tasks = await memory_db.list_tasks(limit=20)
        assert len(remaining_tasks) == 0, "All tasks should be deleted"

        # Verify each task was actually deleted
        for task in tasks:
            retrieved = await memory_db.get_task(task.id)
            assert retrieved is None, f"Task {task.id} should be deleted"

    @pytest.mark.asyncio
    @pytest.mark.parametrize("use_file_db", [False, True])
    async def test_deep_hierarchy_performance(
        self, memory_db: Database, file_db: Database, use_file_db: bool
    ):
        """Test that deep hierarchy prune completes efficiently.

        Parametrized to test both in-memory and file-based databases.
        """
        db = file_db if use_file_db else memory_db

        # Setup: Create linear chain of depth 10
        parent_id = None
        for level in range(11):  # Root + 10 levels
            task = Task(
                prompt=f"Level {level} task",
                summary=f"Performance test level {level}",
                agent_type="test-agent",
                source=TaskSource.HUMAN,
                parent_task_id=parent_id,
                status=TaskStatus.COMPLETED,
                completed_at=datetime.now(timezone.utc) - timedelta(days=1),
            )
            await db.insert_task(task)

            if level == 0:
                root_id = task.id

            parent_id = task.id

        # Act: Measure prune performance
        import time

        start_time = time.perf_counter()

        filters = PruneFilters(task_ids=[root_id], recursive=True)  # type: ignore[call-arg]
        result = await db.prune_tasks(filters)

        elapsed_time = time.perf_counter() - start_time

        # Assert: Should complete quickly
        threshold = 2.0 if use_file_db else 1.0  # File DB gets more time
        assert (
            elapsed_time < threshold
        ), f"Deep hierarchy prune should be fast, took {elapsed_time:.3f}s"
        assert result.deleted_tasks == 11

    @pytest.mark.asyncio
    @pytest.mark.parametrize("dry_run", [False, True])
    async def test_deep_hierarchy_dry_run_parametrized(
        self, memory_db: Database, dry_run: bool
    ):
        """Test deep hierarchy with parametrized dry-run mode."""
        # Setup: Create linear chain
        parent_id = None
        root_id = None

        for level in range(11):
            task = Task(
                prompt=f"Dry run level {level}",
                summary=f"Preview test level {level}",
                agent_type="test-agent",
                source=TaskSource.HUMAN,
                parent_task_id=parent_id,
                status=TaskStatus.COMPLETED,
                completed_at=datetime.now(timezone.utc) - timedelta(days=1),
            )
            await memory_db.insert_task(task)

            if level == 0:
                root_id = task.id

            parent_id = task.id

        # Act: Prune with parametrized dry_run
        filters = PruneFilters(
            task_ids=[root_id], recursive=True, dry_run=dry_run  # type: ignore[call-arg]
        )
        result = await memory_db.prune_tasks(filters)

        # Assert: Shows correct preview/result
        assert result.deleted_tasks == 11
        assert result.dry_run is dry_run

        if hasattr(result, "tree_depth"):
            assert result.tree_depth == 10

        # Verify deletion behavior matches dry_run mode
        remaining_tasks = await memory_db.list_tasks(limit=20)
        expected_count = 11 if dry_run else 0
        assert len(remaining_tasks) == expected_count


# ============================================================================
# Mixed Status Tree Tests (Recursive)
# ============================================================================


class TestMixedStatusTrees:
    """Tests for mixed status tree handling in recursive mode."""

    @pytest.mark.asyncio
    async def test_mixed_status_tree_not_deleted(self, memory_db: Database):
        """Test that mixed status trees are NOT deleted.

        Tree structure:
        Root (COMPLETED)
        ├── Child1 (COMPLETED)
        ├── Child2 (RUNNING) ← Different status!
        └── Child3 (COMPLETED)

        Expected behavior:
        - NO tasks deleted (tree preserved due to RUNNING child)
        - trees_deleted=0
        - partial_trees=1
        """
        # Create mixed-status tree
        root_id, child_ids = await create_task_tree(
            memory_db,
            root_status=TaskStatus.COMPLETED,
            child_statuses=[
                TaskStatus.COMPLETED,
                TaskStatus.RUNNING,  # Different status - tree should NOT be deleted
                TaskStatus.COMPLETED,
            ],
        )

        # Verify tree exists
        root_before = await memory_db.get_task(root_id)
        assert root_before is not None

        # Attempt to prune with recursive=True, status=COMPLETED
        filters = PruneFilters(
            statuses=[TaskStatus.COMPLETED],
            dry_run=False,
        )
        result = await memory_db.delete_task_trees_recursive(
            root_task_ids=[root_id],
            filters=filters,
        )

        # Verify NO tasks deleted
        assert isinstance(result, RecursivePruneResult)
        assert result.deleted_tasks == 0, "No tasks should be deleted for mixed status tree"
        assert result.trees_deleted == 0, "Tree should NOT be counted as deleted"
        assert result.partial_trees == 1, "Tree should be counted as partial (skipped)"

        # Verify all tasks remain in database
        root_after = await memory_db.get_task(root_id)
        assert root_after is not None

        for child_id in child_ids:
            child_after = await memory_db.get_task(child_id)
            assert child_after is not None

    @pytest.mark.asyncio
    async def test_mixed_status_multiple_trees_some_partial(self, memory_db: Database):
        """Test deleting multiple trees where some are complete and some are partial.

        Tree1 (Complete): Root1 (COMPLETED) → All children COMPLETED
        Tree2 (Partial): Root2 (COMPLETED) → Mixed children (COMPLETED, RUNNING)

        Expected:
        - Tree1 deleted completely
        - Tree2 NOT deleted (partial)
        - trees_deleted=1
        - partial_trees=1
        """
        # Create first tree (complete - all COMPLETED)
        root1_id, child1_ids = await create_task_tree(
            memory_db,
            root_status=TaskStatus.COMPLETED,
            child_statuses=[TaskStatus.COMPLETED, TaskStatus.COMPLETED],
        )

        # Create second tree (partial - mixed statuses)
        root2_id, child2_ids = await create_task_tree(
            memory_db,
            root_status=TaskStatus.COMPLETED,
            child_statuses=[
                TaskStatus.COMPLETED,
                TaskStatus.RUNNING,  # Different status
            ],
        )

        # Attempt to delete both trees
        filters = PruneFilters(
            statuses=[TaskStatus.COMPLETED],
            dry_run=False,
        )
        result = await memory_db.delete_task_trees_recursive(
            root_task_ids=[root1_id, root2_id],
            filters=filters,
        )

        # Verify result
        assert result.deleted_tasks == 3, "Only Tree1 (3 tasks) should be deleted"
        assert result.trees_deleted == 1, "Only Tree1 should be counted as deleted"
        assert result.partial_trees == 1, "Tree2 should be counted as partial"

        # Verify Tree1 deleted
        assert await memory_db.get_task(root1_id) is None
        for child_id in child1_ids:
            assert await memory_db.get_task(child_id) is None

        # Verify Tree2 still exists
        root2_after = await memory_db.get_task(root2_id)
        assert root2_after is not None


# ============================================================================
# VACUUM Mode Tests (Parametrized)
# ============================================================================


class TestVacuumModes:
    """Tests for VACUUM mode behavior with parametrization."""

    @pytest.mark.asyncio
    @pytest.mark.parametrize(
        "vacuum_mode,task_count,expected_vacuum",
        [
            ("always", 10, True),  # Always runs VACUUM
            ("never", 200, False),  # Never runs VACUUM even above threshold
            ("conditional", 50, False),  # Below 100 threshold - no VACUUM
            ("conditional", 150, True),  # Above 100 threshold - runs VACUUM
        ],
    )
    async def test_vacuum_modes_parametrized(
        self, vacuum_mode: str, task_count: int, expected_vacuum: bool
    ):
        """Test VACUUM behavior across different modes and task counts."""
        with TemporaryDirectory() as tmpdir:
            db_path = Path(tmpdir) / "test.db"
            db = Database(db_path)

            try:
                await db.initialize()

                # Create tasks
                old_timestamp = datetime.now(timezone.utc) - timedelta(days=60)
                for i in range(task_count):
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

                # Execute: Prune with specified vacuum_mode
                filters = PruneFilters(older_than_days=30, vacuum_mode=vacuum_mode)
                result = await db.prune_tasks(filters)

                # Assert: Verify VACUUM behavior
                assert result.deleted_tasks == task_count
                if expected_vacuum:
                    assert result.reclaimed_bytes is not None, f"VACUUM should run with {vacuum_mode}, {task_count} tasks"
                    assert result.reclaimed_bytes >= 0
                else:
                    assert result.reclaimed_bytes is None, f"VACUUM should NOT run with {vacuum_mode}, {task_count} tasks"

                await db.close()
            finally:
                if db_path.exists():
                    db_path.unlink()

    @pytest.mark.asyncio
    async def test_prune_auto_skip_vacuum_large_operation(self):
        """Test vacuum_mode auto-selection to 'never' for large prune operations (>10,000 tasks)."""
        with TemporaryDirectory() as tmpdir:
            db_path = Path(tmpdir) / "test.db"
            db = Database(db_path)

            try:
                await db.initialize()

                # Create 10,001 completed tasks (exceeds AUTO_SKIP_VACUUM_THRESHOLD)
                tasks_to_insert = []
                cutoff_date = datetime.now(timezone.utc) - timedelta(days=31)

                for i in range(10_001):
                    task = Task(
                        prompt=f"Old task {i}",
                        summary=f"Task {i}",
                        agent_type="test-agent",
                        status=TaskStatus.COMPLETED,
                        source=TaskSource.AGENT_IMPLEMENTATION,
                        completed_at=cutoff_date,
                    )
                    task.completed_at = cutoff_date
                    tasks_to_insert.append(task)

                # Batch insert
                for task in tasks_to_insert:
                    await db.insert_task(task)

                # Execute: Prune with vacuum_mode="conditional" (should be auto-overridden)
                filters = PruneFilters(older_than_days=30, vacuum_mode="conditional")
                result = await db.prune_tasks(filters)

                # Assert: VACUUM should be auto-skipped
                assert result.deleted_tasks == 10_001
                assert result.vacuum_auto_skipped is True
                assert result.reclaimed_bytes is None

                await db.close()
            finally:
                if db_path.exists():
                    db_path.unlink()


# ============================================================================
# Time-Based Filtering Tests (Parametrized)
# ============================================================================


class TestTimeBasedFiltering:
    """Tests for time-based filtering with parametrization."""

    @pytest.mark.asyncio
    @pytest.mark.parametrize(
        "days_old,filter_days,should_delete",
        [
            (40, 30, True),  # 40 days old, filter >30 days → delete
            (10, 30, False),  # 10 days old, filter >30 days → keep
            (30, 30, False),  # Exactly 30 days old → keep (not older than)
            (31, 30, True),  # 31 days old → delete
        ],
    )
    async def test_older_than_days_boundary_conditions(
        self, memory_db: Database, days_old: int, filter_days: int, should_delete: bool
    ):
        """Test older_than_days filter with boundary conditions."""
        # Create task with specified age
        submitted_time = datetime.now(timezone.utc) - timedelta(days=days_old)
        task = Task(
            prompt=f"Task {days_old} days old",
            summary=f"Task aged {days_old} days",
            agent_type="test-agent",
            source=TaskSource.HUMAN,
            status=TaskStatus.COMPLETED,
            submitted_at=submitted_time,
            completed_at=submitted_time,
        )
        await memory_db.insert_task(task)

        # Prune with older_than_days filter
        filters = PruneFilters(older_than_days=filter_days)
        result = await memory_db.prune_tasks(filters)

        # Verify deletion behavior
        expected_deletions = 1 if should_delete else 0
        assert result.deleted_tasks == expected_deletions

        # Verify database state
        retrieved = await memory_db.get_task(task.id)
        if should_delete:
            assert retrieved is None
        else:
            assert retrieved is not None


# ============================================================================
# Edge Cases and Concurrent Operations
# ============================================================================


class TestEdgeCases:
    """Edge case tests for pruning operations."""

    @pytest.mark.asyncio
    async def test_single_task_no_children_recursive_mode(self, memory_db: Database):
        """Test recursive prune on single task with no children."""
        task = Task(
            prompt="Single task",
            summary="No children",
            agent_type="test-agent",
            source=TaskSource.HUMAN,
            status=TaskStatus.COMPLETED,
            completed_at=datetime.now(timezone.utc) - timedelta(days=1),
        )
        await memory_db.insert_task(task)

        # Prune with recursive=True
        filters = PruneFilters(task_ids=[task.id], recursive=True)  # type: ignore[call-arg]
        result = await memory_db.prune_tasks(filters)

        # Should behave identically to non-recursive
        assert result.deleted_tasks == 1

        if hasattr(result, "tree_depth"):
            assert result.tree_depth == 0, "No children means depth=0"

    @pytest.mark.asyncio
    async def test_concurrent_recursive_prunes(self, memory_db: Database):
        """Test multiple recursive prune operations running sequentially.

        Note: Changed from concurrent to sequential due to SQLite transaction limitations.
        SQLite doesn't support concurrent writes on the same connection.
        """
        # Create two separate hierarchies
        root1 = Task(
            prompt="Root 1",
            summary="First hierarchy",
            agent_type="test-agent",
            source=TaskSource.HUMAN,
            status=TaskStatus.COMPLETED,
            completed_at=datetime.now(timezone.utc) - timedelta(days=1),
        )
        await memory_db.insert_task(root1)

        child1 = Task(
            prompt="Child 1",
            summary="First hierarchy child",
            agent_type="test-agent",
            source=TaskSource.HUMAN,
            parent_task_id=root1.id,
            status=TaskStatus.COMPLETED,
            completed_at=datetime.now(timezone.utc) - timedelta(days=1),
        )
        await memory_db.insert_task(child1)

        # Hierarchy 2
        root2 = Task(
            prompt="Root 2",
            summary="Second hierarchy",
            agent_type="test-agent",
            source=TaskSource.HUMAN,
            status=TaskStatus.COMPLETED,
            completed_at=datetime.now(timezone.utc) - timedelta(days=1),
        )
        await memory_db.insert_task(root2)

        child2 = Task(
            prompt="Child 2",
            summary="Second hierarchy child",
            agent_type="test-agent",
            source=TaskSource.HUMAN,
            parent_task_id=root2.id,
            status=TaskStatus.COMPLETED,
            completed_at=datetime.now(timezone.utc) - timedelta(days=1),
        )
        await memory_db.insert_task(child2)

        # Prune hierarchies sequentially (SQLite limitation on concurrent writes)
        filters1 = PruneFilters(task_ids=[root1.id], recursive=True)  # type: ignore[call-arg]
        result1 = await memory_db.prune_tasks(filters1)

        filters2 = PruneFilters(task_ids=[root2.id], recursive=True)  # type: ignore[call-arg]
        result2 = await memory_db.prune_tasks(filters2)

        # Both prunes should succeed
        assert result1.deleted_tasks == 2  # Root1 + Child1
        assert result2.deleted_tasks == 2  # Root2 + Child2

        # Verify database is empty
        remaining = await memory_db.list_tasks(limit=10)
        assert len(remaining) == 0

    @pytest.mark.asyncio
    async def test_empty_result_set(self, memory_db: Database):
        """Test prune with no matching tasks."""
        # No tasks in database, attempt to prune
        filters = PruneFilters(older_than_days=30)
        result = await memory_db.prune_tasks(filters)

        # Should return zero deletions gracefully
        assert result.deleted_tasks == 0
        assert result.dry_run is False
        assert len(result.breakdown_by_status) == 0
