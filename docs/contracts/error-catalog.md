# Error Catalog

> `src/domain/errors.rs`

## `DomainError` Variants

All domain operations return `DomainResult<T> = Result<T, DomainError>`.

### `GoalNotFound(Uuid)`

**When raised:**
- `GoalRepository::get()` returns `None` when the goal is expected to exist
- `GoalService` operations on a non-existent goal

**Caller action:** Check that the goal ID is valid. For event handlers
processing goal events, this may indicate a race where the goal was
deleted between event emission and handling — log and skip.

---

### `TaskNotFound(Uuid)`

**When raised:**
- `TaskRepository::get()` returns `None` when the task is expected to exist
- `TaskService` operations on a non-existent task
- `WorkflowEngine` operations on a task without workflow state
- Optimistic locking conflict resolution (entity gone)

**Caller action:** Check that the task ID is valid. In event handlers,
this can happen if the task was pruned or deleted between event emission
and handling — log and skip.

---

### `InvalidStateTransition { from, to, reason }`

**When raised:**
- `Task::transition_to()` with an invalid status change
- E.g., `Complete → Running`, `Canceled → Ready`

**Caller action:**
- For `claim_task()`: task is no longer `Ready` (another agent claimed it) — use `claim_task_atomic()` instead
- For `complete_task()` / `fail_task()`: task already in terminal state — idempotent skip
- For `cancel_task()`: task already terminal — idempotent skip

**Fields:**
- `from`: current status as string
- `to`: attempted target status
- `reason`: human-readable explanation

---

### `DependencyCycle(Uuid)`

**When raised:**
- `TaskRepository::add_dependency()` detects a cycle via recursive CTE
- `TaskService::submit_task()` during dependency validation

**Caller action:** Do not retry. The dependency graph is invalid. Remove
the offending dependency or restructure the DAG.

**Detection:** The SQLite repository uses a recursive CTE:
```sql
WITH RECURSIVE reachable AS (
    SELECT depends_on_id FROM task_dependencies WHERE task_id = ?1
    UNION ALL
    SELECT td.depends_on_id FROM task_dependencies td
    JOIN reachable r ON td.task_id = r.depends_on_id
)
SELECT 1 FROM reachable WHERE depends_on_id = ?2 LIMIT 1
```

---

### `AgentNotFound(String)`

**When raised:**
- `AgentRepository::get()` returns `None`
- Agent template lookup fails

**Caller action:** Verify agent type name. May indicate a template was
disabled or removed.

---

### `MemoryNotFound(Uuid)`

**When raised:**
- `MemoryRepository::get()` returns `None`
- Memory operations on a pruned or expired memory entry

**Caller action:** Memory may have been pruned by the maintenance daemon.
Gracefully handle absence.

---

### `ValidationFailed(String)`

**When raised:** Broad validation failures:

| Context | Message pattern | Meaning |
|---------|-----------------|---------|
| `TaskService::submit_task()` | "Workflow constraint" | Parent has active workflow phase subtasks |
| `TaskService::retry_task()` | "Cannot retry" | Task not Failed or retries exhausted |
| `WorkflowEngine::advance()` | "Cannot advance" | Wrong workflow state |
| `WorkflowEngine::advance()` | "subtask(s) are still running" | Concurrent advance guard |
| `WorkflowEngine::select_workflow()` | "must be in Pending state" | Spine change after first phase |
| `WorkflowEngine::get_template()` | "Unknown workflow template" | Template name not found |
| `Task::validate()` | Various | Field validation failures |

**Caller action:** Read the message string to determine the specific
failure. These are generally logic errors or precondition violations
that should not be retried without fixing the root cause.

---

### `DatabaseError(String)`

**When raised:**
- `sqlx::Error` converted via `From` impl
- SQL execution failures, connection errors, constraint violations

**Caller action:** Check for transient vs permanent failure. Connection
errors may be retried. Constraint violations indicate logic errors.

**Note:** UNIQUE constraint failures on the events table are handled
internally by the EventBus (sequence re-sync + retry). They do not
propagate as `DatabaseError`.

---

### `SerializationError(String)`

**When raised:**
- `serde_json::Error` converted via `From` impl
- JSON serialization/deserialization of task context, workflow state,
  routing hints, or event payloads

