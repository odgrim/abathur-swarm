# Task Queue MCP Migration Guide

## Overview

The Abathur platform has transitioned from using TodoWrite to a more robust Task Queue MCP (Master Control Program) system. This guide helps you migrate your existing workflows to the new task management approach.

## Key Changes

### 1. Task Enqueuing
- **Old Approach:** Using `TodoWrite` for simple task tracking
- **New Approach:** Using `task_enqueue` MCP tool for comprehensive task management

### 2. Task Lifecycle Management
- Explicit task dependencies
- Priority setting
- Agent type specification
- Detailed task tracking and status management

## Migration Steps

### Task Creation

**Old TodoWrite Approach:**
```python
# Before
todo_write({
    "task": "Process user requirements",
    "status": "pending"
})
```

**New task_enqueue Approach:**
```python
# After
task_enqueue({
    "description": "Process user requirements",
    "source": "requirements-gatherer",
    "priority": 5,  # 1-10 scale
    "agent_type": "requirements-specialist",
    "prerequisite_task_ids": [],  # Optional task dependencies
    "deadline": "2025-12-31T23:59:59Z"  # Optional deadline
})
```

### Task Dependencies

```python
# Example of tasks with dependencies
task_enqueue({
    "description": "Design System Architecture",
    "source": "technical-lead",
    "prerequisite_task_ids": ["requirements-gathering-task-uuid"]
})
```

## MCP Tools for Task Management

1. `task_enqueue`: Create and queue tasks
2. `task_list`: List and filter tasks
3. `task_get`: Retrieve specific task details
4. `task_queue_status`: Get overall queue statistics
5. `task_cancel`: Cancel tasks with optional cascade
6. `task_execution_plan`: Visualize task dependencies

## Best Practices

- Always specify a `source` for traceability
- Use meaningful task descriptions
- Leverage `prerequisite_task_ids` for complex workflows
- Set appropriate priorities (1-10 scale)
- Utilize agent_type for targeted task routing

## Example Workflow

```python
# Requirements Gathering Phase
requirements_task = task_enqueue({
    "description": "Gather Initial User Requirements",
    "source": "workflow-orchestrator",
    "priority": 8,
    "agent_type": "requirements-specialist"
})

# Technical Specification Phase
spec_task = task_enqueue({
    "description": "Create Technical Specification",
    "source": "workflow-orchestrator",
    "priority": 7,
    "agent_type": "technical-architect",
    "prerequisite_task_ids": [requirements_task['task_id']]
})
```

## Migration Checklist

- [ ] Replace all `TodoWrite` calls with `task_enqueue`
- [ ] Add `source` parameter to tasks
- [ ] Set priorities for tasks
- [ ] Define task dependencies using `prerequisite_task_ids`
- [ ] Update agent templates to use new MCP tools

## Troubleshooting

- Check task dependencies carefully to prevent circular references
- Use `task_list` and `task_get` to debug task status
- Ensure source and agent types are correctly specified

## Benefits of New Approach

- Explicit task dependencies
- Better visibility into task lifecycle
- Improved tracking and reporting
- More flexible task management
- Support for complex, multi-agent workflows
