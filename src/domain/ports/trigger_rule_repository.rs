//! Repository port for trigger rules.

use async_trait::async_trait;
use uuid::Uuid;

use crate::domain::errors::DomainResult;
use crate::services::trigger_rules::TriggerRule;

/// Repository for persisting and querying trigger rules.
#[async_trait]
pub trait TriggerRuleRepository: Send + Sync {
    /// Create a new trigger rule.
    async fn create(&self, rule: &TriggerRule) -> DomainResult<()>;

    /// Get a trigger rule by ID.
    async fn get(&self, id: Uuid) -> DomainResult<Option<TriggerRule>>;

    /// Get a trigger rule by name.
    async fn get_by_name(&self, name: &str) -> DomainResult<Option<TriggerRule>>;

    /// Update an existing trigger rule.
    async fn update(&self, rule: &TriggerRule) -> DomainResult<()>;

    /// Delete a trigger rule.
    async fn delete(&self, id: Uuid) -> DomainResult<()>;

    /// List all trigger rules.
    async fn list(&self) -> DomainResult<Vec<TriggerRule>>;

    /// List only enabled trigger rules.
    async fn list_enabled(&self) -> DomainResult<Vec<TriggerRule>>;
}
