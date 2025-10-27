use anyhow::Result;

use crate::domain::models::{Memory as DomainMemory, MemoryType};
use crate::services::MemoryService as RealMemoryService;

/// Adapter to make the domain MemoryService compatible with CLI commands
///
/// This adapter wraps the real memory service and provides a compatible
/// interface for CLI command handlers.
pub struct MemoryServiceAdapter {
    service: RealMemoryService,
}

impl MemoryServiceAdapter {
    pub fn new(service: RealMemoryService) -> Self {
        Self { service }
    }

    /// Search memories by namespace prefix and type
    pub async fn search(
        &self,
        namespace_prefix: &str,
        memory_type: Option<MemoryType>,
        limit: Option<usize>,
    ) -> Result<Vec<DomainMemory>> {
        self.service.search(namespace_prefix, memory_type, limit).await
    }

    /// Get a memory by namespace and key
    pub async fn get(
        &self,
        namespace: &str,
        key: &str,
    ) -> Result<Option<DomainMemory>> {
        self.service.get(namespace, key).await
    }

    /// Count memories matching criteria
    pub async fn count(
        &self,
        namespace_prefix: &str,
        memory_type: Option<MemoryType>,
    ) -> Result<usize> {
        self.service.count(namespace_prefix, memory_type).await
    }
}
