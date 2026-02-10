//! Bridge between the task system and the convergence engine.
//!
//! Provides conversion functions that translate task-domain objects into
//! convergence-domain objects, and vice versa. This module implements
//! Parts 1.4 and 2.1-2.2 of the convergence-task integration spec.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use uuid::Uuid;

use crate::domain::errors::DomainResult;
use crate::domain::models::convergence::{
    ArtifactReference, AttractorType, ConvergenceEngineConfig, ConvergencePolicy,
    OverseerSignals, PriorityHint, Reference, ReferenceType, StrategyEntry,
    StrategyKind, TaskSubmission, Trajectory,
};
use crate::domain::models::task::{Complexity, ExecutionMode, Task, TaskPriority};
use crate::domain::ports::{StrategyStats, TrajectoryRepository};
use crate::services::swarm_orchestrator::types::SwarmConfig;

/// Convert a Task into a TaskSubmission for the convergence engine.
///
/// This is a lossy conversion -- Task has fields TaskSubmission doesn't care about
/// (dependencies, priority, parent), and TaskSubmission has fields Task doesn't
/// have (discovered infrastructure, anti-patterns). The bridge fills the gap.
pub fn task_to_submission(task: &Task, goal_id: Option<Uuid>) -> TaskSubmission {
    let mut submission = TaskSubmission::new(task.description.clone());

    // Propagate goal linkage for memory queries and event correlation
    submission.goal_id = goal_id;

    // Map complexity directly -- both systems use the same enum
    submission.inferred_complexity = task.routing_hints.complexity;

    // Map parallel samples from execution mode
    if let ExecutionMode::Convergent { parallel_samples } = &task.execution_mode {
        submission.parallel_samples = *parallel_samples;
    }

    // Extract constraints and anti-patterns from task context hints
    for hint in &task.context.hints {
        if let Some(constraint) = hint.strip_prefix("constraint:") {
            submission = submission.with_constraint(constraint.trim().to_string());
        }
        if let Some(anti_pattern) = hint.strip_prefix("anti-pattern:") {
            submission = submission.with_anti_pattern(anti_pattern.trim().to_string());
        }
    }

    // Map relevant files to references
    for file in &task.context.relevant_files {
        submission = submission.with_reference(Reference {
            path: file.clone(),
            reference_type: ReferenceType::CodeFile,
            description: None,
        });
    }

    // Map task priority to convergence priority hint
    match task.priority {
        TaskPriority::Critical | TaskPriority::High => {
            submission = submission.with_priority_hint(PriorityHint::Thorough);
        }
        TaskPriority::Low => {
            submission = submission.with_priority_hint(PriorityHint::Fast);
        }
        _ => {} // Normal -- no hint, use defaults
    }

    submission
}

/// Collect an artifact reference from the task's worktree after a substrate invocation.
///
/// In the worktree model, the artifact is the state of the worktree after the agent runs.
/// Overseers run commands (cargo build, cargo test, etc.) against this path.
pub fn collect_artifact(
    worktree_path: &str,
    content_hash: &str,
) -> ArtifactReference {
    ArtifactReference::new(worktree_path, content_hash)
}

