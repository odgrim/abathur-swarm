//! Evolution Loop service for agent template refinement.
//!
//! Implements the evolution paradigm from the design docs:
//! 1. Execute: Run agents on tasks, track outcomes
//! 2. Evaluate: Measure effectiveness based on success rate
//! 3. Evolve: Refine struggling templates, version all changes

use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use uuid::Uuid;

/// Configuration for the evolution loop.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvolutionConfig {
    /// Minimum tasks before evaluation.
    pub min_tasks_for_evaluation: usize,
    /// Success rate threshold for refinement (60%).
    pub refinement_threshold: f64,
    /// Success rate threshold for major refinement (40%).
    pub major_refinement_threshold: f64,
    /// Tasks required for major refinement evaluation.
    pub major_refinement_min_tasks: usize,
    /// Window for detecting regression after version change.
    pub regression_detection_window_hours: i64,
    /// Minimum tasks after version change to detect regression.
    pub regression_min_tasks: usize,
    /// Threshold for regression detection (success rate drop).
    pub regression_threshold: f64,
    /// Whether to automatically revert on regression.
    pub auto_revert_enabled: bool,
}

impl Default for EvolutionConfig {
    fn default() -> Self {
        Self {
            min_tasks_for_evaluation: 5,
            refinement_threshold: 0.60,
            major_refinement_threshold: 0.40,
            major_refinement_min_tasks: 10,
            regression_detection_window_hours: 24,
            regression_min_tasks: 3,
            regression_threshold: 0.15, // 15% drop
            auto_revert_enabled: true,
        }
    }
}

/// Outcome of a task execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TaskOutcome {
    Success,
    Failure,
    GoalViolation,
}

/// Recorded task execution for tracking.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskExecution {
    pub task_id: Uuid,
    pub template_name: String,
    pub template_version: u32,
    pub outcome: TaskOutcome,
    pub executed_at: DateTime<Utc>,
    pub turns_used: u32,
    pub tokens_used: u64,
    pub downstream_tasks: Vec<Uuid>,
}

/// Statistics for an agent template.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemplateStats {
    pub template_name: String,
    pub template_version: u32,
    pub total_tasks: usize,
    pub successful_tasks: usize,
    pub failed_tasks: usize,
    pub goal_violations: usize,
    pub success_rate: f64,
    pub avg_turns: f64,
    pub avg_tokens: f64,
    pub first_execution: Option<DateTime<Utc>>,
    pub last_execution: Option<DateTime<Utc>>,
}

impl TemplateStats {
    pub fn new(template_name: String, template_version: u32) -> Self {
        Self {
            template_name,
            template_version,
            total_tasks: 0,
            successful_tasks: 0,
            failed_tasks: 0,
            goal_violations: 0,
            success_rate: 0.0,
            avg_turns: 0.0,
            avg_tokens: 0.0,
            first_execution: None,
            last_execution: None,
        }
    }

    fn update(&mut self, execution: &TaskExecution) {
        self.total_tasks += 1;

        match execution.outcome {
            TaskOutcome::Success => self.successful_tasks += 1,
            TaskOutcome::Failure => self.failed_tasks += 1,
            TaskOutcome::GoalViolation => {
                self.failed_tasks += 1;
                self.goal_violations += 1;
            }
        }

        self.success_rate = if self.total_tasks > 0 {
            self.successful_tasks as f64 / self.total_tasks as f64
        } else {
            0.0
        };

        // Update averages
        let n = self.total_tasks as f64;
        self.avg_turns = (self.avg_turns * (n - 1.0) + execution.turns_used as f64) / n;
        self.avg_tokens = (self.avg_tokens * (n - 1.0) + execution.tokens_used as f64) / n;

        if self.first_execution.is_none() {
            self.first_execution = Some(execution.executed_at);
        }
        self.last_execution = Some(execution.executed_at);
    }
}

/// Evolution trigger type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EvolutionTrigger {
    /// Success rate below refinement threshold.
    LowSuccessRate,
    /// Success rate below major refinement threshold.
    VeryLowSuccessRate,
    /// Goal violations detected.
    GoalViolations,
    /// Downstream impact degraded.
    DownstreamImpact,
    /// Regression after version change.
    Regression,
}

/// Evolution event for audit/logging.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvolutionEvent {
    pub id: Uuid,
    pub template_name: String,
    pub template_version: u32,
    pub trigger: EvolutionTrigger,
    pub stats_at_trigger: TemplateStats,
    pub action_taken: EvolutionAction,
    pub occurred_at: DateTime<Utc>,
}

