//! In-memory caching layer for hot-path repository reads.
//!
//! Uses `moka` for TTL-based concurrent caching with write-through
//! invalidation. Wraps repository traits as decorators.

pub mod cached_agent_repository;

pub use cached_agent_repository::CachedAgentRepository;
