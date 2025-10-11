# Task Queue System - Decision Points

**Project:** Abathur Enhanced Task Queue
**Date:** 2025-10-10
**Status:** Awaiting Human Decisions

## Critical Architectural Decisions

These decisions must be resolved before implementation begins to prevent agent blockages.

---

## 1. Database Migration Strategy

**Question:** How should we handle the database schema migration for existing Abathur installations?

**Options:**
- [ x ] Automatic migration on startup (transparent to users)
- [ ] Manual migration script (user runs explicitly)
- [ ] Opt-in feature flag (users enable enhanced queue explicitly)
- [ ] Other: _______________

**Implications:**
- Automatic: Seamless but risky if migration fails
- Manual: Safer but requires user action
- Opt-in: Safest but delays adoption

**Suggestion:** Automatic migration with backup/rollback mechanism. On startup, check schema version, run migrations if needed, create backup before migration.
**Decision** Automatic migration with backup/rollback mechanism. On startup, check schema version, run migrations if needed, create backup before migration.

---

## 2. Maximum Dependency Limits

**Question:** What limits should we enforce to prevent pathological dependency graphs?

**MAX_DEPENDENCIES_PER_TASK:**
- [ ] 10 (conservative, simple graphs)
- [ ] 20 (recommended, balances flexibility and complexity)
- [ x ] 50 (permissive, allows complex workflows)
- [ ] Unlimited (no artificial limits)
- [ ] Other: _______________

**MAX_DEPENDENCY_DEPTH:**
- [ ] 5 levels (shallow trees)
- [ x] 10 levels (recommended, reasonable complexity)
- [ ] 20 levels (deep hierarchies)
- [ ] Unlimited
- [ ] Other: _______________

**Implications:**
- Too restrictive: Limits legitimate use cases
- Too permissive: Allows performance issues, hard-to-debug graphs

**Suggestion:** MAX_DEPENDENCIES_PER_TASK = 20, MAX_DEPENDENCY_DEPTH = 10. Sufficient for most workflows while preventing abuse.

**Decision:**
- MAX_DEPENDENCIES_PER_TASK: 50, but configurable to any number including unlimited
- MAX_DEPENDENCY_DEPTH: 10, but configurable to any number including unlimited

---

## 3. Priority Recalculation Frequency

**Question:** How often should we recalculate dynamic priorities for pending/blocked tasks?

**Options:**
- [x ] Real-time (on every task state change) - most accurate but expensive
- [ ] Every 1 minute - high responsiveness, moderate overhead
- [ ] Every 5 minutes (recommended) - balanced approach
- [ ] Every 15 minutes - lower overhead, less responsive
- [ ] On-demand only (user-triggered) - minimal overhead, least responsive
- [ ] Other: _______________

**Implications:**
- More frequent: Better priority accuracy, higher CPU usage
- Less frequent: Lower overhead, stale priorities

**Suggestion:** Every 5 minutes with on-demand API for urgent cases. Background task runs recalculation, agents can trigger immediate recalc if needed.

**Decision:** Every state change

---

## 4. Priority Calculation Weights

**Question:** What weights should we use for priority calculation factors?

**Formula:**
```
priority = base_priority * base_weight
           + urgency_score * urgency_weight
           + dependency_score * dependency_weight
           + starvation_score * starvation_weight
           + source_score * source_weight
```

**Proposed Weights:**
- base_weight: [ ] 0.5  [ ] 1.0 (default)  [ ] 2.0  [ ] ___
- urgency_weight: [ ] 1.0  [ ] 2.0 (default)  [ ] 3.0  [ ] ___
- dependency_weight: [ ] 1.0  [ ] 1.5 (default)  [ ] 2.0  [ ] ___
- starvation_weight: [ ] 0.5 (default)  [ ] 1.0  [ ] 1.5  [ ] ___
- source_weight: [ ] 0.5  [ ] 1.0 (default)  [ ] 2.0  [ ] ___

**Implications:**
- Higher urgency_weight: Deadline-driven tasks prioritized aggressively
- Higher dependency_weight: Tasks unblocking others prioritized
- Higher starvation_weight: Long-waiting tasks get more boost

**Suggestion:** Use defaults (1.0, 2.0, 1.5, 0.5, 1.0) as starting point. Make configurable so users can tune based on workload characteristics.

**Decision:**
use suggestion

---

## 5. Circular Dependency Handling

**Question:** When a circular dependency is detected, what should happen?

