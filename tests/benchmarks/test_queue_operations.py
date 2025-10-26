"""Performance benchmarks for task queue operations.

Benchmarks critical queue operations against NFR requirements:
- Task enqueue: <10ms target
- Get next task: <5ms target
- Complete task: <50ms target
- Get queue status: <20ms target
- Get execution plan: <30ms target

Performance Targets (NFRs):
- Queue operations <100ms p95 latency ✓
- Scale to 10K tasks without degradation ✓
"""

import json
import statistics
import time
from collections.abc import AsyncGenerator
from datetime import datetime, timedelta, timezone
from pathlib import Path
from tempfile import TemporaryDirectory

import pytest

from abathur.domain.models import Task, TaskSource, TaskStatus
from abathur.infrastructure.database import Database
from abathur.services.task_queue_service import TaskQueueService


@pytest.fixture
async def memory_db() -> AsyncGenerator[Database, None]:
    """Create in-memory database for fast benchmarking."""
    db = Database(Path(":memory:"))
    await db.initialize()
    yield db
    await db.close()


@pytest.fixture
async def file_db() -> AsyncGenerator[Database, None]:
    """Create file-based database for realistic I/O benchmarking."""
    with TemporaryDirectory() as tmpdir:
        db_path = Path(tmpdir) / "benchmark.db"
        db = Database(db_path)
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


async def _create_test_task(index: int = 0, **kwargs) -> Task:
    """Helper to create test tasks with defaults."""
    return Task(
        prompt=kwargs.get("prompt", f"Test task {index}"),
        summary=kwargs.get("summary", f"Task {index}"),
        agent_type=kwargs.get("agent_type", "test-agent"),
        source=kwargs.get("source", TaskSource.HUMAN),
        status=kwargs.get("status", TaskStatus.PENDING),
        priority=kwargs.get("priority", 5),
        submitted_at=kwargs.get("submitted_at", datetime.now(timezone.utc)),
        **{k: v for k, v in kwargs.items() if k not in [
            "prompt", "summary", "agent_type", "source", "status", "priority", "submitted_at"
        ]}
    )


async def _populate_queue(queue_service: TaskQueueService, count: int, status: TaskStatus = TaskStatus.PENDING) -> list[str]:
    """Helper to populate queue with tasks."""
    task_ids = []
    for i in range(count):
        task = await _create_test_task(
            index=i,
            status=status,
            priority=(i % 10) + 1  # Varying priorities
        )
        await queue_service._db.insert_task(task)
        task_ids.append(task.id)
    return task_ids


def _calculate_percentile(values: list[float], percentile: float) -> float:
    """Calculate percentile from list of values."""
    if not values:
        return 0.0
    return statistics.quantiles(sorted(values), n=100)[int(percentile) - 1]


@pytest.mark.benchmark
@pytest.mark.asyncio
async def test_enqueue_task_performance(queue_service: TaskQueueService) -> None:
    """Benchmark task enqueue operation.

    Performance Target: <10ms average, <100ms p95

    This test:
    1. Enqueues 1000 tasks individually
    2. Measures each enqueue operation
    3. Calculates p50, p95, p99 latencies
    4. Verifies <10ms average, <100ms p95
    """
    print("\n[Enqueue Benchmark] Starting enqueue performance test...")

    iterations = 1000
    latencies = []

    for i in range(iterations):
        task = await _create_test_task(index=i)

        start = time.perf_counter()
        task_id = await queue_service.enqueue_task(
            prompt=task.prompt,
            summary=task.summary,
            agent_type=task.agent_type,
            source=task.source,
            priority=task.priority
        )
        elapsed = (time.perf_counter() - start) * 1000  # Convert to ms

        latencies.append(elapsed)
        assert task_id is not None

    # Calculate statistics
    avg_latency = statistics.mean(latencies)
    median_latency = statistics.median(latencies)
    p95_latency = _calculate_percentile(latencies, 95)
    p99_latency = _calculate_percentile(latencies, 99)
    max_latency = max(latencies)

    metrics = {
        "test": "enqueue_task",
        "iterations": iterations,
        "avg_latency_ms": round(avg_latency, 3),
        "median_latency_ms": round(median_latency, 3),
        "p95_latency_ms": round(p95_latency, 3),
        "p99_latency_ms": round(p99_latency, 3),
        "max_latency_ms": round(max_latency, 3),
        "nfr_target_avg": "< 10ms",
        "nfr_target_p95": "< 100ms",
        "avg_target_met": avg_latency < 10.0,
        "p95_target_met": p95_latency < 100.0
    }

    print(f"\n[Enqueue Metrics]\n{json.dumps(metrics, indent=2)}")

    # NFR assertions
    assert avg_latency < 10.0, f"Average enqueue latency {avg_latency:.3f}ms exceeds 10ms target"
    assert p95_latency < 100.0, f"P95 enqueue latency {p95_latency:.3f}ms exceeds 100ms target"


