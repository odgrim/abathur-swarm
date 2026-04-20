# T11 — Decompose `SwarmOrchestrator` God Object

> Design spec for review. Produced by a planner agent reading current main.
> Implementation should follow the migration plan in §5; each commit must compile + pass tests.

## 1. Summary

**Current state:**
- `SwarmOrchestrator<G, T, W, A, M>` has **42 fields** (lines 53–157 in `mod.rs`).
- `mod.rs` contains **1,583 LoC** of real logic + tests (not a thin index).
- Fields logically cluster into 7 categories: core deps, runtime state, core services, event plumbing, daemon handles, optional services, federation, convergence, and middleware.
- Related files total **~15,275 LoC** across infrastructure, goal_processing, agent_lifecycle, handler_registration, helpers, and convergent_execution submodules.

**Pain points:**
- New developers cannot quickly understand field responsibility or initialization order.
- Constructor `new()` is verbose; non-obvious which fields go together.
- Submodules access 8–15 raw fields; one sub-struct per concern would be cleaner.
- Shutdown sequencing is fragile: daemon handles, oneshot/broadcast channels, and cancellation tokens are scattered across 6 `Arc<RwLock<Option<_>>>` fields.
- Test fixtures must populate 42 fields, making updates error-prone.
- Generic parameters `<G, T, W, A, M>` propagate to all substructures even when only core deps need them.

## 2. Proposed Decomposition

Target: **6 sub-structs** + outer orchestrator shell.

### A. `CoreDeps<G, T, W, A, M>` (inline)
**Purpose:** Immutable repository references and configuration passed through every module.

Fields:
- `goal_repo: Arc<G>`
- `task_repo: Arc<T>`
- `worktree_repo: Arc<W>`
- `agent_repo: Arc<A>`
- `substrate: Arc<dyn Substrate>`
- `config: SwarmConfig`

Methods: none (pure container; field access via `core_deps.goal_repo` etc.).

### B. `RuntimeState` (inline)
**Purpose:** Mutable runtime counters, caches, and state needed by the main loop.

Fields:
- `status: Arc<RwLock<OrchestratorStatus>>`
- `stats: Arc<RwLock<SwarmStats>>`
- `agent_semaphore: Arc<Semaphore>`
- `total_tokens: Arc<AtomicU64>`
- `active_goals_cache: Arc<RwLock<Vec<Goal>>>`
- `escalation_store: Arc<RwLock<HashMap<uuid::Uuid, HumanEscalationEvent>>>`

Methods (moved from infrastructure.rs / agent_lifecycle.rs):
- `async status() -> OrchestratorStatus`
- `async stats() -> SwarmStats`
- `async pause()`
- `async stop()`
- `total_tokens() -> u64`
- `async refresh_active_goals_cache() -> DomainResult<()>`

### C. `SubsystemServices` (inline)
**Purpose:** Long-lived service instances always present (not Option).

Fields:
- `audit_log: Arc<AuditLogService>`
- `circuit_breaker: Arc<CircuitBreakerService>`
- `evolution_loop: Arc<EvolutionLoop>`
- `restructure_service: Arc<tokio::sync::Mutex<DagRestructureService>>`
- `guardrails: Arc<Guardrails>`
- `event_bus: Arc<crate::services::event_bus::EventBus>`
- `event_reactor: Arc<EventReactor>`
- `event_scheduler: Arc<EventScheduler>`

Methods (accessor methods moved from `mod.rs`):
- `overmind() -> Option<&Arc<OvermindService>>`
- `guardrails() -> &Arc<Guardrails>`
- `audit_log() -> &Arc<AuditLogService>`
- `circuit_breaker() -> &Arc<CircuitBreakerService>`
- `evolution_loop() -> &Arc<EvolutionLoop>`

### D. `DaemonHandles` (inline, with `impl Drop`)
**Purpose:** Lifecycle of all background tasks: decay daemon, hourly reset, MCP shutdown, outbox poller, convergence poller/publisher.

