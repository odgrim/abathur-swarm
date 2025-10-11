# Phase 4 Implementation Context - Task Queue Service

**Project:** Abathur Enhanced Task Queue System
**Phase:** Phase 4 - Task Queue Service Implementation
**Agent:** python-backend-developer
**Date:** 2025-10-10
**Status:** READY TO BEGIN

---

## Phase Overview

**Objective:** Implement the TaskQueueService that integrates all Phase 1-3 components (schema, dependency resolution, priority calculation) into a cohesive task queue with dependency management and priority-based scheduling.

**Duration:** 3 days (estimated)

**Prerequisites:**
- ✓ Phase 1: Schema & Domain Models - APPROVED
- ✓ Phase 2: Dependency Resolution - APPROVED
- ✓ Phase 3: Priority Calculation - APPROVED

---

## Deliverables

### 1. Enhanced TaskQueueService Implementation

**File:** `/Users/odgrim/dev/home/agentics/abathur/src/abathur/services/task_queue_service.py`

**Requirements:**
- NEW service class that coordinates all Phase 1-3 components
- Integrates DependencyResolver and PriorityCalculator
- Implements 7 core methods (enqueue, dequeue, complete, fail, cancel, status, execution plan)
- Transaction management for atomic operations
- Comprehensive error handling and logging
- Async/await throughout for non-blocking operation

### 2. Core Methods to Implement

#### Method 1: `enqueue_task`

```python
async def enqueue_task(
    self,
    description: str,
    source: TaskSource,
    parent_task_id: str | None = None,
    prerequisites: list[str] | None = None,
    base_priority: int = 5,
    deadline: datetime | None = None,
    estimated_duration_seconds: int | None = None,
    agent_type: str = "general",
    session_id: str | None = None,
    input_data: dict[str, Any] | None = None,
) -> Task:
    """
    Enqueue a new task with dependency validation and priority calculation.

    Steps:
    1. Validate prerequisites exist in database
    2. Check for circular dependencies (DependencyResolver.validate_new_dependency)
    3. Calculate dependency depth (DependencyResolver.calculate_dependency_depth)
    4. Calculate initial priority (PriorityCalculator.calculate_priority)
    5. Determine initial status:
       - READY if no prerequisites or all already completed
       - BLOCKED if has unmet prerequisites
    6. Insert task into database (atomic transaction)
    7. Insert task dependencies (same transaction)
    8. Return created task

    Raises:
        ValueError: If prerequisites don't exist or circular dependency detected
        DatabaseError: If transaction fails
    """
```

**Validation Logic:**
- Check all prerequisite task IDs exist in database (early validation)
- Use DependencyResolver to check for circular dependencies BEFORE insert
- Validate base_priority in range [0, 10]
- Validate source is valid TaskSource enum value

**Status Determination:**
```python
if not prerequisites:
    initial_status = TaskStatus.READY
else:
    # Check if any prerequisites are not completed
    unmet = await self._get_unmet_prerequisites(prerequisites)
    if unmet:
        initial_status = TaskStatus.BLOCKED
    else:
        initial_status = TaskStatus.READY
```

#### Method 2: `get_next_task`

```python
async def get_next_task(self) -> Task | None:
    """
    Returns highest priority READY task and marks it as RUNNING.

    Query:
        SELECT * FROM tasks
        WHERE status = 'ready'
        ORDER BY calculated_priority DESC, submitted_at ASC
        LIMIT 1

    Steps:
    1. Query for highest priority READY task
    2. If found, update status to RUNNING (atomic)
    3. Update started_at timestamp
    4. Return task
    5. If not found, return None

    Returns:
        Next task to execute, or None if queue empty
    """
```

**Performance Notes:**
- Uses composite index: `idx_tasks_ready_priority ON tasks(status, calculated_priority DESC, submitted_at ASC)`
- Target: <5ms query time
- Atomic status update to prevent race conditions

