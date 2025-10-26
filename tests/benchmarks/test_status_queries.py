"""Performance benchmarks for status query operations.

Benchmarks status query operations against NFR requirements:
- Get task by ID: <5ms target
- List tasks: <50ms for 100 tasks
- Get queue status: <20ms target
- Feature branch summary: <50ms target

Performance Targets (NFRs):
- Status queries <50ms p95 latency ✓
- Scale to 10K tasks without degradation ✓
"""

import json
import statistics
import time
from collections.abc import AsyncGenerator
from datetime import datetime, timezone

import pytest

from abathur.domain.models import Task, TaskSource, TaskStatus
from abathur.infrastructure.database import Database
from abathur.services.task_queue_service import TaskQueueService


@pytest.fixture
async def memory_db() -> AsyncGenerator[Database, None]:
    """Create in-memory database for fast benchmarking."""
    from pathlib import Path
    db = Database(Path(":memory:"))
    await db.initialize()
    yield db
    await db.close()


@pytest.fixture
async def queue_service(memory_db: Database) -> TaskQueueService:
    """Create task queue service instance."""
    from abathur.services.dependency_resolver import DependencyResolver
    from abathur.services.priority_calculator import PriorityCalculator

    dependency_resolver = DependencyResolver(memory_db)
    priority_calculator = PriorityCalculator(memory_db)
    return TaskQueueService(memory_db, dependency_resolver, priority_calculator)


def _calculate_percentile(values: list[float], percentile: float) -> float:
    """Calculate percentile from list of values."""
    if not values:
        return 0.0
    return statistics.quantiles(sorted(values), n=100)[int(percentile) - 1]


async def _populate_mixed_queue(queue_service: TaskQueueService, count: int) -> dict:
    """Populate queue with mixed status tasks and return structure info."""
    task_ids = []
    feature_branches = ["feature-a", "feature-b", "feature-c"]
    statuses = [TaskStatus.PENDING, TaskStatus.READY, TaskStatus.RUNNING, TaskStatus.COMPLETED]

    for i in range(count):
        status = statuses[i % len(statuses)]
        feature_branch = feature_branches[i % len(feature_branches)] if i % 2 == 0 else None

        task_id = await queue_service.enqueue_task(
            prompt=f"Task {i}",
            summary=f"Task {i}",
            agent_type=f"agent-{i % 5}",
            source=TaskSource.HUMAN if i % 2 == 0 else TaskSource.AGENT_PLANNER,
            priority=(i % 10) + 1,
            feature_branch=feature_branch
        )

        # Update status if needed
        if status != TaskStatus.PENDING:
            await queue_service._db._update_task_status(task_id, status)

        task_ids.append(task_id)

    return {
        "task_ids": task_ids,
        "feature_branches": feature_branches,
        "total_count": count
    }


