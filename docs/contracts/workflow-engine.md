# Workflow Engine Contract

> `src/services/workflow_engine.rs`, `src/domain/models/workflow_template.rs`,
> `src/domain/models/workflow_state.rs`

## Overview

The workflow engine is a **deterministic state machine** — no LLM calls.
It drives phase ordering and tracks completion for tasks enrolled in a
workflow. The Overmind provides gate decisions and creates agents for each
phase.

Workflow state is stored as JSON in `task.context.custom["workflow_state"]`.

## Workflow Templates

### Builtin Templates

| Name | Phases | Workspace | Output |
|------|--------|-----------|--------|
| `code` | research → plan → implement → review | Worktree | PullRequest |
| `analysis` | research → analyze → synthesize | None | MemoryOnly |
| `docs` | research → write → review | Worktree | PullRequest |
| `review` | review | None | MemoryOnly |
| `external` | triage → research → plan → implement → review | Worktree | PullRequest |

### Phase Properties

| Property | Type | Description |
|----------|------|-------------|
| `name` | String | Phase identifier (e.g., "research", "implement") |
| `description` | String | What this phase does |
| `role` | String | Agent role description |
| `tools` | Vec<String> | Granted tools (from: read, write, edit, shell, glob, grep, memory, task_status, tasks, agents) |
| `read_only` | bool | If true, agent produces findings via memory, not code |
| `dependency` | PhaseDependency | `Root`, `Sequential`, or `AllPrevious` |
| `verify` | bool | Run intent verification after phase completes |

### Template-Level Properties

| Property | Default | Description |
|----------|---------|-------------|
| `workspace_kind` | `Worktree` | `Worktree`, `TempDir`, or `None` |
| `output_delivery` | `PullRequest` | `PullRequest`, `DirectMerge`, or `MemoryOnly` |
| `tool_grants` | `[]` | Template-wide tools merged into all phases |
| `max_verification_retries` | 2 | Max retries before escalating to gate |

### Gate Phases

Phases named `"triage"` or `"review"` are gate phases. After subtasks
complete, the engine parks at `PhaseGate` and requires an Overmind verdict
before proceeding.

## Workflow State Machine

```
┌─────────┐
│ Pending  │  (auto-enrolled, awaiting first advance)
└────┬─────┘
     │ advance()
     ▼
┌────────────┐
│ PhaseReady │  (overmind decides single vs fan-out)
└──┬─────┬───┘
   │     │
   │     │ fan_out()
   │     ▼
   │  ┌──────────────┐     ┌─────────────┐
   │  │ PhaseRunning  │     │ FanningOut   │
   │  │ (1 subtask)   │     │ (N subtasks) │
   │  └──────┬────────┘     └──────┬───────┘
   │         │                     │
   │         │ subtask(s) done     │ all slices done
   │         │                     ▼
   │         │              ┌──────────────┐
   │         │              │ Aggregating   │
   │         │              │ (merge task)  │
   │         │              └──────┬───────┘
   │         │                     │
   │         ▼                     ▼
   │  ┌──────────────────────────────────┐
   │  │ Decision point:                   │
   │  │  • All converged? → skip verify   │
   │  │  • verify:true? → Verifying       │
   │  │  • Gate phase? → PhaseGate        │
   │  │  • Otherwise → advance()          │
   │  └──┬───────┬──────────┬────────────┘
   │     │       │          │
   │     ▼       ▼          ▼
   │ ┌────────┐ ┌────────┐ ┌───────────┐
   │ │Verifying│ │PhaseGate│ │(next phase)│
   │ └──┬──┬──┘ └──┬─────┘ └───────────┘
   │    │  │       │
   │    │  │       │ verdict
   │    │  │       ▼
   │    │  │    Approved → advance()
   │    │  │    Rejected → Rejected (terminal)
   │    │  │
   │    │  │ verification fails (retries remain)
   │    │  └──► PhaseRetried → PhaseRunning (rework)
   │    │
   │    │ verification passes
   │    └──► advance() (next phase or Completed)
   │
   │ (all phases done)
   ▼
┌───────────┐
│ Completed  │  (parent task → Complete)
└───────────┘

┌───────────┐
│ Rejected   │  (gate verdict rejected; terminal)
└───────────┘

┌───────────┐
│ Failed     │  (phase subtask failed, retries exhausted; terminal)
└───────────┘
```

