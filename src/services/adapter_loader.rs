//! Adapter plugin loader.
//!
//! Discovers and loads adapter plugins from the `.abathur/adapters/` directory.
//! Each adapter lives in its own subdirectory and declares itself via an
//! `adapter.toml` manifest. Prompt adapters also include an `ADAPTER.md` file
//! containing instructions for the LLM.
//!
//! Loading is non-fatal: adapters that fail to load are logged with
//! [`tracing::warn`] and skipped. The system continues with whatever
//! adapters loaded successfully.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use thiserror::Error;

use crate::domain::models::adapter::{AdapterManifest, AdapterType};
use crate::domain::ports::adapter::{EgressAdapter, IngestionAdapter};
use crate::services::prompt_adapter::PromptAdapter;

/// Errors that can occur while loading a single adapter.
#[derive(Debug, Error)]
pub enum AdapterLoadError {
    /// The adapter directory does not contain an `adapter.toml` file.
    #[error("Missing adapter.toml in {0}")]
    MissingManifest(PathBuf),

    /// Failed to read a file from the adapter directory.
    #[error("IO error reading {path}: {source}")]
    Io {
        /// Path that could not be read.
        path: PathBuf,
        /// Underlying IO error.
        source: std::io::Error,
    },

    /// The `adapter.toml` file could not be parsed.
    #[error("Failed to parse adapter.toml in {path}: {source}")]
    TomlParse {
        /// Path to the manifest file.
        path: PathBuf,
        /// Underlying TOML parse error.
        source: toml::de::Error,
    },

    /// The manifest failed validation.
    #[error("Manifest validation failed for '{name}': {reason}")]
    ValidationFailed {
        /// Adapter name (or directory name if name is unknown).
        name: String,
        /// Validation error message.
        reason: String,
    },

    /// Required environment variables are not set.
    #[error("Missing required environment variables for '{name}': {vars:?}")]
    MissingEnvVars {
        /// Adapter name.
        name: String,
        /// List of missing environment variable names.
        vars: Vec<String>,
    },

    /// The ADAPTER.md file is required for prompt adapters but was not found.
    #[error("Prompt adapter '{name}' is missing ADAPTER.md")]
    MissingPromptFile {
        /// Adapter name.
        name: String,
    },

    /// Native adapter creation failed.
    #[error("Failed to create native adapter '{name}': {reason}")]
    NativeCreationFailed {
        /// Adapter name.
        name: String,
        /// Error message from the native factory.
        reason: String,
    },
}

/// A successfully loaded adapter with its manifest and trait implementations.
///
/// Depending on the adapter's direction, it may have an ingestion implementation,
/// an egress implementation, or both. Prompt adapters also carry their resolved
/// prompt content for injection into agent context.
pub struct LoadedAdapter {
    /// The adapter's parsed and validated manifest.
    pub manifest: AdapterManifest,
    /// Ingestion adapter implementation, if the adapter supports ingestion.
    pub ingestion: Option<Box<dyn IngestionAdapter>>,
    /// Egress adapter implementation, if the adapter supports egress.
    pub egress: Option<Box<dyn EgressAdapter>>,
    /// Resolved prompt content for prompt adapters (from ADAPTER.md).
    pub prompt_content: Option<String>,
}

impl std::fmt::Debug for LoadedAdapter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LoadedAdapter")
            .field("manifest", &self.manifest)
            .field("has_ingestion", &self.ingestion.is_some())
            .field("has_egress", &self.egress.is_some())
            .field("has_prompt", &self.prompt_content.is_some())
            .finish()
    }
}

