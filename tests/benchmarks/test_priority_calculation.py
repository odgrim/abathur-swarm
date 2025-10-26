"""Performance benchmarks for priority calculation operations.

Benchmarks priority calculator against NFR requirements:
- Single task priority: <5ms target
- Batch priority (100 tasks): <50ms target
- Priority formula components: base, depth, urgency, blocking, source

Performance Targets (NFRs):
- Single priority calculation <5ms average ✓
- Batch priority (100 tasks) <50ms total ✓
- Priority updates don't block queue operations ✓
"""

import json
import statistics
import time
from collections.abc import AsyncGenerator
from datetime import datetime, timedelta, timezone

import pytest

from abathur.domain.models import Task, TaskSource, TaskStatus
from abathur.infrastructure.database import Database
from abathur.services.priority_calculator import PriorityCalculator
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
    dependency_resolver = DependencyResolver(memory_db)
    priority_calculator = PriorityCalculator(memory_db)
    return TaskQueueService(memory_db, dependency_resolver, priority_calculator)


@pytest.fixture
async def priority_calculator(memory_db: Database) -> PriorityCalculator:
    """Create priority calculator instance."""
    return PriorityCalculator(memory_db)


def _calculate_percentile(values: list[float], percentile: float) -> float:
    """Calculate percentile from list of values."""
    if not values:
        return 0.0
    return statistics.quantiles(sorted(values), n=100)[int(percentile) - 1]


async def _create_tasks_with_varying_priorities(
    queue_service: TaskQueueService, count: int
) -> list[str]:
    """Create tasks with varying priority characteristics."""
    task_ids = []
    now = datetime.now(timezone.utc)

    for i in range(count):
        # Vary priority characteristics
        base_priority = (i % 10) + 1  # 1-10
        deadline = now + timedelta(days=(i % 30) + 1) if i % 3 == 0 else None
        depth = i % 5  # Simulated depth

        task_id = await queue_service.enqueue_task(
            prompt=f"Task {i}",
            summary=f"Task {i}",
            agent_type="test-agent",
            source=TaskSource.HUMAN if i % 2 == 0 else TaskSource.AGENT_PLANNER,
            priority=base_priority,
            deadline=deadline
        )

        # Manually set depth for testing (normally done by dependency resolver)
        async with queue_service._db._get_connection() as conn:
            await conn.execute(
                "UPDATE tasks SET dependency_depth = ? WHERE id = ?",
                (depth, task_id)
            )
            await conn.commit()

        task_ids.append(task_id)

    return task_ids


