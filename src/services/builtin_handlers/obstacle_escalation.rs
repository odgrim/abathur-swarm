//! Built-in reactive event handler.
//!
//! All handlers are **idempotent** — safe to run even if the poll loop already
//! handled the same state change. They check current state before acting.

#![allow(unused_imports)]

use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use async_trait::async_trait;
use tokio::sync::{RwLock, Semaphore};

use crate::domain::errors::DomainError;
use crate::domain::models::adapter::IngestionItemKind;
use crate::domain::models::convergence::{AmendmentSource, SpecificationAmendment};
use crate::domain::models::task_schedule::*;
use crate::domain::models::workflow_state::WorkflowState;
use crate::domain::models::{Goal, HumanEscalationEvent, Task, TaskSource, TaskStatus};
use crate::domain::ports::{
    GoalRepository, MemoryRepository, TaskRepository, TaskScheduleRepository, TrajectoryRepository,
    WorktreeRepository,
};
#[cfg(test)]
use crate::services::event_bus::ConvergenceTerminatedPayload;
use crate::services::event_bus::{
    EventBus, EventCategory, EventId, EventPayload, EventSeverity, HumanEscalationPayload,
    SequenceNumber, SwarmStatsPayload, TaskResultPayload, UnifiedEvent,
};
use crate::services::event_reactor::{
    ErrorStrategy, EventFilter, EventHandler, HandlerContext, HandlerId, HandlerMetadata,
    HandlerPriority, Reaction,
};
use crate::services::event_store::EventStore;
use crate::services::goal_context_service::GoalContextService;
use crate::services::memory_service::MemoryService;
use crate::services::swarm_orchestrator::SwarmStats;
use crate::services::task_service::TaskService;

use super::{try_update_task, update_with_retry};

// ============================================================================
// ObstacleEscalationHandler
// ============================================================================

/// Tracks repeated task failure patterns by agent_type + normalized error.
/// When the same class of obstacle causes failures beyond a threshold within
/// a sliding window, creates a new Goal to address the systematic issue.
///
/// This directly satisfies the "obstacle-escalation" constraint: "If the same
/// class of obstacle causes repeated failures without a template change or
/// memory update, it must escalate to a new goal rather than being silently
/// retried."
pub struct ObstacleEscalationHandler<T: TaskRepository, M: MemoryRepository, G: GoalRepository> {
    task_repo: Arc<T>,
    memory_repo: Arc<M>,
    goal_repo: Arc<G>,
    command_bus: Arc<crate::services::command_bus::CommandBus>,
    /// Number of failures before escalation (from config).
    threshold: u32,
    /// Sliding window duration in seconds (from config).
    window_secs: u64,
}

impl<T: TaskRepository, M: MemoryRepository, G: GoalRepository> ObstacleEscalationHandler<T, M, G> {
    pub fn new(
        task_repo: Arc<T>,
        memory_repo: Arc<M>,
        goal_repo: Arc<G>,
        command_bus: Arc<crate::services::command_bus::CommandBus>,
        threshold: u32,
        window_secs: u64,
    ) -> Self {
        Self {
            task_repo,
            memory_repo,
            goal_repo,
            command_bus,
            threshold,
            window_secs,
        }
    }

    /// Normalize an error string into a stable pattern key.
    /// Takes the first line, trims, lowercases, and truncates to 100 chars.
    fn normalize_error(error: &str) -> String {
        let first_line = error.lines().next().unwrap_or(error);
        let trimmed = first_line.trim();
        let lowered = trimmed.to_lowercase();
        lowered.chars().take(100).collect()
    }

    /// Build a deterministic pattern key from agent_type and normalized error.
    fn pattern_key(agent_type: &str, normalized_error: &str) -> String {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let mut hasher = DefaultHasher::new();
        normalized_error.hash(&mut hasher);
        let hash = hasher.finish();
        format!("failure-pattern:{}:{:016x}", agent_type, hash)
    }

