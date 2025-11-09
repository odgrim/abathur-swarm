use thiserror::Error;

/// Errors that can occur during BERT embedding operations
///
/// This error type covers all aspects of BERT model usage:
/// - Model downloading and initialization
/// - Tokenization of input text
/// - Tensor operations and inference
/// - GPU/CPU device management
#[derive(Error, Debug)]
pub enum BertError {
    /// Model file download or loading failed
    ///
    /// This includes failures from:
    /// - HuggingFace Hub API errors
    /// - Network connectivity issues
    /// - File system I/O errors during download
    /// - Corrupted model files
    ///
    /// # Examples
    ///
    /// ```
    /// use abathur_cli::infrastructure::vector::bert_error::BertError;
    ///
    /// let error = BertError::ModelLoadError {
    ///     model_name: "sentence-transformers/all-MiniLM-L6-v2".to_string(),
    ///     reason: "Network timeout".to_string(),
    /// };
    /// assert!(error.to_string().contains("all-MiniLM-L6-v2"));
    /// ```
    #[error("Failed to load model '{model_name}': {reason}")]
    ModelLoadError {
        /// The name/identifier of the model that failed to load
        model_name: String,
        /// The specific reason for the failure
        reason: String,
    },

    /// Model file validation failed
    ///
    /// Occurs when downloaded model files are corrupted or invalid:
    /// - Safetensor checksum mismatch
    /// - Invalid model configuration
    /// - Missing required files (tokenizer.json, config.json)
    #[error("Model validation failed for '{model_name}': {reason}")]
    ModelValidationError {
        /// The name/identifier of the model
        model_name: String,
        /// Validation failure details
        reason: String,
    },

