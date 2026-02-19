//! Adapter port traits.
//!
//! These define the interface that ingestion and egress adapters must
//! implement. The swarm interacts with external systems exclusively
//! through these traits, keeping the domain layer decoupled from any
//! specific external system.

use async_trait::async_trait;
use chrono::{DateTime, Utc};

use crate::domain::errors::DomainResult;
use crate::domain::models::adapter::{
    AdapterManifest, EgressAction, EgressResult, IngestionItem,
};

/// Port for adapters that pull work items from an external system.
///
/// Ingestion adapters poll an external source (issue tracker, queue, etc.)
/// and return normalized [`IngestionItem`]s that the swarm converts into
/// tasks. The `last_poll` parameter enables incremental polling — adapters
/// should return only items updated since that timestamp.
#[async_trait]
pub trait IngestionAdapter: Send + Sync {
    /// Returns this adapter's manifest describing its identity and capabilities.
    fn manifest(&self) -> &AdapterManifest;

    /// Poll the external system for new or updated items.
    ///
    /// If `last_poll` is `Some`, the adapter should only return items
    /// that have been created or updated since that timestamp. If `None`,
    /// the adapter should perform a full initial sync.
    async fn poll(&self, last_poll: Option<DateTime<Utc>>) -> DomainResult<Vec<IngestionItem>>;
}

/// Port for adapters that push results to an external system.
///
/// Egress adapters execute actions against an external system (e.g.,
/// updating a ticket's status, posting a comment, attaching artifacts).
/// Each action is represented by an [`EgressAction`] variant.
#[async_trait]
pub trait EgressAdapter: Send + Sync {
    /// Returns this adapter's manifest describing its identity and capabilities.
    fn manifest(&self) -> &AdapterManifest;

    /// Execute an egress action against the external system.
    ///
    /// Returns an [`EgressResult`] indicating whether the action succeeded,
    /// along with any external identifiers or URLs for the affected resource.
    async fn execute(&self, action: &EgressAction) -> DomainResult<EgressResult>;
}
