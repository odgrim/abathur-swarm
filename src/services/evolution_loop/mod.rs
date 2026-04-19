//! Evolution Loop service for agent template refinement.
//!
//! Implements the evolution paradigm from the design docs:
//! 1. Execute: Run agents on tasks, track outcomes
//! 2. Evaluate: Measure effectiveness based on success rate
//! 3. Evolve: Refine struggling templates, version all changes

use async_trait::async_trait;
#[cfg(test)]
use chrono::Duration;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use uuid::Uuid;

use crate::domain::ports::AgentRepository;

mod evaluate;
mod outcomes;
mod persistence;
mod refine;
mod timeouts;

#[cfg(test)]
mod tests;

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
pub(crate) struct EvolutionState {
    /// Task executions by template name.
    pub(crate) executions: HashMap<String, Vec<TaskExecution>>,
    /// Computed stats by template name.
    pub(crate) stats: HashMap<String, TemplateStats>,
    /// Pending refinement requests.
    pub(crate) refinement_queue: Vec<RefinementRequest>,
    /// Evolution events for audit.
    pub(crate) events: Vec<EvolutionEvent>,
    /// Previous version stats for regression detection.
    pub(crate) previous_version_stats: HashMap<String, TemplateStats>,
    /// Version change timestamps.
    pub(crate) version_change_times: HashMap<String, (u32, DateTime<Utc>)>,
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
    pub(crate) config: EvolutionConfig,
    pub(crate) state: Arc<RwLock<EvolutionState>>,
    pub(crate) refinement_repo: Option<Arc<dyn RefinementRepository>>,
    pub(crate) agent_repo: Option<Arc<dyn AgentRepository>>,
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
