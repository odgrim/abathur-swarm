//! Task-complexity-aware model routing.
//!
//! Selects the most cost-effective Claude model based on task complexity,
//! agent tier, and retry count. Haiku for simple tasks (~19x cheaper
//! than Opus), Sonnet for medium, Opus for complex.

use crate::domain::models::task::Complexity;

/// Agent tier for model routing decisions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentTierHint {
    /// Architect-level agents always get at least Moderate.
    Architect,
    /// Specialist agents.
    Specialist,
    /// Worker agents — most cost-sensitive.
    Worker,
}

/// Configuration for model routing.
#[derive(Debug, Clone)]
pub struct ModelRoutingConfig {
    /// Whether model routing is enabled (if false, always returns default).
    pub enabled: bool,
    /// Model for trivial tasks.
    pub trivial_model: String,
    /// Model for simple tasks.
    pub simple_model: String,
    /// Model for moderate tasks.
    pub moderate_model: String,
    /// Model for complex tasks.
    pub complex_model: String,
    /// Whether to escalate model on retry failures.
    pub retry_escalation: bool,
    /// Whether architect agents always get the complex model.
    pub architect_always_complex: bool,
}

impl Default for ModelRoutingConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            trivial_model: "haiku".to_string(),
            simple_model: "haiku".to_string(),
            moderate_model: "sonnet".to_string(),
            complex_model: "opus".to_string(),
            retry_escalation: true,
            architect_always_complex: true,
        }
    }
}

/// Result of a model routing decision.
#[derive(Debug, Clone)]
pub struct ModelSelection {
    /// Selected model name/alias.
    pub model: String,
    /// Reason for selection.
    pub reason: String,
    /// Whether this was escalated from a cheaper model.
    pub escalated: bool,
}

/// Model router for selecting cost-effective models per task.
#[derive(Debug, Clone)]
pub struct ModelRouter {
    config: ModelRoutingConfig,
}

impl ModelRouter {
    pub fn new(config: ModelRoutingConfig) -> Self {
        Self { config }
    }

    pub fn with_defaults() -> Self {
        Self::new(ModelRoutingConfig::default())
    }

    /// Select a model based on task complexity, agent tier, and retry attempt.
    pub fn select_model(
        &self,
        complexity: Complexity,
        agent_tier: Option<AgentTierHint>,
        retry_attempt: u32,
    ) -> ModelSelection {
        if !self.config.enabled {
            return ModelSelection {
                model: self.config.complex_model.clone(),
                reason: "routing disabled".to_string(),
                escalated: false,
            };
        }

        // Architect agents always get at least moderate
        if self.config.architect_always_complex && agent_tier == Some(AgentTierHint::Architect) {
            return ModelSelection {
                model: self.config.complex_model.clone(),
                reason: "architect agent".to_string(),
                escalated: false,
            };
        }

        // Base complexity determines starting model
        let base_complexity = complexity;

        // Escalate complexity based on retry attempt
        let effective_complexity = if self.config.retry_escalation && retry_attempt > 0 {
            escalate_complexity(base_complexity, retry_attempt)
        } else {
            base_complexity
        };

        let escalated = effective_complexity != base_complexity;

        let model = match effective_complexity {
            Complexity::Trivial => self.config.trivial_model.clone(),
            Complexity::Simple => self.config.simple_model.clone(),
            Complexity::Moderate => self.config.moderate_model.clone(),
            Complexity::Complex => self.config.complex_model.clone(),
        };

        let reason = if escalated {
            format!(
                "{:?} task escalated to {:?} (retry #{})",
                base_complexity, effective_complexity, retry_attempt
            )
        } else {
            format!("{:?} complexity", effective_complexity)
        };

        ModelSelection {
            model,
            reason,
            escalated,
        }
    }
}

