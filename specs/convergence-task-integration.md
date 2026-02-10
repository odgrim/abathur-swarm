# Convergence-Task Integration

## Problem Statement

The task lifecycle (`Pending → Ready → Running → Complete/Failed`) and the convergence engine (`prepare → converge → outcome`) are two fully-implemented but disconnected systems. Today, when a task enters Running, the orchestrator does a single-shot substrate invocation — the agent either solves it or fails. There is no iterative refinement, no attractor tracking, no strategy learning. The convergence engine exists but is never invoked outside of tests.

Not all tasks benefit from convergence. A trivial rename, a config change, or a templated scaffold does not need attractor classification or Thompson Sampling. Convergence adds latency, token cost, and complexity that should only be paid when the task warrants it. The integration must make convergence **opt-in per task**, with a sensible classification heuristic as the default.

## Design Principles

1. **Convergence is a task execution mode, not a replacement.** The existing single-shot path remains the default. Convergence wraps it as an iterative envelope.
2. **The Task is the source of truth for lifecycle state.** The convergence engine informs outcomes but does not own task status transitions — the orchestrator does.
3. **One iteration = one substrate invocation.** Each cycle of the convergence loop maps to exactly one agent execution, keeping the substrate model unchanged.
4. **The convergence engine is stateless across invocations.** It reads trajectory state from the repository at the start of each iteration and writes it back. The orchestrator can restart without losing convergence progress.
5. **The orchestrator owns the outer loop; the engine owns the inner logic.** The convergence engine exposes both a monolithic `converge()` method (used in tests and standalone mode) and granular primitives (`iterate_once`, `check_loop_control`, `select_strategy`, `initialize_bandit`). The orchestrator uses the granular primitives because it must inject substrate execution between strategy selection and overseer measurement — a step the engine intentionally does not own. This split keeps the substrate model decoupled from convergence logic.
6. **Worktrees are long-lived during convergence.** In direct mode, a worktree is created before execution and destroyed after. In convergent mode, the same worktree persists across all iterations so each substrate invocation builds on the previous artifact. The worktree is only destroyed after convergence terminates.

---

## Part 1: Execution Mode

### 1.1 ExecutionMode Enum

Add a field to `Task` that declares how the task should be executed:

```
enum ExecutionMode {
    /// Single-shot substrate invocation. Agent runs once; result is
    /// accepted or the task fails. This is the current behavior.
    Direct,

    /// Convergence-guided iterative execution. The convergence engine
    /// wraps repeated substrate invocations with strategy selection,
    /// overseer measurement, and attractor tracking.
    Convergent {
        /// Whether to use parallel trajectory sampling (spec 6.6).
        /// When None, uses sequential mode (default).
        /// When Some(n), spawns n parallel trajectories and selects the best.
        parallel_samples: Option<u32>,
    },
}
```

Default: `ExecutionMode::Direct`.

The `parallel_samples` field maps directly to the existing `ConvergenceMode` enum and the `TaskSubmission.parallel_samples` field. Parallel mode is appropriate for narrow-basin tasks where independent starts outperform iterative refinement.

### 1.2 Setting Execution Mode

Execution mode can be set explicitly or inferred:

**Explicit** — The submitter sets it:
```
Task::new("Implement OAuth2 login flow")
    .with_execution_mode(ExecutionMode::Convergent { parallel_samples: None })
```

**Inferred** — A classification heuristic runs during `submit_task()` when no explicit mode is set. The heuristic considers:

| Signal | Points toward Convergent |
|--------|--------------------------|
| `complexity == Complex` | Strong |
| `complexity == Moderate` and description > 200 words | Moderate |
| Description contains test expectations or acceptance criteria | Moderate |
| Task has anti-patterns or constraints in context | Moderate |
| `source == SubtaskOf` and parent is Convergent | Inherit |
| `complexity == Trivial` or `complexity == Simple` | Strong toward Direct |
| `priority_hint == Fast` | Strong toward Direct |
| Historical memory: similar tasks failed in Direct mode | Moderate |

The heuristic produces a recommendation. If `SwarmConfig::default_execution_mode` is set to `Direct`, inference is skipped and all tasks default to Direct unless explicitly marked Convergent. This gives operators a kill switch.

### 1.3 SwarmConfig Extension

```
struct SwarmConfig {
    // ... existing fields ...

    /// Default execution mode when not explicitly set and heuristic is disabled.
    /// When set to Some(mode), all tasks without an explicit mode use this.
    /// When None, the classification heuristic decides.
    pub default_execution_mode: Option<ExecutionMode>,

    /// Whether convergent execution is enabled at all.
    /// When false, all tasks run Direct regardless of their execution_mode field.
    /// This is the global kill switch.
    pub convergence_enabled: bool,
}
```

### 1.4 ConvergenceEngineConfig Assembly

The orchestrator must construct a `ConvergenceEngineConfig` for the engine. This bridges the existing `SwarmConfig` and `ConvergenceLoopConfig` to the engine's expected configuration:

```
fn build_engine_config(swarm_config: &SwarmConfig) -> ConvergenceEngineConfig {
    let loop_config = &swarm_config.convergence;

    // Start with default policy, overlay SwarmConfig values
    let mut policy = ConvergencePolicy::default();
    policy.acceptance_threshold = loop_config.min_confidence_threshold;
    policy.partial_acceptance = loop_config.auto_retry_partial;

    ConvergenceEngineConfig {
        default_policy: policy,
        max_parallel_trajectories: 3, // or from config
        enable_proactive_decomposition: true,
        memory_enabled: swarm_config.convergence.task_learning_enabled
            .unwrap_or(true),
        event_emission_enabled: true,
    }
}
```

