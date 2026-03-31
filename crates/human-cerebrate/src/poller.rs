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
