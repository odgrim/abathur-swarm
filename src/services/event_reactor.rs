//! Reactive event handler system.
//!
//! The EventReactor subscribes to the EventBus and dispatches events to
//! matching handlers. Handlers can produce reactions (new events) that
//! are fed back into the EventBus, enabling reactive event chains.
//!
//! Safety mechanisms prevent runaway chains: max chain depth, per-handler
//! circuit breakers, rate limiting, and dedup.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use tokio::sync::RwLock;
use uuid::Uuid;

use super::event_bus::{
    EventBus, EventCategory, EventId, EventPayload, EventSeverity, SequenceNumber, UnifiedEvent,
};
use super::event_store::EventStore;
use super::supervise_with_handle;

/// Unique identifier for a registered handler.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct HandlerId(pub Uuid);

impl HandlerId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for HandlerId {
    fn default() -> Self {
        Self::new()
    }
}

/// Thread-safe predicate for custom event filtering.
pub type EventPredicate = Arc<dyn Fn(&UnifiedEvent) -> bool + Send + Sync>;

/// Filter that determines which events a handler matches.
#[derive(Default)]
pub struct EventFilter {
    /// Match events in these categories (empty = match all).
    pub categories: Vec<EventCategory>,
    /// Match events at or above this severity.
    pub min_severity: Option<EventSeverity>,
    /// Match events for a specific goal.
    pub goal_id: Option<Uuid>,
    /// Match events for a specific task.
    pub task_id: Option<Uuid>,
    /// Match events whose payload variant name is in this list (empty = match all).
    pub payload_types: Vec<String>,
    /// Custom predicate for advanced filtering.
    pub custom_predicate: Option<EventPredicate>,
}

impl EventFilter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn categories(mut self, cats: Vec<EventCategory>) -> Self {
        self.categories = cats;
        self
    }

    pub fn min_severity(mut self, sev: EventSeverity) -> Self {
        self.min_severity = Some(sev);
        self
    }

    pub fn payload_types(mut self, types: Vec<String>) -> Self {
        self.payload_types = types;
        self
    }

    /// Check if an event matches this filter.
    pub fn matches(&self, event: &UnifiedEvent) -> bool {
        // Category filter
        if !self.categories.is_empty() && !self.categories.contains(&event.category) {
            return false;
        }

        // Severity filter
        if let Some(min_sev) = self.min_severity
            && severity_order(event.severity) < severity_order(min_sev)
        {
            return false;
        }

        // Goal filter
        if let Some(gid) = self.goal_id
            && event.goal_id != Some(gid)
        {
            return false;
        }

        // Task filter
        if let Some(tid) = self.task_id
            && event.task_id != Some(tid)
        {
            return false;
        }

        // Payload type filter
        if !self.payload_types.is_empty() {
            let variant = event.payload.variant_name();
            if !self.payload_types.iter().any(|t| t == variant) {
                return false;
            }
        }

        // Custom predicate
        if let Some(ref pred) = self.custom_predicate
            && !pred(event)
        {
            return false;
        }

        true
    }
}

fn severity_order(s: EventSeverity) -> u8 {
    match s {
        EventSeverity::Debug => 0,
        EventSeverity::Info => 1,
        EventSeverity::Warning => 2,
        EventSeverity::Error => 3,
        EventSeverity::Critical => 4,
    }
}

/// Priority ordering for handlers (lower value = executes first).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct HandlerPriority(pub u32);

impl HandlerPriority {
    pub const SYSTEM: Self = Self(0);
    pub const HIGH: Self = Self(100);
    pub const NORMAL: Self = Self(500);
    pub const LOW: Self = Self(1000);
}

impl Default for HandlerPriority {
    fn default() -> Self {
        Self::NORMAL
    }
}

/// What a handler can produce as a reaction.
pub enum Reaction {
    /// Emit new events into the EventBus.
    EmitEvents(Vec<UnifiedEvent>),
    /// No reaction.
    None,
}

/// Strategy for handling handler errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorStrategy {
    /// Log and continue.
    LogAndContinue,
    /// Trip circuit breaker after threshold.
    CircuitBreak,
}

impl Default for ErrorStrategy {
    fn default() -> Self {
        Self::CircuitBreak
    }
}

/// Metadata describing a handler.
pub struct HandlerMetadata {
    pub id: HandlerId,
    pub name: String,
    pub filter: EventFilter,
    pub priority: HandlerPriority,
    pub error_strategy: ErrorStrategy,
    /// Critical handlers are essential for system invariants (e.g. task lifecycle
    /// transitions). When their circuit breaker trips, they use aggressive retry
    /// with exponential backoff instead of the default flat cooldown.
    pub critical: bool,
}

/// Context passed to handlers during event processing.
pub struct HandlerContext {
    /// Current chain depth for this event cascade.
    pub chain_depth: u32,
    /// Correlation ID for tracking related events.
    pub correlation_id: Option<Uuid>,
}

/// Trait for reactive event handlers.
#[async_trait]
pub trait EventHandler: Send + Sync {
    /// Return metadata describing this handler.
    fn metadata(&self) -> HandlerMetadata;

    /// Handle an event and optionally produce a reaction.
    async fn handle(&self, event: &UnifiedEvent, ctx: &HandlerContext) -> Result<Reaction, String>;
}

/// Per-handler circuit breaker state.
struct CircuitBreakerState {
    failure_count: u32,
    last_failure: Option<Instant>,
    tripped: bool,
    tripped_at: Option<Instant>,
    /// Whether this handler is marked as critical.
    critical: bool,
    /// Current backoff attempt for critical handlers (used for exponential backoff).
    backoff_attempt: u32,
}

impl CircuitBreakerState {
    fn new() -> Self {
        Self {
            failure_count: 0,
            last_failure: None,
            tripped: false,
            tripped_at: None,
            critical: false,
            backoff_attempt: 0,
        }
    }

    fn record_failure(&mut self, threshold: u32, window: Duration) {
        let now = Instant::now();
        // Reset if outside the window
        if let Some(last) = self.last_failure
            && now.duration_since(last) > window
        {
            self.failure_count = 0;
        }
        self.failure_count += 1;
        self.last_failure = Some(now);

        if self.failure_count >= threshold {
            if !self.tripped {
                // First time tripping — start backoff from 0
                self.backoff_attempt = 0;
            }
            self.tripped = true;
            self.tripped_at = Some(now);
        }
    }

    fn is_tripped_with_config(
        &self,
        cooldown: Duration,
        critical_initial_secs: u64,
        critical_max_secs: u64,
    ) -> bool {
        if !self.tripped {
            return false;
        }
        let effective_cooldown =
            self.effective_cooldown_with_config(cooldown, critical_initial_secs, critical_max_secs);
        // Auto-reset after cooldown
        if let Some(tripped_at) = self.tripped_at
            && Instant::now().duration_since(tripped_at) > effective_cooldown
        {
            return false;
        }
        true
    }

    fn reset_if_cooled_with_config(
        &mut self,
        cooldown: Duration,
        critical_initial_secs: u64,
        critical_max_secs: u64,
    ) {
        if self.tripped
            && let Some(tripped_at) = self.tripped_at
        {
            let effective_cooldown = self.effective_cooldown_with_config(
                cooldown,
                critical_initial_secs,
                critical_max_secs,
            );
            if Instant::now().duration_since(tripped_at) > effective_cooldown {
                self.tripped = false;
                self.failure_count = 0;
                self.tripped_at = None;
                if self.critical {
                    // Increment backoff for next potential trip
                    self.backoff_attempt = self.backoff_attempt.saturating_add(1);
                }
            }
        }
    }

    /// Reset backoff on successful execution (critical handlers only).
    fn reset_backoff(&mut self) {
        self.backoff_attempt = 0;
    }

    /// Calculate effective cooldown: critical handlers use exponential backoff
    /// (initial_secs * 2^attempt, capped at max_secs), non-critical use flat cooldown.
    // reason: thin wrapper around effective_cooldown_with_config used by unit
    // tests to exercise the default initial/max values; production code calls
    // effective_cooldown_with_config directly with config-supplied values.
    #[cfg(test)]
    fn effective_cooldown(&self, default_cooldown: Duration) -> Duration {
        if !self.critical {
            return default_cooldown;
        }
        // Critical handler exponential backoff: 1s, 2s, 4s, 8s, 16s (capped)
        // These defaults are overridden when called with config values via effective_cooldown_with_config
        let initial_secs = 1u64;
        let max_secs = 16u64;
        Self::compute_critical_cooldown(initial_secs, max_secs, self.backoff_attempt)
    }