/// Build a convergent prompt for a specific strategy and trajectory state.
///
/// Each iteration of a convergent task builds a specialized prompt that includes
/// convergence context the agent doesn't get in direct mode.
pub fn build_convergent_prompt(
    task: &Task,
    trajectory: &Trajectory,
    strategy: &StrategyKind,
) -> String {
    let mut sections = vec![];

    // Use the effective specification, NOT the raw task description.
    sections.push(trajectory.specification.effective.content.clone());

    // Strategy-specific instructions
    match strategy {
        StrategyKind::RetryWithFeedback => {
            if let Some(obs) = trajectory.observations.last() {
                let summary = format_overseer_summary(&obs.overseer_signals);
                if !summary.is_empty() {
                    sections.push(format!(
                        "Previous attempt feedback:\n{}",
                        summary
                    ));
                }
            }
        }
        StrategyKind::FocusedRepair => {
            let gaps = persistent_gaps(trajectory);
            if !gaps.is_empty() {
                sections.push(format!(
                    "Focus on fixing these specific issues:\n{}",
                    gaps.join("\n")
                ));
            }
        }
        StrategyKind::FreshStart { carry_forward } => {
            sections.push(format!(
                "Start fresh. Key learnings from previous attempts:\n{}\n\nRemaining gaps:\n{}",
                carry_forward.failure_summary,
                carry_forward.remaining_gaps.iter()
                    .map(|g| format!("- {}", g.description))
                    .collect::<Vec<_>>()
                    .join("\n")
            ));
        }
        StrategyKind::IncrementalRefinement => {
            sections.push(
                "The current implementation is partially correct. \
                 Make minimal, targeted changes to address remaining failures \
                 without breaking what already works.".to_string()
            );
        }
        StrategyKind::Reframe => {
            sections.push(
                "Reconsider the approach from scratch. The previous approach \
                 has diverged from the goal. Think about the problem differently.".to_string()
            );
        }
        StrategyKind::AlternativeApproach => {
            let tried: Vec<String> = trajectory.strategy_log.iter()
                .map(|e| format!("- {}", e.strategy_kind.kind_name()))
                .collect();
            if !tried.is_empty() {
                sections.push(format!(
                    "Previous approaches that did not converge:\n{}\n\n\
                     Try a fundamentally different approach.",
                    tried.join("\n")
                ));
            }
        }
        StrategyKind::Decompose => {
            sections.push(
                "This task is too complex to solve in one pass. Break it into \
                 smaller, independently verifiable subtasks.".to_string()
            );
        }
        StrategyKind::ArchitectReview => {
            sections.push(
                "Review the overall architecture and identify structural issues \
                 preventing convergence. Suggest a revised approach.".to_string()
            );
        }
        StrategyKind::RevertAndBranch { .. } => {
            sections.push(
                "The current changes have accumulated regressions. Revert to \
                 the last known-good state and try a different approach.".to_string()
            );
        }
        StrategyKind::RetryAugmented => {
            sections.push(
                "Retry with additional context. Pay close attention to the \
                 failing tests and build errors below.".to_string()
            );
        }
    }

    // Acceptance criteria from overseers (latest observation)
    if let Some(obs) = trajectory.observations.last() {
        if let Some(ref test_results) = obs.overseer_signals.test_results {
            if !test_results.failing_test_names.is_empty() {
                sections.push(format!(
                    "Failing tests that must pass:\n{}",
                    test_results.failing_test_names.iter()
                        .map(|f| format!("- {}", f))
                        .collect::<Vec<_>>()
                        .join("\n")
                ));
            }
        }
        if let Some(ref build_result) = obs.overseer_signals.build_result {
            if !build_result.success {
                sections.push(format!(
                    "Build errors that must be fixed:\n{}",
                    build_result.errors.join("\n")
                ));
            }
        }
    }

    // Suppress the unused variable warning for `task` -- it is accepted for
    // future enrichment (e.g. injecting task-level context hints).
    let _ = task;

    sections.join("\n\n---\n\n")
}

/// Build a ConvergenceEngineConfig from SwarmConfig.
///
/// Bridges the existing SwarmConfig and ConvergenceLoopConfig to the engine's
/// expected configuration.
pub fn build_engine_config(config: &SwarmConfig) -> ConvergenceEngineConfig {
    let loop_config = &config.convergence;

    let mut policy = ConvergencePolicy::default();
    policy.acceptance_threshold = loop_config.min_confidence_threshold;
    policy.partial_acceptance = loop_config.auto_retry_partial;

    ConvergenceEngineConfig {
        default_policy: policy,
        max_parallel_trajectories: 3,
        enable_proactive_decomposition: true,
        memory_enabled: config.polling.task_learning_enabled,
        event_emission_enabled: true,
    }
}

/// Build a default ConvergenceEngineConfig without a SwarmConfig reference.
///
/// Used as a fallback when the orchestrator's `convergence_engine_config` is
/// not explicitly set. Produces reasonable defaults matching the production
/// configuration shape.
pub fn build_engine_config_from_defaults() -> ConvergenceEngineConfig {
    ConvergenceEngineConfig {
        default_policy: ConvergencePolicy::default(),
        max_parallel_trajectories: 3,
        enable_proactive_decomposition: true,
        memory_enabled: true,
        event_emission_enabled: true,
    }
}

// ---------------------------------------------------------------------------
// DynTrajectoryRepository -- Sized wrapper for trait objects
// ---------------------------------------------------------------------------

/// A sized wrapper around `Arc<dyn TrajectoryRepository>` that implements
/// `TrajectoryRepository`.
///
/// The convergence engine requires its generic parameter `T: TrajectoryRepository`
/// to be `Sized`, which precludes passing `dyn TrajectoryRepository` directly.
/// This newtype bridges the gap by delegating every method to the inner trait
/// object while itself being `Sized`.
pub struct DynTrajectoryRepository(pub Arc<dyn TrajectoryRepository>);

#[async_trait]
impl TrajectoryRepository for DynTrajectoryRepository {
    async fn save(&self, trajectory: &Trajectory) -> DomainResult<()> {
        self.0.save(trajectory).await
    }

    async fn get(&self, trajectory_id: &str) -> DomainResult<Option<Trajectory>> {
        self.0.get(trajectory_id).await
    }

    async fn get_by_task(&self, task_id: &str) -> DomainResult<Vec<Trajectory>> {
        self.0.get_by_task(task_id).await
    }

    async fn get_by_goal(&self, goal_id: &str) -> DomainResult<Vec<Trajectory>> {
        self.0.get_by_goal(goal_id).await
    }

    async fn get_recent(&self, limit: usize) -> DomainResult<Vec<Trajectory>> {
        self.0.get_recent(limit).await
    }

    async fn get_successful_strategies(
        &self,
        attractor_type: &AttractorType,
        limit: usize,
    ) -> DomainResult<Vec<StrategyEntry>> {
        self.0.get_successful_strategies(attractor_type, limit).await
    }

    async fn delete(&self, trajectory_id: &str) -> DomainResult<()> {
        self.0.delete(trajectory_id).await
    }