### State Variants

| State | Fields | Description |
|-------|--------|-------------|
| `Pending` | `workflow_name` | Enrolled, awaiting first `advance()` |
| `PhaseReady` | `workflow_name, phase_index, phase_name` | Ready for `fan_out()` |
| `PhaseRunning` | `workflow_name, phase_index, phase_name, subtask_ids` | Single subtask executing |
| `FanningOut` | `...subtask_ids, slice_count` | Parallel subtasks executing |
| `Aggregating` | `...subtask_ids` | Aggregation subtask running after fan-out |
| `Verifying` | `...subtask_ids, retry_count` | Intent verification in progress |
| `PhaseGate` | `workflow_name, phase_index, phase_name` | Awaiting Overmind verdict |
| `Completed` | `workflow_name` | All phases done (terminal) |
| `Rejected` | `workflow_name, phase_index, reason` | Gate rejected (terminal) |
| `Failed` | `workflow_name, error` | Unrecoverable failure (terminal) |

## Method Contracts

### `advance(task_id)`

**Preconditions:**
- Task exists and has workflow state
- State is `Pending`, `PhaseGate`, or an active-phase state with all subtasks terminal

**Postconditions:**
- If more phases remain: state → `PhaseReady`, events emitted (`WorkflowPhaseReady`, `WorkflowAdvanced`)
- If all phases done: state → `Completed`, parent task → `Complete`, `WorkflowCompleted` event

**Errors:**
- `TaskNotFound` — task doesn't exist
- `ValidationFailed` — wrong state, or subtasks still running (prevents double-advance)

**Concurrency guard:** Checks `all_subtasks_done()` before allowing advance
from active-phase states, preventing races between concurrent callers.

### `fan_out(task_id, slices)`

**Preconditions:**
- State is `PhaseReady`
- Phase template exists for current `phase_index`

**Postconditions:**
- Subtask(s) created as children of the workflow task
- Each subtask gets `workflow_phase` in context (prevents recursive workflow enrollment)
- Each subtask inherits worktree path from parent
- State → `PhaseRunning` (single subtask) or `FanningOut` (multiple slices)
- `WorkflowPhaseStarted` event emitted

**Subtask properties:**
- `parent_id` = workflow task ID
- `source` = `SubtaskOf(workflow_task_id)`
- `task_type` = `Standard` (or `Research` if read-only phase)
- `execution_mode` = inherited from parent or classified by heuristic
- `priority` = inherited from parent

### `handle_phase_complete(parent_task_id, subtask_id)`

**Preconditions:**
- Parent task has workflow state in an active-phase state
- `subtask_id` is in the current phase's `subtask_ids`

**Decision logic (after all subtasks done):**

1. **Any subtask failed?**
   - Try `retry_failed_phase_subtasks()` — resets failed subtask to Ready
   - If retries exhausted → state → `Failed`, parent task → `Failed`
   - Events: `WorkflowPhaseFailed`, `TaskFailed`

2. **FanningOut state?**
   - All fan-out subtasks done → `handle_fan_in()` creates aggregation subtask
   - State → `Aggregating`

3. **Phase has `verify: true` AND verification enabled AND not all subtasks converged?**
   - Parent task → `Validating` (TaskStatus)
   - State → `Verifying`
   - `WorkflowVerificationRequested` event emitted

4. **Gate phase (triage/review)?**
   - State → `PhaseGate`
   - `WorkflowGateReached` event emitted

5. **Otherwise:**
   - Auto-advance to next phase (calls `advance()`)

**Idempotency:** Ignores completions for subtask IDs not in the current phase.

### `handle_verification_result(task_id, satisfied)`

**Preconditions:**
- State is `Verifying`

**If satisfied:**
- If gate phase → state → `PhaseGate`
- Otherwise → auto-advance