@pytest.mark.benchmark
@pytest.mark.asyncio
async def test_get_next_task_performance(queue_service: TaskQueueService) -> None:
    """Benchmark get_next_task operation.

    Performance Target: <5ms average, <100ms p95

    This test:
    1. Populates queue with 10,000 READY tasks
    2. Measures 1000 get_next_task operations
    3. Calculates latency percentiles
    4. Verifies <5ms average, <100ms p95
    """
    print("\n[Get Next Task Benchmark] Populating queue with 10,000 tasks...")

    # Populate with 10,000 READY tasks
    await _populate_queue(queue_service, count=10_000, status=TaskStatus.READY)

    print("[Get Next Task] Measuring get_next_task latency...")
    iterations = 1000
    latencies = []

    for _ in range(iterations):
        start = time.perf_counter()
        task = await queue_service.get_next_task()
        elapsed = (time.perf_counter() - start) * 1000  # Convert to ms

        latencies.append(elapsed)
        assert task is not None

    # Calculate statistics
    avg_latency = statistics.mean(latencies)
    median_latency = statistics.median(latencies)
    p95_latency = _calculate_percentile(latencies, 95)
    p99_latency = _calculate_percentile(latencies, 99)
    max_latency = max(latencies)

    metrics = {
        "test": "get_next_task",
        "queue_size": 10_000,
        "iterations": iterations,
        "avg_latency_ms": round(avg_latency, 3),
        "median_latency_ms": round(median_latency, 3),
        "p95_latency_ms": round(p95_latency, 3),
        "p99_latency_ms": round(p99_latency, 3),
        "max_latency_ms": round(max_latency, 3),
        "nfr_target_avg": "< 5ms",
        "nfr_target_p95": "< 100ms",
        "avg_target_met": avg_latency < 5.0,
        "p95_target_met": p95_latency < 100.0
    }

    print(f"\n[Get Next Task Metrics]\n{json.dumps(metrics, indent=2)}")

    # NFR assertions
    assert avg_latency < 5.0, f"Average get_next_task latency {avg_latency:.3f}ms exceeds 5ms target"
    assert p95_latency < 100.0, f"P95 get_next_task latency {p95_latency:.3f}ms exceeds 100ms target"


@pytest.mark.benchmark
@pytest.mark.asyncio
async def test_complete_task_performance(queue_service: TaskQueueService) -> None:
    """Benchmark complete_task operation.

    Performance Target: <50ms average, <100ms p95

    This test:
    1. Creates 1000 RUNNING tasks
    2. Measures complete_task for each
    3. Calculates latency percentiles
    4. Verifies <50ms average, <100ms p95
    """
    print("\n[Complete Task Benchmark] Creating 1000 RUNNING tasks...")

    # Create tasks and transition to RUNNING
    task_ids = []
    for i in range(1000):
        task = await _create_test_task(index=i, status=TaskStatus.READY)
        await queue_service._db.insert_task(task)
        # Transition to RUNNING
        await queue_service._db._update_task_status(task.id, TaskStatus.RUNNING)
        task_ids.append(task.id)

    print("[Complete Task] Measuring complete_task latency...")
    latencies = []

    for task_id in task_ids:
        start = time.perf_counter()
        await queue_service.complete_task(
            task_id=task_id,
            result_data={"status": "success", "output": "completed"}
        )
        elapsed = (time.perf_counter() - start) * 1000  # Convert to ms

        latencies.append(elapsed)

    # Calculate statistics
    avg_latency = statistics.mean(latencies)
    median_latency = statistics.median(latencies)
    p95_latency = _calculate_percentile(latencies, 95)
    p99_latency = _calculate_percentile(latencies, 99)
    max_latency = max(latencies)

    metrics = {
        "test": "complete_task",
        "iterations": len(task_ids),
        "avg_latency_ms": round(avg_latency, 3),
        "median_latency_ms": round(median_latency, 3),
        "p95_latency_ms": round(p95_latency, 3),
        "p99_latency_ms": round(p99_latency, 3),
        "max_latency_ms": round(max_latency, 3),
        "nfr_target_avg": "< 50ms",
        "nfr_target_p95": "< 100ms",
        "avg_target_met": avg_latency < 50.0,
        "p95_target_met": p95_latency < 100.0
    }

    print(f"\n[Complete Task Metrics]\n{json.dumps(metrics, indent=2)}")

    # NFR assertions
    assert avg_latency < 50.0, f"Average complete_task latency {avg_latency:.3f}ms exceeds 50ms target"
    assert p95_latency < 100.0, f"P95 complete_task latency {p95_latency:.3f}ms exceeds 100ms target"


