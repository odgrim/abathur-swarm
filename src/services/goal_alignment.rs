//! Goal alignment service for holistic evaluation.
//!
//! Evaluates tasks and work against ALL active goals simultaneously,
//! ensuring the swarm's work converges toward satisfying all goals
//! rather than optimizing for just one.

use std::collections::HashMap;
use std::sync::Arc;
use uuid::Uuid;

use crate::domain::errors::DomainResult;
use crate::domain::models::{Goal, GoalStatus, Task};
use crate::domain::ports::GoalRepository;

/// Configuration for goal alignment evaluation.
#[derive(Debug, Clone)]
pub struct AlignmentConfig {
    /// Minimum alignment score to pass (0.0-1.0).
    pub min_alignment_score: f64,
    /// Weight for priority in scoring.
    pub priority_weight: f64,
    /// Weight for constraint violations.
    pub constraint_violation_penalty: f64,
    /// Whether to check all active goals.
    pub check_all_active_goals: bool,
    /// Minimum number of goals that must be satisfied.
    pub min_goals_satisfied: Option<usize>,
}

impl Default for AlignmentConfig {
    fn default() -> Self {
        Self {
            min_alignment_score: 0.6,
            priority_weight: 1.5,
            constraint_violation_penalty: 0.3,
            check_all_active_goals: true,
            min_goals_satisfied: None,
        }
    }
}

/// Result of evaluating a task against a single goal.
#[derive(Debug, Clone)]
pub struct GoalAlignmentResult {
    /// Goal ID.
    pub goal_id: Uuid,
    /// Goal name.
    pub goal_name: String,
    /// Alignment score (0.0-1.0).
    pub score: f64,
    /// Whether constraints are satisfied.
    pub constraints_satisfied: bool,
    /// Constraint violations.
    pub violations: Vec<ConstraintViolation>,
    /// Positive contributions to this goal.
    pub contributions: Vec<String>,
    /// Potential concerns or conflicts.
    pub concerns: Vec<String>,
}

impl GoalAlignmentResult {
    /// Check if this result indicates good alignment.
    pub fn is_aligned(&self, threshold: f64) -> bool {
        self.score >= threshold && self.constraints_satisfied
    }
}

/// A constraint violation.
#[derive(Debug, Clone)]
pub struct ConstraintViolation {
    /// Constraint name.
    pub constraint_name: String,
    /// Description of the violation.
    pub description: String,
    /// Severity (0.0-1.0, higher is more severe).
    pub severity: f64,
}

/// Result of holistic evaluation against all goals.
#[derive(Debug, Clone)]
pub struct HolisticEvaluation {
    /// Task being evaluated.
    pub task_id: Uuid,
    /// Individual goal alignments.
    pub goal_alignments: Vec<GoalAlignmentResult>,
    /// Overall alignment score (weighted average).
    pub overall_score: f64,
    /// Whether the evaluation passes.
    pub passes: bool,
    /// Number of goals satisfied.
    pub goals_satisfied: usize,
    /// Number of goals with concerns.
    pub goals_with_concerns: usize,
    /// Summary of the evaluation.
    pub summary: String,
    /// Recommended actions.
    pub recommendations: Vec<String>,
}

impl HolisticEvaluation {
    /// Get goals that are well-aligned.
    pub fn well_aligned_goals(&self, threshold: f64) -> Vec<&GoalAlignmentResult> {
        self.goal_alignments
            .iter()
            .filter(|g| g.is_aligned(threshold))
            .collect()
    }

    /// Get goals with violations.
    pub fn goals_with_violations(&self) -> Vec<&GoalAlignmentResult> {
        self.goal_alignments
            .iter()
            .filter(|g| !g.violations.is_empty())
            .collect()
    }

    /// Get the most critical violation.
    pub fn most_critical_violation(&self) -> Option<(&GoalAlignmentResult, &ConstraintViolation)> {
        self.goal_alignments
            .iter()
            .flat_map(|g| g.violations.iter().map(move |v| (g, v)))
            .max_by(|(_, a), (_, b)| a.severity.partial_cmp(&b.severity).unwrap_or(std::cmp::Ordering::Equal))
    }
}

