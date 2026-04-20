//! `CoreDeps` — immutable repository references and configuration that every
//! orchestrator subsystem touches.
//!
//! Part of the T11 god-object decomposition (see
//! `specs/T11-swarm-orchestrator-decomposition.md`). Holds the five repository
//! handles, the substrate trait object, and the static `SwarmConfig`. Pure
//! container — no methods.

use std::sync::Arc;

use crate::domain::ports::{
    AgentRepository, GoalRepository, Substrate, TaskRepository, WorktreeRepository,
};

use super::types::SwarmConfig;

/// Core dependencies (required, immutable) shared across all orchestrator
/// subsystems. Field access is intentional and direct (no accessors); these
/// fields are read constantly and adding accessors would add noise without
/// adding safety.
// dead_code: introduced in T11 step 1; field access lands in step 7.
#[allow(dead_code)]
pub(super) struct CoreDeps<G, T, W, A>
where
    G: GoalRepository + 'static,
    T: TaskRepository + 'static,
    W: WorktreeRepository + 'static,
    A: AgentRepository + 'static,
{
    pub(super) goal_repo: Arc<G>,
    pub(super) task_repo: Arc<T>,
    pub(super) worktree_repo: Arc<W>,
    pub(super) agent_repo: Arc<A>,
    pub(super) substrate: Arc<dyn Substrate>,
    pub(super) config: SwarmConfig,
}
