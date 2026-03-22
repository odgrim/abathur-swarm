# Task Lifecycle Contract

> `src/domain/models/task.rs`, `src/services/task_service.rs`,
> `src/adapters/sqlite/task_repository.rs`, `src/domain/ports/task_repository.rs`

## Task Model

A `Task` is the atomic unit of work in the swarm. Key fields:

| Field | Type | Description |
|-------|------|-------------|
| `id` | `Uuid` | Unique identifier |
| `parent_id` | `Option<Uuid>` | Parent task (subtask hierarchy) |
| `title` | `String` | Human-readable title |
| `description` | `String` | Detailed prompt/instructions |
| `agent_type` | `Option<String>` | Assigned agent type |
| `routing_hints` | `RoutingHints` | Routing metadata |
| `depends_on` | `Vec<Uuid>` | DAG dependency edges |
| `status` | `TaskStatus` | Current state |
| `priority` | `TaskPriority` | Low(1), Normal(2), High(3), Critical(4) |
| `retry_count` | `u32` | Failed attempts so far |
| `max_retries` | `u32` | Maximum retries (default: 3) |
| `version` | `u64` | Optimistic locking counter |
| `execution_mode` | `ExecutionMode` | `Direct` or `Convergent { parallel_samples }` |
| `task_type` | `TaskType` | `Standard`, `Verification`, `Research`, `Review` |
| `source` | `TaskSource` | `Human`, `System`, `SubtaskOf(Uuid)`, `Schedule(Uuid)`, `Adapter(String)` |
| `deadline` | `Option<DateTime<Utc>>` | SLA deadline |
| `idempotency_key` | `Option<String>` | Deduplication key |
| `trajectory_id` | `Option<Uuid>` | Convergent task trajectory reference |
| `loaded_version` | `VersionTag` | Interior-mutable version for optimistic locking |

## State Machine

```
                    ┌─────────────┐
                    │   Pending   │
                    └──┬──┬──┬────┘
                       │  │  │
            ┌──────────┘  │  └──────────┐
            ▼             ▼             ▼
        ┌───────┐    ┌─────────┐   ┌──────────┐
        │ Ready │    │ Blocked │   │ Canceled  │
        └──┬──┬─┘    └──┬──┬──┘   └───────────┘
           │  │         │  │        (terminal)
           │  └─────┐   │  └──────────►
           ▼        ▼   ▼
      ┌─────────┐  ┌─────────┐
      │ Running │  │         │
      └┬─┬──┬──┘  │         │
       │ │  │     │         │
       │ │  └─────┼─────────┼──────────►
       │ ▼        │         │
       │ ┌────────┴──┐      │
       │ │Validating │      │
       │ └┬──┬──┬────┘      │
       │  │  │  │           │
       │  │  │  └───────────┼──────────►
       ▼  ▼  │              │
  ┌──────────┐│         ┌───┘
  │ Complete ││         │
  └──────────┘│         │
   (terminal) ▼         │
          ┌────────┐    │
          │ Failed │────┘ (retry → Ready)
          └────────┘
           (terminal if retries exhausted)
```

### Valid Transitions

| From | To | Condition |
|------|----|-----------|
| Pending | Ready | All dependencies Complete |
| Pending | Blocked | Any dependency Failed/Canceled |
| Pending | Canceled | Manual cancellation |
| Ready | Running | Agent claims task |
| Ready | Blocked | Dependency fails after becoming ready |
| Ready | Canceled | Manual cancellation |
| Blocked | Ready | Blocking condition resolved |
| Blocked | Canceled | Manual cancellation |
| Running | Validating | Workflow enters verification phase |
| Running | Complete | Task succeeds |
| Running | Failed | Task fails |
| Running | Canceled | Manual cancellation |
| Validating | Running | Verification triggers rework |
| Validating | Complete | Verification passes |
| Validating | Failed | Verification fails fatally |
| Validating | Canceled | Manual cancellation |
| Failed | Ready | `retry_task()` when `retry_count < max_retries` |

### Terminal States

`Complete`, `Failed` (when retries exhausted), `Canceled` — no further
transitions allowed.

### Transition Side Effects

- `→ Running`: sets `started_at`
- `→ Complete/Failed/Canceled`: sets `completed_at`
- Every transition: increments `version`, sets `updated_at`

---

## TaskService Method Contracts

### `submit_task()`

**Signature:**
```rust
pub async fn submit_task(
    title, description, parent_id, priority, agent_type,
    depends_on, context, idempotency_key, source,
    deadline, task_type, execution_mode,
) -> DomainResult<(Task, Vec<UnifiedEvent>)>
```