    async fn avg_iterations_by_complexity(&self, complexity: Complexity) -> DomainResult<f64> {
        self.0.avg_iterations_by_complexity(complexity).await
    }

    async fn strategy_effectiveness(
        &self,
        strategy: crate::domain::models::convergence::StrategyKind,
    ) -> DomainResult<StrategyStats> {
        self.0.strategy_effectiveness(strategy).await
    }

    async fn attractor_distribution(&self) -> DomainResult<HashMap<String, u32>> {
        self.0.attractor_distribution().await
    }

    async fn convergence_rate_by_task_type(&self, category: &str) -> DomainResult<f64> {
        self.0.convergence_rate_by_task_type(category).await
    }

    async fn get_similar_trajectories(
        &self,
        description: &str,
        tags: &[String],
        limit: usize,
    ) -> DomainResult<Vec<Trajectory>> {
        self.0.get_similar_trajectories(description, tags, limit).await
    }
}

// -- Internal helpers --

/// Format overseer signals into a human-readable summary.
fn format_overseer_summary(signals: &OverseerSignals) -> String {
    let mut parts = vec![];

    if let Some(ref test_results) = signals.test_results {
        parts.push(format!(
            "Tests: {}/{} passed, {} failures",
            test_results.passed, test_results.total,
            test_results.failing_test_names.len()
        ));
    }
    if let Some(ref build_result) = signals.build_result {
        if build_result.success {
            parts.push("Build: SUCCESS".to_string());
        } else {
            parts.push(format!("Build: FAILED - {}", build_result.errors.join("; ")));
        }
    }
    if let Some(ref type_check) = signals.type_check {
        if type_check.clean {
            parts.push("Type check: CLEAN".to_string());
        } else {
            parts.push(format!("Type check: {} errors", type_check.error_count));
        }
    }
    if let Some(ref lint_results) = signals.lint_results {
        if lint_results.error_count == 0 {
            parts.push("Lint: CLEAN".to_string());
        } else {
            parts.push(format!("Lint: {} errors", lint_results.error_count));
        }
    }

    parts.join("\n")
}

/// Extract persistent gaps from the trajectory's observation history.
fn persistent_gaps(trajectory: &Trajectory) -> Vec<String> {
    // Delegate to Trajectory's built-in persistent_gaps method if it has data,
    // otherwise fall back to extracting from the latest observation.
    let gaps = trajectory.persistent_gaps();
    if !gaps.is_empty() {
        return gaps.iter().map(|g| format!("- {}", g)).collect();
    }

    // Fallback: extract from latest observation
    let mut result = vec![];
    if let Some(obs) = trajectory.observations.last() {
        if let Some(ref test_results) = obs.overseer_signals.test_results {
            for failure in &test_results.failing_test_names {
                result.push(format!("- Failing test: {}", failure));
            }
        }
        if let Some(ref build_result) = obs.overseer_signals.build_result {
            if !build_result.success {
                for error in &build_result.errors {
                    result.push(format!("- Build error: {}", error));
                }
            }
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::models::task::{Task, ExecutionMode, TaskPriority};

    #[test]
    fn test_task_to_submission_basic() {
        let task = Task::new("Implement OAuth2 login flow");
        let submission = task_to_submission(&task, None);
        assert_eq!(submission.description, "Implement OAuth2 login flow");
        assert!(submission.goal_id.is_none());
    }

    #[test]
    fn test_task_to_submission_with_goal() {
        let goal_id = Uuid::new_v4();
        let task = Task::new("Implement feature");
        let submission = task_to_submission(&task, Some(goal_id));
        assert_eq!(submission.goal_id, Some(goal_id));
    }

    #[test]
    fn test_task_to_submission_priority_mapping() {
        let task = Task::new("High priority task")
            .with_priority(TaskPriority::Critical);
        let submission = task_to_submission(&task, None);
        assert_eq!(submission.priority_hint, Some(PriorityHint::Thorough));

        let task = Task::new("Low priority task")
            .with_priority(TaskPriority::Low);
        let submission = task_to_submission(&task, None);
        assert_eq!(submission.priority_hint, Some(PriorityHint::Fast));
    }

    #[test]
    fn test_task_to_submission_convergent_parallel() {
        let task = Task::new("Complex task")
            .with_execution_mode(ExecutionMode::Convergent { parallel_samples: Some(3) });
        let submission = task_to_submission(&task, None);
        assert_eq!(submission.parallel_samples, Some(3));
    }

    #[test]
    fn test_task_to_submission_constraints() {
        let mut task = Task::new("Task with constraints");
        task.context.hints.push("constraint: must use async".to_string());
        task.context.hints.push("anti-pattern: no unwrap()".to_string());
        let submission = task_to_submission(&task, None);
        assert_eq!(submission.constraints.len(), 1);
        assert_eq!(submission.anti_patterns.len(), 1);
    }

    #[test]
    fn test_collect_artifact() {
        let artifact = collect_artifact("/tmp/worktree", "abc123");
        assert_eq!(artifact.path, "/tmp/worktree");
        assert_eq!(artifact.content_hash, "abc123");
    }
}
