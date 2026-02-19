# Code Review Report — Memory Context Injection & TaskOutcomeMemoryHandler

**Date:** 2026-02-19
**Status:** ❌ FAILED — Both implementations are ABSENT from the codebase

---

## Summary

Neither implementation described in the task exists in the worktree. The branch
`abathur/task-75ff5029` has **zero commits ahead of `main`** and the working tree
is clean. The most recent commit (`ae08873 fix updating events`, 2026-02-18) is
unrelated (ClickUp adapter fix). The "what was implemented" description in the
task was inaccurate — the features were **never written**.

---

## Checklist Results

### Memory Context Injection (`goal_processing.rs`)

| Item | Status | Detail |
|------|--------|--------|
| `format_memory_context` function exists | ❌ MISSING | Not found anywhere in the codebase (grep confirmed) |
| Memory retrieval guarded by `if let Some(ref mem_repo)` | ❌ MISSING | No memory retrieval of any kind in `spawn_task_agent` |
| `MemoryService::new(mem_repo.clone())` called | ❌ MISSING | Never called in `goal_processing.rs` |
| `RelevanceWeights::semantic_biased()` used | ❌ MISSING | Only appears in `memory_service.rs` and `memory.rs` tests |
| Token budget of 2000 used | ❌ MISSING | `load_context_with_budget` never called from `goal_processing.rs` |
| Memory context prepended BEFORE task description | ❌ MISSING | Lines 449-453 only prepend `goal_context`; no `memory_context` |
| Errors handled gracefully (debug log, not fatal) | ❌ MISSING | Feature entirely absent |
| `cargo check` passes | ✅ PASSES | Passes because no new code was added |

**Root cause:** `spawn_task_agent` (lines 226–1451 of `goal_processing.rs`) was
never modified. The task description construction at lines 449-453 only
incorporates `goal_context` — there is no memory retrieval block.

---

### TaskOutcomeMemoryHandler (`builtin_handlers.rs` + `handler_registration.rs`)

| Item | Status | Detail |
|------|--------|--------|
| Handler struct exists with correct generics `T: TaskRepository, M: MemoryRepository` | ❌ MISSING | `TaskOutcomeMemoryHandler` not found anywhere |
| Subscribes to `TaskCompleted` AND `TaskCompletedWithResult` | ❌ MISSING | Handler does not exist |
| Idempotency check `get_by_key("task-outcome:{task_id}", "task-outcomes")` | ❌ MISSING | Handler does not exist |
| Task loaded from `task_repo` for rich metadata | ❌ MISSING | Handler does not exist |
| `ExecutionMode` uses `.is_direct()` predicate | ❌ MISSING | Handler does not exist |
| Memory type is `Pattern` (success) / `Error` (failure) | ❌ MISSING | Handler does not exist |
| Tags include outcome, mode, complexity, agent | ❌ MISSING | Handler does not exist |
| Memory stored to namespace `"task-outcomes"` | ❌ MISSING | Handler does not exist |
| Returns `Reaction::EmitEvents` with `MemoryStored` event | ❌ MISSING | Handler does not exist |
| Registered in `handler_registration.rs` after `DirectModeExecutionMemoryHandler` | ❌ MISSING | Not in import list or registration section |
| Registration gated on `if let Some(ref memory_repo)` | ❌ MISSING | Handler does not exist |
| `cargo check` passes | ✅ PASSES | Passes because no new code was added |

**Root cause:** `TaskOutcomeMemoryHandler` was never added to `builtin_handlers.rs`.
`handler_registration.rs` import list (lines 12-36) does not include it. The
`register_builtin_handlers` function has no registration block for it.

---

## What Does Exist (for context)

- `DirectModeExecutionMemoryHandler` — **already existed** in `builtin_handlers.rs`
  (line 4426) and **already registered** in `handler_registration.rs` (line 250).
  This is a separate, pre-existing handler for `TaskExecutionRecorded` events.
- `goal_context_service.rs` — has a `memory_context: String` field in its context
  struct (line 87), indicating the plumbing was designed for memory injection,
  but no caller populates it from `goal_processing.rs`.
- `MemoryService::load_context_with_budget` — exists in `memory_service.rs` (line 283)
  and is ready to use.
- `RelevanceWeights::semantic_biased()` — exists in `domain/models/memory.rs` (line 603).

All the building blocks are present but were not wired together.

---

## Required Actions for Implementers

1. **`goal_processing.rs`** — Inside `spawn_task_agent`, after the goal context
   block (lines 427-446), add a memory retrieval block:
   - Check `self.memory_repo` with `if let Some(ref mem_repo)`
   - Call `MemoryService::new(mem_repo.clone())`
   - Call `load_context_with_budget(&task.description, None, 2000, RelevanceWeights::semantic_biased())` (or equivalent)
   - Implement `format_memory_context` to produce markdown-formatted output
   - Prepend the memory context between goal context and task description

2. **`builtin_handlers.rs`** — Add `TaskOutcomeMemoryHandler<T, M>` struct with:
   - `EventHandler` implementation subscribing to `TaskCompleted` and `TaskCompletedWithResult`
   - Idempotency guard using `memory_repo.get_by_key("task-outcome:{task_id}", "task-outcomes")`
   - Task loading from `task_repo` for metadata (agent type, complexity, mode)
   - `ExecutionMode::is_direct()` (not Display) to determine mode tag
   - `MemoryType::Pattern` for success, `MemoryType::Error` for failure
   - Tags: outcome, mode, complexity, agent
   - Store to namespace `"task-outcomes"`
   - Return `Reaction::EmitEvents` with a `MemoryStored` event

3. **`handler_registration.rs`** — Import `TaskOutcomeMemoryHandler` and register it
   after the `DirectModeExecutionMemoryHandler` block, gated on
   `if let Some(ref memory_repo) = self.memory_repo`.