**Preconditions:**
- If `parent_id` is set, parent task must exist
- If parent has active workflow phase subtasks, submission is **rejected** (workflow constraint)
- All `depends_on` task IDs must exist
- No dependency cycles

**Postconditions:**
1. Task persisted in repository
2. Status determined by dependency state:
   - No dependencies → `Ready`
   - All dependencies Complete → `Ready`
   - Any dependency Failed → `Blocked`
   - Otherwise → `Pending`
3. If `idempotency_key` matches existing task → returns existing (no duplicate)
4. Execution mode classified (heuristic or explicit)
5. Workflow auto-enrollment if applicable

**Events emitted:**
- `TaskSubmitted` (always)
- `WorkflowEnrolled` (if auto-enrolled)
- `TaskReady` (if immediately ready)

**Errors:**
- `TaskNotFound` — parent or dependency doesn't exist
- `ValidationFailed` — cycle detected, validation failed, or workflow constraint
- `DependencyCycle` — circular dependency

### `claim_task()`

**Preconditions:**
- Task exists
- Task status is `Ready`

**Postconditions:**
- Status transitions to `Running`
- `agent_type` is set (overwritten if previously set)
- `started_at` is set

**Events:** `TaskClaimed`

**Errors:**
- `TaskNotFound`
- `InvalidStateTransition` — not in `Ready` state

**Concurrency note:** Use `claim_task_atomic()` at the repository level
for race-free claiming (SQL `UPDATE ... WHERE status = 'ready'`).

### `complete_task()`

**Preconditions:**
- Task exists
- Task status is `Running` or `Validating`

**Postconditions:**
- Status transitions to `Complete`
- `completed_at` is set

**Events:** `TaskCompleted`, `TaskExecutionRecorded`

### `fail_task()`

**Preconditions:**
- Task exists
- Task status is `Running` or `Validating`

**Postconditions:**
- Status transitions to `Failed`
- Error message appended to context hints (FIFO, max 20)

**Events:** `TaskFailed`, `TaskExecutionRecorded`

**Downstream effect:** `TaskFailedBlockHandler` checks if retries
exhausted; if so, blocks all dependents.

### `retry_task()`

**Preconditions:**
- Task status is `Failed`
- `retry_count < max_retries`

**Postconditions:**
- `retry_count` incremented
- Status transitions to `Ready`
- For convergent tasks: `trajectory_id` preserved; if "trapped" hint
  present, adds `convergence:fresh_start` hint

**Events:** `TaskRetrying`

**Errors:** `ValidationFailed` — cannot retry (exhausted or wrong state)

### `transition_to_ready()`

**Preconditions:**
- Task not already `Ready` or terminal

**Postconditions:**
- Status transitions to `Ready`
- **Idempotent:** if already Ready/terminal, returns `Ok` with empty events

**Events:** `TaskReady`

**Used by:** `TaskCompletedReadinessHandler` to cascade readiness.

### `transition_to_blocked()`

**Preconditions:**
- Task not already `Blocked` or terminal

**Postconditions:**
- Status transitions to `Blocked`
- **Idempotent:** if already Blocked/terminal, returns `Ok` with empty events

**Events:** None

**Used by:** `TaskFailedBlockHandler` to cascade blocks.

### `cancel_task()`

**Preconditions:**
- Task not in terminal state

**Postconditions:**
- Status transitions to `Canceled` (terminal)

**Events:** `TaskCanceled`

**Errors:** `ValidationFailed` — already terminal

---

## Optimistic Locking

### How It Works

1. **Load:** Task fetched with current `version`. The value is stored in
   `loaded_version: VersionTag` (interior-mutable `AtomicU64`).

2. **Mutate:** Changes applied in-memory. `version` is incremented.

3. **Persist:** SQL `UPDATE tasks SET ... WHERE id = ? AND version = ?`
   using `loaded_version` in the WHERE clause.

4. **Outcome:**
   - **Success (rows_affected > 0):** `loaded_version` synced to new version.
   - **Failure (rows_affected == 0):**
     - If entity still exists → `ConcurrencyConflict { entity: "Task", id }`
     - If entity missing → `TaskNotFound`

### Handler Pattern for Conflict

Event handlers that update shared tasks (e.g., `TaskCompletedReadinessHandler`)
use this pattern:

```
match task_service.transition_to_ready(task_id).await {
    Ok(_) => { /* success */ }
    Err(DomainError::ConcurrencyConflict { .. }) => {
        // Another handler/thread won the race.
        // The transition is idempotent — safe to skip.
        log::debug!("Conflict on {task_id}, skipping");
    }
    Err(e) => return Err(e),  // Real error, propagate
}
```

