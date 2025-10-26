/// Retry policy with exponential backoff for Claude API requests
use std::future::Future;
use std::time::Duration;
use tokio::time::sleep;

use super::error::ClaudeApiError;

/// Retry policy with exponential backoff
///
/// Implements retry logic with exponential backoff for transient errors.
/// Backoff time doubles with each retry: 10s → 20s → 40s → 80s → 160s → 300s (max)
///
/// # Retry Decision
/// - Retry on: 429 (rate limit), 500, 502, 503, 504, 529 (server errors), network errors
/// - Do NOT retry: 400, 401, 403, 404 (client errors)
pub struct RetryPolicy {
    /// Maximum number of retries before giving up
    pub max_retries: u32,

    /// Initial backoff duration in milliseconds
    pub initial_backoff_ms: u64,

    /// Maximum backoff duration in milliseconds
    pub max_backoff_ms: u64,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_retries: 3,
            initial_backoff_ms: 10_000,  // 10 seconds
            max_backoff_ms: 300_000,     // 5 minutes
        }
    }
}

impl RetryPolicy {
    /// Create a new retry policy with custom settings
    pub fn new(max_retries: u32, initial_backoff_ms: u64, max_backoff_ms: u64) -> Self {
        Self {
            max_retries,
            initial_backoff_ms,
            max_backoff_ms,
        }
    }

    /// Execute an async operation with retry logic
    ///
    /// # Arguments
    /// * `operation` - Async function to execute, returns Result<T, anyhow::Error>
    ///
    /// # Returns
    /// * `Ok(T)` - Operation succeeded
    /// * `Err(anyhow::Error)` - Operation failed after all retries
    ///
    /// # Type Parameters
    /// * `F` - Future factory function
    /// * `Fut` - Future type returned by F
    /// * `T` - Success type
    ///
    /// # Example
    /// ```
    /// use retry_policy::RetryPolicy;
    ///
    /// let policy = RetryPolicy::default();
    /// let result = policy.execute(|| async {
    ///     // Your async operation here
    ///     Ok(42)
    /// }).await;
    /// ```
    pub async fn execute<F, Fut, T>(&self, mut operation: F) -> Result<T, anyhow::Error>
    where
        F: FnMut() -> Fut,
        Fut: Future<Output = Result<T, anyhow::Error>>,
    {
        let mut attempt = 0;

        loop {
            match operation().await {
                Ok(result) => return Ok(result),
                Err(err) => {
                    // Check if we should retry
                    let should_retry = if let Some(claude_err) = err.downcast_ref::<ClaudeApiError>() {
                        claude_err.is_transient() && attempt < self.max_retries
                    } else {
                        // For non-ClaudeApiError, retry network-like errors
                        attempt < self.max_retries
                    };

                    if !should_retry {
                        return Err(err);
                    }

                    // Calculate backoff duration
                    let backoff = self.calculate_backoff(attempt);

                    // Log retry attempt (in production, use tracing)
                    eprintln!(
                        "Retry attempt {}/{}: waiting {:?} before retry. Error: {}",
                        attempt + 1,
                        self.max_retries,
                        backoff,
                        err
                    );

                    // Wait before retrying
                    sleep(backoff).await;

                    attempt += 1;
                }
            }
        }
    }

