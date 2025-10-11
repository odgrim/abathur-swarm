# Phase 3 Context Document: Priority Calculation Service

**Project:** Abathur Enhanced Task Queue System
**Phase:** 3 - Priority Calculation Service
**Date:** 2025-10-10
**Assigned To:** python-backend-developer agent
**Status:** READY TO START

---

## Executive Summary

Phase 3 implements the PriorityCalculator service that dynamically calculates task priorities based on multiple factors:  urgency (deadline proximity), dependency depth, blocking impact, task source, and starvation prevention. This service integrates with the completed Phase 2 DependencyResolver to provide real-time priority scoring for task queue scheduling.

### Phase 3 Objectives

1. Implement PriorityCalculator service with weighted multi-factor scoring
2. Integrate with DependencyResolver for depth calculations
3. Achieve <5ms single priority calculation performance
4. Support batch priority recalculation (<50ms for 100 tasks)
5. Provide comprehensive unit and performance tests

---

## 1. Prerequisites (Phase 2 Deliverables)

### Phase 2 Status: APPROVED

Phase 2 has been validated and approved with the following deliverables ready for integration:

**DependencyResolver Service** (`/Users/odgrim/dev/home/agentics/abathur/src/abathur/services/dependency_resolver.py`):
- ✓ 176 lines, 11 methods implemented
- ✓ 98.86% test coverage
- ✓ All performance targets met

**Available Methods for Phase 3:**
```python
async def calculate_dependency_depth(task_id: UUID) -> int
    # Returns: 0 for root tasks, 1+ for dependent tasks
    # Performance: 1.04ms for 10-level chain (cached: <0.01ms)

async def get_blocked_tasks(prerequisite_task_id: UUID) -> list[UUID]
    # Returns: List of task IDs blocked by this prerequisite
    # Performance: O(1) indexed query

async def are_all_dependencies_met(task_id: UUID) -> bool
    # Returns: True if all dependencies resolved
    # Performance: O(1) COUNT query
```

**Phase 2 Performance Results:**
- Cycle detection: 0.68ms (target <10ms) ✓
- Topological sort: 11.65ms (target <15ms) ✓
- Depth calculation: 1.04ms (target <5ms) ✓
- Graph cache hit: 0.003ms (target <1ms) ✓

---

## 2. Phase 3 Deliverables

### 2.1 PriorityCalculator Service

**File:** `/Users/odgrim/dev/home/agentics/abathur/src/abathur/services/priority_calculator.py`

**Class Structure:**

