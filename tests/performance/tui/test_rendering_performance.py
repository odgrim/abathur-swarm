"""Performance tests for TUI rendering.

Benchmarks rendering performance with different dataset sizes:
- 100 tasks: <500ms (NFR001)
- 500 tasks: <2s
- 1000 tasks: <5s
- Cache hit performance: <50ms
- Layout computation performance
"""

import pytest
import time
import tempfile
from pathlib import Path
from datetime import datetime, timezone
from uuid import uuid4

from abathur.infrastructure.database import Database
from abathur.domain.models import Task, TaskStatus, TaskSource
from abathur.tui.rendering.tree_renderer import TreeRenderer


# Import mock service from unit tests
import sys
sys.path.insert(0, str(Path(__file__).parent.parent.parent / "unit" / "tui"))
from test_task_data_service import MockTaskDataService


@pytest.fixture
async def db_with_n_tasks():
    """Factory fixture to create database with N tasks."""

    async def _create_db(n: int) -> Database:
        """Create temporary database with N tasks."""
        # Use temporary file for performance tests (closer to production)
        temp_file = tempfile.NamedTemporaryFile(suffix=".db", delete=False)
        db_path = Path(temp_file.name)
        temp_file.close()

        db = Database(db_path)
        await db.initialize()

        # Insert N tasks with varied properties
        for i in range(n):
            task = Task(
                id=uuid4(),
                prompt=f"Performance test task {i}",
                summary=f"Task {i}",
                agent_type="test-agent" if i % 2 == 0 else "python-specialist",
                status=list(TaskStatus)[i % len(TaskStatus)],
                calculated_priority=float(i % 20),
                dependency_depth=i % 10,
                submitted_at=datetime.now(timezone.utc),
                source=TaskSource.HUMAN,
                feature_branch=f"feature/test-{i % 5}" if i % 3 == 0 else None,
            )
            await db.insert_task(task)

        return db

    return _create_db


class TestRenderPerformance100Tasks:
    """Test suite for 100-task rendering performance (NFR001)."""

    @pytest.mark.performance
    @pytest.mark.asyncio
    async def test_render_100_tasks_under_500ms(self, db_with_n_tasks):
        """Test rendering 100 tasks meets <500ms target (NFR001)."""
        # Arrange
        db = await db_with_n_tasks(100)
        data_service = MockTaskDataService(db)
        renderer = TreeRenderer()

        # Act - measure total time for fetch + render
        start = time.perf_counter()

        tasks = await data_service.fetch_tasks()
        rendered_lines = renderer.render_flat_list(tasks)

        elapsed = time.perf_counter() - start

        # Assert - under 500ms
        elapsed_ms = elapsed * 1000
        assert elapsed < 0.500, (
            f"Render took {elapsed_ms:.1f}ms, expected <500ms (NFR001)"
        )

        # Verify data integrity
        assert len(tasks) == 100
        assert len(rendered_lines) == 100

        # Cleanup
        await db.close()
        db_path = db.db_path
        if db_path.exists() and db_path != Path(":memory:"):
            db_path.unlink()

    @pytest.mark.performance
    @pytest.mark.asyncio
    async def test_render_100_tasks_multiple_runs(self, db_with_n_tasks):
        """Test consistent performance across multiple render runs."""
        # Arrange
        db = await db_with_n_tasks(100)
        data_service = MockTaskDataService(db)
        renderer = TreeRenderer()

        times = []

        # Act - run 5 times and measure
        for _ in range(5):
            start = time.perf_counter()
            tasks = await data_service.fetch_tasks(force_refresh=True)
            rendered_lines = renderer.render_flat_list(tasks)
            elapsed = time.perf_counter() - start
            times.append(elapsed)

        # Assert - all runs under 500ms
        avg_time = sum(times) / len(times)
        max_time = max(times)

        assert max_time < 0.500, f"Max time {max_time*1000:.1f}ms exceeded 500ms"
        assert avg_time < 0.400, f"Avg time {avg_time*1000:.1f}ms should be well under target"

        # Cleanup
        await db.close()
        db_path = db.db_path
        if db_path.exists() and db_path != Path(":memory:"):
            db_path.unlink()


class TestRenderPerformance500Tasks:
    """Test suite for 500-task rendering performance."""

    @pytest.mark.performance
    @pytest.mark.asyncio
    async def test_render_500_tasks_under_2s(self, db_with_n_tasks):
        """Test rendering 500 tasks meets <2s target."""
        # Arrange
        db = await db_with_n_tasks(500)
        data_service = MockTaskDataService(db)
        renderer = TreeRenderer()

        # Act - measure render time
        start = time.perf_counter()

        tasks = await data_service.fetch_tasks()
        rendered_lines = renderer.render_flat_list(tasks)

        elapsed = time.perf_counter() - start

        # Assert - under 2 seconds
        assert elapsed < 2.0, f"Render took {elapsed:.2f}s, expected <2s"

        # Verify data integrity
        assert len(tasks) == 500
        assert len(rendered_lines) == 500

        # Cleanup
        await db.close()
        db_path = db.db_path
        if db_path.exists() and db_path != Path(":memory:"):
            db_path.unlink()


