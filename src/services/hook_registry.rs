///! Hook Registry Service
///!
///! Manages hook registration, condition matching, and execution coordination.

use crate::domain::models::{
    HookCondition, HookContext, HookEvent, HookResult,
    HooksConfig, Task, TaskHook,
};
use crate::services::hook_executor::HookExecutor;
use anyhow::{Context, Result};
use regex::Regex;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use tracing::{debug, info, instrument, warn};

/// Registry for managing and executing hooks
pub struct HookRegistry {
    /// Hooks organized by event type
    hooks: HashMap<HookEvent, Vec<TaskHook>>,

    /// Executor for running hook actions
    executor: Arc<HookExecutor>,

    /// Whether hooks are enabled globally
    enabled: bool,
}

impl HookRegistry {
    /// Create a new hook registry
    pub fn new(executor: Arc<HookExecutor>) -> Self {
        Self {
            hooks: HashMap::new(),
            executor,
            enabled: true,
        }
    }

    /// Load hooks from configuration file
    #[instrument(skip(self))]
    pub fn load_from_file(&mut self, config_path: &Path) -> Result<()> {
        info!(path = ?config_path, "Loading hooks from configuration");

        let config_str =
            std::fs::read_to_string(config_path).context("Failed to read hooks config file")?;

        let config: HooksConfig =
            serde_yaml::from_str(&config_str).context("Failed to parse hooks config")?;

        self.load_from_config(config)
    }

    /// Load hooks from configuration object
    #[instrument(skip(self, config))]
    pub fn load_from_config(&mut self, config: HooksConfig) -> Result<()> {
        self.hooks.clear();

        for hook in config.hooks {
            if !hook.enabled {
                debug!(hook_id = %hook.id, "Skipping disabled hook");
                continue;
            }

            info!(
                hook_id = %hook.id,
                event = ?hook.event,
                "Registering hook"
            );

            self.hooks
                .entry(hook.event.clone())
                .or_default()
                .push(hook);
        }

        // Sort hooks by priority (highest first)
        for hooks in self.hooks.values_mut() {
            hooks.sort_by(|a, b| b.priority.cmp(&a.priority));
        }

        info!(total_hooks = self.hooks.values().map(|v| v.len()).sum::<usize>(), "Hooks loaded successfully");

        Ok(())
    }

    /// Execute all hooks for a given event
    #[instrument(skip(self, task, context), fields(task_id = ?task.id, event = ?event))]
    pub async fn execute_hooks(
        &self,
        event: HookEvent,
        task: &Task,
        context: &HookContext,
    ) -> Result<HookResult> {
        if !self.enabled {
            debug!("Hook system disabled, skipping execution");
            return Ok(HookResult::Continue);
        }

        let Some(hooks) = self.hooks.get(&event) else {
            debug!("No hooks registered for event");
            return Ok(HookResult::Continue);
        };

        debug!(hook_count = hooks.len(), "Executing hooks for event");

        for hook in hooks {
            if !hook.enabled {
                continue;
            }

            // Check if conditions match
            if !self.matches_conditions(&hook.conditions, task, context).await? {
                debug!(hook_id = %hook.id, "Hook conditions not met, skipping");
                continue;
            }

            info!(hook_id = %hook.id, "Executing hook");

            // Execute all actions for this hook
            for action in &hook.actions {
                let result = self
                    .executor
                    .execute_action(action, task, context)
                    .await
                    .context(format!("Failed to execute action for hook {}", hook.id))?;

                // If any action blocks, stop immediately
                if result.should_block() {
                    warn!(hook_id = %hook.id, "Hook blocked execution");
                    return Ok(result);
                }
            }
        }

        Ok(HookResult::Continue)
    }

    /// Check if all conditions match for a given task and context
    #[instrument(skip(self, conditions, task, context))]
    async fn matches_conditions(
        &self,
        conditions: &[HookCondition],
        task: &Task,
        context: &HookContext,
    ) -> Result<bool> {
        if conditions.is_empty() {
            // No conditions means always match
            return Ok(true);
        }

        for condition in conditions {
            if !self.matches_condition(condition, task, context).await? {
                return Ok(false);
            }
        }

        Ok(true)
    }

