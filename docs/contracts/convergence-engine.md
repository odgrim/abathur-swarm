# Convergence Engine Contract

> `src/services/convergence_engine.rs`, `src/services/convergence_bridge.rs`,
> `src/domain/models/convergence/`

## Overview

The convergence engine drives iterative task convergence through strategy
selection, overseer measurement, attractor classification, and trajectory
management. It is used when a task's `execution_mode` is `Convergent`.

Unlike the workflow engine (deterministic), the convergence engine
involves LLM execution and overseer subprocess calls.

## Lifecycle Phases

```
SETUP/PREPARE ──► ITERATE (loop) ──► RESOLVE
```

### SETUP Phase

1. Estimate basin width from task signals
2. Allocate token budget based on complexity
3. Assemble convergence policy with priority hint overlays
4. Discover infrastructure and amend specification
5. Create trajectory with initial state

### ITERATE Phase (loop)

```
┌──────────────────────────────────────────┐
│                                          │
│  Select Strategy (bandit/policy)         │
│        │                                 │
│        ▼                                 │
│  Execute (LLM call with strategy prompt) │
│        │                                 │
│        ▼                                 │
│  Measure (OverseerMeasurer)              │
│        │                                 │
│        ▼                                 │
│  Record Observation                      │
│        │                                 │
│        ▼                                 │
│  Classify Attractor                      │
│        │                                 │
│        ▼                                 │
│  Loop Control Check ─────────────────────┤
│        │                                 │
│   ┌────┴────┐                            │
│   │Continue │────────────────────────────┘
│   └─────────┘
│
│   Other outcomes:
│   ├── IntentCheck → LLM verification
│   ├── Exhausted → budget consumed
│   ├── Trapped → limit cycle, no escape
│   ├── Decompose → break into subtasks
│   ├── RequestExtension → ask for more budget
│   └── OverseerConverged → all checks passing 2+ times
```

### RESOLVE Phase

1. Persist convergence memory
2. Emit terminal event
3. Update bandit state (strategy selection learning)
4. Return `ConvergenceOutcome`

## Configuration: `ConvergenceEngineConfig`

| Field | Description |
|-------|-------------|
| `default_policy` | Default convergence policy |
| `max_parallel_trajectories` | Max concurrent trajectories |
| `enable_proactive_decomposition` | Allow auto-decomposition |
| `memory_enabled` | Store convergence memory |
| `event_emission_enabled` | Emit convergence events |

## Convergence Policy

Controls iteration behavior:

| Field | Description |
|-------|-------------|
| `max_iterations` | Hard cap on iterations |
| `max_tokens` | Token budget |
| `convergence_threshold` | Convergence level to declare success |
| `enable_fresh_starts` | Allow context reset |
| `max_fresh_starts` | Cap on fresh starts |
| `enable_budget_extensions` | Allow requesting more tokens |
| `overseer_phases` | Which overseers to run (cheap/moderate/expensive) |

## Strategies: `StrategyKind`

| Strategy | When Used | Description |
|----------|-----------|-------------|
| `RetryWithFeedback` | Initial failures | Retry with overseer feedback in prompt |
| `FocusedRepair` | Specific test failures | Target specific failing checks |
| `FreshStart` | Context degradation | Reset context, optionally carry forward learnings |
| `IncrementalRefinement` | Near convergence | Small targeted improvements |
| `Reframe` | Stuck in local minimum | Restate the problem differently |
| `AlternativeApproach` | Multiple failures | Try a fundamentally different approach |
| `Decompose` | Task too complex | Break into convergent subtasks |
| `ArchitectReview` | Structural issues | Request higher-tier review |
| `RevertAndBranch` | Regression detected | Revert to known-good state, branch |
| `RetryAugmented` | Missing context | Retry with additional context/tools |

## Attractor Types

| Type | Description | Escape Strategy |
|------|-------------|-----------------|
| `FixedPoint` | Converged to stable solution | None needed (success) |
| `LimitCycle { period }` | Oscillating between states | `FreshStart`, `Reframe` |
| `StrangeAttractor` | Chaotic, unpredictable | `Decompose`, `AlternativeApproach` |
| `Divergent` | Getting worse each iteration | `FreshStart`, `ArchitectReview` |
| `Unknown` | Insufficient data | Continue observing |

## Overseer Measurement

### `OverseerMeasurer` Trait

```rust
trait OverseerMeasurer: Send + Sync {
    async fn measure(
        artifact: &ArtifactReference,
        policy: &ConvergencePolicy,
    ) -> DomainResult<OverseerSignals>;
}
```

### `OverseerSignals`

| Signal | Type | Fields |
|--------|------|--------|
| `test_results` | `TestResults` | passed, failed, skipped, regression_count, failing_test_names |
| `type_check` | `TypeCheckResult` | clean, error_count, errors |
| `lint_results` | `LintResults` | error_count, warning_count, errors |
| `build_result` | `BuildResult` | success, error_count, errors |
| `security_scan` | `SecurityScanResult` | critical, high, medium counts, findings |
| `custom_checks` | `Vec<CustomCheckResult>` | name, passed, details |

### Phased Execution

Overseers run in cost-ordered phases — if cheap checks fail, expensive
checks are skipped:

| Phase | Overseers | Cost |
|-------|-----------|------|
| 1 | Compilation, TypeCheck, Build | Cheap |
| 2 | Lint, SecurityScan | Moderate |
| 3 | TestSuite, AcceptanceTest | Expensive |

