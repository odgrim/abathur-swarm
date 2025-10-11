# Task Queue System Migration Guide

## Overview

This guide helps users migrate from a simple task queue system to the enhanced Abathur Task Queue System with advanced dependency management and dynamic prioritization.

## What's New

### Key Enhancements
- Hierarchical task submission
- Automatic dependency management
- Dynamic priority calculation
- Source tracking for tasks
- Advanced status management

## Migration Steps

### 1. Database Schema Update

#### Old Schema
```sql
CREATE TABLE tasks (
    id TEXT PRIMARY KEY,
    description TEXT NOT NULL,
    status TEXT NOT NULL,
    priority INTEGER
);
```

#### New Schema
```sql
CREATE TABLE tasks (
    id TEXT PRIMARY KEY,
    prompt TEXT NOT NULL,
    status TEXT NOT NULL,
    priority INTEGER,
    -- New fields
    source TEXT NOT NULL,
    parent_task_id TEXT,
    dependencies TEXT,
    calculated_priority REAL,
    deadline TIMESTAMP
);

CREATE TABLE task_dependencies (
    id TEXT PRIMARY KEY,
    dependent_task_id TEXT NOT NULL,
    prerequisite_task_id TEXT NOT NULL,
    dependency_type TEXT NOT NULL,
    resolved_at TIMESTAMP
);
```

**Migration Script:**
```python
async def migrate_task_queue():
    # 1. Add new columns to tasks table
    await database.execute("""
        ALTER TABLE tasks
        ADD COLUMN source TEXT NOT NULL DEFAULT 'human';
        ALTER TABLE tasks
        ADD COLUMN parent_task_id TEXT;
        ALTER TABLE tasks
        ADD COLUMN calculated_priority REAL;
        ALTER TABLE tasks
        ADD COLUMN deadline TIMESTAMP;
    """)

    # 2. Create task_dependencies table
    await database.execute("""
        CREATE TABLE task_dependencies (
            id TEXT PRIMARY KEY,
            dependent_task_id TEXT NOT NULL,
            prerequisite_task_id TEXT NOT NULL,
            dependency_type TEXT NOT NULL DEFAULT 'sequential',
            resolved_at TIMESTAMP,

            FOREIGN KEY (dependent_task_id) REFERENCES tasks(id),
            FOREIGN KEY (prerequisite_task_id) REFERENCES tasks(id)
        );
    """)

    # 3. Update task statuses
    await database.execute("""
        UPDATE tasks
        SET status =
            CASE
                WHEN status = 'pending' AND EXISTS (
                    SELECT 1 FROM task_dependencies
                    WHERE dependent_task_id = tasks.id
                ) THEN 'blocked'
                ELSE 'ready'
            END
    """)
```

### 2. Code Changes

#### Before (Simple Task Queue)
```python
class SimpleTaskQueue:
    async def submit_task(self, description: str, priority: int = 5):
        task = Task(description=description, priority=priority)
        await self.database.insert(task)
        return task

    async def get_next_task(self):
        tasks = await self.database.list_tasks(status='pending')
        return max(tasks, key=lambda t: t.priority) if tasks else None
```

#### After (Enhanced Task Queue)
```python
from abathur.services import TaskQueueService
from abathur.domain.models import TaskSource, DependencyType

class EnhancedTaskQueue:
    def __init__(self):
        self.queue_service = TaskQueueService()

    async def submit_task(
        self,
        prompt: str,
        priority: int = 5,
        dependencies: list[UUID] = None
    ):
        task = await self.queue_service.submit_task(
            prompt=prompt,
            priority=priority,
            source=TaskSource.HUMAN,
            dependencies=dependencies,
            dependency_type=DependencyType.SEQUENTIAL
        )
        return task

    async def get_next_task(self):
        return await self.queue_service.get_next_task()
```

### 3. Status Management

#### Old Statuses
- `pending`
- `completed`
- `failed`

#### New Statuses
- `PENDING`: Task submitted, no dependencies
- `BLOCKED`: Task waiting for dependencies
- `READY`: Dependencies met, ready to execute
- `RUNNING`: Currently executing
- `COMPLETED`: Successfully finished
- `FAILED`: Execution failed
- `CANCELLED`: Manually stopped

### 4. Dependency Handling

#### Old Approach
- Manual dependency tracking
- No automatic blocking/unblocking

#### New Approach
- Automatic dependency resolution
- Circular dependency prevention
- Automatic status updates
- Hierarchical task breakdown

### Example Migration

```python
# Old code
def process_authentication_task():
    schema_task = task_queue.submit_task("Design database schema")
    jwt_task = task_queue.submit_task("Implement JWT")
    # Manual tracking of dependencies

# New code
async def process_authentication_task():
    queue_service = TaskQueueService()

    # Parent task for entire authentication system
    auth_task = await queue_service.submit_task(
        prompt="Implement user authentication",
        source=TaskSource.HUMAN,
        priority=8
    )

    # Subtasks with automatic dependency management
    schema_task = await queue_service.submit_task(
        prompt="Design database schema",
        source=TaskSource.AGENT_REQUIREMENTS,
        parent_task_id=auth_task.id,
        priority=7
    )

    jwt_task = await queue_service.submit_task(
        prompt="Implement JWT generation",
        source=TaskSource.AGENT_PLANNER,
        dependencies=[schema_task.id],
        parent_task_id=auth_task.id,
        priority=6
    )
```

## Breaking Changes

1. Tasks now have more complex lifecycle
2. Requires explicit dependency management
3. New status tracking mechanism
4. Dynamic priority calculation
5. Source tracking mandatory

## Configuration & Tuning

### Priority Calculation
```python
PRIORITY_WEIGHTS = {
    "base_weight": 1.0,
    "urgency_weight": 2.0,
    "dependency_weight": 1.5,
    "starvation_weight": 0.5,
    "source_weight": 1.0,
}
```

### Dependency Limits
```python
MAX_DEPENDENCIES_PER_TASK = 20
MAX_DEPENDENCY_DEPTH = 10
```

## Troubleshooting

### Common Migration Issues
- Existing tasks without source
- Missing dependency records
- Status transition complexities

### Recommendations
- Test migration on staging environment
- Validate data integrity
- Monitor system performance during transition

## Performance Impact

- Slightly increased query complexity
- More sophisticated priority calculation
- Better scalability for complex workflows

## Conclusion

The enhanced task queue system provides a more robust, flexible approach to task management with intelligent dependency resolution and prioritization.
