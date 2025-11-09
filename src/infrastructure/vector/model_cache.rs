//! HuggingFace model cache manager with retry logic and exponential backoff
//!
//! Provides robust model file downloads from HuggingFace Hub with:
//! - Automatic retry with exponential backoff (3 retries: 1s, 2s, 4s)
//! - Caching to ~/.cache/huggingface/hub/
//! - Safetensor file validation
//! - Async I/O with tokio
//!
//! # Examples
//!
//! ```no_run
//! use abathur_cli::infrastructure::vector::model_cache::ModelCache;
//!
//! # async fn example() -> anyhow::Result<()> {
//! let cache = ModelCache::new()?;
//!
//! // Download model with automatic retry
//! let model_path = cache.download_model(
//!     "sentence-transformers/all-MiniLM-L6-v2",
//!     &["model.safetensors", "tokenizer.json", "config.json"]
//! ).await?;
//!
//! println!("Model cached at: {:?}", model_path);
//! # Ok(())
//! # }
//! ```

use crate::infrastructure::vector::bert_error::{BertError, BertResult};
use std::future::Future;
use std::path::{Path, PathBuf};
use tokio::time::{sleep, Duration};
use tracing::{info, warn};

/// Retry policy for model downloads with exponential backoff
///
/// Default configuration:
/// - Max retries: 3 attempts
/// - Initial backoff: 1 second
/// - Max backoff: 8 seconds
/// - Backoff multiplier: 2.0 (exponential)
///
/// Backoff sequence: 1s → 2s → 4s → 8s
///
/// # Examples
///
/// ```
/// use abathur_cli::infrastructure::vector::model_cache::RetryPolicy;
///
/// let policy = RetryPolicy::default();
/// assert_eq!(policy.max_retries(), 3);
/// ```
#[derive(Debug, Clone)]
pub struct RetryPolicy {
    /// Maximum number of retry attempts
    max_retries: u32,
    /// Initial backoff duration in milliseconds
    initial_backoff_ms: u64,
    /// Maximum backoff duration in milliseconds
    max_backoff_ms: u64,
    /// Backoff multiplier for exponential growth
    backoff_multiplier: f64,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_retries: 3,
            initial_backoff_ms: 1000,  // 1 second
            max_backoff_ms: 8000,       // 8 seconds max
            backoff_multiplier: 2.0,     // Double each time
        }
    }
}

impl RetryPolicy {
    /// Create a custom retry policy
    ///
    /// # Arguments
    /// * `max_retries` - Maximum number of retry attempts
    /// * `initial_backoff_ms` - Initial backoff in milliseconds
    /// * `max_backoff_ms` - Maximum backoff in milliseconds
    /// * `backoff_multiplier` - Exponential growth factor
    ///
    /// # Examples
    ///
    /// ```
    /// use abathur_cli::infrastructure::vector::model_cache::RetryPolicy;
    ///
    /// // More aggressive retry: 5 attempts with faster backoff
    /// let policy = RetryPolicy::new(5, 500, 10000, 2.0);
    /// assert_eq!(policy.max_retries(), 5);
    /// ```
    pub fn new(
        max_retries: u32,
        initial_backoff_ms: u64,
        max_backoff_ms: u64,
        backoff_multiplier: f64,
    ) -> Self {
        Self {
            max_retries,
            initial_backoff_ms,
            max_backoff_ms,
            backoff_multiplier,
        }
    }

    /// Get maximum retries
    pub fn max_retries(&self) -> u32 {
        self.max_retries
    }