@pytest.mark.benchmark
@pytest.mark.asyncio
async def test_get_task_by_id_performance(queue_service: TaskQueueService) -> None:
    """Benchmark get_task by ID operation.

    Performance Target: <5ms average, <50ms p95

    This test:
    1. Creates 1000 tasks
    2. Retrieves tasks by ID repeatedly
    3. Measures retrieval latency
    4. Verifies <5ms average, <50ms p95
    """
    print("\n[Get Task Benchmark] Creating 1000 tasks...")

    # Create tasks
    queue_info = await _populate_mixed_queue(queue_service, count=1_000)
    task_ids = queue_info["task_ids"]

    print("[Get Task] Measuring get_task latency...")
    iterations = 1000
    latencies = []

    for i in range(iterations):
        task_id = task_ids[i % len(task_ids)]

        start = time.perf_counter()
        task = await queue_service.get_task(task_id)
        elapsed = (time.perf_counter() - start) * 1000  # Convert to ms

        latencies.append(elapsed)
        assert task is not None
        assert task.id == task_id

    # Calculate statistics
    avg_latency = statistics.mean(latencies)
    median_latency = statistics.median(latencies)
    p95_latency = _calculate_percentile(latencies, 95)
    p99_latency = _calculate_percentile(latencies, 99)
    max_latency = max(latencies)

    metrics = {
        "test": "get_task_by_id",
        "queue_size": len(task_ids),
        "iterations": iterations,
        "avg_latency_ms": round(avg_latency, 3),
        "median_latency_ms": round(median_latency, 3),
        "p95_latency_ms": round(p95_latency, 3),
        "p99_latency_ms": round(p99_latency, 3),
        "max_latency_ms": round(max_latency, 3),
        "nfr_target_avg": "< 5ms",
        "nfr_target_p95": "< 50ms",
        "avg_target_met": avg_latency < 5.0,
        "p95_target_met": p95_latency < 50.0
    }

    print(f"\n[Get Task Metrics]\n{json.dumps(metrics, indent=2)}")

    # NFR assertions
    assert avg_latency < 5.0, f"Average get_task latency {avg_latency:.3f}ms exceeds 5ms target"
    assert p95_latency < 50.0, f"P95 get_task latency {p95_latency:.3f}ms exceeds 50ms target"


@pytest.mark.benchmark
@pytest.mark.asyncio
async def test_list_tasks_performance(queue_service: TaskQueueService) -> None:
    """Benchmark list_tasks operation.

    Performance Target: <50ms for 100 tasks, <50ms p95

    This test:
    1. Populates queue with 10,000 tasks
    2. Lists tasks with varying limits (10, 50, 100, 500)
    3. Measures list operation latency
    4. Verifies performance scales appropriately
    """
    print("\n[List Tasks Benchmark] Populating queue with 10,000 tasks...")

    # Create tasks
    await _populate_mixed_queue(queue_service, count=10_000)

    print("[List Tasks] Measuring list_tasks latency at different limits...")
    limits = [10, 50, 100, 500]
    results = []

    for limit in limits:
        latencies = []

        for _ in range(100):
            start = time.perf_counter()
            tasks = await queue_service.list_tasks(limit=limit)
            elapsed = (time.perf_counter() - start) * 1000

            latencies.append(elapsed)
            assert len(tasks) <= limit

        avg_latency = statistics.mean(latencies)
        p95_latency = _calculate_percentile(latencies, 95)

        results.append({
            "limit": limit,
            "avg_latency_ms": round(avg_latency, 3),
            "p95_latency_ms": round(p95_latency, 3),
            "target_met": p95_latency < 50.0
        })

        print(f"[List Tasks] Limit={limit}: Avg={avg_latency:.3f}ms, P95={p95_latency:.3f}ms")

    metrics = {
        "test": "list_tasks",
        "queue_size": 10_000,
        "results": results,
        "nfr_target": "< 50ms p95 for 100 tasks",
        "all_targets_met": all(r["target_met"] for r in results)
    }

    print(f"\n[List Tasks Metrics]\n{json.dumps(metrics, indent=2)}")

    # NFR assertion for limit=100
    limit_100_result = next(r for r in results if r["limit"] == 100)
    assert limit_100_result["p95_latency_ms"] < 50.0, \
        f"P95 list_tasks(100) latency {limit_100_result['p95_latency_ms']:.3f}ms exceeds 50ms target"


