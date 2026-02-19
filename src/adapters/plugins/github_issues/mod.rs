//! GitHub Issues native adapter.
//!
//! Provides bidirectional integration with GitHub repository issues.
//! Ingestion polls issues from a configurable repository; egress supports
//! state updates (open/close), comments, issue creation, and pull request
//! creation via a custom action.

pub mod client;
pub mod egress;
pub mod ingestion;
pub mod models;
