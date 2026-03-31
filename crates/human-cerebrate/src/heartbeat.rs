//! Background heartbeat loop that keeps the cerebrate connected in the overmind.

use std::sync::Arc;

use tracing::{info, warn};

use crate::server::AppState;
use crate::state;

/// Run the background heartbeat loop.
pub async fn run_heartbeat(
    state: Arc<AppState>,
    mut shutdown: tokio::sync::broadcast::Receiver<()>,
) {
    let interval_secs = state.config.parent.heartbeat_interval_secs;
    let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(interval_secs));

    info!("Heartbeat started, interval = {interval_secs}s");

    loop {
        tokio::select! {
            _ = interval.tick() => {},
            _ = shutdown.recv() => {
                info!("Heartbeat shutting down");
                return;
            }
        }

        let load = match state::count_active(&state.db).await {
            Ok(count) => count as f64 / state.config.identity.max_concurrent_tasks.max(1) as f64,
            Err(e) => {
                warn!("Failed to count active tasks for heartbeat: {e}");
                0.0
            }
        };

        if let Err(e) = state
            .federation_client
            .send_heartbeat(
                &state.config.parent.overmind_url,
                &state.config.identity.cerebrate_id,
                load,
            )
            .await
        {
            warn!("Heartbeat failed: {e}");
        }
    }
}
