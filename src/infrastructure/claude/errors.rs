use reqwest::StatusCode;
use thiserror::Error;

/// Errors that can occur when interacting with the Claude API
#[derive(Error, Debug)]
pub enum ClaudeApiError {
    /// Invalid request parameters (HTTP 400)
    #[error("Invalid request: {0}")]
    InvalidRequest(String),

    /// Invalid or missing API key (HTTP 401)
    #[error("Invalid API key - authentication failed")]
    InvalidApiKey,

    /// Forbidden - permission denied (HTTP 403)
    #[error("Forbidden: {0}")]
    Forbidden(String),

    /// Resource not found (HTTP 404)
    #[error("Resource not found")]
    NotFound,

    /// Rate limit exceeded (HTTP 429)
    #[error("Rate limit exceeded - too many requests")]
    RateLimitExceeded,

    /// Server error from Claude API (HTTP 500, 502, 503, 504, 529)
    #[error("Server error ({0}): {1}")]
    ServerError(StatusCode, String),

    /// Network or connection error
    #[error("Network error: {0}")]
    NetworkError(#[from] reqwest::Error),

    /// JSON serialization/deserialization error
    #[error("JSON error: {0}")]
    JsonError(#[from] serde_json::Error),

    /// Request timeout
    #[error("Request timeout")]
    Timeout,

    /// Unknown or unexpected error
    #[error("Unknown error ({0}): {1}")]
    UnknownError(StatusCode, String),
}

impl ClaudeApiError {
    /// Returns true if this error is transient and should be retried
    pub fn is_transient(&self) -> bool {
        matches!(
            self,
            ClaudeApiError::RateLimitExceeded
                | ClaudeApiError::ServerError(_, _)
                | ClaudeApiError::Timeout
                | ClaudeApiError::NetworkError(_)
        )
    }

    /// Returns true if this is a permanent error that should not be retried
    pub fn is_permanent(&self) -> bool {
        matches!(
            self,
            ClaudeApiError::InvalidRequest(_)
                | ClaudeApiError::InvalidApiKey
                | ClaudeApiError::Forbidden(_)
                | ClaudeApiError::NotFound
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_transient_errors() {
        assert!(ClaudeApiError::RateLimitExceeded.is_transient());
        assert!(
            ClaudeApiError::ServerError(StatusCode::INTERNAL_SERVER_ERROR, "test".to_string())
                .is_transient()
        );
        assert!(ClaudeApiError::Timeout.is_transient());
    }

    #[test]
    fn test_permanent_errors() {
        assert!(ClaudeApiError::InvalidRequest("test".to_string()).is_permanent());
        assert!(ClaudeApiError::InvalidApiKey.is_permanent());
        assert!(ClaudeApiError::Forbidden("test".to_string()).is_permanent());
        assert!(ClaudeApiError::NotFound.is_permanent());
    }

    #[test]
    fn test_error_exclusivity() {
        let rate_limit_error = ClaudeApiError::RateLimitExceeded;
        assert!(rate_limit_error.is_transient());
        assert!(!rate_limit_error.is_permanent());

        let invalid_request_error = ClaudeApiError::InvalidRequest("test".to_string());
        assert!(!invalid_request_error.is_transient());
        assert!(invalid_request_error.is_permanent());
    }
}
