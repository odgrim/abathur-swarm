# Task Queue System with Dependency Management - Architecture Design

**Project:** Abathur Enhanced Task Queue System
**Date:** 2025-10-10
**Status:** Phase 0 - Architecture & Design

## Executive Summary

This document defines the architecture for an enhanced task queue system that supports hierarchical task submission, dependency management, priority-based scheduling, and multi-agent coordination. The system enables agents to programmatically submit subtasks, enforces dependency blocking, tracks task origins, and implements dynamic prioritization based on Chapter 20 patterns.

### Key Objectives

1. **Hierarchical Task Submission**: Enable agents to break down work into subtasks
2. **Dependency Management**: Block tasks until prerequisites complete, detect cycles
3. **Priority Scheduling**: Implement dynamic prioritization (Chapter 20 patterns)
4. **Source Tracking**: Differentiate human tickets from agent-generated subtasks
5. **Performance**: 1000+ tasks/sec enqueue, <10ms dependency resolution

### Critical Success Factors

- Agents can submit subtasks without human intervention
- Dependencies are automatically enforced (no manual polling)
- Priority changes dynamically based on context (urgency, dependencies, deadline proximity)
- System handles circular dependencies gracefully
- Integration with existing memory system (session_id linkage)

---

## 1. Requirements Analysis

### 1.1 Functional Requirements

#### FR1: Hierarchical Task Submission
- **Human/Ticket Level**: Users submit high-level work requests
- **Requirements Level**: Requirements-gathering agents analyze and create requirement tasks
- **Planning Level**: Task-planner agents create executable subtasks with dependencies
- **Implementation Level**: Implementation agents can subdivide complex work further

#### FR2: Dependency Management
- Tasks with unmet dependencies enter BLOCKED status (new state)
- Dependency resolution occurs automatically when prerequisites complete
- Circular dependency detection prevents deadlocks
- Support for multiple dependency types:
  - **Sequential**: Task B requires Task A to complete first
  - **Parallel Prerequisites**: Task C requires both A AND B to complete

#### FR3: Priority-Based Scheduling
- Base priority (0-10 scale) assigned at creation
- Dynamic re-prioritization based on:
  - **Urgency**: Approaching deadlines
  - **Importance**: Impact on project objectives
  - **Dependencies**: Tasks blocking others get priority boost
  - **Starvation Prevention**: Long-waiting tasks gain priority
- Priority calculation algorithm (Chapter 20 patterns)

#### FR4: Source Tracking
- **TaskSource enum**: HUMAN, AGENT_REQUIREMENTS, AGENT_PLANNER, AGENT_IMPLEMENTATION
- Audit trail for work breakdown structure
- Different validation rules based on source

#### FR5: Agent Coordination
- Asynchronous task submission API for agents
- Task lifecycle notifications (completion → unblock dependents)
- Integration with existing Agent model and session system

### 1.2 Non-Functional Requirements

#### NFR1: Performance
- **Enqueue throughput**: 1000+ tasks/second
- **Dependency resolution**: <10ms for 100 task dependency graph
- **Priority calculation**: <5ms per task
- **Database queries**: Use indexes, avoid full table scans

#### NFR2: Reliability
- **ACID transactions**: Dependency updates must be atomic
- **Deadlock prevention**: Circular dependency detection before insert
- **Data integrity**: Foreign key constraints, referential integrity

#### NFR3: Scalability
- Support 10,000+ tasks in queue concurrently
- Efficient indexing for status, priority, dependencies
- Batch operations for dependency updates

#### NFR4: Maintainability
- Clear separation: domain models, service layer, database layer
- Comprehensive test coverage (unit, integration, performance)
- Documented algorithms (dependency resolution, priority calculation)

---

## 2. Design Document Synthesis

### 2.1 Key Patterns from Chapter 7 (Multi-Agent Collaboration)

**Hierarchical Structures**: Manager agents delegate to worker agents dynamically. Applied here:
- Requirements-gatherer → Task-planner → Implementation agents
- Parent tasks spawn subtasks, creating task hierarchy

**Coordination Models**:
- **Sequential Handoffs**: Requirements → Planning → Implementation
- **Parallel Execution**: Independent subtasks run concurrently
- **Supervisor Pattern**: Task queue acts as central coordinator

### 2.2 Key Patterns from Chapter 15 (Inter-Agent Communication)

**Message Attributes**: Priority, creation time metadata
**Applied**: Task model includes priority, submitted_at, source tracking

**Contextual Continuity**: Server-generated contextId groups related tasks
**Applied**: parent_task_id creates task hierarchy, session_id provides memory context

**Asynchronous Communication**: Agents communicate over async protocols
**Applied**: Task submission API is async, agents don't block waiting for results

