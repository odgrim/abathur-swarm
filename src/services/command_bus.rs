//! Command Bus for routing typed domain commands to their handlers.
//!
//! The CommandBus provides a unified dispatch layer: callers construct a
//! `CommandEnvelope<DomainCommand>` and submit it here. The bus routes
//! each command variant to the appropriate handler, which validates,
//! executes the mutation, and emits events.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use std::fmt;
use std::sync::Arc;
use uuid::Uuid;

use crate::domain::errors::DomainError;
use crate::domain::models::{
    ExecutionMode, Goal, GoalConstraint, GoalPriority, GoalStatus, Memory, MemoryMetadata,
    MemoryTier, MemoryType, Task, TaskContext, TaskPriority, TaskSource, TaskStatus, TaskType,
};
use crate::services::event_bus::{EventBus, UnifiedEvent};
use crate::services::memory_service::MaintenanceReport;

/// Unique identifier for a command.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CommandId(pub Uuid);

impl CommandId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for CommandId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for CommandId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Where a command originated from.
#[derive(Debug, Clone)]
pub enum CommandSource {
    /// CLI user or external human.
    Human,
    /// Internal system (orchestrator, reconciliation, etc.).
    System,
    /// Reactive event handler.
    EventHandler(String),
    /// Scheduled task.
    Scheduler(String),
    /// Federated A2A delegation.
    A2A(String),
    /// Webhook trigger.
    Webhook(String),
    /// MCP HTTP server.
    Mcp(String),
}

impl fmt::Display for CommandSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Human => write!(f, "human"),
            Self::System => write!(f, "system"),
            Self::EventHandler(name) => write!(f, "handler:{}", name),
            Self::Scheduler(name) => write!(f, "scheduler:{}", name),
            Self::A2A(swarm) => write!(f, "a2a:{}", swarm),
            Self::Webhook(name) => write!(f, "webhook:{}", name),
            Self::Mcp(server) => write!(f, "mcp:{}", server),
        }
    }
}

/// Envelope wrapping a command with metadata.
#[derive(Debug, Clone)]
pub struct CommandEnvelope<C> {
    pub id: CommandId,
    pub timestamp: DateTime<Utc>,
    pub correlation_id: Option<Uuid>,
    pub source: CommandSource,
    pub command: C,
}

impl<C> CommandEnvelope<C> {
    pub fn new(source: CommandSource, command: C) -> Self {
        Self {
            id: CommandId::new(),
            timestamp: Utc::now(),
            correlation_id: None,
            source,
            command,
        }
    }

    pub fn with_correlation(mut self, id: Uuid) -> Self {
        self.correlation_id = Some(id);
        self
    }
}

// ---------------------------------------------------------------------------
// Domain commands
// ---------------------------------------------------------------------------

/// Top-level domain command enum.
#[derive(Debug, Clone)]
pub enum DomainCommand {
    Task(TaskCommand),
    Goal(GoalCommand),
    Memory(MemoryCommand),
}

/// Task mutation commands.
#[derive(Debug, Clone)]
pub enum TaskCommand {
    Submit {
        title: Option<String>,
        description: String,
        parent_id: Option<Uuid>,
        priority: TaskPriority,
        agent_type: Option<String>,
        depends_on: Vec<Uuid>,
        context: Box<Option<TaskContext>>,
        idempotency_key: Option<String>,
        source: TaskSource,
        deadline: Option<chrono::DateTime<chrono::Utc>>,
        task_type: Option<TaskType>,
        execution_mode: Option<ExecutionMode>,
    },
    Claim {
        task_id: Uuid,
        agent_type: String,
    },
    Complete {
        task_id: Uuid,
        tokens_used: u64,
    },
    Fail {
        task_id: Uuid,
        error: Option<String>,
    },
    Retry {
        task_id: Uuid,
    },
    Cancel {
        task_id: Uuid,
        reason: String,
    },
    Transition {
        task_id: Uuid,
        new_status: TaskStatus,
    },
}

