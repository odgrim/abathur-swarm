"""Unit tests for DependencyResolver service.

Tests cover:
- Circular dependency detection (simple, complex, transitive cycles)
- Topological sorting (linear, branching, diamond patterns)
- Dependency depth calculation (single level, multi-level, max depth)
- Edge cases (empty graph, single node, disconnected components)
- Graph caching and invalidation
- Error handling and validation
"""

from datetime import datetime, timedelta, timezone
from pathlib import Path

import pytest
from abathur.domain.models import DependencyType, Task, TaskDependency, TaskStatus
from abathur.infrastructure.database import Database
from abathur.services.dependency_resolver import CircularDependencyError, DependencyResolver


@pytest.fixture
async def db():
    """Create an in-memory database for testing."""
    database = Database(db_path=Path(":memory:"))
    await database.initialize()
    yield database
    await database.close()


@pytest.fixture
async def resolver(db):
    """Create a DependencyResolver instance."""
    return DependencyResolver(db, cache_ttl_seconds=60.0)


@pytest.mark.asyncio
class TestCircularDependencyDetection:
    """Test circular dependency detection algorithm."""

    async def test_detect_simple_cycle(self, db, resolver):
        """Test detection of simple A -> B -> A cycle."""
        # Create task A
        task_a = Task(prompt="Task A", status=TaskStatus.PENDING)
        await db.insert_task(task_a)

        # Create task B that depends on A
        task_b = Task(prompt="Task B", status=TaskStatus.PENDING)
        await db.insert_task(task_b)

        dep_b_a = TaskDependency(
            dependent_task_id=task_b.id,
            prerequisite_task_id=task_a.id,
            dependency_type=DependencyType.SEQUENTIAL,
        )
        await db.insert_task_dependency(dep_b_a)

        # Try to make A depend on B (creates cycle)
        with pytest.raises(CircularDependencyError) as exc_info:
            await resolver.detect_circular_dependencies([task_b.id], task_a.id)

        assert "Circular dependency detected" in str(exc_info.value)

    async def test_detect_complex_cycle(self, db, resolver):
        """Test detection of complex A -> B -> C -> D -> B cycle."""
        # Create tasks
        task_a = Task(prompt="Task A", status=TaskStatus.PENDING)
        task_b = Task(prompt="Task B", status=TaskStatus.PENDING)
        task_c = Task(prompt="Task C", status=TaskStatus.PENDING)
        task_d = Task(prompt="Task D", status=TaskStatus.PENDING)

        for task in [task_a, task_b, task_c, task_d]:
            await db.insert_task(task)

        # Create dependencies: A -> B -> C -> D
        deps = [
            TaskDependency(
                dependent_task_id=task_b.id,
                prerequisite_task_id=task_a.id,
                dependency_type=DependencyType.SEQUENTIAL,
            ),
            TaskDependency(
                dependent_task_id=task_c.id,
                prerequisite_task_id=task_b.id,
                dependency_type=DependencyType.SEQUENTIAL,
            ),
            TaskDependency(
                dependent_task_id=task_d.id,
                prerequisite_task_id=task_c.id,
                dependency_type=DependencyType.SEQUENTIAL,
            ),
        ]

        for dep in deps:
            await db.insert_task_dependency(dep)

        # Try to make B depend on D (creates cycle: B -> C -> D -> B)
        with pytest.raises(CircularDependencyError) as exc_info:
            await resolver.detect_circular_dependencies([task_d.id], task_b.id)

        assert "Circular dependency detected" in str(exc_info.value)

    async def test_detect_no_cycle_linear(self, db, resolver):
        """Test that linear dependency chain A -> B -> C has no cycle."""
        # Create tasks
        task_a = Task(prompt="Task A", status=TaskStatus.PENDING)
        task_b = Task(prompt="Task B", status=TaskStatus.PENDING)
        task_c = Task(prompt="Task C", status=TaskStatus.PENDING)

        for task in [task_a, task_b, task_c]:
            await db.insert_task(task)

        # Create dependencies: A -> B -> C
        deps = [
            TaskDependency(
                dependent_task_id=task_b.id,
                prerequisite_task_id=task_a.id,
                dependency_type=DependencyType.SEQUENTIAL,
            ),
            TaskDependency(
                dependent_task_id=task_c.id,
                prerequisite_task_id=task_b.id,
                dependency_type=DependencyType.SEQUENTIAL,
            ),
        ]

        for dep in deps:
            await db.insert_task_dependency(dep)

        # Should not raise - linear chain is valid
        cycles = await resolver.detect_circular_dependencies([task_b.id], task_c.id)
        assert len(cycles) == 0

    async def test_detect_cycle_in_diamond(self, db, resolver):
        """Test that adding back-edge in diamond pattern creates cycle."""
        # Create tasks
        task_a = Task(prompt="Task A", status=TaskStatus.PENDING)
        task_b = Task(prompt="Task B", status=TaskStatus.PENDING)
        task_c = Task(prompt="Task C", status=TaskStatus.PENDING)
        task_d = Task(prompt="Task D", status=TaskStatus.PENDING)

        for task in [task_a, task_b, task_c, task_d]:
            await db.insert_task(task)

        # Create diamond: A -> B,C -> D
        deps = [
            TaskDependency(
                dependent_task_id=task_b.id,
                prerequisite_task_id=task_a.id,
                dependency_type=DependencyType.SEQUENTIAL,
            ),
            TaskDependency(
                dependent_task_id=task_c.id,
                prerequisite_task_id=task_a.id,
                dependency_type=DependencyType.SEQUENTIAL,
            ),
            TaskDependency(
                dependent_task_id=task_d.id,
                prerequisite_task_id=task_b.id,
                dependency_type=DependencyType.PARALLEL,
            ),
            TaskDependency(
                dependent_task_id=task_d.id,
                prerequisite_task_id=task_c.id,
                dependency_type=DependencyType.PARALLEL,
            ),
        ]

        for dep in deps:
            await db.insert_task_dependency(dep)

        # Test 1: Adding D as prerequisite for A would create cycle (D->A->B->D and D->A->C->D)
        with pytest.raises(CircularDependencyError):
            await resolver.detect_circular_dependencies([task_d.id], task_a.id)

        # Test 2: Adding D as prerequisite for B would create cycle (D->B->D)
        with pytest.raises(CircularDependencyError):
            await resolver.detect_circular_dependencies([task_d.id], task_b.id)

    async def test_self_dependency_rejected(self, db, resolver):
        """Test that self-dependency is rejected."""
        task_a = Task(prompt="Task A", status=TaskStatus.PENDING)
        await db.insert_task(task_a)

        # Try to make A depend on itself
        with pytest.raises(CircularDependencyError) as exc_info:
            await resolver.detect_circular_dependencies([task_a.id], task_a.id)

        assert "Self-dependency not allowed" in str(exc_info.value)

    async def test_transitive_cycle_detection(self, db, resolver):
        """Test detection of transitive cycle A -> B -> C, adding A as prerequisite for C."""
        # Create tasks
        task_a = Task(prompt="Task A", status=TaskStatus.PENDING)
        task_b = Task(prompt="Task B", status=TaskStatus.PENDING)
        task_c = Task(prompt="Task C", status=TaskStatus.PENDING)

        for task in [task_a, task_b, task_c]:
            await db.insert_task(task)

        # Create A -> B -> C (A is prereq for B, B is prereq for C)
        deps = [
            TaskDependency(
                dependent_task_id=task_b.id,
                prerequisite_task_id=task_a.id,
                dependency_type=DependencyType.SEQUENTIAL,
            ),
            TaskDependency(
                dependent_task_id=task_c.id,
                prerequisite_task_id=task_b.id,
                dependency_type=DependencyType.SEQUENTIAL,
            ),
        ]

        for dep in deps:
            await db.insert_task_dependency(dep)

        # Try to make A depend on C (creates transitive cycle: A->B->C->A)
        # This means C is prerequisite for A
        with pytest.raises(CircularDependencyError):
            await resolver.detect_circular_dependencies([task_c.id], task_a.id)


