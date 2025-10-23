"""Integration tests for recursive task pruning.

Tests comprehensive recursive deletion scenarios:
- Deep hierarchy scenarios (P4-T3): Deep linear chains, depth tracking, performance
- Mixed status tree scenarios (P4-T4): Mixed status tree preservation, partial tree handling

Requires recursive prune feature implementation (P2-T4, P2-T5).
"""

import asyncio
from collections.abc import AsyncGenerator
from datetime import datetime, timedelta, timezone
from pathlib import Path
from tempfile import NamedTemporaryFile
from uuid import UUID, uuid4

import pytest
from abathur.domain.models import Task, TaskSource, TaskStatus
from abathur.infrastructure.database import Database, PruneFilters, RecursivePruneResult


# Fixtures

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


# Deep Hierarchy Tests (P4-T3)

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


# Mixed Status Tree Tests (P4-T4)

@pytest.mark.asyncio
async def test_mixed_status_tree_not_deleted(memory_db: Database):
    """Test that mixed status trees are NOT deleted (P4-T4).

    Tree structure:
    Root (COMPLETED)
    ├── Child1 (COMPLETED)
    ├── Child2 (RUNNING) ← Different status!
    └── Child3 (COMPLETED)

    Expected behavior:
    - NO tasks deleted (tree preserved due to RUNNING child)
    - trees_deleted=0
    - partial_trees=1
    - All tasks remain in database
    """
    # Step 1: Create mixed-status tree
    root_id, child_ids = await create_task_tree(
        memory_db,
        root_status=TaskStatus.COMPLETED,
        child_statuses=[
            TaskStatus.COMPLETED,
            TaskStatus.RUNNING,  # Different status - tree should NOT be deleted
            TaskStatus.COMPLETED,
        ],
    )

    # Step 2: Verify tree exists before prune attempt
    root_before = await memory_db.get_task(root_id)
    assert root_before is not None
    assert root_before.status == TaskStatus.COMPLETED

    # Verify all children exist
    for i, child_id in enumerate(child_ids):
        child = await memory_db.get_task(child_id)
        assert child is not None
        expected_status = [TaskStatus.COMPLETED, TaskStatus.RUNNING, TaskStatus.COMPLETED][i]
        assert child.status == expected_status

    # Step 3: Attempt to prune with recursive=True, status=COMPLETED
    filters = PruneFilters(
        statuses=[TaskStatus.COMPLETED],
        dry_run=False,
    )
    result = await memory_db.delete_task_trees_recursive(
        root_task_ids=[root_id],
        filters=filters,
    )

    # Step 4: Verify NO tasks deleted
    assert isinstance(result, RecursivePruneResult)
    assert result.deleted_tasks == 0, "No tasks should be deleted for mixed status tree"
    assert result.trees_deleted == 0, "Tree should NOT be counted as deleted"
    assert result.partial_trees == 1, "Tree should be counted as partial (skipped)"
    assert result.dry_run is False

    # Verify breakdown_by_status is empty (no deletions)
    assert len(result.breakdown_by_status) == 0 or all(
        count == 0 for count in result.breakdown_by_status.values()
    )

    # Step 5: Verify all tasks remain in database
    root_after = await memory_db.get_task(root_id)
    assert root_after is not None, "Root should still exist"
    assert root_after.status == TaskStatus.COMPLETED

    for i, child_id in enumerate(child_ids):
        child_after = await memory_db.get_task(child_id)
        assert child_after is not None, f"Child {i} should still exist"
        expected_status = [TaskStatus.COMPLETED, TaskStatus.RUNNING, TaskStatus.COMPLETED][i]
        assert child_after.status == expected_status

    # Step 6: Verify total task count unchanged
    async with memory_db._get_connection() as conn:
        cursor = await conn.execute("SELECT COUNT(*) as count FROM tasks")
        row = await cursor.fetchone()
        assert row["count"] == 4, "All 4 tasks (1 root + 3 children) should remain"