#### Method 3: `complete_task`

```python
async def complete_task(self, task_id: str) -> list[str]:
    """
    Mark task as COMPLETED and unblock dependent tasks.

    Steps:
    1. Update task status to COMPLETED
    2. Update completed_at timestamp
    3. Resolve dependencies in task_dependencies table (set resolved_at)
    4. Get all tasks that were blocked by this one
    5. For each blocked task:
       a. Check if ALL its prerequisites are now met
       b. If yes, update status from BLOCKED → READY
       c. Recalculate priority (may have changed with new depth)
       d. Update calculated_priority in database
    6. Return list of newly-unblocked task IDs

    Returns:
        List of task IDs that were unblocked (BLOCKED → READY)
    """
```

**Dependency Resolution Logic:**
```python
# Mark this task's dependencies as resolved
UPDATE task_dependencies
SET resolved_at = NOW()
WHERE prerequisite_task_id = task_id AND resolved_at IS NULL

# Find potentially unblocked tasks
SELECT DISTINCT dependent_task_id
FROM task_dependencies
WHERE prerequisite_task_id = task_id

# For each dependent, check if ALL dependencies met
SELECT COUNT(*) FROM task_dependencies
WHERE dependent_task_id = ? AND resolved_at IS NULL

# If count = 0, all dependencies met → update to READY
```

#### Method 4: `fail_task`

```python
async def fail_task(self, task_id: str, error_message: str) -> list[str]:
    """
    Mark task as FAILED and cascade cancellation to dependent tasks.

    Steps:
    1. Update task status to FAILED
    2. Set error_message field
    3. Get all tasks that depend on this one (recursively)
    4. Update all dependent tasks to CANCELLED status
    5. Return list of cancelled task IDs

    Rationale:
    If a task fails, tasks depending on it cannot proceed.
    Cascading cancellation prevents orphaned blocked tasks.

    Returns:
        List of task IDs that were cancelled due to failure
    """
```

**Cascading Logic:**
```python
# Get direct dependents
SELECT dependent_task_id FROM task_dependencies
WHERE prerequisite_task_id = task_id

# Recursively get transitive dependents
# Update all to CANCELLED status
UPDATE tasks
SET status = 'cancelled'
WHERE id IN (dependent_ids)
```

#### Method 5: `cancel_task`

```python
async def cancel_task(self, task_id: str) -> list[str]:
    """
    Cancel task and cascade cancellation to dependents.

    Similar to fail_task but without error message.
    Used for user-initiated cancellations.

    Steps:
    1. Update task status to CANCELLED
    2. Get all dependent tasks (recursively)
    3. Update dependents to CANCELLED
    4. Return list of cancelled task IDs

    Returns:
        List of task IDs that were cancelled
    """
```

#### Method 6: `get_queue_status`

```python
async def get_queue_status(self) -> dict:
    """
    Return queue statistics for monitoring.

    Returns:
    {
        "total_tasks": int,
        "pending": int,
        "blocked": int,
        "ready": int,
        "running": int,
        "completed": int,
        "failed": int,
        "cancelled": int,
        "avg_priority": float,
        "max_depth": int,
        "oldest_pending": datetime | None,
        "newest_task": datetime | None,
    }

    Query:
    SELECT status, COUNT(*) as count, AVG(calculated_priority) as avg_priority
    FROM tasks
    GROUP BY status
    """
```

**Performance Notes:**
- Use aggregate queries with GROUP BY
- Target: <20ms query time
- Cache results if called frequently (optional optimization)

#### Method 7: `get_task_execution_plan`

