# Task Queue System API Reference

## Table of Contents
1. [TaskQueueService](#taskqueueservice)
2. [DependencyResolver](#dependencyresolver)
3. [PriorityCalculator](#prioritycalculator)

## TaskQueueService

The primary service for task queue operations, providing methods for task submission, retrieval, and management.

### `submit_task`

Submit a new task to the queue.

```python
async def submit_task(
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
) -> Task
```

**Parameters:**
- `prompt`: Task description/instruction
- `agent_type`: Specialized agent type (default: "general")
- `priority`: Base priority (0-10, default: 5)
- `source`: Task origin (default: TaskSource.HUMAN)
- `created_by`: Agent or user ID creating the task
- `parent_task_id`: ID of parent task for hierarchical breakdown
- `dependencies`: List of prerequisite task IDs
- `dependency_type`: Dependency relationship (SEQUENTIAL or PARALLEL)
- `deadline`: Optional task deadline for urgency calculation
- `estimated_duration_seconds`: Expected task execution time
- `session_id`: Optional session context
- `input_data`: Optional input data for task execution

**Returns:** Created `Task` object

**Raises:**
- `CircularDependencyError`: If dependencies create a cycle
- `TaskNotFoundError`: If a dependency task does not exist

**Example:**
```python
task = await queue_service.submit_task(
    prompt="Implement JWT token generation",
    source=TaskSource.AGENT_PLANNER,
    priority=6,
    dependencies=[schema_task_id],
    deadline=datetime.now() + timedelta(days=7)
)
```

### `get_next_task`

Retrieve the next task to execute, prioritized by calculated priority.

```python
async def get_next_task() -> Task | None
```

**Returns:**
- Next `Task` with highest calculated priority
- `None` if no tasks are ready

**Example:**
```python
next_task = await queue_service.get_next_task()
if next_task:
    # Process the task
```

### `complete_task`

Mark a task as completed and process its dependencies.

```python
async def complete_task(
    task_id: UUID,
    result_data: dict[str, Any] | None = None
) -> list[UUID]
```

**Parameters:**
- `task_id`: ID of the completed task
- `result_data`: Optional result data from task execution

**Returns:** List of task IDs unblocked by this task's completion

**Example:**
```python
unblocked_tasks = await queue_service.complete_task(
    task_id,
    result_data={"jwt_token": generated_token}
)
```

### `fail_task`

Mark a task as failed and handle potential cascade effects.

```python
async def fail_task(
    task_id: UUID,
    error_message: str
) -> list[UUID]
```

**Parameters:**
- `task_id`: ID of the failed task
- `error_message`: Detailed error description

**Returns:** List of task IDs potentially impacted by this failure

**Example:**
```python
impacted_tasks = await queue_service.fail_task(
    task_id,
    error_message="JWT generation failed: invalid secret"
)
```

### `cancel_task`

Cancel a task and optionally cascade cancellation to dependent tasks.

```python
async def cancel_task(task_id: UUID) -> list[UUID]
```

**Parameters:**
- `task_id`: ID of task to cancel

**Returns:** List of tasks cancelled due to dependency chain

**Example:**
```python
cancelled_tasks = await queue_service.cancel_task(task_id)
```

### `get_queue_status`

Retrieve current task queue statistics.

```python
async def get_queue_status() -> dict[str, Any]
```

**Returns:** Dictionary with queue metrics:
- Total tasks
- Tasks by status
- Estimated processing time
- Resource utilization

**Example:**
```python
status = await queue_service.get_queue_status()
print(f"Pending tasks: {status['pending_tasks']}")
```

### `get_task_execution_plan`

Generate a dependency-aware execution plan for given tasks.

```python
async def get_task_execution_plan(task_ids: list[UUID]) -> list[list[UUID]]
```

**Parameters:**
- `task_ids`: Tasks to include in execution plan

**Returns:** Topologically sorted lists of task IDs, representing execution stages

**Example:**
```python
execution_plan = await queue_service.get_task_execution_plan([task1, task2, task3])
```

## DependencyResolver

Service for managing task dependencies and graph operations.

### `detect_circular_dependencies`

Check if adding new dependencies creates a dependency cycle.

```python
async def detect_circular_dependencies(
    prerequisite_ids: list[UUID],
    task_id: UUID
) -> None
```

**Raises:** `CircularDependencyError` if a cycle is detected

### `calculate_dependency_depth`

Calculate how deep a task is in the dependency hierarchy.

```python
async def calculate_dependency_depth(task_id: UUID) -> int
```

**Returns:** Depth of task in dependency tree

### `get_execution_order`

Determine optimal task execution order using topological sorting.

```python
async def get_execution_order(task_ids: list[UUID]) -> list[UUID]
```

**Returns:** Tasks sorted for sequential execution

### `are_all_dependencies_met`

Check if all prerequisites for a task have been completed.

```python
async def are_all_dependencies_met(task_id: UUID) -> bool
```

**Returns:** `True` if all dependencies completed, `False` otherwise

## PriorityCalculator

Dynamic priority calculation service.

### `calculate`

Calculate the dynamic priority score for a task.

```python
async def calculate(task: Task) -> float
```

**Returns:** Calculated priority score (0.0 - 10.0)

**Calculation Factors:**
- Base priority
- Deadline proximity
- Dependency blocking impact
- Task waiting time
- Task source

**Configurable Weights:**
```python
PRIORITY_WEIGHTS = {
    "base_weight": 1.0,
    "urgency_weight": 2.0,
    "dependency_weight": 1.5,
    "starvation_weight": 0.5,
    "source_weight": 1.0,
}
```

**Example:**
```python
priority_score = await priority_calculator.calculate(task)
```

---

## Performance Considerations

- **Dependency Resolution:** <10ms for 100 tasks
- **Priority Calculation:** <5ms per task
- **Task Dequeue:** <5ms
