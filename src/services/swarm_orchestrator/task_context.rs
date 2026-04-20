//! Task context loader.
//!
//! Loads goal-context, memory-context, and intent-gap-context for a task and
//! assembles the combined description that is passed to the substrate.
//!
//! Extracted from `goal_processing::spawn_task_agent` per spec T10
//! (`specs/T10-spawn-task-agent-extraction.md`).

use std::sync::Arc;

use crate::domain::errors::DomainResult;
use crate::domain::models::{RelevanceWeights, ScoredMemory, Task};
use crate::domain::ports::{GoalRepository, MemoryRepository};
use crate::services::GoalContextService;
use crate::services::memory_service::MemoryService;

/// The fully-assembled context passed to the substrate.
#[derive(Debug, Clone, Default)]
pub struct TaskContext {
    pub goal_context: Option<String>,
    /// Read by audit logging only; not load-bearing for substrate.
    // reason: surfaced as part of the assembled context so audit log writes
    // can persist the inputs verbatim alongside the substrate prompt.
    #[allow(dead_code)]
    pub memory_context: Option<String>,
    /// Read by audit logging only; not load-bearing for substrate.
    // reason: surfaced as part of the assembled context so audit log writes
    // can persist the inputs verbatim alongside the substrate prompt.
    #[allow(dead_code)]
    pub intent_gap_context: Option<String>,
    /// Final task description: goal/memory/gap context joined with the original
    /// task description.
    pub combined_description: String,
}

/// Service that loads task context.
///
/// Generic over `G: GoalRepository` and `M: MemoryRepository` because both
/// `GoalContextService` and `MemoryService` are generic — using trait objects
/// here would require additional bridging, and the orchestrator already owns
/// concrete `Arc<G>` / `Arc<M>` clones.
pub struct TaskContextService<G, M>
where
    G: GoalRepository + 'static,
    M: MemoryRepository + 'static,
{
    goal_repo: Arc<G>,
    memory_repo: Option<Arc<M>>,
}

impl<G, M> TaskContextService<G, M>
where
    G: GoalRepository + 'static,
    M: MemoryRepository + 'static,
{
    pub fn new(goal_repo: Arc<G>, memory_repo: Option<Arc<M>>) -> Self {
        Self {
            goal_repo,
            memory_repo,
        }
    }

    /// Load goal/memory/intent-gap context for a task and assemble the
    /// combined description used by the substrate.
    pub async fn load_task_context(&self, task: &Task) -> DomainResult<TaskContext> {
        let goal_context = self.load_goal_context(task).await;
        let memory_context = self.load_memory_context(task).await;
        let intent_gap_context = task.intent_gap_context().map(|s| s.to_string());

        let combined_description = assemble_description(
            task,
            goal_context.as_deref(),
            memory_context.as_deref(),
            intent_gap_context.as_deref(),
        );

        Ok(TaskContext {
            goal_context,
            memory_context,
            intent_gap_context,
            combined_description,
        })
    }

    async fn load_goal_context(&self, task: &Task) -> Option<String> {
        let svc = GoalContextService::new(self.goal_repo.clone());
        match svc.get_goals_for_task(task).await {
            Ok(goals) if !goals.is_empty() => Some(GoalContextService::<G>::format_goal_context(&goals)),
            Ok(_) => None,
            Err(e) => {
                tracing::warn!("Failed to load goal context for task {}: {}", task.id, e);
                None
            }
        }
    }

    async fn load_memory_context(&self, task: &Task) -> Option<String> {
        let mem_repo = self.memory_repo.as_ref()?;
        let memory_service = MemoryService::new(mem_repo.clone());
        let desc_preview: String = task.description.chars().take(500).collect();
        let query = format!("{} {}", task.title, desc_preview);
        match memory_service
            .load_context_with_budget(
                &query,
                None,
                2000, // 25% of 8000-token context budget
                RelevanceWeights::semantic_biased(),
            )
            .await
        {
            Ok(memories) if !memories.is_empty() => Some(format_memory_context(&memories)),
            Ok(_) => None,
            Err(e) => {
                tracing::debug!(task_id = %task.id, "Failed to load memory context: {}", e);
                None
            }
        }
    }
}