```python
async def get_task_execution_plan(self, task_ids: list[str]) -> list[list[str]]:
    """
    Return execution plan as list of batches (topological sort).

    Uses DependencyResolver.get_execution_order() to compute topological sort.
    Tasks in same batch can execute in parallel (no dependencies between them).

    Example:
    Input: [A, B, C, D] where A→B, A→C, B→D, C→D
    Output: [[A], [B, C], [D]]

    Batch 0: [A] - no dependencies, execute first
    Batch 1: [B, C] - depend on A, can execute in parallel
    Batch 2: [D] - depends on B and C, execute after both complete

    Returns:
        List of batches, each batch is list of task IDs
    """
```

**Algorithm:**
- Delegate to DependencyResolver.get_execution_order()
- Group tasks by depth level (tasks at same depth can run in parallel)
- Return ordered list of batches

### 3. Helper Methods

```python
async def _get_unmet_prerequisites(self, prerequisite_ids: list[str]) -> list[str]:
    """
    Get list of prerequisite tasks that are not yet completed.

    Query:
    SELECT id FROM tasks
    WHERE id IN (prerequisite_ids)
    AND status NOT IN ('completed', 'cancelled')
    """

async def _validate_prerequisites_exist(self, prerequisite_ids: list[str]) -> None:
    """
    Validate all prerequisite task IDs exist in database.

    Raises ValueError if any prerequisite doesn't exist.
    """

async def _get_dependent_tasks_recursive(self, task_id: str) -> list[str]:
    """
    Get all tasks that transitively depend on this task.

    Uses recursive query or iterative BFS to find all descendants.
    """

async def _update_task_priority(self, task_id: str) -> float:
    """
    Recalculate and update task priority.

    1. Fetch task from database
    2. Calculate new priority using PriorityCalculator
    3. Update calculated_priority in database
    4. Return new priority
    """
```

---

## Integration Specifications

### Integration with Phase 1 (Schema)

**Database Operations:**

```python
# Use existing Database methods
await self._db.insert_task(task)
await self._db.update_task_status(task_id, new_status)
task = await self._db.get_task(task_id)

# New dependency operations
await self._db.insert_task_dependency(dependency)
dependencies = await self._db.get_task_dependencies(task_id)
await self._db.resolve_dependency(prerequisite_task_id)
```

**Task Model Fields:**
```python
task = Task(
    id=uuid4(),
    prompt=description,
    agent_type=agent_type,
    priority=base_priority,
    status=initial_status,
    source=source,
    parent_task_id=parent_task_id,
    session_id=session_id,
    input_data=input_data or {},
    deadline=deadline,
    estimated_duration_seconds=estimated_duration_seconds,
    calculated_priority=initial_priority,  # From PriorityCalculator
    dependency_depth=depth,  # From DependencyResolver
    submitted_at=datetime.now(timezone.utc),
    last_updated_at=datetime.now(timezone.utc),
)
```

**Dependency Model:**
```python
dependency = TaskDependency(
    id=uuid4(),
    dependent_task_id=task.id,
    prerequisite_task_id=prerequisite_id,
    dependency_type=DependencyType.SEQUENTIAL,
    created_at=datetime.now(timezone.utc),
)
```

### Integration with Phase 2 (DependencyResolver)

**Initialization:**
```python
class TaskQueueService:
    def __init__(self, database: Database, dependency_resolver: DependencyResolver, priority_calculator: PriorityCalculator):
        self._db = database
        self._dependency_resolver = dependency_resolver
        self._priority_calculator = priority_calculator
```

**Usage:**
```python
# Circular dependency check
await self._dependency_resolver.validate_new_dependency(task_id, prerequisite_id)

# Dependency depth calculation
depth = await self._dependency_resolver.calculate_dependency_depth(task_id)

# Execution order (topological sort)
batches = await self._dependency_resolver.get_execution_order(task_ids)

# Get ready tasks
ready_ids = await self._dependency_resolver.get_ready_tasks(task_ids)

# Get blocked tasks
blocked_ids = await self._dependency_resolver.get_blocked_tasks(task_id)
```

