//! Shared helpers for built-in handlers.

use crate::domain::errors::DomainError;
use crate::domain::models::Task;
use crate::domain::ports::TaskRepository;

/// Attempt a task repository update, logging and ignoring ConcurrencyConflict errors.
/// Returns `true` if the update succeeded, `false` if a conflict was suppressed.
pub(crate) async fn try_update_task<T: TaskRepository>(
    repo: &T,
    task: &Task,
    context: &str,
) -> Result<bool, String> {
    match repo.update(task).await {
        Ok(()) => Ok(true),
        Err(DomainError::ConcurrencyConflict { entity, id }) => {
            tracing::warn!(
                "ReconciliationHandler: ConcurrencyConflict on {} {} during {}; skipping",
                entity,
                id,
                context
            );
            Ok(false)
        }
        Err(e) => Err(format!("Failed to update ({}): {}", context, e)),
    }
}

/// Retry-aware task update: re-fetches the task on ConcurrencyConflict and re-applies the
/// mutation. The mutation closure receives a `&mut Task` and should return:
///   - `Ok(true)`  → proceed with the update (mutation was applied)
///   - `Ok(false)` → skip this task (precondition no longer met, e.g. already transitioned)
///   - `Err(msg)`  → non-retryable error
///
/// Returns `Ok(Some(task))` on successful update, `Ok(None)` if the mutation signalled
/// "not applicable", or `Err` on a non-retryable / exhausted-retries error.
pub(crate) async fn update_with_retry<T: TaskRepository>(
    repo: &T,
    task_id: uuid::Uuid,
    mutation: impl Fn(&mut Task) -> Result<bool, String>,
    max_retries: usize,
    context: &str,
) -> Result<Option<Task>, String> {
    for attempt in 0..max_retries {
        let mut task = repo
            .get(task_id)
            .await
            .map_err(|e| format!("Failed to get task ({}): {}", context, e))?
            .ok_or_else(|| format!("Task {} not found ({})", task_id, context))?;

        match mutation(&mut task) {
            Ok(true) => { /* proceed to update */ }
            Ok(false) => return Ok(None),
            Err(e) => return Err(e),
        }

        match repo.update(&task).await {
            Ok(()) => return Ok(Some(task)),
            Err(DomainError::ConcurrencyConflict { entity, id }) => {
                if attempt < max_retries - 1 {
                    tracing::debug!(
                        "{}: ConcurrencyConflict on {} {} (attempt {}/{}), retrying",
                        context,
                        entity,
                        id,
                        attempt + 1,
                        max_retries
                    );
                    continue;
                }
                tracing::warn!(
                    "{}: ConcurrencyConflict on {} {} after {} attempts, giving up",
                    context,
                    entity,
                    id,
                    max_retries
                );
                return Err(format!(
                    "ConcurrencyConflict on {} {} after {} retries ({})",
                    entity, id, max_retries, context
                ));
            }
            Err(e) => return Err(format!("Failed to update ({}): {}", context, e)),
        }
    }
    Err(format!(
        "update_with_retry called with max_retries=0 ({})",
        context
    ))
}
