//! Extension traits for federation behavior.
//!
//! These traits define pluggable hooks for delegation strategy, result processing,
//! task transformation, and result schema validation. Default implementations are
//! provided so the system works out of the box without AI decision logic.

use std::time::Duration;
use uuid::Uuid;

use crate::domain::models::a2a::{
    Artifact, CerebrateStatus, FederationResult, FederationTaskEnvelope,
    MessagePriority,
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
        required.iter().all(|req| cerebrate.capabilities.iter().any(|c| c == req))
    }
}

impl FederationDelegationStrategy for DefaultDelegationStrategy {
    fn select_cerebrate(
        &self,
        task: &FederationTaskEnvelope,
        cerebrates: &[CerebrateStatus],
    ) -> Option<String> {
        cerebrates
            .iter()
            .filter(|c| c.can_accept_task())
            .filter(|c| Self::has_required_capabilities(c, &task.required_capabilities))
            .min_by(|a, b| a.load.partial_cmp(&b.load).unwrap_or(std::cmp::Ordering::Equal))
            .map(|c| c.id.clone())
    }

    fn on_rejection(
        &self,
        task: &FederationTaskEnvelope,
        rejected_by: &str,
        _reason: &str,
        remaining: &[CerebrateStatus],
    ) -> DelegationDecision {
        // Try next available cerebrate (excluding the one that rejected)
        let others: Vec<_> = remaining
            .iter()
            .filter(|c| c.id != rejected_by && c.can_accept_task())
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
    EmitEvent {
        description: String,
    },
    /// Escalate to the overmind / human.
    Escalate {
        reason: String,
        goal_id: Option<Uuid>,
    },
    /// Update goal progress.
    UpdateGoalProgress {
        goal_id: Uuid,
        summary: String,
    },
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
        let mut envelope = FederationTaskEnvelope::new(task_id, title, description)
            .with_priority(priority);
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
    fn test_default_result_processor_success() {
        let processor = DefaultResultProcessor;
        let result = FederationResult::completed(Uuid::new_v4(), Uuid::new_v4(), "All done");
        let ctx = ParentContext {
            goal_id: Some(Uuid::new_v4()),
            ..Default::default()
        };

        let reactions = processor.process_result(&result, &ctx);
        assert_eq!(reactions.len(), 1);
        assert!(matches!(reactions[0], FederationReaction::UpdateGoalProgress { .. }));
    }

    #[test]
    fn test_default_result_processor_failure() {
        let processor = DefaultResultProcessor;
        let result = FederationResult::failed(
            Uuid::new_v4(),
            Uuid::new_v4(),
            "Failed",
            "CI broke",
        );
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