    /// Record a failure occurrence and return (threshold_exceeded, count).
    async fn record_and_check_threshold(&self, pattern_key: &str) -> Result<(bool, u32), String> {
        let now = chrono::Utc::now();
        let window_start = now - chrono::Duration::seconds(self.window_secs as i64);

        // Load existing failure timestamps from memory
        let existing = self
            .memory_repo
            .get_by_key(pattern_key, "obstacle-escalation")
            .await
            .map_err(|e| format!("Failed to load failure pattern: {}", e))?;

        let mut timestamps: Vec<i64> = if let Some(ref mem) = existing {
            serde_json::from_str(&mem.content).unwrap_or_default()
        } else {
            Vec::new()
        };

        // Filter to sliding window
        timestamps.retain(|&ts| ts >= window_start.timestamp());

        // Add current failure
        timestamps.push(now.timestamp());

        let count = timestamps.len() as u32;

        // Store updated record (update if exists, store if new)
        let content = serde_json::to_string(&timestamps)
            .map_err(|e| format!("Failed to serialize timestamps: {}", e))?;

        if let Some(mut existing_mem) = existing {
            existing_mem.content = content;
            self.memory_repo
                .update(&existing_mem)
                .await
                .map_err(|e| format!("Failed to update failure pattern: {}", e))?;
        } else {
            let memory = crate::domain::models::Memory::episodic(pattern_key.to_string(), content)
                .with_namespace("obstacle-escalation")
                .with_type(crate::domain::models::MemoryType::Pattern)
                .with_source("obstacle_escalation_handler");

            self.memory_repo
                .store(&memory)
                .await
                .map_err(|e| format!("Failed to store failure pattern: {}", e))?;
        }

        Ok((count >= self.threshold, count))
    }

    /// Check if an escalation goal already exists for this pattern.
    async fn has_existing_escalation(&self, pattern_key: &str) -> Result<bool, String> {
        // Check memory for an existing escalation record
        let escalation_key = format!("escalated:{}", pattern_key);
        let existing = self
            .memory_repo
            .get_by_key(&escalation_key, "escalations")
            .await
            .map_err(|e| format!("Failed to check escalation: {}", e))?;

        if existing.is_some() {
            // Verify the goal still exists and is active
            let goals = self
                .goal_repo
                .find_by_domains(&["obstacle-escalation".to_string()])
                .await
                .map_err(|e| format!("Failed to query goals: {}", e))?;

            // If any active goal mentions this pattern key, skip
            for goal in &goals {
                if goal.status == crate::domain::models::GoalStatus::Active
                    && goal.description.contains(pattern_key)
                {
                    return Ok(true);
                }
            }
            // Goal was retired/deleted — allow re-escalation
        }

        Ok(false)
    }

