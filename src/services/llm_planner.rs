//! LLM-powered planning for goal decomposition.
//!
//! Uses Claude (via Claude Code CLI or direct API) to intelligently
//! decompose goals into executable task DAGs.

use serde::{Deserialize, Serialize};
use std::process::Stdio;
use tokio::process::Command;
use uuid::Uuid;

use crate::domain::errors::{DomainError, DomainResult};
use crate::domain::models::{Goal, TaskPriority};
use crate::services::meta_planner::{Complexity, TaskSpec};

/// Configuration for the LLM planner.
#[derive(Debug, Clone)]
pub struct LlmPlannerConfig {
    /// Use Claude Code CLI (true) or direct API (false).
    pub use_claude_code: bool,
    /// Maximum tokens for the response.
    pub max_tokens: u32,
    /// Model to use (e.g., "claude-opus-4-6-20250616").
    pub model: String,
    /// Temperature for generation (0.0-1.0).
    pub temperature: f32,
    /// Path to Claude Code CLI.
    pub claude_code_path: Option<String>,
    /// API key for direct API usage (optional if using Claude Code).
    pub api_key: Option<String>,
}

impl Default for LlmPlannerConfig {
    fn default() -> Self {
        Self {
            use_claude_code: true,
            max_tokens: 4096,
            model: "claude-opus-4-6-20250616".to_string(),
            temperature: 0.3,
            claude_code_path: None,
            api_key: None,
        }
    }
}

/// LLM-generated decomposition plan.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmDecomposition {
    /// Analysis of the goal.
    pub analysis: String,
    /// Generated tasks.
    pub tasks: Vec<LlmTaskSpec>,
    /// Required capabilities/skills.
    pub required_capabilities: Vec<String>,
    /// Estimated complexity.
    pub complexity: String,
    /// Any concerns or risks identified.
    pub concerns: Vec<String>,
}

/// LLM-generated task specification.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmTaskSpec {
    /// Task title.
    pub title: String,
    /// Task description.
    pub description: String,
    /// Priority (low, normal, high, critical).
    pub priority: String,
    /// Suggested agent type.
    pub agent_type: String,
    /// Dependencies (titles of other tasks).
    pub depends_on: Vec<String>,
    /// Whether this task modifies code.
    pub modifies_code: bool,
    /// Acceptance criteria.
    pub acceptance_criteria: Vec<String>,
}

impl LlmTaskSpec {
    /// Convert to internal TaskSpec format.
    pub fn to_task_spec(&self, task_index_map: &std::collections::HashMap<String, usize>) -> TaskSpec {
        let priority = match self.priority.to_lowercase().as_str() {
            "low" => TaskPriority::Low,
            "high" => TaskPriority::High,
            "critical" => TaskPriority::Critical,
            _ => TaskPriority::Normal,
        };

        let depends_on_indices: Vec<usize> = self
            .depends_on
            .iter()
            .filter_map(|title| task_index_map.get(title).copied())
            .collect();

        TaskSpec {
            title: self.title.clone(),
            description: self.description.clone(),
            priority,
            agent_type: Some(self.agent_type.clone()),
            depends_on_indices,
            needs_worktree: self.modifies_code,
        }
    }
}

/// LLM Planner for goal decomposition.
pub struct LlmPlanner {
    config: LlmPlannerConfig,
    http_client: reqwest::Client,
}

