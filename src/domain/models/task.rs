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
    pub fn valid_transitions(&self) -> Vec<TaskStatus> {
        match self {
            Self::Pending => vec![Self::Ready, Self::Blocked, Self::Canceled],
            Self::Ready => vec![Self::Running, Self::Blocked, Self::Canceled],
            Self::Blocked => vec![Self::Ready, Self::Canceled],
            Self::Running => vec![Self::Validating, Self::Complete, Self::Failed, Self::Canceled],
            Self::Validating => vec![Self::Running, Self::Complete, Self::Failed],
            Self::Complete => vec![],
            Self::Failed => vec![Self::Ready], // Can retry
            Self::Canceled => vec![],
        }
    }

    pub fn can_transition_to(&self, new_status: Self) -> bool {
        self.valid_transitions().contains(&new_status)
    }
}

impl std::fmt::Display for TaskStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
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
    /// Workflow name hint (e.g., "code", "analysis", "docs", "review").
    /// When set, the overmind uses this to enroll the task in the named workflow.
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

    /// Check if can transition to given status.
    pub fn can_transition_to(&self, new_status: TaskStatus) -> bool {
        self.status.can_transition_to(new_status)
    }

    /// Transition to new status with strict state machine enforcement.
    pub fn transition_to(&mut self, new_status: TaskStatus) -> Result<(), String> {
        if !self.can_transition_to(new_status) {
            let valid = self.status.valid_transitions();
            let valid_strs: Vec<&str> = valid.iter().map(|s| s.as_str()).collect();
            return Err(format!(
                "Invalid state transition from '{}' to '{}'. Valid transitions from '{}': [{}]",
                self.status.as_str(),
                new_status.as_str(),
                self.status.as_str(),
                if valid_strs.is_empty() {
                    "none (terminal state)".to_string()
                } else {
                    valid_strs.join(", ")
                }
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
        self.status = TaskStatus::Ready;
        self.updated_at = Utc::now();
        self.version += 1;
        Ok(())
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
        task.status = TaskStatus::Failed;

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

    // ===== State Machine Transition Tests =====

    const ALL_STATUSES: [TaskStatus; 8] = [
        TaskStatus::Pending,
        TaskStatus::Ready,
        TaskStatus::Blocked,
        TaskStatus::Running,
        TaskStatus::Validating,
        TaskStatus::Complete,
        TaskStatus::Failed,
        TaskStatus::Canceled,
    ];

    #[test]
    fn test_valid_transitions_pending() {
        let valid = TaskStatus::Pending.valid_transitions();
        assert_eq!(valid, vec![TaskStatus::Ready, TaskStatus::Blocked, TaskStatus::Canceled]);
    }

    #[test]
    fn test_valid_transitions_ready() {
        let valid = TaskStatus::Ready.valid_transitions();
        assert_eq!(valid, vec![TaskStatus::Running, TaskStatus::Blocked, TaskStatus::Canceled]);
    }

    #[test]
    fn test_valid_transitions_blocked() {
        let valid = TaskStatus::Blocked.valid_transitions();
        assert_eq!(valid, vec![TaskStatus::Ready, TaskStatus::Canceled]);
    }

    #[test]
    fn test_valid_transitions_running() {
        let valid = TaskStatus::Running.valid_transitions();
        assert_eq!(valid, vec![TaskStatus::Validating, TaskStatus::Complete, TaskStatus::Failed, TaskStatus::Canceled]);
    }

    #[test]
    fn test_valid_transitions_validating() {
        let valid = TaskStatus::Validating.valid_transitions();
        assert_eq!(valid, vec![TaskStatus::Running, TaskStatus::Complete, TaskStatus::Failed]);
    }

    #[test]
    fn test_valid_transitions_complete() {
        let valid = TaskStatus::Complete.valid_transitions();
        assert!(valid.is_empty(), "Terminal state Complete should have no valid transitions");
    }

    #[test]
    fn test_valid_transitions_failed() {
        let valid = TaskStatus::Failed.valid_transitions();
        assert_eq!(valid, vec![TaskStatus::Ready]);
    }

    #[test]
    fn test_valid_transitions_canceled() {
        let valid = TaskStatus::Canceled.valid_transitions();
        assert!(valid.is_empty(), "Terminal state Canceled should have no valid transitions");
    }

    #[test]
    fn test_can_transition_to_matches_valid_transitions_exhaustive() {
        // Verify that the optimized matches! implementation agrees with
        // valid_transitions() for all 64 (8x8) state pairs.
        for from in &ALL_STATUSES {
            let valid = from.valid_transitions();
            for to in &ALL_STATUSES {
                let expected = valid.contains(to);
                assert_eq!(
                    from.can_transition_to(*to),
                    expected,
                    "can_transition_to({} -> {}) returned {} but valid_transitions says {}",
                    from.as_str(),
                    to.as_str(),
                    from.can_transition_to(*to),
                    expected
                );
            }
        }
    }

    #[test]
    fn test_invalid_transitions_rejected() {
        // Every (from, to) pair NOT in valid_transitions must be rejected
        let mut tested_invalid = 0;
        for from in &ALL_STATUSES {
            let valid = from.valid_transitions();
            for to in &ALL_STATUSES {
                if !valid.contains(to) {
                    let mut task = Task::new("Test invalid transition");
                    task.status = *from;
                    let result = task.transition_to(*to);
                    assert!(
                        result.is_err(),
                        "Transition {} -> {} should be rejected but was allowed",
                        from.as_str(),
                        to.as_str(),
                    );
                    tested_invalid += 1;
                }
            }
        }
        // 64 total pairs - 16 valid = 48 invalid (including self-transitions)
        assert!(tested_invalid > 40, "Expected at least 40 invalid transitions tested, got {}", tested_invalid);
    }

    #[test]
    fn test_all_valid_transitions_succeed() {
        let valid_pairs: Vec<(TaskStatus, TaskStatus)> = vec![
            (TaskStatus::Pending, TaskStatus::Ready),
            (TaskStatus::Pending, TaskStatus::Blocked),
            (TaskStatus::Pending, TaskStatus::Canceled),
            (TaskStatus::Ready, TaskStatus::Running),
            (TaskStatus::Ready, TaskStatus::Blocked),
            (TaskStatus::Ready, TaskStatus::Canceled),
            (TaskStatus::Blocked, TaskStatus::Ready),
            (TaskStatus::Blocked, TaskStatus::Canceled),
            (TaskStatus::Running, TaskStatus::Validating),
            (TaskStatus::Running, TaskStatus::Complete),
            (TaskStatus::Running, TaskStatus::Failed),
            (TaskStatus::Running, TaskStatus::Canceled),
            (TaskStatus::Validating, TaskStatus::Running),
            (TaskStatus::Validating, TaskStatus::Complete),
            (TaskStatus::Validating, TaskStatus::Failed),
            (TaskStatus::Failed, TaskStatus::Ready),
        ];

        for (from, to) in &valid_pairs {
            let mut task = Task::new("Test valid transition");
            task.status = *from;
            let initial_version = task.version;
            let result = task.transition_to(*to);
            assert!(
                result.is_ok(),
                "Transition {} -> {} should succeed but failed: {:?}",
                from.as_str(),
                to.as_str(),
                result.err()
            );
            assert_eq!(task.status, *to, "Status should be {} after transition", to.as_str());
            assert_eq!(task.version, initial_version + 1, "Version should increment on transition");
        }
    }

    #[test]
    fn test_terminal_states_reject_all_transitions() {
        let terminal_states = [TaskStatus::Complete, TaskStatus::Canceled];
        for terminal in &terminal_states {
            for target in &ALL_STATUSES {
                let mut task = Task::new("Test terminal rejection");
                task.status = *terminal;
                let result = task.transition_to(*target);
                assert!(
                    result.is_err(),
                    "Terminal state {} should reject transition to {}",
                    terminal.as_str(),
                    target.as_str(),
                );
            }
        }
    }

    #[test]
    fn test_failed_only_allows_ready() {
        for target in &ALL_STATUSES {
            let mut task = Task::new("Test failed transitions");
            task.status = TaskStatus::Failed;
            let result = task.transition_to(*target);
            if *target == TaskStatus::Ready {
                assert!(result.is_ok(), "Failed -> Ready should be allowed (retry)");
            } else {
                assert!(result.is_err(), "Failed -> {} should be rejected", target.as_str());
            }
        }
    }

    #[test]
    fn test_self_transitions_rejected() {
        for status in &ALL_STATUSES {
            let mut task = Task::new("Test self-transition");
            task.status = *status;
            let result = task.transition_to(*status);
            assert!(
                result.is_err(),
                "Self-transition {} -> {} should be rejected",
                status.as_str(),
                status.as_str(),
            );
        }
    }

    #[test]
    fn test_transition_error_message_contains_states_and_valid_targets() {
        let mut task = Task::new("Test error message");
        // Pending cannot go directly to Complete
        let result = task.transition_to(TaskStatus::Complete);
        let err = result.unwrap_err();
        assert!(err.contains("pending"), "Error should contain from-state 'pending': {}", err);
        assert!(err.contains("complete"), "Error should contain to-state 'complete': {}", err);
        assert!(err.contains("ready"), "Error should list valid target 'ready': {}", err);
    }

    #[test]
    fn test_terminal_state_error_message_shows_none() {
        let mut task = Task::new("Test terminal error");
        task.status = TaskStatus::Complete;
        let result = task.transition_to(TaskStatus::Running);
        let err = result.unwrap_err();
        assert!(err.contains("none (terminal state)"), "Error for terminal state should indicate no valid transitions: {}", err);
    }

    #[test]
    fn test_transition_timestamps() {
        // Running -> Complete sets completed_at
        let mut task = Task::new("Test timestamps");
        task.status = TaskStatus::Running;
        assert!(task.completed_at.is_none());
        task.transition_to(TaskStatus::Complete).unwrap();
        assert!(task.completed_at.is_some());

        // Pending -> Ready -> Running sets started_at
        let mut task2 = Task::new("Test timestamps 2");
        assert!(task2.started_at.is_none());
        task2.transition_to(TaskStatus::Ready).unwrap();
        assert!(task2.started_at.is_none());
        task2.transition_to(TaskStatus::Running).unwrap();
        assert!(task2.started_at.is_some());

        // Running -> Canceled sets completed_at
        let mut task3 = Task::new("Test timestamps 3");
        task3.status = TaskStatus::Running;
        task3.transition_to(TaskStatus::Canceled).unwrap();
        assert!(task3.completed_at.is_some());

        // Running -> Failed sets completed_at
        let mut task4 = Task::new("Test timestamps 4");
        task4.status = TaskStatus::Running;
        task4.transition_to(TaskStatus::Failed).unwrap();
        assert!(task4.completed_at.is_some());

        // Running -> Validating does NOT set completed_at
        let mut task5 = Task::new("Test timestamps 5");
        task5.status = TaskStatus::Running;
        task5.transition_to(TaskStatus::Validating).unwrap();
        assert!(task5.completed_at.is_none());
    }

    #[test]
    fn test_retry_from_non_failed_state_rejected() {
        for status in &ALL_STATUSES {
            if *status == TaskStatus::Failed {
                continue;
            }
            let mut task = Task::new("Test retry rejection");
            task.status = *status;
            assert!(
                task.retry().is_err(),
                "retry() should be rejected from state {}",
                status.as_str(),
            );
        }
    }

    #[test]
    fn test_retry_max_retries_exceeded() {
        let mut task = Task::new("Test retry limit");
        task.status = TaskStatus::Failed;
        task.max_retries = 2;
        task.retry_count = 2;
        assert!(!task.can_retry());
        assert!(task.retry().is_err());
    }

    #[test]
    fn test_task_status_display() {
        assert_eq!(format!("{}", TaskStatus::Pending), "pending");
        assert_eq!(format!("{}", TaskStatus::Ready), "ready");
        assert_eq!(format!("{}", TaskStatus::Blocked), "blocked");
        assert_eq!(format!("{}", TaskStatus::Running), "running");
        assert_eq!(format!("{}", TaskStatus::Validating), "validating");
        assert_eq!(format!("{}", TaskStatus::Complete), "complete");
        assert_eq!(format!("{}", TaskStatus::Failed), "failed");
        assert_eq!(format!("{}", TaskStatus::Canceled), "canceled");
    }

}
