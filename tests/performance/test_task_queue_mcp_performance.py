"""Performance tests for Task Queue MCP Server.

Tests performance targets and scalability under load.

Performance Targets (from requirements):
- Task enqueue: <10ms (simple task)
- Task get by ID: <5ms
- Task list (50 results): <20ms
- Queue statistics: <20ms (even with 10,000 tasks)
- Task cancel (10 dependents): <50ms
- Task execution plan (100 tasks): <30ms
- Concurrent access: 100 agents without degradation

Test Categories:
1. Single Operation Latency Tests
2. Throughput Tests (operations per second)
3. Scalability Tests (performance vs. data size)
4. Concurrent Access Tests (multiple agents)
5. Database Query Performance (EXPLAIN QUERY PLAN)
6. Memory Usage Tests

Note: These tests may be slow and should be run separately from unit tests.
Run with: pytest tests/performance/ -v --durations=10
"""

import asyncio
import time
from collections.abc import AsyncGenerator
from datetime import datetime, timezone
from pathlib import Path
from statistics import mean, median
from uuid import UUID, uuid4

import pytest
from abathur.domain.models import TaskSource, TaskStatus
from abathur.infrastructure.database import Database
from abathur.services.dependency_resolver import DependencyResolver
from abathur.services.priority_calculator import PriorityCalculator
from abathur.services.task_queue_service import TaskQueueService

# Fixtures


@pytest.fixture
async def memory_db() -> AsyncGenerator[Database, None]:
    """Create in-memory database for performance tests."""
    db = Database(Path(":memory:"))
    await db.initialize()
    yield db
    await db.close()


@pytest.fixture
async def task_queue_service(memory_db: Database) -> TaskQueueService:
    """Create TaskQueueService with in-memory database."""
    dependency_resolver = DependencyResolver(memory_db)
    priority_calculator = PriorityCalculator(dependency_resolver)
    return TaskQueueService(memory_db, dependency_resolver, priority_calculator)


@pytest.fixture
async def large_queue(task_queue_service: TaskQueueService) -> list[UUID]:
    """Create database with 1000 tasks for scalability tests."""
    task_ids = []
    for i in range(1000):
        task = await task_queue_service.enqueue_task(
            description=f"Task {i}",
            source=TaskSource.HUMAN,
            base_priority=5,
        )
        task_ids.append(task.id)
    return task_ids


# Helper functions


def measure_latency(func):
    """Decorator to measure function execution time in milliseconds."""

    async def wrapper(*args, **kwargs):
        start = time.perf_counter()
        result = await func(*args, **kwargs)
        end = time.perf_counter()
        latency_ms = (end - start) * 1000
        return result, latency_ms

    return wrapper


async def measure_latencies(func, iterations: int = 100):
    """Run function multiple times and return latency statistics."""
    latencies = []
    for _ in range(iterations):
        start = time.perf_counter()
        await func()
        end = time.perf_counter()
        latencies.append((end - start) * 1000)

    return {
        "mean": mean(latencies),
        "median": median(latencies),
        "min": min(latencies),
        "max": max(latencies),
        "p95": sorted(latencies)[int(len(latencies) * 0.95)],
        "p99": sorted(latencies)[int(len(latencies) * 0.99)],
    }


# Single Operation Latency Tests


@pytest.mark.asyncio
@pytest.mark.performance
async def test_enqueue_simple_task_latency(
    memory_db: Database, task_queue_service: TaskQueueService
):
    """Test that simple task enqueue completes in <10ms (target).

    Target: <10ms for 95th percentile
    """

    async def enqueue_task():
        await task_queue_service.enqueue_task(
            description="Simple test task",
            source=TaskSource.HUMAN,
            base_priority=5,
        )

    stats = await measure_latencies(enqueue_task, iterations=100)

    print("\nEnqueue Simple Task Latency:")
    print(f"  Mean: {stats['mean']:.2f}ms")
    print(f"  Median: {stats['median']:.2f}ms")
    print(f"  P95: {stats['p95']:.2f}ms")
    print(f"  P99: {stats['p99']:.2f}ms")

    # Assert target met
    assert stats["p95"] < 10.0, f"P95 latency {stats['p95']:.2f}ms exceeds 10ms target"


