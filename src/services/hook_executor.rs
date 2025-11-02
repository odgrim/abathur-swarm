///! Hook Executor Service
///!
///! Executes hook actions including running scripts, spawning tasks, and more.

use crate::application::task_coordinator::TaskCoordinator;
use crate::domain::models::{HookAction, HookContext, HookResult, Task};
use anyhow::{Context, Result};
use std::collections::HashMap;
use std::process::Stdio;
use std::sync::Arc;
use tokio::process::Command;
use tracing::{debug, error, info, instrument, warn};

/// Executor for hook actions
pub struct HookExecutor {
    /// Optional task coordinator for spawning tasks
    task_coordinator: Option<Arc<TaskCoordinator>>,
}

impl HookExecutor {
    /// Create a new hook executor
    pub fn new(task_coordinator: Option<Arc<TaskCoordinator>>) -> Self {
        Self { task_coordinator }
    }

    /// Execute a single hook action
    #[instrument(skip(self, task, context), fields(task_id = ?task.id))]
    pub async fn execute_action(
        &self,
        action: &HookAction,
        task: &Task,
        context: &HookContext,
    ) -> Result<HookResult> {
        match action {
            HookAction::RunScript { script_path, args } => {
                self.run_script(script_path, args, task, context).await
            }
            HookAction::SpawnTask {
                agent_type,
                summary,
                description,
                priority,
            } => {
                self.spawn_task(agent_type, summary, description, *priority, task, context)
                    .await
            }
            HookAction::UpdateField { field, value } => {
                self.update_field(field, value, task, context).await
            }
            HookAction::BlockTransition { reason } => {
                self.block_transition(reason, task, context).await
            }
            HookAction::LogMessage { level, message } => {
                self.log_message(level, message, task, context).await
            }
            HookAction::MergeBranch {
                source,
                target,
                strategy,
            } => {
                self.merge_branch(source, target, strategy, task, context)
                    .await
            }
            HookAction::DeleteBranch {
                branch,
                cleanup_worktree,
            } => {
                self.delete_branch(branch, *cleanup_worktree, task, context)
                    .await
            }
            HookAction::CreateTag { name, message } => {
                self.create_tag(name, message, task, context).await
            }
            HookAction::NotifyWebhook { url, payload } => {
                self.notify_webhook(url, payload, task, context).await
            }
        }
    }