/// Goal mutation commands.
#[derive(Debug, Clone)]
pub enum GoalCommand {
    Create {
        name: String,
        description: String,
        priority: GoalPriority,
        parent_id: Option<Uuid>,
        constraints: Vec<GoalConstraint>,
        domains: Vec<String>,
    },
    TransitionStatus {
        goal_id: Uuid,
        new_status: GoalStatus,
    },
    UpdateDomains {
        goal_id: Uuid,
        domains: Vec<String>,
    },
    Delete {
        goal_id: Uuid,
    },
}

/// Memory mutation commands.
#[derive(Debug, Clone)]
pub enum MemoryCommand {
    Store {
        key: String,
        content: String,
        namespace: String,
        tier: MemoryTier,
        memory_type: MemoryType,
        metadata: Option<MemoryMetadata>,
    },
    Recall {
        id: Uuid,
    },
    RecallByKey {
        key: String,
        namespace: String,
    },
    Forget {
        id: Uuid,
    },
    PruneExpired,
    RunMaintenance,
}

// ---------------------------------------------------------------------------
// Command results and errors
// ---------------------------------------------------------------------------

/// Typed result of command execution.
#[derive(Debug)]
pub enum CommandResult {
    Task(Task),
    Goal(Goal),
    Memory(Memory),
    MemoryOpt(Option<Memory>),
    MaintenanceReport(MaintenanceReport),
    PruneCount(u64),
    Unit,
}

/// Outcome of a command handler: the business result plus any events produced.
///
/// The CommandBus journals and broadcasts the events after successful execution.
/// Services no longer publish events directly; they return them here.
#[derive(Debug)]
pub struct CommandOutcome {
    pub result: CommandResult,
    pub events: Vec<UnifiedEvent>,
}

/// Errors that can occur during command dispatch.
#[derive(Debug, thiserror::Error)]
pub enum CommandError {
    #[error("Entity not found: {0}")]
    NotFound(String),

    #[error("Validation failed: {0}")]
    ValidationFailed(String),

    #[error("Invalid state transition from {from} to {to}")]
    InvalidTransition { from: String, to: String },

    #[error("Duplicate command: {0}")]
    DuplicateCommand(CommandId),

    #[error("Domain error: {0}")]
    DomainError(#[from] DomainError),
}

// ---------------------------------------------------------------------------
// Handler traits
// ---------------------------------------------------------------------------

/// Handler for task commands.
#[async_trait]
pub trait TaskCommandHandler: Send + Sync {
    async fn handle(&self, cmd: TaskCommand) -> Result<CommandOutcome, CommandError>;
}

/// Handler for goal commands.
#[async_trait]
pub trait GoalCommandHandler: Send + Sync {
    async fn handle(&self, cmd: GoalCommand) -> Result<CommandOutcome, CommandError>;
}

/// Handler for memory commands.
#[async_trait]
pub trait MemoryCommandHandler: Send + Sync {
    async fn handle(&self, cmd: MemoryCommand) -> Result<CommandOutcome, CommandError>;
}

// ---------------------------------------------------------------------------
// CommandBus
// ---------------------------------------------------------------------------

/// Central command dispatcher.
///
/// Holds one handler per domain and routes `DomainCommand` variants to them.
/// Owns event journaling: after a handler returns a `CommandOutcome`, the bus
/// writes all events to the EventStore (journal) and then broadcasts them
/// in-memory via the EventBus.
///
/// Includes an LRU dedup cache to prevent duplicate command processing
/// (important for replay scenarios).
pub struct CommandBus {
    task_handler: Arc<dyn TaskCommandHandler>,
    goal_handler: Arc<dyn GoalCommandHandler>,
    memory_handler: Arc<dyn MemoryCommandHandler>,
    /// EventBus for journaling and broadcasting events produced by handlers.
    event_bus: Arc<EventBus>,
    /// In-memory cache of recently processed command IDs for fast deduplication.
    processed_commands: moka::future::Cache<CommandId, ()>,
    /// Optional DB pool for persistent dedup across restarts.
    pool: Option<sqlx::SqlitePool>,
}

impl CommandBus {
    pub fn new(
        task_handler: Arc<dyn TaskCommandHandler>,
        goal_handler: Arc<dyn GoalCommandHandler>,
        memory_handler: Arc<dyn MemoryCommandHandler>,
        event_bus: Arc<EventBus>,
    ) -> Self {
        Self {
            task_handler,
            goal_handler,
            memory_handler,
            event_bus,
            processed_commands: moka::future::Cache::builder()
                .max_capacity(1000)
                .time_to_live(std::time::Duration::from_secs(3600))
                .build(),
            pool: None,
        }
    }

