//! Test-only helpers for wiring up SQLite-backed repositories and services.
//!
//! These helpers exist so that service-layer tests do not need to import
//! concrete SQLite adapters directly: they can call a single function from
//! here and get back ready-to-use repositories/services without ever
//! naming `crate::adapters::sqlite::*` in their own test modules.
//!
//! The hexagonal boundary: services depend only on domain ports. Tests that
//! need a real database go through this module rather than reaching into
//! `adapters::sqlite` themselves.

#![allow(dead_code)]

use std::sync::Arc;

use sqlx::SqlitePool;

use crate::adapters::sqlite::{
    SqliteAgentRepository, SqliteGoalRepository, SqliteMemoryRepository, SqliteTaskRepository,
    SqliteWorktreeRepository, create_migrated_test_pool,
};

// ---------------------------------------------------------------------------
// Concrete test-backend type aliases
//
// These aliases let test modules avoid naming `SqliteXxxRepository` directly
// while still being able to use it for turbofish-style type arguments
// (e.g. `Service::<TestTaskRepo>::associated_fn(...)`).
// ---------------------------------------------------------------------------

pub type TestTaskRepo = SqliteTaskRepository;
pub type TestGoalRepo = SqliteGoalRepository;
pub type TestMemoryRepo = SqliteMemoryRepository;
pub type TestAgentRepo = SqliteAgentRepository;
pub type TestWorktreeRepo = SqliteWorktreeRepository;

// ---------------------------------------------------------------------------
// Pool helpers
// ---------------------------------------------------------------------------

/// Create a fully-migrated in-memory pool for tests. Panics on failure, since
/// migration failures in a test harness mean the test binary is broken.
pub async fn setup_pool() -> SqlitePool {
    create_migrated_test_pool()
        .await
        .expect("migrated test pool")
}

// ---------------------------------------------------------------------------
// Single-repo helpers
// ---------------------------------------------------------------------------

pub async fn setup_task_repo() -> Arc<SqliteTaskRepository> {
    let pool = setup_pool().await;
    Arc::new(SqliteTaskRepository::new(pool))
}

pub async fn setup_goal_repo() -> Arc<SqliteGoalRepository> {
    let pool = setup_pool().await;
    Arc::new(SqliteGoalRepository::new(pool))
}

pub async fn setup_memory_repo() -> Arc<SqliteMemoryRepository> {
    let pool = setup_pool().await;
    Arc::new(SqliteMemoryRepository::new(pool))
}

pub async fn setup_agent_repo() -> Arc<SqliteAgentRepository> {
    let pool = setup_pool().await;
    Arc::new(SqliteAgentRepository::new(pool))
}

pub async fn setup_worktree_repo() -> Arc<SqliteWorktreeRepository> {
    let pool = setup_pool().await;
    Arc::new(SqliteWorktreeRepository::new(pool))
}

// ---------------------------------------------------------------------------
// Same-pool multi-repo helpers (FK constraints need to share a pool)
// ---------------------------------------------------------------------------

pub async fn setup_task_and_memory_repos() -> (Arc<SqliteTaskRepository>, Arc<SqliteMemoryRepository>)
{
    let pool = setup_pool().await;
    (
        Arc::new(SqliteTaskRepository::new(pool.clone())),
        Arc::new(SqliteMemoryRepository::new(pool)),
    )
}

pub async fn setup_goal_and_task_repos() -> (Arc<SqliteGoalRepository>, Arc<SqliteTaskRepository>) {
    let pool = setup_pool().await;
    (
        Arc::new(SqliteGoalRepository::new(pool.clone())),
        Arc::new(SqliteTaskRepository::new(pool)),
    )
}

pub async fn setup_task_goal_memory_repos() -> (
    Arc<SqliteTaskRepository>,
    Arc<SqliteGoalRepository>,
    Arc<SqliteMemoryRepository>,
) {
    let pool = setup_pool().await;
    (
        Arc::new(SqliteTaskRepository::new(pool.clone())),
        Arc::new(SqliteGoalRepository::new(pool.clone())),
        Arc::new(SqliteMemoryRepository::new(pool)),
    )
}