```python
from datetime import datetime, timezone
from uuid import UUID

from abathur.domain.models import Task, TaskSource, TaskStatus
from abathur.services.dependency_resolver import DependencyResolver


class PriorityCalculator:
    """Calculates dynamic task priorities based on multiple factors.

    Implements Chapter 20 prioritization patterns with weighted multi-factor scoring:
    - Base priority (user-specified): 30% weight
    - Dependency depth (deeper = higher): 25% weight
    - Deadline urgency (closer = higher): 25% weight
    - Blocking impact (more blocked tasks = higher): 15% weight
    - Task source (HUMAN > AGENT): 5% weight

    Performance targets:
    - Single calculation: <5ms
    - Batch calculation (100 tasks): <50ms
    """

    def __init__(
        self,
        dependency_resolver: DependencyResolver,
        base_weight: float = 0.3,
        depth_weight: float = 0.25,
        urgency_weight: float = 0.25,
        blocking_weight: float = 0.15,
        source_weight: float = 0.05,
    ):
        """Initialize priority calculator with tunable weights.

        Args:
            dependency_resolver: DependencyResolver instance for depth calculations
            base_weight: Weight for user-specified base priority (default: 0.3)
            depth_weight: Weight for dependency depth score (default: 0.25)
            urgency_weight: Weight for deadline urgency score (default: 0.25)
            blocking_weight: Weight for blocking tasks count (default: 0.15)
            source_weight: Weight for task source priority (default: 0.05)
        """
        self._dependency_resolver = dependency_resolver
        self._base_weight = base_weight
        self._depth_weight = depth_weight
        self._urgency_weight = urgency_weight
        self._blocking_weight = blocking_weight
        self._source_weight = source_weight

    async def calculate_priority(
        self,
        task: Task,
        all_tasks: list[Task] | None = None
    ) -> float:
        """Calculate dynamic priority score (0.0-100.0) for a task.

        Priority Formula:
        priority = (
            base_priority * base_weight +
            depth_score * depth_weight +
            urgency_score * urgency_weight +
            blocking_score * blocking_weight +
            source_score * source_weight
        )

        Args:
            task: Task to calculate priority for
            all_tasks: Optional list of all tasks for context (for blocking count)

        Returns:
            Priority score (0.0-100.0)

        Performance:
            Target: <5ms per calculation
        """
        # 1. Base priority (0-10 scale, normalize to 0-100)
        base_score = task.priority * 10.0  # Scale 0-10 to 0-100

        # 2. Dependency depth score (0-100)
        depth_score = await self._calculate_depth_score(task)

        # 3. Urgency score based on deadline (0-100)
        urgency_score = self._calculate_urgency_score(
            task.deadline, task.estimated_duration_seconds
        )

        # 4. Blocking impact score (0-100)
        blocking_score = await self._calculate_blocking_score(task)

        # 5. Source priority score (0-100)
        source_score = self._calculate_source_score(task.source)

        # Weighted sum
        priority = (
            base_score * self._base_weight +
            depth_score * self._depth_weight +
            urgency_score * self._urgency_weight +
            blocking_score * self._blocking_weight +
            source_score * self._source_weight
        )

        return max(0.0, min(100.0, priority))  # Clamp to [0, 100]

    async def recalculate_priorities(
        self,
        affected_task_ids: list[UUID],
        db: Database
    ) -> dict[UUID, float]:
        """Recalculate priorities for multiple tasks (batch operation).

        Used after state changes (task completion, new task submission) to
        update priorities for affected tasks.

        Args:
            affected_task_ids: List of task IDs to recalculate
            db: Database instance for fetching tasks

        Returns:
            Mapping of task_id -> new_priority

        Performance:
            Target: <50ms for 100 tasks
        """
        results = {}

        for task_id in affected_task_ids:
            task = await db.get_task(task_id)
            if task and task.status in [TaskStatus.PENDING, TaskStatus.BLOCKED, TaskStatus.READY]:
                new_priority = await self.calculate_priority(task)
                results[task_id] = new_priority

        return results

    async def _calculate_depth_score(self, task: Task) -> float:
        """Calculate priority score based on dependency depth.

        Deeper tasks (more prerequisites completed) get higher priority.
        This encourages completion of task chains.

        Scoring:
        - Depth 0 (root): 0 points
        - Depth 1: 10 points
        - Depth 2: 20 points
        - ...
        - Depth 10+: 100 points (capped)

        Args:
            task: Task to score

        Returns:
            Depth score (0-100)
        """
        depth = await self._dependency_resolver.calculate_dependency_depth(task.id)
        return min(100.0, depth * 10.0)

    def _calculate_urgency_score(
        self,
        deadline: datetime | None,
        estimated_duration: int | None
    ) -> float:
        """Calculate urgency score based on deadline proximity.

        Tasks with approaching deadlines get higher urgency scores.
        Considers estimated duration to detect "too late" scenarios.

        Scoring:
        - No deadline: 0 points
        - > 1 week: 10 points
        - 1 week: 30 points
        - 1 day: 50 points
        - 1 hour: 80 points
        - < 1 minute or past deadline: 100 points

        If estimated_duration provided:
        - If time_to_deadline < estimated_duration: 100 points (urgent)

        Args:
            deadline: Task deadline (None if no deadline)
            estimated_duration: Estimated execution time in seconds (None if unknown)

        Returns:
            Urgency score (0-100)
        """
        if not deadline:
            return 0.0

        now = datetime.now(timezone.utc)
        time_remaining = (deadline - now).total_seconds()

        # Past deadline or negative time
        if time_remaining <= 0:
            return 100.0

        # Check if not enough time to complete
        if estimated_duration and time_remaining < estimated_duration:
            return 100.0  # Urgent - may miss deadline

        # Exponential urgency scaling
        if time_remaining < 60:  # < 1 minute
            return 100.0
        elif time_remaining < 3600:  # < 1 hour
            return 80.0
        elif time_remaining < 86400:  # < 1 day
            return 50.0
        elif time_remaining < 604800:  # < 1 week
            return 30.0
        else:  # > 1 week
            return 10.0

    async def _calculate_blocking_score(self, task: Task) -> float:
        """Calculate score based on number of tasks blocked by this one.

        Tasks that are blocking other tasks get priority boost to unblock
        the dependency chain.

        Scoring:
        - 0 blocked tasks: 0 points
        - 1-2 blocked: 20 points
        - 3-5 blocked: 40 points
        - 6-10 blocked: 60 points
        - 11-20 blocked: 80 points
        - 20+ blocked: 100 points

        Uses logarithmic scaling to prevent extreme priority inflation.

        Args:
            task: Task to score

        Returns:
            Blocking score (0-100)
        """
        # Get tasks blocked by this one
        blocked_task_ids = await self._dependency_resolver.get_blocked_tasks(task.id)
        num_blocked = len(blocked_task_ids)

        if num_blocked == 0:
            return 0.0

        # Logarithmic scaling with manual breakpoints
        if num_blocked <= 2:
            return 20.0
        elif num_blocked <= 5:
            return 40.0
        elif num_blocked <= 10:
            return 60.0
        elif num_blocked <= 20:
            return 80.0
        else:
            return 100.0

    def _calculate_source_score(self, source: TaskSource) -> float:
        """Calculate priority score based on task source.

        Human-submitted tasks get higher priority than agent-generated subtasks.
        This ensures user requests are prioritized over internal agent work.

        Scoring:
        - HUMAN: 100 points
        - AGENT_REQUIREMENTS: 75 points
        - AGENT_PLANNER: 50 points
        - AGENT_IMPLEMENTATION: 25 points

        Args:
            source: Task source enum

        Returns:
            Source priority score (0-100)
        """
        if source == TaskSource.HUMAN:
            return 100.0
        elif source == TaskSource.AGENT_REQUIREMENTS:
            return 75.0
        elif source == TaskSource.AGENT_PLANNER:
            return 50.0
        elif source == TaskSource.AGENT_IMPLEMENTATION:
            return 25.0
        else:
            return 0.0
```

