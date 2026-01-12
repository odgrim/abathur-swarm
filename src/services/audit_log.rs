//! Audit logging service for observability.
//!
//! Records all state changes and autonomous decisions with full rationale.
//! Supports structured querying for post-hoc analysis and debugging.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::sync::Arc;
use tokio::sync::RwLock;
use uuid::Uuid;

/// Configuration for the audit log service.
#[derive(Debug, Clone)]
pub struct AuditLogConfig {
    /// Maximum entries to keep in memory.
    pub max_entries: usize,
    /// Whether to persist entries to database.
    pub persist_to_db: bool,
    /// Log level threshold.
    pub min_level: AuditLevel,
    /// Whether to log decision rationale.
    pub log_rationale: bool,
    /// Whether to redact sensitive data.
    pub redact_sensitive: bool,
}

impl Default for AuditLogConfig {
    fn default() -> Self {
        Self {
            max_entries: 10000,
            persist_to_db: true,
            min_level: AuditLevel::Info,
            log_rationale: true,
            redact_sensitive: true,
        }
    }
}

/// Audit log level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AuditLevel {
    /// Detailed debugging information.
    Debug,
    /// General information about operations.
    Info,
    /// Important decisions or state changes.
    Decision,
    /// Warning conditions.
    Warning,
    /// Error conditions.
    Error,
    /// Critical issues requiring attention.
    Critical,
}

impl AuditLevel {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Debug => "debug",
            Self::Info => "info",
            Self::Decision => "decision",
            Self::Warning => "warning",
            Self::Error => "error",
            Self::Critical => "critical",
        }
    }

    pub fn parse_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "debug" => Some(Self::Debug),
            "info" => Some(Self::Info),
            "decision" => Some(Self::Decision),
            "warning" => Some(Self::Warning),
            "error" => Some(Self::Error),
            "critical" => Some(Self::Critical),
            _ => None,
        }
    }
}

/// Category of audit event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditCategory {
    /// Task-related events.
    Task,
    /// Goal-related events.
    Goal,
    /// Agent-related events.
    Agent,
    /// Memory-related events.
    Memory,
    /// DAG execution events.
    Execution,
    /// System/orchestrator events.
    System,
    /// Security-related events.
    Security,
    /// Configuration changes.
    Config,
}

impl AuditCategory {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Task => "task",
            Self::Goal => "goal",
            Self::Agent => "agent",
            Self::Memory => "memory",
            Self::Execution => "execution",
            Self::System => "system",
            Self::Security => "security",
            Self::Config => "config",
        }
    }
}

/// Type of state change or action.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditAction {
    // Task actions
    TaskCreated,
    TaskStateChanged,
    TaskAssigned,
    TaskCompleted,
    TaskFailed,
    TaskRetried,

    // Goal actions
    GoalCreated,
    GoalUpdated,
    GoalCompleted,
    GoalFailed,
    GoalEvaluated,

    // Agent actions
    AgentSpawned,
    AgentCompleted,
    AgentFailed,
    TemplateCreated,
    TemplateUpdated,
    TemplateRefined,

    // Execution actions
    WaveStarted,
    WaveCompleted,
    DagRestructured,
    ExecutionPaused,
    ExecutionResumed,

    // Memory actions
    MemoryStored,
    MemoryAccessed,
    MemoryPruned,
    MemoryPromoted,

    // System actions
    SwarmStarted,
    SwarmStopped,
    ConfigChanged,
    LimitReached,
    CircuitBreakerTriggered,

    // Security actions
    SecurityViolation,
    AccessDenied,
    AuditReview,

    // Decisions
    AutonomousDecision,
    ExtensionGranted,
    ExtensionDenied,
}

