///! Task Hook System Domain Models
///!
///! Provides lifecycle hooks for tasks and branches with configurable
///! conditions and actions.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

/// Hook lifecycle events that can trigger actions
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HookEvent {
    /// Before task transitions to Ready status
    PreReady,
    /// After task transitions to Ready status
    PostReady,
    /// Before task starts execution
    PreStart,
    /// After task starts execution
    PostStart,
    /// Before task completes
    PreComplete,
    /// After task completes
    PostComplete,
    /// When a child task is spawned
    OnChildSpawned,
    /// Before validation runs
    OnValidation,
    /// When all tasks in a branch reach terminal state
    OnBranchComplete { branch_type: BranchType },
}

/// Type of branch for completion events
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BranchType {
    /// Individual task branch (task/xxx-yyy-zzz)
    TaskBranch,
    /// Feature branch (feature/xxx)
    FeatureBranch,
}

/// Conditions that must be met for a hook to execute
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct HookCondition {
    /// Match specific agent type
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_type: Option<String>,

    /// Match parent agent type
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_agent_type: Option<String>,

    /// Require minimum number of children spawned
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_children_spawned: Option<usize>,

    /// Match specific child agent types
    #[serde(skip_serializing_if = "Option::is_none")]
    pub child_agent_types: Option<Vec<String>>,

    /// Match branch name pattern (regex)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch_pattern: Option<String>,

    /// Match specific feature branch
    #[serde(skip_serializing_if = "Option::is_none")]
    pub feature_branch: Option<String>,

    /// Only trigger if all tasks succeeded
    #[serde(skip_serializing_if = "Option::is_none")]
    pub all_tasks_succeeded: Option<bool>,

    /// Require minimum number of completed tasks
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_tasks_completed: Option<usize>,

    /// Require specific agent types in completed tasks
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed_agent_types_include: Option<Vec<String>>,
}

impl Default for HookCondition {
    fn default() -> Self {
        Self {
            agent_type: None,
            parent_agent_type: None,
            min_children_spawned: None,
            child_agent_types: None,
            branch_pattern: None,
            feature_branch: None,
            all_tasks_succeeded: None,
            min_tasks_completed: None,
            completed_agent_types_include: None,
        }
    }
}

/// Actions to execute when hook conditions are met
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum HookAction {
    /// Run a shell script
    RunScript {
        script_path: String,
        #[serde(default)]
        args: Vec<String>,
    },

    /// Spawn a new task
    SpawnTask {
        agent_type: String,
        summary: String,
        description: String,
        #[serde(default = "default_priority")]
        priority: u8,
    },

    /// Update a task field
    UpdateField {
        field: String,
        value: serde_json::Value,
    },

    /// Block state transition with reason
    BlockTransition { reason: String },

    /// Log a message
    LogMessage { level: String, message: String },

    /// Merge branches
    MergeBranch {
        source: String,
        target: String,
        #[serde(default)]
        strategy: MergeStrategy,
    },

    /// Delete a branch
    DeleteBranch {
        branch: String,
        #[serde(default)]
        cleanup_worktree: bool,
    },

    /// Create a git tag
    CreateTag { name: String, message: String },

    /// Send webhook notification
    NotifyWebhook {
        url: String,
        payload: serde_json::Value,
    },
}

fn default_priority() -> u8 {
    5
}

/// Git merge strategy
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MergeStrategy {
    /// Automatic merge if no conflicts
    #[default]
    Auto,
    /// Require review task first
    RequireReview,
    /// Squash commits
    SquashMerge,
}

/// Complete hook definition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskHook {
    /// Unique identifier for this hook
    pub id: String,

    /// Optional description
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    /// Event that triggers this hook
    pub event: HookEvent,

    /// Conditions that must be met
    #[serde(default)]
    pub conditions: Vec<HookCondition>,

    /// Actions to execute
    pub actions: Vec<HookAction>,

    /// Execution priority (higher = earlier)
    #[serde(default = "default_priority")]
    pub priority: u8,

    /// Whether this hook is enabled
    #[serde(default = "default_enabled")]
    pub enabled: bool,
}

fn default_enabled() -> bool {
    true
}

/// Context provided to hooks during execution
#[derive(Debug, Clone)]
pub struct HookContext {
    /// Variables available for template substitution
    pub variables: HashMap<String, String>,

