"""Integration tests for recursive tree deletion (mixed status scenario).

Tests end-to-end workflows for delete_task_trees_recursive with mixed status trees:
- Mixed status tree preservation (tree NOT deleted when descendants have different statuses)
- Correct partial_trees count
- All tasks remain in database when tree doesn't match criteria
- Dry run mode shows correct partial_trees count
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


# Test Cases

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