/// Escalate complexity by a number of levels based on retry count.
fn escalate_complexity(base: Complexity, retry_attempt: u32) -> Complexity {
    let base_level = match base {
        Complexity::Trivial => 0,
        Complexity::Simple => 1,
        Complexity::Moderate => 2,
        Complexity::Complex => 3,
    };

    let escalated_level = (base_level + retry_attempt).min(3);

    match escalated_level {
        0 => Complexity::Trivial,
        1 => Complexity::Simple,
        2 => Complexity::Moderate,
        _ => Complexity::Complex,
    }
}

impl Default for ModelRouter {
    fn default() -> Self {
        Self::with_defaults()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_task_gets_haiku() {
        let router = ModelRouter::with_defaults();
        let selection = router.select_model(Complexity::Simple, None, 0);
        assert_eq!(selection.model, "haiku");
        assert!(!selection.escalated);
    }

    #[test]
    fn test_moderate_task_gets_sonnet() {
        let router = ModelRouter::with_defaults();
        let selection = router.select_model(Complexity::Moderate, None, 0);
        assert_eq!(selection.model, "sonnet");
    }

    #[test]
    fn test_complex_task_gets_opus() {
        let router = ModelRouter::with_defaults();
        let selection = router.select_model(Complexity::Complex, None, 0);
        assert_eq!(selection.model, "opus");
    }

    #[test]
    fn test_retry_escalation() {
        let router = ModelRouter::with_defaults();

        // Simple task, first retry -> Moderate -> sonnet
        let selection = router.select_model(Complexity::Simple, None, 1);
        assert_eq!(selection.model, "sonnet");
        assert!(selection.escalated);

        // Simple task, second retry -> Complex -> opus
        let selection = router.select_model(Complexity::Simple, None, 2);
        assert_eq!(selection.model, "opus");
        assert!(selection.escalated);
    }

    #[test]
    fn test_architect_always_complex() {
        let router = ModelRouter::with_defaults();
        let selection = router.select_model(Complexity::Trivial, Some(AgentTierHint::Architect), 0);
        assert_eq!(selection.model, "opus");
    }

    #[test]
    fn test_worker_gets_cheap_model() {
        let router = ModelRouter::with_defaults();
        let selection = router.select_model(Complexity::Trivial, Some(AgentTierHint::Worker), 0);
        assert_eq!(selection.model, "haiku");
    }

    #[test]
    fn test_routing_disabled() {
        let config = ModelRoutingConfig {
            enabled: false,
            ..Default::default()
        };
        let router = ModelRouter::new(config);
        let selection = router.select_model(Complexity::Trivial, None, 0);
        assert_eq!(selection.model, "opus");
        assert!(selection.reason.contains("disabled"));
    }

    #[test]
    fn test_escalate_complexity() {
        assert_eq!(escalate_complexity(Complexity::Trivial, 0), Complexity::Trivial);
        assert_eq!(escalate_complexity(Complexity::Trivial, 1), Complexity::Simple);
        assert_eq!(escalate_complexity(Complexity::Trivial, 2), Complexity::Moderate);
        assert_eq!(escalate_complexity(Complexity::Trivial, 3), Complexity::Complex);
        assert_eq!(escalate_complexity(Complexity::Trivial, 10), Complexity::Complex); // caps at Complex
    }

    #[test]
    fn test_custom_config() {
        let config = ModelRoutingConfig {
            enabled: true,
            trivial_model: "my-haiku".to_string(),
            simple_model: "my-haiku".to_string(),
            moderate_model: "my-sonnet".to_string(),
            complex_model: "my-opus".to_string(),
            retry_escalation: false,
            architect_always_complex: false,
        };
        let router = ModelRouter::new(config);

        // No retry escalation
        let selection = router.select_model(Complexity::Simple, None, 5);
        assert_eq!(selection.model, "my-haiku");
        assert!(!selection.escalated);

        // Architect not forced to complex
        let selection = router.select_model(Complexity::Simple, Some(AgentTierHint::Architect), 0);
        assert_eq!(selection.model, "my-haiku");
    }
}
