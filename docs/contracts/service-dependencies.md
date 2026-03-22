# Service Dependencies & Concurrency Boundaries

> Cross-service dependency map, shared state inventory, and concurrency model.

## Service Dependency Graph

```
                        ┌─────────────────────┐
                        │  SwarmOrchestrator   │ (top-level coordinator)
                        └──┬──┬──┬──┬──┬──┬───┘
                           │  │  │  │  │  │
          ┌────────────────┘  │  │  │  │  └────────────────┐
          │     ┌─────────────┘  │  │  └──────────┐        │
          │     │     ┌──────────┘  │             │        │
          ▼     ▼     ▼             ▼             ▼        ▼
      ┌───────┐ ┌──────────┐  ┌──────────┐  ┌─────────┐ ┌──────────┐
      │Overmind│ │TaskService│  │WorkflowEng│  │Converge │ │EventBus  │
      └───┬───┘ └────┬─────┘  └─────┬─────┘  │Engine   │ └────┬─────┘
          │          │               │        └────┬────┘      │
          │          │               │             │           │
          ▼          ▼               ▼             ▼           ▼
      ┌────────┐ ┌────────┐    ┌────────┐   ┌──────────┐ ┌──────────┐
      │Substrate│ │TaskRepo│    │TaskRepo│   │Trajectory│ │EventStore│
      └────────┘ └────────┘    └────────┘   │Repo      │ └──────────┘
                                            └──────────┘
                                                 │
          ┌──────────────────────────────────────┘
          ▼
     ┌──────────┐   ┌───────────┐   ┌──────────┐
     │Overseer  │   │Memory     │   │Budget    │
     │Cluster   │   │Repository │   │Tracker   │
     └──────────┘   └───────────┘   └──────────┘
```

## Service Inventory

### SwarmOrchestrator

**Role:** Top-level coordinator. Wires all services, manages lifecycle.

**Dependencies:**
- GoalRepository, TaskRepository, WorktreeRepository, AgentRepository, MemoryRepository
- EventBus, EventReactor, EventScheduler
- TaskService, GoalService, WorktreeService
- WorkflowEngine, ConvergenceEngine
- Overmind, MetaPlanner, LlmPlanner
- Guardrails, CircuitBreaker, BudgetTracker, CostTracker
- IntegrationVerifier
- SubstrateRegistry
- AdapterRegistry

**Shared state:**
- `Arc<RwLock<OrchestratorStatus>>` — running/paused/stopped
- `Arc<RwLock<SwarmStats>>` — aggregate statistics
- `Arc<RwLock<Vec<Goal>>>` — active goals cache
- `Arc<Semaphore>` — agent slot limiter

**Concurrency:** Single instance. All mutable state behind `Arc<RwLock<>>`.

### TaskService

**Role:** Task lifecycle management (create, claim, complete, fail, retry).

**Dependencies:**
- `TaskRepository` (generic, via trait)
- `EventBus` (for publishing events)
- `SpawnLimitConfig` (for subtask limits)

**Shared state:** None owned. Repository handles persistence.

**Concurrency:** Stateless service. All concurrency handled by repository's
optimistic locking. Multiple concurrent calls are safe.

### WorkflowEngine

**Role:** Deterministic workflow state machine.

**Dependencies:**
- `TaskRepository` (reads/writes workflow state in task context)
- `EventBus` (event emission)
- `HashMap<String, WorkflowTemplate>` (builtin templates, immutable after init)

**Shared state:** None. Workflow state lives in task context JSON.

**Concurrency:** State mutations go through `TaskRepository::update()` with
optimistic locking. Concurrent `advance()` calls are guarded by
`all_subtasks_done()` check.

### ConvergenceEngine

**Role:** Iterative convergence loop.

**Dependencies:**
- `TrajectoryRepository` (trajectory persistence)
- `MemoryRepository` (convergence memory)
- `OverseerMeasurer` (artifact measurement)

**Shared state:** Per-trajectory state. No cross-trajectory sharing.

**Concurrency:** `max_parallel_trajectories` semaphore. Each trajectory
is independent.

### EventBus

**Role:** Broadcast pub/sub + persistence.

**Dependencies:**
- `EventStore` (optional, for persistence)

**Shared state:**
- `AtomicU64` — sequence counter (lock-free)
- `broadcast::Sender` — subscriber list (Tokio-managed)
- `Arc<RwLock<Option<Uuid>>>` — correlation context

**Concurrency:** Lock-free sequence assignment. Broadcast is thread-safe.

### EventReactor

**Role:** Dispatches events to handlers.

**Dependencies:**
- `EventBus` (subscribes for events)
- `EventStore` (watermark tracking)
- All registered `EventHandler` implementations

**Shared state:**
- `Arc<RwLock<Vec<Arc<dyn EventHandler>>>>` — handler registry
- `Arc<RwLock<HashMap<String, CircuitBreakerState>>>` — per-handler breakers
- `Arc<RwLock<VecDeque<EventId>>>` — dedup buffer

**Concurrency:** Handlers execute sequentially within priority bands.
Circuit breakers prevent cascading failures.

### Overmind

**Role:** Strategic LLM-based decision making.

**Dependencies:**
- `Substrate` (LLM execution)

**Shared state:**
- `Arc<Semaphore>` — max 2 concurrent invocations

**Concurrency:** Semaphore-limited. Retry with timeout (120s default, 2 retries).

### Guardrails

**Role:** Safety constraints and runtime limits.

**Dependencies:** None (self-contained).

**Shared state:**
- `AtomicU64` — tokens_used_this_hour, total_tokens, tasks_started, etc.
- `AtomicU64` — cost_hundredths (CAS-based)