@pytest.mark.asyncio
class TestDependencyDepthCalculation:
    """Test dependency depth calculation algorithm."""

    async def test_root_task_depth_zero(self, db, resolver):
        """Test that tasks with no dependencies have depth 0."""
        task = Task(prompt="Root task", status=TaskStatus.PENDING)
        await db.insert_task(task)

        depth = await resolver.calculate_dependency_depth(task.id)
        assert depth == 0

    async def test_linear_dependency_depths(self, db, resolver):
        """Test depth calculation for linear chain A -> B -> C."""
        # Create tasks
        task_a = Task(prompt="Task A", status=TaskStatus.PENDING)
        task_b = Task(prompt="Task B", status=TaskStatus.PENDING)
        task_c = Task(prompt="Task C", status=TaskStatus.PENDING)

        for task in [task_a, task_b, task_c]:
            await db.insert_task(task)

        # Create dependencies: A -> B -> C
        deps = [
            TaskDependency(
                dependent_task_id=task_b.id,
                prerequisite_task_id=task_a.id,
                dependency_type=DependencyType.SEQUENTIAL,
            ),
            TaskDependency(
                dependent_task_id=task_c.id,
                prerequisite_task_id=task_b.id,
                dependency_type=DependencyType.SEQUENTIAL,
            ),
        ]

        for dep in deps:
            await db.insert_task_dependency(dep)

        # Check depths
        assert await resolver.calculate_dependency_depth(task_a.id) == 0
        assert await resolver.calculate_dependency_depth(task_b.id) == 1
        assert await resolver.calculate_dependency_depth(task_c.id) == 2

    async def test_branching_dependency_depths(self, db, resolver):
        """Test depth calculation for branching A -> B,C."""
        # Create tasks
        task_a = Task(prompt="Task A", status=TaskStatus.PENDING)
        task_b = Task(prompt="Task B", status=TaskStatus.PENDING)
        task_c = Task(prompt="Task C", status=TaskStatus.PENDING)

        for task in [task_a, task_b, task_c]:
            await db.insert_task(task)

        # Create dependencies: A -> B, A -> C
        deps = [
            TaskDependency(
                dependent_task_id=task_b.id,
                prerequisite_task_id=task_a.id,
                dependency_type=DependencyType.SEQUENTIAL,
            ),
            TaskDependency(
                dependent_task_id=task_c.id,
                prerequisite_task_id=task_a.id,
                dependency_type=DependencyType.SEQUENTIAL,
            ),
        ]

        for dep in deps:
            await db.insert_task_dependency(dep)

        # Check depths
        assert await resolver.calculate_dependency_depth(task_a.id) == 0
        assert await resolver.calculate_dependency_depth(task_b.id) == 1
        assert await resolver.calculate_dependency_depth(task_c.id) == 1

    async def test_resolved_dependencies_ignored(self, db, resolver):
        """Test that resolved dependencies don't affect depth calculation."""
        # Create tasks
        task_a = Task(prompt="Task A", status=TaskStatus.COMPLETED)
        task_b = Task(prompt="Task B", status=TaskStatus.PENDING)

        await db.insert_task(task_a)
        await db.insert_task(task_b)

        # Create dependency: A -> B
        dep = TaskDependency(
            dependent_task_id=task_b.id,
            prerequisite_task_id=task_a.id,
            dependency_type=DependencyType.SEQUENTIAL,
            resolved_at=datetime.now(timezone.utc),  # Already resolved
        )
        await db.insert_task_dependency(dep)

        # B should have depth 0 since its dependency is resolved
        depth = await resolver.calculate_dependency_depth(task_b.id)
        assert depth == 0


