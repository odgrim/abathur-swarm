//! Service layer module
//!
//! This module contains business logic services that coordinate domain operations:
//! - Task queue service
//! - Dependency resolver
//! - Priority calculator
//! - Memory service
//! - Session service
//!
//! Services use domain models and port traits to implement business workflows.

pub mod dependency_resolver;
pub mod memory_service;

pub use dependency_resolver::DependencyResolver;
pub use memory_service::MemoryService;