    /// Calculate effective cooldown using explicit config values.
    fn effective_cooldown_with_config(
        &self,
        default_cooldown: Duration,
        critical_initial_secs: u64,
        critical_max_secs: u64,
    ) -> Duration {
        if !self.critical {
            return default_cooldown;
        }
        Self::compute_critical_cooldown(
            critical_initial_secs,
            critical_max_secs,
            self.backoff_attempt,
        )
    }

    fn compute_critical_cooldown(initial_secs: u64, max_secs: u64, attempt: u32) -> Duration {
        let backoff_secs =
            initial_secs.saturating_mul(1u64.checked_shl(attempt).unwrap_or(u64::MAX));
        Duration::from_secs(backoff_secs.min(max_secs))
    }

    /// Atomic "may I allow this invocation?" — combines cooldown reset + tripped check
    /// under a single locked section so the decision cannot race with concurrent mutations.
    ///
    /// Returns true if the handler should be invoked, false if the CB is still tripped.
    fn try_allow(
        &mut self,
        cooldown: Duration,
        critical_initial_secs: u64,
        critical_max_secs: u64,
    ) -> bool {
        self.reset_if_cooled_with_config(cooldown, critical_initial_secs, critical_max_secs);
        !self.is_tripped_with_config(cooldown, critical_initial_secs, critical_max_secs)
    }

    /// Atomic "record a failure and tell me if we're tripped now (+ backoff attempt)".
    ///
    /// Combines record_failure + is_tripped_with_config + backoff_attempt read in a
    /// single locked section so callers observe a consistent post-failure state.
    fn record_failure_and_report(
        &mut self,
        threshold: u32,
        window: Duration,
        cooldown: Duration,
        critical_initial_secs: u64,
        critical_max_secs: u64,
    ) -> (bool, u32) {
        self.record_failure(threshold, window);
        let tripped =
            self.is_tripped_with_config(cooldown, critical_initial_secs, critical_max_secs);
        (tripped, self.backoff_attempt)
    }
}

/// Configuration for the EventReactor.
#[derive(Debug, Clone)]
pub struct ReactorConfig {
    /// Maximum depth of event chain cascades.
    pub max_chain_depth: u32,
    /// Maximum events processed per second (excess dropped from reactive processing).
    pub max_events_per_second: u32,
    /// Per-handler timeout in milliseconds.
    pub handler_timeout_ms: u64,
    /// Number of failures before a handler's circuit breaker trips.
    pub circuit_breaker_threshold: u32,
    /// Window in seconds for counting circuit breaker failures.
    pub circuit_breaker_window_secs: u64,
    /// Cooldown in seconds before a tripped circuit breaker auto-resets (non-critical handlers).
    pub circuit_breaker_cooldown_secs: u64,
    /// Initial cooldown in seconds for critical handler circuit breakers (exponential backoff base).
    pub critical_cooldown_initial_secs: u64,
    /// Maximum cooldown in seconds for critical handler circuit breakers (backoff cap).
    pub critical_cooldown_max_secs: u64,
    /// Maximum size of the dedup set (LRU of recent sequence numbers).
    pub dedup_set_capacity: usize,
    /// Maximum number of events to replay during startup catch-up.
    /// None means replay all missed events (unbounded).
    pub startup_max_replay_events: Option<usize>,
}

impl Default for ReactorConfig {
    fn default() -> Self {
        Self {
            max_chain_depth: 8,
            max_events_per_second: 500,
            handler_timeout_ms: 15000,
            circuit_breaker_threshold: 5,
            circuit_breaker_window_secs: 600, // 10 minutes
            circuit_breaker_cooldown_secs: 60,
            critical_cooldown_initial_secs: 1,
            critical_cooldown_max_secs: 16,
            dedup_set_capacity: 50_000,
            startup_max_replay_events: Some(10_000),
        }
    }
}

/// The reactive event dispatcher.
///
/// Subscribes to EventBus, dispatches matching events to registered handlers,
/// and processes reactions (cascaded events).
pub struct EventReactor {
    handlers: Arc<RwLock<Vec<Arc<dyn EventHandler>>>>,
    event_bus: Arc<EventBus>,
    config: ReactorConfig,
    running: Arc<AtomicBool>,
    events_processed: Arc<AtomicU64>,
    events_dropped: Arc<AtomicU64>,
    circuit_breakers: Arc<RwLock<HashMap<HandlerId, CircuitBreakerState>>>,
    event_store: Option<Arc<dyn EventStore>>,
    watermark_buffer: Arc<RwLock<HashMap<String, SequenceNumber>>>,
    last_watermark_flush: Arc<RwLock<Instant>>,
    watermark_event_count: Arc<AtomicU64>,
}

