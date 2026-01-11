//! DAG restructuring service.
//!
//! Handles intelligent re-planning when tasks permanently fail by invoking
//! the Meta-Planner with failure context to find alternative approaches.

use std::collections::HashMap;
use std::time::{Duration, Instant};
use uuid::Uuid;

use crate::domain::errors::DomainResult;
use crate::domain::models::{Goal, Task, TaskStatus};

/// Configuration for DAG restructuring.
#[derive(Debug, Clone)]
pub struct RestructureConfig {
    /// Maximum times a task subtree can be restructured.
    pub max_restructure_attempts: u32,
    /// Minimum time between restructure attempts for the same parent.
    pub restructure_cooldown: Duration,
    /// Whether to use LLM for restructure decisions.
    pub use_llm_restructure: bool,
    /// Maximum depth to propagate failure before restructuring.
    pub max_propagation_depth: usize,
}

impl Default for RestructureConfig {
    fn default() -> Self {
        Self {
            max_restructure_attempts: 3,
            restructure_cooldown: Duration::from_secs(300),
            use_llm_restructure: false,
            max_propagation_depth: 2,
        }
    }
}

/// Trigger condition for restructuring.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RestructureTrigger {
    /// Task exhausted all retries.
    PermanentFailure { task_id: Uuid, retries_exhausted: u32 },
    /// Spawn limit exceeded and extension denied.
    SpawnLimitExceeded { task_id: Uuid },
    /// Circular dependency detected.
    CircularDependency { task_ids: Vec<Uuid> },
    /// Agent explicitly reported cannot proceed.
    AgentBlocked { task_id: Uuid, reason: String },
    /// Multiple tasks in a subtree have failed.
    SubtreeFailures { parent_id: Uuid, failed_count: u32 },
}

impl RestructureTrigger {
    /// Get the primary task ID for this trigger.
    pub fn primary_task_id(&self) -> Option<Uuid> {
        match self {
            RestructureTrigger::PermanentFailure { task_id, .. } => Some(*task_id),
            RestructureTrigger::SpawnLimitExceeded { task_id } => Some(*task_id),
            RestructureTrigger::CircularDependency { task_ids } => task_ids.first().copied(),
            RestructureTrigger::AgentBlocked { task_id, .. } => Some(*task_id),
            RestructureTrigger::SubtreeFailures { parent_id, .. } => Some(*parent_id),
        }
    }
}

/// Context for restructuring decision.
#[derive(Debug, Clone)]
pub struct RestructureContext {
    /// The goal being worked on.
    pub goal: Goal,
    /// The failed task.
    pub failed_task: Task,
    /// Failure reason.
    pub failure_reason: String,
    /// Previous attempts at this task.
    pub previous_attempts: Vec<FailedAttempt>,
    /// Related failed tasks in the same subtree.
    pub related_failures: Vec<Task>,
    /// Available alternative approaches.
    pub available_approaches: Vec<String>,
    /// Restructure attempt number.
    pub attempt_number: u32,
    /// Time since last restructure.
    pub time_since_last: Option<Duration>,
}

/// Record of a failed attempt.
#[derive(Debug, Clone)]
pub struct FailedAttempt {
    /// When the attempt was made.
    pub timestamp: chrono::DateTime<chrono::Utc>,
    /// Agent that attempted the task.
    pub agent_type: String,
    /// Error or failure message.
    pub error: String,
    /// Number of turns used.
    pub turns_used: u32,
}

/// Decision from restructuring analysis.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RestructureDecision {
    /// Retry the task with a different approach.
    RetryDifferentApproach {
        new_approach: String,
        new_agent_type: Option<String>,
    },
    /// Find an alternative path to achieve the same result.
    AlternativePath {
        description: String,
        new_tasks: Vec<NewTaskSpec>,
    },
    /// Decompose the task differently.
    DecomposeDifferently {
        new_subtasks: Vec<NewTaskSpec>,
        remove_original: bool,
    },
    /// Escalate to human attention.
    Escalate {
        reason: String,
        context: String,
    },
    /// Wait and retry later.
    WaitAndRetry {
        delay: Duration,
        reason: String,
    },
    /// Mark as permanently failed, no recovery possible.
    AcceptFailure {
        reason: String,
    },
}

