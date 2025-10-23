"""Performance tests for recursive prune_tasks operation.

Validates NFR001 performance targets:
- NFR001: 1000-task tree prune < 5 seconds
- NFR002: 10k-task tree memory usage < 500MB
- NFR003: 100-level deep tree performance
- NFR004: 5000-child wide tree performance
- NFR005: CTE-based deletion efficiency

Uses pytest-benchmark for reliable measurements and memory_profiler for memory analysis.
"""

import asyncio
import os
import time
from datetime import datetime, timezone
from pathlib import Path
from statistics import mean, median
from typing import AsyncGenerator
from uuid import uuid4

import psutil
import pytest
from abathur.domain.models import TaskSource, TaskStatus
from abathur.infrastructure.database import Database, PruneFilters


# Fixtures


@pytest.fixture
async def memory_db() -> AsyncGenerator[Database, None]:
    """In-memory database for fast benchmarking."""
    db = Database(Path(":memory:"))
    await db.initialize()
    yield db
    await db.close()


@pytest.fixture
async def file_db(tmp_path: Path) -> AsyncGenerator[Database, None]:
    """File-based database for realistic I/O performance testing."""
    db_path = tmp_path / "perf_test.db"
    db = Database(db_path)
    await db.initialize()
    yield db
    await db.close()


# Helper functions


async def create_task_tree(
    db: Database, num_tasks: int, depth: int = 3, children_per_level: int = 10
) -> list[str]:
    """Create hierarchical task tree for testing.

    Args:
        db: Database instance
        num_tasks: Total number of tasks to create (approximate)
        depth: Tree depth (default: 3 levels)
        children_per_level: Number of children per parent (default: 10)

    Returns:
        List of task IDs created
    """
    task_ids = []

    async with db._get_connection() as conn:
        # Create root task
        root_id = str(uuid4())
        await conn.execute(
            """
            INSERT INTO tasks (
                id, prompt, agent_type, priority, status, calculated_priority,
                submitted_at, last_updated_at, source, dependency_depth, input_data
            ) VALUES (?, ?, ?, ?, ?, ?, datetime('now'), datetime('now'), ?, ?, '{}')
            """,
            (
                root_id,
                "Root task",
                "requirements-gatherer",
                5,
                TaskStatus.COMPLETED.value,
                5.0,
                TaskSource.HUMAN.value,
                0,
            ),
        )
        task_ids.append(root_id)

        # Build tree structure recursively
        current_level = [root_id]
        tasks_created = 1

        for level in range(1, depth + 1):
            next_level = []
            for parent_id in current_level:
                for i in range(children_per_level):
                    if tasks_created >= num_tasks:
                        break

                    child_id = str(uuid4())
                    await conn.execute(
                        """
                        INSERT INTO tasks (
                            id, prompt, agent_type, priority, status, calculated_priority,
                            submitted_at, last_updated_at, source, dependency_depth,
                            input_data, parent_task_id
                        ) VALUES (?, ?, ?, ?, ?, ?, datetime('now'), datetime('now'), ?, ?, '{}', ?)
                        """,
                        (
                            child_id,
                            f"Child task level {level}",
                            "requirements-gatherer",
                            5,
                            TaskStatus.COMPLETED.value,
                            5.0,
                            TaskSource.HUMAN.value,
                            level,
                            parent_id,
                        ),
                    )
                    task_ids.append(child_id)
                    next_level.append(child_id)
                    tasks_created += 1

                if tasks_created >= num_tasks:
                    break

            current_level = next_level
            if tasks_created >= num_tasks or not current_level:
                break

        await conn.commit()

    return task_ids


async def create_flat_tasks(db: Database, num_tasks: int, status: TaskStatus) -> list[str]:
    """Create flat list of tasks (no hierarchy) for testing.

    Args:
        db: Database instance
        num_tasks: Number of tasks to create
        status: Task status

    Returns:
        List of task IDs created
    """
    task_ids = []

    async with db._get_connection() as conn:
        for i in range(num_tasks):
            task_id = str(uuid4())
            await conn.execute(
                """
                INSERT INTO tasks (
                    id, prompt, agent_type, priority, status, calculated_priority,
                    submitted_at, last_updated_at, source, dependency_depth, input_data
                ) VALUES (?, ?, ?, ?, ?, ?, datetime('now'), datetime('now'), ?, ?, '{}')
                """,
                (
                    task_id,
                    f"Flat task {i}",
                    "requirements-gatherer",
                    5,
                    status.value,
                    5.0,
                    TaskSource.HUMAN.value,
                    0,
                ),
            )
            task_ids.append(task_id)

        await conn.commit()

    return task_ids


