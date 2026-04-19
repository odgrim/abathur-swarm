//! EventBus service for unified event streaming and distribution.
//!
//! Provides a broadcast-based event system with sequence numbering,
//! optional persistence, and correlation tracking.
//!
//! # `EventPayload` Variant Organization
//!
//! `EventPayload` is a large flat enum (~175 variants). For ease of navigation,
//! variants are grouped below into sections delimited by `// ====` banners,
//! matching the canonical [`EventCategory`] values. A mini table of contents:
//!
//! - **Orchestrator** — lifecycle, reconciliation, handler/error infra, trigger rules, subsystem errors
//! - **Goal** — goal lifecycle, status changes, constraints, domains, descriptions
//! - **Task** — task lifecycle, SLAs, stale-task warnings, worktrees, PRs, merges, execution-recorded
//! - **Execution** — DAG execution lifecycle, waves, restructure decisions
//! - **Agent** — agent creation, templates, instance lifecycle, specialist spawns, evolution
//! - **Verification** — intent/wave/branch verification, task alignment, `TaskVerified`
//! - **Escalation** — human escalation and responses (blocking and non-blocking)
//! - **Memory** — memory CRUD, conflicts, pruning, daemon maintenance
//! - **Scheduler** — scheduled events, quiet-window enter/exit
//! - **Convergence** — convergence loop iterations, attractor transitions, fresh starts, termination
//! - **Workflow** — state-machine workflow phases, gates, retries
//! - **Adapter** — adapter ingestion and egress
//! - **Budget** — budget pressure and opportunity
//! - **Federation** — cerebrate connectivity, task delegation, federated goals, swarm DAGs
//!
//! # Named Payload Structs
//!
//! Variants with 6+ named fields wrap a `…Payload` struct (e.g. `TaskResultPayload`,
//! `HumanEscalationPayload`) defined alongside the enum. Smaller variants stay
//! as inline-fields tuple/struct variants. This keeps the enum flat (so that
//! the ~500+ match arms across the codebase remain simple) while reducing the
//! worst-case variant girth.

mod conversions;
mod payload;
#[cfg(test)]
mod tests;
mod types;

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::sync::{RwLock, broadcast};
use uuid::Uuid;

use super::dag_executor::ExecutionEvent;
use super::event_store::EventStore;
use super::swarm_orchestrator::SwarmEvent;

// Re-exports: preserve the crate-wide public surface of `event_bus`.
pub use conversions::{
    ExecutionResultsPayload, ExecutionStatusPayload, SwarmStatsPayload, TaskResultPayload,
};
pub use payload::{
    ConvergenceIterationPayload, ConvergenceTerminatedPayload, EventPayload,
    HumanEscalationPayload, IntentVerificationCompletedPayload,
    WorkflowVerificationCompletedPayload,
};
pub use types::{
    BudgetPressureLevel, EventCategory, EventId, EventSeverity, SequenceNumber, UnifiedEvent,
};

/// Configuration for the EventBus.
#[derive(Debug, Clone)]
pub struct EventBusConfig {
    /// Channel capacity for the broadcast channel.
    pub channel_capacity: usize,
    /// Whether to persist events to storage.
    ///
    /// Note: Task and Workflow category events are always persisted regardless
    /// of this setting, as they are state-bearing and their loss causes
    /// correctness issues.
    pub persist_events: bool,
}

impl Default for EventBusConfig {
    fn default() -> Self {
        Self {
            channel_capacity: 1024,
            persist_events: true,
        }
    }
}

/// Central event bus for broadcasting events to multiple consumers.
pub struct EventBus {
    sender: broadcast::Sender<UnifiedEvent>,
    sequence: AtomicU64,
    store: Option<Arc<dyn EventStore>>,
    correlation_context: Arc<RwLock<Option<Uuid>>>,
    config: EventBusConfig,
    /// Unique ID for this EventBus instance (process). Used to identify
    /// events originating from this process for cross-process dedup.
    process_id: Uuid,
    /// Counter of events dropped due to broadcast channel being full or having no receivers.
    dropped_count: AtomicU64,
}

impl EventBus {
    /// Create a new EventBus with the given configuration.
    pub fn new(config: EventBusConfig) -> Self {
        let (sender, _) = broadcast::channel(config.channel_capacity);
        Self {
            sender,
            sequence: AtomicU64::new(0),
            store: None,
            correlation_context: Arc::new(RwLock::new(None)),
            config,
            process_id: Uuid::new_v4(),
            dropped_count: AtomicU64::new(0),
        }
    }

    /// Add an event store for persistence.
    pub fn with_store(mut self, store: Arc<dyn EventStore>) -> Self {
        self.store = Some(store);
        self
    }