class TestRenderPerformance1000Tasks:
    """Test suite for 1000-task rendering performance."""

    @pytest.mark.performance
    @pytest.mark.asyncio
    async def test_render_1000_tasks_under_5s(self, db_with_n_tasks):
        """Test rendering 1000 tasks meets <5s target."""
        # Arrange
        db = await db_with_n_tasks(1000)
        data_service = MockTaskDataService(db)
        renderer = TreeRenderer()

        # Act - measure render time
        start = time.perf_counter()

        tasks = await data_service.fetch_tasks()
        rendered_lines = renderer.render_flat_list(tasks)

        elapsed = time.perf_counter() - start

        # Assert - under 5 seconds
        assert elapsed < 5.0, f"Render took {elapsed:.2f}s, expected <5s"

        # Verify data integrity
        assert len(tasks) == 1000
        assert len(rendered_lines) == 1000

        # Cleanup
        await db.close()
        db_path = db.db_path
        if db_path.exists() and db_path != Path(":memory:"):
            db_path.unlink()


class TestCacheHitPerformance:
    """Test suite for cache hit performance."""

    @pytest.mark.performance
    @pytest.mark.asyncio
    async def test_cache_hit_under_50ms(self, db_with_n_tasks):
        """Test cache hit response time <50ms."""
        # Arrange
        db = await db_with_n_tasks(100)
        data_service = MockTaskDataService(db, cache_ttl_seconds=60)

        # Prime cache
        await data_service.fetch_tasks()

        # Act - measure cache hit time
        start = time.perf_counter()
        cached_tasks = await data_service.fetch_tasks()
        elapsed = time.perf_counter() - start

        # Assert - under 50ms
        elapsed_ms = elapsed * 1000
        assert elapsed < 0.050, (
            f"Cache hit took {elapsed_ms:.1f}ms, expected <50ms"
        )

        # Verify cache was hit
        assert len(cached_tasks) == 100

        # Cleanup
        await db.close()
        db_path = db.db_path
        if db_path.exists() and db_path != Path(":memory:"):
            db_path.unlink()

    @pytest.mark.performance
    @pytest.mark.asyncio
    async def test_cache_hit_significantly_faster_than_db(self, db_with_n_tasks):
        """Test cache hit is significantly faster than database fetch."""
        # Arrange
        db = await db_with_n_tasks(500)
        data_service = MockTaskDataService(db)

        # Measure initial fetch (database)
        start = time.perf_counter()
        await data_service.fetch_tasks()
        db_time = time.perf_counter() - start

        # Measure cache hit
        start = time.perf_counter()
        await data_service.fetch_tasks()
        cache_time = time.perf_counter() - start

        # Assert - cache at least 10x faster
        speedup = db_time / cache_time
        assert speedup >= 10, (
            f"Cache only {speedup:.1f}x faster, expected >=10x speedup"
        )

        # Cleanup
        await db.close()
        db_path = db.db_path
        if db_path.exists() and db_path != Path(":memory:"):
            db_path.unlink()


class TestLayoutComputationPerformance:
    """Test suite for layout algorithm performance."""

    @pytest.mark.performance
    def test_format_task_node_performance(self):
        """Test format_task_node() performance with 1000 tasks."""
        # Arrange
        renderer = TreeRenderer()
        tasks = [
            Task(
                id=uuid4(),
                prompt=f"Task {i}",
                summary=f"Task {i} with a longer summary for testing",
                agent_type="test",
                status=TaskStatus.PENDING,
                calculated_priority=float(i % 20),
                dependency_depth=0,
                submitted_at=datetime.now(timezone.utc),
                source=TaskSource.HUMAN,
            )
            for i in range(1000)
        ]

        # Act - measure formatting time
        start = time.perf_counter()

        for task in tasks:
            formatted = renderer.format_task_node(task)

        elapsed = time.perf_counter() - start

        # Assert - should be very fast (< 100ms for 1000 tasks)
        assert elapsed < 0.100, (
            f"Formatting 1000 tasks took {elapsed*1000:.1f}ms, expected <100ms"
        )

    @pytest.mark.performance
    def test_render_flat_list_performance_1000_tasks(self):
        """Test render_flat_list() performance with 1000 tasks."""
        # Arrange
        renderer = TreeRenderer()
        tasks = [
            Task(
                id=uuid4(),
                prompt=f"Task {i}",
                summary=f"Task {i}",
                agent_type="test",
                status=list(TaskStatus)[i % len(TaskStatus)],
                calculated_priority=float(i % 20),
                dependency_depth=0,
                submitted_at=datetime.now(timezone.utc),
                source=TaskSource.HUMAN,
            )
            for i in range(1000)
        ]

        # Act - measure rendering time
        start = time.perf_counter()
        rendered_lines = renderer.render_flat_list(tasks)
        elapsed = time.perf_counter() - start

        # Assert - reasonable performance (<500ms for 1000 tasks)
        assert elapsed < 0.500, (
            f"Rendering 1000 tasks took {elapsed*1000:.1f}ms, expected <500ms"
        )
        assert len(rendered_lines) == 1000


