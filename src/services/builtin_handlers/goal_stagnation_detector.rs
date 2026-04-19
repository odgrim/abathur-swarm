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
// GoalStagnationDetectorHandler
// ============================================================================

/// Detects goals that have not been evaluated in a convergence check recently.
///
/// Fires a `HumanEscalationRequired` event for any active goal whose
/// `last_convergence_check_at` exceeds the stall threshold, unless the goal
/// was created recently (grace period) or an alert was already emitted within
/// the threshold window (in-memory dedup).
///
/// Triggered by `ScheduledEventFired { name: "system-stall-check" }` — no
/// new schedule needed.
pub struct GoalStagnationDetectorHandler<G: GoalRepository> {
    goal_repo: Arc<G>,
    /// Maximum time (seconds) a goal may go without a convergence check before alerting.
    stall_threshold_secs: u64,
    /// In-memory dedup: maps goal_id → last alert timestamp.
    last_alerted: RwLock<std::collections::HashMap<uuid::Uuid, chrono::DateTime<chrono::Utc>>>,
}

impl<G: GoalRepository> GoalStagnationDetectorHandler<G> {
    pub fn new(goal_repo: Arc<G>, stall_threshold_secs: u64) -> Self {
        Self {
            goal_repo,
            stall_threshold_secs,
            last_alerted: RwLock::new(std::collections::HashMap::new()),
        }
    }
}

