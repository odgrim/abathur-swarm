//! Meta-Planner service for agent creation and evolution.
//!
//! The meta-planner enables the swarm to create new agent templates,
//! decompose goals into tasks, and improve agent performance over time.
//!
//! Supports both heuristic decomposition and LLM-powered decomposition
//! using Claude Code CLI.

use std::sync::Arc;
use uuid::Uuid;

use crate::domain::errors::{DomainError, DomainResult};
use crate::domain::models::{
    AgentTemplate, AgentTier, Task, TaskPriority, ToolCapability,
};
use crate::domain::ports::{AgentRepository, GoalRepository, TaskRepository};
use crate::services::llm_planner::{LlmPlanner, LlmPlannerConfig, PlanningContext};

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
            use_llm_decomposition: false,
            llm_config: None,
        }
    }
}

/// A task decomposition plan.
#[derive(Debug, Clone)]
pub struct DecompositionPlan {
    /// Goal being decomposed.
    pub goal_id: Uuid,
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
    goal_repo: Arc<G>,
    task_repo: Arc<T>,
    agent_repo: Arc<A>,
    config: MetaPlannerConfig,
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
            goal_repo,
            task_repo,
            agent_repo,
            config,
        }
    }

    /// Decompose a goal into tasks.
    ///
    /// If LLM decomposition is enabled, uses Claude to intelligently
    /// analyze the goal and create a detailed task DAG. Otherwise,
    /// falls back to simple heuristic decomposition.
    pub async fn decompose_goal(&self, goal_id: Uuid) -> DomainResult<DecompositionPlan> {
        if self.config.use_llm_decomposition {
            // Use LLM-based decomposition
            self.decompose_goal_with_llm(goal_id, None).await
        } else {
            // Use heuristic decomposition
            self.decompose_goal_heuristic(goal_id).await
        }
    }

    /// Decompose a goal using LLM (Claude Code CLI).
    ///
    /// This method uses the LlmPlanner to intelligently decompose the goal
    /// into a detailed task DAG with proper dependencies.
    pub async fn decompose_goal_with_llm(
        &self,
        goal_id: Uuid,
        context: Option<PlanningContext>,
    ) -> DomainResult<DecompositionPlan> {
        let goal = self.goal_repo.get(goal_id).await?
            .ok_or(DomainError::GoalNotFound(goal_id))?;

        // Get LLM config or use defaults
        let llm_config = self.config.llm_config.clone()
            .unwrap_or_else(LlmPlannerConfig::default);

        let llm_planner = LlmPlanner::new(llm_config);

        // Build planning context
        let planning_context = if let Some(ctx) = context {
            ctx
        } else {
            // Get existing agent types for context
            use crate::domain::ports::AgentFilter;
            let agents = self.agent_repo.list_templates(AgentFilter::default()).await?;
            let agent_names: Vec<String> = agents.iter().map(|a| a.name.clone()).collect();

            PlanningContext::new().with_agents(agent_names)
        };

        // Get LLM decomposition
        let decomposition = llm_planner.decompose_goal(&goal, &planning_context).await?;

        // Convert to internal TaskSpec format
        let tasks = llm_planner.to_task_specs(&decomposition);
        let complexity = llm_planner.parse_complexity(&decomposition.complexity);

        // Limit tasks if configured
        let tasks = if tasks.len() > self.config.max_tasks_per_decomposition {
            tasks.into_iter()
                .take(self.config.max_tasks_per_decomposition)
                .collect()
        } else {
            tasks
        };

        Ok(DecompositionPlan {
            goal_id,
            tasks,
            required_agents: decomposition.required_capabilities,
            estimated_complexity: complexity,
        })
    }

    /// Decompose a goal using simple heuristics.
    ///
    /// Creates a single task from the goal. Use this as a fallback
    /// when LLM decomposition is unavailable or for simple goals.
    pub async fn decompose_goal_heuristic(&self, goal_id: Uuid) -> DomainResult<DecompositionPlan> {
        let goal = self.goal_repo.get(goal_id).await?
            .ok_or(DomainError::GoalNotFound(goal_id))?;

        // Create a simple single-task decomposition
        let task_spec = TaskSpec {
            title: format!("Implement: {}", goal.name),
            description: goal.description.clone(),
            priority: match goal.priority {
                crate::domain::models::GoalPriority::Low => TaskPriority::Low,
                crate::domain::models::GoalPriority::Normal => TaskPriority::Normal,
                crate::domain::models::GoalPriority::High => TaskPriority::High,
                crate::domain::models::GoalPriority::Critical => TaskPriority::Critical,
            },
            agent_type: Some("default".to_string()),
            depends_on_indices: vec![],
            needs_worktree: true,
        };

        let tasks = vec![task_spec];
        let complexity = Complexity::from_task_count(tasks.len());

        Ok(DecompositionPlan {
            goal_id,
            tasks,
            required_agents: vec!["default".to_string()],
            estimated_complexity: complexity,
        })
    }

    /// Execute a decomposition plan by creating the tasks.
    pub async fn execute_plan(&self, plan: &DecompositionPlan) -> DomainResult<Vec<Task>> {
        let mut created_tasks = Vec::new();
        let mut task_id_map: std::collections::HashMap<usize, Uuid> = std::collections::HashMap::new();

        for (idx, spec) in plan.tasks.iter().enumerate() {
            // Build dependencies from previously created tasks
            let mut depends_on = Vec::new();
            for &dep_idx in &spec.depends_on_indices {
                if let Some(&dep_id) = task_id_map.get(&dep_idx) {
                    depends_on.push(dep_id);
                }
            }

            // Create the task
            let mut task = Task::new(&spec.title, &spec.description)
                .with_goal(plan.goal_id)
                .with_priority(spec.priority);

            if let Some(ref agent) = spec.agent_type {
                task = task.with_agent(agent);
            }

            for dep_id in depends_on {
                task = task.with_dependency(dep_id);
            }

            task.validate().map_err(DomainError::ValidationFailed)?;
            self.task_repo.create(&task).await?;

            task_id_map.insert(idx, task.id);
            created_tasks.push(task);
        }

        Ok(created_tasks)
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
    pub async fn analyze_agent_performance(&self, agent_name: &str) -> DomainResult<AgentMetrics> {
        // In a real system, this would query execution history
        // For now, return placeholder metrics
        let _agent = self.agent_repo.get_template_by_name(agent_name).await?
            .ok_or_else(|| DomainError::ValidationFailed(format!("Agent not found: {}", agent_name)))?;

        // Placeholder - would need execution history tracking
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
        create_test_pool, SqliteAgentRepository, SqliteGoalRepository, SqliteTaskRepository,
        Migrator, all_embedded_migrations,
    };

    async fn setup_meta_planner() -> MetaPlanner<SqliteGoalRepository, SqliteTaskRepository, SqliteAgentRepository> {
        let pool = create_test_pool().await.unwrap();
        let migrator = Migrator::new(pool.clone());
        migrator.run_embedded_migrations(all_embedded_migrations()).await.unwrap();

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
