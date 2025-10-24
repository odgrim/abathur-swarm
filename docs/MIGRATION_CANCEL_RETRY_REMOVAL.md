# Migration Guide: Cancel and Retry Command Removal

**Version:** 0.2.0
**Date:** 2025-10-24
**Status:** Active

---

## Overview

This guide helps you migrate from the deprecated `cancel` and `retry` commands to the unified `update` command. The consolidation simplifies the API surface while providing greater flexibility for task status management.

### What Changed

The standalone `abathur task cancel` and `abathur task retry` commands have been removed in favor of the more flexible `abathur task update` command with a `--status` flag.

**Key Benefits:**
- **Unified Interface**: Single command for all task status updates
- **Greater Flexibility**: Update multiple attributes (status, priority, agent type) in one operation
- **Simplified Validation**: Consistent validation logic at the CLI layer
- **Better User Experience**: Predictable command structure with dry-run support

---

## Command Equivalence Table

| Old Command | New Command | Notes |
|------------|-------------|-------|
| `abathur task cancel TASK_ID` | `abathur task update TASK_ID --status cancelled` | Basic cancellation |
| `abathur task cancel TASK_ID --force` | `abathur task update TASK_ID --status cancelled` | Force flag removed - update is always permissive |
| `abathur task retry TASK_ID` | `abathur task update TASK_ID --status pending` | Reset failed task to pending for retry |
| `abathur task retry TASK_ID --max-retries 5` | *Not supported via CLI* | Use programmatic API if custom retry limits needed |

### Command Syntax

**Old Commands (Deprecated):**
```bash
# Cancel commands (removed)
abathur task cancel abc123
abathur task cancel abc123 --force

# Retry commands (removed)
abathur task retry abc123
```

**New Command (Current):**
```bash
# Cancel a task
abathur task update abc123 --status cancelled

# Retry a failed task
abathur task update abc123 --status pending

# Preview changes before applying
abathur task update abc123 --status cancelled --dry-run

# Update multiple attributes at once
abathur task update abc123 --status ready --priority 9
```

---

## Migration Steps

### 1. Update Shell Scripts

If you have shell scripts that use the old commands, update them to use the new syntax:

**Before:**
```bash
#!/bin/bash
# Cancel all pending tasks (old)
for task_id in $(abathur task list --status pending --format json | jq -r '.[].id'); do
  abathur task cancel "$task_id" --force
done
```

**After:**
```bash
#!/bin/bash
# Cancel all pending tasks (new)
for task_id in $(abathur task list --status pending --format json | jq -r '.[].id'); do
  abathur task update "$task_id" --status cancelled
done
```

### 2. Update Python Code

**Application Layer Changes:**

The application layer methods `cancel_task()` and `retry_task()` have been removed from `TaskCoordinator`. Use `update_task_status()` instead:

**Before:**
```python
from abathur.application import TaskCoordinator
from uuid import UUID

task_id = UUID("abc-123-def-456")

# Old method (removed)
success = await task_coordinator.cancel_task(task_id)

# Old method (removed)
success = await task_coordinator.retry_task(task_id)
```

**After:**
```python
from abathur.application import TaskCoordinator
from abathur.domain.models import TaskStatus
from uuid import UUID

task_id = UUID("abc-123-def-456")

# New unified method
await task_coordinator.update_task_status(
    task_id=task_id,
    new_status=TaskStatus.CANCELLED,
    error_message="Cancelled by user"  # Optional
)

# Retry by resetting to pending
await task_coordinator.update_task_status(
    task_id=task_id,
    new_status=TaskStatus.PENDING
)
```

**Service Layer (Preserved):**

The service layer `cancel_task()` method remains available for MCP server usage and cascading cancellations:

```python
from abathur.services.task_service import TaskService

# Service layer cancel_task() still available
# Used for cascading cancellations of child tasks
await task_service.cancel_task(task_id, cascade=True)
```

### 3. Test with Dry Run

Before applying changes in production, use the `--dry-run` flag to preview changes:

```bash
# Preview what will change
abathur task update abc123 --status cancelled --dry-run

# Preview multiple changes
abathur task update abc123 --status ready --priority 10 --dry-run
```

### 4. Verify Behavior

After migration, verify that your workflow still functions correctly:

```bash
# List tasks by status
abathur task list --status pending

# Update a task
abathur task update abc123 --status cancelled

# Verify the update
abathur task show abc123
```

