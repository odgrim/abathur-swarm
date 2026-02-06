//! Overmind service.
//!
//! The Overmind service provides the interface between deterministic orchestration
//! code and the Overmind agent. It handles:
//! - Invoking the Overmind with structured requests
//! - Parsing structured JSON responses
//! - Timeout and retry behavior
//! - Concurrency limiting (max 2 Architect instances)
//! - Fallback behavior when Overmind fails

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::Semaphore;
use tokio::time::timeout;
use tracing::{debug, error, info, warn};
use uuid::Uuid;

use crate::domain::errors::{DomainError, DomainResult};
use crate::domain::models::{
    overmind::*,
    SessionStatus, SubstrateConfig, SubstrateRequest,
    OVERMIND_SYSTEM_PROMPT,
};
use crate::domain::ports::Substrate;

/// Configuration for the Overmind service.
#[derive(Debug, Clone)]
pub struct OvermindConfig {
    /// Timeout for Overmind decisions (default: 120s).
    pub decision_timeout: Duration,
    /// Number of retry attempts (default: 2).
    pub retry_attempts: u32,
    /// Cooldown between retries (default: 1s).
    pub retry_cooldown: Duration,
    /// Maximum concurrent Overmind invocations (default: 2).
    pub max_concurrent: usize,
    /// Maximum turns for Overmind (default: 50).
    pub max_turns: u32,
}

impl Default for OvermindConfig {
    fn default() -> Self {
        Self {
            decision_timeout: Duration::from_secs(120),
            retry_attempts: 2,
            retry_cooldown: Duration::from_secs(1),
            max_concurrent: 2,
            max_turns: 50,
        }
    }
}

/// The Overmind service.
///
/// Provides the interface for invoking the Overmind agent to make strategic
/// decisions. All methods accept strongly-typed request structs and return
/// strongly-typed decision structs.
pub struct OvermindService {
    substrate: Arc<dyn Substrate>,
    config: OvermindConfig,
    /// Semaphore to limit concurrent Overmind invocations.
    concurrency_limiter: Semaphore,
    /// System prompt for the Overmind agent.
    /// Loaded from DB template at init, falls back to `OVERMIND_SYSTEM_PROMPT`.
    system_prompt: String,
}

impl OvermindService {
    /// Create a new Overmind service.
    pub fn new(substrate: Arc<dyn Substrate>, config: OvermindConfig) -> Self {
        let max_concurrent = config.max_concurrent;
        Self {
            substrate,
            config,
            concurrency_limiter: Semaphore::new(max_concurrent),
            system_prompt: OVERMIND_SYSTEM_PROMPT.to_string(),
        }
    }

    /// Create with default configuration.
    pub fn with_defaults(substrate: Arc<dyn Substrate>) -> Self {
        Self::new(substrate, OvermindConfig::default())
    }

    /// Create with a custom system prompt (loaded from DB template).
    pub fn with_system_prompt(mut self, prompt: String) -> Self {
        self.system_prompt = prompt;
        self
    }

    // ========================================================================
    // Public Decision Methods
    // ========================================================================

    /// Request goal decomposition from the Overmind.
    pub async fn decompose_goal(
        &self,
        request: GoalDecompositionRequest,
    ) -> DomainResult<GoalDecompositionDecision> {
        let overmind_request = OvermindRequest::GoalDecomposition(request);
        let decision = self.invoke(overmind_request).await?;

        match decision {
            OvermindDecision::GoalDecomposition(d) => Ok(d),
            _ => Err(DomainError::ExecutionFailed(
                "Overmind returned unexpected decision type".to_string(),
            )),
        }
    }

    /// Request goal prioritization from the Overmind.
    pub async fn prioritize_goals(
        &self,
        request: PrioritizationRequest,
    ) -> DomainResult<PrioritizationDecision> {
        let overmind_request = OvermindRequest::Prioritization(request);
        let decision = self.invoke(overmind_request).await?;

        match decision {
            OvermindDecision::Prioritization(d) => Ok(d),
            _ => Err(DomainError::ExecutionFailed(
                "Overmind returned unexpected decision type".to_string(),
            )),
        }
    }