async def create_deep_tree(db: Database, depth: int) -> list[str]:
    """Create very deep linear tree (one child per level).

    Args:
        db: Database instance
        depth: Tree depth (e.g., 100 levels)

    Returns:
        List of task IDs created
    """
    task_ids = []
    parent_id = None

    async with db._get_connection() as conn:
        for level in range(depth):
            task_id = str(uuid4())
            await conn.execute(
                """
                INSERT INTO tasks (
                    id, prompt, agent_type, priority, status, calculated_priority,
                    submitted_at, last_updated_at, source, dependency_depth,
                    input_data, parent_task_id
                ) VALUES (?, ?, ?, ?, ?, ?, datetime('now'), datetime('now'), ?, ?, '{}', ?)
                """,
                (
                    task_id,
                    f"Deep task level {level}",
                    "requirements-gatherer",
                    5,
                    TaskStatus.COMPLETED.value,
                    5.0,
                    TaskSource.HUMAN.value,
                    level,
                    parent_id,
                ),
            )
            task_ids.append(task_id)
            parent_id = task_id

        await conn.commit()

    return task_ids


async def create_wide_tree(db: Database, num_children: int) -> list[str]:
    """Create very wide tree (one root, many children).

    Args:
        db: Database instance
        num_children: Number of children (e.g., 5000)

    Returns:
        List of task IDs created
    """
    task_ids = []

    async with db._get_connection() as conn:
        # Create root
        root_id = str(uuid4())
        await conn.execute(
            """
            INSERT INTO tasks (
                id, prompt, agent_type, priority, status, calculated_priority,
                submitted_at, last_updated_at, source, dependency_depth, input_data
            ) VALUES (?, ?, ?, ?, ?, ?, datetime('now'), datetime('now'), ?, ?, '{}')
            """,
            (
                root_id,
                "Root task",
                "requirements-gatherer",
                5,
                TaskStatus.COMPLETED.value,
                5.0,
                TaskSource.HUMAN.value,
                0,
            ),
        )
        task_ids.append(root_id)

        # Create children
        for i in range(num_children):
            child_id = str(uuid4())
            await conn.execute(
                """
                INSERT INTO tasks (
                    id, prompt, agent_type, priority, status, calculated_priority,
                    submitted_at, last_updated_at, source, dependency_depth,
                    input_data, parent_task_id
                ) VALUES (?, ?, ?, ?, ?, ?, datetime('now'), datetime('now'), ?, ?, '{}', ?)
                """,
                (
                    child_id,
                    f"Wide child {i}",
                    "requirements-gatherer",
                    5,
                    TaskStatus.COMPLETED.value,
                    5.0,
                    TaskSource.HUMAN.value,
                    1,
                    root_id,
                ),
            )
            task_ids.append(child_id)

        await conn.commit()

    return task_ids


# Benchmark Tests


@pytest.mark.performance
def test_1000_task_tree_under_5s(benchmark, tmp_path: Path):
    """NFR001: 1000-task tree prune < 5 seconds.

    Tests the core performance requirement for recursive prune operation.
    Uses file-based database for realistic I/O performance.

    Performance Target: Mean < 5.0 seconds
    """
    print("\nPreparing 1000-task tree prune benchmark...")

    # Counter for unique database files per iteration
    iteration = [0]

    # Define complete setup + prune + teardown operation
    def prune_1000_tasks():
        """Setup database, prune all completed tasks, teardown."""
        # Use unique database file for each iteration
        db_path = tmp_path / f"perf_test_{iteration[0]}.db"
        iteration[0] += 1

        async def full_operation():
            # Setup: Create database and tasks
            db = Database(db_path)
            await db.initialize()
            task_ids = await create_task_tree(db, num_tasks=1000, depth=3, children_per_level=10)
            num_tasks = len(task_ids)

            # Prune operation (what we're benchmarking)
            result = await db.prune_tasks(
                PruneFilters(
                    statuses=[TaskStatus.COMPLETED],
                    vacuum_mode="never",  # Skip VACUUM for pure delete performance
                )
            )

            # Teardown
            await db.close()
            return result, num_tasks

        return asyncio.run(full_operation())

    # Run benchmark
    result, num_tasks = benchmark(prune_1000_tasks)

    print(f"\nPrune Result:")
    print(f"  Deleted tasks: {result.deleted_tasks}")
    print(f"  Deleted dependencies: {result.deleted_dependencies}")
    print(f"\nBenchmark Stats:")
    print(f"  Mean:   {benchmark.stats['mean']:.3f}s")
    print(f"  Median: {benchmark.stats['median']:.3f}s")
    print(f"  Min:    {benchmark.stats['min']:.3f}s")
    print(f"  Max:    {benchmark.stats['max']:.3f}s")

    # Validate NFR001: Mean < 5.0 seconds
    assert benchmark.stats["mean"] < 5.0, (
        f"NFR001 FAILED: Mean {benchmark.stats['mean']:.3f}s exceeds 5.0s target"
    )

    # Verify all tasks were deleted
    assert result.deleted_tasks == num_tasks, f"Expected {num_tasks} tasks deleted"


