//! Background poller that checks ClickUp task statuses and sends results to the overmind.

use std::collections::HashMap;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use futures::stream::{self, StreamExt};
use tracing::{error, info, warn};
use uuid::Uuid;

use abathur::domain::models::a2a::FederationResult;

use crate::parser::parse_human_response;
use crate::server::AppState;
use crate::state::{self, TaskMapping};

/// Run the background poller loop.
pub async fn run_poller(
    state: Arc<AppState>,
    mut shutdown: tokio::sync::broadcast::Receiver<()>,
) {
    let interval_secs = state.config.polling.interval_secs;
    let progress_interval = chrono::Duration::seconds(state.config.polling.progress_interval_secs as i64);
    let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(interval_secs));
    let mut last_progress: HashMap<String, DateTime<Utc>> = HashMap::new();

    info!("Poller started, interval = {interval_secs}s");

    loop {
        tokio::select! {
            _ = interval.tick() => {},
            _ = shutdown.recv() => {
                info!("Poller shutting down");
                return;
            }
        }

        let mappings = match state::get_active_mappings(&state.db).await {
            Ok(m) => m,
            Err(e) => {
                error!("Failed to query active mappings: {e}");
                continue;
            }
        };

        if mappings.is_empty() {
            continue;
        }

        // Process mappings concurrently, bounded to 10
        let state_ref = &state;
        let results: Vec<(String, Option<PollAction>)> = stream::iter(mappings)
            .map(|mapping| async move {
                let action = poll_single_task(state_ref, &mapping).await;
                (mapping.federation_task_id.clone(), action)
            })
            .buffer_unordered(10)
            .collect()
            .await;

        let now = Utc::now();
        for (task_id, action) in results {
            match action {
                Some(PollAction::SendResult(result, new_status, clickup_status, human_response)) => {
                    // Update DB status
                    if let Err(e) = state::update_status(&state.db, &task_id, &new_status, &clickup_status).await {
                        error!("Failed to update status for {task_id}: {e}");
                    }
                    if let Some(resp) = &human_response
                        && let Err(e) = state::update_human_response(&state.db, &task_id, resp).await
                    {
                        error!("Failed to update human response for {task_id}: {e}");
                    }
                    // Send result to overmind
                    match state.federation_client.send_result(&state.config.parent.overmind_url, &result).await {
                        Ok(()) => {
                            info!("Sent result for task {task_id}");
                            if let Err(e) = state::mark_result_sent(&state.db, &task_id).await {
                                error!("Failed to mark result sent for {task_id}: {e}");
                            }
                            last_progress.remove(&task_id);
                        }
                        Err(e) => {
                            warn!("Failed to send result for {task_id}, will retry: {e}");
                        }
                    }
                }
                Some(PollAction::UpdateClickUpStatus(clickup_status)) => {
                    if let Err(e) = state::update_status(&state.db, &task_id, "pending", &clickup_status).await {
                        error!("Failed to update clickup_status for {task_id}: {e}");
                    }
                    // Send periodic progress if needed
                    let should_send = last_progress
                        .get(&task_id)
                        .map(|last| now - *last >= progress_interval)
                        .unwrap_or(true);
                    if should_send {
                        send_progress_update(&state, &task_id, &clickup_status).await;
                        last_progress.insert(task_id, now);
                    }
                }
                None => {
                    // No changes, but still check if we need progress
                    let should_send = last_progress
                        .get(&task_id)
                        .map(|last| now - *last >= progress_interval)
                        .unwrap_or(true);
                    if should_send {
                        send_progress_update(&state, &task_id, "").await;
                        last_progress.insert(task_id, now);
                    }
                }
            }
        }
    }
}

#[allow(clippy::large_enum_variant)]
enum PollAction {
    /// Send a result to the overmind: (result, new_status, clickup_status, human_response)
    SendResult(FederationResult, String, String, Option<String>),
    /// Just update the ClickUp status in our DB
    UpdateClickUpStatus(String),
}