    /// Execute an async operation with retry logic and exponential backoff
    ///
    /// # Type Parameters
    /// * `F` - Closure that produces a future
    /// * `Fut` - The future type returned by F
    /// * `T` - Success type
    ///
    /// # Arguments
    /// * `operation` - Async operation to execute with retries
    ///
    /// # Returns
    /// * `Ok(T)` - If operation succeeds within max retries
    /// * `Err(BertError)` - If all retries exhausted or permanent error
    ///
    /// # Retry Logic
    /// - Retries on transient errors: network, timeout, connection, 429, 502, 503
    /// - Fails fast on permanent errors: 404, 401, 403, validation errors
    /// - Exponential backoff with jitter to prevent thundering herd
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use abathur_cli::infrastructure::vector::model_cache::RetryPolicy;
    /// use abathur_cli::infrastructure::vector::bert_error::BertResult;
    ///
    /// # async fn example() -> BertResult<()> {
    /// let policy = RetryPolicy::default();
    ///
    /// let result = policy.execute(|| async {
    ///     // Your async operation here
    ///     Ok::<_, crate::infrastructure::vector::bert_error::BertError>("success")
    /// }).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn execute<F, Fut, T>(&self, mut operation: F) -> BertResult<T>
    where
        F: FnMut() -> Fut,
        Fut: Future<Output = BertResult<T>>,
    {
        let mut attempt = 0;

        loop {
            match operation().await {
                Ok(result) => return Ok(result),
                Err(err) => {
                    // Check if error is permanent (don't retry)
                    if err.is_permanent() {
                        warn!(
                            error = %err,
                            "Permanent error encountered, not retrying"
                        );
                        return Err(err);
                    }

                    // Check if we should retry
                    if attempt >= self.max_retries {
                        warn!(
                            attempt = attempt,
                            max_retries = self.max_retries,
                            error = %err,
                            "Max retries exhausted"
                        );
                        return Err(err);
                    }

                    // Check if error is transient
                    if !self.should_retry(&err) {
                        warn!(
                            error = %err,
                            "Non-transient error, not retrying"
                        );
                        return Err(err);
                    }

                    // Calculate backoff
                    let backoff = self.calculate_backoff(attempt);

                    warn!(
                        attempt = attempt + 1,
                        max_retries = self.max_retries,
                        backoff_ms = backoff.as_millis(),
                        error = %err,
                        "Retrying after error"
                    );

                    sleep(backoff).await;
                    attempt += 1;
                }
            }
        }
    }

    /// Calculate exponential backoff duration for a given attempt
    ///
    /// Formula: initial_backoff * multiplier^attempt, capped at max_backoff
    ///
    /// # Arguments
    /// * `attempt` - Current attempt number (0-indexed)
    ///
    /// # Returns
    /// Backoff duration with exponential growth
    ///
    /// # Examples
    ///
    /// ```
    /// use abathur_cli::infrastructure::vector::model_cache::RetryPolicy;
    /// use tokio::time::Duration;
    ///
    /// let policy = RetryPolicy::default();
    /// assert_eq!(policy.calculate_backoff(0), Duration::from_millis(1000));
    /// assert_eq!(policy.calculate_backoff(1), Duration::from_millis(2000));
    /// assert_eq!(policy.calculate_backoff(2), Duration::from_millis(4000));
    /// assert_eq!(policy.calculate_backoff(3), Duration::from_millis(8000));
    /// ```
    pub fn calculate_backoff(&self, attempt: u32) -> Duration {
        let backoff_ms = (self.initial_backoff_ms as f64
            * self.backoff_multiplier.powi(attempt as i32))
            .min(self.max_backoff_ms as f64) as u64;

        Duration::from_millis(backoff_ms)
    }

    /// Determine if an error should trigger a retry
    ///
    /// Transient errors (retry):
    /// - Network connectivity issues
    /// - Timeouts
    /// - HTTP 429 (rate limit)
    /// - HTTP 502 (bad gateway)
    /// - HTTP 503 (service unavailable)
    ///
    /// Permanent errors (don't retry):
    /// - HTTP 404 (not found)
    /// - HTTP 401 (unauthorized)
    /// - HTTP 403 (forbidden)
    /// - Validation errors
    /// - Configuration errors
    ///
    /// # Arguments
    /// * `error` - The error to classify
    ///
    /// # Returns
    /// * `true` - Error is transient, should retry
    /// * `false` - Error is permanent, don't retry
    fn should_retry(&self, error: &BertError) -> bool {
        let err_str = error.to_string().to_lowercase();

        // Retry on transient network errors
        err_str.contains("timeout")
            || err_str.contains("connection")
            || err_str.contains("network")
            || err_str.contains("503") // Service unavailable
            || err_str.contains("502") // Bad gateway
            || err_str.contains("429") // Too many requests
    }
}