impl AuditAction {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::TaskCreated => "task_created",
            Self::TaskStateChanged => "task_state_changed",
            Self::TaskAssigned => "task_assigned",
            Self::TaskCompleted => "task_completed",
            Self::TaskFailed => "task_failed",
            Self::TaskRetried => "task_retried",
            Self::GoalCreated => "goal_created",
            Self::GoalUpdated => "goal_updated",
            Self::GoalCompleted => "goal_completed",
            Self::GoalFailed => "goal_failed",
            Self::GoalEvaluated => "goal_evaluated",
            Self::AgentSpawned => "agent_spawned",
            Self::AgentCompleted => "agent_completed",
            Self::AgentFailed => "agent_failed",
            Self::TemplateCreated => "template_created",
            Self::TemplateUpdated => "template_updated",
            Self::TemplateRefined => "template_refined",
            Self::WaveStarted => "wave_started",
            Self::WaveCompleted => "wave_completed",
            Self::DagRestructured => "dag_restructured",
            Self::ExecutionPaused => "execution_paused",
            Self::ExecutionResumed => "execution_resumed",
            Self::MemoryStored => "memory_stored",
            Self::MemoryAccessed => "memory_accessed",
            Self::MemoryPruned => "memory_pruned",
            Self::MemoryPromoted => "memory_promoted",
            Self::SwarmStarted => "swarm_started",
            Self::SwarmStopped => "swarm_stopped",
            Self::ConfigChanged => "config_changed",
            Self::LimitReached => "limit_reached",
            Self::CircuitBreakerTriggered => "circuit_breaker_triggered",
            Self::SecurityViolation => "security_violation",
            Self::AccessDenied => "access_denied",
            Self::AuditReview => "audit_review",
            Self::AutonomousDecision => "autonomous_decision",
            Self::ExtensionGranted => "extension_granted",
            Self::ExtensionDenied => "extension_denied",
        }
    }
}

/// Actor that caused the audit event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditActor {
    /// System/orchestrator action.
    System,
    /// Specific agent action.
    Agent { id: Uuid, name: String },
    /// CLI/user action.
    User { identifier: String },
    /// Automated process.
    Daemon { name: String },
    /// External A2A request.
    External { source: String },
}

/// Decision rationale for autonomous decisions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecisionRationale {
    /// The decision that was made.
    pub decision: String,
    /// Why this decision was made.
    pub reasoning: String,
    /// Alternatives that were considered.
    pub alternatives: Vec<String>,
    /// Data/factors that influenced the decision.
    pub factors: Vec<(String, String)>,
    /// Confidence in the decision (0.0 - 1.0).
    pub confidence: f64,
}

impl DecisionRationale {
    pub fn new(decision: impl Into<String>, reasoning: impl Into<String>) -> Self {
        Self {
            decision: decision.into(),
            reasoning: reasoning.into(),
            alternatives: Vec::new(),
            factors: Vec::new(),
            confidence: 1.0,
        }
    }

    pub fn with_alternative(mut self, alt: impl Into<String>) -> Self {
        self.alternatives.push(alt.into());
        self
    }

    pub fn with_factor(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.factors.push((name.into(), value.into()));
        self
    }

    pub fn with_confidence(mut self, confidence: f64) -> Self {
        self.confidence = confidence.clamp(0.0, 1.0);
        self
    }
}

/// A single audit log entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEntry {
    /// Unique entry ID.
    pub id: Uuid,
    /// When the event occurred.
    pub timestamp: DateTime<Utc>,
    /// Log level.
    pub level: AuditLevel,
    /// Event category.
    pub category: AuditCategory,
    /// Action/event type.
    pub action: AuditAction,
    /// Actor that caused the event.
    pub actor: AuditActor,
    /// Related entity ID (task, goal, agent, etc.).
    pub entity_id: Option<Uuid>,
    /// Related entity type.
    pub entity_type: Option<String>,
    /// Human-readable message.
    pub message: String,
    /// Previous state (for state changes).
    pub previous_state: Option<String>,
    /// New state (for state changes).
    pub new_state: Option<String>,
    /// Decision rationale (for autonomous decisions).
    pub rationale: Option<DecisionRationale>,
    /// Additional metadata.
    pub metadata: std::collections::HashMap<String, serde_json::Value>,
}

impl AuditEntry {
    /// Create a new audit entry.
    pub fn new(
        level: AuditLevel,
        category: AuditCategory,
        action: AuditAction,
        actor: AuditActor,
        message: impl Into<String>,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            timestamp: Utc::now(),
            level,
            category,
            action,
            actor,
            entity_id: None,
            entity_type: None,
            message: message.into(),
            previous_state: None,
            new_state: None,
            rationale: None,
            metadata: std::collections::HashMap::new(),
        }
    }

    /// Set the related entity.
    pub fn with_entity(mut self, id: Uuid, entity_type: impl Into<String>) -> Self {
        self.entity_id = Some(id);
        self.entity_type = Some(entity_type.into());
        self
    }

    /// Set state transition.
    pub fn with_state_change(
        mut self,
        previous: impl Into<String>,
        new: impl Into<String>,
    ) -> Self {
        self.previous_state = Some(previous.into());
        self.new_state = Some(new.into());
        self
    }

    /// Set decision rationale.
    pub fn with_rationale(mut self, rationale: DecisionRationale) -> Self {
        self.rationale = Some(rationale);
        self
    }

    /// Add metadata.
    pub fn with_metadata(mut self, key: impl Into<String>, value: serde_json::Value) -> Self {
        self.metadata.insert(key.into(), value);
        self
    }
}

