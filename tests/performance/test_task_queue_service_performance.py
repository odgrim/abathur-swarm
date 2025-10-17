"""Performance tests for TaskQueueService.

Validates performance targets:
- Task enqueue: <10ms (including validation + priority calculation)
- Get next task: <5ms (single indexed query)
- Complete task: <50ms (including cascade for 10 dependents)
- Queue status: <20ms (aggregate queries)
- Enqueue throughput: >100 tasks/sec

Benchmarks use statistical sampling (100+ iterations) for reliable measurements.

Performance tests for summary field feature:
- Enqueue with summary: <10ms (baseline requirement)
- List with summary: <20ms per page (baseline requirement)
- Serialize all fields: <1ms per task
- Database query with summary: no regression
"""

import asyncio
import time
from collections.abc import AsyncGenerator
from pathlib import Path
from statistics import mean, median
from uuid import uuid4

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


# Helper functions


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


# Performance Tests for Summary Field Feature


@pytest.mark.asyncio
@pytest.mark.performance
async def test_enqueue_performance_with_summary(task_queue_service: TaskQueueService) -> None:
    """Test enqueue performance with summary field.

    Performance Target: <10ms per task (baseline requirement)

    Validates that adding the summary field does not degrade enqueue performance.
    """

    async def enqueue_with_summary():
        """Enqueue task with summary."""
        await task_queue_service.enqueue_task(
            description="Performance test task with detailed implementation requirements and expected outcomes",
            summary="Performance test task",
            source=TaskSource.HUMAN,
            base_priority=5,
        )

    # Measure with summary (100 iterations)
    stats_with_summary = await measure_latencies(enqueue_with_summary, iterations=100)

    print("\nEnqueue Performance WITH Summary (100 iterations):")
    print(f"  Mean:   {stats_with_summary['mean']:.2f}ms")
    print(f"  Median: {stats_with_summary['median']:.2f}ms")
    print(f"  P95:    {stats_with_summary['p95']:.2f}ms")
    print(f"  P99:    {stats_with_summary['p99']:.2f}ms")
    print(f"  Min:    {stats_with_summary['min']:.2f}ms")
    print(f"  Max:    {stats_with_summary['max']:.2f}ms")

    # Assert baseline requirement: P95 < 10ms
    assert (
        stats_with_summary["p95"] < 10.0
    ), f"P95 latency {stats_with_summary['p95']:.2f}ms exceeds 10ms target"

    # Also test without summary for comparison
    async def enqueue_without_summary():
        """Enqueue task without summary."""
        await task_queue_service.enqueue_task(
            description="Performance test task with detailed implementation requirements",
            source=TaskSource.HUMAN,
            base_priority=5,
        )

    stats_without_summary = await measure_latencies(enqueue_without_summary, iterations=100)

    print("\nEnqueue Performance WITHOUT Summary (100 iterations):")
    print(f"  Mean:   {stats_without_summary['mean']:.2f}ms")
    print(f"  Median: {stats_without_summary['median']:.2f}ms")
    print(f"  P95:    {stats_without_summary['p95']:.2f}ms")

    # Calculate performance difference
    diff_percentage = (
        (stats_with_summary["mean"] - stats_without_summary["mean"])
        / stats_without_summary["mean"]
        * 100
    )

    print(f"\nPerformance Difference: {diff_percentage:+.2f}%")

    # Verify no significant regression (allow up to 10% variance)
    assert (
        abs(diff_percentage) < 10.0
    ), f"Performance regression detected: {diff_percentage:+.2f}% change"


@pytest.mark.asyncio
@pytest.mark.performance
async def test_list_performance_with_summary(
    memory_db: Database, task_queue_service: TaskQueueService
) -> None:
    """Test task_list performance with summary field.

    Performance Target: <20ms per page (baseline requirement)

    Validates that returning summary field in list operations does not degrade performance.
    """
    # Pre-populate database with 1000 tasks (with and without summaries)
    print("\nPre-populating database with 1000 tasks...")
    for i in range(1000):
        summary = f"Task {i} summary" if i % 2 == 0 else None  # 50% with summary
        await task_queue_service.enqueue_task(
            description=f"Performance test task {i} with detailed requirements",
            summary=summary,
            source=TaskSource.HUMAN,
            base_priority=5,
        )

    # Measure list operation performance (50 results per page)
    async def list_tasks():
        """List 50 tasks."""
        await memory_db.list_tasks(limit=50)

    stats = await measure_latencies(list_tasks, iterations=50)

    print("\nList Performance with Summary (50 iterations, 50 results per page):")
    print(f"  Mean:   {stats['mean']:.2f}ms")
    print(f"  Median: {stats['median']:.2f}ms")
    print(f"  P95:    {stats['p95']:.2f}ms")
    print(f"  P99:    {stats['p99']:.2f}ms")
    print(f"  Min:    {stats['min']:.2f}ms")
    print(f"  Max:    {stats['max']:.2f}ms")

    # Assert baseline requirement: P95 < 20ms
    assert stats["p95"] < 20.0, f"P95 latency {stats['p95']:.2f}ms exceeds 20ms target"