/// Load all adapters from the given base directory.
///
/// Scans `{base_dir}/adapters/*/adapter.toml` for adapter manifests,
/// validates them, and creates the appropriate adapter implementations.
/// Adapters that fail to load are logged with [`tracing::warn`] and
/// skipped — the function never panics.
///
/// Returns a vector of successfully loaded adapters along with a map
/// of adapter name to prompt content (for prompt adapters).
pub async fn load_adapters(base_dir: &Path) -> Vec<LoadedAdapter> {
    let adapters_dir = base_dir.join("adapters");

    if !adapters_dir.exists() {
        tracing::info!(
            path = %adapters_dir.display(),
            "No adapters directory found, skipping adapter loading"
        );
        return Vec::new();
    }

    let entries = match std::fs::read_dir(&adapters_dir) {
        Ok(entries) => entries,
        Err(e) => {
            tracing::warn!(
                path = %adapters_dir.display(),
                error = %e,
                "Failed to read adapters directory"
            );
            return Vec::new();
        }
    };

    let mut loaded = Vec::new();

    for entry in entries {
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                tracing::warn!(error = %e, "Failed to read directory entry");
                continue;
            }
        };

        let path = entry.path();
        if !path.is_dir() {
            continue;
        }

        match load_single_adapter(&path).await {
            Ok(adapter) => {
                tracing::info!(
                    adapter = %adapter.manifest.name,
                    adapter_type = %adapter.manifest.adapter_type.as_str(),
                    direction = %adapter.manifest.direction.as_str(),
                    "Loaded adapter"
                );
                loaded.push(adapter);
            }
            Err(e) => {
                tracing::warn!(
                    path = %path.display(),
                    error = %e,
                    "Skipping adapter that failed to load"
                );
            }
        }
    }

    tracing::info!(count = loaded.len(), "Adapter loading complete");
    loaded
}

/// Attempt to load a single adapter from its directory.
///
/// Reads `adapter.toml`, validates the manifest, checks environment
/// variables, and creates the appropriate adapter implementation based
/// on the adapter type.
async fn load_single_adapter(adapter_dir: &Path) -> Result<LoadedAdapter, AdapterLoadError> {
    let manifest_path = adapter_dir.join("adapter.toml");

    if !manifest_path.exists() {
        return Err(AdapterLoadError::MissingManifest(adapter_dir.to_path_buf()));
    }

    // Read and parse the manifest
    let manifest_content = std::fs::read_to_string(&manifest_path).map_err(|e| {
        AdapterLoadError::Io {
            path: manifest_path.clone(),
            source: e,
        }
    })?;

    let manifest: AdapterManifest =
        toml::from_str(&manifest_content).map_err(|e| AdapterLoadError::TomlParse {
            path: manifest_path.clone(),
            source: e,
        })?;

    // Validate the manifest
    manifest.validate().map_err(|reason| {
        AdapterLoadError::ValidationFailed {
            name: manifest.name.clone(),
            reason,
        }
    })?;

    // Check for missing environment variables referenced in config
    let missing = find_missing_env_vars(&manifest);
    if !missing.is_empty() {
        return Err(AdapterLoadError::MissingEnvVars {
            name: manifest.name.clone(),
            vars: missing,
        });
    }

    // Create adapter based on type
    match manifest.adapter_type {
        AdapterType::Prompt => load_prompt_adapter(adapter_dir, manifest).await,
        AdapterType::Native => load_native_adapter(manifest).await,
    }
}

/// Load a prompt-based adapter by reading its ADAPTER.md file.
async fn load_prompt_adapter(
    adapter_dir: &Path,
    manifest: AdapterManifest,
) -> Result<LoadedAdapter, AdapterLoadError> {
    let prompt_path = adapter_dir.join("ADAPTER.md");

    if !prompt_path.exists() {
        return Err(AdapterLoadError::MissingPromptFile {
            name: manifest.name.clone(),
        });
    }

    let raw_content = std::fs::read_to_string(&prompt_path).map_err(|e| AdapterLoadError::Io {
        path: prompt_path,
        source: e,
    })?;

    let prompt_content = resolve_env_placeholders(&raw_content);
    let prompt_adapter = PromptAdapter::new(manifest.clone(), prompt_content.clone());

    // Prompt adapters provide egress via prompt injection (not programmatic execution).
    // If the adapter also declares ingestion direction, we cannot fulfill it with
    // a prompt-only adapter, so ingestion is None.
    let egress: Option<Box<dyn EgressAdapter>> = if manifest.direction.supports_egress() {
        Some(Box::new(prompt_adapter))
    } else {
        None
    };

    Ok(LoadedAdapter {
        manifest,
        ingestion: None,
        egress,
        prompt_content: Some(prompt_content),
    })
}

/// Load a native (compiled Rust) adapter.
///
/// Delegates to the native adapter factory in `crate::adapters::plugins`.
async fn load_native_adapter(
    manifest: AdapterManifest,
) -> Result<LoadedAdapter, AdapterLoadError> {
    let (ingestion, egress) =
        crate::adapters::plugins::create_native_adapter(&manifest, "").map_err(|reason| {
            AdapterLoadError::NativeCreationFailed {
                name: manifest.name.clone(),
                reason,
            }
        })?;

    Ok(LoadedAdapter {
        manifest,
        ingestion,
        egress,
        prompt_content: None,
    })
}

