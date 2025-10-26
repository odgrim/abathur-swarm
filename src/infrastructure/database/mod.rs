//! Database infrastructure layer
//!
//! This module provides SQLite database connectivity with:
//! - Connection pooling with sqlx
//! - WAL mode for concurrent access
//! - Automatic migrations
//! - Repository implementations (to be added)

pub mod connection;

pub use connection::DatabaseConnection;