### 2.3 Key Patterns from Chapter 20 (Prioritization)

**Criteria Definition**:
- **Urgency**: Time sensitivity (deadline proximity)
- **Importance**: Impact on primary objective (human vs agent tasks)
- **Dependencies**: Prerequisite for other tasks
- **Resource Availability**: Agent availability, system load
- **Cost/Benefit**: Effort vs expected outcome

**Dynamic Re-Prioritization**:
- **Event-Driven**: New critical task arrives → re-evaluate queue
- **Time-Based**: Approaching deadlines → increase urgency score
- **Dependency-Driven**: Task completion → unblock dependents, boost priority

**Task Evaluation**: Multi-factor scoring algorithm:
```
Priority Score = base_priority
                 + urgency_boost
                 + dependency_boost
                 + starvation_prevention_boost
```

---

## 3. Enhanced Domain Models

### 3.1 TaskStatus Enum (Updated)

```python
class TaskStatus(str, Enum):
    """Task lifecycle states."""
    PENDING = "pending"          # Submitted, no unmet dependencies
    BLOCKED = "blocked"          # NEW: Waiting for dependencies
    READY = "ready"              # NEW: Dependencies met, ready for execution
    RUNNING = "running"          # Currently executing
    COMPLETED = "completed"      # Successfully finished
    FAILED = "failed"            # Execution failed
    CANCELLED = "cancelled"      # Manually cancelled
```

**State Transitions**:
- `PENDING` → `BLOCKED` (if dependencies exist and unmet)
- `PENDING` → `READY` (if no dependencies or all met)
- `BLOCKED` → `READY` (when last dependency completes)
- `READY` → `RUNNING` (when agent picks up task)
- `RUNNING` → `COMPLETED|FAILED|CANCELLED`

### 3.2 TaskSource Enum (NEW)

```python
class TaskSource(str, Enum):
    """Origin of task submission."""
    HUMAN = "human"                          # User/ticket system
    AGENT_REQUIREMENTS = "agent_requirements"  # Requirements gatherer
    AGENT_PLANNER = "agent_planner"           # Task planner
    AGENT_IMPLEMENTATION = "agent_implementation"  # Implementation agent
```

### 3.3 DependencyType Enum (NEW)

```python
class DependencyType(str, Enum):
    """Type of dependency relationship."""
    SEQUENTIAL = "sequential"      # B depends on A completing
    PARALLEL = "parallel"          # C depends on A AND B both completing
```

### 3.4 Enhanced Task Model

```python
class Task(BaseModel):
    """Enhanced task model with dependency support."""

    # Existing fields
    id: UUID = Field(default_factory=uuid4)
    prompt: str
    agent_type: str = "general"
    priority: int = Field(default=5, ge=0, le=10)  # Base priority
    status: TaskStatus = Field(default=TaskStatus.PENDING)
    input_data: dict[str, Any] = Field(default_factory=dict)
    result_data: dict[str, Any] | None = None
    error_message: str | None = None
    retry_count: int = Field(default=0, ge=0)
    max_retries: int = Field(default=3, ge=0)
    max_execution_timeout_seconds: int = Field(default=3600, ge=60)
    submitted_at: datetime = Field(default_factory=lambda: datetime.now(timezone.utc))
    started_at: datetime | None = None
    completed_at: datetime | None = None
    last_updated_at: datetime = Field(default_factory=lambda: datetime.now(timezone.utc))
    session_id: str | None = None

    # NEW: Hierarchical task support
    parent_task_id: UUID | None = None
    source: TaskSource = Field(default=TaskSource.HUMAN)
    created_by: str | None = None  # Agent ID or user ID

    # NEW: Dependency management
    dependencies: list[UUID] = Field(default_factory=list)
    dependency_type: DependencyType = Field(default=DependencyType.SEQUENTIAL)
    blocked_by: list[UUID] = Field(default_factory=list)  # Currently blocking dependencies
    blocking_tasks: list[UUID] = Field(default_factory=list)  # Tasks waiting on this one

    # NEW: Priority calculation fields
    calculated_priority: float = Field(default=5.0)  # Dynamic priority score
    deadline: datetime | None = None
    estimated_duration_seconds: int | None = None
    dependency_depth: int = Field(default=0)  # How deep in dependency tree

    model_config = ConfigDict(
        json_encoders={
            UUID: str,
            datetime: lambda v: v.isoformat(),
        }
    )
```

### 3.5 TaskDependency Model (NEW)

