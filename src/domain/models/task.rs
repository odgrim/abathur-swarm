//! Task domain model.
//!
//! Tasks are discrete units of work that agents execute.
//! They form a DAG with dependencies.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Status of a task in the execution pipeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    /// Task is defined but dependencies not met
    Pending,
    /// Task is ready to be picked up (dependencies met)
    Ready,
    /// Task is blocked by failed dependencies
    Blocked,
    /// Task is currently being executed
    Running,
    /// Task execution finished, awaiting post-completion verification
    Validating,
    /// Task completed successfully
    Complete,
    /// Task failed during execution
    Failed,
    /// Task was cancelled
    Canceled,
}

impl Default for TaskStatus {
    fn default() -> Self {
        Self::Pending
    }
}

impl TaskStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Ready => "ready",
            Self::Blocked => "blocked",
            Self::Running => "running",
            Self::Validating => "validating",
            Self::Complete => "complete",
            Self::Failed => "failed",
            Self::Canceled => "canceled",
        }
    }

    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "pending" => Some(Self::Pending),
            "ready" => Some(Self::Ready),
            "blocked" => Some(Self::Blocked),
            "running" => Some(Self::Running),
            "validating" => Some(Self::Validating),
            "complete" | "completed" => Some(Self::Complete),
            "failed" => Some(Self::Failed),
            "canceled" | "cancelled" => Some(Self::Canceled),
            _ => None,
        }
    }

    /// Check if this is a terminal state.
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Complete | Self::Failed | Self::Canceled)
    }

    /// Check if this is an active (non-terminal) state.
    pub fn is_active(&self) -> bool {
        !self.is_terminal()
    }

    /// Valid transitions from this status.
    pub fn valid_transitions(&self) -> &'static [TaskStatus] {
        match self {
            Self::Pending => &[Self::Ready, Self::Blocked, Self::Canceled],
            Self::Ready => &[Self::Running, Self::Blocked, Self::Canceled, Self::Pending],
            Self::Blocked => &[Self::Ready, Self::Canceled],
            Self::Running => &[Self::Validating, Self::Complete, Self::Failed, Self::Canceled],
            Self::Validating => &[Self::Complete, Self::Failed, Self::Canceled],
            Self::Complete => &[],
            Self::Failed => &[Self::Ready], // Can retry
            Self::Canceled => &[],
        }
    }

    pub fn can_transition_to(&self, new_status: Self) -> bool {
        self.valid_transitions().contains(&new_status)
    }
}

/// Where a task originated from.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum TaskSource {
    /// Task submitted directly by a human
    Human,
    /// Task created by the system (e.g., specialist triggers, diagnostics)
    System,
    /// Subtask spawned by another task during execution
    SubtaskOf(Uuid),
    /// Task created by a periodic schedule
    Schedule(Uuid),
    /// Task ingested from an external system via a named adapter
    Adapter(String),
}

impl Default for TaskSource {
    fn default() -> Self {
        Self::Human
    }
}

/// What kind of work this task represents.
///
/// This is a semantic discriminator for the class of work, orthogonal to
/// status, source, or execution mode. It enables filtering and display
/// customization per task category.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskType {
    /// General-purpose implementation task (default).
    Standard,
    /// Intent verification — LLM-based evaluation of whether work satisfies original intent.
    Verification,
    /// Research or analysis task (read-only, produces findings).
    Research,
    /// Code review task.
    Review,
}

impl Default for TaskType {
    fn default() -> Self {
        Self::Standard
    }
}

impl TaskType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Standard => "standard",
            Self::Verification => "verification",
            Self::Research => "research",
            Self::Review => "review",
        }
    }

    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "standard" => Some(Self::Standard),
            "verification" => Some(Self::Verification),
            "research" => Some(Self::Research),
            "review" => Some(Self::Review),
            _ => None,
        }
    }

    /// Whether this task type is a verification task.
    pub fn is_verification(&self) -> bool {
        matches!(self, Self::Verification)
    }

    /// Whether this task type is a review task.
    pub fn is_review(&self) -> bool {
        matches!(self, Self::Review)
    }
}

/// Priority level for tasks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskPriority {
    Low = 1,
    Normal = 2,
    High = 3,
    Critical = 4,
}

impl Default for TaskPriority {
    fn default() -> Self {
        Self::Normal
    }
}

impl TaskPriority {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Normal => "normal",
            Self::High => "high",
            Self::Critical => "critical",
        }
    }

    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "low" => Some(Self::Low),
            "normal" => Some(Self::Normal),
            "high" => Some(Self::High),
            "critical" => Some(Self::Critical),
            _ => None,
        }
    }
}

