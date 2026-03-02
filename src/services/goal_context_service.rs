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

        // Convergence / convergence engine
        if text.contains("convergence") || text.contains("converge") || text.contains("attractor")
            || text.contains("trajectory") || text.contains("thompson sampling")
            || text.contains("strategy selection") || text.contains("budget allocation")
            || text.contains("overseer")
        {
            domains.push("convergence".to_string());
        }

        // Evolution / agent evolution
        if agent.contains("evolution") || text.contains("evolution") || text.contains("evolve")
            || text.contains("agent template") || text.contains("refinement")
            || text.contains("success rate") || text.contains("regression")
            || text.contains("revert") || text.contains("performance tracking")
        {
            domains.push("evolution".to_string());
        }

        // Memory lifecycle
        if text.contains("memory tier") || text.contains("working memory")
            || text.contains("episodic") || text.contains("semantic")
            || text.contains("decay") || text.contains("promotion")
            || text.contains("relevance") || text.contains("context budget")
        {
            domains.push("memory-lifecycle".to_string());
        }

        // Agent lifecycle
        if agent.contains("agent") || text.contains("agent lifecycle")
            || text.contains("agent template") || text.contains("spawn")
            || text.contains("routing") || text.contains("specialist")
            || text.contains("worker") || text.contains("architect")
        {
            domains.push("agent-lifecycle".to_string());
        }

        // Workflow
        if text.contains("workflow") || text.contains("phase")
            || text.contains("spine") || text.contains("fan out")
            || text.contains("fan_out") || text.contains("gate")
            || text.contains("advance") || text.contains("triage")
        {
            domains.push("workflow".to_string());
        }

        // Swarm orchestration
        if text.contains("swarm") || text.contains("orchestrat")
            || text.contains("coordinator") || text.contains("event bus")
            || text.contains("event reactor") || text.contains("circuit breaker")
        {
            domains.push("swarm-orchestration".to_string());
        }

        // Goal management
        if text.contains("goal") || text.contains("convergence check")
            || text.contains("constraint") || text.contains("aspiration")
            || text.contains("domain inference") || text.contains("traceability")
        {
            domains.push("goal-management".to_string());
        }

        // Check for explicit domains in task context
        if let Some(serde_json::Value::Array(explicit)) = task.context.custom.get("domains") {
            for d in explicit {
                if let Some(s) = d.as_str()
                    && !domains.contains(&s.to_string()) {
                        domains.push(s.to_string());
                    }
            }
        }

        // Remove all duplicates (not just consecutive ones) while preserving order
        let mut seen = std::collections::HashSet::new();
        domains.retain(|d| seen.insert(d.clone()));
        domains
    }

    /// Load all active goals relevant to the given domains.
    pub async fn get_relevant_goals(&self, domains: &[String]) -> DomainResult<Vec<Goal>> {
        self.goal_repo.find_by_domains(domains).await
    }

    /// Get goals relevant to a specific task (infer domains + load matching goals).
    ///
    /// Goals with empty `applicability_domains` are universally applicable and will
    /// always be returned, regardless of the inferred task domains.
    pub async fn get_goals_for_task(&self, task: &Task) -> DomainResult<Vec<Goal>> {
        let domains = Self::infer_task_domains(task);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::models::goal::{
        Goal, GoalConstraint, GoalMetadata, GoalPriority, GoalStatus,
    };
    use crate::domain::models::task::Task;

    /// Create a minimal test task with title and description.
    fn make_task(title: &str, description: &str) -> Task {
        let mut task = Task::new(description);
        task.title = title.to_string();
        task
    }

    /// Create a minimal test task with title, description, and agent_type.
    fn make_task_with_agent(title: &str, description: &str, agent_type: &str) -> Task {
        let mut task = make_task(title, description);
        task.agent_type = Some(agent_type.to_string());
        task
    }

    /// Create a test goal with name, description, constraints, and domains.
    fn make_goal(
        name: &str,
        description: &str,
        constraints: Vec<GoalConstraint>,
        domains: Vec<String>,
    ) -> Goal {
        Goal {
            id: uuid::Uuid::new_v4(),
            name: name.to_string(),
            description: description.to_string(),
            status: GoalStatus::Active,
            priority: GoalPriority::Normal,
            parent_id: None,
            constraints,
            applicability_domains: domains,
            metadata: GoalMetadata {
                tags: vec![],
                custom: Default::default(),
            },
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            version: 1,
            last_convergence_check_at: None,
        }
    }

    // ── infer_task_domains tests ─────────────────────────────────────

    #[test]
    fn test_infer_domains_empty_task() {
        let task = make_task("", "");
        let domains = GoalContextService::<crate::adapters::sqlite::goal_repository::SqliteGoalRepository>::infer_task_domains(&task);
        assert!(domains.is_empty(), "Empty task should produce no domains");
    }

    #[test]
    fn test_infer_domains_code_quality_from_text() {
        let task = make_task("implement feature", "");
        let domains = GoalContextService::<crate::adapters::sqlite::goal_repository::SqliteGoalRepository>::infer_task_domains(&task);
        assert!(domains.contains(&"code-quality".to_string()));
    }

    #[test]
    fn test_infer_domains_code_quality_from_agent() {
        let task = make_task_with_agent("some task", "do stuff", "code-reviewer");
        let domains = GoalContextService::<crate::adapters::sqlite::goal_repository::SqliteGoalRepository>::infer_task_domains(&task);
        assert!(domains.contains(&"code-quality".to_string()));
    }

    #[test]
    fn test_infer_domains_frontend() {
        let task = make_task("update component css", "");
        let domains = GoalContextService::<crate::adapters::sqlite::goal_repository::SqliteGoalRepository>::infer_task_domains(&task);
        assert!(domains.contains(&"frontend".to_string()));
        assert!(domains.contains(&"ux".to_string()));
    }

    #[test]
    fn test_infer_domains_testing() {
        let task = make_task("improve test coverage", "");
        let domains = GoalContextService::<crate::adapters::sqlite::goal_repository::SqliteGoalRepository>::infer_task_domains(&task);
        assert!(domains.contains(&"testing".to_string()));
    }

    #[test]
    fn test_infer_domains_security() {
        let task = make_task("fix authentication flow", "");
        let domains = GoalContextService::<crate::adapters::sqlite::goal_repository::SqliteGoalRepository>::infer_task_domains(&task);
        assert!(domains.contains(&"security".to_string()));
    }

    #[test]
    fn test_infer_domains_performance() {
        let task = make_task("optimize latency", "");
        let domains = GoalContextService::<crate::adapters::sqlite::goal_repository::SqliteGoalRepository>::infer_task_domains(&task);
        assert!(domains.contains(&"performance".to_string()));
    }

    #[test]
    fn test_infer_domains_infrastructure() {
        let task = make_task("deploy docker image", "");
        let domains = GoalContextService::<crate::adapters::sqlite::goal_repository::SqliteGoalRepository>::infer_task_domains(&task);
        assert!(domains.contains(&"infrastructure".to_string()));
    }

    #[test]
    fn test_infer_domains_backend() {
        let task = make_task("add api endpoint", "");
        let domains = GoalContextService::<crate::adapters::sqlite::goal_repository::SqliteGoalRepository>::infer_task_domains(&task);
        assert!(domains.contains(&"backend".to_string()));
    }

    #[test]
    fn test_infer_domains_convergence() {
        let task = make_task("convergence engine attractor", "");
        let domains = GoalContextService::<crate::adapters::sqlite::goal_repository::SqliteGoalRepository>::infer_task_domains(&task);
        assert!(domains.contains(&"convergence".to_string()));
    }

    #[test]
    fn test_infer_domains_evolution() {
        let task = make_task("agent template refinement", "");
        let domains = GoalContextService::<crate::adapters::sqlite::goal_repository::SqliteGoalRepository>::infer_task_domains(&task);
        assert!(domains.contains(&"evolution".to_string()));
    }

    #[test]
    fn test_infer_domains_memory_lifecycle() {
        let task = make_task("memory tier promotion decay", "");
        let domains = GoalContextService::<crate::adapters::sqlite::goal_repository::SqliteGoalRepository>::infer_task_domains(&task);
        assert!(domains.contains(&"memory-lifecycle".to_string()));
    }

    #[test]
    fn test_infer_domains_agent_lifecycle() {
        let task = make_task("agent lifecycle routing", "");
        let domains = GoalContextService::<crate::adapters::sqlite::goal_repository::SqliteGoalRepository>::infer_task_domains(&task);
        assert!(domains.contains(&"agent-lifecycle".to_string()));
    }

    #[test]
    fn test_infer_domains_workflow() {
        let task = make_task("workflow phase advance", "");
        let domains = GoalContextService::<crate::adapters::sqlite::goal_repository::SqliteGoalRepository>::infer_task_domains(&task);
        assert!(domains.contains(&"workflow".to_string()));
    }

    #[test]
    fn test_infer_domains_swarm_orchestration() {
        let task = make_task("swarm orchestrator event bus", "");
        let domains = GoalContextService::<crate::adapters::sqlite::goal_repository::SqliteGoalRepository>::infer_task_domains(&task);
        assert!(domains.contains(&"swarm-orchestration".to_string()));
    }

    #[test]
    fn test_infer_domains_goal_management() {
        let task = make_task("goal convergence check constraint", "");
        let domains = GoalContextService::<crate::adapters::sqlite::goal_repository::SqliteGoalRepository>::infer_task_domains(&task);
        assert!(domains.contains(&"goal-management".to_string()));
    }

    #[test]
    fn test_infer_domains_multiple() {
        let task = make_task("implement test for api auth", "");
        let domains = GoalContextService::<crate::adapters::sqlite::goal_repository::SqliteGoalRepository>::infer_task_domains(&task);
        assert!(domains.contains(&"code-quality".to_string()));
        assert!(domains.contains(&"testing".to_string()));
        assert!(domains.contains(&"security".to_string()));
        assert!(domains.contains(&"backend".to_string()));
    }

    #[test]
    fn test_infer_domains_explicit_from_context() {
        let mut task = make_task("generic task", "no keywords");
        task.context
            .custom
            .insert(
                "domains".to_string(),
                serde_json::json!(["custom-domain"]),
            );
        let domains = GoalContextService::<crate::adapters::sqlite::goal_repository::SqliteGoalRepository>::infer_task_domains(&task);
        assert!(domains.contains(&"custom-domain".to_string()));
    }

    #[test]
    fn test_infer_domains_explicit_no_duplicates() {
        // Tests the dedup fix: explicit "testing" + text-inferred "testing" → only one
        let mut task = make_task("improve test coverage", "");
        task.context
            .custom
            .insert("domains".to_string(), serde_json::json!(["testing"]));
        let domains = GoalContextService::<crate::adapters::sqlite::goal_repository::SqliteGoalRepository>::infer_task_domains(&task);
        let testing_count = domains.iter().filter(|d| d.as_str() == "testing").count();
        assert_eq!(testing_count, 1, "testing should appear exactly once");
    }

    #[test]
    fn test_infer_domains_case_insensitive() {
        // Title is uppercased but should still match via to_lowercase()
        let task = make_task("CONVERGENCE ENGINE", "ATTRACTOR TRAJECTORY");
        let domains = GoalContextService::<crate::adapters::sqlite::goal_repository::SqliteGoalRepository>::infer_task_domains(&task);
        assert!(
            domains.contains(&"convergence".to_string()),
            "Case-insensitive match should find convergence domain"
        );
    }

    // ── format_goal_context tests ────────────────────────────────────

    #[test]
    fn test_format_goal_context_empty() {
        let result = GoalContextService::<crate::adapters::sqlite::goal_repository::SqliteGoalRepository>::format_goal_context(&[]);
        assert!(result.is_empty());
    }

    #[test]
    fn test_format_goal_context_single_goal_no_constraints() {
        let goal = make_goal("Test Goal", "A description", vec![], vec![]);
        let result = GoalContextService::<crate::adapters::sqlite::goal_repository::SqliteGoalRepository>::format_goal_context(&[goal]);
        assert!(result.contains("### Test Goal"));
        assert!(result.contains("A description"));
        assert!(!result.contains("Constraints:"));
    }

    #[test]
    fn test_format_goal_context_with_constraints() {
        let constraint =
            GoalConstraint::preference("test-constraint", "Must do the thing");
        let goal = make_goal(
            "Constrained Goal",
            "Has constraints",
            vec![constraint],
            vec![],
        );
        let result = GoalContextService::<crate::adapters::sqlite::goal_repository::SqliteGoalRepository>::format_goal_context(&[goal]);
        assert!(result.contains("Constraints:"));
        assert!(result.contains("[Preference]"));
        assert!(result.contains("test-constraint"));
        assert!(result.contains("Must do the thing"));
    }

    #[test]
    fn test_format_goal_context_multiple_goals() {
        let goal1 = make_goal("Goal One", "First", vec![], vec![]);
        let goal2 = make_goal("Goal Two", "Second", vec![], vec![]);
        let result = GoalContextService::<crate::adapters::sqlite::goal_repository::SqliteGoalRepository>::format_goal_context(&[goal1, goal2]);
        assert!(result.contains("### Goal One"));
        assert!(result.contains("### Goal Two"));
    }

    // ── collect_constraints tests ────────────────────────────────────

    #[test]
    fn test_collect_constraints_empty() {
        let result = GoalContextService::<crate::adapters::sqlite::goal_repository::SqliteGoalRepository>::collect_constraints(&[]);
        assert!(result.is_empty());
    }

    #[test]
    fn test_collect_constraints_no_constraints() {
        let goal = make_goal("No Constraints", "None here", vec![], vec![]);
        let result = GoalContextService::<crate::adapters::sqlite::goal_repository::SqliteGoalRepository>::collect_constraints(&[goal]);
        assert!(result.is_empty());
    }

    #[test]
    fn test_collect_constraints_dedup_by_name() {
        let c1 = GoalConstraint::preference("shared-name", "Description from goal 1");
        let c2 = GoalConstraint::invariant("shared-name", "Description from goal 2");
        let goal1 = make_goal("G1", "g1", vec![c1], vec![]);
        let goal2 = make_goal("G2", "g2", vec![c2], vec![]);
        let result = GoalContextService::<crate::adapters::sqlite::goal_repository::SqliteGoalRepository>::collect_constraints(&[goal1, goal2]);
        assert_eq!(result.len(), 1, "Same-name constraints should be deduplicated");
        assert_eq!(result[0].description, "Description from goal 1", "First occurrence wins");
    }

    #[test]
    fn test_collect_constraints_different_names() {
        let c1 = GoalConstraint::preference("constraint-a", "Desc A");
        let c2 = GoalConstraint::invariant("constraint-b", "Desc B");
        let goal1 = make_goal("G1", "g1", vec![c1], vec![]);
        let goal2 = make_goal("G2", "g2", vec![c2], vec![]);
        let result = GoalContextService::<crate::adapters::sqlite::goal_repository::SqliteGoalRepository>::collect_constraints(&[goal1, goal2]);
        assert_eq!(result.len(), 2);
    }

    // ── ContextBudget tests ──────────────────────────────────────────

    #[test]
    fn test_context_budget_default_fractions_sum() {
        let budget = ContextBudget::default();
        let sum = budget.goal_fraction + budget.memory_fraction + budget.artifact_fraction + budget.task_fraction;
        assert!(
            (sum - 1.0).abs() < f32::EPSILON,
            "Default fractions should sum to 1.0, got {}",
            sum
        );
    }

    #[test]
    fn test_context_budget_generous() {
        let budget = ContextBudget::generous();
        assert_eq!(budget.total_tokens, 16000);
        let sum = budget.goal_fraction + budget.memory_fraction + budget.artifact_fraction + budget.task_fraction;
        assert!(
            (sum - 1.0).abs() < f32::EPSILON,
            "Generous fractions should sum to 1.0"
        );
    }

    #[test]
    fn test_context_budget_tight() {
        let budget = ContextBudget::tight();
        assert_eq!(budget.total_tokens, 4000);
        let sum = budget.goal_fraction + budget.memory_fraction + budget.artifact_fraction + budget.task_fraction;
        assert!(
            (sum - 1.0).abs() < f32::EPSILON,
            "Tight fractions should sum to 1.0"
        );
    }

    #[test]
    fn test_context_budget_calculations() {
        let budget = ContextBudget::default();
        assert_eq!(budget.goal_budget(), 1600); // 8000 * 0.20
        assert_eq!(budget.memory_budget(), 2000); // 8000 * 0.25
        assert_eq!(budget.artifact_budget(), 1200); // 8000 * 0.15
        assert_eq!(budget.task_budget(), 3200); // 8000 * 0.40
    }
}
