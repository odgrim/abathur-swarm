"""Performance tests for PriorityCalculator service.

Tests performance targets:
- Single priority calculation: <5ms
- Batch calculation (100 tasks): <50ms
- 10-level cascade recalculation: <100ms
"""

import json
import time
from datetime import datetime, timedelta, timezone
from pathlib import Path

import pytest
from abathur.domain.models import (
    DependencyType,
    Task,
    TaskDependency,
    TaskSource,
    TaskStatus,
)
from abathur.infrastructure.database import Database
from abathur.services.dependency_resolver import DependencyResolver
from abathur.services.priority_calculator import PriorityCalculator


@pytest.fixture
async def db():
    """In-memory database for testing."""
    database = Database(db_path=Path(":memory:"))
    await database.initialize()
    yield database
    await database.close()


@pytest.fixture
async def resolver(db):
    """DependencyResolver instance."""
    return DependencyResolver(db, cache_ttl_seconds=60.0)


@pytest.fixture
async def calculator(resolver):
    """PriorityCalculator instance."""
    return PriorityCalculator(resolver)


def create_complex_task(
    priority: int = 5,
    has_deadline: bool = True,
    has_duration: bool = True,
    source: TaskSource = TaskSource.HUMAN,
) -> Task:
    """Create a task with all factors for performance testing."""
    deadline = None
    if has_deadline:
        deadline = datetime.now(timezone.utc) + timedelta(hours=2)

    estimated_duration = None
    if has_duration:
        estimated_duration = 3600  # 1 hour

    return Task(
        prompt="Performance test task",
        priority=priority,
        status=TaskStatus.READY,
        source=source,
        deadline=deadline,
        estimated_duration_seconds=estimated_duration,
    )


async def create_dependency_chain(db: Database, length: int) -> list[Task]:
    """Create linear dependency chain of specified length."""
    tasks = []
    for i in range(length):
        task = create_complex_task()
        await db.insert_task(task)
        tasks.append(task)

        if i > 0:
            dep = TaskDependency(
                dependent_task_id=task.id,
                prerequisite_task_id=tasks[i - 1].id,
                dependency_type=DependencyType.SEQUENTIAL,
            )
            await db.insert_task_dependency(dep)

    return tasks


# =============================================================================
# Performance Test 1: Single Calculation
# =============================================================================


@pytest.mark.asyncio
async def test_single_priority_calculation_performance(db, calculator):
    """Test single priority calculation meets <5ms target.

    Creates a task with all factors (depth, urgency, blocking, etc.) and
    measures calculation time over 100 iterations.
    """
    # Create dependency chain for depth
    tasks = await create_dependency_chain(db, 5)
    test_task = tasks[-1]  # Task at depth 5

    # Create some blocked tasks
    for _ in range(5):
        blocked = create_complex_task()
        await db.insert_task(blocked)
        dep = TaskDependency(
            dependent_task_id=blocked.id,
            prerequisite_task_id=test_task.id,
            dependency_type=DependencyType.SEQUENTIAL,
        )
        await db.insert_task_dependency(dep)

    # Warm up cache
    await calculator.calculate_priority(test_task)

    # Measure average time over 100 iterations
    iterations = 100
    start = time.perf_counter()

    for _ in range(iterations):
        await calculator.calculate_priority(test_task)

    elapsed = time.perf_counter() - start
    avg_time_ms = (elapsed / iterations) * 1000

    print(f"\nSingle priority calculation: {avg_time_ms:.3f}ms average (target <5ms)")

    # Assert performance target
    assert avg_time_ms < 5.0, f"Single calculation took {avg_time_ms:.3f}ms, target <5ms (FAILED)"

    # Save benchmark result
    benchmark = {
        "test_name": "single_priority_calculation",
        "iterations": iterations,
        "total_time_ms": elapsed * 1000,
        "avg_time_ms": avg_time_ms,
        "target_ms": 5.0,
        "status": "PASS" if avg_time_ms < 5.0 else "FAIL",
        "timestamp": datetime.now(timezone.utc).isoformat(),
    }

    # Write to file
    output_path = (
        Path(__file__).parent.parent.parent / "design_docs" / "PHASE3_PERFORMANCE_BENCHMARKS.json"
    )
    benchmarks = []
    if output_path.exists():
        with open(output_path) as f:
            data = json.load(f)
            benchmarks = data.get("benchmarks", [])

    benchmarks.append(benchmark)
    output_path.parent.mkdir(parents=True, exist_ok=True)
    with open(output_path, "w") as f:
        json.dump({"benchmarks": benchmarks}, f, indent=2)


