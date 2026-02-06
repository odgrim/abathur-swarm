//! Goal evaluation service for periodic goal assessment.
//!
//! Evaluates whether goals are being met by examining completed tasks,
//! identifies gaps, and creates corrective tasks to address unmet criteria.

use std::sync::Arc;
use uuid::Uuid;

use crate::domain::errors::DomainResult;
use crate::domain::models::goal::Goal;
use crate::domain::models::task::{Task, TaskPriority, TaskSource, TaskStatus};
use crate::domain::ports::goal_repository::GoalRepository;
use crate::domain::ports::task_repository::TaskRepository;
use crate::services::goal_context_service::GoalContextService;

/// How well a goal's evaluation criteria are being satisfied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SatisfactionLevel {
    /// All evaluation criteria are addressed by completed work
    Met,
    /// Some but not all criteria are addressed
    PartiallyMet,
    /// No criteria are addressed by completed work
    NotMet,
    /// Cannot determine (e.g., no evaluation criteria defined)
    Unknown,
}

/// A gap identified between a goal's criteria and completed work.
#[derive(Debug, Clone)]
pub struct GoalGap {
    /// Description of what is missing
    pub description: String,
    /// Severity: "low", "medium", or "high"
    pub severity: String,
}

/// A task suggested to address a goal gap.
#[derive(Debug, Clone)]
pub struct SuggestedTask {
    /// Task title
    pub title: String,
    /// Task description
    pub description: String,
    /// Relevant domains for this task
    pub domains: Vec<String>,
    /// Priority for the suggested task
    pub priority: TaskPriority,
}

/// Result of evaluating a single goal.
#[derive(Debug, Clone)]
pub struct GoalEvaluationResult {
    /// The goal that was evaluated
    pub goal_id: Uuid,
    /// Human-readable goal name
    pub goal_name: String,
    /// Overall satisfaction level
    pub satisfaction_level: SatisfactionLevel,
    /// Evidence of criteria being met (matched task titles/descriptions)
    pub evidence: Vec<String>,
    /// Identified gaps where criteria are not met
    pub gaps: Vec<GoalGap>,
    /// Suggested corrective tasks
    pub suggested_tasks: Vec<SuggestedTask>,
}

/// Summary report for a full evaluation cycle.
#[derive(Debug, Clone)]
pub struct EvaluationCycleReport {
    /// Number of goals evaluated
    pub evaluated_count: usize,
    /// Number of goals fully met
    pub goals_met: usize,
    /// Number of goals partially met
    pub goals_partially_met: usize,
    /// Total gaps found across all goals
    pub gaps_found: usize,
    /// Number of corrective tasks created
    pub tasks_created: usize,
}

/// Service that periodically evaluates goals against completed work
/// and creates corrective tasks to address gaps.
pub struct GoalEvaluationService<G: GoalRepository, T: TaskRepository> {
    goal_repo: Arc<G>,
    task_repo: Arc<T>,
}

impl<G: GoalRepository, T: TaskRepository> GoalEvaluationService<G, T> {
    pub fn new(goal_repo: Arc<G>, task_repo: Arc<T>) -> Self {
        Self {
            goal_repo,
            task_repo,
        }
    }

    /// Evaluate all active goals against completed work.
    pub async fn evaluate_all_goals(&self) -> DomainResult<Vec<GoalEvaluationResult>> {
        let active_goals = self.goal_repo.get_active_with_constraints().await?;
        let completed_tasks = self.task_repo.list_by_status(TaskStatus::Complete).await?;

        let mut results = Vec::new();
        for goal in &active_goals {
            results.push(self.evaluate_goal(goal, &completed_tasks));
        }

        Ok(results)
    }

    /// Evaluate a single goal against completed tasks.
    ///
    /// Finds completed tasks whose content overlaps with the goal's applicability
    /// domains, then checks each evaluation criterion for evidence of completion.
    pub fn evaluate_goal(&self, goal: &Goal, completed_tasks: &[Task]) -> GoalEvaluationResult {
        // If no evaluation criteria, we can't assess
        if goal.evaluation_criteria.is_empty() {
            return GoalEvaluationResult {
                goal_id: goal.id,
                goal_name: goal.name.clone(),
                satisfaction_level: SatisfactionLevel::Unknown,
                evidence: Vec::new(),
                gaps: Vec::new(),
                suggested_tasks: Vec::new(),
            };
        }

        // Filter completed tasks to those relevant to this goal's domains
        let relevant_tasks: Vec<&Task> = completed_tasks
            .iter()
            .filter(|task| self.task_overlaps_domains(task, &goal.applicability_domains))
            .collect();

        let mut evidence = Vec::new();
        let mut gaps = Vec::new();
        let mut suggested_tasks = Vec::new();
        let mut criteria_met = 0usize;

        for criterion in &goal.evaluation_criteria {
            let criterion_lower = criterion.to_lowercase();

            // Check if any relevant completed task addresses this criterion
            let matching_task = relevant_tasks.iter().find(|task| {
                let task_text =
                    format!("{} {}", task.title, task.description).to_lowercase();
                criterion_keywords_match(&criterion_lower, &task_text)
            });

            if let Some(task) = matching_task {
                criteria_met += 1;
                evidence.push(format!(
                    "Criterion '{}' addressed by task '{}'",
                    criterion, task.title
                ));
            } else {
                // Determine severity based on goal priority
                let severity = match goal.priority {
                    crate::domain::models::goal::GoalPriority::Critical => "high",
                    crate::domain::models::goal::GoalPriority::High => "high",
                    crate::domain::models::goal::GoalPriority::Normal => "medium",
                    crate::domain::models::goal::GoalPriority::Low => "low",
                };

                gaps.push(GoalGap {
                    description: format!(
                        "Criterion not met for goal '{}': {}",
                        goal.name, criterion
                    ),
                    severity: severity.to_string(),
                });

                // Suggest a corrective task
                let priority = match severity {
                    "high" => TaskPriority::High,
                    "medium" => TaskPriority::Normal,
                    _ => TaskPriority::Low,
                };

                suggested_tasks.push(SuggestedTask {
                    title: format!("Address: {}", criterion),
                    description: format!(
                        "Goal '{}' has unmet criterion: {}. Create work to satisfy this requirement.",
                        goal.name, criterion
                    ),
                    domains: goal.applicability_domains.clone(),
                    priority,
                });
            }
        }

        let total_criteria = goal.evaluation_criteria.len();
        let satisfaction_level = if criteria_met == total_criteria {
            SatisfactionLevel::Met
        } else if criteria_met > 0 {
            SatisfactionLevel::PartiallyMet
        } else {
            SatisfactionLevel::NotMet
        };

        GoalEvaluationResult {
            goal_id: goal.id,
            goal_name: goal.name.clone(),
            satisfaction_level,
            evidence,
            gaps,
            suggested_tasks,
        }
    }

