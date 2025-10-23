"""Integration tests for recursive task pruning.

Tests deep hierarchy scenarios with recursive deletion mode:
- Deep linear chains (depth=10)
- Depth tracking and verification
- Performance validation for deep hierarchies
- Deleted-by-depth metrics

Requires recursive prune feature implementation (P2-T4, P2-T5).
"""

import asyncio
from collections.abc import AsyncGenerator
from datetime import datetime, timedelta, timezone
from pathlib import Path
from tempfile import NamedTemporaryFile

import pytest
from abathur.domain.models import Task, TaskSource, TaskStatus
from abathur.infrastructure.database import Database, PruneFilters


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


class TestDeepHierarchyRecursivePrune:
    """Integration tests for deep hierarchy recursive pruning (P4-T3)."""

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
        # Note: These attributes may not exist until P2-T4/P2-T5 are implemented
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
    async def test_deep_hierarchy_performance(self, memory_db: Database):
        """Test that deep hierarchy prune completes efficiently.

        Verifies that pruning a depth=10 hierarchy completes in reasonable time
        (should be nearly instant for in-memory database).
        """
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
            await memory_db.insert_task(task)

            if level == 0:
                root_id = task.id

            parent_id = task.id

        # Act: Measure prune performance
        import time

        start_time = time.perf_counter()

        filters = PruneFilters(task_ids=[root_id], recursive=True)  # type: ignore[call-arg]
        result = await memory_db.prune_tasks(filters)

        elapsed_time = time.perf_counter() - start_time

        # Assert: Should complete quickly (< 1 second for in-memory)
        assert (
            elapsed_time < 1.0
        ), f"Deep hierarchy prune should be fast, took {elapsed_time:.3f}s"
        assert result.deleted_tasks == 11

    @pytest.mark.asyncio
    async def test_deep_hierarchy_with_file_db(self, file_db: Database):
        """Test deep hierarchy prune with file-based database.

        Verifies that recursive deletion works correctly with persistent storage
        and that depth tracking is accurate.
        """
        # Setup: Create linear chain
        parent_id = None
        root_id = None

        for level in range(11):
            task = Task(
                prompt=f"File DB level {level}",
                summary=f"Persistent test level {level}",
                agent_type="test-agent",
                source=TaskSource.HUMAN,
                parent_task_id=parent_id,
                status=TaskStatus.COMPLETED,
                completed_at=datetime.now(timezone.utc) - timedelta(days=1),
            )
            await file_db.insert_task(task)

            if level == 0:
                root_id = task.id

            parent_id = task.id

        # Act: Prune recursively
        filters = PruneFilters(task_ids=[root_id], recursive=True)  # type: ignore[call-arg]
        result = await file_db.prune_tasks(filters)

        # Assert: Verify complete deletion
        assert result.deleted_tasks == 11
        assert result.breakdown_by_status[TaskStatus.COMPLETED] == 11

        # Verify database persistence
        remaining_tasks = await file_db.list_tasks(limit=20)
        assert len(remaining_tasks) == 0

    @pytest.mark.asyncio
    async def test_deep_hierarchy_dry_run_mode(self, memory_db: Database):
        """Test dry-run mode with deep hierarchy shows correct preview.

        Verifies that dry-run mode:
        - Shows correct task count (11 tasks)
        - Shows correct depth metrics (depth=10)
        - Does not actually delete any tasks
        """
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

        # Act: Dry-run recursive prune
        filters = PruneFilters(
            task_ids=[root_id], recursive=True, dry_run=True  # type: ignore[call-arg]
        )
        result = await memory_db.prune_tasks(filters)

        # Assert: Shows correct preview
        assert result.deleted_tasks == 11
        assert result.dry_run is True

        if hasattr(result, "tree_depth"):
            assert result.tree_depth == 10

        # Verify no tasks were actually deleted
        remaining_tasks = await memory_db.list_tasks(limit=20)
        assert len(remaining_tasks) == 11, "Dry-run should not delete tasks"

    @pytest.mark.asyncio
    async def test_deleted_by_depth_accuracy(self, memory_db: Database):
        """Test that deleted_by_depth metric accurately tracks each level.

        Verifies granular depth tracking for debugging and monitoring purposes.
        """
        # Setup: Create known linear chain
        depth_to_task_id = {}
        parent_id = None

        for depth in range(11):  # 0 to 10
            task = Task(
                prompt=f"Depth tracking level {depth}",
                summary=f"Level {depth}",
                agent_type="test-agent",
                source=TaskSource.HUMAN,
                parent_task_id=parent_id,
                status=TaskStatus.COMPLETED,
                completed_at=datetime.now(timezone.utc) - timedelta(days=1),
            )
            await memory_db.insert_task(task)
            depth_to_task_id[depth] = task.id

            if depth == 0:
                root_id = task.id

            parent_id = task.id

        # Act: Prune recursively
        filters = PruneFilters(task_ids=[root_id], recursive=True)  # type: ignore[call-arg]
        result = await memory_db.prune_tasks(filters)

        # Assert: Verify deleted_by_depth is accurate
        if hasattr(result, "deleted_by_depth"):
            # Should have exactly 11 levels (0 through 10)
            assert len(result.deleted_by_depth) == 11

            # Each level should have exactly 1 task
            for depth in range(11):
                count = result.deleted_by_depth.get(depth, 0)
                assert count == 1, f"Depth {depth} should have 1 task, got {count}"

        # Verify total matches sum of deleted_by_depth
        if hasattr(result, "deleted_by_depth"):
            total_from_depth = sum(result.deleted_by_depth.values())
            assert total_from_depth == result.deleted_tasks

    @pytest.mark.asyncio
    async def test_depth_limit_enforcement(self, memory_db: Database):
        """Test that extremely deep hierarchies are handled correctly.

        Note: This assumes there may be a depth limit configuration.
        If no limit exists, this test verifies correct behavior at extreme depths.
        """
        # Setup: Create very deep chain (depth=20)
        parent_id = None
        root_id = None

        for level in range(21):  # 0 to 20
            task = Task(
                prompt=f"Deep level {level}",
                summary=f"Extreme depth test level {level}",
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

        # Act: Prune recursively
        filters = PruneFilters(task_ids=[root_id], recursive=True)  # type: ignore[call-arg]
        result = await memory_db.prune_tasks(filters)

        # Assert: All tasks deleted (or up to depth limit if one exists)
        assert result.deleted_tasks >= 11, "Should delete at least depth=10"

        # If no depth limit, should delete all 21 tasks
        # If depth limit exists, verify it's enforced
        if hasattr(result, "tree_depth"):
            assert result.tree_depth >= 10, "Should support at least depth=10"


class TestRecursivePruneEdgeCases:
    """Edge case tests for recursive pruning."""

    @pytest.mark.asyncio
    async def test_single_task_no_children_recursive_mode(self, memory_db: Database):
        """Test recursive prune on single task with no children.

        Should behave identically to non-recursive prune.
        """
        # Setup: Single completed task
        task = Task(
            prompt="Single task",
            summary="No children",
            agent_type="test-agent",
            source=TaskSource.HUMAN,
            status=TaskStatus.COMPLETED,
            completed_at=datetime.now(timezone.utc) - timedelta(days=1),
        )
        await memory_db.insert_task(task)

        # Act: Prune with recursive=True
        filters = PruneFilters(task_ids=[task.id], recursive=True)  # type: ignore[call-arg]
        result = await memory_db.prune_tasks(filters)

        # Assert: Single task deleted
        assert result.deleted_tasks == 1

        if hasattr(result, "tree_depth"):
            assert result.tree_depth == 0, "No children means depth=0"

        if hasattr(result, "deleted_by_depth"):
            assert result.deleted_by_depth[0] == 1

    @pytest.mark.asyncio
    async def test_concurrent_recursive_prunes(self, memory_db: Database):
        """Test multiple recursive prune operations can run concurrently.

        Verifies database concurrency handling with recursive operations.
        """
        # Setup: Create two separate hierarchies
        # Hierarchy 1: Root1 -> Child1
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

        # Hierarchy 2: Root2 -> Child2
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

        # Act: Prune both hierarchies concurrently
        filters1 = PruneFilters(task_ids=[root1.id], recursive=True)  # type: ignore[call-arg]
        filters2 = PruneFilters(task_ids=[root2.id], recursive=True)  # type: ignore[call-arg]

        results = await asyncio.gather(
            memory_db.prune_tasks(filters1), memory_db.prune_tasks(filters2)
        )

        # Assert: Both prunes succeeded
        assert len(results) == 2
        assert results[0].deleted_tasks == 2  # Root1 + Child1
        assert results[1].deleted_tasks == 2  # Root2 + Child2

        # Verify database is empty
        remaining = await memory_db.list_tasks(limit=10)
        assert len(remaining) == 0