---

## Frequently Asked Questions

### Q: Why were cancel and retry commands removed?

**A:** These commands were thin wrappers around the more flexible `update` command. Consolidating to a single `update` command simplifies the API, reduces code duplication, and provides more flexibility. Users can now update multiple task attributes (status, priority, agent type) in a single operation.

### Q: Is the `--force` flag still needed?

**A:** No, the `--force` flag has been removed. The `update` command is always permissive - it allows status transitions that may not be strictly valid according to the state machine. Validation logic has moved to the CLI layer where appropriate, providing clear error messages when operations cannot be performed.

### Q: What about programmatic API usage?

**A:** Use `coordinator.update_task_status()` instead of `coordinator.cancel_task()` or `coordinator.retry_task()` at the application layer. The service layer `cancel_task()` method (`services.task_service.cancel_task()`) is preserved for MCP server usage and handles cascading cancellations.

**API Layers:**
- **CLI Layer**: `abathur task update --status cancelled`
- **Application Layer**: `coordinator.update_task_status(task_id, TaskStatus.CANCELLED)` (REMOVED: `coordinator.cancel_task()`)
- **Service Layer**: `task_service.cancel_task(task_id, cascade=True)` (PRESERVED for MCP/cascading)

### Q: Are there any breaking changes?

**A:** Yes, the following are breaking changes:

1. **CLI Commands**: `abathur task cancel` and `abathur task retry` commands are removed
2. **Application Layer**: `TaskCoordinator.cancel_task()` and `TaskCoordinator.retry_task()` methods are removed
3. **Service Layer**: `TaskService.cancel_task()` method is **preserved** (no breaking change)

### Q: Can I still cancel running tasks?

**A:** Yes, you can cancel tasks in any status using the `update` command:

```bash
# Cancel a running task (no force flag needed)
abathur task update abc123 --status cancelled
```

The `update` command is permissive and allows status transitions even if the task is currently running.

### Q: How do I handle retry limits?

**A:** The CLI command resets a task to `pending` status but does not modify the `retry_count` or `max_retries` fields. If you need fine-grained control over retry behavior, use the programmatic API:

```python
# Reset retry counter (requires direct database access or service method)
await task_service.reset_retry_counter(task_id)

# Or update task status via coordinator
await task_coordinator.update_task_status(task_id, TaskStatus.PENDING)
```

### Q: What happens to child tasks when I cancel a parent?

**A:** Cascading cancellation is handled by the **service layer** `cancel_task()` method, which is still available:

```python
from abathur.services.task_service import TaskService

# Cancel parent and all children
await task_service.cancel_task(parent_task_id, cascade=True)
```

The CLI `update` command only affects the specified task. For cascading cancellations, use the service layer method directly or via MCP server tools.

### Q: Can I update multiple fields at once?

**A:** Yes! The `update` command allows updating status, priority, and agent type in a single operation:

```bash
# Update status and priority together
abathur task update abc123 --status ready --priority 9

# Update agent type and status
abathur task update abc123 --status pending --agent-type requirements-gatherer

# Preview complex update
abathur task update abc123 --status ready --priority 10 --agent-type meta-agent --dry-run
```

---

## Edge Cases and Special Scenarios

### Cascading Cancellations

**Scenario**: Cancel a parent task and all its children

**Solution**: Use the service layer `cancel_task()` method with `cascade=True`:

```python
from abathur.services.task_service import TaskService

async def cancel_with_children(task_id: UUID) -> None:
    """Cancel task and all descendants."""
    task_service = TaskService(database)
    await task_service.cancel_task(task_id, cascade=True)
```

The CLI `update` command only affects the specified task and does not cascade to children.

### Running Tasks

**Scenario**: Cancel a task that is currently executing

**Old Behavior**: Required `--force` flag
```bash
abathur task cancel abc123 --force  # Old
```

**New Behavior**: No force flag needed
```bash
abathur task update abc123 --status cancelled  # New
```

The `update` command is always permissive and allows cancelling running tasks.

### Completed Tasks

**Scenario**: Attempt to retry a completed task

**Behavior**: Completed tasks are in a terminal state and cannot transition to pending:

```bash
$ abathur task update abc123 --status pending
Error: Cannot transition from COMPLETED to PENDING
Completed tasks are in a terminal state
```