    /// Calculate backoff duration for a given attempt
    ///
    /// Uses exponential backoff: initial * 2^attempt, capped at max_backoff_ms
    ///
    /// # Arguments
    /// * `attempt` - Current retry attempt number (0-indexed)
    ///
    /// # Returns
    /// Duration to wait before next retry
    fn calculate_backoff(&self, attempt: u32) -> Duration {
        let backoff_ms = self
            .initial_backoff_ms
            .saturating_mul(2_u64.saturating_pow(attempt))
            .min(self.max_backoff_ms);

        Duration::from_millis(backoff_ms)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::anyhow;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;

    #[test]
    fn test_calculate_backoff() {
        let policy = RetryPolicy::default();

        assert_eq!(policy.calculate_backoff(0), Duration::from_millis(10_000));
        assert_eq!(policy.calculate_backoff(1), Duration::from_millis(20_000));
        assert_eq!(policy.calculate_backoff(2), Duration::from_millis(40_000));
        assert_eq!(policy.calculate_backoff(3), Duration::from_millis(80_000));
        assert_eq!(policy.calculate_backoff(4), Duration::from_millis(160_000));
        assert_eq!(policy.calculate_backoff(5), Duration::from_millis(300_000)); // Capped at max
        assert_eq!(policy.calculate_backoff(6), Duration::from_millis(300_000)); // Still capped
    }

    #[tokio::test]
    async fn test_retry_success_on_first_attempt() {
        let policy = RetryPolicy::default();
        let call_count = Arc::new(AtomicU32::new(0));

        let result = policy
            .execute(|| {
                let count = Arc::clone(&call_count);
                async move {
                    count.fetch_add(1, Ordering::SeqCst);
                    Ok(42)
                }
            })
            .await;

        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 42);
        assert_eq!(call_count.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn test_retry_success_on_second_attempt() {
        let policy = RetryPolicy::new(3, 100, 1000); // Fast retries for testing
        let call_count = Arc::new(AtomicU32::new(0));

        let result = policy
            .execute(|| {
                let count = Arc::clone(&call_count);
                async move {
                    let attempt = count.fetch_add(1, Ordering::SeqCst);
                    if attempt == 0 {
                        Err(anyhow!(ClaudeApiError::ServerError(
                            reqwest::StatusCode::INTERNAL_SERVER_ERROR,
                            "Server error".to_string()
                        )))
                    } else {
                        Ok(42)
                    }
                }
            })
            .await;

        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 42);
        assert_eq!(call_count.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn test_retry_permanent_error() {
        let policy = RetryPolicy::new(3, 100, 1000);
        let call_count = Arc::new(AtomicU32::new(0));

        let result: Result<(), _> = policy
            .execute(|| {
                let count = Arc::clone(&call_count);
                async move {
                    count.fetch_add(1, Ordering::SeqCst);
                    Err(anyhow!(ClaudeApiError::InvalidApiKey))
                }
            })
            .await;

        assert!(result.is_err());
        // Should not retry permanent errors
        assert_eq!(call_count.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn test_retry_exhausted() {
        let policy = RetryPolicy::new(2, 100, 1000);
        let call_count = Arc::new(AtomicU32::new(0));

        let result: Result<(), _> = policy
            .execute(|| {
                let count = Arc::clone(&call_count);
                async move {
                    count.fetch_add(1, Ordering::SeqCst);
                    Err(anyhow!(ClaudeApiError::RateLimitExceeded))
                }
            })
            .await;

        assert!(result.is_err());
        // Should try 3 times total (initial + 2 retries)
        assert_eq!(call_count.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn test_retry_transient_errors() {
        let policy = RetryPolicy::new(1, 100, 1000);

        // Test that transient errors are retried
        let transient_errors = vec![
            ClaudeApiError::RateLimitExceeded,
            ClaudeApiError::ServerError(
                reqwest::StatusCode::INTERNAL_SERVER_ERROR,
                "".to_string(),
            ),
            ClaudeApiError::ServerError(reqwest::StatusCode::BAD_GATEWAY, "".to_string()),
            ClaudeApiError::ServerError(
                reqwest::StatusCode::SERVICE_UNAVAILABLE,
                "".to_string(),
            ),
            ClaudeApiError::ServerError(reqwest::StatusCode::GATEWAY_TIMEOUT, "".to_string()),
        ];

        for error in transient_errors {
            let call_count = Arc::new(AtomicU32::new(0));
            let error_clone = error.clone();

            let result: Result<(), _> = policy
                .execute(|| {
                    let count = Arc::clone(&call_count);
                    let err = error_clone.clone();
                    async move {
                        count.fetch_add(1, Ordering::SeqCst);
                        Err(anyhow!(err))
                    }
                })
                .await;

            assert!(result.is_err());
            // Should retry once (initial + 1 retry = 2 calls)
            assert_eq!(
                call_count.load(Ordering::SeqCst),
                2,
                "Should retry for {:?}",
                error
            );
        }
    }
}