class TestDatabaseQueryPerformance:
    """Test suite for database query performance."""

    @pytest.mark.performance
    @pytest.mark.asyncio
    async def test_list_tasks_performance_100(self, db_with_n_tasks):
        """Test database list_tasks query performance with 100 tasks."""
        # Arrange
        db = await db_with_n_tasks(100)

        # Act - measure query time
        start = time.perf_counter()
        tasks = await db.list_tasks()
        elapsed = time.perf_counter() - start

        # Assert - database query should be fast
        assert elapsed < 0.100, (
            f"Database query took {elapsed*1000:.1f}ms, expected <100ms"
        )
        assert len(tasks) == 100

        # Cleanup
        await db.close()
        db_path = db.db_path
        if db_path.exists() and db_path != Path(":memory:"):
            db_path.unlink()

    @pytest.mark.performance
    @pytest.mark.asyncio
    async def test_list_tasks_performance_1000(self, db_with_n_tasks):
        """Test database list_tasks query performance with 1000 tasks."""
        # Arrange
        db = await db_with_n_tasks(1000)

        # Act - measure query time
        start = time.perf_counter()
        tasks = await db.list_tasks()
        elapsed = time.perf_counter() - start

        # Assert - should still be reasonable
        assert elapsed < 1.0, (
            f"Database query took {elapsed:.2f}s, expected <1s"
        )
        assert len(tasks) == 1000

        # Cleanup
        await db.close()
        db_path = db.db_path
        if db_path.exists() and db_path != Path(":memory:"):
            db_path.unlink()


class TestFilteringPerformance:
    """Test suite for filtering performance."""

    @pytest.mark.performance
    @pytest.mark.asyncio
    async def test_filtering_performance_1000_tasks(self, db_with_n_tasks):
        """Test filtering 1000 tasks is performant."""
        # Arrange
        db = await db_with_n_tasks(1000)
        data_service = MockTaskDataService(db)

        # Import MockFilterState
        from test_task_data_service import MockFilterState

        filter_state = MockFilterState(status=TaskStatus.PENDING)

        # Act - measure filtering time
        start = time.perf_counter()
        filtered_tasks = await data_service.fetch_tasks(filter_state=filter_state)
        elapsed = time.perf_counter() - start

        # Assert - filtering should be fast
        assert elapsed < 1.0, (
            f"Filtering took {elapsed:.2f}s, expected <1s"
        )
        assert len(filtered_tasks) > 0

        # Cleanup
        await db.close()
        db_path = db.db_path
        if db_path.exists() and db_path != Path(":memory:"):
            db_path.unlink()


class TestPerformanceSummary:
    """Test suite for generating performance summary."""

    @pytest.mark.performance
    @pytest.mark.asyncio
    async def test_performance_summary_report(self, db_with_n_tasks):
        """Generate performance summary across all test sizes."""
        # Test sizes
        sizes = [100, 500, 1000]
        results = []

        for size in sizes:
            # Arrange
            db = await db_with_n_tasks(size)
            data_service = MockTaskDataService(db)
            renderer = TreeRenderer()

            # Measure
            start = time.perf_counter()
            tasks = await data_service.fetch_tasks()
            rendered_lines = renderer.render_flat_list(tasks)
            elapsed = time.perf_counter() - start

            results.append({
                "size": size,
                "time_ms": elapsed * 1000,
                "tasks_per_second": size / elapsed if elapsed > 0 else 0,
            })

            # Cleanup
            await db.close()
            db_path = db.db_path
            if db_path.exists() and db_path != Path(":memory:"):
                db_path.unlink()

        # Print summary
        print("\n=== Performance Summary ===")
        for result in results:
            print(
                f"{result['size']} tasks: {result['time_ms']:.1f}ms "
                f"({result['tasks_per_second']:.0f} tasks/sec)"
            )

        # Assert - verify all within targets
        assert results[0]["time_ms"] < 500, "100 tasks exceeded 500ms target"
        assert results[1]["time_ms"] < 2000, "500 tasks exceeded 2s target"
        assert results[2]["time_ms"] < 5000, "1000 tasks exceeded 5s target"