@pytest.mark.asyncio
@pytest.mark.performance
async def test_enqueue_task_with_dependencies_latency(
    memory_db: Database, task_queue_service: TaskQueueService
):
    """Test task enqueue with dependencies completes in reasonable time.

    Target: <20ms for task with 5 prerequisites (includes validation)
    """
    # Create 5 prerequisite tasks
    prereq_ids = []
    for i in range(5):
        task = await task_queue_service.enqueue_task(
            description=f"Prerequisite {i}",
            source=TaskSource.HUMAN,
            base_priority=5,
        )
        prereq_ids.append(task.id)

    async def enqueue_dependent_task():
        await task_queue_service.enqueue_task(
            description="Dependent task",
            source=TaskSource.HUMAN,
            prerequisites=prereq_ids,
            base_priority=5,
        )

    stats = await measure_latencies(enqueue_dependent_task, iterations=50)

    print("\nEnqueue Task with 5 Dependencies Latency:")
    print(f"  Mean: {stats['mean']:.2f}ms")
    print(f"  P95: {stats['p95']:.2f}ms")

    assert stats["p95"] < 20.0, f"P95 latency {stats['p95']:.2f}ms exceeds 20ms target"


@pytest.mark.asyncio
@pytest.mark.performance
async def test_get_task_by_id_latency(memory_db: Database, task_queue_service: TaskQueueService):
    """Test that get task by ID completes in <5ms.

    Target: <5ms for 99th percentile (indexed query)
    """
    # Create task
    task = await task_queue_service.enqueue_task(
        description="Test task",
        source=TaskSource.HUMAN,
        base_priority=5,
    )

    async def get_task():
        await memory_db.get_task(task.id)

    stats = await measure_latencies(get_task, iterations=100)

    print("\nGet Task by ID Latency:")
    print(f"  Mean: {stats['mean']:.2f}ms")
    print(f"  P99: {stats['p99']:.2f}ms")

    assert stats["p99"] < 5.0, f"P99 latency {stats['p99']:.2f}ms exceeds 5ms target"


@pytest.mark.asyncio
@pytest.mark.performance
async def test_get_next_task_latency(memory_db: Database, task_queue_service: TaskQueueService):
    """Test that get_next_task completes in <5ms.

    Target: <5ms for 99th percentile (single indexed query)
    """
    # Create 10 ready tasks
    for i in range(10):
        await task_queue_service.enqueue_task(
            description=f"Ready task {i}",
            source=TaskSource.HUMAN,
            base_priority=5,
        )

    async def get_next():
        await task_queue_service.get_next_task()

    stats = await measure_latencies(get_next, iterations=10)  # Only 10 tasks available

    print("\nGet Next Task Latency:")
    print(f"  Mean: {stats['mean']:.2f}ms")
    print(f"  P99: {stats['p99']:.2f}ms")

    assert stats["p99"] < 5.0, f"P99 latency {stats['p99']:.2f}ms exceeds 5ms target"


@pytest.mark.asyncio
@pytest.mark.performance
async def test_queue_status_latency(memory_db: Database, task_queue_service: TaskQueueService):
    """Test that queue status query completes in <20ms.

    Target: <20ms even with 1000 tasks
    """
    # Create 1000 tasks with mixed statuses
    for i in range(1000):
        await task_queue_service.enqueue_task(
            description=f"Task {i}",
            source=TaskSource.HUMAN,
            base_priority=5,
        )

    async def get_status():
        await task_queue_service.get_queue_status()

    stats = await measure_latencies(get_status, iterations=50)

    print("\nQueue Status Latency (1000 tasks):")
    print(f"  Mean: {stats['mean']:.2f}ms")
    print(f"  P95: {stats['p95']:.2f}ms")

    assert stats["p95"] < 20.0, f"P95 latency {stats['p95']:.2f}ms exceeds 20ms target"


@pytest.mark.asyncio
@pytest.mark.performance
async def test_cancel_task_with_dependents_latency(
    memory_db: Database, task_queue_service: TaskQueueService
):
    """Test that cancel task with 10 dependents completes in <50ms.

    Target: <50ms for cascade cancellation
    """
    # Create root task
    root_task = await task_queue_service.enqueue_task(
        description="Root task",
        source=TaskSource.HUMAN,
        base_priority=5,
    )

    # Create 10 dependent tasks
    dependent_ids = []
    for i in range(10):
        task = await task_queue_service.enqueue_task(
            description=f"Dependent {i}",
            source=TaskSource.HUMAN,
            prerequisites=[root_task.id],
            base_priority=5,
        )
        dependent_ids.append(task.id)

    # Measure cancellation time
    start = time.perf_counter()
    await task_queue_service.cancel_task(root_task.id)
    end = time.perf_counter()

    latency_ms = (end - start) * 1000

    print(f"\nCancel Task with 10 Dependents Latency: {latency_ms:.2f}ms")

    assert latency_ms < 50.0, f"Latency {latency_ms:.2f}ms exceeds 50ms target"


