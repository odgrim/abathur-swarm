//! Meta-Planner service for agent creation and evolution.
//!
//! The meta-planner enables the swarm to create new agent templates,
//! decompose goals into tasks, and improve agent performance over time.
//!
//! Supports both heuristic decomposition and LLM-powered decomposition
//! using Claude Code CLI.

use std::sync::Arc;

use crate::domain::errors::{DomainError, DomainResult};
use crate::domain::models::{
    AgentTemplate, AgentTier, TaskPriority, ToolCapability,
};
use crate::domain::ports::{AgentRepository, GoalRepository, MemoryRepository, TaskRepository};
use crate::services::llm_planner::LlmPlannerConfig;
use crate::services::EvolutionLoop;

/// Configuration for the meta-planner.
#[derive(Debug, Clone)]
pub struct MetaPlannerConfig {
    /// Maximum decomposition depth.
    pub max_decomposition_depth: usize,
    /// Default agent tier for generated templates.
    pub default_agent_tier: AgentTier,
    /// Whether to auto-generate agent templates.
    pub auto_generate_agents: bool,
    /// Maximum tasks per goal decomposition.
    pub max_tasks_per_decomposition: usize,
    /// Whether to use LLM for decomposition (vs heuristic).
    pub use_llm_decomposition: bool,
    /// LLM planner configuration.
    pub llm_config: Option<LlmPlannerConfig>,
}

impl Default for MetaPlannerConfig {
    fn default() -> Self {
        Self {
            max_decomposition_depth: 3,
            default_agent_tier: AgentTier::Worker,
            auto_generate_agents: false,
            max_tasks_per_decomposition: 10,
            use_llm_decomposition: true,
            llm_config: None,
        }
    }
}

/// A task decomposition plan.
#[derive(Debug, Clone)]
pub struct DecompositionPlan {
    /// Generated tasks.
    pub tasks: Vec<TaskSpec>,
    /// Required agent types.
    pub required_agents: Vec<String>,
    /// Estimated complexity.
    pub estimated_complexity: Complexity,
}

/// Specification for a task to be created.
#[derive(Debug, Clone)]
pub struct TaskSpec {
    /// Task title.
    pub title: String,
    /// Task description.
    pub description: String,
    /// Priority level.
    pub priority: TaskPriority,
    /// Agent type to use.
    pub agent_type: Option<String>,
    /// Dependencies (indices into tasks vec).
    pub depends_on_indices: Vec<usize>,
    /// Whether this task needs a worktree.
    pub needs_worktree: bool,
}

/// Complexity estimate for a decomposition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Complexity {
    Trivial,
    Simple,
    Moderate,
    Complex,
    VeryComplex,
}

impl Complexity {
    pub fn from_task_count(count: usize) -> Self {
        match count {
            0..=1 => Self::Trivial,
            2..=3 => Self::Simple,
            4..=6 => Self::Moderate,
            7..=10 => Self::Complex,
            _ => Self::VeryComplex,
        }
    }
}

/// Specification for a new agent template.
#[derive(Debug, Clone)]
pub struct AgentSpec {
    /// Agent name.
    pub name: String,
    /// Agent description.
    pub description: String,
    /// Agent tier.
    pub tier: AgentTier,
    /// System prompt.
    pub system_prompt: String,
    /// Required tools.
    pub tools: Vec<ToolCapability>,
    /// Suggested max turns.
    pub max_turns: u32,
}

/// Performance metrics for an agent template.
#[derive(Debug, Clone, Default)]
pub struct AgentMetrics {
    /// Total tasks executed.
    pub total_tasks: u64,
    /// Tasks completed successfully.
    pub successful_tasks: u64,
    /// Tasks that failed.
    pub failed_tasks: u64,
    /// Average turns per task.
    pub avg_turns_per_task: f64,
    /// Average tokens per task.
    pub avg_tokens_per_task: f64,
    /// Success rate.
    pub success_rate: f64,
}

impl AgentMetrics {
    pub fn calculate_success_rate(&mut self) {
        if self.total_tasks > 0 {
            self.success_rate = self.successful_tasks as f64 / self.total_tasks as f64;
        }
    }
}

/// The Meta-Planner service.
pub struct MetaPlanner<G, T, A>
where
    G: GoalRepository + 'static,
    T: TaskRepository + 'static,
    A: AgentRepository + 'static,
{
    // Kept for API compatibility; may be used again when decomposition is re-added.
    _goal_repo: Arc<G>,
    _task_repo: Arc<T>,
    agent_repo: Arc<A>,
    memory_repo: Option<Arc<dyn MemoryRepository>>,
    config: MetaPlannerConfig,
    /// Optional Overmind service for Substrate-compatible LLM decomposition.
    overmind: Option<Arc<crate::services::OvermindService>>,
    /// Optional Evolution Loop for real agent performance metrics.
    evolution_loop: Option<Arc<EvolutionLoop>>,
}

