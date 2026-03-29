//! Repository port for quiet window persistence.

use async_trait::async_trait;
use uuid::Uuid;

use crate::domain::errors::DomainResult;
use crate::domain::models::quiet_window::{QuietWindow, QuietWindowStatus};

/// Filter for listing quiet windows.
#[derive(Debug, Default)]
pub struct QuietWindowFilter {
    pub status: Option<QuietWindowStatus>,
}

#[async_trait]
pub trait QuietWindowRepository: Send + Sync {
    /// Create a new quiet window.
    async fn create(&self, window: &QuietWindow) -> DomainResult<()>;

    /// Get a quiet window by ID.
    async fn get(&self, id: Uuid) -> DomainResult<Option<QuietWindow>>;

    /// Get a quiet window by name.
    async fn get_by_name(&self, name: &str) -> DomainResult<Option<QuietWindow>>;

    /// Update an existing quiet window.
    async fn update(&self, window: &QuietWindow) -> DomainResult<()>;

    /// Delete a quiet window by ID.
    async fn delete(&self, id: Uuid) -> DomainResult<()>;

    /// List quiet windows with optional filter.
    async fn list(&self, filter: QuietWindowFilter) -> DomainResult<Vec<QuietWindow>>;

    /// List all enabled quiet windows.
    async fn list_enabled(&self) -> DomainResult<Vec<QuietWindow>>;
}
