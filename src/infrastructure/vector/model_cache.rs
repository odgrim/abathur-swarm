//! Model cache management for embedding models
//!
//! Handles model downloads, caching, and verification.
//! Models are cached in ~/.cache/abathur/models/ or a custom directory.

use crate::domain::models::EmbeddingModel;
use anyhow::{anyhow, Result};
use std::path::{Path, PathBuf};

/// Model cache manager
///
/// Manages local caching of embedding models downloaded from HuggingFace.
pub struct ModelCache {
    cache_dir: PathBuf,
}

impl ModelCache {
    /// Create a new model cache with default directory
    ///
    /// Default directory is ~/.cache/abathur/models/
    pub fn new() -> Self {
        let cache_dir = dirs::cache_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("abathur")
            .join("models");

        Self::with_dir(cache_dir)
    }

    /// Create a new model cache with custom directory
    pub fn with_dir(cache_dir: PathBuf) -> Self {
        // Create directory if it doesn't exist
        if !cache_dir.exists() {
            if let Err(e) = std::fs::create_dir_all(&cache_dir) {
                tracing::warn!("Failed to create cache directory: {}", e);
            }
        }

        Self { cache_dir }
    }

    /// Check if a model is cached locally
    ///
    /// # Arguments
    /// * `model` - The embedding model to check
    ///
    /// # Returns
    /// * `true` if the model is cached
    /// * `false` if the model needs to be downloaded
    pub fn is_model_cached(&self, model: &EmbeddingModel) -> bool {
        if !model.is_local() {
            // Cloud models don't need caching
            return true;
        }

        // Check if model directory exists
        // Note: rust-bert caches models in ~/.cache/huggingface/ or ~/.cache/torch/
        // We're just tracking whether we've downloaded it before
        let model_marker = self.cache_dir.join(format!("{}.marker", self.model_key(model)));
        model_marker.exists()
    }

    /// Get the cache directory path
    pub fn cache_dir(&self) -> &Path {
        &self.cache_dir
    }

    /// Mark a model as cached (creates a marker file)
    ///
    /// # Arguments
    /// * `model` - The embedding model to mark as cached
    pub fn mark_cached(&self, model: &EmbeddingModel) -> Result<()> {
        if !model.is_local() {
            return Ok(());
        }

        let model_marker = self.cache_dir.join(format!("{}.marker", self.model_key(model)));

        std::fs::write(&model_marker, model.model_name())
            .map_err(|e| anyhow!("Failed to create cache marker: {}", e))?;

        Ok(())
    }

    /// Remove cache marker for a model
    ///
    /// # Arguments
    /// * `model` - The embedding model to uncache
    pub fn clear_cache(&self, model: &EmbeddingModel) -> Result<()> {
        if !model.is_local() {
            return Ok(());
        }

        let model_marker = self.cache_dir.join(format!("{}.marker", self.model_key(model)));

        if model_marker.exists() {
            std::fs::remove_file(&model_marker)
                .map_err(|e| anyhow!("Failed to remove cache marker: {}", e))?;
        }

        Ok(())
    }

    /// Get cache statistics
    pub fn stats(&self) -> CacheStats {
        let mut stats = CacheStats {
            cache_dir: self.cache_dir.clone(),
            total_size: 0,
            models_cached: Vec::new(),
        };

        // Check which models are cached
        for model in &[EmbeddingModel::LocalMiniLM, EmbeddingModel::LocalMPNet] {
            if self.is_model_cached(model) {
                stats.models_cached.push(*model);
            }
        }

        // Try to get directory size
        if let Ok(metadata) = std::fs::metadata(&self.cache_dir) {
            if metadata.is_dir() {
                stats.total_size = Self::dir_size(&self.cache_dir).unwrap_or(0);
            }
        }

        stats
    }

    /// Get a unique key for a model
    fn model_key(&self, model: &EmbeddingModel) -> String {
        match model {
            EmbeddingModel::LocalMiniLM => "minilm-l6-v2".to_string(),
            EmbeddingModel::LocalMPNet => "mpnet-base-v2".to_string(),
            EmbeddingModel::OpenAIAda002 => "openai-ada-002".to_string(),
        }
    }

    /// Calculate directory size recursively
    fn dir_size(path: &Path) -> Result<u64> {
        let mut total = 0;

        if path.is_dir() {
            for entry in std::fs::read_dir(path)? {
                let entry = entry?;
                let path = entry.path();

                if path.is_dir() {
                    total += Self::dir_size(&path)?;
                } else {
                    total += entry.metadata()?.len();
                }
            }
        }

        Ok(total)
    }
}

impl Default for ModelCache {
    fn default() -> Self {
        Self::new()
    }
}

/// Cache statistics
#[derive(Debug, Clone)]
pub struct CacheStats {
    /// Cache directory path
    pub cache_dir: PathBuf,

    /// Total size of cached models in bytes
    pub total_size: u64,

    /// List of cached models
    pub models_cached: Vec<EmbeddingModel>,
}

impl CacheStats {
    /// Format total size as human-readable string
    pub fn total_size_human(&self) -> String {
        let size = self.total_size as f64;

        if size < 1024.0 {
            format!("{} B", size)
        } else if size < 1024.0 * 1024.0 {
            format!("{:.2} KB", size / 1024.0)
        } else if size < 1024.0 * 1024.0 * 1024.0 {
            format!("{:.2} MB", size / (1024.0 * 1024.0))
        } else {
            format!("{:.2} GB", size / (1024.0 * 1024.0 * 1024.0))
        }
    }

    /// Get the number of cached models
    pub fn model_count(&self) -> usize {
        self.models_cached.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_new_cache() {
        let cache = ModelCache::new();
        assert!(cache.cache_dir.ends_with("abathur/models"));
    }

    #[test]
    fn test_with_dir() {
        let temp = TempDir::new().unwrap();
        let cache = ModelCache::with_dir(temp.path().to_path_buf());
        assert_eq!(cache.cache_dir, temp.path());
    }

    #[test]
    fn test_is_model_cached() {
        let temp = TempDir::new().unwrap();
        let cache = ModelCache::with_dir(temp.path().to_path_buf());

        let model = EmbeddingModel::LocalMiniLM;
        assert!(!cache.is_model_cached(&model));

        // Mark as cached
        cache.mark_cached(&model).unwrap();
        assert!(cache.is_model_cached(&model));
    }

    #[test]
    fn test_clear_cache() {
        let temp = TempDir::new().unwrap();
        let cache = ModelCache::with_dir(temp.path().to_path_buf());

        let model = EmbeddingModel::LocalMiniLM;
        cache.mark_cached(&model).unwrap();
        assert!(cache.is_model_cached(&model));

        cache.clear_cache(&model).unwrap();
        assert!(!cache.is_model_cached(&model));
    }

    #[test]
    fn test_cache_stats() {
        let temp = TempDir::new().unwrap();
        let cache = ModelCache::with_dir(temp.path().to_path_buf());

        let model = EmbeddingModel::LocalMiniLM;
        cache.mark_cached(&model).unwrap();

        let stats = cache.stats();
        assert_eq!(stats.model_count(), 1);
        assert!(stats.models_cached.contains(&model));
    }

    #[test]
    fn test_cloud_model_always_cached() {
        let temp = TempDir::new().unwrap();
        let cache = ModelCache::with_dir(temp.path().to_path_buf());

        let model = EmbeddingModel::OpenAIAda002;
        assert!(cache.is_model_cached(&model));
    }
}