@pytest.mark.asyncio
@pytest.mark.performance
async def test_10k_task_tree_memory(file_db: Database):
    """NFR002: 10k-task tree memory usage < 500MB.

    Tests memory consumption for large-scale prune operation.

    Performance Target: Peak memory < 500MB
    """
    # Pre-populate with 10k-task tree
    print("\nCreating 10k-task hierarchical tree...")
    task_ids = await create_task_tree(file_db, num_tasks=10000, depth=4, children_per_level=22)

    print(f"Created {len(task_ids)} tasks in hierarchical tree")

    # Measure memory before
    process = psutil.Process(os.getpid())
    mem_before = process.memory_info().rss / 1024 / 1024  # MB

    # Run prune operation
    start = time.perf_counter()
    result = await file_db.prune_tasks(
        PruneFilters(
            statuses=[TaskStatus.COMPLETED],
            vacuum_mode="never",  # Skip VACUUM for pure delete performance
        )
    )
    duration = time.perf_counter() - start

    # Measure memory after
    mem_after = process.memory_info().rss / 1024 / 1024  # MB
    mem_used = mem_after - mem_before

    print(f"\nPrune Result:")
    print(f"  Deleted tasks: {result.deleted_tasks}")
    print(f"  Duration: {duration:.2f}s")
    print(f"  Memory before: {mem_before:.2f} MB")
    print(f"  Memory after: {mem_after:.2f} MB")
    print(f"  Memory used: {mem_used:.2f} MB")

    # Validate NFR002: Memory usage < 500MB
    assert mem_used < 500, f"NFR002 FAILED: Memory usage {mem_used:.2f}MB exceeds 500MB limit"

    # Verify all tasks were deleted
    assert result.deleted_tasks == len(task_ids)


@pytest.mark.performance
def test_deep_tree_performance(benchmark, tmp_path: Path):
    """NFR003: 100-level deep tree performance.

    Tests performance on pathologically deep hierarchies.
    Validates that orphaning logic handles deep recursion efficiently.

    Performance Target: Mean < 2.0 seconds for 100-level tree
    """
    db_path = tmp_path / "deep_tree.db"

    # Setup
    async def setup():
        db = Database(db_path)
        await db.initialize()
        task_ids = await create_deep_tree(db, depth=100)
        await db.close()
        return len(task_ids)

    print("\nCreating 100-level deep tree...")
    num_tasks = asyncio.run(setup())
    print(f"Created {num_tasks} tasks in 100-level deep tree")

    # Define prune operation
    def prune_deep_tree():
        """Prune all completed tasks."""
        async def do_prune():
            db = Database(db_path)
            await db.initialize()
            result = await db.prune_tasks(
                PruneFilters(
                    statuses=[TaskStatus.COMPLETED],
                    vacuum_mode="never",
                )
            )
            await db.close()
            return result

        return asyncio.run(do_prune())

    # Run benchmark
    result = benchmark(prune_deep_tree)

    print(f"\nPrune Result:")
    print(f"  Deleted tasks: {result.deleted_tasks}")
    print(f"\nBenchmark Stats:")
    print(f"  Mean:   {benchmark.stats['mean']:.3f}s")
    print(f"  Median: {benchmark.stats['median']:.3f}s")

    # Validate NFR003: Mean < 2.0 seconds for 100-level tree
    assert benchmark.stats["mean"] < 2.0, (
        f"NFR003 FAILED: Mean {benchmark.stats['mean']:.3f}s exceeds 2.0s target"
    )

    # Verify all tasks were deleted
    assert result.deleted_tasks == num_tasks


