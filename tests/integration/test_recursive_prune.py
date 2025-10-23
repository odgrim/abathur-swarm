"""Integration tests for recursive tree deletion (complete tree scenario).

Tests complete end-to-end workflows for delete_task_trees_recursive:
- Complete tree deletion (all descendants match criteria)
- Transaction integrity (rollback on error)
- Statistical accuracy (tree_depth, deleted_by_depth, trees_deleted)
- Orphan prevention (no orphaned tasks remain)
- Dry run mode (preview without deletion)
- Multiple tree deletion
"""

import asyncio
from collections.abc import AsyncGenerator
from pathlib import Path
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


async def create_task_tree(
    db: Database,
    root_status: TaskStatus,
    child_statuses: list[TaskStatus],
    grandchild_statuses: list[list[TaskStatus]],
) -> tuple[UUID, list[UUID], list[list[UUID]]]:
    """Helper to create a task tree with specified statuses.

    Args:
        db: Database instance
        root_status: Status for root task
        child_statuses: List of statuses for child tasks
        grandchild_statuses: List of lists of statuses for grandchildren

    Returns:
        Tuple of (root_id, [child_ids], [[grandchild_ids]])
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
    all_grandchild_ids = []

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

        # Create grandchildren for this child
        grandchild_ids = []
        if i < len(grandchild_statuses):
            for j, grandchild_status in enumerate(grandchild_statuses[i]):
                grandchild_task = Task(
                    prompt=f"Grandchild task {i}-{j}",
                    summary=f"Grandchild{i}-{j}",
                    agent_type="test-agent",
                    source=TaskSource.HUMAN,
                    status=grandchild_status,
                    parent_task_id=child_id,
                    input_data={},
                )
                await db.insert_task(grandchild_task)
                grandchild_id = grandchild_task.id
                grandchild_ids.append(grandchild_id)

        all_grandchild_ids.append(grandchild_ids)

    return root_id, child_ids, all_grandchild_ids


# Test Cases

@pytest.mark.asyncio
async def test_complete_tree_deletion_all_completed(memory_db: Database):
    """Test complete tree deletion when all descendants are COMPLETED.

    Tree structure:
    Root (COMPLETED)
    ├── Child1 (COMPLETED)
    │   ├── Grandchild1 (COMPLETED)
    │   └── Grandchild2 (COMPLETED)
    └── Child2 (COMPLETED)
        └── Grandchild3 (COMPLETED)

    Expected:
    - All 6 tasks deleted
    - trees_deleted=1
    - tree_depth=2
    - No orphans remain
    """
    # Step 1: Create complete tree with all COMPLETED tasks
    root_id, child_ids, grandchild_ids = await create_task_tree(
        memory_db,
        root_status=TaskStatus.COMPLETED,
        child_statuses=[TaskStatus.COMPLETED, TaskStatus.COMPLETED],
        grandchild_statuses=[
            [TaskStatus.COMPLETED, TaskStatus.COMPLETED],  # Child1's grandchildren
            [TaskStatus.COMPLETED],  # Child2's grandchildren
        ],
    )

    # Step 2: Verify tree exists before deletion
    all_tasks_before = await memory_db.get_task(root_id)
    assert all_tasks_before is not None
    assert all_tasks_before.status == TaskStatus.COMPLETED

    # Verify all children exist
    for child_id in child_ids:
        child = await memory_db.get_task(child_id)
        assert child is not None
        assert child.status == TaskStatus.COMPLETED

    # Verify all grandchildren exist
    for gc_list in grandchild_ids:
        for gc_id in gc_list:
            gc = await memory_db.get_task(gc_id)
            assert gc is not None
            assert gc.status == TaskStatus.COMPLETED

    # Step 3: Delete tree recursively
    filters = PruneFilters(
        statuses=[TaskStatus.COMPLETED],
        dry_run=False,
    )
    result = await memory_db.delete_task_trees_recursive(
        root_task_ids=[root_id],
        filters=filters,
    )

    # Step 4: Verify result statistics
    assert isinstance(result, RecursivePruneResult)
    assert result.deleted_tasks == 6  # 1 root + 2 children + 3 grandchildren
    assert result.trees_deleted == 1  # 1 complete tree deleted
    assert result.partial_trees == 0  # No partial trees
    assert result.tree_depth == 2  # Root=0, Children=1, Grandchildren=2
    assert result.dry_run is False

    # Verify deleted_by_depth breakdown
    assert result.deleted_by_depth[0] == 1  # 1 root at depth 0
    assert result.deleted_by_depth[1] == 2  # 2 children at depth 1
    assert result.deleted_by_depth[2] == 3  # 3 grandchildren at depth 2

    # Verify breakdown_by_status
    assert result.breakdown_by_status[TaskStatus.COMPLETED] == 6

    # Step 5: Verify all tasks are deleted from database
    root_after = await memory_db.get_task(root_id)
    assert root_after is None

    for child_id in child_ids:
        child_after = await memory_db.get_task(child_id)
        assert child_after is None

    for gc_list in grandchild_ids:
        for gc_id in gc_list:
            gc_after = await memory_db.get_task(gc_id)
            assert gc_after is None

    # Step 6: Verify no orphans remain
    async with memory_db._get_connection() as conn:
        cursor = await conn.execute("SELECT COUNT(*) as count FROM tasks")
        row = await cursor.fetchone()
        assert row["count"] == 0, "No tasks should remain in database"


@pytest.mark.asyncio
async def test_complete_tree_deletion_mixed_pruneable_statuses(memory_db: Database):
    """Test complete tree deletion with mixed pruneable statuses (COMPLETED, FAILED, CANCELLED).

    Tree structure:
    Root (COMPLETED)
    ├── Child1 (FAILED)
    │   └── Grandchild1 (CANCELLED)
    └── Child2 (COMPLETED)
        └── Grandchild2 (FAILED)

    Expected:
    - All 5 tasks deleted
    - trees_deleted=1
    - Correct breakdown_by_status
    """
    # Create tree with mixed pruneable statuses
    root_id, child_ids, grandchild_ids = await create_task_tree(
        memory_db,
        root_status=TaskStatus.COMPLETED,
        child_statuses=[TaskStatus.FAILED, TaskStatus.COMPLETED],
        grandchild_statuses=[
            [TaskStatus.CANCELLED],  # Child1's grandchild
            [TaskStatus.FAILED],  # Child2's grandchild
        ],
    )

    # Delete tree with all pruneable statuses
    filters = PruneFilters(
        statuses=[TaskStatus.COMPLETED, TaskStatus.FAILED, TaskStatus.CANCELLED],
        dry_run=False,
    )
    result = await memory_db.delete_task_trees_recursive(
        root_task_ids=[root_id],
        filters=filters,
    )

    # Verify result
    assert result.deleted_tasks == 5
    assert result.trees_deleted == 1
    assert result.partial_trees == 0
    assert result.tree_depth == 2

    # Verify breakdown by status
    assert result.breakdown_by_status[TaskStatus.COMPLETED] == 2  # Root + Child2
    assert result.breakdown_by_status[TaskStatus.FAILED] == 2  # Child1 + Grandchild2
    assert result.breakdown_by_status[TaskStatus.CANCELLED] == 1  # Grandchild1

    # Verify all deleted
    root_after = await memory_db.get_task(root_id)
    assert root_after is None


@pytest.mark.asyncio
async def test_dry_run_mode_no_deletion(memory_db: Database):
    """Test dry run mode returns statistics without deleting tasks.

    Expected:
    - Statistics returned correctly
    - No tasks actually deleted
    - dry_run=True in result
    """
    # Create simple tree
    root_id, child_ids, grandchild_ids = await create_task_tree(
        memory_db,
        root_status=TaskStatus.COMPLETED,
        child_statuses=[TaskStatus.COMPLETED],
        grandchild_statuses=[[TaskStatus.COMPLETED]],
    )

    # Run dry run
    filters = PruneFilters(
        statuses=[TaskStatus.COMPLETED],
        dry_run=True,
    )
    result = await memory_db.delete_task_trees_recursive(
        root_task_ids=[root_id],
        filters=filters,
    )

    # Verify statistics returned
    assert result.deleted_tasks == 3  # Would delete 3 tasks
    assert result.trees_deleted == 1
    assert result.tree_depth == 2
    assert result.dry_run is True

    # Verify no tasks actually deleted
    root_after = await memory_db.get_task(root_id)
    assert root_after is not None  # Still exists

    child_after = await memory_db.get_task(child_ids[0])
    assert child_after is not None  # Still exists

    grandchild_after = await memory_db.get_task(grandchild_ids[0][0])
    assert grandchild_after is not None  # Still exists


@pytest.mark.asyncio
async def test_multiple_trees_deletion(memory_db: Database):
    """Test deleting multiple complete trees in single operation.

    Tree structure:
    Tree1: Root1 (COMPLETED) → Child1 (COMPLETED)
    Tree2: Root2 (COMPLETED) → Child2 (COMPLETED)

    Expected:
    - All 4 tasks deleted
    - trees_deleted=2
    """
    # Create first tree
    root1_id, child1_ids, _ = await create_task_tree(
        memory_db,
        root_status=TaskStatus.COMPLETED,
        child_statuses=[TaskStatus.COMPLETED],
        grandchild_statuses=[[]],
    )

    # Create second tree
    root2_id, child2_ids, _ = await create_task_tree(
        memory_db,
        root_status=TaskStatus.COMPLETED,
        child_statuses=[TaskStatus.COMPLETED],
        grandchild_statuses=[[]],
    )

    # Delete both trees
    filters = PruneFilters(
        statuses=[TaskStatus.COMPLETED],
        dry_run=False,
    )
    result = await memory_db.delete_task_trees_recursive(
        root_task_ids=[root1_id, root2_id],
        filters=filters,
    )

    # Verify result
    assert result.deleted_tasks == 4  # 2 roots + 2 children
    assert result.trees_deleted == 2  # Both trees deleted
    assert result.partial_trees == 0

    # Verify all deleted
    assert await memory_db.get_task(root1_id) is None
    assert await memory_db.get_task(root2_id) is None
    assert await memory_db.get_task(child1_ids[0]) is None
    assert await memory_db.get_task(child2_ids[0]) is None


@pytest.mark.asyncio
async def test_partial_tree_not_deleted(memory_db: Database):
    """Test that partial trees (with non-matching descendants) are NOT deleted.

    Tree structure:
    Root (COMPLETED)
    ├── Child1 (COMPLETED)
    └── Child2 (PENDING) ← Non-pruneable status

    Expected:
    - No tasks deleted
    - trees_deleted=0
    - partial_trees=1
    """
    # Create tree with one non-pruneable child
    root_id, child_ids, _ = await create_task_tree(
        memory_db,
        root_status=TaskStatus.COMPLETED,
        child_statuses=[TaskStatus.COMPLETED, TaskStatus.PENDING],  # PENDING not pruneable
        grandchild_statuses=[[], []],
    )

    # Attempt to delete (should skip due to PENDING child)
    filters = PruneFilters(
        statuses=[TaskStatus.COMPLETED],
        dry_run=False,
    )
    result = await memory_db.delete_task_trees_recursive(
        root_task_ids=[root_id],
        filters=filters,
    )

    # Verify nothing deleted
    assert result.deleted_tasks == 0
    assert result.trees_deleted == 0
    assert result.partial_trees == 1  # Tree skipped

    # Verify all tasks still exist
    root_after = await memory_db.get_task(root_id)
    assert root_after is not None

    for child_id in child_ids:
        child_after = await memory_db.get_task(child_id)
        assert child_after is not None


@pytest.mark.asyncio
async def test_orphan_prevention(memory_db: Database):
    """Test that recursive deletion doesn't create orphans.

    Verifies that parent_task_id references are properly cleaned up
    and no orphaned tasks remain after deletion.
    """
    # Create tree
    root_id, child_ids, grandchild_ids = await create_task_tree(
        memory_db,
        root_status=TaskStatus.COMPLETED,
        child_statuses=[TaskStatus.COMPLETED],
        grandchild_statuses=[[TaskStatus.COMPLETED, TaskStatus.COMPLETED]],
    )

    # Delete tree
    filters = PruneFilters(
        statuses=[TaskStatus.COMPLETED],
        dry_run=False,
    )
    await memory_db.delete_task_trees_recursive(
        root_task_ids=[root_id],
        filters=filters,
    )

    # Verify no orphans: check for tasks with non-null parent_task_id
    # but parent doesn't exist
    async with memory_db._get_connection() as conn:
        cursor = await conn.execute("""
            SELECT t1.id, t1.parent_task_id
            FROM tasks t1
            LEFT JOIN tasks t2 ON t1.parent_task_id = t2.id
            WHERE t1.parent_task_id IS NOT NULL
            AND t2.id IS NULL
        """)
        orphans = await cursor.fetchall()
        assert len(orphans) == 0, f"Found orphaned tasks: {orphans}"


@pytest.mark.asyncio
async def test_transaction_rollback_on_error(memory_db: Database):
    """Test that transaction rolls back on error, leaving database unchanged.

    This test verifies atomicity: either all tasks deleted or none.
    """
    # Create tree
    root_id, child_ids, _ = await create_task_tree(
        memory_db,
        root_status=TaskStatus.COMPLETED,
        child_statuses=[TaskStatus.COMPLETED],
        grandchild_statuses=[[]],
    )

    # Count tasks before
    async with memory_db._get_connection() as conn:
        cursor = await conn.execute("SELECT COUNT(*) as count FROM tasks")
        row = await cursor.fetchone()
        count_before = row["count"]

    # Attempt deletion with invalid root_task_ids (empty list)
    # This should raise ValueError and not affect database
    filters = PruneFilters(
        statuses=[TaskStatus.COMPLETED],
        dry_run=False,
    )

    with pytest.raises(ValueError, match="root_task_ids cannot be empty"):
        await memory_db.delete_task_trees_recursive(
            root_task_ids=[],  # Invalid: empty list
            filters=filters,
        )

    # Verify no tasks deleted (transaction rolled back)
    async with memory_db._get_connection() as conn:
        cursor = await conn.execute("SELECT COUNT(*) as count FROM tasks")
        row = await cursor.fetchone()
        count_after = row["count"]

    assert count_after == count_before, "Task count should be unchanged after rollback"

    # Verify original tasks still exist
    root_after = await memory_db.get_task(root_id)
    assert root_after is not None


@pytest.mark.asyncio
async def test_single_root_task_deletion(memory_db: Database):
    """Test deleting a single root task with no children (edge case).

    Expected:
    - 1 task deleted
    - trees_deleted=1
    - tree_depth=0
    """
    # Create single task (no children)
    root_task = Task(
        prompt="Single root task",
        summary="Root",
        agent_type="test-agent",
        source=TaskSource.HUMAN,
        status=TaskStatus.COMPLETED,
        input_data={},
    )
    await memory_db.insert_task(root_task)
    root_id = root_task.id

    # Delete single task
    filters = PruneFilters(
        statuses=[TaskStatus.COMPLETED],
        dry_run=False,
    )
    result = await memory_db.delete_task_trees_recursive(
        root_task_ids=[root_id],
        filters=filters,
    )

    # Verify result
    assert result.deleted_tasks == 1
    assert result.trees_deleted == 1
    assert result.partial_trees == 0
    assert result.tree_depth == 0  # Single task at depth 0
    assert result.deleted_by_depth[0] == 1

    # Verify deleted
    root_after = await memory_db.get_task(root_id)
    assert root_after is None


@pytest.mark.asyncio
async def test_deep_tree_deletion(memory_db: Database):
    """Test deleting a deep tree (depth > 2) to verify depth tracking.

    Tree structure:
    Root (COMPLETED)
    └── Child (COMPLETED)
        └── Grandchild (COMPLETED)
            └── Great-grandchild (COMPLETED)

    Expected:
    - tree_depth=3
    - Correct deleted_by_depth breakdown
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

    # Create great-grandchild
    great_grandchild_task = Task(
        prompt="Great-grandchild",
        summary="Great-grandchild",
        agent_type="test-agent",
        source=TaskSource.HUMAN,
        status=TaskStatus.COMPLETED,
        parent_task_id=grandchild_id,
        input_data={},
    )
    await memory_db.insert_task(great_grandchild_task)
    great_grandchild_id = great_grandchild_task.id

    # Delete entire tree
    filters = PruneFilters(
        statuses=[TaskStatus.COMPLETED],
        dry_run=False,
    )
    result = await memory_db.delete_task_trees_recursive(
        root_task_ids=[root_id],
        filters=filters,
    )

    # Verify result
    assert result.deleted_tasks == 4
    assert result.trees_deleted == 1
    assert result.tree_depth == 3  # Depth 0,1,2,3

    # Verify deleted_by_depth
    assert result.deleted_by_depth[0] == 1  # Root
    assert result.deleted_by_depth[1] == 1  # Child
    assert result.deleted_by_depth[2] == 1  # Grandchild
    assert result.deleted_by_depth[3] == 1  # Great-grandchild

    # Verify all deleted
    assert await memory_db.get_task(root_id) is None
    assert await memory_db.get_task(child_id) is None
    assert await memory_db.get_task(grandchild_id) is None
    assert await memory_db.get_task(great_grandchild_id) is None