@pytest.mark.benchmark
@pytest.mark.asyncio
async def test_list_tasks_with_filters_performance(queue_service: TaskQueueService) -> None:
    """Benchmark list_tasks with status and agent type filters.

    Performance Target: <50ms p95 with filters

    This test:
    1. Populates queue with 10,000 mixed tasks
    2. Filters by status, agent_type, feature_branch
    3. Measures filtered query latency
    4. Verifies indexes are being used effectively
    """
    print("\n[Filtered List Benchmark] Populating queue with 10,000 tasks...")

    # Create tasks
    await _populate_mixed_queue(queue_service, count=10_000)

    print("[Filtered List] Measuring filtered list_tasks latency...")
    filter_tests = [
        {"name": "status_filter", "status": TaskStatus.READY},
        {"name": "agent_type_filter", "agent_type": "agent-1"},
        {"name": "feature_branch_filter", "feature_branch": "feature-a"},
        {"name": "combined_filters", "status": TaskStatus.READY, "feature_branch": "feature-a"}
    ]

    results = []

    for filter_test in filter_tests:
        filter_name = filter_test.pop("name")
        latencies = []

        for _ in range(100):
            start = time.perf_counter()
            tasks = await queue_service.list_tasks(limit=100, **filter_test)
            elapsed = (time.perf_counter() - start) * 1000

            latencies.append(elapsed)

        avg_latency = statistics.mean(latencies)
        p95_latency = _calculate_percentile(latencies, 95)

        results.append({
            "filter": filter_name,
            "avg_latency_ms": round(avg_latency, 3),
            "p95_latency_ms": round(p95_latency, 3),
            "target_met": p95_latency < 50.0
        })

    metrics = {
        "test": "filtered_list_tasks",
        "queue_size": 10_000,
        "results": results,
        "nfr_target": "< 50ms p95",
        "all_targets_met": all(r["target_met"] for r in results)
    }

    print(f"\n[Filtered List Metrics]\n{json.dumps(metrics, indent=2)}")

    # NFR assertion
    assert all(r["target_met"] for r in results), "Some filtered queries exceeded 50ms p95 target"


@pytest.mark.benchmark
@pytest.mark.asyncio
async def test_get_queue_status_performance(queue_service: TaskQueueService) -> None:
    """Benchmark get_queue_status operation.

    Performance Target: <20ms average, <50ms p95

    This test:
    1. Populates queue with 10,000 mixed status tasks
    2. Repeatedly queries queue status
    3. Measures status aggregation latency
    4. Verifies <20ms average, <50ms p95
    """
    print("\n[Queue Status Benchmark] Populating queue with 10,000 tasks...")

    # Create tasks
    await _populate_mixed_queue(queue_service, count=10_000)

    print("[Queue Status] Measuring get_queue_status latency...")
    iterations = 1000
    latencies = []

    for _ in range(iterations):
        start = time.perf_counter()
        status = await queue_service.get_queue_status()
        elapsed = (time.perf_counter() - start) * 1000

        latencies.append(elapsed)
        assert status is not None
        assert status.total_tasks == 10_000

    # Calculate statistics
    avg_latency = statistics.mean(latencies)
    median_latency = statistics.median(latencies)
    p95_latency = _calculate_percentile(latencies, 95)
    p99_latency = _calculate_percentile(latencies, 99)
    max_latency = max(latencies)

    metrics = {
        "test": "get_queue_status",
        "queue_size": 10_000,
        "iterations": iterations,
        "avg_latency_ms": round(avg_latency, 3),
        "median_latency_ms": round(median_latency, 3),
        "p95_latency_ms": round(p95_latency, 3),
        "p99_latency_ms": round(p99_latency, 3),
        "max_latency_ms": round(max_latency, 3),
        "nfr_target_avg": "< 20ms",
        "nfr_target_p95": "< 50ms",
        "avg_target_met": avg_latency < 20.0,
        "p95_target_met": p95_latency < 50.0
    }

    print(f"\n[Queue Status Metrics]\n{json.dumps(metrics, indent=2)}")

    # NFR assertions
    assert avg_latency < 20.0, f"Average queue status latency {avg_latency:.3f}ms exceeds 20ms target"
    assert p95_latency < 50.0, f"P95 queue status latency {p95_latency:.3f}ms exceeds 50ms target"