pub async fn setup_task_agent_goal_repos() -> (
    Arc<SqliteTaskRepository>,
    Arc<SqliteAgentRepository>,
    Arc<SqliteGoalRepository>,
) {
    let pool = setup_pool().await;
    (
        Arc::new(SqliteTaskRepository::new(pool.clone())),
        Arc::new(SqliteAgentRepository::new(pool.clone())),
        Arc::new(SqliteGoalRepository::new(pool)),
    )
}

pub async fn setup_all_repos() -> (
    Arc<SqliteGoalRepository>,
    Arc<SqliteTaskRepository>,
    Arc<SqliteWorktreeRepository>,
    Arc<SqliteAgentRepository>,
    Arc<SqliteMemoryRepository>,
) {
    let pool = setup_pool().await;
    (
        Arc::new(SqliteGoalRepository::new(pool.clone())),
        Arc::new(SqliteTaskRepository::new(pool.clone())),
        Arc::new(SqliteWorktreeRepository::new(pool.clone())),
        Arc::new(SqliteAgentRepository::new(pool.clone())),
        Arc::new(SqliteMemoryRepository::new(pool)),
    )
}

// ---------------------------------------------------------------------------
// Service builders
// ---------------------------------------------------------------------------

pub fn make_task_service(
    repo: &Arc<SqliteTaskRepository>,
) -> Arc<crate::services::task_service::TaskService<SqliteTaskRepository>> {
    Arc::new(crate::services::task_service::TaskService::new(repo.clone()))
}

pub fn make_goal_service(
    repo: &Arc<SqliteGoalRepository>,
) -> Arc<crate::services::goal_service::GoalService<SqliteGoalRepository>> {
    Arc::new(crate::services::goal_service::GoalService::new(repo.clone()))
}

pub fn make_memory_service(
    repo: &Arc<SqliteMemoryRepository>,
) -> Arc<crate::services::memory_service::MemoryService<SqliteMemoryRepository>> {
    Arc::new(crate::services::memory_service::MemoryService::new(
        repo.clone(),
    ))
}

/// Fresh migrated pool + memory service in one call.
pub async fn setup_memory_service()
-> crate::services::memory_service::MemoryService<SqliteMemoryRepository> {
    let repo = setup_memory_repo().await;
    crate::services::memory_service::MemoryService::new(repo)
}

/// Fresh migrated pool + task service in one call.
pub async fn setup_task_service()
-> crate::services::task_service::TaskService<SqliteTaskRepository> {
    let repo = setup_task_repo().await;
    crate::services::task_service::TaskService::new(repo)
}

/// Fresh migrated pool + goal service in one call.
pub async fn setup_goal_service()
-> crate::services::goal_service::GoalService<SqliteGoalRepository> {
    let repo = setup_goal_repo().await;
    crate::services::goal_service::GoalService::new(repo)
}

/// Fresh migrated pool + worktree service in one call.
pub async fn setup_worktree_service(
    config: crate::services::worktree_service::WorktreeConfig,
) -> crate::services::worktree_service::WorktreeService<SqliteWorktreeRepository> {
    let repo = setup_worktree_repo().await;
    crate::services::worktree_service::WorktreeService::new(repo, config)
}

/// Build a `CommandBus` wired to the given repos. Uses an in-memory event bus
/// with persistence disabled.
pub fn make_command_bus(
    task_repo: &Arc<SqliteTaskRepository>,
    goal_repo: &Arc<SqliteGoalRepository>,
    memory_repo: &Arc<SqliteMemoryRepository>,
) -> Arc<crate::services::command_bus::CommandBus> {
    let task_service = make_task_service(task_repo);
    let goal_service = make_goal_service(goal_repo);
    let memory_service = make_memory_service(memory_repo);
    let event_bus = Arc::new(crate::services::EventBus::new(
        crate::services::EventBusConfig {
            persist_events: false,
            ..Default::default()
        },
    ));
    Arc::new(crate::services::command_bus::CommandBus::new(
        task_service,
        goal_service,
        memory_service,
        event_bus,
    ))
}

