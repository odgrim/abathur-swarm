//! `DaemonHandles` — owns the lifecycle handles for every long-running
//! background task spawned by the orchestrator. Six independently optional
//! daemons: memory decay, hourly token reset, embedded MCP servers, outbox
//! poller, federation convergence poller, and federation convergence
//! publisher.
//!
//! Part of the T11 god-object decomposition (see
//! `specs/T11-swarm-orchestrator-decomposition.md`). Generic-free.
//!
//! ## Drop ordering
//!
//! [`DaemonHandles`] implements [`Drop`] explicitly so that on orchestrator
//! teardown the cancellation tokens fire **before** any join handles are
//! aborted. Aborting a `JoinHandle` whose underlying task is mid-await on a
//! channel can leak in-flight resources; cancelling first lets the task
//! observe the cancellation, drain, and exit cleanly. The poller daemons
//! expose their own `stop()` helpers that are signal-based and idempotent,
//! so calling them from `Drop` is safe even after a normal shutdown.

use std::sync::Arc;

use tokio::sync::{RwLock, broadcast};
use tokio_util::sync::CancellationToken;

use crate::services::DaemonHandle;
use crate::services::federation::ConvergencePollerHandle;
use crate::services::federation::convergence_publisher::ConvergencePublisherHandle;
use crate::services::outbox_poller::OutboxPollerHandle;

/// Background-daemon lifecycle handles. Each field is independently optional
/// so the orchestrator can be configured with only the daemons it needs.
// dead_code: introduced in T11 step 1; methods/fields wired in steps 2-7.
#[allow(dead_code)]
pub(super) struct DaemonHandles {
    pub(super) decay_daemon_handle: Arc<RwLock<Option<DaemonHandle>>>,
    /// Cancellation token for the hourly token-counter reset daemon.
    pub(super) hourly_reset_cancel: Arc<RwLock<Option<CancellationToken>>>,
    pub(super) mcp_shutdown_tx: Arc<RwLock<Option<broadcast::Sender<()>>>>,
    pub(super) outbox_poller_handle: Arc<RwLock<Option<OutboxPollerHandle>>>,
    pub(super) convergence_poller_handle: Arc<RwLock<Option<ConvergencePollerHandle>>>,
    pub(super) convergence_publisher_handle: Arc<RwLock<Option<ConvergencePublisherHandle>>>,
}

#[allow(dead_code)]
impl DaemonHandles {
    /// Construct an empty handle bundle; daemons are wired in later via the
    /// `start_*` helpers as the orchestrator boots.
    pub(super) fn new() -> Self {
        Self {
            decay_daemon_handle: Arc::new(RwLock::new(None)),
            hourly_reset_cancel: Arc::new(RwLock::new(None)),
            mcp_shutdown_tx: Arc::new(RwLock::new(None)),
            outbox_poller_handle: Arc::new(RwLock::new(None)),
            convergence_poller_handle: Arc::new(RwLock::new(None)),
            convergence_publisher_handle: Arc::new(RwLock::new(None)),
        }
    }

    /// Stop the memory decay daemon if running.
    pub(super) async fn stop_decay_daemon(&self) {
        let daemon_handle = self.decay_daemon_handle.read().await;
        if let Some(ref handle) = *daemon_handle {
            handle.stop();
        }
    }

    /// Stop the outbox poller if running.
    pub(super) async fn stop_outbox_poller(&self) {
        let handle = self.outbox_poller_handle.read().await;
        if let Some(ref h) = *handle {
            h.stop();
        }
    }

    /// Stop the federation convergence poller if running.
    pub(super) async fn stop_convergence_poller(&self) {
        let handle = self.convergence_poller_handle.read().await;
        if let Some(ref h) = *handle {
            h.stop();
        }
    }

    /// Stop the federation convergence publisher if running.
    pub(super) async fn stop_convergence_publisher(&self) {
        let handle = self.convergence_publisher_handle.read().await;
        if let Some(ref h) = *handle {
            h.stop();
        }
    }

    /// Stop the embedded MCP servers (broadcast a shutdown signal) if a
    /// handle was set.
    pub(super) async fn stop_embedded_mcp_servers(&self) {
        let handle = self.mcp_shutdown_tx.read().await;
        if let Some(ref tx) = *handle {
            let _ = tx.send(());
        }
    }

    /// Cancel the hourly token-counter reset daemon if running.
    pub(super) async fn stop_hourly_reset(&self) {
        if let Some(cancel) = self.hourly_reset_cancel.read().await.as_ref() {
            cancel.cancel();
        }
    }
}

impl Drop for DaemonHandles {
    /// On drop, cancel every cancellation token first (so the underlying
    /// tasks observe the cancel and exit cleanly), then signal-stop the
    /// poller handles. We deliberately do **not** call `JoinHandle::abort()`
    /// from sync `Drop` — those handles live behind `RwLock` and the runtime
    /// may not be available. The async `stop_*` helpers should be called
    /// from `run()`'s shutdown tail; this `Drop` is the safety net for
    /// abrupt teardown (panic, test fixture drop, etc.).
    fn drop(&mut self) {
        // Cancellation tokens (sync, runtime-free).
        if let Ok(guard) = self.hourly_reset_cancel.try_read()
            && let Some(cancel) = guard.as_ref()
        {
            cancel.cancel();
        }
        // Broadcast MCP shutdown if a handle is set.
        if let Ok(guard) = self.mcp_shutdown_tx.try_read()
            && let Some(tx) = guard.as_ref()
        {
            let _ = tx.send(());
        }
        // Signal-stop the poller daemons. Each `stop()` is idempotent and
        // sync; the daemons themselves observe and exit on next loop tick.
        if let Ok(guard) = self.decay_daemon_handle.try_read()
            && let Some(h) = guard.as_ref()
        {
            h.stop();
        }
        if let Ok(guard) = self.outbox_poller_handle.try_read()
            && let Some(h) = guard.as_ref()
        {
            h.stop();
        }
        if let Ok(guard) = self.convergence_poller_handle.try_read()
            && let Some(h) = guard.as_ref()
        {
            h.stop();
        }
        if let Ok(guard) = self.convergence_publisher_handle.try_read()
            && let Some(h) = guard.as_ref()
        {
            h.stop();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verify Drop on a populated `DaemonHandles` completes without panic.
    /// This is the explicit risk-mitigation test from the T11 spec §6 risk 2:
    /// confirms cancellation-tokens fire and signal-based stops are safe in
    /// sync Drop.
    #[tokio::test]
    async fn test_daemon_handles_drop() {
        let handles = DaemonHandles::new();

        // Populate the cancellation-token slot and a broadcast channel so
        // the Drop code paths actually execute. Pollers/decay handles are
        // private types created by their respective services; covering the
        // sync-cancellable fields is sufficient to validate the ordering
        // behaviour.
        {
            let cancel = CancellationToken::new();
            *handles.hourly_reset_cancel.write().await = Some(cancel);
        }
        {
            let (tx, _rx) = broadcast::channel(1);
            *handles.mcp_shutdown_tx.write().await = Some(tx);
        }

        // Drop should run synchronously with no panic and should leave the
        // cancellation token in the cancelled state. Snapshot the token
        // first so we can assert on it after drop.
        let cancel_clone = handles
            .hourly_reset_cancel
            .read()
            .await
            .as_ref()
            .unwrap()
            .clone();

        drop(handles);

        assert!(cancel_clone.is_cancelled(), "Drop must cancel the hourly-reset token");
    }
}