**Workaround**: Completed tasks cannot be retried. Create a new task instead.

### Failed Tasks with Multiple Retries

**Scenario**: A task has failed after multiple retry attempts

**Solution**: Reset to pending to retry manually:

```bash
# Check current retry count
abathur task show abc123 | grep retry_count

# Reset to pending (retry_count is not reset)
abathur task update abc123 --status pending

# Task will be picked up by orchestrator
# If retry_count >= max_retries, automatic retry may not occur
```

**Note**: The CLI command resets status but not `retry_count`. For a full reset, use the programmatic API or service layer.

### Blocked Tasks

**Scenario**: Unblock a task waiting for dependencies

**Solution**: Transition blocked task to ready:

```bash
# Check why task is blocked
abathur task show abc123

# Unblock and mark as ready
abathur task update abc123 --status ready

# Or keep as pending if dependencies still exist
abathur task update abc123 --status pending
```

### Dry Run for Safety

**Scenario**: Preview changes before applying in production

**Solution**: Use `--dry-run` flag for all updates:

```bash
# Preview cancellation
abathur task update abc123 --status cancelled --dry-run

# Preview status + priority change
abathur task update abc123 --status ready --priority 10 --dry-run
```

The dry run shows proposed changes without modifying the database.

---

## Code Reference Locations

The following code locations are relevant for understanding the implementation:

### CLI Layer
- Command Definition: `src/abathur/cli/main.py:563` (`update_task` command)
- Task ID Resolution: `src/abathur/cli/main.py:40` (`_resolve_task_id` helper)

### Application Layer
- Update Method: `src/abathur/application/task_coordinator.py:145` (`update_task_status`)
- Removed Methods: `cancel_task()` and `retry_task()` (deleted in Phase 1)

### Service Layer
- Cancel Service: `src/abathur/services/task_service.py` (`cancel_task` method - **preserved**)
- Cascading Logic: `src/abathur/services/task_service.py` (handles child task cancellation)

### Domain Layer
- Task Model: `src/abathur/domain/models.py` (`Task` and `TaskStatus`)
- Status Transitions: `src/abathur/domain/models.py` (`TaskStatus` enum)

---

## Migration Checklist

Use this checklist to ensure complete migration:

- [ ] **Search Codebase**: Find all usages of `cancel_task()` and `retry_task()`
  ```bash
  grep -r "cancel_task\|retry_task" --include="*.py" .
  ```

- [ ] **Update CLI Scripts**: Replace `abathur task cancel` with `abathur task update --status cancelled`

- [ ] **Update Python Code**: Replace application layer method calls with `update_task_status()`

- [ ] **Remove Force Flags**: Remove all `--force` flags from cancel commands

- [ ] **Test with Dry Run**: Verify changes with `--dry-run` before applying

- [ ] **Update Documentation**: Update internal documentation and runbooks

- [ ] **Verify Cascading**: Ensure cascading cancellations still work via service layer

- [ ] **Run Tests**: Execute test suite to verify no regressions
  ```bash
  pytest tests/unit/test_cli*.py
  pytest tests/integration/test_cli*.py
  ```

- [ ] **Update CI/CD**: Modify any CI/CD pipelines using old commands

- [ ] **Train Team**: Educate team members on new command syntax

---

## Additional Resources

### Documentation
- [API Reference](./API_REFERENCE.md) - Updated examples with `update_task_status()`
- [CLI Documentation](./CLI_USAGE.md) - Complete CLI command reference
- [Task Status Lifecycle](./TASK_STATUS_LIFECYCLE.md) - State machine and transitions

### Code Examples
- [Task Coordinator Usage](../examples/task_coordinator_example.py)
- [Batch Task Updates](../examples/batch_update_example.py)

### Support
- **GitHub Issues**: [Report migration issues](https://github.com/your-org/abathur/issues)
- **Discussion Forum**: [Ask questions](https://github.com/your-org/abathur/discussions)

---

## Timeline

| Date | Milestone |
|------|-----------|
| 2025-10-24 | Cancel/retry commands deprecated, `update` command introduced |
| 2025-11-01 | Migration guide published |
| 2025-11-15 | Deprecation warnings added to old commands |
| 2025-12-01 | Old commands removed from codebase |

---

**Last Updated**: 2025-10-24
**Document Version**: 1.0
**Maintained by**: Abathur Core Team
