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
use serde::{Deserialize, Serialize};
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

/// Serializable form of ScheduleType for DB persistence.
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
enum ScheduleData {
    #[serde(rename = "once")]
    Once { at: String },
    #[serde(rename = "interval")]
    Interval { every_secs: u64 },
    #[serde(rename = "cron")]
    Cron { cron: String },
}

impl From<&ScheduleType> for ScheduleData {
    fn from(st: &ScheduleType) -> Self {
        match st {
            ScheduleType::Once { at } => ScheduleData::Once { at: at.to_rfc3339() },
            ScheduleType::Interval { every } => ScheduleData::Interval { every_secs: every.as_secs() },
            ScheduleType::Cron { expression } => ScheduleData::Cron { cron: expression.clone() },
        }
    }
}

impl ScheduleData {
    fn to_schedule_type(&self) -> Option<ScheduleType> {
        match self {
            ScheduleData::Once { at } => {
                DateTime::parse_from_rfc3339(at).ok().map(|dt| ScheduleType::Once { at: dt.with_timezone(&Utc) })
            }
            ScheduleData::Interval { every_secs } => {
                Some(ScheduleType::Interval { every: Duration::from_secs(*every_secs) })
            }
            ScheduleData::Cron { cron } => Some(ScheduleType::Cron { expression: cron.clone() }),
        }
    }
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
    /// Optional SQLite pool for schedule persistence.
    pool: Option<sqlx::SqlitePool>,
    /// Counter for batching fire-state updates.
    fire_state_dirty: Arc<std::sync::atomic::AtomicU32>,
}

impl EventScheduler {
    pub fn new(event_bus: Arc<EventBus>, config: SchedulerConfig) -> Self {
        Self {
            event_bus,
            config,
            schedules: Arc::new(RwLock::new(Vec::new())),
            running: Arc::new(AtomicBool::new(false)),
            pool: None,
            fire_state_dirty: Arc::new(std::sync::atomic::AtomicU32::new(0)),
        }
    }

    /// Attach a SQLite pool for schedule persistence.
    pub fn with_pool(mut self, pool: sqlx::SqlitePool) -> Self {
        self.pool = Some(pool);
        self
    }

    /// Load existing schedules from the database.
    ///
    /// Called on startup before `register_builtin_schedules`. Loads all active
    /// rows from `scheduled_events` into memory. Past-due one-shot schedules
    /// that haven't fired are marked to fire immediately. Past-due interval
    /// schedules fire on the next tick (don't accumulate missed firings).
    pub async fn initialize_from_store(&self) {
        let pool = match &self.pool {
            Some(p) => p,
            None => return,
        };

        let rows = match sqlx::query_as::<_, ScheduleRow>(
            "SELECT id, name, schedule_type, schedule_data, payload, category, severity,
                    goal_id, task_id, active, created_at, last_fired, fire_count
             FROM scheduled_events WHERE active = 1"
        )
        .fetch_all(pool)
        .await
        {
            Ok(rows) => rows,
            Err(e) => {
                tracing::error!("Failed to load scheduled events from DB: {}", e);
                return;
            }
        };

        let mut schedules = self.schedules.write().await;
        let mut loaded = 0;

        for row in rows {
            if let Some(sched) = row.to_scheduled_event() {
                // Skip if already registered (by name)
                if schedules.iter().any(|s| s.name == sched.name) {
                    continue;
                }
                schedules.push(sched);
                loaded += 1;
            }
        }

        tracing::info!("Loaded {} scheduled events from database", loaded);
    }

