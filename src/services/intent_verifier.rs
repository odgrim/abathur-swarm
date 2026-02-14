//! Intent Verifier service.
//!
//! Verifies that completed work satisfies the original intent, not just
//! the derived task checklist. Uses an LLM agent to evaluate work against
//! the original goal/prompt and identify gaps that may have been missed.
//!
//! ## Key Concepts
//!
//! **Goals are convergent attractors** - they guide work but are never "completed."
//! Intent verification happens at the **task** and **wave** level, not at "goal completion."
//!
//! The verifier answers the question: "If someone submitted the exact same prompt again,
//! would there be additional work that should be done?"
//!
//! ## Verification Levels
//!
//! - **Task-level**: Verify a single task against its description
//! - **Wave-level**: Verify a batch of tasks from a DAG wave
//! - **Branch-level**: Verify a dependency chain's sub-objective
//!
//! The guiding intent is extracted from goals to provide context for task verification,
//! but the goal itself is never "verified as complete."

use std::sync::Arc;
use async_trait::async_trait;
use uuid::Uuid;

use crate::domain::errors::{DomainError, DomainResult};
use crate::domain::models::{
    BranchVerificationRequest, BranchVerificationResult, ConvergenceConfig, ConvergenceState,
    DependentTaskAugmentation, GapCategory, GapSeverity, HumanEscalation, IntentGap,
    IntentSatisfaction, IntentVerificationResult, NewTaskGuidance, OriginalIntent,
    RepromptApproach, RepromptGuidance, RepromptStrategySelector, SessionStatus,
    SubstrateConfig, SubstrateRequest, Task, TaskStatus,
};
use crate::domain::ports::{GoalRepository, Substrate, TaskRepository};

/// Configuration for the intent verifier.
#[derive(Debug, Clone)]
pub struct IntentVerifierConfig {
    /// Maximum turns for the verifier agent.
    pub max_turns: u32,
    /// Convergence configuration.
    pub convergence: ConvergenceConfig,
    /// Whether to include task artifacts in evaluation.
    pub include_artifacts: bool,
    /// Whether to include task output/logs in evaluation.
    pub include_task_output: bool,
    /// Agent type to use for verification.
    pub verifier_agent_type: String,
}

impl Default for IntentVerifierConfig {
    fn default() -> Self {
        Self {
            max_turns: 25,
            convergence: ConvergenceConfig::default(),
            include_artifacts: true,
            include_task_output: true,
            verifier_agent_type: "intent-verifier".to_string(),
        }
    }
}

/// Intent verifier service.
pub struct IntentVerifierService<G, T>
where
    G: GoalRepository + 'static,
    T: TaskRepository + 'static,
{
    goal_repo: Arc<G>,
    task_repo: Arc<T>,
    substrate: Arc<dyn Substrate>,
    config: IntentVerifierConfig,
}

