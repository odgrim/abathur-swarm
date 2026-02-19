//! Adapter registry for loaded adapters.
//!
//! The [`AdapterRegistry`] holds all successfully loaded adapters and
//! provides lookup methods by name. It also stores prompt content for
//! prompt-based adapters and can generate a consolidated prompt section
//! for injection into agent system prompts.

use std::collections::HashMap;

use crate::domain::models::adapter::AdapterManifest;
use crate::domain::ports::adapter::{EgressAdapter, IngestionAdapter};
use crate::services::adapter_loader::LoadedAdapter;

/// Central registry of loaded adapters.
///
/// Provides indexed access to adapter manifests, ingestion implementations,
/// egress implementations, and prompt content. Constructed from the output
/// of [`load_adapters`](crate::services::adapter_loader::load_adapters).
pub struct AdapterRegistry {
    /// Adapter manifests keyed by adapter name.
    manifests: HashMap<String, AdapterManifest>,
    /// Ingestion adapter implementations keyed by adapter name.
    ingestion: HashMap<String, Box<dyn IngestionAdapter>>,
    /// Egress adapter implementations keyed by adapter name.
    egress: HashMap<String, Box<dyn EgressAdapter>>,
    /// Prompt content keyed by adapter name (for prompt adapters).
    prompts: HashMap<String, String>,
}

impl std::fmt::Debug for AdapterRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AdapterRegistry")
            .field("manifests", &self.manifests.keys().collect::<Vec<_>>())
            .field("ingestion", &self.ingestion.keys().collect::<Vec<_>>())
            .field("egress", &self.egress.keys().collect::<Vec<_>>())
            .field("prompts", &self.prompts.keys().collect::<Vec<_>>())
            .finish()
    }
}

impl Default for AdapterRegistry {
    /// Returns an empty registry with no adapters loaded.
    fn default() -> Self {
        Self {
            manifests: HashMap::new(),
            ingestion: HashMap::new(),
            egress: HashMap::new(),
            prompts: HashMap::new(),
        }
    }
}

impl AdapterRegistry {
    /// Build a registry from loaded adapters and their prompt content.
    ///
    /// Takes ownership of the loaded adapters and indexes them by name.
    /// The `prompts` map provides prompt content for prompt-based adapters
    /// (keyed by adapter name).
    pub fn from_loaded(
        adapters: Vec<LoadedAdapter>,
        prompts: HashMap<String, String>,
    ) -> Self {
        let mut manifests = HashMap::new();
        let mut ingestion = HashMap::new();
        let mut egress = HashMap::new();

        for adapter in adapters {
            let name = adapter.manifest.name.clone();
            manifests.insert(name.clone(), adapter.manifest);

            if let Some(ing) = adapter.ingestion {
                ingestion.insert(name.clone(), ing);
            }
            if let Some(egr) = adapter.egress {
                egress.insert(name.clone(), egr);
            }
        }

        Self {
            manifests,
            ingestion,
            egress,
            prompts,
        }
    }

    /// Look up an ingestion adapter by name.
    pub fn get_ingestion(&self, name: &str) -> Option<&dyn IngestionAdapter> {
        self.ingestion.get(name).map(|a| a.as_ref())
    }

    /// Look up an egress adapter by name.
    pub fn get_egress(&self, name: &str) -> Option<&dyn EgressAdapter> {
        self.egress.get(name).map(|a| a.as_ref())
    }

    /// Look up an adapter manifest by name.
    pub fn get_manifest(&self, name: &str) -> Option<&AdapterManifest> {
        self.manifests.get(name)
    }

    /// Returns the names of all registered adapters.
    pub fn adapter_names(&self) -> Vec<&str> {
        self.manifests.keys().map(|s| s.as_str()).collect()
    }

    /// Returns all registered manifests.
    pub fn manifests(&self) -> &HashMap<String, AdapterManifest> {
        &self.manifests
    }

    /// Returns the names of adapters that provide ingestion.
    pub fn ingestion_names(&self) -> Vec<&str> {
        self.ingestion.keys().map(|s| s.as_str()).collect()
    }

    /// Returns the names of adapters that provide egress.
    pub fn egress_names(&self) -> Vec<&str> {
        self.egress.keys().map(|s| s.as_str()).collect()
    }

    /// Whether any adapters are registered.
    pub fn has_adapters(&self) -> bool {
        !self.manifests.is_empty()
    }

