//! Time-based event scheduler.
//!
//! Fires events into the EventBus on configurable schedules:
//! one-shot, interval, or cron-based. Used for periodic maintenance,
//! escalation checks, stats updates, etc.

use std::str::FromStr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use tokio::sync::RwLock;
use uuid::Uuid;

use super::event_bus::{
    EventBus, EventCategory, EventId, EventPayload, EventSeverity, SequenceNumber, UnifiedEvent,
};

/// Type of schedule.
#[derive(Debug, Clone)]
pub enum ScheduleType {
    /// Fire once at a specific time.
    Once { at: DateTime<Utc> },
    /// Fire at a fixed interval.
    Interval { every: Duration },
    /// Fire according to a cron expression.
    Cron { expression: String },
}

impl ScheduleType {
    pub fn as_str(&self) -> &str {
        match self {
            Self::Once { .. } => "once",
            Self::Interval { .. } => "interval",
            Self::Cron { .. } => "cron",
        }
    }
}

/// A registered scheduled event.
#[derive(Debug, Clone)]
pub struct ScheduledEvent {
    pub id: Uuid,
    pub name: String,
    pub schedule: ScheduleType,
    pub payload: EventPayload,
    pub category: EventCategory,
    pub severity: EventSeverity,
    pub goal_id: Option<Uuid>,
    pub task_id: Option<Uuid>,
    pub active: bool,
    pub created_at: DateTime<Utc>,
    pub last_fired: Option<DateTime<Utc>>,
    pub fire_count: u64,
}

/// Configuration for the EventScheduler.
#[derive(Debug, Clone)]
pub struct SchedulerConfig {
    /// Tick interval in milliseconds.
    pub tick_interval_ms: u64,
    /// Maximum number of schedules.
    pub max_schedules: usize,
}

impl Default for SchedulerConfig {
    fn default() -> Self {
        Self {
            tick_interval_ms: 1000,
            max_schedules: 100,
        }
    }
}

/// Time-based event scheduler.
pub struct EventScheduler {
    event_bus: Arc<EventBus>,
    config: SchedulerConfig,
    schedules: Arc<RwLock<Vec<ScheduledEvent>>>,
    running: Arc<AtomicBool>,
}

impl EventScheduler {
    pub fn new(event_bus: Arc<EventBus>, config: SchedulerConfig) -> Self {
        Self {
            event_bus,
            config,
            schedules: Arc::new(RwLock::new(Vec::new())),
            running: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Register a new scheduled event. Returns the schedule ID.
    pub async fn register(&self, schedule: ScheduledEvent) -> Option<Uuid> {
        let mut schedules = self.schedules.write().await;
        if schedules.len() >= self.config.max_schedules {
            tracing::warn!("EventScheduler: max schedules ({}) reached, rejecting", self.config.max_schedules);
            return None;
        }
        let id = schedule.id;

        // Emit registration event
        let reg_event = UnifiedEvent {
            id: EventId::new(),
            sequence: SequenceNumber(0),
            timestamp: Utc::now(),
            severity: EventSeverity::Debug,
            category: EventCategory::Scheduler,
            goal_id: None,
            task_id: None,
            correlation_id: None,
            payload: EventPayload::ScheduledEventRegistered {
                schedule_id: id,
                name: schedule.name.clone(),
                schedule_type: schedule.schedule.as_str().to_string(),
            },
        };
        self.event_bus.publish(reg_event).await;

        schedules.push(schedule);
        Some(id)
    }

    /// Cancel a scheduled event by ID. Returns true if found and canceled.
    pub async fn cancel(&self, id: Uuid) -> bool {
        let mut schedules = self.schedules.write().await;
        if let Some(sched) = schedules.iter_mut().find(|s| s.id == id) {
            let name = sched.name.clone();
            sched.active = false;

            // Emit cancellation event
            let cancel_event = UnifiedEvent {
                id: EventId::new(),
                sequence: SequenceNumber(0),
                timestamp: Utc::now(),
                severity: EventSeverity::Debug,
                category: EventCategory::Scheduler,
                goal_id: None,
                task_id: None,
                correlation_id: None,
                payload: EventPayload::ScheduledEventCanceled {
                    schedule_id: id,
                    name,
                },
            };
            // Can't await here while holding the write lock, so drop first
            drop(schedules);
            self.event_bus.publish(cancel_event).await;
            true
        } else {
            false
        }
    }

    /// List all schedules.
    pub async fn list(&self) -> Vec<ScheduledEvent> {
        self.schedules.read().await.clone()
    }

    /// Start the scheduler tick loop. Returns a JoinHandle.
    pub fn start(&self) -> tokio::task::JoinHandle<()> {
        self.running.store(true, Ordering::SeqCst);

        let schedules = self.schedules.clone();
        let event_bus = self.event_bus.clone();
        let running = self.running.clone();
        let tick_interval = Duration::from_millis(self.config.tick_interval_ms);

        tokio::spawn(async move {
            while running.load(Ordering::SeqCst) {
                tokio::time::sleep(tick_interval).await;

                let now = Utc::now();
                let mut to_fire: Vec<(usize, UnifiedEvent)> = Vec::new();

                {
                    let scheds = schedules.read().await;
                    for (idx, sched) in scheds.iter().enumerate() {
                        if !sched.active {
                            continue;
                        }

                        let should_fire = match &sched.schedule {
                            ScheduleType::Once { at } => now >= *at && sched.fire_count == 0,
                            ScheduleType::Interval { every } => {
                                match sched.last_fired {
                                    None => true, // Never fired, fire now
                                    Some(last) => {
                                        let elapsed = now.signed_duration_since(last);
                                        elapsed >= chrono::Duration::from_std(*every).unwrap_or(chrono::TimeDelta::MAX)
                                    }
                                }
                            }
                            ScheduleType::Cron { expression } => {
                                if let Ok(schedule) = cron::Schedule::from_str(expression) {
                                    let reference_time = sched.last_fired.unwrap_or(sched.created_at);
                                    schedule.after(&reference_time).next().map_or(false, |next| now >= next)
                                } else {
                                    false
                                }
                            }
                        };

                        if should_fire {
                            let event = UnifiedEvent {
                                id: EventId::new(),
                                sequence: SequenceNumber(0),
                                timestamp: now,
                                severity: sched.severity,
                                category: sched.category,
                                goal_id: sched.goal_id,
                                task_id: sched.task_id,
                                correlation_id: None,
                                payload: EventPayload::ScheduledEventFired {
                                    schedule_id: sched.id,
                                    name: sched.name.clone(),
                                },
                            };
                            to_fire.push((idx, event));
                        }
                    }
                }

                // Update state and publish events
                if !to_fire.is_empty() {
                    let mut scheds = schedules.write().await;
                    for (idx, _event) in &to_fire {
                        if let Some(sched) = scheds.get_mut(*idx) {
                            sched.last_fired = Some(now);
                            sched.fire_count += 1;

                            // Auto-deactivate one-shot schedules
                            if matches!(sched.schedule, ScheduleType::Once { .. }) {
                                sched.active = false;
                            }
                        }
                    }
                    drop(scheds);

                    for (_, event) in to_fire {
                        event_bus.publish(event).await;
                    }
                }
            }
        })
    }

    /// Stop the scheduler.
    pub fn stop(&self) {
        self.running.store(false, Ordering::SeqCst);
    }

    /// Check if the scheduler is running.
    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }
}

/// Helper to create a named interval schedule.
pub fn interval_schedule(
    name: impl Into<String>,
    every: Duration,
    category: EventCategory,
    severity: EventSeverity,
) -> ScheduledEvent {
    ScheduledEvent {
        id: Uuid::new_v4(),
        name: name.into(),
        schedule: ScheduleType::Interval { every },
        payload: EventPayload::OrchestratorStarted, // placeholder, overridden by ScheduledEventFired
        category,
        severity,
        goal_id: None,
        task_id: None,
        active: true,
        created_at: Utc::now(),
        last_fired: None,
        fire_count: 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::event_bus::EventBusConfig;

    #[tokio::test]
    async fn test_scheduler_register_and_list() {
        let bus = Arc::new(EventBus::new(EventBusConfig::default()));
        let scheduler = EventScheduler::new(bus, SchedulerConfig::default());

        let sched = interval_schedule("test", Duration::from_secs(60), EventCategory::Scheduler, EventSeverity::Debug);
        let id = scheduler.register(sched).await;
        assert!(id.is_some());

        let list = scheduler.list().await;
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].name, "test");
    }

