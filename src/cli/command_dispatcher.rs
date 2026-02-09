//! Thin CLI dispatcher that routes domain mutations through the CommandBus
//! with `CommandSource::Human` tracking.

use std::sync::Arc;

use sqlx::SqlitePool;

use crate::adapters::sqlite::{
    goal_repository::SqliteGoalRepository, SqliteMemoryRepository, SqliteTaskRepository,
};
use crate::services::command_bus::{
    CommandBus, CommandEnvelope, CommandError, CommandResult, CommandSource, DomainCommand,
};
use crate::services::event_bus::EventBus;
use crate::services::goal_service::GoalService;
use crate::services::memory_service::MemoryService;
use crate::services::task_service::TaskService;

/// CLI command dispatcher that wraps a `CommandBus` and tags all commands
/// with `CommandSource::Human`.
pub struct CliCommandDispatcher {
    command_bus: Arc<CommandBus>,
}

impl CliCommandDispatcher {
    /// Build a dispatcher from a SQLite pool and a shared EventBus.
    pub fn new(pool: SqlitePool, event_bus: Arc<EventBus>) -> Self {
        let task_repo = Arc::new(SqliteTaskRepository::new(pool.clone()));
        let goal_repo = Arc::new(SqliteGoalRepository::new(pool.clone()));
        let memory_repo = Arc::new(SqliteMemoryRepository::new(pool.clone()));

        let task_service = Arc::new(TaskService::new(task_repo));
        let goal_service = Arc::new(GoalService::new(goal_repo));
        let memory_service = Arc::new(MemoryService::new(memory_repo));

        let command_bus = Arc::new(
            CommandBus::new(task_service, goal_service, memory_service, event_bus)
                .with_pool(pool),
        );

        Self { command_bus }
    }

    /// Dispatch a domain command with `CommandSource::Human`.
    pub async fn dispatch(&self, cmd: DomainCommand) -> Result<CommandResult, CommandError> {
        let envelope = CommandEnvelope::new(CommandSource::Human, cmd);
        self.command_bus.dispatch(envelope).await
    }
}
