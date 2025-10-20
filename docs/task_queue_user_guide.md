# Task Queue System User Guide

## Introduction

The Abathur Task Queue System is an advanced, dependency-aware task management system designed to help agents and users break down complex work into manageable, interconnected tasks. This system provides powerful features for hierarchical task submission, automatic dependency management, and intelligent priority scheduling.

## Key Concepts

### Tasks
A task represents a unit of work with the following key attributes:
- **Description**: A clear instruction or goal
- **Priority**: Importance level (0-10)
- **Status**: Current state of the task (PENDING, BLOCKED, READY, RUNNING, COMPLETED, FAILED, CANCELLED)
- **Dependencies**: Other tasks that must complete before this task

### Task Sources
Tasks can originate from different sources:
- **HUMAN**: Directly submitted by a user
- **AGENT_REQUIREMENTS**: Created by requirements-gathering agents
- **AGENT_PLANNER**: Created by task planning agents
- **AGENT_IMPLEMENTATION**: Created by implementation agents

### Dependency Types
The system supports two types of dependencies:
- **Sequential**: Task B requires Task A to complete first
- **Parallel**: Task C requires both Task A and Task B to complete

## Getting Started

### Submitting a Basic Task

```python
from abathur.services import TaskQueueService
from abathur.domain.models import TaskSource

async def submit_simple_task():
    queue_service = TaskQueueService()
    task = await queue_service.submit_task(
        prompt="Implement user authentication system",
        source=TaskSource.HUMAN,
        priority=8
    )
```

### Submitting a Task with Dependencies

```python
async def submit_task_with_dependencies():
    queue_service = TaskQueueService()

    # First task: Database schema design
    schema_task = await queue_service.submit_task(
        prompt="Design authentication database schema",
        source=TaskSource.AGENT_REQUIREMENTS,
        priority=7
    )

    # Second task: JWT implementation (depends on schema task)
    jwt_task = await queue_service.submit_task(
        prompt="Implement JWT token generation",
        source=TaskSource.AGENT_PLANNER,
        priority=6,
        dependencies=[schema_task.id]
    )
```

## Advanced Features

### Parallel Dependencies

When a task requires multiple prerequisite tasks to complete:

```python
async def parallel_dependencies_example():
    queue_service = TaskQueueService()

    # Independent tasks
    user_data_task = await queue_service.submit_task(
        prompt="Fetch user data from API",
        priority=5
    )
    catalog_task = await queue_service.submit_task(
        prompt="Fetch product catalog from API",
        priority=5
    )

    # Task requiring both previous tasks
    recommendation_task = await queue_service.submit_task(
        prompt="Generate recommendation report",
        priority=8,
        dependencies=[user_data_task.id, catalog_task.id]
    )
```

### Priority Calculation

The system dynamically calculates task priority based on:
- Base priority
- Deadline proximity
- Dependency impact
- Task source
- Wait time (starvation prevention)

### Hierarchical Task Breakdown

Agents can create subtasks to decompose complex work:

```python
async def hierarchical_task_breakdown():
    queue_service = TaskQueueService()

    # Parent task
    auth_system_task = await queue_service.submit_task(
        prompt="Implement user authentication system",
        source=TaskSource.HUMAN,
        priority=8
    )

    # Subtasks with parent relationship and dependencies
    requirements_task = await queue_service.submit_task(
        prompt="Define authentication requirements",
        source=TaskSource.AGENT_REQUIREMENTS,
        parent_task_id=auth_system_task.id,
        dependencies=[auth_system_task.id],
        priority=7
    )
```

## Best Practices

1. Break complex tasks into smaller, manageable subtasks
2. Use dependencies to enforce logical task order
3. Set realistic deadlines to help priority calculation
4. Leverage different task sources for clear workflow tracking
5. Monitor task status and handle failures gracefully

## Troubleshooting

### Common Issues

- **BLOCKED Tasks**: Tasks waiting for dependencies to complete
- **Circular Dependencies**: Avoid creating dependency cycles
- **Priority Conflicts**: Use base priority and deadlines to manage

### Debugging Tips

- Use `get_task_dependencies()` to inspect task relationships
- Use `get_dependency_chain()` to visualize task dependencies
- Check task status and error messages for detailed information

## Task Deletion and Database Maintenance

### Pruning Tasks

The `prune` command removes old or completed tasks from the database to manage storage and improve performance.

#### Basic Usage

```bash
# Delete a specific task by ID
abathur task prune ebec23ad --force

# Delete all completed tasks
abathur task prune --status completed --force

# Delete tasks older than 30 days
abathur task prune --older-than 30d --force

# Preview what would be deleted (dry-run)
abathur task prune --older-than 30d --dry-run
```

### Understanding VACUUM Performance

When deleting tasks, SQLite marks the space as "free" but doesn't immediately return it to the operating system. The VACUUM operation reclaims this space by rebuilding the database file.

