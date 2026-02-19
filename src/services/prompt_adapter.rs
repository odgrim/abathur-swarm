//! Prompt-only adapter implementation.
//!
//! A prompt adapter wraps an [`AdapterManifest`] and a prompt content string
//! (typically loaded from an `ADAPTER.md` file). It does not execute actions
//! directly; instead, the prompt content is injected into the agent's context
//! so the LLM can follow the adapter's instructions natively.

use async_trait::async_trait;

use crate::domain::errors::{DomainError, DomainResult};
use crate::domain::models::adapter::{
    AdapterManifest, EgressAction, EgressResult,
};
use crate::domain::ports::adapter::EgressAdapter;

/// An adapter backed entirely by a prompt template.
///
/// Prompt adapters do not interact with external systems programmatically.
/// Instead, their prompt content is injected into the agent's system prompt
/// so the LLM can generate the correct output format. When `execute` is
/// called directly, it returns an error directing the caller to use the
/// prompt instructions instead.
#[derive(Debug, Clone)]
pub struct PromptAdapter {
    /// The adapter manifest declaring identity and capabilities.
    manifest: AdapterManifest,
    /// The prompt content loaded from ADAPTER.md (with env vars resolved).
    prompt_content: String,
}

impl PromptAdapter {
    /// Create a new prompt adapter.
    pub fn new(manifest: AdapterManifest, prompt_content: String) -> Self {
        Self {
            manifest,
            prompt_content,
        }
    }

    /// Returns the prompt content for injection into agent context.
    pub fn prompt_content(&self) -> &str {
        &self.prompt_content
    }
}

#[async_trait]
impl EgressAdapter for PromptAdapter {
    fn manifest(&self) -> &AdapterManifest {
        &self.manifest
    }

    /// Always returns an error — prompt adapters do not execute actions
    /// directly. The agent should follow the prompt instructions instead.
    async fn execute(&self, _action: &EgressAction) -> DomainResult<EgressResult> {
        Err(DomainError::ExecutionFailed(format!(
            "Adapter '{}' is a prompt-only adapter. Use the prompt instructions \
             injected into the agent context instead of calling execute directly.",
            self.manifest.name
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::models::adapter::{
        AdapterCapability, AdapterDirection, AdapterType,
    };

    fn test_manifest() -> AdapterManifest {
        AdapterManifest::new("test-prompt", AdapterType::Prompt, AdapterDirection::Egress)
            .with_description("A test prompt adapter")
            .with_capability(AdapterCapability::UpdateStatus)
    }

    #[test]
    fn test_prompt_adapter_new() {
        let manifest = test_manifest();
        let adapter = PromptAdapter::new(manifest.clone(), "# Instructions\nDo the thing.".into());

        assert_eq!(adapter.manifest().name, "test-prompt");
        assert_eq!(adapter.prompt_content(), "# Instructions\nDo the thing.");
    }

    #[tokio::test]
    async fn test_prompt_adapter_execute_returns_error() {
        let manifest = test_manifest();
        let adapter = PromptAdapter::new(manifest, "some prompt".into());

        let action = EgressAction::UpdateStatus {
            external_id: "PROJ-1".into(),
            new_status: "Done".into(),
        };

        let result = adapter.execute(&action).await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("prompt-only adapter"));
    }
}
