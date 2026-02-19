//! Native adapter plugins.
//!
//! This module contains compiled Rust adapter implementations. Each
//! sub-module corresponds to an external system (e.g., ClickUp, Jira)
//! and provides both ingestion and egress adapters that implement the
//! domain port traits directly.

pub mod clickup;

use std::sync::Arc;

use crate::domain::models::adapter::AdapterManifest;
use crate::domain::ports::adapter::{EgressAdapter, IngestionAdapter};

use self::clickup::client::ClickUpClient;
use self::clickup::egress::ClickUpEgressAdapter;
use self::clickup::ingestion::ClickUpIngestionAdapter;

/// Create native adapter instances from a manifest.
///
/// Returns a tuple of `(Option<IngestionAdapter>, Option<EgressAdapter>)`
/// based on the adapter's declared direction. The `_prompt_content`
/// parameter is ignored for native adapters but accepted for API
/// compatibility with the prompt-based adapter factory.
///
/// # Errors
///
/// Returns `Err` if the adapter name is unknown or if required
/// environment variables (e.g., `CLICKUP_API_KEY`) are not set.
pub fn create_native_adapter(
    manifest: &AdapterManifest,
    _prompt_content: &str,
) -> Result<
    (
        Option<Arc<dyn IngestionAdapter>>,
        Option<Arc<dyn EgressAdapter>>,
    ),
    String,
> {
    match manifest.name.as_str() {
        "clickup" => {
            let client = Arc::new(ClickUpClient::from_env()?);

            let ingestion: Option<Arc<dyn IngestionAdapter>> =
                if manifest.direction.supports_ingestion() {
                    Some(Arc::new(ClickUpIngestionAdapter::new(
                        manifest.clone(),
                        Arc::clone(&client),
                    )))
                } else {
                    None
                };

            let egress: Option<Arc<dyn EgressAdapter>> =
                if manifest.direction.supports_egress() {
                    Some(Arc::new(ClickUpEgressAdapter::new(
                        manifest.clone(),
                        Arc::clone(&client),
                    )))
                } else {
                    None
                };

            Ok((ingestion, egress))
        }
        unknown => Err(format!(
            "Unknown native adapter: '{unknown}'. Available native adapters: clickup"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::models::adapter::{
        AdapterCapability, AdapterDirection, AdapterType,
    };

    #[test]
    fn test_unknown_adapter_returns_error() {
        let manifest =
            AdapterManifest::new("nonexistent", AdapterType::Native, AdapterDirection::Ingestion)
                .with_capability(AdapterCapability::PollItems);

        let result = create_native_adapter(&manifest, "");
        match result {
            Err(msg) => assert!(msg.contains("Unknown native adapter"), "got: {msg}"),
            Ok(_) => panic!("Expected error for unknown adapter"),
        }
    }

    #[test]
    fn test_clickup_missing_env_var() {
        // Ensure the env var is not set.
        std::env::remove_var("CLICKUP_API_KEY");

        let manifest = AdapterManifest::new(
            "clickup",
            AdapterType::Native,
            AdapterDirection::Bidirectional,
        )
        .with_capability(AdapterCapability::PollItems)
        .with_capability(AdapterCapability::UpdateStatus);

        let result = create_native_adapter(&manifest, "");
        match result {
            Err(msg) => assert!(msg.contains("CLICKUP_API_KEY"), "got: {msg}"),
            Ok(_) => panic!("Expected error when CLICKUP_API_KEY is not set"),
        }
    }
}
