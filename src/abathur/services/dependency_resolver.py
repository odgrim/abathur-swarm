"""Dependency resolution service for task queue dependency management.

This module implements graph algorithms for dependency resolution including:
- Circular dependency detection using DFS
- Topological sorting using Kahn's algorithm
- Dependency depth calculation with memoization
- Graph caching with TTL for performance optimization
"""

from collections import defaultdict, deque
from datetime import datetime, timedelta, timezone
from typing import TYPE_CHECKING
from uuid import UUID

from abathur.domain.models import TaskStatus
from abathur.infrastructure.logger import get_logger

if TYPE_CHECKING:
    from abathur.infrastructure.database import Database

logger = get_logger(__name__)


class CircularDependencyError(Exception):
    """Raised when a circular dependency is detected."""

    pass


class DependencyResolver:
    """Handles dependency graph operations and validation.

    This service provides efficient algorithms for:
    - Detecting circular dependencies before task insertion
    - Computing topological orderings for execution planning
    - Calculating dependency depths for hierarchical scheduling
    - Identifying ready tasks with all dependencies met

    Performance targets:
    - Cycle detection: <10ms for 100-task graph
    - Topological sort: <10ms for 100-task graph
    - Depth calculation: <5ms for 10-level graph
    - Cache hit: <1ms
    """

    def __init__(self, database: "Database", cache_ttl_seconds: float = 60.0):
        """Initialize dependency resolver.

        Args:
            database: Database instance for accessing task dependencies
            cache_ttl_seconds: Time-to-live for dependency graph cache (default: 60s)
        """
        self.db = database
        self._graph_cache: dict[UUID, set[UUID]] | None = None
        self._cache_timestamp: datetime | None = None
        self._cache_ttl = timedelta(seconds=cache_ttl_seconds)

        # Memoization cache for depth calculations
        self._depth_cache: dict[UUID, int] = {}

    async def detect_circular_dependencies(
        self, new_dependencies: list[UUID], task_id: UUID | None = None
    ) -> list[list[UUID]]:
        """Detect circular dependencies using depth-first search.

        This method checks if adding the specified dependencies would create
        any circular dependency chains in the task graph. It uses DFS with
        path tracking to identify and report all cycles.

        Args:
            new_dependencies: List of prerequisite task IDs to add
            task_id: ID of the dependent task (None for new tasks)

        Returns:
            List of cycles found (empty list if no cycles).
            Each cycle is represented as a list of task IDs forming the cycle.

        Raises:
            CircularDependencyError: If cycles detected with detailed cycle path

        Performance:
            O(V + E) where V = number of tasks, E = number of dependencies
        """
        # Build current dependency graph
        graph = await self._build_dependency_graph()

        # Add new edges to graph for simulation
        if task_id:
            if task_id not in graph:
                graph[task_id] = set()

            for prereq_id in new_dependencies:
                # Check for self-dependency
                if task_id == prereq_id:
                    raise CircularDependencyError(
                        f"Self-dependency not allowed: task {task_id} cannot depend on itself"
                    )

                # Simulate adding the edge
                if prereq_id not in graph:
                    graph[prereq_id] = set()
                graph[prereq_id].add(task_id)

        # Detect cycles using DFS
        cycles = []
        visited = set()
        rec_stack = set()
        path: list[UUID] = []

        def dfs(node: UUID) -> bool:
            """DFS helper to detect cycles.

            Args:
                node: Current node being visited

            Returns:
                True if cycle detected, False otherwise
            """
            if node in rec_stack:
                # Found a cycle - extract it from path
                cycle_start = path.index(node)
                cycle = path[cycle_start:] + [node]
                cycles.append(cycle)
                return True

            if node in visited:
                return False

            visited.add(node)
            rec_stack.add(node)
            path.append(node)

            for neighbor in graph.get(node, set()):
                if dfs(neighbor):
                    # Continue searching for more cycles
                    pass

            path.pop()
            rec_stack.remove(node)
            return False

        # Check all nodes for cycles
        for node in graph:
            if node not in visited:
                dfs(node)

        if cycles:
            # Format error message with cycle details
            cycle_strs = []
            for cycle in cycles:
                cycle_str = " -> ".join(str(tid) for tid in cycle)
                cycle_strs.append(cycle_str)

            error_msg = "Circular dependency detected. Cycles found:\n" + "\n".join(
                f"  - {cs}" for cs in cycle_strs
            )
            raise CircularDependencyError(error_msg)

        return cycles

    async def calculate_dependency_depth(self, task_id: UUID) -> int:
        """Calculate maximum depth from root tasks.

        Root tasks (tasks with no prerequisites) have depth 0.
        Each level of dependency adds 1 to the depth.
        Uses memoization for efficient repeated calculations.

        Args:
            task_id: Task ID to calculate depth for

        Returns:
            Depth level (0 = no dependencies, 1 = depends on root tasks, etc.)

        Performance:
            O(V + E) with memoization, amortized O(1) for cached results
        """
        # Check memoization cache
        if task_id in self._depth_cache:
            return self._depth_cache[task_id]

        # Get task dependencies
        dependencies = await self.db.get_task_dependencies(task_id)

        # Filter unresolved dependencies
        unresolved_deps = [
            dep.prerequisite_task_id for dep in dependencies if dep.resolved_at is None
        ]

        if not unresolved_deps:
            # Root task - no dependencies
            depth = 0
        else:
            # Recursive depth calculation
            max_prereq_depth = 0
            for prereq_id in unresolved_deps:
                prereq_depth = await self.calculate_dependency_depth(prereq_id)
                max_prereq_depth = max(max_prereq_depth, prereq_depth)

            depth = max_prereq_depth + 1

        # Cache the result
        self._depth_cache[task_id] = depth
        return depth

    async def get_execution_order(self, task_ids: list[UUID]) -> list[UUID]:
        """Return topological sort of tasks using Kahn's algorithm.

        This method computes a valid execution order for the given tasks
        that respects all dependency relationships. Tasks with no dependencies
        appear first, followed by tasks whose dependencies have been satisfied.

        Args:
            task_ids: List of task IDs to sort

        Returns:
            List of task IDs in execution order (topologically sorted)

        Raises:
            CircularDependencyError: If graph contains cycles

        Performance:
            O(V + E) where V = number of tasks, E = number of dependencies
        """
        if not task_ids:
            return []

        # Build subgraph for requested tasks
        graph: dict[UUID, set[UUID]] = defaultdict(set)
        in_degree: dict[UUID, int] = defaultdict(int)

        # Initialize all task IDs
        for task_id in task_ids:
            if task_id not in in_degree:
                in_degree[task_id] = 0

        # Build adjacency list and in-degree map
        for task_id in task_ids:
            dependencies = await self.db.get_task_dependencies(task_id)

            for dep in dependencies:
                if dep.resolved_at is None and dep.prerequisite_task_id in task_ids:
                    # Edge: prerequisite -> dependent
                    prereq = dep.prerequisite_task_id
                    dependent = dep.dependent_task_id

                    graph[prereq].add(dependent)
                    in_degree[dependent] += 1

                    # Ensure prerequisite is in the map
                    if prereq not in in_degree:
                        in_degree[prereq] = 0

        # Kahn's algorithm: Start with nodes that have no dependencies
        queue = deque([task_id for task_id in task_ids if in_degree[task_id] == 0])
        result = []

        while queue:
            node = queue.popleft()
            result.append(node)

            # Process neighbors
            for neighbor in graph.get(node, set()):
                in_degree[neighbor] -= 1
                if in_degree[neighbor] == 0:
                    queue.append(neighbor)

        # Check if all tasks were processed
        if len(result) != len(task_ids):
            # Cycle detected - not all nodes could be processed
            unprocessed = set(task_ids) - set(result)
            raise CircularDependencyError(
                f"Cannot create execution order: circular dependencies detected. "
                f"Unprocessed tasks: {unprocessed}"
            )

        return result

    async def validate_new_dependency(self, task_id: UUID, depends_on_task_id: UUID) -> bool:
        """Check if adding a dependency would create a cycle.

        This is a lightweight check that can be called before inserting
        a dependency to ensure it won't create circular dependencies.

        Args:
            task_id: The dependent task ID
            depends_on_task_id: The prerequisite task ID

        Returns:
            True if dependency is valid, False if would create cycle
        """
        try:
            await self.detect_circular_dependencies([depends_on_task_id], task_id)
            return True
        except CircularDependencyError:
            return False

    async def get_unmet_dependencies(self, dependency_ids: list[UUID]) -> list[UUID]:
        """Get dependencies that haven't completed yet.

        Args:
            dependency_ids: List of task IDs to check

        Returns:
            List of task IDs that are not in COMPLETED status

        Performance:
            O(N) database query with index usage
        """
        if not dependency_ids:
            return []

        async with self.db._get_connection() as conn:
            placeholders = ",".join(["?" for _ in dependency_ids])
            cursor = await conn.execute(
                f"""
                SELECT id FROM tasks
                WHERE id IN ({placeholders})
                AND status NOT IN (?, ?)
                """,
                [str(dep_id) for dep_id in dependency_ids]
                + [TaskStatus.COMPLETED.value, TaskStatus.CANCELLED.value],
            )
            rows = await cursor.fetchall()
            return [UUID(row[0]) for row in rows]

    async def are_all_dependencies_met(self, task_id: UUID) -> bool:
        """Check if all dependencies for a task are met.

        A dependency is considered met if it has been resolved (resolved_at is set).

        Args:
            task_id: Task ID to check

        Returns:
            True if all dependencies resolved, False otherwise

        Performance:
            O(1) - single indexed database query
        """
        async with self.db._get_connection() as conn:
            cursor = await conn.execute(
                """
                SELECT COUNT(*) FROM task_dependencies
                WHERE dependent_task_id = ? AND resolved_at IS NULL
                """,
                (str(task_id),),
            )
            row = await cursor.fetchone()
            unmet_count: int = row[0] if row else 0

        return unmet_count == 0

    async def get_ready_tasks(self, task_ids: list[UUID]) -> list[UUID]:
        """Get tasks that have all prerequisites completed.

        Args:
            task_ids: List of task IDs to filter

        Returns:
            List of task IDs that are ready for execution
        """
        ready_tasks = []

        for task_id in task_ids:
            if await self.are_all_dependencies_met(task_id):
                ready_tasks.append(task_id)

        return ready_tasks

    async def _build_dependency_graph(self) -> dict[UUID, set[UUID]]:
        """Build adjacency list from task_dependencies table.

        Uses caching with TTL to minimize database queries.
        Cache represents: prerequisite_task_id -> set of dependent_task_ids

        Returns:
            Graph as adjacency list

        Performance:
            O(E) where E = number of dependencies (on cache miss)
            O(1) on cache hit
        """
        # Check cache validity
        now = datetime.now(timezone.utc)

        if (
            self._graph_cache is not None
            and self._cache_timestamp is not None
            and (now - self._cache_timestamp) < self._cache_ttl
        ):
            logger.debug("Dependency graph cache hit")
            return self._graph_cache

        # Cache miss - rebuild graph
        logger.debug("Dependency graph cache miss - rebuilding")

        async with self.db._get_connection() as conn:
            cursor = await conn.execute(
                """
                SELECT dependent_task_id, prerequisite_task_id
                FROM task_dependencies
                WHERE resolved_at IS NULL
                """
            )
            rows = await cursor.fetchall()

        # Build adjacency list
        graph: dict[UUID, set[UUID]] = defaultdict(set)

        for row in rows:
            dependent = UUID(row[0])
            prerequisite = UUID(row[1])

            # Edge: prerequisite -> dependent
            if prerequisite not in graph:
                graph[prerequisite] = set()
            graph[prerequisite].add(dependent)

            # Ensure dependent exists in graph
            if dependent not in graph:
                graph[dependent] = set()

        # Update cache
        self._graph_cache = dict(graph)
        self._cache_timestamp = now

        logger.debug(f"Dependency graph rebuilt: {len(graph)} nodes")
        return self._graph_cache

    def invalidate_cache(self) -> None:
        """Invalidate graph cache after dependency updates.

        Call this method after inserting, updating, or deleting dependencies
        to ensure the cache reflects the latest state.
        """
        self._graph_cache = None
        self._cache_timestamp = None
        self._depth_cache.clear()
        logger.debug("Dependency graph cache invalidated")

    async def get_blocked_tasks(self, prerequisite_task_id: UUID) -> list[UUID]:
        """Get tasks that are blocked waiting for a specific prerequisite.

        Args:
            prerequisite_task_id: The prerequisite task ID

        Returns:
            List of task IDs blocked by this prerequisite
        """
        async with self.db._get_connection() as conn:
            cursor = await conn.execute(
                """
                SELECT dependent_task_id FROM task_dependencies
                WHERE prerequisite_task_id = ? AND resolved_at IS NULL
                """,
                (str(prerequisite_task_id),),
            )
            rows = await cursor.fetchall()
            return [UUID(row[0]) for row in rows]

    async def get_dependency_chain(self, task_id: UUID) -> list[list[UUID]]:
        """Get full dependency chain for a task.

        Returns the dependency chain as a list of levels, where each level
        contains tasks at the same dependency depth.

        Args:
            task_id: Root task ID

        Returns:
            List of levels, each containing task IDs at that depth
        """
        # Get all tasks in dependency chain
        visited = set()
        levels: dict[int, list[UUID]] = defaultdict(list)

        async def traverse(tid: UUID, depth: int = 0) -> None:
            if tid in visited:
                return

            visited.add(tid)
            levels[depth].append(tid)

            # Get dependencies
            dependencies = await self.db.get_task_dependencies(tid)
            for dep in dependencies:
                if dep.resolved_at is None:
                    await traverse(dep.prerequisite_task_id, depth + 1)

        await traverse(task_id)

        # Convert to sorted list of levels
        max_depth = max(levels.keys()) if levels else 0
        return [levels[i] for i in range(max_depth + 1)]