/// Filter for querying audit logs.
#[derive(Debug, Clone, Default)]
pub struct AuditFilter {
    /// Filter by minimum level.
    pub min_level: Option<AuditLevel>,
    /// Filter by category.
    pub category: Option<AuditCategory>,
    /// Filter by action.
    pub action: Option<AuditAction>,
    /// Filter by entity ID.
    pub entity_id: Option<Uuid>,
    /// Filter by time range start.
    pub from: Option<DateTime<Utc>>,
    /// Filter by time range end.
    pub to: Option<DateTime<Utc>>,
    /// Limit results.
    pub limit: Option<usize>,
}

impl AuditFilter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_min_level(mut self, level: AuditLevel) -> Self {
        self.min_level = Some(level);
        self
    }

    pub fn with_category(mut self, category: AuditCategory) -> Self {
        self.category = Some(category);
        self
    }

    pub fn with_action(mut self, action: AuditAction) -> Self {
        self.action = Some(action);
        self
    }

    pub fn with_entity(mut self, id: Uuid) -> Self {
        self.entity_id = Some(id);
        self
    }

    pub fn with_time_range(mut self, from: DateTime<Utc>, to: DateTime<Utc>) -> Self {
        self.from = Some(from);
        self.to = Some(to);
        self
    }

    pub fn with_limit(mut self, limit: usize) -> Self {
        self.limit = Some(limit);
        self
    }

    /// Check if an entry matches this filter.
    pub fn matches(&self, entry: &AuditEntry) -> bool {
        if let Some(min_level) = self.min_level {
            if entry.level < min_level {
                return false;
            }
        }

        if let Some(ref category) = self.category {
            if &entry.category != category {
                return false;
            }
        }

        if let Some(ref action) = self.action {
            if &entry.action != action {
                return false;
            }
        }

        if let Some(entity_id) = self.entity_id {
            if entry.entity_id != Some(entity_id) {
                return false;
            }
        }

        if let Some(from) = self.from {
            if entry.timestamp < from {
                return false;
            }
        }

        if let Some(to) = self.to {
            if entry.timestamp > to {
                return false;
            }
        }

        true
    }
}

/// Statistics about the audit log.
#[derive(Debug, Clone, Default, Serialize)]
pub struct AuditStats {
    pub total_entries: usize,
    pub by_level: std::collections::HashMap<String, usize>,
    pub by_category: std::collections::HashMap<String, usize>,
    pub oldest_entry: Option<DateTime<Utc>>,
    pub newest_entry: Option<DateTime<Utc>>,
    pub decisions_logged: usize,
}

/// In-memory audit log service.
pub struct AuditLogService {
    config: AuditLogConfig,
    entries: Arc<RwLock<VecDeque<AuditEntry>>>,
}

impl AuditLogService {
    /// Create a new audit log service.
    pub fn new(config: AuditLogConfig) -> Self {
        Self {
            config,
            entries: Arc::new(RwLock::new(VecDeque::new())),
        }
    }

    /// Create with default configuration.
    pub fn with_defaults() -> Self {
        Self::new(AuditLogConfig::default())
    }

    /// Log an audit entry.
    pub async fn log(&self, entry: AuditEntry) {
        // Check level threshold
        if entry.level < self.config.min_level {
            return;
        }

        let mut entries = self.entries.write().await;

        // Enforce max entries
        while entries.len() >= self.config.max_entries {
            entries.pop_front();
        }

        entries.push_back(entry);
    }

    /// Log a simple info event.
    pub async fn info(&self, category: AuditCategory, action: AuditAction, message: impl Into<String>) {
        self.log(AuditEntry::new(
            AuditLevel::Info,
            category,
            action,
            AuditActor::System,
            message,
        ))
        .await;
    }

    /// Log a decision with rationale.
    pub async fn log_decision(
        &self,
        category: AuditCategory,
        actor: AuditActor,
        message: impl Into<String>,
        rationale: DecisionRationale,
    ) {
        if !self.config.log_rationale {
            return;
        }

        self.log(
            AuditEntry::new(
                AuditLevel::Decision,
                category,
                AuditAction::AutonomousDecision,
                actor,
                message,
            )
            .with_rationale(rationale),
        )
        .await;
    }