    #[tokio::test]
    async fn test_scheduler_cancel() {
        let bus = Arc::new(EventBus::new(EventBusConfig::default()));
        let scheduler = EventScheduler::new(bus, SchedulerConfig::default());

        let sched = interval_schedule("cancel-me", Duration::from_secs(60), EventCategory::Scheduler, EventSeverity::Debug);
        let id = scheduler.register(sched).await.unwrap();

        assert!(scheduler.cancel(id).await);

        let list = scheduler.list().await;
        assert!(!list[0].active);
    }

    #[tokio::test]
    async fn test_scheduler_fires_interval() {
        let bus = Arc::new(EventBus::new(EventBusConfig::default()));
        let mut rx = bus.subscribe();

        let config = SchedulerConfig {
            tick_interval_ms: 50,
            ..Default::default()
        };
        let scheduler = EventScheduler::new(bus.clone(), config);

        let sched = interval_schedule(
            "fast-tick",
            Duration::from_millis(100),
            EventCategory::Scheduler,
            EventSeverity::Debug,
        );
        scheduler.register(sched).await;

        let handle = scheduler.start();

        // Wait for at least one fire
        let mut fired = false;
        for _ in 0..20 {
            match tokio::time::timeout(Duration::from_millis(200), rx.recv()).await {
                Ok(Ok(event)) => {
                    if matches!(event.payload, EventPayload::ScheduledEventFired { .. }) {
                        fired = true;
                        break;
                    }
                }
                _ => continue,
            }
        }

        assert!(fired, "Scheduler should have fired at least once");

        scheduler.stop();
        handle.abort();
    }

    #[tokio::test]
    async fn test_scheduler_max_schedules() {
        let bus = Arc::new(EventBus::new(EventBusConfig::default()));
        let config = SchedulerConfig {
            max_schedules: 2,
            ..Default::default()
        };
        let scheduler = EventScheduler::new(bus, config);

        let s1 = interval_schedule("s1", Duration::from_secs(60), EventCategory::Scheduler, EventSeverity::Debug);
        let s2 = interval_schedule("s2", Duration::from_secs(60), EventCategory::Scheduler, EventSeverity::Debug);
        let s3 = interval_schedule("s3", Duration::from_secs(60), EventCategory::Scheduler, EventSeverity::Debug);

        assert!(scheduler.register(s1).await.is_some());
        assert!(scheduler.register(s2).await.is_some());
        assert!(scheduler.register(s3).await.is_none()); // Exceeds max
    }
}