    /// Create an escalation goal and record it in memory.
    async fn create_escalation_goal(
        &self,
        agent_type: &str,
        normalized_error: &str,
        pattern_key: &str,
        failure_count: u32,
    ) -> Result<(), String> {
        use crate::services::command_bus::{
            CommandEnvelope, CommandSource, DomainCommand, GoalCommand,
        };

        let goal_name = format!(
            "Address repeated {} failures: {}",
            agent_type,
            if normalized_error.len() > 60 {
                format!(
                    "{}...",
                    &normalized_error.chars().take(60).collect::<String>()
                )
            } else {
                normalized_error.to_string()
            }
        );
        let goal_description = format!(
            "The agent type '{}' has failed {} times within the escalation window \
             with the same error class:\n\n> {}\n\n\
             Pattern key: {}\n\n\
             This goal was auto-created by the obstacle escalation handler. \
             Investigate the root cause and update the agent template or \
             add memory to prevent recurrence.",
            agent_type, failure_count, normalized_error, pattern_key
        );

        let envelope = CommandEnvelope::new(
            CommandSource::EventHandler("ObstacleEscalationHandler".to_string()),
            DomainCommand::Goal(GoalCommand::Create {
                name: goal_name,
                description: goal_description,
                priority: crate::domain::models::GoalPriority::High,
                parent_id: None,
                constraints: vec![
                    crate::domain::models::GoalConstraint::preference(
                        "failure-pattern",
                        format!(
                            "Agent '{}' fails repeatedly with: {}",
                            agent_type, normalized_error
                        ),
                    ),
                    crate::domain::models::GoalConstraint::preference(
                        "resolution-action",
                        "Either update the agent template, add error-handling memory, or fix the underlying infrastructure issue",
                    ),
                ],
                domains: vec!["obstacle-escalation".to_string()],
            }),
        );

        match self.command_bus.dispatch(envelope).await {
            Ok(_) => {
                tracing::info!(
                    agent_type = agent_type,
                    pattern_key = pattern_key,
                    failure_count = failure_count,
                    "Created escalation goal for repeated failure pattern"
                );
            }
            Err(e) => {
                tracing::warn!(
                    agent_type = agent_type,
                    pattern_key = pattern_key,
                    "Failed to create escalation goal: {}",
                    e
                );
                return Err(format!("Failed to dispatch goal creation: {}", e));
            }
        }

        // Record escalation in memory for deduplication
        let escalation_key = format!("escalated:{}", pattern_key);
        let escalation_content = serde_json::json!({
            "agent_type": agent_type,
            "pattern_key": pattern_key,
            "failure_count": failure_count,
            "escalated_at": chrono::Utc::now().to_rfc3339(),
        })
        .to_string();

        let escalation_memory =
            crate::domain::models::Memory::episodic(escalation_key, escalation_content)
                .with_namespace("escalations")
                .with_type(crate::domain::models::MemoryType::Decision)
                .with_source("obstacle_escalation_handler");

        if let Err(e) = self.memory_repo.store(&escalation_memory).await {
            tracing::warn!("Failed to record escalation: {}", e);
        }

        Ok(())
    }
}

#[async_trait]
impl<T: TaskRepository + 'static, M: MemoryRepository + 'static, G: GoalRepository + 'static>
    EventHandler for ObstacleEscalationHandler<T, M, G>
{
    fn metadata(&self) -> HandlerMetadata {
        HandlerMetadata {
            id: HandlerId::new(),
            name: "ObstacleEscalationHandler".to_string(),
            filter: EventFilter::new()
                .categories(vec![EventCategory::Task])
                .payload_types(vec!["TaskFailed".to_string()]),
            priority: HandlerPriority::LOW,
            error_strategy: ErrorStrategy::LogAndContinue,
            critical: false,
        }
    }

    async fn handle(
        &self,
        event: &UnifiedEvent,
        _ctx: &HandlerContext,
    ) -> Result<Reaction, String> {
        // 1. Extract task_id and error from TaskFailed event
        let (task_id, error) = match &event.payload {
            EventPayload::TaskFailed { task_id, error, .. } => (*task_id, error.clone()),
            _ => return Ok(Reaction::None),
        };

        // 2. Load task to get agent_type
        let task = self
            .task_repo
            .get(task_id)
            .await
            .map_err(|e| format!("Failed to load task {}: {}", task_id, e))?;

        let task = match task {
            Some(t) => t,
            None => {
                tracing::debug!(
                    task_id = %task_id,
                    "Task not found, skipping obstacle tracking"
                );
                return Ok(Reaction::None);
            }
        };

        let agent_type = task
            .agent_type
            .clone()
            .unwrap_or_else(|| "unknown".to_string());

        // 3. Normalize error and build pattern key
        let normalized = Self::normalize_error(&error);
        let pattern_key = Self::pattern_key(&agent_type, &normalized);

        // 4. Record failure and check threshold
        let (threshold_exceeded, count) = self.record_and_check_threshold(&pattern_key).await?;

        if !threshold_exceeded {
            tracing::debug!(
                agent_type = %agent_type,
                pattern_key = %pattern_key,
                count = count,
                threshold = self.threshold,
                "Failure recorded, below escalation threshold"
            );
            return Ok(Reaction::None);
        }

        // 5. Check for existing escalation to avoid duplicates
        if self.has_existing_escalation(&pattern_key).await? {
            tracing::debug!(
                pattern_key = %pattern_key,
                "Escalation already exists for this pattern, skipping"
            );
            return Ok(Reaction::None);
        }

        // 6. Create escalation goal
        tracing::warn!(
            agent_type = %agent_type,
            pattern_key = %pattern_key,
            count = count,
            "Failure pattern exceeded threshold, escalating to goal"
        );

        self.create_escalation_goal(&agent_type, &normalized, &pattern_key, count)
            .await?;

        Ok(Reaction::None)
    }
}

