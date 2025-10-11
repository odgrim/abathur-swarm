"""Performance tests for DependencyResolver service.

Tests validate that all algorithms meet performance targets:
- Cycle detection: <10ms for 100-task graph
- Topological sort: <10ms for 100-task graph
- Depth calculation: <5ms for 10-level graph
- Cache hit: <1ms
- Validate dependency: <10ms for large graph
"""

import json
import time
from datetime import datetime, timezone
from pathlib import Path

import pytest
from abathur.domain.models import DependencyType, Task, TaskDependency, TaskStatus
from abathur.infrastructure.database import Database
from abathur.services.dependency_resolver import DependencyResolver


@pytest.fixture
async def db():
    """Create an in-memory database for performance testing."""
    database = Database(db_path=Path(":memory:"))
    await database.initialize()
    yield database
    await database.close()


@pytest.fixture
async def resolver(db):
    """Create a DependencyResolver instance."""
    return DependencyResolver(db, cache_ttl_seconds=60.0)


def measure_time(func):
    """Decorator to measure execution time in milliseconds."""

    async def wrapper(*args, **kwargs):
        start = time.perf_counter()
        result = await func(*args, **kwargs)
        end = time.perf_counter()
        elapsed_ms = (end - start) * 1000
        return result, elapsed_ms

    return wrapper


@pytest.mark.asyncio
class TestCycleDetectionPerformance:
    """Performance tests for circular dependency detection."""

    async def test_detect_cycles_100_task_graph(self, db, resolver):
        """Test cycle detection performance on 100-task graph.

        Target: <10ms for 100-task graph
        """
        # Create 100 tasks in a complex graph structure
        tasks = []
        for i in range(100):
            task = Task(prompt=f"Task {i}", status=TaskStatus.PENDING)
            await db.insert_task(task)
            tasks.append(task)

        # Create dependencies: linear chain with some branches
        # Pattern: 0->1->2->3... with some cross-links
        for i in range(99):
            dep = TaskDependency(
                dependent_task_id=tasks[i + 1].id,
                prerequisite_task_id=tasks[i].id,
                dependency_type=DependencyType.SEQUENTIAL,
            )
            await db.insert_task_dependency(dep)

        # Add some branch dependencies
        for i in range(0, 90, 10):
            if i + 5 < 100:
                dep = TaskDependency(
                    dependent_task_id=tasks[i + 5].id,
                    prerequisite_task_id=tasks[i].id,
                    dependency_type=DependencyType.PARALLEL,
                )
                await db.insert_task_dependency(dep)

        # Measure cycle detection time (checking if adding edge would create cycle)
        @measure_time
        async def detect_cycle():
            try:
                # This should NOT create a cycle (99 -> 50 is backward but not cyclic)
                await resolver.detect_circular_dependencies([tasks[50].id], tasks[99].id)
                return True
            except Exception:
                return False

        result, elapsed_ms = await detect_cycle()

        # Validate performance
        assert elapsed_ms < 10, f"Cycle detection took {elapsed_ms:.2f}ms, target is <10ms"

        print(f"\n✓ Cycle detection (100 tasks): {elapsed_ms:.2f}ms")

    async def test_detect_cycles_complex_graph_200_edges(self, db, resolver):
        """Test cycle detection with 100 tasks and 200 edges.

        Target: <10ms
        """
        # Create 100 tasks
        tasks = []
        for i in range(100):
            task = Task(prompt=f"Task {i}", status=TaskStatus.PENDING)
            await db.insert_task(task)
            tasks.append(task)

        # Create 200 random dependencies (ensure no cycles)
        import random

        random.seed(42)  # Reproducible

        for _ in range(200):
            # Pick random tasks ensuring prerequisite comes before dependent
            i = random.randint(0, 98)
            j = random.randint(i + 1, 99)

            dep = TaskDependency(
                dependent_task_id=tasks[j].id,
                prerequisite_task_id=tasks[i].id,
                dependency_type=DependencyType.SEQUENTIAL,
            )
            try:
                await db.insert_task_dependency(dep)
            except Exception:
                # Skip duplicates
                pass

        # Measure cycle detection
        @measure_time
        async def detect_cycle():
            try:
                await resolver.detect_circular_dependencies([tasks[0].id], tasks[99].id)
                return True
            except Exception:
                return False

        result, elapsed_ms = await detect_cycle()

        assert elapsed_ms < 10, f"Cycle detection took {elapsed_ms:.2f}ms, target is <10ms"

        print(f"✓ Cycle detection (100 tasks, 200 edges): {elapsed_ms:.2f}ms")


