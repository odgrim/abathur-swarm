# Phase 2 Context Document - Dependency Resolution

**Project:** Abathur Enhanced Task Queue System
**Phase:** 2 - Dependency Resolution Algorithms
**Agent:** algorithm-design-specialist
**Date:** 2025-10-10
**Status:** Ready to Begin

---

## Phase 2 Objectives

Implement the dependency resolution layer that enables:
1. Circular dependency detection before task insertion
2. Dependency graph construction and traversal
3. Unmet dependency checking
4. Topological sorting for execution order planning
5. Performance-optimized graph operations

---

## Phase 1 Completion Summary

**Status:** APPROVED - All deliverables complete

**What Was Delivered:**
- Enhanced Task model with dependency fields
- TaskDependency model for dependency relationships
- task_dependencies table in database
- Database helper methods: insert_task_dependency, get_task_dependencies, resolve_dependency
- 6 performance indexes for dependency queries
- 100% test coverage on domain models
- All 13 validation checks passing

**Database Operations Available:**
```python
# From /Users/odgrim/dev/home/agentics/abathur/src/abathur/infrastructure/database.py
await db.insert_task_dependency(dependency: TaskDependency) -> None
await db.get_task_dependencies(task_id: UUID) -> list[TaskDependency]
await db.resolve_dependency(prerequisite_task_id: UUID) -> None
```

**Database Schema:**
```sql
CREATE TABLE task_dependencies (
    id TEXT PRIMARY KEY,
    dependent_task_id TEXT NOT NULL,
    prerequisite_task_id TEXT NOT NULL,
    dependency_type TEXT NOT NULL DEFAULT 'sequential',
    created_at TIMESTAMP NOT NULL,
    resolved_at TIMESTAMP,
    FOREIGN KEY (dependent_task_id) REFERENCES tasks(id) ON DELETE CASCADE,
    FOREIGN KEY (prerequisite_task_id) REFERENCES tasks(id) ON DELETE CASCADE,
    CHECK(dependency_type IN ('sequential', 'parallel')),
    CHECK(dependent_task_id != prerequisite_task_id),
    UNIQUE(dependent_task_id, prerequisite_task_id)
)
```

**Indexes Available:**
- idx_task_dependencies_prerequisite (prerequisite_task_id, resolved_at WHERE resolved_at IS NULL)
- idx_task_dependencies_dependent (dependent_task_id, resolved_at WHERE resolved_at IS NULL)

---

## Phase 2 Deliverables

### 1. DependencyResolver Service

**Location:** `/Users/odgrim/dev/home/agentics/abathur/src/abathur/services/dependency_resolver.py`

**Class Structure:**
```python
class DependencyResolver:
    """Handles dependency graph operations and validation."""

    def __init__(self, database: Database):
        self.db = database
        self._graph_cache: dict[UUID, list[UUID]] | None = None
        self._cache_timestamp: datetime | None = None

    async def detect_circular_dependencies(
        self,
        new_dependencies: list[UUID],
        task_id: UUID | None = None
    ) -> list[list[UUID]]:
        """Detect circular dependencies using DFS.

        Returns:
            List of cycles found (empty list if no cycles)
            Each cycle is a list of task IDs forming the cycle

        Raises:
            CircularDependencyError: If cycles detected
        """
        pass

    async def calculate_dependency_depth(self, task_id: UUID) -> int:
        """Calculate max depth from root tasks (tasks with no dependencies).

        Returns:
            Depth level (0 = no dependencies, 1 = depends on root tasks, etc.)
        """
        pass

    async def get_execution_order(self, task_ids: list[UUID]) -> list[UUID]:
        """Return topological sort of tasks.

        Uses Kahn's algorithm for topological sorting.

        Returns:
            List of task IDs in execution order

        Raises:
            CircularDependencyError: If graph contains cycles
        """
        pass

    async def validate_new_dependency(
        self,
        task_id: UUID,
        depends_on_task_id: UUID
    ) -> bool:
        """Check if adding dependency would create a cycle.

        Returns:
            True if dependency is valid, False if would create cycle
        """
        pass

    async def get_unmet_dependencies(self, dependency_ids: list[UUID]) -> list[UUID]:
        """Get dependencies that haven't completed yet.

        Returns:
            List of task IDs that are not in COMPLETED status
        """
        pass

    async def are_all_dependencies_met(self, task_id: UUID) -> bool:
        """Check if all dependencies for a task are resolved.

        Returns:
            True if all dependencies resolved, False otherwise
        """
        pass

    async def _build_dependency_graph(self) -> dict[UUID, list[UUID]]:
        """Build adjacency list from task_dependencies table.

        Uses caching with 60-second TTL to minimize database queries.

        Returns:
            Graph as adjacency list: {prerequisite_task_id: [dependent_task_ids]}
        """
        pass

    def _invalidate_cache(self) -> None:
        """Invalidate graph cache after dependency updates."""
        self._graph_cache = None
        self._cache_timestamp = None
```

