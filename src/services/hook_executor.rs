///! Hook Executor Service
///!
///! Executes hook actions including running scripts, spawning tasks, and more.

use crate::application::task_coordinator::TaskCoordinator;
use crate::domain::models::task::TaskStatus;
use crate::domain::models::{HookAction, HookContext, HookResult, Task};
use crate::domain::ports::TaskQueueService;
use anyhow::{Context, Result};
use std::collections::HashMap;
use std::process::Stdio;
use std::sync::Arc;
use tokio::process::Command;
use tracing::{debug, error, info, instrument, warn};
use uuid::Uuid;

/// Executor for hook actions
pub struct HookExecutor {
    /// Optional task coordinator for spawning tasks
    task_coordinator: Option<Arc<TaskCoordinator>>,
    /// Optional task queue service for updating task fields
    task_queue: Option<Arc<dyn TaskQueueService>>,
}

impl HookExecutor {
    /// Create a new hook executor
    pub fn new(
        task_coordinator: Option<Arc<TaskCoordinator>>,
        task_queue: Option<Arc<dyn TaskQueueService>>,
    ) -> Self {
        Self {
            task_coordinator,
            task_queue,
        }
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

            // Parse ABATHUR_* variables from script output
            let mut updates = HashMap::new();
            for line in stdout.lines() {
                if line.starts_with("ABATHUR_") {
                    if let Some((key, value)) = line.split_once('=') {
                        let key = key.trim().to_string();
                        let value = value.trim().to_string();
                        debug!(key = %key, value = %value, "Parsed hook output variable");
                        updates.insert(key, value);
                    }
                }
            }

            // Update task fields if we have updates and task_queue is available
            if !updates.is_empty() {
                if let Some(ref task_queue) = self.task_queue {
                    info!(
                        task_id = %task.id,
                        update_count = updates.len(),
                        "Updating task fields from hook script output"
                    );
                    self.update_task_fields(task.id, updates, task_queue).await?;
                } else {
                    warn!(
                        task_id = %task.id,
                        "Task queue not available, cannot update task fields from hook output"
                    );
                }
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

    /// Update task fields from hook script output
    ///
    /// Parses ABATHUR_* variables and updates the corresponding task fields.
    async fn update_task_fields(
        &self,
        task_id: Uuid,
        updates: HashMap<String, String>,
        task_queue: &Arc<dyn TaskQueueService>,
    ) -> Result<()> {
        // Fetch current task
        let mut task = task_queue.get_task(task_id).await
            .context("Failed to fetch task for field update")?;

        let mut updated = false;

        // Apply updates
        if let Some(feature_branch) = updates.get("ABATHUR_FEATURE_BRANCH") {
            task.feature_branch = Some(feature_branch.clone());
            info!(task_id = %task_id, feature_branch = %feature_branch, "Updated task feature_branch");
            updated = true;
        }

        if let Some(branch) = updates.get("ABATHUR_BRANCH") {
            task.branch = Some(branch.clone());
            info!(task_id = %task_id, branch = %branch, "Updated task branch");
            updated = true;
        }

        if let Some(worktree_path) = updates.get("ABATHUR_WORKTREE_PATH") {
            task.worktree_path = Some(worktree_path.clone());
            info!(task_id = %task_id, worktree_path = %worktree_path, "Updated task worktree_path");
            updated = true;
        }

        // Save updated task if any fields were changed
        if updated {
            task_queue.update_task(&task).await
                .context("Failed to save updated task")?;
            info!(task_id = %task_id, "Task fields successfully updated in database");
        }

        Ok(())
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

    /// Update a task field
    ///
    /// Currently supports updating 'status' and 'priority' fields through the task coordinator.
    /// Other fields require fetching the full task, modifying it, and using a general update method
    /// which is not currently exposed through the TaskQueueService interface.
    async fn update_field(
        &self,
        field: &str,
        value: &serde_json::Value,
        task: &Task,
        _context: &HookContext,
    ) -> Result<HookResult> {
        info!(field = field, value = ?value, "Update field action");

        let Some(ref coordinator) = self.task_coordinator else {
            warn!("Task coordinator not available, cannot update field");
            return Ok(HookResult::Continue);
        };

        match field {
            "status" => {
                let status_str = value.as_str()
                    .ok_or_else(|| anyhow::anyhow!("Status value must be a string"))?;
                let status: TaskStatus = status_str.parse()
                    .context(format!("Invalid status value: {}", status_str))?;

                coordinator
                    .update_task_status(task.id, status)
                    .await
                    .context("Failed to update task status")?;

                info!(task_id = %task.id, new_status = ?status, "Updated task status via hook");
            }
            "priority" => {
                let priority = value.as_f64()
                    .or_else(|| value.as_u64().map(|v| v as f64))
                    .or_else(|| value.as_i64().map(|v| v as f64))
                    .ok_or_else(|| anyhow::anyhow!("Priority value must be a number"))?;

                coordinator
                    .update_task_priority(task.id, priority)
                    .await
                    .context("Failed to update task priority")?;

                info!(task_id = %task.id, new_priority = priority, "Updated task priority via hook");
            }
            _ => {
                warn!(
                    field = field,
                    "Unsupported field update. Only 'status' and 'priority' can be updated via hooks. \
                     For other fields, consider using a custom script or extending the TaskQueueService interface."
                );
            }
        }

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

        let mut merge_task = Task::new(
            Task::create_summary_with_prefix("", &format!("Merge {} into {}", source, target)),
            description,
        );
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

    /// Delete a git branch
    ///
    /// Deletes the specified branch and optionally removes its associated worktree.
    /// Uses `git branch -d` for safe deletion (only deletes if merged) or `-D` if forced.
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
            "Deleting branch"
        );

        // First, check if the branch exists
        let check_output = Command::new("git")
            .args(["rev-parse", "--verify", &branch])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await
            .context("Failed to check if branch exists")?;

        if !check_output.success() {
            warn!(branch = %branch, "Branch does not exist, skipping deletion");
            return Ok(HookResult::Continue);
        }

        // Delete the branch (using -d for safe deletion, which requires branch to be merged)
        let delete_output = Command::new("git")
            .args(["branch", "-d", &branch])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await
            .context(format!("Failed to delete branch: {}", branch))?;

        if !delete_output.status.success() {
            let stderr = String::from_utf8_lossy(&delete_output.stderr);
            error!(branch = %branch, stderr = %stderr, "Failed to delete branch");
            return Err(anyhow::anyhow!("Git branch deletion failed: {}", stderr));
        }

        info!(branch = %branch, "Branch deleted successfully");

        // Clean up worktree if requested
        if cleanup_worktree {
            // List worktrees to find the one associated with this branch
            let worktree_output = Command::new("git")
                .args(["worktree", "list", "--porcelain"])
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .output()
                .await
                .context("Failed to list worktrees")?;

            if worktree_output.status.success() {
                let output_str = String::from_utf8_lossy(&worktree_output.stdout);

                // Parse worktree list to find worktree path for this branch
                let mut worktree_path: Option<String> = None;
                let mut current_path: Option<String> = None;

                for line in output_str.lines() {
                    if line.starts_with("worktree ") {
                        current_path = Some(line.strip_prefix("worktree ").unwrap_or("").to_string());
                    } else if line.starts_with("branch ") {
                        let worktree_branch = line.strip_prefix("branch ").unwrap_or("");
                        if worktree_branch.ends_with(&branch) {
                            worktree_path = current_path.take();
                            break;
                        }
                    }
                }

                if let Some(path) = worktree_path {
                    info!(path = %path, "Removing worktree");

                    let remove_output = Command::new("git")
                        .args(["worktree", "remove", &path])
                        .stdout(Stdio::piped())
                        .stderr(Stdio::piped())
                        .output()
                        .await
                        .context("Failed to remove worktree")?;

                    if !remove_output.status.success() {
                        let stderr = String::from_utf8_lossy(&remove_output.stderr);
                        warn!(path = %path, stderr = %stderr, "Failed to remove worktree, trying with --force");

                        // Try with force flag
                        let force_output = Command::new("git")
                            .args(["worktree", "remove", "--force", &path])
                            .stdout(Stdio::piped())
                            .stderr(Stdio::piped())
                            .output()
                            .await?;

                        if !force_output.status.success() {
                            let force_stderr = String::from_utf8_lossy(&force_output.stderr);
                            error!(path = %path, stderr = %force_stderr, "Failed to force remove worktree");
                        } else {
                            info!(path = %path, "Worktree removed successfully with --force");
                        }
                    } else {
                        info!(path = %path, "Worktree removed successfully");
                    }
                } else {
                    debug!(branch = %branch, "No worktree found for branch");
                }
            }
        }

        Ok(HookResult::Continue)
    }

    /// Create a git tag
    ///
    /// Creates an annotated git tag with the specified name and message.
    /// Annotated tags are recommended over lightweight tags as they store
    /// metadata (tagger, date) and can be cryptographically signed.
    async fn create_tag(
        &self,
        name: &str,
        message: &str,
        _task: &Task,
        context: &HookContext,
    ) -> Result<HookResult> {
        let name = self.substitute_variables(name, context);
        let message = self.substitute_variables(message, context);
        info!(tag = %name, message = %message, "Creating git tag");

        // Create an annotated tag
        let tag_output = Command::new("git")
            .args(["tag", "-a", &name, "-m", &message])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await
            .context(format!("Failed to create tag: {}", name))?;

        if tag_output.status.success() {
            info!(tag = %name, "Tag created successfully");
            Ok(HookResult::Continue)
        } else {
            let stderr = String::from_utf8_lossy(&tag_output.stderr);
            error!(tag = %name, stderr = %stderr, "Failed to create tag");
            Err(anyhow::anyhow!("Git tag creation failed: {}", stderr))
        }
    }

    /// Send webhook notification
    ///
    /// Sends an HTTP POST request with JSON payload to the specified webhook URL.
    /// This is useful for integrating with external services like Slack, Discord,
    /// CI/CD pipelines, or custom notification systems.
    ///
    /// # Timeout
    /// Requests timeout after 30 seconds to prevent hooks from blocking indefinitely.
    async fn notify_webhook(
        &self,
        url: &str,
        payload: &serde_json::Value,
        _task: &Task,
        context: &HookContext,
    ) -> Result<HookResult> {
        let url = self.substitute_variables(url, context);
        info!(url = %url, payload = ?payload, "Sending webhook notification");

        // Create HTTP client with timeout
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .context("Failed to create HTTP client for webhook")?;

        // Send POST request
        let response = client
            .post(&url)
            .json(payload)
            .send()
            .await
            .context(format!("Failed to send webhook to {}", url))?;

        if response.status().is_success() {
            info!(
                url = %url,
                status = %response.status(),
                "Webhook notification sent successfully"
            );
            Ok(HookResult::Continue)
        } else {
            let status = response.status();
            let body = response
                .text()
                .await
                .unwrap_or_else(|_| "<failed to read response body>".to_string());

            error!(
                url = %url,
                status = %status,
                response_body = %body,
                "Webhook notification failed"
            );

            Err(anyhow::anyhow!(
                "Webhook notification failed with status {}: {}",
                status,
                body
            ))
        }
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

        if let Some(ref branch) = task.branch {
            vars.insert("branch".to_string(), branch.clone());
        }

        if let Some(ref feature_branch) = task.feature_branch {
            vars.insert("feature_branch".to_string(), feature_branch.clone());
        }

        if let Some(ref worktree_path) = task.worktree_path {
            vars.insert("worktree_path".to_string(), worktree_path.clone());
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
        let executor = HookExecutor::new(None, None);

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
        let executor = HookExecutor::new(None, None);
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
        let executor = HookExecutor::new(None, None);
        let task = create_test_task();
        let context = HookContext::from_task(task.id, HashMap::new());

        let result = executor
            .block_transition("Test blocking", &task, &context)
            .await
            .unwrap();

        assert!(result.should_block());
    }
}
