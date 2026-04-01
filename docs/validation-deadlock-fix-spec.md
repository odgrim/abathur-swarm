# Validation Deadlock Fix Spec

## Problem Statement

Task `4da0bf68` (Goal Convergence Check, overmind agent) got stuck in `TaskStatus::Validating` with `WorkflowState::PhaseReady` — an illegal combination. The overmind exhausted 51 turns while the workflow was in PhaseReady, and auto-completion set TaskStatus to Validating, creating a state machine deadlock. No subtasks could finalize because the parent wasn't terminal, and the parent couldn't advance because no agent was driving `workflow_fan_out`.

## Fix 1: State Consistency Invariant (TaskStatus ↔ WorkflowState)

**File:** `src/services/workflow_engine.rs`

- [ ] Add `validate_state_consistency(task_status: TaskStatus, workflow_state: &WorkflowState) -> Result<()>` enforcing:
  - `Validating` is only valid with `WorkflowState::Verifying`
  - `Running` is only valid with `PhaseRunning | FanningOut | Aggregating | PhaseReady | PhaseGate`
- [ ] Call as post-condition in: `write_state()` (line ~109), `advance()` (line ~185), `fan_out()` (line ~1020), `handle_phase_complete()` (line ~338), `handle_verification_result()` (line ~652)
- [ ] In `advance()` (line ~199): when detecting Validating+PhaseReady, auto-correct TaskStatus → Running before returning error (transition is valid per task.rs:130)

## Fix 2: Workflow-Aware Turn Exhaustion

**File:** `src/services/swarm_orchestrator/goal_processing.rs`

- [ ] At line ~973 (auto-complete on max_turns): before setting `target_status = TaskStatus::Validating`, check if task has non-terminal WorkflowState. If so, FAIL instead.
- [ ] At line ~1779 (auto-complete fallback): same guard as above.
- [ ] Split `is_max_turns_auto_completable` (line ~2073) into two concerns: (a) was it a max-turns completion? (b) is auto-complete safe given workflow context? Add `can_safely_auto_complete(task) -> bool` that returns false for non-terminal WorkflowState.

**File:** `src/services/swarm_orchestrator/infrastructure.rs`

- [ ] In `run_startup_reconciliation()` (line ~894): add step after stale-Running check (lines 898-940) to find Validating tasks with inconsistent WorkflowState:
  - PhaseReady → transition TaskStatus to Running
  - Terminal workflow state → match TaskStatus to workflow terminal state
  - Log audit warning

## Fix 3: Break Validation Cascade Deadlock in try_auto_ship

**File:** `src/services/swarm_orchestrator/helpers.rs`

- [ ] In `try_auto_ship()` (line ~1115), before the terminal check at line ~1147, add recovery:
  - Root is Validating + WorkflowState::Completed → force-complete root, proceed
  - Root is Validating + WorkflowState::Failed/Rejected → force-fail root, proceed
  - Root is Validating + WorkflowState::PhaseReady → fail root with "Workflow deadlock: Validating+PhaseReady", proceed
  - Root is Validating + WorkflowState::Verifying → return None (verification genuinely in progress)
- [ ] At line ~1157-1165 (all-descendants-terminal check): if any descendant is Validating with inconsistent WorkflowState, force-fail it before checking.

**File:** `src/services/builtin_handlers.rs`

- [ ] In `WorkflowSubtaskCompletionHandler::handle()` (line ~5885): before calling `engine.handle_phase_complete` (line ~5976), load parent and check state consistency. If parent is Validating + WorkflowState is NOT Verifying → transition parent to Running first.

## Fix 4: CLI Force-Transition and Unstick Commands

**File:** `src/cli/commands/task.rs`

- [ ] Add `TaskCommands::ForceTransition { id, status, reason }` subcommand
- [ ] Add `TaskCommands::Unstick { id, strategy }` convenience subcommand (strategy: fail/complete/retry)
- [ ] Unstick logic: inspect WorkflowState to determine correct action:
  - Validating+PhaseReady → fail
  - Validating+Verifying → re-emit WorkflowVerificationRequested
  - strategy=complete → force-complete + set WorkflowState::Completed
  - Always log audit entry

**File:** `src/services/task_service.rs`

- [ ] Add `pub async fn force_transition(&self, task_id: Uuid, new_status: TaskStatus, reason: &str) -> DomainResult<(Task, Vec<UnifiedEvent>)>`
- [ ] Bypasses `valid_transitions()` checks
- [ ] Updates both TaskStatus AND WorkflowState consistently (e.g. Complete → WorkflowState::Completed, Failed → WorkflowState::Failed)
- [ ] Emits appropriate events
- [ ] Logs warning-level audit entry

**File:** `src/services/command_bus.rs`