/// Paths to downloaded model files
///
/// Contains paths to all required files for BERT model inference.
///
/// # Examples
///
/// ```no_run
/// use abathur_cli::infrastructure::vector::model_cache::ModelPaths;
/// use std::path::PathBuf;
///
/// let paths = ModelPaths {
///     model: PathBuf::from("/cache/model.safetensors"),
///     tokenizer: PathBuf::from("/cache/tokenizer.json"),
///     config: PathBuf::from("/cache/config.json"),
/// };
///
/// assert!(paths.validate().is_ok());
/// ```
#[derive(Debug, Clone)]
pub struct ModelPaths {
    /// Path to model weights (model.safetensors or pytorch_model.bin)
    pub model: PathBuf,
    /// Path to tokenizer configuration (tokenizer.json)
    pub tokenizer: PathBuf,
    /// Path to model configuration (config.json)
    pub config: PathBuf,
}

impl ModelPaths {
    /// Validate that all model files exist
    ///
    /// # Returns
    /// * `Ok(())` - All files exist
    /// * `Err(BertError)` - One or more files missing
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use abathur_cli::infrastructure::vector::model_cache::ModelPaths;
    /// use std::path::PathBuf;
    ///
    /// # fn example() -> anyhow::Result<()> {
    /// let paths = ModelPaths {
    ///     model: PathBuf::from("/cache/model.safetensors"),
    ///     tokenizer: PathBuf::from("/cache/tokenizer.json"),
    ///     config: PathBuf::from("/cache/config.json"),
    /// };
    ///
    /// paths.validate()?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn validate(&self) -> BertResult<()> {
        if !self.model.exists() {
            return Err(BertError::ModelValidationError {
                model_name: self.model.display().to_string(),
                reason: "Model file not found".to_string(),
            });
        }

        if !self.tokenizer.exists() {
            return Err(BertError::ModelValidationError {
                model_name: self.tokenizer.display().to_string(),
                reason: "Tokenizer file not found".to_string(),
            });
        }

        if !self.config.exists() {
            return Err(BertError::ModelValidationError {
                model_name: self.config.display().to_string(),
                reason: "Config file not found".to_string(),
            });
        }

        // Validate file sizes (sanity check for corruption)
        let model_size = std::fs::metadata(&self.model)
            .map_err(BertError::IoError)?
            .len();

        if model_size < 1024 {
            return Err(BertError::ModelValidationError {
                model_name: self.model.display().to_string(),
                reason: format!("Model file too small: {} bytes", model_size),
            });
        }

        Ok(())
    }
}

/// HuggingFace model cache manager
///
/// Manages model downloads from HuggingFace Hub with automatic retry,
/// exponential backoff, and local caching.
///
/// # Features
/// - Downloads models from HuggingFace Hub
/// - Caches to ~/.cache/huggingface/hub/
/// - Automatic retry with exponential backoff (3 retries: 1s, 2s, 4s)
/// - Safetensor file validation
/// - Async I/O with tokio
///
/// # Examples
///
/// ```no_run
/// use abathur_cli::infrastructure::vector::model_cache::ModelCache;
///
/// # async fn example() -> anyhow::Result<()> {
/// let cache = ModelCache::new()?;
///
/// // Check cache first
/// if let Some(path) = cache.get_cached_path("sentence-transformers/all-MiniLM-L6-v2")? {
///     println!("Model already cached at: {:?}", path);
/// } else {
///     // Download with retry
///     let path = cache.download_model(
///         "sentence-transformers/all-MiniLM-L6-v2",
///         &["model.safetensors", "tokenizer.json", "config.json"]
///     ).await?;
///     println!("Model downloaded to: {:?}", path);
/// }
/// # Ok(())
/// # }
/// ```
pub struct ModelCache {
    /// HuggingFace API client
    api: hf_hub::api::tokio::Api,
    /// Cache directory (typically ~/.cache/huggingface/hub)
    cache_dir: PathBuf,
    /// Retry policy for downloads
    retry_policy: RetryPolicy,
}

