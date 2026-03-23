//! A2A (Agent2Agent) protocol adapter.
//!
//! Provides an HTTP client implementing the standard A2A wire protocol
//! for swarm-to-swarm communication.

pub mod client;

pub use client::{A2AClient, A2AWireError, HttpA2AClient};