/// Work artifact to evaluate.
#[derive(Debug, Clone)]
pub struct WorkArtifact {
    /// Description of the work.
    pub description: String,
    /// Files modified.
    pub files_modified: Vec<String>,
    /// Tests affected.
    pub tests_affected: Vec<String>,
    /// Documentation updated.
    pub docs_updated: Vec<String>,
    /// Additional context.
    pub context: HashMap<String, String>,
}

/// Goal alignment service.
pub struct GoalAlignmentService<G>
where
    G: GoalRepository + 'static,
{
    goal_repo: Arc<G>,
    config: AlignmentConfig,
}

impl<G> GoalAlignmentService<G>
where
    G: GoalRepository + 'static,
{
    pub fn new(goal_repo: Arc<G>, config: AlignmentConfig) -> Self {
        Self { goal_repo, config }
    }

    pub fn with_defaults(goal_repo: Arc<G>) -> Self {
        Self::new(goal_repo, AlignmentConfig::default())
    }

    /// Evaluate a task against all active goals.
    pub async fn evaluate_task(&self, task: &Task) -> DomainResult<HolisticEvaluation> {
        // Get all active goals
        let goals = self.get_active_goals().await?;

        if goals.is_empty() {
            return Ok(self.create_empty_evaluation(task.id));
        }

        // Evaluate against each goal
        let mut alignments = Vec::new();
        for goal in &goals {
            let alignment = self.evaluate_against_goal(task, goal)?;
            alignments.push(alignment);
        }

        // Calculate overall score
        let overall_score = self.calculate_overall_score(&alignments, &goals);

        // Determine if passes
        let goals_satisfied = alignments
            .iter()
            .filter(|a| a.is_aligned(self.config.min_alignment_score))
            .count();

        let goals_with_concerns = alignments
            .iter()
            .filter(|a| !a.concerns.is_empty())
            .count();

        let passes = self.check_passes(overall_score, goals_satisfied, goals.len());

        // Generate summary and recommendations
        let summary = self.generate_summary(&alignments, overall_score, goals_satisfied, goals.len());
        let recommendations = self.generate_recommendations(&alignments);

        Ok(HolisticEvaluation {
            task_id: task.id,
            goal_alignments: alignments,
            overall_score,
            passes,
            goals_satisfied,
            goals_with_concerns,
            summary,
            recommendations,
        })
    }

    /// Evaluate work artifacts against all active goals.
    pub async fn evaluate_work(
        &self,
        task: &Task,
        artifacts: &[WorkArtifact],
    ) -> DomainResult<HolisticEvaluation> {
        // Get all active goals
        let goals = self.get_active_goals().await?;

        if goals.is_empty() {
            return Ok(self.create_empty_evaluation(task.id));
        }

        // Evaluate against each goal
        let mut alignments = Vec::new();
        for goal in &goals {
            let alignment = self.evaluate_work_against_goal(task, artifacts, goal)?;
            alignments.push(alignment);
        }

        // Calculate overall score
        let overall_score = self.calculate_overall_score(&alignments, &goals);

        // Determine if passes
        let goals_satisfied = alignments
            .iter()
            .filter(|a| a.is_aligned(self.config.min_alignment_score))
            .count();

        let goals_with_concerns = alignments
            .iter()
            .filter(|a| !a.concerns.is_empty())
            .count();

        let passes = self.check_passes(overall_score, goals_satisfied, goals.len());

        // Generate summary and recommendations
        let summary = self.generate_summary(&alignments, overall_score, goals_satisfied, goals.len());
        let recommendations = self.generate_recommendations(&alignments);

        Ok(HolisticEvaluation {
            task_id: task.id,
            goal_alignments: alignments,
            overall_score,
            passes,
            goals_satisfied,
            goals_with_concerns,
            summary,
            recommendations,
        })
    }

    /// Get all active goals.
    async fn get_active_goals(&self) -> DomainResult<Vec<Goal>> {
        use crate::domain::ports::GoalFilter;

        let filter = GoalFilter {
            status: Some(GoalStatus::Active),
            ..Default::default()
        };

        self.goal_repo.list(filter).await
    }

    /// Evaluate a task against a single goal.
    fn evaluate_against_goal(&self, task: &Task, goal: &Goal) -> DomainResult<GoalAlignmentResult> {
        let mut score = 0.5; // Base score
        let mut violations = Vec::new();
        let mut contributions = Vec::new();
        let mut concerns = Vec::new();

        // Check if task is directly associated with this goal
        if task.goal_id == Some(goal.id) {
            score += 0.2;
            contributions.push("Task directly contributes to this goal".to_string());
        }

        // Check constraints
        for constraint in &goal.constraints {
            let satisfied = self.check_constraint(task, constraint);
            if !satisfied {
                violations.push(ConstraintViolation {
                    constraint_name: constraint.name.clone(),
                    description: format!("Task may violate: {}", constraint.description),
                    severity: 0.5,
                });
                score -= self.config.constraint_violation_penalty;
            }
        }

        // Keyword matching for relevance (simple heuristic)
        let goal_keywords = self.extract_keywords(&goal.description);
        let task_keywords = self.extract_keywords(&task.description);
        let overlap = goal_keywords
            .iter()
            .filter(|k| task_keywords.contains(*k))
            .count();

        if overlap > 0 {
            let relevance_boost = (overlap as f64 / goal_keywords.len().max(1) as f64) * 0.2;
            score += relevance_boost;
            contributions.push(format!("Task has {} relevant keywords", overlap));
        }

        // Check for potential concerns based on goal type
        if goal.description.to_lowercase().contains("security") {
            if !task.description.to_lowercase().contains("security")
                && !task.description.to_lowercase().contains("auth")
            {
                concerns.push("Task may not address security considerations".to_string());
            }
        }

        if goal.description.to_lowercase().contains("test") {
            if !task.description.to_lowercase().contains("test") {
                concerns.push("Task may need test coverage".to_string());
            }
        }

        // Clamp score
        score = score.clamp(0.0, 1.0);

        Ok(GoalAlignmentResult {
            goal_id: goal.id,
            goal_name: goal.name.clone(),
            score,
            constraints_satisfied: violations.is_empty(),
            violations,
            contributions,
            concerns,
        })
    }

    /// Evaluate work artifacts against a single goal.
    fn evaluate_work_against_goal(
        &self,
        task: &Task,
        artifacts: &[WorkArtifact],
        goal: &Goal,
    ) -> DomainResult<GoalAlignmentResult> {
        // Start with task evaluation
        let mut result = self.evaluate_against_goal(task, goal)?;

        // Enhance with artifact analysis
        for artifact in artifacts {
            // Check if files align with goal
            if goal.description.to_lowercase().contains("test") {
                let test_files = artifact
                    .files_modified
                    .iter()
                    .filter(|f| f.contains("test") || f.contains("spec"))
                    .count();

                if test_files > 0 {
                    result.score += 0.1;
                    result.contributions.push(format!("Modified {} test files", test_files));
                }
            }

            // Check documentation for doc-related goals
            if goal.description.to_lowercase().contains("document") {
                if !artifact.docs_updated.is_empty() {
                    result.score += 0.1;
                    result.contributions.push(format!(
                        "Updated {} documentation files",
                        artifact.docs_updated.len()
                    ));
                } else if !artifact.files_modified.is_empty() {
                    result.concerns.push("No documentation updated for code changes".to_string());
                }
            }
        }

        // Clamp score
        result.score = result.score.clamp(0.0, 1.0);

        Ok(result)
    }

    /// Check if a constraint is satisfied.
    fn check_constraint(
        &self,
        task: &Task,
        constraint: &crate::domain::models::GoalConstraint,
    ) -> bool {
        // Simple heuristic check - in production this would be more sophisticated
        let constraint_lower = constraint.description.to_lowercase();
        let task_lower = task.description.to_lowercase();

        // If constraint mentions "must not" or "never", check for violations
        if constraint_lower.contains("must not") || constraint_lower.contains("never") {
            // Extract the forbidden action and check if task does it
            // This is a simplified check
            return true; // Assume satisfied unless we have evidence otherwise
        }

        // If constraint mentions "must" or "always", check for compliance
        if constraint_lower.contains("must") || constraint_lower.contains("always") {
            // Check if task description mentions relevant terms
            let keywords = self.extract_keywords(&constraint.description);
            return keywords.iter().any(|k| task_lower.contains(k));
        }

        true // Default to satisfied
    }

    /// Extract keywords from text.
    fn extract_keywords(&self, text: &str) -> Vec<String> {
        let stopwords = ["the", "a", "an", "is", "are", "was", "were", "be", "been", "being",
            "have", "has", "had", "do", "does", "did", "will", "would", "could", "should",
            "may", "might", "must", "shall", "can", "need", "to", "of", "in", "for", "on",
            "with", "at", "by", "from", "as", "into", "through", "during", "before", "after",
            "above", "below", "between", "under", "and", "but", "or", "nor", "not", "so",
            "yet", "both", "either", "neither", "all", "each", "every", "some", "any", "no",
            "more", "most", "other", "such", "only", "own", "same", "than", "too", "very",
            "just", "also", "now", "that", "this", "these", "those", "it", "its"];

        text.to_lowercase()
            .split(|c: char| !c.is_alphanumeric())
            .filter(|w| w.len() > 2 && !stopwords.contains(w))
            .map(|s| s.to_string())
            .collect()
    }

    /// Calculate overall score from individual alignments.
    fn calculate_overall_score(&self, alignments: &[GoalAlignmentResult], goals: &[Goal]) -> f64 {
        if alignments.is_empty() {
            return 1.0;
        }

        let mut weighted_sum = 0.0;
        let mut weight_total = 0.0;

        for (alignment, goal) in alignments.iter().zip(goals.iter()) {
            let weight = match goal.priority {
                crate::domain::models::GoalPriority::Critical => 2.0 * self.config.priority_weight,
                crate::domain::models::GoalPriority::High => 1.5 * self.config.priority_weight,
                crate::domain::models::GoalPriority::Normal => 1.0,
                crate::domain::models::GoalPriority::Low => 0.5,
            };

            weighted_sum += alignment.score * weight;
            weight_total += weight;
        }

        if weight_total > 0.0 {
            weighted_sum / weight_total
        } else {
            1.0
        }
    }

    /// Check if evaluation passes.
    fn check_passes(&self, overall_score: f64, goals_satisfied: usize, total_goals: usize) -> bool {
        if overall_score < self.config.min_alignment_score {
            return false;
        }

        if let Some(min) = self.config.min_goals_satisfied {
            if goals_satisfied < min {
                return false;
            }
        }

        // Require at least half the goals to be satisfied
        if total_goals > 0 && goals_satisfied < (total_goals + 1) / 2 {
            return false;
        }

        true
    }

    /// Generate evaluation summary.
    fn generate_summary(
        &self,
        alignments: &[GoalAlignmentResult],
        overall_score: f64,
        goals_satisfied: usize,
        total_goals: usize,
    ) -> String {
        format!(
            "Holistic evaluation: {:.0}% alignment ({}/{} goals satisfied). {}",
            overall_score * 100.0,
            goals_satisfied,
            total_goals,
            if alignments.iter().any(|a| !a.violations.is_empty()) {
                "Some constraints require attention."
            } else {
                "All constraints satisfied."
            }
        )
    }

    /// Generate recommendations based on alignments.
    fn generate_recommendations(&self, alignments: &[GoalAlignmentResult]) -> Vec<String> {
        let mut recommendations = Vec::new();

        for alignment in alignments {
            if alignment.score < self.config.min_alignment_score {
                recommendations.push(format!(
                    "Improve alignment with goal '{}' (currently {:.0}%)",
                    alignment.goal_name,
                    alignment.score * 100.0
                ));
            }

            for violation in &alignment.violations {
                recommendations.push(format!(
                    "Address constraint violation in '{}': {}",
                    alignment.goal_name, violation.description
                ));
            }
        }

        recommendations
    }

    /// Create an empty evaluation (when no goals exist).
    fn create_empty_evaluation(&self, task_id: Uuid) -> HolisticEvaluation {
        HolisticEvaluation {
            task_id,
            goal_alignments: Vec::new(),
            overall_score: 1.0,
            passes: true,
            goals_satisfied: 0,
            goals_with_concerns: 0,
            summary: "No active goals to evaluate against".to_string(),
            recommendations: Vec::new(),
        }
    }

    /// Get configuration.
    pub fn config(&self) -> &AlignmentConfig {
        &self.config
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_default() {
        let config = AlignmentConfig::default();
        assert_eq!(config.min_alignment_score, 0.6);
        assert!(config.check_all_active_goals);
    }

    #[test]
    fn test_goal_alignment_result_is_aligned() {
        let result = GoalAlignmentResult {
            goal_id: Uuid::new_v4(),
            goal_name: "Test".to_string(),
            score: 0.8,
            constraints_satisfied: true,
            violations: vec![],
            contributions: vec![],
            concerns: vec![],
        };

        assert!(result.is_aligned(0.6));
        assert!(result.is_aligned(0.8));
        assert!(!result.is_aligned(0.9));
    }

    #[test]
    fn test_goal_alignment_with_violations_not_aligned() {
        let result = GoalAlignmentResult {
            goal_id: Uuid::new_v4(),
            goal_name: "Test".to_string(),
            score: 0.9,
            constraints_satisfied: false,
            violations: vec![ConstraintViolation {
                constraint_name: "test".to_string(),
                description: "violation".to_string(),
                severity: 0.5,
            }],
            contributions: vec![],
            concerns: vec![],
        };

        assert!(!result.is_aligned(0.6)); // High score but violations
    }

    #[test]
    fn test_holistic_evaluation_well_aligned_goals() {
        let eval = HolisticEvaluation {
            task_id: Uuid::new_v4(),
            goal_alignments: vec![
                GoalAlignmentResult {
                    goal_id: Uuid::new_v4(),
                    goal_name: "High".to_string(),
                    score: 0.9,
                    constraints_satisfied: true,
                    violations: vec![],
                    contributions: vec![],
                    concerns: vec![],
                },
                GoalAlignmentResult {
                    goal_id: Uuid::new_v4(),
                    goal_name: "Low".to_string(),
                    score: 0.3,
                    constraints_satisfied: true,
                    violations: vec![],
                    contributions: vec![],
                    concerns: vec![],
                },
            ],
            overall_score: 0.6,
            passes: true,
            goals_satisfied: 1,
            goals_with_concerns: 0,
            summary: "Test".to_string(),
            recommendations: vec![],
        };

        let well_aligned = eval.well_aligned_goals(0.6);
        assert_eq!(well_aligned.len(), 1);
        assert_eq!(well_aligned[0].goal_name, "High");
    }

    #[test]
    fn test_extract_keywords() {
        use crate::adapters::sqlite::{create_test_pool, SqliteGoalRepository, Migrator, all_embedded_migrations};
        use tokio::runtime::Runtime;

        let rt = Runtime::new().unwrap();
        rt.block_on(async {
            let pool = create_test_pool().await.unwrap();
            let migrator = Migrator::new(pool.clone());
            migrator.run_embedded_migrations(all_embedded_migrations()).await.unwrap();
            let repo = Arc::new(SqliteGoalRepository::new(pool));
            let service = GoalAlignmentService::with_defaults(repo);

            let keywords = service.extract_keywords("Implement user authentication with OAuth2");
            assert!(keywords.contains(&"implement".to_string()));
            assert!(keywords.contains(&"user".to_string()));
            assert!(keywords.contains(&"authentication".to_string()));
            assert!(keywords.contains(&"oauth2".to_string()));
            assert!(!keywords.contains(&"with".to_string())); // Stopword
        });
    }

    #[test]
    fn test_constraint_violation_severity() {
        let violation = ConstraintViolation {
            constraint_name: "Security".to_string(),
            description: "Missing input validation".to_string(),
            severity: 0.8,
        };

        assert_eq!(violation.severity, 0.8);
    }
}
