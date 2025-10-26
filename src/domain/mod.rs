//! Domain layer module
//!
//! This module contains the core business logic and domain models:
//! - Domain models (Task, Agent, Queue, Memory, Session)
//! - Port trait definitions (repository interfaces, client interfaces)
//!
//! This layer is framework-agnostic and contains no infrastructure dependencies.

pub mod error;
pub mod models;
pub mod ports;

// Re-export error types for convenient access
pub use error::{DomainError, TaskError};
pub mod models;
pub mod ports;
pub mod models;
pub mod ports;