    /// Check if a single condition matches
    async fn matches_condition(
        &self,
        condition: &HookCondition,
        task: &Task,
        context: &HookContext,
    ) -> Result<bool> {
        // Check agent_type
        if let Some(ref expected_agent) = condition.agent_type {
            if &task.agent_type != expected_agent {
                return Ok(false);
            }
        }

        // Check parent_agent_type (requires parent task lookup from context)
        if let Some(ref expected_parent) = condition.parent_agent_type {
            let parent_agent = context
                .variables
                .get("parent_agent_type")
                .map(|s| s.as_str());
            if parent_agent != Some(expected_parent.as_str()) {
                return Ok(false);
            }
        }

        // Check min_children_spawned
        if let Some(min_children) = condition.min_children_spawned {
            let children_count: usize = context
                .variables
                .get("children_count")
                .and_then(|s| s.parse().ok())
                .unwrap_or(0);
            if children_count < min_children {
                return Ok(false);
            }
        }

        // Check child_agent_types
        if let Some(ref expected_types) = condition.child_agent_types {
            let child_types = context
                .variables
                .get("child_agent_types")
                .map(|s| s.split(',').map(|s| s.trim()).collect::<Vec<_>>())
                .unwrap_or_default();

            for expected in expected_types {
                if !child_types.contains(&expected.as_str()) {
                    return Ok(false);
                }
            }
        }

        // Check branch_pattern (for branch completion events)
        if let Some(ref pattern) = condition.branch_pattern {
            if let Some(ref branch_ctx) = context.branch_context {
                let regex = Regex::new(pattern).context("Invalid branch pattern regex")?;
                if !regex.is_match(&branch_ctx.branch_name) {
                    return Ok(false);
                }
            } else {
                return Ok(false);
            }
        }

        // Check feature_branch
        if let Some(ref expected_feature) = condition.feature_branch {
            let feature_branch = task
                .feature_branch
                .as_ref()
                .or_else(|| {
                    context
                        .branch_context
                        .as_ref()
                        .and_then(|b| b.feature_branch.as_ref())
                })
                .map(|s| s.as_str());

            if feature_branch != Some(expected_feature.as_str()) {
                return Ok(false);
            }
        }

        // Check all_tasks_succeeded (for branch completion)
        if let Some(expected_success) = condition.all_tasks_succeeded {
            if let Some(ref branch_ctx) = context.branch_context {
                if branch_ctx.all_succeeded != expected_success {
                    return Ok(false);
                }
            } else {
                return Ok(false);
            }
        }

        // Check min_tasks_completed (for branch completion)
        if let Some(min_tasks) = condition.min_tasks_completed {
            if let Some(ref branch_ctx) = context.branch_context {
                if branch_ctx.total_tasks < min_tasks {
                    return Ok(false);
                }
            } else {
                return Ok(false);
            }
        }

        // Check completed_agent_types_include (for branch completion)
        if let Some(ref required_types) = condition.completed_agent_types_include {
            if let Some(ref branch_ctx) = context.branch_context {
                for required in required_types {
                    if !branch_ctx.completed_agent_types.contains(required) {
                        return Ok(false);
                    }
                }
            } else {
                return Ok(false);
            }
        }

        Ok(true)
    }

