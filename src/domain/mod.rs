//! Domain layer module
//!
//! This module contains the core business logic and domain models:
//! - Domain models (Task, Agent, Queue, Memory, Session)
//! - Port trait definitions (repository interfaces, client interfaces)
//!
//! This layer is framework-agnostic and contains no infrastructure dependencies.

pub mod models;
pub mod ports;
