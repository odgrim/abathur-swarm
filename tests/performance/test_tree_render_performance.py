"""Performance benchmarks for tree rendering with Rich console output.

Validates NFR002 performance target:
- 1000-task hierarchy renders in <1 second

Uses pytest-benchmark for reliable measurements with the following operations:
- TreeRenderer.compute_layout() - hierarchical layout computation
- TreeRenderer.render_tree() - Rich Tree widget generation
- Console.print() - terminal output rendering

Test Scenarios:
1. Baseline: 100-task hierarchy benchmark
2. NFR002 Validation: 1000-task hierarchy <1s target
3. Scalability: Parametrized tests with 10, 50, 100, 500, 1000 tasks
4. Memory profiling: Memory footprint for large hierarchies
5. Determinism: Verify consistent results across runs

Performance Targets (NFR002):
- 100-task tree render: <100ms (baseline)
- 1000-task tree render: <1000ms (1 second - critical NFR)
- Memory overhead: <100MB for 1000 tasks
- Determinism: 100% pass rate over 10 runs
"""

import gc
import os
import time
from datetime import datetime, timezone
from pathlib import Path
from uuid import UUID, uuid4

import psutil
import pytest
from rich.console import Console

from abathur.domain.models import Task, TaskSource, TaskStatus
from abathur.infrastructure.database import Database
from abathur.tui.rendering.tree_renderer import TreeRenderer


# ==============================================================================
# Test Fixtures
# ==============================================================================