    /// Build a markdown section describing all egress adapters for
    /// injection into an agent's system prompt.
    ///
    /// For prompt adapters, the full prompt content is included.
    /// For native adapters, a summary of capabilities is generated.
    pub fn build_egress_prompt_section(&self) -> String {
        if self.egress.is_empty() {
            return String::new();
        }

        let mut section = String::from("## External System Adapters\n\n");
        section.push_str(
            "The following adapters are available for interacting with external systems:\n\n",
        );

        for (name, manifest) in &self.manifests {
            if !manifest.direction.supports_egress() {
                continue;
            }

            section.push_str(&format!("### {}\n\n", name));
            if !manifest.description.is_empty() {
                section.push_str(&format!("{}\n\n", manifest.description));
            }

            // Include capabilities
            if !manifest.capabilities.is_empty() {
                section.push_str("**Capabilities:** ");
                let cap_strs: Vec<&str> =
                    manifest.capabilities.iter().map(|c| c.as_str()).collect();
                section.push_str(&cap_strs.join(", "));
                section.push_str("\n\n");
            }

            // Include prompt content for prompt adapters
            if let Some(prompt) = self.prompts.get(name) {
                section.push_str(prompt);
                section.push_str("\n\n");
            }
        }

        section
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::models::adapter::{
        AdapterCapability, AdapterDirection, AdapterType,
    };

    fn make_egress_manifest(name: &str) -> AdapterManifest {
        AdapterManifest::new(name, AdapterType::Prompt, AdapterDirection::Egress)
            .with_description(format!("{} adapter", name))
            .with_capability(AdapterCapability::UpdateStatus)
    }

    fn make_ingestion_manifest(name: &str) -> AdapterManifest {
        AdapterManifest::new(name, AdapterType::Native, AdapterDirection::Ingestion)
            .with_description(format!("{} ingestion", name))
            .with_capability(AdapterCapability::PollItems)
    }

    #[test]
    fn test_default_registry_is_empty() {
        let registry = AdapterRegistry::default();
        assert!(!registry.has_adapters());
        assert!(registry.adapter_names().is_empty());
        assert!(registry.ingestion_names().is_empty());
        assert!(registry.egress_names().is_empty());
    }

    #[test]
    fn test_from_loaded_with_egress() {
        let manifest = make_egress_manifest("jira");
        let loaded = vec![LoadedAdapter {
            manifest: manifest.clone(),
            ingestion: None,
            egress: Some(Box::new(crate::services::prompt_adapter::PromptAdapter::new(
                manifest,
                "# Jira Instructions".into(),
            ))),
            prompt_content: Some("# Jira Instructions".into()),
        }];

        let mut prompts = HashMap::new();
        prompts.insert("jira".into(), "# Jira Instructions".into());

        let registry = AdapterRegistry::from_loaded(loaded, prompts);

        assert!(registry.has_adapters());
        assert_eq!(registry.adapter_names().len(), 1);
        assert!(registry.get_manifest("jira").is_some());
        assert!(registry.get_egress("jira").is_some());
        assert!(registry.get_ingestion("jira").is_none());
    }

    #[test]
    fn test_from_loaded_no_adapters() {
        let registry = AdapterRegistry::from_loaded(Vec::new(), HashMap::new());
        assert!(!registry.has_adapters());
    }

    #[test]
    fn test_egress_names() {
        let manifest = make_egress_manifest("github");
        let loaded = vec![LoadedAdapter {
            manifest: manifest.clone(),
            ingestion: None,
            egress: Some(Box::new(crate::services::prompt_adapter::PromptAdapter::new(
                manifest,
                "# GitHub".into(),
            ))),
            prompt_content: Some("# GitHub".into()),
        }];

        let registry = AdapterRegistry::from_loaded(loaded, HashMap::new());
        let names = registry.egress_names();
        assert_eq!(names.len(), 1);
        assert!(names.contains(&"github"));
    }

    #[test]
    fn test_manifests_returns_all() {
        let m1 = make_egress_manifest("adapter-a");
        let m2 = make_egress_manifest("adapter-b");

        let loaded = vec![
            LoadedAdapter {
                manifest: m1.clone(),
                ingestion: None,
                egress: Some(Box::new(crate::services::prompt_adapter::PromptAdapter::new(
                    m1, "a".into(),
                ))),
                prompt_content: Some("a".into()),
            },
            LoadedAdapter {
                manifest: m2.clone(),
                ingestion: None,
                egress: Some(Box::new(crate::services::prompt_adapter::PromptAdapter::new(
                    m2, "b".into(),
                ))),
                prompt_content: Some("b".into()),
            },
        ];

        let registry = AdapterRegistry::from_loaded(loaded, HashMap::new());
        assert_eq!(registry.manifests().len(), 2);
    }

    #[test]
    fn test_build_egress_prompt_section_empty() {
        let registry = AdapterRegistry::default();
        let section = registry.build_egress_prompt_section();
        assert!(section.is_empty());
    }

    #[test]
    fn test_build_egress_prompt_section_with_content() {
        let manifest = make_egress_manifest("linear");
        let loaded = vec![LoadedAdapter {
            manifest: manifest.clone(),
            ingestion: None,
            egress: Some(Box::new(crate::services::prompt_adapter::PromptAdapter::new(
                manifest,
                "# Linear Prompt".into(),
            ))),
            prompt_content: Some("# Linear Prompt".into()),
        }];

        let mut prompts = HashMap::new();
        prompts.insert("linear".into(), "# Linear Prompt".into());

        let registry = AdapterRegistry::from_loaded(loaded, prompts);
        let section = registry.build_egress_prompt_section();

        assert!(section.contains("External System Adapters"));
        assert!(section.contains("linear"));
        assert!(section.contains("# Linear Prompt"));
        assert!(section.contains("update_status"));
    }

    #[test]
    fn test_build_egress_prompt_section_skips_ingestion_only() {
        let egress_manifest = make_egress_manifest("egress-only");
        let ingestion_manifest = make_ingestion_manifest("ingestion-only");

        let loaded = vec![
            LoadedAdapter {
                manifest: egress_manifest.clone(),
                ingestion: None,
                egress: Some(Box::new(crate::services::prompt_adapter::PromptAdapter::new(
                    egress_manifest,
                    "egress prompt".into(),
                ))),
                prompt_content: Some("egress prompt".into()),
            },
            LoadedAdapter {
                manifest: ingestion_manifest,
                ingestion: None, // Would be Some in a real scenario
                egress: None,
                prompt_content: None,
            },
        ];

        let registry = AdapterRegistry::from_loaded(loaded, HashMap::new());
        let section = registry.build_egress_prompt_section();

        assert!(section.contains("egress-only"));
        assert!(!section.contains("ingestion-only"));
    }

    #[test]
    fn test_debug_impl() {
        let registry = AdapterRegistry::default();
        let debug = format!("{:?}", registry);
        assert!(debug.contains("AdapterRegistry"));
    }
}