impl LlmPlanner {
    pub fn new(config: LlmPlannerConfig) -> Self {
        let http_client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(120))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        Self { config, http_client }
    }

    pub fn with_default_config() -> Self {
        Self::new(LlmPlannerConfig::default())
    }

    /// Decompose a goal using the LLM.
    pub async fn decompose_goal(
        &self,
        goal: &Goal,
        context: &PlanningContext,
    ) -> DomainResult<LlmDecomposition> {
        let prompt = self.build_decomposition_prompt(goal, context);

        let response = if self.config.use_claude_code {
            self.query_claude_code(&prompt).await?
        } else {
            self.query_direct_api(&prompt).await?
        };

        self.parse_decomposition(&response)
    }

    /// Build the decomposition prompt.
    fn build_decomposition_prompt(&self, goal: &Goal, context: &PlanningContext) -> String {
        let constraints_text = if goal.constraints.is_empty() {
            "None specified".to_string()
        } else {
            goal.constraints
                .iter()
                .map(|c| format!("- {}: {}", c.name, c.description))
                .collect::<Vec<_>>()
                .join("\n")
        };

        let existing_agents_text = if context.existing_agents.is_empty() {
            "None registered".to_string()
        } else {
            context.existing_agents.join(", ")
        };

        let memory_patterns_text = if context.memory_patterns.is_empty() {
            "No historical patterns available".to_string()
        } else {
            context.memory_patterns
                .iter()
                .enumerate()
                .map(|(i, p)| format!("{}. {}", i + 1, p))
                .collect::<Vec<_>>()
                .join("\n")
        };

        format!(
            r#"You are a task decomposition assistant for a multi-agent swarm system.

## Goal to Decompose
ID: {}
Name: {}
Description: {}
Priority: {:?}

## Constraints
{}

## Available Agent Types
{}

## Project Context
{}

## Historical Patterns (from memory)
{}

## Instructions
Analyze this goal and decompose it into a set of executable tasks. Consider:

1. **Task Granularity**: Each task should be atomic and achievable by a single agent in one session.
2. **Dependencies**: Identify which tasks must complete before others can start.
3. **Agent Selection**: Choose appropriate agent types for each task.
4. **Parallelization**: Maximize opportunities for parallel execution where safe.
5. **Verification**: Include verification tasks where appropriate.
6. **Historical Patterns**: Learn from the patterns above when applicable.

## Required Output Format (JSON)
Respond with a JSON object containing:
```json
{{
  "analysis": "Brief analysis of the goal",
  "tasks": [
    {{
      "title": "Short task title",
      "description": "Detailed description of what needs to be done",
      "priority": "normal|low|high|critical",
      "agent_type": "suggested agent type",
      "depends_on": ["titles of dependent tasks"],
      "modifies_code": true|false,
      "acceptance_criteria": ["criterion 1", "criterion 2"]
    }}
  ],
  "required_capabilities": ["capability1", "capability2"],
  "complexity": "trivial|simple|moderate|complex|very_complex",
  "concerns": ["any risks or concerns"]
}}
```

IMPORTANT: Output ONLY the JSON object, no other text."#,
            goal.id,
            goal.name,
            goal.description,
            goal.priority,
            constraints_text,
            existing_agents_text,
            context.project_context.as_deref().unwrap_or("No additional context"),
            memory_patterns_text
        )
    }

    /// Query Claude Code CLI.
    async fn query_claude_code(&self, prompt: &str) -> DomainResult<String> {
        let claude_path = self
            .config
            .claude_code_path
            .clone()
            .unwrap_or_else(|| "claude".to_string());

        let mut cmd = Command::new(claude_path);
        cmd.arg("--print")
            .arg("--output-format")
            .arg("text")
            .arg(prompt)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let output = cmd.output().await.map_err(|e| {
            DomainError::ValidationFailed(format!("Failed to execute Claude Code: {}", e))
        })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(DomainError::ValidationFailed(format!(
                "Claude Code failed: {}",
                stderr
            )));
        }

        let response = String::from_utf8_lossy(&output.stdout).to_string();
        Ok(response)
    }

    /// Query the Anthropic Messages API directly.
    async fn query_direct_api(&self, prompt: &str) -> DomainResult<String> {
        let api_key = self.config.api_key.clone()
            .or_else(|| std::env::var("ANTHROPIC_API_KEY").ok())
            .ok_or_else(|| DomainError::ValidationFailed(
                "API key required for direct API mode. Set api_key in config or ANTHROPIC_API_KEY env var.".to_string()
            ))?;

        let request_body = serde_json::json!({
            "model": self.config.model,
            "max_tokens": self.config.max_tokens,
            "temperature": self.config.temperature,
            "messages": [
                {
                    "role": "user",
                    "content": prompt
                }
            ]
        });

        let response = self.http_client
            .post("https://api.anthropic.com/v1/messages")
            .header("content-type", "application/json")
            .header("x-api-key", &api_key)
            .header("anthropic-version", "2023-06-01")
            .json(&request_body)
            .send()
            .await
            .map_err(|e| DomainError::ValidationFailed(
                format!("Direct API request failed: {}", e)
            ))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(DomainError::ValidationFailed(
                format!("Anthropic API error {}: {}", status, body)
            ));
        }

        // Parse the Messages API response
        let result: serde_json::Value = response.json().await
            .map_err(|e| DomainError::ValidationFailed(
                format!("Failed to parse API response: {}", e)
            ))?;

        // Extract text from content blocks
        let text = result["content"]
            .as_array()
            .map(|blocks| {
                blocks.iter()
                    .filter_map(|block| {
                        if block["type"].as_str() == Some("text") {
                            block["text"].as_str().map(String::from)
                        } else {
                            None
                        }
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .unwrap_or_default();

        if text.is_empty() {
            return Err(DomainError::ValidationFailed(
                "API returned no text content".to_string()
            ));
        }

        Ok(text)
    }

    /// Parse the LLM response into a decomposition.
    fn parse_decomposition(&self, response: &str) -> DomainResult<LlmDecomposition> {
        // Try to extract JSON from the response
        let json_str = super::extract_json_from_response(response);

        serde_json::from_str(&json_str).map_err(|e| {
            DomainError::ValidationFailed(format!(
                "Failed to parse LLM response as JSON: {}. Response: {}",
                e, response
            ))
        })
    }

    /// Extract JSON from the response (handles markdown code blocks).
    /// Convert LLM decomposition to internal TaskSpec format.
    pub fn to_task_specs(&self, decomposition: &LlmDecomposition) -> Vec<TaskSpec> {
        // Build task index map for dependencies
        let task_index_map: std::collections::HashMap<String, usize> = decomposition
            .tasks
            .iter()
            .enumerate()
            .map(|(i, t)| (t.title.clone(), i))
            .collect();

        decomposition
            .tasks
            .iter()
            .map(|t| t.to_task_spec(&task_index_map))
            .collect()
    }

    /// Estimate complexity from LLM response.
    pub fn parse_complexity(&self, complexity_str: &str) -> Complexity {
        match complexity_str.to_lowercase().as_str() {
            "trivial" => Complexity::Trivial,
            "simple" => Complexity::Simple,
            "moderate" => Complexity::Moderate,
            "complex" => Complexity::Complex,
            "very_complex" | "very complex" => Complexity::VeryComplex,
            _ => Complexity::Moderate,
        }
    }
}

/// Context for planning.
#[derive(Debug, Clone, Default)]
pub struct PlanningContext {
    /// Existing agent types available.
    pub existing_agents: Vec<String>,
    /// Project-specific context.
    pub project_context: Option<String>,
    /// Related goals for holistic evaluation.
    pub related_goals: Vec<Uuid>,
    /// Codebase structure summary.
    pub codebase_summary: Option<String>,
    /// Patterns from memory that may help with decomposition.
    pub memory_patterns: Vec<String>,
}

impl PlanningContext {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_agents(mut self, agents: Vec<String>) -> Self {
        self.existing_agents = agents;
        self
    }

    pub fn with_project_context(mut self, context: String) -> Self {
        self.project_context = Some(context);
        self
    }

    pub fn with_codebase_summary(mut self, summary: String) -> Self {
        self.codebase_summary = Some(summary);
        self
    }

    pub fn with_related_goals(mut self, goals: Vec<Uuid>) -> Self {
        self.related_goals = goals;
        self
    }

    pub fn with_memory_patterns(mut self, patterns: Vec<String>) -> Self {
        self.memory_patterns = patterns;
        self
    }
}

/// Agent refinement suggestion from LLM analysis.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentRefinementSuggestion {
    /// Agent name being refined.
    pub agent_name: String,
    /// Analysis of current performance.
    pub analysis: String,
    /// Suggested prompt improvements.
    pub prompt_improvements: Vec<String>,
    /// Suggested tool additions.
    pub suggested_tools: Vec<String>,
    /// Suggested constraints.
    pub suggested_constraints: Vec<String>,
    /// Confidence in suggestions (0.0-1.0).
    pub confidence: f32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_json_plain() {
        let input = r#"{"analysis": "test"}"#;
        assert_eq!(crate::services::extract_json_from_response(input), r#"{"analysis": "test"}"#);
    }

    #[test]
    fn test_extract_json_code_block() {
        let input = "```json\n{\"analysis\": \"test\"}\n```";
        assert_eq!(crate::services::extract_json_from_response(input), r#"{"analysis": "test"}"#);
    }

    #[test]
    fn test_parse_complexity() {
        let planner = LlmPlanner::with_default_config();
        assert_eq!(planner.parse_complexity("trivial"), Complexity::Trivial);
        assert_eq!(planner.parse_complexity("simple"), Complexity::Simple);
        assert_eq!(planner.parse_complexity("moderate"), Complexity::Moderate);
        assert_eq!(planner.parse_complexity("complex"), Complexity::Complex);
        assert_eq!(planner.parse_complexity("very_complex"), Complexity::VeryComplex);
        assert_eq!(planner.parse_complexity("unknown"), Complexity::Moderate);
    }

    #[test]
    fn test_parse_decomposition() {
        let planner = LlmPlanner::with_default_config();
        let json = r#"{
            "analysis": "Test goal analysis",
            "tasks": [
                {
                    "title": "Task 1",
                    "description": "Do something",
                    "priority": "normal",
                    "agent_type": "code-writer",
                    "depends_on": [],
                    "modifies_code": true,
                    "acceptance_criteria": ["Works"]
                }
            ],
            "required_capabilities": ["coding"],
            "complexity": "simple",
            "concerns": []
        }"#;

        let decomp = planner.parse_decomposition(json).unwrap();
        assert_eq!(decomp.analysis, "Test goal analysis");
        assert_eq!(decomp.tasks.len(), 1);
        assert_eq!(decomp.tasks[0].title, "Task 1");
    }

    #[test]
    fn test_to_task_specs() {
        let planner = LlmPlanner::with_default_config();
        let decomp = LlmDecomposition {
            analysis: "Test".to_string(),
            tasks: vec![
                LlmTaskSpec {
                    title: "Task A".to_string(),
                    description: "First task".to_string(),
                    priority: "high".to_string(),
                    agent_type: "implementer".to_string(),
                    depends_on: vec![],
                    modifies_code: true,
                    acceptance_criteria: vec!["Done".to_string()],
                },
                LlmTaskSpec {
                    title: "Task B".to_string(),
                    description: "Second task".to_string(),
                    priority: "normal".to_string(),
                    agent_type: "tester".to_string(),
                    depends_on: vec!["Task A".to_string()],
                    modifies_code: false,
                    acceptance_criteria: vec!["Tested".to_string()],
                },
            ],
            required_capabilities: vec!["coding".to_string()],
            complexity: "simple".to_string(),
            concerns: vec![],
        };

        let specs = planner.to_task_specs(&decomp);
        assert_eq!(specs.len(), 2);
        assert_eq!(specs[0].title, "Task A");
        assert!(specs[0].needs_worktree);
        assert_eq!(specs[1].depends_on_indices, vec![0]);
    }

    #[test]
    fn test_planning_context_builder() {
        let ctx = PlanningContext::new()
            .with_agents(vec!["agent-a".to_string()])
            .with_project_context("Test project".to_string())
            .with_codebase_summary("Rust project".to_string());

        assert_eq!(ctx.existing_agents, vec!["agent-a"]);
        assert_eq!(ctx.project_context, Some("Test project".to_string()));
        assert_eq!(ctx.codebase_summary, Some("Rust project".to_string()));
    }
}