    /// Task being processed (for task-level hooks)
    pub task_id: Option<Uuid>,

    /// Branch completion context (for branch-level hooks)
    pub branch_context: Option<BranchCompletionContext>,
}

impl HookContext {
    /// Create context for task-level hook
    pub fn from_task(task_id: Uuid, variables: HashMap<String, String>) -> Self {
        Self {
            variables,
            task_id: Some(task_id),
            branch_context: None,
        }
    }

    /// Create context for branch-level hook
    pub fn from_branch(
        branch_context: BranchCompletionContext,
        variables: HashMap<String, String>,
    ) -> Self {
        Self {
            variables,
            task_id: None,
            branch_context: Some(branch_context),
        }
    }
}

/// Branch completion statistics and context
#[derive(Debug, Clone)]
pub struct BranchCompletionContext {
    /// Name of the completed branch
    pub branch_name: String,

    /// Type of branch (task or feature)
    pub branch_type: BranchType,

    /// All tasks that were in this branch
    pub completed_task_ids: Vec<Uuid>,

    /// Feature branch (for task branches)
    pub feature_branch: Option<String>,

    /// Whether all tasks succeeded
    pub all_succeeded: bool,

    /// Number of failed tasks
    pub failed_task_count: usize,

    /// IDs of failed tasks
    pub failed_task_ids: Vec<Uuid>,

    /// Total number of tasks
    pub total_tasks: usize,

    /// Agent types that completed
    pub completed_agent_types: Vec<String>,
}

/// Result of hook execution
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HookResult {
    /// Continue with normal execution
    Continue,

    /// Block the operation
    Blocked { reason: String },
}

impl HookResult {
    /// Check if execution should be blocked
    pub fn should_block(&self) -> bool {
        matches!(self, HookResult::Blocked { .. })
    }
}

/// Configuration for all hooks
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HooksConfig {
    /// List of configured hooks
    #[serde(default)]
    pub hooks: Vec<TaskHook>,
}

impl Default for HooksConfig {
    fn default() -> Self {
        Self { hooks: Vec::new() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hook_event_serialization() {
        let event = HookEvent::PreReady;
        let json = serde_json::to_string(&event).unwrap();
        assert_eq!(json, r#""pre_ready"#);

        let event = HookEvent::OnBranchComplete {
            branch_type: BranchType::TaskBranch,
        };
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("on_branch_complete"));
    }

    #[test]
    fn test_hook_action_serialization() {
        let action = HookAction::RunScript {
            script_path: "./test.sh".to_string(),
            args: vec!["arg1".to_string()],
        };
        let json = serde_json::to_string(&action).unwrap();
        assert!(json.contains("run_script"));
        assert!(json.contains("test.sh"));
    }

    #[test]
    fn test_hook_condition_defaults() {
        let condition = HookCondition::default();
        assert!(condition.agent_type.is_none());
        assert!(condition.min_children_spawned.is_none());
    }

    #[test]
    fn test_hook_result_should_block() {
        let continue_result = HookResult::Continue;
        assert!(!continue_result.should_block());

        let blocked_result = HookResult::Blocked {
            reason: "test".to_string(),
        };
        assert!(blocked_result.should_block());
    }

    #[test]
    fn test_task_hook_deserialization() {
        let yaml = r#"
id: test-hook
description: Test hook
event: pre_ready
conditions:
  - agent_type: test-agent
actions:
  - type: log_message
    level: info
    message: Test message
priority: 10
enabled: true
"#;

        let hook: TaskHook = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(hook.id, "test-hook");
        assert_eq!(hook.priority, 10);
        assert!(hook.enabled);
        assert_eq!(hook.conditions.len(), 1);
        assert_eq!(hook.actions.len(), 1);
    }

    #[test]
    fn test_hooks_config_deserialization() {
        let yaml = r#"
hooks:
  - id: hook1
    event: pre_ready
    actions:
      - type: log_message
        level: info
        message: Test
  - id: hook2
    event: post_complete
    actions:
      - type: spawn_task
        agent_type: test-agent
        summary: Test task
        description: Test description
        priority: 8
"#;

        let config: HooksConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.hooks.len(), 2);
        assert_eq!(config.hooks[0].id, "hook1");
        assert_eq!(config.hooks[1].id, "hook2");
    }
}
