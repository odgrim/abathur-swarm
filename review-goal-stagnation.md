# Code Review: Goal Stagnation Detection

**Reviewed by**: code-reviewer agent
**Date**: 2026-02-19
**Branch**: abathur/task-e16156a5
**Scope**: Migration 006, Goal struct, GoalRepository, GoalConvergenceCheckHandler,
GoalStagnationDetectorHandler, and handler_registration.

---

## Compilation & Test Results

| Command | Result |
|---|---|
| `cargo check` | ✅ PASS — zero errors, zero warnings |
| `cargo test -- goal` | ✅ PASS — all goal-related tests pass |
| `cargo test -- stagnation` | ⚠️ 0 tests matched (see note #2 below) |
| `cargo test -- update_last_check` | ⚠️ 0 matched (tests live in goal_repository module, see item 10) |
| `cargo test` (all tests) | ✅ PASS — 1234 total, 0 failed |

---

## Checklist Results

### 1. ✅ Compilation
`cargo check` passes with zero errors.

### 2. ✅ Migration correctness
`migrations/006_goal_convergence_check_at.sql` contains:
```sql
ALTER TABLE goals ADD COLUMN last_convergence_check_at TEXT;
```
Correct idiomatic SQLite for adding a nullable column.
`all_embedded_migrations()` in `migrations.rs` registers it as version 6 with correct
`include_str!` path. Applied in order after migration 5.

### 3. ✅ Goal struct field
`Goal::new()` initialises `last_convergence_check_at: None`. ✓
Field declared as `pub last_convergence_check_at: Option<DateTime<Utc>>` with doc comment. ✓

### 4. ✅ All 5 SELECT queries updated
Every SELECT in `SqliteGoalRepository` includes `last_convergence_check_at`:
- `get()` (line 54) ✓
- `list()` (line 109) ✓
- `get_children()` (line 141) ✓
- `get_active_with_constraints()` (line 152) ✓
- `find_by_domains()` (line 163) ✓

### 5. ✅ TryFrom mapping
`parse_optional_datetime(row.last_convergence_check_at)` called at line 250. ✓
Result assigned as `last_convergence_check_at` in the `Goal { ... }` struct literal at line 265. ✓
The `parse_optional_datetime` helper maps `None → None`, `Some(s) → parse_rfc3339 → Ok(Some(dt))` or `Err`. ✓

### 6. ✅ update_last_check timing
In `GoalConvergenceCheckHandler::handle()`, `update_last_check` is invoked **only inside the `Ok(_)` arm** of the `command_bus.dispatch(envelope).await` match. The `DuplicateCommand` arm logs and skips; the `Err(e)` arm logs a warning — neither calls `update_last_check`. ✓

### 7. ✅ GoalStagnationDetectorHandler — event subscription
Subscribes via `EventFilter::new().categories([Scheduler]).payload_types(["ScheduledEventFired"])`.
The `handle()` method immediately returns `Reaction::None` unless `name == "system-stall-check"`.
Correct, though see Minor Observation #1 below.

### 7b. ✅ Grace period — None + recent goal
```rust
if goal.last_convergence_check_at.is_none() {
    let age_secs = (now - goal.created_at).num_seconds();
    if age_secs < threshold_secs {
        continue;  // skip — within grace period
    }
}
```
New goals with no check and `age < threshold` are correctly skipped. ✓

### 7c. ✅ Grace period — None + old goal
After the `is_none()` block, old goals with no check fall through to:
```rust
let is_stagnant = match goal.last_convergence_check_at {
    Some(last_check) => (now - last_check).num_seconds() > threshold_secs,
    None => true,   // No check ever AND outside grace period
};
```
Old goals with `last_convergence_check_at = None` are correctly marked stagnant. ✓

### 8. ✅ Alert dedup
`last_alerted: RwLock<HashMap<Uuid, DateTime<Utc>>>` used correctly:
- Before emitting, checks `secs_since_alert > threshold_secs`
- Records `goal.id → now` before pushing the event
Prevents alert storms within the threshold window. ✓

### 9. ✅ Handler registration
- Threshold: `(p.goal_convergence_check_interval_secs * 3) / 2` = 1.5× interval ✓
- Both `GoalConvergenceCheckHandler` and `GoalStagnationDetectorHandler` registration are inside `if p.goal_convergence_check_enabled { ... }` ✓

### 10. ✅ Tests for update_last_check
Three dedicated tests in `adapters/sqlite/goal_repository.rs`:
- `test_update_last_check_initial_none` — new goal has `None` ✓
- `test_update_last_check_persists` — timestamp round-trips through DB within 2s ✓
- `test_update_last_check_not_found` — returns error for missing goal ID ✓

---

## Edge Cases Verified

| Scenario | Behaviour | Correct? |
|---|---|---|
| `None` check + goal created recently | Grace period — skip | ✅ |
| `None` check + goal created long ago | Mark stagnant, alert | ✅ |
| `Some(ts)` within threshold | Not stagnant, skip | ✅ |
| `Some(ts)` beyond threshold | Mark stagnant, alert | ✅ |
| Paused goals | Excluded by `status = 'active'` in `get_active_with_constraints()` | ✅ |
| Alert dedup within window | Suppressed via `last_alerted` | ✅ |
| Duplicate convergence check command | `update_last_check` NOT called | ✅ |

---

## Minor Observations (non-blocking)

### 1. GoalStagnationDetectorHandler filter could use custom_predicate
The handler's `EventFilter` has no `custom_predicate` narrowing it to `system-stall-check`.
As a result it is invoked for **every** `ScheduledEventFired` event from the Scheduler category
(e.g., reconciliation, stats-update, retry-check …), then exits early via the name guard.
`GoalConvergenceCheckHandler` uses `custom_predicate` to avoid this overhead.
**Impact**: Tiny extra CPU per scheduled tick. Functionally correct. Not a bug.
**Note**: The `EventFilter` builder does not expose a `custom_predicate()` method — it can only be
set via struct literal — so using the builder pattern inherently omits it.

### 2. No unit tests for GoalStagnationDetectorHandler logic
The stagnation handler's grace period, dedup, and alert-emission logic are not directly unit-tested
(hence `cargo test -- stagnation` matched 0 tests). The repository layer (`update_last_check`) is
well covered. A handler-level test exercising the grace/stagnant/dedup branches would strengthen confidence.

### 3. version field hardcoded in TryFrom
`TryFrom<GoalRow> for Goal` always sets `version: 1` — pre-existing behaviour, not introduced
by this PR. A proper version column in the DB would be needed to support optimistic locking.

---

## Overall Assessment

**APPROVE.** The implementation is correct, compiles cleanly, and all existing tests pass.
The migration, struct field, repository method, SELECT queries, TryFrom mapping, handler
update-timing, stagnation logic, dedup, and registration all satisfy the review checklist.
The two non-blocking observations (filter efficiency, missing handler unit tests) are style/
coverage concerns that don't affect correctness.