/// Find environment variable names referenced in the manifest config that
/// are not set in the current environment.
///
/// Scans all config values for `{{ENV_VAR}}` patterns and checks whether
/// each referenced variable is present in the environment.
pub fn find_missing_env_vars(manifest: &AdapterManifest) -> Vec<String> {
    let mut missing = Vec::new();
    let mut seen = std::collections::HashSet::new();

    for value in manifest.config.values() {
        let text = match value {
            serde_json::Value::String(s) => s.clone(),
            other => other.to_string(),
        };

        for var_name in extract_env_var_names(&text) {
            if seen.insert(var_name.clone()) && std::env::var(&var_name).is_err() {
                missing.push(var_name);
            }
        }
    }

    missing
}

/// Extract environment variable names from `{{ENV_VAR}}` patterns in text.
pub fn extract_env_var_names(text: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut remaining = text;

    while let Some(start) = remaining.find("{{") {
        let after_open = &remaining[start + 2..];
        if let Some(end) = after_open.find("}}") {
            let var_name = after_open[..end].trim().to_string();
            if !var_name.is_empty() {
                names.push(var_name);
            }
            remaining = &after_open[end + 2..];
        } else {
            break;
        }
    }

    names
}

/// Resolve `{{ENV_VAR}}` placeholders in content by replacing them with
/// the corresponding environment variable values.
///
/// If an environment variable is not set, the placeholder is left as-is.
/// This allows for graceful degradation when optional env vars are missing.
pub fn resolve_env_placeholders(content: &str) -> String {
    let mut result = String::with_capacity(content.len());
    let mut remaining = content;

    while let Some(start) = remaining.find("{{") {
        // Copy everything before the placeholder
        result.push_str(&remaining[..start]);

        let after_open = &remaining[start + 2..];
        if let Some(end) = after_open.find("}}") {
            let var_name = after_open[..end].trim();
            if var_name.is_empty() {
                // Empty placeholder — preserve as-is
                result.push_str("{{}}");
            } else {
                match std::env::var(var_name) {
                    Ok(value) => result.push_str(&value),
                    Err(_) => {
                        // Leave unresolved placeholders intact
                        result.push_str("{{");
                        result.push_str(&after_open[..end]);
                        result.push_str("}}");
                    }
                }
            }
            remaining = &after_open[end + 2..];
        } else {
            // No closing `}}` — copy rest as-is
            result.push_str(&remaining[start..]);
            remaining = "";
            break;
        }
    }

    // Copy any remaining text after the last placeholder
    result.push_str(remaining);
    result
}

/// Collect prompt content from loaded adapters.
///
/// Returns a map of adapter name to resolved prompt content for all
/// prompt adapters that have prompt content available.
pub fn collect_prompt_content(adapters: &[LoadedAdapter]) -> HashMap<String, String> {
    let mut prompts = HashMap::new();

    for adapter in adapters {
        if let Some(content) = &adapter.prompt_content {
            prompts.insert(adapter.manifest.name.clone(), content.clone());
        }
    }

    prompts
}

#[cfg(test)]
mod tests {
    use super::*;

    // SAFETY for all unsafe blocks below: test-only env var manipulation.
    // Tests are run with `--test-threads=1` or have unique var names to avoid races.

    #[test]
    fn test_resolve_env_placeholders_with_set_var() {
        unsafe { std::env::set_var("TEST_ADAPTER_VAR_1", "hello_world") };
        let input = "Base URL: {{TEST_ADAPTER_VAR_1}}/api";
        let result = resolve_env_placeholders(input);
        assert_eq!(result, "Base URL: hello_world/api");
        unsafe { std::env::remove_var("TEST_ADAPTER_VAR_1") };
    }

    #[test]
    fn test_resolve_env_placeholders_unset_var_preserved() {
        unsafe { std::env::remove_var("DEFINITELY_NOT_SET_XYZZY") };
        let input = "Token: {{DEFINITELY_NOT_SET_XYZZY}}";
        let result = resolve_env_placeholders(input);
        assert_eq!(result, "Token: {{DEFINITELY_NOT_SET_XYZZY}}");
    }

