//! Context window guard for pre-flight token estimation.
//!
//! Estimates prompt token count before substrate invocation and blocks
//! requests that would leave insufficient room for output generation.

use crate::services::context_truncation::estimate_tokens;

/// Context window sizes for known models (in tokens).
pub fn model_context_window(_model: &str) -> usize {
    // All current Anthropic models (opus, sonnet, haiku) share the same 200k window.
    // If model-specific windows are needed in the future, match on model name here.
    200_000
}

/// Thresholds for the context window guard.
#[derive(Debug, Clone)]
pub struct ContextWindowGuardConfig {
    /// Warn when remaining tokens drop below this threshold.
    pub warn_threshold_tokens: usize,
    /// Block execution when remaining tokens drop below this threshold.
    pub block_threshold_tokens: usize,
}

impl Default for ContextWindowGuardConfig {
    fn default() -> Self {
        Self {
            warn_threshold_tokens: 32_000,
            block_threshold_tokens: 16_000,
        }
    }
}

/// Result of a context window pre-flight check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContextWindowCheck {
    /// Sufficient room — proceed normally.
    Ok {
        estimated_prompt_tokens: usize,
        remaining_tokens: usize,
    },
    /// Low on remaining tokens — proceed with warning.
    Warn {
        estimated_prompt_tokens: usize,
        remaining_tokens: usize,
    },
    /// Insufficient room — block execution (no retries).
    Block {
        estimated_prompt_tokens: usize,
        remaining_tokens: usize,
        context_window: usize,
    },
}

impl ContextWindowCheck {
    /// Whether execution should proceed.
    pub fn should_proceed(&self) -> bool {
        !matches!(self, Self::Block { .. })
    }

    /// Whether a warning should be logged.
    pub fn should_warn(&self) -> bool {
        matches!(self, Self::Warn { .. })
    }
}

/// Context window guard.
pub struct ContextWindowGuard {
    config: ContextWindowGuardConfig,
}

impl ContextWindowGuard {
    pub fn new(config: ContextWindowGuardConfig) -> Self {
        Self { config }
    }

    pub fn with_defaults() -> Self {
        Self::new(ContextWindowGuardConfig::default())
    }

    /// Check whether a prompt fits within the model's context window.
    ///
    /// Estimates total prompt tokens from the system prompt and user prompt,
    /// then checks remaining capacity against thresholds.
    pub fn check(
        &self,
        model: &str,
        system_prompt: &str,
        user_prompt: &str,
    ) -> ContextWindowCheck {
        let context_window = model_context_window(model);
        let prompt_tokens = estimate_tokens(system_prompt) + estimate_tokens(user_prompt);
        let remaining = context_window.saturating_sub(prompt_tokens);

        if remaining < self.config.block_threshold_tokens {
            ContextWindowCheck::Block {
                estimated_prompt_tokens: prompt_tokens,
                remaining_tokens: remaining,
                context_window,
            }
        } else if remaining < self.config.warn_threshold_tokens {
            ContextWindowCheck::Warn {
                estimated_prompt_tokens: prompt_tokens,
                remaining_tokens: remaining,
            }
        } else {
            ContextWindowCheck::Ok {
                estimated_prompt_tokens: prompt_tokens,
                remaining_tokens: remaining,
            }
        }
    }
}

impl Default for ContextWindowGuard {
    fn default() -> Self {
        Self::with_defaults()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_model_context_window() {
        assert_eq!(model_context_window("opus"), 200_000);
        assert_eq!(model_context_window("claude-opus-4-6-20250616"), 200_000);
        assert_eq!(model_context_window("haiku"), 200_000);
        assert_eq!(model_context_window("sonnet"), 200_000);
    }

    #[test]
    fn test_check_ok() {
        let guard = ContextWindowGuard::with_defaults();
        let check = guard.check("opus", "short system prompt", "short user prompt");
        assert!(check.should_proceed());
        assert!(!check.should_warn());
    }

    #[test]
    fn test_check_warn() {
        let guard = ContextWindowGuard::new(ContextWindowGuardConfig {
            warn_threshold_tokens: 100,
            block_threshold_tokens: 10,
        });
        // Create a prompt that leaves ~50 tokens remaining out of 200K
        // 200K - 50 = 199,950 tokens needed = ~799,800 chars
        let large_prompt = "x".repeat(799_600);
        let check = guard.check("opus", &large_prompt, "hello");
        assert!(check.should_proceed());
        assert!(check.should_warn());
    }

    #[test]
    fn test_check_block() {
        let guard = ContextWindowGuard::new(ContextWindowGuardConfig {
            warn_threshold_tokens: 100,
            block_threshold_tokens: 50,
        });
        let large_prompt = "x".repeat(800_000);
        let check = guard.check("opus", &large_prompt, "hello");
        assert!(!check.should_proceed());
    }

    #[test]
    fn test_small_prompts_always_ok() {
        let guard = ContextWindowGuard::with_defaults();
        let check = guard.check("opus", "", "");
        assert!(check.should_proceed());
        if let ContextWindowCheck::Ok { remaining_tokens, .. } = check {
            assert_eq!(remaining_tokens, 200_000);
        }
    }
}
