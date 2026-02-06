//! Goal context service for selective goal loading.
//!
//! Provides contextual goal guidance by inferring which domains a task touches
//! and loading relevant goals as aspirational guidance.

use crate::domain::errors::DomainResult;
use crate::domain::models::goal::{Goal, GoalConstraint};
use crate::domain::models::task::Task;
use crate::domain::ports::goal_repository::GoalRepository;
use std::sync::Arc;

/// Service that selectively loads relevant goals as contextual guidance for tasks.
/// Goals are aspirational - they don't own tasks but provide guidance when relevant.
pub struct GoalContextService<G: GoalRepository> {
    goal_repo: Arc<G>,
}

impl<G: GoalRepository> GoalContextService<G> {
    pub fn new(goal_repo: Arc<G>) -> Self {
        Self { goal_repo }
    }

    /// Infer which applicability domains a task touches based on its content.
    pub fn infer_task_domains(task: &Task) -> Vec<String> {
        let mut domains = Vec::new();
        let text = format!("{} {} {}", task.title, task.description, task.context.input).to_lowercase();
        let agent = task.agent_type.as_deref().unwrap_or("").to_lowercase();

        // Code quality is almost always relevant for code-producing tasks
        if agent.contains("code") || agent.contains("developer") || agent.contains("engineer")
            || text.contains("implement") || text.contains("refactor") || text.contains("write")
            || text.contains("build") || text.contains("create") || text.contains("fix")
        {
            domains.push("code-quality".to_string());
        }

        // Frontend/UX
        if agent.contains("frontend") || text.contains("ui") || text.contains("ux")
            || text.contains("component") || text.contains("css") || text.contains("layout")
            || text.contains("design") || text.contains("user interface")
        {
            domains.push("frontend".to_string());
            domains.push("ux".to_string());
        }

        // Testing
        if text.contains("test") || text.contains("spec") || text.contains("coverage")
            || text.contains("assertion") || text.contains("mock")
        {
            domains.push("testing".to_string());
        }

        // Security
        if agent.contains("security") || text.contains("auth") || text.contains("encrypt")
            || text.contains("vulnerab") || text.contains("permission") || text.contains("credential")
            || text.contains("token") || text.contains("secret")
        {
            domains.push("security".to_string());
        }

        // Performance
        if text.contains("perf") || text.contains("optimiz") || text.contains("cache")
            || text.contains("latency") || text.contains("throughput") || text.contains("speed")
        {
            domains.push("performance".to_string());
        }

        // Infrastructure
        if text.contains("deploy") || text.contains("infra") || text.contains("terraform")
            || text.contains("docker") || text.contains("ci/cd") || text.contains("pipeline")
            || text.contains("kubernetes") || text.contains("k8s")
        {
            domains.push("infrastructure".to_string());
        }

        // Backend
        if agent.contains("backend") || text.contains("api") || text.contains("endpoint")
            || text.contains("database") || text.contains("query") || text.contains("migration")
            || text.contains("server")
        {
            domains.push("backend".to_string());
        }

        // Check for explicit domains in task context
        if let Some(serde_json::Value::Array(explicit)) = task.context.custom.get("domains") {
            for d in explicit {
                if let Some(s) = d.as_str() {
                    if !domains.contains(&s.to_string()) {
                        domains.push(s.to_string());
                    }
                }
            }
        }

        domains.dedup();
        domains
    }

    /// Load all active goals relevant to the given domains.
    pub async fn get_relevant_goals(&self, domains: &[String]) -> DomainResult<Vec<Goal>> {
        self.goal_repo.find_by_domains(domains).await
    }

    /// Get goals relevant to a specific task (infer domains + load matching goals).
    pub async fn get_goals_for_task(&self, task: &Task) -> DomainResult<Vec<Goal>> {
        let domains = Self::infer_task_domains(task);
        if domains.is_empty() {
            return Ok(Vec::new());
        }
        self.get_relevant_goals(&domains).await
    }

    /// Format goals as contextual guidance text for inclusion in an agent prompt.
    pub fn format_goal_context(goals: &[Goal]) -> String {
        if goals.is_empty() {
            return String::new();
        }

        let mut output = String::from("## Guiding Goals\nThe following organizational goals are relevant to this task. Use them as guidance:\n\n");

        for goal in goals {
            output.push_str(&format!("### {}\n", goal.name));
            output.push_str(&format!("{}\n", goal.description));

            if !goal.constraints.is_empty() {
                output.push_str("Constraints:\n");
                for c in &goal.constraints {
                    output.push_str(&format!(
                        "- [{:?}] {}: {}\n",
                        c.constraint_type, c.name, c.description
                    ));
                }
            }

            if !goal.evaluation_criteria.is_empty() {
                output.push_str("Success criteria:\n");
                for criterion in &goal.evaluation_criteria {
                    output.push_str(&format!("- {}\n", criterion));
                }
            }

            output.push('\n');
        }

        output
    }

    /// Collect all constraints from relevant goals (flattened, deduplicated by name).
    pub fn collect_constraints(goals: &[Goal]) -> Vec<GoalConstraint> {
        let mut constraints = Vec::new();
        let mut seen_names = std::collections::HashSet::new();

        for goal in goals {
            for c in &goal.constraints {
                if seen_names.insert(c.name.clone()) {
                    constraints.push(c.clone());
                }
            }
        }

        constraints
    }
}
