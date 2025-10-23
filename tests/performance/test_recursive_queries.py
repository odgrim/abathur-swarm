"""Performance benchmarks for WITH RECURSIVE task tree queries.

Tests recursive CTE performance across various tree shapes:
- Small tree (10 tasks, depth 3)
- Medium tree (100 tasks, depth 5)
- Large tree (1000 tasks, depth 10)
- Wide tree (1000 tasks, depth 2, many siblings)

Performance targets (from NFR001):
- 1000-task tree traversal < 5 seconds
- Queries should use idx_tasks_parent index for optimal performance

NOTE: These tests depend on get_task_tree_with_status (P1-T2) being implemented.
Tests will be skipped if the method is not yet available.
"""

import time
from datetime import datetime, timezone
from pathlib import Path
from uuid import UUID, uuid4

import pytest
from abathur.domain.models import Task, TaskStatus, TaskSource, DependencyType
from abathur.infrastructure.database import Database

# Check if get_task_tree_with_status is implemented
HAS_RECURSIVE_QUERY = hasattr(Database, 'get_task_tree_with_status')
requires_recursive_query = pytest.mark.skipif(
    not HAS_RECURSIVE_QUERY,
    reason="Requires get_task_tree_with_status (P1-T2) to be implemented"
)


class TestRecursiveQueryPerformance:
    """Benchmark WITH RECURSIVE CTE performance for task tree traversal."""

    @requires_recursive_query
    @pytest.mark.asyncio
    async def test_small_tree_performance(self) -> None:
        """Benchmark small tree (10 tasks, depth 3) traversal.

        Tree structure:
            Root
            ├── Child 1
            │   ├── Grandchild 1
            │   └── Grandchild 2
            ├── Child 2
            │   ├── Grandchild 3
            │   └── Grandchild 4
            └── Child 3
                ├── Grandchild 5
                └── Grandchild 6

        Total: 1 root + 3 children + 6 grandchildren = 10 tasks
        """
        db = Database(Path(":memory:"))
        await db.initialize()

        # Create small tree
        root_id, task_count = await self._create_balanced_tree(
            db, depth=3, branching_factor=2
        )
        assert task_count == 15  # 1 + 2 + 4 + 8 = 15 tasks

        # Warm-up query
        await db.get_task_tree_with_status([root_id])

        # Benchmark 50 retrievals
        latencies = []
        for _ in range(50):
            start = time.perf_counter()
            tree = await db.get_task_tree_with_status([root_id])
            latencies.append((time.perf_counter() - start) * 1000)  # Convert to ms

        # Calculate statistics
        latencies.sort()
        avg_latency = sum(latencies) / len(latencies)
        p50 = latencies[24]
        p95 = latencies[47]
        p99 = latencies[49]

        print("\nSmall tree (10 tasks, depth 3) performance:")
        print(f"  Average: {avg_latency:.2f}ms")
        print(f"  p50: {p50:.2f}ms")
        print(f"  p95: {p95:.2f}ms")
        print(f"  p99: {p99:.2f}ms")
        print(f"  Tree size: {len(tree)} tasks")

        # Performance target: p99 < 50ms for small trees
        assert p99 < 50, f"Small tree p99={p99:.2f}ms (target <50ms)"
        assert len(tree) == task_count

    @requires_recursive_query
    @pytest.mark.asyncio
    async def test_medium_tree_performance(self) -> None:
        """Benchmark medium tree (100 tasks, depth 5) traversal.

        Tree structure: Balanced binary tree (branching factor 2)
        - Level 0: 1 root
        - Level 1: 2 tasks
        - Level 2: 4 tasks
        - Level 3: 8 tasks
        - Level 4: 16 tasks
        - Level 5: 32 tasks
        Total: 1 + 2 + 4 + 8 + 16 + 32 = 63 tasks (closest to 100 with depth 5)

        For ~100 tasks, use branching factor 3:
        - Level 0: 1 root
        - Level 1: 3 tasks
        - Level 2: 9 tasks
        - Level 3: 27 tasks
        - Level 4: 81 tasks
        Total: 1 + 3 + 9 + 27 + 81 = 121 tasks
        """
        db = Database(Path(":memory:"))
        await db.initialize()

        # Create medium tree
        root_id, task_count = await self._create_balanced_tree(
            db, depth=6, branching_factor=2
        )
        assert task_count == 127  # 1 + 2 + 4 + 8 + 16 + 32 + 64 = 127 tasks

        # Warm-up query
        await db.get_task_tree_with_status([root_id])

        # Benchmark 20 retrievals
        latencies = []
        for _ in range(20):
            start = time.perf_counter()
            tree = await db.get_task_tree_with_status([root_id])
            latencies.append((time.perf_counter() - start) * 1000)

        # Calculate statistics
        latencies.sort()
        avg_latency = sum(latencies) / len(latencies)
        p50 = latencies[9]
        p95 = latencies[18]
        p99 = latencies[19]

        print("\nMedium tree (121 tasks, depth 5) performance:")
        print(f"  Average: {avg_latency:.2f}ms")
        print(f"  p50: {p50:.2f}ms")
        print(f"  p95: {p95:.2f}ms")
        print(f"  p99: {p99:.2f}ms")
        print(f"  Tree size: {len(tree)} tasks")

        # Performance target: p99 < 200ms for medium trees
        assert p99 < 200, f"Medium tree p99={p99:.2f}ms (target <200ms)"
        assert len(tree) == task_count

    @requires_recursive_query
    @pytest.mark.asyncio
    async def test_large_tree_performance(self) -> None:
        """Benchmark large tree (1000 tasks, depth 10) traversal.

        This is the critical performance test from NFR001:
        - 1000-task tree traversal must complete in < 5 seconds

        Tree structure: Balanced tree with branching factor 2-3
        - Depth 10 with branching factor 2:
          1 + 2 + 4 + 8 + 16 + 32 + 64 + 128 + 256 + 512 = 1023 tasks
        """
        db = Database(Path(":memory:"))
        await db.initialize()

        # Create large tree (2047 tasks, depth 10)
        root_id, task_count = await self._create_balanced_tree(
            db, depth=10, branching_factor=2
        )
        assert task_count == 2047  # 1 + 2 + 4 + ... + 1024 = 2047 tasks

        # Warm-up query
        await db.get_task_tree_with_status([root_id])

        # Benchmark 10 retrievals
        latencies = []
        for _ in range(10):
            start = time.perf_counter()
            tree = await db.get_task_tree_with_status([root_id])
            duration = time.perf_counter() - start
            latencies.append(duration * 1000)  # Convert to ms

        # Calculate statistics
        latencies.sort()
        avg_latency = sum(latencies) / len(latencies)
        p50 = latencies[4]
        p95 = latencies[9]
        p99 = latencies[9]

        print("\nLarge tree (1023 tasks, depth 10) performance:")
        print(f"  Average: {avg_latency:.2f}ms ({avg_latency/1000:.3f}s)")
        print(f"  p50: {p50:.2f}ms ({p50/1000:.3f}s)")
        print(f"  p95: {p95:.2f}ms ({p95/1000:.3f}s)")
        print(f"  p99: {p99:.2f}ms ({p99/1000:.3f}s)")
        print(f"  Tree size: {len(tree)} tasks")

        # Critical NFR001 requirement: 1000-task tree < 5 seconds
        assert p99 < 5000, f"Large tree p99={p99:.2f}ms ({p99/1000:.3f}s) (NFR001: target <5s)"
        assert len(tree) == task_count

    @requires_recursive_query
    @pytest.mark.asyncio
    async def test_wide_tree_performance(self) -> None:
        """Benchmark wide tree (1000 tasks, depth 2, many siblings).

        Tree structure: Wide and shallow
        - 1 root
        - 999 children (all siblings at depth 1)
        Total: 1000 tasks

        This tests performance with high branching factor (many siblings).
        """
        db = Database(Path(":memory:"))
        await db.initialize()

        # Create wide tree (1000 tasks, depth 2)
        root_id, task_count = await self._create_wide_tree(
            db, root_children_count=999
        )
        assert task_count == 1000  # 1 root + 999 children

        # Warm-up query
        await db.get_task_tree_with_status([root_id])

        # Benchmark 10 retrievals
        latencies = []
        for _ in range(10):
            start = time.perf_counter()
            tree = await db.get_task_tree_with_status([root_id])
            duration = time.perf_counter() - start
            latencies.append(duration * 1000)

        # Calculate statistics
        latencies.sort()
        avg_latency = sum(latencies) / len(latencies)
        p50 = latencies[4]
        p95 = latencies[9]
        p99 = latencies[9]

        print("\nWide tree (1000 tasks, depth 2, 999 siblings) performance:")
        print(f"  Average: {avg_latency:.2f}ms ({avg_latency/1000:.3f}s)")
        print(f"  p50: {p50:.2f}ms ({p50/1000:.3f}s)")
        print(f"  p95: {p95:.2f}ms ({p95/1000:.3f}s)")
        print(f"  p99: {p99:.2f}ms ({p99/1000:.3f}s)")
        print(f"  Tree size: {len(tree)} tasks")

        # Performance target: wide tree < 5 seconds (same as NFR001)
        assert p99 < 5000, f"Wide tree p99={p99:.2f}ms ({p99/1000:.3f}s) (target <5s)"
        assert len(tree) == task_count

    @requires_recursive_query
    @pytest.mark.asyncio
    async def test_status_filtering_performance(self) -> None:
        """Benchmark status filtering on large tree.

        Creates a large tree with mixed statuses and measures performance
        when filtering for specific statuses (e.g., only COMPLETED tasks).
        """
        db = Database(Path(":memory:"))
        await db.initialize()

        # Create large tree with mixed statuses
        root_id, task_count = await self._create_balanced_tree_mixed_status(
            db, depth=8, branching_factor=3
        )
        assert task_count > 100  # Ensure we have a reasonable tree size

        # Warm-up query
        await db.get_task_tree_with_status([root_id])

        # Benchmark status-filtered query
        latencies = []
        for _ in range(10):
            start = time.perf_counter()
            tree = await db.get_task_tree_with_status(
                [root_id], filter_statuses=[TaskStatus.COMPLETED]
            )
            duration = time.perf_counter() - start
            latencies.append(duration * 1000)

        latencies.sort()
        avg_latency = sum(latencies) / len(latencies)
        p99 = latencies[9]

        print("\nStatus filtering performance (COMPLETED tasks only):")
        print(f"  Total tree size: {task_count} tasks")
        print(f"  Filtered tree size: {len(tree)} tasks")
        print(f"  Average: {avg_latency:.2f}ms")
        print(f"  p99: {p99:.2f}ms")

        # Performance target: filtering should be fast even on large trees
        assert p99 < 2000, f"Status filtering p99={p99:.2f}ms (target <2s)"

    @requires_recursive_query
    @pytest.mark.asyncio
    async def test_concurrent_recursive_queries(self) -> None:
        """Test concurrent recursive queries on multiple trees.

        This simulates multiple concurrent prune operations or tree traversals.
        """
        db = Database(Path(":memory:"))
        await db.initialize()

        # Create 5 medium trees
        root_ids = []
        for i in range(5):
            root_id, _ = await self._create_balanced_tree(
                db, depth=5, branching_factor=2
            )
            root_ids.append(root_id)

        # Benchmark concurrent queries
        import asyncio

        start_time = time.perf_counter()
        tasks = [
            db.get_task_tree_with_status([root_id]) for root_id in root_ids
        ]
        results = await asyncio.gather(*tasks)
        duration = time.perf_counter() - start_time

        print(f"\nConcurrent recursive queries: 5 trees in {duration:.3f}s")
        print(f"  Trees retrieved: {len(results)}")
        print(f"  Average per tree: {duration/5:.3f}s")

        assert len(results) == 5
        assert all(len(tree) > 0 for tree in results)
        # All 5 queries should complete in < 10 seconds
        assert duration < 10.0, f"5 concurrent queries took {duration:.3f}s (target <10s)"

    # Helper methods for creating test trees

    async def _create_balanced_tree(
        self, db: Database, depth: int, branching_factor: int
    ) -> tuple[UUID, int]:
        """Create a balanced tree with specified depth and branching factor.

        Args:
            db: Database instance
            depth: Tree depth (0 = root only)
            branching_factor: Number of children per node

        Returns:
            Tuple of (root_task_id, total_task_count)
        """
        root_id = uuid4()
        task_count = 0

        # Create root task
        root_task = Task(
            id=root_id,
            prompt="Root task (level 0)",
            agent_type="test-agent",
            priority=5,
            status=TaskStatus.COMPLETED,
            input_data={},
            submitted_at=datetime.now(timezone.utc),
            last_updated_at=datetime.now(timezone.utc),
            source=TaskSource.HUMAN,
            dependency_type=DependencyType.SEQUENTIAL,
            summary="Root task (level 0)",
        )
        await db.insert_task(root_task)
        task_count += 1

        # Track tasks by level for parent-child relationships
        tasks_by_level: dict[int, list[UUID]] = {0: [root_id]}

        # Create children level by level
        for level in range(depth):
            tasks_by_level[level + 1] = []

            for parent_id in tasks_by_level[level]:
                # Create children for this parent
                for i in range(branching_factor):
                    child_id = uuid4()
                    child_task = Task(
                        id=child_id,
                        prompt=f"Task at level {level + 1}",
                        agent_type="test-agent",
                        priority=5,
                        status=TaskStatus.COMPLETED,
                        input_data={},
                        submitted_at=datetime.now(timezone.utc),
                        last_updated_at=datetime.now(timezone.utc),
                        source=TaskSource.HUMAN,
                        dependency_type=DependencyType.SEQUENTIAL,
                        parent_task_id=parent_id,
                        summary=f"Task at level {level + 1}",
                    )
                    await db.insert_task(child_task)
                    tasks_by_level[level + 1].append(child_id)
                    task_count += 1

        return root_id, task_count

    async def _create_wide_tree(
        self, db: Database, root_children_count: int
    ) -> tuple[UUID, int]:
        """Create a wide tree with one root and many children.

        Args:
            db: Database instance
            root_children_count: Number of children for the root

        Returns:
            Tuple of (root_task_id, total_task_count)
        """
        root_id = uuid4()

        # Create root task
        root_task = Task(
            id=root_id,
            prompt="Root task",
            agent_type="test-agent",
            priority=5,
            status=TaskStatus.COMPLETED,
            input_data={},
            submitted_at=datetime.now(timezone.utc),
            last_updated_at=datetime.now(timezone.utc),
            source=TaskSource.HUMAN,
            dependency_type=DependencyType.SEQUENTIAL,
            summary="Root task",
        )
        await db.insert_task(root_task)

        # Create children
        for i in range(root_children_count):
            child_id = uuid4()
            child_task = Task(
                id=child_id,
                prompt=f"Child task {i}",
                agent_type="test-agent",
                priority=5,
                status=TaskStatus.COMPLETED,
                input_data={},
                submitted_at=datetime.now(timezone.utc),
                last_updated_at=datetime.now(timezone.utc),
                source=TaskSource.HUMAN,
                dependency_type=DependencyType.SEQUENTIAL,
                parent_task_id=root_id,
                summary=f"Child task {i}",
            )
            await db.insert_task(child_task)

        return root_id, 1 + root_children_count

    async def _create_balanced_tree_mixed_status(
        self, db: Database, depth: int, branching_factor: int
    ) -> tuple[UUID, int]:
        """Create a balanced tree with mixed task statuses.

        Args:
            db: Database instance
            depth: Tree depth
            branching_factor: Number of children per node

        Returns:
            Tuple of (root_task_id, total_task_count)
        """
        root_id = uuid4()
        task_count = 0

        # Cycle through statuses
        statuses = [
            TaskStatus.COMPLETED,
            TaskStatus.FAILED,
            TaskStatus.CANCELLED,
            TaskStatus.RUNNING,
        ]

        # Create root task
        root_task = Task(
            id=root_id,
            prompt="Root task (level 0)",
            agent_type="test-agent",
            priority=5,
            status=statuses[task_count % len(statuses)],
            input_data={},
            submitted_at=datetime.now(timezone.utc),
            last_updated_at=datetime.now(timezone.utc),
            source=TaskSource.HUMAN,
            dependency_type=DependencyType.SEQUENTIAL,
            summary="Root task (level 0)",
        )
        await db.insert_task(root_task)
        task_count += 1

        # Track tasks by level for parent-child relationships
        tasks_by_level: dict[int, list[UUID]] = {0: [root_id]}

        # Create children level by level
        for level in range(depth):
            tasks_by_level[level + 1] = []

            for parent_id in tasks_by_level[level]:
                # Create children for this parent
                for i in range(branching_factor):
                    child_id = uuid4()
                    child_task = Task(
                        id=child_id,
                        prompt=f"Task at level {level + 1}",
                        agent_type="test-agent",
                        priority=5,
                        status=statuses[task_count % len(statuses)],
                        input_data={},
                        submitted_at=datetime.now(timezone.utc),
                        last_updated_at=datetime.now(timezone.utc),
                        source=TaskSource.HUMAN,
                        dependency_type=DependencyType.SEQUENTIAL,
                        parent_task_id=parent_id,
                        summary=f"Task at level {level + 1}",
                    )
                    await db.insert_task(child_task)
                    tasks_by_level[level + 1].append(child_id)
                    task_count += 1

        return root_id, task_count