/// Specification for a new task created during restructuring.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewTaskSpec {
    /// Task title.
    pub title: String,
    /// Task description.
    pub description: String,
    /// Suggested agent type.
    pub agent_type: Option<String>,
    /// Dependencies (titles or IDs).
    pub depends_on: Vec<String>,
    /// Priority relative to original.
    pub priority: TaskPriorityModifier,
}

/// Priority modifier for restructured tasks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskPriorityModifier {
    /// Same priority as the original.
    Same,
    /// Increase priority.
    Higher,
    /// Decrease priority.
    Lower,
}

/// Restructure state for tracking attempts.
#[derive(Debug, Clone)]
struct RestructureState {
    /// Number of restructure attempts.
    attempts: u32,
    /// Last restructure time.
    last_attempt: Option<Instant>,
    /// Decisions made.
    decisions: Vec<RestructureDecision>,
}

/// DAG restructuring service.
pub struct DagRestructureService {
    config: RestructureConfig,
    /// Track restructure state per task subtree.
    state: HashMap<Uuid, RestructureState>,
}

impl DagRestructureService {
    pub fn new(config: RestructureConfig) -> Self {
        Self {
            config,
            state: HashMap::new(),
        }
    }

    pub fn with_defaults() -> Self {
        Self::new(RestructureConfig::default())
    }

    /// Check if restructuring should be triggered.
    pub fn should_restructure(&self, trigger: &RestructureTrigger) -> bool {
        let task_id = match trigger.primary_task_id() {
            Some(id) => id,
            None => return false,
        };

        // Check if we've exceeded max attempts
        if let Some(state) = self.state.get(&task_id) {
            if state.attempts >= self.config.max_restructure_attempts {
                return false;
            }

            // Check cooldown
            if let Some(last) = state.last_attempt {
                if last.elapsed() < self.config.restructure_cooldown {
                    return false;
                }
            }
        }

        true
    }

    /// Analyze the failure and decide on restructuring action.
    pub fn analyze_and_decide(
        &mut self,
        context: &RestructureContext,
    ) -> DomainResult<RestructureDecision> {
        let task_id = context.failed_task.id;

        // Get current attempt count
        let current_attempts = self.state
            .get(&task_id)
            .map(|s| s.attempts)
            .unwrap_or(0);

        let new_attempts = current_attempts + 1;

        // Check if we've hit the limit
        if new_attempts > self.config.max_restructure_attempts {
            return Ok(RestructureDecision::AcceptFailure {
                reason: format!(
                    "Maximum restructure attempts ({}) exceeded",
                    self.config.max_restructure_attempts
                ),
            });
        }

        // Use heuristic decision making (LLM integration would go here)
        let decision = self.heuristic_decision(context, new_attempts);

        // Record this attempt
        let state = self.state.entry(task_id).or_insert_with(|| RestructureState {
            attempts: 0,
            last_attempt: None,
            decisions: Vec::new(),
        });

        state.attempts = new_attempts;
        state.last_attempt = Some(Instant::now());
        state.decisions.push(decision.clone());

        Ok(decision)
    }