**Invariant:** All handlers that mutate task state via `TaskService` must
handle `ConcurrencyConflict` gracefully. Ignoring this error leads to
silent state corruption or stuck tasks.

---

## Dependency Graph

### Storage
- Junction table: `task_dependencies(task_id, depends_on_id)`
- Cycle detection via recursive CTE on `add_dependency()`

### Readiness Cascade

When a task completes:
1. `TaskCompletedReadinessHandler` fires (System priority)
2. Fetches all dependents of the completed task
3. For each dependent in `Pending` or `Blocked`:
   - Checks if **all** its dependencies are now `Complete`
   - If yes → `transition_to_ready()`
4. Each `TaskReady` event may trigger agent assignment

When a task fails (retries exhausted) or is canceled:
1. `TaskFailedBlockHandler` fires (System priority)
2. Blocks all direct dependents via `transition_to_blocked()`
3. Blocked dependents' own dependents are **not** recursively blocked
   (they stay `Pending` until explicitly checked)

---

## Spawn Limits

Configuration: `SpawnLimitConfig`

| Limit | Default | Description |
|-------|---------|-------------|
| `max_subtask_depth` | 5 | Max depth from root task |
| `max_subtasks_per_task` | 10 | Max direct children per task |
| `max_total_descendants` | 100 | Max total descendants from root |
| `allow_limit_extensions` | true | Whether specialists can request extensions |

`check_spawn_limits(parent_id)` returns:
- `Allowed` — spawn permitted
- `LimitExceeded { limit_type, current, limit, can_request_extension }` — soft limit
- `HardLimit { limit_type, reason }` — cannot spawn

---

## Execution Mode Classification

When `execution_mode` is not explicitly set, `TaskService` uses a scoring
heuristic:

| Signal | Score |
|--------|-------|
| Execution agent type (coder, implementer, fixer) | +5 |
| Orchestration agent type (overmind, planner) | -5 |
| Complex complexity | +3 |
| Trivial/Simple complexity | -3 |
| Description contains acceptance criteria keywords | +2 |
| Context hints mention constraints/anti-patterns | +2 |
| Parent is convergent | +3 |
| Low priority | -2 |

**Threshold:** score >= 3 → `Convergent`; otherwise → `Direct`

**Overrides:**
- Explicit `execution_mode` in `submit_task()` → used as-is
- Operator `default_execution_mode` config → used as-is (kills heuristic)

---

## Workflow Auto-Enrollment

`infer_workflow_name()` logic:

| Condition | Workflow |
|-----------|----------|
| `TaskSource::Adapter(_)` | `"external"` (triage-first) |
| `task_type` is Verification or Review | None (not enrolled) |
| `context.custom` contains `workflow_phase` | None (already in workflow) |
| Explicit `routing_hints.workflow_name` | That value |
| Root task (no `parent_id`) | `"code"` (default) |
| Subtask with parent | None (not enrolled) |

Enrollment sets `workflow_state: WorkflowState::Pending` in task context
and emits `WorkflowEnrolled`.

---

## Idempotency

- Tasks with an `idempotency_key` are deduplicated at `submit_task()`.
- If a task with the same key exists, it is returned without creating a new one.
- Scheduled tasks use keys of the form `sched:{schedule_id}:{fire_count}`.

---

## Task Pruning

`prune_tasks(filter, force, dry_run)` deletes completed/failed tasks.

**Safety checks (unless `force`):**
- Skip tasks in active DAG (ancestors, descendants, dependencies still running)

**Dry run:** Returns what would be pruned without deleting.

---

## HTTP API

`src/adapters/mcp/tasks_http.rs`

| Endpoint | Method | Description |
|----------|--------|-------------|
| `/api/v1/tasks` | GET | List tasks (filters: status, task_type, limit) |
| `/api/v1/tasks` | POST | Submit new task |
| `/api/v1/tasks/{id}` | GET | Get task by ID |
| `/api/v1/tasks/ready` | GET | List ready tasks |
| `/api/v1/tasks/{id}/claim` | POST | Claim task (agent_type required) |
| `/api/v1/tasks/{id}/complete` | POST | Mark complete |
| `/api/v1/tasks/{id}/fail` | POST | Mark failed (optional error) |
| `/api/v1/tasks/{id}/retry` | POST | Retry failed task |
| `/api/v1/tasks/stats` | GET | Queue statistics |
