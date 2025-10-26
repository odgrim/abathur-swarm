<<<<<<< HEAD
=======
/// Token bucket rate limiter for Claude API requests
>>>>>>> task_claude-api-integration-tests_20251025-210007
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;
use tokio::time::sleep;

<<<<<<< HEAD
/// Token bucket rate limiter for API request throttling
///
/// Implements the token bucket algorithm to ensure API requests
/// stay within configured rate limits.
#[derive(Clone)]
pub struct TokenBucketRateLimiter {
    /// Current number of available tokens
    tokens: Arc<Mutex<f64>>,
    /// Maximum token capacity (should equal refill_rate for burst tolerance)
    capacity: f64,
    /// Tokens added per second
    refill_rate: f64,
=======
/// Token bucket rate limiter
///
/// Implements the token bucket algorithm for rate limiting API requests.
/// Tokens are refilled continuously based on elapsed time.
///
/// # Algorithm
/// - Capacity: Maximum number of tokens (burst capacity)
/// - Refill rate: Tokens added per second
/// - On acquire: Wait until at least 1 token is available, then consume it
/// - Refill: Tokens = min(tokens + elapsed_seconds * refill_rate, capacity)
pub struct TokenBucketRateLimiter {
    /// Current number of tokens (protected by mutex for async safety)
    tokens: Arc<Mutex<f64>>,

    /// Maximum number of tokens (burst capacity)
    capacity: f64,