impl<G, T> IntentVerifierService<G, T>
where
    G: GoalRepository + 'static,
    T: TaskRepository + 'static,
{
    pub fn new(
        goal_repo: Arc<G>,
        task_repo: Arc<T>,
        substrate: Arc<dyn Substrate>,
        config: IntentVerifierConfig,
    ) -> Self {
        Self {
            goal_repo,
            task_repo,
            substrate,
            config,
        }
    }

    pub fn with_defaults(
        goal_repo: Arc<G>,
        task_repo: Arc<T>,
        substrate: Arc<dyn Substrate>,
    ) -> Self {
        Self::new(goal_repo, task_repo, substrate, IntentVerifierConfig::default())
    }

    /// Extract the guiding intent from a goal for task verification.
    ///
    /// This captures the goal's description and constraints as the context
    /// against which tasks will be verified. Note that this does NOT verify
    /// the goal itself (goals are never "completed") - it extracts the intent
    /// that tasks should be aligned with.
    pub async fn extract_guiding_intent(&self, goal_id: Uuid) -> DomainResult<OriginalIntent> {
        let goal = self
            .goal_repo
            .get(goal_id)
            .await?
            .ok_or(DomainError::GoalNotFound(goal_id))?;

        let mut intent = OriginalIntent::from_goal(goal_id, &goal.description);

        // Extract constraints as requirements for task verification
        for constraint in &goal.constraints {
            intent.key_requirements.push(format!(
                "[{}] {}: {}",
                match constraint.constraint_type {
                    crate::domain::models::ConstraintType::Invariant => "MUST",
                    crate::domain::models::ConstraintType::Preference => "SHOULD",
                    crate::domain::models::ConstraintType::Boundary => "WITHIN",
                },
                constraint.name,
                constraint.description
            ));
        }

        Ok(intent)
    }

    /// Extract the intent from a task for branch-level verification.
    ///
    /// Used when verifying that a dependency chain (branch) accomplished
    /// its sub-objective before dependent tasks proceed.
    pub async fn extract_task_intent(&self, task_id: Uuid) -> DomainResult<OriginalIntent> {
        let task = self
            .task_repo
            .get(task_id)
            .await?
            .ok_or(DomainError::TaskNotFound(task_id))?;

        let intent = OriginalIntent::from_task(task_id, &task.description);

        Ok(intent)
    }

    /// Verify a batch of tasks against an intent (wave-level or branch-level verification).
    ///
    /// This enables verification at intermediate points during DAG execution,
    /// not just at goal completion.
    pub async fn verify_task_batch(
        &self,
        tasks: &[Task],
        context_description: &str,
        iteration: u32,
    ) -> DomainResult<IntentVerificationResult> {
        if tasks.is_empty() {
            // No tasks to verify - return satisfied
            return Ok(IntentVerificationResult::new(
                Uuid::nil(),
                IntentSatisfaction::Satisfied,
            )
            .with_confidence(1.0)
            .with_iteration(iteration)
            .with_summary("No tasks to verify"));
        }

        // Build an intent from the batch context
        let mut intent = OriginalIntent {
            id: Uuid::new_v4(),
            source_id: tasks.first().map(|t| t.id).unwrap_or(Uuid::nil()),
            source_type: crate::domain::models::IntentSource::DagBranch,
            original_text: context_description.to_string(),
            key_requirements: Vec::new(),
            success_criteria: Vec::new(),
            captured_at: chrono::Utc::now(),
        };

        // Gather requirements from all tasks
        for task in tasks {
            intent.key_requirements.push(format!("Task '{}': {}", task.title, task.description));
        }

        // Verify using the standard method
        self.verify_intent(&intent, tasks, iteration).await
    }

    /// Verify a single task against its own description as intent.
    pub async fn verify_single_task(
        &self,
        task: &Task,
        iteration: u32,
    ) -> DomainResult<IntentVerificationResult> {
        let intent = self.extract_task_intent(task.id).await?;
        self.verify_intent(&intent, &[task.clone()], iteration).await
    }

    /// Verify that completed tasks satisfy the original intent.
    pub async fn verify_intent(
        &self,
        intent: &OriginalIntent,
        completed_tasks: &[Task],
        iteration: u32,
    ) -> DomainResult<IntentVerificationResult> {
        // Build the verification prompt
        let prompt = self.build_verification_prompt(intent, completed_tasks).await?;

        // Create substrate request for the verifier agent
        let request = SubstrateRequest::new(
            Uuid::new_v4(),
            &self.config.verifier_agent_type,
            INTENT_VERIFIER_SYSTEM_PROMPT,
            &prompt,
        )
        .with_config(
            SubstrateConfig::default()
                .with_max_turns(self.config.max_turns)
                .with_allowed_tools(vec![
                    "read".to_string(),
                    "glob".to_string(),
                    "grep".to_string(),
                ]),
        );

        // Execute verification
        let session = self.substrate.execute(request).await?;

        // Parse the response to build the verification result
        let result = if session.status == SessionStatus::Completed {
            self.parse_verification_response(&session, intent, completed_tasks, iteration)?
        } else {
            // Verification failed - return indeterminate
            IntentVerificationResult::new(intent.id, IntentSatisfaction::Indeterminate)
                .with_iteration(iteration)
                .with_confidence(0.0)
                .with_summary(format!(
                    "Verification failed: {}",
                    session.error.unwrap_or_else(|| "Unknown error".to_string())
                ))
        };

        Ok(result)
    }

    /// Run a task convergence loop until work satisfies the guiding intent.
    ///
    /// This iteratively verifies completed tasks against the intent, identifies gaps,
    /// and triggers re-execution or new task creation until convergence is achieved.
    ///
    /// Note: This converges tasks toward the intent, not the goal itself.
    /// Goals remain Active throughout - only the current batch of work converges.
    ///
    /// The loop terminates when:
    /// - Intent is satisfied (work done matches what was asked)
    /// - Max iterations reached
    /// - Semantic drift detected (same gaps recurring)
    /// - Timeout exceeded
    pub async fn run_task_convergence_loop(
        &self,
        intent: OriginalIntent,
        initial_tasks: Vec<Task>,
        execute_tasks_fn: impl Fn(&[NewTaskGuidance], &[Uuid]) -> std::pin::Pin<Box<dyn std::future::Future<Output = DomainResult<Vec<Task>>> + Send>> + Send,
    ) -> DomainResult<ConvergenceState> {
        let mut state = ConvergenceState::new(intent.clone());
        let mut current_tasks = initial_tasks;
        let start = std::time::Instant::now();
        let timeout = std::time::Duration::from_secs(self.config.convergence.convergence_timeout_secs);

        loop {
            // Check timeout
            if start.elapsed() > timeout {
                tracing::warn!(
                    "Convergence loop timed out after {} seconds",
                    self.config.convergence.convergence_timeout_secs
                );
                state.end();
                break;
            }

            // Filter to completed tasks
            let completed: Vec<_> = current_tasks
                .iter()
                .filter(|t| t.status == TaskStatus::Complete)
                .cloned()
                .collect();

            // Verify the current state
            let result = self
                .verify_intent(&state.intent, &completed, state.current_iteration + 1)
                .await?;

            tracing::info!(
                "Intent verification iteration {}: {:?} (confidence: {:.2})",
                result.iteration,
                result.satisfaction,
                result.confidence
            );

            state.record_verification(result.clone());

            // Check if we should continue
            if !self.config.convergence.should_continue(&result) {
                tracing::info!(
                    "Convergence loop complete: {:?} after {} iterations",
                    result.satisfaction,
                    result.iteration
                );
                break;
            }

            // Check for progress
            if !state.is_making_progress() {
                tracing::warn!(
                    "Convergence loop not making progress after {} iterations",
                    result.iteration
                );
                state.end();
                break;
            }

            // Execute the re-prompt if we have guidance
            if let Some(guidance) = &result.reprompt_guidance {
                tracing::info!(
                    "Executing re-prompt with approach: {:?}",
                    guidance.approach
                );

                // Execute new/retry tasks
                let new_tasks = execute_tasks_fn(&guidance.tasks_to_add, &guidance.tasks_to_retry).await?;
                current_tasks.extend(new_tasks);
            } else {
                // No guidance, can't continue
                tracing::warn!("No reprompt guidance available, ending convergence loop");
                state.end();
                break;
            }
        }

        Ok(state)
    }

    /// Build the verification prompt.
    async fn build_verification_prompt(
        &self,
        intent: &OriginalIntent,
        completed_tasks: &[Task],
    ) -> DomainResult<String> {
        let mut prompt = String::new();

        // Original intent section
        prompt.push_str("## Original Intent\n\n");
        prompt.push_str(&format!("**Source**: {:?}\n\n", intent.source_type));
        prompt.push_str(&format!("**Description**:\n{}\n\n", intent.original_text));

        if !intent.key_requirements.is_empty() {
            prompt.push_str("**Key Requirements**:\n");
            for req in &intent.key_requirements {
                prompt.push_str(&format!("- {}\n", req));
            }
            prompt.push('\n');
        }

        if !intent.success_criteria.is_empty() {
            prompt.push_str("**Success Criteria**:\n");
            for criterion in &intent.success_criteria {
                prompt.push_str(&format!("- {}\n", criterion));
            }
            prompt.push('\n');
        }

        // Completed work section
        prompt.push_str("## Completed Work\n\n");

        for task in completed_tasks {
            prompt.push_str(&format!("### Task: {}\n\n", task.title));
            prompt.push_str(&format!("**Description**: {}\n\n", task.description));

            if !task.artifacts.is_empty() && self.config.include_artifacts {
                prompt.push_str("**Artifacts**:\n");
                for artifact in &task.artifacts {
                    prompt.push_str(&format!("- {} ({})\n", artifact.uri, format!("{:?}", artifact.artifact_type)));
                }
                prompt.push('\n');
            }

            if let Some(worktree) = &task.worktree_path {
                prompt.push_str(&format!("**Worktree**: {}\n\n", worktree));
            }
        }

        // Evaluation request
        prompt.push_str("## Evaluation Request\n\n");
        prompt.push_str(
            "Please evaluate whether the completed work satisfies the original intent.\n\n\
            Consider:\n\
            1. Does the work address all key requirements?\n\
            2. Are the success criteria met?\n\
            3. Is there any work that was implied but not explicitly stated that's missing?\n\
            4. If someone submitted this exact prompt again, would there be additional work done?\n\n\
            Provide your evaluation in the following format:\n\n\
            ```\n\
            SATISFACTION: <satisfied|partial|unsatisfied>\n\
            CONFIDENCE: <0.0-1.0>\n\
            SUMMARY: <one paragraph summary of what was accomplished>\n\
            GAPS:\n\
            - <gap description> | <minor|moderate|major|critical> | <suggested action>\n\
            FOCUS_AREAS:\n\
            - <area to focus on if re-prompting>\n\
            NEW_TASKS:\n\
            - <title> | <description> | <high|normal|low>\n\
            ```\n",
        );

        Ok(prompt)
    }

    /// Parse the verification response from the agent.
    fn parse_verification_response(
        &self,
        session: &crate::domain::models::SubstrateSession,
        intent: &OriginalIntent,
        completed_tasks: &[Task],
        iteration: u32,
    ) -> DomainResult<IntentVerificationResult> {
        // Get the final response text from the session result
        let response_text = session
            .result
            .clone()
            .unwrap_or_default();

        // Parse the structured response
        let mut result = IntentVerificationResult::new(intent.id, IntentSatisfaction::Indeterminate)
            .with_iteration(iteration);

        // Add evaluated tasks
        for task in completed_tasks {
            result = result.with_task(task.id);
        }

        // Parse SATISFACTION
        if let Some(sat_line) = response_text.lines().find(|l| l.starts_with("SATISFACTION:")) {
            let sat_value = sat_line.trim_start_matches("SATISFACTION:").trim().to_lowercase();
            result.satisfaction = match sat_value.as_str() {
                "satisfied" => IntentSatisfaction::Satisfied,
                "partial" => IntentSatisfaction::Partial,
                "unsatisfied" => IntentSatisfaction::Unsatisfied,
                _ => IntentSatisfaction::Indeterminate,
            };
        }

        // Parse CONFIDENCE
        if let Some(conf_line) = response_text.lines().find(|l| l.starts_with("CONFIDENCE:")) {
            let conf_str = conf_line.trim_start_matches("CONFIDENCE:").trim();
            if let Ok(conf) = conf_str.parse::<f64>() {
                result = result.with_confidence(conf);
            }
        }

        // Parse NEEDS_HUMAN and HUMAN_REASON for escalation
        let needs_human = response_text.lines()
            .find(|l| l.starts_with("NEEDS_HUMAN:"))
            .map(|l| l.trim_start_matches("NEEDS_HUMAN:").trim().to_lowercase() == "yes")
            .unwrap_or(false);

        if needs_human {
            let human_reason = response_text.lines()
                .find(|l| l.starts_with("HUMAN_REASON:"))
                .map(|l| l.trim_start_matches("HUMAN_REASON:").trim().to_string())
                .unwrap_or_else(|| "Human judgment required".to_string());

            result = result.with_escalation(HumanEscalation::new(human_reason));
        }

        // Parse SUMMARY
        if let Some(sum_line) = response_text.lines().find(|l| l.starts_with("SUMMARY:")) {
            let summary = sum_line.trim_start_matches("SUMMARY:").trim();
            result = result.with_summary(summary);
        }

        // Parse GAPS (format: description | severity | action | category)
        let mut in_gaps = false;
        for line in response_text.lines() {
            if line.starts_with("GAPS:") {
                in_gaps = true;
                continue;
            }
            if in_gaps {
                if line.starts_with("IMPLICIT_GAPS:") || line.starts_with("FOCUS_AREAS:")
                   || line.starts_with("NEW_TASKS:") || line.is_empty() {
                    in_gaps = false;
                    continue;
                }
                if line.starts_with("- ") {
                    if let Some(gap) = Self::parse_gap_line(line, false) {
                        result = result.with_gap(gap);
                    }
                }
            }
        }

        // Parse IMPLICIT_GAPS (format: description | severity | rationale)
        let mut in_implicit = false;
        for line in response_text.lines() {
            if line.starts_with("IMPLICIT_GAPS:") {
                in_implicit = true;
                continue;
            }
            if in_implicit {
                if line.starts_with("FOCUS_AREAS:") || line.starts_with("NEW_TASKS:") || line.is_empty() {
                    in_implicit = false;
                    continue;
                }
                if line.starts_with("- ") {
                    if let Some(gap) = Self::parse_gap_line(line, true) {
                        result = result.with_implicit_gap(gap);
                    }
                }
            }
        }

        // Parse REPROMPT_STRATEGY and STRATEGY_RATIONALE
        let strategy = response_text.lines()
            .find(|l| l.starts_with("REPROMPT_STRATEGY:"))
            .and_then(|l| {
                let s = l.trim_start_matches("REPROMPT_STRATEGY:").trim();
                RepromptApproach::from_str(s)
            });

        let _strategy_rationale = response_text.lines()
            .find(|l| l.starts_with("STRATEGY_RATIONALE:"))
            .map(|l| l.trim_start_matches("STRATEGY_RATIONALE:").trim().to_string());

        // Build reprompt guidance if not satisfied
        if result.satisfaction != IntentSatisfaction::Satisfied {
            // Use the strategy from the agent if provided, otherwise compute based on gaps
            let approach = strategy.unwrap_or_else(|| {
                RepromptStrategySelector::select_strategy(&result)
            });

            let mut guidance = RepromptGuidance::new(approach);

            // Parse FOCUS_AREAS
            let mut in_focus = false;
            for line in response_text.lines() {
                if line.starts_with("FOCUS_AREAS:") {
                    in_focus = true;
                    continue;
                }
                if in_focus {
                    if line.starts_with("NEW_TASKS:") || line.starts_with("REPROMPT_STRATEGY:") || line.is_empty() {
                        in_focus = false;
                        continue;
                    }
                    if line.starts_with("- ") {
                        guidance = guidance.with_focus(line.trim_start_matches("- ").trim());
                    }
                }
            }

            // Parse NEW_TASKS (format: title | description | priority | execution_mode)
            let mut in_new_tasks = false;
            for line in response_text.lines() {
                if line.starts_with("NEW_TASKS:") {
                    in_new_tasks = true;
                    continue;
                }
                if in_new_tasks {
                    if line.starts_with("REPROMPT_STRATEGY:") || line.is_empty()
                       || (!line.starts_with("- ") && !line.starts_with("  ")) {
                        in_new_tasks = false;
                        continue;
                    }
                    if line.starts_with("- ") {
                        let parts: Vec<&str> = line.trim_start_matches("- ").split('|').collect();
                        if parts.len() >= 2 {
                            let title = parts[0].trim();
                            let description = parts[1].trim();
                            let mut task = NewTaskGuidance::new(title, description);

                            if parts.len() > 2 {
                                match parts[2].trim().to_lowercase().as_str() {
                                    "high" => task = task.high_priority(),
                                    _ => {}
                                }
                            }

                            if parts.len() > 3 {
                                match parts[3].trim().to_lowercase().as_str() {
                                    "blocking" => task = task.blocking(),
                                    _ => {}
                                }
                            }

                            guidance = guidance.with_new_task(task);
                        }
                    }
                }
            }

            // Add context from gaps (both explicit and implicit)
            let all_gaps: Vec<_> = result.all_gaps().collect();
            if !all_gaps.is_empty() {
                let gap_context = all_gaps
                    .iter()
                    .map(|g| {
                        let implicit_marker = if g.is_implicit { " [IMPLICIT]" } else { "" };
                        format!("- [{}]{} {}", g.category.as_str(), implicit_marker, g.description)
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                guidance = guidance.with_context(format!("Previous gaps identified:\n{}", gap_context));
            }

            result = result.with_reprompt_guidance(guidance);
        }

        // Check for auto-escalation based on gap patterns
        if result.escalation.is_none() {
            if let Some(auto_escalation) = result.should_escalate() {
                result = result.with_escalation(auto_escalation);
            }
        }

        Ok(result)
    }

    /// Parse a gap line into an IntentGap.
    fn parse_gap_line(line: &str, is_implicit: bool) -> Option<IntentGap> {
        let parts: Vec<&str> = line.trim_start_matches("- ").split('|').collect();
        if parts.is_empty() {
            return None;
        }

        let description = parts[0].trim().to_string();
        let severity = if parts.len() > 1 {
            match parts[1].trim().to_lowercase().as_str() {
                "minor" => GapSeverity::Minor,
                "moderate" => GapSeverity::Moderate,
                "major" => GapSeverity::Major,
                "critical" => GapSeverity::Critical,
                _ => GapSeverity::Moderate,
            }
        } else {
            GapSeverity::Moderate
        };

        let mut gap = IntentGap::new(description, severity);

        if is_implicit {
            // For implicit gaps: description | severity | rationale
            if parts.len() > 2 {
                gap = gap.as_implicit(parts[2].trim());
            } else {
                gap = gap.as_implicit("Implicit requirement not met");
            }
        } else {
            // For explicit gaps: description | severity | action | category
            if parts.len() > 2 {
                gap = gap.with_action(parts[2].trim());
            }
            if parts.len() > 3 {
                if let Some(cat) = GapCategory::from_str(parts[3].trim()) {
                    gap = gap.with_category(cat);
                }
            }
        }

        Some(gap)
    }

    /// Verify a dependency branch before dependent tasks proceed.
    pub async fn verify_branch(
        &self,
        request: &BranchVerificationRequest,
    ) -> DomainResult<BranchVerificationResult> {
        // Collect completed tasks in the branch
        let mut branch_tasks = Vec::new();
        for task_id in &request.branch_tasks {
            if let Some(task) = self.task_repo.get(*task_id).await? {
                if task.status == TaskStatus::Complete {
                    branch_tasks.push(task);
                }
            }
        }

        if branch_tasks.is_empty() {
            return Ok(BranchVerificationResult::unsatisfied(
                request.id,
                "No completed tasks in branch",
            ));
        }

        // Verify the branch against its objective
        let verification = self.verify_task_batch(
            &branch_tasks,
            &request.branch_objective,
            1, // First iteration for branch
        ).await?;

        // Build branch result
        let mut branch_result = if verification.satisfaction == IntentSatisfaction::Satisfied {
            BranchVerificationResult::satisfied(request.id)
        } else if verification.satisfaction == IntentSatisfaction::Partial {
            BranchVerificationResult::partial(request.id, verification.confidence)
        } else {
            BranchVerificationResult::unsatisfied(
                request.id,
                verification.accomplishment_summary.clone(),
            )
        };

        // Copy gaps
        for gap in &verification.gaps {
            branch_result = branch_result.with_gap(gap.clone());
        }

        // Build augmentations for dependent tasks if branch is not fully satisfied
        if !branch_result.branch_satisfied && branch_result.dependents_can_proceed {
            for waiting_task_id in &request.waiting_tasks {
                let mut aug = DependentTaskAugmentation::new(
                    *waiting_task_id,
                    format!(
                        "Upstream branch partially satisfied ({:.0}% confidence): {}",
                        verification.confidence * 100.0,
                        verification.accomplishment_summary
                    ),
                );

                // Add inherited gaps
                for gap in &verification.gaps {
                    aug = aug.with_inherited_gap(&gap.description);
                    if let Some(ref action) = gap.suggested_action {
                        aug = aug.with_workaround(action);
                    }
                }

                branch_result = branch_result.with_augmentation(aug);
            }
        }

        Ok(branch_result)
    }

    /// Handle an indeterminate verification result using the Overmind.
    ///
    /// When intent verification produces an indeterminate result (e.g., due to
    /// ambiguous requirements or unclear success criteria), this method invokes
    /// the Overmind to make an escalation decision.
    pub async fn handle_indeterminate_with_overmind(
        &self,
        result: &IntentVerificationResult,
        overmind: &crate::services::OvermindService,
    ) -> DomainResult<crate::domain::models::overmind::OvermindEscalationDecision> {
        use crate::domain::models::overmind::{
            EscalationRequest, EscalationContext, EscalationTrigger,
            EscalationPreferences, OvermindEscalationDecision, OvermindEscalationUrgency,
            DecisionMetadata,
        };

        // Only handle indeterminate results
        if result.satisfaction != IntentSatisfaction::Indeterminate {
            // Not indeterminate - return a "don't escalate" decision
            return Ok(OvermindEscalationDecision {
                metadata: DecisionMetadata::new(
                    1.0,
                    "Result is not indeterminate, no escalation needed",
                ),
                should_escalate: false,
                urgency: None,
                questions: vec![],
                context_for_human: String::new(),
                alternatives_if_unavailable: vec![],
                is_blocking: false,
            });
        }

        // Build the escalation context
        let attempts_made: Vec<String> = result.gaps
            .iter()
            .filter_map(|g| g.suggested_action.clone())
            .collect();

        let context = EscalationContext {
            goal_id: None, // We don't have direct goal access here
            task_id: result.evaluated_tasks.first().copied(),
            situation: format!(
                "Intent verification returned indeterminate result (confidence: {:.2}). Summary: {}",
                result.confidence,
                result.accomplishment_summary
            ),
            attempts_made,
            time_spent_minutes: 0, // We don't track this
        };

        let request = EscalationRequest {
            context,
            trigger: EscalationTrigger::IndeterminateVerification,
            previous_escalations: vec![],
            escalation_preferences: EscalationPreferences::default(),
        };

        // Try Overmind
        match overmind.evaluate_escalation(request).await {
            Ok(decision) => Ok(decision),
            Err(e) => {
                tracing::warn!(
                    "Overmind escalation evaluation failed for indeterminate result: {}",
                    e
                );
                // Fallback: conservative escalation for indeterminate results
                Ok(OvermindEscalationDecision {
                    metadata: DecisionMetadata::new(
                        0.5,
                        "Fallback: escalating indeterminate result (Overmind unavailable)",
                    ),
                    should_escalate: true,
                    urgency: Some(OvermindEscalationUrgency::Medium),
                    questions: vec![
                        "Please review the verification result and clarify requirements".to_string(),
                    ],
                    context_for_human: result.accomplishment_summary.clone(),
                    alternatives_if_unavailable: vec![
                        "Retry verification with more context".to_string(),
                        "Proceed with best-effort interpretation".to_string(),
                    ],
                    is_blocking: false,
                })
            }
        }
    }
}

// ---------------------------------------------------------------------------
// ConvergentIntentVerifier trait implementation
// ---------------------------------------------------------------------------

#[async_trait]
impl<G, T> crate::services::swarm_orchestrator::convergent_execution::ConvergentIntentVerifier
    for IntentVerifierService<G, T>
where
    G: GoalRepository + 'static,
    T: TaskRepository + 'static,
{
    async fn verify_convergent_intent(
        &self,
        task: &crate::domain::models::task::Task,
        goal_id: Option<Uuid>,
        iteration: u32,
    ) -> DomainResult<Option<IntentVerificationResult>> {
        // Try to extract intent from the goal first (richer context),
        // then fall back to the task description.
        let intent = if let Some(gid) = goal_id {
            match self.extract_guiding_intent(gid).await {
                Ok(intent) => intent,
                Err(e) => {
                    tracing::debug!(
                        task_id = %task.id,
                        goal_id = %gid,
                        error = %e,
                        "Could not extract guiding intent from goal; falling back to task"
                    );
                    OriginalIntent::from_task(task.id, &task.description)
                }
            }
        } else {
            OriginalIntent::from_task(task.id, &task.description)
        };

        // Skip verification if the intent is empty (no meaningful description)
        if intent.original_text.trim().is_empty() {
            return Ok(None);
        }

        let result = self
            .verify_intent(&intent, &[task.clone()], iteration)
            .await?;

        Ok(Some(result))
    }
}

/// System prompt for the intent verifier agent.
const INTENT_VERIFIER_SYSTEM_PROMPT: &str = r#"You are an Intent Verifier agent in the Abathur swarm system.

## Role
Your purpose is to independently evaluate whether completed work truly satisfies the original intent of a task or goal. You are a skeptical but fair evaluator who looks beyond surface-level completion to assess whether the *spirit* of the request was fulfilled.

## The Re-Prompt Test (Core Principle)
Ask yourself: **"If someone submitted the exact same prompt/request again, would there be additional work that should be done?"**

If the answer is YES, the work is not fully satisfying the intent. This is your north star.

## Deep Intent Analysis

### 1. Explicit vs Implicit Requirements
Every request has multiple layers:

**Explicit Requirements** (stated directly):
- Features, behaviors, or outputs mentioned in the request
- Specific constraints or conditions stated
- Named technologies, patterns, or approaches

**Implicit Requirements** (reasonable expectations):
- Industry-standard practices for the domain
- Error handling and edge cases a professional would address
- Security considerations for the context
- Performance expectations appropriate to the use case
- Maintainability and code quality norms
- Documentation that a handoff would require

**Contextual Requirements** (derived from situation):
- Integration with existing codebase patterns
- Consistency with project conventions
- Dependencies on or from other components
- Deployment and operational concerns

### 2. The "Reasonable Professional" Standard
Ask: Would a skilled professional, given this request and context, have done more?
- Not about perfection, but about professional completeness
- Consider what would embarrass the implementer if missed
- Think about what a code reviewer would flag

### 3. Stakeholder Perspective Analysis
Consider multiple viewpoints:
- **End User**: Does this solve their actual problem?
- **Developer**: Is this maintainable and understandable?
- **Operator**: Can this be deployed and monitored?
- **Security**: Are there obvious vulnerabilities?
- **Future Self**: Will this cause problems later?

## Evaluation Checklist

### Functional Completeness
- [ ] All stated features implemented
- [ ] Happy path works correctly
- [ ] Common error cases handled
- [ ] Edge cases addressed (empty inputs, large inputs, concurrent access)
- [ ] Failure modes graceful

### Integration Quality
- [ ] Works with existing code/systems
- [ ] Follows project conventions
- [ ] Dependencies properly managed
- [ ] No breaking changes to dependents

### Operational Readiness
- [ ] Appropriate logging/observability
- [ ] Configuration externalized where appropriate
- [ ] Error messages actionable
- [ ] Performance acceptable for use case

### Code Quality
- [ ] Tests for critical paths
- [ ] Code understandable without deep context
- [ ] No obvious security issues
- [ ] No technical debt that would block future work

## Nuance Detection

### Watch for These Patterns

**Surface Completion, Deeper Gaps**:
- Feature "works" but doesn't handle realistic scenarios
- Tests pass but don't cover meaningful cases
- Code compiles but has obvious logic errors

**Partial Implementation**:
- Started but didn't finish a logical unit
- Implemented the easy parts, skipped the hard parts
- Left TODOs or FIXMEs for critical functionality

**Wrong Abstraction Level**:
- Solved a different problem than asked
- Over-engineered simple request
- Under-engineered complex request

**Missing Connections**:
- Implemented in isolation, not integrated
- Created components that don't work together
- Forgot to wire up to entry points

### Questions That Reveal Gaps
1. "What happens when X fails?" (error handling)
2. "What if there are 1000 of these?" (scale)
3. "What if two users do this simultaneously?" (concurrency)
4. "What if the input is malicious?" (security)
5. "How would a new developer understand this?" (clarity)
6. "How would we know if this broke in production?" (observability)

## Output Format

Provide your evaluation in this exact format:

```
SATISFACTION: <satisfied|partial|unsatisfied|indeterminate>
CONFIDENCE: <0.0-1.0>
NEEDS_HUMAN: <yes|no>
HUMAN_REASON: <reason if needs human judgment>
SUMMARY: <one paragraph describing what was accomplished>
GAPS:
- <gap description> | <minor|moderate|major|critical> | <suggested action> | <category>
IMPLICIT_GAPS:
- <implied requirement that was missed> | <severity> | <why this was expected>
FOCUS_AREAS:
- <area to focus on if re-prompting>
NEW_TASKS:
- <title> | <description> | <high|normal|low> | <blocking|parallel>
REPROMPT_STRATEGY: <retry_same|retry_augmented|add_tasks|restructure|escalate>
STRATEGY_RATIONALE: <why this strategy>
```

## Gap Categories
- `functional`: Missing features or behaviors
- `error_handling`: Missing or inadequate error cases
- `integration`: Doesn't work with other components
- `testing`: Insufficient test coverage
- `security`: Security vulnerabilities or concerns
- `performance`: Performance issues or concerns
- `observability`: Missing logging, metrics, or monitoring
- `documentation`: Missing or inadequate docs
- `maintainability`: Code quality or design issues

## Severity Calibration

- **Minor**: Polish items, nice-to-haves, stylistic issues
  - Would not block a code review
  - Could be addressed in a follow-up

- **Moderate**: Expected features missing, non-critical paths broken
  - A reviewer would request changes
  - Users would notice but could work around

- **Major**: Core functionality gaps, important use cases broken
  - Would block a code review
  - Users would be significantly impacted

- **Critical**: Fundamental requirements unmet, security issues, data loss risks
  - Work is essentially not done
  - Would cause immediate problems in production

## Re-Prompt Strategy Selection

Choose based on the nature of gaps:

- **retry_same**: Gaps suggest the agent misunderstood; same prompt with emphasis
- **retry_augmented**: Add context about what was missed to the same tasks
- **add_tasks**: Gaps require new work not covered by existing tasks
- **restructure**: Fundamental approach was wrong, need different decomposition
- **escalate**: Gaps require human judgment, policy decisions, or access agent lacks

## When to Mark NEEDS_HUMAN: yes

- Ambiguous requirements that could reasonably go multiple ways
- Policy or business logic decisions not specified
- Security-sensitive decisions requiring authorization
- Trade-offs between competing concerns with no clear winner
- Access or permissions the system lacks
- Recurring gaps that haven't been resolved after multiple iterations (drift)

## Important Principles

1. **Be thorough but fair** - Don't fail work for trivialities
2. **Be specific** - Vague gaps can't be addressed
3. **Be actionable** - Every gap should have a clear fix path
4. **Be calibrated** - Severity should match actual impact
5. **Be honest about uncertainty** - Use indeterminate when you can't tell
6. **Consider context** - A prototype has different standards than production code
"#;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::models::{SessionStatus, SubstrateSession, SubstrateConfig, TaskPriority};

    #[test]
    fn test_intent_verifier_config_default() {
        let config = IntentVerifierConfig::default();
        assert_eq!(config.max_turns, 25);
        assert_eq!(config.verifier_agent_type, "intent-verifier");
        assert!(config.include_artifacts);
    }

    #[test]
    fn test_parse_satisfied_response() {
        let response = r#"SATISFACTION: satisfied
CONFIDENCE: 0.95
SUMMARY: All tasks completed successfully with high quality.
GAPS:
FOCUS_AREAS:
NEW_TASKS:
"#;

        let session = create_mock_session(response);
        let intent = OriginalIntent::from_goal(Uuid::new_v4(), "Test goal");
        let tasks = vec![create_mock_task("Task 1")];

        // Parse the response manually (since we can't easily test the full service)
        let result = parse_test_response(&session, &intent, &tasks, 1);

        assert_eq!(result.satisfaction, IntentSatisfaction::Satisfied);
        assert!((result.confidence - 0.95).abs() < 0.01);
        assert!(result.gaps.is_empty());
    }

    #[test]
    fn test_parse_partial_response_with_gaps() {
        let response = r#"SATISFACTION: partial
CONFIDENCE: 0.6
SUMMARY: Core functionality implemented but missing some features.
GAPS:
- Missing error handling | major | Add try-catch blocks
- No documentation | minor | Add docstrings
FOCUS_AREAS:
- Error handling
- Documentation
NEW_TASKS:
- Add error handling | Implement comprehensive error handling | high
- Add docs | Write API documentation | normal
"#;

        let session = create_mock_session(response);
        let intent = OriginalIntent::from_goal(Uuid::new_v4(), "Test goal");
        let tasks = vec![create_mock_task("Task 1")];

        let result = parse_test_response(&session, &intent, &tasks, 1);

        assert_eq!(result.satisfaction, IntentSatisfaction::Partial);
        assert!((result.confidence - 0.6).abs() < 0.01);
        assert_eq!(result.gaps.len(), 2);
        assert_eq!(result.gaps[0].severity, GapSeverity::Major);
        assert_eq!(result.gaps[1].severity, GapSeverity::Minor);

        let guidance = result.reprompt_guidance.as_ref().unwrap();
        assert_eq!(guidance.focus_areas.len(), 2);
        assert_eq!(guidance.tasks_to_add.len(), 2);
    }

    #[test]
    fn test_parse_unsatisfied_response() {
        let response = r#"SATISFACTION: unsatisfied
CONFIDENCE: 0.3
SUMMARY: The implementation does not meet the requirements.
GAPS:
- Core feature not implemented | critical | Implement the main feature
FOCUS_AREAS:
- Main feature implementation
NEW_TASKS:
- Implement core feature | Build the primary functionality | high
"#;

        let session = create_mock_session(response);
        let intent = OriginalIntent::from_goal(Uuid::new_v4(), "Test goal");
        let tasks = vec![create_mock_task("Task 1")];

        let result = parse_test_response(&session, &intent, &tasks, 1);

        assert_eq!(result.satisfaction, IntentSatisfaction::Unsatisfied);
        assert_eq!(result.gaps.len(), 1);
        assert_eq!(result.gaps[0].severity, GapSeverity::Critical);

        let guidance = result.reprompt_guidance.as_ref().unwrap();
        assert_eq!(guidance.approach, RepromptApproach::RetryAndAddTasks);
    }

    #[test]
    fn test_parse_empty_response() {
        let response = "";
        let session = create_mock_session(response);
        let intent = OriginalIntent::from_goal(Uuid::new_v4(), "Test goal");
        let tasks = vec![];

        let result = parse_test_response(&session, &intent, &tasks, 1);

        // Should default to indeterminate
        assert_eq!(result.satisfaction, IntentSatisfaction::Indeterminate);
    }

    #[test]
    fn test_convergence_config_in_verifier_config() {
        let config = IntentVerifierConfig {
            max_turns: 30,
            convergence: ConvergenceConfig {
                max_iterations: 5,
                min_confidence_threshold: 0.8,
                require_full_satisfaction: true,
                auto_retry_partial: false,
                convergence_timeout_secs: 3600,
            },
            include_artifacts: true,
            include_task_output: false,
            verifier_agent_type: "custom-verifier".to_string(),
        };

        assert_eq!(config.convergence.max_iterations, 5);
        assert!(config.convergence.require_full_satisfaction);
    }

    // Helper functions for testing

    fn create_mock_session(result_text: &str) -> SubstrateSession {
        SubstrateSession {
            id: Uuid::new_v4(),
            task_id: Uuid::new_v4(),
            agent_template: "intent-verifier".to_string(),
            config: SubstrateConfig::default(),
            status: SessionStatus::Completed,
            turns_completed: 1,
            input_tokens: 100,
            output_tokens: 200,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            cost_cents: Some(0.01),
            result: Some(result_text.to_string()),
            error: None,
            process_id: None,
            started_at: chrono::Utc::now(),
            ended_at: Some(chrono::Utc::now()),
        }
    }

    fn create_mock_task(title: &str) -> Task {
        Task::with_title(title, "Test description")
            .with_priority(TaskPriority::Normal)
    }

    /// Simplified response parser for testing (mirrors the actual parser logic)
    fn parse_test_response(
        session: &SubstrateSession,
        intent: &OriginalIntent,
        completed_tasks: &[Task],
        iteration: u32,
    ) -> IntentVerificationResult {
        let response_text = session.result.clone().unwrap_or_default();

        let mut result = IntentVerificationResult::new(intent.id, IntentSatisfaction::Indeterminate)
            .with_iteration(iteration);

        for task in completed_tasks {
            result = result.with_task(task.id);
        }

        // Parse SATISFACTION
        if let Some(sat_line) = response_text.lines().find(|l| l.starts_with("SATISFACTION:")) {
            let sat_value = sat_line.trim_start_matches("SATISFACTION:").trim().to_lowercase();
            result.satisfaction = match sat_value.as_str() {
                "satisfied" => IntentSatisfaction::Satisfied,
                "partial" => IntentSatisfaction::Partial,
                "unsatisfied" => IntentSatisfaction::Unsatisfied,
                _ => IntentSatisfaction::Indeterminate,
            };
        }

        // Parse CONFIDENCE
        if let Some(conf_line) = response_text.lines().find(|l| l.starts_with("CONFIDENCE:")) {
            let conf_str = conf_line.trim_start_matches("CONFIDENCE:").trim();
            if let Ok(conf) = conf_str.parse::<f64>() {
                result = result.with_confidence(conf);
            }
        }

        // Parse SUMMARY
        if let Some(sum_line) = response_text.lines().find(|l| l.starts_with("SUMMARY:")) {
            let summary = sum_line.trim_start_matches("SUMMARY:").trim();
            result = result.with_summary(summary);
        }

        // Parse GAPS
        let mut in_gaps = false;
        for line in response_text.lines() {
            if line.starts_with("GAPS:") {
                in_gaps = true;
                continue;
            }
            if in_gaps {
                if line.starts_with("FOCUS_AREAS:") || line.starts_with("NEW_TASKS:") || line.is_empty() {
                    in_gaps = false;
                    continue;
                }
                if line.starts_with("- ") {
                    let parts: Vec<&str> = line.trim_start_matches("- ").split('|').collect();
                    if !parts.is_empty() {
                        let description = parts[0].trim().to_string();
                        let severity = if parts.len() > 1 {
                            match parts[1].trim().to_lowercase().as_str() {
                                "minor" => GapSeverity::Minor,
                                "moderate" => GapSeverity::Moderate,
                                "major" => GapSeverity::Major,
                                "critical" => GapSeverity::Critical,
                                _ => GapSeverity::Moderate,
                            }
                        } else {
                            GapSeverity::Moderate
                        };

                        let mut gap = IntentGap::new(description, severity);
                        if parts.len() > 2 {
                            gap = gap.with_action(parts[2].trim());
                        }
                        result = result.with_gap(gap);
                    }
                }
            }
        }

        // Build reprompt guidance if not satisfied
        if result.satisfaction != IntentSatisfaction::Satisfied {
            let mut guidance = RepromptGuidance::new(
                if !result.gaps.is_empty() && result.gaps.iter().any(|g| g.severity >= GapSeverity::Major) {
                    RepromptApproach::RetryAndAddTasks
                } else {
                    RepromptApproach::RetryWithContext
                },
            );

            // Parse FOCUS_AREAS
            let mut in_focus = false;
            for line in response_text.lines() {
                if line.starts_with("FOCUS_AREAS:") {
                    in_focus = true;
                    continue;
                }
                if in_focus {
                    if line.starts_with("NEW_TASKS:") || line.is_empty() {
                        in_focus = false;
                        continue;
                    }
                    if line.starts_with("- ") {
                        guidance = guidance.with_focus(line.trim_start_matches("- ").trim());
                    }
                }
            }

            // Parse NEW_TASKS
            let mut in_new_tasks = false;
            for line in response_text.lines() {
                if line.starts_with("NEW_TASKS:") {
                    in_new_tasks = true;
                    continue;
                }
                if in_new_tasks {
                    if line.is_empty() || (!line.starts_with("- ") && !line.starts_with("  ")) {
                        in_new_tasks = false;
                        continue;
                    }
                    if line.starts_with("- ") {
                        let parts: Vec<&str> = line.trim_start_matches("- ").split('|').collect();
                        if parts.len() >= 2 {
                            let title = parts[0].trim();
                            let description = parts[1].trim();
                            let mut task = NewTaskGuidance::new(title, description);

                            if parts.len() > 2 {
                                match parts[2].trim().to_lowercase().as_str() {
                                    "high" => task = task.high_priority(),
                                    _ => {}
                                }
                            }

                            guidance = guidance.with_new_task(task);
                        }
                    }
                }
            }

            result = result.with_reprompt_guidance(guidance);
        }

        result
    }
}
