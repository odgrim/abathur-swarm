//! Trigger Rules Engine for declarative event-driven automation.
//!
//! `TriggerRule` pairs a filter + condition with an action. The
//! `TriggerRuleEngine` is an `EventHandler` that evaluates all enabled
//! rules on every event and fires matching actions (emit events or issue
//! commands via the `CommandBus`).

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use uuid::Uuid;

use crate::services::command_bus::{
    CommandBus, CommandEnvelope, CommandSource, DomainCommand,
};
use crate::services::event_bus::{
    EventCategory, EventId, EventPayload, EventSeverity, SequenceNumber, UnifiedEvent,
};
use crate::services::event_reactor::{
    EventFilter, EventHandler, HandlerContext, HandlerId, HandlerMetadata, HandlerPriority,
    ErrorStrategy, Reaction,
};

// ---------------------------------------------------------------------------
// Serializable event filter (persistable subset of EventFilter)
// ---------------------------------------------------------------------------

/// A serializable subset of `EventFilter` that can be stored in the DB.
///
/// Cannot express `custom_predicate` — that is only for hardcoded handlers.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SerializableEventFilter {
    #[serde(default)]
    pub categories: Vec<EventCategory>,
    pub min_severity: Option<EventSeverity>,
    #[serde(default)]
    pub payload_types: Vec<String>,
    pub goal_id: Option<Uuid>,
    pub task_id: Option<Uuid>,
}

impl SerializableEventFilter {
    /// Convert to a runtime `EventFilter`.
    pub fn to_event_filter(&self) -> EventFilter {
        EventFilter {
            categories: self.categories.clone(),
            min_severity: self.min_severity,
            goal_id: self.goal_id,
            task_id: self.task_id,
            payload_types: self.payload_types.clone(),
            custom_predicate: None,
        }
    }

    /// Check if an event matches this filter.
    pub fn matches(&self, event: &UnifiedEvent) -> bool {
        self.to_event_filter().matches(event)
    }
}

// ---------------------------------------------------------------------------
// Trigger condition
// ---------------------------------------------------------------------------

/// When a trigger should fire (beyond the filter match).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TriggerCondition {
    /// Fire on every matching event.
    Always,
    /// Fire only after N matching events arrive within a time window.
    CountThreshold {
        count: u32,
        window_secs: u64,
    },
    /// Fire when an expected event does NOT arrive within a deadline after
    /// a triggering event. For example: "If TaskStarted fires but no
    /// TaskCompleted arrives within 1800s, escalate."
    ///
    /// `trigger_type` is the event type that starts the timer (matched by
    /// the rule's filter). `expected_type` is the event type that must arrive
    /// within `deadline_secs` to cancel the timer. If the deadline expires
    /// without the expected event, the rule fires.
    Absence {
        /// The event type that starts the absence timer (already matched by filter).
        trigger_type: String,
        /// The event type that must arrive to cancel the timer.
        expected_type: String,
        /// Seconds to wait for the expected event before firing.
        deadline_secs: u64,
    },
}

// ---------------------------------------------------------------------------
// Trigger action
// ---------------------------------------------------------------------------

/// What happens when a trigger fires.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TriggerAction {
    /// Emit an event into the EventBus.
    EmitEvent {
        payload: TriggerEventPayload,
        category: EventCategory,
        severity: EventSeverity,
    },
    /// Issue a domain command through the CommandBus.
    IssueCommand {
        command: SerializableDomainCommand,
    },
    /// Both emit an event and issue a command.
    EmitAndCommand {
        payload: TriggerEventPayload,
        category: EventCategory,
        severity: EventSeverity,
        command: SerializableDomainCommand,
    },
}

/// Serializable event payload for trigger-emitted events.
///
/// Triggers cannot construct arbitrary `EventPayload` variants at runtime,
/// so they work through a small set of well-known payloads.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TriggerEventPayload {
    /// Emit a scheduled-event-fired signal (triggers the named handler).
    ScheduledEventFired { name: String },
    /// Emit a human escalation event.
    HumanEscalation { reason: String },
    /// Request a reconciliation sweep.
    ReconciliationRequested,
    /// Request goal evaluation (optionally for a specific goal).
    GoalEvaluationRequested { goal_id: Option<Uuid> },
}

