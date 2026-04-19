//! Shared supervision utilities for long-lived `tokio::spawn`ed tasks.
//!
//! Fire-and-forget spawns lose panic and error information — the spawned
//! future's panic is caught by the Tokio runtime but no diagnostic surfaces.
//! `supervise` wraps the spawn with a shim that `.await`s the `JoinHandle`
//! and logs clean exits, errors, and panics.

use std::future::Future;
use tokio::task::JoinHandle;

/// Spawn a long-lived daemon task with supervision (fire-and-forget).
///
/// Wraps `tokio::spawn` with a shim that awaits the `JoinHandle` and logs:
/// - `info!` on clean exit (generally unexpected for a long-lived task)
/// - `error!` on panic (with the payload if available)
/// - `error!` on other abnormal termination (cancellation, runtime shutdown)
///
/// Returns `()` so callers don't need `let _ = ` at every site. Use
/// [`supervise_with_handle`] if you need the supervision shim's `JoinHandle`.
///
/// # Arguments
/// - `daemon_name`: Stable identifier used in log messages; short and
///   specific like `"federation_heartbeat"` or `"outbox_poller"`.
/// - `fut`: The future to run in the supervised task.
///
/// # Example
/// ```ignore
/// supervise("outbox_poller", async move {
///     loop {
///         self.tick().await;
///         tokio::time::sleep(interval).await;
///     }
/// });
/// ```
pub fn supervise<F>(daemon_name: &'static str, fut: F)
where
    F: Future<Output = ()> + Send + 'static,
{
    drop(supervise_with_handle(daemon_name, fut));
}

/// Like [`supervise`], but returns the supervision shim's `JoinHandle` so the
/// caller can `.await` it to block until the inner task exits. The inner task's
/// output is not recoverable via this handle — the shim returns `()` after
/// logging; use a channel if you need the inner task's return value.
pub fn supervise_with_handle<F>(daemon_name: &'static str, fut: F) -> JoinHandle<()>
where
    F: Future<Output = ()> + Send + 'static,
{
    let inner = tokio::spawn(fut);
    tokio::spawn(async move {
        match inner.await {
            Ok(()) => tracing::info!(
                daemon = daemon_name,
                "daemon exited cleanly — unexpected for long-lived task"
            ),
            Err(join_err) if join_err.is_panic() => {
                tracing::error!(
                    daemon = daemon_name,
                    panic = ?join_err.into_panic(),
                    "daemon panicked"
                );
            }
            Err(join_err) => tracing::error!(
                daemon = daemon_name,
                error = ?join_err,
                "daemon terminated abnormally"
            ),
        }
    })
}

/// Variant for futures that return a `Result`. Logs the `Err` case. Fire-and-forget.
pub fn supervise_result<F, E>(daemon_name: &'static str, fut: F)
where
    F: Future<Output = Result<(), E>> + Send + 'static,
    E: std::fmt::Debug + Send + 'static,
{
    let inner = tokio::spawn(fut);
    tokio::spawn(async move {
        match inner.await {
            Ok(Ok(())) => tracing::info!(
                daemon = daemon_name,
                "daemon exited cleanly — unexpected for long-lived task"
            ),
            Ok(Err(e)) => tracing::error!(
                daemon = daemon_name,
                error = ?e,
                "daemon exited with error"
            ),
            Err(join_err) if join_err.is_panic() => {
                tracing::error!(
                    daemon = daemon_name,
                    panic = ?join_err.into_panic(),
                    "daemon panicked"
                );
            }
            Err(join_err) => tracing::error!(
                daemon = daemon_name,
                error = ?join_err,
                "daemon terminated abnormally"
            ),
        }
    });
}