The existing `ConvergenceLoopConfig` fields map as:
- `max_iterations` → `ConvergenceBudget.max_iterations`
- `min_confidence_threshold` → `ConvergencePolicy.acceptance_threshold`
- `convergence_timeout_secs` → `ConvergenceBudget.max_wall_time`
- `auto_retry_partial` → `ConvergencePolicy.partial_acceptance`
- `require_full_satisfaction` → inverts `ConvergencePolicy.partial_acceptance`

---

## Part 2: The Bridge — Task to Trajectory

### 2.1 Task-to-TaskSubmission Conversion

When a Convergent task enters the execution path, the orchestrator must construct a `TaskSubmission` from the `Task`. This is a lossy conversion — `Task` has fields `TaskSubmission` doesn't care about (dependencies, priority, parent), and `TaskSubmission` has fields `Task` doesn't have (discovered infrastructure, anti-patterns). The bridge fills the gap:

```
fn task_to_submission(task: &Task, goal_id: Option<Uuid>) -> TaskSubmission {
    let mut submission = TaskSubmission::new(task.description.clone());

    // Propagate goal linkage for memory queries and event correlation
    submission.goal_id = goal_id;

    // Map complexity directly — both systems use the same enum
    submission.inferred_complexity = task.routing_hints.complexity;

    // Map parallel samples from execution mode
    if let ExecutionMode::Convergent { parallel_samples } = &task.execution_mode {
        submission.parallel_samples = *parallel_samples;
    }

    // Extract constraints and anti-patterns from task context hints
    for hint in &task.context.hints {
        if hint.starts_with("constraint:") {
            submission = submission.with_constraint(
                hint.trim_start_matches("constraint:").trim().to_string()
            );
        }
        if hint.starts_with("anti-pattern:") {
            submission = submission.with_anti_pattern(
                hint.trim_start_matches("anti-pattern:").trim().to_string()
            );
        }
    }

    // Map relevant files to references
    for file in &task.context.relevant_files {
        submission = submission.with_reference(Reference {
            path: file.clone(),
            reference_type: ReferenceType::ContextFile,
            description: None,
        });
    }

    // Map task priority to convergence priority hint
    match task.priority {
        TaskPriority::Critical | TaskPriority::High => {
            submission = submission.with_priority_hint(PriorityHint::Thorough);
        }
        TaskPriority::Low => {
            submission = submission.with_priority_hint(PriorityHint::Fast);
        }
        _ => {} // Normal — no hint, use defaults
    }

    submission
}
```

Note: `DiscoveredInfrastructure` is assembled separately during `engine.prepare()` through project introspection (detecting cargo, npm, pytest, etc. from the worktree). The bridge does not populate it — that is the engine's responsibility.

### 2.2 Trajectory-Task Linkage

The `Trajectory` already has a `task_id` field. After `engine.prepare()` creates a trajectory, the orchestrator stores the trajectory ID on the task for cross-referencing:

```
// On Task
pub trajectory_id: Option<Uuid>,
```

This field is `Uuid`, not `String` — matching the trajectory's `id: Uuid` type. It is set once during the first iteration of a convergent task and never changes. It allows:
- Looking up convergence state from a task
- Looking up the originating task from a trajectory
- Joining task events with convergence events in logs
- Detecting retry-with-existing-trajectory (Part 4.2)

---

## Part 3: Orchestrator Integration

### 3.1 Execution Flow — Direct (Unchanged)

```
Ready → claim_task() → Running → substrate.execute() → Complete/Failed
```

No changes. This is the existing `spawn_task_agent` path.

### 3.2 Execution Flow — Convergent

```
Ready → claim_task() → Running → convergence_loop() → Validating → Complete/Failed
```

Two differences from direct mode:
1. Instead of calling `substrate.execute()` once, the orchestrator enters a convergence loop that calls it repeatedly, guided by the convergence engine.
2. Before marking Complete, the task transitions through `Validating` for the final verification pass, giving external observers a signal that convergence succeeded and final checks are in progress.

### 3.3 The Convergence Loop (in goal_processing.rs)

Within the spawned tokio task (inside `spawn_task_agent`), after claiming the task:

```
if task.execution_mode.is_convergent() && config.convergence_enabled {
    run_convergent_execution(task, substrate, overseer_cluster, ...).await
} else {
    run_direct_execution(task, substrate, ...).await  // existing code
}
```

The `run_convergent_execution` function:

```
async fn run_convergent_execution(task, substrate, engine, ...) {
    // 1. PREPARE — or resume existing trajectory on retry
    let (mut trajectory, infrastructure, mut bandit) = if let Some(tid) = task.trajectory_id {
        // Retry path: load existing trajectory (see Part 4.2)
        let trajectory = engine.load_trajectory(tid).await?;
        let infrastructure = engine.rebuild_infrastructure(&trajectory);
        let bandit = engine.initialize_bandit(&trajectory).await;
        (trajectory, infrastructure, bandit)
    } else {
        // First run: prepare from scratch
        let submission = task_to_submission(&task, goal_id);
        let (trajectory, infrastructure) = engine.prepare(&submission).await?;

        // Store trajectory linkage on the task
        task.trajectory_id = Some(trajectory.id);
        task_repo.update(&task).await?;

        let bandit = engine.initialize_bandit(&trajectory).await;
        (trajectory, infrastructure, bandit)
    };

    // Emit ConvergenceStarted event
    event_bus.publish(convergence_event(
        EventSeverity::Info, task.id,
        EventPayload::ConvergenceStarted {
            task_id: task.id,
            trajectory_id: trajectory.id,
            estimated_iterations: trajectory.budget.max_iterations,
            basin_width: trajectory.basin_classification_name(),
        }
    )).await;

    // 2. DECIDE — proactive decomposition check
    if engine.config.enable_proactive_decomposition {
        if let Some(outcome) = engine.maybe_decompose_proactively(&trajectory).await? {
            spawn_decomposed_subtasks(&task, &trajectory, &outcome, ...).await;
            return;
        }
    }

    // 3. ITERATE
    loop {
        // 3a. Check for cancellation
        if cancellation_token.is_cancelled() {
            engine.persist_trajectory(&trajectory).await?;
            return; // Task cancellation handled by caller
        }

        // 3b. Select strategy
        let strategy = engine.select_strategy(&trajectory, &bandit);

        // 3c. Build agent prompt from strategy + trajectory context
        let prompt = build_convergent_prompt(
            &task, &trajectory, &strategy, &infrastructure
        );

        // 3d. Execute one substrate invocation (= one iteration)
        let session = substrate.execute(
            SubstrateRequest::new(task.id, &agent_type, &system_prompt, &prompt)
                .with_config(config)
        ).await?;

        // 3e. Collect the artifact (worktree state after agent runs)
        let artifact = collect_artifact_from_session(&session, &task);

        // 3f. Measure with overseers (build, test, lint, type check)
        let signals = overseer_cluster.measure(&artifact, &trajectory.policy).await?;

        // 3g. Build observation from artifact + signals
        let observation = Observation {
            id: Uuid::new_v4(),
            sequence: trajectory.observations.len() as u32,
            timestamp: Utc::now(),
            artifact,
            overseer_signals: signals,
            verification: None, // filled by iterate_once if verification scheduled
            metrics: None,      // computed by iterate_once
            tokens_used: session.total_tokens(),
            wall_time_ms: session.wall_time_ms(),
            strategy_used: strategy.clone(),
        };

        // 3h. Record observation, classify attractor, update bandit
        let loop_control = engine.iterate_once(
            &mut trajectory, &mut bandit, &strategy, observation
        ).await?;

        // 3i. Emit iteration event
        emit_convergence_iteration_event(&event_bus, &task, &trajectory).await;

        // 3j. Act on loop control
        match loop_control {
            LoopControl::Continue => continue,

            LoopControl::Converged => {
                // Transition through Validating before Complete
                task_service.transition_task(task.id, TaskStatus::Validating).await?;
                engine.finalize(&trajectory, ConvergenceOutcome::Converged { .. }, &bandit).await?;
                complete_task(task.id).await;
                break;
            }

            LoopControl::Exhausted => {
                // Use the policy's partial_threshold, not a derived value
                let best = trajectory.best_observation();
                let accept_partial = trajectory.policy.partial_acceptance
                    && best.map_or(false, |b| {
                        b.metrics.as_ref().map_or(false, |m|
                            m.convergence_level >= trajectory.policy.partial_threshold
                        )
                    });

                engine.finalize(
                    &trajectory,
                    ConvergenceOutcome::Exhausted { .. },
                    &bandit,
                ).await?;

                if accept_partial {
                    task_service.transition_task(task.id, TaskStatus::Validating).await?;
                    complete_task(task.id).await;
                } else {
                    fail_task(task.id, "convergence budget exhausted").await;
                }
                break;
            }

            LoopControl::Trapped => {
                engine.finalize(
                    &trajectory,
                    ConvergenceOutcome::Trapped { .. },
                    &bandit,
                ).await?;
                fail_task(task.id, format!(
                    "trapped in {:?} attractor",
                    trajectory.attractor_state.classification
                )).await;
                break;
            }

            LoopControl::Decompose => {
                let outcome = engine.decompose_and_coordinate(&trajectory).await?;
                spawn_decomposed_subtasks(&task, &trajectory, &outcome, ...).await;
                // Parent stays Running — ConvergenceCoordinationHandler
                // completes it when all children finish (Part 4.3)
                break;
            }

            LoopControl::RequestExtension => {
                if engine.request_extension(&mut trajectory).await? {
                    continue;
                } else {
                    engine.finalize(
                        &trajectory,
                        ConvergenceOutcome::BudgetDenied { .. },
                        &bandit,
                    ).await?;
                    fail_task(task.id, "budget extension denied").await;
                    break;
                }
            }
        }
    }

    // Emit terminal event
    event_bus.publish(convergence_event(
        EventSeverity::Info, task.id,
        EventPayload::ConvergenceTerminated { ... }
    )).await;
}
```

### 3.4 Why the Orchestrator Owns the Loop

The convergence engine has a self-contained `converge()` method that runs the entire loop internally. This works in tests and standalone scenarios. However, the orchestrator cannot delegate to `converge()` because:

1. **Substrate injection**: The engine does not invoke substrates. The substrate execution step (3d) lives in the orchestrator and produces the artifact that the engine measures. The engine's `converge()` expects artifacts to already exist.
2. **Cancellation**: The orchestrator must check a cancellation token between iterations (3a). The engine has no concept of external cancellation.
3. **Event emission**: The orchestrator emits task-level events (`ConvergenceStarted`, `ConvergenceIteration`) that the engine doesn't know about.
4. **Prompt assembly**: Building the agent prompt (3c) requires task context, goal context, and convergence state — information that spans both systems.

The engine's primitive methods (`iterate_once`, `check_loop_control`, `select_strategy`, `initialize_bandit`, `finalize`) provide the convergence logic without prescribing the execution model. The orchestrator composes them with substrate execution.

### 3.5 Prompt Assembly for Convergent Tasks

Each iteration of a convergent task builds a specialized prompt that includes convergence context the agent doesn't get in direct mode:

```
fn build_convergent_prompt(task, trajectory, strategy, infrastructure) -> String {
    let mut sections = vec![];

    // Use the effective specification, NOT the raw task description.
    // SpecificationEvolution may have accumulated amendments from
    // previous iterations (discovered requirements, user hints, etc.)
    sections.push(trajectory.specification.effective_description());

    // Strategy-specific instructions
    match strategy {
        RetryWithFeedback => {
            sections.push(format!(
                "Previous attempt feedback:\n{}",
                trajectory.latest_verification_summary()
            ));
        }
        FocusedRepair => {
            sections.push(format!(
                "Focus on fixing these specific issues:\n{}",
                trajectory.persistent_gaps().join("\n")
            ));
        }
        FreshStart { carry_forward } => {
            sections.push(format!(
                "Start fresh. Key learnings from previous attempts:\n{}\n\nRemaining gaps:\n{}",
                carry_forward.failure_summary,
                carry_forward.remaining_gaps.iter()
                    .map(|g| g.description.clone())
                    .collect::<Vec<_>>()
                    .join("\n")
            ));
        }
        IncrementalRefinement => {
            sections.push(
                "The current implementation is partially correct. \
                 Make minimal, targeted changes to address remaining failures \
                 without breaking what already works.".to_string()
            );
        }
        Reframe => {
            sections.push(
                "Reconsider the approach from scratch. The previous approach \
                 has diverged from the goal. Think about the problem differently.".to_string()
            );
        }
        AlternativeApproach => {
            // Include which approaches were already tried so the agent avoids them
            let tried = trajectory.strategy_log.iter()
                .map(|e| format!("- {}", e.strategy_name))
                .collect::<Vec<_>>()
                .join("\n");
            sections.push(format!(
                "Previous approaches that did not converge:\n{}\n\n\
                 Try a fundamentally different approach.",
                tried
            ));
        }
        // ... other strategies shape the prompt differently
    }

    // Acceptance criteria from overseers
    if let Some(obs) = trajectory.observations.last() {
        if let Some(test_results) = &obs.overseer_signals.test_results {
            if !test_results.failures.is_empty() {
                sections.push(format!(
                    "Failing tests that must pass:\n{}",
                    test_results.failures.iter()
                        .map(|f| format!("- {}", f))
                        .collect::<Vec<_>>()
                        .join("\n")
                ));
            }
        }
        if let Some(build_result) = &obs.overseer_signals.build_result {
            if !build_result.success {
                sections.push(format!(
                    "Build errors that must be fixed:\n{}",
                    build_result.errors.join("\n")
                ));
            }
        }
    }

    // Constraints from infrastructure
    if !infrastructure.invariants.is_empty() {
        sections.push(format!(
            "Constraints that must be maintained:\n{}",
            infrastructure.invariants.iter()
                .map(|c| format!("- {}", c))
                .collect::<Vec<_>>()
                .join("\n")
        ));
    }

    sections.join("\n\n---\n\n")
}
```

### 3.6 Artifact Collection

After each substrate invocation, the orchestrator must extract a measurable artifact. In the worktree model, the artifact is the state of the worktree after the agent runs:

```
fn collect_artifact_from_session(session, task) -> ArtifactReference {
    ArtifactReference {
        // The worktree path IS the artifact — overseers run commands against it
        location: task.worktree_path.clone().unwrap_or_default(),
        metadata: {
            let mut m = HashMap::new();
            m.insert("tokens_used".into(), session.total_tokens().to_string());
            m.insert("turns_used".into(), session.turns_completed.to_string());
            m
        },
    }
}
```

Overseers (`cargo build`, `cargo test`, `cargo clippy`, etc.) execute against this worktree path. The `OverseerClusterService` already handles this — it just needs the path.

### 3.7 Worktree Lifecycle in Convergent Mode

In direct mode, the orchestrator creates a worktree before execution and can destroy it after. In convergent mode, the worktree lifecycle differs:

1. **Create once**: The worktree is created before the first iteration, exactly as in direct mode.
2. **Persist across iterations**: The worktree is NOT destroyed between iterations. Each substrate invocation modifies the same worktree, building on previous work. This is essential — overseers run `cargo test` etc. against accumulated changes, not isolated diffs.
3. **Destroy on terminal**: The worktree is destroyed only after convergence terminates (converged, exhausted, trapped) and the task reaches a terminal state.
4. **FreshStart exception**: When the `FreshStart` strategy is selected, the orchestrator resets the worktree to the base branch state (`git checkout -- .` + `git clean -fd`) before the substrate invocation. This provides a clean slate while preserving the worktree allocation. The carry-forward context in the prompt provides the agent with learnings from previous attempts without polluting the working tree.

---

## Part 4: Outcome Mapping

### 4.1 ConvergenceOutcome → TaskStatus

| ConvergenceOutcome | TaskStatus | Notes |
|---|---|---|
| `Converged` | `Validating` → `Complete` | Task succeeded via convergence; final verification pass before completion |
| `Exhausted` (partial accepted) | `Validating` → `Complete` | Best-effort accepted per policy (`convergence_level >= partial_threshold`) |
| `Exhausted` (no partial) | `Failed` | Budget consumed without satisfactory result |
| `Trapped` | `Failed` | Stuck in attractor with no escape strategies |
| `Decomposed` | remains `Running` | Parent waits for child tasks; `ConvergenceCoordinationHandler` cascades completion (Part 4.3) |
| `BudgetDenied` | `Failed` | Extension request denied |

### 4.2 Retry Semantics for Convergent Tasks

When a convergent task fails and is retried (`retry_task()`), the retry **resumes the existing trajectory** rather than starting fresh. The trajectory already contains the full history of observations, attractor state, and bandit learning. A retry:

1. Transitions the task Failed → Ready (existing behavior)
2. Task is claimed and enters Running (existing behavior)
3. The orchestrator detects `task.trajectory_id.is_some()` and loads the existing trajectory instead of calling `engine.prepare()`
4. Grants additional budget (e.g., 50% of original allocation) via `engine.request_extension()`
5. Resets the trajectory phase from its terminal state (Exhausted/Trapped) back to Iterating
6. Re-enters the convergence loop from where it left off

This is fundamentally different from direct-mode retry, which starts from scratch with no memory of what was tried.

If the trajectory is in a Trapped state at retry time, the engine forces a `FreshStart` strategy with carry-forward extracted from the existing trajectory. This ensures the retry learns from the trap rather than repeating it.

If the trajectory is in an Exhausted state, the bandit state is preserved. The additional budget allows strategies that were working to continue.

### 4.3 Decomposition → Child Tasks

When `LoopControl::Decompose` fires, the orchestrator converts the engine's `TaskDecomposition` objects into real `Task` objects using the existing task system.

**The problem with the dependency model**: The `TaskCompletedReadinessHandler` operates on `depends_on` edges (the DAG dependency graph), NOT on `parent_id` relationships. A task with `source: SubtaskOf(parent_id)` is a child in the hierarchy but not a dependency unless an explicit `depends_on` edge exists. Furthermore, the parent task is already in `Running` state when decomposition fires, and the task state machine does not allow `Running → Blocked` or `Running → Pending`. We cannot retroactively add dependency edges to a Running task and expect the readiness handler to cascade.

**Solution**: A new `ConvergenceCoordinationHandler` that monitors child task completion through parent-child relationships AND trajectory phase:

```
async fn spawn_decomposed_subtasks(parent_task, trajectory, outcome, ...) {
    let child_ids = Vec::new();

    for subtask_spec in &outcome.subtasks {
        let child = task_service.submit_task(
            title: Some(subtask_spec.title.clone()),
            description: subtask_spec.description.clone(),
            parent_id: Some(parent_task.id),
            source: TaskSource::SubtaskOf(parent_task.id),
            execution_mode: ExecutionMode::Convergent { parallel_samples: None },
            context: subtask_spec.context.clone(),
            // Dependencies BETWEEN children (ordering) are added here
            depends_on: subtask_spec.depends_on_siblings.clone(),
            // ...
        ).await?;

        child_ids.push(child.id);
    }

    // Update the trajectory to Coordinating phase with child trajectory IDs
    // (children will create their own trajectories during prepare)
    trajectory.phase = ConvergencePhase::Coordinating {
        children: child_ids.clone(),
    };
    trajectory_store.update(&trajectory).await?;
}
```

The `ConvergenceCoordinationHandler` is a new event handler:

```
pub struct ConvergenceCoordinationHandler<T, Tr> {
    task_repo: Arc<T>,
    trajectory_store: Arc<Tr>,
}

// Listens for TaskCompleted and TaskFailed events
// On each event:
//   1. Check if completed task has a parent_id
//   2. Load parent task
//   3. Check if parent has a trajectory_id and trajectory is in Coordinating phase
//   4. Load all sibling tasks (get_subtasks(parent_id))
//   5. If ALL siblings are Complete → complete parent task
//   6. If ANY sibling is Failed → fail parent task (coordination failed)
//   7. Otherwise → do nothing (still waiting)
```

This handler supplements `TaskCompletedReadinessHandler` (which handles DAG dependencies) with parent-child coordination for decomposed convergent tasks. The two handlers serve different purposes:
- `TaskCompletedReadinessHandler`: "All my upstream dependencies are done, I can start"
- `ConvergenceCoordinationHandler`: "All my children finished, I can finish"

### 4.4 Cancellation Semantics

When a convergent task is canceled mid-loop:

1. The `cancellation_token` (a `tokio_util::sync::CancellationToken`) is triggered
2. The convergence loop checks the token at the top of each iteration (3a)
3. On cancellation:
   - The current trajectory is persisted in its current state
   - No finalization (no memory storage, no bandit persistence) — the trajectory is "frozen"
   - The task transitions to Canceled via the normal `cancel_task()` path
   - The worktree is destroyed
4. If the task is later retried (not typical for canceled tasks, but possible), the frozen trajectory can be resumed with additional budget

For decomposed tasks (parent in Coordinating phase):
- Canceling the parent should cascade-cancel all Running/Ready children
- A `ConvergenceCancellationHandler` listens for `TaskCanceled` on tasks with `trajectory_id` in Coordinating phase and cancels all child tasks

---

## Part 5: Overseer Wiring

### 5.1 OverseerCluster in the Orchestrator

The orchestrator needs an `OverseerClusterService` configured with the project's build/test/lint commands. This is assembled once during `SwarmOrchestrator::new()`:

```
// In SwarmOrchestrator
pub(super) overseer_cluster: Option<Arc<OverseerClusterService>>,
```

Builder method:
```
pub fn with_overseer_cluster(mut self, cluster: OverseerClusterService) -> Self {
    self.overseer_cluster = Some(Arc::new(cluster));
    self
}
```

Operators configure the cluster at startup:
```
let mut cluster = OverseerClusterService::new();
cluster.add(Box::new(BuildOverseer::cargo_build()));
cluster.add(Box::new(TestSuiteOverseer::cargo_test()));
cluster.add(Box::new(LintOverseer::cargo_clippy()));
// ... project-specific overseers

orchestrator.with_overseer_cluster(cluster)
```

If no overseer cluster is configured and a task is Convergent, the engine runs with a no-op measurer that returns empty signals. Convergence still works (strategy selection, budget tracking, context degradation) but without external verification. This is a degraded mode logged as a warning.

### 5.2 Overseer Execution Context

