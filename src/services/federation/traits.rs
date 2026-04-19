//! Extension traits for federation behavior.
//!
//! These traits define pluggable hooks for delegation strategy, result processing,
//! task transformation, and result schema validation. Default implementations are
//! provided so the system works out of the box without AI decision logic.

use std::time::Duration;
use uuid::Uuid;

use crate::domain::models::a2a::{
    Artifact, CerebrateStatus, FederationResult, FederationTaskEnvelope, MessagePriority,
};

// ============================================================================
// Delegation Strategy
// ============================================================================

/// Decision returned by `on_rejection` when a cerebrate rejects a task.
#[derive(Debug, Clone)]
pub enum DelegationDecision {
    /// Try delegating to a different cerebrate.
    Redelegate(String),
    /// Execute the task locally instead.
    ExecuteLocally,
    /// Wait and retry after a duration.
    RetryAfter(Duration),
    /// Give up.
    Fail(String),
}

/// Strategy for selecting which cerebrate receives a delegated task.
///
/// The default implementation (`DefaultDelegationStrategy`) uses round-robin
/// weighted by current load.
pub trait FederationDelegationStrategy: Send + Sync {
    /// Select the best cerebrate for the given task envelope.
    /// Returns `None` if no suitable cerebrate is available.
    fn select_cerebrate(
        &self,
        task: &FederationTaskEnvelope,
        cerebrates: &[CerebrateStatus],
    ) -> Option<String>;

    /// Called when a cerebrate rejects a task.
    fn on_rejection(
        &self,
        task: &FederationTaskEnvelope,
        rejected_by: &str,
        reason: &str,
        remaining: &[CerebrateStatus],
    ) -> DelegationDecision;
}

/// Default delegation strategy: selects the connected cerebrate with the lowest load
/// whose capabilities satisfy the task's requirements.
pub struct DefaultDelegationStrategy;

impl DefaultDelegationStrategy {
    /// Check whether a cerebrate's capabilities satisfy all required capabilities.
    fn has_required_capabilities(cerebrate: &CerebrateStatus, required: &[String]) -> bool {
        required
            .iter()
            .all(|req| cerebrate.capabilities.iter().any(|c| c == req))
    }
}

/// Maximum number of re-delegation attempts before giving up on a task and
/// falling through to local execution. Prevents infinite redelegation loops.
const MAX_REJECTION_COUNT: u32 = 5;

impl FederationDelegationStrategy for DefaultDelegationStrategy {
    fn select_cerebrate(
        &self,
        task: &FederationTaskEnvelope,
        cerebrates: &[CerebrateStatus],
    ) -> Option<String> {
        // Build a quick lookup of in-flight counts from the envelope's peer
        // hints. Strategy: prefer the under-loaded peer when hints are present;
        // fall back to the reported `load` metric otherwise.
        let hint_for = |id: &str| -> Option<u32> {
            task.peer_load_hints
                .iter()
                .find(|(cid, _)| cid == id)
                .map(|(_, n)| *n)
        };

        cerebrates
            .iter()
            .filter(|c| c.can_accept_task())
            .filter(|c| Self::has_required_capabilities(c, &task.required_capabilities))
            // Never pick a cerebrate that has already rejected this task.
            .filter(|c| !task.rejected_by.iter().any(|r| r == &c.id))
            .min_by(|a, b| {
                match (hint_for(&a.id), hint_for(&b.id)) {
                    (Some(la), Some(lb)) => la.cmp(&lb),
                    _ => a
                        .load
                        .partial_cmp(&b.load)
                        .unwrap_or(std::cmp::Ordering::Equal),
                }
            })
            .map(|c| c.id.clone())
    }