    /// Persist a schedule to the database.
    async fn persist_schedule(&self, sched: &ScheduledEvent) {
        let pool = match &self.pool {
            Some(p) => p,
            None => return,
        };

        let schedule_data = serde_json::to_string(&ScheduleData::from(&sched.schedule))
            .unwrap_or_default();
        // payload column kept for DB schema compatibility; the scheduler always
        // fires ScheduledEventFired events directly, so the stored value is unused.
        let payload_json = "\"ScheduledEventFired\"";

        let category = format!("{:?}", sched.category).to_lowercase();
        let severity = format!("{:?}", sched.severity).to_lowercase();
        let id = sched.id.to_string();
        let goal_id = sched.goal_id.map(|g| g.to_string());
        let task_id = sched.task_id.map(|t| t.to_string());
        let created_at = sched.created_at.to_rfc3339();
        let last_fired = sched.last_fired.map(|dt| dt.to_rfc3339());

        if let Err(e) = sqlx::query(
            "INSERT OR REPLACE INTO scheduled_events
             (id, name, schedule_type, schedule_data, payload, category, severity,
              goal_id, task_id, active, created_at, last_fired, fire_count)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)"
        )
        .bind(&id)
        .bind(&sched.name)
        .bind(sched.schedule.as_str())
        .bind(&schedule_data)
        .bind(&payload_json)
        .bind(&category)
        .bind(&severity)
        .bind(&goal_id)
        .bind(&task_id)
        .bind(sched.active as i32)
        .bind(&created_at)
        .bind(&last_fired)
        .bind(sched.fire_count as i64)
        .execute(pool)
        .await
        {
            tracing::warn!("Failed to persist schedule '{}': {}", sched.name, e);
        }
    }

    /// Register a new scheduled event. Returns the schedule ID.
    ///
    /// If a schedule with the same name already exists, returns its existing ID
    /// without duplicating. Persists the schedule to the database if a pool is
    /// configured.
    pub async fn register(&self, schedule: ScheduledEvent) -> Option<Uuid> {
        let mut schedules = self.schedules.write().await;

        // Dedup by name: if a schedule with this name already exists, return its ID
        if let Some(existing) = schedules.iter().find(|s| s.name == schedule.name) {
            return Some(existing.id);
        }

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
            source_process_id: None,
            payload: EventPayload::ScheduledEventRegistered {
                schedule_id: id,
                name: schedule.name.clone(),
                schedule_type: schedule.schedule.as_str().to_string(),
            },
        };
        self.event_bus.publish(reg_event).await;

        // Persist to DB before adding to in-memory list
        self.persist_schedule(&schedule).await;

        schedules.push(schedule);
        Some(id)
    }

    /// Cancel a scheduled event by ID. Returns true if found and canceled.
    pub async fn cancel(&self, id: Uuid) -> bool {
        let mut schedules = self.schedules.write().await;
        if let Some(sched) = schedules.iter_mut().find(|s| s.id == id) {
            let name = sched.name.clone();
            sched.active = false;

            // Persist deactivation to DB
            if let Some(ref pool) = self.pool {
                let id_str = id.to_string();
                let _ = sqlx::query("UPDATE scheduled_events SET active = 0 WHERE id = ?1")
                    .bind(&id_str)
                    .execute(pool)
                    .await;
            }

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
                source_process_id: None,
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
        let fire_state_dirty = self.fire_state_dirty.clone();
        let pool = self.pool.clone();

        tokio::spawn(async move {
            let mut tick_count: u64 = 0;

            while running.load(Ordering::SeqCst) {
                tokio::time::sleep(tick_interval).await;
                tick_count += 1;

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
                                    schedule.after(&reference_time).next().is_some_and(|next| now >= next)
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
                                source_process_id: None,
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

                    fire_state_dirty.fetch_add(1, Ordering::Release);

                    for (_, event) in to_fire {
                        event_bus.publish(event).await;
                    }
                }

                // Batch-flush fire state to DB every 10 ticks
                if let Some(pool_ref) = pool.as_ref()
                    && tick_count.is_multiple_of(10) && fire_state_dirty.load(Ordering::Acquire) > 0 {
                        let scheds = schedules.read().await;
                        for sched in scheds.iter() {
                            if let Some(last_fired) = sched.last_fired {
                                let id = sched.id.to_string();
                                let last_fired_str = last_fired.to_rfc3339();
                                let _ = sqlx::query(
                                    "UPDATE scheduled_events SET last_fired = ?1, fire_count = ?2, active = ?3 WHERE id = ?4"
                                )
                                .bind(&last_fired_str)
                                .bind(sched.fire_count as i64)
                                .bind(sched.active as i32)
                                .bind(&id)
                                .execute(pool_ref)
                                .await;
                            }
                        }
                        fire_state_dirty.store(0, Ordering::Release);
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
        category,
        severity,
        goal_id: None,
        task_id: None,
        active: true,
        created_at: Utc::now(),
        last_fired: Some(Utc::now()),
        fire_count: 0,
    }
}

/// Row from the `scheduled_events` table.
#[derive(sqlx::FromRow)]
struct ScheduleRow {
    id: String,
    name: String,
    #[allow(dead_code)]
    schedule_type: String,
    schedule_data: String,
    #[allow(dead_code)]
    payload: String,
    category: String,
    severity: String,
    goal_id: Option<String>,
    task_id: Option<String>,
    active: i32,
    created_at: String,
    last_fired: Option<String>,
    fire_count: i64,
}

impl ScheduleRow {
    fn to_scheduled_event(&self) -> Option<ScheduledEvent> {
        let schedule_data: ScheduleData = serde_json::from_str(&self.schedule_data).ok()?;
        let schedule = schedule_data.to_schedule_type()?;
        let category = match self.category.as_str() {
            "scheduler" => EventCategory::Scheduler,
            "task" => EventCategory::Task,
            "goal" => EventCategory::Goal,
            "agent" => EventCategory::Agent,
            "system" | "orchestrator" => EventCategory::Orchestrator,
            "execution" => EventCategory::Execution,
            "verification" => EventCategory::Verification,
            "escalation" => EventCategory::Escalation,
            "memory" => EventCategory::Memory,
            _ => EventCategory::Scheduler,
        };
        let severity = match self.severity.as_str() {
            "debug" => EventSeverity::Debug,
            "info" => EventSeverity::Info,
            "warning" => EventSeverity::Warning,
            "error" => EventSeverity::Error,
            "critical" => EventSeverity::Critical,
            _ => EventSeverity::Debug,
        };
        let id = Uuid::parse_str(&self.id).ok()?;
        let goal_id = self.goal_id.as_ref().and_then(|s| Uuid::parse_str(s).ok());
        let task_id = self.task_id.as_ref().and_then(|s| Uuid::parse_str(s).ok());
        let created_at = DateTime::parse_from_rfc3339(&self.created_at)
            .ok()
            .map(|dt| dt.with_timezone(&Utc))
            .unwrap_or_else(Utc::now);
        let last_fired = self.last_fired.as_ref().and_then(|s| {
            DateTime::parse_from_rfc3339(s).ok().map(|dt| dt.with_timezone(&Utc))
        });

        Some(ScheduledEvent {
            id,
            name: self.name.clone(),
            schedule,
            category,
            severity,
            goal_id,
            task_id,
            active: self.active != 0,
            created_at,
            last_fired,
            fire_count: self.fire_count as u64,
        })
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
