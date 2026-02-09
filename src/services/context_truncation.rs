//! Context truncation service for managing token budgets.
//!
//! Ensures no single context section consumes more than a configurable
//! share of the model's context window. Uses a 4 chars/token heuristic
//! and preserves newline boundaries when truncating.

/// Approximate characters per token (conservative heuristic).
const CHARS_PER_TOKEN: usize = 4;

/// Maximum share of context window any single section may consume.
const DEFAULT_MAX_CONTEXT_SHARE: f32 = 0.3;

/// Hard maximum characters per section (~100K tokens).
const DEFAULT_HARD_MAX_CHARS: usize = 400_000;

/// Minimum characters to keep when truncating.
const DEFAULT_MIN_KEEP_CHARS: usize = 2_000;

/// Default context window size in tokens (Claude models).
const DEFAULT_CONTEXT_WINDOW: usize = 200_000;

/// Configuration for context truncation.
#[derive(Debug, Clone)]
pub struct TruncationConfig {
    /// Characters per token heuristic.
    pub chars_per_token: usize,
    /// Maximum share of context window per section (0.0-1.0).
    pub max_context_share: f32,
    /// Hard maximum characters per section.
    pub hard_max_chars: usize,
    /// Minimum characters to keep when truncating.
    pub min_keep_chars: usize,
    /// Model context window size in tokens.
    pub context_window_tokens: usize,
}

impl Default for TruncationConfig {
    fn default() -> Self {
        Self {
            chars_per_token: CHARS_PER_TOKEN,
            max_context_share: DEFAULT_MAX_CONTEXT_SHARE,
            hard_max_chars: DEFAULT_HARD_MAX_CHARS,
            min_keep_chars: DEFAULT_MIN_KEEP_CHARS,
            context_window_tokens: DEFAULT_CONTEXT_WINDOW,
        }
    }
}

impl TruncationConfig {
    /// Calculate the maximum characters allowed for a single section.
    pub fn max_section_chars(&self) -> usize {
        let budget_chars =
            (self.context_window_tokens as f32 * self.max_context_share) as usize * self.chars_per_token;
        budget_chars.min(self.hard_max_chars)
    }
}

/// Estimate the number of tokens in a string using the chars/token heuristic.
pub fn estimate_tokens(text: &str) -> usize {
    (text.len() + CHARS_PER_TOKEN - 1) / CHARS_PER_TOKEN
}

/// Truncate a context section to fit within the configured budget.
///
/// Preserves newline boundaries — truncates at the last newline before the limit.
/// Adds a truncation marker when content is cut.
pub fn truncate_section(text: &str, config: &TruncationConfig) -> String {
    if text.is_empty() {
        return text.to_string();
    }

    let max_chars = config.max_section_chars();

    if text.len() <= max_chars {
        return text.to_string();
    }

    let keep_chars = max_chars.max(config.min_keep_chars);
    let truncate_at = if keep_chars < text.len() {
        text[..keep_chars].rfind('\n').unwrap_or(keep_chars)
    } else {
        text.len()
    };

    let truncated = &text[..truncate_at];
    let original_tokens = estimate_tokens(text);
    let kept_tokens = estimate_tokens(truncated);

    format!(
        "{}\n\n[... truncated: showing ~{}K of ~{}K tokens ...]\n",
        truncated,
        kept_tokens / 1000,
        original_tokens / 1000,
    )
}

/// Truncate text to a specific token budget.
pub fn truncate_to_token_budget(text: &str, token_budget: usize) -> String {
    let max_chars = token_budget * CHARS_PER_TOKEN;

    if text.len() <= max_chars {
        return text.to_string();
    }

    let truncate_at = text[..max_chars].rfind('\n').unwrap_or(max_chars);
    let truncated = &text[..truncate_at];

    format!(
        "{}\n\n[... truncated to ~{}K token budget ...]\n",
        truncated,
        token_budget / 1000,
    )
}

/// Truncate all context sections, logging any that were truncated.
/// Returns the vector of (name, truncated_content) and whether any were truncated.
pub fn truncate_context_sections(
    sections: Vec<(&str, String)>,
    config: &TruncationConfig,
) -> (Vec<(String, String)>, bool) {
    let mut any_truncated = false;
    let mut result = Vec::with_capacity(sections.len());
    let max_chars = config.max_section_chars();

    for (name, content) in sections {
        let was_truncated = content.len() > max_chars;
        let truncated = truncate_section(&content, config);
        if was_truncated {
            any_truncated = true;
            tracing::warn!(
                "Context section '{}' truncated from {} chars (~{}K tokens, limit {})",
                name,
                content.len(),
                estimate_tokens(&content) / 1000,
                max_chars,
            );
        }
        result.push((name.to_string(), truncated));
    }

    (result, any_truncated)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_estimate_tokens() {
        assert_eq!(estimate_tokens(""), 0);
        assert_eq!(estimate_tokens("abcd"), 1);
        assert_eq!(estimate_tokens("abcde"), 2);
        assert_eq!(estimate_tokens("abcdefgh"), 2);
    }

    #[test]
    fn test_truncate_section_no_truncation_needed() {
        let config = TruncationConfig::default();
        let short = "Hello, world!";
        assert_eq!(truncate_section(short, &config), short);
    }

    #[test]
    fn test_truncate_section_with_truncation() {
        let config = TruncationConfig {
            hard_max_chars: 100,
            min_keep_chars: 10,
            ..Default::default()
        };
        let long = "line1\nline2\nline3\n".repeat(20);
        let result = truncate_section(&long, &config);
        assert!(result.len() < long.len());
        assert!(result.contains("[... truncated"));
    }

    #[test]
    fn test_truncate_section_empty() {
        let config = TruncationConfig::default();
        assert_eq!(truncate_section("", &config), "");
    }

    #[test]
    fn test_truncate_to_token_budget() {
        let text = "word ".repeat(1000);
        let result = truncate_to_token_budget(&text, 100);
        assert!(result.len() < text.len());
        assert!(result.contains("[... truncated"));
    }

    #[test]
    fn test_truncate_to_token_budget_no_truncation() {
        let text = "short text";
        assert_eq!(truncate_to_token_budget(text, 100), text);
    }

    #[test]
    fn test_truncation_config_max_section() {
        let config = TruncationConfig {
            context_window_tokens: 200_000,
            max_context_share: 0.3,
            chars_per_token: 4,
            hard_max_chars: 400_000,
            min_keep_chars: 2_000,
        };
        // 200K * 0.3 = 60K tokens * 4 chars = 240K chars (under 400K hard max)
        assert_eq!(config.max_section_chars(), 240_000);
    }

    #[test]
    fn test_truncation_config_hard_max_applies() {
        let config = TruncationConfig {
            context_window_tokens: 1_000_000,
            max_context_share: 0.5,
            chars_per_token: 4,
            hard_max_chars: 400_000,
            min_keep_chars: 2_000,
        };
        // 1M * 0.5 = 500K tokens * 4 = 2M chars, capped at 400K
        assert_eq!(config.max_section_chars(), 400_000);
    }

    #[test]
    fn test_truncate_context_sections() {
        let config = TruncationConfig {
            hard_max_chars: 30,
            min_keep_chars: 10,
            ..Default::default()
        };
        let sections = vec![
            ("short", "hello".to_string()),
            ("long", "line one\nline two\nline three\nline four\nline five\nline six".to_string()),
        ];
        let (result, any_truncated) = truncate_context_sections(sections, &config);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].1, "hello");
        assert!(any_truncated);
    }
}