/// Action taken by the evolution loop.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum EvolutionAction {
    /// Template flagged for refinement.
    FlaggedForRefinement { severity: RefinementSeverity },
    /// Automatic reversion to previous version.
    Reverted { from_version: u32, to_version: u32 },
    /// No action taken (informational).
    NoAction { reason: String },
}

/// Severity of refinement needed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RefinementSeverity {
    Minor,
    Major,
    Immediate,
}

/// Refinement request for the Meta-Planner.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RefinementRequest {
    pub id: Uuid,
    pub template_name: String,
    pub template_version: u32,
    pub severity: RefinementSeverity,
    pub trigger: EvolutionTrigger,
    pub stats: TemplateStats,
    pub failed_task_ids: Vec<Uuid>,
    pub created_at: DateTime<Utc>,
    pub status: RefinementStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RefinementStatus {
    Pending,
    InProgress,
    Completed,
    Failed,
}

impl RefinementRequest {
    pub fn new(
        template_name: String,
        template_version: u32,
        severity: RefinementSeverity,
        trigger: EvolutionTrigger,
        stats: TemplateStats,
        failed_task_ids: Vec<Uuid>,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            template_name,
            template_version,
            severity,
            trigger,
            stats,
            failed_task_ids,
            created_at: Utc::now(),
            status: RefinementStatus::Pending,
        }
    }
}

