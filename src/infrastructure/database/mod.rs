//! Database infrastructure layer
//!
//! This module provides `SQLite` database connectivity with:
//! - Connection pooling with sqlx
//! - WAL mode for concurrent access
//! - Automatic migrations
//! - Repository implementations

pub mod agent_repo;
pub mod connection;
pub mod errors;
pub mod memory_repo;
pub mod session_repo;
pub mod task_repo;

pub use agent_repo::AgentRepositoryImpl;
pub use connection::DatabaseConnection;
pub use errors::DatabaseError;
pub use memory_repo::MemoryRepositoryImpl;
pub use session_repo::SessionRepositoryImpl;
pub use task_repo::TaskRepositoryImpl;