async fn poll_single_task(state: &AppState, mapping: &TaskMapping) -> Option<PollAction> {
    let task_id: Uuid = mapping.federation_task_id.parse().ok()?;
    let correlation_id: Uuid = mapping.correlation_id.parse().ok()?;

    // Fetch task from ClickUp
    let clickup_task = match state.clickup.get_task(&mapping.clickup_task_id).await {
        Ok(Some(task)) => task,
        Ok(None) => {
            // Task deleted
            let result = FederationResult::failed(
                task_id,
                correlation_id,
                "ClickUp task deleted",
                "Task was deleted from ClickUp",
            );
            return Some(PollAction::SendResult(result, "failed".to_string(), "deleted".to_string(), None));
        }
        Err(e) => {
            warn!("Failed to fetch ClickUp task {}: {e}", mapping.clickup_task_id);
            return None;
        }
    };

    let clickup_status = clickup_task.status.status.to_lowercase();

    // Check completed
    if state.config.clickup.completed_statuses.iter().any(|s| s.eq_ignore_ascii_case(&clickup_status)) {
        let comments = state
            .clickup
            .get_comments(&mapping.clickup_task_id)
            .await
            .unwrap_or_default();

        let parsed = parse_human_response(&comments);
        let mut result = FederationResult::completed(task_id, correlation_id, &parsed.summary);
        for artifact in &parsed.artifacts {
            result = result.with_artifact(artifact.clone());
        }

        let human_response = comments
            .iter()
            .map(|c| c.comment_text.clone())
            .collect::<Vec<_>>()
            .join("\n---\n");

        return Some(PollAction::SendResult(
            result,
            "completed".to_string(),
            clickup_status,
            Some(human_response),
        ));
    }

    // Check failed
    if state.config.clickup.failed_statuses.iter().any(|s| s.eq_ignore_ascii_case(&clickup_status)) {
        let result = FederationResult::failed(
            task_id,
            correlation_id,
            "Task rejected in ClickUp",
            format!("Task marked as '{}' in ClickUp", clickup_status),
        );
        return Some(PollAction::SendResult(result, "failed".to_string(), clickup_status, None));
    }

    // Check deadline
    if let Ok(deadline) = chrono::DateTime::parse_from_rfc3339(&mapping.deadline_at)
        && Utc::now() > deadline
    {
        let result = FederationResult::failed(
            task_id,
            correlation_id,
            "Task timed out",
            format!("Timed out after {} seconds", state.config.polling.task_deadline_secs),
        );
        return Some(PollAction::SendResult(result, "failed".to_string(), clickup_status, None));
    }

    // Still active — update status if changed
    if clickup_status != mapping.clickup_status {
        Some(PollAction::UpdateClickUpStatus(clickup_status))
    } else {
        None
    }
}