```python
class TaskDependency(BaseModel):
    """Represents a dependency relationship between tasks."""

    id: UUID = Field(default_factory=uuid4)
    dependent_task_id: UUID  # Task that depends
    prerequisite_task_id: UUID  # Task that must complete first
    dependency_type: DependencyType
    created_at: datetime = Field(default_factory=lambda: datetime.now(timezone.utc))
    resolved_at: datetime | None = None  # When prerequisite completed

    model_config = ConfigDict(
        json_encoders={
            UUID: str,
            datetime: lambda v: v.isoformat(),
        }
    )
```

---

## 4. Database Schema Design

### 4.1 Updated Tasks Table

```sql
CREATE TABLE tasks (
    -- Existing columns
    id TEXT PRIMARY KEY,
    prompt TEXT NOT NULL,
    agent_type TEXT NOT NULL DEFAULT 'general',
    priority INTEGER NOT NULL DEFAULT 5,
    status TEXT NOT NULL,
    input_data TEXT NOT NULL,
    result_data TEXT,
    error_message TEXT,
    retry_count INTEGER DEFAULT 0,
    max_retries INTEGER DEFAULT 3,
    max_execution_timeout_seconds INTEGER DEFAULT 3600,
    submitted_at TIMESTAMP NOT NULL,
    started_at TIMESTAMP,
    completed_at TIMESTAMP,
    last_updated_at TIMESTAMP NOT NULL,
    session_id TEXT,

    -- NEW: Hierarchical and source tracking
    parent_task_id TEXT,
    source TEXT NOT NULL DEFAULT 'human',
    created_by TEXT,

    -- NEW: Priority calculation fields
    calculated_priority REAL NOT NULL DEFAULT 5.0,
    deadline TIMESTAMP,
    estimated_duration_seconds INTEGER,
    dependency_depth INTEGER DEFAULT 0,

    -- Constraints
    FOREIGN KEY (parent_task_id) REFERENCES tasks(id) ON DELETE SET NULL,
    FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE SET NULL,
    CHECK(status IN ('pending', 'blocked', 'ready', 'running', 'completed', 'failed', 'cancelled')),
    CHECK(source IN ('human', 'agent_requirements', 'agent_planner', 'agent_implementation')),
    CHECK(priority >= 0 AND priority <= 10),
    CHECK(calculated_priority >= 0)
);
```

### 4.2 NEW: Task Dependencies Table

```sql
CREATE TABLE task_dependencies (
    id TEXT PRIMARY KEY,
    dependent_task_id TEXT NOT NULL,
    prerequisite_task_id TEXT NOT NULL,
    dependency_type TEXT NOT NULL DEFAULT 'sequential',
    created_at TIMESTAMP NOT NULL,
    resolved_at TIMESTAMP,

    -- Constraints
    FOREIGN KEY (dependent_task_id) REFERENCES tasks(id) ON DELETE CASCADE,
    FOREIGN KEY (prerequisite_task_id) REFERENCES tasks(id) ON DELETE CASCADE,
    CHECK(dependency_type IN ('sequential', 'parallel')),
    CHECK(dependent_task_id != prerequisite_task_id),  -- No self-dependencies
    UNIQUE(dependent_task_id, prerequisite_task_id)  -- No duplicate dependencies
);
```

### 4.3 Performance Indexes

```sql
-- Existing indexes (keep)
CREATE INDEX idx_tasks_status_priority
    ON tasks(status, priority DESC, submitted_at ASC);

CREATE INDEX idx_tasks_submitted_at
    ON tasks(submitted_at);

CREATE INDEX idx_tasks_parent
    ON tasks(parent_task_id);

CREATE INDEX idx_tasks_running_timeout
    ON tasks(status, last_updated_at)
    WHERE status = 'running';

CREATE INDEX idx_tasks_session
    ON tasks(session_id, submitted_at DESC)
    WHERE session_id IS NOT NULL;

-- NEW: Dependency resolution indexes
CREATE INDEX idx_task_dependencies_prerequisite
    ON task_dependencies(prerequisite_task_id, resolved_at)
    WHERE resolved_at IS NULL;

CREATE INDEX idx_task_dependencies_dependent
    ON task_dependencies(dependent_task_id, resolved_at)
    WHERE resolved_at IS NULL;

-- NEW: Priority queue index (composite for calculated priority)
CREATE INDEX idx_tasks_ready_priority
    ON tasks(status, calculated_priority DESC, submitted_at ASC)
    WHERE status = 'ready';

-- NEW: Source tracking index
CREATE INDEX idx_tasks_source_created
    ON tasks(source, created_by, submitted_at DESC);

-- NEW: Deadline urgency index
CREATE INDEX idx_tasks_deadline
    ON tasks(deadline, status)
    WHERE deadline IS NOT NULL AND status IN ('pending', 'blocked', 'ready');
```

---

## 5. Service Layer Architecture

### 5.1 TaskQueueService (Enhanced)