**Error Handling:**
```python
from abathur.services.dependency_resolver import CircularDependencyError

try:
    await self._dependency_resolver.validate_new_dependency(task_id, prerequisite_id)
except CircularDependencyError as e:
    logger.error(f"Circular dependency detected: {e}")
    raise ValueError(f"Cannot add dependency: {e}")
```

### Integration with Phase 3 (PriorityCalculator)

**Usage:**
```python
# Calculate initial priority for new task
task = create_task(...)
priority = await self._priority_calculator.calculate_priority(task)
task.calculated_priority = priority

# Recalculate priorities after state change
affected_task_ids = [...]
results = await self._priority_calculator.recalculate_priorities(affected_task_ids, self._db)

# Update database with new priorities
for task_id, new_priority in results.items():
    await self._db.execute(
        "UPDATE tasks SET calculated_priority = ? WHERE id = ?",
        (new_priority, str(task_id))
    )
```

---

## State Transition Rules

### Valid Transitions

```
PENDING → READY         (all dependencies met)
PENDING → BLOCKED       (has unmet dependencies)
PENDING → CANCELLED     (parent task failed/cancelled)

BLOCKED → READY         (last dependency completed)
BLOCKED → CANCELLED     (prerequisite failed/cancelled)

READY → RUNNING         (dequeued by get_next_task)
READY → CANCELLED       (user cancellation)

RUNNING → COMPLETED     (successful execution)
RUNNING → FAILED        (error during execution)
RUNNING → CANCELLED     (user cancellation)

COMPLETED → (none)      (terminal state)
FAILED → (none)         (terminal state)
CANCELLED → (none)      (terminal state)
```

### Invalid Transitions

```
RUNNING → PENDING       (INVALID)
RUNNING → BLOCKED       (INVALID)
RUNNING → READY         (INVALID)
COMPLETED → *           (INVALID - terminal)
FAILED → *              (INVALID - terminal)
CANCELLED → *           (INVALID - terminal)
```

**Enforcement:**
- Validate transitions in update_task_status
- Log warnings for invalid transition attempts
- Raise ValueError for invalid transitions

---

## Performance Requirements

| Operation | Target | Validation Method |
|-----------|--------|-------------------|
| Task enqueue | <10ms | Performance test: insert 100 tasks with dependencies |
| Get next task | <5ms | Query plan analysis + benchmark |
| Complete task | <50ms | Test with 10 dependents to unblock |
| Fail task | <30ms | Test with 10 dependents to cancel |
| Cancel task | <30ms | Test with 10 dependents to cancel |
| Queue status | <20ms | Aggregate query benchmark |
| Execution plan | <30ms | Test with 100-task graph |

**Index Usage:**
- Enqueue: Uses `idx_tasks_status_priority` for dependency checks
- Get next task: Uses `idx_tasks_ready_priority` (composite index)
- Complete task: Uses `idx_task_dependencies_prerequisite` for unblocking
- Queue status: Uses `idx_tasks_status_priority` for aggregates

**Query Optimization:**
- Use EXPLAIN QUERY PLAN to validate index usage
- Batch dependency insertions in single transaction
- Minimize database roundtrips (fetch once, update in bulk)

---

## Error Handling Strategy

### Exception Hierarchy

```python
class TaskQueueError(Exception):
    """Base exception for task queue errors."""

class CircularDependencyError(TaskQueueError):
    """Raised when circular dependency detected."""

class TaskNotFoundError(TaskQueueError):
    """Raised when task ID doesn't exist."""

class InvalidTransitionError(TaskQueueError):
    """Raised when invalid status transition attempted."""
```

### Error Handling Patterns

**Validation Errors:**
```python
if not prerequisites_exist:
    logger.error(f"Invalid prerequisites: {missing_ids}")
    raise ValueError(f"Prerequisites not found: {missing_ids}")
```

**Database Errors:**
```python
try:
    async with self._db._get_connection() as conn:
        # Database operations
        await conn.commit()
except Exception as e:
    logger.error(f"Database error during enqueue: {e}", exc_info=True)
    await conn.rollback()
    raise DatabaseError(f"Failed to enqueue task: {e}")
```

