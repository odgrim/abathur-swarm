//! Background poller for the transactional event outbox.
//!
//! Reads unpublished events from the outbox table and publishes them
//! to the EventBus. Follows the same daemon pattern as [`MemoryDecayDaemon`].

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use crate::domain::ports::OutboxRepository;
use crate::services::clock::{DynClock, system_clock};
use crate::services::event_bus::EventBus;
use crate::services::supervise;

/// Configuration for the outbox poller.
#[derive(Debug, Clone)]
pub struct OutboxPollerConfig {
    /// How often to poll for unpublished events.
    pub poll_interval: Duration,
    /// Maximum number of events to fetch per poll cycle.
    pub batch_size: usize,
    /// Maximum consecutive failures before stopping.
    pub max_consecutive_failures: u32,
    /// How long to keep published events before pruning.
    pub prune_after: Duration,
}

impl Default for OutboxPollerConfig {
    fn default() -> Self {
        Self {
            poll_interval: Duration::from_millis(500),
            batch_size: 100,
            max_consecutive_failures: 10,
            prune_after: Duration::from_secs(3600), // 1 hour
        }
    }
}

/// Handle to control the outbox poller.
pub struct OutboxPollerHandle {
    stop_flag: Arc<AtomicBool>,
}

impl OutboxPollerHandle {
    /// Request the poller to stop.
    pub fn stop(&self) {
        self.stop_flag.store(true, Ordering::Release);
    }
}

/// Background daemon that polls the outbox and publishes events.
pub struct OutboxPoller {
    outbox: Arc<dyn OutboxRepository>,
    event_bus: Arc<EventBus>,
    config: OutboxPollerConfig,
    stop_flag: Arc<AtomicBool>,
    clock: DynClock,
}

impl OutboxPoller {
    /// Create a new outbox poller.
    pub fn new(
        outbox: Arc<dyn OutboxRepository>,
        event_bus: Arc<EventBus>,
        config: OutboxPollerConfig,
    ) -> Self {
        Self {
            outbox,
            event_bus,
            config,
            stop_flag: Arc::new(AtomicBool::new(false)),
            clock: system_clock(),
        }
    }

    /// Inject a custom clock (for deterministic testing).
    pub fn with_clock(mut self, clock: DynClock) -> Self {
        self.clock = clock;
        self
    }

    /// Get a handle to control the poller.
    pub fn handle(&self) -> OutboxPollerHandle {
        OutboxPollerHandle {
            stop_flag: self.stop_flag.clone(),
        }
    }

    /// Start the poller as a background task. Returns a handle to stop it.
    pub fn start(self) -> OutboxPollerHandle {
        let handle = self.handle();
        supervise("outbox_poller", async move {
            self.run_loop().await;
        });
        handle
    }

    /// Main poll loop.
    async fn run_loop(self) {
        tracing::info!(
            poll_interval_ms = self.config.poll_interval.as_millis() as u64,
            batch_size = self.config.batch_size,
            "Outbox poller started"
        );

        let mut consecutive_failures = 0u32;
        let mut poll_count = 0u64;
        let mut interval = tokio::time::interval(self.config.poll_interval);

        loop {
            interval.tick().await;

            if self.stop_flag.load(Ordering::Acquire) {
                tracing::info!("Outbox poller stopping (requested)");
                break;
            }

            poll_count += 1;

            match self.poll_once().await {
                Ok(published) => {
                    consecutive_failures = 0;
                    if published > 0 {
                        tracing::debug!(published, poll_count, "Outbox poller published events");
                    }
                }
                Err(e) => {
                    consecutive_failures += 1;
                    tracing::warn!(
                        error = %e,
                        consecutive_failures,
                        max = self.config.max_consecutive_failures,
                        "Outbox poller failed"
                    );
                    if consecutive_failures >= self.config.max_consecutive_failures {
                        tracing::error!(
                            "Outbox poller stopping after {} consecutive failures",
                            consecutive_failures
                        );
                        break;
                    }
                }
            }

            // Periodic prune of old published events (every 100 polls)
            if poll_count.is_multiple_of(100)
                && let Err(e) = self.outbox.prune_published(self.config.prune_after).await
            {
                tracing::warn!(error = %e, "Failed to prune published outbox events");
            }
        }

        tracing::info!("Outbox poller stopped");
    }

    /// Execute one poll cycle: fetch unpublished events, publish them, mark as published.
    async fn poll_once(&self) -> Result<usize, Box<dyn std::error::Error + Send + Sync>> {
        let events = self
            .outbox
            .fetch_unpublished(self.config.batch_size)
            .await?;

        if events.is_empty() {
            return Ok(0);
        }

        let count = events.len();
        for event in events {
            let event_id = event.id.0.to_string();
            self.event_bus.publish(event).await;
            self.outbox.mark_published(&event_id).await?;
        }

        Ok(count)
    }
}