---

## 3. Test Suite Requirements

### 3.1 Unit Tests

**File:** `/Users/odgrim/dev/home/agentics/abathur/tests/unit/services/test_priority_calculator.py`

**Test Coverage Requirements:**

1. **Base Priority Tests** (3 tests)
   - `test_calculate_priority_base_only` - Only base priority set, others default
   - `test_calculate_priority_base_scaling` - Base priority scales 0-10 to 0-100
   - `test_calculate_priority_clamping` - Priority clamped to [0, 100]

2. **Depth Score Tests** (4 tests)
   - `test_depth_score_root_task` - Depth 0 → 0 points
   - `test_depth_score_linear_scaling` - Depth 1→10pts, 2→20pts, 3→30pts
   - `test_depth_score_capping` - Depth 10+ capped at 100 points
   - `test_calculate_priority_with_depth` - Integration with DependencyResolver

3. **Urgency Score Tests** (7 tests)
   - `test_urgency_score_no_deadline` - No deadline → 0 points
   - `test_urgency_score_past_deadline` - Past deadline → 100 points
   - `test_urgency_score_one_minute` - < 1 minute → 100 points
   - `test_urgency_score_one_hour` - < 1 hour → 80 points
   - `test_urgency_score_one_day` - < 1 day → 50 points
   - `test_urgency_score_one_week` - < 1 week → 30 points
   - `test_urgency_score_insufficient_time` - time_remaining < estimated_duration → 100 points

4. **Blocking Score Tests** (6 tests)
   - `test_blocking_score_no_blocked_tasks` - 0 blocked → 0 points
   - `test_blocking_score_1_2_blocked` - 1-2 blocked → 20 points
   - `test_blocking_score_3_5_blocked` - 3-5 blocked → 40 points
   - `test_blocking_score_6_10_blocked` - 6-10 blocked → 60 points
   - `test_blocking_score_11_20_blocked` - 11-20 blocked → 80 points
   - `test_blocking_score_20_plus_blocked` - 20+ blocked → 100 points

5. **Source Score Tests** (4 tests)
   - `test_source_score_human` - HUMAN → 100 points
   - `test_source_score_agent_requirements` - AGENT_REQUIREMENTS → 75 points
   - `test_source_score_agent_planner` - AGENT_PLANNER → 50 points
   - `test_source_score_agent_implementation` - AGENT_IMPLEMENTATION → 25 points