# =============================================================================
# Performance Test 2: Batch Calculation (100 tasks)
# =============================================================================


@pytest.mark.asyncio
async def test_batch_priority_calculation_100_tasks(db, calculator):
    """Test batch calculation of 100 tasks meets <50ms target.

    Creates 100 tasks with mixed properties and measures batch recalculation time.
    """
    # Create 100 tasks with varying properties
    task_ids = []
    for i in range(100):
        task = create_complex_task(
            priority=i % 10,
            has_deadline=(i % 3 == 0),
            has_duration=(i % 2 == 0),
            source=TaskSource.HUMAN if i % 4 == 0 else TaskSource.AGENT_PLANNER,
        )
        await db.insert_task(task)
        task_ids.append(task.id)

    # Warm up cache
    await calculator.recalculate_priorities(task_ids[:10], db)

    # Measure batch recalculation time
    start = time.perf_counter()
    results = await calculator.recalculate_priorities(task_ids, db)
    elapsed_ms = (time.perf_counter() - start) * 1000

    print(f"\nBatch calculation (100 tasks): {elapsed_ms:.2f}ms (target <50ms)")

    # Assert performance target
    assert elapsed_ms < 50.0, f"Batch calculation took {elapsed_ms:.2f}ms, target <50ms (FAILED)"

    # Verify all tasks calculated
    assert len(results) == 100, f"Expected 100 results, got {len(results)}"

    # Save benchmark result
    benchmark = {
        "test_name": "batch_priority_calculation_100_tasks",
        "task_count": 100,
        "elapsed_ms": elapsed_ms,
        "target_ms": 50.0,
        "avg_per_task_ms": elapsed_ms / 100,
        "status": "PASS" if elapsed_ms < 50.0 else "FAIL",
        "timestamp": datetime.now(timezone.utc).isoformat(),
    }

    # Append to file
    output_path = (
        Path(__file__).parent.parent.parent / "design_docs" / "PHASE3_PERFORMANCE_BENCHMARKS.json"
    )
    if output_path.exists():
        with open(output_path) as f:
            data = json.load(f)
            benchmarks = data.get("benchmarks", [])
    else:
        benchmarks = []

    benchmarks.append(benchmark)
    with open(output_path, "w") as f:
        json.dump({"benchmarks": benchmarks}, f, indent=2)


# =============================================================================
# Performance Test 3: 10-Level Cascade Recalculation
# =============================================================================