impl EventReactor {
    /// Create a new EventReactor.
    pub fn new(event_bus: Arc<EventBus>, config: ReactorConfig) -> Self {
        Self {
            handlers: Arc::new(RwLock::new(Vec::new())),
            event_bus,
            config,
            running: Arc::new(AtomicBool::new(false)),
            events_processed: Arc::new(AtomicU64::new(0)),
            events_dropped: Arc::new(AtomicU64::new(0)),
            circuit_breakers: Arc::new(RwLock::new(HashMap::new())),
            event_store: None,
            watermark_buffer: Arc::new(RwLock::new(HashMap::new())),
            last_watermark_flush: Arc::new(RwLock::new(Instant::now())),
            watermark_event_count: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Add an event store for watermark tracking and replay.
    pub fn with_store(mut self, store: Arc<dyn EventStore>) -> Self {
        self.event_store = Some(store);
        self
    }

    /// Register a handler.
    pub async fn register(&self, handler: Arc<dyn EventHandler>) {
        let meta = handler.metadata();
        {
            let mut cbs = self.circuit_breakers.write().await;
            let mut cb_state = CircuitBreakerState::new();
            cb_state.critical = meta.critical;
            cbs.insert(meta.id, cb_state);
        }
        let mut handlers = self.handlers.write().await;
        handlers.push(handler);
        // Sort by priority (lower value first)
        handlers.sort_by_key(|h| h.metadata().priority);
    }

    /// Start the reactor event loop. Returns a JoinHandle that can be aborted on shutdown.
    pub fn start(&self) -> tokio::task::JoinHandle<()> {
        self.running.store(true, Ordering::SeqCst);

        let handlers = self.handlers.clone();
        let event_bus = self.event_bus.clone();
        let config = self.config.clone();
        let running = self.running.clone();
        let events_processed = self.events_processed.clone();
        let events_dropped = self.events_dropped.clone();
        let circuit_breakers = self.circuit_breakers.clone();
        let event_store = self.event_store.clone();
        let watermark_buffer = self.watermark_buffer.clone();
        let last_watermark_flush = self.last_watermark_flush.clone();
        let watermark_event_count = self.watermark_event_count.clone();

        supervise_with_handle("event_reactor_dispatch", async move {
            let mut receiver = event_bus.subscribe();
            // Rate limiting state
            let mut rate_window_start = Instant::now();
            let mut rate_count: u32 = 0;
            // Dedup set (sequence number -> seen)
            let mut dedup: std::collections::VecDeque<u64> = std::collections::VecDeque::new();
            // Chain depth tracking: correlation_id -> depth
            let mut chain_depths: HashMap<Uuid, u32> = HashMap::new();
            // Track last successfully processed sequence for lag recovery
            let mut last_processed_sequence: u64 = 0;

            while running.load(Ordering::SeqCst) {
                let event = match tokio::time::timeout(Duration::from_secs(1), receiver.recv())
                    .await
                {
                    Ok(Ok(event)) => event,
                    Ok(Err(tokio::sync::broadcast::error::RecvError::Lagged(n))) => {
                        tracing::warn!(
                            "EventReactor lagged, missed {} events - triggering catchup",
                            n
                        );
                        // Recover missed events from the journal
                        if let Some(ref store) = event_store {
                            match store
                                .replay_since(SequenceNumber(last_processed_sequence))
                                .await
                            {
                                Ok(missed) => {
                                    tracing::info!(
                                        "EventReactor: recovering {} events from journal",
                                        missed.len()
                                    );
                                    let hs = handlers.read().await;
                                    for missed_event in &missed {
                                        if dedup.contains(&missed_event.sequence.0) {
                                            continue;
                                        }
                                        for handler in hs.iter() {
                                            let meta = handler.metadata();
                                            if !meta.filter.matches(missed_event) {
                                                continue;
                                            }
                                            let ctx = HandlerContext {
                                                chain_depth: 0,
                                                correlation_id: missed_event.correlation_id,
                                            };
                                            let _ = tokio::time::timeout(
                                                Duration::from_millis(config.handler_timeout_ms),
                                                handler.handle(missed_event, &ctx),
                                            )
                                            .await;
                                        }
                                        // Advance watermarks for all handlers (processed or filter-skipped)
                                        {
                                            let mut wm_buf = watermark_buffer.write().await;
                                            for handler in hs.iter() {
                                                wm_buf.insert(
                                                    handler.metadata().name.clone(),
                                                    missed_event.sequence,
                                                );
                                            }
                                        }
                                        if missed_event.sequence.0 > last_processed_sequence {
                                            last_processed_sequence = missed_event.sequence.0;
                                        }
                                        dedup.push_back(missed_event.sequence.0);
                                        if dedup.len() > config.dedup_set_capacity {
                                            dedup.pop_front();
                                        }
                                    }
                                }
                                Err(e) => {
                                    tracing::error!(
                                        "EventReactor: failed to recover from lag: {}",
                                        e
                                    );
                                }
                            }
                        }
                        continue;
                    }
                    Ok(Err(tokio::sync::broadcast::error::RecvError::Closed)) => {
                        tracing::info!("EventReactor: EventBus channel closed, stopping");
                        break;
                    }
                    Err(_) => {
                        // Timeout - just loop to check running flag
                        continue;
                    }
                };

                // Rate limiting
                let now = Instant::now();
                if now.duration_since(rate_window_start) >= Duration::from_secs(1) {
                    rate_window_start = now;
                    rate_count = 0;
                }
                rate_count += 1;
                if rate_count > config.max_events_per_second {
                    events_dropped.fetch_add(1, Ordering::Relaxed);
                    continue;
                }

                // Dedup check
                let seq = event.sequence.0;
                if dedup.contains(&seq) {
                    continue;
                }
                dedup.push_back(seq);
                if dedup.len() > config.dedup_set_capacity {
                    dedup.pop_front();
                }

                // Chain depth check
                let chain_depth = if let Some(corr_id) = event.correlation_id {
                    let depth = chain_depths.entry(corr_id).or_insert(0);
                    *depth += 1;
                    let d = *depth;
                    // Clean up old entries periodically
                    if chain_depths.len() > 1000 {
                        chain_depths.retain(|_, v| *v < config.max_chain_depth);
                    }
                    d
                } else {
                    0
                };

                let suppress_reactions = chain_depth > config.max_chain_depth;
                if suppress_reactions {
                    tracing::warn!(
                        "EventReactor: chain depth {} exceeds max {} for correlation {:?}, suppressing reactions",
                        chain_depth,
                        config.max_chain_depth,
                        event.correlation_id
                    );
                }

                // Dispatch to matching handlers
                let handlers_snapshot = handlers.read().await;
                let mut reactions: Vec<UnifiedEvent> = Vec::new();
                // Track handlers that successfully processed this event (for watermark updates)
                let mut successful_handlers: Vec<String> = Vec::new();
                // Track handlers that skipped this event due to filter mismatch
                // (their watermarks should still advance — a replayed event would be skipped again)
                let mut skipped_handlers: Vec<String> = Vec::new();

                for handler in handlers_snapshot.iter() {
                    let meta = handler.metadata();

                    // Check circuit breaker — combined reset+tripped check under one lock
                    // section to avoid torn reads between concurrent dispatches.
                    {
                        let mut cbs = circuit_breakers.write().await;
                        if let Some(cb) = cbs.get_mut(&meta.id)
                            && !cb.try_allow(
                                Duration::from_secs(config.circuit_breaker_cooldown_secs),
                                config.critical_cooldown_initial_secs,
                                config.critical_cooldown_max_secs,
                            )
                        {
                            continue;
                        }
                    }

                    // Check filter match
                    if !meta.filter.matches(&event) {
                        skipped_handlers.push(meta.name.clone());
                        continue;
                    }

                    let ctx = HandlerContext {
                        chain_depth,
                        correlation_id: event.correlation_id,
                    };

                    // Metrics: handler dispatch. Handler names are
                    // static strings registered at startup — bounded cardinality.
                    metrics::counter!(
                        "abathur_handler_invocations_total",
                        "handler" => meta.name.clone()
                    )
                    .increment(1);
                    let handler_start = std::time::Instant::now();

                    // Execute with timeout
                    let result = tokio::time::timeout(
                        Duration::from_millis(config.handler_timeout_ms),
                        handler.handle(&event, &ctx),
                    )
                    .await;

                    metrics::histogram!(
                        "abathur_handler_duration_seconds",
                        "handler" => meta.name.clone()
                    )
                    .record(handler_start.elapsed().as_secs_f64());
                    if !matches!(result, Ok(Ok(_))) {
                        metrics::counter!(
                            "abathur_handler_failures_total",
                            "handler" => meta.name.clone()
                        )
                        .increment(1);
                    }

                    match result {
                        Ok(Ok(Reaction::EmitEvents(events))) if !suppress_reactions => {
                            reactions.extend(events);
                            successful_handlers.push(meta.name.clone());
                            // Reset backoff on success for critical handlers
                            if meta.critical {
                                let mut cbs = circuit_breakers.write().await;
                                if let Some(cb) = cbs.get_mut(&meta.id) {
                                    cb.reset_backoff();
                                }
                            }
                        }
                        Ok(Ok(_)) => {
                            // Reaction::None or suppressed — still a successful invocation
                            successful_handlers.push(meta.name.clone());
                            // Reset backoff on success for critical handlers
                            if meta.critical {
                                let mut cbs = circuit_breakers.write().await;
                                if let Some(cb) = cbs.get_mut(&meta.id) {
                                    cb.reset_backoff();
                                }
                            }
                        }
                        Ok(Err(e)) => {
                            tracing::warn!("EventReactor: handler '{}' error: {}", meta.name, e);
                            // Write to dead letter queue
                            if let Some(ref store) = event_store
                                && let Err(dlq_err) = store
                                    .append_dead_letter(
                                        &event.id.0.to_string(),
                                        event.sequence.0,
                                        &meta.name,
                                        &e,
                                        3,
                                    )
                                    .await
                            {
                                tracing::warn!(
                                    "EventReactor: failed to write DLQ entry: {}",
                                    dlq_err
                                );
                            }
                            let mut tripped = false;
                            let mut backoff_attempt = 0u32;
                            if meta.error_strategy == ErrorStrategy::CircuitBreak {
                                // Atomic record-and-report under one lock section to avoid
                                // torn reads / double-trip / mis-reported backoff.
                                let mut cbs = circuit_breakers.write().await;
                                if let Some(cb) = cbs.get_mut(&meta.id) {
                                    let (t, b) = cb.record_failure_and_report(
                                        config.circuit_breaker_threshold,
                                        Duration::from_secs(config.circuit_breaker_window_secs),
                                        Duration::from_secs(config.circuit_breaker_cooldown_secs),
                                        config.critical_cooldown_initial_secs,
                                        config.critical_cooldown_max_secs,
                                    );
                                    tripped = t;
                                    backoff_attempt = b;
                                }
                            }
                            // Emit HandlerError event for monitoring
                            reactions.push(UnifiedEvent {
                                id: EventId::new(),
                                sequence: SequenceNumber(0),
                                timestamp: chrono::Utc::now(),
                                severity: if tripped {
                                    EventSeverity::Error
                                } else {
                                    EventSeverity::Warning
                                },
                                category: EventCategory::Orchestrator,
                                goal_id: None,
                                task_id: None,
                                correlation_id: event.correlation_id,
                                source_process_id: None,
                                payload: EventPayload::HandlerError {
                                    handler_name: meta.name.clone(),
                                    event_sequence: event.sequence.0,
                                    error: e.clone(),
                                    circuit_breaker_tripped: tripped,
                                },
                            });
                            // Emit CriticalHandlerDegraded when a critical handler's CB trips
                            if tripped && meta.critical {
                                tracing::error!(
                                    "EventReactor: CRITICAL handler '{}' circuit breaker tripped (backoff attempt {})",
                                    meta.name,
                                    backoff_attempt
                                );
                                reactions.push(UnifiedEvent {
                                    id: EventId::new(),
                                    sequence: SequenceNumber(0),
                                    timestamp: chrono::Utc::now(),
                                    severity: EventSeverity::Critical,
                                    category: EventCategory::Orchestrator,
                                    goal_id: None,
                                    task_id: None,
                                    correlation_id: event.correlation_id,
                                    source_process_id: None,
                                    payload: EventPayload::CriticalHandlerDegraded {
                                        handler_name: meta.name.clone(),
                                        error: e.clone(),
                                        failure_count: config.circuit_breaker_threshold,
                                        backoff_attempt,
                                    },
                                });
                            }
                            // Do NOT advance watermark for failed handlers
                        }
                        Err(_) => {
                            let timeout_msg =
                                format!("handler timed out after {}ms", config.handler_timeout_ms);
                            tracing::warn!("EventReactor: handler '{}' {}", meta.name, timeout_msg);
                            // Write to dead letter queue
                            if let Some(ref store) = event_store
                                && let Err(dlq_err) = store
                                    .append_dead_letter(
                                        &event.id.0.to_string(),
                                        event.sequence.0,
                                        &meta.name,
                                        &timeout_msg,
                                        3,
                                    )
                                    .await
                            {
                                tracing::warn!(
                                    "EventReactor: failed to write DLQ entry: {}",
                                    dlq_err
                                );
                            }
                            // Atomic record-and-report under one lock section; also pick up
                            // whether the timeout just tripped the CB so we can emit
                            // CriticalHandlerDegraded symmetrically with the error branch.
                            let mut tripped = false;
                            let mut backoff_attempt = 0u32;
                            {
                                let mut cbs = circuit_breakers.write().await;
                                if let Some(cb) = cbs.get_mut(&meta.id) {
                                    let (t, b) = cb.record_failure_and_report(
                                        config.circuit_breaker_threshold,
                                        Duration::from_secs(config.circuit_breaker_window_secs),
                                        Duration::from_secs(config.circuit_breaker_cooldown_secs),
                                        config.critical_cooldown_initial_secs,
                                        config.critical_cooldown_max_secs,
                                    );
                                    tripped = t;
                                    backoff_attempt = b;
                                }
                            }
                            // Emit CriticalHandlerDegraded when a critical handler's CB trips
                            // on timeout — matches the error-path behavior.
                            if tripped && meta.critical {
                                tracing::error!(
                                    "EventReactor: CRITICAL handler '{}' circuit breaker tripped on timeout (backoff attempt {})",
                                    meta.name,
                                    backoff_attempt
                                );
                                reactions.push(UnifiedEvent {
                                    id: EventId::new(),
                                    sequence: SequenceNumber(0),
                                    timestamp: chrono::Utc::now(),
                                    severity: EventSeverity::Critical,
                                    category: EventCategory::Orchestrator,
                                    goal_id: None,
                                    task_id: None,
                                    correlation_id: event.correlation_id,
                                    source_process_id: None,
                                    payload: EventPayload::CriticalHandlerDegraded {
                                        handler_name: meta.name.clone(),
                                        error: timeout_msg.clone(),
                                        failure_count: config.circuit_breaker_threshold,
                                        backoff_attempt,
                                    },
                                });
                            }
                            // Do NOT advance watermark for timed-out handlers
                        }
                    }
                }

                events_processed.fetch_add(1, Ordering::Relaxed);

                // Track last processed sequence for lag recovery
                if event.sequence.0 > last_processed_sequence {
                    last_processed_sequence = event.sequence.0;
                }

                // Buffer watermark updates for handlers that successfully processed
                // or were filter-skipped (but NOT circuit-breaker-skipped) for this event
                if event_store.is_some() {
                    let mut wm_buf = watermark_buffer.write().await;
                    for handler_name in successful_handlers.iter().chain(skipped_handlers.iter()) {
                        wm_buf.insert(handler_name.clone(), event.sequence);
                    }

                    // Flush watermarks every 10 seconds or 100 events
                    let count = watermark_event_count.fetch_add(1, Ordering::Relaxed) + 1;
                    let should_flush = count >= 100 || {
                        let last = last_watermark_flush.read().await;
                        Instant::now().duration_since(*last) >= Duration::from_secs(10)
                    };

                    if should_flush {
                        if let Some(ref store) = event_store {
                            let to_flush: HashMap<String, SequenceNumber> =
                                wm_buf.drain().collect();
                            drop(wm_buf);
                            for (name, seq) in &to_flush {
                                if let Err(e) = store.set_watermark(name, *seq).await {
                                    tracing::warn!("Failed to flush watermark for {}: {}", name, e);
                                }
                            }

                            // Flush circuit breaker states alongside watermarks
                            let cbs = circuit_breakers.read().await;
                            let hs = handlers.read().await;
                            for handler in hs.iter() {
                                let meta = handler.metadata();
                                if let Some(cb) = cbs.get(&meta.id)
                                    && (cb.failure_count > 0 || cb.tripped)
                                {
                                    let tripped_at = cb.tripped_at.map(|_| chrono::Utc::now());
                                    let last_failure_at =
                                        cb.last_failure.map(|_| chrono::Utc::now());
                                    if let Err(e) = store
                                        .save_circuit_breaker_state(
                                            &meta.name,
                                            cb.failure_count,
                                            cb.tripped,
                                            tripped_at,
                                            last_failure_at,
                                        )
                                        .await
                                    {
                                        tracing::warn!(
                                            "Failed to flush CB state for {}: {}",
                                            meta.name,
                                            e
                                        );
                                    }
                                }
                            }
                        } else {
                            drop(wm_buf);
                        }
                        watermark_event_count.store(0, Ordering::Relaxed);
                        let mut last = last_watermark_flush.write().await;
                        *last = Instant::now();
                    }
                }

                // Publish reactions back into the EventBus
                for reaction_event in reactions {
                    event_bus.publish(reaction_event).await;
                }
            }
        })
    }

    /// Stop the reactor.
    pub fn stop(&self) {
        self.running.store(false, Ordering::SeqCst);
    }

    /// Get the number of events processed.
    pub fn events_processed(&self) -> u64 {
        self.events_processed.load(Ordering::Relaxed)
    }

    /// Get the number of events dropped due to rate limiting.
    pub fn events_dropped(&self) -> u64 {
        self.events_dropped.load(Ordering::Relaxed)
    }

    /// Check if the reactor is running.
    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    /// Get the names of all registered handlers (snapshot).
    pub async fn handler_names(&self) -> Vec<String> {
        let handlers = self.handlers.read().await;
        handlers.iter().map(|h| h.metadata().name).collect()
    }

    /// Load persisted circuit breaker states from the event store.
    /// Call this after registering handlers and before starting the reactor.
    pub async fn load_circuit_breaker_states(&self) {
        let store = match &self.event_store {
            Some(s) => s.clone(),
            None => return,
        };

        match store.load_circuit_breaker_states().await {
            Ok(records) => {
                let handlers = self.handlers.read().await;
                let mut cbs = self.circuit_breakers.write().await;
                for record in records {
                    // Find the handler by name to get its HandlerId
                    if let Some(handler) = handlers
                        .iter()
                        .find(|h| h.metadata().name == record.handler_name)
                    {
                        let id = handler.metadata().id;
                        let cb = cbs.entry(id).or_insert_with(CircuitBreakerState::new);
                        cb.failure_count = record.failure_count;
                        cb.tripped = record.tripped;
                        // We can't restore exact Instant from DateTime, but we can approximate
                        // by checking if the tripped state is recent enough to still matter
                        if record.tripped {
                            // Use current instant as "tripped_at" to give it the full cooldown
                            cb.tripped_at = Some(Instant::now());
                        }
                    }
                }
                tracing::info!(
                    "Loaded circuit breaker states for {} handlers",
                    handlers.len()
                );
            }
            Err(e) => {
                tracing::warn!("Failed to load circuit breaker states: {}", e);
            }
        }
    }

    /// Replay events that handlers missed during downtime.
    ///
    /// Finds the minimum watermark across all registered handlers, queries
    /// the store for events since that point, and dispatches only to handlers
    /// whose watermark is below each event's sequence number.
    /// Reaction return values are ignored during replay (they were already
    /// persisted/emitted in the original run).
    /// Flush all buffered watermarks to the event store immediately.
    ///
    /// Should be called during shutdown to ensure no watermark updates are lost.
    pub async fn flush_watermarks(&self) {
        let store = match &self.event_store {
            Some(s) => s.clone(),
            None => return,
        };

        let to_flush: HashMap<String, SequenceNumber> = {
            let mut buf = self.watermark_buffer.write().await;
            buf.drain().collect()
        };

        if to_flush.is_empty() {
            return;
        }

        for (name, seq) in &to_flush {
            if let Err(e) = store.set_watermark(name, *seq).await {
                tracing::warn!(
                    "Failed to flush watermark for {} during shutdown: {}",
                    name,
                    e
                );
            }
        }

        tracing::info!(
            "Flushed {} handler watermarks during shutdown",
            to_flush.len()
        );
    }

    pub async fn replay_missed_events(&self) -> Result<u64, String> {
        let store = match &self.event_store {
            Some(s) => s.clone(),
            None => return Ok(0),
        };

        let handlers = self.handlers.read().await;
        if handlers.is_empty() {
            return Ok(0);
        }

        // Find minimum watermark across all registered handlers
        let mut min_watermark: Option<SequenceNumber> = None;
        let mut handler_watermarks: HashMap<String, SequenceNumber> = HashMap::new();

        for handler in handlers.iter() {
            let meta = handler.metadata();
            let wm = store
                .get_watermark(&meta.name)
                .await
                .map_err(|e| format!("Failed to get watermark for {}: {}", meta.name, e))?;

            let seq = wm.unwrap_or(SequenceNumber(0));
            handler_watermarks.insert(meta.name.clone(), seq);

            min_watermark = Some(match min_watermark {
                Some(current) if seq < current => seq,
                Some(current) => current,
                None => seq,
            });
        }

        let min_seq = match min_watermark {
            Some(s) => s,
            None => return Ok(0),
        };

        // Query events since the minimum watermark
        let mut events = store
            .replay_since(min_seq)
            .await
            .map_err(|e| format!("Failed to replay events: {}", e))?;

        // Apply max replay limit from config
        if let Some(max) = self.config.startup_max_replay_events
            && events.len() > max
        {
            tracing::warn!(
                "Truncating replay from {} to {} events (startup_max_replay_events)",
                events.len(),
                max
            );
            events.truncate(max);
        }

        let mut replayed_count: u64 = 0;

        for event in &events {
            for handler in handlers.iter() {
                let meta = handler.metadata();

                // Only dispatch to handlers whose watermark is below this event's sequence
                let handler_wm = handler_watermarks
                    .get(&meta.name)
                    .copied()
                    .unwrap_or(SequenceNumber(0));

                if event.sequence <= handler_wm {
                    continue;
                }

                if !meta.filter.matches(event) {
                    // Filter-skipped: still advance watermark (replay would skip again)
                    handler_watermarks.insert(meta.name.clone(), event.sequence);
                    continue;
                }

                let ctx = HandlerContext {
                    chain_depth: 0,
                    correlation_id: event.correlation_id,
                };

                // Execute handler, ignore reactions during replay
                match tokio::time::timeout(
                    Duration::from_millis(self.config.handler_timeout_ms),
                    handler.handle(event, &ctx),
                )
                .await
                {
                    Ok(Ok(_)) => {
                        // Update handler watermark in our local map
                        handler_watermarks.insert(meta.name.clone(), event.sequence);
                    }
                    Ok(Err(e)) => {
                        tracing::warn!(
                            "Replay: handler '{}' error on seq {}: {}",
                            meta.name,
                            event.sequence,
                            e
                        );
                    }
                    Err(_) => {
                        tracing::warn!(
                            "Replay: handler '{}' timed out on seq {}",
                            meta.name,
                            event.sequence
                        );
                    }
                }
            }
            replayed_count += 1;
        }

        // Flush updated watermarks to store
        for (name, seq) in &handler_watermarks {
            if let Err(e) = store.set_watermark(name, *seq).await {
                tracing::warn!("Failed to update watermark for {}: {}", name, e);
            }
        }

        Ok(replayed_count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::event_bus::{EventBusConfig, EventId, EventPayload, SequenceNumber};
    use chrono::Utc;

    struct TestHandler {
        id: HandlerId,
        name: String,
        filter: EventFilter,
        call_count: Arc<AtomicU64>,
        should_fail: bool,
    }

    #[async_trait]
    impl EventHandler for TestHandler {
        fn metadata(&self) -> HandlerMetadata {
            HandlerMetadata {
                id: self.id,
                name: self.name.clone(),
                filter: EventFilter {
                    categories: self.filter.categories.clone(),
                    min_severity: self.filter.min_severity,
                    goal_id: self.filter.goal_id,
                    task_id: self.filter.task_id,
                    payload_types: self.filter.payload_types.clone(),
                    custom_predicate: None,
                },
                priority: HandlerPriority::NORMAL,
                error_strategy: ErrorStrategy::CircuitBreak,
                critical: false,
            }
        }

        async fn handle(
            &self,
            _event: &UnifiedEvent,
            _ctx: &HandlerContext,
        ) -> Result<Reaction, String> {
            self.call_count.fetch_add(1, Ordering::Relaxed);
            if self.should_fail {
                Err("test failure".to_string())
            } else {
                Ok(Reaction::None)
            }
        }
    }

    fn make_test_event(category: EventCategory) -> UnifiedEvent {
        let payload = match category {
            EventCategory::Task => EventPayload::TaskCanceled {
                task_id: Uuid::new_v4(),
                reason: "test".to_string(),
            },
            EventCategory::Goal => EventPayload::GoalStatusChanged {
                goal_id: Uuid::new_v4(),
                from_status: "active".to_string(),
                to_status: "paused".to_string(),
            },
            _ => EventPayload::OrchestratorStarted,
        };
        UnifiedEvent {
            id: EventId::new(),
            sequence: SequenceNumber(0),
            timestamp: Utc::now(),
            severity: EventSeverity::Info,
            category,
            goal_id: None,
            task_id: None,
            correlation_id: None,
            source_process_id: None,
            payload,
        }
    }

    #[tokio::test]
    async fn test_reactor_dispatches_to_matching_handler() {
        let bus = Arc::new(EventBus::new(EventBusConfig::default()));
        let reactor = EventReactor::new(bus.clone(), ReactorConfig::default());

        let call_count = Arc::new(AtomicU64::new(0));
        let handler = Arc::new(TestHandler {
            id: HandlerId::new(),
            name: "test".to_string(),
            filter: EventFilter {
                categories: vec![EventCategory::Task],
                ..Default::default()
            },
            call_count: call_count.clone(),
            should_fail: false,
        });

        reactor.register(handler).await;
        let handle = reactor.start();

        // Give reactor time to subscribe
        tokio::time::sleep(Duration::from_millis(50)).await;

        // Publish a matching event
        bus.publish(make_test_event(EventCategory::Task)).await;

        // Wait for processing
        tokio::time::sleep(Duration::from_millis(100)).await;

        assert_eq!(call_count.load(Ordering::Relaxed), 1);

        // Publish a non-matching event
        bus.publish(make_test_event(EventCategory::Goal)).await;
        tokio::time::sleep(Duration::from_millis(100)).await;

        // Still 1 - the Goal event didn't match
        assert_eq!(call_count.load(Ordering::Relaxed), 1);

        reactor.stop();
        handle.abort();
    }

    #[tokio::test]
    async fn test_event_filter_matching() {
        let filter = EventFilter {
            categories: vec![EventCategory::Task],
            payload_types: vec!["TaskCompleted".to_string()],
            ..Default::default()
        };

        let mut event = make_test_event(EventCategory::Task);
        event.payload = EventPayload::TaskCompleted {
            task_id: Uuid::new_v4(),
            tokens_used: 100,
        };
        assert!(filter.matches(&event));

        // Wrong category
        let event2 = make_test_event(EventCategory::Goal);
        assert!(!filter.matches(&event2));

        // Right category, wrong payload type
        let mut event3 = make_test_event(EventCategory::Task);
        event3.payload = EventPayload::TaskReady {
            task_id: Uuid::new_v4(),
            task_title: "test".to_string(),
        };
        assert!(!filter.matches(&event3));
    }

    #[tokio::test]
    async fn test_reactor_chain_depth_protection() {
        let bus = Arc::new(EventBus::new(EventBusConfig::default()));
        let config = ReactorConfig {
            max_chain_depth: 2,
            ..Default::default()
        };
        let reactor = EventReactor::new(bus.clone(), config);

        let call_count = Arc::new(AtomicU64::new(0));
        // This handler emits a new event on every call, creating a chain
        struct ChainHandler {
            call_count: Arc<AtomicU64>,
        }
        #[async_trait]
        impl EventHandler for ChainHandler {
            fn metadata(&self) -> HandlerMetadata {
                HandlerMetadata {
                    id: HandlerId::new(),
                    name: "chain-test".to_string(),
                    filter: EventFilter::default(),
                    priority: HandlerPriority::NORMAL,
                    error_strategy: ErrorStrategy::LogAndContinue,
                    critical: false,
                }
            }
            async fn handle(
                &self,
                event: &UnifiedEvent,
                _ctx: &HandlerContext,
            ) -> Result<Reaction, String> {
                self.call_count.fetch_add(1, Ordering::Relaxed);
                // Emit a chain event with same correlation
                let mut new_event = UnifiedEvent {
                    id: EventId::new(),
                    sequence: SequenceNumber(0),
                    timestamp: Utc::now(),
                    severity: EventSeverity::Info,
                    category: EventCategory::Orchestrator,
                    goal_id: None,
                    task_id: None,
                    correlation_id: event.correlation_id,
                    source_process_id: None,
                    payload: EventPayload::OrchestratorStarted,
                };
                new_event.correlation_id = event.correlation_id.or(Some(Uuid::new_v4()));
                Ok(Reaction::EmitEvents(vec![new_event]))
            }
        }

        let handler = Arc::new(ChainHandler {
            call_count: call_count.clone(),
        });
        reactor.register(handler).await;
        let handle = reactor.start();

        tokio::time::sleep(Duration::from_millis(50)).await;

        // Publish initial event with correlation ID
        let mut event = make_test_event(EventCategory::Task);
        event.correlation_id = Some(Uuid::new_v4());
        bus.publish(event).await;

        // Wait for chain to play out
        tokio::time::sleep(Duration::from_millis(500)).await;

        // Handler should have been called, but chain should be capped
        let count = call_count.load(Ordering::Relaxed);
        assert!(count > 0, "Handler should have been called at least once");
        // With max_chain_depth=2, reactions are suppressed after depth 2
        // The handler will still be called for events, but reactions won't cascade infinitely
        assert!(count < 20, "Chain should be bounded, got {} calls", count);

        reactor.stop();
        handle.abort();
    }

    // --- Configurable test handler for circuit breaker / error strategy tests ---

    struct ConfigurableTestHandler {
        id: HandlerId,
        name: String,
        filter: EventFilter,
        call_count: Arc<AtomicU64>,
        should_fail: bool,
        priority: HandlerPriority,
        error_strategy: ErrorStrategy,
    }

    #[async_trait]
    impl EventHandler for ConfigurableTestHandler {
        fn metadata(&self) -> HandlerMetadata {
            HandlerMetadata {
                id: self.id,
                name: self.name.clone(),
                filter: EventFilter {
                    categories: self.filter.categories.clone(),
                    min_severity: self.filter.min_severity,
                    goal_id: self.filter.goal_id,
                    task_id: self.filter.task_id,
                    payload_types: self.filter.payload_types.clone(),
                    custom_predicate: None,
                },
                priority: self.priority,
                error_strategy: self.error_strategy,
                critical: false,
            }
        }

        async fn handle(
            &self,
            _event: &UnifiedEvent,
            _ctx: &HandlerContext,
        ) -> Result<Reaction, String> {
            self.call_count.fetch_add(1, Ordering::Relaxed);
            if self.should_fail {
                Err("test failure".to_string())
            } else {
                Ok(Reaction::None)
            }
        }
    }

    fn make_sequenced_event(category: EventCategory, seq: u64) -> UnifiedEvent {
        let mut event = make_test_event(category);
        event.sequence = SequenceNumber(seq);
        event
    }

    // --- Handler that records execution order for priority tests ---

    struct OrderTrackingHandler {
        id: HandlerId,
        name: String,
        priority: HandlerPriority,
        execution_order: Arc<tokio::sync::Mutex<Vec<String>>>,
    }

    #[async_trait]
    impl EventHandler for OrderTrackingHandler {
        fn metadata(&self) -> HandlerMetadata {
            HandlerMetadata {
                id: self.id,
                name: self.name.clone(),
                filter: EventFilter::default(),
                priority: self.priority,
                error_strategy: ErrorStrategy::LogAndContinue,
                critical: false,
            }
        }

        async fn handle(
            &self,
            _event: &UnifiedEvent,
            _ctx: &HandlerContext,
        ) -> Result<Reaction, String> {
            let mut order = self.execution_order.lock().await;
            order.push(self.name.clone());
            Ok(Reaction::None)
        }
    }

    #[tokio::test]
    async fn test_circuit_breaker_trips_after_threshold() {
        let bus = Arc::new(EventBus::new(EventBusConfig::default()));
        let config = ReactorConfig {
            circuit_breaker_threshold: 2,
            circuit_breaker_window_secs: 600,
            circuit_breaker_cooldown_secs: 300,
            ..Default::default()
        };
        let reactor = EventReactor::new(bus.clone(), config);

        let call_count = Arc::new(AtomicU64::new(0));
        let handler = Arc::new(ConfigurableTestHandler {
            id: HandlerId::new(),
            name: "cb-test".to_string(),
            filter: EventFilter::default(),
            call_count: call_count.clone(),
            should_fail: true,
            priority: HandlerPriority::NORMAL,
            error_strategy: ErrorStrategy::CircuitBreak,
        });

        reactor.register(handler).await;
        let handle = reactor.start();
        tokio::time::sleep(Duration::from_millis(50)).await;

        // Publish 3 events with unique sequence numbers
        for i in 1..=3 {
            bus.publish(make_sequenced_event(EventCategory::Task, 100 + i))
                .await;
            tokio::time::sleep(Duration::from_millis(150)).await;
        }

        // After 2 failures the CB trips; the 3rd event should be skipped
        let count = call_count.load(Ordering::Relaxed);
        assert_eq!(
            count, 2,
            "Handler should be called exactly 2 times before CB trips, got {}",
            count
        );

        reactor.stop();
        handle.abort();
    }

    #[tokio::test]
    async fn test_circuit_breaker_auto_resets_after_cooldown() {
        let bus = Arc::new(EventBus::new(EventBusConfig::default()));
        let config = ReactorConfig {
            circuit_breaker_threshold: 1,
            circuit_breaker_cooldown_secs: 1,
            circuit_breaker_window_secs: 600,
            ..Default::default()
        };
        let reactor = EventReactor::new(bus.clone(), config);

        let call_count = Arc::new(AtomicU64::new(0));
        let handler = Arc::new(ConfigurableTestHandler {
            id: HandlerId::new(),
            name: "cb-reset-test".to_string(),
            filter: EventFilter::default(),
            call_count: call_count.clone(),
            should_fail: true,
            priority: HandlerPriority::NORMAL,
            error_strategy: ErrorStrategy::CircuitBreak,
        });

        reactor.register(handler).await;
        let handle = reactor.start();
        tokio::time::sleep(Duration::from_millis(50)).await;

        // First event: handler fails → CB trips (threshold=1)
        bus.publish(make_sequenced_event(EventCategory::Task, 200))
            .await;
        tokio::time::sleep(Duration::from_millis(150)).await;
        assert_eq!(call_count.load(Ordering::Relaxed), 1);

        // Second event while CB is tripped: handler should NOT be called
        bus.publish(make_sequenced_event(EventCategory::Task, 201))
            .await;
        tokio::time::sleep(Duration::from_millis(150)).await;
        assert_eq!(
            call_count.load(Ordering::Relaxed),
            1,
            "Handler should not be called while CB is tripped"
        );

        // Wait for cooldown to expire (1s cooldown + margin)
        tokio::time::sleep(Duration::from_millis(1200)).await;

        // Third event after cooldown: CB auto-resets → handler called again
        bus.publish(make_sequenced_event(EventCategory::Task, 202))
            .await;
        tokio::time::sleep(Duration::from_millis(150)).await;
        assert_eq!(
            call_count.load(Ordering::Relaxed),
            2,
            "Handler should be called again after CB cooldown"
        );

        reactor.stop();
        handle.abort();
    }

    #[tokio::test]
    async fn test_dedup_skips_duplicate_sequence() {
        // Note: EventBus::publish assigns monotonically increasing sequence numbers,
        // so duplicate sequences cannot occur through normal publish. The reactor's
        // VecDeque-based dedup protects against duplicates during lag recovery
        // (broadcast channel overflow). We test this by using an InMemoryEventStore:
        // pre-populate the store with events, publish the same events through the bus
        // (which assigns them new sequences), then trigger replay_missed_events()
        // which uses the store's original sequences. The watermark-based replay
        // skips events already processed.
        use crate::services::event_store::InMemoryEventStore;

        let store = Arc::new(InMemoryEventStore::new());
        let bus = Arc::new(EventBus::new(EventBusConfig::default()));
        let config = ReactorConfig::default();
        let reactor = EventReactor::new(bus.clone(), config).with_store(store.clone());

        let call_count = Arc::new(AtomicU64::new(0));
        let handler = Arc::new(TestHandler {
            id: HandlerId::new(),
            name: "dedup-test".to_string(),
            filter: EventFilter {
                categories: vec![EventCategory::Task],
                ..Default::default()
            },
            call_count: call_count.clone(),
            should_fail: false,
        });

        reactor.register(handler).await;

        // Pre-populate the store with an event at sequence 10
        let mut stored_event = make_test_event(EventCategory::Task);
        stored_event.sequence = SequenceNumber(10);
        store.append(&stored_event).await.unwrap();

        // Set watermark to 0 so replay would try to replay seq=10
        store
            .set_watermark("dedup-test", SequenceNumber(0))
            .await
            .unwrap();

        let handle = reactor.start();
        tokio::time::sleep(Duration::from_millis(50)).await;

        // Publish an event normally — EventBus assigns it seq=0
        bus.publish(make_test_event(EventCategory::Task)).await;
        tokio::time::sleep(Duration::from_millis(150)).await;
        assert_eq!(
            call_count.load(Ordering::Relaxed),
            1,
            "First event should be processed"
        );

        // Now call replay_missed_events — it replays seq=10 from store
        // The handler should be called because seq=10 is not in the dedup set
        // and watermark is 0 (below seq=10)
        let replayed = reactor.replay_missed_events().await.unwrap();
        assert!(replayed > 0, "Should have replayed at least one event");
        tokio::time::sleep(Duration::from_millis(100)).await;

        // Handler should have been called twice: once from publish, once from replay
        let count = call_count.load(Ordering::Relaxed);
        assert_eq!(
            count, 2,
            "Handler should be called for both published and replayed events, got {}",
            count
        );

        reactor.stop();
        handle.abort();
    }

    #[tokio::test]
    async fn test_handlers_execute_in_priority_order() {
        let bus = Arc::new(EventBus::new(EventBusConfig::default()));
        let reactor = EventReactor::new(bus.clone(), ReactorConfig::default());

        let execution_order = Arc::new(tokio::sync::Mutex::new(Vec::new()));

        // Register handlers in reverse priority order (LOW first) to verify sorting
        let low_handler = Arc::new(OrderTrackingHandler {
            id: HandlerId::new(),
            name: "low".to_string(),
            priority: HandlerPriority::LOW,
            execution_order: execution_order.clone(),
        });
        let normal_handler = Arc::new(OrderTrackingHandler {
            id: HandlerId::new(),
            name: "normal".to_string(),
            priority: HandlerPriority::NORMAL,
            execution_order: execution_order.clone(),
        });
        let system_handler = Arc::new(OrderTrackingHandler {
            id: HandlerId::new(),
            name: "system".to_string(),
            priority: HandlerPriority::SYSTEM,
            execution_order: execution_order.clone(),
        });

        // Register in deliberately wrong order
        reactor.register(low_handler).await;
        reactor.register(normal_handler).await;
        reactor.register(system_handler).await;

        let handle = reactor.start();
        tokio::time::sleep(Duration::from_millis(50)).await;

        bus.publish(make_sequenced_event(EventCategory::Task, 300))
            .await;
        tokio::time::sleep(Duration::from_millis(200)).await;

        let order = execution_order.lock().await;
        assert_eq!(
            order.len(),
            3,
            "All 3 handlers should have been called, got {}",
            order.len()
        );
        assert_eq!(order[0], "system", "SYSTEM priority should execute first");
        assert_eq!(order[1], "normal", "NORMAL priority should execute second");
        assert_eq!(order[2], "low", "LOW priority should execute last");

        reactor.stop();
        handle.abort();
    }

    #[tokio::test]
    async fn test_log_and_continue_does_not_trip_circuit_breaker() {
        let bus = Arc::new(EventBus::new(EventBusConfig::default()));
        let config = ReactorConfig {
            circuit_breaker_threshold: 1,
            circuit_breaker_window_secs: 600,
            circuit_breaker_cooldown_secs: 300,
            ..Default::default()
        };
        let reactor = EventReactor::new(bus.clone(), config);

        let call_count = Arc::new(AtomicU64::new(0));
        let handler = Arc::new(ConfigurableTestHandler {
            id: HandlerId::new(),
            name: "log-continue-test".to_string(),
            // Use Task filter to avoid matching HandlerError reaction events
            // (which have category Orchestrator and would cause cascading calls)
            filter: EventFilter {
                categories: vec![EventCategory::Task],
                ..Default::default()
            },
            call_count: call_count.clone(),
            should_fail: true,
            priority: HandlerPriority::NORMAL,
            error_strategy: ErrorStrategy::LogAndContinue,
        });

        reactor.register(handler).await;
        let handle = reactor.start();
        tokio::time::sleep(Duration::from_millis(50)).await;

        // Publish 3 events — all should reach the handler despite failures
        // because LogAndContinue doesn't trip the circuit breaker
        for i in 1..=3 {
            bus.publish(make_sequenced_event(EventCategory::Task, 400 + i))
                .await;
            tokio::time::sleep(Duration::from_millis(150)).await;
        }

        let count = call_count.load(Ordering::Relaxed);
        assert_eq!(
            count, 3,
            "LogAndContinue handler should be called all 3 times, got {}",
            count
        );

        reactor.stop();
        handle.abort();
    }

    // --- Critical handler tests ---

    /// Configurable handler that supports critical flag and dynamic failure control.
    struct CriticalTestHandler {
        id: HandlerId,
        name: String,
        call_count: Arc<AtomicU64>,
        should_fail: Arc<AtomicBool>,
        critical: bool,
        error_strategy: ErrorStrategy,
    }

    #[async_trait]
    impl EventHandler for CriticalTestHandler {
        fn metadata(&self) -> HandlerMetadata {
            HandlerMetadata {
                id: self.id,
                name: self.name.clone(),
                filter: EventFilter {
                    categories: vec![EventCategory::Task],
                    ..Default::default()
                },
                priority: HandlerPriority::NORMAL,
                error_strategy: self.error_strategy,
                critical: self.critical,
            }
        }

        async fn handle(
            &self,
            _event: &UnifiedEvent,
            _ctx: &HandlerContext,
        ) -> Result<Reaction, String> {
            self.call_count.fetch_add(1, Ordering::Relaxed);
            if self.should_fail.load(Ordering::Relaxed) {
                Err("critical test failure".to_string())
            } else {
                Ok(Reaction::None)
            }
        }
    }

    #[test]
    fn test_critical_handler_uses_shorter_cooldown() {
        // Critical handler with backoff_attempt=0 should use 1s cooldown (initial),
        // not the default 60s.
        let mut cb = CircuitBreakerState::new();
        cb.critical = true;
        cb.backoff_attempt = 0;

        let default_cooldown = Duration::from_secs(60);
        let effective = cb.effective_cooldown(default_cooldown);
        assert_eq!(
            effective,
            Duration::from_secs(1),
            "Critical handler initial cooldown should be 1s, got {:?}",
            effective
        );

        // Non-critical should use the default 60s
        let mut cb_normal = CircuitBreakerState::new();
        cb_normal.critical = false;
        let effective_normal = cb_normal.effective_cooldown(default_cooldown);
        assert_eq!(
            effective_normal,
            Duration::from_secs(60),
            "Non-critical handler should use default 60s cooldown"
        );
    }

    #[test]
    fn test_critical_handler_exponential_backoff() {
        // Test the exponential backoff sequence: 1s, 2s, 4s, 8s, 16s (capped)
        let mut cb = CircuitBreakerState::new();
        cb.critical = true;
        let default_cooldown = Duration::from_secs(60);

        let expected = [1, 2, 4, 8, 16, 16]; // 16 is the cap
        for (attempt, &expected_secs) in expected.iter().enumerate() {
            cb.backoff_attempt = attempt as u32;
            let effective = cb.effective_cooldown(default_cooldown);
            assert_eq!(
                effective,
                Duration::from_secs(expected_secs),
                "Attempt {} should have {}s cooldown, got {:?}",
                attempt,
                expected_secs,
                effective
            );
        }
    }

    #[test]
    fn test_critical_handler_backoff_reset_on_success() {
        let mut cb = CircuitBreakerState::new();
        cb.critical = true;
        cb.backoff_attempt = 3;
        assert_eq!(cb.backoff_attempt, 3);

        cb.reset_backoff();
        assert_eq!(
            cb.backoff_attempt, 0,
            "Backoff attempt should reset to 0 after reset_backoff()"
        );

        // After reset, effective cooldown should be back to initial (1s)
        let effective = cb.effective_cooldown(Duration::from_secs(60));
        assert_eq!(effective, Duration::from_secs(1));
    }

    #[test]
    fn test_critical_handler_configurable_backoff_params() {
        // Test with custom initial/max via effective_cooldown_with_config
        let mut cb = CircuitBreakerState::new();
        cb.critical = true;

        // Custom config: initial=2s, max=10s
        // Expected sequence: 2, 4, 8, 10 (capped), 10
        cb.backoff_attempt = 0;
        assert_eq!(
            cb.effective_cooldown_with_config(Duration::from_secs(60), 2, 10),
            Duration::from_secs(2)
        );
        cb.backoff_attempt = 1;
        assert_eq!(
            cb.effective_cooldown_with_config(Duration::from_secs(60), 2, 10),
            Duration::from_secs(4)
        );
        cb.backoff_attempt = 2;
        assert_eq!(
            cb.effective_cooldown_with_config(Duration::from_secs(60), 2, 10),
            Duration::from_secs(8)
        );
        cb.backoff_attempt = 3;
        assert_eq!(
            cb.effective_cooldown_with_config(Duration::from_secs(60), 2, 10),
            Duration::from_secs(10)
        ); // capped
    }

    #[tokio::test]
    async fn test_critical_handler_emits_degraded_event() {
        // When a critical handler's CB trips, a CriticalHandlerDegraded event should be emitted.
        let bus = Arc::new(EventBus::new(EventBusConfig::default()));
        let config = ReactorConfig {
            circuit_breaker_threshold: 1, // Trip after 1 failure
            circuit_breaker_window_secs: 600,
            circuit_breaker_cooldown_secs: 300,
            critical_cooldown_initial_secs: 1,
            critical_cooldown_max_secs: 16,
            ..Default::default()
        };
        let reactor = EventReactor::new(bus.clone(), config);

        // Subscribe to catch the CriticalHandlerDegraded event
        let mut monitor = bus.subscribe();

        let call_count = Arc::new(AtomicU64::new(0));
        let handler = Arc::new(CriticalTestHandler {
            id: HandlerId::new(),
            name: "critical-degraded-test".to_string(),
            call_count: call_count.clone(),
            should_fail: Arc::new(AtomicBool::new(true)),
            critical: true,
            error_strategy: ErrorStrategy::CircuitBreak,
        });

        reactor.register(handler).await;
        let handle = reactor.start();
        tokio::time::sleep(Duration::from_millis(50)).await;

        // Publish event that will cause critical handler to fail and trip CB
        bus.publish(make_sequenced_event(EventCategory::Task, 500))
            .await;
        tokio::time::sleep(Duration::from_millis(300)).await;

        // Collect events from the bus to find CriticalHandlerDegraded
        let mut found_degraded = false;
        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline {
            match tokio::time::timeout(Duration::from_millis(100), monitor.recv()).await {
                Ok(Ok(event)) => {
                    if let EventPayload::CriticalHandlerDegraded {
                        handler_name,
                        failure_count,
                        ..
                    } = &event.payload
                    {
                        assert_eq!(handler_name, "critical-degraded-test");
                        assert_eq!(*failure_count, 1); // threshold is 1
                        assert_eq!(event.severity, EventSeverity::Critical);
                        found_degraded = true;
                        break;
                    }
                }
                _ => continue,
            }
        }
        assert!(
            found_degraded,
            "Should have received a CriticalHandlerDegraded event"
        );

        reactor.stop();
        handle.abort();
    }

    #[tokio::test]
    async fn test_critical_handler_resets_backoff_on_success() {
        // After a critical handler succeeds, its backoff should reset so the next trip
        // starts from the initial cooldown again.
        let bus = Arc::new(EventBus::new(EventBusConfig::default()));
        let config = ReactorConfig {
            circuit_breaker_threshold: 1,
            circuit_breaker_window_secs: 600,
            circuit_breaker_cooldown_secs: 300,
            critical_cooldown_initial_secs: 1,
            critical_cooldown_max_secs: 16,
            ..Default::default()
        };
        let reactor = EventReactor::new(bus.clone(), config);

        let call_count = Arc::new(AtomicU64::new(0));
        let should_fail = Arc::new(AtomicBool::new(true));
        let handler_id = HandlerId::new();
        let handler = Arc::new(CriticalTestHandler {
            id: handler_id,
            name: "critical-reset-test".to_string(),
            call_count: call_count.clone(),
            should_fail: should_fail.clone(),
            critical: true,
            error_strategy: ErrorStrategy::CircuitBreak,
        });

        reactor.register(handler).await;
        let handle = reactor.start();
        tokio::time::sleep(Duration::from_millis(50)).await;

        // Fail once to trip CB
        bus.publish(make_sequenced_event(EventCategory::Task, 600))
            .await;
        tokio::time::sleep(Duration::from_millis(150)).await;
        assert_eq!(call_count.load(Ordering::Relaxed), 1);

        // Wait for critical cooldown (1s initial + margin)
        tokio::time::sleep(Duration::from_millis(1200)).await;

        // Now succeed
        should_fail.store(false, Ordering::Relaxed);
        bus.publish(make_sequenced_event(EventCategory::Task, 601))
            .await;
        tokio::time::sleep(Duration::from_millis(150)).await;
        assert_eq!(
            call_count.load(Ordering::Relaxed),
            2,
            "Handler should be called after CB cooldown"
        );

        // Check that backoff was reset
        let cbs = reactor.circuit_breakers.read().await;
        let cb = cbs.get(&handler_id).expect("CB state should exist");
        assert_eq!(
            cb.backoff_attempt, 0,
            "Backoff should reset to 0 after successful execution"
        );

        reactor.stop();
        handle.abort();
    }
}
