//! `Middleware` — pre-spawn and post-completion middleware chains. Wraps the
//! two `RwLock<Chain>` fields previously inlined on the orchestrator and
//! exposes the two builder helpers that register additional middleware.
//!
//! Part of the T11 god-object decomposition (see
//! `specs/T11-swarm-orchestrator-decomposition.md`). Generic-free.
//!
//! Note: the existing module `middleware` (`middleware.rs` / `middleware/`)
//! holds the trait definitions and built-in implementations. This new file
//! is named `middleware_bundle.rs` to avoid colliding with that module —
//! the bundle is a thin holder for the chain instances.

use std::sync::Arc;

use tokio::sync::RwLock;

use super::middleware::{
    PostCompletionChain, PostCompletionMiddleware, PreSpawnChain, PreSpawnMiddleware,
};

/// The pair of registration chains the orchestrator runs around every task
/// spawn. Both fields are populated with built-in middleware in
/// `register_builtin_middleware()` during `run()`; external callers may
/// register additional middleware via the `register_*` helpers below.
pub(crate) struct Middleware {
    pub(crate) pre_spawn_chain: Arc<RwLock<PreSpawnChain>>,
    pub(crate) post_completion_chain: Arc<RwLock<PostCompletionChain>>,
}

impl Middleware {
    /// Construct empty chains. Built-in middleware is registered later in
    /// `SwarmOrchestrator::register_builtin_middleware()`.
    pub(crate) fn new() -> Self {
        Self {
            pre_spawn_chain: Arc::new(RwLock::new(PreSpawnChain::new())),
            post_completion_chain: Arc::new(RwLock::new(PostCompletionChain::new())),
        }
    }

    /// Register an additional pre-spawn middleware. Registration order is
    /// preserved.
    pub(crate) async fn register_pre_spawn(&self, mw: Arc<dyn PreSpawnMiddleware>) {
        self.pre_spawn_chain.write().await.register(mw);
    }

    /// Register an additional post-completion middleware. Registration order
    /// is preserved.
    pub(crate) async fn register_post_completion(&self, mw: Arc<dyn PostCompletionMiddleware>) {
        self.post_completion_chain.write().await.register(mw);
    }
}
