# Contract Shortcomings

Analysis of inter-functional contract violations and gaps across the
Abathur swarm service layer. Each shortcoming describes the contract
boundary that is violated, the concrete failure mode, code evidence,
and a suggested fix.

Severity levels:
- **Critical** — Can cause stuck tasks, data loss, or silent state corruption
- **High** — Can cause duplicate work, missed handler execution, or degraded reliability
- **Medium** — Suboptimal behavior that the reconciliation layer partially masks

---

## Dependency DAG Between Constructs

Understanding which construct depends on which contract is essential
for seeing where breaks propagate.

```
┌──────────────┐
│   Human /    │
│   Adapter    │
└──────┬───────┘
       │ submit
       ▼
┌──────────────┐    auto-enroll     ┌──────────────────┐
│ TaskService  │───────────────────►│  WorkflowEngine  │
│              │                    │  (state machine)  │
│ • create     │◄───────────────────│  • advance        │
│ • claim      │   direct repo      │  • fan_out        │
│ • complete   │   update (!)       │  • handle_phase   │
│ • fail       │                    │    _complete      │
│ • retry      │                    └────────┬─────────┘
└──────┬───────┘                             │
       │ events                     subtask  │ creates subtasks
       ▼                           complete  │ via TaskService
┌──────────────┐                             │
│  EventBus    │◄────────────────────────────┘
│  (broadcast) │         events
└──────┬───────┘
       │ dispatch
       ▼
┌──────────────────────────────────────────────────────┐
│                   EventReactor                        │
│                                                      │
│  SYSTEM priority:                                    │
│  ├─ TaskCompletedReadinessHandler  ──► cascade Ready │
│  ├─ TaskFailedBlockHandler         ──► cascade Block │
│  └─ AgentTerminationHandler        ──► kill agent    │
│                                                      │
│  HIGH priority:                                      │
│  ├─ WorkflowSubtaskCompletionHandler ──► advance wf  │
│  └─ ConvergenceCoordinationHandler   ──► converge    │
│                                                      │
│  NORMAL priority:                                    │
│  ├─ TaskFailedRetryHandler         ──► retry         │
│  ├─ GoalEvaluationHandler          ──► verify intent │
│  └─ ...40+ more handlers                            │
│                                                      │
│  LOW priority:                                       │
│  ├─ ReconciliationHandler          ──► fix drift     │
│  ├─ ReadyTaskPollingHandler        ──► assign agents │
│  └─ EventPruningHandler           ──► prune old     │
└──────────────────────────────────────────────────────┘
       │ dispatch tasks
       ▼
┌──────────────┐     measure      ┌──────────────────┐
│  Convergence │────────────────►│  OverseerCluster  │
│  Engine      │                  │  (compile, test,  │
│              │◄─────────────────│   lint, security) │
│  • iterate   │    signals       └──────────────────┘
│  • strategy  │
│  • resolve   │
└──────┬───────┘
       │ outcome
       ▼
┌──────────────┐
│  Overmind    │  (gate verdicts, decomposition, escalation)
└──────────────┘
```

Key edges where contracts break:

| Edge | Issue |
|------|-------|
| WorkflowEngine → TaskRepo (direct) | Bypasses TaskService contracts |
| TaskCompletedReadinessHandler → dependents | Not recursive past one level |
| EventBus → EventStore → EventPruningHandler | Pruning ignores watermarks |
| ConvergenceEngine → BudgetTracker | No integration exists |
| TaskService → Guardrails | No integration exists |
| TaskCompleted + TaskCompletedWithResult | Dual emission causes double handler invocation |

---

## S1. WorkflowEngine Bypasses TaskService for Task Mutations

**Severity: Critical**
**Contracts violated:** task-lifecycle.md (TaskService is the single authority for task state transitions), error-catalog.md (ConcurrencyConflict handling)

### Problem

The `WorkflowEngine` directly calls `task_repo.update()` in multiple
places instead of routing through `TaskService`:

1. **`write_state()`** (`workflow_engine.rs:100-112`) — loads task, mutates
   `context.custom["workflow_state"]`, calls `task_repo.update()` directly.

2. **`advance()` completion** (`workflow_engine.rs:265-270`) — transitions parent
   task to `Complete` via `parent.transition_to(TaskStatus::Complete)` then
   `task_repo.update(&parent)`, bypassing `TaskService::complete_task()`.

3. **`handle_phase_complete()` failure** (`workflow_engine.rs:425-429`) — same
   pattern for `Failed` transitions, bypassing `TaskService::fail_task()`.

### Failure Modes