@pytest.mark.asyncio
class TestTopologicalSortPerformance:
    """Performance tests for topological sorting."""

    async def test_topological_sort_100_task_graph(self, db, resolver):
        """Test topological sort performance on 100-task graph.

        Target: <10ms for 100-task graph
        """
        # Create 100 tasks
        tasks = []
        for i in range(100):
            task = Task(prompt=f"Task {i}", status=TaskStatus.PENDING)
            await db.insert_task(task)
            tasks.append(task)

        # Create linear dependencies with branches
        for i in range(99):
            dep = TaskDependency(
                dependent_task_id=tasks[i + 1].id,
                prerequisite_task_id=tasks[i].id,
                dependency_type=DependencyType.SEQUENTIAL,
            )
            await db.insert_task_dependency(dep)

        # Add cross-links
        for i in range(0, 90, 10):
            if i + 5 < 100:
                dep = TaskDependency(
                    dependent_task_id=tasks[i + 5].id,
                    prerequisite_task_id=tasks[i].id,
                    dependency_type=DependencyType.PARALLEL,
                )
                await db.insert_task_dependency(dep)

        # Measure topological sort time
        @measure_time
        async def topo_sort():
            task_ids = [t.id for t in tasks]
            return await resolver.get_execution_order(task_ids)

        result, elapsed_ms = await topo_sort()

        # Validate performance
        assert elapsed_ms < 10, f"Topological sort took {elapsed_ms:.2f}ms, target is <10ms"
        assert len(result) == 100

        print(f"✓ Topological sort (100 tasks): {elapsed_ms:.2f}ms")

    async def test_topological_sort_1000_tasks(self, db, resolver):
        """Test topological sort scalability with 1000 tasks.

        Target: <50ms for 1000-task graph
        """
        # Create 1000 tasks
        tasks = []
        for i in range(1000):
            task = Task(prompt=f"Task {i}", status=TaskStatus.PENDING)
            await db.insert_task(task)
            tasks.append(task)

        # Create linear dependencies
        for i in range(999):
            dep = TaskDependency(
                dependent_task_id=tasks[i + 1].id,
                prerequisite_task_id=tasks[i].id,
                dependency_type=DependencyType.SEQUENTIAL,
            )
            await db.insert_task_dependency(dep)

        # Measure topological sort time
        @measure_time
        async def topo_sort():
            task_ids = [t.id for t in tasks]
            return await resolver.get_execution_order(task_ids)

        result, elapsed_ms = await topo_sort()

        # Validate performance
        assert elapsed_ms < 50, f"Topological sort took {elapsed_ms:.2f}ms, target is <50ms"
        assert len(result) == 1000

        print(f"✓ Topological sort (1000 tasks): {elapsed_ms:.2f}ms")


@pytest.mark.asyncio
class TestDepthCalculationPerformance:
    """Performance tests for dependency depth calculation."""

    async def test_dependency_depth_10_levels(self, db, resolver):
        """Test depth calculation performance for 10-level deep graph.

        Target: <5ms for 10-level graph
        """
        # Create 10-level linear dependency chain
        tasks = []
        for i in range(10):
            task = Task(prompt=f"Task {i}", status=TaskStatus.PENDING)
            await db.insert_task(task)
            tasks.append(task)

        # Create linear dependencies: 0 -> 1 -> 2 -> ... -> 9
        for i in range(9):
            dep = TaskDependency(
                dependent_task_id=tasks[i + 1].id,
                prerequisite_task_id=tasks[i].id,
                dependency_type=DependencyType.SEQUENTIAL,
            )
            await db.insert_task_dependency(dep)

        # Measure depth calculation for deepest task
        @measure_time
        async def calc_depth():
            return await resolver.calculate_dependency_depth(tasks[-1].id)

        depth, elapsed_ms = await calc_depth()

        # Validate performance and correctness
        assert elapsed_ms < 5, f"Depth calculation took {elapsed_ms:.2f}ms, target is <5ms"
        assert depth == 9  # Last task is at depth 9

        print(f"✓ Depth calculation (10 levels): {elapsed_ms:.2f}ms")

    async def test_dependency_depth_with_memoization(self, db, resolver):
        """Test that memoization improves depth calculation performance."""
        # Create 10-level dependency chain
        tasks = []
        for i in range(10):
            task = Task(prompt=f"Task {i}", status=TaskStatus.PENDING)
            await db.insert_task(task)
            tasks.append(task)

        for i in range(9):
            dep = TaskDependency(
                dependent_task_id=tasks[i + 1].id,
                prerequisite_task_id=tasks[i].id,
                dependency_type=DependencyType.SEQUENTIAL,
            )
            await db.insert_task_dependency(dep)

        # First calculation (cache miss)
        @measure_time
        async def first_calc():
            return await resolver.calculate_dependency_depth(tasks[-1].id)

        depth1, time1 = await first_calc()

        # Second calculation (cache hit)
        @measure_time
        async def second_calc():
            return await resolver.calculate_dependency_depth(tasks[-1].id)

        depth2, time2 = await second_calc()

        # Cache hit should be much faster
        assert time2 < time1, "Memoized calculation should be faster"
        assert depth1 == depth2 == 9

        print(f"✓ Depth memoization: {time1:.3f}ms → {time2:.3f}ms (cached)")


