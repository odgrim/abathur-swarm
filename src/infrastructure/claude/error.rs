use thiserror::Error;

/// Errors that can occur when interacting with the Claude API
#[derive(Error, Debug)]
pub enum ClaudeApiError {
    /// Invalid request parameters or malformed request
    #[error("Invalid request: {0}")]
    InvalidRequest(String),

    /// Authentication failed due to invalid or missing API key
    #[error("Authentication failed: {0}")]
    AuthenticationFailed(String),

    /// Rate limit exceeded, retry after waiting
    #[error("Rate limit exceeded")]
    RateLimitExceeded,

    /// API server encountered an internal error
    #[error("API server error: {0}")]
    ServerError(String),

    /// API server is overloaded, retry later
    #[error("API server overloaded")]
    Overloaded,

    /// Network error occurred during request
    #[error("Network error: {0}")]
    NetworkError(#[from] reqwest::Error),

    /// JSON serialization or deserialization error
    #[error("JSON serialization error: {0}")]
    SerializationError(#[from] serde_json::Error),

    /// Request timed out waiting for response
    #[error("Timeout waiting for response")]
    Timeout,

    /// Unknown error occurred
    #[error("Unknown error: {0}")]
    Unknown(String),
}

impl ClaudeApiError {
    /// Returns true if this error is transient and should be retried
    ///
    /// Transient errors include:
    /// - Rate limit exceeded
    /// - Server errors (5xx)
    /// - Server overloaded
    /// - Timeout
    ///
    /// # Examples
    ///
    /// ```
    /// use abathur::infrastructure::claude::error::ClaudeApiError;
    ///
    /// let error = ClaudeApiError::RateLimitExceeded;
    /// assert!(error.is_transient());
    ///
    /// let error = ClaudeApiError::AuthenticationFailed("Invalid key".to_string());
    /// assert!(!error.is_transient());
    /// ```
    pub fn is_transient(&self) -> bool {
        matches!(
            self,
            ClaudeApiError::RateLimitExceeded
                | ClaudeApiError::ServerError(_)
                | ClaudeApiError::Overloaded
                | ClaudeApiError::Timeout
        )
    }

    /// Create error from HTTP status code and response body
    ///
    /// Maps HTTP status codes to appropriate error variants according to
    /// the Claude API specification:
    /// - 400: Invalid request
    /// - 401, 403: Authentication failed
    /// - 429: Rate limit exceeded
    /// - 500: Server error
    /// - 529: Server overloaded
    /// - Other: Unknown error
    ///
    /// # Examples
    ///
    /// ```
    /// use abathur::infrastructure::claude::error::ClaudeApiError;
    /// use reqwest::StatusCode;
    ///
    /// let error = ClaudeApiError::from_status(
    ///     StatusCode::TOO_MANY_REQUESTS,
    ///     "Rate limit exceeded".to_string()
    /// );
    /// assert!(matches!(error, ClaudeApiError::RateLimitExceeded));
    /// ```
    pub fn from_status(status: reqwest::StatusCode, body: String) -> Self {
        match status.as_u16() {
            400 => ClaudeApiError::InvalidRequest(body),
            401 | 403 => ClaudeApiError::AuthenticationFailed(body),
            429 => ClaudeApiError::RateLimitExceeded,
            500 => ClaudeApiError::ServerError(body),
            529 => ClaudeApiError::Overloaded,
            _ => ClaudeApiError::Unknown(format!("HTTP {}: {}", status, body)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::StatusCode;

    #[test]
    fn test_is_transient_rate_limit() {
        let error = ClaudeApiError::RateLimitExceeded;
        assert!(error.is_transient());
    }

    #[test]
    fn test_is_transient_server_error() {
        let error = ClaudeApiError::ServerError("Internal error".to_string());
        assert!(error.is_transient());
    }

    #[test]
    fn test_is_transient_overloaded() {
        let error = ClaudeApiError::Overloaded;
        assert!(error.is_transient());
    }

    #[test]
    fn test_is_transient_timeout() {
        let error = ClaudeApiError::Timeout;
        assert!(error.is_transient());
    }

    #[test]
    fn test_is_not_transient_invalid_request() {
        let error = ClaudeApiError::InvalidRequest("Bad params".to_string());
        assert!(!error.is_transient());
    }

    #[test]
    fn test_is_not_transient_authentication_failed() {
        let error = ClaudeApiError::AuthenticationFailed("Invalid key".to_string());
        assert!(!error.is_transient());
    }

    #[test]
    fn test_is_not_transient_unknown() {
        let error = ClaudeApiError::Unknown("Something went wrong".to_string());
        assert!(!error.is_transient());
    }

    #[test]
    fn test_from_status_400() {
        let error =
            ClaudeApiError::from_status(StatusCode::BAD_REQUEST, "Invalid parameters".to_string());
        assert!(matches!(error, ClaudeApiError::InvalidRequest(_)));
    }

    #[test]
    fn test_from_status_401() {
        let error =
            ClaudeApiError::from_status(StatusCode::UNAUTHORIZED, "Invalid API key".to_string());
        assert!(matches!(error, ClaudeApiError::AuthenticationFailed(_)));
    }

    #[test]
    fn test_from_status_403() {
        let error = ClaudeApiError::from_status(StatusCode::FORBIDDEN, "Access denied".to_string());
        assert!(matches!(error, ClaudeApiError::AuthenticationFailed(_)));
    }

    #[test]
    fn test_from_status_429() {
        let error = ClaudeApiError::from_status(
            StatusCode::TOO_MANY_REQUESTS,
            "Rate limit exceeded".to_string(),
        );
        assert!(matches!(error, ClaudeApiError::RateLimitExceeded));
    }

    #[test]
    fn test_from_status_500() {
        let error = ClaudeApiError::from_status(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Server error".to_string(),
        );
        assert!(matches!(error, ClaudeApiError::ServerError(_)));
    }

    #[test]
    fn test_from_status_529() {
        let error = ClaudeApiError::from_status(
            StatusCode::from_u16(529).unwrap(),
            "Overloaded".to_string(),
        );
        assert!(matches!(error, ClaudeApiError::Overloaded));
    }

    #[test]
    fn test_from_status_unknown() {
        let error =
            ClaudeApiError::from_status(StatusCode::IM_A_TEAPOT, "I'm a teapot".to_string());
        assert!(matches!(error, ClaudeApiError::Unknown(_)));
        // StatusCode formats as "418 I'm a teapot" (lowercase canonical reason phrase)
        let error_msg = error.to_string();
        assert!(error_msg.starts_with("Unknown error: HTTP 418"));
        assert!(error_msg.contains("I'm a teapot"));
    }

    #[test]
    fn test_error_display() {
        let error = ClaudeApiError::InvalidRequest("Bad params".to_string());
        assert_eq!(error.to_string(), "Invalid request: Bad params");

        let error = ClaudeApiError::AuthenticationFailed("Invalid key".to_string());
        assert_eq!(error.to_string(), "Authentication failed: Invalid key");

        let error = ClaudeApiError::RateLimitExceeded;
        assert_eq!(error.to_string(), "Rate limit exceeded");

        let error = ClaudeApiError::ServerError("Internal error".to_string());
        assert_eq!(error.to_string(), "API server error: Internal error");

        let error = ClaudeApiError::Overloaded;
        assert_eq!(error.to_string(), "API server overloaded");

        let error = ClaudeApiError::Timeout;
        assert_eq!(error.to_string(), "Timeout waiting for response");
    }

    // Note: From<reqwest::Error> conversion is automatically tested by the compiler
    // due to the #[from] attribute on the NetworkError variant

    #[test]
    fn test_from_serde_error() {
        let json = r#"{"invalid": json}"#;
        let serde_error: serde_json::Error =
            serde_json::from_str::<serde_json::Value>(json).unwrap_err();
        let claude_error: ClaudeApiError = serde_error.into();
        assert!(matches!(
            claude_error,
            ClaudeApiError::SerializationError(_)
        ));
    }
}