@pytest.mark.asyncio
class TestTopologicalSort:
    """Test topological sort (Kahn's algorithm) implementation."""

    async def test_topological_sort_linear(self, db, resolver):
        """Test topological sort for linear chain A -> B -> C."""
        # Create tasks
        task_a = Task(prompt="Task A", status=TaskStatus.PENDING)
        task_b = Task(prompt="Task B", status=TaskStatus.PENDING)
        task_c = Task(prompt="Task C", status=TaskStatus.PENDING)

        for task in [task_a, task_b, task_c]:
            await db.insert_task(task)

        # Create dependencies: A -> B -> C
        deps = [
            TaskDependency(
                dependent_task_id=task_b.id,
                prerequisite_task_id=task_a.id,
                dependency_type=DependencyType.SEQUENTIAL,
            ),
            TaskDependency(
                dependent_task_id=task_c.id,
                prerequisite_task_id=task_b.id,
                dependency_type=DependencyType.SEQUENTIAL,
            ),
        ]

        for dep in deps:
            await db.insert_task_dependency(dep)

        # Get execution order
        order = await resolver.get_execution_order([task_a.id, task_b.id, task_c.id])

        # Should be [A, B, C]
        assert order == [task_a.id, task_b.id, task_c.id]

    async def test_topological_sort_diamond(self, db, resolver):
        """Test topological sort for diamond pattern A -> B,C -> D."""
        # Create tasks
        task_a = Task(prompt="Task A", status=TaskStatus.PENDING)
        task_b = Task(prompt="Task B", status=TaskStatus.PENDING)
        task_c = Task(prompt="Task C", status=TaskStatus.PENDING)
        task_d = Task(prompt="Task D", status=TaskStatus.PENDING)

        for task in [task_a, task_b, task_c, task_d]:
            await db.insert_task(task)

        # Create diamond: A -> B,C -> D
        deps = [
            TaskDependency(
                dependent_task_id=task_b.id,
                prerequisite_task_id=task_a.id,
                dependency_type=DependencyType.SEQUENTIAL,
            ),
            TaskDependency(
                dependent_task_id=task_c.id,
                prerequisite_task_id=task_a.id,
                dependency_type=DependencyType.SEQUENTIAL,
            ),
            TaskDependency(
                dependent_task_id=task_d.id,
                prerequisite_task_id=task_b.id,
                dependency_type=DependencyType.PARALLEL,
            ),
            TaskDependency(
                dependent_task_id=task_d.id,
                prerequisite_task_id=task_c.id,
                dependency_type=DependencyType.PARALLEL,
            ),
        ]

        for dep in deps:
            await db.insert_task_dependency(dep)

        # Get execution order
        order = await resolver.get_execution_order([task_a.id, task_b.id, task_c.id, task_d.id])

        # A must come first, D must come last
        # B and C can be in any order between A and D
        assert order[0] == task_a.id
        assert order[-1] == task_d.id
        assert set(order[1:3]) == {task_b.id, task_c.id}

    async def test_topological_sort_with_cycle_fails(self, db, resolver):
        """Test that topological sort raises error for cyclic graph."""
        # Create tasks
        task_a = Task(prompt="Task A", status=TaskStatus.PENDING)
        task_b = Task(prompt="Task B", status=TaskStatus.PENDING)

        for task in [task_a, task_b]:
            await db.insert_task(task)

        # Create cycle: A -> B -> A
        deps = [
            TaskDependency(
                dependent_task_id=task_b.id,
                prerequisite_task_id=task_a.id,
                dependency_type=DependencyType.SEQUENTIAL,
            ),
            TaskDependency(
                dependent_task_id=task_a.id,
                prerequisite_task_id=task_b.id,
                dependency_type=DependencyType.SEQUENTIAL,
            ),
        ]

        for dep in deps:
            await db.insert_task_dependency(dep)

        # Should raise CircularDependencyError
        with pytest.raises(CircularDependencyError):
            await resolver.get_execution_order([task_a.id, task_b.id])

    async def test_topological_sort_empty_list(self, db, resolver):
        """Test topological sort with empty task list."""
        order = await resolver.get_execution_order([])
        assert order == []

    async def test_topological_sort_single_task(self, db, resolver):
        """Test topological sort with single task."""
        task = Task(prompt="Task A", status=TaskStatus.PENDING)
        await db.insert_task(task)

        order = await resolver.get_execution_order([task.id])
        assert order == [task.id]