async fn send_progress_update(state: &AppState, task_id: &str, clickup_status: &str) {
    let task_uuid: Uuid = match task_id.parse() {
        Ok(id) => id,
        Err(_) => return,
    };
    let status_info = if clickup_status.is_empty() {
        String::new()
    } else {
        format!(", current status: {clickup_status}")
    };
    if let Err(e) = state
        .federation_client
        .send_progress(
            &state.config.parent.overmind_url,
            task_uuid,
            &state.config.identity.cerebrate_id,
            "awaiting_human",
            0.0,
            &format!("Waiting for human to complete ClickUp task{status_info}"),
        )
        .await
    {
        warn!("Failed to send progress for {task_id}: {e}");
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
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::time::Duration;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    /// Mock ClickUp implementation that records call counts and returns
    /// configurable responses.
    struct ScriptedClickUp {
        get_task_calls: AtomicU32,
        get_comments_calls: AtomicU32,
        mode: ScriptedMode,
    }

    enum ScriptedMode {
        /// Always return Err on get_task.
        AlwaysError,
        /// Return a task with the given completed status; comments returned
        /// as a single text comment.
        Completed { status: String, comment_text: String },
    }

    #[async_trait]
    impl ClickUpApi for ScriptedClickUp {
        async fn create_task(&self, _list_id: &str, _req: &CreateTaskRequest) -> Result<CreateTaskResponse> {
            unreachable!("poller tests do not exercise create_task")
        }
        async fn get_task(&self, _task_id: &str) -> Result<Option<ClickUpTask>> {
            self.get_task_calls.fetch_add(1, Ordering::SeqCst);
            match &self.mode {
                ScriptedMode::AlwaysError => Err(anyhow::anyhow!("simulated clickup failure")),
                ScriptedMode::Completed { status, .. } => Ok(Some(ClickUpTask {
                    id: "cu_1".to_string(),
                    name: "task".to_string(),
                    status: ClickUpStatus { status: status.clone() },
                    date_created: "0".to_string(),
                    due_date: None,
                })),
            }
        }
        async fn get_comments(&self, _task_id: &str) -> Result<Vec<ClickUpComment>> {
            self.get_comments_calls.fetch_add(1, Ordering::SeqCst);
            match &self.mode {
                ScriptedMode::AlwaysError => Ok(vec![]),
                ScriptedMode::Completed { comment_text, .. } => Ok(vec![ClickUpComment {
                    id: "c1".to_string(),
                    comment_text: comment_text.clone(),
                    date: "0".to_string(),
                    user: ClickUpUser {
                        id: 1,
                        username: "tester".to_string(),
                    },
                }]),
            }
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
                heartbeat_interval_secs: 60,
            },
            clickup: ClickUpConfig {
                workspace_id: "ws".to_string(),
                list_id: "list".to_string(),
                completed_statuses: vec!["complete".to_string()],
                failed_statuses: vec!["cancelled".to_string()],
            },
            polling: PollingConfig {
                interval_secs,
                task_deadline_secs: 1209600,
                progress_interval_secs: 900,
            },
            database: DatabaseConfig { path: ":memory:".to_string() },
            tls: TlsConfig::default(),
        }
    }

    async fn setup_db() -> sqlx::SqlitePool {
        let db = sqlx::SqlitePool::connect_with(
            SqliteConnectOptions::from_str(":memory:")
                .unwrap()
                .create_if_missing(true),
        )
        .await
        .unwrap();
        crate::state::run_migrations(&db).await.unwrap();
        db
    }

    fn insert_active_mapping(
        db: &sqlx::SqlitePool,
        federation_task_id: Uuid,
        correlation_id: Uuid,
    ) {
        let now = Utc::now().to_rfc3339();
        let deadline = (Utc::now() + chrono::Duration::days(14)).to_rfc3339();
        let mapping = crate::state::TaskMapping {
            federation_task_id: federation_task_id.to_string(),
            correlation_id: correlation_id.to_string(),
            clickup_task_id: "cu_1".to_string(),
            title: "t".to_string(),
            status: "pending".to_string(),
            priority: "normal".to_string(),
            parent_goal_id: None,
            envelope_json: "{}".to_string(),
            clickup_status: "to do".to_string(),
            human_response: None,
            created_at: now.clone(),
            updated_at: now,
            deadline_at: deadline,
            result_sent: false,
        };
        // Block on the insert for ergonomics in tests.
        let db = db.clone();
        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async move {
                crate::state::insert_mapping(&db, &mapping).await.unwrap();
            })
        });
    }

    /// Empty queue: poll runs, finds no mappings, sleeps, repeats. We assert
    /// the loop survives multiple intervals and that the ClickUp mock is
    /// never invoked because get_active_mappings returns nothing.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn poll_empty_queue_sleeps_and_continues() {
        let db = setup_db().await;
        let clickup = Arc::new(ScriptedClickUp {
            get_task_calls: AtomicU32::new(0),
            get_comments_calls: AtomicU32::new(0),
            mode: ScriptedMode::AlwaysError,
        });
        let calls_handle = clickup.clone();
        let state = Arc::new(AppState {
            config: cfg("http://127.0.0.1:1".to_string(), 1),
            db,
            clickup,
            federation_client: FederationHttpClient::new(),
        });

        let (tx, rx) = tokio::sync::broadcast::channel(1);
        let handle = tokio::spawn(run_poller(state, rx));

        // Allow the loop to tick a couple of times.
        tokio::time::sleep(Duration::from_millis(1500)).await;
        assert!(!handle.is_finished(), "poller should still be running");
        assert_eq!(
            calls_handle.get_task_calls.load(Ordering::SeqCst),
            0,
            "empty queue must not invoke clickup"
        );

        let _ = tx.send(());
        let _ = tokio::time::timeout(Duration::from_secs(2), handle).await.unwrap();
    }

    /// Task claimed: insert one active mapping, ClickUp reports it as
    /// "complete" with a comment, and the overmind result endpoint receives
    /// the FederationResult. After the result is sent, the DB row must be
    /// marked result_sent = 1.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn poll_dispatches_completed_task_and_marks_sent() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/federation/result"))
            .respond_with(ResponseTemplate::new(200).set_body_string("{}"))
            .mount(&server)
            .await;

        let db = setup_db().await;
        let task_id = Uuid::new_v4();
        let corr_id = Uuid::new_v4();
        insert_active_mapping(&db, task_id, corr_id);

        let clickup = Arc::new(ScriptedClickUp {
            get_task_calls: AtomicU32::new(0),
            get_comments_calls: AtomicU32::new(0),
            mode: ScriptedMode::Completed {
                status: "complete".to_string(),
                comment_text: "all done — see https://example.com".to_string(),
            },
        });
        let calls_handle = clickup.clone();
        let state = Arc::new(AppState {
            config: cfg(server.uri(), 1),
            db: db.clone(),
            clickup,
            federation_client: FederationHttpClient::new(),
        });

        let (tx, rx) = tokio::sync::broadcast::channel(1);
        let handle = tokio::spawn(run_poller(state, rx));

        // Wait for at least one full poll iteration to fire and complete.
        tokio::time::sleep(Duration::from_millis(1500)).await;
        let _ = tx.send(());
        let _ = tokio::time::timeout(Duration::from_secs(2), handle).await.unwrap();

        assert!(
            calls_handle.get_task_calls.load(Ordering::SeqCst) >= 1,
            "poller should have polled clickup at least once"
        );

        let received = server.received_requests().await.unwrap();
        assert!(
            !received.is_empty(),
            "overmind /federation/result must have been called"
        );

        let mapping = crate::state::get_mapping(&db, &task_id.to_string())
            .await
            .unwrap()
            .expect("mapping persists after completion");
        assert_eq!(mapping.status, "completed");
        assert!(mapping.result_sent, "result_sent flag must be set after dispatch");
    }

    /// Error path: ClickUp errors propagate to poll_single_task which
    /// returns None — the loop logs a warning and continues. The DB row
    /// must NOT be updated, and the loop must remain alive.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn poll_clickup_error_continues_without_crashing() {
        let db = setup_db().await;
        let task_id = Uuid::new_v4();
        let corr_id = Uuid::new_v4();
        insert_active_mapping(&db, task_id, corr_id);

        let clickup = Arc::new(ScriptedClickUp {
            get_task_calls: AtomicU32::new(0),
            get_comments_calls: AtomicU32::new(0),
            mode: ScriptedMode::AlwaysError,
        });
        let calls_handle = clickup.clone();
        let state = Arc::new(AppState {
            // Bad URL so progress sends also fail — but the loop must
            // still keep ticking.
            config: cfg("http://127.0.0.1:1".to_string(), 1),
            db: db.clone(),
            clickup,
            federation_client: FederationHttpClient::new(),
        });

        let (tx, rx) = tokio::sync::broadcast::channel(1);
        let handle = tokio::spawn(run_poller(state, rx));

        tokio::time::sleep(Duration::from_millis(1500)).await;
        assert!(!handle.is_finished(), "poller must survive clickup errors");
        assert!(
            calls_handle.get_task_calls.load(Ordering::SeqCst) >= 1,
            "poller should have attempted to fetch the task"
        );

        // Mapping must still be pending — error path returns None and we
        // never call update_status.
        let mapping = crate::state::get_mapping(&db, &task_id.to_string())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(mapping.status, "pending");
        assert!(!mapping.result_sent);

        let _ = tx.send(());
        let _ = tokio::time::timeout(Duration::from_secs(2), handle).await.unwrap();
    }
}