impl<G, T, A> MetaPlanner<G, T, A>
where
    G: GoalRepository + 'static,
    T: TaskRepository + 'static,
    A: AgentRepository + 'static,
{
    pub fn new(
        goal_repo: Arc<G>,
        task_repo: Arc<T>,
        agent_repo: Arc<A>,
        config: MetaPlannerConfig,
    ) -> Self {
        Self {
            _goal_repo: goal_repo,
            _task_repo: task_repo,
            agent_repo,
            memory_repo: None,
            config,
            overmind: None,
            evolution_loop: None,
        }
    }

    /// Set the memory repository for pattern queries.
    pub fn with_memory_repo(mut self, memory_repo: Arc<dyn MemoryRepository>) -> Self {
        self.memory_repo = Some(memory_repo);
        self
    }

    /// Set the Overmind service for Substrate-compatible LLM decomposition.
    pub fn with_overmind(mut self, overmind: Arc<crate::services::OvermindService>) -> Self {
        self.overmind = Some(overmind);
        self
    }

    /// Set the Evolution Loop for real agent performance metrics.
    pub fn with_evolution_loop(mut self, evolution_loop: Arc<EvolutionLoop>) -> Self {
        self.evolution_loop = Some(evolution_loop);
        self
    }

    /// Query memory for relevant patterns to assist goal decomposition.
    ///
    /// Searches for:
    /// - Previous successful decomposition patterns
    /// - Agent performance data for similar tasks
    /// - Common task patterns for the goal type
    pub async fn query_decomposition_patterns(&self, goal_description: &str) -> DomainResult<Vec<String>> {
        let Some(ref memory_repo) = self.memory_repo else {
            return Ok(Vec::new());
        };

        // Search memory for patterns related to this goal
        let patterns = memory_repo
            .search(goal_description, Some("decomposition_patterns"), 10)
            .await?;

        let pattern_hints: Vec<String> = patterns
            .into_iter()
            .map(|m| m.content)
            .collect();

        Ok(pattern_hints)
    }

    /// Query memory for agent performance patterns.
    pub async fn query_agent_patterns(&self, agent_type: &str) -> DomainResult<Vec<String>> {
        let Some(ref memory_repo) = self.memory_repo else {
            return Ok(Vec::new());
        };

        // Search for agent-specific patterns
        let memories = memory_repo
            .search(agent_type, Some("agent_performance"), 5)
            .await?;

        Ok(memories.into_iter().map(|m| m.content).collect())
    }

    /// Generate a new agent template specification.
    pub fn generate_agent_spec(
        &self,
        name: &str,
        purpose: &str,
        tier: AgentTier,
    ) -> AgentSpec {
        let system_prompt = format!(
            "You are a specialized agent for: {}.\n\n\
            Your purpose is to execute tasks related to this domain efficiently.\n\
            Follow the constraints and guidelines provided with each task.",
            purpose
        );

        let tools = self.suggest_tools_for_purpose(purpose);

        AgentSpec {
            name: name.to_string(),
            description: purpose.to_string(),
            tier,
            system_prompt,
            tools,
            max_turns: tier.max_turns(),
        }
    }

    /// Create an agent template from a spec.
    pub async fn create_agent_from_spec(&self, spec: &AgentSpec) -> DomainResult<AgentTemplate> {
        let mut template = AgentTemplate::new(&spec.name, spec.tier)
            .with_description(&spec.description)
            .with_prompt(&spec.system_prompt)
            .with_max_turns(spec.max_turns);

        for tool in &spec.tools {
            template = template.with_tool(tool.clone());
        }

        self.agent_repo.create_template(&template).await?;
        Ok(template)
    }

    /// Suggest tools based on the agent's purpose.
    fn suggest_tools_for_purpose(&self, purpose: &str) -> Vec<ToolCapability> {
        let purpose_lower = purpose.to_lowercase();
        let mut tools = Vec::new();

        // Basic file operations are almost always needed
        tools.push(ToolCapability::new("Read", "Read file contents"));
        tools.push(ToolCapability::new("Glob", "Find files by pattern"));

        if purpose_lower.contains("code") || purpose_lower.contains("implement") {
            tools.push(ToolCapability::new("Edit", "Edit file contents"));
            tools.push(ToolCapability::new("Write", "Write new files"));
            tools.push(ToolCapability::new("Bash", "Execute shell commands"));
            tools.push(ToolCapability::new("Grep", "Search file contents"));
        }

        if purpose_lower.contains("test") {
            tools.push(ToolCapability::new("Bash", "Execute shell commands"));
        }

        if purpose_lower.contains("research") || purpose_lower.contains("search") {
            tools.push(ToolCapability::new("WebSearch", "Search the web"));
            tools.push(ToolCapability::new("WebFetch", "Fetch web content"));
        }

        if purpose_lower.contains("document") || purpose_lower.contains("write") {
            tools.push(ToolCapability::new("Edit", "Edit file contents"));
            tools.push(ToolCapability::new("Write", "Write new files"));
        }

        tools
    }

    /// Analyze agent performance and suggest improvements.
    ///
    /// Queries the EvolutionLoop for real execution statistics if available,
    /// falling back to default metrics when no evolution loop is configured.
    pub async fn analyze_agent_performance(&self, agent_name: &str) -> DomainResult<AgentMetrics> {
        let _agent = self.agent_repo.get_template_by_name(agent_name).await?
            .ok_or_else(|| DomainError::ValidationFailed(format!("Agent not found: {}", agent_name)))?;

        // Query real metrics from EvolutionLoop if available
        if let Some(ref evo) = self.evolution_loop
            && let Some(stats) = evo.get_stats(agent_name).await {
                return Ok(AgentMetrics {
                    total_tasks: stats.total_tasks as u64,
                    successful_tasks: stats.successful_tasks as u64,
                    failed_tasks: stats.failed_tasks as u64,
                    avg_turns_per_task: stats.avg_turns,
                    avg_tokens_per_task: stats.avg_tokens,
                    success_rate: stats.success_rate,
                });
            }

        Ok(AgentMetrics::default())
    }

    /// Check if an agent template exists for a given type.
    pub async fn agent_exists(&self, agent_type: &str) -> DomainResult<bool> {
        let agent = self.agent_repo.get_template_by_name(agent_type).await?;
        Ok(agent.is_some())
    }

    /// Get or create an agent template for a task type.
    pub async fn ensure_agent(&self, agent_type: &str, purpose: &str) -> DomainResult<AgentTemplate> {
        if let Some(existing) = self.agent_repo.get_template_by_name(agent_type).await? {
            return Ok(existing);
        }

        if self.config.auto_generate_agents {
            let spec = self.generate_agent_spec(agent_type, purpose, self.config.default_agent_tier);
            self.create_agent_from_spec(&spec).await
        } else {
            Err(DomainError::ValidationFailed(format!(
                "Agent template '{}' not found and auto-generation is disabled",
                agent_type
            )))
        }
    }

}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::sqlite::{
        create_migrated_test_pool, SqliteAgentRepository, SqliteGoalRepository, SqliteTaskRepository,
    };

    async fn setup_meta_planner() -> MetaPlanner<SqliteGoalRepository, SqliteTaskRepository, SqliteAgentRepository> {
        let pool = create_migrated_test_pool().await.unwrap();

        let goal_repo = Arc::new(SqliteGoalRepository::new(pool.clone()));
        let task_repo = Arc::new(SqliteTaskRepository::new(pool.clone()));
        let agent_repo = Arc::new(SqliteAgentRepository::new(pool));
        let config = MetaPlannerConfig::default();

        MetaPlanner::new(goal_repo, task_repo, agent_repo, config)
    }

    #[tokio::test]
    async fn test_complexity_estimation() {
        assert_eq!(Complexity::from_task_count(1), Complexity::Trivial);
        assert_eq!(Complexity::from_task_count(3), Complexity::Simple);
        assert_eq!(Complexity::from_task_count(5), Complexity::Moderate);
        assert_eq!(Complexity::from_task_count(8), Complexity::Complex);
        assert_eq!(Complexity::from_task_count(15), Complexity::VeryComplex);
    }

    #[tokio::test]
    async fn test_generate_agent_spec() {
        let planner = setup_meta_planner().await;

        let spec = planner.generate_agent_spec(
            "code-writer",
            "Write and implement code features",
            AgentTier::Worker,
        );

        assert_eq!(spec.name, "code-writer");
        assert!(!spec.tools.is_empty());
        assert!(spec.tools.iter().any(|t| t.name == "Edit"));
    }

    #[tokio::test]
    async fn test_suggest_tools() {
        let planner = setup_meta_planner().await;

        let code_tools = planner.suggest_tools_for_purpose("implement code feature");
        assert!(code_tools.iter().any(|t| t.name == "Edit"));
        assert!(code_tools.iter().any(|t| t.name == "Write"));

        let research_tools = planner.suggest_tools_for_purpose("research best practices");
        assert!(research_tools.iter().any(|t| t.name == "WebSearch"));
    }
}