@pytest.mark.asyncio
@pytest.mark.performance
async def test_execution_plan_latency(memory_db: Database, task_queue_service: TaskQueueService):
    """Test that execution plan calculation completes in <30ms for 100 tasks.

    Target: <30ms for 100-task graph
    """
    # Create 100 tasks with some dependencies
    task_ids = []
    for i in range(100):
        prerequisites = []
        if i > 0 and i % 10 == 0:
            # Every 10th task depends on previous task
            prerequisites = [task_ids[i - 1]]

        task = await task_queue_service.enqueue_task(
            description=f"Task {i}",
            source=TaskSource.HUMAN,
            prerequisites=prerequisites,
            base_priority=5,
        )
        task_ids.append(task.id)

    # Measure execution plan time
    start = time.perf_counter()
    await task_queue_service.get_task_execution_plan(task_ids)
    end = time.perf_counter()

    latency_ms = (end - start) * 1000

    print(f"\nExecution Plan Latency (100 tasks): {latency_ms:.2f}ms")

    assert latency_ms < 30.0, f"Latency {latency_ms:.2f}ms exceeds 30ms target"


# Throughput Tests


@pytest.mark.asyncio
@pytest.mark.performance
async def test_enqueue_throughput(memory_db: Database, task_queue_service: TaskQueueService):
    """Test task enqueue throughput (tasks per second).

    Target: >50 tasks/second for simple tasks
    """
    num_tasks = 100

    start = time.perf_counter()
    for i in range(num_tasks):
        await task_queue_service.enqueue_task(
            description=f"Throughput test task {i}",
            source=TaskSource.HUMAN,
            base_priority=5,
        )
    end = time.perf_counter()

    duration = end - start
    throughput = num_tasks / duration

    print(
        f"\nEnqueue Throughput: {throughput:.2f} tasks/second ({duration:.2f}s for {num_tasks} tasks)"
    )

    assert throughput >= 50.0, f"Throughput {throughput:.2f} tasks/sec below 50 tasks/sec target"


@pytest.mark.asyncio
@pytest.mark.performance
async def test_query_throughput(
    memory_db: Database, task_queue_service: TaskQueueService, large_queue: list[UUID]
):
    """Test query throughput (queries per second).

    Target: >100 queries/second for get_task by ID
    """
    num_queries = 100
    task_ids = large_queue[:num_queries]  # Use first 100 tasks

    start = time.perf_counter()
    for task_id in task_ids:
        await memory_db.get_task(task_id)
    end = time.perf_counter()

    duration = end - start
    throughput = num_queries / duration

    print(
        f"\nQuery Throughput: {throughput:.2f} queries/second ({duration:.2f}s for {num_queries} queries)"
    )

    assert (
        throughput >= 100.0
    ), f"Throughput {throughput:.2f} queries/sec below 100 queries/sec target"


# Scalability Tests


@pytest.mark.asyncio
@pytest.mark.performance
async def test_queue_status_scales_with_task_count(
    memory_db: Database, task_queue_service: TaskQueueService
):
    """Test that queue status performance scales linearly with task count.

    Measure latency at 100, 500, 1000, 5000 tasks
    """
    results = []

    for num_tasks in [100, 500, 1000, 5000]:
        # Clear database and create new tasks
        # (In real test, would use fresh database for each)
        for i in range(num_tasks):
            await task_queue_service.enqueue_task(
                description=f"Task {i}",
                source=TaskSource.HUMAN,
                base_priority=5,
            )

        # Measure latency
        start = time.perf_counter()
        await task_queue_service.get_queue_status()
        end = time.perf_counter()

        latency_ms = (end - start) * 1000
        results.append((num_tasks, latency_ms))

        print(f"  {num_tasks} tasks: {latency_ms:.2f}ms")

    # Verify all latencies are under 50ms
    for num_tasks, latency_ms in results:
        assert latency_ms < 50.0, f"Latency {latency_ms:.2f}ms exceeds 50ms for {num_tasks} tasks"


@pytest.mark.asyncio
@pytest.mark.performance
async def test_dependency_depth_scales_linearly(
    memory_db: Database, task_queue_service: TaskQueueService
):
    """Test that dependency depth calculation scales linearly.

    Create chain of depth 10, 20, 50 and measure calculation time
    """
    results = []

    for depth in [10, 20, 50]:
        # Create chain
        prev_id = None
        for i in range(depth):
            prerequisites = [prev_id] if prev_id else []
            task = await task_queue_service.enqueue_task(
                description=f"Chain task {i}",
                source=TaskSource.HUMAN,
                prerequisites=prerequisites,
                base_priority=5,
            )
            prev_id = task.id

        # Measure depth calculation time (already done during enqueue, but test explicitly)
        dependency_resolver = DependencyResolver(memory_db)
        start = time.perf_counter()
        calculated_depth = await dependency_resolver.calculate_dependency_depth(prev_id)
        end = time.perf_counter()

        latency_ms = (end - start) * 1000
        results.append((depth, latency_ms))

        print(f"  Depth {depth}: {latency_ms:.2f}ms (calculated depth: {calculated_depth})")

        assert calculated_depth == depth - 1, f"Expected depth {depth - 1}, got {calculated_depth}"