@pytest.mark.asyncio
class TestGraphCachePerformance:
    """Performance tests for graph caching."""

    async def test_graph_cache_hit_performance(self, db, resolver):
        """Test graph cache hit performance.

        Target: <1ms for cache hit
        """
        # Create some tasks and dependencies
        tasks = []
        for i in range(50):
            task = Task(prompt=f"Task {i}", status=TaskStatus.PENDING)
            await db.insert_task(task)
            tasks.append(task)

        for i in range(49):
            dep = TaskDependency(
                dependent_task_id=tasks[i + 1].id,
                prerequisite_task_id=tasks[i].id,
                dependency_type=DependencyType.SEQUENTIAL,
            )
            await db.insert_task_dependency(dep)

        # First build (cache miss)
        @measure_time
        async def first_build():
            return await resolver._build_dependency_graph()

        graph1, time1 = await first_build()

        # Second build (cache hit)
        @measure_time
        async def second_build():
            return await resolver._build_dependency_graph()

        graph2, time2 = await second_build()

        # Cache hit should be <1ms
        assert time2 < 1.0, f"Cache hit took {time2:.2f}ms, target is <1ms"
        assert graph1 == graph2

        print(f"✓ Graph cache: {time1:.2f}ms (miss) → {time2:.3f}ms (hit)")

    async def test_graph_build_1000_tasks(self, db, resolver):
        """Test graph building performance for 1000 tasks.

        Target: <50ms for 1000-task database
        """
        # Create 1000 tasks with dependencies
        tasks = []
        for i in range(1000):
            task = Task(prompt=f"Task {i}", status=TaskStatus.PENDING)
            await db.insert_task(task)
            tasks.append(task)

        # Create dependencies
        for i in range(999):
            dep = TaskDependency(
                dependent_task_id=tasks[i + 1].id,
                prerequisite_task_id=tasks[i].id,
                dependency_type=DependencyType.SEQUENTIAL,
            )
            await db.insert_task_dependency(dep)

        # Measure graph building time
        @measure_time
        async def build_graph():
            return await resolver._build_dependency_graph()

        graph, elapsed_ms = await build_graph()

        # Validate performance
        assert elapsed_ms < 50, f"Graph building took {elapsed_ms:.2f}ms, target is <50ms"

        print(f"✓ Graph building (1000 tasks): {elapsed_ms:.2f}ms")


@pytest.mark.asyncio
class TestValidationPerformance:
    """Performance tests for dependency validation."""

    async def test_validate_dependency_large_graph(self, db, resolver):
        """Test validation performance on large graph.

        Target: <10ms for validation on 100-task graph
        """
        # Create 100 tasks
        tasks = []
        for i in range(100):
            task = Task(prompt=f"Task {i}", status=TaskStatus.PENDING)
            await db.insert_task(task)
            tasks.append(task)

        # Create complex dependencies
        for i in range(99):
            dep = TaskDependency(
                dependent_task_id=tasks[i + 1].id,
                prerequisite_task_id=tasks[i].id,
                dependency_type=DependencyType.SEQUENTIAL,
            )
            await db.insert_task_dependency(dep)

        # Measure validation time
        @measure_time
        async def validate():
            return await resolver.validate_new_dependency(tasks[99].id, tasks[50].id)

        is_valid, elapsed_ms = await validate()

        # Validate performance
        assert elapsed_ms < 10, f"Validation took {elapsed_ms:.2f}ms, target is <10ms"

        print(f"✓ Dependency validation (100 tasks): {elapsed_ms:.2f}ms")