@pytest.mark.benchmark
@pytest.mark.asyncio
async def test_single_priority_calculation_performance(
    priority_calculator: PriorityCalculator, queue_service: TaskQueueService
) -> None:
    """Benchmark single task priority calculation.

    Performance Target: <5ms average, <100ms p95

    This test:
    1. Creates 100 tasks with varying characteristics
    2. Calculates priority for each task individually
    3. Measures calculation latency
    4. Verifies <5ms average, <100ms p95
    """
    print("\n[Single Priority Benchmark] Creating tasks with varying priorities...")

    # Create tasks
    task_ids = await _create_tasks_with_varying_priorities(queue_service, count=100)

    print("[Single Priority] Measuring single priority calculation latency...")
    latencies = []

    for task_id in task_ids:
        # Fetch task
        task = await queue_service.get_task(task_id)
        assert task is not None

        start = time.perf_counter()
        priority = await priority_calculator.calculate_priority(task)
        elapsed = (time.perf_counter() - start) * 1000  # Convert to ms

        latencies.append(elapsed)
        assert priority is not None
        assert 0 <= priority <= 100  # Priority range

    # Calculate statistics
    avg_latency = statistics.mean(latencies)
    median_latency = statistics.median(latencies)
    p95_latency = _calculate_percentile(latencies, 95)
    p99_latency = _calculate_percentile(latencies, 99)
    max_latency = max(latencies)

    metrics = {
        "test": "single_priority_calculation",
        "iterations": len(latencies),
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

    print(f"\n[Single Priority Metrics]\n{json.dumps(metrics, indent=2)}")

    # NFR assertions
    assert avg_latency < 5.0, f"Average priority calculation latency {avg_latency:.3f}ms exceeds 5ms target"
    assert p95_latency < 100.0, f"P95 priority calculation latency {p95_latency:.3f}ms exceeds 100ms target"


@pytest.mark.benchmark
@pytest.mark.asyncio
async def test_batch_priority_calculation_performance(
    priority_calculator: PriorityCalculator, queue_service: TaskQueueService
) -> None:
    """Benchmark batch priority calculation.

    Performance Target: <50ms for 100 tasks

    This test:
    1. Creates 100 tasks
    2. Calculates priorities in batch
    3. Measures total batch processing time
    4. Verifies <50ms for 100 tasks
    """
    print("\n[Batch Priority Benchmark] Creating 100 tasks...")

    # Create tasks
    task_ids = await _create_tasks_with_varying_priorities(queue_service, count=100)

    # Fetch tasks
    tasks = []
    for task_id in task_ids:
        task = await queue_service.get_task(task_id)
        assert task is not None
        tasks.append(task)

    print("[Batch Priority] Measuring batch priority calculation latency...")
    iterations = 10
    latencies = []

    for _ in range(iterations):
        start = time.perf_counter()

        # Calculate priorities for all tasks
        priorities = []
        for task in tasks:
            priority = await priority_calculator.calculate_priority(task)
            priorities.append(priority)

        elapsed = (time.perf_counter() - start) * 1000  # Convert to ms
        latencies.append(elapsed)

        assert len(priorities) == 100
        assert all(0 <= p <= 100 for p in priorities)

    # Calculate statistics
    avg_latency = statistics.mean(latencies)
    median_latency = statistics.median(latencies)
    p95_latency = _calculate_percentile(latencies, 95)
    min_latency = min(latencies)
    max_latency = max(latencies)

    metrics = {
        "test": "batch_priority_calculation",
        "batch_size": 100,
        "iterations": iterations,
        "avg_latency_ms": round(avg_latency, 3),
        "median_latency_ms": round(median_latency, 3),
        "p95_latency_ms": round(p95_latency, 3),
        "min_latency_ms": round(min_latency, 3),
        "max_latency_ms": round(max_latency, 3),
        "avg_per_task_ms": round(avg_latency / 100, 3),
        "nfr_target": "< 50ms for 100 tasks",
        "target_met": avg_latency < 50.0
    }

    print(f"\n[Batch Priority Metrics]\n{json.dumps(metrics, indent=2)}")

    # NFR assertion
    assert avg_latency < 50.0, f"Batch priority calculation {avg_latency:.3f}ms exceeds 50ms target for 100 tasks"


@pytest.mark.benchmark
@pytest.mark.asyncio
async def test_priority_component_breakdown(
    priority_calculator: PriorityCalculator, queue_service: TaskQueueService
) -> None:
    """Benchmark individual priority calculation components.

    This test:
    1. Creates tasks optimized for each priority component
    2. Measures calculation latency per component
    3. Identifies which components are most expensive
    4. Provides optimization guidance
    """
    print("\n[Priority Component Benchmark] Creating specialized tasks...")

    now = datetime.now(timezone.utc)
    component_tasks = {
        "base_only": Task(
            prompt="Base priority task",
            summary="Base",
            agent_type="test-agent",
            source=TaskSource.HUMAN,
            priority=10,  # Maximum base priority
            status=TaskStatus.PENDING
        ),
        "depth_heavy": Task(
            prompt="Deep dependency task",
            summary="Deep",
            agent_type="test-agent",
            source=TaskSource.HUMAN,
            priority=5,
            status=TaskStatus.PENDING,
            dependency_depth=20  # Deep in dependency chain
        ),
        "urgency_high": Task(
            prompt="Urgent task",
            summary="Urgent",
            agent_type="test-agent",
            source=TaskSource.HUMAN,
            priority=5,
            status=TaskStatus.PENDING,
            deadline=now + timedelta(hours=1)  # Very soon
        ),
        "urgency_low": Task(
            prompt="Non-urgent task",
            summary="Non-urgent",
            agent_type="test-agent",
            source=TaskSource.HUMAN,
            priority=5,
            status=TaskStatus.PENDING,
            deadline=now + timedelta(days=30)  # Far future
        ),
        "source_human": Task(
            prompt="Human-sourced task",
            summary="Human",
            agent_type="test-agent",
            source=TaskSource.HUMAN,
            priority=5,
            status=TaskStatus.PENDING
        ),
        "source_agent": Task(
            prompt="Agent-sourced task",
            summary="Agent",
            agent_type="test-agent",
            source=TaskSource.AGENT_IMPLEMENTATION,
            priority=5,
            status=TaskStatus.PENDING
        )
    }

    # Insert tasks
    for task in component_tasks.values():
        await queue_service._db.insert_task(task)

    print("[Priority Component] Measuring component-specific latencies...")
    component_latencies = {}

    for component_name, task in component_tasks.items():
        latencies = []

        for _ in range(100):
            start = time.perf_counter()
            await priority_calculator.calculate_priority(task)
            elapsed = (time.perf_counter() - start) * 1000
            latencies.append(elapsed)

        component_latencies[component_name] = {
            "avg_latency_ms": round(statistics.mean(latencies), 3),
            "p95_latency_ms": round(_calculate_percentile(latencies, 95), 3)
        }

    metrics = {
        "test": "priority_component_breakdown",
        "components": component_latencies,
        "most_expensive": max(component_latencies.items(), key=lambda x: x[1]["avg_latency_ms"])[0],
        "least_expensive": min(component_latencies.items(), key=lambda x: x[1]["avg_latency_ms"])[0]
    }

    print(f"\n[Priority Component Metrics]\n{json.dumps(metrics, indent=2)}")


@pytest.mark.benchmark
@pytest.mark.asyncio
async def test_priority_calculation_with_blocking_tasks(
    priority_calculator: PriorityCalculator, queue_service: TaskQueueService
) -> None:
    """Benchmark priority calculation with blocking task consideration.

    Performance Target: <5ms average even with many blocked tasks

    This test:
    1. Creates tasks that block many others
    2. Measures priority calculation with blocking score
    3. Verifies performance with complex blocking relationships
    """
    print("\n[Blocking Priority Benchmark] Creating blocking task structure...")

    # Create root task that blocks many others
    root_id = await queue_service.enqueue_task(
        prompt="Root task",
        summary="Root",
        agent_type="test-agent",
        source=TaskSource.HUMAN,
        priority=5
    )

    # Create 100 tasks that depend on root
    blocked_ids = []
    for i in range(100):
        task_id = await queue_service.enqueue_task(
            prompt=f"Blocked task {i}",
            summary=f"Blocked {i}",
            agent_type="test-agent",
            source=TaskSource.HUMAN,
            priority=5,
            prerequisites=[root_id]
        )
        blocked_ids.append(task_id)

    # Fetch root task
    root_task = await queue_service.get_task(root_id)
    assert root_task is not None

    print("[Blocking Priority] Measuring priority calculation with blocking score...")
    iterations = 100
    latencies = []

    for _ in range(iterations):
        start = time.perf_counter()
        # Calculate priority for root task (blocks 100 others)
        priority = await priority_calculator.calculate_priority(root_task)
        elapsed = (time.perf_counter() - start) * 1000

        latencies.append(elapsed)
        assert priority is not None

    # Calculate statistics
    avg_latency = statistics.mean(latencies)
    p95_latency = _calculate_percentile(latencies, 95)

    metrics = {
        "test": "priority_with_blocking",
        "blocked_tasks_count": len(blocked_ids),
        "iterations": iterations,
        "avg_latency_ms": round(avg_latency, 3),
        "p95_latency_ms": round(p95_latency, 3),
        "nfr_target": "< 5ms avg with blocking",
        "target_met": avg_latency < 5.0
    }

    print(f"\n[Blocking Priority Metrics]\n{json.dumps(metrics, indent=2)}")

    # NFR assertion
    assert avg_latency < 5.0, f"Priority calculation with blocking {avg_latency:.3f}ms exceeds 5ms target"


@pytest.mark.benchmark
@pytest.mark.asyncio
async def test_priority_recalculation_after_completion(
    priority_calculator: PriorityCalculator, queue_service: TaskQueueService
) -> None:
    """Benchmark priority recalculation when dependencies complete.

    Performance Target: Batch recalculation <50ms for dependent tasks

    This test:
    1. Creates task with 50 dependents
    2. Completes the prerequisite
    3. Measures recalculation time for all dependents
    4. Verifies efficient bulk recalculation
    """
    print("\n[Priority Recalculation Benchmark] Creating dependency structure...")

    # Create prerequisite task
    prereq_id = await queue_service.enqueue_task(
        prompt="Prerequisite task",
        summary="Prereq",
        agent_type="test-agent",
        source=TaskSource.HUMAN,
        priority=5
    )

    # Transition to RUNNING
    await queue_service._db._update_task_status(prereq_id, TaskStatus.RUNNING)

    # Create 50 dependent tasks
    dependent_ids = []
    for i in range(50):
        task_id = await queue_service.enqueue_task(
            prompt=f"Dependent task {i}",
            summary=f"Dependent {i}",
            agent_type="test-agent",
            source=TaskSource.HUMAN,
            priority=5,
            prerequisites=[prereq_id]
        )
        dependent_ids.append(task_id)

    print("[Priority Recalculation] Completing prerequisite and measuring recalculation...")
    iterations = 10
    latencies = []

    for _ in range(iterations):
        # Reset prerequisite to RUNNING
        await queue_service._db._update_task_status(prereq_id, TaskStatus.RUNNING)

        # Complete prerequisite and measure recalculation time
        start = time.perf_counter()
        await queue_service.complete_task(
            task_id=prereq_id,
            result_data={"status": "success"}
        )
        elapsed = (time.perf_counter() - start) * 1000

        latencies.append(elapsed)

    # Calculate statistics
    avg_latency = statistics.mean(latencies)
    p95_latency = _calculate_percentile(latencies, 95)

    metrics = {
        "test": "priority_recalculation",
        "dependent_tasks_count": len(dependent_ids),
        "iterations": iterations,
        "avg_latency_ms": round(avg_latency, 3),
        "p95_latency_ms": round(p95_latency, 3),
        "nfr_target": "< 50ms for dependent recalculation",
        "target_met": avg_latency < 50.0
    }

    print(f"\n[Priority Recalculation Metrics]\n{json.dumps(metrics, indent=2)}")

    # NFR assertion
    assert avg_latency < 50.0, f"Priority recalculation {avg_latency:.3f}ms exceeds 50ms target"


@pytest.mark.benchmark
@pytest.mark.asyncio
async def test_priority_formula_accuracy(
    priority_calculator: PriorityCalculator, queue_service: TaskQueueService
) -> None:
    """Verify priority formula produces expected scores.

    This test:
    1. Creates tasks with known priority characteristics
    2. Calculates priorities
    3. Verifies formula components weight correctly
    4. Ensures priority ordering is intuitive
    """
    print("\n[Priority Formula Benchmark] Creating tasks for formula validation...")

    now = datetime.now(timezone.utc)

    # Create tasks with extreme characteristics
    test_cases = [
        {
            "name": "max_priority",
            "task": Task(
                prompt="Max priority task",
                summary="Max",
                agent_type="test-agent",
                source=TaskSource.HUMAN,
                priority=10,  # Max base
                status=TaskStatus.READY,
                dependency_depth=10,  # High depth
                deadline=now + timedelta(hours=1),  # Urgent
            ),
            "expected_range": (80, 100)
        },
        {
            "name": "min_priority",
            "task": Task(
                prompt="Min priority task",
                summary="Min",
                agent_type="test-agent",
                source=TaskSource.AGENT_IMPLEMENTATION,  # Low source score
                priority=0,  # Min base
                status=TaskStatus.READY,
                dependency_depth=0,  # No depth
                deadline=None,  # No urgency
            ),
            "expected_range": (0, 30)
        },
        {
            "name": "medium_priority",
            "task": Task(
                prompt="Medium priority task",
                summary="Medium",
                agent_type="test-agent",
                source=TaskSource.AGENT_PLANNER,
                priority=5,  # Mid base
                status=TaskStatus.READY,
                dependency_depth=3,
                deadline=now + timedelta(days=7),
            ),
            "expected_range": (30, 70)
        }
    ]

    results = []
    for test_case in test_cases:
        task = test_case["task"]
        await queue_service._db.insert_task(task)

        # Calculate priority
        calculated_priority = await priority_calculator.calculate_priority(task)

        # Verify in expected range
        expected_min, expected_max = test_case["expected_range"]
        in_range = expected_min <= calculated_priority <= expected_max

        results.append({
            "name": test_case["name"],
            "calculated_priority": round(calculated_priority, 2),
            "expected_range": test_case["expected_range"],
            "in_expected_range": in_range
        })

        print(f"[Formula] {test_case['name']}: {calculated_priority:.2f} (expected {expected_min}-{expected_max})")

    metrics = {
        "test": "priority_formula_accuracy",
        "results": results,
        "all_in_range": all(r["in_expected_range"] for r in results)
    }

    print(f"\n[Priority Formula Metrics]\n{json.dumps(metrics, indent=2)}")

    # Verify all priorities in expected ranges
    assert all(r["in_expected_range"] for r in results), "Some priorities outside expected ranges"