@pytest.mark.benchmark
@pytest.mark.asyncio
async def test_feature_branch_summary_performance(queue_service: TaskQueueService) -> None:
    """Benchmark get_feature_branch_summary operation.

    Performance Target: <50ms average, <50ms p95

    This test:
    1. Creates tasks across multiple feature branches
    2. Queries summary for each feature branch
    3. Measures summary aggregation latency
    4. Verifies <50ms average, <50ms p95
    """
    print("\n[Feature Branch Summary Benchmark] Creating feature branch tasks...")

    # Create tasks across feature branches
    queue_info = await _populate_mixed_queue(queue_service, count=5_000)
    feature_branches = queue_info["feature_branches"]

    print("[Feature Branch Summary] Measuring summary latency...")
    latencies = []

    for _ in range(300):  # 100 iterations per branch
        branch = feature_branches[_ % len(feature_branches)]

        start = time.perf_counter()
        summary = await queue_service.get_feature_branch_summary(branch)
        elapsed = (time.perf_counter() - start) * 1000

        latencies.append(elapsed)
        assert summary is not None

    # Calculate statistics
    avg_latency = statistics.mean(latencies)
    median_latency = statistics.median(latencies)
    p95_latency = _calculate_percentile(latencies, 95)
    p99_latency = _calculate_percentile(latencies, 99)
    max_latency = max(latencies)

    metrics = {
        "test": "feature_branch_summary",
        "total_tasks": 5_000,
        "feature_branches": len(feature_branches),
        "iterations": len(latencies),
        "avg_latency_ms": round(avg_latency, 3),
        "median_latency_ms": round(median_latency, 3),
        "p95_latency_ms": round(p95_latency, 3),
        "p99_latency_ms": round(p99_latency, 3),
        "max_latency_ms": round(max_latency, 3),
        "nfr_target_avg": "< 50ms",
        "nfr_target_p95": "< 50ms",
        "avg_target_met": avg_latency < 50.0,
        "p95_target_met": p95_latency < 50.0
    }

    print(f"\n[Feature Branch Summary Metrics]\n{json.dumps(metrics, indent=2)}")

    # NFR assertions
    assert avg_latency < 50.0, f"Average feature branch summary latency {avg_latency:.3f}ms exceeds 50ms target"
    assert p95_latency < 50.0, f"P95 feature branch summary latency {p95_latency:.3f}ms exceeds 50ms target"


@pytest.mark.benchmark
@pytest.mark.asyncio
async def test_status_query_scaling(queue_service: TaskQueueService) -> None:
    """Verify status queries scale to 10,000 tasks without degradation.

    NFR Requirement: Status queries scale to 10K tasks

    This test:
    1. Tests status queries at 1K, 5K, and 10K scales
    2. Measures get_queue_status latency at each scale
    3. Verifies < 2x degradation from 1K to 10K
    """
    print("\n[Status Query Scaling Benchmark] Testing at different scales...")

    scales = [1_000, 5_000, 10_000]
    results = []

    for scale in scales:
        print(f"\n[Scale: {scale}] Populating queue with {scale} tasks...")

        # Fresh database for each scale
        from pathlib import Path
        await queue_service._db.close()
        queue_service._db = Database(Path(":memory:"))
        await queue_service._db.initialize()

        # Populate
        await _populate_mixed_queue(queue_service, count=scale)

        # Measure get_queue_status
        latencies = []
        for _ in range(100):
            start = time.perf_counter()
            await queue_service.get_queue_status()
            elapsed = (time.perf_counter() - start) * 1000
            latencies.append(elapsed)

        avg_latency = statistics.mean(latencies)
        p95_latency = _calculate_percentile(latencies, 95)

        results.append({
            "scale": scale,
            "avg_latency_ms": round(avg_latency, 3),
            "p95_latency_ms": round(p95_latency, 3)
        })

        print(f"[Scale: {scale}] Avg: {avg_latency:.3f}ms, P95: {p95_latency:.3f}ms")

    # Analyze degradation
    baseline = results[0]["avg_latency_ms"]
    max_scale = results[-1]["avg_latency_ms"]
    degradation_factor = max_scale / baseline if baseline > 0 else 0

    metrics = {
        "test": "status_query_scaling",
        "results": results,
        "baseline_latency_ms": baseline,
        "10k_latency_ms": max_scale,
        "degradation_factor": round(degradation_factor, 2),
        "nfr_target": "< 2x degradation at 10K",
        "target_met": degradation_factor < 2.0
    }

    print(f"\n[Status Query Scaling Metrics]\n{json.dumps(metrics, indent=2)}")

    # NFR assertion
    assert degradation_factor < 2.0, \
        f"Status queries degraded {degradation_factor:.2f}x at 10K tasks (target: < 2x)"


