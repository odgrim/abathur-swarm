# A2A Federation: Finishing Touches

Three runtime features close the gap between the implemented abstractions and a working end-to-end federation pipeline. Each is small (one service/handler), builds on existing patterns, and requires no new abstractions.

## 1. Convergence Polling Daemon

**Problem**: The parent swarm creates `FederatedGoal` records when it delegates goals to child swarms, but nothing periodically checks whether those goals have converged. The `SwarmOverseer` can measure child state, and `ConvergenceContract::is_satisfied()` can evaluate signals, but no background loop connects them.

**What to build**: A `ConvergencePollingDaemon` service that periodically polls each active `FederatedGoal`, measures child swarm state, evaluates the convergence contract, transitions the goal state, and emits lifecycle events.

### Design

```
src/services/federation/convergence_poller.rs
```

```rust
pub struct ConvergencePollingDaemon {
    federation_service: Arc<FederationService>,
    a2a_client: Arc<dyn A2AClient>,
    federated_goal_repo: Arc<dyn FederatedGoalRepository>,
    event_bus: Arc<EventBus>,
    poll_interval: Duration,
}
```

**Lifecycle**: Spawned as a `tokio::spawn` background task when the swarm starts with federation enabled. Stopped via a broadcast shutdown channel (same pattern as `MemoryDecayDaemon`).

**Each tick**:
1. Query `federated_goal_repo.get_active()` for all non-terminal goals
2. For each goal in `Delegated` or `Active` or `Converging` state:
   a. Build a `SwarmOverseer` from the goal's `cerebrate_id`, `remote_task_id`, and the cerebrate's URL
   b. Call `overseer.measure()` to get `OverseerSignals`
   c. Build a `ConvergenceSignalSnapshot` from the signals
   d. Call `goal.convergence_contract.is_satisfied(&snapshot)`
   e. Update goal state: `Delegated → Active` on first signal, `Active → Converging` on positive delta, `Converging → Converged` when contract satisfied
   f. Call `federated_goal_repo.update_state()` and `update_signals()`
   g. Emit `FederatedGoalProgress` on each measurement
   h. Emit `FederatedGoalConverged` or `FederatedGoalFailed` on terminal transitions
3. Sleep for `poll_interval` (default: per-goal `contract.poll_interval_secs`, minimum 30s)

**Error handling**: If `SwarmOverseer::measure()` fails (child unreachable), increment a miss counter on the goal. After N consecutive misses (configurable, default 5), transition to `Failed` with reason "child swarm unreachable".

### Files to create/modify

| Action | File |
|--------|------|
| Create | `src/services/federation/convergence_poller.rs` |
| Modify | `src/services/federation/mod.rs` — add module + re-export |
| Modify | `src/services/swarm_orchestrator/infrastructure.rs` — spawn daemon on start if federation enabled |

### Existing code to reuse

- `SwarmOverseer::new()` and `measure()` — already handles A2A polling and signal extraction
- `ConvergenceContract::is_satisfied()` — already evaluates all signal types
- `FederatedGoalRepository::update_state()` / `update_signals()` — atomic SQLite updates
- `MemoryDecayDaemon` in `src/services/memory_decay_daemon.rs` — follow the same spawn/shutdown/tick pattern
- `event_factory::federation_event()` — for event emission

### Tests

- Unit: mock A2AClient returns task with convergence artifacts → daemon transitions goal to Converged
- Unit: mock A2AClient returns error N times → daemon transitions goal to Failed
- Unit: contract not yet satisfied → goal stays in Active, signals updated
- Integration: full cycle with SQLite repo

---

## 2. Child Convergence Signal Publishing

**Problem**: The child swarm handles `goal_delegate` by creating an `InMemoryTask` in `Working` state, but never updates it with convergence artifacts. The parent's `SwarmOverseer` polls the child's A2A task expecting structured convergence data in artifacts, but finds nothing.

**What to build**: A background task on the child swarm that periodically snapshots local convergence state and attaches it as an A2A artifact on the federated task.

### Design

```
src/services/federation/convergence_publisher.rs
```

```rust
pub struct ConvergencePublisher {
    tasks: Arc<RwLock<HashMap<String, InMemoryTask>>>,
    goal_repo: Arc<dyn GoalRepository>,
    task_repo: Arc<dyn TaskRepository>,
    event_bus: Arc<EventBus>,
    poll_interval: Duration,
}
```

**Each tick**:
1. Scan `tasks` for any InMemoryTask with `metadata.abathur:federation.intent == "goal_delegate"` and state `Working`
2. For each such task:
   a. Read the convergence contract from `metadata.abathur:federation.convergence_contract`
   b. Query local state to build signal values:
      - `build_passing`: run the configured build check command (from `abathur.toml [checks].build`)
      - `test_pass_rate`: run the configured test command, parse results
      - `convergence_level`: compute from overseer signals if convergence engine is active, else from task completion ratio
      - `tasks_completed` / `tasks_total`: query `task_repo` for tasks under the local goal
   c. Build an `A2AProtocolArtifact` with a `Data` part containing the signal snapshot:
      ```json
      {
        "convergence_level": 0.73,
        "build_passing": true,
        "test_pass_rate": 0.95,
        "type_check_clean": true,
        "tasks_completed": 12,
        "tasks_total": 15
      }
      ```
   d. Append the artifact to the InMemoryTask's artifact list
   e. If the convergence contract is self-satisfied (child can evaluate locally), transition the task to `Completed`

