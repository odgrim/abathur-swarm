//! Database infrastructure layer
//!
//! This module provides SQLite database connectivity with:
//! - Connection pooling with sqlx
//! - WAL mode for concurrent access
//! - Automatic migrations
//! - Repository implementations

pub mod connection;
pub mod memory_repo;

pub use connection::DatabaseConnection;
pub use memory_repo::MemoryRepositoryImpl;