**Options:**
- [x] Reject task submission (fail fast, recommended)
- [ ] Log warning and allow (best-effort execution)
- [ ] Auto-break cycle by removing lowest-priority dependency
- [ ] Queue for human review/resolution
- [ ] Other: _______________

**Implications:**
- Reject: Safest, forces users to fix dependencies
- Allow: Flexible but risks deadlocks
- Auto-break: Clever but might break user intent

**Suggestion:** Reject task submission with detailed error message showing cycle path. Clear error message helps users fix the issue.

**Decision:** follow suggestion

---

## 6. Task Status Transitions

**Question:** Should we add intermediate states between BLOCKED and READY?

**Proposed States:**
- PENDING (submitted, dependencies not yet checked)
- BLOCKED (waiting for dependencies)
- READY (dependencies met, ready to run)
- RUNNING (currently executing)
- COMPLETED/FAILED/CANCELLED (terminal states)

**Alternative:**
- Add SCHEDULED state (READY but not yet picked up by agent)
- Add PAUSED state (user-paused, can resume)
- Keep it simple with current states

**Implications:**
- More states: Finer-grained tracking but more complexity
- Fewer states: Simpler but less visibility

**Suggestion:** Keep proposed states (PENDING/BLOCKED/READY/RUNNING/COMPLETED/FAILED/CANCELLED). Sufficient granularity without over-complicating.

**Decision:**
- [x ] Use proposed states (recommended)
- [ ] Add SCHEDULED state
- [ ] Add PAUSED state
- [ ] Other: _______________

---

## 7. Agent Subtask Submission Authority

**Question:** Should all agents be able to submit subtasks, or restrict to specific roles?

**Options:**
- [x ] All agents can submit subtasks (most flexible)
- [ ] Only specialized agents (requirements-gatherer, task-planner) can submit
- [ ] Whitelist approach (configurable per agent type)
- [ ] Require human approval for agent-submitted tasks
- [ ] Other: _______________

**Implications:**
- All agents: Maximum flexibility, risk of runaway task generation
- Restricted: Safer but limits agentic autonomy
- Approval: Safest but defeats purpose of autonomous agents

**Suggestion:** All agents can submit subtasks, but enforce rate limits (e.g., max 10 subtasks per parent task). Log all agent submissions for audit.

**Decision:** All agents can submit subtasks, but enforce rate limits (e.g., max 50 subtasks per parent task). Log all agent submissions for audit.


---

## 8. Dependency Type Semantics

**Question:** How should we interpret PARALLEL vs SEQUENTIAL dependencies?

**SEQUENTIAL (default):**
- Task B depends on A → B waits for A to complete

**PARALLEL:**
**Option 1:** Task C depends on [A, B] with PARALLEL type → C waits for ALL of [A, B] to complete (AND logic)
**Option 2:** Task C depends on [A, B] with PARALLEL type → C can start when ANY of [A, B] completes (OR logic)

**Implications:**
- AND logic: More restrictive, ensures all prerequisites met
- OR logic: More flexible, allows partial execution

**Suggestion:** PARALLEL = AND logic (wait for all). More intuitive, matches common dependency semantics. Add separate OR_ANY type if OR logic needed later.

**Decision:**
- [x] PARALLEL = AND (wait for all) - recommended
- [ ] PARALLEL = OR (wait for any)
- [ ] Add both PARALLEL_AND and PARALLEL_OR types
- [ ] Other: _______________

---

## 9. Performance vs Accuracy Tradeoffs

**Question:** When system load is high, should we prioritize throughput or priority accuracy?

**Scenarios:**
1. **High task submission rate**: Priority recalculation falling behind
2. **Complex dependency graphs**: Circular detection taking >100ms
3. **Many blocked tasks**: Unblocking checks expensive

**Options:**
- [ ] Always maintain accuracy (may slow down under load)
- [ ] Degrade gracefully (skip recalcs, cache dependency checks)
- [ ] Hybrid: Accurate for human tasks, best-effort for agent tasks
- [ x ] Configurable threshold (e.g., if queue > 1000 tasks, skip recalcs)
- [ ] Other: _______________

**Implications:**
- Accuracy: Better user experience, may hit performance limits
- Degradation: Maintains throughput, priority staleness

**Suggestion:** Configurable threshold approach. Under normal load (<1000 tasks), maintain accuracy. Above threshold, skip periodic recalcs, only recalc on explicit dequeue.