- [ ] Add `ForceTransition { task_id, new_status, reason }` variant to `TaskCommand`
- [ ] Handle it by calling `task_service.force_transition()`

## Fix 5: Wire Up and Complete VALIDATING Timeout Watchdog

**Existing:** `ReconciliationHandler` in `builtin_handlers.rs` (lines ~1671-1750) has 3-tier stale-Validating detection (50%/80%/100% timeout thresholds).

**File:** `src/services/config.rs`

- [ ] Verify `stale_validating_timeout_secs` is defined in `LimitsConfig` (line ~98). If missing, add with default 1800.

**File:** `src/services/builtin_handlers.rs`

- [ ] Make existing ReconciliationHandler workflow-aware when timing out Validating tasks:
  - If WorkflowState::Verifying → re-emit WorkflowVerificationRequested to retry
  - If inconsistent state → fail with state-inconsistency error
  - If no workflow state (standalone) → fail as existing logic does
- [ ] Use `force_transition` (from Fix 4) for state changes to keep TaskStatus+WorkflowState consistent

**File:** `src/services/swarm_orchestrator/infrastructure.rs`

- [ ] Extend `run_startup_reconciliation()` to include stale Validating tasks (currently only handles Running at lines 898-940). Apply same timeout logic on startup.

## Fix 6: Complete Graceful Git Remote Absence Handling

**Existing:** `sync_with_remote()` (helpers.rs:60-82) already handles missing remotes gracefully.

**File:** `src/services/swarm_orchestrator/helpers.rs`

- [ ] In `try_push_with_rebase()` (line ~86): add early check — run `git remote get-url origin`, return false immediately if no remote (skip retry loop).

**File:** `src/services/worktree_service.rs`

- [ ] Add `check_remote_available(repo_path: &Path) -> bool` helper.

**File:** `src/services/swarm_orchestrator/infrastructure.rs`

- [ ] On startup: check if `origin` remote exists, log prominent WARN if missing, cache `remote_available` flag.
- [ ] When `remote_available=false`, auto-degrade to `fetch_on_sync=false` behavior.

**File:** `src/services/swarm_orchestrator/helpers.rs`

- [ ] In try_auto_ship ship path: check `remote_available` before attempting push/PR creation.

## Fix 7: Guard All 5 Validating-Setting Paths

There are 5 code paths that set `TaskStatus::Validating`:

1. `goal_processing.rs:973` — auto-complete on max_turns (covered by Fix 2)
2. `goal_processing.rs:1584` — explicit transition in direct execution
3. `goal_processing.rs:1779` — auto-complete fallback (covered by Fix 2)
4. `task_service.rs:1081` — `transition_to_validating()` method (central chokepoint)
5. `builtin_handlers.rs:4885` — TaskValidatingHandler on verification start

**File:** `src/services/task_service.rs`

- [ ] In `transition_to_validating()` (line ~1081): add pre-condition — if task has WorkflowState of `PhaseReady` or `PhaseGate`, refuse transition and return error. NOTE: we specifically only block these states (not all non-Verifying states) because the normal workflow flow calls `transition_to_validating()` BEFORE updating WorkflowState to Verifying (i.e., while still in Aggregating/PhaseRunning). This is the single chokepoint preventing deadlock-causing collisions.

**File:** `src/services/swarm_orchestrator/goal_processing.rs`

- [ ] At line ~1584 (direct execution path): verify this is only used for non-workflow tasks. If it can be called for workflow parents, add workflow-state guard.

**File:** `src/services/builtin_handlers.rs`

- [ ] At line ~4885 (TaskValidatingHandler): verify WorkflowState::Verifying is always set BEFORE TaskStatus::Validating. Check the event flow to ensure ordering.

## Fix 8: Comprehensive Tests

- [ ] State consistency invariant: Validating+PhaseReady → auto-corrects to Running+PhaseReady
- [ ] Turn exhaustion with workflow: overmind exhausts turns in PhaseReady → task Failed (not Validating)
- [ ] try_auto_ship deadlock recovery: parent Validating+PhaseReady with completed subtasks → parent force-failed
- [ ] WorkflowSubtaskCompletionHandler with inconsistent parent: parent Validating+PhaseReady, subtask completes → parent transitions to Running
- [ ] Validation timeout: Validating task past timeout → ReconciliationHandler fails it
- [ ] Force-transition CLI: stuck task + force-transition → state change + audit log
- [ ] No-remote graceful degradation: try_push_with_rebase with no origin → early return without retries
- [ ] Startup reconciliation with stale Validating: Validating+PhaseReady on restart → reconciliation fixes state

## Fix 9: Final Review

- [ ] All checklist items in this spec are complete
- [ ] No regressions in existing tests
- [ ] `cargo build` succeeds
- [ ] `cargo test` passes
