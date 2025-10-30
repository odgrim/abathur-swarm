# How to Manage Tasks

This guide provides practical recipes for common task management operations in Abathur. Each recipe includes clear steps, working examples, and verification methods.

## Prerequisites

- Abathur CLI installed and initialized
- Basic familiarity with command-line interfaces
- Understanding of [task concepts](../explanation/task-queue.md)

## Overview

This guide covers:

- Submitting tasks with different priorities
- Setting up task dependencies
- Canceling and retrying tasks
- Monitoring queue status
- Handling task failures
- Filtering and searching tasks
- Batch operations on multiple tasks

---

## How to Submit a High-Priority Task

**Goal**: Create a task that executes before lower-priority tasks in the queue.

### Steps

1. Submit the task with elevated priority (0-10 scale, default is 5):

```bash
abathur task submit \
  --description "Fix critical production bug in authentication" \
  --agent-type "rust-error-types-specialist" \
  --priority 9
```

2. Verify the task was created:

```bash
abathur task list
```

**Expected Output**:
```
Tasks:
┌──────────┬────────┬─────────────────────────┬──────────┬──────────┐
│ ID       │ Status │ Summary                 │ Priority │ Agent    │
├──────────┼────────┼─────────────────────────┼──────────┼──────────┤
│ 550e8400 │ Pending│ Fix critical production │ 9        │ rust-... │
└──────────┴────────┴─────────────────────────┴──────────┴──────────┘

Showing 1 task(s)
```

### Alternative Approach: Using Summary

If you need a concise summary separate from the detailed description:

```bash
abathur task submit \
  --summary "Critical auth bug fix" \
  --description "Fix authentication bug causing login failures in production. Error occurs when users have special characters in passwords." \
  --agent-type "rust-error-types-specialist" \
  --priority 9
```

!!! tip "Priority Guidelines"
    - **0-3**: Low priority (documentation, refactoring)
    - **4-6**: Normal priority (features, enhancements)
    - **7-8**: High priority (important bugs, deadlines)
    - **9-10**: Critical (production issues, blockers)

---

## How to Create Task Dependencies

**Goal**: Ensure tasks execute in a specific order by defining dependencies.

### Steps

1. Submit the first task and note its ID:

```bash
abathur task submit \
  --description "Set up database schema for user authentication" \
  --agent-type "rust-sqlx-database-specialist"
```

**Output**:
```
Task submitted successfully!
  Task ID: 550e8400-e29b-41d4-a716-446655440000
  Summary: Set up database schema for user authentication
  ...
```

2. Submit the second task with dependency on the first:

```bash
abathur task submit \
  --description "Implement user registration service layer" \
  --agent-type "rust-service-layer-specialist" \
  --dependencies "550e8400"
```

!!! note "Short ID Prefixes"
    Abathur supports short UUID prefixes for convenience. You can use just the first 8 characters (or even fewer if unique) instead of the full UUID.

3. Submit a third task that depends on the second:

```bash
abathur task submit \
  --description "Add registration endpoint to API" \
  --agent-type "rust-http-api-client-specialist" \
  --dependencies "a1b2c3d4"
```

4. Verify the dependency chain:

```bash
abathur task show 550e8400
```

**Expected Output**:
```
Task Details:
  ID: 550e8400-e29b-41d4-a716-446655440000
  Status: Pending
  Summary: Set up database schema for user authentication
  ...
```

```bash
abathur task show a1b2c3d4
```

**Expected Output**:
```
Task Details:
  ID: a1b2c3d4-...
  Status: Blocked
  ...
  Dependencies:
    - 550e8400-e29b-41d4-a716-446655440000
```

### Alternative Approach: Multiple Dependencies

Submit a task that depends on multiple tasks completing first:

```bash
abathur task submit \
  --description "Deploy authentication system to staging" \
  --agent-type "github-actions-deployment-specialist" \
  --dependencies "550e8400,a1b2c3d4,f9e8d7c6"
```

!!! warning "Circular Dependencies"
    Avoid creating circular dependencies (Task A depends on B, B depends on A). The system will detect this and fail gracefully.

---

## How to Monitor Queue Status