Fields:
- `decay_daemon_handle: Arc<RwLock<Option<DaemonHandle>>>`
- `hourly_reset_cancel: Arc<RwLock<Option<tokio_util::sync::CancellationToken>>>`
- `mcp_shutdown_tx: Arc<RwLock<Option<tokio::sync::broadcast::Sender<()>>>>`
- `outbox_poller_handle: Arc<RwLock<Option<OutboxPollerHandle>>>`
- `convergence_poller_handle: Arc<RwLock<Option<ConvergencePollerHandle>>>`
- `convergence_publisher_handle: Arc<RwLock<Option<ConvergencePublisherHandle>>>`

Methods (moved from infrastructure.rs):
- `async start_decay_daemon()`, `async stop_decay_daemon()`
- `async start_outbox_poller()`, `async stop_outbox_poller()`
- `async stop_convergence_poller()`, `async stop_convergence_publisher()`
- `async stop_embedded_mcp_servers()`
- **`impl Drop`**: cancel all CancellationTokens first, then abort all `JoinHandle`s.

### E. `AdvancedServices<G, T, W, A, M>` (Arc-wrapped, Option<_>)
**Purpose:** Progressive-enhancement features gated by builder methods.

Fields (all `Option<_>`):
- `memory_repo: Option<Arc<M>>`
- `intent_verifier: Option<Arc<IntentVerifierService<G, T>>>`
- `overmind: Option<Arc<OvermindService>>`
- `command_bus: Arc<RwLock<Option<Arc<CommandBus>>>>`
- `pool: Option<sqlx::SqlitePool>`
- `outbox_repo: Option<Arc<dyn OutboxRepository>>`
- `trigger_rule_repo: Option<Arc<dyn TriggerRuleRepository>>`
- `merge_request_repo: Option<Arc<dyn MergeRequestRepository>>`
- `adapter_registry: Option<Arc<AdapterRegistry>>`
- `budget_tracker: Option<Arc<BudgetTracker>>`
- `cost_window_service: Option<Arc<CostWindowService>>`
- `federation_client: Option<Arc<FederationClient>>`
- `federation_service: Option<Arc<FederationService>>`
- `overseer_cluster: Option<Arc<OverseerClusterService>>`
- `trajectory_repo: Option<Arc<dyn TrajectoryRepository>>`
- `convergence_engine_config: Option<ConvergenceEngineConfig>`

Methods: none (pure containers; access via builder methods + field access).

### F. `Middleware` (inline)
**Purpose:** Pre-spawn and post-completion middleware chains.

Fields:
- `pre_spawn_chain: Arc<RwLock<middleware::PreSpawnChain>>`
- `post_completion_chain: Arc<RwLock<middleware::PostCompletionChain>>`

Methods (moved from `mod.rs`):
- `async with_pre_spawn_middleware(mw: Arc<dyn PreSpawnMiddleware>) -> Self`
- `async with_post_completion_middleware(mw: Arc<dyn PostCompletionMiddleware>) -> Self`

### Outer `SwarmOrchestrator<G, T, W, A, M>` (now thin)

Remaining fields:
- `core_deps: CoreDeps<G, T, W, A, M>`
- `runtime_state: RuntimeState`
- `subsystem_services: SubsystemServices`
- `daemon_handles: DaemonHandles`
- `advanced_services: AdvancedServices<G, T, W, A, M>`
- `middleware: Middleware`

Public methods (unchanged signatures):
- `new(...)` → constructs sub-structs
- `run(event_tx) -> DomainResult<()>` → main loop, accesses all sub-structs
- `tick() -> DomainResult<SwarmStats>` → single iteration
- `validate_dependencies() -> DomainResult<()>`
- All `with_*` builders → delegate to `advanced_services`
- All public accessors → delegate to sub-structs

Generics:
- `<G, T, W, A, M>` stays on outer struct, propagates to `CoreDeps` and `AdvancedServices`.
- Other sub-structs are generic-free.

## 3. Method Relocation Table