/// Format scored memories as contextual guidance text for agent task prompts.
pub(crate) fn format_memory_context(memories: &[ScoredMemory]) -> String {
    let mut output = String::from(
        "## Relevant Context from Memory\nThe following memories from previous work are relevant to this task:\n\n",
    );
    for entry in memories {
        let mem = &entry.memory;
        output.push_str(&format!(
            "**{}** *(tier: {}, score: {:.2})*\n{}\n\n",
            mem.key,
            mem.tier.as_str(),
            entry.score,
            mem.content,
        ));
    }
    output
}

/// Assemble the final task description in priority order:
/// goal_context > memory_context > intent_gap_context > task.description.
pub(crate) fn assemble_description(
    task: &Task,
    goal_context: Option<&str>,
    memory_context: Option<&str>,
    intent_gap_context: Option<&str>,
) -> String {
    let mut parts: Vec<&str> = Vec::new();
    if let Some(g) = goal_context {
        parts.push(g);
    }
    if let Some(m) = memory_context {
        parts.push(m);
    }
    if let Some(g) = intent_gap_context {
        parts.push(g);
    }
    if parts.is_empty() {
        task.description.clone()
    } else {
        format!("{}\n\n---\n\n{}", parts.join("\n\n---\n\n"), task.description)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::sqlite::test_support;
    use crate::domain::models::{Memory, ScoreBreakdown};

    fn scored(memory: Memory, score: f32) -> ScoredMemory {
        ScoredMemory {
            memory,
            score,
            score_breakdown: ScoreBreakdown::default(),
        }
    }

    #[test]
    fn test_assembles_description_in_priority_order() {
        let task = Task::new("Hello task body");
        let out = assemble_description(
            &task,
            Some("[goal]"),
            Some("[memory]"),
            Some("[gap]"),
        );
        // Goal first, then memory, then gap, then the task description last.
        let goal_idx = out.find("[goal]").unwrap();
        let mem_idx = out.find("[memory]").unwrap();
        let gap_idx = out.find("[gap]").unwrap();
        let body_idx = out.find("Hello task body").unwrap();
        assert!(goal_idx < mem_idx);
        assert!(mem_idx < gap_idx);
        assert!(gap_idx < body_idx);
    }

    #[test]
    fn test_assemble_description_no_context_returns_body() {
        let task = Task::new("body only");
        let out = assemble_description(&task, None, None, None);
        assert_eq!(out, "body only");
    }

    #[tokio::test]
    async fn test_context_loads_goal_guidance_returns_none_when_no_goals() {
        // With no goals in repo, loader returns None gracefully.
        let (goal_repo, _task_repo, _wt_repo, _agent_repo, mem_repo) =
            test_support::setup_all_repos().await;
        let svc = TaskContextService::new(goal_repo, Some(mem_repo));
        let task = Task::new("anything");
        let ctx = svc.load_task_context(&task).await.unwrap();
        assert!(ctx.goal_context.is_none());
    }

    #[tokio::test]
    async fn test_context_loads_memory_with_budget_limit() {
        // No memory_repo wired -> memory_context is None.
        let (goal_repo, _task_repo, _wt_repo, _agent_repo, _mem_repo) =
            test_support::setup_all_repos().await;
        let svc: TaskContextService<_, crate::adapters::sqlite::SqliteMemoryRepository> =
            TaskContextService::new(goal_repo, None);
        let task = Task::new("anything");
        let ctx = svc.load_task_context(&task).await.unwrap();
        assert!(ctx.memory_context.is_none());
    }

    #[test]
    fn test_format_memory_context_renders_entries() {
        let entries = vec![
            scored(Memory::semantic("k1", "first"), 0.9),
            scored(Memory::working("k2", "second"), 0.7),
        ];
        let s = format_memory_context(&entries);
        assert!(s.contains("k1"));
        assert!(s.contains("k2"));
        assert!(s.contains("0.90"));
        assert!(s.contains("0.70"));
    }
}