/// Complexity classification for routing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Complexity {
    Trivial,
    Simple,
    Moderate,
    Complex,
}

impl Default for Complexity {
    fn default() -> Self {
        Self::Moderate
    }
}

/// Controls which system prompt sections are included for a task.
///
/// Simpler tiers skip expensive context sections (goals, project context,
/// iteration context) to reduce input tokens for cheap/simple tasks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PromptTier {
    /// All context sections included.
    Full,
    /// Skip project context, goal context, and iteration context.
    Standard,
    /// Minimal: base prompt only, skip constraints, artifacts, MCP URLs too.
    Minimal,
}

impl Default for PromptTier {
    fn default() -> Self {
        Self::Full
    }
}

impl PromptTier {
    /// Whether this tier includes goal context.
    pub fn include_goal_context(&self) -> bool {
        matches!(self, Self::Full)
    }

    /// Whether this tier includes project context.
    pub fn include_project_context(&self) -> bool {
        matches!(self, Self::Full)
    }

    /// Whether this tier includes agent constraints.
    pub fn include_constraints(&self) -> bool {
        matches!(self, Self::Full | Self::Standard)
    }

    /// Whether this tier includes upstream artifacts.
    pub fn include_upstream_artifacts(&self) -> bool {
        matches!(self, Self::Full | Self::Standard)
    }

    /// Whether this tier includes MCP URLs.
    pub fn include_mcp_urls(&self) -> bool {
        matches!(self, Self::Full | Self::Standard)
    }

    /// Whether this tier includes iteration context.
    pub fn include_iteration_context(&self) -> bool {
        matches!(self, Self::Full)
    }

    /// Whether this tier includes git instructions.
    pub fn include_git_instructions(&self) -> bool {
        matches!(self, Self::Full | Self::Standard)
    }
}

/// How a task should be executed.
///
/// Direct mode is the default: a single substrate invocation.
/// Convergent mode wraps repeated invocations with strategy selection,
/// overseer measurement, and attractor tracking.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum ExecutionMode {
    /// Single-shot substrate invocation. Agent runs once; result is
    /// accepted or the task fails.
    Direct,

    /// Convergence-guided iterative execution. The convergence engine
    /// wraps repeated substrate invocations with strategy selection,
    /// overseer measurement, and attractor tracking.
    Convergent {
        /// When Some(n), spawns n parallel trajectories and selects the best.
        /// When None, uses sequential mode (default).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        parallel_samples: Option<u32>,
    },
}

impl Default for ExecutionMode {
    fn default() -> Self {
        Self::Direct
    }
}

impl ExecutionMode {
    /// Whether this is convergent mode.
    pub fn is_convergent(&self) -> bool {
        matches!(self, Self::Convergent { .. })
    }

    /// Whether this is direct mode.
    pub fn is_direct(&self) -> bool {
        matches!(self, Self::Direct)
    }

    /// Get parallel samples count if in convergent mode.
    pub fn parallel_samples(&self) -> Option<u32> {
        match self {
            Self::Convergent { parallel_samples } => *parallel_samples,
            Self::Direct => None,
        }
    }
}

/// Hints for agent routing.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoutingHints {
    /// Preferred agent type
    pub preferred_agent: Option<String>,
    /// Required tools/capabilities
    pub required_tools: Vec<String>,
    /// Estimated complexity
    pub complexity: Complexity,
    /// Prompt tier for context assembly
    pub prompt_tier: PromptTier,
    /// Workflow template name for phase-orchestrated execution.
    /// When set, the task is routed to the PhaseOrchestrator instead of
    /// the Overmind fallback. Use "external" for adapter-sourced tasks.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workflow_name: Option<String>,
}

/// Type of artifact produced.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactType {
    File,
    Code,
    Document,
    Data,
    Other,
}

/// Reference to an artifact produced by a task.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactRef {
    /// URI (e.g., worktree://task-id/path/to/file)
    pub uri: String,
    /// Type of artifact
    pub artifact_type: ArtifactType,
    /// Optional checksum
    pub checksum: Option<String>,
}

/// Context passed to an agent for task execution.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TaskContext {
    /// Input data/instructions
    pub input: String,
    /// Additional hints
    pub hints: Vec<String>,
    /// Relevant file paths
    pub relevant_files: Vec<String>,
    /// Custom key-value pairs
    pub custom: std::collections::HashMap<String, serde_json::Value>,
}

