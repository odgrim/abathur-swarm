//! Federation module for cross-swarm task delegation.
//!
//! Federation enables a parent swarm (Overmind) to delegate tasks to child swarms
//! (Cerebrates) over the network, receive structured results back, and reactively
//! create follow-up work. The topology is a recursive tree — cerebrates can have
//! their own cerebrates.
//!
//! This module provides:
//! - Extension traits with pluggable strategies for delegation, result processing,
//!   task transformation, and result schema validation.
//! - Default implementations for each trait.
//! - The `FederationService` that manages cerebrate connections, heartbeats,
//!   task delegation, and result ingestion.
//! - Configuration types for the `[federation]` TOML section.

pub mod config;
pub mod convergence_poller;
pub mod convergence_publisher;
pub mod dag_handler;
pub mod handler;
pub mod service;
pub mod swarm_dag_executor;
pub mod traits;

pub use config::{
    CerebrateConfig, FederationConfig, FederationParentConfig,
    FederationRole as FederationConfigRole, FederationTlsConfig,
};
pub use convergence_poller::{
    ConvergencePollerConfig, ConvergencePollerHandle, ConvergencePollingDaemon,
};
pub use convergence_publisher::ConvergencePublisher;
pub use dag_handler::SwarmDagEventHandler;
pub use handler::FederationResultHandler;
pub use service::{FederationHttpClient, FederationService};
pub use swarm_dag_executor::SwarmDagExecutor;
pub use traits::{
    DefaultDelegationStrategy, DefaultResultProcessor, DefaultTaskTransformer, DelegationDecision,
    FederationDelegationStrategy, FederationReaction, FederationResultProcessor,
    FederationTaskTransformer, ResultSchema, StandardV1Schema,
};