**Concurrency:** All metrics use atomics. CAS loop for cost updates.

### BudgetTracker

**Role:** Token budget pressure tracking.

**Dependencies:**
- `EventBus` (emits pressure change events)

**Shared state:**
- `Arc<RwLock<BudgetState>>` — current pressure level and windows

**Concurrency:** RwLock for state. Reads are frequent, writes on budget signals.

### CircuitBreaker

**Role:** Failure detection and isolation.

**Configuration presets:**
| Preset | Failures to Open | Open Timeout | Successes to Close |
|--------|-------------------|--------------|---------------------|
| default | 5 | 5 min | 2 |
| sensitive | 3 | 2 min | 1 |
| resilient | 10 | 10 min | 3 |

**Scopes:** `TaskChain(Uuid)`, `Agent(String)`, `Operation(String)`, `Global`

**States:** `Closed` → `Open` (blocking) → `HalfOpen` (testing) → `Closed`

### GoalService

**Role:** Goal lifecycle management.

**Dependencies:**
- `GoalRepository`
- `EventBus`

**Shared state:** None owned.

### WorktreeService

**Role:** Git worktree lifecycle.

**Dependencies:**
- `WorktreeRepository`
- Git CLI (subprocess)

**Worktree states:** `Creating → Active → Completed → Merging → Merged`

### IntegrationVerifier

**Role:** Post-task verification (tests, lint, format, commits).

**Dependencies:**
- `TaskRepository`, `GoalRepository`, `WorktreeRepository`
- Shell commands (test runners, linters)

**Configuration:**
| Check | Default |
|-------|---------|
| `run_tests` | true |
| `run_lint` | true |
| `check_format` | true |
| `require_commits` | true |
| `test_timeout_secs` | 300 |
| `fail_on_warnings` | false |

### CostTracker

**Role:** LLM cost estimation and accounting.

**Model pricing (per 1M tokens):**
| Model | Input | Output | Cache Read | Cache Write |
|-------|-------|--------|------------|-------------|
| claude-opus-4-6 | $15 | $75 | $1.50 | $18.75 |
| claude-sonnet-4-5 | $3 | $15 | $0.30 | $3.75 |
| claude-haiku-4-5 | $0.80 | $4 | $0.08 | $1.00 |

### ColdStart

**Role:** Project analysis on first run.

Scans project structure, detects conventions, extracts dependencies,
populates memory. Supports: Rust, Node, Python, Go, Java, Mixed.

### MetaPlanner / LlmPlanner

**Role:** Goal decomposition into task DAGs.

**MetaPlanner** orchestrates decomposition:
- Max depth: 3 levels
- Max tasks per decomposition: 10
- Can use LLM decomposition or heuristic

**LlmPlanner** calls Claude for decomposition:
- Modes: Claude Code CLI or direct Anthropic API
- Default model: claude-opus-4-6
- Temperature: 0.3

## Shared State Summary

| State | Owner | Type | Access Pattern |
|-------|-------|------|----------------|
| Orchestrator status | SwarmOrchestrator | `Arc<RwLock<OrchestratorStatus>>` | Rare writes, frequent reads |
| Swarm stats | SwarmOrchestrator | `Arc<RwLock<SwarmStats>>` | Periodic writes, frequent reads |
| Active goals cache | SwarmOrchestrator | `Arc<RwLock<Vec<Goal>>>` | Event-driven refresh |
| Agent slots | SwarmOrchestrator | `Arc<Semaphore>` | Acquire on spawn, release on complete |
| Event sequence | EventBus | `AtomicU64` | Lock-free increment |
| Runtime metrics | Guardrails | `AtomicU64` (multiple) | Lock-free CAS |
| Budget state | BudgetTracker | `Arc<RwLock<BudgetState>>` | Write on signal, read on dispatch |
| Overmind slots | Overmind | `Arc<Semaphore>` | Max 2 concurrent |
| Handler registry | EventReactor | `Arc<RwLock<Vec>>` | Write once at startup |
| Circuit breakers | EventReactor | `Arc<RwLock<HashMap>>` | Per-handler failure tracking |
| Task version | TaskRepository | `u64` column + `VersionTag` | Optimistic locking |

## Concurrency Boundaries

### Database Layer
- All repository operations are single-transaction atomic (`exec_tx!` macro)
- Multi-step operations across repositories are **NOT** atomic
- Idempotent event handlers compensate for partial failures

### Event Processing
- Events are processed sequentially within priority bands
- Handlers at the same priority level execute in registration order
- Cross-handler state mutations use optimistic locking (retry on conflict)

### Task Claiming
- `claim_task_atomic()` is a single SQL UPDATE with WHERE clause
- Prevents TOCTOU race between checking Ready and transitioning to Running
- Returns `None` if another agent won the race

### Agent Execution
- Each agent runs in its own tokio task
- Agents are isolated via git worktrees (filesystem-level isolation)
- Agent slot semaphore prevents exceeding `max_concurrent_agents`

## Command Bus

The `CommandBus` provides a unified dispatch point:

| Source | Description |
|--------|-------------|
| `Human` | CLI/external user |
| `System` | Internal orchestrator |
| `EventHandler(name)` | Reactive event handler |
| `Scheduler(name)` | Scheduled task |
| `A2A(id)` | Federated delegation |
| `Webhook(id)` | Webhook trigger |
| `Mcp(id)` | MCP HTTP server |
| `Adapter(name)` | External adapter |

Commands are dispatched through typed handlers. Each handler:
1. Validates the command
2. Executes the mutation
3. Returns events to be published

The command bus journals events and broadcasts them via the EventBus.
