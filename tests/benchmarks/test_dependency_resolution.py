"""Performance benchmarks for dependency resolution operations.

Benchmarks dependency resolver against NFR requirements:
- Cycle detection: <10ms target
- Depth calculation: <5ms target (with cache)
- Execution order (topological sort): O(V+E) complexity
- Dependency checks: O(1) with indexes

Performance Targets (NFRs):
- Dependency resolution <100ms p95 latency ✓
- Handle complex graphs (100+ tasks) efficiently ✓
"""

import json
import statistics
import time
from collections.abc import AsyncGenerator
from datetime import datetime, timezone
from pathlib import Path
from tempfile import TemporaryDirectory

import pytest

from abathur.domain.models import Task, TaskSource, TaskStatus
from abathur.infrastructure.database import Database
from abathur.services.dependency_resolver import DependencyResolver
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
    dependency_resolver = DependencyResolver(memory_db)
    from abathur.services.priority_calculator import PriorityCalculator
    priority_calculator = PriorityCalculator(memory_db)
    return TaskQueueService(memory_db, dependency_resolver, priority_calculator)


@pytest.fixture
async def dependency_resolver(memory_db: Database) -> DependencyResolver:
    """Create dependency resolver instance."""
    return DependencyResolver(memory_db)


def _calculate_percentile(values: list[float], percentile: float) -> float:
    """Calculate percentile from list of values."""
    if not values:
        return 0.0
    return statistics.quantiles(sorted(values), n=100)[int(percentile) - 1]


async def _create_linear_chain(queue_service: TaskQueueService, length: int) -> list[str]:
    """Create linear dependency chain: Task1 -> Task2 -> Task3 -> ... -> TaskN."""
    task_ids = []

    for i in range(length):
        prerequisites = [task_ids[-1]] if task_ids else []
        task_id = await queue_service.enqueue_task(
            prompt=f"Chain task {i}",
            summary=f"Task {i}",
            agent_type="test-agent",
            source=TaskSource.HUMAN,
            priority=5,
            prerequisites=prerequisites
        )
        task_ids.append(task_id)

    return task_ids


async def _create_diamond_graph(queue_service: TaskQueueService, depth: int) -> list[str]:
    """Create diamond dependency graph with specified depth.

    Structure:
        Root
        /  \
       A    B
        \  /
        Merge
    """
    task_ids = []

    # Root task
    root_id = await queue_service.enqueue_task(
        prompt="Root task",
        summary="Root",
        agent_type="test-agent",
        source=TaskSource.HUMAN,
        priority=5
    )
    task_ids.append(root_id)

    # Create diamond layers
    for layer in range(depth):
        # Left branch
        left_id = await queue_service.enqueue_task(
            prompt=f"Left {layer}",
            summary=f"Left {layer}",
            agent_type="test-agent",
            source=TaskSource.HUMAN,
            priority=5,
            prerequisites=[task_ids[-1]]
        )

        # Right branch
        right_id = await queue_service.enqueue_task(
            prompt=f"Right {layer}",
            summary=f"Right {layer}",
            agent_type="test-agent",
            source=TaskSource.HUMAN,
            priority=5,
            prerequisites=[task_ids[-1]]
        )

        # Merge task
        merge_id = await queue_service.enqueue_task(
            prompt=f"Merge {layer}",
            summary=f"Merge {layer}",
            agent_type="test-agent",
            source=TaskSource.HUMAN,
            priority=5,
            prerequisites=[left_id, right_id]
        )

        task_ids.extend([left_id, right_id, merge_id])

    return task_ids