| Method | Current location | New location | Notes |
|--------|------------------|--------------|-------|
| `status()` | infrastructure.rs | `RuntimeState` | accessor |
| `stats()` | infrastructure.rs | `RuntimeState` | accessor |
| `pause()` | infrastructure.rs | `RuntimeState` | status transition |
| `stop()` | infrastructure.rs | `RuntimeState` | status transition |
| `total_tokens()` | infrastructure.rs | `RuntimeState` | atomic read |
| `refresh_active_goals_cache()` | agent_lifecycle.rs | `RuntimeState` | cache update |
| `overmind()` | mod.rs | `SubsystemServices` | accessor |
| `guardrails()` | mod.rs | `SubsystemServices` | accessor |
| `audit_log()` | mod.rs | `SubsystemServices` | accessor |
| `circuit_breaker()` | mod.rs | `SubsystemServices` | accessor |
| `evolution_loop()` | mod.rs | `SubsystemServices` | accessor |
| `with_pre_spawn_middleware()` | mod.rs | `Middleware` | builder |
| `with_post_completion_middleware()` | mod.rs | `Middleware` | builder |
| `start_decay_daemon()` | infrastructure.rs | `DaemonHandles` | lifecycle |
| `stop_decay_daemon()` | infrastructure.rs | `DaemonHandles` | lifecycle |
| `start_outbox_poller()` | infrastructure.rs | `DaemonHandles` | lifecycle |
| `stop_outbox_poller()` | infrastructure.rs | `DaemonHandles` | lifecycle |
| `stop_convergence_poller()` | infrastructure.rs | `DaemonHandles` | lifecycle |
| `stop_convergence_publisher()` | infrastructure.rs | `DaemonHandles` | lifecycle |
| `stop_embedded_mcp_servers()` | infrastructure.rs | `DaemonHandles` | lifecycle |
| `process_goals()` | goal_processing.rs | stays (private) | accesses multiple sub-structs |
| `spawn_task_agent()` | goal_processing.rs | stays (private) | accesses multiple sub-structs |
| `process_evolution_refinements()` | agent_lifecycle.rs | stays (private) | accesses evolution_loop, agent_repo |
| `register_all_agent_templates()` | agent_lifecycle.rs | stays (private) | accesses agent_repo, config |
| `process_specialist_triggers()` | specialist_triggers.rs | stays (private) | accesses multiple sub-structs |
| `drain_ready_tasks()` | mod.rs | stays (private) | main loop helper |
| `drain_specialist_tasks()` | mod.rs | stays (private) | main loop helper |
| All builder `with_*()` (except middleware) | mod.rs | `advanced_services` builders | progressive enhancement |

## 4. Module File Layout

```
src/services/swarm_orchestrator/
├── mod.rs (~250 LoC, was 1,583)
│   ├── SwarmOrchestrator struct (6 fields)
│   ├── new() / run() / tick() / validate_dependencies()
│   ├── public accessors (one-line delegates)
│   └── #[cfg(test)] tests
│
├── core_deps.rs           (NEW, ~50 LoC)
├── runtime_state.rs       (NEW, ~200 LoC)
├── subsystem_services.rs  (NEW, ~100 LoC)
├── daemon_handles.rs      (NEW, ~300 LoC; includes impl Drop)
├── advanced_services.rs   (NEW, ~50 LoC)
├── middleware.rs          (KEEP, ~530 LoC, unchanged)
├── infrastructure.rs      (~250 LoC after decomposition)
├── goal_processing.rs     (~100 LoC after decomposition; modified to take &CoreDeps etc.)
├── agent_lifecycle.rs     (~100 LoC after decomposition; modified similarly)
├── [other submodules unchanged]
└── types.rs               (KEEP, ~1,026 LoC, unchanged)
```

## 5. Migration Plan

Each commit compiles + passes tests independently.

