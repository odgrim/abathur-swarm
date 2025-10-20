"""Performance benchmarks for VACUUM operations.

Benchmarks VACUUM performance across different database sizes:
- Small database (100 tasks)
- Medium database (10k tasks)
- Large database (100k tasks)
- Incremental impact analysis

Performance Targets:
- Small DB (<1 second)
- Medium DB (<60 seconds)
- Large DB (<300 seconds / 5 minutes)
"""

import asyncio
import json
import time
from collections.abc import AsyncGenerator
from datetime import datetime, timedelta, timezone
from pathlib import Path
from tempfile import TemporaryDirectory

import pytest

from abathur.domain.models import Task, TaskSource, TaskStatus
from abathur.infrastructure.database import Database, PruneFilters


@pytest.fixture
async def file_db() -> AsyncGenerator[Database, None]:
    """Create file-based database for VACUUM benchmarks.

    File-based database is required for VACUUM operations
    (in-memory databases don't support VACUUM in the same way).
    """
    with TemporaryDirectory() as tmpdir:
        db_path = Path(tmpdir) / "benchmark.db"
        db = Database(db_path)
        await db.initialize()
        yield db
        await db.close()


async def _create_tasks(db: Database, count: int, age_days: int = 60) -> list[str]:
    """Helper to create tasks for benchmarking.

    Args:
        db: Database instance
        count: Number of tasks to create
        age_days: Age of tasks in days (for pruning tests)

    Returns:
        List of created task IDs
    """
    old_date = datetime.now(timezone.utc) - timedelta(days=age_days)
    task_ids = []

    # Create tasks in batches for better performance
    batch_size = 1000
    for batch_start in range(0, count, batch_size):
        batch_end = min(batch_start + batch_size, count)
        tasks = []

        for i in range(batch_start, batch_end):
            task = Task(
                prompt=f"Benchmark task {i}",
                summary=f"Task for performance testing {i}",
                agent_type="benchmark-agent",
                source=TaskSource.HUMAN,
                status=TaskStatus.COMPLETED,
                submitted_at=old_date,
                completed_at=old_date,
                input_data={"index": i, "batch": batch_start // batch_size},
                result_data={"output": f"result_{i}", "status": "success"},
            )
            tasks.append(task)

        # Insert batch
        for task in tasks:
            await db.insert_task(task)
            task_ids.append(task.id)

    return task_ids


async def _get_db_size(db_path: Path) -> int:
    """Get database file size in bytes."""
    return db_path.stat().st_size if db_path.exists() else 0


@pytest.mark.benchmark
@pytest.mark.asyncio
async def test_vacuum_small_db(file_db: Database) -> None:
    """Benchmark VACUUM on small database (100 tasks).

    Performance Target: <1 second

    This test:
    1. Creates 100 tasks
    2. Deletes 50 tasks
    3. Measures VACUUM duration
    4. Logs metrics: duration, space reclaimed
    """
    # Arrange: Create 100 tasks
    print("\n[Small DB Benchmark] Creating 100 tasks...")
    task_ids = await _create_tasks(file_db, count=100, age_days=60)
    assert len(task_ids) == 100

    # Get database path for size measurements
    db_path = file_db.db_path
    size_before = await _get_db_size(db_path)
    print(f"[Small DB] Database size before deletion: {size_before:,} bytes")

    # Act: Delete 50 tasks and measure VACUUM time
    print("[Small DB] Deleting 50 tasks with VACUUM...")
    start_time = time.perf_counter()

    filters = PruneFilters(
        older_than_days=30,
        vacuum_mode="always",  # Force VACUUM
        limit=50
    )
    result = await file_db.prune_tasks(filters)

    elapsed = time.perf_counter() - start_time

    # Assert: Verify deletion
    assert result.deleted_tasks == 50
    assert result.dry_run is False

    # Assert: VACUUM completed
    assert result.reclaimed_bytes is not None
    assert isinstance(result.reclaimed_bytes, int)
    assert result.reclaimed_bytes >= 0

    # Get final size
    size_after = await _get_db_size(db_path)
    actual_reclaimed = size_before - size_after

    # Log metrics
    metrics = {
        "test": "vacuum_small_db",
        "task_count": 100,
        "deleted_count": 50,
        "duration_seconds": round(elapsed, 3),
        "size_before_bytes": size_before,
        "size_after_bytes": size_after,
        "reclaimed_bytes": result.reclaimed_bytes,
        "actual_reclaimed_bytes": actual_reclaimed,
        "performance_target": "< 1 second",
        "target_met": elapsed < 1.0
    }
    print(f"\n[Small DB Metrics]\n{json.dumps(metrics, indent=2)}")

    # Assert: Performance target met
    assert elapsed < 1.0, f"VACUUM took {elapsed:.3f}s, expected < 1.0s"


@pytest.mark.benchmark
@pytest.mark.asyncio
async def test_vacuum_medium_db(file_db: Database) -> None:
    """Benchmark VACUUM on medium database (10k tasks).

    Performance Target: <60 seconds

    This test:
    1. Creates 10,000 tasks
    2. Deletes 5,000 tasks
    3. Measures VACUUM duration
    4. Asserts duration < 60 seconds
    5. Logs detailed metrics
    """
    # Arrange: Create 10,000 tasks
    print("\n[Medium DB Benchmark] Creating 10,000 tasks...")
    create_start = time.perf_counter()
    task_ids = await _create_tasks(file_db, count=10_000, age_days=60)
    create_elapsed = time.perf_counter() - create_start
    assert len(task_ids) == 10_000
    print(f"[Medium DB] Tasks created in {create_elapsed:.2f}s")

    # Get database size
    db_path = file_db.db_path
    size_before = await _get_db_size(db_path)
    print(f"[Medium DB] Database size before deletion: {size_before:,} bytes")

    # Act: Delete 5,000 tasks and measure VACUUM time
    print("[Medium DB] Deleting 5,000 tasks with VACUUM...")
    start_time = time.perf_counter()

    filters = PruneFilters(
        older_than_days=30,
        vacuum_mode="always",
        limit=5_000
    )
    result = await file_db.prune_tasks(filters)

    elapsed = time.perf_counter() - start_time

    # Assert: Verify deletion
    assert result.deleted_tasks == 5_000
    assert result.dry_run is False

    # Assert: VACUUM completed
    assert result.reclaimed_bytes is not None
    assert isinstance(result.reclaimed_bytes, int)
    assert result.reclaimed_bytes >= 0

    # Get final size
    size_after = await _get_db_size(db_path)
    actual_reclaimed = size_before - size_after
    reclaim_percentage = (actual_reclaimed / size_before * 100) if size_before > 0 else 0

    # Log metrics
    metrics = {
        "test": "vacuum_medium_db",
        "task_count": 10_000,
        "deleted_count": 5_000,
        "create_duration_seconds": round(create_elapsed, 3),
        "vacuum_duration_seconds": round(elapsed, 3),
        "size_before_bytes": size_before,
        "size_after_bytes": size_after,
        "reclaimed_bytes": result.reclaimed_bytes,
        "actual_reclaimed_bytes": actual_reclaimed,
        "reclaim_percentage": round(reclaim_percentage, 2),
        "performance_target": "< 60 seconds",
        "target_met": elapsed < 60.0
    }
    print(f"\n[Medium DB Metrics]\n{json.dumps(metrics, indent=2)}")

    # Assert: Performance target met
    assert elapsed < 60.0, f"VACUUM took {elapsed:.3f}s, expected < 60.0s"


@pytest.mark.benchmark
@pytest.mark.slow
@pytest.mark.asyncio
async def test_vacuum_large_db(file_db: Database) -> None:
    """Benchmark VACUUM on large database (100k tasks).

    Performance Target: <300 seconds (5 minutes)

    This test:
    1. Creates 100,000 tasks (may take several minutes)
    2. Deletes 50,000 tasks
    3. Measures VACUUM duration
    4. Asserts duration < 300 seconds
    5. Logs detailed metrics

    Note: This test is marked as 'slow' and may take 5-10 minutes total.
    Run with: pytest -m "benchmark and slow"
    """
    # Arrange: Create 100,000 tasks
    print("\n[Large DB Benchmark] Creating 100,000 tasks (this may take several minutes)...")
    create_start = time.perf_counter()
    task_ids = await _create_tasks(file_db, count=100_000, age_days=60)
    create_elapsed = time.perf_counter() - create_start
    assert len(task_ids) == 100_000
    print(f"[Large DB] Tasks created in {create_elapsed:.2f}s ({create_elapsed/60:.2f} minutes)")

    # Get database size
    db_path = file_db.db_path
    size_before = await _get_db_size(db_path)
    size_mb = size_before / (1024 * 1024)
    print(f"[Large DB] Database size before deletion: {size_before:,} bytes ({size_mb:.2f} MB)")

    # Act: Delete 50,000 tasks and measure VACUUM time
    print("[Large DB] Deleting 50,000 tasks with VACUUM...")
    start_time = time.perf_counter()

    filters = PruneFilters(
        older_than_days=30,
        vacuum_mode="always",
        limit=50_000
    )
    result = await file_db.prune_tasks(filters)

    elapsed = time.perf_counter() - start_time

    # Assert: Verify deletion
    assert result.deleted_tasks == 50_000
    assert result.dry_run is False

    # Assert: VACUUM completed
    assert result.reclaimed_bytes is not None
    assert isinstance(result.reclaimed_bytes, int)
    assert result.reclaimed_bytes >= 0

    # Get final size
    size_after = await _get_db_size(db_path)
    size_after_mb = size_after / (1024 * 1024)
    actual_reclaimed = size_before - size_after
    reclaimed_mb = actual_reclaimed / (1024 * 1024)
    reclaim_percentage = (actual_reclaimed / size_before * 100) if size_before > 0 else 0

    # Log detailed metrics
    metrics = {
        "test": "vacuum_large_db",
        "task_count": 100_000,
        "deleted_count": 50_000,
        "create_duration_seconds": round(create_elapsed, 3),
        "create_duration_minutes": round(create_elapsed / 60, 2),
        "vacuum_duration_seconds": round(elapsed, 3),
        "vacuum_duration_minutes": round(elapsed / 60, 2),
        "size_before_bytes": size_before,
        "size_before_mb": round(size_mb, 2),
        "size_after_bytes": size_after,
        "size_after_mb": round(size_after_mb, 2),
        "reclaimed_bytes": result.reclaimed_bytes,
        "actual_reclaimed_bytes": actual_reclaimed,
        "actual_reclaimed_mb": round(reclaimed_mb, 2),
        "reclaim_percentage": round(reclaim_percentage, 2),
        "performance_target": "< 300 seconds (5 minutes)",
        "target_met": elapsed < 300.0
    }
    print(f"\n[Large DB Metrics]\n{json.dumps(metrics, indent=2)}")

    # Assert: Performance target met
    assert elapsed < 300.0, f"VACUUM took {elapsed:.3f}s ({elapsed/60:.2f} min), expected < 300s (5 min)"


@pytest.mark.benchmark
@pytest.mark.asyncio
async def test_vacuum_incremental_impact(file_db: Database) -> None:
    """Measure VACUUM impact on database of varying sizes.

    This test analyzes the performance curve of VACUUM operations
    across different database sizes to understand scaling behavior.

    Sizes tested: 100, 500, 1000, 5000, 10000 tasks

    For each size:
    1. Create tasks
    2. Delete half
    3. Measure VACUUM time
    4. Record size vs duration relationship

    Logs complete performance curve for analysis.
    """
    sizes = [100, 500, 1_000, 5_000, 10_000]
    results = []

    print("\n[Incremental Impact Benchmark] Testing VACUUM across multiple sizes...")

    for size in sizes:
        print(f"\n[Size: {size}] Creating {size} tasks...")
        create_start = time.perf_counter()

        # Create tasks
        task_ids = await _create_tasks(file_db, count=size, age_days=60)
        create_elapsed = time.perf_counter() - create_start

        # Get size before
        db_path = file_db.db_path
        size_before = await _get_db_size(db_path)

        # Delete half with VACUUM
        delete_count = size // 2
        print(f"[Size: {size}] Deleting {delete_count} tasks with VACUUM...")
        vacuum_start = time.perf_counter()

        filters = PruneFilters(
            older_than_days=30,
            vacuum_mode="always",
            limit=delete_count
        )
        result = await file_db.prune_tasks(filters)

        vacuum_elapsed = time.perf_counter() - vacuum_start

        # Get size after
        size_after = await _get_db_size(db_path)
        actual_reclaimed = size_before - size_after

        # Record metrics
        size_result = {
            "size": size,
            "deleted": delete_count,
            "create_duration_seconds": round(create_elapsed, 3),
            "vacuum_duration_seconds": round(vacuum_elapsed, 3),
            "size_before_bytes": size_before,
            "size_after_bytes": size_after,
            "reclaimed_bytes": result.reclaimed_bytes,
            "actual_reclaimed_bytes": actual_reclaimed,
            "reclaim_percentage": round((actual_reclaimed / size_before * 100) if size_before > 0 else 0, 2)
        }
        results.append(size_result)

        print(f"[Size: {size}] VACUUM completed in {vacuum_elapsed:.3f}s, reclaimed {actual_reclaimed:,} bytes")

        # Clean up: delete remaining tasks for next iteration
        cleanup_filters = PruneFilters(
            older_than_days=1,  # Delete all old tasks
            vacuum_mode="never"  # Don't vacuum during cleanup
        )
        await file_db.prune_tasks(cleanup_filters)

    # Log complete performance curve
    print(f"\n[Incremental Impact - Performance Curve]\n{json.dumps(results, indent=2)}")

    # Analyze scaling behavior
    print("\n[Scaling Analysis]")
    for i in range(1, len(results)):
        prev = results[i - 1]
        curr = results[i]

        size_ratio = curr["size"] / prev["size"]
        time_ratio = curr["vacuum_duration_seconds"] / prev["vacuum_duration_seconds"] if prev["vacuum_duration_seconds"] > 0 else 0

        print(f"  {prev['size']} → {curr['size']} tasks:")
        print(f"    Size increase: {size_ratio:.2f}x")
        print(f"    Time increase: {time_ratio:.2f}x")
        print(f"    Scaling efficiency: {(size_ratio / time_ratio if time_ratio > 0 else 0):.2f}")

    # Assert: All tests completed successfully
    assert len(results) == len(sizes)
    assert all(r["reclaimed_bytes"] is not None for r in results)


@pytest.mark.benchmark
@pytest.mark.asyncio
async def test_vacuum_conditional_vs_always_performance(file_db: Database) -> None:
    """Compare performance of conditional vs always VACUUM modes.

    This test benchmarks the performance difference between:
    - vacuum_mode='conditional' (only runs if >= 100 tasks deleted)
    - vacuum_mode='always' (always runs VACUUM)

    Uses 1000 tasks with 500 deletions to test both modes.
    """
    print("\n[Conditional vs Always Benchmark] Comparing VACUUM modes...")

    # Test 1: Conditional mode with 500 deletions (above threshold)
    print("\n[Test 1] vacuum_mode='conditional' with 500 deletions...")
    await _create_tasks(file_db, count=1_000, age_days=60)

    conditional_start = time.perf_counter()
    conditional_result = await file_db.prune_tasks(
        PruneFilters(older_than_days=30, vacuum_mode="conditional", limit=500)
    )
    conditional_elapsed = time.perf_counter() - conditional_start

    # Clean up
    await file_db.prune_tasks(
        PruneFilters(older_than_days=1, vacuum_mode="never")
    )

    # Test 2: Always mode with 500 deletions
    print("\n[Test 2] vacuum_mode='always' with 500 deletions...")
    await _create_tasks(file_db, count=1_000, age_days=60)

    always_start = time.perf_counter()
    always_result = await file_db.prune_tasks(
        PruneFilters(older_than_days=30, vacuum_mode="always", limit=500)
    )
    always_elapsed = time.perf_counter() - always_start

    # Test 3: Never mode with 500 deletions (no VACUUM)
    await file_db.prune_tasks(
        PruneFilters(older_than_days=1, vacuum_mode="never")
    )
    await _create_tasks(file_db, count=1_000, age_days=60)

    never_start = time.perf_counter()
    never_result = await file_db.prune_tasks(
        PruneFilters(older_than_days=30, vacuum_mode="never", limit=500)
    )
    never_elapsed = time.perf_counter() - never_start

    # Compare results
    comparison = {
        "conditional_mode": {
            "duration_seconds": round(conditional_elapsed, 3),
            "reclaimed_bytes": conditional_result.reclaimed_bytes,
            "vacuum_ran": conditional_result.reclaimed_bytes is not None
        },
        "always_mode": {
            "duration_seconds": round(always_elapsed, 3),
            "reclaimed_bytes": always_result.reclaimed_bytes,
            "vacuum_ran": always_result.reclaimed_bytes is not None
        },
        "never_mode": {
            "duration_seconds": round(never_elapsed, 3),
            "reclaimed_bytes": never_result.reclaimed_bytes,
            "vacuum_ran": never_result.reclaimed_bytes is not None
        },
        "analysis": {
            "conditional_vs_always_ratio": round(conditional_elapsed / always_elapsed if always_elapsed > 0 else 0, 2),
            "never_vs_always_speedup": round(always_elapsed / never_elapsed if never_elapsed > 0 else 0, 2),
            "vacuum_overhead_seconds": round(always_elapsed - never_elapsed, 3)
        }
    }

    print(f"\n[Mode Comparison Results]\n{json.dumps(comparison, indent=2)}")

    # Assert: Conditional mode should have run VACUUM (above threshold)
    assert conditional_result.reclaimed_bytes is not None

    # Assert: Always mode should have run VACUUM
    assert always_result.reclaimed_bytes is not None

    # Assert: Never mode should NOT have run VACUUM
    assert never_result.reclaimed_bytes is None

    # Assert: Never mode should be fastest (no VACUUM overhead)
    assert never_elapsed <= always_elapsed