impl TaskContext {
    /// Maximum number of hints retained in the context.
    ///
    /// When this cap is reached, the oldest hints are evicted from the front
    /// of the vector so that the most recent hints are always preserved.
    pub const MAX_HINTS: usize = 20;

    /// Push a hint, enforcing the [`MAX_HINTS`](Self::MAX_HINTS) cap.
    ///
    /// If the hints vector would exceed `MAX_HINTS` after the push, the
    /// oldest entries are drained from the front until the length is within
    /// the cap. This retains the most recently added hints.
    pub fn push_hint_bounded(&mut self, hint: String) {
        self.hints.push(hint);
        if self.hints.len() > Self::MAX_HINTS {
            let excess = self.hints.len() - Self::MAX_HINTS;
            self.hints.drain(..excess);
        }
    }
}

/// A discrete unit of work that can be executed by an agent.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Task {
    /// Unique identifier
    pub id: Uuid,
    /// Parent task (for subtasks)
    pub parent_id: Option<Uuid>,
    /// Human-readable title
    pub title: String,
    /// Detailed description/prompt
    pub description: String,
    /// Assigned agent type
    pub agent_type: Option<String>,
    /// Routing hints
    pub routing_hints: RoutingHints,
    /// Task IDs this depends on
    pub depends_on: Vec<Uuid>,
    /// Current status
    pub status: TaskStatus,
    /// Priority
    pub priority: TaskPriority,
    /// Retry count
    pub retry_count: u32,
    /// Maximum retries
    pub max_retries: u32,
    /// Produced artifacts
    pub artifacts: Vec<ArtifactRef>,
    /// Worktree path (if using git isolation)
    pub worktree_path: Option<String>,
    /// Execution context
    pub context: TaskContext,
    /// Where this task originated from
    pub source: TaskSource,
    /// When created
    pub created_at: DateTime<Utc>,
    /// When last updated
    pub updated_at: DateTime<Utc>,
    /// When execution started
    pub started_at: Option<DateTime<Utc>>,
    /// When execution completed
    pub completed_at: Option<DateTime<Utc>>,
    /// Optional deadline for SLA enforcement
    pub deadline: Option<DateTime<Utc>>,
    /// Version for optimistic locking
    pub version: u64,
    /// Idempotency key for deduplication
    pub idempotency_key: Option<String>,
    /// How this task should be executed (direct or convergent).
    pub execution_mode: ExecutionMode,
    /// Trajectory ID for convergent tasks (links to convergence engine state).
    pub trajectory_id: Option<Uuid>,
    /// What kind of work this task represents (standard, verification, research, review).
    pub task_type: TaskType,
}