    /// Log a state change.
    pub async fn log_state_change(
        &self,
        category: AuditCategory,
        action: AuditAction,
        actor: AuditActor,
        entity_id: Uuid,
        entity_type: impl Into<String>,
        previous_state: impl Into<String>,
        new_state: impl Into<String>,
    ) {
        let prev = previous_state.into();
        let new = new_state.into();
        self.log(
            AuditEntry::new(
                AuditLevel::Info,
                category,
                action,
                actor,
                format!("State changed from {} to {}", prev, new),
            )
            .with_entity(entity_id, entity_type)
            .with_state_change(prev, new),
        )
        .await;
    }

    /// Log a warning.
    pub async fn warn(&self, category: AuditCategory, action: AuditAction, message: impl Into<String>) {
        self.log(AuditEntry::new(
            AuditLevel::Warning,
            category,
            action,
            AuditActor::System,
            message,
        ))
        .await;
    }

    /// Log an error.
    pub async fn error(&self, category: AuditCategory, action: AuditAction, message: impl Into<String>) {
        self.log(AuditEntry::new(
            AuditLevel::Error,
            category,
            action,
            AuditActor::System,
            message,
        ))
        .await;
    }

    /// Query audit entries.
    pub async fn query(&self, filter: AuditFilter) -> Vec<AuditEntry> {
        let entries = self.entries.read().await;
        let mut results: Vec<AuditEntry> = entries
            .iter()
            .filter(|e| filter.matches(e))
            .cloned()
            .collect();

        // Sort by timestamp descending (newest first)
        results.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));

        // Apply limit
        if let Some(limit) = filter.limit {
            results.truncate(limit);
        }

        results
    }

    /// Get entries for a specific entity.
    pub async fn get_entity_history(&self, entity_id: Uuid) -> Vec<AuditEntry> {
        self.query(AuditFilter::new().with_entity(entity_id)).await
    }

    /// Get recent decisions.
    pub async fn get_recent_decisions(&self, limit: usize) -> Vec<AuditEntry> {
        self.query(
            AuditFilter::new()
                .with_min_level(AuditLevel::Decision)
                .with_limit(limit),
        )
        .await
    }

    /// Get statistics.
    pub async fn stats(&self) -> AuditStats {
        let entries = self.entries.read().await;

        let mut by_level: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
        let mut by_category: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
        let mut decisions_logged = 0;

        for entry in entries.iter() {
            *by_level.entry(entry.level.as_str().to_string()).or_default() += 1;
            *by_category.entry(entry.category.as_str().to_string()).or_default() += 1;
            if entry.rationale.is_some() {
                decisions_logged += 1;
            }
        }

        AuditStats {
            total_entries: entries.len(),
            by_level,
            by_category,
            oldest_entry: entries.front().map(|e| e.timestamp),
            newest_entry: entries.back().map(|e| e.timestamp),
            decisions_logged,
        }
    }

    /// Clear all entries.
    pub async fn clear(&self) {
        let mut entries = self.entries.write().await;
        entries.clear();
    }

    /// Export entries as JSON.
    pub async fn export_json(&self, filter: AuditFilter) -> String {
        let entries = self.query(filter).await;
        serde_json::to_string_pretty(&entries).unwrap_or_default()
    }
}

/// Helper to create system actor.
pub fn system_actor() -> AuditActor {
    AuditActor::System
}

/// Helper to create agent actor.
pub fn agent_actor(id: Uuid, name: impl Into<String>) -> AuditActor {
    AuditActor::Agent {
        id,
        name: name.into(),
    }
}

/// Helper to create user actor.
pub fn user_actor(identifier: impl Into<String>) -> AuditActor {
    AuditActor::User {
        identifier: identifier.into(),
    }
}

