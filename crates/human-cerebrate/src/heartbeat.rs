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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clickup::client::ClickUpApi;
    use crate::clickup::models::*;
    use crate::config::*;
    use crate::server::AppState;
    use abathur::services::federation::service::FederationHttpClient;
    use anyhow::Result;
    use async_trait::async_trait;
    use sqlx::sqlite::SqliteConnectOptions;
    use std::str::FromStr;
    use std::time::Duration;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    struct DummyClickUp;

    #[async_trait]
    impl ClickUpApi for DummyClickUp {
        async fn create_task(&self, _list_id: &str, _req: &CreateTaskRequest) -> Result<CreateTaskResponse> {
            unreachable!("heartbeat tests do not exercise create_task")
        }
        async fn get_task(&self, _task_id: &str) -> Result<Option<ClickUpTask>> {
            Ok(None)
        }
        async fn get_comments(&self, _task_id: &str) -> Result<Vec<ClickUpComment>> {
            Ok(vec![])
        }
    }

    fn cfg(overmind_url: String, interval_secs: u64) -> Config {
        Config {
            server: ServerConfig {
                bind_address: "127.0.0.1".to_string(),
                port: 0,
            },
            identity: IdentityConfig {
                cerebrate_id: "test-cerebrate".to_string(),
                display_name: "Test Human".to_string(),
                capabilities: vec!["real-world".to_string()],
                max_concurrent_tasks: 5,
            },
            parent: ParentConfig {
                overmind_url,
                heartbeat_interval_secs: interval_secs,
            },
            clickup: ClickUpConfig {
                workspace_id: "ws".to_string(),
                list_id: "list".to_string(),
                completed_statuses: vec!["complete".to_string()],
                failed_statuses: vec!["cancelled".to_string()],
            },
            polling: PollingConfig {
                interval_secs: 60,
                task_deadline_secs: 1209600,
                progress_interval_secs: 900,
            },
            database: DatabaseConfig { path: ":memory:".to_string() },
            tls: TlsConfig::default(),
        }
    }

    async fn build_state(overmind_url: String, interval_secs: u64) -> Arc<AppState> {
        let db = sqlx::SqlitePool::connect_with(
            SqliteConnectOptions::from_str(":memory:")
                .unwrap()
                .create_if_missing(true),
        )
        .await
        .unwrap();
        crate::state::run_migrations(&db).await.unwrap();
        Arc::new(AppState {
            config: cfg(overmind_url, interval_secs),
            db,
            clickup: Arc::new(DummyClickUp),
            federation_client: FederationHttpClient::new(),
        })
    }

    /// Heartbeat fires repeatedly at the configured cadence. With a 1-second
    /// interval, tokio::time::interval ticks immediately at t=0 then at
    /// t=1s, so within ~1.5s the mock should observe at least 2 hits.
    #[tokio::test]
    async fn heartbeat_fires_at_configured_interval() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/federation/heartbeat"))
            .respond_with(ResponseTemplate::new(200).set_body_string("{}"))
            .mount(&server)
            .await;

        let state = build_state(server.uri(), 1).await;
        let (tx, rx) = tokio::sync::broadcast::channel(1);
        let handle = tokio::spawn(run_heartbeat(state, rx));

        // Wait long enough to capture the immediate first tick + one more.
        tokio::time::sleep(Duration::from_millis(1300)).await;
        let _ = tx.send(());
        let _ = handle.await;

        let received = server.received_requests().await.unwrap();
        assert!(
            received.len() >= 2,
            "expected at least 2 heartbeats within 1.3s at 1s interval, got {}",
            received.len()
        );
    }

    /// Heartbeat send failures are logged but the loop must continue running
    /// — i.e. subsequent ticks still happen. We point the client at a
    /// closed/non-routable URL by stopping the mock immediately, then verify
    /// the spawned task is still alive a few ticks later.
    #[tokio::test]
    async fn heartbeat_failure_does_not_terminate_loop() {
        // Use a URL that will reliably fail to connect: an unreachable port.
        // reqwest will return a connect error, which send_heartbeat surfaces
        // as Err — the loop must merely log and continue.
        let bad_url = "http://127.0.0.1:1".to_string();
        let state = build_state(bad_url, 1).await;
        let (tx, rx) = tokio::sync::broadcast::channel(1);
        let handle = tokio::spawn(run_heartbeat(state, rx));

        // Let several ticks fail.
        tokio::time::sleep(Duration::from_millis(1500)).await;
        assert!(!handle.is_finished(), "heartbeat loop must not exit on send error");

        // Shutdown cleanly to confirm the loop is still responsive.
        let _ = tx.send(());
        let res = tokio::time::timeout(Duration::from_secs(2), handle).await;
        assert!(res.is_ok(), "loop should respond to shutdown after errors");
    }
}