// ---------------------------------------------------------------------------
// Port-typed dyn helpers (for code that specifically wants `Arc<dyn Port>`)
// ---------------------------------------------------------------------------

pub async fn setup_dyn_task_repo() -> Arc<dyn crate::domain::ports::TaskRepository> {
    setup_task_repo().await
}

pub async fn setup_dyn_goal_repo() -> Arc<dyn crate::domain::ports::GoalRepository> {
    setup_goal_repo().await
}

pub async fn setup_dyn_agent_repo() -> Arc<dyn crate::domain::ports::AgentRepository> {
    setup_agent_repo().await
}

pub async fn setup_dyn_worktree_repo() -> Arc<dyn crate::domain::ports::WorktreeRepository> {
    setup_worktree_repo().await
}

/// Tuple of `Arc<dyn Port>` repositories all sharing a common lazy in-memory
/// pool. Used by swarm middleware tests that only need typecheckable
/// repositories and never exercise them.
pub fn lazy_dyn_repos_minimal() -> (
    Arc<dyn crate::domain::ports::TaskRepository>,
    Arc<dyn crate::domain::ports::AgentRepository>,
    Arc<dyn crate::domain::ports::GoalRepository>,
) {
    let pool = SqlitePool::connect_lazy("sqlite::memory:").expect("lazy pool");
    let task_repo: Arc<dyn crate::domain::ports::TaskRepository> =
        Arc::new(SqliteTaskRepository::new(pool.clone()));
    let agent_repo: Arc<dyn crate::domain::ports::AgentRepository> =
        Arc::new(SqliteAgentRepository::new(pool.clone()));
    let goal_repo: Arc<dyn crate::domain::ports::GoalRepository> =
        Arc::new(SqliteGoalRepository::new(pool));
    (task_repo, agent_repo, goal_repo)
}

/// Tuple of `Arc<dyn Port>` repositories suitable for
/// `PostCompletionContext` in swarm middleware tests.
pub fn lazy_dyn_repos_post_completion() -> (
    Arc<dyn crate::domain::ports::TaskRepository>,
    Arc<dyn crate::domain::ports::GoalRepository>,
    Arc<dyn crate::domain::ports::WorktreeRepository>,
) {
    let pool = SqlitePool::connect_lazy("sqlite::memory:").expect("lazy pool");
    let task_repo: Arc<dyn crate::domain::ports::TaskRepository> =
        Arc::new(SqliteTaskRepository::new(pool.clone()));
    let goal_repo: Arc<dyn crate::domain::ports::GoalRepository> =
        Arc::new(SqliteGoalRepository::new(pool.clone()));
    let worktree_repo: Arc<dyn crate::domain::ports::WorktreeRepository> =
        Arc::new(SqliteWorktreeRepository::new(pool));
    (task_repo, goal_repo, worktree_repo)
}

// ---------------------------------------------------------------------------
// Adapter-internal helpers (for tests that need raw DB setup)
//
// These deliberately expose the pool so that tests requiring FK setup or
// other adapter-level concerns (e.g. `insert_test_task`) can obtain one
// without importing `adapters::sqlite` directly.
// ---------------------------------------------------------------------------

/// Returns an `(AgentRepository, pool)` pair for agent-service tests that
/// need to seed task rows directly to satisfy foreign-key constraints.
pub async fn setup_agent_repo_with_pool() -> (Arc<SqliteAgentRepository>, SqlitePool) {
    let pool = setup_pool().await;
    let repo = Arc::new(SqliteAgentRepository::new(pool.clone()));
    (repo, pool)
}

/// Insert a minimal task row so that foreign-key constraints referencing
/// `tasks(id)` are satisfied.
pub async fn insert_test_task(pool: &SqlitePool, task_id: uuid::Uuid) {
    crate::adapters::sqlite::insert_test_task(pool, task_id).await;
}
