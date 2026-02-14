//! Short ID prefix resolution for CLI show commands.
//!
//! Allows users to specify any unique prefix of a UUID instead of the full 32-char ID,
//! similar to git short hashes.

use anyhow::{bail, Result};
use sqlx::SqlitePool;
use uuid::Uuid;

/// Resolve a task ID prefix to a full UUID.
pub async fn resolve_task_id(pool: &SqlitePool, prefix: &str) -> Result<Uuid> {
    resolve_prefix(pool, prefix, "task", TASK_QUERY)
        .await
}

/// Resolve a goal ID prefix to a full UUID.
pub async fn resolve_goal_id(pool: &SqlitePool, prefix: &str) -> Result<Uuid> {
    resolve_prefix(pool, prefix, "goal", GOAL_QUERY)
        .await
}

/// Resolve a worktree ID prefix to a full UUID.
///
/// Searches both the worktree `id` and `task_id` columns.
pub async fn resolve_worktree_id(pool: &SqlitePool, prefix: &str) -> Result<Uuid> {
    resolve_prefix(pool, prefix, "worktree", WORKTREE_QUERY)
        .await
}

/// Resolve a memory ID prefix to a full UUID.
pub async fn resolve_memory_id(pool: &SqlitePool, prefix: &str) -> Result<Uuid> {
    resolve_prefix(pool, prefix, "memory", MEMORY_QUERY)
        .await
}

/// Resolve a trigger rule ID prefix to a full UUID.
pub async fn resolve_trigger_rule_id(pool: &SqlitePool, prefix: &str) -> Result<Uuid> {
    resolve_prefix(pool, prefix, "trigger_rule", TRIGGER_RULE_QUERY)
        .await
}

/// Resolve an event ID prefix to a full UUID.
pub async fn resolve_event_id(pool: &SqlitePool, prefix: &str) -> Result<Uuid> {
    resolve_prefix(pool, prefix, "event", EVENT_QUERY)
        .await
}

/// Resolve a dead letter entry ID prefix to a full ID string.
///
/// Returns `String` rather than `Uuid` because the `EventStore` trait uses
/// `&str` for DLQ IDs.  Only unresolved entries are searched.
pub async fn resolve_dlq_id(pool: &SqlitePool, prefix: &str) -> Result<String> {
    resolve_prefix_raw(pool, prefix, "dead letter entry", DLQ_QUERY).await
}

const TASK_QUERY: &str = "SELECT id FROM tasks WHERE id LIKE ?";
const GOAL_QUERY: &str = "SELECT id FROM goals WHERE id LIKE ?";
const WORKTREE_QUERY: &str =
    "SELECT id FROM worktrees WHERE id LIKE ?1 UNION SELECT id FROM worktrees WHERE task_id LIKE ?1";
const MEMORY_QUERY: &str = "SELECT id FROM memories WHERE id LIKE ?";
const TRIGGER_RULE_QUERY: &str = "SELECT id FROM trigger_rules WHERE id LIKE ?";
const EVENT_QUERY: &str = "SELECT id FROM events WHERE id LIKE ?";
const DLQ_QUERY: &str = "SELECT id FROM dead_letter_events WHERE id LIKE ? AND resolved_at IS NULL";

fn validate_prefix(prefix: &str) -> Result<()> {
    if prefix.is_empty() {
        bail!("ID prefix must not be empty");
    }
    if !prefix.chars().all(|c| c.is_ascii_hexdigit() || c == '-') {
        bail!(
            "Invalid ID prefix '{}': must contain only hex characters and dashes",
            prefix
        );
    }
    Ok(())
}

/// Core prefix resolution returning the raw ID string.
async fn resolve_prefix_raw(
    pool: &SqlitePool,
    prefix: &str,
    entity: &str,
    query: &str,
) -> Result<String> {
    if Uuid::parse_str(prefix).is_ok() {
        return Ok(prefix.to_string());
    }

    validate_prefix(prefix)?;

    let pattern = format!("{}%", prefix);
    let rows: Vec<(String,)> = sqlx::query_as(query)
        .bind(&pattern)
        .fetch_all(pool)
        .await?;

    match rows.len() {
        0 => bail!("No {} found matching '{}'", entity, prefix),
        1 => Ok(rows[0].0.clone()),
        n => {
            let mut msg = format!(
                "Ambiguous prefix '{}': matches {} {}s:",
                prefix, n, entity
            );
            for row in &rows {
                msg.push_str(&format!("\n  {}", row.0));
            }
            bail!("{}", msg)
        }
    }
}

async fn resolve_prefix(
    pool: &SqlitePool,
    prefix: &str,
    entity: &str,
    query: &str,
) -> Result<Uuid> {
    if let Ok(uuid) = Uuid::parse_str(prefix) {
        return Ok(uuid);
    }
    let raw = resolve_prefix_raw(pool, prefix, entity, query).await?;
    Ok(Uuid::parse_str(&raw)?)
}
