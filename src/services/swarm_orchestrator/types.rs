//! Public types for the swarm orchestrator.
//!
//! Configuration structs, event types, status enums, and statistics
//! used throughout the orchestrator subsystem.

use std::path::PathBuf;
use uuid::Uuid;

/// Configuration for the swarm orchestrator.
#[derive(Debug, Clone)]
pub struct SwarmConfig {
    /// Maximum concurrent agents.
    pub max_agents: usize,
    /// Default max turns per agent invocation.
    pub default_max_turns: u32,
    /// Whether to use worktrees for task isolation.
    pub use_worktrees: bool,
    /// Maximum execution time per goal (seconds).
    pub goal_timeout_secs: u64,
    /// Whether to auto-retry failed tasks.
    pub auto_retry: bool,
    /// Maximum retries per task.
    pub max_task_retries: u32,
    /// Base path for worktrees.
    pub worktree_base_path: PathBuf,
    /// Repository path.
    pub repo_path: PathBuf,
    /// Default base ref for worktrees.
    pub default_base_ref: String,
    /// Whether to use LLM for task decomposition.
    pub use_llm_decomposition: bool,
    /// Whether to run integration verification on task completion.
    pub verify_on_completion: bool,
    /// Whether to use merge queue for controlled merging.
    pub use_merge_queue: bool,
    /// Whether to prefer creating pull requests over direct merges.
    /// When true, tries `gh pr create` first; falls back to merge queue on failure.
    pub prefer_pull_requests: bool,
    /// Whether to track agent evolution metrics.
    pub track_evolution: bool,
    /// MCP server addresses for agent access to system services.
    /// These get passed to substrate requests so agents can access memory, tasks, etc.
    pub mcp_servers: McpServerConfig,
    /// Spawn limits for task creation (subtask depth, count, etc.).
    pub spawn_limits: crate::services::config::SpawnLimitsConfig,
    /// Whether to enable intent verification and convergence loops.
    pub enable_intent_verification: bool,
    /// Configuration for convergence behavior.
    pub convergence: ConvergenceLoopConfig,
    /// Interval in seconds for the reconciliation safety-net loop (default: 30).
    pub reconciliation_interval_secs: Option<u64>,
}

/// Configuration for the convergence loop behavior.
#[derive(Debug, Clone)]
pub struct ConvergenceLoopConfig {
    /// Maximum iterations before giving up.
    pub max_iterations: u32,
    /// Minimum confidence to accept partial satisfaction.
    pub min_confidence_threshold: f64,
    /// Whether to require full satisfaction (vs. partial).
    pub require_full_satisfaction: bool,
    /// Whether to automatically retry on partial satisfaction.
    pub auto_retry_partial: bool,
    /// Timeout for the entire convergence loop (seconds).
    pub convergence_timeout_secs: u64,
    /// Verification level: "goal" for goal-level, "wave" for per-wave, "task" for per-task.
    pub verification_level: VerificationLevel,
}

/// Level at which intent verification is performed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VerificationLevel {
    /// Verify only at goal completion (default).
    #[default]
    Goal,
    /// Verify after each wave of parallel tasks completes.
    Wave,
    /// Verify each task individually after completion.
    Task,
}

impl VerificationLevel {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Goal => "goal",
            Self::Wave => "wave",
            Self::Task => "task",
        }
    }
}

impl Default for ConvergenceLoopConfig {
    fn default() -> Self {
        Self {
            max_iterations: 3,
            min_confidence_threshold: 0.7,
            require_full_satisfaction: false,
            auto_retry_partial: true,
            convergence_timeout_secs: 7200, // 2 hours
            verification_level: VerificationLevel::default(),
        }
    }
}

/// MCP server configuration for agent access to system services.
#[derive(Debug, Clone, Default)]
pub struct McpServerConfig {
    /// Memory MCP server address (e.g., "http://localhost:9100")
    pub memory_server: Option<String>,
    /// Tasks MCP server address (e.g., "http://localhost:9101")
    pub tasks_server: Option<String>,
    /// A2A gateway address (e.g., "http://localhost:8080")
    pub a2a_gateway: Option<String>,
    /// Whether to auto-start embedded MCP servers.
    /// When true, the orchestrator will start MCP servers in-process before goal processing.
    pub auto_start_servers: bool,
    /// Host to bind embedded servers to.
    pub bind_host: String,
    /// Base port for embedded servers (memory=base, tasks=base+1, a2a=base+2).
    pub base_port: u16,
}

impl McpServerConfig {
    /// Create a new config with auto-start enabled on default ports.
    pub fn auto_start() -> Self {
        Self {
            memory_server: Some("http://127.0.0.1:9100".to_string()),
            tasks_server: Some("http://127.0.0.1:9101".to_string()),
            a2a_gateway: Some("http://127.0.0.1:8080".to_string()),
            auto_start_servers: true,
            bind_host: "127.0.0.1".to_string(),
            base_port: 9100,
        }
    }
}

impl Default for SwarmConfig {
    fn default() -> Self {
        Self {
            max_agents: 4,
            default_max_turns: 25,
            use_worktrees: true,
            goal_timeout_secs: 3600,
            auto_retry: true,
            max_task_retries: 3,
            worktree_base_path: PathBuf::from(".abathur/worktrees"),
            repo_path: PathBuf::from("."),
            default_base_ref: "main".to_string(),
            use_llm_decomposition: true,
            verify_on_completion: true,
            use_merge_queue: true,
            prefer_pull_requests: true,
            track_evolution: true,
            mcp_servers: McpServerConfig::default(),
            spawn_limits: crate::services::config::SpawnLimitsConfig::default(),
            enable_intent_verification: true,
            convergence: ConvergenceLoopConfig::default(),
            reconciliation_interval_secs: None,
        }
    }
}