1. **Introduce sub-struct types (no behavior change).** Add the 5 new files with type definitions only. Update `mod.rs` to import them.
2. **Construct sub-structs in `new()` (state duplication).** Update `SwarmOrchestrator::new()` to instantiate sub-structs alongside old fields. Add 6 new fields; keep old 42 intact temporarily.
3. **Migrate accessors to sub-structs.** Move `status(), stats(), pause(), stop(), total_tokens()` to `RuntimeState`. Move accessor methods to `SubsystemServices`. Update call sites in `run()` to access via `self.runtime_state.status()`. Delete old methods.
4. **Migrate middleware builders.** Move `with_pre_spawn_middleware()`, `with_post_completion_middleware()` to `Middleware`. Update call sites.
5. **Migrate daemon lifecycle.** Move all `start_*_daemon()` / `stop_*_daemon()` methods to `DaemonHandles`. Implement `impl Drop`. Update `run()` to call via `self.daemon_handles.start_decay_daemon()`.
6. **Refactor submodules to accept `&CoreDeps`, `&RuntimeState`, etc.** Split out helper functions in goal_processing, agent_lifecycle, infrastructure that accept decomposed parameters instead of `&self`. Public orchestrator API unchanged.
7. **Remove duplicated fields.** Delete the 42 old fields. `SwarmOrchestrator` now has 6. Update all `self.goal_repo` → `self.core_deps.goal_repo`. Update builder methods to mutate the right sub-struct.
8. **Cleanup + tests.** Run full suite. Update test fixtures for new constructor signatures. Verify clippy + no API changes.

## 6. Risks

**Risk 1: Generic type parameter propagation.** `<G, T, W, A, M>` belongs on `CoreDeps` and `AdvancedServices` (memory_repo). Other sub-structs don't need generics. Submodules that only use those don't need generic bounds.
*Mitigation:* Document which sub-structs are generic. Submodules accept `&CoreDeps` and `&RuntimeState` as separate parameters.

**Risk 2: Shutdown ordering / Drop.** `DaemonHandles` holds 6 `Option<Handle>` + cancellation tokens. Drop order matters; aborting handle before cancelling its token may leak.
*Mitigation:* Explicit `impl Drop for DaemonHandles` cancels all `CancellationToken`s first, then aborts all `JoinHandle`s. Test that constructs `DaemonHandles` with all fields populated and verifies Drop completes without panic.

**Risk 3: Test fixtures.** Some tests construct orchestrators by hand (`test_support`). They'll need to populate 6 sub-struct constructors.
*Mitigation:* Add `for_testing()` / `test_builder()` factory on each sub-struct returning sensible defaults. Use `Arc::new(Default::default())` where possible. Update `test_support` once.

**Risk 4: Breaking internal contracts with submodules.** goal_processing, agent_lifecycle, infrastructure are `pub(crate)` submodules. Changing their `impl SwarmOrchestrator` blocks to accept `(&CoreDeps, &RuntimeState, ...)` instead of `&self` breaks their pattern.
*Mitigation:* Keep public `impl` block unchanged. Only refactor private helpers. Public methods on `SwarmOrchestrator` stay; they call private helpers that decompose.

**Risk 5: Option<_> fields modeling optional subsystems.** Fields like `memory_repo` are `Option<_>` because subsystem is optional. Stay `Option<_>` on `AdvancedServices`, or wrap whole sub-struct in `Option`?
*Mitigation:* Keep `Option<_>` on the sub-struct. Each feature is independently optional. Multi-field features (convergence requires trajectory_repo + overseer_cluster + intent_verifier) are validated in `validate_dependencies()`.

## 7. Test Strategy

**New unit tests:**
- `test_core_deps_construction()`
- `test_runtime_state_defaults()`
- `test_daemon_handles_drop()` — verify cancellation + abort order

**Update existing tests:**
- `test_orchestrator_creation()` — constructs via sub-struct chain; verify wiring.
- `test_status_pause_resume()` — accessors route through `RuntimeState`; behavior unchanged.
- `test_tick()` — main loop accesses all sub-structs.

**Submodule tests pass unchanged** — goal_processing/agent_lifecycle tests call public orchestrator methods, not private helpers.

**Dependency validation:** `test_validate_dependencies_*` unchanged — calls `orchestrator.validate_dependencies()`.

**Public API stability:** new `test_public_api_unchanged()` — list all public methods, verify they exist on outer `SwarmOrchestrator` with same signature.

## 8. Out of Scope

- **T10: Extract `spawn_task_agent` into cohesive services.** spawn_task_agent is its own concern. After T11, T10 can extract into `TaskSpawningService` accepting `&CoreDeps, &RuntimeState, &SubsystemServices`.
- **T14, T15: split memory_service, federation/service.** Already completed.
- **T20: Remove deprecated event_bus publish APIs.** Separate effort.
- **T16: Dead code suppressions.** Likely resolved post-T11; defer.