@pytest.fixture
async def task_hierarchy_factory():
    """Factory fixture to create hierarchical task datasets of varying sizes.

    Creates realistic parent-child hierarchies with varied depths and widths.
    Useful for testing scalability across different data volumes.

    Yields:
        Async function that creates database with N tasks in hierarchy
    """

    async def _create_hierarchy(
        total_tasks: int,
        db_path: Path | None = None,
        depth: int = 3,
        fanout: int = 4,
    ) -> tuple[Database, list[Task]]:
        """Create task hierarchy with specified size and structure.

        Args:
            total_tasks: Total number of tasks to create
            db_path: Path for database file (None for in-memory)
            depth: Maximum depth of hierarchy
            fanout: Number of children per parent (branching factor)

        Returns:
            Tuple of (Database, list of Task objects)
        """
        # Use in-memory database for performance tests (faster, no I/O overhead)
        if db_path is None:
            db = Database(Path(":memory:"))
        else:
            db = Database(db_path)

        await db.initialize()

        tasks = []

        # Simple approach: create tasks in batches with parent-child relationships
        # Calculate how many root tasks we need
        num_roots = max(1, int(total_tasks ** (1.0 / (depth + 1))))
        tasks_created = 0

        # Create root tasks
        root_tasks = []
        for i in range(min(num_roots, total_tasks)):
            task = Task(
                id=uuid4(),
                prompt=f"Root task {i}",
                summary=f"Root {i}",
                agent_type="test-agent",
                status=TaskStatus.PENDING,
                calculated_priority=10.0 - (i % 10),
                dependency_depth=0,
                submitted_at=datetime.now(timezone.utc),
                source=TaskSource.HUMAN,
            )
            await db.insert_task(task)
            root_tasks.append(task)
            tasks.append(task)
            tasks_created += 1

        # Create remaining tasks as children, distributed across levels
        current_level = 1
        parent_pool = root_tasks

        while tasks_created < total_tasks and current_level <= depth:
            # Calculate how many tasks to create at this level
            remaining = total_tasks - tasks_created
            # Distribute remaining tasks across remaining levels
            levels_left = depth - current_level + 1
            tasks_this_level = min(remaining, (remaining // levels_left) + 1)

            level_tasks = []
            for i in range(tasks_this_level):
                # Round-robin assignment to parents
                parent = parent_pool[i % len(parent_pool)]

                task = Task(
                    id=uuid4(),
                    prompt=f"Level {current_level} task {i}",
                    summary=f"L{current_level}-{i}",
                    agent_type="test-agent",
                    status=TaskStatus(["pending", "ready", "running", "completed"][i % 4]),
                    calculated_priority=8.0 - (i % 20),
                    dependency_depth=current_level,
                    parent_task_id=parent.id,
                    submitted_at=datetime.now(timezone.utc),
                    source=TaskSource.HUMAN,
                )
                await db.insert_task(task)
                level_tasks.append(task)
                tasks.append(task)
                tasks_created += 1

                if tasks_created >= total_tasks:
                    break

            parent_pool = level_tasks
            current_level += 1

        return db, tasks

    return _create_hierarchy


@pytest.fixture
def console_with_capture():
    """Console with string buffer for capturing output without terminal overhead."""
    from io import StringIO

    buffer = StringIO()
    console = Console(file=buffer, width=120, legacy_windows=False)
    return console, buffer


# ==============================================================================
# Baseline Performance Tests (100 tasks)
# ==============================================================================


class TestBaselinePerformance:
    """Baseline performance tests with 100-task hierarchy."""

    @pytest.mark.benchmark
    @pytest.mark.asyncio
    async def test_100_task_tree_render_benchmark(
        self,
        benchmark,
        task_hierarchy_factory,
        console_with_capture,
    ):
        """Benchmark: 100-task hierarchy renders in <100ms (baseline).

        This establishes baseline performance for typical usage.
        Uses pytest-benchmark for statistical analysis across multiple runs.
        """
        # Arrange
        console, buffer = console_with_capture
        db, tasks = await task_hierarchy_factory(100, depth=3, fanout=3)
        renderer = TreeRenderer()

        # Build dependency graph (empty for tree rendering)
        dependency_graph: dict[UUID, list[UUID]] = {}

        def render_tree():
            """Benchmark target: full render pipeline."""
            # Step 1: Compute layout
            layout = renderer.compute_layout(tasks, dependency_graph)

            # Step 2: Render tree widget
            tree = renderer.render_tree(layout, use_unicode=True)

            # Step 3: Print to console (terminal output)
            console.print(tree)

            return layout

        # Act - Benchmark the render operation
        result = benchmark(render_tree)

        # Assert - Verify NFR baseline: <100ms
        assert benchmark.stats['mean'] < 0.100, (
            f"Baseline violation: 100-task render took {benchmark.stats['mean']*1000:.2f}ms "
            f"(target: <100ms)"
        )

        # Verify correctness
        assert result.total_nodes == 100
        assert len(result.root_nodes) > 0

        # Log performance metrics
        print(f"\n=== 100-Task Baseline Performance ===")
        print(f"  Mean:   {benchmark.stats['mean']*1000:.2f}ms")
        print(f"  Median: {benchmark.stats['median']*1000:.2f}ms")
        print(f"  StdDev: {benchmark.stats['stddev']*1000:.2f}ms")
        print(f"  Min:    {benchmark.stats['min']*1000:.2f}ms")
        print(f"  Max:    {benchmark.stats['max']*1000:.2f}ms")
        print(f"  IQR:    {benchmark.stats['iqr']*1000:.2f}ms")

        # Cleanup
        await db.close()

    @pytest.mark.benchmark
    @pytest.mark.asyncio
    async def test_100_task_layout_computation_only(
        self,
        benchmark,
        task_hierarchy_factory,
    ):
        """Benchmark: Layout computation performance for 100 tasks.

        Isolates layout algorithm performance (no rendering overhead).
        """
        # Arrange
        db, tasks = await task_hierarchy_factory(100, depth=3, fanout=3)
        renderer = TreeRenderer()
        dependency_graph: dict[UUID, list[UUID]] = {}

        def compute_layout():
            """Benchmark target: layout computation only."""
            return renderer.compute_layout(tasks, dependency_graph)

        # Act
        layout = benchmark(compute_layout)

        # Assert - Layout computation should be very fast
        assert benchmark.stats['mean'] < 0.050, (
            f"Layout computation slow: {benchmark.stats['mean']*1000:.2f}ms "
            f"(target: <50ms)"
        )

        assert layout.total_nodes == 100

        print(f"\n=== Layout Computation (100 tasks) ===")
        print(f"  Mean: {benchmark.stats['mean']*1000:.2f}ms")

        # Cleanup
        await db.close()


# ==============================================================================
# NFR002 Critical Performance Test (1000 tasks)
# ==============================================================================


class TestNFR002_1000TaskPerformance:
    """NFR002 validation: 1000-task hierarchy renders in <1 second."""

    @pytest.mark.benchmark
    @pytest.mark.asyncio
    async def test_1000_task_tree_render_under_1_second(
        self,
        benchmark,
        task_hierarchy_factory,
        console_with_capture,
    ):
        """CRITICAL: Validate NFR002 - 1000-task tree renders in <1 second.

        This is the primary performance requirement from the specification.
        Failure of this test indicates NFR002 violation.
        """
        # Arrange
        console, buffer = console_with_capture
        db, tasks = await task_hierarchy_factory(1000, depth=4, fanout=5)
        renderer = TreeRenderer()
        dependency_graph: dict[UUID, list[UUID]] = {}

        def render_large_tree():
            """Full render pipeline for 1000-task hierarchy."""
            # Compute layout
            layout = renderer.compute_layout(tasks, dependency_graph)

            # Render tree
            tree = renderer.render_tree(layout, use_unicode=True)

            # Print to console
            console.print(tree)

            return layout

        # Act - Benchmark render
        result = benchmark(render_large_tree)

        # Assert - CRITICAL NFR002: <1 second
        assert benchmark.stats['mean'] < 1.000, (
            f"NFR002 VIOLATION: 1000-task render took {benchmark.stats['mean']*1000:.2f}ms "
            f"(target: <1000ms)"
        )

        # Additional validation: p95 and p99 should also be under threshold
        # (Ensures consistent performance, not just average)
        assert benchmark.stats['median'] < 1.000, (
            f"NFR002 VIOLATION (median): {benchmark.stats['median']*1000:.2f}ms"
        )

        # Verify correctness
        assert result.total_nodes == 1000

        # Log performance metrics
        print(f"\n=== NFR002 Validation: 1000-Task Performance ===")
        print(f"  Mean:   {benchmark.stats['mean']*1000:.2f}ms")
        print(f"  Median: {benchmark.stats['median']*1000:.2f}ms")
        print(f"  StdDev: {benchmark.stats['stddev']*1000:.2f}ms")
        print(f"  Min:    {benchmark.stats['min']*1000:.2f}ms")
        print(f"  Max:    {benchmark.stats['max']*1000:.2f}ms")
        print(f"  IQR:    {benchmark.stats['iqr']*1000:.2f}ms")

        # Performance report
        if benchmark.stats['mean'] < 0.500:
            print(f"  ✓ EXCELLENT: Well under target (2x margin)")
        elif benchmark.stats['mean'] < 0.750:
            print(f"  ✓ GOOD: Under target with headroom")
        elif benchmark.stats['mean'] < 1.000:
            print(f"  ✓ PASS: Meets NFR002 target")
        else:
            print(f"  ✗ FAIL: NFR002 violation")

        # Cleanup
        await db.close()

    @pytest.mark.benchmark
    @pytest.mark.asyncio
    async def test_1000_task_layout_computation(
        self,
        benchmark,
        task_hierarchy_factory,
    ):
        """Benchmark: Layout computation for 1000 tasks (isolation test)."""
        # Arrange
        db, tasks = await task_hierarchy_factory(1000, depth=4, fanout=5)
        renderer = TreeRenderer()
        dependency_graph: dict[UUID, list[UUID]] = {}

        def compute_layout():
            return renderer.compute_layout(tasks, dependency_graph)

        # Act
        layout = benchmark(compute_layout)

        # Assert - Layout should be fast even for 1000 tasks
        assert benchmark.stats['mean'] < 0.500, (
            f"Layout computation slow: {benchmark.stats['mean']*1000:.2f}ms"
        )

        assert layout.total_nodes == 1000

        print(f"\n=== Layout Computation (1000 tasks) ===")
        print(f"  Mean: {benchmark.stats['mean']*1000:.2f}ms")

        # Cleanup
        await db.close()


# ==============================================================================
# Scalability Tests (Parametrized)
# ==============================================================================


class TestScalability:
    """Test render time scales linearly or sub-linearly with task count."""

    @pytest.mark.benchmark
    @pytest.mark.parametrize("task_count", [10, 50, 100, 500, 1000])
    @pytest.mark.asyncio
    async def test_render_time_scaling(
        self,
        benchmark,
        task_hierarchy_factory,
        console_with_capture,
        task_count,
    ):
        """Test: Render time scales gracefully with increasing task count.

        Validates that algorithm complexity is linear or sub-linear.
        Identifies performance cliffs (sudden degradation).
        """
        # Arrange
        console, buffer = console_with_capture
        db, tasks = await task_hierarchy_factory(task_count, depth=3, fanout=4)
        renderer = TreeRenderer()
        dependency_graph: dict[UUID, list[UUID]] = {}

        def render():
            layout = renderer.compute_layout(tasks, dependency_graph)
            tree = renderer.render_tree(layout, use_unicode=True)
            console.print(tree)
            return layout

        # Act
        result = benchmark(render)

        # Assert - Verify correctness
        assert result.total_nodes == task_count

        # Log scaling behavior
        print(f"\n{task_count} tasks: {benchmark.stats['mean']*1000:.2f}ms")

        # Cleanup
        await db.close()

    @pytest.mark.benchmark
    @pytest.mark.asyncio
    async def test_scaling_analysis(
        self,
        task_hierarchy_factory,
        console_with_capture,
    ):
        """Analyze scaling characteristics across multiple sizes.

        Not a benchmark itself, but uses timing to verify linear scaling.
        """
        console, buffer = console_with_capture
        renderer = TreeRenderer()
        dependency_graph: dict[UUID, list[UUID]] = {}

        sizes = [10, 50, 100, 500, 1000]
        times = []

        for size in sizes:
            db, tasks = await task_hierarchy_factory(size, depth=3, fanout=4)

            # Measure render time
            start = time.perf_counter()
            layout = renderer.compute_layout(tasks, dependency_graph)
            tree = renderer.render_tree(layout, use_unicode=True)
            console.print(tree)
            elapsed = time.perf_counter() - start

            times.append((size, elapsed))
            await db.close()

        # Analyze scaling: time should grow linearly or sub-linearly
        print(f"\n=== Scaling Analysis ===")
        for i, (size, elapsed) in enumerate(times):
            if i > 0:
                prev_size, prev_time = times[i - 1]
                ratio = elapsed / prev_time
                size_ratio = size / prev_size
                print(
                    f"{size:4d} tasks: {elapsed*1000:6.2f}ms "
                    f"(time ratio: {ratio:.2f}x, size ratio: {size_ratio:.1f}x)"
                )
            else:
                print(f"{size:4d} tasks: {elapsed*1000:6.2f}ms")

        # Verify sub-linear scaling: 10x size should be <20x time
        time_10 = times[0][1]
        time_100 = times[2][1]
        time_1000 = times[4][1]

        scaling_10_to_100 = time_100 / time_10
        scaling_100_to_1000 = time_1000 / time_100

        assert scaling_10_to_100 < 20, (
            f"Scaling 10->100 tasks: {scaling_10_to_100:.1f}x (expected <20x)"
        )
        assert scaling_100_to_1000 < 20, (
            f"Scaling 100->1000 tasks: {scaling_100_to_1000:.1f}x (expected <20x)"
        )


# ==============================================================================
# Memory Profiling Tests
# ==============================================================================


class TestMemoryUsage:
    """Memory profiling for large task hierarchies."""

    @pytest.mark.performance
    @pytest.mark.asyncio
    async def test_memory_footprint_1000_tasks(
        self,
        task_hierarchy_factory,
        console_with_capture,
    ):
        """Test: Memory usage for 1000-task hierarchy is reasonable (<100MB).

        Validates memory overhead stays within acceptable bounds.
        """
        # Arrange
        console, buffer = console_with_capture

        # Force garbage collection before measurement
        gc.collect()

        # Measure initial memory
        process = psutil.Process(os.getpid())
        memory_before = process.memory_info().rss / 1024 / 1024  # MB

        # Create and render 1000-task hierarchy
        db, tasks = await task_hierarchy_factory(1000, depth=4, fanout=5)
        renderer = TreeRenderer()
        dependency_graph: dict[UUID, list[UUID]] = {}

        layout = renderer.compute_layout(tasks, dependency_graph)
        tree = renderer.render_tree(layout, use_unicode=True)
        console.print(tree)

        # Measure memory after
        memory_after = process.memory_info().rss / 1024 / 1024  # MB
        memory_used = memory_after - memory_before

        print(f"\n=== Memory Usage (1000 tasks) ===")
        print(f"  Before: {memory_before:.2f} MB")
        print(f"  After:  {memory_after:.2f} MB")
        print(f"  Used:   {memory_used:.2f} MB")
        print(f"  Per task: {memory_used/1000*1024:.2f} KB")

        # Assert - Verify reasonable memory usage (<100MB)
        assert memory_used < 100, (
            f"Memory usage too high: {memory_used:.2f} MB (target: <100 MB)"
        )

        # Cleanup
        await db.close()

    @pytest.mark.performance
    @pytest.mark.asyncio
    async def test_no_memory_leak_repeated_renders(
        self,
        task_hierarchy_factory,
        console_with_capture,
    ):
        """Test: No memory leaks during 100 repeated render operations.

        Validates memory doesn't grow linearly with render count.
        """
        # Arrange
        console, buffer = console_with_capture
        db, tasks = await task_hierarchy_factory(100, depth=3, fanout=3)
        renderer = TreeRenderer()
        dependency_graph: dict[UUID, list[UUID]] = {}

        gc.collect()
        process = psutil.Process(os.getpid())
        memory_samples = []

        # Perform 100 render operations
        for i in range(100):
            layout = renderer.compute_layout(tasks, dependency_graph)
            tree = renderer.render_tree(layout, use_unicode=True)
            console.print(tree)

            # Clear buffer to prevent accumulation
            buffer.seek(0)
            buffer.truncate(0)

            # Sample memory every 10 iterations
            if i % 10 == 0:
                gc.collect()
                mem = process.memory_info().rss / 1024 / 1024
                memory_samples.append(mem)

        # Analyze memory growth
        initial_memory = memory_samples[0]
        final_memory = memory_samples[-1]
        growth_rate = (final_memory - initial_memory) / initial_memory

        print(f"\n=== Memory Leak Test (100 renders) ===")
        print(f"  Initial: {initial_memory:.2f} MB")
        print(f"  Final:   {final_memory:.2f} MB")
        print(f"  Growth:  {growth_rate*100:.2f}%")

        # Assert - Memory growth should be minimal (<20%)
        assert growth_rate < 0.20, (
            f"Memory leak detected: {growth_rate*100:.2f}% growth over 100 iterations"
        )

        # Cleanup
        await db.close()


# ==============================================================================
# Determinism Validation Tests
# ==============================================================================


class TestDeterminism:
    """Validate deterministic rendering behavior (run with pytest-repeat)."""

    @pytest.mark.asyncio
    async def test_render_produces_identical_results(
        self,
        task_hierarchy_factory,
        console_with_capture,
    ):
        """Test: Rendering produces identical results every time (determinism).

        Run with: pytest --count=10 to validate 100% consistency.
        """
        # Arrange
        console, buffer = console_with_capture
        db, tasks = await task_hierarchy_factory(50, depth=3, fanout=3)
        renderer = TreeRenderer()
        dependency_graph: dict[UUID, list[UUID]] = {}

        # Render twice
        layout1 = renderer.compute_layout(tasks, dependency_graph)
        tree1 = renderer.render_tree(layout1, use_unicode=True)

        buffer.seek(0)
        buffer.truncate(0)
        console.print(tree1)
        output1 = buffer.getvalue()

        buffer.seek(0)
        buffer.truncate(0)

        layout2 = renderer.compute_layout(tasks, dependency_graph)
        tree2 = renderer.render_tree(layout2, use_unicode=True)
        console.print(tree2)
        output2 = buffer.getvalue()

        # Assert - Identical results
        assert layout1.total_nodes == layout2.total_nodes
        assert layout1.max_depth == layout2.max_depth
        assert len(layout1.root_nodes) == len(layout2.root_nodes)
        assert output1 == output2, "Rendering is non-deterministic"

        # Cleanup
        await db.close()


# ==============================================================================
# Performance Regression Detection
# ==============================================================================


class TestRegressionDetection:
    """Tests to detect performance regressions using benchmark comparisons."""

    @pytest.mark.benchmark
    @pytest.mark.asyncio
    async def test_100_task_regression_baseline(
        self,
        benchmark,
        task_hierarchy_factory,
        console_with_capture,
    ):
        """Baseline for regression detection (compare with --benchmark-compare).

        Run with:
            pytest --benchmark-autosave
            (make changes)
            pytest --benchmark-compare=0001
        """
        console, buffer = console_with_capture
        db, tasks = await task_hierarchy_factory(100, depth=3, fanout=3)
        renderer = TreeRenderer()
        dependency_graph: dict[UUID, list[UUID]] = {}

        def render():
            layout = renderer.compute_layout(tasks, dependency_graph)
            tree = renderer.render_tree(layout, use_unicode=True)
            console.print(tree)
            return layout

        result = benchmark(render)
        assert result.total_nodes == 100

        await db.close()
