use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use sqlx::SqlitePool;
use uuid::Uuid;

use crate::domain::models::{Agent, AgentStatus};
use crate::domain::ports::AgentRepository;
use crate::infrastructure::database::DatabaseError;

/// SQLite implementation of AgentRepository using sqlx
///
/// Provides persistent storage for agents with compile-time verified queries.
/// Uses SQLite with WAL mode for better concurrency.
pub struct AgentRepositoryImpl {
    pool: SqlitePool,
}

/// Raw agent row data from database queries
///
/// This struct helps reduce the number of function parameters and satisfies
/// clippy's argument count limits.
struct AgentRowData {
    id: String,
    agent_type: String,
    status: String,
    current_task_id: Option<String>,
    heartbeat_at: String,
    memory_usage_bytes: i64,
    cpu_usage_percent: f64,
    created_at: String,
    terminated_at: Option<String>,
}

impl AgentRepositoryImpl {
    /// Create a new agent repository instance
    ///
    /// # Arguments
    /// * `pool` - SQLite connection pool
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Helper function to parse a row into an Agent struct
    fn parse_agent_row(row: AgentRowData) -> Result<Agent, DatabaseError> {
        Ok(Agent {
            id: Uuid::parse_str(&row.id)
                .map_err(|e| DatabaseError::ParseError(format!("Invalid UUID: {}", e)))?,
            agent_type: row.agent_type,
            status: row
                .status
                .parse()
                .map_err(|e: anyhow::Error| DatabaseError::ParseError(e.to_string()))?,
            current_task_id: row
                .current_task_id
                .as_ref()
                .map(|s| Uuid::parse_str(s))
                .transpose()
                .map_err(|e| DatabaseError::ParseError(format!("Invalid UUID: {}", e)))?,
            heartbeat_at: DateTime::parse_from_rfc3339(&row.heartbeat_at)
                .map_err(|e| DatabaseError::ParseError(format!("Invalid timestamp: {}", e)))?
                .with_timezone(&Utc),
            memory_usage_bytes: row.memory_usage_bytes as u64,
            cpu_usage_percent: row.cpu_usage_percent,
            created_at: DateTime::parse_from_rfc3339(&row.created_at)
                .map_err(|e| DatabaseError::ParseError(format!("Invalid timestamp: {}", e)))?
                .with_timezone(&Utc),
            terminated_at: row
                .terminated_at
                .as_ref()
                .map(|s| DateTime::parse_from_rfc3339(s))
                .transpose()
                .map_err(|e| DatabaseError::ParseError(format!("Invalid timestamp: {}", e)))?
                .map(|dt| dt.with_timezone(&Utc)),
        })
    }
}

#[async_trait]
impl AgentRepository for AgentRepositoryImpl {
    async fn insert(&self, agent: Agent) -> Result<(), DatabaseError> {
        let id_str = agent.id.to_string();
        let status_str = agent.status.to_string();
        let current_task_str = agent.current_task_id.map(|id| id.to_string());
        let heartbeat_str = agent.heartbeat_at.to_rfc3339();
        let memory_bytes = agent.memory_usage_bytes as i64;
        let created_str = agent.created_at.to_rfc3339();
        let terminated_str = agent.terminated_at.map(|dt| dt.to_rfc3339());

        sqlx::query!(
            r#"
            INSERT INTO agents (
                id, agent_type, status, current_task_id, heartbeat_at,
                memory_usage_bytes, cpu_usage_percent, created_at, terminated_at
            )
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
            "#,
            id_str,
            agent.agent_type,
            status_str,
            current_task_str,
            heartbeat_str,
            memory_bytes,
            agent.cpu_usage_percent,
            created_str,
            terminated_str
        )
        .execute(&self.pool)
        .await
        .map_err(DatabaseError::QueryFailed)?;

        Ok(())
    }