```python
class TaskQueueService:
    """Enhanced task queue with dependency management and priority scheduling."""

    def __init__(self, database: Database):
        self.db = database
        self.dependency_resolver = DependencyResolver(database)
        self.priority_calculator = PriorityCalculator()

    async def submit_task(
        self,
        prompt: str,
        agent_type: str = "general",
        priority: int = 5,
        source: TaskSource = TaskSource.HUMAN,
        created_by: str | None = None,
        parent_task_id: UUID | None = None,
        dependencies: list[UUID] | None = None,
        dependency_type: DependencyType = DependencyType.SEQUENTIAL,
        deadline: datetime | None = None,
        estimated_duration_seconds: int | None = None,
        session_id: str | None = None,
        input_data: dict[str, Any] | None = None,
    ) -> Task:
        """Submit a new task with dependency checking."""

        # 1. Detect circular dependencies BEFORE insert
        if dependencies:
            await self.dependency_resolver.check_circular_dependencies(
                dependencies, parent_task_id
            )

        # 2. Calculate initial status based on dependencies
        initial_status = TaskStatus.PENDING
        if dependencies:
            unmet_deps = await self.dependency_resolver.get_unmet_dependencies(dependencies)
            if unmet_deps:
                initial_status = TaskStatus.BLOCKED
            else:
                initial_status = TaskStatus.READY
        else:
            initial_status = TaskStatus.READY

        # 3. Create task object
        task = Task(
            prompt=prompt,
            agent_type=agent_type,
            priority=priority,
            status=initial_status,
            source=source,
            created_by=created_by,
            parent_task_id=parent_task_id,
            dependencies=dependencies or [],
            dependency_type=dependency_type,
            deadline=deadline,
            estimated_duration_seconds=estimated_duration_seconds,
            session_id=session_id,
            input_data=input_data or {},
        )

        # 4. Calculate initial dynamic priority
        task.calculated_priority = await self.priority_calculator.calculate(task)

        # 5. Insert task and dependencies in transaction
        async with self.db._get_connection() as conn:
            await self.db.insert_task(task)

            if dependencies:
                for prereq_id in dependencies:
                    dep = TaskDependency(
                        dependent_task_id=task.id,
                        prerequisite_task_id=prereq_id,
                        dependency_type=dependency_type,
                    )
                    await self._insert_dependency(conn, dep)

            await conn.commit()

        return task

    async def dequeue_next_task(self) -> Task | None:
        """Get next READY task with highest calculated priority."""
        async with self.db._get_connection() as conn:
            cursor = await conn.execute(
                """
                SELECT * FROM tasks
                WHERE status = 'ready'
                ORDER BY calculated_priority DESC, submitted_at ASC
                LIMIT 1
                """,
            )
            row = await cursor.fetchone()
            if row:
                task = self.db._row_to_task(row)
                await self.db.update_task_status(task.id, TaskStatus.RUNNING)
                task.status = TaskStatus.RUNNING
                task.started_at = datetime.now(timezone.utc)
                return task
            return None

    async def complete_task(self, task_id: UUID, result_data: dict[str, Any] | None = None) -> None:
        """Mark task complete and unblock dependent tasks."""
        async with self.db._get_connection() as conn:
            # 1. Update task status
            await self.db.update_task_status(task_id, TaskStatus.COMPLETED)

            # 2. Update result data
            if result_data:
                await conn.execute(
                    "UPDATE tasks SET result_data = ? WHERE id = ?",
                    (json.dumps(result_data), str(task_id)),
                )

            # 3. Resolve dependencies
            await conn.execute(
                """
                UPDATE task_dependencies
                SET resolved_at = ?
                WHERE prerequisite_task_id = ? AND resolved_at IS NULL
                """,
                (datetime.now(timezone.utc).isoformat(), str(task_id)),
            )

            # 4. Find tasks to unblock
            cursor = await conn.execute(
                """
                SELECT DISTINCT td.dependent_task_id
                FROM task_dependencies td
                WHERE td.prerequisite_task_id = ?
                AND td.resolved_at IS NOT NULL
                """,
                (str(task_id),),
            )
            potentially_unblocked = [UUID(row[0]) for row in await cursor.fetchall()]

            # 5. For each potentially unblocked task, check if ALL dependencies met
            for dep_task_id in potentially_unblocked:
                all_met = await self.dependency_resolver.are_all_dependencies_met(dep_task_id)
                if all_met:
                    # Update status from BLOCKED → READY
                    await conn.execute(
                        "UPDATE tasks SET status = 'ready' WHERE id = ?",
                        (str(dep_task_id),),
                    )
                    # Recalculate priority (now ready to run)
                    task = await self.db.get_task(dep_task_id)
                    new_priority = await self.priority_calculator.calculate(task)
                    await conn.execute(
                        "UPDATE tasks SET calculated_priority = ? WHERE id = ?",
                        (new_priority, str(dep_task_id)),
                    )

            await conn.commit()

    async def recalculate_all_priorities(self) -> None:
        """Periodically recalculate priorities for pending/ready tasks."""
        tasks = await self.db.list_tasks(status=None)
        for task in tasks:
            if task.status in [TaskStatus.PENDING, TaskStatus.BLOCKED, TaskStatus.READY]:
                new_priority = await self.priority_calculator.calculate(task)
                async with self.db._get_connection() as conn:
                    await conn.execute(
                        "UPDATE tasks SET calculated_priority = ? WHERE id = ?",
                        (new_priority, str(task.id)),
                    )
                    await conn.commit()
```