6. **Integration Tests** (4 tests)
   - `test_calculate_priority_all_factors` - All factors combined
   - `test_calculate_priority_weighted_sum` - Verify weight application
   - `test_recalculate_priorities_batch` - Batch recalculation
   - `test_recalculate_priorities_filters_status` - Only PENDING/BLOCKED/READY tasks

**Target Coverage:** >80% code coverage

### 3.2 Performance Tests

**File:** `/Users/odgrim/dev/home/agentics/abathur/tests/performance/test_priority_calculator_performance.py`

**Performance Test Requirements:**

1. **Single Calculation Performance**
   ```python
   async def test_single_priority_calculation(db, resolver, calculator):
       """Test single priority calculation meets <5ms target."""
       # Create task with all factors
       task = create_complex_task(...)

       start = time.perf_counter()
       priority = await calculator.calculate_priority(task)
       elapsed_ms = (time.perf_counter() - start) * 1000

       assert elapsed_ms < 5.0, f"Single calculation took {elapsed_ms:.2f}ms, target <5ms"
   ```

2. **Batch Calculation Performance**
   ```python
   async def test_batch_priority_calculation_100_tasks(db, resolver, calculator):
       """Test batch calculation of 100 tasks meets <50ms target."""
       # Create 100 tasks
       task_ids = [create_task(...).id for _ in range(100)]

       start = time.perf_counter()
       results = await calculator.recalculate_priorities(task_ids, db)
       elapsed_ms = (time.perf_counter() - start) * 1000

       assert elapsed_ms < 50.0, f"Batch calculation took {elapsed_ms:.2f}ms, target <50ms"
   ```

3. **Priority Recalculation Cascade**
   ```python
   async def test_priority_recalculation_cascade(db, resolver, calculator):
       """Test cascading priority updates after task completion."""
       # Create 10-level dependency chain
       # Complete root task → recalculate all dependent tasks

       start = time.perf_counter()
       # Complete task + recalculate priorities
       elapsed_ms = (time.perf_counter() - start) * 1000

       assert elapsed_ms < 100.0, f"Cascade took {elapsed_ms:.2f}ms, target <100ms"
   ```

**Performance Targets:**
- Single calculation: <5ms
- Batch 100 tasks: <50ms
- 10-level cascade: <100ms

---

## 4. Integration Points

### 4.1 DependencyResolver Integration

**Required:**
- Import DependencyResolver in PriorityCalculator constructor
- Call `calculate_dependency_depth()` in `_calculate_depth_score()`
- Call `get_blocked_tasks()` in `_calculate_blocking_score()`

**Example:**
```python
from abathur.services.dependency_resolver import DependencyResolver

class PriorityCalculator:
    def __init__(self, dependency_resolver: DependencyResolver):
        self._dependency_resolver = dependency_resolver

    async def _calculate_depth_score(self, task: Task) -> float:
        depth = await self._dependency_resolver.calculate_dependency_depth(task.id)
        return min(100.0, depth * 10.0)
```

### 4.2 Database Integration

**Required:**
- Accept Database instance in `recalculate_priorities()` method
- Use `db.get_task(task_id)` to fetch task objects
- Filter by status: only recalculate PENDING/BLOCKED/READY tasks

**Example:**
```python
async def recalculate_priorities(
    self,
    affected_task_ids: list[UUID],
    db: Database
) -> dict[UUID, float]:
    results = {}
    for task_id in affected_task_ids:
        task = await db.get_task(task_id)
        if task and task.status in [TaskStatus.PENDING, TaskStatus.BLOCKED, TaskStatus.READY]:
            new_priority = await self.calculate_priority(task)
            results[task_id] = new_priority
    return results
```

---

## 5. Acceptance Criteria

Phase 3 will be approved if:

| Criterion | Target | Validation Method |
|-----------|--------|-------------------|
| PriorityCalculator service implemented | Complete | Code review |
| All 5 scoring functions implemented | 5/5 | Unit tests |
| Weighted priority formula correct | Accurate | Unit tests |
| Integration with DependencyResolver | Working | Integration tests |
| Single calculation performance | <5ms | Performance test |
| Batch calculation performance | <50ms for 100 tasks | Performance test |
| Cascade recalculation performance | <100ms for 10 levels | Performance test |
| Unit test coverage | >80% | pytest-cov |
| All unit tests pass | 100% | pytest |
| All performance tests pass | 100% | pytest |
| Code quality | Production-ready | Code review |
| Documentation | Complete | Docstring review |

---

## 6. Implementation Guidelines