**Decision:**  Configurable threshold approach. Under normal load (<1000 tasks), maintain accuracy. Above threshold, skip periodic recalcs, only recalc on explicit dequeue.


---

## 10. Backward Compatibility

**Question:** Should the new task queue maintain backward compatibility with existing task submissions?

**Considerations:**
- Existing code submits tasks without dependencies, source, etc.
- New fields should have sensible defaults
- Should we support old TaskStatus enum values?

**Options:**
- [ ] Full backward compatibility (old API still works, maps to new schema)
- [ ] Deprecation period (old API works but logs warnings)
- [ x ] Breaking change (require migration to new API)
- [ ] Other: _______________

**Suggestion:** Full backward compatibility. Old submit_task calls work, new fields default to HUMAN source, no dependencies, calculated_priority = base priority. Gradual migration path for users.

**Decision:** Just break it- no one uses this yet

---

## 11. Task Deadline Handling

**Question:** What should happen when a task misses its deadline?

**Options:**
- [ x ] No automatic action (just affects priority)
- [ ] Automatically cancel task
- [ ] Move to FAILED status with "deadline_exceeded" error
- [ ] Notify human for decision
- [ ] Configurable per-task behavior
- [ ] Other: _______________

**Implications:**
- No action: Flexible but may run stale tasks
- Cancel: Prevents wasted work but might cancel critical tasks
- Failed: Clear signal but prevents retries

**Suggestion:** No automatic action (default). Deadline only affects priority calculation (urgency score). Optionally allow per-task deadline_action field (cancel/fail/notify).

**Decision:** No automatic action (default). Deadline only affects priority calculation (urgency score). As we have insight into the return state of the underlying api calls and cli invocations we should know when work is still going vs failed. We should have a max timeout to kill the subprocess and start over or break down the prompt after so many retries.



---

## 12. Dependency Visualization

**Question:** Should we provide tools to visualize dependency graphs?

**Options:**
- [ x] Yes, build GraphViz/Mermaid export (recommended for debugging)
- [ ] Yes, build interactive web UI
- [ ] No, too complex for MVP
- [ ] Later, after core functionality stable
- [ ] Other: _______________

**Implications:**
- Visualization: Very helpful for debugging, adds dev time
- No visualization: Faster MVP, harder to debug complex graphs

**Suggestion:** Build simple text-based export in MVP (show task tree with ASCII). Defer fancy visualization to post-MVP.

**Decision:** GraphViz/Mermaid

---

## 13. Testing Strategy

**Question:** What level of test coverage is required before production?
**Unit Tests:**
- [ ] >80% coverage (recommended)
- [x ] >90% coverage (rigorous)
- [ ] Best-effort (pragmatic)
- [ ] _______________

**Integration Tests:**
- [x ] All user-facing workflows (recommended)
- [ ] Critical paths only
- [ ] _______________

**Performance Tests:**
- [x ] All performance targets validated (recommended)
- [ ] Spot checks only
- [ ] _______________

**Suggestion:** Unit tests >80%, integration tests for all workflows, performance tests for all targets. Comprehensive testing reduces production issues.

**Decision:**
see checkboxes
---

## 14. Logging and Observability

**Question:** What events should we log for debugging and monitoring?

**Options:**
- [ ] Minimal: Task state transitions only
- [ ] Standard: State transitions + dependency events + priority recalcs
- [ ] Verbose: Everything (all calculations, all checks)
- [x ] Configurable log level
- [ ] Other: _______________

**Suggestion:** Configurable log level. Default = Standard (state transitions, dependency resolution, priority recalcs). Enable Verbose for debugging specific issues.

**Decision:** follow suggestion

---

## Summary of Recommendations

**For quick start, recommend accepting all default suggestions:**

1. Automatic migration with backup
2. MAX_DEPENDENCIES_PER_TASK=20, MAX_DEPENDENCY_DEPTH=10
3. Priority recalculation every 5 minutes
4. Default priority weights (1.0, 2.0, 1.5, 0.5, 1.0)
5. Reject circular dependencies
6. Use proposed task states
7. All agents can submit subtasks (with rate limits)
8. PARALLEL = AND logic (wait for all)
9. Configurable degradation threshold
10. Full backward compatibility
11. Deadlines affect priority only (no auto-cancel)
12. Simple text-based dependency export
13. Unit >80%, comprehensive integration/performance tests
14. Configurable log level (default = Standard)

**Estimated implementation time with defaults: 10-12 days**

---
