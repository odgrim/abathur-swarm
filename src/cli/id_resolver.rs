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

const TASK_QUERY: &str = "SELECT id FROM tasks WHERE id LIKE ?";
const GOAL_QUERY: &str = "SELECT id FROM goals WHERE id LIKE ?";
const WORKTREE_QUERY: &str =
    "SELECT id FROM worktrees WHERE id LIKE ? UNION SELECT id FROM worktrees WHERE task_id LIKE ?";
const MEMORY_QUERY: &str = "SELECT id FROM memories WHERE id LIKE ?";

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

async fn resolve_prefix(
    pool: &SqlitePool,
    prefix: &str,
    entity: &str,
    query: &str,
) -> Result<Uuid> {
    // Fast path: if it parses as a full UUID, return directly
    if let Ok(uuid) = Uuid::parse_str(prefix) {
        return Ok(uuid);
    }

    validate_prefix(prefix)?;

    let pattern = format!("{}%", prefix);

    let rows: Vec<(String,)> = if query == WORKTREE_QUERY {
        // Worktree query has two bind params (id LIKE ? UNION task_id LIKE ?)
        sqlx::query_as(query)
            .bind(&pattern)
            .bind(&pattern)
            .fetch_all(pool)
            .await?
    } else {
        sqlx::query_as(query)
            .bind(&pattern)
            .fetch_all(pool)
            .await?
    };

    match rows.len() {
        0 => bail!("No {} found matching '{}'", entity, prefix),
        1 => Ok(Uuid::parse_str(&rows[0].0)?),
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