### 6.1 Priority Formula Design

**Weighted Sum Approach:**
```
priority = Σ (factor_score * factor_weight)

Where:
- factor_score ∈ [0, 100] (normalized)
- factor_weight ∈ [0, 1] (percentage)
- Σ weights = 1.0 (100%)
```

**Default Weights:**
- Base priority: 30%
- Dependency depth: 25%
- Deadline urgency: 25%
- Blocking impact: 15%
- Task source: 5%

**Rationale:**
- Base priority (30%): User intent is most important
- Depth + Urgency (25% each): Balance between workflow progress and time sensitivity
- Blocking (15%): Prevent dependency chain stalls
- Source (5%): Subtle bias toward human tasks

### 6.2 Performance Optimization

**Caching Strategy:**
- DependencyResolver has internal caching (60s TTL)
- PriorityCalculator should NOT cache (priorities change frequently)
- Batch operations should minimize database queries

**Optimization Tips:**
1. Use DependencyResolver's cached methods (depth, blocked tasks)
2. Filter tasks by status BEFORE calculating priorities
3. Consider parallel calculation for large batches (future optimization)

### 6.3 Error Handling

**Expected Errors:**
- Task not found in database → Return None, skip in batch operations
- DependencyResolver errors → Propagate to caller
- Invalid task status → Skip (only calculate for PENDING/BLOCKED/READY)

**Error Logging:**
- DEBUG: Priority calculation details
- WARNING: Tasks skipped due to invalid status
- ERROR: Unexpected errors in calculation

---

## 7. Reference Materials

### 7.1 Architecture Documents

1. **Task Queue Architecture** (`/Users/odgrim/dev/home/agentics/abathur/design_docs/TASK_QUEUE_ARCHITECTURE.md`)
   - Section 5.3: PriorityCalculator specification
   - Section 9: Performance targets

2. **Task Queue Decision Points** (`/Users/odgrim/dev/home/agentics/abathur/design_docs/TASK_QUEUE_DECISION_POINTS.md`)
   - Decision 8: Priority calculation weights
   - Decision 9: Priority recalculation frequency

3. **Phase 2 Validation Report** (`/Users/odgrim/dev/home/agentics/abathur/design_docs/PHASE2_VALIDATION_REPORT.md`)
   - DependencyResolver API surface
   - Performance baseline

### 7.2 Domain Models

**Task Model** (`/Users/odgrim/dev/home/agentics/abathur/src/abathur/domain/models.py`):
```python
class Task(BaseModel):
    id: UUID
    priority: int  # Base priority 0-10
    status: TaskStatus
    source: TaskSource
    deadline: datetime | None
    estimated_duration_seconds: int | None
    calculated_priority: float  # Updated by PriorityCalculator
    # ... other fields
```

**TaskStatus Enum:**
```python
class TaskStatus(str, Enum):
    PENDING = "pending"
    BLOCKED = "blocked"
    READY = "ready"
    RUNNING = "running"
    COMPLETED = "completed"
    FAILED = "failed"
    CANCELLED = "cancelled"
```

**TaskSource Enum:**
```python
class TaskSource(str, Enum):
    HUMAN = "human"
    AGENT_REQUIREMENTS = "agent_requirements"
    AGENT_PLANNER = "agent_planner"
    AGENT_IMPLEMENTATION = "agent_implementation"
```

---

## 8. Testing Strategy

### 8.1 Test Fixtures

**Required Fixtures:**
```python
@pytest.fixture
async def db():
    """In-memory database for testing."""
    database = Database(db_path=Path(":memory:"))
    await database.initialize()
    yield database
    await database.close()

@pytest.fixture
async def resolver(db):
    """DependencyResolver instance."""
    return DependencyResolver(db, cache_ttl_seconds=60.0)

@pytest.fixture
async def calculator(resolver):
    """PriorityCalculator instance with default weights."""
    return PriorityCalculator(resolver)

@pytest.fixture
async def calculator_custom_weights(resolver):
    """PriorityCalculator with custom weights for testing."""
    return PriorityCalculator(
        resolver,
        base_weight=0.4,
        depth_weight=0.2,
        urgency_weight=0.2,
        blocking_weight=0.1,
        source_weight=0.1,
    )
```

### 8.2 Test Data Builders