@pytest.mark.performance
def test_wide_tree_performance(benchmark, tmp_path: Path):
    """NFR004: 5000-child wide tree performance.

    Tests performance on very wide hierarchies.
    Validates that orphaning logic handles many children efficiently.

    Performance Target: Mean < 3.0 seconds for 5000-child tree
    """
    db_path = tmp_path / "wide_tree.db"

    # Setup
    async def setup():
        db = Database(db_path)
        await db.initialize()
        task_ids = await create_wide_tree(db, num_children=5000)
        await db.close()
        return len(task_ids)

    print("\nCreating wide tree (1 root + 5000 children)...")
    num_tasks = asyncio.run(setup())
    print(f"Created {num_tasks} tasks in wide tree")

    # Define prune operation
    def prune_wide_tree():
        """Prune all completed tasks."""
        async def do_prune():
            db = Database(db_path)
            await db.initialize()
            result = await db.prune_tasks(
                PruneFilters(
                    statuses=[TaskStatus.COMPLETED],
                    vacuum_mode="never",
                )
            )
            await db.close()
            return result

        return asyncio.run(do_prune())

    # Run benchmark
    result = benchmark(prune_wide_tree)

    print(f"\nPrune Result:")
    print(f"  Deleted tasks: {result.deleted_tasks}")
    print(f"\nBenchmark Stats:")
    print(f"  Mean:   {benchmark.stats['mean']:.3f}s")
    print(f"  Median: {benchmark.stats['median']:.3f}s")

    # Validate NFR004: Mean < 3.0 seconds for 5000-child tree
    assert benchmark.stats["mean"] < 3.0, (
        f"NFR004 FAILED: Mean {benchmark.stats['mean']:.3f}s exceeds 3.0s target"
    )

    # Verify all tasks were deleted
    assert result.deleted_tasks == num_tasks


@pytest.mark.asyncio
@pytest.mark.performance
async def test_batch_deletion_efficiency(file_db: Database):
    """NFR005: Batch deletion efficiency (handles 10k+ tasks).

    Tests that batching logic (900 task IDs per batch) works efficiently.
    Validates no performance degradation due to batching overhead.

    Performance Target: Linear scaling (10k tasks ~10x slower than 1k tasks)
    """
    # Test with multiple batch sizes
    batch_sizes = [100, 500, 1000, 5000, 10000]
    results = []

    for size in batch_sizes:
        print(f"\nTesting batch size: {size} tasks")

        # Create flat tasks
        task_ids = await create_flat_tasks(file_db, num_tasks=size, status=TaskStatus.COMPLETED)

        # Measure prune performance
        start = time.perf_counter()
        result = await file_db.prune_tasks(
            PruneFilters(
                statuses=[TaskStatus.COMPLETED],
                vacuum_mode="never",
            )
        )
        duration = time.perf_counter() - start

        results.append({"size": size, "duration": duration, "deleted": result.deleted_tasks})

        print(f"  Deleted: {result.deleted_tasks} tasks in {duration:.3f}s")
        print(f"  Throughput: {result.deleted_tasks / duration:.1f} tasks/second")

    # Validate linear scaling (tolerance: 2x deviation)
    # Expected: 10k tasks should take ~10x longer than 1k tasks
    ratio_1k_to_10k = results[-1]["duration"] / results[2]["duration"]
    expected_ratio = results[-1]["size"] / results[2]["size"]

    print(f"\nScaling Analysis:")
    print(f"  1k tasks: {results[2]['duration']:.3f}s")
    print(f"  10k tasks: {results[-1]['duration']:.3f}s")
    print(f"  Actual ratio: {ratio_1k_to_10k:.2f}x")
    print(f"  Expected ratio: {expected_ratio:.2f}x")

    # Allow 2x tolerance for linear scaling
    assert ratio_1k_to_10k < expected_ratio * 2, (
        f"NFR005 FAILED: Scaling non-linear. "
        f"Actual {ratio_1k_to_10k:.2f}x vs expected {expected_ratio:.2f}x"
    )


# Query Optimization Tests


