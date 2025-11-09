//! Database infrastructure layer
//!
//! This module provides `SQLite` database connectivity with:
//! - Connection pooling with sqlx
//! - WAL mode for concurrent access
//! - Automatic migrations
//! - Repository implementations
//! - SQLite extension registration (sqlite-vec)

pub mod agent_repo;
pub mod chain_repo;
pub mod connection;
pub mod errors;
pub mod extensions;
pub mod memory_repo;
pub mod session_repo;
pub mod task_repo;
pub mod utils;

pub use agent_repo::AgentRepositoryImpl;
pub use chain_repo::ChainRepositoryImpl;
pub use connection::DatabaseConnection;
pub use errors::DatabaseError;
pub use extensions::register_sqlite_vec;
pub use memory_repo::MemoryRepositoryImpl;
pub use session_repo::SessionRepositoryImpl;
pub use task_repo::TaskRepositoryImpl;