    /// HuggingFace Hub API error
    ///
    /// Wraps errors from the hf-hub crate during model downloads
    #[error("HuggingFace Hub API error: {0}")]
    HubApiError(#[from] hf_hub::api::sync::ApiError),

    /// File system I/O error during model operations
    ///
    /// Includes errors during:
    /// - Reading model files from cache
    /// - Writing downloaded files
    /// - Creating cache directories
    #[error("File I/O error: {0}")]
    IoError(#[from] std::io::Error),

    /// Tokenization of input text failed
    ///
    /// Occurs when:
    /// - Input text contains invalid characters
    /// - Input exceeds maximum sequence length (usually 512 tokens)
    /// - Tokenizer configuration is invalid
    ///
    /// # Examples
    ///
    /// ```
    /// use abathur_cli::infrastructure::vector::bert_error::BertError;
    ///
    /// let error = BertError::TokenizationError {
    ///     text_preview: "Invalid text...".to_string(),
    ///     reason: "Sequence length exceeds 512 tokens".to_string(),
    /// };
    /// assert!(!error.is_transient());
    /// ```
    #[error("Tokenization failed for text '{text_preview}': {reason}")]
    TokenizationError {
        /// Preview of the problematic text (truncated to 50 chars)
        text_preview: String,
        /// The specific tokenization error
        reason: String,
    },

    /// BERT model inference (forward pass) failed
    ///
    /// Errors during the neural network forward pass:
    /// - Tensor shape mismatches
    /// - Numerical instability (NaN/Inf)
    /// - GPU memory exhausted during computation
    /// - Model architecture incompatibility
    ///
    /// # Examples
    ///
    /// ```
    /// use abathur_cli::infrastructure::vector::bert_error::BertError;
    ///
    /// let error = BertError::InferenceError {
    ///     operation: "forward pass".to_string(),
    ///     reason: "Tensor shape mismatch".to_string(),
    /// };
    /// assert!(!error.is_transient());
    /// ```
    #[error("Inference failed during {operation}: {reason}")]
    InferenceError {
        /// The operation that failed (e.g., "forward pass", "mean pooling")
        operation: String,
        /// The specific error details
        reason: String,
    },

    /// Candle tensor operation error
    ///
    /// Wraps errors from the candle-core crate during tensor operations
    #[error("Tensor operation error: {0}")]
    CandleError(#[from] candle_core::Error),

    /// Device initialization or selection failed
    ///
    /// Errors related to GPU/CPU device setup:
    /// - CUDA not available when requested
    /// - GPU driver version mismatch
    /// - Insufficient GPU memory for model
    /// - Device selection failed
    ///
    /// # Examples
    ///
    /// ```
    /// use abathur_cli::infrastructure::vector::bert_error::BertError;
    ///
    /// let error = BertError::DeviceError {
    ///     device_type: "CUDA".to_string(),
    ///     reason: "CUDA not available".to_string(),
    /// };
    /// assert!(!error.is_transient());
    /// ```
    #[error("Device error for {device_type}: {reason}")]
    DeviceError {
        /// The type of device (e.g., "CUDA", "CPU", "Metal")
        device_type: String,
        /// The specific error details
        reason: String,
    },

    /// GPU out of memory error
    ///
    /// Specific case of DeviceError when GPU runs out of memory.
    /// This error includes the attempted operation to help with debugging.
    ///
    /// # Examples
    ///
    /// ```
    /// use abathur_cli::infrastructure::vector::bert_error::BertError;
    ///
    /// let error = BertError::GpuOutOfMemory {
    ///     operation: "batched inference with 32 inputs".to_string(),
    /// };
    /// assert!(error.to_string().contains("GPU out of memory"));
    /// ```
    #[error("GPU out of memory during {operation}. Try reducing batch size or using CPU.")]
    GpuOutOfMemory {
        /// The operation that triggered OOM
        operation: String,
    },

    /// Invalid embedding dimensions
    ///
    /// Occurs when the model produces embeddings with unexpected dimensions,
    /// or when dimension conversion fails.
    #[error("Invalid embedding dimensions: expected {expected}, got {actual}")]
    InvalidDimensions {
        /// Expected embedding dimension
        expected: usize,
        /// Actual embedding dimension
        actual: usize,
    },

    /// Configuration error
    ///
    /// Errors in model configuration or parameters
    #[error("Configuration error: {0}")]
    ConfigError(String),

    /// Serialization or deserialization error
    ///
    /// Wraps serde_json errors when parsing model config or metadata
    #[error("Serialization error: {0}")]
    SerializationError(#[from] serde_json::Error),
}

impl BertError {
    /// Returns true if this error is transient and the operation should be retried
    ///
    /// Transient errors include:
    /// - Network errors during model download
    /// - Temporary I/O errors
    /// - HuggingFace Hub API rate limiting
    ///
    /// Non-transient errors include:
    /// - Invalid input text
    /// - Model validation failures
    /// - GPU out of memory (requires configuration change, not retry)
    /// - Device initialization failures
    ///
    /// # Examples
    ///
    /// ```
    /// use abathur_cli::infrastructure::vector::bert_error::BertError;
    ///
    /// // Network error during download is transient
    /// let error = BertError::ModelLoadError {
    ///     model_name: "test-model".to_string(),
    ///     reason: "Connection timeout".to_string(),
    /// };
    /// // Note: Currently ModelLoadError is not classified as transient
    /// // because model loading happens once at startup
    /// assert!(!error.is_transient());
    ///
    /// // Tokenization error is permanent
    /// let error = BertError::TokenizationError {
    ///     text_preview: "invalid".to_string(),
    ///     reason: "Invalid characters".to_string(),
    /// };
    /// assert!(!error.is_transient());
    /// ```
    pub fn is_transient(&self) -> bool {
        // Currently, most BERT errors are not transient because:
        // 1. Model loading happens once at startup (not in hot path)
        // 2. Tokenization/inference errors indicate programming bugs or invalid input
        // 3. Device errors require configuration changes
        //
        // If we add retry logic for model downloads in the future,
        // we could classify network-related ModelLoadError as transient
        matches!(self, BertError::IoError(_) if self.is_io_transient())
    }

    /// Returns true if this error is permanent and should not be retried
    ///
    /// Permanent errors include:
    /// - Validation failures
    /// - Invalid input
    /// - Configuration errors
    /// - GPU out of memory
    /// - Unsupported devices
    ///
    /// # Examples
    ///
    /// ```
    /// use abathur_cli::infrastructure::vector::bert_error::BertError;
    ///
    /// let error = BertError::GpuOutOfMemory {
    ///     operation: "inference".to_string(),
    /// };
    /// assert!(error.is_permanent());
    ///
    /// let error = BertError::TokenizationError {
    ///     text_preview: "test".to_string(),
    ///     reason: "Invalid input".to_string(),
    /// };
    /// assert!(error.is_permanent());
    /// ```
    pub fn is_permanent(&self) -> bool {
        matches!(
            self,
            BertError::ModelValidationError { .. }
                | BertError::TokenizationError { .. }
                | BertError::DeviceError { .. }
                | BertError::GpuOutOfMemory { .. }
                | BertError::InvalidDimensions { .. }
                | BertError::ConfigError(_)
        )
    }

    /// Helper to check if an I/O error is transient
    fn is_io_transient(&self) -> bool {
        if let BertError::IoError(io_err) = self {
            matches!(
                io_err.kind(),
                std::io::ErrorKind::TimedOut
                    | std::io::ErrorKind::Interrupted
                    | std::io::ErrorKind::WouldBlock
            )
        } else {
            false
        }
    }

    /// Create a ModelLoadError from model name and source error
    ///
    /// Helper constructor for common model loading failures.
    ///
    /// # Examples
    ///
    /// ```
    /// use abathur_cli::infrastructure::vector::bert_error::BertError;
    ///
    /// let error = BertError::model_load_error(
    ///     "sentence-transformers/all-MiniLM-L6-v2",
    ///     "Network timeout"
    /// );
    /// assert!(error.to_string().contains("all-MiniLM-L6-v2"));
    /// ```
    pub fn model_load_error(model_name: impl Into<String>, reason: impl Into<String>) -> Self {
        BertError::ModelLoadError {
            model_name: model_name.into(),
            reason: reason.into(),
        }
    }

    /// Create a TokenizationError with text preview and reason
    ///
    /// Automatically truncates text preview to 50 characters for error messages.
    ///
    /// # Examples
    ///
    /// ```
    /// use abathur_cli::infrastructure::vector::bert_error::BertError;
    ///
    /// let long_text = "a".repeat(100);
    /// let error = BertError::tokenization_error(&long_text, "Too long");
    /// let preview = error.to_string();
    /// assert!(preview.len() < 200); // Truncated in error message
    /// ```
    pub fn tokenization_error(text: impl AsRef<str>, reason: impl Into<String>) -> Self {
        let text_str = text.as_ref();
        let preview = if text_str.len() > 50 {
            format!("{}...", &text_str[..47])
        } else {
            text_str.to_string()
        };

        BertError::TokenizationError {
            text_preview: preview,
            reason: reason.into(),
        }
    }

    /// Create a GpuOutOfMemory error for a specific operation
    ///
    /// # Examples
    ///
    /// ```
    /// use abathur_cli::infrastructure::vector::bert_error::BertError;
    ///
    /// let error = BertError::gpu_oom("batch inference with 64 samples");
    /// assert!(error.to_string().contains("GPU out of memory"));
    /// assert!(error.to_string().contains("64 samples"));
    /// ```
    pub fn gpu_oom(operation: impl Into<String>) -> Self {
        BertError::GpuOutOfMemory {
            operation: operation.into(),
        }
    }

    /// Create a DeviceError for a specific device type
    ///
    /// # Examples
    ///
    /// ```
    /// use abathur_cli::infrastructure::vector::bert_error::BertError;
    ///
    /// let error = BertError::device_error("CUDA", "No CUDA-capable device found");
    /// assert!(error.to_string().contains("CUDA"));
    /// ```
    pub fn device_error(device_type: impl Into<String>, reason: impl Into<String>) -> Self {
        BertError::DeviceError {
            device_type: device_type.into(),
            reason: reason.into(),
        }
    }

    /// Create an InferenceError for a specific operation
    ///
    /// # Examples
    ///
    /// ```
    /// use abathur_cli::infrastructure::vector::bert_error::BertError;
    ///
    /// let error = BertError::inference_error("mean pooling", "Tensor shape mismatch");
    /// assert!(error.to_string().contains("mean pooling"));
    /// ```
    pub fn inference_error(operation: impl Into<String>, reason: impl Into<String>) -> Self {
        BertError::InferenceError {
            operation: operation.into(),
            reason: reason.into(),
        }
    }
}

/// Result type alias for BERT operations
pub type BertResult<T> = std::result::Result<T, BertError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_model_load_error_display() {
        let error = BertError::ModelLoadError {
            model_name: "sentence-transformers/all-MiniLM-L6-v2".to_string(),
            reason: "Network timeout".to_string(),
        };
        let display = error.to_string();
        assert!(display.contains("all-MiniLM-L6-v2"));
        assert!(display.contains("Network timeout"));
    }

    #[test]
    fn test_model_validation_error_display() {
        let error = BertError::ModelValidationError {
            model_name: "test-model".to_string(),
            reason: "Checksum mismatch".to_string(),
        };
        assert!(error.to_string().contains("validation failed"));
        assert!(error.to_string().contains("test-model"));
    }

    #[test]
    fn test_tokenization_error_display() {
        let error = BertError::TokenizationError {
            text_preview: "Some invalid text".to_string(),
            reason: "Invalid characters".to_string(),
        };
        let display = error.to_string();
        assert!(display.contains("Some invalid text"));
        assert!(display.contains("Invalid characters"));
    }

    #[test]
    fn test_inference_error_display() {
        let error = BertError::InferenceError {
            operation: "forward pass".to_string(),
            reason: "Tensor shape mismatch".to_string(),
        };
        assert!(error.to_string().contains("forward pass"));
        assert!(error.to_string().contains("Tensor shape mismatch"));
    }

    #[test]
    fn test_device_error_display() {
        let error = BertError::DeviceError {
            device_type: "CUDA".to_string(),
            reason: "No CUDA-capable device found".to_string(),
        };
        assert!(error.to_string().contains("CUDA"));
        assert!(error.to_string().contains("No CUDA-capable device found"));
    }

    #[test]
    fn test_gpu_oom_display() {
        let error = BertError::GpuOutOfMemory {
            operation: "batch inference".to_string(),
        };
        let display = error.to_string();
        assert!(display.contains("GPU out of memory"));
        assert!(display.contains("batch inference"));
        assert!(display.contains("reducing batch size"));
    }

    #[test]
    fn test_invalid_dimensions_display() {
        let error = BertError::InvalidDimensions {
            expected: 384,
            actual: 768,
        };
        assert!(error.to_string().contains("384"));
        assert!(error.to_string().contains("768"));
    }

    #[test]
    fn test_is_permanent_validation_error() {
        let error = BertError::ModelValidationError {
            model_name: "test".to_string(),
            reason: "invalid".to_string(),
        };
        assert!(error.is_permanent());
        assert!(!error.is_transient());
    }

    #[test]
    fn test_is_permanent_tokenization_error() {
        let error = BertError::TokenizationError {
            text_preview: "test".to_string(),
            reason: "invalid".to_string(),
        };
        assert!(error.is_permanent());
        assert!(!error.is_transient());
    }

    #[test]
    fn test_is_permanent_device_error() {
        let error = BertError::DeviceError {
            device_type: "CUDA".to_string(),
            reason: "Not available".to_string(),
        };
        assert!(error.is_permanent());
        assert!(!error.is_transient());
    }

    #[test]
    fn test_is_permanent_gpu_oom() {
        let error = BertError::GpuOutOfMemory {
            operation: "inference".to_string(),
        };
        assert!(error.is_permanent());
        assert!(!error.is_transient());
    }

    #[test]
    fn test_is_permanent_config_error() {
        let error = BertError::ConfigError("Invalid config".to_string());
        assert!(error.is_permanent());
        assert!(!error.is_transient());
    }

    #[test]
    fn test_helper_model_load_error() {
        let error = BertError::model_load_error("test-model", "Network error");
        assert!(matches!(error, BertError::ModelLoadError { .. }));
        assert!(error.to_string().contains("test-model"));
        assert!(error.to_string().contains("Network error"));
    }

    #[test]
    fn test_helper_tokenization_error_short_text() {
        let error = BertError::tokenization_error("short", "reason");
        if let BertError::TokenizationError { text_preview, .. } = error {
            assert_eq!(text_preview, "short");
        } else {
            panic!("Expected TokenizationError");
        }
    }

    #[test]
    fn test_helper_tokenization_error_long_text() {
        let long_text = "a".repeat(100);
        let error = BertError::tokenization_error(&long_text, "reason");
        if let BertError::TokenizationError { text_preview, .. } = error {
            assert!(text_preview.len() <= 50);
            assert!(text_preview.ends_with("..."));
        } else {
            panic!("Expected TokenizationError");
        }
    }

    #[test]
    fn test_helper_gpu_oom() {
        let error = BertError::gpu_oom("batch inference");
        assert!(matches!(error, BertError::GpuOutOfMemory { .. }));
        assert!(error.to_string().contains("batch inference"));
    }

    #[test]
    fn test_helper_device_error() {
        let error = BertError::device_error("Metal", "Not supported");
        assert!(matches!(error, BertError::DeviceError { .. }));
        assert!(error.to_string().contains("Metal"));
        assert!(error.to_string().contains("Not supported"));
    }

    #[test]
    fn test_helper_inference_error() {
        let error = BertError::inference_error("mean pooling", "NaN detected");
        assert!(matches!(error, BertError::InferenceError { .. }));
        assert!(error.to_string().contains("mean pooling"));
        assert!(error.to_string().contains("NaN detected"));
    }

    #[test]
    fn test_from_io_error() {
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "File not found");
        let bert_err: BertError = io_err.into();
        assert!(matches!(bert_err, BertError::IoError(_)));
    }

    #[test]
    fn test_from_serde_error() {
        let json = r#"{"invalid": json}"#;
        let serde_err: serde_json::Error =
            serde_json::from_str::<serde_json::Value>(json).unwrap_err();
        let bert_err: BertError = serde_err.into();
        assert!(matches!(bert_err, BertError::SerializationError(_)));
    }

    #[test]
    fn test_io_error_transient_timeout() {
        let io_err = std::io::Error::new(std::io::ErrorKind::TimedOut, "Timeout");
        let bert_err: BertError = io_err.into();
        assert!(bert_err.is_transient());
    }

    #[test]
    fn test_io_error_transient_interrupted() {
        let io_err = std::io::Error::new(std::io::ErrorKind::Interrupted, "Interrupted");
        let bert_err: BertError = io_err.into();
        assert!(bert_err.is_transient());
    }

    #[test]
    fn test_io_error_not_transient_not_found() {
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "Not found");
        let bert_err: BertError = io_err.into();
        assert!(!bert_err.is_transient());
    }

    #[test]
    fn test_result_type_alias() {
        let success: BertResult<i32> = Ok(42);
        assert_eq!(success.unwrap(), 42);

        let failure: BertResult<i32> = Err(BertError::ConfigError("test".to_string()));
        assert!(failure.is_err());
    }

    #[test]
    fn test_error_debug_derive() {
        let error = BertError::ConfigError("test".to_string());
        let debug = format!("{:?}", error);
        assert!(debug.contains("ConfigError"));
        assert!(debug.contains("test"));
    }
}