#[async_trait]
impl<G: GoalRepository + 'static> EventHandler for GoalStagnationDetectorHandler<G> {
    fn metadata(&self) -> HandlerMetadata {
        HandlerMetadata {
            id: HandlerId::new(),
            name: "GoalStagnationDetectorHandler".to_string(),
            filter: EventFilter::new()
                .categories(vec![EventCategory::Scheduler])
                .payload_types(vec!["ScheduledEventFired".to_string()]),
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
        let name = match &event.payload {
            EventPayload::ScheduledEventFired { name, .. } => name.as_str(),
            _ => return Ok(Reaction::None),
        };

        if name != "system-stall-check" {
            return Ok(Reaction::None);
        }

        let goals = self
            .goal_repo
            .get_active_with_constraints()
            .await
            .map_err(|e| format!("GoalStagnationDetector: failed to get active goals: {}", e))?;

        if goals.is_empty() {
            return Ok(Reaction::None);
        }

        let now = chrono::Utc::now();
        let threshold_secs = self.stall_threshold_secs as i64;
        let mut last_alerted = self.last_alerted.write().await;
        let mut events = Vec::new();

        for goal in &goals {
            // Grace period: new goals without a check yet AND created recently should not alert
            if goal.last_convergence_check_at.is_none() {
                let age_secs = (now - goal.created_at).num_seconds();
                if age_secs < threshold_secs {
                    tracing::debug!(
                        goal_id = %goal.id,
                        age_secs,
                        threshold = threshold_secs,
                        "GoalStagnationDetector: goal within grace period, skipping"
                    );
                    continue;
                }
            }

            // Determine if this goal is stagnant
            let is_stagnant = match goal.last_convergence_check_at {
                Some(last_check) => {
                    let secs_since_check = (now - last_check).num_seconds();
                    secs_since_check > threshold_secs
                }
                None => {
                    // No check ever AND outside grace period
                    true
                }
            };

            if !is_stagnant {
                continue;
            }

            // Dedup: skip if we already alerted for this goal within the threshold window
            let should_alert = match last_alerted.get(&goal.id) {
                Some(last_alert_time) => {
                    let secs_since_alert = (now - *last_alert_time).num_seconds();
                    secs_since_alert > threshold_secs
                }
                None => true,
            };

            if !should_alert {
                tracing::debug!(
                    goal_id = %goal.id,
                    "GoalStagnationDetector: alert already emitted recently, skipping"
                );
                continue;
            }

            tracing::warn!(
                goal_id = %goal.id,
                goal_name = %goal.name,
                "GoalStagnationDetector: goal has not been evaluated in a convergence check recently"
            );

            last_alerted.insert(goal.id, now);

            events.push(crate::services::event_factory::make_event(
                EventSeverity::Warning,
                EventCategory::Escalation,
                Some(goal.id),
                None,
                EventPayload::HumanEscalationRequired(HumanEscalationPayload {
                    goal_id: Some(goal.id),
                    task_id: None,
                    reason: format!(
                        "Goal '{}' (id: {}) has not been evaluated in a convergence check for more than {} seconds. This may indicate goal stagnation.",
                        goal.name, goal.id, threshold_secs
                    ),
                    urgency: "high".to_string(),
                    questions: vec![
                        format!("Goal '{}' may be stagnating. Is there work being done toward this goal?", goal.name),
                        "Consider triggering a manual goal convergence check to generate new tasks.".to_string(),
                    ],
                    is_blocking: false,
                }),
            ));
        }

        if events.is_empty() {
            Ok(Reaction::None)
        } else {
            Ok(Reaction::EmitEvents(events))
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(unused_imports)]
    use super::super::*;
    use super::*;

    use super::*;
    use crate::adapters::sqlite::test_support::setup_goal_repo;
    use crate::domain::models::GoalPriority;
    use crate::domain::ports::goal_repository::GoalRepository;
    use std::sync::Arc;

    fn make_stall_check_event() -> UnifiedEvent {
        UnifiedEvent {
            id: EventId::new(),
            sequence: SequenceNumber(0),
            timestamp: chrono::Utc::now(),
            severity: EventSeverity::Info,
            category: EventCategory::Scheduler,
            goal_id: None,
            task_id: None,
            correlation_id: None,
            source_process_id: None,
            payload: EventPayload::ScheduledEventFired {
                schedule_id: uuid::Uuid::new_v4(),
                name: "system-stall-check".to_string(),
            },
        }
    }

    #[tokio::test]
    async fn test_no_goals_returns_none() {
        let repo = setup_goal_repo().await;
        let handler = GoalStagnationDetectorHandler::new(repo, 3600);
        let event = make_stall_check_event();
        let ctx = HandlerContext {
            chain_depth: 0,
            correlation_id: None,
        };

        let reaction = handler.handle(&event, &ctx).await.unwrap();
        assert!(
            matches!(reaction, Reaction::None),
            "No goals should produce Reaction::None"
        );
    }

    #[tokio::test]
    async fn test_ignores_non_stall_check_events() {
        let repo = setup_goal_repo().await;
        let handler = GoalStagnationDetectorHandler::new(repo, 3600);

        let event = UnifiedEvent {
            id: EventId::new(),
            sequence: SequenceNumber(0),
            timestamp: chrono::Utc::now(),
            severity: EventSeverity::Info,
            category: EventCategory::Scheduler,
            goal_id: None,
            task_id: None,
            correlation_id: None,
            source_process_id: None,
            payload: EventPayload::ScheduledEventFired {
                schedule_id: uuid::Uuid::new_v4(),
                name: "some-other-schedule".to_string(),
            },
        };

        let ctx = HandlerContext {
            chain_depth: 0,
            correlation_id: None,
        };
        let reaction = handler.handle(&event, &ctx).await.unwrap();
        assert!(
            matches!(reaction, Reaction::None),
            "Should ignore non-stall-check events"
        );
    }

    #[tokio::test]
    async fn test_stagnant_goal_emits_escalation() {
        let repo = setup_goal_repo().await;
        // Set a very short stall threshold (1 second) so our goal is immediately stagnant
        let handler = GoalStagnationDetectorHandler::new(repo.clone(), 1);

        let mut goal =
            Goal::new("Stagnant Goal", "Has not been checked").with_priority(GoalPriority::High);
        goal.created_at = chrono::Utc::now() - chrono::Duration::seconds(100);
        repo.create(&goal).await.unwrap();

        // Wait a tiny bit to ensure the threshold is exceeded
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let event = make_stall_check_event();
        let ctx = HandlerContext {
            chain_depth: 0,
            correlation_id: None,
        };

        let reaction = handler.handle(&event, &ctx).await.unwrap();
        match reaction {
            Reaction::EmitEvents(events) => {
                assert_eq!(events.len(), 1, "Should emit exactly one escalation event");
                assert!(matches!(
                    events[0].payload,
                    EventPayload::HumanEscalationRequired(_)
                ));
            }
            Reaction::None => panic!("Expected escalation event for stagnant goal"),
        }
    }

    #[tokio::test]
    async fn test_new_goal_within_grace_period_not_alerted() {
        let repo = setup_goal_repo().await;
        // Stall threshold = 1 hour. New goal just created should be in grace period.
        let handler = GoalStagnationDetectorHandler::new(repo.clone(), 3600);

        let goal = Goal::new("Fresh Goal", "Just created").with_priority(GoalPriority::Normal);
        repo.create(&goal).await.unwrap();

        let event = make_stall_check_event();
        let ctx = HandlerContext {
            chain_depth: 0,
            correlation_id: None,
        };

        let reaction = handler.handle(&event, &ctx).await.unwrap();
        assert!(
            matches!(reaction, Reaction::None),
            "New goal within grace period should not trigger alert"
        );
    }

    #[tokio::test]
    async fn test_dedup_prevents_repeated_alerts() {
        let repo = setup_goal_repo().await;
        // Very short threshold so goal is immediately stagnant
        let handler = GoalStagnationDetectorHandler::new(repo.clone(), 1);

        let mut goal =
            Goal::new("Stagnant Goal", "Will alert once").with_priority(GoalPriority::High);
        goal.created_at = chrono::Utc::now() - chrono::Duration::seconds(100);
        repo.create(&goal).await.unwrap();

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let event = make_stall_check_event();
        let ctx = HandlerContext {
            chain_depth: 0,
            correlation_id: None,
        };

        // First call should emit an escalation
        let reaction = handler.handle(&event, &ctx).await.unwrap();
        assert!(
            matches!(reaction, Reaction::EmitEvents(_)),
            "First call should emit escalation"
        );

        // Second call immediately after should be deduped (within threshold window)
        let reaction2 = handler.handle(&event, &ctx).await.unwrap();
        assert!(
            matches!(reaction2, Reaction::None),
            "Second call should be deduped to Reaction::None"
        );
    }

    #[tokio::test]
    async fn test_recently_checked_goal_not_stagnant() {
        let repo = setup_goal_repo().await;
        // Threshold is 3600 seconds (1 hour)
        let handler = GoalStagnationDetectorHandler::new(repo.clone(), 3600);

        let mut goal =
            Goal::new("Active Goal", "Recently checked").with_priority(GoalPriority::Normal);
        goal.created_at = chrono::Utc::now() - chrono::Duration::seconds(7200);
        repo.create(&goal).await.unwrap();
        // Last convergence check was 10 seconds ago — well within threshold
        repo.update_last_check(goal.id, chrono::Utc::now() - chrono::Duration::seconds(10))
            .await
            .unwrap();

        let event = make_stall_check_event();
        let ctx = HandlerContext {
            chain_depth: 0,
            correlation_id: None,
        };

        let reaction = handler.handle(&event, &ctx).await.unwrap();
        assert!(
            matches!(reaction, Reaction::None),
            "Recently checked goal should not trigger alert"
        );
    }

    #[tokio::test]
    async fn test_multiple_goals_only_stagnant_ones_alert() {
        let repo = setup_goal_repo().await;
        // Short threshold so stagnant goals trigger immediately
        let handler = GoalStagnationDetectorHandler::new(repo.clone(), 1);

        // Stagnant goal: old, never checked
        let mut stagnant =
            Goal::new("Stagnant Goal", "Old and unchecked").with_priority(GoalPriority::High);
        stagnant.created_at = chrono::Utc::now() - chrono::Duration::seconds(200);
        repo.create(&stagnant).await.unwrap();

        // Fresh goal: recently checked
        let mut fresh = Goal::new("Fresh Goal", "Just checked").with_priority(GoalPriority::Normal);
        fresh.created_at = chrono::Utc::now() - chrono::Duration::seconds(200);
        repo.create(&fresh).await.unwrap();
        repo.update_last_check(fresh.id, chrono::Utc::now())
            .await
            .unwrap();

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let event = make_stall_check_event();
        let ctx = HandlerContext {
            chain_depth: 0,
            correlation_id: None,
        };

        let reaction = handler.handle(&event, &ctx).await.unwrap();
        match reaction {
            Reaction::EmitEvents(events) => {
                assert_eq!(
                    events.len(),
                    1,
                    "Should emit exactly one escalation for the stagnant goal only"
                );
                match &events[0].payload {
                    EventPayload::HumanEscalationRequired(p) => {
                        assert!(
                            p.reason.contains("Stagnant Goal"),
                            "Alert should be for the stagnant goal"
                        );
                    }
                    _ => panic!("Expected HumanEscalationRequired payload"),
                }
            }
            Reaction::None => panic!("Expected escalation event for stagnant goal"),
        }
    }

    #[tokio::test]
    async fn test_non_scheduled_payload_returns_none() {
        let repo = setup_goal_repo().await;
        let handler = GoalStagnationDetectorHandler::new(repo, 3600);

        // Use a completely different payload type (not ScheduledEventFired)
        let event = UnifiedEvent {
            id: EventId::new(),
            sequence: SequenceNumber(0),
            timestamp: chrono::Utc::now(),
            severity: EventSeverity::Info,
            category: EventCategory::Task,
            goal_id: None,
            task_id: None,
            correlation_id: None,
            source_process_id: None,
            payload: EventPayload::TaskCompleted {
                task_id: uuid::Uuid::new_v4(),
                tokens_used: 0,
            },
        };

        let ctx = HandlerContext {
            chain_depth: 0,
            correlation_id: None,
        };
        let reaction = handler.handle(&event, &ctx).await.unwrap();
        assert!(
            matches!(reaction, Reaction::None),
            "Non-ScheduledEventFired should return None"
        );
    }

    #[tokio::test]
    async fn test_stagnation_dedup_expires_after_window() {
        let repo = setup_goal_repo().await;
        // Stall threshold = 1 second — alert fires immediately for old goals,
        // and the dedup window also expires after 1 second.
        let handler = GoalStagnationDetectorHandler::new(repo.clone(), 1);

        let mut goal = Goal::new("Dedup Expiry Goal", "Tests that dedup window expires")
            .with_priority(GoalPriority::High);
        goal.created_at = chrono::Utc::now() - chrono::Duration::seconds(200);
        repo.create(&goal).await.unwrap();

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let event = make_stall_check_event();
        let ctx = HandlerContext {
            chain_depth: 0,
            correlation_id: None,
        };

        // First call: should emit escalation
        let reaction1 = handler.handle(&event, &ctx).await.unwrap();
        assert!(
            matches!(reaction1, Reaction::EmitEvents(_)),
            "First call should emit escalation"
        );

        // Second call immediately: should be deduped
        let reaction2 = handler.handle(&event, &ctx).await.unwrap();
        assert!(
            matches!(reaction2, Reaction::None),
            "Immediate second call should be deduped"
        );

        // Wait for the dedup window to expire. The dedup check uses
        // `num_seconds() > threshold_secs` (strictly greater), and num_seconds()
        // truncates, so we need > 1 full second past the threshold (i.e. > 2s total).
        tokio::time::sleep(std::time::Duration::from_millis(2050)).await;

        // Third call after dedup window expires: should re-emit escalation
        let reaction3 = handler.handle(&event, &ctx).await.unwrap();
        assert!(
            matches!(reaction3, Reaction::EmitEvents(_)),
            "Call after dedup window expires should re-emit escalation"
        );
    }
}