**Contract:** If phase 1 fails, phases 2-3 are skipped. If phase 2
fails, phase 3 is skipped (configurable via policy).

## Loop Control Outcomes

| Outcome | Condition | Action |
|---------|-----------|--------|
| `Continue` | Budget remains, not converged | Next iteration |
| `OverseerConverged` | All overseers passing 2+ consecutive observations | Success |
| `IntentCheck` | Periodic or on plateau | Run LLM intent verification |
| `Exhausted` | Budget fully consumed | Return best observation |
| `Trapped` | Limit cycle detected, no escape strategies remain | Terminal failure |
| `RequestExtension` | Near budget, evidence of progress | Ask for more tokens |
| `Decompose` | Task too complex for single convergence | Break into subtasks |

## Convergence Outcomes (Terminal)

| Outcome | Description |
|---------|-------------|
| `Converged { trajectory_id, final_observation_sequence }` | Successfully converged |
| `Exhausted { trajectory_id, best_observation_sequence }` | Budget consumed, best-effort result |
| `Trapped { trajectory_id, attractor_type }` | Stuck in unescapable cycle |
| `Decomposed { parent_trajectory_id, child_trajectory_ids }` | Split into subtasks |
| `BudgetDenied { trajectory_id }` | Budget extension denied |

## Trajectory Model

A `Trajectory` tracks the full convergence history:

| Field | Type | Description |
|-------|------|-------------|
| `id` | `Uuid` | Trajectory identifier |
| `task_id` | `Uuid` | Associated task |
| `goal_id` | `Option<Uuid>` | Associated goal |
| `specification` | `SpecificationEvolution` | Evolving task spec (amendments tracked) |
| `observations` | `Vec<Observation>` | One per iteration |
| `attractor_state` | `AttractorState` | Current attractor classification |
| `budget` | `ConvergenceBudget` | Token/iteration budgets |
| `policy` | `ConvergencePolicy` | Iteration policy |
| `strategy_log` | `Vec<StrategyEntry>` | Strategy history |
| `phase` | `ConvergencePhase` | Current lifecycle phase |
| `context_health` | `ContextHealth` | Context window degradation tracking |
| `total_fresh_starts` | `u32` | Number of context resets |
| `complexity` | `Option<Complexity>` | Task complexity hint |

## Convergence Bridge

`src/services/convergence_bridge.rs` adapts between the task model and
convergence engine:

### `task_to_submission(task) -> TaskSubmission`
- Converts task fields to convergence input
- Propagates complexity, execution mode, constraints, anti-patterns
- Merges intent verification gaps into description

### `build_convergent_prompt(strategy, spec, feedback, ...) -> String`
- Constructs strategy-specific prompts
- Includes effective specification, overseer feedback, gaps, learnings
- Different templates per strategy kind

### Priority Mapping
| Task Priority | Convergence Hint |
|---------------|------------------|
| Critical/High | Thorough |
| Low | Fast |
| Normal | (no hint) |

## Convergence Events (Domain Model)

Separate from `EventPayload`, these are internal convergence events:

```rust
enum ConvergenceEvent {
    TrajectoryStarted { trajectory_id, task_id, goal_id, budget, timestamp },
    TrajectoryConverged { trajectory_id, total_observations, total_tokens_used, ... },
    TrajectoryExhausted { trajectory_id, best_observation_sequence, reason, ... },
    TrajectoryTrapped { trajectory_id, attractor_type, cycle_period, escape_attempts, ... },
    ObservationRecorded { trajectory_id, convergence_delta, convergence_level, strategy_used, ... },
    AttractorClassified { trajectory_id, attractor_type, confidence },
    StrategySelected { trajectory_id, strategy, reason, ... },
    ContextDegradationDetected { trajectory_id, health_score, fresh_start_number },
    BudgetExtensionRequested / Granted / Denied,
    SpecificationAmended { trajectory_id, amendment_source, amendment_summary },
    SpecificationAmbiguityDetected { task_id, contradictions, suggested_clarifications },
    DecompositionRecommended / Triggered,
    ParallelConvergenceStarted { trajectory_id, parallel_count },
}
```

These are mapped to `EventPayload` variants for the unified event bus.

## Retry Semantics

When a convergent task is retried via `TaskService::retry_task()`:

1. `trajectory_id` is **preserved** (doesn't start fresh trajectory)
2. If context hints contain "trapped" → adds `convergence:fresh_start` hint
3. Convergence engine reads this hint to force `FreshStart` strategy
4. Learned observations from previous attempts are still available

**Contract:** Callers retrying convergent tasks must NOT clear the
`trajectory_id` or context hints — this destroys convergence state.

## Dependencies

| Dependency | Purpose |
|------------|---------|
| `TrajectoryRepository` | Trajectory persistence |
| `MemoryRepository` | Convergence memory (patterns learned) |
| `OverseerMeasurer` | Artifact measurement |
| `BudgetCalibrationTracker` | Token budget tracking |
| `Substrate` | LLM execution (via orchestrator) |

## Concurrency

- `max_parallel_trajectories` limits concurrent trajectory execution
- Per-trajectory observation sequence prevents interleaving
- Bandit algorithm uses immutable snapshots for strategy selection
- Trajectories are independent — no cross-trajectory locking