### 5.2 DependencyResolver

```python
class DependencyResolver:
    """Handles dependency graph operations."""

    def __init__(self, database: Database):
        self.db = database

    async def check_circular_dependencies(
        self,
        new_dependencies: list[UUID],
        task_id: UUID | None = None
    ) -> None:
        """Detect circular dependencies using DFS.

        Raises:
            CircularDependencyError: If adding these dependencies creates a cycle
        """
        # Build dependency graph
        graph = await self._build_dependency_graph()

        # Add new edges
        if task_id:
            for dep in new_dependencies:
                if self._creates_cycle(graph, task_id, dep):
                    raise CircularDependencyError(
                        f"Adding dependency {dep} to task {task_id} creates circular dependency"
                    )

    def _creates_cycle(self, graph: dict[UUID, list[UUID]], source: UUID, target: UUID) -> bool:
        """Check if adding edge source → target creates cycle using DFS."""
        visited = set()

        def dfs(node: UUID) -> bool:
            if node == source:
                return True  # Cycle detected
            if node in visited:
                return False
            visited.add(node)
            for neighbor in graph.get(node, []):
                if dfs(neighbor):
                    return True
            return False

        return dfs(target)

    async def _build_dependency_graph(self) -> dict[UUID, list[UUID]]:
        """Build adjacency list of task dependencies."""
        async with self.db._get_connection() as conn:
            cursor = await conn.execute(
                "SELECT dependent_task_id, prerequisite_task_id FROM task_dependencies WHERE resolved_at IS NULL"
            )
            rows = await cursor.fetchall()

        graph: dict[UUID, list[UUID]] = {}
        for row in rows:
            dependent = UUID(row[0])
            prerequisite = UUID(row[1])
            if prerequisite not in graph:
                graph[prerequisite] = []
            graph[prerequisite].append(dependent)

        return graph

    async def get_unmet_dependencies(self, dependency_ids: list[UUID]) -> list[UUID]:
        """Get dependencies that haven't completed yet."""
        async with self.db._get_connection() as conn:
            placeholders = ','.join(['?' for _ in dependency_ids])
            cursor = await conn.execute(
                f"""
                SELECT id FROM tasks
                WHERE id IN ({placeholders})
                AND status NOT IN ('completed', 'cancelled')
                """,
                [str(dep_id) for dep_id in dependency_ids],
            )
            unmet = [UUID(row[0]) for row in await cursor.fetchall()]
        return unmet

    async def are_all_dependencies_met(self, task_id: UUID) -> bool:
        """Check if all dependencies for a task are met."""
        async with self.db._get_connection() as conn:
            cursor = await conn.execute(
                """
                SELECT COUNT(*) FROM task_dependencies
                WHERE dependent_task_id = ? AND resolved_at IS NULL
                """,
                (str(task_id),),
            )
            unmet_count = (await cursor.fetchone())[0]
        return unmet_count == 0
```

### 5.3 PriorityCalculator

