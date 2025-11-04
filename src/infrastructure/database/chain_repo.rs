use crate::domain::models::prompt_chain::{ChainExecution, ChainStatus, PromptChain};
use crate::domain::ports::chain_repository::{ChainRepository, ChainStats};
use crate::infrastructure::database::utils::parse_datetime;
use anyhow::{Context, Result};
use async_trait::async_trait;
use sqlx::{Row, SqlitePool};

/// SQLite implementation of ChainRepository
///
/// Provides async database operations for prompt chain storage with:
/// - Dynamic queries using sqlx::query with .bind() for runtime flexibility
/// - JSON serialization for steps and results fields
pub struct ChainRepositoryImpl {
    pool: SqlitePool,
}

impl ChainRepositoryImpl {
    /// Create a new ChainRepositoryImpl
    ///
    /// # Arguments
    /// * `pool` - SQLite connection pool
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl ChainRepository for ChainRepositoryImpl {
    async fn insert_chain(&self, chain: &PromptChain) -> Result<()> {
        let steps_json =
            serde_json::to_string(&chain.steps).context("Failed to serialize steps")?;
        let validation_rules_json = serde_json::to_string(&chain.validation_rules)
            .context("Failed to serialize validation rules")?;
        let created_at_str = chain.created_at.to_rfc3339();
        let updated_at_str = chain.updated_at.to_rfc3339();

        sqlx::query(
            r#"
            INSERT INTO prompt_chains (
                id, name, description, steps, validation_rules,
                created_at, updated_at
            )
            VALUES (?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(&chain.id)
        .bind(&chain.name)
        .bind(&chain.description)
        .bind(&steps_json)
        .bind(&validation_rules_json)
        .bind(&created_at_str)
        .bind(&updated_at_str)
        .execute(&self.pool)
        .await
        .context("Failed to insert chain")?;

        Ok(())
    }

    async fn get_chain(&self, chain_id: &str) -> Result<Option<PromptChain>> {
        let row = sqlx::query(
            r#"
            SELECT id, name, description, steps, validation_rules,
                   created_at, updated_at
            FROM prompt_chains
            WHERE id = ?
            "#,
        )
        .bind(chain_id)
        .fetch_optional(&self.pool)
        .await
        .context("Failed to query chain")?;

        match row {
            Some(r) => {
                let chain = PromptChain {
                    id: r.get("id"),
                    name: r.get("name"),
                    description: r.get("description"),
                    steps: serde_json::from_str(r.get::<String, _>("steps").as_str())
                        .context("Failed to deserialize steps")?,
                    validation_rules: serde_json::from_str(r.get::<String, _>("validation_rules").as_str())
                        .context("Failed to deserialize validation rules")?,
                    created_at: parse_datetime(r.get::<String, _>("created_at").as_str())?,
                    updated_at: parse_datetime(r.get::<String, _>("updated_at").as_str())?,
                };
                Ok(Some(chain))
            }
            None => Ok(None),
        }
    }

    async fn get_chain_by_name(&self, name: &str) -> Result<Option<PromptChain>> {
        let row = sqlx::query(
            r#"
            SELECT id, name, description, steps, validation_rules,
                   created_at, updated_at
            FROM prompt_chains
            WHERE name = ?
            ORDER BY created_at DESC
            LIMIT 1
            "#,
        )
        .bind(name)
        .fetch_optional(&self.pool)
        .await
        .context("Failed to query chain by name")?;

        match row {
            Some(r) => {
                let chain = PromptChain {
                    id: r.get("id"),
                    name: r.get("name"),
                    description: r.get("description"),
                    steps: serde_json::from_str(r.get::<String, _>("steps").as_str())
                        .context("Failed to deserialize steps")?,
                    validation_rules: serde_json::from_str(r.get::<String, _>("validation_rules").as_str())
                        .context("Failed to deserialize validation rules")?,
                    created_at: parse_datetime(r.get::<String, _>("created_at").as_str())?,
                    updated_at: parse_datetime(r.get::<String, _>("updated_at").as_str())?,
                };
                Ok(Some(chain))
            }
            None => Ok(None),
        }
    }

    async fn list_chains(&self) -> Result<Vec<PromptChain>> {
        let rows = sqlx::query(
            r#"
            SELECT id, name, description, steps, validation_rules,
                   created_at, updated_at
            FROM prompt_chains
            ORDER BY created_at DESC
            "#,
        )
        .fetch_all(&self.pool)
        .await
        .context("Failed to list chains")?;

        let mut chains = Vec::new();
        for r in rows {
            let chain = PromptChain {
                id: r.get("id"),
                name: r.get("name"),
                description: r.get("description"),
                steps: serde_json::from_str(r.get::<String, _>("steps").as_str())
                    .context("Failed to deserialize steps")?,
                validation_rules: serde_json::from_str(r.get::<String, _>("validation_rules").as_str())
                    .context("Failed to deserialize validation rules")?,
                created_at: parse_datetime(r.get::<String, _>("created_at").as_str())?,
                updated_at: parse_datetime(r.get::<String, _>("updated_at").as_str())?,
            };
            chains.push(chain);
        }

        Ok(chains)
    }

    async fn update_chain(&self, chain: &PromptChain) -> Result<()> {
        let steps_json =
            serde_json::to_string(&chain.steps).context("Failed to serialize steps")?;
        let validation_rules_json = serde_json::to_string(&chain.validation_rules)
            .context("Failed to serialize validation rules")?;
        let updated_at_str = chain.updated_at.to_rfc3339();

        sqlx::query(
            r#"
            UPDATE prompt_chains
            SET name = ?, description = ?, steps = ?,
                validation_rules = ?, updated_at = ?
            WHERE id = ?
            "#,
        )
        .bind(&chain.name)
        .bind(&chain.description)
        .bind(&steps_json)
        .bind(&validation_rules_json)
        .bind(&updated_at_str)
        .bind(&chain.id)
        .execute(&self.pool)
        .await
        .context("Failed to update chain")?;

        Ok(())
    }

    async fn delete_chain(&self, chain_id: &str) -> Result<()> {
        sqlx::query(
            r#"
            DELETE FROM prompt_chains
            WHERE id = ?
            "#,
        )
        .bind(chain_id)
        .execute(&self.pool)
        .await
        .context("Failed to delete chain")?;

        Ok(())
    }

    async fn insert_execution(&self, execution: &ChainExecution) -> Result<()> {
        let step_results_json = serde_json::to_string(&execution.step_results)
            .context("Failed to serialize step results")?;
        let status_str = execution.status.to_string();
        let started_at_str = execution.started_at.to_rfc3339();
        let completed_at_str = execution.completed_at.map(|dt| dt.to_rfc3339());
        let current_step = execution.current_step as i64;

        sqlx::query(
            r#"
            INSERT INTO chain_executions (
                id, chain_id, task_id, current_step, step_results,
                status, started_at, completed_at
            )
            VALUES (?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(&execution.id)
        .bind(&execution.chain_id)
        .bind(&execution.task_id)
        .bind(current_step)
        .bind(&step_results_json)
        .bind(&status_str)
        .bind(&started_at_str)
        .bind(&completed_at_str)
        .execute(&self.pool)
        .await
        .context("Failed to insert execution")?;

        Ok(())
    }

    async fn get_execution(&self, execution_id: &str) -> Result<Option<ChainExecution>> {
        let row = sqlx::query(
            r#"
            SELECT id, chain_id, task_id, current_step, step_results,
                   status, started_at, completed_at
            FROM chain_executions
            WHERE id = ?
            "#,
        )
        .bind(execution_id)
        .fetch_optional(&self.pool)
        .await
        .context("Failed to query execution")?;

        match row {
            Some(r) => {
                let status = parse_chain_status(r.get::<String, _>("status").as_str())?;
                let execution = ChainExecution {
                    id: r.get("id"),
                    chain_id: r.get("chain_id"),
                    task_id: r.get("task_id"),
                    current_step: r.get::<i64, _>("current_step") as usize,
                    step_results: serde_json::from_str(r.get::<String, _>("step_results").as_str())
                        .context("Failed to deserialize step results")?,
                    status,
                    started_at: parse_datetime(r.get::<String, _>("started_at").as_str())?,
                    completed_at: r
                        .try_get::<Option<String>, _>("completed_at")?
                        .as_ref()
                        .map(|s| parse_datetime(s))
                        .transpose()?,
                };
                Ok(Some(execution))
            }
            None => Ok(None),
        }
    }

    async fn list_executions_for_chain(
        &self,
        chain_id: &str,
        limit: usize,
    ) -> Result<Vec<ChainExecution>> {
        let limit_i64 = limit as i64;
        let rows = sqlx::query(
            r#"
            SELECT id, chain_id, task_id, current_step, step_results,
                   status, started_at, completed_at
            FROM chain_executions
            WHERE chain_id = ?
            ORDER BY started_at DESC
            LIMIT ?
            "#,
        )
        .bind(chain_id)
        .bind(limit_i64)
        .fetch_all(&self.pool)
        .await
        .context("Failed to list executions for chain")?;

        let mut executions = Vec::new();
        for r in rows {
            let status = parse_chain_status(r.get::<String, _>("status").as_str())?;
            let execution = ChainExecution {
                id: r.get("id"),
                chain_id: r.get("chain_id"),
                task_id: r.get("task_id"),
                current_step: r.get::<i64, _>("current_step") as usize,
                step_results: serde_json::from_str(r.get::<String, _>("step_results").as_str())
                    .context("Failed to deserialize step results")?,
                status,
                started_at: parse_datetime(r.get::<String, _>("started_at").as_str())?,
                completed_at: r
                    .try_get::<Option<String>, _>("completed_at")?
                    .as_ref()
                    .map(|s| parse_datetime(s))
                    .transpose()?,
            };
            executions.push(execution);
        }

        Ok(executions)
    }

    async fn list_executions_for_task(&self, task_id: &str) -> Result<Vec<ChainExecution>> {
        let rows = sqlx::query(
            r#"
            SELECT id, chain_id, task_id, current_step, step_results,
                   status, started_at, completed_at
            FROM chain_executions
            WHERE task_id = ?
            ORDER BY started_at DESC
            "#,
        )
        .bind(task_id)
        .fetch_all(&self.pool)
        .await
        .context("Failed to list executions for task")?;

        let mut executions = Vec::new();
        for r in rows {
            let status = parse_chain_status(r.get::<String, _>("status").as_str())?;
            let execution = ChainExecution {
                id: r.get("id"),
                chain_id: r.get("chain_id"),
                task_id: r.get("task_id"),
                current_step: r.get::<i64, _>("current_step") as usize,
                step_results: serde_json::from_str(r.get::<String, _>("step_results").as_str())
                    .context("Failed to deserialize step results")?,
                status,
                started_at: parse_datetime(r.get::<String, _>("started_at").as_str())?,
                completed_at: r
                    .try_get::<Option<String>, _>("completed_at")?
                    .as_ref()
                    .map(|s| parse_datetime(s))
                    .transpose()?,
            };
            executions.push(execution);
        }

        Ok(executions)
    }

    async fn update_execution(&self, execution: &ChainExecution) -> Result<()> {
        let step_results_json = serde_json::to_string(&execution.step_results)
            .context("Failed to serialize step results")?;
        let status_str = execution.status.to_string();
        let completed_at_str = execution.completed_at.map(|dt| dt.to_rfc3339());
        let current_step = execution.current_step as i64;

        sqlx::query(
            r#"
            UPDATE chain_executions
            SET current_step = ?, step_results = ?, status = ?, completed_at = ?
            WHERE id = ?
            "#,
        )
        .bind(current_step)
        .bind(&step_results_json)
        .bind(&status_str)
        .bind(&completed_at_str)
        .bind(&execution.id)
        .execute(&self.pool)
        .await
        .context("Failed to update execution")?;

        Ok(())
    }

    async fn get_chain_stats(&self, chain_id: &str) -> Result<ChainStats> {
        let row = sqlx::query(
            r#"
            SELECT
                COUNT(*) as total,
                SUM(CASE WHEN status = 'completed' THEN 1 ELSE 0 END) as completed,
                SUM(CASE WHEN status LIKE 'failed%' THEN 1 ELSE 0 END) as failed,
                SUM(CASE WHEN status = 'validation_failed' THEN 1 ELSE 0 END) as validation_failed,
                SUM(CASE WHEN status = 'running' THEN 1 ELSE 0 END) as running,
                AVG(
                    CASE
                        WHEN completed_at IS NOT NULL AND started_at IS NOT NULL
                        THEN (julianday(completed_at) - julianday(started_at)) * 86400
                        ELSE NULL
                    END
                ) as avg_duration
            FROM chain_executions
            WHERE chain_id = ?
            "#,
        )
        .bind(chain_id)
        .fetch_one(&self.pool)
        .await
        .context("Failed to get chain stats")?;

        Ok(ChainStats {
            total_executions: row.get::<i64, _>("total") as usize,
            completed: row.try_get::<Option<i64>, _>("completed")?.unwrap_or(0) as usize,
            failed: row.try_get::<Option<i64>, _>("failed")?.unwrap_or(0) as usize,
            validation_failed: row.try_get::<Option<i64>, _>("validation_failed")?.unwrap_or(0) as usize,
            running: row.try_get::<Option<i64>, _>("running")?.unwrap_or(0) as usize,
            avg_duration_secs: row.try_get::<Option<f64>, _>("avg_duration")?,
        })
    }
}

/// Parse chain status from string
fn parse_chain_status(status_str: &str) -> Result<ChainStatus> {
    if status_str == "running" {
        Ok(ChainStatus::Running)
    } else if status_str == "completed" {
        Ok(ChainStatus::Completed)
    } else if let Some(error) = status_str.strip_prefix("failed: ") {
        Ok(ChainStatus::Failed(error.to_string()))
    } else if let Some(error) = status_str.strip_prefix("validation_failed: ") {
        Ok(ChainStatus::ValidationFailed(error.to_string()))
    } else {
        anyhow::bail!("Unknown chain status: {}", status_str)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::models::prompt_chain::{OutputFormat, PromptStep};

    #[test]
    fn test_parse_chain_status() {
        assert!(matches!(
            parse_chain_status("running").unwrap(),
            ChainStatus::Running
        ));
        assert!(matches!(
            parse_chain_status("completed").unwrap(),
            ChainStatus::Completed
        ));
        assert!(matches!(
            parse_chain_status("failed: error message").unwrap(),
            ChainStatus::Failed(_)
        ));
        assert!(matches!(
            parse_chain_status("validation_failed: validation error").unwrap(),
            ChainStatus::ValidationFailed(_)
        ));
    }
}