    /// Request capability gap analysis from the Overmind.
    pub async fn analyze_capability_gap(
        &self,
        request: CapabilityGapRequest,
    ) -> DomainResult<CapabilityGapDecision> {
        let overmind_request = OvermindRequest::CapabilityGap(request);
        let decision = self.invoke(overmind_request).await?;

        match decision {
            OvermindDecision::CapabilityGap(d) => Ok(d),
            _ => Err(DomainError::ExecutionFailed(
                "Overmind returned unexpected decision type".to_string(),
            )),
        }
    }

    /// Request conflict resolution from the Overmind.
    pub async fn resolve_conflict(
        &self,
        request: ConflictResolutionRequest,
    ) -> DomainResult<ConflictResolutionDecision> {
        let overmind_request = OvermindRequest::ConflictResolution(request);
        let decision = self.invoke(overmind_request).await?;

        match decision {
            OvermindDecision::ConflictResolution(d) => Ok(d),
            _ => Err(DomainError::ExecutionFailed(
                "Overmind returned unexpected decision type".to_string(),
            )),
        }
    }

    /// Request stuck state recovery analysis from the Overmind.
    pub async fn recover_from_stuck(
        &self,
        request: StuckStateRecoveryRequest,
    ) -> DomainResult<StuckStateRecoveryDecision> {
        let overmind_request = OvermindRequest::StuckStateRecovery(request);
        let decision = self.invoke(overmind_request).await?;

        match decision {
            OvermindDecision::StuckStateRecovery(d) => Ok(d),
            _ => Err(DomainError::ExecutionFailed(
                "Overmind returned unexpected decision type".to_string(),
            )),
        }
    }

    /// Request escalation evaluation from the Overmind.
    pub async fn evaluate_escalation(
        &self,
        request: EscalationRequest,
    ) -> DomainResult<OvermindEscalationDecision> {
        let overmind_request = OvermindRequest::Escalation(request);
        let decision = self.invoke(overmind_request).await?;

        match decision {
            OvermindDecision::Escalation(d) => Ok(d),
            _ => Err(DomainError::ExecutionFailed(
                "Overmind returned unexpected decision type".to_string(),
            )),
        }
    }

    // ========================================================================
    // Core Invocation
    // ========================================================================

    /// Invoke the Overmind with a request and parse the response.
    async fn invoke(&self, request: OvermindRequest) -> DomainResult<OvermindDecision> {
        // Acquire concurrency permit
        let _permit = self.concurrency_limiter.acquire().await.map_err(|e| {
            DomainError::ExecutionFailed(format!("Failed to acquire Overmind permit: {}", e))
        })?;

        let request_type = request.request_type_name();
        info!("Invoking Overmind for {} decision", request_type);

        // Build the prompt
        let prompt = self.build_prompt(&request)?;

        // Retry loop
        let mut last_error = None;
        for attempt in 0..=self.config.retry_attempts {
            if attempt > 0 {
                warn!(
                    "Retrying Overmind invocation (attempt {}/{})",
                    attempt + 1,
                    self.config.retry_attempts + 1
                );
                tokio::time::sleep(self.config.retry_cooldown).await;
            }

            match self.invoke_once(&prompt, request_type).await {
                Ok(decision) => {
                    info!(
                        "Overmind {} decision completed with confidence {:.2}",
                        request_type,
                        decision.metadata().confidence
                    );
                    return Ok(decision);
                }
                Err(e) => {
                    warn!("Overmind invocation failed: {}", e);
                    last_error = Some(e);
                }
            }
        }

        Err(last_error.unwrap_or_else(|| {
            DomainError::ExecutionFailed("Overmind invocation failed".to_string())
        }))
    }