### 2. Circular Dependency Detection Algorithm

**Algorithm:** Depth-First Search (DFS) with visited/recursion stack tracking

**Pseudocode:**
```
function detect_cycle(graph, start_node):
    visited = set()
    rec_stack = set()

    function dfs(node):
        if node in rec_stack:
            return True  # Cycle detected
        if node in visited:
            return False

        visited.add(node)
        rec_stack.add(node)

        for neighbor in graph[node]:
            if dfs(neighbor):
                return True

        rec_stack.remove(node)
        return False

    return dfs(start_node)
```

**Performance Target:** <10ms for 100-task graph

### 3. Topological Sort Implementation

**Algorithm:** Kahn's Algorithm

**Pseudocode:**
```
function topological_sort(graph):
    in_degree = compute_in_degrees(graph)
    queue = [node for node in graph if in_degree[node] == 0]
    result = []

    while queue:
        node = queue.pop(0)
        result.append(node)

        for neighbor in graph[node]:
            in_degree[neighbor] -= 1
            if in_degree[neighbor] == 0:
                queue.append(neighbor)

    if len(result) != len(graph):
        raise CircularDependencyError("Graph contains cycle")

    return result
```

**Performance Target:** <10ms for 100-task graph

### 4. Test Files

#### Unit Tests
**Location:** `/Users/odgrim/dev/home/agentics/abathur/tests/unit/services/test_dependency_resolver.py`

**Test Cases:**
- test_detect_simple_cycle - A -> B -> A
- test_detect_complex_cycle - A -> B -> C -> A
- test_detect_no_cycle - A -> B -> C (linear)
- test_topological_sort_linear - Single path
- test_topological_sort_branching - Diamond pattern
- test_topological_sort_parallel - Multiple independent paths
- test_depth_calculation_single_level - Depth = 1
- test_depth_calculation_multi_level - Depth = 3
- test_validate_dependency_valid - No cycle created
- test_validate_dependency_invalid - Would create cycle
- test_unmet_dependencies - Mix of completed/pending
- test_all_dependencies_met_true
- test_all_dependencies_met_false

#### Performance Tests
**Location:** `/Users/odgrim/dev/home/agentics/abathur/tests/performance/test_dependency_resolver_performance.py`

**Benchmarks:**
- test_cycle_detection_100_tasks - <10ms target
- test_topological_sort_100_tasks - <10ms target
- test_depth_calculation_10_levels - <20ms target
- test_graph_build_1000_tasks - <50ms target

---

## Architecture Context

### Relevant Architecture Sections

**From TASK_QUEUE_ARCHITECTURE.md:**
- Section 5.2: DependencyResolver service design
- Section 9: Performance targets

**From TASK_QUEUE_DECISION_POINTS.md:**
- Decision 2: MAX_DEPENDENCIES_PER_TASK = 50, MAX_DEPENDENCY_DEPTH = 10
- Decision 5: Reject circular dependencies (fail fast)
- Decision 8: PARALLEL = AND logic (wait for all)

### Integration Points

**Database Layer:**
- Use `db.get_task_dependencies()` to fetch dependencies
- Query unresolved dependencies: `WHERE resolved_at IS NULL`
- Leverage indexes: idx_task_dependencies_prerequisite, idx_task_dependencies_dependent

**Domain Models:**
- TaskDependency model available at `/Users/odgrim/dev/home/agentics/abathur/src/abathur/domain/models.py`
- DependencyType enum: SEQUENTIAL, PARALLEL

**Error Handling:**
- Raise CircularDependencyError when cycles detected
- Provide clear error messages with cycle path

---

## Performance Requirements

### Targets

| Operation | Target | Measurement |
|-----------|--------|-------------|
| Circular detection | <10ms | 100-task graph |
| Topological sort | <10ms | 100-task graph |
| Depth calculation | <20ms | 10-level deep graph |
| Graph building | <50ms | 1000-task database |
| Unmet dep check | <5ms | Single query |

### Optimization Strategies