Overseers run commands against the task's worktree. The worktree path is passed through the `ArtifactReference` to the overseer cluster. Each overseer's `measure()` receives this path and executes its command there (e.g., `cargo test` in the worktree directory).

The `OverseerClusterService` already implements phased execution (cheap → moderate → expensive) with early bailout on blocking failures (build errors skip expensive overseers). The convergence policy's `skip_expensive_overseers` flag is respected during measurement.

For tasks without worktrees (`use_worktrees = false`), overseers run against the repo root. This is riskier (concurrent tasks can interfere) but functional for single-agent setups. A warning is logged when multiple convergent tasks run without worktree isolation.

---

## Part 6: Event Integration

### 6.1 New Event Payloads

Add convergence-specific payloads to `EventPayload`:

```
// Emitted when convergence loop starts for a task
ConvergenceStarted {
    task_id: Uuid,
    trajectory_id: Uuid,
    estimated_iterations: u32,
    basin_width: String, // "Wide", "Moderate", "Narrow"
    convergence_mode: String, // "sequential" or "parallel"
}

// Emitted after each iteration
ConvergenceIteration {
    task_id: Uuid,
    trajectory_id: Uuid,
    iteration: u32,
    strategy: String,
    convergence_delta: f64,
    convergence_level: f64,
    attractor_type: String,
    budget_remaining_fraction: f64,
}

// Emitted when attractor classification changes
ConvergenceAttractorTransition {
    task_id: Uuid,
    trajectory_id: Uuid,
    from: String,
    to: String,
    confidence: f64,
}

// Emitted when convergence requests a budget extension
ConvergenceBudgetExtension {
    task_id: Uuid,
    trajectory_id: Uuid,
    granted: bool,
    additional_iterations: u32,
    additional_tokens: u64,
}

// Emitted when a fresh start is triggered
ConvergenceFreshStart {
    task_id: Uuid,
    trajectory_id: Uuid,
    fresh_start_number: u32,
    reason: String, // "context_degradation", "security_regression", etc.
}

// Emitted when convergence completes (success or failure)
ConvergenceTerminated {
    task_id: Uuid,
    trajectory_id: Uuid,
    outcome: String, // "converged", "exhausted", "trapped", "decomposed", "budget_denied"
    total_iterations: u32,
    total_tokens: u64,
    final_convergence_level: f64,
}
```

These events flow through the existing `EventBus` and are visible in the TUI, audit log, and event store.

Note: The convergence engine internally emits `ObservationRecorded` events through its own event emission path (when `event_emission_enabled` is true). The orchestrator-level events above supplement these with task-correlated lifecycle events. Both event streams share the same `correlation_id` for tracing.

### 6.2 Event Category

Add `Convergence` to the `EventCategory` enum:
```
pub enum EventCategory {
    // ... existing ...
    Convergence,
}
```

### 6.3 Reconciling Engine Events and Orchestrator Events

The convergence engine can emit events internally (it accepts an event bus reference). The orchestrator also emits events. To avoid duplication:

- **Engine events** are low-level: `ObservationRecorded`, attractor state changes, bandit updates. These are emitted within `iterate_once()` and `finalize()`.
- **Orchestrator events** are high-level: `ConvergenceStarted`, `ConvergenceIteration` (a summary), `ConvergenceTerminated`. These are emitted by the orchestrator loop.
- Both share `correlation_id` so they can be joined in queries.
- The engine's `event_emission_enabled` flag can be turned off if only orchestrator-level events are desired (reduces noise for operators who don't need per-observation detail).

---

## Part 7: Intervention Points

### 7.1 Surface Model

The convergence engine defines `InterventionPoint` — natural boundaries where the engine pauses for input. These include attractor transitions, strategy escalations, budget extensions, ambiguity detection, partial results, and human escalations.

In the integrated model, intervention points surface to the orchestrator through the `LoopControl` return value and through dedicated events. The orchestrator maps them to the existing escalation system:

| InterventionPoint | Orchestrator Action |
|---|---|
| `AttractorTransition` | Emit `ConvergenceAttractorTransition` event. If priority is `Thorough`, emit `HumanEscalationNeeded` and pause. Otherwise, continue automatically. |
| `StrategyEscalation` | If the proposed strategy is `ArchitectReview` or `Decompose`, emit `HumanEscalationNeeded`. Otherwise, auto-approve. |
| `BudgetExtension` | Handled by `LoopControl::RequestExtension`. Emits `ConvergenceBudgetExtension` event. Auto-grants up to `max_extensions`; beyond that, requires human approval via escalation. |
| `AmbiguityDetected` | Emit `HumanEscalationNeeded` with contradiction details. Pause convergence loop and wait for response. Response is added as a specification amendment. |
| `PartialResult` | Handled by `LoopControl::Exhausted` with partial acceptance logic. |
| `HumanEscalation` | Emit `HumanEscalationRequired` (critical severity). The convergence loop pauses. If a response arrives (via `HumanResponseReceived` event), it is applied as a hint or specification amendment and the loop resumes. If the escalation expires, the task fails. |

### 7.2 Escalation-Convergence Feedback

When a human responds to an escalation during convergence:

1. The `HumanResponseReceived` event is processed by a handler
2. The handler loads the trajectory and appends the response as a specification amendment
3. The convergence loop resumes on its next iteration check
4. The amended specification feeds into prompt assembly and strategy selection

This reuses the existing `HumanEscalationRequired` / `HumanResponseReceived` event infrastructure.

---

## Part 8: SLA and Deadline Integration

### 8.1 Deadline → Budget Ceiling

When a task has a `deadline`, the convergence budget's `max_wall_time` is capped to not exceed the deadline:

```
let wall_time_cap = task.deadline
    .map(|d| d - Utc::now())
    .filter(|d| d.num_seconds() > 0)
    .map(|d| Duration::from_secs(d.num_seconds() as u64));

if let Some(cap) = wall_time_cap {
    trajectory.budget.max_wall_time = trajectory.budget.max_wall_time.min(cap);
}
```

This ensures convergent tasks respect SLA deadlines. If the cap is very tight, the budget will exhaust quickly, leading to either partial acceptance or failure — but never SLA breach.

### 8.2 SLA Events as Budget Pressure

The existing `TaskSLAWarning` and `TaskSLACritical` events can be consumed by a handler that adjusts convergence behavior:

- `TaskSLAWarning`: Lower the `acceptance_threshold` to `partial_threshold` if not already. This makes convergence more willing to accept a "good enough" result.
- `TaskSLACritical`: Force the convergence policy to `skip_expensive_overseers = true` and reduce remaining iterations. Prioritize speed over thoroughness.

---

## Part 9: Evolution Loop Integration

### 9.1 Convergent TaskExecution Recording

The evolution loop currently records a single `TaskExecution` per task. For convergent tasks, this needs adaptation:

- **One TaskExecution per convergence run** (not per iteration). The execution records the aggregate:
  - `tokens_used`: Total across all iterations
  - `turns_used`: Total substrate turns across all iterations
  - `outcome`: Derived from `ConvergenceOutcome` (Converged → Success, Exhausted with partial → Success, others → Failure)
- **Additional convergence metadata** attached to the execution:
  - `iterations_used`: How many convergence iterations ran
  - `attractor_path`: Sequence of attractor classifications encountered
  - `strategies_used`: Which strategies were tried and their deltas
  - `final_convergence_level`: The convergence level at termination

This metadata feeds the evolution loop's `TemplateStats` to track which agent templates perform better under convergent execution.

### 9.2 Convergence-Informed Template Evolution

Over time, the evolution loop accumulates convergent execution data. This enables:

- Identifying agent templates that converge faster (fewer iterations for equivalent complexity)
- Identifying templates that get trapped more often (poor at escaping limit cycles)
- Adjusting template routing: templates with high convergence success can be preferred for convergent tasks

---

## Part 10: Memory Feedback Loop

### 10.1 On Convergence Success

When a convergent task completes successfully, the engine's `store_success_memory()` persists:
- The winning strategy sequence
- The attractor path (how the trajectory evolved)
- The task complexity and basin width
- The total iterations and tokens consumed

This feeds future tasks: when a similar task arrives, the bandit can be warm-started with priors from this success.

### 10.2 On Convergence Failure

When a convergent task fails, `store_failure_memory()` persists:
- The strategies that were tried and their outcomes
- The attractor trap that couldn't be escaped
- The persistent gaps that were never resolved

This feeds future tasks: the bandit deprioritizes strategies that failed on similar tasks.

### 10.3 On Direct Task Completion (Opportunistic)

Even direct-mode tasks can contribute to convergence memory. When a direct task completes, a lightweight "observation" is recorded:
- Task complexity
- Whether it succeeded or failed
- Tokens consumed

This builds the dataset that the classification heuristic (Part 1.2) uses to decide which complexity levels benefit from convergence.

---

## Part 11: Parallel Convergence Mode

### 11.1 When to Use Parallel Mode

The existing `ConvergenceMode::Parallel { initial_samples }` variant runs multiple independent trajectories and selects the best. This is triggered when:

- `ExecutionMode::Convergent { parallel_samples: Some(n) }` is explicitly set, OR
- The basin width is Narrow AND the priority hint is not Fast

Parallel mode is most effective for narrow-basin tasks where the correct approach is hard to find but easy to verify once found.

### 11.2 Orchestrator Mechanics

In parallel mode, the orchestrator loop changes:

1. **Phase 1 — Independent starts**: Spawn `n` substrate invocations, each with a different random strategy. Each produces an artifact. Measure all artifacts.
2. **Phase 2 — Thompson selection**: The engine selects the best trajectory based on convergence level and continues iterating on it sequentially.

This requires `n` worktrees (one per parallel trajectory). The orchestrator:
- Creates `n` worktrees from the base branch
- Runs `n` substrate invocations concurrently (bounded by `agent_semaphore`)
- Measures each worktree independently
- Selects the best, destroys the rest, and continues sequentially on the winner

If worktrees are disabled, parallel mode falls back to sequential (logged as a warning).

### 11.3 Budget Partitioning

The convergence budget is partitioned:
- Phase 1 gets `initial_samples` iterations worth of budget (one per trajectory)
- Phase 2 gets the remaining budget for sequential iteration on the winner

---

## Part 12: Migration and Persistence

### 12.1 Task Table Changes

New migration adding the execution mode and trajectory link to the tasks table:

```sql
ALTER TABLE tasks ADD COLUMN execution_mode TEXT NOT NULL DEFAULT 'direct';
ALTER TABLE tasks ADD COLUMN trajectory_id TEXT;
```

The `execution_mode` column stores the serialized enum. For `Convergent { parallel_samples: Some(3) }`, this serializes to `'convergent:3'` or a JSON blob depending on the serde strategy. Using JSON is more extensible:

```sql
-- execution_mode stores: '{"Direct":{}}' or '{"Convergent":{"parallel_samples":3}}'
ALTER TABLE tasks ADD COLUMN execution_mode TEXT NOT NULL DEFAULT '{"Direct":{}}';
```

### 12.2 Convergence Config in SwarmConfig

The existing `ConvergenceLoopConfig` in `SwarmConfig` already has `max_iterations` and `min_confidence_threshold`. These map to the convergence engine's policy via the assembly function in Part 1.4.

The new `convergence_enabled` field gates the entire system.

---

## Implementation Phases