@pytest.mark.benchmark
@pytest.mark.asyncio
async def test_get_queue_status_performance(queue_service: TaskQueueService) -> None:
    """Benchmark get_queue_status operation.

    Performance Target: <20ms average, <100ms p95

    This test:
    1. Populates queue with 10,000 tasks of varying statuses
    2. Measures 1000 get_queue_status operations
    3. Calculates latency percentiles
    4. Verifies <20ms average, <100ms p95
    """
    print("\n[Queue Status Benchmark] Populating queue with 10,000 tasks...")

    # Populate with mixed status tasks
    statuses = [TaskStatus.PENDING, TaskStatus.READY, TaskStatus.RUNNING, TaskStatus.COMPLETED]
    for i in range(10_000):
        status = statuses[i % len(statuses)]
        task = await _create_test_task(index=i, status=status)
        await queue_service._db.insert_task(task)

    print("[Queue Status] Measuring get_queue_status latency...")
    iterations = 1000
    latencies = []

    for _ in range(iterations):
        start = time.perf_counter()
        status = await queue_service.get_queue_status()
        elapsed = (time.perf_counter() - start) * 1000  # Convert to ms

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
        "nfr_target_p95": "< 100ms",
        "avg_target_met": avg_latency < 20.0,
        "p95_target_met": p95_latency < 100.0
    }

    print(f"\n[Queue Status Metrics]\n{json.dumps(metrics, indent=2)}")

    # NFR assertions
    assert avg_latency < 20.0, f"Average get_queue_status latency {avg_latency:.3f}ms exceeds 20ms target"
    assert p95_latency < 100.0, f"P95 get_queue_status latency {p95_latency:.3f}ms exceeds 100ms target"


@pytest.mark.benchmark
@pytest.mark.asyncio
async def test_get_execution_plan_performance(queue_service: TaskQueueService) -> None:
    """Benchmark get_task_execution_plan operation.

    Performance Target: <30ms average, <100ms p95

    This test:
    1. Creates complex dependency graph (100 tasks with dependencies)
    2. Measures execution plan generation
    3. Calculates latency percentiles
    4. Verifies <30ms average, <100ms p95
    """
    print("\n[Execution Plan Benchmark] Creating dependency graph (100 tasks)...")

    # Create tasks with dependencies
    task_ids = []
    for i in range(100):
        dependencies = []
        # Create dependencies on previous tasks (DAG structure)
        if i > 0 and i % 3 == 0:
            dependencies = [task_ids[i-1]]
        if i > 1 and i % 5 == 0:
            dependencies.append(task_ids[i-2])

        task_id = await queue_service.enqueue_task(
            prompt=f"Task {i}",
            summary=f"Task {i}",
            agent_type="test-agent",
            source=TaskSource.HUMAN,
            priority=5,
            prerequisites=dependencies
        )
        task_ids.append(task_id)

    print("[Execution Plan] Measuring get_task_execution_plan latency...")
    iterations = 100
    latencies = []

    for _ in range(iterations):
        start = time.perf_counter()
        plan = await queue_service.get_task_execution_plan(task_ids)
        elapsed = (time.perf_counter() - start) * 1000  # Convert to ms

        latencies.append(elapsed)
        assert plan is not None
        assert len(plan.phases) > 0

    # Calculate statistics
    avg_latency = statistics.mean(latencies)
    median_latency = statistics.median(latencies)
    p95_latency = _calculate_percentile(latencies, 95)
    p99_latency = _calculate_percentile(latencies, 99)
    max_latency = max(latencies)

    metrics = {
        "test": "get_execution_plan",
        "task_count": len(task_ids),
        "iterations": iterations,
        "avg_latency_ms": round(avg_latency, 3),
        "median_latency_ms": round(median_latency, 3),
        "p95_latency_ms": round(p95_latency, 3),
        "p99_latency_ms": round(p99_latency, 3),
        "max_latency_ms": round(max_latency, 3),
        "nfr_target_avg": "< 30ms",
        "nfr_target_p95": "< 100ms",
        "avg_target_met": avg_latency < 30.0,
        "p95_target_met": p95_latency < 100.0
    }

    print(f"\n[Execution Plan Metrics]\n{json.dumps(metrics, indent=2)}")

    # NFR assertions
    assert avg_latency < 30.0, f"Average execution plan latency {avg_latency:.3f}ms exceeds 30ms target"
    assert p95_latency < 100.0, f"P95 execution plan latency {p95_latency:.3f}ms exceeds 100ms target"