    /// Publish a unified event.
    pub async fn publish(&self, mut event: UnifiedEvent) {
        // Assign sequence number
        let seq = self.sequence.fetch_add(1, Ordering::SeqCst);
        event.sequence = SequenceNumber(seq);

        // Stamp with this process's ID if not already set
        if event.source_process_id.is_none() {
            event.source_process_id = Some(self.process_id);
        }

        // Add correlation ID from context if not set
        if event.correlation_id.is_none() {
            let ctx = self.correlation_context.read().await;
            event.correlation_id = *ctx;
        }

        #[cfg(debug_assertions)]
        if let Some(expected) = event.payload.expected_category() {
            debug_assert_eq!(
                event.category,
                expected,
                "EventBus: category mismatch for payload {}: expected {:?}, got {:?}",
                event.payload.variant_name(),
                expected,
                event.category
            );
        }

        #[cfg(not(debug_assertions))]
        if let Some(expected) = event.payload.expected_category() {
            if event.category != expected {
                tracing::warn!(
                    "EventBus: category mismatch for payload {}: expected {:?}, got {:?}",
                    event.payload.variant_name(),
                    expected,
                    event.category
                );
            }
        }

        // Determine whether to persist: always persist Task and Workflow category
        // events (state-bearing, loss causes correctness issues), otherwise honor config.
        let should_persist = self.config.persist_events
            || matches!(
                event.category,
                EventCategory::Task | EventCategory::Workflow
            );

        if should_persist
            && let Some(ref store) = self.store
            && let Err(e) = store.append(&event).await
        {
            let err_msg = e.to_string();
            if err_msg.contains("UNIQUE constraint failed: events.sequence") {
                // Sequence collision with another process — re-sync counter and retry
                if let Ok(Some(latest)) = store.latest_sequence().await {
                    let new_seq = latest.0 + 1;
                    self.sequence.store(new_seq + 1, Ordering::SeqCst);
                    event.sequence = SequenceNumber(new_seq);
                    if let Err(e2) = store.append(&event).await {
                        tracing::warn!("Failed to persist event after sequence re-sync: {}", e2);
                    }
                }
            } else {
                tracing::warn!("Failed to persist event: {}", e);
            }
        }

        // Broadcast to subscribers, tracking drops
        if let Err(e) = self.sender.send(event) {
            self.dropped_count.fetch_add(1, Ordering::Relaxed);
            if self.sender.receiver_count() == 0 {
                // No receivers is normal in CLI mode — log at debug, not warn
                tracing::debug!("EventBus: dropped event (no receivers): {}", e);
            } else {
                tracing::warn!("EventBus: dropped event (receivers lagged): {}", e);
            }
        }
    }

    /// Publish a SwarmEvent (converts to UnifiedEvent).
    ///
    /// **Deprecated**: Prefer constructing `UnifiedEvent` directly using
    /// `event_factory::make_event()` or dispatching through the `CommandBus`.
    #[deprecated(note = "Use event_factory::make_event() or dispatch through CommandBus")]
    pub async fn publish_swarm_event(&self, event: SwarmEvent) {
        self.publish(event.into()).await;
    }

    /// Publish an ExecutionEvent (converts to UnifiedEvent).
    ///
    /// **Deprecated**: Prefer constructing `UnifiedEvent` directly using
    /// `event_factory::make_event()` or dispatching through the `CommandBus`.
    #[deprecated(note = "Use event_factory::make_event() or dispatch through CommandBus")]
    pub async fn publish_execution_event(&self, event: ExecutionEvent) {
        self.publish(event.into()).await;
    }

    /// Subscribe to the event stream.
    pub fn subscribe(&self) -> broadcast::Receiver<UnifiedEvent> {
        self.sender.subscribe()
    }

    /// Get the current sequence number.
    pub fn current_sequence(&self) -> SequenceNumber {
        SequenceNumber(self.sequence.load(Ordering::SeqCst))
    }

    /// Start a new correlation context for tracking related events.
    pub async fn start_correlation(&self) -> Uuid {
        let id = Uuid::new_v4();
        let mut ctx = self.correlation_context.write().await;
        *ctx = Some(id);
        id
    }

    /// End the current correlation context.
    pub async fn end_correlation(&self) {
        let mut ctx = self.correlation_context.write().await;
        *ctx = None;
    }

    /// Get the event store if configured.
    pub fn store(&self) -> Option<Arc<dyn EventStore>> {
        self.store.clone()
    }

    /// Get the unique process ID of this EventBus instance.
    pub fn process_id(&self) -> Uuid {
        self.process_id
    }

    /// Get the number of active subscribers.
    pub fn subscriber_count(&self) -> usize {
        self.sender.receiver_count()
    }

    /// Get the total number of events dropped since this EventBus was created.
    pub fn dropped_count(&self) -> u64 {
        self.dropped_count.load(Ordering::Relaxed)
    }

    /// Initialize the sequence counter from the event store.
    ///
    /// Reads the latest sequence number from the store and sets the atomic
    /// counter to `latest + 1` to prevent sequence overlap after restart.
    /// Must be called during startup before reactor/scheduler start.
    pub async fn initialize_sequence_from_store(&self) {
        if let Some(ref store) = self.store {
            match store.latest_sequence().await {
                Ok(Some(latest)) => {
                    self.sequence.store(latest.0 + 1, Ordering::SeqCst);
                    tracing::info!(
                        "EventBus: initialized sequence from store at {}",
                        latest.0 + 1
                    );
                }
                Ok(None) => {
                    // Empty store, start from 0
                }
                Err(e) => {
                    tracing::warn!("EventBus: failed to read latest sequence from store: {}", e);
                }
            }
        }
    }
}
