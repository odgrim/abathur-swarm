//! Abathur - Agentic Swarm Orchestrator
//!
//! Abathur is a task queue and swarm orchestration system for managing AI agents
//! with hierarchical memory, MCP integration, and priority-based task scheduling.
//!
//! # Architecture
//!
//! This crate follows Clean Architecture / Hexagonal Architecture principles:
//!
//! - **Domain Layer** (`domain`): Pure business logic and domain models
//! - **Application Layer** (`application`): Use case orchestration and workflows
//! - **Service Layer** (`services`): Business logic coordination
//! - **Infrastructure Layer** (`infrastructure`): External integrations and adapters
//! - **CLI Layer** (`cli`): Command-line interface
//!
//! # Example
//!
//! ```ignore
//! use abathur::application::Orchestrator;
//!
//! #[tokio::main]
//! async fn main() -> anyhow::Result<()> {
//!     // Initialize and run orchestrator
//!     Ok(())
//! }
//! ```

pub mod cli;
pub mod application;
pub mod domain;
pub mod infrastructure;
pub mod services;