    /// Enable or disable the hook system
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
        info!(enabled, "Hook system state changed");
    }

    /// Get count of registered hooks
    pub fn hook_count(&self) -> usize {
        self.hooks.values().map(|v| v.len()).sum()
    }

    /// Get hooks for a specific event
    pub fn get_hooks_for_event(&self, event: &HookEvent) -> Option<&[TaskHook]> {
        self.hooks.get(event).map(|v| v.as_slice())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::models::{BranchType, HookAction};

    fn create_test_task(agent_type: &str) -> Task {
        let mut task = Task::new("Test task".to_string(), "Test description".to_string());
        task.agent_type = agent_type.to_string();
        task
    }

    fn create_test_executor() -> Arc<HookExecutor> {
        Arc::new(HookExecutor::new(None))
    }

    #[test]
    fn test_registry_creation() {
        let executor = create_test_executor();
        let registry = HookRegistry::new(executor);
        assert_eq!(registry.hook_count(), 0);
        assert!(registry.enabled);
    }

    #[test]
    fn test_load_hooks_from_config() {
        let executor = create_test_executor();
        let mut registry = HookRegistry::new(executor);

        let yaml = r#"
hooks:
  - id: test-hook
    event: pre_ready
    actions:
      - type: log_message
        level: info
        message: Test
    enabled: true
"#;

        let config: HooksConfig = serde_yaml::from_str(yaml).unwrap();
        registry.load_from_config(config).unwrap();

        assert_eq!(registry.hook_count(), 1);
    }

    #[tokio::test]
    async fn test_matches_agent_type_condition() {
        let executor = create_test_executor();
        let registry = HookRegistry::new(executor);

        let condition = HookCondition {
            agent_type: Some("test-agent".to_string()),
            ..Default::default()
        };

        let task = create_test_task("test-agent");
        let context = HookContext::from_task(task.id, HashMap::new());

        let matches = registry
            .matches_condition(&condition, &task, &context)
            .await
            .unwrap();
        assert!(matches);

        let other_task = create_test_task("other-agent");
        let matches = registry
            .matches_condition(&condition, &other_task, &context)
            .await
            .unwrap();
        assert!(!matches);
    }

    #[tokio::test]
    async fn test_matches_min_children_condition() {
        let executor = create_test_executor();
        let registry = HookRegistry::new(executor);

        let condition = HookCondition {
            min_children_spawned: Some(3),
            ..Default::default()
        };

        let task = create_test_task("test-agent");

        // Context with 3 children
        let mut variables = HashMap::new();
        variables.insert("children_count".to_string(), "3".to_string());
        let context = HookContext::from_task(task.id, variables);

        let matches = registry
            .matches_condition(&condition, &task, &context)
            .await
            .unwrap();
        assert!(matches);

        // Context with 2 children (below minimum)
        let mut variables = HashMap::new();
        variables.insert("children_count".to_string(), "2".to_string());
        let context = HookContext::from_task(task.id, variables);

        let matches = registry
            .matches_condition(&condition, &task, &context)
            .await
            .unwrap();
        assert!(!matches);
    }

    #[tokio::test]
    async fn test_matches_branch_pattern_condition() {
        let executor = create_test_executor();
        let registry = HookRegistry::new(executor);

        let condition = HookCondition {
            branch_pattern: Some("^task/.*".to_string()),
            ..Default::default()
        };

        let task = create_test_task("test-agent");

        let branch_ctx = BranchCompletionContext {
            branch_name: "task/010-test".to_string(),
            branch_type: BranchType::TaskBranch,
            completed_task_ids: vec![],
            feature_branch: None,
            all_succeeded: true,
            failed_task_count: 0,
            failed_task_ids: vec![],
            total_tasks: 1,
            completed_agent_types: vec![],
        };

        let context = HookContext::from_branch(branch_ctx, HashMap::new());

        let matches = registry
            .matches_condition(&condition, &task, &context)
            .await
            .unwrap();
        assert!(matches);
    }

    #[tokio::test]
    async fn test_disabled_hook_skipped() {
        let executor = create_test_executor();
        let mut registry = HookRegistry::new(executor);

        let yaml = r#"
hooks:
  - id: disabled-hook
    event: pre_ready
    actions:
      - type: log_message
        level: info
        message: Should not execute
    enabled: false
"#;

        let config: HooksConfig = serde_yaml::from_str(yaml).unwrap();
        registry.load_from_config(config).unwrap();

        // Hook is disabled, so it shouldn't be loaded
        assert_eq!(registry.hook_count(), 0);
    }
}
