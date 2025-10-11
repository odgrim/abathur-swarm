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

## FAQ

**Q: How many dependencies can a task have?**
A: By default, a task can have up to 20 dependencies.

**Q: What happens if a task fails?**
A: Failed tasks can trigger cascade cancellation of dependent tasks, depending on configuration.

**Q: Can I change a task's priority?**
A: Yes, the system dynamically recalculates priority based on various factors.

**Q: How are human tasks prioritized?**
A: Human tasks (HUMAN source) receive a higher priority boost compared to agent-generated tasks.

## Conclusion

The Abathur Task Queue System provides a flexible, intelligent platform for managing complex, multi-agent workflows with robust dependency management and dynamic prioritization.
