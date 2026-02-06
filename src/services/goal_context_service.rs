//! Goal context service for selective goal loading.
//!
//! Provides contextual goal guidance by inferring which domains a task touches
//! and loading relevant goals as aspirational guidance.

use crate::domain::errors::DomainResult;
use crate::domain::models::goal::{Goal, GoalConstraint};
use crate::domain::models::task::Task;
use crate::domain::ports::goal_repository::GoalRepository;
use std::sync::Arc;

/// Configuration for context budget management.
///
/// Implements the Manus AI "context engineering" approach:
/// - Allocate a total token budget across context sections
/// - Each section (goals, memories, artifacts) gets a proportional share
/// - Within each section, entries are ranked and selected to fit
#[derive(Debug, Clone)]
pub struct ContextBudget {
    /// Total token budget for all context (system prompt is separate).
    pub total_tokens: usize,
    /// Fraction of budget allocated to goal context (0.0-1.0).
    pub goal_fraction: f32,
    /// Fraction of budget allocated to memory context (0.0-1.0).
    pub memory_fraction: f32,
    /// Fraction of budget allocated to upstream artifact context (0.0-1.0).
    pub artifact_fraction: f32,
    /// Fraction reserved for the task description itself (0.0-1.0).
    pub task_fraction: f32,
}

impl Default for ContextBudget {
    fn default() -> Self {
        Self {
            total_tokens: 8000, // Conservative default for agent context
            goal_fraction: 0.20,    // 20% for goals
            memory_fraction: 0.25,  // 25% for relevant memories
            artifact_fraction: 0.15, // 15% for upstream artifacts
            task_fraction: 0.40,    // 40% for the task itself
        }
    }
}

impl ContextBudget {
    /// Create a generous budget (for large context windows).
    pub fn generous() -> Self {
        Self {
            total_tokens: 16000,
            ..Default::default()
        }
    }

    /// Create a tight budget (for small context windows).
    pub fn tight() -> Self {
        Self {
            total_tokens: 4000,
            goal_fraction: 0.15,
            memory_fraction: 0.20,
            artifact_fraction: 0.10,
            task_fraction: 0.55,
        }
    }

    pub fn goal_budget(&self) -> usize {
        (self.total_tokens as f32 * self.goal_fraction) as usize
    }

    pub fn memory_budget(&self) -> usize {
        (self.total_tokens as f32 * self.memory_fraction) as usize
    }

    pub fn artifact_budget(&self) -> usize {
        (self.total_tokens as f32 * self.artifact_fraction) as usize
    }

    pub fn task_budget(&self) -> usize {
        (self.total_tokens as f32 * self.task_fraction) as usize
    }
}

/// Assembled context for an agent, with budget tracking.
#[derive(Debug, Clone, Default)]
pub struct AgentContext {
    /// Goal guidance text.
    pub goal_context: String,
    /// Memory context (relevant memories from previous work).
    pub memory_context: String,
    /// Upstream artifact references.
    pub artifact_context: String,
    /// The task description (potentially compressed).
    pub task_context: String,
    /// Token usage breakdown.
    pub token_usage: ContextTokenUsage,
}

/// Token usage breakdown for agent context.
#[derive(Debug, Clone, Default)]
pub struct ContextTokenUsage {
    pub goal_tokens: usize,
    pub memory_tokens: usize,
    pub artifact_tokens: usize,
    pub task_tokens: usize,
    pub total_tokens: usize,
    pub budget_tokens: usize,
    pub utilization: f32,
}

impl AgentContext {
    /// Format the full context as a single string for injection into agent prompt.
    pub fn format_for_prompt(&self) -> String {
        let mut sections = Vec::new();

        if !self.goal_context.is_empty() {
            sections.push(self.goal_context.clone());
        }
        if !self.memory_context.is_empty() {
            sections.push(self.memory_context.clone());
        }
        if !self.artifact_context.is_empty() {
            sections.push(self.artifact_context.clone());
        }

        sections.join("\n\n---\n\n")
    }
}

/// Service that selectively loads relevant goals as contextual guidance for tasks.
/// Goals are aspirational - they don't own tasks but provide guidance when relevant.
///
/// Enhanced with:
/// - Context budget management (Manus AI pattern)
/// - Domain-scoped memory loading for context isolation (DynTaskMAS pattern)
/// - Multi-factor relevance scoring for memory selection
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