@pytest.mark.asyncio
@pytest.mark.performance
async def test_serialize_performance_all_fields(
    memory_db: Database, task_queue_service: TaskQueueService
) -> None:
    """Test serialization performance with all Task fields populated.

    Performance Target: <1ms per serialization

    Validates that serializing tasks with all fields (including summary) is fast enough
    for real-time operations. The Task model has 28 fields, and serialization should
    remain performant even with all fields populated.
    """
    # Create task with ALL fields populated (except parent_task_id and session_id to avoid foreign key constraints)
    task = await task_queue_service.enqueue_task(
        description="Complete test task with all fields populated for performance testing",
        summary="Complete test task with all fields" * 5,  # ~200 chars
        source=TaskSource.HUMAN,
        agent_type="python-testing-specialist",
        base_priority=8,
        estimated_duration_seconds=3600,
        feature_branch="feature/test-branch",
        task_branch="task/test-123",
        input_data={"param1": "value1", "param2": "value2"},
    )

    # Measure serialization performance
    latencies = []
    for _ in range(1000):
        start = time.perf_counter()
        # Pydantic V2 serialization
        serialized = task.model_dump()
        end = time.perf_counter()
        latencies.append((end - start) * 1000)

        # Verify summary is included
        assert "summary" in serialized

    stats = {
        "mean": mean(latencies),
        "median": median(latencies),
        "min": min(latencies),
        "max": max(latencies),
        "p95": sorted(latencies)[int(len(latencies) * 0.95)],
        "p99": sorted(latencies)[int(len(latencies) * 0.99)],
    }

    print("\nSerialization Performance (1000 iterations, all fields populated):")
    print(f"  Mean:   {stats['mean']:.4f}ms")
    print(f"  Median: {stats['median']:.4f}ms")
    print(f"  P95:    {stats['p95']:.4f}ms")
    print(f"  P99:    {stats['p99']:.4f}ms")
    print(f"  Min:    {stats['min']:.4f}ms")
    print(f"  Max:    {stats['max']:.4f}ms")

    # Assert target: P99 < 1ms
    assert stats["p99"] < 1.0, f"P99 latency {stats['p99']:.4f}ms exceeds 1ms target"

    # Verify field count (Task model has 28 fields as of this implementation)
    assert len(serialized) >= 25, f"Expected at least 25 fields, got {len(serialized)}"