class TestRecursiveQueryIndexUsage:
    """Verify recursive queries use idx_tasks_parent index for optimal performance."""

    @requires_recursive_query
    @pytest.mark.asyncio
    async def test_recursive_query_uses_parent_index(self) -> None:
        """Verify WITH RECURSIVE query uses idx_tasks_parent index.

        The idx_tasks_parent index on parent_task_id is critical for
        recursive CTE performance. This test verifies it's being used.
        """
        db = Database(Path(":memory:"))
        await db.initialize()

        # Create a simple tree
        root_id = uuid4()
        child_id = uuid4()

        root_task = Task(
            id=root_id,
            prompt="Root",
            agent_type="test",
            priority=5,
            status=TaskStatus.COMPLETED,
            input_data={},
            submitted_at=datetime.now(timezone.utc),
            last_updated_at=datetime.now(timezone.utc),
            source=TaskSource.HUMAN,
            dependency_type=DependencyType.SEQUENTIAL,
            summary="Root",
        )
        await db.insert_task(root_task)

        child_task = Task(
            id=child_id,
            prompt="Child",
            agent_type="test",
            priority=5,
            status=TaskStatus.COMPLETED,
            input_data={},
            submitted_at=datetime.now(timezone.utc),
            last_updated_at=datetime.now(timezone.utc),
            source=TaskSource.HUMAN,
            dependency_type=DependencyType.SEQUENTIAL,
            parent_task_id=root_id,
            summary="Child",
        )
        await db.insert_task(child_task)

        # Get the query plan for the recursive CTE
        # This is the core query from get_task_tree_with_status
        query = """
            WITH RECURSIVE task_tree AS (
                -- Base case: root tasks
                SELECT
                    id,
                    parent_task_id,
                    status,
                    0 AS depth
                FROM tasks
                WHERE id IN (?)

                UNION ALL

                -- Recursive case: children of current level
                SELECT
                    t.id,
                    t.parent_task_id,
                    t.status,
                    tt.depth + 1 AS depth
                FROM tasks t
                INNER JOIN task_tree tt ON t.parent_task_id = tt.id
                WHERE tt.depth < ?
            )
            SELECT
                id,
                parent_task_id,
                status,
                depth
            FROM task_tree
            ORDER BY depth ASC, id ASC
        """

        plan = await db.explain_query_plan(query, (str(root_id), 100))
        plan_text = " ".join(plan)

        print("\nRecursive CTE query plan:")
        for line in plan:
            print(f"  {line}")

        # Verify index usage
        # The recursive step should use idx_tasks_parent for the JOIN
        assert (
            "idx_tasks_parent" in plan_text or "USING INDEX" in plan_text
        ), f"Expected idx_tasks_parent index usage, got: {plan_text}"

        # Should NOT be doing a full table scan
        assert "SCAN TABLE tasks" not in plan_text or "USING INDEX" in plan_text, \
            f"Unexpected table scan in recursive query: {plan_text}"

    @pytest.mark.asyncio
    async def test_verify_parent_index_exists(self) -> None:
        """Verify idx_tasks_parent index exists in database schema."""
        db = Database(Path(":memory:"))
        await db.initialize()

        # Get all indexes
        index_info = await db.get_index_usage()

        print("\nAll indexes on tasks table:")
        task_indexes = [
            idx for idx in index_info["indexes"] if idx["table"] == "tasks"
        ]
        for idx in task_indexes:
            print(f"  - {idx['name']} on {idx['table']}")

        # Verify idx_tasks_parent exists
        parent_index_exists = any(
            idx["name"] == "idx_tasks_parent" for idx in task_indexes
        )

        assert parent_index_exists, "idx_tasks_parent index not found in schema"