    fn on_rejection(
        &self,
        task: &FederationTaskEnvelope,
        rejected_by: &str,
        _reason: &str,
        remaining: &[CerebrateStatus],
    ) -> DelegationDecision {
        // Bail out of the redelegation loop if we've exhausted the budget.
        if task.rejection_count >= MAX_REJECTION_COUNT {
            return DelegationDecision::Fail(format!(
                "Task {} rejected {} times; giving up",
                task.task_id, task.rejection_count
            ));
        }

        // Exclude the rejecting cerebrate AND anyone in the rejection history
        // so we don't ping-pong the task between the same peers.
        let others: Vec<_> = remaining
            .iter()
            .filter(|c| c.id != rejected_by && c.can_accept_task())
            .filter(|c| !task.rejected_by.iter().any(|r| r == &c.id))
            .cloned()
            .collect();

        if let Some(next_id) = self.select_cerebrate(task, &others) {
            DelegationDecision::Redelegate(next_id)
        } else {
            DelegationDecision::ExecuteLocally
        }
    }
}

// ============================================================================
// Result Processor
// ============================================================================

/// Reaction to emit after processing a federation result.
#[derive(Debug, Clone)]
pub enum FederationReaction {
    /// Create a new local task.
    CreateTask {
        title: String,
        description: String,
        parent_goal_id: Option<Uuid>,
    },
    /// Delegate follow-up work to a cerebrate.
    DelegateFollowUp {
        envelope: Box<FederationTaskEnvelope>,
        preferred_cerebrate: Option<String>,
    },
    /// Emit an event through the EventBus.
    EmitEvent { description: String },
    /// Escalate to the overmind / human.
    Escalate {
        reason: String,
        goal_id: Option<Uuid>,
    },
    /// Update goal progress.
    UpdateGoalProgress { goal_id: Uuid, summary: String },
    /// No action needed.
    None,
}

/// Context from the parent swarm passed to the result processor.
#[derive(Debug, Clone, Default)]
pub struct ParentContext {
    pub goal_id: Option<Uuid>,
    pub goal_summary: Option<String>,
    pub task_title: Option<String>,
}

/// Strategy for processing results received from cerebrates.
///
/// Default implementation: emits `UpdateGoalProgress` on success, `Escalate` on failure.
pub trait FederationResultProcessor: Send + Sync {
    /// Process a successful or partial result.
    fn process_result(
        &self,
        result: &FederationResult,
        parent_context: &ParentContext,
    ) -> Vec<FederationReaction>;

    /// Process a failed result.
    fn process_failure(
        &self,
        result: &FederationResult,
        parent_context: &ParentContext,
    ) -> Vec<FederationReaction>;
}

/// Default result processor.
pub struct DefaultResultProcessor;

impl FederationResultProcessor for DefaultResultProcessor {
    fn process_result(
        &self,
        result: &FederationResult,
        parent_context: &ParentContext,
    ) -> Vec<FederationReaction> {
        let mut reactions = Vec::new();
        if let Some(goal_id) = parent_context.goal_id {
            reactions.push(FederationReaction::UpdateGoalProgress {
                goal_id,
                summary: result.summary.clone(),
            });
        }
        reactions
    }

    fn process_failure(
        &self,
        result: &FederationResult,
        parent_context: &ParentContext,
    ) -> Vec<FederationReaction> {
        vec![FederationReaction::Escalate {
            reason: result
                .failure_reason
                .clone()
                .unwrap_or_else(|| "Federation task failed".to_string()),
            goal_id: parent_context.goal_id,
        }]
    }
}

// ============================================================================
// Task Transformer
// ============================================================================

/// Transforms between local tasks/goals and federation envelopes.
pub trait FederationTaskTransformer: Send + Sync {
    /// Convert a local task description into a federation envelope.
    fn to_envelope(
        &self,
        task_id: Uuid,
        title: &str,
        description: &str,
        goal_id: Option<Uuid>,
        priority: MessagePriority,
    ) -> FederationTaskEnvelope;

    /// Extract goal creation parameters from a received envelope.
    fn parse_envelope(
        &self,
        envelope: &FederationTaskEnvelope,
        parent_id: Option<Uuid>,
    ) -> GoalCreationParams;
}