    async fn get(&self, id: Uuid) -> Result<Option<Agent>, DatabaseError> {
        let id_str = id.to_string();
        let row = sqlx::query!(
            r#"
            SELECT id, agent_type, status, current_task_id, heartbeat_at,
                   memory_usage_bytes, cpu_usage_percent, created_at, terminated_at
            FROM agents
            WHERE id = ?
            "#,
            id_str
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(DatabaseError::QueryFailed)?;

        match row {
            Some(r) => {
                let agent = Self::parse_agent_row(AgentRowData {
                    id: r.id,
                    agent_type: r.agent_type,
                    status: r.status,
                    current_task_id: r.current_task_id,
                    heartbeat_at: r.heartbeat_at,
                    memory_usage_bytes: r.memory_usage_bytes,
                    cpu_usage_percent: r.cpu_usage_percent,
                    created_at: r.created_at,
                    terminated_at: r.terminated_at,
                })?;
                Ok(Some(agent))
            }
            None => Ok(None),
        }
    }

    async fn update(&self, agent: Agent) -> Result<(), DatabaseError> {
        let status_str = agent.status.to_string();
        let current_task_str = agent.current_task_id.map(|id| id.to_string());
        let heartbeat_str = agent.heartbeat_at.to_rfc3339();
        let memory_bytes = agent.memory_usage_bytes as i64;
        let terminated_str = agent.terminated_at.map(|dt| dt.to_rfc3339());
        let id_str = agent.id.to_string();

        sqlx::query!(
            r#"
            UPDATE agents SET
                agent_type = ?,
                status = ?,
                current_task_id = ?,
                heartbeat_at = ?,
                memory_usage_bytes = ?,
                cpu_usage_percent = ?,
                terminated_at = ?
            WHERE id = ?
            "#,
            agent.agent_type,
            status_str,
            current_task_str,
            heartbeat_str,
            memory_bytes,
            agent.cpu_usage_percent,
            terminated_str,
            id_str
        )
        .execute(&self.pool)
        .await
        .map_err(DatabaseError::QueryFailed)?;

        Ok(())
    }

    async fn list(&self, status: Option<AgentStatus>) -> Result<Vec<Agent>, DatabaseError> {
        // Use runtime query for dynamic WHERE clause
        let mut query = String::from(
            "SELECT id, agent_type, status, current_task_id, heartbeat_at, \
             memory_usage_bytes, cpu_usage_percent, created_at, terminated_at \
             FROM agents",
        );

        let agents = match status {
            Some(s) => {
                query.push_str(" WHERE status = ? ORDER BY created_at DESC");
                let status_str = s.to_string();
                sqlx::query_as::<
                    _,
                    (
                        String,
                        String,
                        String,
                        Option<String>,
                        String,
                        i64,
                        f64,
                        String,
                        Option<String>,
                    ),
                >(&query)
                .bind(status_str)
                .fetch_all(&self.pool)
                .await
                .map_err(DatabaseError::QueryFailed)?
            }
            None => {
                query.push_str(" ORDER BY created_at DESC");
                sqlx::query_as::<
                    _,
                    (
                        String,
                        String,
                        String,
                        Option<String>,
                        String,
                        i64,
                        f64,
                        String,
                        Option<String>,
                    ),
                >(&query)
                .fetch_all(&self.pool)
                .await
                .map_err(DatabaseError::QueryFailed)?
            }
        };

        // Map rows to Agent structs
        agents
            .into_iter()
            .map(
                |(
                    id,
                    agent_type,
                    status,
                    current_task_id,
                    heartbeat_at,
                    memory_usage_bytes,
                    cpu_usage_percent,
                    created_at,
                    terminated_at,
                )| {
                    Self::parse_agent_row(AgentRowData {
                        id,
                        agent_type,
                        status,
                        current_task_id,
                        heartbeat_at,
                        memory_usage_bytes,
                        cpu_usage_percent,
                        created_at,
                        terminated_at,
                    })
                },
            )
            .collect()
    }

    async fn find_stale_agents(
        &self,
        heartbeat_threshold: Duration,
    ) -> Result<Vec<Agent>, DatabaseError> {
        // Calculate the cutoff timestamp
        let cutoff = Utc::now() - heartbeat_threshold;
        let cutoff_str = cutoff.to_rfc3339();

        let rows = sqlx::query!(
            r#"
            SELECT id, agent_type, status, current_task_id, heartbeat_at,
                   memory_usage_bytes, cpu_usage_percent, created_at, terminated_at
            FROM agents
            WHERE heartbeat_at < ?
            AND status != 'terminated'
            ORDER BY heartbeat_at ASC
            "#,
            cutoff_str
        )
        .fetch_all(&self.pool)
        .await
        .map_err(DatabaseError::QueryFailed)?;

        // Map rows to Agent structs
        rows.into_iter()
            .map(|r| {
                Self::parse_agent_row(AgentRowData {
                    id: r.id,
                    agent_type: r.agent_type,
                    status: r.status,
                    current_task_id: r.current_task_id,
                    heartbeat_at: r.heartbeat_at,
                    memory_usage_bytes: r.memory_usage_bytes,
                    cpu_usage_percent: r.cpu_usage_percent,
                    created_at: r.created_at,
                    terminated_at: r.terminated_at,
                })
            })
            .collect()
    }

    async fn update_heartbeat(&self, id: Uuid) -> Result<(), DatabaseError> {
        let now = Utc::now().to_rfc3339();
        let id_str = id.to_string();

        let result = sqlx::query!(
            r#"
            UPDATE agents
            SET heartbeat_at = ?
            WHERE id = ?
            "#,
            now,
            id_str
        )
        .execute(&self.pool)
        .await
        .map_err(DatabaseError::QueryFailed)?;

        if result.rows_affected() == 0 {
            return Err(DatabaseError::NotFound(id));
        }

        Ok(())
    }
}
