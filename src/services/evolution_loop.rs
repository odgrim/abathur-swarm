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

use crate::domain::models::AgentStatus;
use crate::domain::ports::AgentRepository;

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
    /// Hours after which a Pending or InProgress refinement request is
    /// considered stale and automatically expired to Failed.
    /// Set to 0 to disable stale expiration.
    pub stale_refinement_timeout_hours: i64,
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
            stale_refinement_timeout_hours: 48,
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
    /// Stale refinement request expired.
    StaleTimeout,
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
    /// Stale refinement request expired without completion.
    StaleExpired { request_id: Uuid },
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

/// Record of a template version change for persistence.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VersionChangeRecord {
    pub template_name: String,
    pub from_version: u32,
    pub to_version: u32,
    pub previous_stats: TemplateStats,
    pub changed_at: DateTime<Utc>,
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

    // ── Template stats persistence (default no-ops for backward compat) ──

    /// Persist or update template stats.
    async fn save_stats(
        &self,
        _stats: &TemplateStats,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        Ok(())
    }

    /// Load all persisted template stats.
    async fn load_all_stats(
        &self,
    ) -> Result<Vec<TemplateStats>, Box<dyn std::error::Error + Send + Sync>> {
        Ok(Vec::new())
    }

    /// Persist a single task execution record.
    async fn save_execution(
        &self,
        _execution: &TaskExecution,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        Ok(())
    }

    /// Load execution records for a given template.
    async fn load_executions(
        &self,
        _template_name: &str,
    ) -> Result<Vec<TaskExecution>, Box<dyn std::error::Error + Send + Sync>> {
        Ok(Vec::new())
    }

    /// Persist a version change record (previous stats snapshot).
    async fn save_version_change(
        &self,
        _template_name: &str,
        _from_version: u32,
        _to_version: u32,
        _previous_stats: &TemplateStats,
        _changed_at: DateTime<Utc>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        Ok(())
    }

    /// Load version change records for all templates.
    async fn load_version_changes(
        &self,
    ) -> Result<Vec<VersionChangeRecord>, Box<dyn std::error::Error + Send + Sync>> {
        Ok(Vec::new())
    }
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
    agent_repo: Option<Arc<dyn AgentRepository>>,
}