**Caller action:** Indicates data corruption or schema mismatch.
Check that stored JSON matches the expected struct. May occur after
code changes that alter struct shapes without migration.

---

### `ConcurrencyConflict { entity, id }`

**When raised:**
- `TaskRepository::update()` when `version` doesn't match
  (`WHERE id = ? AND version = ?` matches 0 rows, but entity exists)
- Any repository using optimistic locking

**Fields:**
- `entity`: Type name (e.g., `"Task"`)
- `id`: Entity ID as string

**Caller action — event handlers:**
```rust
match result {
    Err(DomainError::ConcurrencyConflict { .. }) => {
        // Another thread won the race. The operation is idempotent
        // (transition_to_ready/blocked), so it's safe to skip.
        Ok(())
    }
    other => other,
}
```

**Caller action — direct callers:**
- Reload the entity, check current state, and decide whether to retry
- Do NOT blindly retry in a loop — check if the desired state was
  already achieved by the winning thread

**Critical invariant:** All event handlers that call `TaskService`
mutation methods MUST handle `ConcurrencyConflict`. Failing to do so
causes handler errors, dead letters, and eventually circuit breaker trips.

---

### `ExecutionFailed(String)`

**When raised:**
- `Overmind` timeout (120s default)
- `Overmind` session failures
- `Overmind` response parsing errors
- `Substrate` execution failures
- `LlmPlanner` failures

**Caller action:** Check the message for timeout vs parse vs session errors.
Timeouts may be transient. Parse errors indicate LLM response format issues.

---

### `TaskScheduleNotFound(Uuid)`

**When raised:**
- `TaskScheduleRepository::get()` returns `None`
- `TaskScheduleService` operations on non-existent schedule

**Caller action:** Verify schedule ID. May have been completed (one-shot)
or deleted.

---

## Error Handling Patterns

### Pattern 1: Idempotent Skip

For terminal/already-done states:
```rust
// If task is already in the desired state, treat as success
match task_service.transition_to_ready(id).await {
    Ok(result) => Ok(result),
    Err(DomainError::InvalidStateTransition { .. }) => Ok(/* no-op */),
    Err(e) => Err(e),
}
```

### Pattern 2: Conflict Retry

For optimistic locking conflicts:
```rust
match task_service.complete_task(id).await {
    Ok(result) => Ok(result),
    Err(DomainError::ConcurrencyConflict { .. }) => {
        // Reload and check — another thread may have completed it
        let task = task_repo.get(id).await?;
        if task.map(|t| t.status == TaskStatus::Complete).unwrap_or(false) {
            Ok(/* already done */)
        } else {
            Err(/* real conflict, escalate */)
        }
    }
    Err(e) => Err(e),
}
```

### Pattern 3: Not-Found Guard

For event handlers processing potentially stale events:
```rust
match task_repo.get(event.task_id).await? {
    Some(task) => { /* process */ }
    None => {
        tracing::warn!("Task {} not found, skipping handler", event.task_id);
        return Ok(Reaction::None);
    }
}
```

### Pattern 4: Overmind Retry with Timeout

```rust
for attempt in 0..=retry_attempts {
    match tokio::time::timeout(decision_timeout, execute()).await {
        Ok(Ok(decision)) => return Ok(decision),
        Ok(Err(e)) if attempt < retry_attempts => {
            tokio::time::sleep(retry_cooldown).await;
            continue;
        }
        Ok(Err(e)) => return Err(DomainError::ExecutionFailed(e.to_string())),
        Err(_) => return Err(DomainError::ExecutionFailed("timeout".into())),
    }
}
```

## Error Propagation Rules

1. **Repository → Service:** All `DomainError` variants propagate upward.
   Services add context (e.g., wrapping `DatabaseError` with task ID).

2. **Service → Handler:** Handlers catch specific variants (see patterns above).
   Unhandled errors become `HandlerError` events + dead letters.

3. **Handler → Reactor:** The reactor logs errors, updates circuit breakers,
   and stores dead letters. It does NOT propagate to the event bus subscriber.

4. **Any → HTTP API:** Mapped to HTTP status codes:
   - `TaskNotFound` / `GoalNotFound` → 404
   - `ValidationFailed` / `InvalidStateTransition` → 400
   - `ConcurrencyConflict` → 409
   - `DatabaseError` / `ExecutionFailed` → 500