@pytest.mark.asyncio
class TestDependencyValidation:
    """Test dependency validation methods."""

    async def test_validate_new_dependency_valid(self, db, resolver):
        """Test validation of valid dependency."""
        task_a = Task(prompt="Task A", status=TaskStatus.PENDING)
        task_b = Task(prompt="Task B", status=TaskStatus.PENDING)

        await db.insert_task(task_a)
        await db.insert_task(task_b)

        # B -> A is valid (no cycle)
        is_valid = await resolver.validate_new_dependency(task_b.id, task_a.id)
        assert is_valid is True

    async def test_validate_new_dependency_invalid(self, db, resolver):
        """Test validation of invalid dependency (would create cycle)."""
        task_a = Task(prompt="Task A", status=TaskStatus.PENDING)
        task_b = Task(prompt="Task B", status=TaskStatus.PENDING)

        await db.insert_task(task_a)
        await db.insert_task(task_b)

        # Create A -> B
        dep = TaskDependency(
            dependent_task_id=task_b.id,
            prerequisite_task_id=task_a.id,
            dependency_type=DependencyType.SEQUENTIAL,
        )
        await db.insert_task_dependency(dep)

        # B -> A would create cycle
        is_valid = await resolver.validate_new_dependency(task_a.id, task_b.id)
        assert is_valid is False