- **Missing `TaskExecutionRecorded` events:** `TaskService::complete_task()` emits
  `TaskExecutionRecorded` for the ML learning loop. The workflow engine's direct
  completion skips this, so convergence heuristic learning never sees workflow
  task outcomes.

- **No `ConcurrencyConflict` handling in `write_state()`:** If TaskService and
  WorkflowEngine update the same task concurrently (e.g., TaskService completing
  a subtask while WorkflowEngine writes workflow state), one gets
  `ConcurrencyConflict`. `write_state()` propagates this as a bare error with
  no retry logic. The caller (usually an event handler) may dead-letter.

- **`loaded_version` not synced:** `write_state()` fetches a fresh task each
  time, so stale-version risk is low for single calls. But if two
  `write_state()` calls happen in quick succession (e.g., fan-out creating
  subtasks then immediately writing state), the second call uses a version
  that doesn't reflect the first write.

### Suggested Fix

**Option A (minimal):** Add retry-on-conflict to `write_state()`:
```rust
async fn write_state(&self, task_id: Uuid, state: &WorkflowState) -> DomainResult<()> {
    for _ in 0..3 {
        let mut task = self.task_repo.get(task_id).await?
            .ok_or(DomainError::TaskNotFound(task_id))?;
        let value = serde_json::to_value(state)?;
        task.context.custom.insert("workflow_state".to_string(), value);
        task.updated_at = chrono::Utc::now();
        match self.task_repo.update(&task).await {
            Ok(()) => return Ok(()),
            Err(DomainError::ConcurrencyConflict { .. }) => continue,
            Err(e) => return Err(e),
        }
    }
    Err(DomainError::ConcurrencyConflict { entity: "Task".into(), id: task_id.to_string() })
}
```

**Option B (thorough):** Route parent task status transitions through
TaskService methods (`complete_task()`, `fail_task()`) so that
`TaskExecutionRecorded`, version sync, and handler cascades work
correctly. Keep `write_state()` for workflow-state-only context updates
but add retry.

---

## S2. Blocking Cascade Is Not Recursive

**Severity: Critical**
**Contracts violated:** task-lifecycle.md (Dependency Graph / Readiness Cascade)

### Problem

`TaskFailedBlockHandler` (`builtin_handlers.rs:186-204`) blocks only
direct dependents of a failed task. It does not recurse to block
dependents of those dependents.

Given a chain: A → B → C → D

If A fails with retries exhausted:
- B is blocked (direct dependent) ✓
- C stays `Pending` forever (B's dependent, not checked) ✗
- D stays `Pending` forever ✗

### Failure Mode

Tasks C and D are zombies — stuck in `Pending` with a blocked ancestor.
They will never become `Ready` because B will never complete. The
`ReconciliationHandler` (LOW priority, periodic) does **not** check for
this pattern — it only detects tasks stuck in `Running` or `Validating`
past a timeout, and `FastReconciliationHandler` checks Pending→Ready
transitions but not whether ancestors are blocked.

These zombie tasks consume queue space and confuse status reporting.
They are only cleaned up by manual pruning or if someone notices and
cancels them.

### Suggested Fix

**Option A (recursive block):** After blocking each direct dependent,
recursively check *its* dependents:

```rust
async fn cascade_block(&self, root_task_id: Uuid, visited: &mut HashSet<Uuid>) {
    if !visited.insert(root_task_id) { return; }
    let dependents = self.task_repo.get_dependents(root_task_id).await?;
    for dep in dependents {
        if !dep.status.is_terminal() && dep.status != TaskStatus::Blocked {
            self.task_service.transition_to_blocked(dep.id).await?;
        }
        self.cascade_block(dep.id, visited).await;
    }
}
```

**Option B (reconciliation-based):** Add a check to
`FastReconciliationHandler` that finds all `Pending` tasks where any
transitive dependency is `Blocked` or terminal-failed, and transitions
them to `Blocked`. This is less timely but simpler.

**Option C (hybrid):** Block direct dependents immediately (current
behavior), but add Option B as a safety net that runs every
reconciliation cycle.

---

## S3. Dual Event Emission Causes Double Handler Invocation

**Severity: High**
**Contracts violated:** event-bus.md (handler idempotency assumptions), event-catalog.md (TaskCompleted semantics)

### Problem

When a task completes, both `TaskCompleted` and `TaskCompletedWithResult`
can be emitted for the same task. `TaskCompletedReadinessHandler`
(`builtin_handlers.rs:59-64`) matches **both** variants:

```rust
filter: EventFilter::new()
    .payload_types(vec![
        "TaskCompleted".to_string(),
        "TaskCompletedWithResult".to_string(),
    ]),
```

This means the readiness cascade executes twice for the same task
completion. The handler checks `if dep.status != Pending && dep.status
!= Blocked` before transitioning, which makes it *partially*
idempotent. But:

- The first invocation transitions dependents to Ready
- The second invocation finds them already Ready and skips
- Both invocations do redundant `get_dependents()` + status checks
- Between the two invocations, another handler could have changed
  dependent state, causing the second check to make a different decision

### Failure Mode

- Wasted work (redundant DB queries per completion)
- Potential for double `TaskReady` event emission if timing aligns
  between the two handler runs
- `WorkflowSubtaskCompletionHandler` has the same dual-match pattern,
  so workflow advancement is also attempted twice

### Suggested Fix

Either:
1. **Deduplicate at emission:** Only emit one of the two events per task
   completion. `TaskCompletedWithResult` is a superset — emit only it
   when result data is available, only `TaskCompleted` when it isn't.
2. **Deduplicate at handler:** Track "already processed" task IDs in
   a short-lived set within the handler (per-event-cycle dedup).

---

## S4. Priority Inversion: Readiness Cascade Before Workflow Advance

**Severity: High**
**Contracts violated:** workflow-engine.md (phase ordering), event-catalog.md (handler execution order semantics)

### Problem

Handler priorities for task completion:
- `TaskCompletedReadinessHandler` — **SYSTEM** (priority 0)
- `WorkflowSubtaskCompletionHandler` — **HIGH** (priority 100)

SYSTEM runs before HIGH. When a workflow subtask completes:

1. `TaskCompletedReadinessHandler` fires first, transitioning dependent
   tasks outside the workflow to `Ready`
2. `WorkflowSubtaskCompletionHandler` fires second, advancing the
   workflow to the next phase

### Failure Mode

If a non-workflow task depends on a workflow's subtask (cross-workflow
dependency), it can become `Ready` and be claimed by an agent **before**
the workflow has advanced. The workflow's sequential phase contract says
work should only proceed after the current phase completes and the gate
(if any) approves. But the readiness cascade doesn't know about
workflows — it just sees "dependency Complete → dependent Ready".