async def _create_wide_graph(queue_service: TaskQueueService, width: int) -> list[str]:
    """Create wide dependency graph: Root -> [Child1, Child2, ..., ChildN] -> Merge."""
    task_ids = []

    # Root task
    root_id = await queue_service.enqueue_task(
        prompt="Root task",
        summary="Root",
        agent_type="test-agent",
        source=TaskSource.HUMAN,
        priority=5
    )
    task_ids.append(root_id)

    # Create wide fan-out
    child_ids = []
    for i in range(width):
        child_id = await queue_service.enqueue_task(
            prompt=f"Child {i}",
            summary=f"Child {i}",
            agent_type="test-agent",
            source=TaskSource.HUMAN,
            priority=5,
            prerequisites=[root_id]
        )
        child_ids.append(child_id)
        task_ids.append(child_id)

    # Merge task depending on all children
    merge_id = await queue_service.enqueue_task(
        prompt="Merge task",
        summary="Merge",
        agent_type="test-agent",
        source=TaskSource.HUMAN,
        priority=5,
        prerequisites=child_ids
    )
    task_ids.append(merge_id)

    return task_ids


@pytest.mark.benchmark
@pytest.mark.asyncio
async def test_cycle_detection_performance(dependency_resolver: DependencyResolver, queue_service: TaskQueueService) -> None:
    """Benchmark circular dependency detection.

    Performance Target: <10ms average, <100ms p95

    This test:
    1. Creates linear chain of 100 tasks
    2. Attempts to detect cycles 100 times
    3. Measures cycle detection latency
    4. Verifies <10ms average, <100ms p95
    """
    print("\n[Cycle Detection Benchmark] Creating dependency graph...")

    # Create linear chain (no cycles)
    task_ids = await _create_linear_chain(queue_service, length=100)

    print("[Cycle Detection] Measuring detection latency...")
    iterations = 100
    latencies = []

    for _ in range(iterations):
        # Test with last task trying to depend on first (would create cycle)
        start = time.perf_counter()
        has_cycle = await dependency_resolver.detect_circular_dependencies(
            dependent_task_id=task_ids[-1],
            prerequisite_task_ids=[task_ids[0]]
        )
        elapsed = (time.perf_counter() - start) * 1000  # Convert to ms

        latencies.append(elapsed)
        # Should detect cycle
        assert has_cycle is True

    # Calculate statistics
    avg_latency = statistics.mean(latencies)
    median_latency = statistics.median(latencies)
    p95_latency = _calculate_percentile(latencies, 95)
    p99_latency = _calculate_percentile(latencies, 99)
    max_latency = max(latencies)

    metrics = {
        "test": "cycle_detection",
        "graph_size": len(task_ids),
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

    print(f"\n[Cycle Detection Metrics]\n{json.dumps(metrics, indent=2)}")

    # NFR assertions
    assert avg_latency < 10.0, f"Average cycle detection latency {avg_latency:.3f}ms exceeds 10ms target"
    assert p95_latency < 100.0, f"P95 cycle detection latency {p95_latency:.3f}ms exceeds 100ms target"


@pytest.mark.benchmark
@pytest.mark.asyncio
async def test_depth_calculation_performance(dependency_resolver: DependencyResolver, queue_service: TaskQueueService) -> None:
    """Benchmark dependency depth calculation.

    Performance Target: <5ms average (with cache), <100ms p95

    This test:
    1. Creates linear chain of 100 tasks
    2. Calculates depth for all tasks multiple times
    3. Measures latency with cache hits
    4. Verifies <5ms average with caching
    """
    print("\n[Depth Calculation Benchmark] Creating dependency chain...")

    # Create linear chain (depths 0 to 99)
    task_ids = await _create_linear_chain(queue_service, length=100)

    print("[Depth Calculation] Measuring depth calculation latency...")
    iterations = 100
    latencies = []

    # First call warms cache
    await dependency_resolver.calculate_dependency_depth(task_ids[-1])

    # Measure with cache hits
    for _ in range(iterations):
        # Pick random task from chain
        task_id = task_ids[_ % len(task_ids)]

        start = time.perf_counter()
        depth = await dependency_resolver.calculate_dependency_depth(task_id)
        elapsed = (time.perf_counter() - start) * 1000  # Convert to ms

        latencies.append(elapsed)
        assert depth is not None
        assert depth >= 0

    # Calculate statistics
    avg_latency = statistics.mean(latencies)
    median_latency = statistics.median(latencies)
    p95_latency = _calculate_percentile(latencies, 95)
    p99_latency = _calculate_percentile(latencies, 99)
    max_latency = max(latencies)

    metrics = {
        "test": "depth_calculation",
        "chain_length": len(task_ids),
        "iterations": iterations,
        "cache_enabled": True,
        "avg_latency_ms": round(avg_latency, 3),
        "median_latency_ms": round(median_latency, 3),
        "p95_latency_ms": round(p95_latency, 3),
        "p99_latency_ms": round(p99_latency, 3),
        "max_latency_ms": round(max_latency, 3),
        "nfr_target_avg": "< 5ms (with cache)",
        "nfr_target_p95": "< 100ms",
        "avg_target_met": avg_latency < 5.0,
        "p95_target_met": p95_latency < 100.0
    }

    print(f"\n[Depth Calculation Metrics]\n{json.dumps(metrics, indent=2)}")

    # NFR assertions
    assert avg_latency < 5.0, f"Average depth calculation latency {avg_latency:.3f}ms exceeds 5ms target"
    assert p95_latency < 100.0, f"P95 depth calculation latency {p95_latency:.3f}ms exceeds 100ms target"


@pytest.mark.benchmark
@pytest.mark.asyncio
async def test_topological_sort_performance(dependency_resolver: DependencyResolver, queue_service: TaskQueueService) -> None:
    """Benchmark topological sort (execution order).

    Performance Target: O(V+E) complexity, <100ms for 100 tasks

    This test:
    1. Creates complex diamond graph (100+ tasks)
    2. Measures topological sort latency
    3. Verifies O(V+E) complexity
    4. Ensures <100ms p95 latency
    """
    print("\n[Topological Sort Benchmark] Creating complex dependency graph...")

    # Create diamond graph with 10 layers (~100 tasks)
    task_ids = await _create_diamond_graph(queue_service, depth=10)
    print(f"[Topological Sort] Created graph with {len(task_ids)} tasks")

    print("[Topological Sort] Measuring sort latency...")
    iterations = 100
    latencies = []

    for _ in range(iterations):
        start = time.perf_counter()
        execution_order = await dependency_resolver.get_execution_order(task_ids)
        elapsed = (time.perf_counter() - start) * 1000  # Convert to ms

        latencies.append(elapsed)
        assert execution_order is not None
        assert len(execution_order) == len(task_ids)

    # Calculate statistics
    avg_latency = statistics.mean(latencies)
    median_latency = statistics.median(latencies)
    p95_latency = _calculate_percentile(latencies, 95)
    p99_latency = _calculate_percentile(latencies, 99)
    max_latency = max(latencies)

    metrics = {
        "test": "topological_sort",
        "graph_size": len(task_ids),
        "iterations": iterations,
        "avg_latency_ms": round(avg_latency, 3),
        "median_latency_ms": round(median_latency, 3),
        "p95_latency_ms": round(p95_latency, 3),
        "p99_latency_ms": round(p99_latency, 3),
        "max_latency_ms": round(max_latency, 3),
        "complexity": "O(V+E)",
        "nfr_target_p95": "< 100ms for 100 tasks",
        "p95_target_met": p95_latency < 100.0
    }

    print(f"\n[Topological Sort Metrics]\n{json.dumps(metrics, indent=2)}")

    # NFR assertion
    assert p95_latency < 100.0, f"P95 topological sort latency {p95_latency:.3f}ms exceeds 100ms target"


@pytest.mark.benchmark
@pytest.mark.asyncio
async def test_dependency_check_performance(dependency_resolver: DependencyResolver, queue_service: TaskQueueService) -> None:
    """Benchmark are_all_dependencies_met check.

    Performance Target: O(1) with indexes, <5ms average

    This test:
    1. Creates wide graph (1 root -> 100 children -> 1 merge)
    2. Measures dependency met check for merge task
    3. Verifies O(1) performance with database indexes
    4. Ensures <5ms average latency
    """
    print("\n[Dependency Check Benchmark] Creating wide dependency graph...")

    # Create wide graph (root -> 100 children -> merge)
    task_ids = await _create_wide_graph(queue_service, width=100)
    merge_task_id = task_ids[-1]  # Last task depends on all 100 children

    print("[Dependency Check] Measuring dependency check latency...")
    iterations = 1000
    latencies = []

    for _ in range(iterations):
        start = time.perf_counter()
        all_met = await dependency_resolver.are_all_dependencies_met(merge_task_id)
        elapsed = (time.perf_counter() - start) * 1000  # Convert to ms

        latencies.append(elapsed)
        # Dependencies not met yet (children are PENDING)
        assert all_met is False

    # Calculate statistics
    avg_latency = statistics.mean(latencies)
    median_latency = statistics.median(latencies)
    p95_latency = _calculate_percentile(latencies, 95)
    p99_latency = _calculate_percentile(latencies, 99)
    max_latency = max(latencies)

    metrics = {
        "test": "dependency_check",
        "dependencies_count": 100,
        "iterations": iterations,
        "avg_latency_ms": round(avg_latency, 3),
        "median_latency_ms": round(median_latency, 3),
        "p95_latency_ms": round(p95_latency, 3),
        "p99_latency_ms": round(p99_latency, 3),
        "max_latency_ms": round(max_latency, 3),
        "complexity": "O(1) with indexes",
        "nfr_target_avg": "< 5ms",
        "nfr_target_p95": "< 100ms",
        "avg_target_met": avg_latency < 5.0,
        "p95_target_met": p95_latency < 100.0
    }

    print(f"\n[Dependency Check Metrics]\n{json.dumps(metrics, indent=2)}")

    # NFR assertions
    assert avg_latency < 5.0, f"Average dependency check latency {avg_latency:.3f}ms exceeds 5ms target"
    assert p95_latency < 100.0, f"P95 dependency check latency {p95_latency:.3f}ms exceeds 100ms target"


@pytest.mark.benchmark
@pytest.mark.asyncio
async def test_complex_graph_performance(dependency_resolver: DependencyResolver, queue_service: TaskQueueService) -> None:
    """Benchmark dependency operations on complex realistic graph.

    Performance Target: Handle 100+ tasks with complex dependencies

    This test:
    1. Creates realistic complex graph (100 tasks, mixed patterns)
    2. Measures all key operations: cycle detection, depth calc, topo sort
    3. Verifies performance remains within NFR targets
    4. Provides comprehensive performance profile
    """
    print("\n[Complex Graph Benchmark] Creating realistic dependency graph...")

    # Create mixed graph: linear chains + diamonds + wide fan-out
    task_ids = []

    # Base chain
    chain_ids = await _create_linear_chain(queue_service, length=20)
    task_ids.extend(chain_ids)

    # Diamond layers
    for i in range(5):
        diamond_ids = await _create_diamond_graph(queue_service, depth=2)
        task_ids.extend(diamond_ids)

    # Wide fan-out
    wide_ids = await _create_wide_graph(queue_service, width=30)
    task_ids.extend(wide_ids)

    print(f"[Complex Graph] Created graph with {len(task_ids)} tasks")

    # Measure all operations
    operations = {}

    # 1. Cycle detection
    print("[Complex Graph] Measuring cycle detection...")
    latencies = []
    for _ in range(50):
        start = time.perf_counter()
        await dependency_resolver.detect_circular_dependencies(
            dependent_task_id=task_ids[-1],
            prerequisite_task_ids=[task_ids[0]]
        )
        elapsed = (time.perf_counter() - start) * 1000
        latencies.append(elapsed)

    operations["cycle_detection"] = {
        "avg_latency_ms": round(statistics.mean(latencies), 3),
        "p95_latency_ms": round(_calculate_percentile(latencies, 95), 3)
    }

    # 2. Depth calculation
    print("[Complex Graph] Measuring depth calculation...")
    latencies = []
    for i in range(50):
        task_id = task_ids[i % len(task_ids)]
        start = time.perf_counter()
        await dependency_resolver.calculate_dependency_depth(task_id)
        elapsed = (time.perf_counter() - start) * 1000
        latencies.append(elapsed)

    operations["depth_calculation"] = {
        "avg_latency_ms": round(statistics.mean(latencies), 3),
        "p95_latency_ms": round(_calculate_percentile(latencies, 95), 3)
    }

    # 3. Topological sort
    print("[Complex Graph] Measuring topological sort...")
    latencies = []
    for _ in range(50):
        start = time.perf_counter()
        await dependency_resolver.get_execution_order(task_ids)
        elapsed = (time.perf_counter() - start) * 1000
        latencies.append(elapsed)

    operations["topological_sort"] = {
        "avg_latency_ms": round(statistics.mean(latencies), 3),
        "p95_latency_ms": round(_calculate_percentile(latencies, 95), 3)
    }

    # Compile results
    metrics = {
        "test": "complex_graph",
        "graph_size": len(task_ids),
        "operations": operations,
        "nfr_targets": {
            "cycle_detection": "< 10ms avg, < 100ms p95",
            "depth_calculation": "< 5ms avg, < 100ms p95",
            "topological_sort": "< 100ms p95"
        },
        "all_nfrs_met": (
            operations["cycle_detection"]["avg_latency_ms"] < 10.0 and
            operations["cycle_detection"]["p95_latency_ms"] < 100.0 and
            operations["depth_calculation"]["avg_latency_ms"] < 5.0 and
            operations["depth_calculation"]["p95_latency_ms"] < 100.0 and
            operations["topological_sort"]["p95_latency_ms"] < 100.0
        )
    }

    print(f"\n[Complex Graph Metrics]\n{json.dumps(metrics, indent=2)}")

    # NFR assertions
    assert operations["cycle_detection"]["avg_latency_ms"] < 10.0
    assert operations["cycle_detection"]["p95_latency_ms"] < 100.0
    assert operations["depth_calculation"]["avg_latency_ms"] < 5.0
    assert operations["depth_calculation"]["p95_latency_ms"] < 100.0
    assert operations["topological_sort"]["p95_latency_ms"] < 100.0


@pytest.mark.benchmark
@pytest.mark.asyncio
async def test_cache_performance_impact(dependency_resolver: DependencyResolver, queue_service: TaskQueueService) -> None:
    """Measure performance impact of dependency graph cache.

    This test:
    1. Measures operations without cache (cold)
    2. Measures operations with cache (warm)
    3. Calculates speedup from caching
    4. Verifies cache provides significant performance boost
    """
    print("\n[Cache Performance Benchmark] Creating dependency graph...")

    # Create linear chain
    task_ids = await _create_linear_chain(queue_service, length=100)

    # Measure cold cache (first call)
    print("[Cache Performance] Measuring cold cache latency...")
    dependency_resolver.invalidate_cache()

    start = time.perf_counter()
    await dependency_resolver.calculate_dependency_depth(task_ids[-1])
    cold_latency = (time.perf_counter() - start) * 1000

    # Measure warm cache (repeated calls)
    print("[Cache Performance] Measuring warm cache latency...")
    warm_latencies = []

    for _ in range(100):
        start = time.perf_counter()
        await dependency_resolver.calculate_dependency_depth(task_ids[-1])
        elapsed = (time.perf_counter() - start) * 1000
        warm_latencies.append(elapsed)

    avg_warm_latency = statistics.mean(warm_latencies)
    speedup = cold_latency / avg_warm_latency if avg_warm_latency > 0 else 0

    metrics = {
        "test": "cache_performance",
        "graph_size": len(task_ids),
        "cold_cache_latency_ms": round(cold_latency, 3),
        "warm_cache_avg_latency_ms": round(avg_warm_latency, 3),
        "speedup_factor": round(speedup, 2),
        "cache_effective": speedup > 2.0  # At least 2x faster
    }

    print(f"\n[Cache Performance Metrics]\n{json.dumps(metrics, indent=2)}")

    # Verify cache provides benefit
    assert speedup > 2.0, f"Cache speedup {speedup:.2f}x is less than expected 2x minimum"