@pytest.mark.asyncio
class TestUnmetDependencies:
    """Test unmet dependency detection."""

    async def test_get_unmet_dependencies(self, db, resolver):
        """Test getting unmet dependencies."""
        # Create tasks
        task_a = Task(prompt="Task A", status=TaskStatus.COMPLETED)
        task_b = Task(prompt="Task B", status=TaskStatus.PENDING)
        task_c = Task(prompt="Task C", status=TaskStatus.RUNNING)

        for task in [task_a, task_b, task_c]:
            await db.insert_task(task)

        # Check which are unmet
        unmet = await resolver.get_unmet_dependencies([task_a.id, task_b.id, task_c.id])

        # A is completed, so B and C are unmet
        assert set(unmet) == {task_b.id, task_c.id}

    async def test_get_unmet_dependencies_empty(self, db, resolver):
        """Test getting unmet dependencies with empty list."""
        unmet = await resolver.get_unmet_dependencies([])
        assert unmet == []

    async def test_are_all_dependencies_met_true(self, db, resolver):
        """Test are_all_dependencies_met returns True when all met."""
        task_a = Task(prompt="Task A", status=TaskStatus.COMPLETED)
        task_b = Task(prompt="Task B", status=TaskStatus.PENDING)

        await db.insert_task(task_a)
        await db.insert_task(task_b)

        # Create dependency: A -> B (resolved)
        dep = TaskDependency(
            dependent_task_id=task_b.id,
            prerequisite_task_id=task_a.id,
            dependency_type=DependencyType.SEQUENTIAL,
            resolved_at=datetime.now(timezone.utc),
        )
        await db.insert_task_dependency(dep)

        # All dependencies met
        all_met = await resolver.are_all_dependencies_met(task_b.id)
        assert all_met is True

    async def test_are_all_dependencies_met_false(self, db, resolver):
        """Test are_all_dependencies_met returns False when unmet."""
        task_a = Task(prompt="Task A", status=TaskStatus.PENDING)
        task_b = Task(prompt="Task B", status=TaskStatus.PENDING)

        await db.insert_task(task_a)
        await db.insert_task(task_b)

        # Create dependency: A -> B (unresolved)
        dep = TaskDependency(
            dependent_task_id=task_b.id,
            prerequisite_task_id=task_a.id,
            dependency_type=DependencyType.SEQUENTIAL,
        )
        await db.insert_task_dependency(dep)

        # Dependencies not met
        all_met = await resolver.are_all_dependencies_met(task_b.id)
        assert all_met is False