impl Task {
    /// Create a new task from a prompt. Title is auto-generated.
    pub fn new(prompt: impl Into<String>) -> Self {
        let description = prompt.into();
        let title = generate_title(&description);
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            parent_id: None,
            title,
            description,
            agent_type: None,
            routing_hints: RoutingHints::default(),
            depends_on: Vec::new(),
            status: TaskStatus::default(),
            priority: TaskPriority::default(),
            retry_count: 0,
            max_retries: 3,
            artifacts: Vec::new(),
            worktree_path: None,
            context: TaskContext::default(),
            source: TaskSource::default(),
            created_at: now,
            updated_at: now,
            started_at: None,
            completed_at: None,
            deadline: None,
            version: 1,
            idempotency_key: None,
            execution_mode: ExecutionMode::default(),
            trajectory_id: None,
            task_type: TaskType::default(),
        }
    }

    /// Create a new task with an explicit title and prompt/description.
    pub fn with_title(title: impl Into<String>, description: impl Into<String>) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            parent_id: None,
            title: title.into(),
            description: description.into(),
            agent_type: None,
            routing_hints: RoutingHints::default(),
            depends_on: Vec::new(),
            status: TaskStatus::default(),
            priority: TaskPriority::default(),
            retry_count: 0,
            max_retries: 3,
            artifacts: Vec::new(),
            worktree_path: None,
            context: TaskContext::default(),
            source: TaskSource::default(),
            created_at: now,
            updated_at: now,
            started_at: None,
            completed_at: None,
            deadline: None,
            version: 1,
            idempotency_key: None,
            execution_mode: ExecutionMode::default(),
            trajectory_id: None,
            task_type: TaskType::default(),
        }
    }

    /// Set parent task.
    pub fn with_parent(mut self, parent_id: Uuid) -> Self {
        self.parent_id = Some(parent_id);
        self
    }

    /// Add a dependency.
    pub fn with_dependency(mut self, task_id: Uuid) -> Self {
        if !self.depends_on.contains(&task_id) && task_id != self.id {
            self.depends_on.push(task_id);
        }
        self
    }

    /// Set priority.
    pub fn with_priority(mut self, priority: TaskPriority) -> Self {
        self.priority = priority;
        self
    }

    /// Set agent type.
    pub fn with_agent(mut self, agent_type: impl Into<String>) -> Self {
        self.agent_type = Some(agent_type.into());
        self
    }

    /// Set task source.
    pub fn with_source(mut self, source: TaskSource) -> Self {
        self.source = source;
        self
    }

    /// Set deadline for SLA enforcement.
    pub fn with_deadline(mut self, deadline: DateTime<Utc>) -> Self {
        self.deadline = Some(deadline);
        self
    }

    /// Set idempotency key.
    pub fn with_idempotency_key(mut self, key: impl Into<String>) -> Self {
        self.idempotency_key = Some(key.into());
        self
    }

    /// Set execution mode.
    pub fn with_execution_mode(mut self, mode: ExecutionMode) -> Self {
        self.execution_mode = mode;
        self
    }

    /// Set task type.
    pub fn with_task_type(mut self, task_type: TaskType) -> Self {
        self.task_type = task_type;
        self
    }

    /// Set the initial status of a newly created task (builder method).
    ///
    /// This is intended for tasks that need to start in a non-default status
    /// (e.g., `Running` for verification tasks). Only call on freshly created
    /// tasks before persistence.
    pub fn with_initial_status(mut self, status: TaskStatus) -> Self {
        self.status = status;
        self.updated_at = Utc::now();
        // Update timestamps for the initial status
        match status {
            TaskStatus::Running => self.started_at = Some(Utc::now()),
            TaskStatus::Complete | TaskStatus::Failed | TaskStatus::Canceled => {
                self.completed_at = Some(Utc::now());
            }
            _ => {}
        }
        self
    }

    /// Check if can transition to given status.
    pub fn can_transition_to(&self, new_status: TaskStatus) -> bool {
        self.status.can_transition_to(new_status)
    }

    /// Transition to new status.
    pub fn transition_to(&mut self, new_status: TaskStatus) -> Result<(), String> {
        if !self.can_transition_to(new_status) {
            return Err(format!(
                "Cannot transition from {} to {}",
                self.status.as_str(),
                new_status.as_str()
            ));
        }

        self.status = new_status;
        self.updated_at = Utc::now();
        self.version += 1;

        // Update timestamps
        match new_status {
            TaskStatus::Running => self.started_at = Some(Utc::now()),
            TaskStatus::Complete | TaskStatus::Failed | TaskStatus::Canceled => {
                self.completed_at = Some(Utc::now());
            }
            _ => {}
        }

        Ok(())
    }

    /// Force a status transition, bypassing the state machine.
    ///
    /// This should only be used in exceptional circumstances such as crash
    /// recovery, startup reconciliation, or test setup. A tracing warning is
    /// emitted every time this is called so that bypass sites are visible in
    /// logs.
    ///
    /// Timestamps (`updated_at`, `started_at`, `completed_at`) and `version`
    /// are updated consistently with [`transition_to`].
    pub fn force_status(&mut self, new_status: TaskStatus, reason: &str) {
        tracing::warn!(
            task_id = %self.id,
            from = %self.status.as_str(),
            to = %new_status.as_str(),
            reason = reason,
            "Forcing task status transition (bypassing state machine)"
        );

        self.status = new_status;
        self.updated_at = Utc::now();
        self.version += 1;

        // Update timestamps consistently with transition_to
        match new_status {
            TaskStatus::Running => self.started_at = Some(Utc::now()),
            TaskStatus::Complete | TaskStatus::Failed | TaskStatus::Canceled => {
                self.completed_at = Some(Utc::now());
            }
            _ => {}
        }
    }

    /// Check if task is terminal.
    pub fn is_terminal(&self) -> bool {
        self.status.is_terminal()
    }

    /// Check if task can be retried.
    pub fn can_retry(&self) -> bool {
        self.status == TaskStatus::Failed && self.retry_count < self.max_retries
    }

    /// Increment retry count and reset to Ready.
    pub fn retry(&mut self) -> Result<(), String> {
        if !self.can_retry() {
            return Err("Cannot retry: either not failed or max retries reached".to_string());
        }
        self.retry_count += 1;
        self.transition_to(TaskStatus::Ready)
    }

    /// Validate task.
    pub fn validate(&self) -> Result<(), String> {
        if self.title.is_empty() {
            return Err("Task title cannot be empty".to_string());
        }
        if self.description.trim().is_empty() {
            return Err("Task prompt cannot be empty".to_string());
        }
        if self.depends_on.contains(&self.id) {
            return Err("Task cannot depend on itself".to_string());
        }
        Ok(())
    }
}

