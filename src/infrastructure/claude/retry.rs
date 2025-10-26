use super::errors::ClaudeApiError;
use std::future::Future;
use std::time::Duration;
use tokio::time::sleep;
use tracing::{debug, warn};

/// Retry policy configuration for handling transient errors
#[derive(Debug, Clone)]
pub struct RetryPolicy {
    /// Maximum number of retry attempts
    max_retries: u32,
    /// Initial backoff duration in milliseconds
    initial_backoff_ms: u64,
    /// Maximum backoff duration in milliseconds
    max_backoff_ms: u64,
}

impl RetryPolicy {
    /// Create a new retry policy
    ///
    /// # Arguments
    /// * `max_retries` - Maximum retry attempts (recommended: 3)
    /// * `initial_backoff_ms` - Starting backoff delay (recommended: 10000ms = 10s)
    /// * `max_backoff_ms` - Maximum backoff delay (recommended: 300000ms = 5min)
    ///
    /// # Example
    /// ```
    /// use abathur::infrastructure::claude::retry::RetryPolicy;
    ///
    /// let policy = RetryPolicy::new(3, 10_000, 300_000);
    /// ```
    pub fn new(max_retries: u32, initial_backoff_ms: u64, max_backoff_ms: u64) -> Self {
        assert!(max_retries > 0, "max_retries must be greater than 0");
        assert!(
            initial_backoff_ms > 0,
            "initial_backoff_ms must be greater than 0"
        );
        assert!(
            max_backoff_ms >= initial_backoff_ms,
            "max_backoff_ms must be >= initial_backoff_ms"
        );

        Self {
            max_retries,
            initial_backoff_ms,
            max_backoff_ms,
        }
    }

    /// Execute an operation with exponential backoff retry logic
    ///
    /// # Arguments
    /// * `operation` - Async function that returns Result<T, ClaudeApiError>
    ///
    /// # Returns
    /// * `Ok(T)` - Operation succeeded
    /// * `Err(ClaudeApiError)` - Operation failed after all retries
    ///
    /// # Example
    /// ```no_run
    /// # use abathur::infrastructure::claude::retry::RetryPolicy;
    /// # use abathur::infrastructure::claude::errors::ClaudeApiError;
    /// # async fn example() -> Result<String, ClaudeApiError> {
    /// let policy = RetryPolicy::new(3, 10_000, 300_000);
    ///
    /// let result = policy.execute(|| async {
    ///     // Your operation here
    ///     Ok("success".to_string())
    /// }).await?;
    /// # Ok(result)
    /// # }
    /// ```
    pub async fn execute<F, Fut, T>(&self, mut operation: F) -> Result<T, ClaudeApiError>
    where
        F: FnMut() -> Fut,
        Fut: Future<Output = Result<T, ClaudeApiError>>,
    {
        let mut attempt = 0;

        loop {
            match operation().await {
                Ok(result) => {
                    if attempt > 0 {
                        debug!("Operation succeeded after {} retries", attempt);
                    }
                    return Ok(result);
                }
                Err(err) => {
                    if self.should_retry(&err, attempt) {
                        let backoff = self.calculate_backoff(attempt);
                        warn!(
                            "Attempt {} failed with transient error: {}. Retrying in {:?}...",
                            attempt + 1,
                            err,
                            backoff
                        );

                        sleep(backoff).await;
                        attempt += 1;
                    } else {
                        if attempt >= self.max_retries {
                            warn!("Operation failed after {} attempts: {}", attempt + 1, err);
                        } else {
                            debug!("Permanent error, not retrying: {}", err);
                        }
                        return Err(err);
                    }
                }
            }
        }
    }

    /// Calculate exponential backoff duration for a given attempt
    ///
    /// Formula: min(initial_backoff * 2^attempt, max_backoff)
    fn calculate_backoff(&self, attempt: u32) -> Duration {
        let backoff_ms = self
            .initial_backoff_ms
            .saturating_mul(2_u64.saturating_pow(attempt))
            .min(self.max_backoff_ms);

        Duration::from_millis(backoff_ms)
    }

    /// Determine if an error should be retried
    ///
    /// Returns true if:
    /// - Attempt count is below max_retries AND
    /// - Error is transient (rate limit, server error, timeout, network error)
    ///
    /// Returns false if:
    /// - Max retries exceeded OR
    /// - Error is permanent (400, 401, 403, 404)
    fn should_retry(&self, error: &ClaudeApiError, attempt: u32) -> bool {
        if attempt >= self.max_retries {
            return false;
        }

        error.is_transient()
    }
}

impl Default for RetryPolicy {
    /// Create a retry policy with recommended defaults:
    /// - Max retries: 3
    /// - Initial backoff: 10 seconds
    /// - Max backoff: 5 minutes
    fn default() -> Self {
        Self::new(3, 10_000, 300_000)
    }
                }
            })
            .await;

        assert_eq!(result.unwrap(), 42);
        assert_eq!(*counter.lock().unwrap(), 1);
    }

    #[tokio::test]
    async fn test_execute_retries_on_transient_error() {
        let policy = RetryPolicy::new(3, 100, 1000);
        let counter = Arc::new(StdMutex::new(0));

        let result = policy
            .execute(|| {
                let counter = counter.clone();
                async move {
                    let mut count = counter.lock().unwrap();
                    *count += 1;

                    if *count < 3 {
                        Err(ClaudeApiError::RateLimitExceeded)
                    } else {
                        Ok::<i32, ClaudeApiError>(42)
                    }
                }
            })
            .await;

        assert_eq!(result.unwrap(), 42);
        assert_eq!(*counter.lock().unwrap(), 3);
    }

    #[tokio::test]
    async fn test_execute_fails_on_permanent_error() {
        let policy = RetryPolicy::new(3, 100, 1000);
        let counter = Arc::new(StdMutex::new(0));

        let result = policy
            .execute(|| {
                let counter = counter.clone();
                async move {
                    let mut count = counter.lock().unwrap();
                    *count += 1;
                    Err::<i32, ClaudeApiError>(ClaudeApiError::InvalidApiKey)
                }
            })
            .await;

        assert!(result.is_err());
        assert_eq!(*counter.lock().unwrap(), 1); // No retries for permanent error
    }

    #[tokio::test]
    async fn test_execute_fails_after_max_retries() {
        let policy = RetryPolicy::new(2, 100, 1000);
        let counter = Arc::new(StdMutex::new(0));

        let result = policy
            .execute(|| {
                let counter = counter.clone();
                async move {
                    let mut count = counter.lock().unwrap();
                    *count += 1;
                    Err::<i32, ClaudeApiError>(ClaudeApiError::RateLimitExceeded)
                }
            })
            .await;

        assert!(result.is_err());
        assert_eq!(*counter.lock().unwrap(), 3); // Initial + 2 retries
    }

    #[test]
    fn test_default_retry_policy() {
        let policy = RetryPolicy::default();
        assert_eq!(policy.max_retries, 3);
        assert_eq!(policy.initial_backoff_ms, 10_000);
        assert_eq!(policy.max_backoff_ms, 300_000);
    }
}