    /// Make a heuristic-based restructure decision.
    fn heuristic_decision(&self, context: &RestructureContext, attempt: u32) -> RestructureDecision {
        // First attempt: Try a different approach
        if attempt == 1 {
            // Check if there are alternative approaches available
            if !context.available_approaches.is_empty() {
                return RestructureDecision::RetryDifferentApproach {
                    new_approach: context.available_approaches[0].clone(),
                    new_agent_type: None,
                };
            }

            // Try decomposing differently
            return RestructureDecision::DecomposeDifferently {
                new_subtasks: vec![
                    NewTaskSpec {
                        title: format!("Research: {}", context.failed_task.title),
                        description: format!(
                            "Investigate why '{}' failed and identify requirements",
                            context.failed_task.title
                        ),
                        agent_type: Some("researcher".to_string()),
                        depends_on: vec![],
                        priority: TaskPriorityModifier::Higher,
                    },
                    NewTaskSpec {
                        title: format!("Implement: {}", context.failed_task.title),
                        description: context.failed_task.description.clone(),
                        agent_type: context.failed_task.agent_type.clone(),
                        depends_on: vec![format!("Research: {}", context.failed_task.title)],
                        priority: TaskPriorityModifier::Same,
                    },
                ],
                remove_original: true,
            };
        }

        // Second attempt: Try alternative path
        if attempt == 2 {
            // Check for related failures - might be a systemic issue
            if context.related_failures.len() > 2 {
                return RestructureDecision::Escalate {
                    reason: "Multiple related tasks have failed".to_string(),
                    context: format!(
                        "{} tasks failed in this subtree: {}",
                        context.related_failures.len(),
                        context.related_failures
                            .iter()
                            .map(|t| t.title.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                };
            }

            return RestructureDecision::AlternativePath {
                description: format!("Alternative approach for: {}", context.failed_task.title),
                new_tasks: vec![NewTaskSpec {
                    title: format!("Alternative: {}", context.failed_task.title),
                    description: format!(
                        "Find an alternative way to achieve: {}. Previous approach failed: {}",
                        context.failed_task.description,
                        context.failure_reason
                    ),
                    agent_type: Some("problem-solver".to_string()),
                    depends_on: vec![],
                    priority: TaskPriorityModifier::Higher,
                }],
            };
        }

        // Third attempt and beyond: Escalate
        RestructureDecision::Escalate {
            reason: format!(
                "Task '{}' has failed {} times despite restructuring",
                context.failed_task.title, context.previous_attempts.len()
            ),
            context: format!(
                "Failure reason: {}\nGoal: {}",
                context.failure_reason, context.goal.name
            ),
        }
    }

    /// Check if a task is eligible for restructuring.
    pub fn is_eligible(&self, task: &Task) -> bool {
        // Task must be in a failed state
        matches!(task.status, TaskStatus::Failed)
    }

    /// Get restructure history for a task.
    pub fn get_history(&self, task_id: Uuid) -> Option<&Vec<RestructureDecision>> {
        self.state.get(&task_id).map(|s| &s.decisions)
    }

    /// Get current attempt count.
    pub fn attempt_count(&self, task_id: Uuid) -> u32 {
        self.state.get(&task_id).map(|s| s.attempts).unwrap_or(0)
    }

    /// Check if task has exceeded restructure limits.
    pub fn has_exceeded_limits(&self, task_id: Uuid) -> bool {
        self.state
            .get(&task_id)
            .map(|s| s.attempts >= self.config.max_restructure_attempts)
            .unwrap_or(false)
    }

    /// Clear state for a task (e.g., after successful completion).
    pub fn clear_state(&mut self, task_id: Uuid) {
        self.state.remove(&task_id);
    }

    /// Get configuration.
    pub fn config(&self) -> &RestructureConfig {
        &self.config
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::models::{GoalPriority, TaskPriority};

    fn create_test_task() -> Task {
        let mut task = Task::new("Test Task", "Test description")
            .with_priority(TaskPriority::Normal);
        task.status = TaskStatus::Failed;
        task
    }

    fn create_test_goal() -> Goal {
        Goal::new("Test Goal", "Test goal description")
            .with_priority(GoalPriority::Normal)
    }

    #[test]
    fn test_config_default() {
        let config = RestructureConfig::default();
        assert_eq!(config.max_restructure_attempts, 3);
        assert_eq!(config.restructure_cooldown, Duration::from_secs(300));
    }

    #[test]
    fn test_trigger_primary_task_id() {
        let task_id = Uuid::new_v4();

        let trigger = RestructureTrigger::PermanentFailure {
            task_id,
            retries_exhausted: 3,
        };
        assert_eq!(trigger.primary_task_id(), Some(task_id));

        let trigger = RestructureTrigger::CircularDependency {
            task_ids: vec![task_id],
        };
        assert_eq!(trigger.primary_task_id(), Some(task_id));

        let trigger = RestructureTrigger::CircularDependency {
            task_ids: vec![],
        };
        assert_eq!(trigger.primary_task_id(), None);
    }

    #[test]
    fn test_should_restructure_first_attempt() {
        let service = DagRestructureService::with_defaults();
        let trigger = RestructureTrigger::PermanentFailure {
            task_id: Uuid::new_v4(),
            retries_exhausted: 3,
        };

        assert!(service.should_restructure(&trigger));
    }

    #[test]
    fn test_should_restructure_exceeded_max() {
        let mut service = DagRestructureService::with_defaults();
        let task_id = Uuid::new_v4();

        // Simulate max attempts
        for _ in 0..3 {
            service.state.insert(
                task_id,
                RestructureState {
                    attempts: 3,
                    last_attempt: Some(Instant::now()),
                    decisions: vec![],
                },
            );
        }

        let trigger = RestructureTrigger::PermanentFailure {
            task_id,
            retries_exhausted: 3,
        };

        assert!(!service.should_restructure(&trigger));
    }

    #[test]
    fn test_analyze_first_attempt_decompose() {
        let mut service = DagRestructureService::with_defaults();

        let context = RestructureContext {
            goal: create_test_goal(),
            failed_task: create_test_task(),
            failure_reason: "Test failure".to_string(),
            previous_attempts: vec![],
            related_failures: vec![],
            available_approaches: vec![],
            attempt_number: 1,
            time_since_last: None,
        };

        let decision = service.analyze_and_decide(&context).unwrap();

        match decision {
            RestructureDecision::DecomposeDifferently { new_subtasks, remove_original } => {
                assert_eq!(new_subtasks.len(), 2);
                assert!(remove_original);
            }
            _ => panic!("Expected DecomposeDifferently decision"),
        }
    }

    #[test]
    fn test_analyze_with_available_approaches() {
        let mut service = DagRestructureService::with_defaults();

        let context = RestructureContext {
            goal: create_test_goal(),
            failed_task: create_test_task(),
            failure_reason: "Test failure".to_string(),
            previous_attempts: vec![],
            related_failures: vec![],
            available_approaches: vec!["Use different library".to_string()],
            attempt_number: 1,
            time_since_last: None,
        };

        let decision = service.analyze_and_decide(&context).unwrap();

        match decision {
            RestructureDecision::RetryDifferentApproach { new_approach, .. } => {
                assert_eq!(new_approach, "Use different library");
            }
            _ => panic!("Expected RetryDifferentApproach decision"),
        }
    }

    #[test]
    fn test_is_eligible() {
        let service = DagRestructureService::with_defaults();

        let mut failed_task = create_test_task();
        failed_task.status = TaskStatus::Failed;
        assert!(service.is_eligible(&failed_task));

        let mut pending_task = create_test_task();
        pending_task.status = TaskStatus::Pending;
        assert!(!service.is_eligible(&pending_task));
    }

    #[test]
    fn test_attempt_count() {
        let mut service = DagRestructureService::with_defaults();
        let task_id = Uuid::new_v4();

        assert_eq!(service.attempt_count(task_id), 0);

        let context = RestructureContext {
            goal: create_test_goal(),
            failed_task: {
                let mut t = create_test_task();
                t.id = task_id;
                t
            },
            failure_reason: "Test".to_string(),
            previous_attempts: vec![],
            related_failures: vec![],
            available_approaches: vec![],
            attempt_number: 1,
            time_since_last: None,
        };

        let _ = service.analyze_and_decide(&context);
        assert_eq!(service.attempt_count(task_id), 1);
    }

    #[test]
    fn test_clear_state() {
        let mut service = DagRestructureService::with_defaults();
        let task_id = Uuid::new_v4();

        service.state.insert(
            task_id,
            RestructureState {
                attempts: 2,
                last_attempt: Some(Instant::now()),
                decisions: vec![],
            },
        );

        assert_eq!(service.attempt_count(task_id), 2);

        service.clear_state(task_id);
        assert_eq!(service.attempt_count(task_id), 0);
    }
}