impl ModelCache {
    /// Create a new model cache with default configuration
    ///
    /// Uses default cache directory: ~/.cache/huggingface/hub
    /// Can be overridden with HF_HOME or HF_HUB_CACHE environment variables.
    ///
    /// # Returns
    /// * `Ok(Self)` - Cache manager ready for use
    /// * `Err(BertError)` - If cache initialization fails
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use abathur_cli::infrastructure::vector::model_cache::ModelCache;
    ///
    /// # fn example() -> anyhow::Result<()> {
    /// let cache = ModelCache::new()?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn new() -> BertResult<Self> {
        // Get default cache directory
        let cache_dir = Self::default_cache_dir()
            .ok_or_else(|| BertError::ConfigError(
                "Could not determine cache directory".to_string()
            ))?;

        Self::with_cache_dir(cache_dir)
    }

    /// Create a model cache with custom cache directory
    ///
    /// # Arguments
    /// * `cache_dir` - Custom cache directory path
    ///
    /// # Returns
    /// * `Ok(Self)` - Cache manager with custom directory
    /// * `Err(BertError)` - If initialization fails
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use abathur_cli::infrastructure::vector::model_cache::ModelCache;
    /// use std::path::PathBuf;
    ///
    /// # fn example() -> anyhow::Result<()> {
    /// let cache = ModelCache::with_cache_dir(PathBuf::from("/tmp/models"))?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn with_cache_dir(cache_dir: PathBuf) -> BertResult<Self> {
        info!("Initializing ModelCache with directory: {:?}", cache_dir);

        // Create API with custom cache directory
        let api = hf_hub::api::tokio::ApiBuilder::new()
            .with_cache_dir(cache_dir.clone())
            .with_progress(true)
            .build()
            .map_err(|e| BertError::model_load_error(
                "HuggingFace API",
                format!("Failed to initialize API: {}", e)
            ))?;

        Ok(Self {
            api,
            cache_dir,
            retry_policy: RetryPolicy::default(),
        })
    }

    /// Create a model cache with custom retry policy
    ///
    /// # Arguments
    /// * `cache_dir` - Cache directory path
    /// * `retry_policy` - Custom retry policy
    ///
    /// # Returns
    /// * `Ok(Self)` - Cache manager with custom retry policy
    /// * `Err(BertError)` - If initialization fails
    pub fn with_retry_policy(cache_dir: PathBuf, retry_policy: RetryPolicy) -> BertResult<Self> {
        let mut cache = Self::with_cache_dir(cache_dir)?;
        cache.retry_policy = retry_policy;
        Ok(cache)
    }

    /// Get the default HuggingFace cache directory
    ///
    /// Priority:
    /// 1. HF_HUB_CACHE environment variable
    /// 2. HF_HOME/hub environment variable
    /// 3. ~/.cache/huggingface/hub (default)
    ///
    /// # Returns
    /// * `Some(PathBuf)` - Default cache directory
    /// * `None` - If home directory cannot be determined
    fn default_cache_dir() -> Option<PathBuf> {
        // Check HF_HUB_CACHE environment variable
        if let Ok(path) = std::env::var("HF_HUB_CACHE") {
            return Some(PathBuf::from(path));
        }

        // Check HF_HOME environment variable
        if let Ok(hf_home) = std::env::var("HF_HOME") {
            return Some(PathBuf::from(hf_home).join("hub"));
        }

        // Default: ~/.cache/huggingface/hub
        dirs::cache_dir().map(|cache| cache.join("huggingface").join("hub"))
    }

    /// Download model files from HuggingFace Hub with retry logic
    ///
    /// Downloads specified files and returns the snapshot directory path.
    /// Automatically retries on transient errors with exponential backoff.
    ///
    /// # Arguments
    /// * `repo_id` - HuggingFace repository ID (e.g., "sentence-transformers/all-MiniLM-L6-v2")
    /// * `model_files` - List of files to download (e.g., ["model.safetensors", "tokenizer.json"])
    ///
    /// # Returns
    /// * `Ok(PathBuf)` - Path to snapshot directory containing all files
    /// * `Err(BertError)` - If download fails after all retries
    ///
    /// # Retry Logic
    /// - 3 retry attempts with backoff: 1s, 2s, 4s
    /// - Retries on: network errors, timeouts, 429, 502, 503
    /// - Fails fast on: 404, 401, 403
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use abathur_cli::infrastructure::vector::model_cache::ModelCache;
    ///
    /// # async fn example() -> anyhow::Result<()> {
    /// let cache = ModelCache::new()?;
    ///
    /// let path = cache.download_model(
    ///     "sentence-transformers/all-MiniLM-L6-v2",
    ///     &["model.safetensors", "tokenizer.json", "config.json"]
    /// ).await?;
    ///
    /// println!("Downloaded to: {:?}", path);
    /// # Ok(())
    /// # }
    /// ```
    pub async fn download_model(
        &self,
        repo_id: &str,
        model_files: &[&str],
    ) -> BertResult<PathBuf> {
        info!(
            repo_id = repo_id,
            files = ?model_files,
            "Starting model download with retry logic"
        );

        // Check cache first to avoid unnecessary downloads
        if let Some(cached_path) = self.get_cached_path(repo_id)? {
            info!(
                repo_id = repo_id,
                path = ?cached_path,
                "Model found in cache, skipping download"
            );
            return Ok(cached_path);
        }

        // Download each file with retry logic
        for file_name in model_files {
            info!(
                repo_id = repo_id,
                file = file_name,
                "Downloading file"
            );

            // Create owned copies for move into closure
            let file_name_owned = file_name.to_string();
            let repo_id_owned = repo_id.to_string();

            // Download with retry - recreate repo handle each time
            let api_ref = &self.api;
            self.retry_policy
                .execute(|| {
                    let file = file_name_owned.clone();
                    let repo_id = repo_id_owned.clone();
                    async move {
                        // Create repo handle inside retry closure
                        let repo = api_ref.model(repo_id.clone());
                        repo.get(&file)
                            .await
                            .map_err(|e| BertError::model_load_error(
                                &repo_id,
                                format!("Failed to download {}: {}", file, e)
                            ))
                    }
                })
                .await?;

            info!(
                repo_id = repo_id,
                file = file_name,
                "File downloaded successfully"
            );
        }

        // Get the snapshot directory path
        let snapshot_path = self.get_cached_path(repo_id)?
            .ok_or_else(|| BertError::model_load_error(
                repo_id,
                "Model not found in cache after download"
            ))?;

        // Verify safetensor files if present
        for file_name in model_files {
            if file_name.ends_with(".safetensors") {
                let file_path = snapshot_path.join(file_name);
                self.verify_safetensors(&file_path)?;
            }
        }

        info!(
            repo_id = repo_id,
            path = ?snapshot_path,
            "Model download completed successfully"
        );

        Ok(snapshot_path)
    }

    /// Check if model is already cached locally
    ///
    /// Queries the HuggingFace cache directory for the specified model.
    ///
    /// # Arguments
    /// * `repo_id` - HuggingFace repository ID
    ///
    /// # Returns
    /// * `Ok(Some(PathBuf))` - Path to cached snapshot directory
    /// * `Ok(None)` - Model not found in cache
    /// * `Err(BertError)` - If cache query fails
    ///
    /// # Cache Layout
    /// ```text
    /// ~/.cache/huggingface/hub/
    /// └── models--sentence-transformers--all-MiniLM-L6-v2/
    ///     └── snapshots/
    ///         └── {revision}/
    ///             ├── model.safetensors
    ///             ├── tokenizer.json
    ///             └── config.json
    /// ```
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use abathur_cli::infrastructure::vector::model_cache::ModelCache;
    ///
    /// # fn example() -> anyhow::Result<()> {
    /// let cache = ModelCache::new()?;
    ///
    /// if let Some(path) = cache.get_cached_path("sentence-transformers/all-MiniLM-L6-v2")? {
    ///     println!("Model cached at: {:?}", path);
    /// } else {
    ///     println!("Model not in cache");
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub fn get_cached_path(&self, repo_id: &str) -> BertResult<Option<PathBuf>> {
        // Convert repo_id to cache directory name
        // "sentence-transformers/all-MiniLM-L6-v2" -> "models--sentence-transformers--all-MiniLM-L6-v2"
        let cache_name = format!("models--{}", repo_id.replace('/', "--"));
        let model_cache_path = self.cache_dir.join(&cache_name);

        if !model_cache_path.exists() {
            return Ok(None);
        }

        // Find latest snapshot
        let snapshots_dir = model_cache_path.join("snapshots");
        if !snapshots_dir.exists() {
            return Ok(None);
        }

        // Get the most recently modified snapshot directory
        let mut snapshot_dirs: Vec<_> = std::fs::read_dir(&snapshots_dir)
            .map_err(|e| BertError::IoError(e))?
            .filter_map(|entry| entry.ok())
            .filter(|entry| entry.path().is_dir())
            .collect();

        if snapshot_dirs.is_empty() {
            return Ok(None);
        }

        // Sort by modification time (most recent first)
        snapshot_dirs.sort_by_key(|entry| {
            entry.metadata()
                .and_then(|m| m.modified())
                .unwrap_or(std::time::SystemTime::UNIX_EPOCH)
        });
        snapshot_dirs.reverse();

        // Return the most recent snapshot
        Ok(Some(snapshot_dirs[0].path()))
    }

    /// Verify safetensor file integrity
    ///
    /// Validates that a safetensor file is not corrupted by checking:
    /// - File exists and is readable
    /// - File size is reasonable (> 1KB)
    /// - File header is valid safetensors format
    ///
    /// # Arguments
    /// * `path` - Path to safetensor file
    ///
    /// # Returns
    /// * `Ok(())` - File is valid
    /// * `Err(BertError)` - File is corrupted or invalid
    ///
    /// # Safetensors Format
    /// Safetensors files start with an 8-byte header containing the metadata size,
    /// followed by JSON metadata, then the tensor data.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use abathur_cli::infrastructure::vector::model_cache::ModelCache;
    /// use std::path::PathBuf;
    ///
    /// # fn example() -> anyhow::Result<()> {
    /// let cache = ModelCache::new()?;
    /// let path = PathBuf::from("/cache/model.safetensors");
    ///
    /// cache.verify_safetensors(&path)?;
    /// println!("Safetensor file is valid");
    /// # Ok(())
    /// # }
    /// ```
    pub fn verify_safetensors(&self, path: &Path) -> BertResult<()> {
        if !path.exists() {
            return Err(BertError::ModelValidationError {
                model_name: path.display().to_string(),
                reason: "Safetensor file not found".to_string(),
            });
        }

        // Check file size (must be > 1KB)
        let metadata = std::fs::metadata(path)
            .map_err(|e| BertError::IoError(e))?;

        let file_size = metadata.len();
        if file_size < 1024 {
            return Err(BertError::ModelValidationError {
                model_name: path.display().to_string(),
                reason: format!("Safetensor file too small: {} bytes", file_size),
            });
        }

        // Read first 8 bytes (header size)
        let file_data = std::fs::read(path)
            .map_err(|e| BertError::IoError(e))?;

        if file_data.len() < 8 {
            return Err(BertError::ModelValidationError {
                model_name: path.display().to_string(),
                reason: "File too small to be valid safetensors".to_string(),
            });
        }

        // Parse header size (little-endian u64)
        let header_size = u64::from_le_bytes([
            file_data[0], file_data[1], file_data[2], file_data[3],
            file_data[4], file_data[5], file_data[6], file_data[7],
        ]);

        // Validate header size is reasonable
        if header_size == 0 || header_size > file_size {
            return Err(BertError::ModelValidationError {
                model_name: path.display().to_string(),
                reason: format!(
                    "Invalid header size: {} (file size: {})",
                    header_size, file_size
                ),
            });
        }

        // Validate metadata JSON (starts at byte 8, length = header_size)
        if file_data.len() < 8 + header_size as usize {
            return Err(BertError::ModelValidationError {
                model_name: path.display().to_string(),
                reason: "File truncated, metadata incomplete".to_string(),
            });
        }

        let metadata_bytes = &file_data[8..8 + header_size as usize];
        let _metadata: serde_json::Value = serde_json::from_slice(metadata_bytes)
            .map_err(|e| BertError::ModelValidationError {
                model_name: path.display().to_string(),
                reason: format!("Invalid metadata JSON: {}", e),
            })?;

        info!(
            path = ?path,
            file_size = file_size,
            header_size = header_size,
            "Safetensor file validated successfully"
        );

        Ok(())
    }

    /// Get the cache directory path
    pub fn cache_dir(&self) -> &Path {
        &self.cache_dir
    }

    /// Get the retry policy
    pub fn retry_policy(&self) -> &RetryPolicy {
        &self.retry_policy
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_retry_policy_default() {
        let policy = RetryPolicy::default();
        assert_eq!(policy.max_retries(), 3);
    }

    #[test]
    fn test_retry_policy_calculate_backoff() {
        let policy = RetryPolicy::default();

        assert_eq!(policy.calculate_backoff(0), Duration::from_millis(1000));
        assert_eq!(policy.calculate_backoff(1), Duration::from_millis(2000));
        assert_eq!(policy.calculate_backoff(2), Duration::from_millis(4000));
        assert_eq!(policy.calculate_backoff(3), Duration::from_millis(8000));

        // Should cap at max_backoff_ms
        assert_eq!(policy.calculate_backoff(10), Duration::from_millis(8000));
    }

    #[test]
    fn test_retry_policy_custom() {
        let policy = RetryPolicy::new(5, 500, 10000, 2.0);
        assert_eq!(policy.max_retries(), 5);
        assert_eq!(policy.calculate_backoff(0), Duration::from_millis(500));
        assert_eq!(policy.calculate_backoff(1), Duration::from_millis(1000));
    }

    #[test]
    fn test_retry_policy_should_retry_transient() {
        let policy = RetryPolicy::default();

        let timeout_err = BertError::model_load_error("test", "Connection timeout");
        assert!(policy.should_retry(&timeout_err));

        let network_err = BertError::model_load_error("test", "Network unreachable");
        assert!(policy.should_retry(&network_err));

        let rate_limit_err = BertError::model_load_error("test", "HTTP 429 Too Many Requests");
        assert!(policy.should_retry(&rate_limit_err));
    }

    #[test]
    fn test_retry_policy_should_not_retry_permanent() {
        let policy = RetryPolicy::default();

        let not_found_err = BertError::model_load_error("test", "HTTP 404 Not Found");
        assert!(!policy.should_retry(&not_found_err));

        let unauthorized_err = BertError::model_load_error("test", "HTTP 401 Unauthorized");
        assert!(!policy.should_retry(&unauthorized_err));

        let validation_err = BertError::ModelValidationError {
            model_name: "test".to_string(),
            reason: "Invalid".to_string(),
        };
        assert!(!policy.should_retry(&validation_err));
    }

    #[test]
    fn test_model_paths_validate_success() {
        use tempfile::tempdir;

        let temp = tempdir().unwrap();
        let model_path = temp.path().join("model.safetensors");
        let tokenizer_path = temp.path().join("tokenizer.json");
        let config_path = temp.path().join("config.json");

        // Create files with sufficient size
        std::fs::write(&model_path, vec![0u8; 2048]).unwrap();
        std::fs::write(&tokenizer_path, "{}").unwrap();
        std::fs::write(&config_path, "{}").unwrap();

        let paths = ModelPaths {
            model: model_path,
            tokenizer: tokenizer_path,
            config: config_path,
        };

        assert!(paths.validate().is_ok());
    }

    #[test]
    fn test_model_paths_validate_missing_file() {
        let paths = ModelPaths {
            model: PathBuf::from("/nonexistent/model.safetensors"),
            tokenizer: PathBuf::from("/nonexistent/tokenizer.json"),
            config: PathBuf::from("/nonexistent/config.json"),
        };

        assert!(paths.validate().is_err());
    }

    #[test]
    fn test_model_paths_validate_file_too_small() {
        use tempfile::tempdir;

        let temp = tempdir().unwrap();
        let model_path = temp.path().join("model.safetensors");
        let tokenizer_path = temp.path().join("tokenizer.json");
        let config_path = temp.path().join("config.json");

        // Create model file that's too small
        std::fs::write(&model_path, vec![0u8; 100]).unwrap();
        std::fs::write(&tokenizer_path, "{}").unwrap();
        std::fs::write(&config_path, "{}").unwrap();

        let paths = ModelPaths {
            model: model_path,
            tokenizer: tokenizer_path,
            config: config_path,
        };

        let result = paths.validate();
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("too small"));
    }

    #[test]
    fn test_model_cache_default_cache_dir() {
        let cache_dir = ModelCache::default_cache_dir();
        assert!(cache_dir.is_some());

        let path = cache_dir.unwrap();
        assert!(path.to_str().unwrap().contains("huggingface"));
        assert!(path.to_str().unwrap().contains("hub"));
    }

    #[test]
    fn test_model_cache_new() {
        let cache = ModelCache::new();
        assert!(cache.is_ok());
    }

    #[test]
    fn test_model_cache_with_custom_dir() {
        use tempfile::tempdir;

        let temp = tempdir().unwrap();
        let cache = ModelCache::with_cache_dir(temp.path().to_path_buf());

        assert!(cache.is_ok());
        let cache = cache.unwrap();
        assert_eq!(cache.cache_dir(), temp.path());
    }

    #[test]
    fn test_verify_safetensors_missing_file() {
        let cache = ModelCache::new().unwrap();
        let result = cache.verify_safetensors(Path::new("/nonexistent/model.safetensors"));

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not found"));
    }

    #[test]
    fn test_verify_safetensors_file_too_small() {
        use tempfile::tempdir;

        let temp = tempdir().unwrap();
        let safetensor_path = temp.path().join("model.safetensors");

        // Create file that's too small
        std::fs::write(&safetensor_path, vec![0u8; 100]).unwrap();

        let cache = ModelCache::new().unwrap();
        let result = cache.verify_safetensors(&safetensor_path);

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("too small"));
    }

    #[test]
    fn test_get_cached_path_nonexistent() {
        let cache = ModelCache::new().unwrap();
        let result = cache.get_cached_path("nonexistent/model");

        assert!(result.is_ok());
        assert!(result.unwrap().is_none());
    }

    #[tokio::test]
    async fn test_retry_policy_execute_success() {
        let policy = RetryPolicy::default();

        let result = policy.execute(|| async {
            Ok::<_, BertError>(42)
        }).await;

        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 42);
    }

    #[tokio::test]
    async fn test_retry_policy_execute_permanent_error() {
        let policy = RetryPolicy::default();

        let result = policy.execute(|| async {
            Err::<i32, _>(BertError::ModelValidationError {
                model_name: "test".to_string(),
                reason: "Invalid".to_string(),
            })
        }).await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_retry_policy_execute_retry_then_success() {
        let policy = RetryPolicy::default();
        let attempts = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));

        let attempts_clone = attempts.clone();
        let result = policy.execute(move || {
            let attempts = attempts_clone.clone();
            async move {
                let count = attempts.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                if count < 2 {
                    Err(BertError::model_load_error("test", "Network timeout"))
                } else {
                    Ok(42)
                }
            }
        }).await;

        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 42);
        assert_eq!(attempts.load(std::sync::atomic::Ordering::SeqCst), 3);
    }
}