**Goal**: Get an overview of all tasks and their current states.

### Steps

1. Check overall queue statistics:

```bash
abathur task status
```

**Expected Output**:
```
Queue Status:
┌─────────┬───────┐
│ Status  │ Count │
├─────────┼───────┤
│ Pending │ 3     │
│ Ready   │ 2     │
│ Running │ 1     │
│ Success │ 15    │
│ Failed  │ 0     │
└─────────┴───────┘
```

2. List all tasks:

```bash
abathur task list
```

3. Get JSON output for programmatic processing:

```bash
abathur --json task status
```

**Expected Output**:
```json
{
  "pending": 3,
  "ready": 2,
  "running": 1,
  "success": 15,
  "failed": 0,
  "blocked": 0,
  "cancelled": 0
}
```

### Alternative Approach: Watch Mode

Monitor the queue continuously using a shell loop:

```bash
watch -n 2 'abathur task status'
```

This updates the status every 2 seconds.

!!! tip "Performance Note"
    The `task status` command is optimized for fast execution and can be run frequently without performance impact.

---

## How to Filter Tasks by Status

**Goal**: View only tasks in a specific state (pending, running, failed, etc.).

### Steps

1. List all pending tasks:

```bash
abathur task list --status pending
```

2. List failed tasks to identify issues:

```bash
abathur task list --status failed
```

3. List completed tasks:

```bash
abathur task list --status success
```

4. Limit the number of results:

```bash
abathur task list --status success --limit 10
```

**Expected Output**:
```
Tasks:
┌──────────┬─────────┬──────────────────────┬──────────┐
│ ID       │ Status  │ Summary              │ Priority │
├──────────┼─────────┼──────────────────────┼──────────┤
│ 550e8400 │ Success │ Database schema setup│ 5        │
│ a1b2c3d4 │ Success │ User registration    │ 5        │
└──────────┴─────────┴──────────────────────┴──────────┘

Showing 2 task(s)
```

### Available Status Values

- `pending`: Submitted but waiting for dependencies
- `ready`: Dependencies met, waiting for execution
- `running`: Currently being executed by an agent
- `success`: Completed successfully
- `failed`: Execution failed with errors
- `blocked`: Dependencies not yet satisfied
- `cancelled`: Manually cancelled

---

## How to Cancel a Task

**Goal**: Stop a task from executing or cancel a running task.

### Steps

1. Find the task ID you want to cancel:

```bash
abathur task list --status pending
```

2. Cancel the task:

```bash
abathur task update 550e8400 --cancel
```

**Expected Output**:
```
Successfully updated 1 task(s):
  - 550e8400-e29b-41d4-a716-446655440000
```

3. Verify cancellation:

```bash
abathur task show 550e8400
```

**Expected Output**:
```
Task Details:
  ID: 550e8400-e29b-41d4-a716-446655440000
  Status: Cancelled
  ...
```

### Alternative Approach: Cancel Multiple Tasks

Cancel several tasks at once:

```bash
abathur task update 550e8400 a1b2c3d4 f9e8d7c6 --cancel
```

!!! warning "Cascade Effect"
    Canceling a task will also cascade to dependent tasks, updating them to blocked status if they cannot proceed.

---

## How to Retry a Failed Task

**Goal**: Re-execute a task that failed due to transient errors.

### Steps

1. List failed tasks:

```bash
abathur task list --status failed
```

2. Review the failure details:

```bash
abathur task show 550e8400
```

3. Retry the task:

```bash
abathur task update 550e8400 --retry
```

**Expected Output**:
```
Successfully updated 1 task(s):
  - 550e8400-e29b-41d4-a716-446655440000
```

4. Verify the task is back in pending state:

```bash
abathur task show 550e8400
```

**Expected Output**:
```
Task Details:
  ID: 550e8400-e29b-41d4-a716-446655440000
  Status: Pending
  ...
```

### When to Retry

**Retry if**:
- Network connectivity issues
- Temporary resource unavailability
- External service timeouts
- Race conditions

**Don't retry if**:
- Invalid configuration
- Code compilation errors
- Logic bugs in implementation
- Missing dependencies