    /// Single invocation attempt.
    async fn invoke_once(
        &self,
        prompt: &str,
        request_type: &str,
    ) -> DomainResult<OvermindDecision> {
        let task_id = Uuid::new_v4();

        let substrate_request = SubstrateRequest::new(
            task_id,
            "overmind",
            &self.system_prompt,
            prompt,
        )
        .with_config(
            SubstrateConfig::default()
                .with_max_turns(self.config.max_turns)
                .with_allowed_tools(vec![
                    "read".to_string(),
                    "glob".to_string(),
                    "grep".to_string(),
                    "memory_query".to_string(),
                ]),
        );

        // Execute with timeout
        let session = match timeout(
            self.config.decision_timeout,
            self.substrate.execute(substrate_request),
        )
        .await
        {
            Ok(result) => result?,
            Err(_) => {
                return Err(DomainError::ExecutionFailed(format!(
                    "Overmind {} decision timed out after {:?}",
                    request_type, self.config.decision_timeout
                )));
            }
        };

        // Check session status
        if session.status != SessionStatus::Completed {
            return Err(DomainError::ExecutionFailed(format!(
                "Overmind session failed: {:?} - {}",
                session.status,
                session.error.unwrap_or_else(|| "Unknown error".to_string())
            )));
        }

        // Extract and parse JSON from the response
        let response = session
            .result
            .ok_or_else(|| DomainError::ExecutionFailed("No response from Overmind".to_string()))?;

        self.parse_decision(&response, request_type)
    }

    /// Build the prompt for the Overmind.
    fn build_prompt(&self, request: &OvermindRequest) -> DomainResult<String> {
        let request_json = serde_json::to_string_pretty(request)?;

        Ok(format!(
            "Make a decision for the following request.\n\n\
            REQUEST:\n{}\n\n\
            Respond with ONLY the JSON decision object matching the schema for this request type. \
            No additional text or formatting.",
            request_json
        ))
    }

    /// Parse the decision from the Overmind response.
    fn parse_decision(
        &self,
        response: &str,
        request_type: &str,
    ) -> DomainResult<OvermindDecision> {
        // Try to extract JSON from the response
        let json_str = self.extract_json(response);

        debug!("Parsing Overmind response for {}: {}", request_type, json_str);

        // Parse based on expected decision type
        let decision: OvermindDecision = serde_json::from_str(&json_str).map_err(|e| {
            error!(
                "Failed to parse Overmind response as {}: {}. Response: {}",
                request_type, e, json_str
            );
            DomainError::SerializationError(format!(
                "Failed to parse Overmind {} decision: {}",
                request_type, e
            ))
        })?;

        Ok(decision)
    }

    /// Extract JSON from a response that might have surrounding text.
    fn extract_json(&self, response: &str) -> String {
        let trimmed = response.trim();

        // If it already looks like JSON, use it directly
        if trimmed.starts_with('{') && trimmed.ends_with('}') {
            return trimmed.to_string();
        }

        // Try to find JSON object in the response
        if let Some(start) = trimmed.find('{') {
            if let Some(end) = trimmed.rfind('}') {
                if end > start {
                    return trimmed[start..=end].to_string();
                }
            }
        }

        // Return as-is if no JSON found
        trimmed.to_string()
    }

    // ========================================================================
    // Configuration
    // ========================================================================

    /// Get the current configuration.
    pub fn config(&self) -> &OvermindConfig {
        &self.config
    }

    /// Check if any Overmind invocation slots are available.
    pub fn has_capacity(&self) -> bool {
        self.concurrency_limiter.available_permits() > 0
    }