impl TriggerEventPayload {
    /// Convert to a real `EventPayload`.
    pub fn to_event_payload(&self) -> EventPayload {
        match self {
            Self::ScheduledEventFired { name } => EventPayload::ScheduledEventFired {
                schedule_id: Uuid::new_v4(),
                name: name.clone(),
            },
            Self::HumanEscalation { reason } => EventPayload::HumanEscalationNeeded {
                goal_id: None,
                task_id: None,
                reason: reason.clone(),
                urgency: "medium".to_string(),
                is_blocking: false,
            },
            Self::ReconciliationRequested => EventPayload::ScheduledEventFired {
                schedule_id: Uuid::new_v4(),
                name: "reconciliation".to_string(),
            },
            Self::GoalEvaluationRequested { .. } => EventPayload::ScheduledEventFired {
                schedule_id: Uuid::new_v4(),
                name: "goal-evaluation".to_string(),
            },
        }
    }
}

/// Serializable domain command for trigger actions.
///
/// A simplified representation that can be stored as JSON and converted
/// to `DomainCommand` at runtime.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "domain", rename_all = "snake_case")]
pub enum SerializableDomainCommand {
    /// Store a memory.
    StoreMemory {
        key: String,
        content: String,
        namespace: String,
        tier: String,
        memory_type: String,
    },
    /// Promote a memory.
    PromoteMemory { memory_id: Uuid },
    /// Transition a goal's status.
    TransitionGoalStatus {
        goal_id: Uuid,
        new_status: String,
    },
    /// Cancel a task.
    CancelTask {
        task_id: Uuid,
        reason: String,
    },
    /// Retry a failed task.
    RetryTask {
        task_id: Uuid,
    },
    /// Submit a new task.
    SubmitTask {
        title: String,
        description: String,
        priority: String,
        agent_type: Option<String>,
    },
    /// Pause a goal.
    PauseGoal {
        goal_id: Uuid,
    },
    /// Delete a memory.
    ForgetMemory {
        memory_id: Uuid,
    },
    /// Run full memory maintenance.
    RunMemoryMaintenance,
}

// ---------------------------------------------------------------------------
// TriggerRule
// ---------------------------------------------------------------------------

/// A declarative automation rule.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TriggerRule {
    pub id: Uuid,
    pub name: String,
    pub description: String,
    pub filter: SerializableEventFilter,
    pub condition: TriggerCondition,
    pub action: TriggerAction,
    /// Minimum time between consecutive firings.
    pub cooldown: Option<Duration>,
    pub enabled: bool,
    /// Last time this rule fired.
    pub last_fired: Option<DateTime<Utc>>,
    pub fire_count: u64,
    pub created_at: DateTime<Utc>,
}

impl TriggerRule {
    pub fn new(name: impl Into<String>, filter: SerializableEventFilter, action: TriggerAction) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            description: String::new(),
            filter,
            condition: TriggerCondition::Always,
            action,
            cooldown: None,
            enabled: true,
            last_fired: None,
            fire_count: 0,
            created_at: Utc::now(),
        }
    }

    pub fn with_description(mut self, desc: impl Into<String>) -> Self {
        self.description = desc.into();
        self
    }

    pub fn with_condition(mut self, cond: TriggerCondition) -> Self {
        self.condition = cond;
        self
    }

    pub fn with_cooldown(mut self, secs: u64) -> Self {
        self.cooldown = Some(Duration::from_secs(secs));
        self
    }
}

// ---------------------------------------------------------------------------
// TriggerRuleEngine (EventHandler)
// ---------------------------------------------------------------------------

/// Pending absence timer: tracks when a trigger started and what event is expected.
#[derive(Debug, Clone)]
struct AbsenceTimer {
    /// Unique ID for persistence.
    id: Uuid,
    /// When the trigger event arrived (starts the clock).
    started_at: DateTime<Utc>,
    /// Deadline in seconds from started_at.
    deadline_secs: u64,
    /// The expected event type that would cancel this timer.
    expected_type: String,
    /// The task_id from the triggering event (for scoping).
    task_id: Option<Uuid>,
    /// Correlation from the triggering event.
    correlation_id: Option<Uuid>,
}

