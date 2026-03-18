//! Domain layer for the Abathur swarm system.

pub mod errors;
pub mod models;
pub mod ports;

pub use errors::{DbErrorCategory, DomainError, DomainResult};