**Circular Dependency Errors:**
```python
try:
    await self._dependency_resolver.validate_new_dependency(task_id, prereq_id)
except CircularDependencyError as e:
    logger.warning(f"Circular dependency rejected: {e}")
    raise ValueError(f"Cannot add dependency - creates cycle: {e}")
```

---

## Testing Strategy

### Unit Tests

**File:** `/Users/odgrim/dev/home/agentics/abathur/tests/unit/services/test_task_queue_service.py`

**Required Tests (>80% coverage):**

1. **Enqueue Task Tests:**
   - `test_enqueue_task_basic` - Simple task with no dependencies
   - `test_enqueue_task_with_prerequisites` - Task starts BLOCKED
   - `test_enqueue_task_all_prerequisites_met` - Task starts READY
   - `test_enqueue_task_circular_dependency` - Raises ValueError
   - `test_enqueue_task_invalid_prerequisites` - Raises ValueError (prereq doesn't exist)
   - `test_enqueue_task_priority_calculation` - Priority calculated correctly
   - `test_enqueue_task_depth_calculation` - Depth calculated correctly

2. **Get Next Task Tests:**
   - `test_get_next_task_priority_order` - Returns highest priority READY task
   - `test_get_next_task_fifo_tiebreaker` - Oldest task if priorities equal
   - `test_get_next_task_no_ready_tasks` - Returns None
   - `test_get_next_task_skips_blocked` - Ignores BLOCKED tasks
   - `test_get_next_task_updates_status` - Task marked RUNNING

3. **Complete Task Tests:**
   - `test_complete_task_simple` - Task marked COMPLETED
   - `test_complete_task_unblocks_dependents` - Dependents become READY
   - `test_complete_task_recalculates_priorities` - Priorities updated
   - `test_complete_task_partial_unblock` - Some dependents still blocked
   - `test_complete_task_no_dependents` - Returns empty list

4. **Fail Task Tests:**
   - `test_fail_task_sets_error` - Error message stored
   - `test_fail_task_cancels_dependents` - Cascade cancellation
   - `test_fail_task_recursive_cancellation` - Transitive dependents cancelled

5. **Cancel Task Tests:**
   - `test_cancel_task_basic` - Task marked CANCELLED
   - `test_cancel_task_cancels_dependents` - Cascade cancellation

6. **Queue Status Tests:**
   - `test_get_queue_status_counts` - Correct task counts by status
   - `test_get_queue_status_avg_priority` - Average priority calculated
   - `test_get_queue_status_empty_queue` - Handles empty queue

7. **Execution Plan Tests:**
   - `test_get_task_execution_plan_linear` - A→B→C returns [[A], [B], [C]]
   - `test_get_task_execution_plan_parallel` - A→(B,C) returns [[A], [B,C]]
   - `test_get_task_execution_plan_diamond` - A→(B,C)→D returns [[A], [B,C], [D]]

8. **State Transition Tests:**
   - `test_state_transitions_valid` - All valid transitions work
   - `test_state_transitions_invalid` - Invalid transitions rejected

**Target:** >80% code coverage

### Integration Tests

**File:** `/Users/odgrim/dev/home/agentics/abathur/tests/integration/test_task_queue_workflow.py`

**Required Workflows:**

1. **Linear Workflow:**
```python
async def test_linear_workflow():
    # A → B → C execution
    task_a = await service.enqueue_task("Task A", TaskSource.HUMAN)
    task_b = await service.enqueue_task("Task B", TaskSource.HUMAN, prerequisites=[task_a.id])
    task_c = await service.enqueue_task("Task C", TaskSource.HUMAN, prerequisites=[task_b.id])

    assert task_a.status == TaskStatus.READY
    assert task_b.status == TaskStatus.BLOCKED
    assert task_c.status == TaskStatus.BLOCKED

    next_task = await service.get_next_task()
    assert next_task.id == task_a.id

    await service.complete_task(task_a.id)
    task_b = await db.get_task(task_b.id)
    assert task_b.status == TaskStatus.READY

    # Continue execution...
```

2. **Parallel Workflow:**
```python
async def test_parallel_workflow():
    # A → (B, C) → D execution
    task_a = await service.enqueue_task("Task A", TaskSource.HUMAN)
    task_b = await service.enqueue_task("Task B", TaskSource.HUMAN, prerequisites=[task_a.id])
    task_c = await service.enqueue_task("Task C", TaskSource.HUMAN, prerequisites=[task_a.id])
    task_d = await service.enqueue_task("Task D", TaskSource.HUMAN, prerequisites=[task_b.id, task_c.id])

    await service.complete_task(task_a.id)
    # Both B and C should become READY
    # D should stay BLOCKED until both B and C complete
```

3. **Diamond Workflow:**
```python
async def test_diamond_workflow():
    #     A
    #    / \
    #   B   C
    #    \ /
    #     D
    # Validate synchronization at D (waits for both B and C)
```

4. **Failure Propagation:**
```python
async def test_failure_propagation():
    # A → B → C
    # Fail A, verify B and C are cancelled
```

5. **Priority Scheduling:**
```python
async def test_priority_scheduling():
    # Create tasks with different priorities
    # Verify high priority tasks dequeued first
```

6. **Source Prioritization:**
```python
async def test_human_vs_agent_priorities():
    # HUMAN tasks should be prioritized over AGENT_* tasks
```

### Performance Tests

**File:** `/Users/odgrim/dev/home/agentics/abathur/tests/performance/test_task_queue_service_performance.py`

**Required Benchmarks:**

1. **Enqueue Throughput:**
```python
async def test_enqueue_throughput():
    start = time.perf_counter()
    for i in range(100):
        await service.enqueue_task(f"Task {i}", TaskSource.HUMAN)
    elapsed = time.perf_counter() - start
    throughput = 100 / elapsed
    assert throughput > 100, f"Throughput {throughput:.1f} tasks/sec, target >100"
```

2. **Get Next Task Latency:**
```python
async def test_get_next_task_latency():
    # Create 100 READY tasks
    # Measure time to dequeue next task
    assert elapsed_ms < 5.0
```

3. **Complete Task Cascade:**
```python
async def test_complete_task_cascade():
    # Create task with 10 dependents
    # Measure time to complete and unblock all
    assert elapsed_ms < 50.0
```

4. **Queue Status Performance:**
```python
async def test_queue_status_performance():
    # Create 1000 tasks in various states
    # Measure time to compute statistics
    assert elapsed_ms < 20.0
```

---

## Implementation Checklist

- [ ] Create TaskQueueService class with constructor
- [ ] Implement enqueue_task method
- [ ] Implement get_next_task method
- [ ] Implement complete_task method
- [ ] Implement fail_task method
- [ ] Implement cancel_task method
- [ ] Implement get_queue_status method
- [ ] Implement get_task_execution_plan method
- [ ] Implement helper methods (_get_unmet_prerequisites, _validate_prerequisites_exist, etc.)
- [ ] Add comprehensive docstrings (module, class, methods)
- [ ] Add type hints throughout
- [ ] Add logging statements (debug, info, warning, error)
- [ ] Write unit tests (>80% coverage target)
- [ ] Write integration tests (all workflows)
- [ ] Write performance tests (all targets)
- [ ] Validate query performance (EXPLAIN QUERY PLAN)
- [ ] Error handling for all edge cases
- [ ] Transaction management for atomic operations
- [ ] Status transition validation

---

## Success Criteria

### Acceptance Criteria

1. **Tasks with dependencies enter BLOCKED status** - MUST PASS
2. **Dependencies automatically resolved on completion** - MUST PASS
3. **Dependent tasks correctly transitioned to READY** - MUST PASS
4. **Priority queue returns highest calculated_priority task** - MUST PASS
5. **Performance: 1000+ tasks/sec enqueue throughput** - MUST PASS
6. **Integration tests pass for all workflows** - MUST PASS
7. **Backward compatibility maintained** - MUST PASS (or document breaking changes)

### Phase Gate Criteria

- All unit tests passing (>80% coverage)
- All integration tests passing (6 workflows)
- All performance tests passing (4 benchmarks)
- No critical bugs or blockers
- Code quality meets standards (docstrings, type hints, logging)
- Integration with Phase 1-3 components validated

---

## References

**Architecture Documents:**
- `/Users/odgrim/dev/home/agentics/abathur/design_docs/02-task-queue/TASK_QUEUE_ARCHITECTURE.md`
- `/Users/odgrim/dev/home/agentics/abathur/design_docs/02-task-queue/TASK_QUEUE_DECISION_POINTS.md`

**Validation Reports:**
- `/Users/odgrim/dev/home/agentics/abathur/design_docs/02-task-queue/reports/TASK_QUEUE_PHASE1_VALIDATION_REPORT.md`
- `/Users/odgrim/dev/home/agentics/abathur/design_docs/02-task-queue/reports/TASK_QUEUE_PHASE2_VALIDATION_REPORT.md` (assumed)
- `/Users/odgrim/dev/home/agentics/abathur/design_docs/02-task-queue/reports/TASK_QUEUE_PHASE3_VALIDATION_REPORT.md`

**Implementation Files (Phase 1-3):**
- `/Users/odgrim/dev/home/agentics/abathur/src/abathur/domain/models.py` (Task, TaskDependency models)
- `/Users/odgrim/dev/home/agentics/abathur/src/abathur/infrastructure/database.py` (Database class)
- `/Users/odgrim/dev/home/agentics/abathur/src/abathur/services/dependency_resolver.py` (Phase 2)
- `/Users/odgrim/dev/home/agentics/abathur/src/abathur/services/priority_calculator.py` (Phase 3)

**Test Files (Phase 3 examples):**
- `/Users/odgrim/dev/home/agentics/abathur/tests/unit/services/test_priority_calculator.py`
- `/Users/odgrim/dev/home/agentics/abathur/tests/performance/test_priority_calculator_performance.py`

---

## Notes for python-backend-developer Agent

1. **Reuse Existing Services:** Import and use DependencyResolver and PriorityCalculator - do not reimplement their functionality

2. **Follow Phase 3 Patterns:** Use same error handling, logging, and docstring patterns as PriorityCalculator

3. **Transaction Management:** Use database transactions for multi-step operations (enqueue + dependencies, complete + unblock)

4. **Performance First:** Use indexed queries, minimize database roundtrips, validate query plans

5. **Test-Driven Development:** Write tests alongside implementation - use Phase 3 tests as examples

6. **State Machine Enforcement:** Validate all status transitions strictly - log invalid attempts

7. **Logging Levels:**
   - DEBUG: Individual operations (enqueue, dequeue, status changes)
   - INFO: Significant events (task completed, dependents unblocked)
   - WARNING: Anomalies (invalid transition, missing prerequisite)
   - ERROR: Failures (database errors, circular dependencies)

8. **Async/Await:** All methods must be async - use `await` for database and service calls

9. **Type Hints:** Use proper type hints throughout (UUID | str for task IDs, list[str] for lists, dict[str, Any] for JSON)

10. **Error Messages:** Provide clear, actionable error messages that include task IDs and context

---

**Context Document Status:** READY FOR IMPLEMENTATION

**Next Step:** Begin implementation of TaskQueueService class

**Estimated Completion:** 3 days (assuming 8-hour workdays)

---

**Document Generated:** 2025-10-10 (task-queue-orchestrator)