    /// Run a shell script
    async fn run_script(
        &self,
        script_path: &str,
        args: &[String],
        task: &Task,
        context: &HookContext,
    ) -> Result<HookResult> {
        info!(script = script_path, "Running hook script");

        // Substitute variables in script path and args
        let script_path = self.substitute_variables(script_path, context);
        let args: Vec<String> = args
            .iter()
            .map(|arg| self.substitute_variables(arg, context))
            .collect();

        debug!(script = %script_path, args = ?args, "Executing script with substituted variables");

        let output = Command::new(&script_path)
            .args(&args)
            .env("TASK_ID", task.id.to_string())
            .env("TASK_AGENT_TYPE", &task.agent_type)
            .env("TASK_SUMMARY", &task.summary)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await
            .context(format!("Failed to execute script: {}", script_path))?;

        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            info!(script = script_path, "Script executed successfully");
            if !stdout.is_empty() {
                debug!(output = %stdout, "Script output");
            }
            Ok(HookResult::Continue)
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            error!(script = script_path, stderr = %stderr, "Script execution failed");
            Err(anyhow::anyhow!(
                "Script {} failed: {}",
                script_path,
                stderr
            ))
        }
    }

    /// Spawn a new task
    async fn spawn_task(
        &self,
        agent_type: &str,
        summary: &str,
        description: &str,
        priority: u8,
        _task: &Task,
        context: &HookContext,
    ) -> Result<HookResult> {
        let Some(ref coordinator) = self.task_coordinator else {
            warn!("Task coordinator not available, cannot spawn task");
            return Ok(HookResult::Continue);
        };

        // Substitute variables
        let agent_type = self.substitute_variables(agent_type, context);
        let summary = self.substitute_variables(summary, context);
        let description = self.substitute_variables(description, context);

        info!(
            agent_type = %agent_type,
            summary = %summary,
            "Spawning task from hook"
        );

        let mut new_task = Task::new(summary, description);
        new_task.agent_type = agent_type;
        new_task.priority = priority;

        // Inherit context from current task
        if let Some(task_id) = context.task_id {
            new_task.parent_task_id = Some(task_id);
            new_task.dependencies = Some(vec![task_id]);
        }

        // Inherit branch context if available
        if let Some(ref branch_ctx) = context.branch_context {
            new_task.feature_branch = branch_ctx.feature_branch.clone();
        }

        let spawned_id = coordinator.submit_task(new_task).await?;

        info!(
            spawned_task_id = %spawned_id,
            "Task spawned successfully from hook"
        );

        Ok(HookResult::Continue)
    }

    /// Update a task field (placeholder - would need more complex implementation)
    async fn update_field(
        &self,
        field: &str,
        value: &serde_json::Value,
        _task: &Task,
        _context: &HookContext,
    ) -> Result<HookResult> {
        info!(field = field, value = ?value, "Update field action");
        // TODO: Implement field update logic via task coordinator
        warn!("UpdateField action not yet fully implemented");
        Ok(HookResult::Continue)
    }

    /// Block a state transition
    async fn block_transition(
        &self,
        reason: &str,
        _task: &Task,
        context: &HookContext,
    ) -> Result<HookResult> {
        let reason = self.substitute_variables(reason, context);
        warn!(reason = %reason, "Blocking transition");
        Ok(HookResult::Blocked { reason })
    }

    /// Log a message
    async fn log_message(
        &self,
        level: &str,
        message: &str,
        _task: &Task,
        context: &HookContext,
    ) -> Result<HookResult> {
        let message = self.substitute_variables(message, context);

        match level.to_lowercase().as_str() {
            "error" => error!("{}", message),
            "warn" | "warning" => warn!("{}", message),
            "info" => info!("{}", message),
            "debug" => debug!("{}", message),
            _ => info!("{}", message),
        }

        Ok(HookResult::Continue)
    }

    /// Merge branches (spawns git-worktree-merge-orchestrator)
    async fn merge_branch(
        &self,
        source: &str,
        target: &str,
        _strategy: &crate::domain::models::MergeStrategy,
        task: &Task,
        context: &HookContext,
    ) -> Result<HookResult> {
        let Some(ref coordinator) = self.task_coordinator else {
            warn!("Task coordinator not available, cannot spawn merge task");
            return Ok(HookResult::Continue);
        };

        let source = self.substitute_variables(source, context);
        let target = self.substitute_variables(target, context);

        info!(source = %source, target = %target, "Spawning merge orchestrator");

        let description = format!(
            r#"# Merge Task Branch into Feature Branch

## Branch Details
- Source Branch: {}
- Target Branch: {}
- Triggered by Hook

## Your Mission
1. Verify all tests pass in source branch
2. Merge source branch into target branch
3. Run integration tests on target branch
4. Clean up source branch and worktree

## Context
Original Task: {}
Agent Type: {}
"#,
            source, target, task.summary, task.agent_type
        );

        let mut merge_task = Task::new(format!("Merge {} into {}", source, target), description);
        merge_task.agent_type = "git-worktree-merge-orchestrator".to_string();
        merge_task.priority = 8;

        if let Some(task_id) = context.task_id {
            merge_task.parent_task_id = Some(task_id);
            merge_task.dependencies = Some(vec![task_id]);
        }

        let spawned_id = coordinator.submit_task(merge_task).await?;

        info!(
            merge_task_id = %spawned_id,
            "Merge orchestrator spawned successfully"
        );

        Ok(HookResult::Continue)
    }

    /// Delete a branch (placeholder)
    async fn delete_branch(
        &self,
        branch: &str,
        cleanup_worktree: bool,
        _task: &Task,
        context: &HookContext,
    ) -> Result<HookResult> {
        let branch = self.substitute_variables(branch, context);
        info!(
            branch = %branch,
            cleanup_worktree = cleanup_worktree,
            "Delete branch action"
        );
        // TODO: Implement branch deletion logic
        warn!("DeleteBranch action not yet fully implemented");
        Ok(HookResult::Continue)
    }

    /// Create a git tag (placeholder)
    async fn create_tag(
        &self,
        name: &str,
        message: &str,
        _task: &Task,
        context: &HookContext,
    ) -> Result<HookResult> {
        let name = self.substitute_variables(name, context);
        let message = self.substitute_variables(message, context);
        info!(tag = %name, message = %message, "Create tag action");
        // TODO: Implement tag creation logic
        warn!("CreateTag action not yet fully implemented");
        Ok(HookResult::Continue)
    }

    /// Send webhook notification (placeholder)
    async fn notify_webhook(
        &self,
        url: &str,
        payload: &serde_json::Value,
        _task: &Task,
        context: &HookContext,
    ) -> Result<HookResult> {
        let url = self.substitute_variables(url, context);
        info!(url = %url, payload = ?payload, "Webhook notification action");
        // TODO: Implement webhook notification
        warn!("NotifyWebhook action not yet fully implemented");
        Ok(HookResult::Continue)
    }

    /// Substitute template variables in strings
    fn substitute_variables(&self, template: &str, context: &HookContext) -> String {
        let mut result = template.to_string();

        for (key, value) in &context.variables {
            let placeholder = format!("${{{}}}", key);
            result = result.replace(&placeholder, value);
        }

        result
    }

    /// Build template variables from task and context
    pub fn build_variables(task: &Task, context: &HookContext) -> HashMap<String, String> {
        let mut vars = context.variables.clone();

        // Task-level variables
        vars.insert("task_id".to_string(), task.id.to_string());
        vars.insert("task_agent_type".to_string(), task.agent_type.clone());
        vars.insert("task_summary".to_string(), task.summary.clone());
        vars.insert("task_status".to_string(), task.status.to_string());

        if let Some(parent_id) = task.parent_task_id {
            vars.insert("parent_task_id".to_string(), parent_id.to_string());
        }

        if let Some(ref feature_branch) = task.feature_branch {
            vars.insert("feature_branch".to_string(), feature_branch.clone());
        }

        if let Some(ref task_branch) = task.task_branch {
            vars.insert("task_branch".to_string(), task_branch.clone());
        }

        // Branch-level variables
        if let Some(ref branch_ctx) = context.branch_context {
            vars.insert("branch_name".to_string(), branch_ctx.branch_name.clone());
            vars.insert(
                "branch_type".to_string(),
                format!("{:?}", branch_ctx.branch_type),
            );
            vars.insert(
                "total_tasks".to_string(),
                branch_ctx.total_tasks.to_string(),
            );
            vars.insert(
                "failed_task_count".to_string(),
                branch_ctx.failed_task_count.to_string(),
            );
            vars.insert(
                "all_succeeded".to_string(),
                branch_ctx.all_succeeded.to_string(),
            );

            if let Some(ref feature) = branch_ctx.feature_branch {
                vars.insert("feature_branch".to_string(), feature.clone());
            }
        }

        vars
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::models::HookContext;

    fn create_test_task() -> Task {
        Task::new("Test task".to_string(), "Test description".to_string())
    }

    #[test]
    fn test_substitute_variables() {
        let executor = HookExecutor::new(None);

        let mut variables = HashMap::new();
        variables.insert("task_id".to_string(), "123".to_string());
        variables.insert("agent_type".to_string(), "test-agent".to_string());

        let task = create_test_task();
        let context = HookContext::from_task(task.id, variables);

        let template = "Task ${task_id} with agent ${agent_type}";
        let result = executor.substitute_variables(template, &context);

        assert_eq!(result, "Task 123 with agent test-agent");
    }

    #[test]
    fn test_build_variables() {
        let mut task = create_test_task();
        task.agent_type = "test-agent".to_string();
        task.feature_branch = Some("feature/test".to_string());

        let context = HookContext::from_task(task.id, HashMap::new());
        let vars = HookExecutor::build_variables(&task, &context);

        assert_eq!(vars.get("task_agent_type").unwrap(), "test-agent");
        assert_eq!(vars.get("feature_branch").unwrap(), "feature/test");
        assert_eq!(vars.get("task_summary").unwrap(), "Test task");
    }

    #[tokio::test]
    async fn test_log_message_action() {
        let executor = HookExecutor::new(None);
        let task = create_test_task();
        let context = HookContext::from_task(task.id, HashMap::new());

        let result = executor
            .log_message("info", "Test message", &task, &context)
            .await
            .unwrap();

        assert_eq!(result, HookResult::Continue);
    }

    #[tokio::test]
    async fn test_block_transition_action() {
        let executor = HookExecutor::new(None);
        let task = create_test_task();
        let context = HookContext::from_task(task.id, HashMap::new());

        let result = executor
            .block_transition("Test blocking", &task, &context)
            .await
            .unwrap();

        assert!(result.should_block());
    }
}
