# Event Bus Contract

> `src/services/event_bus.rs`, `src/services/event_reactor.rs`,
> `src/services/event_store.rs`, `src/services/event_factory.rs`,
> `src/services/event_scheduler.rs`

## Architecture

The event bus is a **broadcast-based pub/sub system** built on
`tokio::sync::broadcast::channel<UnifiedEvent>`. Every published event is
delivered to all active subscribers. Events are optionally persisted to an
`EventStore` for replay and audit.

```
Producer ──publish()──► EventBus ──broadcast──► Subscriber (EventReactor)
                           │                         │
                           ▼                         ▼
                       EventStore              EventHandler.handle()
                       (SQLite)                      │
                                                     ▼
                                              Reaction::EmitEvents
                                                     │
                                              re-publish (chain)
```

## Event Envelope: `UnifiedEvent`

Every event is wrapped in a `UnifiedEvent` with metadata:

| Field | Type | Description |
|-------|------|-------------|
| `id` | `EventId(Uuid)` | Unique event identifier |
| `sequence` | `SequenceNumber(u64)` | Monotonically increasing, auto-assigned by EventBus |
| `timestamp` | `DateTime<Utc>` | Creation time |
| `severity` | `EventSeverity` | `Debug`, `Info`, `Warning`, `Error`, `Critical` |
| `category` | `EventCategory` | One of 14 categories (see below) |
| `goal_id` | `Option<Uuid>` | Associated goal |
| `task_id` | `Option<Uuid>` | Associated task |
| `correlation_id` | `Option<Uuid>` | Causal chain tracking |
| `source_process_id` | `Option<Uuid>` | Cross-process deduplication |
| `payload` | `EventPayload` | The actual event data (120+ variants) |

### Event Categories

```
Orchestrator, Goal, Task, Execution, Agent, Verification,
Escalation, Memory, Scheduler, Convergence, Workflow, Adapter, Budget, Federation
```

### Event Severity

```
Debug < Info < Warning < Error < Critical
```

## `EventBus` Contract

### `publish(event: UnifiedEvent)`

**Preconditions:**
- `event.payload` is a valid `EventPayload` variant.
- The bus has been initialized (constructor called).

**Postconditions:**
- `event.sequence` is set to the next monotonic value.
- `event.source_process_id` is set to this process's UUID.
- If `persist_events` is true and a store is configured, the event is
  appended to the `EventStore`. Sequence collisions are retried
  (re-sync from `latest_sequence()` + retry).
- The event is broadcast to all active subscribers.
- If no subscribers are listening, the event is still persisted but
  the broadcast is a no-op.

**Errors:**
- Persistence failures are logged but do **not** prevent broadcast.
- Broadcast failures (all receivers dropped) are logged.

**Concurrency:**
- Sequence assignment uses `AtomicU64::fetch_add` — lock-free.
- Multiple concurrent `publish()` calls are safe; sequence order matches
  `fetch_add` order (not wall-clock order).

### `subscribe() -> broadcast::Receiver<UnifiedEvent>`

**Postconditions:**
- Returns a new receiver. Events published *after* this call are delivered.
- Late subscribers miss events published before subscription.
  Use `EventStore::replay_since()` for catch-up.

**Concurrency:**
- Thread-safe. Can be called from any task.

### Configuration: `EventBusConfig`

| Field | Default | Description |
|-------|---------|-------------|
| `channel_capacity` | 1024 | Broadcast channel buffer size |
| `persist_events` | true | Whether to write to EventStore |

When the channel buffer is full, **slow subscribers are dropped** (Tokio
`broadcast` lagged behavior). The `dropped_count` counter tracks this.

## Event Factory

`src/services/event_factory.rs` provides typed constructors:

```rust
task_event(severity, goal_id, task_id, payload) -> UnifiedEvent
goal_event(severity, goal_id, payload) -> UnifiedEvent
memory_event(severity, payload) -> UnifiedEvent
workflow_event(severity, task_id, payload) -> UnifiedEvent
// ... etc.
```

**Contract:** Always use factory functions to construct events. They ensure
the `category` field matches the payload type. Hand-constructing a
`UnifiedEvent` with mismatched category/payload is a contract violation.

## Event Reactor

`src/services/event_reactor.rs` dispatches events to registered handlers.

### Handler Dispatch Order

1. Filter handlers whose `matches()` returns true for the event.
2. Sort by priority: `System` < `High` < `Normal` < `Low`.
3. Execute sequentially within each priority band.