/// Orchestrator status.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OrchestratorStatus {
    /// Not running.
    Idle,
    /// Running and processing goals.
    Running,
    /// Paused but can resume.
    Paused,
    /// Shutting down gracefully.
    ShuttingDown,
    /// Stopped.
    Stopped,
}

/// Event emitted by the orchestrator.
#[derive(Debug, Clone)]
pub enum SwarmEvent {
    /// Orchestrator started.
    Started,
    /// Goal processing started.
    GoalStarted { goal_id: Uuid, goal_name: String },
    /// Goal decomposed into tasks.
    GoalDecomposed { goal_id: Uuid, task_count: usize },
    /// Task submitted (created and added to the system).
    TaskSubmitted { task_id: Uuid, task_title: String, goal_id: Uuid },
    /// Task readiness updated.
    TaskReady { task_id: Uuid, task_title: String },
    /// Task spawned.
    TaskSpawned { task_id: Uuid, task_title: String, agent_type: Option<String> },
    /// Worktree created for task.
    WorktreeCreated { task_id: Uuid, path: String },
    /// Task completed.
    TaskCompleted { task_id: Uuid, tokens_used: u64 },
    /// Task failed.
    TaskFailed { task_id: Uuid, error: String, retry_count: u32 },
    /// Task retrying.
    TaskRetrying { task_id: Uuid, attempt: u32, max_attempts: u32 },
    /// Task verified.
    TaskVerified { task_id: Uuid, passed: bool, checks_passed: usize, checks_total: usize },
    /// Task queued for merge.
    TaskQueuedForMerge { task_id: Uuid, stage: String },
    /// Pull request created for completed task.
    PullRequestCreated { task_id: Uuid, pr_url: String, branch: String },
    /// Task claimed by an agent.
    TaskClaimed { task_id: Uuid, agent_type: String },
    /// Task merged successfully.
    TaskMerged { task_id: Uuid, commit_sha: String },
    /// Agent instance completed.
    AgentInstanceCompleted { instance_id: Uuid, task_id: Uuid, tokens_used: u64 },
    /// Reconciliation completed.
    ReconciliationCompleted { corrections_made: u32 },
    /// Evolution event triggered.
    EvolutionTriggered { template_name: String, trigger: String },
    /// Specialist agent spawned for special handling.
    SpecialistSpawned { specialist_type: String, trigger: String, task_id: Option<Uuid> },
    /// Agent dynamically created through capability-driven genesis.
    AgentCreated { agent_type: String, tier: String },
    /// Goal alignment evaluated.
    GoalAlignmentEvaluated { task_id: Uuid, overall_score: f64, passes: bool },
    /// DAG restructure triggered for a permanently failed task.
    RestructureTriggered { task_id: Uuid, decision: String },
    /// Spawn limit exceeded, specialist evaluation requested.
    SpawnLimitExceeded {
        parent_task_id: Uuid,
        limit_type: String,
        current_value: u32,
        limit_value: u32,
    },
    /// Goal iteration completed (all current tasks done, goal remains active).
    ///
    /// Goals are never "completed" - they are convergent attractors.
    /// This event indicates a successful iteration of work toward the goal.
    GoalIterationCompleted { goal_id: Uuid, tasks_completed: usize },
    /// Goal paused (human-initiated).
    GoalPaused { goal_id: Uuid, reason: String },
    /// Intent verification started.
    IntentVerificationStarted { goal_id: Uuid, iteration: u32 },
    /// Intent verification completed.
    IntentVerificationCompleted {
        goal_id: Uuid,
        satisfaction: String,
        confidence: f64,
        gaps_count: usize,
        iteration: u32,
        will_retry: bool,
    },
    /// Convergence loop completed.
    ConvergenceCompleted {
        goal_id: Uuid,
        converged: bool,
        iterations: u32,
        final_satisfaction: String,
    },
    /// Human escalation required.
    HumanEscalationRequired {
        goal_id: Option<Uuid>,
        task_id: Option<Uuid>,
        reason: String,
        urgency: String,
        questions: Vec<String>,
        is_blocking: bool,
    },
    /// Human response received to escalation.
    HumanResponseReceived {
        escalation_id: Uuid,
        decision: String,
        allows_continuation: bool,
    },
    /// Branch verification started.
    BranchVerificationStarted {
        branch_task_ids: Vec<Uuid>,
        waiting_task_ids: Vec<Uuid>,
    },
    /// Branch verification completed.
    BranchVerificationCompleted {
        branch_satisfied: bool,
        dependents_can_proceed: bool,
        gaps_count: usize,
    },
    /// Semantic drift detected in convergence loop.
    SemanticDriftDetected {
        goal_id: Uuid,
        recurring_gaps: Vec<String>,
        iterations: u32,
    },
    /// Orchestrator paused.
    Paused,
    /// Orchestrator resumed.
    Resumed,
    /// Orchestrator stopped.
    Stopped,
    /// Status update.
    StatusUpdate(SwarmStats),
}

/// Statistics about the swarm.
#[derive(Debug, Clone, Default)]
pub struct SwarmStats {
    pub active_goals: usize,
    pub pending_tasks: usize,
    pub ready_tasks: usize,
    pub running_tasks: usize,
    pub completed_tasks: usize,
    pub failed_tasks: usize,
    pub active_agents: usize,
    pub active_worktrees: usize,
    pub total_tokens_used: u64,
}