/// Repository trait for persisting refinement requests across process restarts.
///
/// Defined here (not in domain/ports) to avoid circular imports: adapters can
/// implement this trait by importing from `services::evolution_loop` without
/// creating a dependency cycle.
#[async_trait]
pub trait RefinementRepository: Send + Sync {
    /// Persist a new refinement request.
    async fn create(
        &self,
        request: &RefinementRequest,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;

    /// Retrieve all requests with status = Pending.
    async fn get_pending(
        &self,
    ) -> Result<Vec<RefinementRequest>, Box<dyn std::error::Error + Send + Sync>>;

    /// Update the status of a refinement request by ID.
    async fn update_status(
        &self,
        id: Uuid,
        status: RefinementStatus,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;

    /// Reset all InProgress requests back to Pending.
    ///
    /// Returns the requests that were reset (previously InProgress).
    /// Used during startup reconciliation to recover refinements interrupted
    /// by a process restart.
    async fn reset_in_progress_to_pending(
        &self,
    ) -> Result<Vec<RefinementRequest>, Box<dyn std::error::Error + Send + Sync>>;
}

/// In-memory state for the evolution loop.
struct EvolutionState {
    /// Task executions by template name.
    executions: HashMap<String, Vec<TaskExecution>>,
    /// Computed stats by template name.
    stats: HashMap<String, TemplateStats>,
    /// Pending refinement requests.
    refinement_queue: Vec<RefinementRequest>,
    /// Evolution events for audit.
    events: Vec<EvolutionEvent>,
    /// Previous version stats for regression detection.
    previous_version_stats: HashMap<String, TemplateStats>,
    /// Version change timestamps.
    version_change_times: HashMap<String, (u32, DateTime<Utc>)>,
}

impl EvolutionState {
    fn new() -> Self {
        Self {
            executions: HashMap::new(),
            stats: HashMap::new(),
            refinement_queue: Vec::new(),
            events: Vec::new(),
            previous_version_stats: HashMap::new(),
            version_change_times: HashMap::new(),
        }
    }
}

/// Evolution Loop service.
pub struct EvolutionLoop {
    config: EvolutionConfig,
    state: Arc<RwLock<EvolutionState>>,
    refinement_repo: Option<Arc<dyn RefinementRepository>>,
}

impl EvolutionLoop {
    pub fn new(config: EvolutionConfig) -> Self {
        Self {
            config,
            state: Arc::new(RwLock::new(EvolutionState::new())),
            refinement_repo: None,
        }
    }

    pub fn with_default_config() -> Self {
        Self::new(EvolutionConfig::default())
    }

    /// Attach a persistence repository for refinement requests.
    ///
    /// When set, new requests are persisted on creation and status changes
    /// are written through to the DB. Failures are non-fatal (logged as warnings).
    pub fn with_repo(mut self, repo: Arc<dyn RefinementRepository>) -> Self {
        self.refinement_repo = Some(repo);
        self
    }

    /// Record a task execution.
    pub async fn record_execution(&self, execution: TaskExecution) {
        let mut state = self.state.write().await;

        // Check if we need to handle version change first
        let needs_version_reset = if let Some(stats) = state.stats.get(&execution.template_name) {
            stats.template_version != execution.template_version
        } else {
            false
        };

        if needs_version_reset {
            // Clone previous stats for regression detection
            if let Some(prev_stats) = state.stats.get(&execution.template_name).cloned() {
                state.previous_version_stats.insert(
                    execution.template_name.clone(),
                    prev_stats,
                );
            }
            state.version_change_times.insert(
                execution.template_name.clone(),
                (execution.template_version, Utc::now()),
            );
            // Remove old stats so we can insert fresh ones
            state.stats.remove(&execution.template_name);
        }

        // Update or create stats
        let stats = state
            .stats
            .entry(execution.template_name.clone())
            .or_insert_with(|| {
                TemplateStats::new(
                    execution.template_name.clone(),
                    execution.template_version,
                )
            });

        stats.update(&execution);

        // Store execution
        state
            .executions
            .entry(execution.template_name.clone())
            .or_default()
            .push(execution);
    }

    /// Evaluate templates and trigger evolution if needed.
    pub async fn evaluate(&self) -> Vec<EvolutionEvent> {
        let mut new_requests: Vec<RefinementRequest> = Vec::new();

        let events = {
            let mut state = self.state.write().await;
            let mut events = Vec::new();

            let template_names: Vec<String> = state.stats.keys().cloned().collect();

            for template_name in template_names {
                let stats = match state.stats.get(&template_name) {
                    Some(s) => s.clone(),
                    None => continue,
                };

                // Skip if not enough tasks
                if stats.total_tasks < self.config.min_tasks_for_evaluation {
                    continue;
                }

                let mut trigger = None;
                let mut severity = RefinementSeverity::Minor;

                // Check for goal violations (immediate review)
                if stats.goal_violations > 0 {
                    trigger = Some(EvolutionTrigger::GoalViolations);
                    severity = RefinementSeverity::Immediate;
                }
                // Check for very low success rate
                else if stats.total_tasks >= self.config.major_refinement_min_tasks
                    && stats.success_rate < self.config.major_refinement_threshold
                {
                    trigger = Some(EvolutionTrigger::VeryLowSuccessRate);
                    severity = RefinementSeverity::Major;
                }
                // Check for low success rate
                else if stats.success_rate < self.config.refinement_threshold {
                    trigger = Some(EvolutionTrigger::LowSuccessRate);
                    severity = RefinementSeverity::Minor;
                }

                // Check for regression
                if trigger.is_none()
                    && let Some((new_version, change_time)) =
                        state.version_change_times.get(&template_name)
                        && stats.template_version == *new_version {
                            let window =
                                Duration::hours(self.config.regression_detection_window_hours);
                            let in_window = Utc::now() - *change_time < window;

                            if in_window && stats.total_tasks >= self.config.regression_min_tasks
                                && let Some(prev_stats) =
                                    state.previous_version_stats.get(&template_name)
                                {
                                    let rate_drop = prev_stats.success_rate - stats.success_rate;
                                    if rate_drop >= self.config.regression_threshold {
                                        trigger = Some(EvolutionTrigger::Regression);
                                        severity = RefinementSeverity::Immediate;
                                    }
                                }
                        }

                if let Some(trig) = trigger {
                    let action = if trig == EvolutionTrigger::Regression
                        && self.config.auto_revert_enabled
                    {
                        // Auto-revert
                        if let Some(prev_stats) = state.previous_version_stats.get(&template_name) {
                            EvolutionAction::Reverted {
                                from_version: stats.template_version,
                                to_version: prev_stats.template_version,
                            }
                        } else {
                            EvolutionAction::FlaggedForRefinement { severity }
                        }
                    } else {
                        // Deduplication: skip if a Pending or InProgress refinement
                        // already exists for this template
                        let has_active = state.refinement_queue.iter().any(|r| {
                            r.template_name == template_name
                                && matches!(r.status, RefinementStatus::Pending | RefinementStatus::InProgress)
                        });

                        if has_active {
                            EvolutionAction::NoAction {
                                reason: format!(
                                    "Refinement already pending/in-progress for '{}'",
                                    template_name,
                                ),
                            }
                        } else {
                            // Create refinement request
                            let failed_task_ids = state
                                .executions
                                .get(&template_name)
                                .map(|execs| {
                                    execs
                                        .iter()
                                        .filter(|e| e.outcome != TaskOutcome::Success)
                                        .map(|e| e.task_id)
                                        .collect()
                                })
                                .unwrap_or_default();

                            let request = RefinementRequest::new(
                                template_name.clone(),
                                stats.template_version,
                                severity,
                                trig,
                                stats.clone(),
                                failed_task_ids,
                            );
                            // Collect for persistence outside the write lock
                            new_requests.push(request.clone());
                            state.refinement_queue.push(request);

                            EvolutionAction::FlaggedForRefinement { severity }
                        }
                    };

                    let event = EvolutionEvent {
                        id: Uuid::new_v4(),
                        template_name: template_name.clone(),
                        template_version: stats.template_version,
                        trigger: trig,
                        stats_at_trigger: stats.clone(),
                        action_taken: action,
                        occurred_at: Utc::now(),
                    };

                    state.events.push(event.clone());
                    events.push(event);
                }
            }

            events
        }; // write lock dropped here

        // Persist new requests outside the write lock (non-fatal on failure)
        if let Some(ref repo) = self.refinement_repo {
            for request in &new_requests {
                if let Err(e) = repo.create(request).await {
                    tracing::warn!(
                        "Failed to persist refinement request {} to DB: {}",
                        request.id,
                        e
                    );
                }
            }
        }

        events
    }

    /// Get stats for a template.
    pub async fn get_stats(&self, template_name: &str) -> Option<TemplateStats> {
        let state = self.state.read().await;
        state.stats.get(template_name).cloned()
    }

    /// Get all template stats.
    pub async fn get_all_stats(&self) -> Vec<TemplateStats> {
        let state = self.state.read().await;
        state.stats.values().cloned().collect()
    }

    /// Get pending refinement requests.
    pub async fn get_pending_refinements(&self) -> Vec<RefinementRequest> {
        let state = self.state.read().await;
        state
            .refinement_queue
            .iter()
            .filter(|r| r.status == RefinementStatus::Pending)
            .cloned()
            .collect()
    }

    /// Check if a template has an active (Pending or InProgress) refinement request.
    pub async fn has_active_refinement(&self, template_name: &str) -> bool {
        let state = self.state.read().await;
        state.refinement_queue.iter().any(|r| {
            r.template_name == template_name
                && matches!(r.status, RefinementStatus::Pending | RefinementStatus::InProgress)
        })
    }

    /// Mark a refinement request as in progress.
    pub async fn start_refinement(&self, request_id: Uuid) -> bool {
        let found = {
            let mut state = self.state.write().await;
            let mut found = false;
            for request in &mut state.refinement_queue {
                if request.id == request_id && request.status == RefinementStatus::Pending {
                    request.status = RefinementStatus::InProgress;
                    found = true;
                    break;
                }
            }
            found
        }; // write lock dropped here

        if found
            && let Some(ref repo) = self.refinement_repo
                && let Err(e) = repo.update_status(request_id, RefinementStatus::InProgress).await {
                    tracing::warn!(
                        "Failed to persist InProgress status for refinement {}: {}",
                        request_id,
                        e
                    );
                }

        found
    }

    /// Mark a refinement request as completed.
    pub async fn complete_refinement(&self, request_id: Uuid, success: bool) {
        let new_status = if success {
            RefinementStatus::Completed
        } else {
            RefinementStatus::Failed
        };

        {
            let mut state = self.state.write().await;
            for request in &mut state.refinement_queue {
                if request.id == request_id {
                    request.status = new_status;
                    break;
                }
            }
        } // write lock dropped here

        if let Some(ref repo) = self.refinement_repo
            && let Err(e) = repo.update_status(request_id, new_status).await {
                tracing::warn!(
                    "Failed to persist {} status for refinement {}: {}",
                    if success { "Completed" } else { "Failed" },
                    request_id,
                    e
                );
            }
    }

    /// Record a version change for a template.
    pub async fn record_version_change(&self, template_name: &str, new_version: u32) {
        let mut state = self.state.write().await;

        // Store current stats as previous version
        let prev_stats = state.stats.get(template_name).cloned();
        if let Some(stats) = prev_stats {
            state.previous_version_stats.insert(
                template_name.to_string(),
                stats,
            );
        }

        // Record version change time
        state.version_change_times.insert(
            template_name.to_string(),
            (new_version, Utc::now()),
        );
    }

    /// Load pending refinement requests from the repository into in-memory state.
    ///
    /// Existing in-memory entries are preserved; only new IDs (from the DB) are added.
    /// This is called on startup after `recover_in_progress_refinements()` to hydrate
    /// the in-memory queue from persisted data.
    pub async fn load_from_repo(&self) {
        let Some(ref repo) = self.refinement_repo else {
            return;
        };

        match repo.get_pending().await {
            Ok(requests) => {
                let mut state = self.state.write().await;
                for request in requests {
                    if !state.refinement_queue.iter().any(|r| r.id == request.id) {
                        state.refinement_queue.push(request);
                    }
                }
            }
            Err(e) => {
                tracing::warn!("Failed to load refinement requests from repository: {}", e);
            }
        }
    }

    /// Recover InProgress refinements from a previous process run.
    ///
    /// During startup reconciliation, any refinement that was InProgress when the
    /// process died is reset to Pending in the DB, then all pending requests are
    /// loaded into the in-memory queue so the evolution loop can re-process them.
    pub async fn recover_in_progress_refinements(&self) {
        let Some(ref repo) = self.refinement_repo else {
            return;
        };

        match repo.reset_in_progress_to_pending().await {
            Ok(recovered) if !recovered.is_empty() => {
                tracing::info!(
                    "Startup recovery: reset {} InProgress refinement request(s) to Pending",
                    recovered.len()
                );
            }
            Ok(_) => {}
            Err(e) => {
                tracing::warn!(
                    "Failed to recover InProgress refinements on startup: {}",
                    e
                );
            }
        }

        // Load all pending (including any just-recovered ones) into memory
        self.load_from_repo().await;
    }

    /// Get evolution events for audit.
    pub async fn get_events(&self, limit: Option<usize>) -> Vec<EvolutionEvent> {
        let state = self.state.read().await;
        let events: Vec<_> = state.events.iter().rev().cloned().collect();
        match limit {
            Some(n) => events.into_iter().take(n).collect(),
            None => events,
        }
    }

    /// Get templates needing attention (sorted by urgency).
    pub async fn get_templates_needing_attention(&self) -> Vec<(String, RefinementSeverity)> {
        let state = self.state.read().await;
        let mut result: Vec<_> = state
            .refinement_queue
            .iter()
            .filter(|r| r.status == RefinementStatus::Pending)
            .map(|r| (r.template_name.clone(), r.severity))
            .collect();

        // Sort by severity (Immediate > Major > Minor)
        result.sort_by_key(|(_, s)| match s {
            RefinementSeverity::Immediate => 0,
            RefinementSeverity::Major => 1,
            RefinementSeverity::Minor => 2,
        });

        result
    }

    /// Clear all state (for testing).
    #[cfg(test)]
    pub async fn clear(&self) {
        let mut state = self.state.write().await;
        state.executions.clear();
        state.stats.clear();
        state.refinement_queue.clear();
        state.events.clear();
        state.previous_version_stats.clear();
        state.version_change_times.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_execution(
        template_name: &str,
        version: u32,
        outcome: TaskOutcome,
    ) -> TaskExecution {
        TaskExecution {
            task_id: Uuid::new_v4(),
            template_name: template_name.to_string(),
            template_version: version,
            outcome,
            executed_at: Utc::now(),
            turns_used: 10,
            tokens_used: 1000,
            downstream_tasks: vec![],
        }
    }

    #[tokio::test]
    async fn test_record_execution() {
        let evolution = EvolutionLoop::with_default_config();

        evolution
            .record_execution(make_execution("test-agent", 1, TaskOutcome::Success))
            .await;
        evolution
            .record_execution(make_execution("test-agent", 1, TaskOutcome::Success))
            .await;
        evolution
            .record_execution(make_execution("test-agent", 1, TaskOutcome::Failure))
            .await;

        let stats = evolution.get_stats("test-agent").await.unwrap();
        assert_eq!(stats.total_tasks, 3);
        assert_eq!(stats.successful_tasks, 2);
        assert_eq!(stats.failed_tasks, 1);
        assert!((stats.success_rate - 0.666).abs() < 0.01);
    }

    #[tokio::test]
    async fn test_low_success_rate_trigger() {
        let config = EvolutionConfig {
            min_tasks_for_evaluation: 2,
            refinement_threshold: 0.60,
            major_refinement_threshold: 0.40,
            major_refinement_min_tasks: 3, // Lower threshold for test
            ..Default::default()
        };
        let evolution = EvolutionLoop::new(config);

        // 1 success, 3 failures = 25% success rate
        evolution
            .record_execution(make_execution("test-agent", 1, TaskOutcome::Success))
            .await;
        evolution
            .record_execution(make_execution("test-agent", 1, TaskOutcome::Failure))
            .await;
        evolution
            .record_execution(make_execution("test-agent", 1, TaskOutcome::Failure))
            .await;
        evolution
            .record_execution(make_execution("test-agent", 1, TaskOutcome::Failure))
            .await;

        let events = evolution.evaluate().await;
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].trigger, EvolutionTrigger::VeryLowSuccessRate);
    }

    #[tokio::test]
    async fn test_goal_violation_trigger() {
        let config = EvolutionConfig {
            min_tasks_for_evaluation: 1,
            ..Default::default()
        };
        let evolution = EvolutionLoop::new(config);

        evolution
            .record_execution(make_execution("test-agent", 1, TaskOutcome::GoalViolation))
            .await;

        let events = evolution.evaluate().await;
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].trigger, EvolutionTrigger::GoalViolations);
    }

    #[tokio::test]
    async fn test_version_change_detection() {
        let evolution = EvolutionLoop::with_default_config();

        // Record executions for version 1
        evolution
            .record_execution(make_execution("test-agent", 1, TaskOutcome::Success))
            .await;
        evolution
            .record_execution(make_execution("test-agent", 1, TaskOutcome::Success))
            .await;

        // Change to version 2
        evolution
            .record_execution(make_execution("test-agent", 2, TaskOutcome::Success))
            .await;

        let stats = evolution.get_stats("test-agent").await.unwrap();
        assert_eq!(stats.template_version, 2);
        assert_eq!(stats.total_tasks, 1); // Reset for new version
    }

    #[tokio::test]
    async fn test_refinement_queue() {
        let config = EvolutionConfig {
            min_tasks_for_evaluation: 2,
            refinement_threshold: 0.80,
            ..Default::default()
        };
        let evolution = EvolutionLoop::new(config);

        // 50% success rate (below 80%)
        evolution
            .record_execution(make_execution("test-agent", 1, TaskOutcome::Success))
            .await;
        evolution
            .record_execution(make_execution("test-agent", 1, TaskOutcome::Failure))
            .await;

        evolution.evaluate().await;

        let pending = evolution.get_pending_refinements().await;
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].template_name, "test-agent");
    }

    #[tokio::test]
    async fn test_refinement_lifecycle() {
        let config = EvolutionConfig {
            min_tasks_for_evaluation: 1,
            refinement_threshold: 0.80,
            ..Default::default()
        };
        let evolution = EvolutionLoop::new(config);

        evolution
            .record_execution(make_execution("test-agent", 1, TaskOutcome::Failure))
            .await;
        evolution.evaluate().await;

        let pending = evolution.get_pending_refinements().await;
        let request_id = pending[0].id;

        // Start refinement
        assert!(evolution.start_refinement(request_id).await);

        // Complete refinement
        evolution.complete_refinement(request_id, true).await;

        // Should no longer be pending
        let pending = evolution.get_pending_refinements().await;
        assert!(pending.is_empty());
    }

    #[tokio::test]
    async fn test_events_history() {
        let config = EvolutionConfig {
            min_tasks_for_evaluation: 1,
            ..Default::default()
        };
        let evolution = EvolutionLoop::new(config);

        evolution
            .record_execution(make_execution("agent-a", 1, TaskOutcome::GoalViolation))
            .await;
        evolution
            .record_execution(make_execution("agent-b", 1, TaskOutcome::GoalViolation))
            .await;

        evolution.evaluate().await;

        let events = evolution.get_events(None).await;
        assert_eq!(events.len(), 2);
    }

    #[tokio::test]
    async fn test_refinement_deduplication_pending() {
        let config = EvolutionConfig {
            min_tasks_for_evaluation: 1,
            refinement_threshold: 0.80,
            ..Default::default()
        };
        let evolution = EvolutionLoop::new(config);

        // Record a failure to trigger refinement
        evolution
            .record_execution(make_execution("test-agent", 1, TaskOutcome::Failure))
            .await;

        // First evaluate creates a refinement request
        let events1 = evolution.evaluate().await;
        assert_eq!(events1.len(), 1);
        assert!(matches!(
            events1[0].action_taken,
            EvolutionAction::FlaggedForRefinement { .. }
        ));

        let pending = evolution.get_pending_refinements().await;
        assert_eq!(pending.len(), 1);

        // Record another failure
        evolution
            .record_execution(make_execution("test-agent", 1, TaskOutcome::Failure))
            .await;

        // Second evaluate should NOT create a duplicate refinement
        let events2 = evolution.evaluate().await;
        assert_eq!(events2.len(), 1);
        assert!(matches!(
            events2[0].action_taken,
            EvolutionAction::NoAction { .. }
        ));

        // Still only one pending refinement
        let pending = evolution.get_pending_refinements().await;
        assert_eq!(pending.len(), 1);
    }

    #[tokio::test]
    async fn test_refinement_deduplication_in_progress() {
        let config = EvolutionConfig {
            min_tasks_for_evaluation: 1,
            refinement_threshold: 0.80,
            ..Default::default()
        };
        let evolution = EvolutionLoop::new(config);

        // Record a failure and trigger refinement
        evolution
            .record_execution(make_execution("test-agent", 1, TaskOutcome::Failure))
            .await;
        evolution.evaluate().await;

        let pending = evolution.get_pending_refinements().await;
        let request_id = pending[0].id;

        // Mark as in progress
        assert!(evolution.start_refinement(request_id).await);

        // Record another failure
        evolution
            .record_execution(make_execution("test-agent", 1, TaskOutcome::Failure))
            .await;

        // Evaluate should not create a duplicate (InProgress blocks too)
        let events = evolution.evaluate().await;
        assert_eq!(events.len(), 1);
        assert!(matches!(
            events[0].action_taken,
            EvolutionAction::NoAction { .. }
        ));
    }

    #[tokio::test]
    async fn test_refinement_allowed_after_completion() {
        let config = EvolutionConfig {
            min_tasks_for_evaluation: 1,
            refinement_threshold: 0.80,
            ..Default::default()
        };
        let evolution = EvolutionLoop::new(config);

        // First cycle: failure -> refinement -> complete
        evolution
            .record_execution(make_execution("test-agent", 1, TaskOutcome::Failure))
            .await;
        evolution.evaluate().await;

        let pending = evolution.get_pending_refinements().await;
        let request_id = pending[0].id;
        assert!(evolution.start_refinement(request_id).await);
        evolution.complete_refinement(request_id, true).await;

        // No longer active
        assert!(!evolution.has_active_refinement("test-agent").await);

        // Another failure — should be allowed to create a new refinement
        evolution
            .record_execution(make_execution("test-agent", 1, TaskOutcome::Failure))
            .await;
        let events = evolution.evaluate().await;
        assert_eq!(events.len(), 1);
        assert!(matches!(
            events[0].action_taken,
            EvolutionAction::FlaggedForRefinement { .. }
        ));

        let pending = evolution.get_pending_refinements().await;
        assert_eq!(pending.len(), 1);
    }

    #[tokio::test]
    async fn test_has_active_refinement() {
        let config = EvolutionConfig {
            min_tasks_for_evaluation: 1,
            refinement_threshold: 0.80,
            ..Default::default()
        };
        let evolution = EvolutionLoop::new(config);

        // Initially no active refinements
        assert!(!evolution.has_active_refinement("test-agent").await);

        // Create a refinement
        evolution
            .record_execution(make_execution("test-agent", 1, TaskOutcome::Failure))
            .await;
        evolution.evaluate().await;

        // Now has active refinement (Pending)
        assert!(evolution.has_active_refinement("test-agent").await);
        // Different template has none
        assert!(!evolution.has_active_refinement("other-agent").await);

        // Start it — still active (InProgress)
        let pending = evolution.get_pending_refinements().await;
        evolution.start_refinement(pending[0].id).await;
        assert!(evolution.has_active_refinement("test-agent").await);

        // Complete it — no longer active
        evolution.complete_refinement(pending[0].id, true).await;
        assert!(!evolution.has_active_refinement("test-agent").await);
    }

    /// Verify that direct-mode successes (with real turns/tokens) populate EvolutionLoop stats.
    ///
    /// Direct-mode executions record actual turns_used and tokens_used because the substrate
    /// runs a single-shot invocation and returns them. This test mirrors the
    /// goal_processing.rs direct-mode success path (line ~1213).
    #[tokio::test]
    async fn test_direct_mode_success_populates_stats() {
        let evolution = EvolutionLoop::with_default_config();

        // Simulate direct-mode execution: turns and tokens are set from the real substrate response
        let exec = TaskExecution {
            task_id: Uuid::new_v4(),
            template_name: "direct-agent".to_string(),
            template_version: 1,
            outcome: TaskOutcome::Success,
            executed_at: Utc::now(),
            turns_used: 12,   // real turn count from substrate
            tokens_used: 5000, // real token count from substrate
            downstream_tasks: vec![],
        };
        evolution.record_execution(exec).await;

        let stats = evolution.get_stats("direct-agent").await.unwrap();
        assert_eq!(stats.total_tasks, 1, "direct-mode success must increment total_tasks");
        assert_eq!(stats.successful_tasks, 1, "direct-mode success must increment successful_tasks");
        assert_eq!(stats.failed_tasks, 0);
        assert!((stats.success_rate - 1.0).abs() < 0.001, "single success = 100% rate");
        assert!((stats.avg_turns - 12.0).abs() < 0.001, "avg_turns must reflect real direct-mode turn count");
        assert!((stats.avg_tokens - 5000.0).abs() < 1.0, "avg_tokens must reflect real direct-mode token count");
    }

    /// Verify that direct-mode failures populate EvolutionLoop stats.
    ///
    /// Both error paths in goal_processing.rs (session error + substrate error) record
    /// TaskOutcome::Failure with real turn/token counts when available.
    #[tokio::test]
    async fn test_direct_mode_failure_populates_stats() {
        let evolution = EvolutionLoop::with_default_config();

        // Simulate a direct-mode failure (session ended in error)
        let exec = TaskExecution {
            task_id: Uuid::new_v4(),
            template_name: "direct-agent".to_string(),
            template_version: 1,
            outcome: TaskOutcome::Failure,
            executed_at: Utc::now(),
            turns_used: 8,
            tokens_used: 3200,
            downstream_tasks: vec![],
        };
        evolution.record_execution(exec).await;

        let stats = evolution.get_stats("direct-agent").await.unwrap();
        assert_eq!(stats.total_tasks, 1, "direct-mode failure must increment total_tasks");
        assert_eq!(stats.failed_tasks, 1, "direct-mode failure must increment failed_tasks");
        assert_eq!(stats.successful_tasks, 0);
        assert!((stats.success_rate - 0.0).abs() < 0.001, "single failure = 0% rate");
    }

    /// Verify that EvolutionLoop.stats accumulates across both direct-mode and
    /// convergent-mode executions for the same template.
    ///
    /// Convergent-mode records turns_used=0 and tokens_used=0 (because iteration counts
    /// and tokens are aggregated inside the convergence loop, not per-execution).
    /// Both paths call record_execution(), so they should aggregate correctly.
    #[tokio::test]
    async fn test_stats_populated_across_both_modes() {
        let evolution = EvolutionLoop::with_default_config();

        // Convergent-mode execution: turns=0, tokens=0 (aggregated inside convergence loop)
        let convergent_exec = TaskExecution {
            task_id: Uuid::new_v4(),
            template_name: "shared-agent".to_string(),
            template_version: 1,
            outcome: TaskOutcome::Success,
            executed_at: Utc::now(),
            turns_used: 0,    // convergent path: tracks iterations, not turns
            tokens_used: 0,   // convergent path: tokens aggregated inside convergence loop
            downstream_tasks: vec![],
        };
        evolution.record_execution(convergent_exec).await;

        // Direct-mode execution: turns and tokens set from real substrate response
        let direct_exec = TaskExecution {
            task_id: Uuid::new_v4(),
            template_name: "shared-agent".to_string(),
            template_version: 1,
            outcome: TaskOutcome::Success,
            executed_at: Utc::now(),
            turns_used: 15,
            tokens_used: 6000,
            downstream_tasks: vec![],
        };
        evolution.record_execution(direct_exec).await;

        let stats = evolution.get_stats("shared-agent").await.unwrap();
        assert_eq!(stats.total_tasks, 2, "both direct and convergent executions must count toward total");
        assert_eq!(stats.successful_tasks, 2, "both successes must be counted");
        assert_eq!(stats.failed_tasks, 0);
        assert!((stats.success_rate - 1.0).abs() < 0.001, "two successes = 100% rate");
    }

    /// Verify that the statistical significance threshold (min_tasks_for_evaluation=5)
    /// prevents premature RefinementRequest creation, even when direct-mode failures are recorded.
    ///
    /// This is the core constraint from the evolution feedback loop goal:
    /// "Refinement triggers must not fire on fewer than the configured minimum tasks."
    #[tokio::test]
    async fn test_statistical_significance_threshold_respected() {
        // Use the default config (min_tasks_for_evaluation = 5)
        let evolution = EvolutionLoop::with_default_config();

        // Record 4 failures (below min_tasks threshold of 5)
        for _ in 0..4 {
            evolution
                .record_execution(make_execution("agent-under-test", 1, TaskOutcome::Failure))
                .await;
        }

        // evaluate() must NOT create a RefinementRequest — sample too small
        let events = evolution.evaluate().await;
        let refinement_events: Vec<_> = events
            .iter()
            .filter(|e| matches!(e.action_taken, EvolutionAction::FlaggedForRefinement { .. }))
            .collect();
        assert!(
            refinement_events.is_empty(),
            "evaluate() must not trigger refinement with only 4 tasks (below min_tasks=5); \
             got {:?}",
            events.iter().map(|e| &e.action_taken).collect::<Vec<_>>()
        );

        let pending = evolution.get_pending_refinements().await;
        assert!(
            pending.is_empty(),
            "no RefinementRequest must be created before min_tasks threshold is reached"
        );

        // Record a 5th failure (now meets the threshold)
        evolution
            .record_execution(make_execution("agent-under-test", 1, TaskOutcome::Failure))
            .await;

        let events = evolution.evaluate().await;
        let flagged = events
            .iter()
            .any(|e| matches!(e.action_taken, EvolutionAction::FlaggedForRefinement { .. }));
        assert!(
            flagged,
            "evaluate() must trigger refinement once min_tasks=5 is reached with 0% success rate"
        );

        let pending = evolution.get_pending_refinements().await;
        assert_eq!(
            pending.len(),
            1,
            "exactly one RefinementRequest must be created after threshold is met"
        );
    }
}