@pytest.mark.asyncio
@pytest.mark.performance
async def test_orphan_children_query_uses_index(file_db: Database):
    """Verify orphan children UPDATE uses parent_task_id index.

    Uses EXPLAIN QUERY PLAN to verify efficient query execution.
    """
    # Create sample tasks
    await create_task_tree(file_db, num_tasks=100, depth=2, children_per_level=10)

    # Get query plan for orphan children operation
    query = """
        UPDATE tasks
        SET parent_task_id = NULL
        WHERE parent_task_id IN (?, ?, ?)
    """
    plan = await file_db.explain_query_plan(query, ("id1", "id2", "id3"))

    plan_text = " ".join(plan)
    print(f"\nOrphan children query plan: {plan_text}")

    # Verify index usage (should use idx_tasks_parent or similar)
    # SQLite UPDATE statements may show "SEARCH" or index name
    assert "SCAN TABLE tasks" not in plan_text or "USING INDEX" in plan_text, (
        f"Query should use index for parent_task_id, got: {plan_text}"
    )


@pytest.mark.asyncio
@pytest.mark.performance
async def test_delete_tasks_query_uses_primary_key(file_db: Database):
    """Verify DELETE query uses primary key for efficient deletion."""
    # Create sample tasks
    await create_flat_tasks(file_db, num_tasks=100, status=TaskStatus.COMPLETED)

    # Get query plan for DELETE operation
    query = "DELETE FROM tasks WHERE id IN (?, ?, ?)"
    plan = await file_db.explain_query_plan(query, ("id1", "id2", "id3"))

    plan_text = " ".join(plan)
    print(f"\nDelete tasks query plan: {plan_text}")

    # Verify primary key usage
    assert "PRIMARY KEY" in plan_text or "USING INDEX" in plan_text, (
        f"Query should use primary key for id, got: {plan_text}"
    )


@pytest.mark.asyncio
@pytest.mark.performance
async def test_vacuum_performance(file_db: Database):
    """Test VACUUM performance on large deletions.

    VACUUM reclaims disk space but can be slow for large databases.
    This test validates VACUUM timing for 1000-task deletion.

    Performance Target: VACUUM < 2.0 seconds for 1000 tasks
    """
    # Create and delete 1000 tasks
    print("\nCreating 1000 tasks for VACUUM test...")
    task_ids = await create_flat_tasks(file_db, num_tasks=1000, status=TaskStatus.COMPLETED)

    # Delete tasks WITHOUT vacuum
    result = await file_db.prune_tasks(
        PruneFilters(
            statuses=[TaskStatus.COMPLETED],
            vacuum_mode="never",
        )
    )

    print(f"Deleted {result.deleted_tasks} tasks")

    # Measure VACUUM performance
    start = time.perf_counter()
    async with file_db._get_connection() as conn:
        await conn.execute("VACUUM")
    vacuum_duration = time.perf_counter() - start

    print(f"\nVACUUM Performance:")
    print(f"  Duration: {vacuum_duration:.3f}s")

    # Validate VACUUM performance
    assert vacuum_duration < 2.0, (
        f"VACUUM performance {vacuum_duration:.3f}s exceeds 2.0s target"
    )


# Memory Leak Detection


@pytest.mark.asyncio
@pytest.mark.performance
async def test_memory_leak_detection(file_db: Database):
    """Test for memory leaks in repeated prune operations.

    Runs prune operation 100 times and checks for memory growth.

    Performance Target: Memory growth < 50MB over 100 iterations
    """
    process = psutil.Process(os.getpid())
    mem_samples = []

    print("\nRunning 100 prune iterations...")

    for i in range(100):
        # Create 10 tasks
        task_ids = await create_flat_tasks(file_db, num_tasks=10, status=TaskStatus.COMPLETED)

        # Prune tasks
        await file_db.prune_tasks(
            PruneFilters(
                statuses=[TaskStatus.COMPLETED],
                vacuum_mode="never",
            )
        )

        # Sample memory every 10 iterations
        if i % 10 == 0:
            mem_mb = process.memory_info().rss / 1024 / 1024
            mem_samples.append(mem_mb)
            print(f"  Iteration {i}: {mem_mb:.2f} MB")

    # Check for memory growth
    initial_mem = mem_samples[0]
    final_mem = mem_samples[-1]
    growth = final_mem - initial_mem

    print(f"\nMemory Growth Analysis:")
    print(f"  Initial: {initial_mem:.2f} MB")
    print(f"  Final: {final_mem:.2f} MB")
    print(f"  Growth: {growth:.2f} MB")

    # Allow some growth, but not excessive (indicates leak)
    assert growth < 50, f"Potential memory leak: {growth:.2f}MB growth over 100 iterations"


if __name__ == "__main__":
    pytest.main([__file__, "-v", "-s", "--benchmark-only"])
