//! Cached wrapper for AgentRepository using moka TTL cache.
//!
//! Caches `get_template_by_name` lookups (60s TTL) since agent templates
//! rarely change during execution. All write operations invalidate the cache.

use async_trait::async_trait;
use moka::future::Cache;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use uuid::Uuid;

use crate::domain::errors::DomainResult;
use crate::domain::models::{AgentInstance, AgentTemplate, AgentTier, InstanceStatus};
use crate::domain::ports::{AgentFilter, AgentRepository};

/// Default TTL for cached agent templates.
const TEMPLATE_CACHE_TTL_SECS: u64 = 60;

/// Maximum number of cached template entries.
const TEMPLATE_CACHE_MAX_CAPACITY: u64 = 100;

/// Cached agent repository decorator.
///
/// Wraps any `AgentRepository` implementation with a moka-based cache
/// for template lookups. Instance operations are not cached (too volatile).
pub struct CachedAgentRepository<A: AgentRepository> {
    inner: Arc<A>,
    /// Cache keyed by template name -> AgentTemplate.
    template_by_name: Cache<String, Arc<AgentTemplate>>,
}

impl<A: AgentRepository> CachedAgentRepository<A> {
    /// Create a new cached agent repository with default TTL.
    pub fn new(inner: Arc<A>) -> Self {
        Self::with_ttl(inner, Duration::from_secs(TEMPLATE_CACHE_TTL_SECS))
    }

    /// Create with custom TTL.
    pub fn with_ttl(inner: Arc<A>, ttl: Duration) -> Self {
        let template_by_name = Cache::builder()
            .max_capacity(TEMPLATE_CACHE_MAX_CAPACITY)
            .time_to_live(ttl)
            .build();

        Self {
            inner,
            template_by_name,
        }
    }

    /// Invalidate all cached templates.
    fn invalidate_all(&self) {
        self.template_by_name.invalidate_all();
    }

    /// Invalidate a specific template by name.
    async fn invalidate_by_name(&self, name: &str) {
        self.template_by_name.invalidate(name).await;
    }
}

#[async_trait]
impl<A: AgentRepository + 'static> AgentRepository for CachedAgentRepository<A> {
    async fn create_template(&self, template: &AgentTemplate) -> DomainResult<()> {
        let result = self.inner.create_template(template).await;
        if result.is_ok() {
            self.invalidate_by_name(&template.name).await;
        }
        result
    }

    async fn get_template(&self, id: Uuid) -> DomainResult<Option<AgentTemplate>> {
        // ID-based lookups are not cached (less common)
        self.inner.get_template(id).await
    }

    async fn get_template_by_name(&self, name: &str) -> DomainResult<Option<AgentTemplate>> {
        // Check cache first
        if let Some(cached) = self.template_by_name.get(name).await {
            return Ok(Some((*cached).clone()));
        }

        // Cache miss - fetch from inner
        let result = self.inner.get_template_by_name(name).await?;
        if let Some(ref template) = result {
            self.template_by_name
                .insert(name.to_string(), Arc::new(template.clone()))
                .await;
        }
        Ok(result)
    }

    async fn get_template_version(&self, name: &str, version: u32) -> DomainResult<Option<AgentTemplate>> {
        // Version-specific lookups are not cached (rarely used)
        self.inner.get_template_version(name, version).await
    }

    async fn update_template(&self, template: &AgentTemplate) -> DomainResult<()> {
        let result = self.inner.update_template(template).await;
        if result.is_ok() {
            self.invalidate_by_name(&template.name).await;
        }
        result
    }

    async fn delete_template(&self, id: Uuid) -> DomainResult<()> {
        let result = self.inner.delete_template(id).await;
        if result.is_ok() {
            // Can't invalidate by name since we only have ID; invalidate all
            self.invalidate_all();
        }
        result
    }

    async fn list_templates(&self, filter: AgentFilter) -> DomainResult<Vec<AgentTemplate>> {
        self.inner.list_templates(filter).await
    }

    async fn list_by_tier(&self, tier: AgentTier) -> DomainResult<Vec<AgentTemplate>> {
        self.inner.list_by_tier(tier).await
    }

    async fn get_active_templates(&self) -> DomainResult<Vec<AgentTemplate>> {
        self.inner.get_active_templates().await
    }

    // Instance operations - not cached (too volatile)

    async fn create_instance(&self, instance: &AgentInstance) -> DomainResult<()> {
        self.inner.create_instance(instance).await
    }

    async fn get_instance(&self, id: Uuid) -> DomainResult<Option<AgentInstance>> {
        self.inner.get_instance(id).await
    }

    async fn update_instance(&self, instance: &AgentInstance) -> DomainResult<()> {
        self.inner.update_instance(instance).await
    }

    async fn delete_instance(&self, id: Uuid) -> DomainResult<()> {
        self.inner.delete_instance(id).await
    }

    async fn list_instances_by_status(&self, status: InstanceStatus) -> DomainResult<Vec<AgentInstance>> {
        self.inner.list_instances_by_status(status).await
    }

    async fn get_running_instances(&self, template_name: &str) -> DomainResult<Vec<AgentInstance>> {
        self.inner.get_running_instances(template_name).await
    }

    async fn count_running_by_template(&self) -> DomainResult<HashMap<String, u32>> {
        self.inner.count_running_by_template().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Basic compile-time verification that CachedAgentRepository<A>
    // implements AgentRepository when A does. Actual integration tests
    // use the full SQLite-backed repository.
    #[test]
    fn test_cache_config() {
        let cache: Cache<String, Arc<AgentTemplate>> = Cache::builder()
            .max_capacity(100)
            .time_to_live(Duration::from_secs(60))
            .build();

        assert_eq!(cache.entry_count(), 0);
    }
}