**If not satisfied:**
- If `retry_count < max_verification_retries` → rework:
  - Increment retry count
  - Reset phase subtask to Ready with feedback in hints
  - State → back to `PhaseRunning`
  - Parent task → back to `Running`
  - `WorkflowPhaseRetried` event
- If retries exhausted → state → `PhaseGate` (escalate to Overmind)

### `handle_gate_verdict(task_id, verdict)`

**Preconditions:**
- State is `PhaseGate`

**Verdicts:**
- `GateVerdict::Approved` → auto-advance
- `GateVerdict::Rejected { reason }` → state → `Rejected`, parent task → `Canceled`
- `GateVerdict::Rework { feedback }` → back to `PhaseReady` with feedback

**Events:** `WorkflowGateVerdict`

### `select_workflow(task_id, workflow_name)`

**Preconditions:**
- Task has workflow state
- State is `Pending` (spine change only allowed before first phase)
- Target template exists

**Postconditions:**
- Workflow state updated with new name
- `routing_hints.workflow_name` updated

**Errors:** `ValidationFailed` if not in `Pending` state

### `get_state(task_id) -> WorkflowStatus`

**Pure read.** Returns current workflow status including phase index,
name, verification state, and retry count.

## Verification Flow Detail

When a phase has `verify: true`:

```
Phase subtasks complete
    │
    ▼
all_subtasks_converged()?
    │
    ├── yes → skip verification, proceed to gate/advance
    │
    └── no → parent task → Validating
             state → Verifying { retry_count: 0 }
             emit WorkflowVerificationRequested
                 │
                 ▼
          WorkflowVerificationHandler runs LLM verification
                 │
                 ├── satisfied → handle_verification_result(true)
                 │                   → gate or advance
                 │
                 └── not satisfied
                     │
                     ├── retry_count < max → rework subtask
                     │   state → PhaseRunning
                     │   parent → Running
                     │   emit WorkflowPhaseRetried
                     │
                     └── retries exhausted → PhaseGate (escalate)
```

**Key invariant:** If all subtasks used convergent execution mode and
converged successfully, verification is skipped. Convergence already
validates via overseers — double-checking is wasteful.

## Fan-Out / Aggregation Flow

```
PhaseReady
    │
    ▼ fan_out(slices=[slice_a, slice_b, slice_c])
    │
FanningOut { subtask_ids: [a, b, c], slice_count: 3 }
    │
    │ all subtasks complete
    ▼
handle_fan_in()
    │ creates aggregation subtask that merges results
    ▼
Aggregating { subtask_ids: [a, b, c, agg] }
    │
    │ aggregation subtask completes
    ▼
(verification / gate / advance)
```

## Event Summary

| Event | When |
|-------|------|
| `WorkflowEnrolled` | Task auto-enrolled in workflow (submit_task) |
| `WorkflowPhaseReady` | Phase ready for fan_out |
| `WorkflowPhaseStarted` | Subtask(s) created for phase |
| `WorkflowAdvanced` | Moved from one phase to next |
| `WorkflowGateReached` | Gate phase awaiting verdict |
| `WorkflowGateVerdict` | Overmind provided gate verdict |
| `WorkflowVerificationRequested` | Verification started |
| `WorkflowVerificationCompleted` | Verification result |
| `WorkflowPhaseRetried` | Phase rework after verification failure |
| `WorkflowPhaseFailed` | Phase failed, retries exhausted |
| `WorkflowCompleted` | All phases done |

## Failure Modes

| Failure | Handling |
|---------|----------|
| Phase subtask fails | Retry subtask (reset to Ready); if exhausted, fail workflow |
| Verification fails | Rework phase subtask; if retries exhausted, escalate to gate |
| Gate rejects | Workflow → `Rejected`, parent task → `Canceled` |
| Fan-out subtask fails | Same as phase subtask failure |
| `ConcurrencyConflict` on state write | Caller retries or handler skips (idempotent) |
| Unknown workflow template | `ValidationFailed` at enrollment or advance |