    #[test]
    fn test_resolve_env_placeholders_multiple() {
        unsafe { std::env::set_var("TEST_ADAPTER_HOST", "example.com") };
        unsafe { std::env::set_var("TEST_ADAPTER_PORT", "8080") };
        let input = "{{TEST_ADAPTER_HOST}}:{{TEST_ADAPTER_PORT}}";
        let result = resolve_env_placeholders(input);
        assert_eq!(result, "example.com:8080");
        unsafe { std::env::remove_var("TEST_ADAPTER_HOST") };
        unsafe { std::env::remove_var("TEST_ADAPTER_PORT") };
    }

    #[test]
    fn test_resolve_env_placeholders_no_placeholders() {
        let input = "No placeholders here";
        let result = resolve_env_placeholders(input);
        assert_eq!(result, "No placeholders here");
    }

    #[test]
    fn test_resolve_env_placeholders_empty_placeholder() {
        let input = "Empty: {{}}";
        let result = resolve_env_placeholders(input);
        assert_eq!(result, "Empty: {{}}");
    }

    #[test]
    fn test_resolve_env_placeholders_unclosed() {
        let input = "Unclosed: {{ no closing";
        let result = resolve_env_placeholders(input);
        assert_eq!(result, "Unclosed: {{ no closing");
    }

    #[test]
    fn test_resolve_env_placeholders_whitespace_trimmed() {
        unsafe { std::env::set_var("TEST_ADAPTER_TRIMMED", "trimmed") };
        let input = "{{ TEST_ADAPTER_TRIMMED }}";
        let result = resolve_env_placeholders(input);
        assert_eq!(result, "trimmed");
        unsafe { std::env::remove_var("TEST_ADAPTER_TRIMMED") };
    }

    #[test]
    fn test_extract_env_var_names() {
        let text = "{{FOO}} and {{BAR}} and {{BAZ}}";
        let names = extract_env_var_names(text);
        assert_eq!(names, vec!["FOO", "BAR", "BAZ"]);
    }

    #[test]
    fn test_extract_env_var_names_empty() {
        let text = "no placeholders";
        let names = extract_env_var_names(text);
        assert!(names.is_empty());
    }

    #[test]
    fn test_find_missing_env_vars_all_present() {
        unsafe { std::env::set_var("TEST_ADAPTER_PRESENT", "yes") };
        let manifest = AdapterManifest::new(
            "test",
            AdapterType::Prompt,
            crate::domain::models::adapter::AdapterDirection::Egress,
        )
        .with_capability(crate::domain::models::adapter::AdapterCapability::UpdateStatus)
        .with_config("url", serde_json::json!("{{TEST_ADAPTER_PRESENT}}"));

        let missing = find_missing_env_vars(&manifest);
        assert!(missing.is_empty());
        unsafe { std::env::remove_var("TEST_ADAPTER_PRESENT") };
    }

    #[test]
    fn test_find_missing_env_vars_some_missing() {
        unsafe { std::env::remove_var("TEST_ADAPTER_MISSING_XYZ") };
        let manifest = AdapterManifest::new(
            "test",
            AdapterType::Prompt,
            crate::domain::models::adapter::AdapterDirection::Egress,
        )
        .with_capability(crate::domain::models::adapter::AdapterCapability::UpdateStatus)
        .with_config("token", serde_json::json!("{{TEST_ADAPTER_MISSING_XYZ}}"));

        let missing = find_missing_env_vars(&manifest);
        assert_eq!(missing, vec!["TEST_ADAPTER_MISSING_XYZ"]);
    }

    #[tokio::test]
    async fn test_load_adapters_nonexistent_dir() {
        let dir = PathBuf::from("/tmp/abathur_test_nonexistent_dir_xyz");
        let result = load_adapters(&dir).await;
        assert!(result.is_empty());
    }