    /// Enable persistent command deduplication via SQLite.
    pub fn with_pool(mut self, pool: sqlx::SqlitePool) -> Self {
        self.pool = Some(pool);
        self
    }

    /// Dispatch a command envelope to the appropriate handler.
    ///
    /// Flow: dedup check -> execute handler -> journal events -> broadcast events -> return result.
    pub async fn dispatch(
        &self,
        envelope: CommandEnvelope<DomainCommand>,
    ) -> Result<CommandResult, CommandError> {
        // 1. Check in-memory dedup cache
        if self.processed_commands.get(&envelope.id).await.is_some() {
            return Err(CommandError::DuplicateCommand(envelope.id));
        }

        // 1b. Check persistent dedup table
        if let Some(ref pool) = self.pool {
            let id_str = envelope.id.0.to_string();
            let exists: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM processed_commands WHERE command_id = ?)",
            )
            .bind(&id_str)
            .fetch_one(pool)
            .await
            .unwrap_or(false);

            if exists {
                // Populate in-memory cache for future fast lookups
                self.processed_commands.insert(envelope.id, ()).await;
                return Err(CommandError::DuplicateCommand(envelope.id));
            }
        }

        tracing::debug!(
            command_id = %envelope.id,
            source = %envelope.source,
            "Dispatching command"
        );

        let command_id = envelope.id;

        // 2. Execute handler -> get CommandOutcome
        let outcome = match envelope.command {
            DomainCommand::Task(cmd) => self.task_handler.handle(cmd).await?,
            DomainCommand::Goal(cmd) => self.goal_handler.handle(cmd).await?,
            DomainCommand::Memory(cmd) => self.memory_handler.handle(cmd).await?,
        };

        // 3. Journal + broadcast events via EventBus (journal-first: EventBus
        //    persists to the store before broadcasting to in-memory subscribers)
        for event in outcome.events {
            self.event_bus.publish(event).await;
        }

        // 4. Record command ID in dedup cache (in-memory + persistent)
        self.processed_commands.insert(command_id, ()).await;
        if let Some(ref pool) = self.pool {
            let id_str = command_id.0.to_string();
            let now = chrono::Utc::now().to_rfc3339();
            let _ = sqlx::query(
                "INSERT OR IGNORE INTO processed_commands (command_id, processed_at) VALUES (?, ?)",
            )
            .bind(&id_str)
            .bind(&now)
            .execute(pool)
            .await;
        }

        // 5. Return result
        Ok(outcome.result)
    }

    /// Prune processed command entries older than the given duration.
    pub async fn prune_old_commands(&self, older_than: std::time::Duration) -> u64 {
        if let Some(ref pool) = self.pool {
            let cutoff = (chrono::Utc::now()
                - chrono::Duration::from_std(older_than).unwrap_or_default())
            .to_rfc3339();
            match sqlx::query("DELETE FROM processed_commands WHERE processed_at < ?")
                .bind(&cutoff)
                .execute(pool)
                .await
            {
                Ok(result) => result.rows_affected(),
                Err(e) => {
                    tracing::warn!("Failed to prune processed commands: {}", e);
                    0
                }
            }
        } else {
            0
        }
    }
}

impl fmt::Debug for CommandBus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CommandBus").finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_command_envelope_creation() {
        let envelope = CommandEnvelope::new(
            CommandSource::Human,
            DomainCommand::Task(TaskCommand::Retry {
                task_id: Uuid::new_v4(),
            }),
        );

        assert!(envelope.correlation_id.is_none());
        assert!(matches!(envelope.source, CommandSource::Human));
    }

    #[test]
    fn test_command_envelope_with_correlation() {
        let corr_id = Uuid::new_v4();
        let envelope = CommandEnvelope::new(
            CommandSource::EventHandler("test-handler".into()),
            DomainCommand::Goal(GoalCommand::Delete {
                goal_id: Uuid::new_v4(),
            }),
        )
        .with_correlation(corr_id);

        assert_eq!(envelope.correlation_id, Some(corr_id));
    }
}
