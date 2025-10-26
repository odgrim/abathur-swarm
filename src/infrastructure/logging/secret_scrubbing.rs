use regex::Regex;
use std::fmt;
use tracing::Subscriber;
use tracing_subscriber::Layer;

/// Layer that scrubs sensitive data from log messages
#[derive(Clone)]
pub struct SecretScrubbingLayer {
    api_key_pattern: Regex,
    token_pattern: Regex,
    bearer_pattern: Regex,
    password_pattern: Regex,
}

impl SecretScrubbingLayer {
    /// Create a new secret scrubbing layer
    pub fn new() -> Self {
        Self {
            // Match Anthropic API keys: sk-ant-api03-...
            api_key_pattern: Regex::new(r"sk-ant-[a-zA-Z0-9-_]{20,}").unwrap(),
            // Match generic tokens
            token_pattern: Regex::new(r#"["']?(?:api_key|apikey|token|secret)["']?\s*[:=]\s*["']?([a-zA-Z0-9-_\.]{20,})["']?"#).unwrap(),
            // Match Bearer tokens in Authorization headers
            bearer_pattern: Regex::new(r"Bearer\s+[a-zA-Z0-9-_\.]+").unwrap(),
            // Match password fields
            password_pattern: Regex::new(r#"["']?password["']?\s*[:=]\s*["']?([^"'\s,}]+)["']?"#).unwrap(),
        }
    }

    /// Scrub a message of sensitive data
    pub fn scrub_message(&self, message: &str) -> String {
        let mut scrubbed = self.api_key_pattern
            .replace_all(message, "[API_KEY_REDACTED]")
            .to_string();
        scrubbed = self.bearer_pattern
            .replace_all(&scrubbed, "Bearer [TOKEN_REDACTED]")
            .to_string();
        scrubbed = self.token_pattern
            .replace_all(&scrubbed, |caps: &regex::Captures| {
                // Extract the field name before the value
                let full_match = &caps[0];
                if let Some(colon_pos) = full_match.find(':') {
                    format!("{}:[REDACTED]", &full_match[..colon_pos])
                } else if let Some(eq_pos) = full_match.find('=') {
                    format!("{}=[REDACTED]", &full_match[..eq_pos])
                } else {
                    "[REDACTED]".to_string()
                }
            })
            .to_string();
        scrubbed = self.password_pattern
            .replace_all(&scrubbed, "password=[REDACTED]")
            .to_string();
        scrubbed
    }
}

impl Default for SecretScrubbingLayer {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for SecretScrubbingLayer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SecretScrubbingLayer").finish()
    }
}

// Note: Full implementation of Layer trait would require intercepting
// all event formatting, which is complex. For now, we provide the
// scrubbing functionality that can be used by the formatter.
// In production, this would be integrated into a custom visitor or format layer.
impl<S: Subscriber> Layer<S> for SecretScrubbingLayer {
    // Layer implementation is intentionally minimal
    // The scrubbing is applied at the formatter level via scrub_message
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_scrub_anthropic_api_key() {
        let scrubber = SecretScrubbingLayer::new();
        let message = "Using API key sk-ant-api03-abc123def456ghi789 for request";
        let scrubbed = scrubber.scrub_message(message);

        assert!(!scrubbed.contains("sk-ant-api03-abc123def456ghi789"));
        assert!(scrubbed.contains("[API_KEY_REDACTED]"));
    }

    #[test]
    fn test_scrub_bearer_token() {
        let scrubber = SecretScrubbingLayer::new();
        let message = "Authorization: Bearer eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0";
        let scrubbed = scrubber.scrub_message(message);

        assert!(!scrubbed.contains("eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9"));
        assert!(scrubbed.contains("Bearer [TOKEN_REDACTED]"));
    }

    #[test]
    fn test_scrub_api_key_field() {
        let scrubber = SecretScrubbingLayer::new();
        let message = r#"{"api_key": "sk-1234567890abcdefghij"}"#;
        let scrubbed = scrubber.scrub_message(message);

        assert!(!scrubbed.contains("sk-1234567890abcdefghij"));
        assert!(scrubbed.contains("[REDACTED]"));
    }

    #[test]
    fn test_scrub_password_field() {
        let scrubber = SecretScrubbingLayer::new();
        let message = r#"{"password": "super_secret_password"}"#;
        let scrubbed = scrubber.scrub_message(message);

        assert!(!scrubbed.contains("super_secret_password"));
        assert!(scrubbed.contains("[REDACTED]"));
    }

    #[test]
    fn test_scrub_multiple_secrets() {
        let scrubber = SecretScrubbingLayer::new();
        let message = "api_key=sk-ant-api03-test123 password=secret123 Bearer token_here";
        let scrubbed = scrubber.scrub_message(message);

        assert!(!scrubbed.contains("sk-ant-api03-test123"));
        assert!(!scrubbed.contains("secret123"));
        assert!(!scrubbed.contains("token_here"));
        assert!(scrubbed.contains("[REDACTED]"));
    }

    #[test]
    fn test_no_scrubbing_needed() {
        let scrubber = SecretScrubbingLayer::new();
        let message = "This is a normal log message with no secrets";
        let scrubbed = scrubber.scrub_message(message);

        assert_eq!(message, scrubbed);
    }
}
