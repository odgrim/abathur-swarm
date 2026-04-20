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

use crate::domain::models::{Goal, HumanEscalationEvent};

use super::types::{OrchestratorStatus, SwarmStats};

/// Runtime state of a running swarm: status, counters, caches, and the
/// channels that primed-spawn signalling rides on. Generic-free.
pub(crate) struct RuntimeState {
    pub(crate) status: Arc<RwLock<OrchestratorStatus>>,
    pub(crate)stats: Arc<RwLock<SwarmStats>>,
    pub(crate)agent_semaphore: Arc<Semaphore>,
    pub(crate)total_tokens: Arc<AtomicU64>,
    pub(crate)active_goals_cache: Arc<RwLock<Vec<Goal>>>,
    pub(crate)escalation_store: Arc<RwLock<HashMap<Uuid, HumanEscalationEvent>>>,

    pub(crate)ready_task_rx: Arc<Mutex<mpsc::Receiver<Uuid>>>,
    pub(crate)ready_task_tx: mpsc::Sender<Uuid>,
    pub(crate)specialist_rx: Arc<Mutex<mpsc::Receiver<Uuid>>>,
    pub(crate)specialist_tx: mpsc::Sender<Uuid>,
}

impl RuntimeState {
    /// Construct fresh runtime state for an orchestrator that hasn't started
    /// yet. Capacities match the historical `mod.rs` constants (256 ready
    /// tasks, 64 specialist).
    pub(crate) fn new(max_agents: usize) -> Self {
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
    pub(crate) async fn status(&self) -> OrchestratorStatus {
        self.status.read().await.clone()
    }

    /// Get current stats snapshot.
    pub(crate) async fn stats(&self) -> SwarmStats {
        self.stats.read().await.clone()
    }

    /// Pause the orchestrator (no-op unless currently Running).
    pub(crate) async fn pause(&self) {
        let mut status = self.status.write().await;
        if *status == OrchestratorStatus::Running {
            *status = OrchestratorStatus::Paused;
        }
    }

    /// Resume the orchestrator (no-op unless currently Paused).
    pub(crate) async fn resume(&self) {
        let mut status = self.status.write().await;
        if *status == OrchestratorStatus::Paused {
            *status = OrchestratorStatus::Running;
        }
    }

    /// Stop the orchestrator gracefully (transition to ShuttingDown).
    pub(crate) async fn stop(&self) {
        let mut status = self.status.write().await;
        *status = OrchestratorStatus::ShuttingDown;
    }

    /// Read the running total token count.
    pub(crate) fn total_tokens(&self) -> u64 {
        self.total_tokens.load(Ordering::Relaxed)
    }

    // The active-goals cache mutator (`refresh_active_goals_cache`) lives on
    // `SwarmOrchestrator` (in `agent_lifecycle.rs`) because it needs the
    // goal repository, which is owned by `CoreDeps`. It writes through
    // `self.runtime_state.active_goals_cache`.
}