/// Parameters for creating a goal from a received federation envelope.
#[derive(Debug, Clone)]
pub struct GoalCreationParams {
    pub title: String,
    pub description: String,
    pub parent_id: Option<Uuid>,
    pub constraints: Vec<String>,
    pub correlation_id: Uuid,
}

/// Default task transformer: maps fields directly.
pub struct DefaultTaskTransformer;

impl FederationTaskTransformer for DefaultTaskTransformer {
    fn to_envelope(
        &self,
        task_id: Uuid,
        title: &str,
        description: &str,
        goal_id: Option<Uuid>,
        priority: MessagePriority,
    ) -> FederationTaskEnvelope {
        let mut envelope =
            FederationTaskEnvelope::new(task_id, title, description).with_priority(priority);
        if let Some(gid) = goal_id {
            envelope = envelope.with_parent_goal(gid);
        }
        envelope
    }

    fn parse_envelope(
        &self,
        envelope: &FederationTaskEnvelope,
        parent_id: Option<Uuid>,
    ) -> GoalCreationParams {
        GoalCreationParams {
            title: envelope.title.clone(),
            description: envelope.description.clone(),
            parent_id,
            constraints: envelope.constraints.clone(),
            correlation_id: envelope.correlation_id,
        }
    }
}

// ============================================================================
// Result Schema
// ============================================================================

/// Validation and extraction of structured federation results.
pub trait ResultSchema: Send + Sync {
    /// Unique identifier for this schema.
    fn schema_id(&self) -> &str;

    /// Validate a result value against this schema.
    fn validate(&self, value: &serde_json::Value) -> Result<(), String>;

    /// Extract artifacts from a result value.
    fn extract_artifacts(&self, value: &serde_json::Value) -> Vec<Artifact>;
}

/// Standard v1 schema: expects a JSON object with optional `artifacts` array.
pub struct StandardV1Schema;

impl ResultSchema for StandardV1Schema {
    fn schema_id(&self) -> &str {
        "standard_v1"
    }

    fn validate(&self, value: &serde_json::Value) -> Result<(), String> {
        if !value.is_object() {
            return Err("Expected a JSON object".to_string());
        }
        Ok(())
    }