@pytest.mark.asyncio
class TestGraphCaching:
    """Test graph caching functionality."""

    async def test_graph_cache_ttl(self, db, resolver):
        """Test that graph cache expires after TTL."""
        # Set very short TTL
        resolver._cache_ttl = timedelta(milliseconds=100)

        # Create task
        task = Task(prompt="Task A", status=TaskStatus.PENDING)
        await db.insert_task(task)

        # First call - cache miss
        graph1 = await resolver._build_dependency_graph()
        assert resolver._graph_cache is not None

        # Second call immediately - cache hit
        graph2 = await resolver._build_dependency_graph()
        assert graph1 == graph2

        # Wait for TTL to expire
        import asyncio

        await asyncio.sleep(0.15)

        # Third call - cache expired, rebuild
        graph3 = await resolver._build_dependency_graph()
        assert graph3 is not None

    async def test_invalidate_cache(self, db, resolver):
        """Test manual cache invalidation."""
        task = Task(prompt="Task A", status=TaskStatus.PENDING)
        await db.insert_task(task)

        # Build cache
        await resolver._build_dependency_graph()
        assert resolver._graph_cache is not None

        # Invalidate
        resolver.invalidate_cache()
        assert resolver._graph_cache is None
        assert resolver._cache_timestamp is None
        assert len(resolver._depth_cache) == 0

    async def test_get_ready_tasks(self, db, resolver):
        """Test getting ready tasks (all dependencies met)."""
        # Create tasks
        task_a = Task(prompt="Task A", status=TaskStatus.COMPLETED)
        task_b = Task(prompt="Task B", status=TaskStatus.PENDING)
        task_c = Task(prompt="Task C", status=TaskStatus.PENDING)

        for task in [task_a, task_b, task_c]:
            await db.insert_task(task)

        # B depends on A (resolved)
        dep_b = TaskDependency(
            dependent_task_id=task_b.id,
            prerequisite_task_id=task_a.id,
            dependency_type=DependencyType.SEQUENTIAL,
            resolved_at=datetime.now(timezone.utc),
        )
        await db.insert_task_dependency(dep_b)

        # C depends on B (unresolved)
        dep_c = TaskDependency(
            dependent_task_id=task_c.id,
            prerequisite_task_id=task_b.id,
            dependency_type=DependencyType.SEQUENTIAL,
        )
        await db.insert_task_dependency(dep_c)

        # Get ready tasks
        ready = await resolver.get_ready_tasks([task_b.id, task_c.id])

        # Only B is ready (A is resolved), C is not ready (B is unresolved)
        assert ready == [task_b.id]


@pytest.mark.asyncio
class TestBlockedTasksAndChains:
    """Test blocked tasks and dependency chain methods."""

    async def test_get_blocked_tasks(self, db, resolver):
        """Test getting tasks blocked by a prerequisite."""
        # Create tasks
        task_a = Task(prompt="Task A", status=TaskStatus.PENDING)
        task_b = Task(prompt="Task B", status=TaskStatus.PENDING)
        task_c = Task(prompt="Task C", status=TaskStatus.PENDING)

        for task in [task_a, task_b, task_c]:
            await db.insert_task(task)

        # B and C depend on A
        deps = [
            TaskDependency(
                dependent_task_id=task_b.id,
                prerequisite_task_id=task_a.id,
                dependency_type=DependencyType.SEQUENTIAL,
            ),
            TaskDependency(
                dependent_task_id=task_c.id,
                prerequisite_task_id=task_a.id,
                dependency_type=DependencyType.SEQUENTIAL,
            ),
        ]

        for dep in deps:
            await db.insert_task_dependency(dep)

        # Get tasks blocked by A
        blocked = await resolver.get_blocked_tasks(task_a.id)
        assert set(blocked) == {task_b.id, task_c.id}

    async def test_get_dependency_chain(self, db, resolver):
        """Test getting full dependency chain."""
        # Create tasks: A -> B -> C
        task_a = Task(prompt="Task A", status=TaskStatus.PENDING)
        task_b = Task(prompt="Task B", status=TaskStatus.PENDING)
        task_c = Task(prompt="Task C", status=TaskStatus.PENDING)

        for task in [task_a, task_b, task_c]:
            await db.insert_task(task)

        # Create dependencies
        deps = [
            TaskDependency(
                dependent_task_id=task_b.id,
                prerequisite_task_id=task_a.id,
                dependency_type=DependencyType.SEQUENTIAL,
            ),
            TaskDependency(
                dependent_task_id=task_c.id,
                prerequisite_task_id=task_b.id,
                dependency_type=DependencyType.SEQUENTIAL,
            ),
        ]

        for dep in deps:
            await db.insert_task_dependency(dep)

        # Get dependency chain for C
        chain = await resolver.get_dependency_chain(task_c.id)

        # Should have 3 levels: [C], [B], [A]
        assert len(chain) == 3
        assert chain[0] == [task_c.id]
        assert chain[1] == [task_b.id]
        assert chain[2] == [task_a.id]