@pytest.mark.asyncio
async def test_priority_recalculation_cascade_10_levels(db, resolver, calculator):
    """Test cascading priority updates after task completion.

    Creates a 10-level dependency chain with 5 tasks at each level (50 total).
    Simulates completing root task and measures time to recalculate all affected tasks.
    """
    # Create 10-level dependency tree
    # Each level has 5 tasks, total 50 tasks
    levels = []
    for level in range(10):
        level_tasks = []
        for i in range(5):
            task = create_complex_task(priority=5 + level)
            await db.insert_task(task)
            level_tasks.append(task)

            # Connect to previous level
            if level > 0:
                # Each task depends on one task from previous level
                prereq = levels[level - 1][i % len(levels[level - 1])]
                dep = TaskDependency(
                    dependent_task_id=task.id,
                    prerequisite_task_id=prereq.id,
                    dependency_type=DependencyType.SEQUENTIAL,
                )
                await db.insert_task_dependency(dep)

        levels.append(level_tasks)

    # Get all task IDs
    all_task_ids = [task.id for level in levels for task in level]

    # Warm up cache
    await calculator.recalculate_priorities(all_task_ids[:10], db)

    # Simulate completing root task and measure cascade recalculation
    start = time.perf_counter()

    # Complete root task (invalidates cache)
    root_task = levels[0][0]
    await db.update_task_status(root_task.id, TaskStatus.COMPLETED)
    await db.resolve_dependency(root_task.id)

    # Invalidate cache to simulate state change
    resolver.invalidate_cache()

    # Recalculate priorities for all affected tasks
    results = await calculator.recalculate_priorities(all_task_ids, db)

    elapsed_ms = (time.perf_counter() - start) * 1000

    print(f"\n10-level cascade recalculation (50 tasks): {elapsed_ms:.2f}ms (target <100ms)")

    # Assert performance target
    assert elapsed_ms < 100.0, f"Cascade took {elapsed_ms:.2f}ms, target <100ms (FAILED)"

    # Verify tasks recalculated
    assert len(results) > 0, "Should have recalculated some tasks"

    # Save benchmark result
    benchmark = {
        "test_name": "priority_recalculation_cascade_10_levels",
        "levels": 10,
        "tasks_per_level": 5,
        "total_tasks": 50,
        "elapsed_ms": elapsed_ms,
        "target_ms": 100.0,
        "tasks_recalculated": len(results),
        "status": "PASS" if elapsed_ms < 100.0 else "FAIL",
        "timestamp": datetime.now(timezone.utc).isoformat(),
    }

    # Append to file
    output_path = (
        Path(__file__).parent.parent.parent / "design_docs" / "PHASE3_PERFORMANCE_BENCHMARKS.json"
    )
    if output_path.exists():
        with open(output_path) as f:
            data = json.load(f)
            benchmarks = data.get("benchmarks", [])
    else:
        benchmarks = []

    benchmarks.append(benchmark)
    with open(output_path, "w") as f:
        json.dump(
            {
                "benchmarks": benchmarks,
                "summary": {
                    "total_tests": len(benchmarks),
                    "passed": sum(1 for b in benchmarks if b["status"] == "PASS"),
                    "failed": sum(1 for b in benchmarks if b["status"] == "FAIL"),
                    "timestamp": datetime.now(timezone.utc).isoformat(),
                },
            },
            f,
            indent=2,
        )


# =============================================================================
# Additional Performance Analysis Tests
# =============================================================================


@pytest.mark.asyncio
async def test_depth_calculation_cache_performance(db, resolver, calculator):
    """Test that depth calculation benefits from caching."""
    # Create deep dependency chain
    tasks = await create_dependency_chain(db, 10)
    deep_task = tasks[-1]

    # First calculation (cold cache)
    start = time.perf_counter()
    priority1 = await calculator.calculate_priority(deep_task)
    first_time_ms = (time.perf_counter() - start) * 1000

    # Second calculation (warm cache)
    start = time.perf_counter()
    priority2 = await calculator.calculate_priority(deep_task)
    second_time_ms = (time.perf_counter() - start) * 1000

    print(f"\nDepth cache performance: cold={first_time_ms:.3f}ms, warm={second_time_ms:.3f}ms")

    # Cached should be faster
    assert second_time_ms < first_time_ms, "Cached calculation should be faster than first"
    assert priority1 == priority2, "Priority should be consistent"


@pytest.mark.asyncio
async def test_blocking_score_performance(db, calculator):
    """Test blocking score calculation with many blocked tasks."""
    prerequisite = create_complex_task()
    await db.insert_task(prerequisite)

    # Create 50 blocked tasks
    for _ in range(50):
        blocked = create_complex_task()
        await db.insert_task(blocked)
        dep = TaskDependency(
            dependent_task_id=blocked.id,
            prerequisite_task_id=prerequisite.id,
            dependency_type=DependencyType.SEQUENTIAL,
        )
        await db.insert_task_dependency(dep)

    # Measure blocking score calculation time
    start = time.perf_counter()
    priority = await calculator.calculate_priority(prerequisite)
    elapsed_ms = (time.perf_counter() - start) * 1000

    print(f"\nBlocking score (50 tasks): {elapsed_ms:.3f}ms")

    # Should still be fast (< 5ms target)
    assert elapsed_ms < 5.0, f"Blocking score took {elapsed_ms:.3f}ms, expected <5ms"
    assert priority > 40, "Task blocking 50 tasks should have high priority"