    /// Tokens added per second
    refill_rate: f64,

>>>>>>> task_claude-api-integration-tests_20251025-210007
    /// Last time tokens were refilled
    last_refill: Arc<Mutex<Instant>>,
}

impl TokenBucketRateLimiter {
<<<<<<< HEAD
    /// Create a new rate limiter
    ///
    /// # Arguments
    /// * `rate_limit_rps` - Requests per second allowed (e.g., 10.0 for 10 requests/sec)
    ///
    /// # Example
    /// ```
    /// use abathur::infrastructure::claude::rate_limiter::TokenBucketRateLimiter;
    ///
    /// let rate_limiter = TokenBucketRateLimiter::new(10.0);
    /// ```
    pub fn new(rate_limit_rps: f64) -> Self {
        assert!(rate_limit_rps > 0.0, "Rate limit must be positive");

        Self {
            tokens: Arc::new(Mutex::new(rate_limit_rps)),
            capacity: rate_limit_rps,
            refill_rate: rate_limit_rps,
=======
    /// Create a new token bucket rate limiter
    ///
    /// # Arguments
    /// * `requests_per_second` - Maximum sustained request rate (refill rate)
    ///
    /// # Returns
    /// A new `TokenBucketRateLimiter` with capacity equal to refill rate
    ///
    /// # Example
    /// ```
    /// use token_bucket_rate_limiter::TokenBucketRateLimiter;
    ///
    /// // Allow 10 requests per second
    /// let rate_limiter = TokenBucketRateLimiter::new(10.0);
    /// ```
    pub fn new(requests_per_second: f64) -> Self {
        assert!(requests_per_second > 0.0, "requests_per_second must be positive");

        Self {
            tokens: Arc::new(Mutex::new(requests_per_second)), // Start with full capacity
            capacity: requests_per_second,
            refill_rate: requests_per_second,
>>>>>>> task_claude-api-integration-tests_20251025-210007
            last_refill: Arc::new(Mutex::new(Instant::now())),
        }
    }

<<<<<<< HEAD
    /// Acquire a token from the bucket, waiting if necessary
    ///
    /// This method blocks until a token is available. Tokens are automatically
    /// refilled based on the elapsed time since the last refill.
    ///
    /// # Example
    /// ```no_run
    /// # use abathur::infrastructure::claude::rate_limiter::TokenBucketRateLimiter;
    /// # async fn example() {
    /// let rate_limiter = TokenBucketRateLimiter::new(10.0);
    /// rate_limiter.acquire().await;
    /// // Make API request here
    /// # }
    /// ```
    pub async fn acquire(&self) {
=======
    /// Acquire a token, waiting if necessary
    ///
    /// This method blocks until at least one token is available, then consumes it.
    /// Tokens are refilled continuously based on elapsed time since last refill.
    ///
    /// # Errors
    /// Returns an error if an internal lock is poisoned (should never happen in practice)
    pub async fn acquire(&self) -> Result<(), String> {
>>>>>>> task_claude-api-integration-tests_20251025-210007
        loop {
            let mut tokens = self.tokens.lock().await;
            let mut last_refill = self.last_refill.lock().await;

            // Refill tokens based on elapsed time
            let now = Instant::now();
            let elapsed = now.duration_since(*last_refill).as_secs_f64();
            let new_tokens = (*tokens + elapsed * self.refill_rate).min(self.capacity);

<<<<<<< HEAD
            if new_tokens >= 1.0 {
                // Token available - consume it and proceed
                *tokens = new_tokens - 1.0;
                *last_refill = now;
                break;
            }

            // No tokens available - calculate wait time
            let tokens_needed = 1.0 - new_tokens;
            let wait_time_secs = tokens_needed / self.refill_rate;
            let wait_duration = Duration::from_secs_f64(wait_time_secs.max(0.01));
=======
            // If we have at least 1 token, consume it and return
            if new_tokens >= 1.0 {
                *tokens = new_tokens - 1.0;
                *last_refill = now;
                return Ok(());
            }

            // Calculate how long to wait until next token is available
            let tokens_needed = 1.0 - new_tokens;
            let wait_time_secs = tokens_needed / self.refill_rate;
            let wait_duration = Duration::from_secs_f64(wait_time_secs);
>>>>>>> task_claude-api-integration-tests_20251025-210007

            // Release locks before sleeping
            drop(tokens);
            drop(last_refill);

            // Wait before retrying
            sleep(wait_duration).await;
        }
    }

<<<<<<< HEAD
    /// Get the current number of available tokens (for testing/monitoring)
    #[cfg(test)]
=======
    /// Get current number of available tokens (for testing/monitoring)
>>>>>>> task_claude-api-integration-tests_20251025-210007
    pub async fn available_tokens(&self) -> f64 {
        let tokens = self.tokens.lock().await;
        let last_refill = self.last_refill.lock().await;

        let now = Instant::now();
        let elapsed = now.duration_since(*last_refill).as_secs_f64();
        (*tokens + elapsed * self.refill_rate).min(self.capacity)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
<<<<<<< HEAD
    use tokio::time::{Instant, sleep};

    #[tokio::test]
    async fn test_rate_limiter_allows_initial_requests() {
        let rate_limiter = TokenBucketRateLimiter::new(10.0);

        // Should allow immediate requests up to capacity
        rate_limiter.acquire().await;
        rate_limiter.acquire().await;
        rate_limiter.acquire().await;

        // Verify tokens were consumed
        let tokens = rate_limiter.available_tokens().await;
        assert!(tokens < 10.0);
    }

    #[tokio::test]
    async fn test_rate_limiter_enforces_delay() {
        let rate_limiter = TokenBucketRateLimiter::new(2.0); // 2 requests/sec

        // Consume all tokens
        rate_limiter.acquire().await;
        rate_limiter.acquire().await;

        // Next request should be delayed
        let start = Instant::now();
        rate_limiter.acquire().await;
        let elapsed = start.elapsed();

        // Should wait approximately 0.5 seconds (1/rate)
        assert!(
            elapsed >= Duration::from_millis(400),
            "Expected delay >= 400ms, got {:?}",
            elapsed
        );
    }

    #[tokio::test]
    async fn test_rate_limiter_refills_over_time() {
        let rate_limiter = TokenBucketRateLimiter::new(10.0);

        // Consume all tokens
        for _ in 0..10 {
            rate_limiter.acquire().await;
        }

        // Verify depleted
        let tokens_before = rate_limiter.available_tokens().await;
        assert!(tokens_before < 1.0);

        // Wait for refill
        sleep(Duration::from_millis(500)).await;

        // Should have ~5 tokens after 0.5 seconds at 10 tokens/sec
        let tokens_after = rate_limiter.available_tokens().await;
        assert!(
            (4.0..=6.0).contains(&tokens_after),
            "Expected ~5 tokens, got {}",
            tokens_after
=======
    use std::time::Instant;

    #[tokio::test]
    async fn test_rate_limiter_basic() {
        let limiter = TokenBucketRateLimiter::new(10.0);

        // Should acquire immediately (starts with full capacity)
        let start = Instant::now();
        limiter.acquire().await.unwrap();
        let elapsed = start.elapsed();

        assert!(elapsed < Duration::from_millis(50), "Should acquire immediately");
    }

    #[tokio::test]
    async fn test_rate_limiter_refill() {
        let limiter = TokenBucketRateLimiter::new(2.0); // 2 requests/second

        // Consume all tokens
        limiter.acquire().await.unwrap();
        limiter.acquire().await.unwrap();

        // Wait for refill
        tokio::time::sleep(Duration::from_millis(500)).await; // 0.5s = 1 token

        // Should have ~1 token now
        let available = limiter.available_tokens().await;
        assert!((available - 1.0).abs() < 0.2, "Should have ~1 token after 0.5s");
    }

    #[tokio::test]
    async fn test_rate_limiter_burst() {
        let limiter = TokenBucketRateLimiter::new(5.0); // 5 requests/second (capacity)

        // Should be able to burst up to capacity
        for _ in 0..5 {
            let start = Instant::now();
            limiter.acquire().await.unwrap();
            let elapsed = start.elapsed();
            assert!(elapsed < Duration::from_millis(50), "Burst should be immediate");
        }
    }

    #[tokio::test]
    async fn test_rate_limiter_blocking() {
        let limiter = TokenBucketRateLimiter::new(2.0); // 2 requests/second

        // Consume all tokens
        limiter.acquire().await.unwrap();
        limiter.acquire().await.unwrap();

        // Next acquire should block for ~0.5 seconds
        let start = Instant::now();
        limiter.acquire().await.unwrap();
        let elapsed = start.elapsed();

        assert!(
            elapsed >= Duration::from_millis(400),
            "Should wait ~0.5s for next token"
        );
        assert!(
            elapsed < Duration::from_millis(700),
            "Should not wait too long"
>>>>>>> task_claude-api-integration-tests_20251025-210007
        );
    }

    #[tokio::test]
<<<<<<< HEAD
    async fn test_rate_limiter_respects_capacity() {
        let rate_limiter = TokenBucketRateLimiter::new(5.0);

        // Wait for potential overflow
        sleep(Duration::from_secs(2)).await;

        // Available tokens should not exceed capacity
        let tokens = rate_limiter.available_tokens().await;
        assert!(tokens <= 5.0, "Tokens ({}) exceeded capacity (5.0)", tokens);
    }

    #[tokio::test]
    async fn test_concurrent_acquire() {
        let rate_limiter = Arc::new(TokenBucketRateLimiter::new(10.0));
        let mut handles = vec![];

        // Spawn 20 concurrent requests
        for _ in 0..20 {
            let rl = rate_limiter.clone();
            handles.push(tokio::spawn(async move {
                rl.acquire().await;
            }));
        }

        // All should complete (some will wait)
        for handle in handles {
            handle.await.unwrap();
        }

        // Verify tokens were properly managed
        let tokens = rate_limiter.available_tokens().await;
        assert!(tokens >= 0.0);
=======
    async fn test_rate_limiter_concurrent() {
        let limiter = Arc::new(TokenBucketRateLimiter::new(10.0));

        let mut handles = vec![];
        for _ in 0..20 {
            let limiter_clone = Arc::clone(&limiter);
            let handle = tokio::spawn(async move {
                limiter_clone.acquire().await.unwrap();
            });
            handles.push(handle);
        }

        // All should complete within reasonable time
        let start = Instant::now();
        for handle in handles {
            handle.await.unwrap();
        }
        let elapsed = start.elapsed();

        // 20 requests at 10/sec should take ~1 second (first 10 immediate, next 10 wait)
        assert!(
            elapsed >= Duration::from_millis(800),
            "Should enforce rate limit"
        );
        assert!(
            elapsed < Duration::from_millis(1500),
            "Should not wait too long"
        );
>>>>>>> task_claude-api-integration-tests_20251025-210007
    }
}