```python
class PriorityCalculator:
    """Calculates dynamic priority scores based on Chapter 20 patterns."""

    def __init__(self):
        # Tunable weights for priority factors
        self.base_weight = 1.0
        self.urgency_weight = 2.0
        self.dependency_weight = 1.5
        self.starvation_weight = 0.5
        self.source_weight = 1.0

    async def calculate(self, task: Task) -> float:
        """Calculate dynamic priority score.

        Formula:
        priority = base_priority * base_weight
                   + urgency_score * urgency_weight
                   + dependency_score * dependency_weight
                   + starvation_score * starvation_weight
                   + source_score * source_weight
        """
        base_score = task.priority * self.base_weight
        urgency_score = self._calculate_urgency(task)
        dependency_score = await self._calculate_dependency_boost(task)
        starvation_score = self._calculate_starvation_prevention(task)
        source_score = self._calculate_source_boost(task)

        total = (
            base_score
            + urgency_score * self.urgency_weight
            + dependency_score * self.dependency_weight
            + starvation_score * self.starvation_weight
            + source_score * self.source_weight
        )

        return max(0.0, total)  # Ensure non-negative

    def _calculate_urgency(self, task: Task) -> float:
        """Calculate urgency based on deadline proximity.

        Returns:
            0.0 (no deadline) to 10.0 (deadline imminent)
        """
        if not task.deadline:
            return 0.0

        now = datetime.now(timezone.utc)
        time_remaining = (task.deadline - now).total_seconds()

        if time_remaining <= 0:
            return 10.0  # Past deadline - maximum urgency

        # Scale: 1 day = 5.0, 1 hour = 8.0, 1 min = 9.5
        if time_remaining < 60:  # < 1 minute
            return 9.5
        elif time_remaining < 3600:  # < 1 hour
            return 8.0
        elif time_remaining < 86400:  # < 1 day
            return 5.0
        elif time_remaining < 604800:  # < 1 week
            return 2.0
        else:
            return 0.5

    async def _calculate_dependency_boost(self, task: Task) -> float:
        """Calculate boost based on tasks blocked by this one.

        Returns:
            0.0 (no tasks waiting) to 5.0 (many tasks blocked)
        """
        # Count tasks blocked by this one
        blocked_count = len(task.blocking_tasks)

        if blocked_count == 0:
            return 0.0
        elif blocked_count == 1:
            return 1.0
        elif blocked_count < 5:
            return 2.0
        elif blocked_count < 10:
            return 3.5
        else:
            return 5.0  # Many tasks blocked - high priority

    def _calculate_starvation_prevention(self, task: Task) -> float:
        """Prevent task starvation by boosting long-waiting tasks.

        Returns:
            0.0 (recently submitted) to 3.0 (starving)
        """
        now = datetime.now(timezone.utc)
        wait_time = (now - task.submitted_at).total_seconds()

        if wait_time < 3600:  # < 1 hour
            return 0.0
        elif wait_time < 86400:  # < 1 day
            return 0.5
        elif wait_time < 604800:  # < 1 week
            return 1.5
        else:  # > 1 week
            return 3.0

    def _calculate_source_boost(self, task: Task) -> float:
        """Boost priority based on task source.

        Human tasks get higher priority than agent-generated subtasks.

        Returns:
            0.0 to 2.0
        """
        if task.source == TaskSource.HUMAN:
            return 2.0  # Human tickets highest priority
        elif task.source == TaskSource.AGENT_REQUIREMENTS:
            return 1.5
        elif task.source == TaskSource.AGENT_PLANNER:
            return 1.0
        elif task.source == TaskSource.AGENT_IMPLEMENTATION:
            return 0.5
        else:
            return 0.0
```

---

## 6. API Interface Specifications

### 6.1 Agent Task Submission API

```python
# For agents to submit subtasks programmatically
async def submit_subtask(
    parent_task_id: UUID,
    prompt: str,
    agent_type: str = "general",
    priority: int = 5,
    dependencies: list[UUID] | None = None,
    deadline: datetime | None = None,
    estimated_duration_seconds: int | None = None,
) -> Task:
    """
    Submit a subtask as an agent.

    Args:
        parent_task_id: ID of parent task creating this subtask
        prompt: Task instruction
        agent_type: Agent specialization to handle this task
        priority: Base priority (0-10)
        dependencies: List of task IDs that must complete first
        deadline: Optional deadline for urgency calculation
        estimated_duration_seconds: Estimated execution time

    Returns:
        Created Task object

    Raises:
        CircularDependencyError: If dependencies create a cycle
        TaskNotFoundError: If parent_task_id doesn't exist
    """
    pass
```

### 6.2 Dependency Query API

```python
async def get_task_dependencies(task_id: UUID) -> list[TaskDependency]:
    """Get all dependencies for a task."""
    pass

async def get_blocked_tasks(task_id: UUID) -> list[Task]:
    """Get tasks blocked by this task."""
    pass

async def get_dependency_chain(task_id: UUID) -> list[list[Task]]:
    """Get full dependency chain (topological sort)."""
    pass
```

---

## 7. Implementation Roadmap

### Phase 1: Schema & Domain Models (2 days)
**Goal**: Database schema updates, enhanced models

**Deliverables**:
1. Database migration script (add new columns, task_dependencies table)
2. Updated domain models (TaskStatus, TaskSource, DependencyType enums)
3. Enhanced Task model with new fields
4. TaskDependency model
5. Database indexes for dependency queries
6. Unit tests for models

**Acceptance Criteria**:
- All migrations run successfully on existing database
- No data loss during migration
- Foreign key constraints enforced
- Indexes created and validated