    /// Get the number of available Overmind slots.
    pub fn available_slots(&self) -> usize {
        self.concurrency_limiter.available_permits()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::substrates::mock::{MockSubstrate, MockResponse};

    fn create_mock_substrate_with_response(response: &str) -> MockSubstrate {
        MockSubstrate::with_default_response(MockResponse::success(response))
    }

    #[tokio::test]
    async fn test_decompose_goal() {
        let response = r#"{
            "decision_type": "goal_decomposition",
            "metadata": {
                "decision_id": "550e8400-e29b-41d4-a716-446655440000",
                "confidence": 0.85,
                "rationale": "Clear decomposition based on requirements",
                "alternatives_considered": ["Single task approach"],
                "risks": ["Dependencies might shift"],
                "decided_at": "2024-01-15T10:30:00Z"
            },
            "strategy": "sequential",
            "tasks": [
                {
                    "title": "Design the API",
                    "description": "Create API specification",
                    "agent_type": "api-designer",
                    "priority": "high",
                    "depends_on": [],
                    "needs_worktree": false,
                    "estimated_complexity": 3,
                    "acceptance_criteria": ["OpenAPI spec created"]
                },
                {
                    "title": "Implement the API",
                    "description": "Implement the API endpoints",
                    "agent_type": "code-implementer",
                    "priority": "high",
                    "depends_on": ["Design the API"],
                    "needs_worktree": true,
                    "estimated_complexity": 4,
                    "acceptance_criteria": ["All endpoints working", "Tests passing"]
                }
            ],
            "verification_points": [
                {
                    "after_tasks": ["Design the API"],
                    "verify": "API spec is complete and valid",
                    "is_blocking": true
                }
            ],
            "execution_hints": ["Design must complete before implementation"]
        }"#;

        let substrate = Arc::new(create_mock_substrate_with_response(response));
        let service = OvermindService::with_defaults(substrate);

        let request = GoalDecompositionRequest {
            goal_id: Uuid::new_v4(),
            goal_name: "Create User API".to_string(),
            goal_description: "Create a REST API for user management".to_string(),
            constraints: vec!["Must be RESTful".to_string()],
            available_agents: vec!["api-designer".to_string(), "code-implementer".to_string()],
            existing_tasks: vec![],
            memory_patterns: vec![],
            max_tasks: 10,
        };

        let decision = service.decompose_goal(request).await.unwrap();

        assert_eq!(decision.metadata.confidence, 0.85);
        assert_eq!(decision.tasks.len(), 2);
        assert_eq!(decision.tasks[0].title, "Design the API");
        assert_eq!(decision.tasks[1].depends_on, vec!["Design the API"]);
        assert_eq!(decision.verification_points.len(), 1);
    }

    #[tokio::test]
    async fn test_evaluate_escalation() {
        let response = r#"{
            "decision_type": "escalation",
            "metadata": {
                "decision_id": "550e8400-e29b-41d4-a716-446655440001",
                "confidence": 0.95,
                "rationale": "API credentials required but not available",
                "alternatives_considered": ["Use mock API"],
                "risks": ["Delay if human unavailable"],
                "decided_at": "2024-01-15T10:30:00Z"
            },
            "should_escalate": true,
            "urgency": "high",
            "questions": ["What API key should be used?"],
            "context_for_human": "Task requires payment API but no credentials found",
            "alternatives_if_unavailable": ["Skip payment integration for now"],
            "is_blocking": true
        }"#;

        let substrate = Arc::new(create_mock_substrate_with_response(response));
        let service = OvermindService::with_defaults(substrate);

        let request = EscalationRequest {
            context: EscalationContext {
                goal_id: Some(Uuid::new_v4()),
                task_id: Some(Uuid::new_v4()),
                situation: "Need API credentials".to_string(),
                attempts_made: vec!["Checked env vars".to_string()],
                time_spent_minutes: 15,
            },
            trigger: EscalationTrigger::AmbiguousRequirements,
            previous_escalations: vec![],
            escalation_preferences: EscalationPreferences::default(),
        };

        let decision = service.evaluate_escalation(request).await.unwrap();

