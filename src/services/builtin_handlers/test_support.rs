//! Shared test fixtures for built-in handler tests.

#![allow(unused_imports)]
#![allow(dead_code)]

use std::sync::Arc;

use crate::adapters::sqlite::{create_migrated_test_pool, task_repository::SqliteTaskRepository};
use crate::services::task_service::TaskService;

pub async fn setup_task_repo() -> Arc<SqliteTaskRepository> {
    let pool = create_migrated_test_pool().await.unwrap();
    Arc::new(SqliteTaskRepository::new(pool))
}

pub fn make_task_service(
    repo: &Arc<SqliteTaskRepository>,
) -> Arc<TaskService<SqliteTaskRepository>> {
    Arc::new(TaskService::new(repo.clone()))
}