**Validation Gate**: Schema integrity check, performance baseline

---

### Phase 2: Dependency Resolution (3 days)
**Goal**: Dependency graph operations, circular detection

**Deliverables**:
1. DependencyResolver service implementation
2. Circular dependency detection algorithm (DFS)
3. Dependency graph builder
4. Unmet dependency checker
5. Integration tests for dependency scenarios
6. Performance tests (100-task dependency graph in <10ms)

**Acceptance Criteria**:
- Circular dependencies detected before insert
- Dependency graph correctly built from database
- Unmet dependencies identified accurately
- Performance: <10ms for 100-task graph

**Validation Gate**: Algorithm correctness, performance validation

---

### Phase 3: Priority Calculation (2 days)
**Goal**: Dynamic priority scoring algorithm

**Deliverables**:
1. PriorityCalculator service implementation
2. Urgency calculation (deadline proximity)
3. Dependency boost calculation (blocking tasks)
4. Starvation prevention calculation (wait time)
5. Source boost calculation (human vs agent)
6. Unit tests for each factor
7. Integration tests for combined scoring

**Acceptance Criteria**:
- Priority formula correctly implemented
- Weights tunable via configuration
- Priority recalculation completes in <5ms per task
- Edge cases handled (no deadline, past deadline, etc.)

**Validation Gate**: Priority calculation accuracy, performance validation

---

### Phase 4: Task Queue Service (3 days)
**Goal**: Enhanced task queue with dependency enforcement

**Deliverables**:
1. TaskQueueService refactor/enhancement
2. submit_task with dependency checking
3. dequeue_next_task (prioritizes READY tasks)
4. complete_task with dependency resolution
5. recalculate_all_priorities method
6. Agent submission API
7. Integration tests for full workflows
8. Performance tests (1000+ tasks/sec enqueue)

**Acceptance Criteria**:
- Tasks with dependencies enter BLOCKED status
- Dependencies automatically resolved on completion
- Dependent tasks unblocked correctly
- Priority queue returns highest calculated_priority task
- Performance: 1000+ tasks/sec enqueue

**Validation Gate**: End-to-end workflow validation, performance validation

---

### Phase 5: Integration & Testing (2 days)
**Goal**: System-wide integration, comprehensive testing

**Deliverables**:
1. Integration with existing Agent model
2. Integration with session/memory system
3. Hierarchical workflow tests (Requirements → Planner → Implementation)
4. Performance benchmarks (report)
5. Documentation updates
6. Example usage code

**Acceptance Criteria**:
- All acceptance criteria from requirements met
- Performance targets achieved
- Integration tests pass
- Documentation complete

**Validation Gate**: Final acceptance testing, go/no-go for production

---

## 8. Agent Team Composition

### Core Management Agents

1. **task-queue-orchestrator** (Sonnet)
   - Coordinates implementation phases
   - Validates deliverables at phase gates
   - Makes go/no-go decisions

### Specialized Implementation Agents

2. **database-schema-architect** (Sonnet)
   - Designs schema updates
   - Creates migration scripts
   - Validates data integrity

3. **algorithm-design-specialist** (Opus)
   - Implements dependency resolution (DFS)
   - Implements priority calculation algorithm
   - Proves algorithmic correctness

4. **python-backend-developer** (Opus)
   - Implements TaskQueueService
   - Implements DependencyResolver
   - Implements PriorityCalculator

5. **test-automation-engineer** (Opus)
   - Writes unit tests
   - Writes integration tests
   - Writes performance tests

6. **performance-optimization-specialist** (Opus)
   - Analyzes query plans
   - Optimizes indexes
   - Runs performance benchmarks

### Support Agents

7. **technical-documentation-writer** (Haiku)
   - API documentation
   - Usage examples
   - Architecture diagrams

8. **python-debugging-specialist** (Opus)
   - Error escalation for implementation agents
   - Debugging complex issues
   - Performance profiling

---

## 9. Performance Targets & Validation

### 9.1 Performance Targets

| Operation | Target | Validation Method |
|-----------|--------|-------------------|
| Task enqueue | 1000+ tasks/sec | Performance test: insert 10k tasks, measure time |
| Dependency resolution | <10ms for 100 tasks | Benchmark: build graph, detect cycles |
| Priority calculation | <5ms per task | Benchmark: calculate priority 1000 times |
| Dequeue next task | <5ms | Query plan analysis, benchmark |
| Complete task + unblock | <20ms | Transaction time measurement |

### 9.2 Validation Methods

**Schema Validation**:
```python
async def validate_schema():
    # Check all columns exist
    # Check indexes created
    # Check foreign keys enforced
    # Run PRAGMA foreign_key_check
    # Measure index usage
```