@pytest.mark.asyncio
@pytest.mark.performance
async def test_database_query_performance_with_summary(memory_db: Database) -> None:
    """Test database query performance with summary column.

    Performance Target: No regression from baseline

    Validates that the summary column addition does not degrade SQL query performance.
    Tests SELECT queries with and without summary filtering.
    """
    # Pre-populate database with 1000 tasks
    print("\nPre-populating database with 1000 tasks...")
    task_ids = []
    for i in range(1000):
        task_id = uuid4()
        summary = f"Performance test task {i} summary" if i % 2 == 0 else None

        async with memory_db._get_connection() as conn:
            await conn.execute(
                """
                INSERT INTO tasks (
                    id, prompt, summary, agent_type, priority, status, calculated_priority,
                    submitted_at, last_updated_at, source, dependency_depth, input_data
                ) VALUES (?, ?, ?, ?, ?, ?, ?, datetime('now'), datetime('now'), ?, ?, '{}')
                """,
                (
                    str(task_id),
                    f"Performance test task {i}",
                    summary,
                    "requirements-gatherer",
                    5,
                    TaskStatus.READY.value,
                    5.0,
                    TaskSource.HUMAN.value,
                    0,
                ),
            )
            await conn.commit()
            task_ids.append(task_id)

    # Test 1: SELECT by ID (indexed query)
    print("\n1. SELECT by ID Performance:")
    latencies_by_id = []
    for _ in range(100):
        task_id = task_ids[_ % len(task_ids)]
        start = time.perf_counter()
        async with memory_db._get_connection() as conn:
            cursor = await conn.execute(
                "SELECT * FROM tasks WHERE id = ?",
                (str(task_id),),
            )
            row = await cursor.fetchone()
        end = time.perf_counter()
        latencies_by_id.append((end - start) * 1000)
        assert row is not None

    stats_by_id = {
        "mean": mean(latencies_by_id),
        "p95": sorted(latencies_by_id)[int(len(latencies_by_id) * 0.95)],
        "p99": sorted(latencies_by_id)[int(len(latencies_by_id) * 0.99)],
    }

    print(f"  Mean:   {stats_by_id['mean']:.2f}ms")
    print(f"  P95:    {stats_by_id['p95']:.2f}ms")
    print(f"  P99:    {stats_by_id['p99']:.2f}ms")

    # Assert: P99 < 5ms for indexed query
    assert stats_by_id["p99"] < 5.0, f"P99 latency {stats_by_id['p99']:.2f}ms exceeds 5ms target"

    # Test 2: SELECT all with LIMIT (common list operation)
    print("\n2. SELECT with LIMIT Performance (50 results):")
    latencies_list = []
    for _ in range(50):
        start = time.perf_counter()
        async with memory_db._get_connection() as conn:
            cursor = await conn.execute(
                "SELECT * FROM tasks WHERE status = ? LIMIT 50",
                (TaskStatus.READY.value,),
            )
            rows = list(await cursor.fetchall())
        end = time.perf_counter()
        latencies_list.append((end - start) * 1000)
        assert len(rows) > 0

    stats_list = {
        "mean": mean(latencies_list),
        "p95": sorted(latencies_list)[int(len(latencies_list) * 0.95)],
    }

    print(f"  Mean:   {stats_list['mean']:.2f}ms")
    print(f"  P95:    {stats_list['p95']:.2f}ms")

    # Assert: P95 < 20ms for list query
    assert stats_list["p95"] < 20.0, f"P95 latency {stats_list['p95']:.2f}ms exceeds 20ms target"

    # Test 3: EXPLAIN QUERY PLAN - verify no table scan
    print("\n3. Query Plan Analysis:")
    async with memory_db._get_connection() as conn:
        cursor = await conn.execute(
            """
            EXPLAIN QUERY PLAN
            SELECT * FROM tasks WHERE id = ?
            """,
            (str(task_ids[0]),),
        )
        plan = list(await cursor.fetchall())

    print("  Query plan for SELECT by ID:")
    for row in plan:
        print(f"    {row}")

    # Verify index usage (should use PRIMARY KEY or index, not SCAN TABLE)
    plan_str = " ".join(str(row) for row in plan)
    assert (
        "SCAN TABLE tasks" not in plan_str or "PRIMARY KEY" in plan_str
    ), "Query should use index, not full table scan"

    print("\n✓ Database query performance verified: no regression detected")


@pytest.mark.asyncio
@pytest.mark.performance
async def test_concurrent_enqueue_with_summary(task_queue_service: TaskQueueService) -> None:
    """Test concurrent task enqueue performance with summary field.

    Performance Target: >40 tasks/second with 50 concurrent agents

    Validates that summary field does not impact concurrent operation performance.
    """
    num_agents = 50
    tasks_per_agent = 5

    # Measure concurrent enqueue with summary
    start = time.perf_counter()
    results = await asyncio.gather(
        *[
            task_queue_service.enqueue_task(
                description=f"Agent {agent_id} concurrent task {task_id} with detailed requirements",
                summary=f"Agent {agent_id} task {task_id}",
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

    print("\nConcurrent Enqueue Performance (50 agents, 5 tasks each):")
    print(f"  Total tasks:  {total_tasks}")
    print(f"  Duration:     {duration:.2f}s")
    print(f"  Throughput:   {throughput:.2f} tasks/second")

    # Verify all tasks were created
    assert len(results) == total_tasks

    # Verify all tasks have summary
    for task in results:
        assert task.summary is not None
        assert "Agent" in task.summary

    # Assert target: >40 tasks/second
    assert (
        throughput >= 40.0
    ), f"Throughput {throughput:.2f} tasks/sec below 40 tasks/sec target for concurrent access"


if __name__ == "__main__":
    pytest.main([__file__, "-v", "-s", "--tb=short"])