    fn extract_artifacts(&self, value: &serde_json::Value) -> Vec<Artifact> {
        let mut artifacts = Vec::new();
        if let Some(arr) = value.get("artifacts").and_then(|v| v.as_array()) {
            for item in arr {
                if let (Some(t), Some(v)) = (
                    item.get("type").and_then(|v| v.as_str()),
                    item.get("value").and_then(|v| v.as_str()),
                ) {
                    artifacts.push(Artifact::new(t, v));
                }
            }
        }
        artifacts
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::models::a2a::ConnectionState;

    #[test]
    fn test_default_delegation_strategy_selects_lowest_load() {
        let strategy = DefaultDelegationStrategy;
        let envelope = FederationTaskEnvelope::new(Uuid::new_v4(), "Test", "Test desc");

        let cerebrates = vec![
            {
                let mut c = CerebrateStatus::new("c1", "Cerebrate 1").with_max_delegations(5);
                c.connection_state = ConnectionState::Connected;
                c.load = 0.8;
                c
            },
            {
                let mut c = CerebrateStatus::new("c2", "Cerebrate 2").with_max_delegations(5);
                c.connection_state = ConnectionState::Connected;
                c.load = 0.2;
                c
            },
        ];

        let selected = strategy.select_cerebrate(&envelope, &cerebrates);
        assert_eq!(selected.as_deref(), Some("c2"));
    }

    #[test]
    fn test_default_delegation_strategy_skips_disconnected() {
        let strategy = DefaultDelegationStrategy;
        let envelope = FederationTaskEnvelope::new(Uuid::new_v4(), "Test", "Test desc");

        let cerebrates = vec![
            {
                let mut c = CerebrateStatus::new("c1", "Cerebrate 1").with_max_delegations(5);
                c.connection_state = ConnectionState::Disconnected;
                c.load = 0.1;
                c
            },
            {
                let mut c = CerebrateStatus::new("c2", "Cerebrate 2").with_max_delegations(5);
                c.connection_state = ConnectionState::Connected;
                c.load = 0.5;
                c
            },
        ];

        let selected = strategy.select_cerebrate(&envelope, &cerebrates);
        assert_eq!(selected.as_deref(), Some("c2"));
    }

    #[test]
    fn test_default_delegation_strategy_returns_none_when_all_full() {
        let strategy = DefaultDelegationStrategy;
        let envelope = FederationTaskEnvelope::new(Uuid::new_v4(), "Test", "Test desc");

        let cerebrates = vec![{
            let mut c = CerebrateStatus::new("c1", "Cerebrate 1").with_max_delegations(1);
            c.connection_state = ConnectionState::Connected;
            c.active_delegations = 1;
            c
        }];

        let selected = strategy.select_cerebrate(&envelope, &cerebrates);
        assert!(selected.is_none());
    }

    #[test]
    fn test_default_delegation_on_rejection_redelegates() {
        let strategy = DefaultDelegationStrategy;
        let envelope = FederationTaskEnvelope::new(Uuid::new_v4(), "Test", "Test desc");

        let remaining = vec![
            {
                let mut c = CerebrateStatus::new("c1", "Cerebrate 1").with_max_delegations(5);
                c.connection_state = ConnectionState::Connected;
                c.load = 0.3;
                c
            },
            {
                let mut c = CerebrateStatus::new("c2", "Cerebrate 2").with_max_delegations(5);
                c.connection_state = ConnectionState::Connected;
                c.load = 0.5;
                c
            },
        ];

        let decision = strategy.on_rejection(&envelope, "c1", "busy", &remaining);
        match decision {
            DelegationDecision::Redelegate(id) => assert_eq!(id, "c2"),
            _ => panic!("Expected Redelegate, got {:?}", decision),
        }
    }

    #[test]
    fn test_default_delegation_avoids_prior_rejectors() {
        let strategy = DefaultDelegationStrategy;
        let envelope = FederationTaskEnvelope::new(Uuid::new_v4(), "Test", "Test desc")
            .with_rejection_history(vec!["c1".to_string(), "c2".to_string()]);

        let remaining = vec![
            {
                let mut c = CerebrateStatus::new("c1", "Cerebrate 1").with_max_delegations(5);
                c.connection_state = ConnectionState::Connected;
                c.load = 0.1;
                c
            },
            {
                let mut c = CerebrateStatus::new("c2", "Cerebrate 2").with_max_delegations(5);
                c.connection_state = ConnectionState::Connected;
                c.load = 0.1;
                c
            },
            {
                let mut c = CerebrateStatus::new("c3", "Cerebrate 3").with_max_delegations(5);
                c.connection_state = ConnectionState::Connected;
                c.load = 0.9;
                c
            },
        ];

        // c1/c2 already rejected; c3 is the only valid target even though
        // its raw `load` is highest.
        let decision = strategy.on_rejection(&envelope, "c2", "busy", &remaining);
        match decision {
            DelegationDecision::Redelegate(id) => assert_eq!(id, "c3"),
            other => panic!("Expected Redelegate to c3, got {:?}", other),
        }
    }

    #[test]
    fn test_default_delegation_prefers_lowest_peer_load_hint() {
        let strategy = DefaultDelegationStrategy;
        // c1 has higher reported `load` but fewer in-flight tasks per the
        // envelope hints; we should prefer c1.
        let envelope = FederationTaskEnvelope::new(Uuid::new_v4(), "Test", "Test desc")
            .with_peer_load_hints(vec![("c1".to_string(), 1), ("c2".to_string(), 4)]);

        let cerebrates = vec![
            {
                let mut c = CerebrateStatus::new("c1", "Cerebrate 1").with_max_delegations(5);
                c.connection_state = ConnectionState::Connected;
                c.load = 0.9;
                c
            },
            {
                let mut c = CerebrateStatus::new("c2", "Cerebrate 2").with_max_delegations(5);
                c.connection_state = ConnectionState::Connected;
                c.load = 0.1;
                c
            },
        ];

        let selected = strategy.select_cerebrate(&envelope, &cerebrates);
        assert_eq!(selected.as_deref(), Some("c1"));
    }

    #[test]
    fn test_default_delegation_fails_after_max_rejections() {
        let strategy = DefaultDelegationStrategy;
        let rejectors: Vec<String> =
            (0..6).map(|i| format!("c{}", i)).collect();
        let envelope = FederationTaskEnvelope::new(Uuid::new_v4(), "Test", "Test desc")
            .with_rejection_history(rejectors);

        let remaining = vec![{
            let mut c = CerebrateStatus::new("c99", "Fresh").with_max_delegations(5);
            c.connection_state = ConnectionState::Connected;
            c.load = 0.1;
            c
        }];

        let decision = strategy.on_rejection(&envelope, "c5", "busy", &remaining);
        assert!(
            matches!(decision, DelegationDecision::Fail(_)),
            "Expected Fail after exceeding MAX_REJECTION_COUNT, got {:?}",
            decision
        );
    }

    #[test]
    fn test_default_result_processor_success() {
        let processor = DefaultResultProcessor;
        let result = FederationResult::completed(Uuid::new_v4(), Uuid::new_v4(), "All done");
        let ctx = ParentContext {
            goal_id: Some(Uuid::new_v4()),
            ..Default::default()
        };

        let reactions = processor.process_result(&result, &ctx);
        assert_eq!(reactions.len(), 1);
        assert!(matches!(
            reactions[0],
            FederationReaction::UpdateGoalProgress { .. }
        ));
    }

    #[test]
    fn test_default_result_processor_failure() {
        let processor = DefaultResultProcessor;
        let result = FederationResult::failed(Uuid::new_v4(), Uuid::new_v4(), "Failed", "CI broke");
        let ctx = ParentContext {
            goal_id: Some(Uuid::new_v4()),
            ..Default::default()
        };

        let reactions = processor.process_failure(&result, &ctx);
        assert_eq!(reactions.len(), 1);
        assert!(matches!(reactions[0], FederationReaction::Escalate { .. }));
    }

    #[test]
    fn test_default_task_transformer_roundtrip() {
        let transformer = DefaultTaskTransformer;
        let task_id = Uuid::new_v4();
        let goal_id = Uuid::new_v4();

        let envelope = transformer.to_envelope(
            task_id,
            "My Task",
            "Do the thing",
            Some(goal_id),
            MessagePriority::High,
        );

        assert_eq!(envelope.task_id, task_id);
        assert_eq!(envelope.parent_goal_id, Some(goal_id));
        assert_eq!(envelope.priority, MessagePriority::High);

        let params = transformer.parse_envelope(&envelope, None);
        assert_eq!(params.title, "My Task");
        assert_eq!(params.description, "Do the thing");
    }

    #[test]
    fn test_standard_v1_schema_validate() {
        let schema = StandardV1Schema;
        assert_eq!(schema.schema_id(), "standard_v1");

        let valid = serde_json::json!({"summary": "done"});
        assert!(schema.validate(&valid).is_ok());

        let invalid = serde_json::json!("not an object");
        assert!(schema.validate(&invalid).is_err());
    }

    #[test]
    fn test_standard_v1_schema_extract_artifacts() {
        let schema = StandardV1Schema;
        let value = serde_json::json!({
            "artifacts": [
                {"type": "pr_url", "value": "https://github.com/org/repo/pull/1"},
                {"type": "commit_sha", "value": "abc123"}
            ]
        });

        let artifacts = schema.extract_artifacts(&value);
        assert_eq!(artifacts.len(), 2);
        assert_eq!(artifacts[0].artifact_type, "pr_url");
        assert_eq!(artifacts[1].value, "abc123");
    }
}