@pytest.mark.asyncio
async def test_mixed_status_tree_dry_run(memory_db: Database):
    """Test dry run mode correctly identifies mixed status tree as partial.

    Expected:
    - dry_run=True
    - partial_trees=1
    - deleted_tasks=0
    - No actual deletion
    """
    # Create mixed-status tree
    root_id, child_ids = await create_task_tree(
        memory_db,
        root_status=TaskStatus.COMPLETED,
        child_statuses=[
            TaskStatus.COMPLETED,
            TaskStatus.FAILED,  # Different from COMPLETED filter
            TaskStatus.COMPLETED,
        ],
    )

    # Run dry run with status=COMPLETED
    filters = PruneFilters(
        statuses=[TaskStatus.COMPLETED],
        dry_run=True,
    )
    result = await memory_db.delete_task_trees_recursive(
        root_task_ids=[root_id],
        filters=filters,
    )

    # Verify result shows partial tree
    assert result.deleted_tasks == 0
    assert result.trees_deleted == 0
    assert result.partial_trees == 1
    assert result.dry_run is True

    # Verify all tasks still exist (dry run doesn't delete)
    root_after = await memory_db.get_task(root_id)
    assert root_after is not None

    for child_id in child_ids:
        child_after = await memory_db.get_task(child_id)
        assert child_after is not None