This is most dangerous with gate phases (triage/review): a dependent
task could start executing before the gate verdict is rendered.

### Suggested Fix

**Option A:** Move `WorkflowSubtaskCompletionHandler` to SYSTEM priority,
executing before `TaskCompletedReadinessHandler`. This ensures workflow
state is updated before readiness cascades.

**Option B:** Have `TaskCompletedReadinessHandler` check whether the
completed task is a workflow subtask and, if so, defer readiness cascade
until after `WorkflowCompleted` or `WorkflowAdvanced` is emitted.

**Option C:** In `ReadyTaskPollingHandler` (which assigns agents to Ready
tasks), add a check: if the task depends on a workflow-enrolled parent
that hasn't completed, don't dispatch it yet.

---

## S5. Event Pruning Ignores Handler Watermarks

**Severity: Critical**
**Contracts violated:** event-bus.md (watermark-based recovery), event-bus.md (EventStore contract)

### Problem

`EventStore::prune_older_than()` deletes events based solely on age.
It does not check whether any handler's watermark points to events
that would be pruned:

```rust
// event_store.rs — no watermark check
events.retain(|e| e.timestamp >= cutoff);
```

### Failure Mode

1. Handler X crashes with watermark at sequence 50
2. EventPruningHandler runs and prunes events older than 1 hour
3. Events 1-80 are pruned (they're old enough)
4. Handler X recovers, tries `replay_since(50)` — gets empty results
5. Events 51-80 are **permanently lost** for handler X
6. Any state transitions those events would have triggered never happen

This is especially dangerous for `TaskCompletedReadinessHandler` (SYSTEM
priority, critical=true). If its events are pruned before replay, tasks
stay stuck in `Pending` forever.

### Suggested Fix

Add a watermark floor to pruning:

```rust
async fn prune_older_than(&self, duration: Duration) -> Result<u64, EventStoreError> {
    let cutoff = Utc::now() - duration;
    let min_watermark = self.minimum_handler_watermark().await?;
    let safe_cutoff_seq = min_watermark.unwrap_or(SequenceNumber(0));

    events.retain(|e| e.timestamp >= cutoff || e.sequence >= safe_cutoff_seq);
    // ...
}
```

The `minimum_handler_watermark()` method already exists in the
EventReactor but is not called during pruning.

---

## S6. Dead Letter Retry Silently Drops Pruned Events

**Severity: High**
**Contracts violated:** event-bus.md (dead letter queue guarantees)

### Problem

When `DeadLetterRetryHandler` finds that the original event has been
pruned from the store, it silently resolves the DLQ entry
(`builtin_handlers.rs:3378-3444`):

```rust
None => {
    tracing::info!("event seq {} no longer in store, resolving DLQ entry", ...);
    self.event_store.resolve_dead_letter(&entry.id).await?;
}
```

### Failure Mode

The handler that originally failed on this event **never gets another
chance**. The failure is permanent and silent. If the original failure
was transient (network blip, temporary DB lock), the handler would have
succeeded on retry — but the event is gone.

Combined with S5 (pruning ignores watermarks), this creates a silent
data loss pipeline: event emitted → handler fails → event pruned →
DLQ retry finds nothing → entry resolved → state transition never
happens.

### Suggested Fix

1. **Emit an alert event** when resolving a DLQ entry due to missing
   original event:
   ```rust
   EventPayload::HandlerError {
       handler_name: entry.handler_name,
       event_sequence: entry.event_sequence,
       error: "Original event pruned before retry — permanent loss".into(),
       circuit_breaker_tripped: false,
   }
   ```

2. **Prevent the root cause** by implementing S5 (watermark-safe pruning).

3. **Consider using the outbox pattern** for critical handlers, so events
   are replayed from the transactional outbox rather than the event store.

---

## S7. Persist-Then-Publish Gap in TaskService

**Severity: High**
**Contracts violated:** event-bus.md (event delivery guarantees), task-lifecycle.md (postcondition: events emitted)

### Problem

`TaskService` methods persist task state changes, then *return* events
to the caller rather than publishing them directly:

```rust
// task_service.rs — complete_task()
self.task_repo.update(&task).await?;  // persist first
// ... build events ...
Ok((task, events))  // caller publishes
```

The `CommandBus` or orchestrator is responsible for publishing these
events after receiving them. If the process crashes between persist and
publish, or if the caller fails to publish:

- Task state is updated in the database
- No events are published
- No handlers fire (no readiness cascade, no workflow advance, no
  convergence coordination)

An `OutboxRepository` exists (`outbox_poller.rs`) but TaskService does
not use it.

### Failure Mode

After a crash-and-restart, the `StartupCatchUpHandler` attempts to fix
orphaned tasks by replaying missed events. But it can only replay events
that were *persisted to the EventStore*. If publish never happened, the
event was never persisted, so catch-up can't find it.

Result: tasks stuck in `Complete`/`Failed` with dependents stuck in
`Pending` because the readiness cascade never fired.

### Suggested Fix

**Option A (outbox pattern):** Write events to the outbox table in the
same transaction as the task state change. The outbox poller then
publishes them. This guarantees at-least-once delivery.

**Option B (reconciliation-based):** Rely on `FastReconciliationHandler`
to detect Pending tasks with all-Complete dependencies and transition
them. This is already partially implemented but runs on a timer (every
~15s), not immediately.

**Option C (publish-in-service):** Have TaskService publish events
directly to EventBus before returning. Simpler but doesn't solve the
crash-between-persist-and-publish gap.

---

## S8. No Global Budget Pressure Check in Convergence Loop

**Severity: Medium**
**Contracts violated:** service-dependencies.md (BudgetTracker contract), convergence-engine.md (budget management)

### Problem

The convergence engine tracks per-trajectory token budgets but has no
integration with the global `BudgetTracker`. There is no import, field,
or call to `BudgetTracker` anywhere in `convergence_engine.rs`.

### Failure Mode

When global budget pressure reaches Critical (>95% consumed), the
BudgetTracker reduces `effective_max_agents()` to 1 and signals
`should_pause_new_work()`. But an already-running convergence loop
ignores this entirely and continues iterating, potentially consuming
the remaining 5% of budget on a single task that may not even converge.

### Suggested Fix

Pass a `BudgetTracker` reference to the convergence engine and check
pressure at the top of each iteration:

```rust
// At top of iterate loop:
if let Some(tracker) = &self.budget_tracker {
    if tracker.should_pause_new_work() {
        return Ok(ConvergenceOutcome::BudgetDenied { trajectory_id });
    }
}
```

Alternatively, have the orchestrator cancel/pause convergent tasks when
budget pressure reaches Critical, using the existing
`ConvergenceSLAPressureHandler` pattern.

---

## S9. No Guardrails Check at Task Submission

**Severity: Medium**
**Contracts violated:** service-dependencies.md (Guardrails contract)

### Problem

`TaskService::submit_task()` does not consult `Guardrails` before
creating a task. There is no import, field, or call to Guardrails in
`task_service.rs`.

### Failure Mode

Tasks can be submitted beyond configured limits (`max_concurrent_tasks`,
`budget_limit_cents`, `max_tokens_per_hour`). They pile up in the Ready
queue. Guardrails are only checked at agent dispatch time (when the
orchestrator assigns agents), so:

- 1000 tasks can be submitted in a burst
- All become Ready immediately
- Guardrails blocks agent assignment at dispatch, but the tasks exist
  and consume DB/memory resources
- Stats show inflated task counts that don't reflect actual capacity

### Suggested Fix

**Option A:** Add Guardrails as a dependency of TaskService and check
`max_concurrent_tasks` before creating a task. Return
`ValidationFailed("Task limit exceeded")` if blocked.

**Option B (softer):** Leave TaskService unaware of Guardrails but have
the HTTP API layer (`tasks_http.rs`) check Guardrails before routing to
the CommandBus. This keeps TaskService pure but adds a pre-flight check
at the API boundary.

---

## S10. Incomplete Reconciliation Coverage

**Severity: Medium**
**Contracts violated:** task-lifecycle.md (state machine invariants)

### Problem

The reconciliation handlers have coverage gaps:

| Scenario | Detected? | Handler |
|----------|-----------|---------|
| Task stuck in Running > timeout | ✓ | ReconciliationHandler |
| Task stuck in Validating > timeout | ✓ | ReconciliationHandler |
| Pending task with all deps Complete | ✓ | FastReconciliationHandler |
| Blocked task with blocking dep now Complete | ✓ | FastReconciliationHandler |
| Workflow state parked too long | ✓ | ReconciliationHandler |
| **Running task with no active agent** | ✗ | None |
| **Pending task with transitively-blocked ancestor** | ✗ | None |
| **Workflow state inconsistent with TaskStatus** | ✗ | None |
| **Convergent task with orphaned trajectory** | ✗ | None |

### Failure Modes

- A task in `Running` whose agent subprocess was killed by OOM or
  signal (not through `AgentTerminationHandler`) stays Running forever
  until the stale-task timeout fires. No handler checks "is any agent
  actually working on this?"

- Workflow state says `Completed` but TaskStatus is still `Running`
  (can happen if `write_state` succeeds but the subsequent
  `task_repo.update()` for status change fails). No handler detects
  this inconsistency.

### Suggested Fix

Add targeted checks to `FastReconciliationHandler`:

1. **Orphaned Running tasks:** Query `Running` tasks whose agent
   subprocess is not in the active agent set. Fail them after a grace
   period.

2. **Zombie Pending tasks:** Query `Pending` tasks and walk their
   dependency chain. If any ancestor is `Blocked` or terminal-failed,
   transition to `Blocked`.

3. **Workflow/status mismatch:** Query tasks where `workflow_state` is
   `Completed` but `TaskStatus` is not `Complete`. Force-complete them.

---

## Summary

| # | Shortcoming | Severity | Root Cause |
|---|-------------|----------|------------|
| S1 | WorkflowEngine bypasses TaskService | Critical | Direct repo access for task mutations |
| S2 | Blocking cascade not recursive | Critical | Handler only processes one dependency level |
| S3 | Dual event emission → double handler run | High | Both TaskCompleted variants in same filter |
| S4 | Priority inversion: readiness before workflow | High | SYSTEM priority cascades before HIGH advances workflow |
| S5 | Event pruning ignores watermarks | Critical | No minimum-watermark check in prune logic |
| S6 | Dead letter retry silently drops pruned events | High | Silent resolve on missing event |
| S7 | Persist-then-publish gap | High | Events returned, not transactionally published |
| S8 | No global budget check in convergence loop | Medium | BudgetTracker not wired to convergence engine |
| S9 | No guardrails at task submission | Medium | Guardrails not wired to TaskService |
| S10 | Incomplete reconciliation coverage | Medium | Missing checks for orphaned/zombie states |

### Recommended Priority Order

1. **S5 + S6** (pruning + dead letter) — Fix together. Watermark-safe
   pruning eliminates the root cause of silent dead letter loss.
2. **S1** (WorkflowEngine bypass) — Add retry to `write_state()`, route
   status transitions through TaskService.
3. **S2** (recursive blocking) — Add transitive blocking to prevent
   zombie Pending tasks.
4. **S7** (persist-publish gap) — Adopt outbox pattern for TaskService.
5. **S4** (priority inversion) — Move workflow handler to SYSTEM priority.
6. **S3** (dual events) — Deduplicate at emission.
7. **S8 + S9 + S10** — Wire BudgetTracker/Guardrails, add reconciliation checks.