#[cfg(test)]
mod tests {
    #![allow(unused_imports)]
    use super::super::*;
    use super::*;

    use super::*;
    use crate::adapters::sqlite::{
        SqliteMemoryRepository, create_migrated_test_pool, goal_repository::SqliteGoalRepository,
        task_repository::SqliteTaskRepository,
    };
    use crate::domain::models::{GoalStatus, Task, TaskStatus};

    use std::sync::Arc;

    async fn setup_obstacle_escalation_handler() -> (
        ObstacleEscalationHandler<
            SqliteTaskRepository,
            SqliteMemoryRepository,
            SqliteGoalRepository,
        >,
        Arc<SqliteTaskRepository>,
        Arc<SqliteMemoryRepository>,
        Arc<SqliteGoalRepository>,
    ) {
        use crate::services::goal_service::GoalService;
        use crate::services::memory_service::MemoryService;
        use crate::services::task_service::TaskService;

        let pool = create_migrated_test_pool().await.unwrap();
        let task_repo = Arc::new(SqliteTaskRepository::new(pool.clone()));
        let memory_repo = Arc::new(SqliteMemoryRepository::new(pool.clone()));
        let goal_repo = Arc::new(SqliteGoalRepository::new(pool.clone()));

        let task_service = Arc::new(TaskService::new(task_repo.clone()));
        let goal_service = Arc::new(GoalService::new(goal_repo.clone()));
        let memory_service = Arc::new(MemoryService::new(memory_repo.clone()));
        let event_bus = Arc::new(crate::services::EventBus::new(
            crate::services::EventBusConfig {
                persist_events: false,
                ..Default::default()
            },
        ));
        let command_bus = Arc::new(crate::services::command_bus::CommandBus::new(
            task_service,
            goal_service,
            memory_service,
            event_bus,
        ));

        let handler = ObstacleEscalationHandler::new(
            task_repo.clone(),
            memory_repo.clone(),
            goal_repo.clone(),
            command_bus,
            3,     // threshold
            86400, // window = 24h in seconds
        );
        (handler, task_repo, memory_repo, goal_repo)
    }

    /// Create a failed task with the given agent_type.
    async fn create_failed_task(task_repo: &SqliteTaskRepository, agent_type: &str) -> Task {
        let mut task = Task::new("Failing task");
        task.description = "Task that fails".to_string();
        task.agent_type = Some(agent_type.to_string());
        task.transition_to(TaskStatus::Ready).unwrap();
        task.transition_to(TaskStatus::Running).unwrap();
        task.transition_to(TaskStatus::Failed).unwrap();
        task_repo.create(&task).await.unwrap();
        task
    }

    /// Build a TaskFailed event for the given task and error.
    fn make_task_failed_event(task_id: uuid::Uuid, error: &str) -> UnifiedEvent {
        UnifiedEvent {
            id: EventId::new(),
            sequence: SequenceNumber(0),
            timestamp: chrono::Utc::now(),
            severity: EventSeverity::Error,
            category: EventCategory::Task,
            goal_id: None,
            task_id: Some(task_id),
            correlation_id: None,
            source_process_id: None,
            payload: EventPayload::TaskFailed {
                task_id,
                error: error.to_string(),
                retry_count: 0,
            },
        }
    }