!!! tip "Best Practice"
    Before retrying, investigate and fix the root cause when possible. Check logs, verify configuration, and ensure prerequisites are met.

---

## How to Modify Task Priority

**Goal**: Change the priority of a pending or blocked task to adjust execution order.

### Steps

1. Check current priority:

```bash
abathur task show 550e8400
```

**Output**:
```
Task Details:
  ID: 550e8400-e29b-41d4-a716-446655440000
  Status: Pending
  Priority: 5 (computed: 5.0)
  ...
```

2. Update the priority:

```bash
abathur task update 550e8400 --priority 8
```

3. Verify the change:

```bash
abathur task show 550e8400
```

**Expected Output**:
```
Task Details:
  ID: 550e8400-e29b-41d4-a716-446655440000
  Status: Pending
  Priority: 8 (computed: 8.0)
  ...
```

### Alternative Approach: Bulk Priority Update

Update priority for multiple related tasks:

```bash
abathur task update 550e8400 a1b2c3d4 f9e8d7c6 --priority 7
```

!!! note "Computed Priority"
    The system calculates a computed priority based on task age, dependencies, and other factors. Your base priority affects this calculation.

---

## How to Manage Task Dependencies After Creation

**Goal**: Add or remove dependencies from existing tasks.

### Add a Dependency

1. Identify the tasks:
   - Task to modify: `a1b2c3d4`
   - Dependency to add: `550e8400`

2. Add the dependency:

```bash
abathur task update a1b2c3d4 --add-dependency 550e8400
```

3. Verify:

```bash
abathur task show a1b2c3d4
```

**Expected Output**:
```
Task Details:
  ID: a1b2c3d4-...
  Status: Blocked
  Dependencies:
    - 550e8400-e29b-41d4-a716-446655440000
```

### Remove a Dependency

1. Remove the dependency:

```bash
abathur task update a1b2c3d4 --remove-dependency 550e8400
```

2. Verify:

```bash
abathur task show a1b2c3d4
```

**Expected Output**:
```
Task Details:
  ID: a1b2c3d4-...
  Status: Ready
  Dependencies: (none)
```

### Modify Multiple Dependencies

Add and remove dependencies in a single operation:

```bash
abathur task update a1b2c3d4 \
  --add-dependency 550e8400,f9e8d7c6 \
  --remove-dependency b2c3d4e5
```

---

## How to Resolve Blocked Tasks

**Goal**: Update tasks from blocked to ready state when their dependencies are satisfied.

### Steps

1. Check for blocked tasks:

```bash
abathur task list --status blocked
```

2. Verify dependency status:

```bash
abathur task show a1b2c3d4
```

**Output**:
```
Task Details:
  ID: a1b2c3d4-...
  Status: Blocked
  Dependencies:
    - 550e8400-e29b-41d4-a716-446655440000
```

3. Check if dependencies are complete:

```bash
abathur task show 550e8400
```

**Output**:
```
Task Details:
  ID: 550e8400-...
  Status: Success
  ...
```

4. Resolve dependencies:

```bash
abathur task resolve
```

**Expected Output**:
```
Task Dependency Resolution
=========================
Tasks updated to Ready: 3

Run 'abathur task list --status ready' to view ready tasks.
```

5. Verify tasks are now ready:

```bash
abathur task list --status ready
```

!!! info "Automatic Resolution"
    The Abathur daemon automatically runs dependency resolution periodically. Manual resolution is useful for immediate updates.

---

## How to Perform Batch Operations

**Goal**: Update multiple tasks efficiently in a single command.

### Batch Cancel

Cancel multiple tasks at once:

```bash
abathur task update 550e8400 a1b2c3d4 f9e8d7c6 --cancel
```

**Expected Output**:
```
Successfully updated 3 task(s):
  - 550e8400-e29b-41d4-a716-446655440000
  - a1b2c3d4-5678-41d4-a716-446655440001
  - f9e8d7c6-9012-41d4-a716-446655440002
```

### Batch Retry

Retry multiple failed tasks:

```bash
abathur task update 550e8400 a1b2c3d4 --retry
```

### Batch Priority Update

Change priority for a group of related tasks:

```bash
abathur task update 550e8400 a1b2c3d4 f9e8d7c6 --priority 8
```

### Batch Agent Type Change

Reassign tasks to a different agent:

```bash
abathur task update 550e8400 a1b2c3d4 --agent-type "rust-testing-specialist"
```

### Handling Batch Errors

If some tasks fail to update, the command reports both successes and failures:

```bash
abathur task update 550e8400 a1b2c3d4 invalid-id --priority 7
```

**Expected Output**:
```
Successfully updated 2 task(s):
  - 550e8400-e29b-41d4-a716-446655440000
  - a1b2c3d4-5678-41d4-a716-446655440001

Failed to update 1 task(s):
  - invalid-id: Failed to resolve task ID 'invalid-id'
```

!!! tip "JSON Output for Scripting"
    Use `--json` flag for batch operations in scripts:
    ```bash
    abathur --json task update 550e8400 a1b2c3d4 --priority 8
    ```

---

## How to Search for Specific Tasks

**Goal**: Find tasks matching specific criteria without listing all tasks.

### Search by Status with Limits

Get the 5 most recent failed tasks:

```bash
abathur task list --status failed --limit 5
```

### Search Using JSON and Tools

For advanced filtering, use JSON output with `jq`:

```bash
# Find all high-priority pending tasks
abathur --json task list --status pending | jq '.[] | select(.base_priority >= 7)'

# Find tasks by agent type
abathur --json task list | jq '.[] | select(.agent_type == "rust-testing-specialist")'

# Find tasks created in the last hour
abathur --json task list | jq --arg date "$(date -u -v-1H '+%Y-%m-%dT%H:%M:%S')" '.[] | select(.created_at > $date)'
```

### Get Detailed Information

View full details of a specific task:

```bash
abathur task show 550e8400
```

For JSON output:

```bash
abathur --json task show 550e8400
```

**Expected Output**:
```json
{
  "id": "550e8400-e29b-41d4-a716-446655440000",
  "summary": "Set up database schema",
  "description": "Set up database schema for user authentication",
  "agent_type": "rust-sqlx-database-specialist",
  "status": "Success",
  "base_priority": 5,
  "computed_priority": 5.0,
  "dependencies": [],
  "created_at": "2025-10-29T10:30:00Z",
  "updated_at": "2025-10-29T10:35:00Z",
  "started_at": "2025-10-29T10:31:00Z",
  "completed_at": "2025-10-29T10:35:00Z"
}
```

---

## Troubleshooting

### Problem: Task Stuck in Blocked Status

**Cause**: Dependencies haven't been resolved or dependency tasks failed.

**Solution**:
1. Check dependency status:
   ```bash
   abathur task show <task-id>
   ```
2. Review dependency task details:
   ```bash
   abathur task show <dependency-id>
   ```
3. If dependencies are complete, run resolution:
   ```bash
   abathur task resolve
   ```
4. If dependency failed, fix and retry it, or remove the dependency.

### Problem: Cannot Cancel a Running Task

**Cause**: Task is actively executing.

**Solution**: The cancel operation marks the task for cancellation, but the agent must respect the cancellation signal. Wait for the agent to complete its current operation.

### Problem: Task Priority Not Affecting Execution Order

**Cause**: Task dependencies override priority ordering.

**Solution**: Priority only affects ordering among tasks with satisfied dependencies. Check if higher-priority tasks are blocked by dependencies:
```bash
abathur task show <high-priority-task-id>
```

### Problem: Batch Update Partially Fails

**Cause**: Invalid task IDs or invalid state transitions.

**Solution**: Review the error output to identify which tasks failed and why. Fix the issues and retry the failed tasks individually:
```bash
abathur task update <failed-task-id> --<operation>
```

---

## Related Documentation

- [Tutorial: Your First Task](../tutorials/first-task.md) - Hands-on introduction to task management
- [Reference: CLI Commands](../reference/cli-commands.md) - Complete command syntax and options
- [Explanation: Task Queue Architecture](../explanation/task-queue.md) - Understanding how the task system works
- [How-To: Troubleshooting](troubleshooting.md) - Resolve common issues
