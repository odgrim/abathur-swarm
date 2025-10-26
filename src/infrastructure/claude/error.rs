/// Error types for Claude API client operations
use reqwest::StatusCode;
use thiserror::Error;

/// Errors that can occur when interacting with the Claude API
#[derive(Error, Debug, Clone)]
pub enum ClaudeApiError {
    /// Invalid request - malformed request body or parameters (400)
    #[error("Invalid request: {0}")]
    InvalidRequest(String),

    /// Invalid API key - authentication failed (401)
    #[error("Invalid API key")]
    InvalidApiKey,

    /// Forbidden - valid API key but insufficient permissions (403)
    #[error("Forbidden: {0}")]
    Forbidden(String),

    /// Resource not found (404)
    #[error("Resource not found")]
    NotFound,

    /// Rate limit exceeded - too many requests (429)
    #[error("Rate limit exceeded")]
    RateLimitExceeded,

    /// Server error - transient server-side error (500, 502, 503, 504, 529)
    #[error("Server error ({0}): {1}")]
    ServerError(StatusCode, String),

    /// Network error - connection failed, timeout, etc.
    #[error("Network error: {0}")]
    NetworkError(String),

    /// Unknown error - unexpected status code
    #[error("Unknown error ({0}): {1}")]
    UnknownError(StatusCode, String),

    /// Rate limiter error
    #[error("Rate limiter error: {0}")]
    RateLimiterError(String),
}

impl ClaudeApiError {
    /// Create an error from HTTP status code and response body
    pub fn from_status(status: StatusCode, body: String) -> Self {
        match status.as_u16() {
            400 => Self::InvalidRequest(body),
            401 => Self::InvalidApiKey,
            403 => Self::Forbidden(body),
            404 => Self::NotFound,
            429 => Self::RateLimitExceeded,
            500 | 502 | 503 | 504 | 529 => Self::ServerError(status, body),
            _ => Self::UnknownError(status, body),
        }
    }

    /// Check if the error is transient and should be retried
    pub fn is_transient(&self) -> bool {
        matches!(
            self,
            Self::RateLimitExceeded | Self::ServerError(_, _) | Self::NetworkError(_)
        )
    }

    /// Check if the error is permanent and should not be retried
    pub fn is_permanent(&self) -> bool {
        !self.is_transient()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_from_status_400() {
        let error = ClaudeApiError::from_status(StatusCode::BAD_REQUEST, "bad request".to_string());
        assert!(matches!(error, ClaudeApiError::InvalidRequest(_)));
        assert!(!error.is_transient());
    }

    #[test]
    fn test_from_status_401() {
        let error = ClaudeApiError::from_status(StatusCode::UNAUTHORIZED, String::new());
        assert!(matches!(error, ClaudeApiError::InvalidApiKey));
        assert!(!error.is_transient());
    }

    #[test]
    fn test_from_status_429() {
        let error = ClaudeApiError::from_status(StatusCode::TOO_MANY_REQUESTS, String::new());
        assert!(matches!(error, ClaudeApiError::RateLimitExceeded));
        assert!(error.is_transient());
    }

    #[test]
    fn test_from_status_500() {
        let error =
            ClaudeApiError::from_status(StatusCode::INTERNAL_SERVER_ERROR, "error".to_string());
        assert!(matches!(error, ClaudeApiError::ServerError(_, _)));
        assert!(error.is_transient());
    }

    #[test]
    fn test_from_status_529() {
        let error = ClaudeApiError::from_status(StatusCode::from_u16(529).unwrap(), "overloaded".to_string());
        assert!(matches!(error, ClaudeApiError::ServerError(_, _)));
        assert!(error.is_transient());
    }
}
