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
    Goal, GoalConstraint, GoalPriority, GoalStatus, Memory, MemoryMetadata, MemoryTier,
    MemoryType, Task, TaskContext, TaskPriority, TaskSource, TaskStatus,
};
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
}

impl fmt::Display for CommandSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Human => write!(f, "human"),
            Self::System => write!(f, "system"),
            Self::EventHandler(name) => write!(f, "handler:{}", name),
            Self::Scheduler(name) => write!(f, "scheduler:{}", name),
            Self::A2A(swarm) => write!(f, "a2a:{}", swarm),
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
    async fn handle(&self, cmd: TaskCommand) -> Result<CommandResult, CommandError>;
}

/// Handler for goal commands.
#[async_trait]
pub trait GoalCommandHandler: Send + Sync {
    async fn handle(&self, cmd: GoalCommand) -> Result<CommandResult, CommandError>;
}

/// Handler for memory commands.
#[async_trait]
pub trait MemoryCommandHandler: Send + Sync {
    async fn handle(&self, cmd: MemoryCommand) -> Result<CommandResult, CommandError>;
}

// ---------------------------------------------------------------------------
// CommandBus
// ---------------------------------------------------------------------------

/// Central command dispatcher.
///
/// Holds one handler per domain and routes `DomainCommand` variants to them.
/// Includes an LRU dedup cache to prevent duplicate command processing
/// (important for replay scenarios).
pub struct CommandBus {
    task_handler: Arc<dyn TaskCommandHandler>,
    goal_handler: Arc<dyn GoalCommandHandler>,
    memory_handler: Arc<dyn MemoryCommandHandler>,
    /// Cache of recently processed command IDs for deduplication (capacity: 1000).
    processed_commands: moka::future::Cache<CommandId, ()>,
}

impl CommandBus {
    pub fn new(
        task_handler: Arc<dyn TaskCommandHandler>,
        goal_handler: Arc<dyn GoalCommandHandler>,
        memory_handler: Arc<dyn MemoryCommandHandler>,
    ) -> Self {
        Self {
            task_handler,
            goal_handler,
            memory_handler,
            processed_commands: moka::future::Cache::builder()
                .max_capacity(1000)
                .time_to_live(std::time::Duration::from_secs(3600))
                .build(),
        }
    }

    /// Dispatch a command envelope to the appropriate handler.
    pub async fn dispatch(
        &self,
        envelope: CommandEnvelope<DomainCommand>,
    ) -> Result<CommandResult, CommandError> {
        // Check dedup cache
        if self.processed_commands.get(&envelope.id).await.is_some() {
            return Err(CommandError::DuplicateCommand(envelope.id));
        }

        tracing::debug!(
            command_id = %envelope.id,
            source = %envelope.source,
            "Dispatching command"
        );

        let command_id = envelope.id;
        let result = match envelope.command {
            DomainCommand::Task(cmd) => self.task_handler.handle(cmd).await,
            DomainCommand::Goal(cmd) => self.goal_handler.handle(cmd).await,
            DomainCommand::Memory(cmd) => self.memory_handler.handle(cmd).await,
        };

        // Record in dedup cache on success
        if result.is_ok() {
            self.processed_commands.insert(command_id, ()).await;
        }

        result
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
