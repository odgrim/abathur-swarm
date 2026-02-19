//! SQLite database migration management.

use sqlx::SqlitePool;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum MigrationError {
    #[error("Failed to execute migration {version}: {source}")]
    ExecutionError { version: i64, #[source] source: sqlx::Error },
    #[error("Failed to get schema version: {0}")]
    VersionCheckError(#[source] sqlx::Error),
}

#[derive(Debug, Clone)]
pub struct Migration {
    pub version: i64,
    pub description: String,
    pub sql: String,
}

pub struct Migrator {
    pool: SqlitePool,
}

impl Migrator {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn run_embedded_migrations(&self, migrations: Vec<Migration>) -> Result<usize, MigrationError> {
        self.ensure_migrations_table().await?;
        let current_version = self.get_current_version().await?;
        let pending: Vec<_> = migrations.into_iter().filter(|m| m.version > current_version).collect();

        if pending.is_empty() {
            return Ok(0);
        }

        for migration in &pending {
            self.apply_migration(migration).await?;
        }

        Ok(pending.len())
    }

    async fn ensure_migrations_table(&self) -> Result<(), MigrationError> {
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS schema_migrations (
                version INTEGER PRIMARY KEY,
                applied_at TEXT NOT NULL DEFAULT (datetime('now')),
                description TEXT
            )"
        )
        .execute(&self.pool)
        .await
        .map_err(|e| MigrationError::ExecutionError { version: 0, source: e })?;
        Ok(())
    }

    pub async fn get_current_version(&self) -> Result<i64, MigrationError> {
        let result: Option<(i64,)> = sqlx::query_as("SELECT COALESCE(MAX(version), 0) FROM schema_migrations")
            .fetch_optional(&self.pool)
            .await
            .map_err(MigrationError::VersionCheckError)?;
        Ok(result.map(|(v,)| v).unwrap_or(0))
    }

    async fn apply_migration(&self, migration: &Migration) -> Result<(), MigrationError> {
        sqlx::raw_sql(&migration.sql)
            .execute(&self.pool)
            .await
            .map_err(|e| MigrationError::ExecutionError { version: migration.version, source: e })?;

        sqlx::query("INSERT OR IGNORE INTO schema_migrations (version, description) VALUES (?, ?)")
            .bind(migration.version)
            .bind(&migration.description)
            .execute(&self.pool)
            .await
            .map_err(|e| MigrationError::ExecutionError { version: migration.version, source: e })?;

        Ok(())
    }
}

pub fn all_embedded_migrations() -> Vec<Migration> {
    vec![
        Migration {
            version: 1,
            description: "Initial schema".to_string(),
            sql: include_str!("../../../migrations/001_initial_schema.sql").to_string(),
        },
        Migration {
            version: 2,
            description: "Workflow schema".to_string(),
            sql: include_str!("../../../migrations/002_workflow_schema.sql").to_string(),
        },
        Migration {
            version: 3,
            description: "Task schedule schema".to_string(),
            sql: include_str!("../../../migrations/003_task_schedule_schema.sql").to_string(),
        },
        Migration {
            version: 4,
            description: "Task type".to_string(),
            sql: include_str!("../../../migrations/004_task_type.sql").to_string(),
        },
        Migration {
            version: 5,
            description: "Distinct accessors for memory promotion integrity".to_string(),
            sql: include_str!("../../../migrations/005_distinct_accessors.sql").to_string(),
        },
        Migration {
            version: 6,
            description: "Per-goal convergence check tracking".to_string(),
            sql: include_str!("../../../migrations/006_goal_convergence_check_at.sql").to_string(),
        },
    ]
}