/// Reactive engine that evaluates trigger rules on incoming events.
pub struct TriggerRuleEngine {
    rules: Arc<RwLock<Vec<TriggerRule>>>,
    command_bus: Arc<CommandBus>,
    /// Ring buffer of recent event timestamps per rule (for CountThreshold).
    event_windows: Arc<RwLock<HashMap<Uuid, VecDeque<DateTime<Utc>>>>>,
    /// Pending absence timers per rule ID. Each rule can have multiple pending timers
    /// (keyed by optional task_id to scope per-task absences).
    absence_timers: Arc<RwLock<HashMap<Uuid, Vec<AbsenceTimer>>>>,
    /// Optional EventBus for emitting trigger rule lifecycle events.
    event_bus: Option<Arc<crate::services::event_bus::EventBus>>,
    /// Optional repository for persisting rule fire state.
    rule_repo: Option<Arc<dyn crate::domain::ports::TriggerRuleRepository>>,
    /// Optional DB pool for persisting absence timers across restarts.
    pool: Option<sqlx::SqlitePool>,
}

impl TriggerRuleEngine {
    pub fn new(command_bus: Arc<CommandBus>) -> Self {
        Self {
            rules: Arc::new(RwLock::new(Vec::new())),
            command_bus,
            event_windows: Arc::new(RwLock::new(HashMap::new())),
            absence_timers: Arc::new(RwLock::new(HashMap::new())),
            event_bus: None,
            rule_repo: None,
            pool: None,
        }
    }

    /// Add an EventBus for emitting trigger rule lifecycle events.
    pub fn with_event_bus(mut self, event_bus: Arc<crate::services::event_bus::EventBus>) -> Self {
        self.event_bus = Some(event_bus);
        self
    }

    /// Add a repository for persisting rule fire state.
    pub fn with_rule_repo(mut self, repo: Arc<dyn crate::domain::ports::TriggerRuleRepository>) -> Self {
        self.rule_repo = Some(repo);
        self
    }

    /// Enable absence timer persistence via SQLite.
    pub fn with_pool(mut self, pool: sqlx::SqlitePool) -> Self {
        self.pool = Some(pool);
        self
    }

    /// Load persisted absence timers from DB into the in-memory map.
    ///
    /// Called during startup to restore timers that were pending when the
    /// process last stopped. Expired timers are left in place — they'll fire
    /// on the next `evaluate()` call.
    pub async fn load_pending_timers(&self) {
        let Some(ref pool) = self.pool else { return };

        #[derive(sqlx::FromRow)]
        struct TimerRow {
            id: String,
            rule_id: String,
            started_at: String,
            deadline_secs: i64,
            expected_payload_type: String,
            scope_task_id: Option<String>,
            scope_correlation_id: Option<String>,
        }

        let rows: Vec<TimerRow> = match sqlx::query_as(
            "SELECT id, rule_id, started_at, deadline_secs, expected_payload_type, scope_task_id, scope_correlation_id FROM trigger_absence_timers"
        )
            .fetch_all(pool)
            .await
        {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!("Failed to load absence timers from DB: {}", e);
                return;
            }
        };

        let mut timers = self.absence_timers.write().await;
        let mut loaded = 0usize;

        for row in rows {
            let rule_id = match Uuid::parse_str(&row.rule_id) {
                Ok(id) => id,
                Err(_) => continue,
            };
            let timer_id = match Uuid::parse_str(&row.id) {
                Ok(id) => id,
                Err(_) => continue,
            };
            let started_at = match chrono::DateTime::parse_from_rfc3339(&row.started_at) {
                Ok(dt) => dt.with_timezone(&Utc),
                Err(_) => continue,
            };

            let timer = AbsenceTimer {
                id: timer_id,
                started_at,
                deadline_secs: row.deadline_secs as u64,
                expected_type: row.expected_payload_type,
                task_id: row.scope_task_id.and_then(|s| Uuid::parse_str(&s).ok()),
                correlation_id: row.scope_correlation_id.and_then(|s| Uuid::parse_str(&s).ok()),
            };

            timers.entry(rule_id).or_default().push(timer);
            loaded += 1;
        }