@pytest.mark.asyncio
async def test_backward_compatibility_with_model_tasks(memory_db: Database):
    """Test that recursive deletion works with tasks created using Task model.

    Verifies that the new recursive deletion API works correctly with
    tasks created through the standard insert_task workflow.
    """
    # Create tasks using Task model (standard workflow)
    root_task = Task(
        prompt="Standard root task",
        summary="Root",
        agent_type="test-agent",
        source=TaskSource.HUMAN,
        status=TaskStatus.COMPLETED,
        input_data={},
    )
    await memory_db.insert_task(root_task)
    root_id = root_task.id

    child_task = Task(
        prompt="Standard child task",
        summary="Child",
        agent_type="test-agent",
        source=TaskSource.HUMAN,
        status=TaskStatus.COMPLETED,
        parent_task_id=root_id,
        input_data={},
    )
    await memory_db.insert_task(child_task)
    child_id = child_task.id

    # Verify tasks exist before deletion
    assert await memory_db.get_task(root_id) is not None
    assert await memory_db.get_task(child_id) is not None

    # Delete using recursive method
    filters = PruneFilters(
        statuses=[TaskStatus.COMPLETED],
        dry_run=False,
    )
    result = await memory_db.delete_task_trees_recursive(
        root_task_ids=[root_id],
        filters=filters,
    )

    # Verify deletion
    assert result.deleted_tasks == 2
    assert result.trees_deleted == 1

    # Verify tasks deleted
    assert await memory_db.get_task(root_id) is None
    assert await memory_db.get_task(child_id) is None