**Helper Functions:**
```python
def create_task(
    priority: int = 5,
    status: TaskStatus = TaskStatus.READY,
    source: TaskSource = TaskSource.HUMAN,
    deadline: datetime | None = None,
    estimated_duration: int | None = None,
) -> Task:
    """Create test task with specified attributes."""
    return Task(
        prompt="Test task",
        priority=priority,
        status=status,
        source=source,
        deadline=deadline,
        estimated_duration_seconds=estimated_duration,
    )

async def create_dependency_chain(db: Database, depth: int) -> list[Task]:
    """Create linear dependency chain of specified depth."""
    tasks = []
    for i in range(depth):
        task = create_task()
        await db.insert_task(task)
        tasks.append(task)

        if i > 0:
            dep = TaskDependency(
                dependent_task_id=task.id,
                prerequisite_task_id=tasks[i-1].id,
                dependency_type=DependencyType.SEQUENTIAL,
            )
            await db.insert_task_dependency(dep)

    return tasks
```

---

## 9. Deliverable Checklist

Before submitting Phase 3 completion:

### Code Deliverables
- [ ] PriorityCalculator service implemented (`priority_calculator.py`)
- [ ] All 5 scoring methods implemented and documented
- [ ] Integration with DependencyResolver working
- [ ] Database integration working
- [ ] Type hints on all methods
- [ ] Comprehensive docstrings (Google style)

### Test Deliverables
- [ ] Unit test file created (`test_priority_calculator.py`)
- [ ] All 28+ unit tests implemented and passing
- [ ] Performance test file created (`test_priority_calculator_performance.py`)
- [ ] All 3 performance benchmarks passing
- [ ] Test coverage >80% (verify with pytest-cov)

### Documentation Deliverables
- [ ] Phase 3 completion report
- [ ] Performance benchmark results (JSON)
- [ ] Algorithm documentation (scoring functions)
- [ ] Integration guide for Phase 4

### Quality Checks
- [ ] All tests pass: `pytest tests/unit/services/test_priority_calculator.py -v`
- [ ] Performance tests pass: `pytest tests/performance/test_priority_calculator_performance.py -v`
- [ ] Code coverage measured: `pytest --cov=src/abathur/services/priority_calculator --cov-report=html`
- [ ] No linting errors: Code follows Python best practices
- [ ] Imports clean: No circular dependencies

---

## 10. Success Metrics

Phase 3 will be considered successful if:

1. **Functionality**: All 5 scoring functions work correctly
2. **Performance**: All 3 performance targets met
3. **Quality**: >80% test coverage, all tests passing
4. **Integration**: DependencyResolver integration seamless
5. **Documentation**: Complete docstrings and reports

**Validation Gate:** task-queue-orchestrator will review deliverables and make go/no-go decision for Phase 4.

---

## 11. Risk Mitigation

### Identified Risks

1. **Performance Risk: Depth calculation overhead**
   - Mitigation: DependencyResolver has 60s cache, depth calculations are O(1) cached
   - Fallback: If too slow, cache depth values in Task model

2. **Performance Risk: Batch recalculation bottleneck**
   - Mitigation: Filter by status before calculation, use cached depth values
   - Fallback: Implement parallel calculation for large batches

3. **Integration Risk: DependencyResolver API mismatch**
   - Mitigation: Phase 2 API is validated and stable
   - Fallback: Wrap DependencyResolver methods if needed

---

## 12. Questions for Clarification

If any of these are unclear, consult the architecture document or escalate to task-queue-orchestrator:

1. **Weight Tuning**: Should weights be configurable at runtime? (Default: NO, constructor params only)
2. **Priority Clamping**: Should priorities be clamped to [0, 100]? (Default: YES)
3. **Status Filtering**: Which statuses should be recalculated? (Default: PENDING, BLOCKED, READY)
4. **Error Handling**: Should errors propagate or be logged and skipped? (Default: Log and skip for batch, propagate for single)

---

## 13. Next Steps After Phase 3

Upon Phase 3 approval, Phase 4 will begin:

**Phase 4: Task Queue Service Enhancement**
- Integrate PriorityCalculator into TaskQueueService
- Implement submit_task with dependency checking
- Implement complete_task with priority recalculation
- Implement dequeue_next_task with priority sorting
- End-to-end workflow testing

---

**Context Document Prepared By:** task-queue-orchestrator agent
**Date:** 2025-10-10
**Phase Status:** READY TO START
**Estimated Duration:** 2 days

---

**End of Phase 3 Context Document**