**Algorithm Validation**:
```python
async def validate_dependency_resolution():
    # Test circular detection (should reject)
    # Test valid dependency chain (should accept)
    # Test complex graph (100 nodes, 200 edges)
    # Measure time: assert < 10ms
```

**Priority Validation**:
```python
async def validate_priority_calculation():
    # Test urgency: deadline in 1 hour → high score
    # Test dependency boost: 10 blocked tasks → high score
    # Test starvation: 1 week wait → boost score
    # Test source: HUMAN > AGENT_* sources
    # Measure time: assert < 5ms
```

**Integration Validation**:
```python
async def validate_end_to_end_workflow():
    # Submit human task (no dependencies)
    # Submit requirements task (depends on human task)
    # Submit planner tasks (depends on requirements)
    # Submit implementation tasks (depends on planner)
    # Assert: correct status transitions (PENDING → BLOCKED → READY → RUNNING → COMPLETED)
    # Assert: priorities calculated correctly
    # Assert: dependencies resolved automatically
    # Measure: total workflow time
```

---

## 10. Risk Assessment & Mitigation

### Risk 1: Circular Dependency Detection Performance
**Risk**: DFS algorithm too slow for large graphs
**Mitigation**:
- Cache dependency graph in memory
- Limit dependency chain depth (e.g., max 10 levels)
- Benchmark early, optimize if needed

### Risk 2: Database Lock Contention
**Risk**: High concurrency causes SQLite lock timeouts
**Mitigation**:
- Use WAL mode (already enabled)
- Keep transactions short
- Batch dependency updates where possible

### Risk 3: Priority Recalculation Overhead
**Risk**: Recalculating priorities for all tasks too expensive
**Mitigation**:
- Only recalculate PENDING/BLOCKED/READY tasks
- Run recalculation periodically (not every task update)
- Use indexes for fast status filtering

### Risk 4: Complex Dependency Graphs
**Risk**: Users create overly complex dependency graphs
**Mitigation**:
- Limit max dependencies per task (e.g., 20)
- Warn when dependency depth exceeds threshold
- Provide tools to visualize dependency graph

---

## 11. Next Steps

1. **Review & Approval**: Review architecture with project stakeholders
2. **Agent Team Creation**: Create specialized agents using agent creation template
3. **Phase 1 Kickoff**: Launch database-schema-architect for schema design
4. **Iterative Development**: Execute phases with validation gates
5. **Performance Monitoring**: Track performance metrics at each phase
6. **Documentation**: Maintain living documentation as implementation progresses

---

## Appendix A: Example Workflows

### Workflow 1: Hierarchical Task Breakdown

```
Human submits: "Implement user authentication system"
  ↓ (TaskSource.HUMAN, priority=8)

Requirements-gatherer creates:
  - Task: "Define authentication requirements" (TaskSource.AGENT_REQUIREMENTS, depends on human task)
  - Task: "Design auth database schema" (TaskSource.AGENT_REQUIREMENTS, depends on requirements)
  ↓

Task-planner creates:
  - Task: "Implement JWT token generation" (TaskSource.AGENT_PLANNER, depends on schema)
  - Task: "Implement password hashing" (TaskSource.AGENT_PLANNER, depends on schema)
  - Task: "Implement login endpoint" (TaskSource.AGENT_PLANNER, depends on JWT + password)
  ↓

Implementation agents execute in order based on dependencies and priorities
```

### Workflow 2: Parallel Prerequisites

```
Task A: "Fetch user data from API" (no dependencies) → READY
Task B: "Fetch product catalog from API" (no dependencies) → READY
Task C: "Generate recommendation report" (depends on A AND B) → BLOCKED

When A completes: C still BLOCKED (waiting for B)
When B completes: C → READY (both prerequisites met)
```

---

## Appendix B: Configuration Parameters

```python
# Priority Calculation Weights (tunable)
PRIORITY_WEIGHTS = {
    "base_weight": 1.0,
    "urgency_weight": 2.0,
    "dependency_weight": 1.5,
    "starvation_weight": 0.5,
    "source_weight": 1.0,
}

# Dependency Limits
MAX_DEPENDENCIES_PER_TASK = 20
MAX_DEPENDENCY_DEPTH = 10
CIRCULAR_DEPENDENCY_CHECK_TIMEOUT_MS = 100

# Priority Recalculation
PRIORITY_RECALC_INTERVAL_SECONDS = 300  # 5 minutes

# Performance Thresholds
ENQUEUE_THROUGHPUT_TARGET = 1000  # tasks/sec
DEPENDENCY_RESOLUTION_TARGET_MS = 10
PRIORITY_CALC_TARGET_MS = 5
DEQUEUE_TARGET_MS = 5
```

---

**End of Architecture Document**
