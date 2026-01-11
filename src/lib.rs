//! Abathur - Self-evolving agentic swarm orchestrator.

pub mod adapters;
pub mod cli;
pub mod domain;
pub mod services;

pub use domain::{DomainError, DomainResult};
pub use services::{Config, ConfigError};
