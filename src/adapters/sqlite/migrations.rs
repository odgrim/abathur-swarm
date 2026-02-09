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
        Ok(())
    }
}

pub fn initial_schema_migration() -> Migration {
    Migration {
        version: 1,
        description: "Initial schema".to_string(),
        sql: include_str!("../../../migrations/001_initial_schema.sql").to_string(),
    }
}

pub fn update_memories_migration() -> Migration {
    Migration {
        version: 2,
        description: "Update memories schema for three-tier system".to_string(),
        sql: include_str!("../../../migrations/002_update_memories_schema.sql").to_string(),
    }
}

pub fn add_agent_instances_migration() -> Migration {
    Migration {
        version: 3,
        description: "Add agent instances table".to_string(),
        sql: include_str!("../../../migrations/003_add_agent_instances.sql").to_string(),
    }
}

pub fn fix_worktrees_fk_migration() -> Migration {
    Migration {
        version: 4,
        description: "Fix worktrees FK constraint".to_string(),
        sql: include_str!("../../../migrations/004_fix_worktrees_fk.sql").to_string(),
    }
}

pub fn add_events_table_migration() -> Migration {
    Migration {
        version: 5,
        description: "Add events table".to_string(),
        sql: include_str!("../../../migrations/005_add_events_table.sql").to_string(),
    }
}

pub fn goal_task_rebuild_migration() -> Migration {
    Migration {
        version: 6,
        description: "Goal-task rebuild".to_string(),
        sql: include_str!("../../../migrations/006_goal_task_rebuild.sql").to_string(),
    }
}

pub fn event_architecture_migration() -> Migration {
    Migration {
        version: 7,
        description: "Event architecture: handler watermarks".to_string(),
        sql: include_str!("../../../migrations/007_event_architecture.sql").to_string(),
    }
}

pub fn event_consistency_migration() -> Migration {
    Migration {
        version: 8,
        description: "Event consistency: scheduled events persistence".to_string(),
        sql: include_str!("../../../migrations/008_event_consistency.sql").to_string(),
    }
}

pub fn trigger_rules_migration() -> Migration {
    Migration {
        version: 9,
        description: "Trigger rules for declarative automation".to_string(),
        sql: include_str!("../../../migrations/009_trigger_rules.sql").to_string(),
    }
}

pub fn event_source_process_migration() -> Migration {
    Migration {
        version: 10,
        description: "Add source_process_id to events for cross-process propagation".to_string(),
        sql: include_str!("../../../migrations/010_event_source_process.sql").to_string(),
    }
}

pub fn dead_letter_queue_migration() -> Migration {
    Migration {
        version: 11,
        description: "Dead letter queue for handler failure retry".to_string(),
        sql: include_str!("../../../migrations/011_dead_letter_queue.sql").to_string(),
    }
}

pub fn all_embedded_migrations() -> Vec<Migration> {
    vec![
        initial_schema_migration(),
        update_memories_migration(),
        add_agent_instances_migration(),
        fix_worktrees_fk_migration(),
        add_events_table_migration(),
        goal_task_rebuild_migration(),
        event_architecture_migration(),
        event_consistency_migration(),
        trigger_rules_migration(),
        event_source_process_migration(),
        dead_letter_queue_migration(),
    ]
}