    /// Create corrective tasks in the repository for evaluation results that have gaps.
    pub async fn create_corrective_tasks(
        &self,
        results: &[GoalEvaluationResult],
    ) -> DomainResult<Vec<Task>> {
        let mut created_tasks = Vec::new();

        for result in results {
            if result.gaps.is_empty() {
                continue;
            }

            for suggested in &result.suggested_tasks {
                // Use idempotency key to avoid duplicate corrective tasks
                let idemp_key = format!(
                    "goal-eval:{}:{}",
                    result.goal_id,
                    slug_from_title(&suggested.title)
                );

                // Check if we already created this corrective task
                if let Some(_existing) =
                    self.task_repo.get_by_idempotency_key(&idemp_key).await?
                {
                    continue;
                }

                let task = Task::with_title(&suggested.title, &suggested.description)
                    .with_priority(suggested.priority)
                    .with_source(TaskSource::GoalEvaluation(result.goal_id))
                    .with_idempotency_key(idemp_key);

                self.task_repo.create(&task).await?;
                created_tasks.push(task);
            }
        }

        Ok(created_tasks)
    }

    /// Run a full evaluation cycle: evaluate all goals, create corrective tasks, return report.
    pub async fn run_evaluation_cycle(&self) -> DomainResult<EvaluationCycleReport> {
        let results = self.evaluate_all_goals().await?;

        let evaluated_count = results.len();
        let goals_met = results
            .iter()
            .filter(|r| r.satisfaction_level == SatisfactionLevel::Met)
            .count();
        let goals_partially_met = results
            .iter()
            .filter(|r| r.satisfaction_level == SatisfactionLevel::PartiallyMet)
            .count();
        let gaps_found: usize = results.iter().map(|r| r.gaps.len()).sum();

        let created_tasks = self.create_corrective_tasks(&results).await?;
        let tasks_created = created_tasks.len();

        Ok(EvaluationCycleReport {
            evaluated_count,
            goals_met,
            goals_partially_met,
            gaps_found,
            tasks_created,
        })
    }

    /// Check if a task's content overlaps with the given applicability domains.
    /// Uses the same keyword-inference approach as GoalContextService but in reverse.
    fn task_overlaps_domains(&self, task: &Task, goal_domains: &[String]) -> bool {
        if goal_domains.is_empty() {
            // If goal has no domains, consider all tasks relevant
            return true;
        }

        let inferred = GoalContextService::<G>::infer_task_domains(task);
        // Check for any overlap between inferred task domains and goal domains
        inferred
            .iter()
            .any(|d| goal_domains.iter().any(|gd| gd == d))
    }
}

/// Simple keyword matching: split the criterion into words and check if
/// a reasonable number of them appear in the task text.
fn criterion_keywords_match(criterion: &str, task_text: &str) -> bool {
    let stop_words: &[&str] = &[
        "a", "an", "the", "is", "are", "was", "were", "be", "been", "being", "have", "has",
        "had", "do", "does", "did", "will", "would", "shall", "should", "may", "might", "must",
        "can", "could", "to", "of", "in", "for", "on", "with", "at", "by", "from", "as", "into",
        "through", "during", "before", "after", "and", "but", "or", "nor", "not", "so", "yet",
        "all", "each", "every", "both", "few", "more", "most", "other", "some", "such", "no",
        "only", "own", "same", "than", "too", "very", "just", "that", "this", "these", "those",
    ];

    let keywords: Vec<&str> = criterion
        .split_whitespace()
        .filter(|w| w.len() > 2 && !stop_words.contains(w))
        .collect();

    if keywords.is_empty() {
        return false;
    }

    let matched = keywords
        .iter()
        .filter(|kw| task_text.contains(**kw))
        .count();

    // Require at least half of the meaningful keywords to match
    matched * 2 >= keywords.len()
}

/// Create a simple slug from a task title for use in idempotency keys.
fn slug_from_title(title: &str) -> String {
    title
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<&str>>()
        .join("-")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_criterion_keywords_match() {
        assert!(criterion_keywords_match(
            "all tests pass with coverage",
            "added tests with full coverage for the module"
        ));
        assert!(!criterion_keywords_match(
            "database migration complete",
            "updated the frontend css styles"
        ));
    }

    #[test]
    fn test_slug_from_title() {
        assert_eq!(
            slug_from_title("Address: all tests pass"),
            "address-all-tests-pass"
        );
    }

    #[test]
    fn test_satisfaction_levels() {
        assert_eq!(SatisfactionLevel::Met, SatisfactionLevel::Met);
        assert_ne!(SatisfactionLevel::Met, SatisfactionLevel::NotMet);
    }
}