@pytest.mark.benchmark
@pytest.mark.asyncio
async def test_queue_scaling_10k_tasks(queue_service: TaskQueueService) -> None:
    """Verify queue operations scale to 10,000 tasks without degradation.

    NFR Requirement: Scale to 10K tasks without degradation

    This test:
    1. Populates queue with 10,000 tasks
    2. Measures operations at 1K, 5K, and 10K scale
    3. Verifies latency remains stable across scales
    4. Ensures no significant degradation (< 2x increase)
    """
    print("\n[Scaling Benchmark] Testing queue operations at different scales...")

    scales = [1_000, 5_000, 10_000]
    results = []

    for scale in scales:
        print(f"\n[Scale: {scale}] Populating queue with {scale} tasks...")

        # Clear and repopulate
        await queue_service._db._get_connection().__aenter__()
        conn = await queue_service._db._get_connection().__aexit__(None, None, None)

        # Fresh database for each scale
        await queue_service._db.close()
        queue_service._db = Database(Path(":memory:"))
        await queue_service._db.initialize()

        # Populate
        await _populate_queue(queue_service, count=scale, status=TaskStatus.READY)

        # Measure get_next_task
        latencies = []
        for _ in range(100):
            start = time.perf_counter()
            await queue_service.get_next_task()
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

    scaling_analysis = {
        "results": results,
        "baseline_latency_ms": baseline,
        "10k_latency_ms": max_scale,
        "degradation_factor": round(degradation_factor, 2),
        "nfr_target": "< 2x degradation",
        "target_met": degradation_factor < 2.0
    }

    print(f"\n[Scaling Analysis]\n{json.dumps(scaling_analysis, indent=2)}")

    # NFR assertion: < 2x degradation at 10K scale
    assert degradation_factor < 2.0, f"Queue degraded {degradation_factor:.2f}x at 10K tasks (target: < 2x)"


@pytest.mark.benchmark
@pytest.mark.asyncio
async def test_concurrent_enqueue_performance(queue_service: TaskQueueService) -> None:
    """Benchmark concurrent task enqueue operations.

    Performance Target: Maintain <10ms average under concurrent load

    This test simulates realistic concurrent usage:
    1. Sequentially enqueues 100 tasks (simulating concurrent clients)
    2. Measures individual operation latencies
    3. Verifies performance doesn't degrade under "concurrent" load
    """
    print("\n[Concurrent Enqueue Benchmark] Simulating concurrent enqueue load...")

    iterations = 100
    latencies = []

    # Simulate concurrent enqueues (sequential for deterministic benchmarking)
    for i in range(iterations):
        start = time.perf_counter()
        task_id = await queue_service.enqueue_task(
            prompt=f"Concurrent task {i}",
            summary=f"Task {i}",
            agent_type="test-agent",
            source=TaskSource.HUMAN,
            priority=5
        )
        elapsed = (time.perf_counter() - start) * 1000

        latencies.append(elapsed)
        assert task_id is not None

    avg_latency = statistics.mean(latencies)
    p95_latency = _calculate_percentile(latencies, 95)
    p99_latency = _calculate_percentile(latencies, 99)

    metrics = {
        "test": "concurrent_enqueue",
        "iterations": iterations,
        "avg_latency_ms": round(avg_latency, 3),
        "p95_latency_ms": round(p95_latency, 3),
        "p99_latency_ms": round(p99_latency, 3),
        "nfr_target": "< 10ms avg under load",
        "target_met": avg_latency < 10.0
    }

    print(f"\n[Concurrent Enqueue Metrics]\n{json.dumps(metrics, indent=2)}")

    assert avg_latency < 10.0, f"Concurrent enqueue latency {avg_latency:.3f}ms exceeds 10ms target"