    #[test]
    fn test_normalize_error_basic() {
        let result = ObstacleEscalationHandler::<
            SqliteTaskRepository,
            SqliteMemoryRepository,
            SqliteGoalRepository,
        >::normalize_error(
            "Compilation error: cannot find type `Foo`\ndetailed backtrace follows...",
        );
        assert_eq!(result, "compilation error: cannot find type `foo`");
    }

    #[test]
    fn test_normalize_error_truncation() {
        let long_error = "a".repeat(200);
        let result = ObstacleEscalationHandler::<
            SqliteTaskRepository,
            SqliteMemoryRepository,
            SqliteGoalRepository,
        >::normalize_error(&long_error);
        assert_eq!(result.len(), 100);
    }

    #[test]
    fn test_normalize_error_multiline() {
        let result = ObstacleEscalationHandler::<
            SqliteTaskRepository,
            SqliteMemoryRepository,
            SqliteGoalRepository,
        >::normalize_error("First line error\nSecond line\nThird line");
        assert_eq!(result, "first line error");
    }

    #[test]
    fn test_pattern_key_deterministic() {
        let key1 = ObstacleEscalationHandler::<
            SqliteTaskRepository,
            SqliteMemoryRepository,
            SqliteGoalRepository,
        >::pattern_key("coder", "some error");
        let key2 = ObstacleEscalationHandler::<
            SqliteTaskRepository,
            SqliteMemoryRepository,
            SqliteGoalRepository,
        >::pattern_key("coder", "some error");
        assert_eq!(key1, key2);
        assert!(key1.starts_with("failure-pattern:coder:"));
    }

    #[test]
    fn test_pattern_key_different_for_different_errors() {
        let key1 = ObstacleEscalationHandler::<
            SqliteTaskRepository,
            SqliteMemoryRepository,
            SqliteGoalRepository,
        >::pattern_key("coder", "error a");
        let key2 = ObstacleEscalationHandler::<
            SqliteTaskRepository,
            SqliteMemoryRepository,
            SqliteGoalRepository,
        >::pattern_key("coder", "error b");
        assert_ne!(key1, key2);
    }

    #[tokio::test]
    async fn test_obstacle_escalation_below_threshold() {
        let (handler, task_repo, _memory_repo, goal_repo) =
            setup_obstacle_escalation_handler().await;
        let ctx = HandlerContext {
            chain_depth: 0,
            correlation_id: None,
        };

        // Send 2 failures (threshold is 3) — should NOT escalate
        for _ in 0..2 {
            let task = create_failed_task(&task_repo, "coder").await;
            let event = make_task_failed_event(task.id, "compilation error: missing type");
            let reaction = handler.handle(&event, &ctx).await.unwrap();
            assert!(matches!(reaction, Reaction::None));
        }

        // Verify no goals created
        use crate::domain::ports::goal_repository::GoalFilter;
        let goals = goal_repo
            .list(GoalFilter {
                status: Some(GoalStatus::Active),
                ..Default::default()
            })
            .await
            .unwrap();
        assert!(
            goals.is_empty(),
            "No goals should be created below threshold"
        );
    }