/// Generate a short title from a prompt string.
/// Takes the first line, truncates at ~80 chars on a word boundary.
fn generate_title(prompt: &str) -> String {
    let first_line = prompt.lines().next().unwrap_or(prompt).trim();
    if first_line.is_empty() {
        return "Untitled task".to_string();
    }
    let max_len = 80;
    if first_line.len() <= max_len {
        return first_line.to_string();
    }
    match first_line[..max_len].rfind(' ') {
        Some(pos) => format!("{}...", &first_line[..pos]),
        None => format!("{}...", &first_line[..max_len]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_task_creation_from_prompt() {
        let task = Task::new("Implement the login feature");
        assert_eq!(task.title, "Implement the login feature");
        assert_eq!(task.description, "Implement the login feature");
        assert_eq!(task.status, TaskStatus::Pending);
    }

    #[test]
    fn test_task_creation_with_title() {
        let task = Task::with_title("Test Task", "Description");
        assert_eq!(task.title, "Test Task");
        assert_eq!(task.description, "Description");
        assert_eq!(task.status, TaskStatus::Pending);
    }

    #[test]
    fn test_generate_title() {
        // Short prompt: title equals full prompt
        assert_eq!(generate_title("Short prompt"), "Short prompt");

        // Multi-line: takes first line only
        assert_eq!(generate_title("First line\nSecond line"), "First line");

        // Long prompt: truncates at word boundary
        let long = "This is a very long prompt that exceeds eighty characters and should be truncated at a word boundary somewhere";
        let title = generate_title(long);
        assert!(title.len() <= 84); // 80 + "..."
        assert!(title.ends_with("..."));
    }

    #[test]
    fn test_task_state_transitions() {
        let mut task = Task::new("Test task description");

        // Pending -> Ready
        assert!(task.can_transition_to(TaskStatus::Ready));
        task.transition_to(TaskStatus::Ready).unwrap();
        assert_eq!(task.status, TaskStatus::Ready);

        // Ready -> Running
        task.transition_to(TaskStatus::Running).unwrap();
        assert!(task.started_at.is_some());

        // Running -> Complete
        task.transition_to(TaskStatus::Complete).unwrap();
        assert!(task.completed_at.is_some());
        assert!(task.is_terminal());
    }

    #[test]
    fn test_task_retry() {
        let mut task = Task::new("Test task description");
        task.force_status(TaskStatus::Failed, "test setup");

        assert!(task.can_retry());
        task.retry().unwrap();
        assert_eq!(task.status, TaskStatus::Ready);
        assert_eq!(task.retry_count, 1);
    }

    #[test]
    fn test_task_dependencies() {
        let dep_id = Uuid::new_v4();
        let task = Task::new("Test task description")
            .with_dependency(dep_id);

        assert!(task.depends_on.contains(&dep_id));
    }

    #[test]
    fn test_validating_transitions() {
        // Running -> Validating -> Complete
        let mut task = Task::new("Test validating flow");
        task.transition_to(TaskStatus::Ready).unwrap();
        task.transition_to(TaskStatus::Running).unwrap();
        task.transition_to(TaskStatus::Validating).unwrap();
        assert_eq!(task.status, TaskStatus::Validating);
        assert!(!task.is_terminal());
        assert!(task.completed_at.is_none());
        task.transition_to(TaskStatus::Complete).unwrap();
        assert!(task.is_terminal());
        assert!(task.completed_at.is_some());

        // Running -> Validating -> Failed
        let mut task2 = Task::new("Test validating failure");
        task2.transition_to(TaskStatus::Ready).unwrap();
        task2.transition_to(TaskStatus::Running).unwrap();
        task2.transition_to(TaskStatus::Validating).unwrap();
        task2.transition_to(TaskStatus::Failed).unwrap();
        assert!(task2.is_terminal());
        assert!(task2.completed_at.is_some());
    }

    #[test]
    fn test_task_validation() {
        // Empty title via with_title
        let task = Task::with_title("", "Some prompt");
        assert!(task.validate().is_err());

        // Empty prompt
        let task = Task::with_title("Valid Title", "");
        assert!(task.validate().is_err());

        // Whitespace-only prompt
        let task = Task::with_title("Valid Title", "   ");
        assert!(task.validate().is_err());

        // Valid task
        let task = Task::new("Valid prompt");
        assert!(task.validate().is_ok());
    }

    #[test]
    fn test_push_hint_bounded_caps_at_max() {
        let mut ctx = TaskContext::default();
        let total = TaskContext::MAX_HINTS + 5;

        for i in 0..total {
            ctx.push_hint_bounded(format!("hint-{}", i));
        }

        // The vec must not exceed MAX_HINTS.
        assert_eq!(ctx.hints.len(), TaskContext::MAX_HINTS);

        // The most recent hints must be retained (oldest were evicted).
        let first_retained = total - TaskContext::MAX_HINTS;
        assert_eq!(ctx.hints[0], format!("hint-{}", first_retained));
        assert_eq!(
            ctx.hints[TaskContext::MAX_HINTS - 1],
            format!("hint-{}", total - 1)
        );
    }

    #[test]
    fn test_push_hint_bounded_under_cap() {
        let mut ctx = TaskContext::default();

        for i in 0..5 {
            ctx.push_hint_bounded(format!("hint-{}", i));
        }

        // Under the cap, all hints are retained.
        assert_eq!(ctx.hints.len(), 5);
        assert_eq!(ctx.hints[0], "hint-0");
        assert_eq!(ctx.hints[4], "hint-4");
    }

    #[test]
    fn test_push_hint_bounded_at_exact_cap() {
        let mut ctx = TaskContext::default();

        // Push exactly MAX_HINTS items — no eviction should occur.
        for i in 0..TaskContext::MAX_HINTS {
            ctx.push_hint_bounded(format!("hint-{}", i));
        }

        assert_eq!(ctx.hints.len(), TaskContext::MAX_HINTS);
        assert_eq!(ctx.hints[0], "hint-0");
        assert_eq!(
            ctx.hints[TaskContext::MAX_HINTS - 1],
            format!("hint-{}", TaskContext::MAX_HINTS - 1)
        );
    }

    // =========================================================================
    // State machine enforcement tests (GitHub #54)
    // =========================================================================

    #[test]
    fn test_all_valid_transitions_succeed() {
        // Pending → Ready
        let mut t = Task::new("p");
        assert!(t.transition_to(TaskStatus::Ready).is_ok());

        // Pending → Blocked
        let mut t = Task::new("p");
        assert!(t.transition_to(TaskStatus::Blocked).is_ok());

        // Pending → Canceled
        let mut t = Task::new("p");
        assert!(t.transition_to(TaskStatus::Canceled).is_ok());

        // Ready → Running
        let mut t = Task::new("p");
        t.transition_to(TaskStatus::Ready).unwrap();
        assert!(t.transition_to(TaskStatus::Running).is_ok());

        // Ready → Blocked
        let mut t = Task::new("p");
        t.transition_to(TaskStatus::Ready).unwrap();
        assert!(t.transition_to(TaskStatus::Blocked).is_ok());

        // Ready → Canceled
        let mut t = Task::new("p");
        t.transition_to(TaskStatus::Ready).unwrap();
        assert!(t.transition_to(TaskStatus::Canceled).is_ok());

        // Ready → Pending (new transition)
        let mut t = Task::new("p");
        t.transition_to(TaskStatus::Ready).unwrap();
        assert!(t.transition_to(TaskStatus::Pending).is_ok());

        // Blocked → Ready
        let mut t = Task::new("p");
        t.transition_to(TaskStatus::Blocked).unwrap();
        assert!(t.transition_to(TaskStatus::Ready).is_ok());

        // Blocked → Canceled
        let mut t = Task::new("p");
        t.transition_to(TaskStatus::Blocked).unwrap();
        assert!(t.transition_to(TaskStatus::Canceled).is_ok());

        // Running → Validating
        let mut t = Task::new("p");
        t.transition_to(TaskStatus::Ready).unwrap();
        t.transition_to(TaskStatus::Running).unwrap();
        assert!(t.transition_to(TaskStatus::Validating).is_ok());

        // Running → Complete
        let mut t = Task::new("p");
        t.transition_to(TaskStatus::Ready).unwrap();
        t.transition_to(TaskStatus::Running).unwrap();
        assert!(t.transition_to(TaskStatus::Complete).is_ok());

        // Running → Failed
        let mut t = Task::new("p");
        t.transition_to(TaskStatus::Ready).unwrap();
        t.transition_to(TaskStatus::Running).unwrap();
        assert!(t.transition_to(TaskStatus::Failed).is_ok());

        // Running → Canceled
        let mut t = Task::new("p");
        t.transition_to(TaskStatus::Ready).unwrap();
        t.transition_to(TaskStatus::Running).unwrap();
        assert!(t.transition_to(TaskStatus::Canceled).is_ok());

        // Validating → Complete
        let mut t = Task::new("p");
        t.transition_to(TaskStatus::Ready).unwrap();
        t.transition_to(TaskStatus::Running).unwrap();
        t.transition_to(TaskStatus::Validating).unwrap();
        assert!(t.transition_to(TaskStatus::Complete).is_ok());

        // Validating → Failed
        let mut t = Task::new("p");
        t.transition_to(TaskStatus::Ready).unwrap();
        t.transition_to(TaskStatus::Running).unwrap();
        t.transition_to(TaskStatus::Validating).unwrap();
        assert!(t.transition_to(TaskStatus::Failed).is_ok());

        // Validating → Canceled (new transition)
        let mut t = Task::new("p");
        t.transition_to(TaskStatus::Ready).unwrap();
        t.transition_to(TaskStatus::Running).unwrap();
        t.transition_to(TaskStatus::Validating).unwrap();
        assert!(t.transition_to(TaskStatus::Canceled).is_ok());

        // Failed → Ready (retry)
        let mut t = Task::new("p");
        t.transition_to(TaskStatus::Ready).unwrap();
        t.transition_to(TaskStatus::Running).unwrap();
        t.transition_to(TaskStatus::Failed).unwrap();
        assert!(t.transition_to(TaskStatus::Ready).is_ok());
    }

    #[test]
    fn test_invalid_transitions_return_err() {
        // Pending → Running (must go through Ready)
        let mut t = Task::new("p");
        assert!(t.transition_to(TaskStatus::Running).is_err());

        // Pending → Complete
        let mut t = Task::new("p");
        assert!(t.transition_to(TaskStatus::Complete).is_err());

        // Pending → Failed
        let mut t = Task::new("p");
        assert!(t.transition_to(TaskStatus::Failed).is_err());

        // Ready → Complete (must go through Running)
        let mut t = Task::new("p");
        t.transition_to(TaskStatus::Ready).unwrap();
        assert!(t.transition_to(TaskStatus::Complete).is_err());

        // Blocked → Running (must go through Ready)
        let mut t = Task::new("p");
        t.transition_to(TaskStatus::Blocked).unwrap();
        assert!(t.transition_to(TaskStatus::Running).is_err());

        // Validating → Running (cannot go back)
        let mut t = Task::new("p");
        t.transition_to(TaskStatus::Ready).unwrap();
        t.transition_to(TaskStatus::Running).unwrap();
        t.transition_to(TaskStatus::Validating).unwrap();
        assert!(t.transition_to(TaskStatus::Running).is_err());
    }

    #[test]
    fn test_terminal_state_enforcement() {
        // Complete → anything
        let mut t = Task::new("p");
        t.transition_to(TaskStatus::Ready).unwrap();
        t.transition_to(TaskStatus::Running).unwrap();
        t.transition_to(TaskStatus::Complete).unwrap();

        assert!(t.transition_to(TaskStatus::Running).is_err());
        assert!(t.transition_to(TaskStatus::Failed).is_err());
        assert!(t.transition_to(TaskStatus::Pending).is_err());
        assert!(t.transition_to(TaskStatus::Ready).is_err());

        // Canceled → anything
        let mut t = Task::new("p");
        t.transition_to(TaskStatus::Canceled).unwrap();

        assert!(t.transition_to(TaskStatus::Running).is_err());
        assert!(t.transition_to(TaskStatus::Pending).is_err());
        assert!(t.transition_to(TaskStatus::Ready).is_err());
    }

    #[test]
    fn test_self_transition_rejected() {
        let mut t = Task::new("p");
        // Pending → Pending
        assert!(t.transition_to(TaskStatus::Pending).is_err());

        // Ready → Ready
        t.transition_to(TaskStatus::Ready).unwrap();
        assert!(t.transition_to(TaskStatus::Ready).is_err());

        // Running → Running
        t.transition_to(TaskStatus::Running).unwrap();
        assert!(t.transition_to(TaskStatus::Running).is_err());
    }

    #[test]
    fn test_force_status_works_for_any_transition() {
        let mut t = Task::new("p");
        assert_eq!(t.status, TaskStatus::Pending);

        // Force an invalid transition: Pending → Complete
        t.force_status(TaskStatus::Complete, "test: force bypass");
        assert_eq!(t.status, TaskStatus::Complete);
        assert!(t.completed_at.is_some());

        // Force from terminal back to Running (impossible via transition_to)
        t.force_status(TaskStatus::Running, "test: force from terminal");
        assert_eq!(t.status, TaskStatus::Running);
        assert!(t.started_at.is_some());
    }

    #[test]
    fn test_force_status_updates_version() {
        let mut t = Task::new("p");
        let v_before = t.version;
        t.force_status(TaskStatus::Running, "test");
        assert_eq!(t.version, v_before + 1);
    }

    #[test]
    fn test_retry_uses_transition_to() {
        // retry() should go through transition_to, which means
        // it should work from Failed state (Failed→Ready is valid)
        let mut t = Task::new("p");
        t.force_status(TaskStatus::Failed, "test setup");

        let v_before = t.version;
        t.retry().unwrap();
        assert_eq!(t.status, TaskStatus::Ready);
        assert_eq!(t.retry_count, 1);
        // transition_to increments version
        assert_eq!(t.version, v_before + 1);

        // retry() should fail from non-Failed state
        let mut t2 = Task::new("p");
        assert!(t2.retry().is_err());
    }

    #[test]
    fn test_timestamp_side_effects() {
        // Running sets started_at
        let mut t = Task::new("p");
        assert!(t.started_at.is_none());
        t.transition_to(TaskStatus::Ready).unwrap();
        t.transition_to(TaskStatus::Running).unwrap();
        assert!(t.started_at.is_some());

        // Complete sets completed_at
        let mut t = Task::new("p");
        assert!(t.completed_at.is_none());
        t.transition_to(TaskStatus::Ready).unwrap();
        t.transition_to(TaskStatus::Running).unwrap();
        t.transition_to(TaskStatus::Complete).unwrap();
        assert!(t.completed_at.is_some());

        // Failed sets completed_at
        let mut t = Task::new("p");
        t.transition_to(TaskStatus::Ready).unwrap();
        t.transition_to(TaskStatus::Running).unwrap();
        t.transition_to(TaskStatus::Failed).unwrap();
        assert!(t.completed_at.is_some());

        // Canceled sets completed_at
        let mut t = Task::new("p");
        t.transition_to(TaskStatus::Canceled).unwrap();
        assert!(t.completed_at.is_some());
    }

    #[test]
    fn test_transition_error_message_content() {
        let mut t = Task::new("p");
        let err = t.transition_to(TaskStatus::Running).unwrap_err();
        assert!(err.contains("pending"), "Error should mention source status: {}", err);
        assert!(err.contains("running"), "Error should mention target status: {}", err);
    }

    #[test]
    fn test_with_initial_status_builder() {
        let t = Task::new("p").with_initial_status(TaskStatus::Running);
        assert_eq!(t.status, TaskStatus::Running);
        assert!(t.started_at.is_some());

        let t = Task::new("p").with_initial_status(TaskStatus::Complete);
        assert_eq!(t.status, TaskStatus::Complete);
        assert!(t.completed_at.is_some());

        // Default Pending should not set started_at or completed_at
        let t = Task::new("p").with_initial_status(TaskStatus::Pending);
        assert_eq!(t.status, TaskStatus::Pending);
        assert!(t.started_at.is_none());
        assert!(t.completed_at.is_none());
    }

    #[test]
    fn test_new_transitions_validating_to_canceled() {
        let mut t = Task::new("p");
        t.transition_to(TaskStatus::Ready).unwrap();
        t.transition_to(TaskStatus::Running).unwrap();
        t.transition_to(TaskStatus::Validating).unwrap();
        // Validating → Canceled is a newly added valid transition
        assert!(t.can_transition_to(TaskStatus::Canceled));
        assert!(t.transition_to(TaskStatus::Canceled).is_ok());
        assert_eq!(t.status, TaskStatus::Canceled);
        assert!(t.completed_at.is_some());
    }

    #[test]
    fn test_new_transitions_ready_to_pending() {
        let mut t = Task::new("p");
        t.transition_to(TaskStatus::Ready).unwrap();
        // Ready → Pending is a newly added valid transition
        assert!(t.can_transition_to(TaskStatus::Pending));
        assert!(t.transition_to(TaskStatus::Pending).is_ok());
        assert_eq!(t.status, TaskStatus::Pending);
    }

    #[test]
    fn test_valid_transitions_returns_static_slice() {
        // Ensure the return type is a static slice (no allocation per call)
        let transitions = TaskStatus::Pending.valid_transitions();
        assert!(transitions.contains(&TaskStatus::Ready));
        assert!(transitions.contains(&TaskStatus::Blocked));
        assert!(transitions.contains(&TaskStatus::Canceled));
    }
}