        if loaded > 0 {
            tracing::info!("Loaded {} pending absence timers from DB", loaded);
        }
    }

    /// Persist an absence timer to DB.
    async fn persist_timer(&self, rule_id: Uuid, timer: &AbsenceTimer) {
        let Some(ref pool) = self.pool else { return };

        if let Err(e) = sqlx::query(
            "INSERT OR REPLACE INTO trigger_absence_timers (id, rule_id, started_at, deadline_secs, expected_payload_type, scope_task_id, scope_correlation_id) VALUES (?, ?, ?, ?, ?, ?, ?)"
        )
            .bind(timer.id.to_string())
            .bind(rule_id.to_string())
            .bind(timer.started_at.to_rfc3339())
            .bind(timer.deadline_secs as i64)
            .bind(&timer.expected_type)
            .bind(timer.task_id.map(|id| id.to_string()))
            .bind(timer.correlation_id.map(|id| id.to_string()))
            .execute(pool)
            .await
        {
            tracing::warn!("Failed to persist absence timer {}: {}", timer.id, e);
        }
    }

    /// Delete an absence timer from DB.
    async fn delete_timer(&self, timer_id: Uuid) {
        let Some(ref pool) = self.pool else { return };

        if let Err(e) = sqlx::query("DELETE FROM trigger_absence_timers WHERE id = ?")
            .bind(timer_id.to_string())
            .execute(pool)
            .await
        {
            tracing::warn!("Failed to delete absence timer {}: {}", timer_id, e);
        }
    }

    /// Delete multiple absence timers from DB by their IDs.
    async fn delete_timers(&self, timer_ids: &[Uuid]) {
        for id in timer_ids {
            self.delete_timer(*id).await;
        }
    }

    /// Load rules (e.g. from database at startup).
    pub async fn load_rules(&self, rules: Vec<TriggerRule>) {
        let mut store = self.rules.write().await;
        *store = rules;
    }

    /// Add a single rule.
    pub async fn add_rule(&self, rule: TriggerRule) {
        let rule_id = rule.id;
        let rule_name = rule.name.clone();
        let mut store = self.rules.write().await;
        store.push(rule);
        drop(store);

        // Emit lifecycle event
        if let Some(ref bus) = self.event_bus {
            let event = UnifiedEvent {
                id: EventId::new(),
                sequence: SequenceNumber(0),
                timestamp: Utc::now(),
                severity: EventSeverity::Info,
                category: EventCategory::Orchestrator,
                goal_id: None,
                task_id: None,
                correlation_id: None,
                source_process_id: None,
                payload: EventPayload::TriggerRuleCreated {
                    rule_id,
                    rule_name,
                },
            };
            bus.publish(event).await;
        }
    }

    /// List current rules (snapshot).
    pub async fn list_rules(&self) -> Vec<TriggerRule> {
        self.rules.read().await.clone()
    }

    /// Enable or disable a rule by ID.
    pub async fn set_enabled(&self, rule_id: Uuid, enabled: bool) -> bool {
        let mut store = self.rules.write().await;
        let result = if let Some(rule) = store.iter_mut().find(|r| r.id == rule_id) {
            rule.enabled = enabled;
            Some(rule.name.clone())
        } else {
            None
        };
        drop(store);

        if let Some(rule_name) = result {
            // Emit lifecycle event
            if let Some(ref bus) = self.event_bus {
                let event = UnifiedEvent {
                    id: EventId::new(),
                    sequence: SequenceNumber(0),
                    timestamp: Utc::now(),
                    severity: EventSeverity::Info,
                    category: EventCategory::Orchestrator,
                    goal_id: None,
                    task_id: None,
                    correlation_id: None,
                    source_process_id: None,
                    payload: EventPayload::TriggerRuleToggled {
                        rule_id,
                        rule_name,
                        enabled,
                    },
                };
                bus.publish(event).await;
            }
            true
        } else {
            false
        }
    }

    /// Evaluate all rules against an event; returns events to emit.
    ///
    /// Commands are dispatched inline via the `CommandBus`.
    async fn evaluate(
        &self,
        event: &UnifiedEvent,
    ) -> Vec<UnifiedEvent> {
        let mut reactions = Vec::new();
        let mut fired_rules: Vec<TriggerRule> = Vec::new();

        let event_type = event.payload.variant_name().to_string();

        // Phase 0: Check if this event cancels any pending absence timers
        let cancelled_timer_ids: Vec<Uuid>;
        {
            let mut timers = self.absence_timers.write().await;
            let mut cancelled = Vec::new();
            for timer_list in timers.values_mut() {
                timer_list.retain(|timer| {
                    // Cancel timer if the expected event arrived (scoped by task_id)
                    let type_match = timer.expected_type == event_type;
                    let scope_match = timer.task_id.is_none()
                        || timer.task_id == event.task_id;
                    if type_match && scope_match {
                        cancelled.push(timer.id);
                        false
                    } else {
                        true
                    }
                });
            }
            // Remove empty entries
            timers.retain(|_, v| !v.is_empty());
            cancelled_timer_ids = cancelled;
        }
        // Delete cancelled timers from DB
        if !cancelled_timer_ids.is_empty() {
            self.delete_timers(&cancelled_timer_ids).await;
        }

        // Phase 0b: Check for expired absence timers and fire rules
        let expired_timer_ids: Vec<Uuid>;
        {
            let now = Utc::now();
            let mut timers = self.absence_timers.write().await;
            let mut rules = self.rules.write().await;
            let mut expired_ids = Vec::new();

            for rule in rules.iter_mut() {
                if !rule.enabled {
                    continue;
                }

                if let Some(timer_list) = timers.get_mut(&rule.id) {
                    let mut expired_indices = Vec::new();
                    for (i, timer) in timer_list.iter().enumerate() {
                        let deadline = timer.started_at + chrono::Duration::seconds(timer.deadline_secs as i64);
                        if now > deadline {
                            expired_indices.push(i);
                        }
                    }

                    // Fire for each expired timer
                    for &i in expired_indices.iter().rev() {
                        let expired = timer_list.remove(i);
                        expired_ids.push(expired.id);

                        // Cooldown check
                        let cooldown_ok = match rule.cooldown {
                            Some(cooldown) => match rule.last_fired {
                                Some(last) => (now - last).to_std().unwrap_or_default() >= cooldown,
                                None => true,
                            },
                            None => true,
                        };

                        if !cooldown_ok {
                            continue;
                        }

                        rule.last_fired = Some(now);
                        rule.fire_count += 1;

                        tracing::info!(
                            rule_name = %rule.name,
                            fire_count = rule.fire_count,
                            expected_type = %expired.expected_type,
                            "Trigger rule fired (absence deadline expired)"
                        );

                        // Build a synthetic event for context
                        let synthetic = UnifiedEvent {
                            id: EventId::new(),
                            sequence: SequenceNumber(0),
                            timestamp: now,
                            severity: EventSeverity::Warning,
                            category: EventCategory::Orchestrator,
                            goal_id: None,
                            task_id: expired.task_id,
                            correlation_id: expired.correlation_id,
                            source_process_id: None,
                            payload: EventPayload::ScheduledEventFired {
                                schedule_id: Uuid::new_v4(),
                                name: format!("absence-timeout:{}", rule.name),
                            },
                        };

                        match &rule.action {
                            TriggerAction::EmitEvent { payload, category, severity } => {
                                reactions.push(self.build_event(payload, *category, *severity, &synthetic));
                            }
                            TriggerAction::IssueCommand { command } => {
                                self.dispatch_command(command, &rule.name).await;
                            }
                            TriggerAction::EmitAndCommand { payload, category, severity, command } => {
                                reactions.push(self.build_event(payload, *category, *severity, &synthetic));
                                self.dispatch_command(command, &rule.name).await;
                            }
                        }

                        fired_rules.push(rule.clone());
                    }
                }
            }
            expired_timer_ids = expired_ids;
        }
        // Delete expired timers from DB
        if !expired_timer_ids.is_empty() {
            self.delete_timers(&expired_timer_ids).await;
        }

        // Phase 1: evaluate rules under write lock, collect fired rules
        let mut new_timers: Vec<(Uuid, AbsenceTimer)> = Vec::new();
        {
            let mut rules = self.rules.write().await;
            let mut windows = self.event_windows.write().await;
            let now = Utc::now();

            for rule in rules.iter_mut() {
                if !rule.enabled {
                    continue;
                }

                // 1. Filter match
                if !rule.filter.matches(event) {
                    continue;
                }

                // 2. Cooldown check
                if let Some(cooldown) = rule.cooldown {
                    if let Some(last) = rule.last_fired {
                        let elapsed = (now - last).to_std().unwrap_or_default();
                        if elapsed < cooldown {
                            continue;
                        }
                    }
                }

                // 3. Condition check
                let condition_met = match &rule.condition {
                    TriggerCondition::Always => true,
                    TriggerCondition::CountThreshold { count, window_secs } => {
                        let window = windows.entry(rule.id).or_default();
                        window.push_back(now);

                        // Evict stale entries
                        let cutoff = now - chrono::Duration::seconds(*window_secs as i64);
                        while window.front().map(|t| *t < cutoff).unwrap_or(false) {
                            window.pop_front();
                        }

                        window.len() >= *count as usize
                    }
                    TriggerCondition::Absence { trigger_type, expected_type, deadline_secs } => {
                        // If the current event matches the trigger_type, start a timer
                        if event_type == *trigger_type {
                            let timer = AbsenceTimer {
                                id: Uuid::new_v4(),
                                started_at: now,
                                deadline_secs: *deadline_secs,
                                expected_type: expected_type.clone(),
                                task_id: event.task_id,
                                correlation_id: event.correlation_id,
                            };
                            new_timers.push((rule.id, timer));
                        }
                        // Absence conditions never fire immediately — they fire on timeout
                        false
                    }
                };

                if !condition_met {
                    continue;
                }

                // 4. Fire!
                rule.last_fired = Some(now);
                rule.fire_count += 1;

                tracing::info!(
                    rule_name = %rule.name,
                    fire_count = rule.fire_count,
                    "Trigger rule fired"
                );

                // Reset window after firing (for CountThreshold)
                if matches!(rule.condition, TriggerCondition::CountThreshold { .. }) {
                    windows.remove(&rule.id);
                }

                match &rule.action {
                    TriggerAction::EmitEvent {
                        payload,
                        category,
                        severity,
                    } => {
                        reactions.push(self.build_event(payload, *category, *severity, event));
                    }
                    TriggerAction::IssueCommand { command } => {
                        self.dispatch_command(command, &rule.name).await;
                    }
                    TriggerAction::EmitAndCommand {
                        payload,
                        category,
                        severity,
                        command,
                    } => {
                        reactions.push(self.build_event(payload, *category, *severity, event));
                        self.dispatch_command(command, &rule.name).await;
                    }
                }

                fired_rules.push(rule.clone());
            }
        } // locks dropped

        // Phase 1b: insert new absence timers into in-memory map and persist to DB
        if !new_timers.is_empty() {
            let mut timers = self.absence_timers.write().await;
            for (rule_id, timer) in &new_timers {
                timers.entry(*rule_id).or_default().push(timer.clone());
            }
            drop(timers);
            for (rule_id, timer) in &new_timers {
                self.persist_timer(*rule_id, timer).await;
            }
        }

        // Phase 2: persist fire state for rules that fired (outside of lock)
        if let Some(ref repo) = self.rule_repo {
            for rule in &fired_rules {
                if let Err(e) = repo.update(rule).await {
                    tracing::warn!(
                        rule_name = %rule.name,
                        error = %e,
                        "Failed to persist trigger rule fire state"
                    );
                }
            }
        }

        reactions
    }

    fn build_event(
        &self,
        payload: &TriggerEventPayload,
        category: EventCategory,
        severity: EventSeverity,
        source_event: &UnifiedEvent,
    ) -> UnifiedEvent {
        UnifiedEvent {
            id: EventId::new(),
            sequence: SequenceNumber(0), // assigned by EventBus
            timestamp: Utc::now(),
            severity,
            category,
            goal_id: source_event.goal_id,
            task_id: source_event.task_id,
            correlation_id: source_event.correlation_id,
            source_process_id: None,
            payload: payload.to_event_payload(),
        }
    }

    async fn dispatch_command(&self, cmd: &SerializableDomainCommand, rule_name: &str) {
        use crate::domain::models::{GoalStatus, MemoryTier, MemoryType, TaskPriority, TaskSource};
        use crate::services::command_bus::{GoalCommand, MemoryCommand, TaskCommand};

        let domain_cmd = match cmd {
            SerializableDomainCommand::StoreMemory {
                key,
                content,
                namespace,
                tier,
                memory_type,
            } => {
                let tier = MemoryTier::from_str(tier).unwrap_or(MemoryTier::Episodic);
                let mtype = MemoryType::from_str(memory_type).unwrap_or(MemoryType::Fact);
                DomainCommand::Memory(MemoryCommand::Store {
                    key: key.clone(),
                    content: content.clone(),
                    namespace: namespace.clone(),
                    tier,
                    memory_type: mtype,
                    metadata: None,
                })
            }
            SerializableDomainCommand::PromoteMemory { memory_id } => {
                // Recall triggers auto-promotion; for explicit promote we recall.
                DomainCommand::Memory(MemoryCommand::Recall { id: *memory_id })
            }
            SerializableDomainCommand::TransitionGoalStatus {
                goal_id,
                new_status,
            } => {
                let status = GoalStatus::from_str(new_status).unwrap_or(GoalStatus::Paused);
                DomainCommand::Goal(GoalCommand::TransitionStatus {
                    goal_id: *goal_id,
                    new_status: status,
                })
            }
            SerializableDomainCommand::CancelTask { task_id, reason } => {
                DomainCommand::Task(TaskCommand::Cancel {
                    task_id: *task_id,
                    reason: reason.clone(),
                })
            }
            SerializableDomainCommand::RetryTask { task_id } => {
                DomainCommand::Task(TaskCommand::Retry { task_id: *task_id })
            }
            SerializableDomainCommand::SubmitTask {
                title,
                description,
                priority,
                agent_type,
            } => {
                let priority = TaskPriority::from_str(priority).unwrap_or(TaskPriority::Normal);
                DomainCommand::Task(TaskCommand::Submit {
                    title: Some(title.clone()),
                    description: description.clone(),
                    parent_id: None,
                    priority,
                    agent_type: agent_type.clone(),
                    depends_on: vec![],
                    context: Box::new(None),
                    idempotency_key: None,
                    source: TaskSource::System,
                    deadline: None,
                })
            }
            SerializableDomainCommand::PauseGoal { goal_id } => {
                DomainCommand::Goal(GoalCommand::TransitionStatus {
                    goal_id: *goal_id,
                    new_status: GoalStatus::Paused,
                })
            }
            SerializableDomainCommand::ForgetMemory { memory_id } => {
                DomainCommand::Memory(MemoryCommand::Forget { id: *memory_id })
            }
            SerializableDomainCommand::RunMemoryMaintenance => {
                DomainCommand::Memory(MemoryCommand::RunMaintenance)
            }
        };

        let envelope = CommandEnvelope::new(
            CommandSource::EventHandler(rule_name.to_string()),
            domain_cmd,
        );

        if let Err(e) = self.command_bus.dispatch(envelope).await {
            tracing::warn!(
                rule_name = %rule_name,
                error = %e,
                "Trigger rule command dispatch failed"
            );
        }
    }
}