### Handler Contract

Each handler implements:

```rust
trait EventHandler: Send + Sync {
    fn metadata(&self) -> HandlerMetadata;
    fn matches(&self, event: &UnifiedEvent) -> bool;
    async fn handle(&self, event: &UnifiedEvent, ctx: &HandlerContext) -> HandlerResult;
}
```

`HandlerResult` can be:
- `Ok(Reaction::None)` — no side effects
- `Ok(Reaction::EmitEvents(Vec<UnifiedEvent>))` — re-publish events (chain)
- `Err(HandlerError)` — logged, dead-lettered, circuit breaker updated

### Safety Mechanisms

| Mechanism | Purpose |
|-----------|---------|
| **Max chain depth** (default: 32) | Prevents infinite event→handler→event recursion |
| **Circuit breakers** (per-handler) | Trips after N failures; cooldown period before retry |
| **Dedup** (`VecDeque<EventId>`) | Prevents duplicate handler invocations for same event |
| **Watermarks** (per-handler sequence) | Tracks last processed event for crash recovery |
| **Dead letter queue** | Failed handler invocations stored for retry with backoff |
| **Critical handler backoff** | Exponential retry (1s→2s→4s→8s→16s cap) for critical handlers |

### Chain Depth Violation

If a handler emits events that trigger further handlers and the chain
exceeds `max_chain_depth`:

- The chain is **aborted**.
- A `HandlerError` event is published.
- The offending handler's circuit breaker is notified.

**Caller responsibility:** Handlers that emit events must ensure they do
not create unbounded chains. Typical pattern: handler A emits event X;
handler B handles X but does not re-emit events that would trigger A.

## Event Persistence: `EventStore`

```rust
trait EventStore: Send + Sync {
    async fn append(&self, event: &UnifiedEvent) -> Result<(), EventStoreError>;
    async fn query(&self, query: EventQuery) -> Result<Vec<UnifiedEvent>, EventStoreError>;
    async fn latest_sequence(&self) -> Result<Option<SequenceNumber>, EventStoreError>;
    async fn prune_older_than(&self, duration: Duration) -> Result<u64, EventStoreError>;
    async fn replay_since(&self, seq: SequenceNumber) -> Result<Vec<UnifiedEvent>, EventStoreError>;
    async fn get_watermark(&self, handler: &str) -> Result<Option<SequenceNumber>, EventStoreError>;
    async fn set_watermark(&self, handler: &str, seq: SequenceNumber) -> Result<(), EventStoreError>;
}
```

**Sequence collision handling:** On UNIQUE constraint failure during
`append()`, the bus re-syncs its sequence counter from `latest_sequence()`
and retries with a new sequence number.

## Event Scheduling

`src/services/event_scheduler.rs` fires time-based events.

### Schedule Types

| Type | Description |
|------|-------------|
| `Once { at }` | Fire once at a specific `DateTime<Utc>` |
| `Interval { every }` | Fire every `Duration` |
| `Cron { expression }` | Fire on cron schedule |

### Lifecycle

1. `register(ScheduledEvent)` → persists to DB, returns `Option<Uuid>`
2. `start()` spawns a tokio task that ticks every `tick_interval_ms`
3. On match: publishes `ScheduledEventFired { schedule_id, name }`
4. `cancel(id)` sets `active = false`

**Contract:** Handlers that react to `ScheduledEventFired` must be
idempotent — the scheduler guarantees at-least-once delivery but may
double-fire on crash recovery.

### Builtin Schedules

Registered at orchestrator startup:
- Memory maintenance
- Event pruning
- Reconciliation
- Stats updates
- Adapter polling (ingestion)
- Federation heartbeat checks
- SLA enforcement
- Worktree reconciliation

## HTTP/WebSocket Streaming

`src/adapters/mcp/events_http.rs` exposes events externally:

| Endpoint | Protocol | Features |
|----------|----------|----------|
| `GET /events` | SSE | All events, `Last-Event-ID` replay, 30s heartbeat |
| `GET /events/goals/{id}` | SSE | Filtered to goal |
| `GET /events/tasks/{id}` | SSE | Filtered to task |
| `GET /events/replay?since=N` | HTTP | Historical replay |
| `GET /events/history?...` | HTTP | Complex query filters |
| `GET /ws/events` | WebSocket | Category filtering, since-sequence replay |
| `POST /api/v1/webhooks` | HTTP | Register webhook subscription |