@pytest.mark.asyncio
class TestPerformanceBenchmarkReport:
    """Generate comprehensive performance benchmark report."""

    async def test_generate_performance_report(self, db, resolver):
        """Generate comprehensive performance benchmark report."""

        results = {
            "test_suite": "DependencyResolver Performance Benchmarks",
            "date": datetime.now(timezone.utc).isoformat(),
            "benchmarks": [],
        }

        # Benchmark 1: Cycle detection (100 tasks)
        tasks_100 = []
        for i in range(100):
            task = Task(prompt=f"Task {i}", status=TaskStatus.PENDING)
            await db.insert_task(task)
            tasks_100.append(task)

        for i in range(99):
            dep = TaskDependency(
                dependent_task_id=tasks_100[i + 1].id,
                prerequisite_task_id=tasks_100[i].id,
                dependency_type=DependencyType.SEQUENTIAL,
            )
            await db.insert_task_dependency(dep)

        @measure_time
        async def cycle_detection():
            try:
                await resolver.detect_circular_dependencies([tasks_100[50].id], tasks_100[99].id)
                return True
            except Exception:
                return False

        _, cycle_time = await cycle_detection()

        results["benchmarks"].append(
            {
                "name": "Cycle Detection (100 tasks)",
                "target_ms": 10,
                "actual_ms": round(cycle_time, 3),
                "passed": cycle_time < 10,
            }
        )

        # Benchmark 2: Topological sort (100 tasks)
        @measure_time
        async def topo_sort():
            return await resolver.get_execution_order([t.id for t in tasks_100])

        _, topo_time = await topo_sort()

        results["benchmarks"].append(
            {
                "name": "Topological Sort (100 tasks)",
                "target_ms": 15,
                "actual_ms": round(topo_time, 3),
                "passed": topo_time < 15,
            }
        )

        # Benchmark 3: Depth calculation (10 levels)
        tasks_10 = []
        for i in range(10):
            task = Task(prompt=f"Depth {i}", status=TaskStatus.PENDING)
            await db.insert_task(task)
            tasks_10.append(task)

        for i in range(9):
            dep = TaskDependency(
                dependent_task_id=tasks_10[i + 1].id,
                prerequisite_task_id=tasks_10[i].id,
                dependency_type=DependencyType.SEQUENTIAL,
            )
            await db.insert_task_dependency(dep)

        @measure_time
        async def depth_calc():
            return await resolver.calculate_dependency_depth(tasks_10[-1].id)

        _, depth_time = await depth_calc()

        results["benchmarks"].append(
            {
                "name": "Depth Calculation (10 levels)",
                "target_ms": 5,
                "actual_ms": round(depth_time, 3),
                "passed": depth_time < 5,
            }
        )

        # Benchmark 4: Cache hit performance
        await resolver._build_dependency_graph()  # Prime cache

        @measure_time
        async def cache_hit():
            return await resolver._build_dependency_graph()

        _, cache_time = await cache_hit()

        results["benchmarks"].append(
            {
                "name": "Graph Cache Hit",
                "target_ms": 1,
                "actual_ms": round(cache_time, 3),
                "passed": cache_time < 1,
            }
        )

        # Summary
        all_passed = all(b["passed"] for b in results["benchmarks"])
        results["summary"] = {
            "total_benchmarks": len(results["benchmarks"]),
            "passed": sum(1 for b in results["benchmarks"] if b["passed"]),
            "failed": sum(1 for b in results["benchmarks"] if not b["passed"]),
            "all_targets_met": all_passed,
        }

        # Save to file
        report_path = Path(
            "/Users/odgrim/dev/home/agentics/abathur/design_docs/PHASE2_PERFORMANCE_BENCHMARKS.json"
        )
        report_path.write_text(json.dumps(results, indent=2))

        # Print report
        print("\n" + "=" * 60)
        print("DEPENDENCY RESOLVER PERFORMANCE BENCHMARK REPORT")
        print("=" * 60)
        for benchmark in results["benchmarks"]:
            status = "✓ PASS" if benchmark["passed"] else "✗ FAIL"
            print(
                f"{status} | {benchmark['name']:40} | {benchmark['actual_ms']:6.2f}ms (target: <{benchmark['target_ms']}ms)"
            )
        print("=" * 60)
        print(
            f"Summary: {results['summary']['passed']}/{results['summary']['total_benchmarks']} benchmarks passed"
        )
        print(f"Report saved to: {report_path}")
        print("=" * 60)

        # Assert all benchmarks passed
        assert all_passed, "Some performance benchmarks failed to meet targets"