        assert!(decision.should_escalate);
        assert_eq!(decision.urgency, Some(OvermindEscalationUrgency::High));
        assert_eq!(decision.questions.len(), 1);
        assert!(decision.is_blocking);
    }

    #[tokio::test]
    async fn test_stuck_state_recovery() {
        let response = r#"{
            "decision_type": "stuck_state_recovery",
            "metadata": {
                "decision_id": "550e8400-e29b-41d4-a716-446655440002",
                "confidence": 0.75,
                "rationale": "Task needs research before implementation",
                "alternatives_considered": ["Retry with same approach"],
                "risks": ["Research might take time"],
                "decided_at": "2024-01-15T10:30:00Z"
            },
            "root_cause": {
                "category": "information_gap",
                "explanation": "The task failed because the required library API is unfamiliar",
                "evidence": ["Error shows unknown method call", "No docs in codebase"]
            },
            "recovery_action": {
                "research_first": {
                    "research_questions": ["How does the X library work?", "What methods are available?"]
                }
            },
            "new_tasks": [],
            "cancel_original": false
        }"#;

        let substrate = Arc::new(create_mock_substrate_with_response(response));
        let service = OvermindService::with_defaults(substrate);

        let request = StuckStateRecoveryRequest {
            task_id: Uuid::new_v4(),
            task_title: "Implement caching".to_string(),
            task_description: "Add caching to the API".to_string(),
            goal_context: GoalContext {
                goal_id: Uuid::new_v4(),
                goal_name: "Improve performance".to_string(),
                goal_description: "Make the API faster".to_string(),
                other_tasks_status: "2 pending, 1 complete".to_string(),
            },
            failure_history: vec![FailureRecord {
                attempt: 1,
                timestamp: chrono::Utc::now(),
                error: "Unknown method".to_string(),
                agent_type: "code-implementer".to_string(),
                turns_used: 15,
            }],
            previous_recovery_attempts: vec![],
            available_approaches: vec!["Use different library".to_string()],
        };

        let decision = service.recover_from_stuck(request).await.unwrap();

        assert_eq!(decision.root_cause.category, RootCauseCategory::InformationGap);
        assert!(!decision.cancel_original);
        match decision.recovery_action {
            RecoveryAction::ResearchFirst { research_questions } => {
                assert_eq!(research_questions.len(), 2);
            }
            _ => panic!("Expected ResearchFirst action"),
        }
    }

    #[tokio::test]
    async fn test_concurrency_limiting() {
        let service = OvermindService::with_defaults(Arc::new(MockSubstrate::default()));

        assert!(service.has_capacity());
        assert_eq!(service.available_slots(), 2); // Default max_concurrent is 2
    }

    #[tokio::test]
    async fn test_json_extraction() {
        let service = OvermindService::with_defaults(Arc::new(MockSubstrate::default()));

        // Plain JSON
        let json = r#"{"key": "value"}"#;
        assert_eq!(service.extract_json(json), json);

        // JSON with surrounding text
        let with_text = r#"Here is the response: {"key": "value"} and some more text"#;
        assert_eq!(service.extract_json(with_text), r#"{"key": "value"}"#);

        // JSON with whitespace
        let with_whitespace = r#"

            {"key": "value"}

        "#;
        assert_eq!(service.extract_json(with_whitespace), r#"{"key": "value"}"#);
    }

    #[tokio::test]
    async fn test_conflict_resolution() {
        let response = r#"{
            "decision_type": "conflict_resolution",
            "metadata": {
                "decision_id": "550e8400-e29b-41d4-a716-446655440003",
                "confidence": 0.8,
                "rationale": "Serialize operations to avoid race condition",
                "alternatives_considered": ["Let one fail", "Merge changes"],
                "risks": ["Slight delay in execution"],
                "decided_at": "2024-01-15T10:30:00Z"
            },
            "approach": "serialize",
            "task_modifications": [
                {
                    "task_id": "550e8400-e29b-41d4-a716-446655440010",
                    "modification_type": {"add_dependency": {"depends_on": "550e8400-e29b-41d4-a716-446655440001"}},
                    "description": "Task B now depends on Task A"
                }
            ],
            "notifications": ["Task B delayed due to resource conflict"]
        }"#;

        let substrate = Arc::new(create_mock_substrate_with_response(response));
        let service = OvermindService::with_defaults(substrate);

        let request = ConflictResolutionRequest {
            conflict_type: ConflictType::ResourceContention,
            parties: vec![
                ConflictParty {
                    party_type: "task".to_string(),
                    id: Uuid::new_v4(),
                    name: "Task A".to_string(),
                    interest: "Write to config.json".to_string(),
                },
                ConflictParty {
                    party_type: "task".to_string(),
                    id: Uuid::new_v4(),
                    name: "Task B".to_string(),
                    interest: "Write to config.json".to_string(),
                },
            ],
            context: "Both tasks want to modify the same file".to_string(),
            previous_attempts: vec![],
        };

        let decision = service.resolve_conflict(request).await.unwrap();

        assert_eq!(decision.metadata.confidence, 0.8);
        assert_eq!(decision.approach, ConflictResolutionApproach::Serialize);
        assert_eq!(decision.task_modifications.len(), 1);
    }
}