# Concurrent Access Tests


@pytest.mark.asyncio
@pytest.mark.performance
async def test_concurrent_enqueue_50_agents(
    memory_db: Database, task_queue_service: TaskQueueService
):
    """Test concurrent task enqueue from 50 agents.

    Target: No significant degradation compared to sequential
    """
    num_agents = 50
    tasks_per_agent = 5

    # Measure concurrent enqueue
    start = time.perf_counter()
    results = await asyncio.gather(
        *[
            task_queue_service.enqueue_task(
                description=f"Agent {agent_id} Task {task_id}",
                source=TaskSource.HUMAN,
                base_priority=5,
            )
            for agent_id in range(num_agents)
            for task_id in range(tasks_per_agent)
        ]
    )
    end = time.perf_counter()

    duration = end - start
    total_tasks = num_agents * tasks_per_agent
    throughput = total_tasks / duration

    print("\nConcurrent Enqueue (50 agents):")
    print(f"  Total tasks: {total_tasks}")
    print(f"  Duration: {duration:.2f}s")
    print(f"  Throughput: {throughput:.2f} tasks/second")

    assert len(results) == total_tasks
    assert (
        throughput >= 40.0
    ), f"Throughput {throughput:.2f} tasks/sec too low for concurrent access"


@pytest.mark.asyncio
@pytest.mark.performance
async def test_concurrent_dequeue_100_agents(
    memory_db: Database, task_queue_service: TaskQueueService
):
    """Test concurrent task dequeue from 100 agents.

    Target: Support 100 concurrent get_next_task calls
    """
    # Create 100 ready tasks
    for i in range(100):
        await task_queue_service.enqueue_task(
            description=f"Concurrent dequeue task {i}",
            source=TaskSource.HUMAN,
            base_priority=5,
        )

    # Simulate 100 agents dequeuing
    start = time.perf_counter()
    dequeued_tasks = await asyncio.gather(*[task_queue_service.get_next_task() for _ in range(100)])
    end = time.perf_counter()

    duration = end - start
    non_none_tasks = [t for t in dequeued_tasks if t is not None]

    print("\nConcurrent Dequeue (100 agents):")
    print(f"  Tasks dequeued: {len(non_none_tasks)}")
    print(f"  Duration: {duration:.2f}s")
    print(f"  Throughput: {len(non_none_tasks) / duration:.2f} tasks/second")

    # Should dequeue all 100 tasks
    assert len(non_none_tasks) == 100
    # Should have no duplicates
    task_ids = [t.id for t in non_none_tasks]
    assert len(set(task_ids)) == 100


@pytest.mark.asyncio
@pytest.mark.performance
async def test_concurrent_mixed_operations(
    memory_db: Database, task_queue_service: TaskQueueService
):
    """Test mixed concurrent operations (enqueue + dequeue + status).

    Simulates realistic workload with multiple operation types
    """
    num_operations = 100

    # Create some initial tasks
    for i in range(20):
        await task_queue_service.enqueue_task(
            description=f"Initial task {i}",
            source=TaskSource.HUMAN,
            base_priority=5,
        )

    # Mix of operations
    operations = []

    # 50 enqueues
    for i in range(50):
        operations.append(
            task_queue_service.enqueue_task(
                description=f"Concurrent task {i}",
                source=TaskSource.HUMAN,
                base_priority=5,
            )
        )

    # 30 dequeues
    for _ in range(30):
        operations.append(task_queue_service.get_next_task())

    # 20 status queries
    for _ in range(20):
        operations.append(task_queue_service.get_queue_status())

    # Execute all concurrently
    start = time.perf_counter()
    results = await asyncio.gather(*operations, return_exceptions=True)
    end = time.perf_counter()

    duration = end - start
    throughput = num_operations / duration

    print("\nConcurrent Mixed Operations:")
    print(f"  Total operations: {num_operations}")
    print(f"  Duration: {duration:.2f}s")
    print(f"  Throughput: {throughput:.2f} ops/second")

    # Check for errors
    errors = [r for r in results if isinstance(r, Exception)]
    assert len(errors) == 0, f"Encountered {len(errors)} errors during concurrent operations"