1. **Graph Caching:** Cache dependency graph with 60-second TTL
2. **Incremental Updates:** Invalidate cache only when dependencies change
3. **Index Usage:** Ensure all queries use idx_task_dependencies_* indexes
4. **Batch Operations:** Fetch all dependencies in single query
5. **Early Termination:** Stop DFS immediately when cycle detected

---

## Test Strategy

### Unit Test Coverage Requirements

- Cycle detection: Simple, complex, transitive cycles
- Topological sort: Linear, branching, diamond, parallel patterns
- Depth calculation: Single level, multi-level, maximum depth
- Edge cases: Empty graph, single node, disconnected components
- Error handling: Invalid task IDs, missing dependencies

**Target:** >80% code coverage

### Integration Test Requirements

- End-to-end workflow: Submit task with dependencies -> validate -> insert
- Concurrent dependency checks (multiple agents)
- Graph cache invalidation on updates
- Database constraint enforcement (CHECK, UNIQUE)

**Target:** 100% of critical workflows covered

### Performance Test Requirements

- Benchmark all operations against targets
- Test with 10, 50, 100, 1000 task graphs
- Measure with cold cache and warm cache
- Profile slow operations with cProfile

**Target:** All performance targets met

---

## Implementation Guidelines

### Code Quality Standards

1. **Type Annotations:** All public methods fully typed
2. **Docstrings:** Google style with Args, Returns, Raises
3. **Error Messages:** Include cycle path in CircularDependencyError
4. **Logging:** Log graph cache hits/misses, cycle detection attempts
5. **Constants:** MAX_DEPENDENCIES_PER_TASK = 50, MAX_DEPENDENCY_DEPTH = 10

### Validation Checks

```python
# Validate before processing
if len(new_dependencies) > MAX_DEPENDENCIES_PER_TASK:
    raise ValueError(f"Exceeds max dependencies: {MAX_DEPENDENCIES_PER_TASK}")

if depth > MAX_DEPENDENCY_DEPTH:
    raise ValueError(f"Exceeds max depth: {MAX_DEPENDENCY_DEPTH}")
```

### Example Usage

```python
# Example: Validate and insert dependencies
resolver = DependencyResolver(database)

# Check for cycles before insert
try:
    cycles = await resolver.detect_circular_dependencies([task_b.id], task_a.id)
    if cycles:
        raise CircularDependencyError(f"Cycle detected: {cycles[0]}")
except CircularDependencyError as e:
    logger.error(f"Cannot add dependency: {e}")
    return

# Insert dependency
dependency = TaskDependency(
    dependent_task_id=task_a.id,
    prerequisite_task_id=task_b.id,
    dependency_type=DependencyType.SEQUENTIAL
)
await database.insert_task_dependency(dependency)
resolver._invalidate_cache()
```

---

## Acceptance Criteria

Phase 2 will be considered complete when:

1. DependencyResolver service implemented with all methods
2. Circular dependency detection works correctly (all test cases pass)
3. Topological sort returns correct execution order
4. Depth calculation handles all edge cases
5. All unit tests pass (>80% coverage)
6. All integration tests pass
7. All performance tests meet targets (<10ms for 100-task graph)
8. Code review passes (type annotations, docstrings, error handling)
9. Documentation complete (algorithm explanations, usage examples)

---

## Next Steps

1. Read architecture documents:
   - `/Users/odgrim/dev/home/agentics/abathur/design_docs/TASK_QUEUE_ARCHITECTURE.md`
   - `/Users/odgrim/dev/home/agentics/abathur/design_docs/TASK_QUEUE_DECISION_POINTS.md`

2. Review Phase 1 implementation:
   - `/Users/odgrim/dev/home/agentics/abathur/src/abathur/domain/models.py`
   - `/Users/odgrim/dev/home/agentics/abathur/src/abathur/infrastructure/database.py`

3. Implement DependencyResolver service:
   - Create `/Users/odgrim/dev/home/agentics/abathur/src/abathur/services/dependency_resolver.py`

4. Write unit tests:
   - Create `/Users/odgrim/dev/home/agentics/abathur/tests/unit/services/test_dependency_resolver.py`

5. Write performance tests:
   - Create `/Users/odgrim/dev/home/agentics/abathur/tests/performance/test_dependency_resolver_performance.py`

6. Run validation:
   - Execute all tests
   - Verify performance targets met
   - Generate Phase 2 completion report

---

**Context prepared by:** task-queue-orchestrator
**Date:** 2025-10-10
**Status:** Ready for algorithm-design-specialist invocation

---