### Phase 1: Domain Model + Execution Mode
- Add `ExecutionMode` enum to `task.rs`
- Add `execution_mode` and `trajectory_id` fields to `Task`
- Add `with_execution_mode()` builder method
- Add migration for new columns
- Add `convergence_enabled` and `default_execution_mode` to `SwarmConfig`
- Add `build_engine_config()` assembly function
- All existing behavior unchanged (everything defaults to Direct)

### Phase 2: Bridge Functions + Events
- Implement `task_to_submission()` conversion (with goal_id, parallel_samples)
- Implement `collect_artifact_from_session()` artifact extraction
- Implement `build_convergent_prompt()` prompt assembly (all strategy variants)
- Add `Convergence` event category and all new event payloads
- Add `ConvergenceCoordinationHandler` for decomposition parent-child tracking
- Add `ConvergenceCancellationHandler` for cascade cancellation
- Unit tests for each conversion function

### Phase 3: Orchestrator Wiring
- Add `overseer_cluster` field to `SwarmOrchestrator`
- Add `with_overseer_cluster()` builder
- Split `spawn_task_agent` into `run_direct_execution` and `run_convergent_execution`
- Implement `run_convergent_execution` loop using engine primitives
- Implement outcome mapping (`ConvergenceOutcome → TaskStatus`) with Validating transition
- Implement decomposition-to-subtask spawning with ConvergenceCoordinationHandler wiring
- Implement worktree lifecycle (long-lived, FreshStart reset)
- Implement cancellation token checking
- Implement SLA/deadline → budget ceiling capping
- Integration tests: direct path unchanged, convergent path converges simple case

### Phase 4: Classification Heuristic + Parallel Mode
- Implement complexity-based execution mode inference in `submit_task()`
- Add configuration for heuristic thresholds
- Implement parallel convergence mode in orchestrator (multi-worktree, concurrent substrate)
- Add opportunistic memory recording for direct tasks
- Integration tests covering both execution paths and parallel mode

### Phase 5: Retry + Memory + Evolution
- Implement trajectory-aware retry for convergent tasks (detect existing trajectory, grant additional budget)
- Handle Trapped retry with forced FreshStart
- Wire `store_success_memory` and `store_failure_memory` to task lifecycle events
- Implement bandit warm-start from memory on convergent task preparation
- Implement convergence-aware evolution tracking (aggregate metrics per convergent run)
- Implement intervention point → escalation mapping
- Implement SLA event → budget pressure handler
- End-to-end integration tests

---

## Appendix A: Conceptual Model

```
                    ┌─────────────────────────────────────────────┐
                    │             Task Lifecycle                   │
                    │  Pending → Ready → Running → Validating     │
                    │                       │        → Complete    │
                    │                       │        → Failed      │
                    │                       ▼                      │
                    │              ┌─────────────────┐             │
                    │              │ Execution Mode?  │             │
                    │              └────┬────────┬────┘             │
                    │                   │        │                  │
                    │              Direct    Convergent             │
                    │                │        │                     │
                    │                ▼        ▼                     │
                    │          ┌────────┐ ┌───────────────────┐     │
                    │          │ Single │ │ Convergence Loop  │     │
                    │          │  Shot  │ │                   │     │
                    │          └───┬────┘ │  prepare()        │     │
                    │              │      │  loop {           │     │
                    │              │      │    strategy       │     │
                    │              │      │    substrate      │     │
                    │              │      │    measure        │     │
                    │              │      │    classify       │     │
                    │              │      │    control?       │     │
                    │              │      │  }                │     │
                    │              │      │  finalize()       │     │
                    │              │      └───────┬───────────┘     │
                    │              │              │                  │
                    │              ▼              ▼                  │
                    │         Complete/Failed  Complete/Failed/     │
                    │                         Decomposed            │
                    └─────────────────────────────────────────────┘

    Decomposed:
        Parent stays Running (Coordinating phase)
        Children: Pending → Ready → Running → ... → Complete
        ConvergenceCoordinationHandler:
            All children Complete → Parent Complete
            Any child Failed → Parent Failed
```

## Appendix B: Correction Log

Issues corrected from the original spec draft:

1. **Decomposition cascade handler**: The original spec claimed `TaskCompletedReadinessHandler` would cascade parent completion when children finish. This is incorrect — that handler operates on `depends_on` DAG edges, not `parent_id` relationships. A Running parent cannot have dependency edges added retroactively (state machine disallows Running → Blocked). Corrected to use a new `ConvergenceCoordinationHandler` that monitors parent-child relationships through `get_subtasks()`.

2. **Partial acceptance threshold**: The original spec used `acceptance_threshold * 0.8` to check partial acceptance. The convergence policy already has a dedicated `partial_threshold` field (default 0.7) for exactly this purpose. Corrected to use `policy.partial_threshold`.

3. **Trajectory ID type**: The original spec declared `trajectory_id: Option<String>`. The trajectory's `id` field is `Uuid`. Corrected to `Option<Uuid>` for type consistency.

4. **Missing goal_id in bridge**: The original `task_to_submission()` function did not propagate `goal_id`. Added goal_id parameter and mapping.

5. **Prompt uses raw description**: The original `build_convergent_prompt()` used `task.description`. In convergent mode, the effective specification (with accumulated amendments) should be used instead. Corrected to use `trajectory.specification.effective_description()`.

6. **Missing Validating state**: The task state machine has a `Validating` intermediate state between Running and Complete. The original spec skipped it entirely. Corrected to transition through Validating on convergence success, giving external observers visibility into the final verification pass.

7. **Engine loop ownership unexplained**: The original spec reimplemented the convergence loop without explaining why the engine's `converge()` method isn't used. Added Design Principle 5 and Part 3.4 explaining the architectural split.