/// Helper to create daemon actor.
pub fn daemon_actor(name: impl Into<String>) -> AuditActor {
    AuditActor::Daemon { name: name.into() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_audit_level_ordering() {
        assert!(AuditLevel::Debug < AuditLevel::Info);
        assert!(AuditLevel::Info < AuditLevel::Decision);
        assert!(AuditLevel::Decision < AuditLevel::Warning);
        assert!(AuditLevel::Warning < AuditLevel::Error);
        assert!(AuditLevel::Error < AuditLevel::Critical);
    }

    #[test]
    fn test_audit_entry_builder() {
        let entry = AuditEntry::new(
            AuditLevel::Info,
            AuditCategory::Task,
            AuditAction::TaskCreated,
            AuditActor::System,
            "Task created",
        )
        .with_entity(Uuid::new_v4(), "task")
        .with_metadata("priority", serde_json::json!("high"));

        assert_eq!(entry.level, AuditLevel::Info);
        assert_eq!(entry.category, AuditCategory::Task);
        assert!(entry.entity_id.is_some());
        assert_eq!(entry.metadata.get("priority"), Some(&serde_json::json!("high")));
    }

    #[test]
    fn test_state_change_entry() {
        let entry = AuditEntry::new(
            AuditLevel::Info,
            AuditCategory::Task,
            AuditAction::TaskStateChanged,
            AuditActor::System,
            "Task state changed",
        )
        .with_state_change("pending", "running");

        assert_eq!(entry.previous_state, Some("pending".to_string()));
        assert_eq!(entry.new_state, Some("running".to_string()));
    }

    #[test]
    fn test_decision_rationale() {
        let rationale = DecisionRationale::new("Grant extension", "Task complexity warrants additional subtasks")
            .with_alternative("Deny extension")
            .with_alternative("Restructure task")
            .with_factor("current_depth", "3")
            .with_factor("subtask_count", "8")
            .with_confidence(0.85);

        assert_eq!(rationale.alternatives.len(), 2);
        assert_eq!(rationale.factors.len(), 2);
        assert!((rationale.confidence - 0.85).abs() < 0.001);
    }

    #[test]
    fn test_filter_matches() {
        let entry = AuditEntry::new(
            AuditLevel::Info,
            AuditCategory::Task,
            AuditAction::TaskCreated,
            AuditActor::System,
            "Test",
        );

        let filter = AuditFilter::new().with_category(AuditCategory::Task);
        assert!(filter.matches(&entry));

        let filter = AuditFilter::new().with_category(AuditCategory::Goal);
        assert!(!filter.matches(&entry));

        let filter = AuditFilter::new().with_min_level(AuditLevel::Warning);
        assert!(!filter.matches(&entry));
    }

    #[tokio::test]
    async fn test_audit_log_service() {
        let service = AuditLogService::with_defaults();

        service
            .info(AuditCategory::Task, AuditAction::TaskCreated, "Task 1 created")
            .await;
        service
            .info(AuditCategory::Task, AuditAction::TaskCompleted, "Task 1 completed")
            .await;
        service
            .info(AuditCategory::Goal, AuditAction::GoalCreated, "Goal created")
            .await;

        let all = service.query(AuditFilter::new()).await;
        assert_eq!(all.len(), 3);

        let tasks = service
            .query(AuditFilter::new().with_category(AuditCategory::Task))
            .await;
        assert_eq!(tasks.len(), 2);
    }

    #[tokio::test]
    async fn test_decision_logging() {
        let service = AuditLogService::with_defaults();

        let rationale = DecisionRationale::new("Accept", "Meets criteria")
            .with_confidence(0.9);

        service
            .log_decision(
                AuditCategory::Execution,
                AuditActor::System,
                "Extension granted",
                rationale,
            )
            .await;

        let decisions = service.get_recent_decisions(10).await;
        assert_eq!(decisions.len(), 1);
        assert!(decisions[0].rationale.is_some());
    }

    #[tokio::test]
    async fn test_max_entries_enforcement() {
        let config = AuditLogConfig {
            max_entries: 5,
            ..Default::default()
        };
        let service = AuditLogService::new(config);

        for i in 0..10 {
            service
                .info(
                    AuditCategory::System,
                    AuditAction::SwarmStarted,
                    format!("Entry {}", i),
                )
                .await;
        }

        let stats = service.stats().await;
        assert_eq!(stats.total_entries, 5);
    }

    #[test]
    fn test_actor_helpers() {
        let sys = system_actor();
        assert!(matches!(sys, AuditActor::System));

        let agent = agent_actor(Uuid::new_v4(), "test-agent");
        assert!(matches!(agent, AuditActor::Agent { .. }));

        let user = user_actor("admin");
        assert!(matches!(user, AuditActor::User { .. }));

        let daemon = daemon_actor("memory-decay");
        assert!(matches!(daemon, AuditActor::Daemon { .. }));
    }
}