@pytest.mark.benchmark
@pytest.mark.asyncio
async def test_concurrent_status_queries(queue_service: TaskQueueService) -> None:
    """Benchmark concurrent status query performance.

    Performance Target: Maintain <50ms p95 under concurrent load

    This test simulates multiple concurrent status queries
    (sequential execution for deterministic benchmarking).
    """
    print("\n[Concurrent Status Benchmark] Populating queue with 10,000 tasks...")

    # Create tasks
    await _populate_mixed_queue(queue_service, count=10_000)

    print("[Concurrent Status] Simulating concurrent status queries...")
    iterations = 200
    latencies = []

    # Simulate concurrent queries (sequential for benchmarking)
    for _ in range(iterations):
        start = time.perf_counter()
        await queue_service.get_queue_status()
        elapsed = (time.perf_counter() - start) * 1000

        latencies.append(elapsed)

    # Calculate statistics
    avg_latency = statistics.mean(latencies)
    p95_latency = _calculate_percentile(latencies, 95)
    p99_latency = _calculate_percentile(latencies, 99)

    metrics = {
        "test": "concurrent_status_queries",
        "queue_size": 10_000,
        "iterations": iterations,
        "avg_latency_ms": round(avg_latency, 3),
        "p95_latency_ms": round(p95_latency, 3),
        "p99_latency_ms": round(p99_latency, 3),
        "nfr_target": "< 50ms p95 under load",
        "target_met": p95_latency < 50.0
    }

    print(f"\n[Concurrent Status Metrics]\n{json.dumps(metrics, indent=2)}")

    # NFR assertion
    assert p95_latency < 50.0, f"Concurrent status query p95 {p95_latency:.3f}ms exceeds 50ms target"


@pytest.mark.benchmark
@pytest.mark.asyncio
async def test_count_by_status_performance(queue_service: TaskQueueService) -> None:
    """Benchmark counting tasks by status.

    Performance Target: <10ms for status counts

    This test measures the performance of status-based aggregations
    which are common in dashboard queries.
    """
    print("\n[Count by Status Benchmark] Populating queue with 10,000 tasks...")

    # Create tasks
    await _populate_mixed_queue(queue_service, count=10_000)

    print("[Count by Status] Measuring count aggregation latency...")
    iterations = 1000
    latencies = []

    for _ in range(iterations):
        start = time.perf_counter()
        # Get queue status includes status counts
        status = await queue_service.get_queue_status()
        elapsed = (time.perf_counter() - start) * 1000

        latencies.append(elapsed)
        assert status is not None

    # Calculate statistics
    avg_latency = statistics.mean(latencies)
    p95_latency = _calculate_percentile(latencies, 95)

    metrics = {
        "test": "count_by_status",
        "queue_size": 10_000,
        "iterations": iterations,
        "avg_latency_ms": round(avg_latency, 3),
        "p95_latency_ms": round(p95_latency, 3),
        "nfr_target": "< 10ms average",
        "target_met": avg_latency < 10.0
    }

    print(f"\n[Count by Status Metrics]\n{json.dumps(metrics, indent=2)}")

    # Performance assertion
    assert avg_latency < 10.0, f"Status count latency {avg_latency:.3f}ms exceeds 10ms target"