**Trade-off:** VACUUM reclaims disk space but takes time and locks the database during execution.

#### VACUUM Modes

Control VACUUM behavior with the `--vacuum` flag:

**`--vacuum=conditional` (default)**
- Runs VACUUM only if ≥100 tasks are deleted
- **Recommended for most operations**
- Balances performance and space reclamation

```bash
# Default behavior - VACUUM runs if deleting ≥100 tasks
abathur task prune --older-than 30d --force
```

**`--vacuum=never`**
- Never runs VACUUM, fastest deletion
- **Recommended for large deletions (>10,000 tasks)**
- Prevents multi-minute delays

```bash
# Fast deletion for large batches
abathur task prune --older-than 180d --vacuum=never --force
```

**`--vacuum=always`**
- Always runs VACUUM, regardless of deletion count
- Use when disk space is critical
- **Warning:** May cause multi-minute delays for large databases

```bash
# Always reclaim space immediately
abathur task prune --older-than 30d --vacuum=always --force
```

### Performance Guidelines by Database Size

| Database Size | Task Count | VACUUM Duration | Recommendation |
|---------------|------------|-----------------|----------------|
| Small         | < 1,000    | < 1 second      | Use default (`conditional`) |
| Medium        | 1,000 - 10,000 | 1-10 seconds | Use default (`conditional`) |
| Large         | 10,000 - 100,000 | 10 sec - 2 min | Use `--vacuum=never` for bulk operations |
| Very Large    | > 100,000  | 2+ minutes      | **Always use `--vacuum=never`** |

#### Critical Performance Recommendation

**For deletions of >10,000 tasks, always use `--vacuum=never`** to avoid multi-minute delays that block database access:

```bash
# CRITICAL: Use --vacuum=never for large deletions
abathur task prune --older-than 180d --vacuum=never --force
```

#### Manual VACUUM During Maintenance

If you skip VACUUM during deletion, you can manually reclaim space during off-hours:

```bash
# Run manual VACUUM during maintenance window
sqlite3 ~/.abathur/abathur.db "VACUUM;"
```

### Best Practices for Task Deletion

1. **Preview before deleting**:
   ```bash
   # Check what will be deleted
   abathur task prune --older-than 30d --dry-run
   ```

2. **Choose appropriate VACUUM mode**:
   - Small deletion (<100 tasks): Use default
   - Medium deletion (100-10,000 tasks): Use default
   - Large deletion (>10,000 tasks): Use `--vacuum=never`

3. **Schedule maintenance VACUUM**:
   ```bash
   # Add to cron for weekly maintenance
   0 2 * * 0 sqlite3 ~/.abathur/abathur.db "VACUUM;"
   ```

4. **Monitor database size**:
   ```bash
   # Check database size
   ls -lh ~/.abathur/abathur.db
   ```

### Troubleshooting VACUUM Issues

**Problem**: Deletion takes too long

**Solution**: Use `--vacuum=never` for large batch deletions:
```bash
abathur task prune --older-than 90d --vacuum=never --force
```

**Problem**: Database file size not shrinking

**Solution**: Run manual VACUUM:
```bash
sqlite3 ~/.abathur/abathur.db "VACUUM;"
```

**Problem**: Database locked during deletion

**Solution**: VACUUM holds exclusive lock. Wait for completion or use `--vacuum=never` next time.

**See Also:**
- Technical benchmark documentation: `tests/benchmarks/README.md`
- Troubleshooting guide: `docs/task_queue_troubleshooting.md`

## FAQ

**Q: How many dependencies can a task have?**
A: By default, a task can have up to 20 dependencies.

**Q: What happens if a task fails?**
A: Failed tasks can trigger cascade cancellation of dependent tasks, depending on configuration.

**Q: Can I change a task's priority?**
A: Yes, the system dynamically recalculates priority based on various factors.

**Q: How are human tasks prioritized?**
A: Human tasks (HUMAN source) receive a higher priority boost compared to agent-generated tasks.

**Q: When should I use `--vacuum=never`?**
A: Always use `--vacuum=never` when deleting more than 10,000 tasks to avoid multi-minute delays. See `src/abathur/infrastructure/database.py:32` for the VACUUM threshold.

**Q: How do I know if VACUUM ran?**
A: When VACUUM runs, the CLI displays: `VACUUM completed: X.XX MB reclaimed`. If VACUUM is skipped, you'll see: `VACUUM skipped (--vacuum=never)` or `VACUUM skipped (conditional mode, only N tasks deleted, threshold is 100)`.

**Q: Can I delete tasks while agents are running?**
A: Yes, but VACUUM (if run) will lock the database briefly. Use `--vacuum=never` to avoid blocking operations.

## Conclusion

The Abathur Task Queue System provides a flexible, intelligent platform for managing complex, multi-agent workflows with robust dependency management and dynamic prioritization. Use the task deletion and VACUUM controls to maintain optimal database performance as your task history grows.
