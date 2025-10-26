//! Configuration management infrastructure
//!
//! Hierarchical configuration using figment:
//! - YAML file loading
//! - Environment variable overrides
//! - Configuration validation
//! - Type-safe config structs

pub mod loader;

pub use loader::{ConfigError, ConfigLoader};
