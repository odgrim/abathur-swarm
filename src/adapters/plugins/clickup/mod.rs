//! ClickUp native adapter.
//!
//! Provides bidirectional integration with the ClickUp project management
//! platform. Ingestion polls tasks from a configurable list; egress supports
//! status updates, comments, and task creation.

pub mod client;
pub mod egress;
pub mod ingestion;
pub mod models;