**Artifact format**: Must match what `SwarmOverseer::measure()` expects — it looks for `A2APart::Data` with keys `convergence_level`, `build_passing`, `test_pass_rate`, `type_check_clean`, `security_issues`.

### Simpler alternative

Instead of a separate daemon, hook into the existing event reactor. When the child's convergence engine produces `ConvergenceIteration` events, translate them into artifact updates on the federated task. This is more reactive and avoids polling overhead.

```rust
// In a new EventHandler registered on the child:
// Listen for ConvergenceIteration { task_id, convergence_level, ... }
// Find the InMemoryTask for the federated goal that owns this task
// Append a convergence artifact
```

### Files to create/modify

| Action | File |
|--------|------|
| Create | `src/services/federation/convergence_publisher.rs` |
| Modify | `src/services/federation/mod.rs` — add module |
| Modify | `src/adapters/mcp/a2a_http.rs` — spawn publisher for goal_delegate tasks |

### Existing code to reuse

- `InMemoryTask` and `A2AProtocolArtifact` types in `a2a_http.rs`
- `OverseerClusterService::measure()` — if the child has overseers configured, reuse their signals
- Convergence check commands from `abathur.toml` `[checks]` section

### Tests

- Unit: publisher builds correct artifact format from mock goal/task state
- Unit: artifact format matches what SwarmOverseer::measure() parses
- Integration: child publishes, parent SwarmOverseer reads — signal values round-trip correctly

---

## 3. DAG Execution from CLI

**Problem**: `abathur swarm dag create --file pipeline.yaml` validates and displays the DAG but doesn't start execution. There's no way to kick off the pipeline.

**What to build**: A `dag start` CLI command that creates a parent goal, wires up the `SwarmDagExecutor`, and begins delegation.

### Design

Add to `DagCommand` enum:
```rust
/// Start executing a DAG (delegates root nodes immediately)
Start {
    /// Path to the YAML DAG specification file
    #[arg(long)]
    file: String,
    /// Goal name for the parent goal that owns this pipeline
    #[arg(long, default_value = "Pipeline execution")]
    goal_name: String,
},
```

**`dag_start()` implementation**:
1. Parse the YAML file into a `SwarmDag` (reuse `dag_create` parsing logic — extract into shared helper)
2. Connect to the running swarm's database and federation service
3. Create a parent goal via `GoalService::create()` with the pipeline name
4. Create a `SwarmDagExecutor` with the running `FederationService` and `EventBus`
5. Call `executor.start(&mut dag, &goal)` to delegate root nodes
6. Store the DAG in the `SwarmDagEventHandler`'s active DAGs map (so convergence events drive it forward)
7. Print the initial DAG state and delegated node IDs

**Runtime integration**: The DAG needs to be stored where the `SwarmDagEventHandler` can find it. Options:
- Pass it via the event bus (emit a `SwarmDagCreated` event that the handler picks up)
- Store in a shared `Arc<RwLock<HashMap<Uuid, SwarmDag>>>` accessible from both CLI and handler
- Persist to the database and load on startup

The database approach is the most robust — add a `swarm_dags` table and a `SwarmDagRepository`.

### Additional commands

```
abathur swarm dag stop <dag-id>     # Cancel all non-terminal nodes
abathur swarm dag retry <dag-id>    # Retry failed nodes
```

### Files to create/modify

| Action | File |
|--------|------|
| Modify | `src/cli/commands/swarm.rs` — add `Start` variant, `dag_start()` handler, extract YAML parsing |
| Create | `src/domain/ports/swarm_dag_repository.rs` — persistence trait (optional, could defer) |
| Create | `src/adapters/sqlite/swarm_dag_repository.rs` — SQLite impl (optional) |
| Modify | `src/services/swarm_orchestrator/infrastructure.rs` — register `SwarmDagEventHandler` on startup |

### Existing code to reuse

- `dag_create()` YAML parsing in `swarm.rs` — extract the `DagYamlSpec::into_swarm_dag()` into a shared function
- `SwarmDagExecutor::start()` — already implemented
- `GoalService::create_goal()` — for creating the parent goal
- `FederationService` — already has `delegate_goal()` wired

### Tests

- Integration: `dag start` with mock federation service → root nodes delegated
- Integration: full pipeline with 3 mock child swarms → all nodes converge in order

---

## Implementation Order

```
1. Child convergence publisher     (unblocks parent polling)
2. Convergence polling daemon      (unblocks DAG progression)
3. DAG start CLI command           (unblocks user interaction)
```

Item 1 must come first because without it, the parent has nothing to measure. Item 2 depends on 1 because it polls what 1 publishes. Item 3 depends on 2 because DAG execution requires convergence events to drive node transitions.

Each item is independently testable — you can verify 1 works by checking the child's A2A task artifacts, verify 2 works by watching FederatedGoalConverged events, and verify 3 works by running the full pipeline.

## Estimated Scope

| Item | New Files | Modified Files | Complexity |
|------|-----------|---------------|------------|
| Convergence publisher | 1 | 2 | Small — follows existing daemon patterns |
| Convergence poller | 1 | 2 | Small — reuses SwarmOverseer + contract eval |
| DAG start CLI | 0-2 | 2 | Medium — needs runtime integration with handler |

All three items together are roughly the same scope as Phase 2 was.