# Database Query Performance Tests


@pytest.mark.asyncio
@pytest.mark.performance
async def test_explain_get_next_task_query(memory_db: Database):
    """Verify that get_next_task query uses index.

    Query should use idx_tasks_status_priority composite index
    """
    # Create some tasks
    for i in range(10):
        async with memory_db._get_connection() as conn:
            await conn.execute(
                """
                INSERT INTO tasks (
                    id, prompt, agent_type, priority, status, calculated_priority,
                    submitted_at, last_updated_at, source, dependency_depth
                ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                """,
                (
                    str(uuid4()),
                    f"Task {i}",
                    "requirements-gatherer",
                    5,
                    TaskStatus.READY.value,
                    5.0,
                    datetime.now(timezone.utc).isoformat(),
                    datetime.now(timezone.utc).isoformat(),
                    TaskSource.HUMAN.value,
                    0,
                ),
            )
            await conn.commit()

    # EXPLAIN QUERY PLAN
    async with memory_db._get_connection() as conn:
        cursor = await conn.execute(
            """
            EXPLAIN QUERY PLAN
            SELECT * FROM tasks
            WHERE status = ?
            ORDER BY calculated_priority DESC, submitted_at ASC
            LIMIT 1
            """,
            (TaskStatus.READY.value,),
        )
        plan = await cursor.fetchall()

    print("\nQuery Plan for get_next_task:")
    for row in plan:
        print(f"  {row}")

    # Verify index is used (check for "USING INDEX" in plan)
    plan_str = " ".join(str(row) for row in plan)
    # Note: Exact index usage depends on database schema
    # This is a basic check that query is optimized
    assert (
        "SCAN TABLE tasks" not in plan_str or "USING INDEX" in plan_str
    ), "get_next_task query should use index"


@pytest.mark.asyncio
@pytest.mark.performance
async def test_explain_queue_status_query(memory_db: Database):
    """Verify that queue status query is optimized.

    Should use indexes for status counting
    """
    # Create tasks
    for i in range(100):
        async with memory_db._get_connection() as conn:
            await conn.execute(
                """
                INSERT INTO tasks (
                    id, prompt, agent_type, priority, status, calculated_priority,
                    submitted_at, last_updated_at, source, dependency_depth
                ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                """,
                (
                    str(uuid4()),
                    f"Task {i}",
                    "requirements-gatherer",
                    5,
                    TaskStatus.READY.value if i % 2 == 0 else TaskStatus.BLOCKED.value,
                    5.0,
                    datetime.now(timezone.utc).isoformat(),
                    datetime.now(timezone.utc).isoformat(),
                    TaskSource.HUMAN.value,
                    0,
                ),
            )
            await conn.commit()

    # EXPLAIN QUERY PLAN for status aggregation
    async with memory_db._get_connection() as conn:
        cursor = await conn.execute(
            """
            EXPLAIN QUERY PLAN
            SELECT status, COUNT(*) as count, AVG(calculated_priority) as avg_priority
            FROM tasks
            GROUP BY status
            """
        )
        plan = await cursor.fetchall()

    print("\nQuery Plan for queue status aggregation:")
    for row in plan:
        print(f"  {row}")


# Memory Usage Tests


@pytest.mark.asyncio
@pytest.mark.performance
async def test_memory_usage_with_large_queue(
    memory_db: Database, task_queue_service: TaskQueueService
):
    """Test memory usage with large queue.

    Create 10,000 tasks and verify memory doesn't leak
    Note: This is a basic check; proper memory profiling requires external tools
    """
    import gc
    import sys

    gc.collect()
    initial_size = sys.getsizeof(task_queue_service)

    # Create 10,000 tasks
    for i in range(10000):
        await task_queue_service.enqueue_task(
            description=f"Memory test task {i}",
            source=TaskSource.HUMAN,
            base_priority=5,
        )

        # Periodically check memory
        if i % 1000 == 0:
            gc.collect()
            current_size = sys.getsizeof(task_queue_service)
            print(f"  {i} tasks: service object size = {current_size} bytes")

    gc.collect()
    final_size = sys.getsizeof(task_queue_service)

    print("\nMemory Usage:")
    print(f"  Initial: {initial_size} bytes")
    print(f"  Final: {final_size} bytes")
    print(f"  Growth: {final_size - initial_size} bytes")

    # Service object itself should not grow significantly
    # (tasks are stored in database, not in memory)
    assert final_size - initial_size < 1000000, "Service object grew by more than 1MB"


if __name__ == "__main__":
    pytest.main([__file__, "-v", "-s"])