    #[tokio::test]
    async fn test_obstacle_escalation_triggers_at_threshold() {
        let (handler, task_repo, _memory_repo, goal_repo) =
            setup_obstacle_escalation_handler().await;
        let ctx = HandlerContext {
            chain_depth: 0,
            correlation_id: None,
        };

        // Send 3 failures with same agent_type + same error
        for _ in 0..3 {
            let task = create_failed_task(&task_repo, "researcher").await;
            let event = make_task_failed_event(task.id, "timeout waiting for response");
            handler.handle(&event, &ctx).await.unwrap();
        }

        // Verify a goal was created
        use crate::domain::ports::goal_repository::GoalFilter;
        let goals = goal_repo
            .list(GoalFilter {
                status: Some(GoalStatus::Active),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(
            goals.len(),
            1,
            "Exactly one escalation goal should be created"
        );
        assert!(
            goals[0].name.contains("researcher"),
            "Goal name should mention agent_type"
        );
        assert!(
            goals[0].description.contains("timeout"),
            "Goal description should mention error"
        );
    }

    #[tokio::test]
    async fn test_obstacle_escalation_deduplication() {
        let (handler, task_repo, _memory_repo, goal_repo) =
            setup_obstacle_escalation_handler().await;
        let ctx = HandlerContext {
            chain_depth: 0,
            correlation_id: None,
        };

        // First, trigger an escalation with 3 failures
        for _ in 0..3 {
            let task = create_failed_task(&task_repo, "implementer").await;
            let event = make_task_failed_event(task.id, "borrow checker error");
            handler.handle(&event, &ctx).await.unwrap();
        }

        // Send 3 more with the same pattern — should NOT create a second goal
        for _ in 0..3 {
            let task = create_failed_task(&task_repo, "implementer").await;
            let event = make_task_failed_event(task.id, "borrow checker error");
            handler.handle(&event, &ctx).await.unwrap();
        }

        use crate::domain::ports::goal_repository::GoalFilter;
        let goals = goal_repo
            .list(GoalFilter {
                status: Some(GoalStatus::Active),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(
            goals.len(),
            1,
            "Should not create duplicate escalation goals"
        );
    }

    #[tokio::test]
    async fn test_obstacle_escalation_different_errors_separate() {
        let (handler, task_repo, _memory_repo, goal_repo) =
            setup_obstacle_escalation_handler().await;
        let ctx = HandlerContext {
            chain_depth: 0,
            correlation_id: None,
        };

        // Send 1 failure each of 3 different error types from same agent
        let errors = ["error type A", "error type B", "error type C"];
        for error in &errors {
            let task = create_failed_task(&task_repo, "planner").await;
            let event = make_task_failed_event(task.id, error);
            handler.handle(&event, &ctx).await.unwrap();
        }

        // No pattern should have reached threshold (each has count=1)
        use crate::domain::ports::goal_repository::GoalFilter;
        let goals = goal_repo
            .list(GoalFilter {
                status: Some(GoalStatus::Active),
                ..Default::default()
            })
            .await
            .unwrap();
        assert!(
            goals.is_empty(),
            "Different errors should track separately, none reaching threshold"
        );
    }

    #[tokio::test]
    async fn test_obstacle_escalation_task_not_found() {
        let (handler, _task_repo, _memory_repo, _goal_repo) =
            setup_obstacle_escalation_handler().await;
        let ctx = HandlerContext {
            chain_depth: 0,
            correlation_id: None,
        };

        let nonexistent_id = uuid::Uuid::new_v4();
        let event = make_task_failed_event(nonexistent_id, "some error");
        let reaction = handler.handle(&event, &ctx).await.unwrap();
        assert!(
            matches!(reaction, Reaction::None),
            "Should return None for nonexistent task"
        );
    }

    #[tokio::test]
    async fn test_obstacle_escalation_ignores_non_task_failed() {
        let (handler, _task_repo, _memory_repo, _goal_repo) =
            setup_obstacle_escalation_handler().await;
        let ctx = HandlerContext {
            chain_depth: 0,
            correlation_id: None,
        };

        // Send a TaskCompleted event (not TaskFailed)
        let event = UnifiedEvent {
            id: EventId::new(),
            sequence: SequenceNumber(0),
            timestamp: chrono::Utc::now(),
            severity: EventSeverity::Info,
            category: EventCategory::Task,
            goal_id: None,
            task_id: Some(uuid::Uuid::new_v4()),
            correlation_id: None,
            source_process_id: None,
            payload: EventPayload::TaskCompleted {
                task_id: uuid::Uuid::new_v4(),
                tokens_used: 0,
            },
        };
        let reaction = handler.handle(&event, &ctx).await.unwrap();
        assert!(
            matches!(reaction, Reaction::None),
            "Should ignore non-TaskFailed events"
        );
    }
}
