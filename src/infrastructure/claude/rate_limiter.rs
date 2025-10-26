/// Token bucket rate limiter for Claude API requests
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;
use tokio::time::sleep;

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

    /// Last time tokens were refilled
    last_refill: Arc<Mutex<Instant>>,
}

impl TokenBucketRateLimiter {
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
            last_refill: Arc::new(Mutex::new(Instant::now())),
        }
    }

    /// Acquire a token, waiting if necessary
    ///
    /// This method blocks until at least one token is available, then consumes it.
    /// Tokens are refilled continuously based on elapsed time since last refill.
    ///
    /// # Errors
    /// Returns an error if an internal lock is poisoned (should never happen in practice)
    pub async fn acquire(&self) -> Result<(), String> {
        loop {
            let mut tokens = self.tokens.lock().await;
            let mut last_refill = self.last_refill.lock().await;

            // Refill tokens based on elapsed time
            let now = Instant::now();
            let elapsed = now.duration_since(*last_refill).as_secs_f64();
            let new_tokens = (*tokens + elapsed * self.refill_rate).min(self.capacity);

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

            // Release locks before sleeping
            drop(tokens);
            drop(last_refill);

            // Wait before retrying
            sleep(wait_duration).await;
        }
    }

    /// Get current number of available tokens (for testing/monitoring)
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
        );
    }

    #[tokio::test]
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
    }
}
