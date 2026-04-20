//! `RuntimeState` — mutable runtime counters, caches, and channels needed by
//! the orchestrator main loop.
//!
//! Part of the T11 god-object decomposition (see
//! `specs/T11-swarm-orchestrator-decomposition.md`). Holds the status, stats,
//! semaphore, atomics, caches, escalation store, and the ready-task /
//! specialist mpsc channels.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use tokio::sync::{Mutex, RwLock, Semaphore, mpsc};
use uuid::Uuid;

use crate::domain::errors::DomainResult;
use crate::domain::models::{Goal, HumanEscalationEvent};

use super::types::{OrchestratorStatus, SwarmStats};

/// Runtime state of a running swarm: status, counters, caches, and the
/// channels that primed-spawn signalling rides on. Generic-free.
// dead_code: introduced in T11 step 1; methods/fields wired in steps 2-7.
#[allow(dead_code)]
pub(super) struct RuntimeState {
    pub(super) status: Arc<RwLock<OrchestratorStatus>>,
    pub(super) stats: Arc<RwLock<SwarmStats>>,
    pub(super) agent_semaphore: Arc<Semaphore>,
    pub(super) total_tokens: Arc<AtomicU64>,
    pub(super) active_goals_cache: Arc<RwLock<Vec<Goal>>>,
    pub(super) escalation_store: Arc<RwLock<HashMap<Uuid, HumanEscalationEvent>>>,

    pub(super) ready_task_rx: Arc<Mutex<mpsc::Receiver<Uuid>>>,
    pub(super) ready_task_tx: mpsc::Sender<Uuid>,
    pub(super) specialist_rx: Arc<Mutex<mpsc::Receiver<Uuid>>>,
    pub(super) specialist_tx: mpsc::Sender<Uuid>,
}

#[allow(dead_code)]
impl RuntimeState {
    /// Construct fresh runtime state for an orchestrator that hasn't started
    /// yet. Capacities match the historical `mod.rs` constants (256 ready
    /// tasks, 64 specialist).
    pub(super) fn new(max_agents: usize) -> Self {
        let (ready_tx, ready_rx) = mpsc::channel(256);
        let (specialist_tx, specialist_rx) = mpsc::channel(64);
        Self {
            status: Arc::new(RwLock::new(OrchestratorStatus::Idle)),
            stats: Arc::new(RwLock::new(SwarmStats::default())),
            agent_semaphore: Arc::new(Semaphore::new(max_agents)),
            total_tokens: Arc::new(AtomicU64::new(0)),
            active_goals_cache: Arc::new(RwLock::new(Vec::new())),
            escalation_store: Arc::new(RwLock::new(HashMap::new())),
            ready_task_rx: Arc::new(Mutex::new(ready_rx)),
            ready_task_tx: ready_tx,
            specialist_rx: Arc::new(Mutex::new(specialist_rx)),
            specialist_tx,
        }
    }

    /// Get current status.
    pub(super) async fn status(&self) -> OrchestratorStatus {
        self.status.read().await.clone()
    }

    /// Get current stats snapshot.
    pub(super) async fn stats(&self) -> SwarmStats {
        self.stats.read().await.clone()
    }

    /// Pause the orchestrator (no-op unless currently Running).
    pub(super) async fn pause(&self) {
        let mut status = self.status.write().await;
        if *status == OrchestratorStatus::Running {
            *status = OrchestratorStatus::Paused;
        }
    }

    /// Resume the orchestrator (no-op unless currently Paused).
    pub(super) async fn resume(&self) {
        let mut status = self.status.write().await;
        if *status == OrchestratorStatus::Paused {
            *status = OrchestratorStatus::Running;
        }
    }

    /// Stop the orchestrator gracefully (transition to ShuttingDown).
    pub(super) async fn stop(&self) {
        let mut status = self.status.write().await;
        *status = OrchestratorStatus::ShuttingDown;
    }

    /// Read the running total token count.
    pub(super) fn total_tokens(&self) -> u64 {
        self.total_tokens.load(Ordering::Relaxed)
    }

    /// Refresh the active-goals cache from a goal repository.
    ///
    /// Generic over the goal repository so this method can be called from
    /// `RuntimeState` directly without dragging the orchestrator's repo
    /// generics onto `RuntimeState` itself.
    pub(super) async fn refresh_active_goals_cache<G>(&self, goal_repo: &G) -> DomainResult<()>
    where
        G: crate::domain::ports::GoalRepository + ?Sized,
    {
        use crate::domain::models::GoalStatus;
        use crate::domain::ports::GoalFilter;

        let active_goals = goal_repo
            .list(GoalFilter {
                status: Some(GoalStatus::Active),
                ..Default::default()
            })
            .await?;

        let mut cache = self.active_goals_cache.write().await;
        *cache = active_goals;
        Ok(())
    }
}