#[async_trait]
impl EventHandler for TriggerRuleEngine {
    fn metadata(&self) -> HandlerMetadata {
        HandlerMetadata {
            id: HandlerId::new(),
            name: "TriggerRuleEngine".to_string(),
            filter: EventFilter::new(), // match everything; rules do their own filtering
            priority: HandlerPriority::NORMAL,
            error_strategy: ErrorStrategy::LogAndContinue,
        }
    }

    async fn handle(
        &self,
        event: &UnifiedEvent,
        _ctx: &HandlerContext,
    ) -> Result<Reaction, String> {
        let events = self.evaluate(event).await;
        if events.is_empty() {
            Ok(Reaction::None)
        } else {
            Ok(Reaction::EmitEvents(events))
        }
    }
}

// ---------------------------------------------------------------------------
// Built-in rules
// ---------------------------------------------------------------------------

/// Create the default set of built-in trigger rules.
pub fn builtin_trigger_rules() -> Vec<TriggerRule> {
    vec![
        // semantic-memory-goal-eval: when a semantic memory is stored, trigger goal evaluation
        TriggerRule::new(
            "semantic-memory-goal-eval",
            SerializableEventFilter {
                categories: vec![EventCategory::Memory],
                min_severity: None,
                payload_types: vec!["MemoryStored".to_string()],
                goal_id: None,
                task_id: None,
            },
            TriggerAction::EmitEvent {
                payload: TriggerEventPayload::ScheduledEventFired {
                    name: "goal-evaluation".to_string(),
                },
                category: EventCategory::Scheduler,
                severity: EventSeverity::Debug,
            },
        )
        .with_description("Trigger goal evaluation when a semantic memory is stored")
        .with_cooldown(60),

        // high-failure-pause: pause goal after 5 task failures in 5 minutes
        TriggerRule::new(
            "high-failure-pause",
            SerializableEventFilter {
                categories: vec![EventCategory::Task],
                min_severity: None,
                payload_types: vec!["TaskFailed".to_string()],
                goal_id: None,
                task_id: None,
            },
            TriggerAction::EmitEvent {
                payload: TriggerEventPayload::HumanEscalation {
                    reason: "High task failure rate detected — consider pausing the goal".to_string(),
                },
                category: EventCategory::Escalation,
                severity: EventSeverity::Warning,
            },
        )
        .with_description("Escalate when 5 tasks fail within 5 minutes")
        .with_condition(TriggerCondition::CountThreshold {
            count: 5,
            window_secs: 300,
        }),

        // batch-promotion: promote frequently accessed memories
        TriggerRule::new(
            "batch-promotion",
            SerializableEventFilter {
                categories: vec![EventCategory::Memory],
                min_severity: None,
                payload_types: vec!["MemoryAccessed".to_string()],
                goal_id: None,
                task_id: None,
            },
            TriggerAction::EmitEvent {
                payload: TriggerEventPayload::ScheduledEventFired {
                    name: "memory-maintenance".to_string(),
                },
                category: EventCategory::Scheduler,
                severity: EventSeverity::Debug,
            },
        )
        .with_description("Trigger memory maintenance after 10 accesses in an hour")
        .with_condition(TriggerCondition::CountThreshold {
            count: 10,
            window_secs: 3600,
        }),

        // conflict-escalation: escalate unresolvable memory conflicts
        TriggerRule::new(
            "conflict-escalation",
            SerializableEventFilter {
                categories: vec![EventCategory::Memory],
                min_severity: None,
                payload_types: vec!["MemoryConflictDetected".to_string()],
                goal_id: None,
                task_id: None,
            },
            TriggerAction::EmitEvent {
                payload: TriggerEventPayload::HumanEscalation {
                    reason: "Memory conflict detected that may require human review".to_string(),
                },
                category: EventCategory::Escalation,
                severity: EventSeverity::Warning,
            },
        )
        .with_description("Escalate memory conflicts for human review")
        .with_cooldown(300),

        // task-completion-timeout: escalate if a claimed task doesn't complete within 30min
        TriggerRule::new(
            "task-completion-timeout",
            SerializableEventFilter {
                categories: vec![EventCategory::Task],
                min_severity: None,
                payload_types: vec!["TaskClaimed".to_string()],
                goal_id: None,
                task_id: None,
            },
            TriggerAction::EmitEvent {
                payload: TriggerEventPayload::HumanEscalation {
                    reason: "Task claimed but not completed within 30 minutes — may be stuck".to_string(),
                },
                category: EventCategory::Escalation,
                severity: EventSeverity::Warning,
            },
        )
        .with_description("Escalate when a claimed task does not complete within 30 minutes")
        .with_condition(TriggerCondition::Absence {
            trigger_type: "TaskClaimed".to_string(),
            expected_type: "TaskCompleted".to_string(),
            deadline_secs: 1800,
        }),

        // goal-progress-timeout: request evaluation if no task completes within 1hr of goal start
        TriggerRule::new(
            "goal-progress-timeout",
            SerializableEventFilter {
                categories: vec![EventCategory::Goal],
                min_severity: None,
                payload_types: vec!["GoalStarted".to_string()],
                goal_id: None,
                task_id: None,
            },
            TriggerAction::EmitEvent {
                payload: TriggerEventPayload::GoalEvaluationRequested { goal_id: None },
                category: EventCategory::Goal,
                severity: EventSeverity::Warning,
            },
        )
        .with_description("Request goal evaluation when no task completes within 1 hour of goal start")
        .with_condition(TriggerCondition::Absence {
            trigger_type: "GoalStarted".to_string(),
            expected_type: "TaskCompleted".to_string(),
            deadline_secs: 3600,
        }),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::event_bus::EventPayload;

    fn make_test_event(payload: EventPayload, category: EventCategory) -> UnifiedEvent {
        UnifiedEvent {
            id: EventId::new(),
            sequence: SequenceNumber(1),
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

    #[test]
    fn test_serializable_filter_matches() {
        let filter = SerializableEventFilter {
            categories: vec![EventCategory::Task],
            min_severity: None,
            payload_types: vec!["TaskCompleted".to_string()],
            goal_id: None,
            task_id: None,
        };

        let event = make_test_event(
            EventPayload::TaskCompleted {
                task_id: Uuid::new_v4(),
                tokens_used: 100,
            },
            EventCategory::Task,
        );
        assert!(filter.matches(&event));

        let wrong_cat = make_test_event(
            EventPayload::TaskCompleted {
                task_id: Uuid::new_v4(),
                tokens_used: 100,
            },
            EventCategory::Memory,
        );
        assert!(!filter.matches(&wrong_cat));
    }

    #[test]
    fn test_trigger_rule_creation() {
        let rules = builtin_trigger_rules();
        assert!(rules.len() >= 6);
        assert!(rules.iter().all(|r| r.enabled));
        assert_eq!(rules[0].name, "semantic-memory-goal-eval");
    }
}