impl EvolutionLoop {
    pub fn new(config: EvolutionConfig) -> Self {
        Self {
            config,
            state: Arc::new(RwLock::new(EvolutionState::new())),
            refinement_repo: None,
            agent_repo: None,
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

    /// Attach an agent repository for auto-revert support.
    ///
    /// When set, auto-revert on regression will actually fetch the previous
    /// template version from the repository and mark it as active.
    /// Without this, auto-revert emits the event but does not restore the template.
    pub fn with_agent_repo(mut self, repo: Arc<dyn AgentRepository>) -> Self {
        self.agent_repo = Some(repo);
        self
    }

    /// Record a task execution.
    pub async fn record_execution(&self, execution: TaskExecution) {
        // Capture data needed for persistence before taking the lock
        let mut version_change_info: Option<(String, u32, u32, TemplateStats, DateTime<Utc>)> =
            None;

        let updated_stats = {
            let mut state = self.state.write().await;

            // Check if we need to handle version change first
            let needs_version_reset =
                if let Some(stats) = state.stats.get(&execution.template_name) {
                    stats.template_version != execution.template_version
                } else {
                    false
                };

            if needs_version_reset {
                // Clone previous stats for regression detection
                if let Some(prev_stats) = state.stats.get(&execution.template_name).cloned() {
                    let change_time = Utc::now();
                    version_change_info = Some((
                        execution.template_name.clone(),
                        prev_stats.template_version,
                        execution.template_version,
                        prev_stats.clone(),
                        change_time,
                    ));
                    state
                        .previous_version_stats
                        .insert(execution.template_name.clone(), prev_stats);
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
            let updated = stats.clone();

            // Store execution
            state
                .executions
                .entry(execution.template_name.clone())
                .or_default()
                .push(execution.clone());

            updated
        };

        // Persist to DB (fire-and-forget with warning on error)
        if let Some(ref repo) = self.refinement_repo {
            if let Err(e) = repo.save_execution(&execution).await {
                tracing::warn!(
                    "Failed to persist execution for {}: {}",
                    execution.template_name,
                    e
                );
            }
            if let Err(e) = repo.save_stats(&updated_stats).await {
                tracing::warn!(
                    "Failed to persist stats for {}: {}",
                    execution.template_name,
                    e
                );
            }
            if let Some((ref name, from_v, to_v, ref prev_stats, changed_at)) =
                version_change_info
            {
                if let Err(e) = repo
                    .save_version_change(name, from_v, to_v, prev_stats, changed_at)
                    .await
                {
                    tracing::warn!(
                        "Failed to persist version change for {}: {}",
                        name,
                        e
                    );
                }
            }
        }
    }

    /// Evaluate templates and trigger evolution if needed.
    pub async fn evaluate(&self) -> Vec<EvolutionEvent> {
        let stale_expired = self.expire_stale_refinements().await;
        let mut stale_events: Vec<EvolutionEvent> = stale_expired
            .into_iter()
            .map(|(template_name, template_version, request_id)| EvolutionEvent {
                id: Uuid::new_v4(),
                template_name: template_name.clone(),
                template_version,
                trigger: EvolutionTrigger::StaleTimeout,
                stats_at_trigger: TemplateStats::new(template_name, template_version),
                action_taken: EvolutionAction::StaleExpired { request_id },
                occurred_at: Utc::now(),
            })
            .collect();
        let mut new_requests: Vec<RefinementRequest> = Vec::new();
        // Collect revert instructions: (template_name, to_version) to execute
        // outside the write lock since agent_repo calls are async.
        let mut revert_instructions: Vec<(String, u32)> = Vec::new();

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
                            let to_version = prev_stats.template_version;
                            revert_instructions.push((template_name.clone(), to_version));
                            EvolutionAction::Reverted {
                                from_version: stats.template_version,
                                to_version,
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

        // Actually restore previous template versions for auto-reverts.
        // This must happen outside the write lock because agent_repo calls are async.
        if !revert_instructions.is_empty() {
            if let Some(ref agent_repo) = self.agent_repo {
                for (template_name, to_version) in &revert_instructions {
                    match agent_repo
                        .get_template_version(template_name, *to_version)
                        .await
                    {
                        Ok(Some(mut prev_template)) => {
                            prev_template.status = AgentStatus::Active;
                            prev_template.updated_at = Utc::now();
                            if let Err(e) = agent_repo.update_template(&prev_template).await {
                                tracing::error!(
                                    "Auto-revert failed: could not update template '{}' v{} to active: {}",
                                    template_name,
                                    to_version,
                                    e,
                                );
                            } else {
                                tracing::info!(
                                    "Auto-revert: restored template '{}' v{} as active",
                                    template_name,
                                    to_version,
                                );
                            }
                        }
                        Ok(None) => {
                            tracing::error!(
                                "Auto-revert failed: template '{}' v{} not found in repository",
                                template_name,
                                to_version,
                            );
                        }
                        Err(e) => {
                            tracing::error!(
                                "Auto-revert failed: could not fetch template '{}' v{}: {}",
                                template_name,
                                to_version,
                                e,
                            );
                        }
                    }
                }
            } else {
                tracing::warn!(
                    "Auto-revert: {} template(s) flagged for revert but no agent repository configured — \
                     revert event emitted but template not actually restored",
                    revert_instructions.len(),
                );
            }
        }

        stale_events.extend(events);
        stale_events
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

    /// Expire stale refinement requests that have been Pending or InProgress
    /// for longer than `stale_refinement_timeout_hours`.
    ///
    /// Returns the number of requests that were expired.
    /// Returns 0 immediately if the timeout is configured as 0 (disabled).
    pub async fn expire_stale_refinements(&self) -> Vec<(String, u32, Uuid)> {
        if self.config.stale_refinement_timeout_hours == 0 {
            return Vec::new();
        }

        let cutoff =
            Utc::now() - Duration::hours(self.config.stale_refinement_timeout_hours);
        let mut expired: Vec<(String, u32, Uuid)> = Vec::new();

        {
            let mut state = self.state.write().await;
            for request in &mut state.refinement_queue {
                if matches!(
                    request.status,
                    RefinementStatus::Pending | RefinementStatus::InProgress
                ) && request.created_at < cutoff
                {
                    request.status = RefinementStatus::Failed;
                    expired.push((
                        request.template_name.clone(),
                        request.template_version,
                        request.id,
                    ));
                }
            }
        }

        if let Some(ref repo) = self.refinement_repo {
            for (_, _, id) in &expired {
                if let Err(e) = repo
                    .update_status(*id, RefinementStatus::Failed)
                    .await
                {
                    tracing::warn!(
                        "Failed to persist Failed status for stale refinement {}: {}",
                        id,
                        e
                    );
                }
            }
        }

        if !expired.is_empty() {
            tracing::info!(
                "Expired {} stale refinement request(s) older than {}h",
                expired.len(),
                self.config.stale_refinement_timeout_hours
            );
        }

        expired
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

    /// Load persisted template stats and version changes from the repository.
    ///
    /// Called on startup after `recover_in_progress_refinements()` to restore
    /// in-memory evolution state from the database so stats survive restarts.
    pub async fn load_persisted_state(&self) {
        let Some(ref repo) = self.refinement_repo else {
            return;
        };

        // Load template stats
        match repo.load_all_stats().await {
            Ok(all_stats) => {
                let mut state = self.state.write().await;
                for stats in all_stats {
                    // Only insert if not already present (in-memory takes precedence)
                    state
                        .stats
                        .entry(stats.template_name.clone())
                        .or_insert(stats);
                }
            }
            Err(e) => {
                tracing::warn!("Failed to load persisted template stats: {}", e);
            }
        }

        // Load version changes to restore previous_version_stats and version_change_times
        match repo.load_version_changes().await {
            Ok(changes) => {
                let mut state = self.state.write().await;
                for change in changes {
                    // Only insert the most recent change per template (they are ordered DESC)
                    state
                        .previous_version_stats
                        .entry(change.template_name.clone())
                        .or_insert(change.previous_stats.clone());
                    state
                        .version_change_times
                        .entry(change.template_name.clone())
                        .or_insert((change.to_version, change.changed_at));
                }
            }
            Err(e) => {
                tracing::warn!("Failed to load persisted version changes: {}", e);
            }
        }
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
    async fn test_refinement_lifecycle_failed_path() {
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
        evolution.evaluate().await;

        let pending = evolution.get_pending_refinements().await;
        assert_eq!(pending.len(), 1);
        let request_id = pending[0].id;

        // Start refinement
        assert!(evolution.start_refinement(request_id).await);

        // Complete refinement with failure (success = false)
        evolution.complete_refinement(request_id, false).await;

        // Should no longer be pending (Failed is a terminal state)
        let pending = evolution.get_pending_refinements().await;
        assert!(pending.is_empty(), "Failed refinement should not appear in pending list");

        // A new evaluation should be able to create a new refinement request
        // since the previous one reached a terminal state (Failed)
        evolution
            .record_execution(make_execution("test-agent", 1, TaskOutcome::Failure))
            .await;
        evolution.evaluate().await;

        let pending = evolution.get_pending_refinements().await;
        assert_eq!(
            pending.len(), 1,
            "New refinement should be created after previous one failed"
        );
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

    #[tokio::test]
    async fn test_regression_detection_triggers_on_rate_drop() {
        let config = EvolutionConfig {
            min_tasks_for_evaluation: 3,
            regression_min_tasks: 3,
            regression_threshold: 0.15,
            regression_detection_window_hours: 24,
            auto_revert_enabled: false,
            // Set low so LowSuccessRate/VeryLowSuccessRate don't fire before regression
            refinement_threshold: 0.01,
            major_refinement_threshold: 0.01,
            major_refinement_min_tasks: 100,
            stale_refinement_timeout_hours: 48,
        };
        let evolution = EvolutionLoop::new(config);

        // v1: 5 successes → 100% rate
        for _ in 0..5 {
            evolution
                .record_execution(make_execution("regress-agent", 1, TaskOutcome::Success))
                .await;
        }
        // Evaluate to establish baseline (no trigger expected since rate is high)
        let events = evolution.evaluate().await;
        assert!(events.is_empty(), "v1 at 100% should not trigger anything");

        // Switch to v2: 1 success + 3 failures → 25% rate (drop of 75%)
        evolution
            .record_execution(make_execution("regress-agent", 2, TaskOutcome::Success))
            .await;
        for _ in 0..3 {
            evolution
                .record_execution(make_execution("regress-agent", 2, TaskOutcome::Failure))
                .await;
        }

        let events = evolution.evaluate().await;
        let regression_event = events
            .iter()
            .find(|e| e.trigger == EvolutionTrigger::Regression);
        assert!(
            regression_event.is_some(),
            "Should detect regression after version change with rate drop (75%) >= threshold (15%); events: {:?}",
            events.iter().map(|e| &e.trigger).collect::<Vec<_>>()
        );
        // With auto_revert_enabled=false, action should be FlaggedForRefinement with Immediate severity
        if let Some(ev) = regression_event {
            assert!(
                matches!(ev.action_taken, EvolutionAction::FlaggedForRefinement { severity: RefinementSeverity::Immediate }),
                "Regression without auto-revert should flag for immediate refinement; got {:?}",
                ev.action_taken,
            );
        }
    }

    #[tokio::test]
    async fn test_auto_revert_when_regression_detected() {
        let config = EvolutionConfig {
            min_tasks_for_evaluation: 3,
            regression_min_tasks: 3,
            regression_threshold: 0.15,
            regression_detection_window_hours: 24,
            auto_revert_enabled: true,
            refinement_threshold: 0.01,
            major_refinement_threshold: 0.01,
            major_refinement_min_tasks: 100,
            stale_refinement_timeout_hours: 48,
        };
        let evolution = EvolutionLoop::new(config);

        // v1: 5 successes → 100% rate
        for _ in 0..5 {
            evolution
                .record_execution(make_execution("revert-agent", 1, TaskOutcome::Success))
                .await;
        }
        evolution.evaluate().await;

        // Switch to v2: 1 success + 2 failures → 33% rate (drop of 67% from 100%)
        evolution
            .record_execution(make_execution("revert-agent", 2, TaskOutcome::Success))
            .await;
        for _ in 0..2 {
            evolution
                .record_execution(make_execution("revert-agent", 2, TaskOutcome::Failure))
                .await;
        }

        let events = evolution.evaluate().await;
        let revert_event = events
            .iter()
            .find(|e| matches!(e.action_taken, EvolutionAction::Reverted { .. }));
        assert!(
            revert_event.is_some(),
            "Should auto-revert when regression detected and auto_revert_enabled=true; events: {:?}",
            events.iter().map(|e| (&e.trigger, &e.action_taken)).collect::<Vec<_>>()
        );
        if let Some(ev) = revert_event {
            match &ev.action_taken {
                EvolutionAction::Reverted {
                    from_version,
                    to_version,
                } => {
                    assert_eq!(*from_version, 2, "Should revert FROM version 2");
                    assert_eq!(*to_version, 1, "Should revert TO version 1");
                }
                other => panic!("Expected Reverted action, got {:?}", other),
            }
        }
    }

    #[tokio::test]
    async fn test_auto_revert_only_applies_to_regression_trigger() {
        // auto_revert_enabled=true, but the trigger is LowSuccessRate (not Regression).
        // The action must be FlaggedForRefinement, NOT Reverted.
        let config = EvolutionConfig {
            min_tasks_for_evaluation: 3,
            regression_min_tasks: 3,
            regression_threshold: 0.15,
            regression_detection_window_hours: 24,
            auto_revert_enabled: true,
            refinement_threshold: 0.70,
            major_refinement_threshold: 0.01,
            major_refinement_min_tasks: 100,
            stale_refinement_timeout_hours: 48,
        };
        let evolution = EvolutionLoop::new(config);

        // All executions on the same version → no regression possible.
        // 1 success + 2 failures on v1 → 33% success rate, below refinement_threshold (0.70).
        evolution
            .record_execution(make_execution("guard-agent", 1, TaskOutcome::Success))
            .await;
        for _ in 0..2 {
            evolution
                .record_execution(make_execution("guard-agent", 1, TaskOutcome::Failure))
                .await;
        }

        let events = evolution.evaluate().await;

        // There should be an event for this agent, and it must NOT be Reverted.
        let agent_events: Vec<_> = events
            .iter()
            .filter(|e| e.template_name == "guard-agent")
            .collect();
        assert!(
            !agent_events.is_empty(),
            "Expected at least one evolution event for guard-agent; got none"
        );
        for ev in &agent_events {
            assert!(
                !matches!(ev.action_taken, EvolutionAction::Reverted { .. }),
                "LowSuccessRate trigger should NOT produce Reverted action; got {:?}",
                ev.action_taken
            );
            assert!(
                matches!(
                    ev.action_taken,
                    EvolutionAction::FlaggedForRefinement { .. }
                ),
                "LowSuccessRate trigger should produce FlaggedForRefinement; got {:?}",
                ev.action_taken
            );
        }
    }

    #[tokio::test]
    async fn test_regression_window_expiry() {
        let config = EvolutionConfig {
            min_tasks_for_evaluation: 3,
            regression_min_tasks: 3,
            regression_threshold: 0.15,
            regression_detection_window_hours: 1, // 1-hour window (will be expired)
            auto_revert_enabled: true,
            refinement_threshold: 0.01,
            major_refinement_threshold: 0.01,
            major_refinement_min_tasks: 100,
            stale_refinement_timeout_hours: 48,
        };
        let evolution = EvolutionLoop::new(config);

        // v1: 5 successes
        for _ in 0..5 {
            evolution
                .record_execution(make_execution("window-agent", 1, TaskOutcome::Success))
                .await;
        }
        evolution.evaluate().await;

        // Switch to v2 (this sets version_change_time to now)
        evolution
            .record_execution(make_execution("window-agent", 2, TaskOutcome::Failure))
            .await;

        // Manually backdate the version change time to >1 hour ago
        {
            let mut state = evolution.state.write().await;
            if let Some(entry) = state.version_change_times.get_mut("window-agent") {
                entry.1 = Utc::now() - Duration::hours(2);
            }
        }

        // Record more failures to meet regression_min_tasks
        for _ in 0..2 {
            evolution
                .record_execution(make_execution("window-agent", 2, TaskOutcome::Failure))
                .await;
        }

        let events = evolution.evaluate().await;
        let has_regression = events
            .iter()
            .any(|e| e.trigger == EvolutionTrigger::Regression);
        assert!(
            !has_regression,
            "Should NOT detect regression outside the detection window; events: {:?}",
            events.iter().map(|e| &e.trigger).collect::<Vec<_>>()
        );
    }

    #[tokio::test]
    async fn test_expire_stale_refinements() {
        let config = EvolutionConfig {
            min_tasks_for_evaluation: 1,
            refinement_threshold: 0.80,
            stale_refinement_timeout_hours: 2,
            ..Default::default()
        };
        let evolution = EvolutionLoop::new(config);

        // Manually insert two Pending refinement requests with different ages
        let old_request = RefinementRequest {
            id: Uuid::new_v4(),
            template_name: "stale-agent".to_string(),
            template_version: 1,
            severity: RefinementSeverity::Minor,
            trigger: EvolutionTrigger::LowSuccessRate,
            stats: TemplateStats::new("stale-agent".to_string(), 1),
            failed_task_ids: vec![],
            created_at: Utc::now() - Duration::hours(3), // 3h old → should expire
            status: RefinementStatus::Pending,
        };
        let fresh_request = RefinementRequest {
            id: Uuid::new_v4(),
            template_name: "fresh-agent".to_string(),
            template_version: 1,
            severity: RefinementSeverity::Minor,
            trigger: EvolutionTrigger::LowSuccessRate,
            stats: TemplateStats::new("fresh-agent".to_string(), 1),
            failed_task_ids: vec![],
            created_at: Utc::now() - Duration::hours(1), // 1h old → should NOT expire
            status: RefinementStatus::Pending,
        };

        {
            let mut state = evolution.state.write().await;
            state.refinement_queue.push(old_request.clone());
            state.refinement_queue.push(fresh_request.clone());
        }

        let expired = evolution.expire_stale_refinements().await;
        assert_eq!(expired.len(), 1, "only the 3h-old request should be expired");
        assert_eq!(expired[0].0, "stale-agent");
        assert_eq!(expired[0].1, 1);
        assert_eq!(expired[0].2, old_request.id);

        let state = evolution.state.read().await;
        let old = state
            .refinement_queue
            .iter()
            .find(|r| r.id == old_request.id)
            .unwrap();
        assert_eq!(old.status, RefinementStatus::Failed);

        let fresh = state
            .refinement_queue
            .iter()
            .find(|r| r.id == fresh_request.id)
            .unwrap();
        assert_eq!(fresh.status, RefinementStatus::Pending);
    }

    #[tokio::test]
    async fn test_expire_stale_refinements_disabled_when_zero() {
        let config = EvolutionConfig {
            min_tasks_for_evaluation: 1,
            refinement_threshold: 0.80,
            stale_refinement_timeout_hours: 0, // disabled
            ..Default::default()
        };
        let evolution = EvolutionLoop::new(config);

        // Insert a very old Pending request
        let ancient_request = RefinementRequest {
            id: Uuid::new_v4(),
            template_name: "ancient-agent".to_string(),
            template_version: 1,
            severity: RefinementSeverity::Minor,
            trigger: EvolutionTrigger::LowSuccessRate,
            stats: TemplateStats::new("ancient-agent".to_string(), 1),
            failed_task_ids: vec![],
            created_at: Utc::now() - Duration::hours(1000), // 1000h old
            status: RefinementStatus::Pending,
        };

        {
            let mut state = evolution.state.write().await;
            state.refinement_queue.push(ancient_request.clone());
        }

        let expired = evolution.expire_stale_refinements().await;
        assert_eq!(expired.len(), 0, "expiry disabled when timeout=0");

        let state = evolution.state.read().await;
        let req = state
            .refinement_queue
            .iter()
            .find(|r| r.id == ancient_request.id)
            .unwrap();
        assert_eq!(
            req.status,
            RefinementStatus::Pending,
            "request must remain Pending when expiry is disabled"
        );
    }

    #[tokio::test]
    async fn test_evaluate_emits_events_for_stale_expirations() {
        let config = EvolutionConfig {
            min_tasks_for_evaluation: 1,
            refinement_threshold: 0.80,
            stale_refinement_timeout_hours: 2,
            ..Default::default()
        };
        let evolution = EvolutionLoop::new(config);

        // Insert a stale Pending refinement request
        let stale_request = RefinementRequest {
            id: Uuid::new_v4(),
            template_name: "stale-agent".to_string(),
            template_version: 3,
            severity: RefinementSeverity::Minor,
            trigger: EvolutionTrigger::LowSuccessRate,
            stats: TemplateStats::new("stale-agent".to_string(), 3),
            failed_task_ids: vec![],
            created_at: Utc::now() - Duration::hours(5),
            status: RefinementStatus::Pending,
        };

        {
            let mut state = evolution.state.write().await;
            state.refinement_queue.push(stale_request.clone());
        }

        let events = evolution.evaluate().await;

        // Should have exactly one StaleExpired event
        let stale_events: Vec<_> = events
            .iter()
            .filter(|e| matches!(e.action_taken, EvolutionAction::StaleExpired { .. }))
            .collect();
        assert_eq!(stale_events.len(), 1, "should emit one StaleExpired event");

        let event = stale_events[0];
        assert_eq!(event.template_name, "stale-agent");
        assert_eq!(event.template_version, 3);
        assert_eq!(event.trigger, EvolutionTrigger::StaleTimeout);
        if let EvolutionAction::StaleExpired { request_id } = &event.action_taken {
            assert_eq!(*request_id, stale_request.id);
        } else {
            panic!("Expected StaleExpired action");
        }
    }

    /// A mock agent repository that records update_template calls for verification.
    struct MockAgentRepo {
        templates: tokio::sync::Mutex<HashMap<(String, u32), crate::domain::models::AgentTemplate>>,
        updated: tokio::sync::Mutex<Vec<crate::domain::models::AgentTemplate>>,
    }

    impl MockAgentRepo {
        fn new() -> Self {
            Self {
                templates: tokio::sync::Mutex::new(HashMap::new()),
                updated: tokio::sync::Mutex::new(Vec::new()),
            }
        }

    }

    #[async_trait]
    impl crate::domain::ports::AgentRepository for MockAgentRepo {
        async fn create_template(
            &self,
            _template: &crate::domain::models::AgentTemplate,
        ) -> crate::domain::errors::DomainResult<()> {
            Ok(())
        }
        async fn get_template(
            &self,
            _id: Uuid,
        ) -> crate::domain::errors::DomainResult<Option<crate::domain::models::AgentTemplate>> {
            Ok(None)
        }
        async fn get_template_by_name(
            &self,
            _name: &str,
        ) -> crate::domain::errors::DomainResult<Option<crate::domain::models::AgentTemplate>> {
            Ok(None)
        }
        async fn get_template_version(
            &self,
            name: &str,
            version: u32,
        ) -> crate::domain::errors::DomainResult<Option<crate::domain::models::AgentTemplate>> {
            let templates = self.templates.lock().await;
            Ok(templates.get(&(name.to_string(), version)).cloned())
        }
        async fn update_template(
            &self,
            template: &crate::domain::models::AgentTemplate,
        ) -> crate::domain::errors::DomainResult<()> {
            let mut updated = self.updated.lock().await;
            updated.push(template.clone());
            Ok(())
        }
        async fn delete_template(
            &self,
            _id: Uuid,
        ) -> crate::domain::errors::DomainResult<()> {
            Ok(())
        }
        async fn list_templates(
            &self,
            _filter: crate::domain::ports::AgentFilter,
        ) -> crate::domain::errors::DomainResult<Vec<crate::domain::models::AgentTemplate>> {
            Ok(vec![])
        }
        async fn list_by_tier(
            &self,
            _tier: crate::domain::models::AgentTier,
        ) -> crate::domain::errors::DomainResult<Vec<crate::domain::models::AgentTemplate>> {
            Ok(vec![])
        }
        async fn get_active_templates(
            &self,
        ) -> crate::domain::errors::DomainResult<Vec<crate::domain::models::AgentTemplate>> {
            Ok(vec![])
        }
        async fn create_instance(
            &self,
            _instance: &crate::domain::models::AgentInstance,
        ) -> crate::domain::errors::DomainResult<()> {
            Ok(())
        }
        async fn get_instance(
            &self,
            _id: Uuid,
        ) -> crate::domain::errors::DomainResult<Option<crate::domain::models::AgentInstance>> {
            Ok(None)
        }
        async fn update_instance(
            &self,
            _instance: &crate::domain::models::AgentInstance,
        ) -> crate::domain::errors::DomainResult<()> {
            Ok(())
        }
        async fn delete_instance(
            &self,
            _id: Uuid,
        ) -> crate::domain::errors::DomainResult<()> {
            Ok(())
        }
        async fn list_instances_by_status(
            &self,
            _status: crate::domain::models::InstanceStatus,
        ) -> crate::domain::errors::DomainResult<Vec<crate::domain::models::AgentInstance>> {
            Ok(vec![])
        }
        async fn get_running_instances(
            &self,
            _template_name: &str,
        ) -> crate::domain::errors::DomainResult<Vec<crate::domain::models::AgentInstance>> {
            Ok(vec![])
        }
        async fn count_running_by_template(
            &self,
        ) -> crate::domain::errors::DomainResult<HashMap<String, u32>> {
            Ok(HashMap::new())
        }
    }

    fn make_template(name: &str, version: u32) -> crate::domain::models::AgentTemplate {
        use crate::domain::models::{AgentCard, AgentStatus, AgentTier};
        crate::domain::models::AgentTemplate {
            id: Uuid::new_v4(),
            name: name.to_string(),
            description: format!("{} v{}", name, version),
            tier: AgentTier::Worker,
            version,
            system_prompt: "test prompt".to_string(),
            tools: vec![],
            constraints: vec![],
            agent_card: AgentCard::default(),
            max_turns: 10,
            read_only: false,
            preferred_model: None,
            status: AgentStatus::Active,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[tokio::test]
    async fn test_auto_revert_actually_restores_template() {
        let v1_template = make_template("revert-real", 1);

        let mock_repo = Arc::new(MockAgentRepo::new());
        {
            let mut templates = mock_repo.templates.lock().await;
            templates.insert((v1_template.name.clone(), v1_template.version), v1_template.clone());
        }

        let config = EvolutionConfig {
            min_tasks_for_evaluation: 3,
            regression_min_tasks: 3,
            regression_threshold: 0.15,
            regression_detection_window_hours: 24,
            auto_revert_enabled: true,
            refinement_threshold: 0.01,
            major_refinement_threshold: 0.01,
            major_refinement_min_tasks: 100,
            stale_refinement_timeout_hours: 48,
        };
        let evolution = EvolutionLoop::new(config).with_agent_repo(mock_repo.clone());

        // v1: 5 successes → 100% rate
        for _ in 0..5 {
            evolution
                .record_execution(make_execution("revert-real", 1, TaskOutcome::Success))
                .await;
        }
        evolution.evaluate().await;

        // Switch to v2: 1 success + 2 failures → 33% rate (drop of 67%)
        evolution
            .record_execution(make_execution("revert-real", 2, TaskOutcome::Success))
            .await;
        for _ in 0..2 {
            evolution
                .record_execution(make_execution("revert-real", 2, TaskOutcome::Failure))
                .await;
        }

        let events = evolution.evaluate().await;

        // Verify the Reverted event was emitted
        let revert_event = events
            .iter()
            .find(|e| matches!(e.action_taken, EvolutionAction::Reverted { .. }));
        assert!(revert_event.is_some(), "Should emit Reverted event");

        // Verify the agent repo was actually called to restore v1
        let updated = mock_repo.updated.lock().await;
        assert_eq!(updated.len(), 1, "Should have called update_template exactly once");
        assert_eq!(updated[0].name, "revert-real");
        assert_eq!(updated[0].version, 1, "Should restore version 1");
        assert_eq!(
            updated[0].status,
            crate::domain::models::AgentStatus::Active,
            "Restored template should be marked Active"
        );
    }

    #[tokio::test]
    async fn test_auto_revert_graceful_when_no_repo() {
        // No agent_repo configured — should still emit the Reverted event
        // without panicking
        let config = EvolutionConfig {
            min_tasks_for_evaluation: 3,
            regression_min_tasks: 3,
            regression_threshold: 0.15,
            regression_detection_window_hours: 24,
            auto_revert_enabled: true,
            refinement_threshold: 0.01,
            major_refinement_threshold: 0.01,
            major_refinement_min_tasks: 100,
            stale_refinement_timeout_hours: 48,
        };
        let evolution = EvolutionLoop::new(config);
        // Deliberately NOT calling with_agent_repo

        // v1: 5 successes
        for _ in 0..5 {
            evolution
                .record_execution(make_execution("no-repo-agent", 1, TaskOutcome::Success))
                .await;
        }
        evolution.evaluate().await;

        // Switch to v2 with regression
        evolution
            .record_execution(make_execution("no-repo-agent", 2, TaskOutcome::Success))
            .await;
        for _ in 0..2 {
            evolution
                .record_execution(make_execution("no-repo-agent", 2, TaskOutcome::Failure))
                .await;
        }

        // Should not panic — just emit the event without repo restoration
        let events = evolution.evaluate().await;
        let revert_event = events
            .iter()
            .find(|e| matches!(e.action_taken, EvolutionAction::Reverted { .. }));
        assert!(
            revert_event.is_some(),
            "Should still emit Reverted event even without agent_repo"
        );
    }

    #[tokio::test]
    async fn test_get_all_stats_returns_all_templates() {
        let evolution = EvolutionLoop::with_default_config();

        evolution
            .record_execution(make_execution("agent-a", 1, TaskOutcome::Success))
            .await;
        evolution
            .record_execution(make_execution("agent-b", 1, TaskOutcome::Failure))
            .await;
        evolution
            .record_execution(make_execution("agent-c", 2, TaskOutcome::Success))
            .await;

        let all_stats = evolution.get_all_stats().await;
        assert_eq!(all_stats.len(), 3);

        let names: Vec<String> = all_stats.iter().map(|s| s.template_name.clone()).collect();
        assert!(names.contains(&"agent-a".to_string()));
        assert!(names.contains(&"agent-b".to_string()));
        assert!(names.contains(&"agent-c".to_string()));
    }

    #[tokio::test]
    async fn test_get_all_stats_empty_when_no_executions() {
        let evolution = EvolutionLoop::with_default_config();
        let all_stats = evolution.get_all_stats().await;
        assert!(all_stats.is_empty());
    }

    #[tokio::test]
    async fn test_get_events_returns_reverse_chronological() {
        let config = EvolutionConfig {
            min_tasks_for_evaluation: 1,
            ..Default::default()
        };
        let evolution = EvolutionLoop::new(config);

        // Record a single agent's goal violation and evaluate
        evolution
            .record_execution(make_execution("agent-first", 1, TaskOutcome::GoalViolation))
            .await;
        evolution.evaluate().await;

        let events = evolution.get_events(None).await;
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].template_name, "agent-first");

        // Now record a second agent and evaluate again
        evolution
            .record_execution(make_execution("agent-second", 1, TaskOutcome::GoalViolation))
            .await;
        evolution.evaluate().await;

        let all_events = evolution.get_events(None).await;
        // Multiple events from both evaluations — verify reverse chronological order
        assert!(all_events.len() >= 2);

        // Most recent event should be last appended (reversed = first returned)
        // The last evaluate produced events for agent-second (and possibly agent-first again)
        // Just verify ordering: each event's occurred_at should be >= the next
        for window in all_events.windows(2) {
            assert!(
                window[0].occurred_at >= window[1].occurred_at,
                "Events should be in reverse chronological order"
            );
        }

        // Test with limit — should return only the most recent event(s)
        let limited = evolution.get_events(Some(1)).await;
        assert_eq!(limited.len(), 1);
        assert_eq!(limited[0].occurred_at, all_events[0].occurred_at);
    }

    #[tokio::test]
    async fn test_get_templates_needing_attention_sorted_by_severity() {
        let config = EvolutionConfig {
            min_tasks_for_evaluation: 1,
            refinement_threshold: 0.80,
            major_refinement_threshold: 0.40,
            major_refinement_min_tasks: 1,
            ..Default::default()
        };
        let evolution = EvolutionLoop::new(config);

        // Agent with minor issue (below refinement_threshold but above major)
        evolution
            .record_execution(make_execution("minor-agent", 1, TaskOutcome::Success))
            .await;
        evolution
            .record_execution(make_execution("minor-agent", 1, TaskOutcome::Failure))
            .await;

        // Agent with goal violation (immediate severity)
        evolution
            .record_execution(make_execution("immediate-agent", 1, TaskOutcome::GoalViolation))
            .await;

        evolution.evaluate().await;

        let attention = evolution.get_templates_needing_attention().await;
        assert!(attention.len() >= 2);

        // Immediate should come before Minor
        let immediate_pos = attention
            .iter()
            .position(|(name, _)| name == "immediate-agent");
        let minor_pos = attention
            .iter()
            .position(|(name, _)| name == "minor-agent");

        if let (Some(imm), Some(min)) = (immediate_pos, minor_pos) {
            assert!(imm < min, "Immediate severity should sort before Minor");
        }
    }

    #[tokio::test]
    async fn test_get_templates_needing_attention_excludes_non_pending() {
        let config = EvolutionConfig {
            min_tasks_for_evaluation: 1,
            refinement_threshold: 0.80,
            ..Default::default()
        };
        let evolution = EvolutionLoop::new(config);

        // Create a refinement
        evolution
            .record_execution(make_execution("test-agent", 1, TaskOutcome::Failure))
            .await;
        evolution.evaluate().await;

        // Verify it shows up
        let attention = evolution.get_templates_needing_attention().await;
        assert_eq!(attention.len(), 1);

        // Start and complete the refinement
        let pending = evolution.get_pending_refinements().await;
        let request_id = pending[0].id;
        evolution.start_refinement(request_id).await;
        evolution.complete_refinement(request_id, true).await;

        // Should no longer need attention (Completed is not Pending)
        let attention = evolution.get_templates_needing_attention().await;
        assert!(
            attention.is_empty(),
            "Completed refinements should not appear in needing-attention list"
        );
    }

    #[tokio::test]
    async fn test_clear_resets_all_state() {
        let config = EvolutionConfig {
            min_tasks_for_evaluation: 1,
            refinement_threshold: 0.80,
            ..Default::default()
        };
        let evolution = EvolutionLoop::new(config);

        // Populate state
        evolution
            .record_execution(make_execution("test-agent", 1, TaskOutcome::Failure))
            .await;
        evolution.evaluate().await;

        // Verify state is populated
        assert!(!evolution.get_all_stats().await.is_empty());
        assert!(!evolution.get_events(None).await.is_empty());
        assert!(!evolution.get_pending_refinements().await.is_empty());

        // Clear
        evolution.clear().await;

        // Verify all state is reset
        assert!(evolution.get_all_stats().await.is_empty());
        assert!(evolution.get_events(None).await.is_empty());
        assert!(evolution.get_pending_refinements().await.is_empty());
    }

    // ── MockRefinementRepo for persistence tests ──

    /// A mock `RefinementRepository` backed by in-memory `Arc<Mutex<Vec<...>>>` collections.
    struct MockRefinementRepo {
        requests: std::sync::Mutex<Vec<RefinementRequest>>,
        stats: std::sync::Mutex<Vec<TemplateStats>>,
        version_changes: std::sync::Mutex<Vec<VersionChangeRecord>>,
    }

    impl MockRefinementRepo {
        fn new() -> Self {
            Self {
                requests: std::sync::Mutex::new(Vec::new()),
                stats: std::sync::Mutex::new(Vec::new()),
                version_changes: std::sync::Mutex::new(Vec::new()),
            }
        }
    }

    #[async_trait]
    impl RefinementRepository for MockRefinementRepo {
        async fn create(
            &self,
            request: &RefinementRequest,
        ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
            self.requests.lock().unwrap().push(request.clone());
            Ok(())
        }

        async fn get_pending(
            &self,
        ) -> Result<Vec<RefinementRequest>, Box<dyn std::error::Error + Send + Sync>> {
            Ok(self
                .requests
                .lock()
                .unwrap()
                .iter()
                .filter(|r| r.status == RefinementStatus::Pending)
                .cloned()
                .collect())
        }

        async fn update_status(
            &self,
            id: Uuid,
            status: RefinementStatus,
        ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
            let mut reqs = self.requests.lock().unwrap();
            if let Some(r) = reqs.iter_mut().find(|r| r.id == id) {
                r.status = status;
            }
            Ok(())
        }

        async fn reset_in_progress_to_pending(
            &self,
        ) -> Result<Vec<RefinementRequest>, Box<dyn std::error::Error + Send + Sync>> {
            let mut reqs = self.requests.lock().unwrap();
            let mut recovered = Vec::new();
            for r in reqs.iter_mut() {
                if r.status == RefinementStatus::InProgress {
                    r.status = RefinementStatus::Pending;
                    recovered.push(r.clone());
                }
            }
            Ok(recovered)
        }

        async fn load_all_stats(
            &self,
        ) -> Result<Vec<TemplateStats>, Box<dyn std::error::Error + Send + Sync>> {
            Ok(self.stats.lock().unwrap().clone())
        }

        async fn load_version_changes(
            &self,
        ) -> Result<Vec<VersionChangeRecord>, Box<dyn std::error::Error + Send + Sync>> {
            Ok(self.version_changes.lock().unwrap().clone())
        }
    }

    #[tokio::test]
    async fn test_load_persisted_state_restores_stats() {
        let repo = Arc::new(MockRefinementRepo::new());
        {
            let mut stats = repo.stats.lock().unwrap();
            let mut s = TemplateStats::new("persisted-agent".to_string(), 3);
            s.total_tasks = 10;
            s.successful_tasks = 7;
            s.success_rate = 0.7;
            stats.push(s);
        }

        let evolution =
            EvolutionLoop::new(EvolutionConfig::default()).with_repo(repo as Arc<dyn RefinementRepository>);
        evolution.load_persisted_state().await;

        let all = evolution.get_all_stats().await;
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].template_name, "persisted-agent");
        assert_eq!(all[0].total_tasks, 10);
        assert_eq!(all[0].template_version, 3);
    }

    #[tokio::test]
    async fn test_load_persisted_state_or_insert_semantics() {
        // In-memory stats should take precedence over repo stats.
        let repo = Arc::new(MockRefinementRepo::new());
        {
            let mut stats = repo.stats.lock().unwrap();
            let mut s = TemplateStats::new("agent-x".to_string(), 1);
            s.total_tasks = 50;
            s.successful_tasks = 25;
            s.success_rate = 0.5;
            stats.push(s);
        }

        let evolution =
            EvolutionLoop::new(EvolutionConfig::default()).with_repo(repo as Arc<dyn RefinementRepository>);

        // Record an execution first (populates in-memory stats for "agent-x")
        evolution
            .record_execution(make_execution("agent-x", 1, TaskOutcome::Success))
            .await;

        // Now load persisted state — should NOT overwrite the in-memory entry
        evolution.load_persisted_state().await;

        let stats = evolution.get_stats("agent-x").await.unwrap();
        assert_eq!(
            stats.total_tasks, 1,
            "in-memory stats (1 task) must take precedence over repo stats (50 tasks)"
        );
    }

    #[tokio::test]
    async fn test_load_persisted_state_restores_version_changes() {
        let repo = Arc::new(MockRefinementRepo::new());
        {
            let mut changes = repo.version_changes.lock().unwrap();
            let prev_stats = TemplateStats::new("vc-agent".to_string(), 1);
            changes.push(VersionChangeRecord {
                template_name: "vc-agent".to_string(),
                from_version: 1,
                to_version: 2,
                previous_stats: prev_stats,
                changed_at: Utc::now() - Duration::hours(1),
            });
        }

        let evolution =
            EvolutionLoop::new(EvolutionConfig::default()).with_repo(repo as Arc<dyn RefinementRepository>);
        evolution.load_persisted_state().await;

        // Verify that previous_version_stats were restored
        let state = evolution.state.read().await;
        assert!(
            state.previous_version_stats.contains_key("vc-agent"),
            "version change should restore previous_version_stats"
        );
        assert_eq!(
            state.previous_version_stats["vc-agent"].template_version, 1,
            "previous stats should be for version 1"
        );
        assert!(
            state.version_change_times.contains_key("vc-agent"),
            "version change should restore version_change_times"
        );
        assert_eq!(
            state.version_change_times["vc-agent"].0, 2,
            "version_change_times should record to_version=2"
        );
    }

    #[tokio::test]
    async fn test_recover_in_progress_refinements() {
        let repo = Arc::new(MockRefinementRepo::new());
        let request_id = Uuid::new_v4();
        {
            let mut reqs = repo.requests.lock().unwrap();
            reqs.push(RefinementRequest {
                id: request_id,
                template_name: "recover-agent".to_string(),
                template_version: 1,
                severity: RefinementSeverity::Minor,
                trigger: EvolutionTrigger::LowSuccessRate,
                stats: TemplateStats::new("recover-agent".to_string(), 1),
                failed_task_ids: vec![],
                created_at: Utc::now(),
                status: RefinementStatus::InProgress,
            });
        }

        let evolution =
            EvolutionLoop::new(EvolutionConfig::default()).with_repo(repo.clone() as Arc<dyn RefinementRepository>);
        evolution.recover_in_progress_refinements().await;

        // The request should have been reset to Pending in the repo
        let repo_reqs = repo.requests.lock().unwrap();
        assert_eq!(repo_reqs[0].status, RefinementStatus::Pending);
        drop(repo_reqs);

        // And loaded into the in-memory queue
        let pending = evolution.get_pending_refinements().await;
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].id, request_id);
        assert_eq!(pending[0].status, RefinementStatus::Pending);
    }

    #[tokio::test]
    async fn test_load_from_repo_dedup() {
        let repo = Arc::new(MockRefinementRepo::new());
        let shared_id = Uuid::new_v4();
        let request = RefinementRequest {
            id: shared_id,
            template_name: "dedup-agent".to_string(),
            template_version: 1,
            severity: RefinementSeverity::Minor,
            trigger: EvolutionTrigger::LowSuccessRate,
            stats: TemplateStats::new("dedup-agent".to_string(), 1),
            failed_task_ids: vec![],
            created_at: Utc::now(),
            status: RefinementStatus::Pending,
        };
        {
            repo.requests.lock().unwrap().push(request.clone());
        }

        let evolution =
            EvolutionLoop::new(EvolutionConfig::default()).with_repo(repo as Arc<dyn RefinementRepository>);

        // Manually insert the same request into in-memory queue first
        {
            let mut state = evolution.state.write().await;
            state.refinement_queue.push(request);
        }

        // load_from_repo should NOT duplicate
        evolution.load_from_repo().await;

        let pending = evolution.get_pending_refinements().await;
        assert_eq!(
            pending.len(),
            1,
            "duplicate IDs from repo must not create duplicates in queue"
        );
    }

    #[tokio::test]
    async fn test_load_persisted_state_no_repo_is_noop() {
        // No repo attached — should not panic and leave state empty
        let evolution = EvolutionLoop::with_default_config();
        evolution.load_persisted_state().await;

        let all = evolution.get_all_stats().await;
        assert!(all.is_empty(), "no repo means no stats loaded");

        // Also test recover_in_progress_refinements with no repo
        evolution.recover_in_progress_refinements().await;
        let pending = evolution.get_pending_refinements().await;
        assert!(pending.is_empty(), "no repo means no refinements recovered");
    }

    #[test]
    fn test_evolution_config_default_values() {
        let config = EvolutionConfig::default();
        assert_eq!(config.min_tasks_for_evaluation, 5);
        assert!((config.refinement_threshold - 0.60).abs() < f64::EPSILON);
        assert!((config.major_refinement_threshold - 0.40).abs() < f64::EPSILON);
        assert_eq!(config.major_refinement_min_tasks, 10);
        assert_eq!(config.regression_detection_window_hours, 24);
        assert_eq!(config.regression_min_tasks, 3);
        assert!((config.regression_threshold - 0.15).abs() < f64::EPSILON);
        assert!(config.auto_revert_enabled);
        assert_eq!(config.stale_refinement_timeout_hours, 48);
    }

    #[tokio::test]
    async fn test_template_stats_average_computation() {
        let mut stats = TemplateStats::new("avg-test".to_string(), 1);

        // First execution: 10 turns, 1000 tokens
        let exec1 = TaskExecution {
            task_id: Uuid::new_v4(),
            template_name: "avg-test".to_string(),
            template_version: 1,
            outcome: TaskOutcome::Success,
            executed_at: Utc::now(),
            turns_used: 10,
            tokens_used: 1000,
            downstream_tasks: vec![],
        };
        stats.update(&exec1);
        assert!((stats.avg_turns - 10.0).abs() < f64::EPSILON);
        assert!((stats.avg_tokens - 1000.0).abs() < f64::EPSILON);
        assert_eq!(stats.total_tasks, 1);

        // Second execution: 20 turns, 3000 tokens → averages become 15.0 and 2000.0
        let exec2 = TaskExecution {
            task_id: Uuid::new_v4(),
            template_name: "avg-test".to_string(),
            template_version: 1,
            outcome: TaskOutcome::Success,
            executed_at: Utc::now(),
            turns_used: 20,
            tokens_used: 3000,
            downstream_tasks: vec![],
        };
        stats.update(&exec2);
        assert!((stats.avg_turns - 15.0).abs() < f64::EPSILON);
        assert!((stats.avg_tokens - 2000.0).abs() < f64::EPSILON);
        assert_eq!(stats.total_tasks, 2);

        // Third execution: 30 turns, 5000 tokens → averages become 20.0 and 3000.0
        let exec3 = TaskExecution {
            task_id: Uuid::new_v4(),
            template_name: "avg-test".to_string(),
            template_version: 1,
            outcome: TaskOutcome::Failure,
            executed_at: Utc::now(),
            turns_used: 30,
            tokens_used: 5000,
            downstream_tasks: vec![],
        };
        stats.update(&exec3);
        assert!((stats.avg_turns - 20.0).abs() < f64::EPSILON);
        assert!((stats.avg_tokens - 3000.0).abs() < f64::EPSILON);
        assert_eq!(stats.total_tasks, 3);
        assert_eq!(stats.successful_tasks, 2);
        assert_eq!(stats.failed_tasks, 1);
    }

    #[test]
    fn test_refinement_request_new_defaults() {
        let stats = TemplateStats::new("req-test".to_string(), 3);
        let failed_ids = vec![Uuid::new_v4(), Uuid::new_v4()];
        let before = Utc::now();

        let request = RefinementRequest::new(
            "req-test".to_string(),
            3,
            RefinementSeverity::Major,
            EvolutionTrigger::LowSuccessRate,
            stats,
            failed_ids.clone(),
        );

        let after = Utc::now();

        assert_eq!(request.template_name, "req-test");
        assert_eq!(request.template_version, 3);
        assert_eq!(request.severity, RefinementSeverity::Major);
        assert_eq!(request.trigger, EvolutionTrigger::LowSuccessRate);
        assert_eq!(request.status, RefinementStatus::Pending);
        assert_eq!(request.failed_task_ids.len(), 2);
        assert_eq!(request.failed_task_ids, failed_ids);
        assert!(request.created_at >= before && request.created_at <= after);
        // id must be a valid non-nil UUID
        assert_ne!(request.id, Uuid::nil());
    }
}