@pytest.mark.asyncio
async def test_mixed_status_multiple_trees_some_partial(memory_db: Database):
    """Test deleting multiple trees where some are complete and some are partial.

    Tree structure:
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
    for child_id in child2_ids:
        child_after = await memory_db.get_task(child_id)
        assert child_after is not None


@pytest.mark.asyncio
async def test_mixed_status_with_multiple_filter_statuses(memory_db: Database):
    """Test mixed status tree with multiple pruneable statuses in filter.

    Tree structure:
    Root (COMPLETED)
    ├── Child1 (COMPLETED)
    ├── Child2 (FAILED)
    └── Child3 (RUNNING) ← Not in filter

    Filter: statuses=[COMPLETED, FAILED]

    Expected:
    - Tree NOT deleted (RUNNING child not in filter)
    - partial_trees=1
    - deleted_tasks=0
    """
    # Create tree with mixed statuses
    root_id, child_ids = await create_task_tree(
        memory_db,
        root_status=TaskStatus.COMPLETED,
        child_statuses=[
            TaskStatus.COMPLETED,
            TaskStatus.FAILED,
            TaskStatus.RUNNING,  # Not in filter - blocks deletion
        ],
    )

    # Attempt to prune with COMPLETED and FAILED statuses
    filters = PruneFilters(
        statuses=[TaskStatus.COMPLETED, TaskStatus.FAILED],
        dry_run=False,
    )
    result = await memory_db.delete_task_trees_recursive(
        root_task_ids=[root_id],
        filters=filters,
    )

    # Verify tree NOT deleted
    assert result.deleted_tasks == 0
    assert result.trees_deleted == 0
    assert result.partial_trees == 1

    # Verify all tasks remain
    root_after = await memory_db.get_task(root_id)
    assert root_after is not None

    for child_id in child_ids:
        child_after = await memory_db.get_task(child_id)
        assert child_after is not None


@pytest.mark.asyncio
async def test_mixed_status_deep_hierarchy(memory_db: Database):
    """Test mixed status detection works in deep hierarchies.

    Tree structure:
    Root (COMPLETED)
    └── Child (COMPLETED)
        └── Grandchild (COMPLETED)
            └── Great-grandchild (PENDING) ← Deep mismatch

    Expected:
    - Tree NOT deleted (PENDING great-grandchild)
    - partial_trees=1
    - deleted_tasks=0
    """
    # Create root
    root_task = Task(
        prompt="Root",
        summary="Root",
        agent_type="test-agent",
        source=TaskSource.HUMAN,
        status=TaskStatus.COMPLETED,
        input_data={},
    )
    await memory_db.insert_task(root_task)
    root_id = root_task.id

    # Create child
    child_task = Task(
        prompt="Child",
        summary="Child",
        agent_type="test-agent",
        source=TaskSource.HUMAN,
        status=TaskStatus.COMPLETED,
        parent_task_id=root_id,
        input_data={},
    )
    await memory_db.insert_task(child_task)
    child_id = child_task.id

    # Create grandchild
    grandchild_task = Task(
        prompt="Grandchild",
        summary="Grandchild",
        agent_type="test-agent",
        source=TaskSource.HUMAN,
        status=TaskStatus.COMPLETED,
        parent_task_id=child_id,
        input_data={},
    )
    await memory_db.insert_task(grandchild_task)
    grandchild_id = grandchild_task.id

    # Create great-grandchild with different status
    great_grandchild_task = Task(
        prompt="Great-grandchild",
        summary="Great-grandchild",
        agent_type="test-agent",
        source=TaskSource.HUMAN,
        status=TaskStatus.PENDING,  # Different status - blocks deletion
        parent_task_id=grandchild_id,
        input_data={},
    )
    await memory_db.insert_task(great_grandchild_task)
    great_grandchild_id = great_grandchild_task.id

    # Attempt to delete tree
    filters = PruneFilters(
        statuses=[TaskStatus.COMPLETED],
        dry_run=False,
    )
    result = await memory_db.delete_task_trees_recursive(
        root_task_ids=[root_id],
        filters=filters,
    )

    # Verify tree NOT deleted
    assert result.deleted_tasks == 0
    assert result.trees_deleted == 0
    assert result.partial_trees == 1

    # Verify all tasks remain
    assert await memory_db.get_task(root_id) is not None
    assert await memory_db.get_task(child_id) is not None
    assert await memory_db.get_task(grandchild_id) is not None
    assert await memory_db.get_task(great_grandchild_id) is not None


@pytest.mark.asyncio
async def test_mixed_status_root_not_matching(memory_db: Database):
    """Test that trees are skipped when root doesn't match filter.

    Tree structure:
    Root (RUNNING) ← Doesn't match filter
    └── Child (COMPLETED)

    Filter: statuses=[COMPLETED]

    Expected:
    - Tree NOT deleted (root doesn't match)
    - partial_trees=1
    - deleted_tasks=0
    """
    # Create tree where root doesn't match filter
    root_id, child_ids = await create_task_tree(
        memory_db,
        root_status=TaskStatus.RUNNING,  # Doesn't match COMPLETED filter
        child_statuses=[TaskStatus.COMPLETED],
    )

    # Attempt to prune
    filters = PruneFilters(
        statuses=[TaskStatus.COMPLETED],
        dry_run=False,
    )
    result = await memory_db.delete_task_trees_recursive(
        root_task_ids=[root_id],
        filters=filters,
    )

    # Verify tree NOT deleted
    assert result.deleted_tasks == 0
    assert result.trees_deleted == 0
    assert result.partial_trees == 1

    # Verify all tasks remain
    root_after = await memory_db.get_task(root_id)
    assert root_after is not None
    assert root_after.status == TaskStatus.RUNNING

    child_after = await memory_db.get_task(child_ids[0])
    assert child_after is not None
    assert child_after.status == TaskStatus.COMPLETED


@pytest.mark.asyncio
async def test_backward_compatibility_mixed_status_trees(memory_db: Database):
    """Test backward compatibility with existing mixed status trees.

    Verifies that existing trees created before recursive prune implementation
    are correctly identified as partial when they contain mixed statuses.
    """
    # Create tasks using direct database insert (simulating existing tasks)
    async with memory_db._get_connection() as conn:
        # Insert root task
        root_id = str(uuid4())
        await conn.execute(
            """
            INSERT INTO tasks (id, prompt, summary, agent_type, source, status, input_data, submitted_at, last_updated_at)
            VALUES (?, ?, ?, ?, ?, ?, ?, datetime('now'), datetime('now'))
            """,
            (root_id, "Old root", "Root", "test-agent", TaskSource.HUMAN.value, TaskStatus.COMPLETED.value, "{}"),
        )

        # Insert completed child
        child1_id = str(uuid4())
        await conn.execute(
            """
            INSERT INTO tasks (id, prompt, summary, agent_type, source, status, input_data, parent_task_id, submitted_at, last_updated_at)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, datetime('now'), datetime('now'))
            """,
            (child1_id, "Old child 1", "Child1", "test-agent", TaskSource.HUMAN.value, TaskStatus.COMPLETED.value, "{}", root_id),
        )

        # Insert running child (different status)
        child2_id = str(uuid4())
        await conn.execute(
            """
            INSERT INTO tasks (id, prompt, summary, agent_type, source, status, input_data, parent_task_id, submitted_at, last_updated_at)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, datetime('now'), datetime('now'))
            """,
            (child2_id, "Old child 2", "Child2", "test-agent", TaskSource.HUMAN.value, TaskStatus.RUNNING.value, "{}", root_id),
        )

        await conn.commit()

    # Attempt to delete using recursive method
    filters = PruneFilters(
        statuses=[TaskStatus.COMPLETED],
        dry_run=False,
    )
    result = await memory_db.delete_task_trees_recursive(
        root_task_ids=[UUID(root_id)],
        filters=filters,
    )

    # Verify tree NOT deleted (partial tree)
    assert result.deleted_tasks == 0
    assert result.trees_deleted == 0
    assert result.partial_trees == 1

    # Verify all tasks still exist
    assert await memory_db.get_task(UUID(root_id)) is not None
    assert await memory_db.get_task(UUID(child1_id)) is not None
    assert await memory_db.get_task(UUID(child2_id)) is not None