    #[tokio::test]
    async fn test_load_single_adapter_missing_manifest() {
        let dir = tempfile::tempdir().unwrap();
        let adapter_dir = dir.path().join("my-adapter");
        std::fs::create_dir_all(&adapter_dir).unwrap();

        let result = load_single_adapter(&adapter_dir).await;
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), AdapterLoadError::MissingManifest(_)));
    }

    #[tokio::test]
    async fn test_load_single_adapter_invalid_toml() {
        let dir = tempfile::tempdir().unwrap();
        let adapter_dir = dir.path().join("bad-adapter");
        std::fs::create_dir_all(&adapter_dir).unwrap();
        std::fs::write(adapter_dir.join("adapter.toml"), "not valid toml {{{{").unwrap();

        let result = load_single_adapter(&adapter_dir).await;
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), AdapterLoadError::TomlParse { .. }));
    }

    #[tokio::test]
    async fn test_load_prompt_adapter_success() {
        let dir = tempfile::tempdir().unwrap();
        let adapter_dir = dir.path().join("test-prompt");
        std::fs::create_dir_all(&adapter_dir).unwrap();

        // Serialize a real manifest to get the correct TOML format.
        let manifest = AdapterManifest::new(
            "test-prompt",
            AdapterType::Prompt,
            crate::domain::models::adapter::AdapterDirection::Egress,
        )
        .with_description("A test prompt adapter")
        .with_capability(crate::domain::models::adapter::AdapterCapability::UpdateStatus);

        let manifest_toml = toml::to_string_pretty(&manifest).unwrap();
        std::fs::write(adapter_dir.join("adapter.toml"), &manifest_toml).unwrap();
        std::fs::write(
            adapter_dir.join("ADAPTER.md"),
            "# Test Prompt\nDo the thing with {{TEST_VAR}}.",
        )
        .unwrap();

        let result = load_single_adapter(&adapter_dir).await;
        assert!(result.is_ok());
        let loaded = result.unwrap();
        assert_eq!(loaded.manifest.name, "test-prompt");
        assert!(loaded.ingestion.is_none());
        assert!(loaded.egress.is_some());
    }

    #[tokio::test]
    async fn test_load_prompt_adapter_missing_md() {
        let dir = tempfile::tempdir().unwrap();
        let adapter_dir = dir.path().join("no-md");
        std::fs::create_dir_all(&adapter_dir).unwrap();

        let manifest = AdapterManifest::new(
            "no-md",
            AdapterType::Prompt,
            crate::domain::models::adapter::AdapterDirection::Egress,
        )
        .with_description("Missing ADAPTER.md")
        .with_capability(crate::domain::models::adapter::AdapterCapability::UpdateStatus);

        let manifest_toml = toml::to_string_pretty(&manifest).unwrap();
        std::fs::write(adapter_dir.join("adapter.toml"), &manifest_toml).unwrap();

        let result = load_single_adapter(&adapter_dir).await;
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            AdapterLoadError::MissingPromptFile { .. }
        ));
    }

    #[tokio::test]
    async fn test_load_unknown_native_adapter_fails() {
        let dir = tempfile::tempdir().unwrap();
        let adapter_dir = dir.path().join("native-test");
        std::fs::create_dir_all(&adapter_dir).unwrap();

        let manifest = AdapterManifest::new(
            "native-test",
            AdapterType::Native,
            crate::domain::models::adapter::AdapterDirection::Egress,
        )
        .with_description("Native adapter test")
        .with_capability(crate::domain::models::adapter::AdapterCapability::UpdateStatus);

        let manifest_toml = toml::to_string_pretty(&manifest).unwrap();
        std::fs::write(adapter_dir.join("adapter.toml"), &manifest_toml).unwrap();

        let result = load_single_adapter(&adapter_dir).await;
        assert!(result.is_err());
        // "native-test" is not a known native adapter, so the factory returns an error
        assert!(matches!(
            result.unwrap_err(),
            AdapterLoadError::NativeCreationFailed { .. }
        ));
    }

    #[tokio::test]
    async fn test_load_adapters_full_scan() {
        let dir = tempfile::tempdir().unwrap();
        let adapters_dir = dir.path().join("adapters");
        std::fs::create_dir_all(&adapters_dir).unwrap();

        // Create a valid prompt adapter
        let adapter1_dir = adapters_dir.join("adapter-one");
        std::fs::create_dir_all(&adapter1_dir).unwrap();

        let manifest1 = AdapterManifest::new(
            "adapter-one",
            AdapterType::Prompt,
            crate::domain::models::adapter::AdapterDirection::Egress,
        )
        .with_description("First adapter")
        .with_capability(crate::domain::models::adapter::AdapterCapability::UpdateStatus);

        std::fs::write(
            adapter1_dir.join("adapter.toml"),
            toml::to_string_pretty(&manifest1).unwrap(),
        )
        .unwrap();
        std::fs::write(adapter1_dir.join("ADAPTER.md"), "# Adapter One").unwrap();

        // Create an invalid adapter (no adapter.toml)
        let adapter2_dir = adapters_dir.join("adapter-two");
        std::fs::create_dir_all(&adapter2_dir).unwrap();

        let result = load_adapters(dir.path()).await;
        // Only one should load successfully
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].manifest.name, "adapter-one");
    }
}
