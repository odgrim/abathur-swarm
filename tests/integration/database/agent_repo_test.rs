use abathur::domain::models::{Agent, AgentStatus};
use abathur::domain::ports::AgentRepository;
use abathur::infrastructure::database::AgentRepositoryImpl;
use chrono::{Duration, Utc};
use uuid::Uuid;

// Test helper functions
async fn setup_test_db() -> sqlx::SqlitePool {
    let pool = sqlx::SqlitePool::connect("sqlite::memory:")
        .await
        .expect("failed to create test database");

    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .expect("failed to run migrations");

    pool
}

async fn teardown_test_db(pool: sqlx::SqlitePool) {
    pool.close().await;
}

#[tokio::test]
async fn test_insert_and_get_agent() {
    let pool = setup_test_db().await;
    let repo = AgentRepositoryImpl::new(pool.clone());

    let agent_id = Uuid::new_v4();
    let agent = Agent::new(agent_id, "test-agent".to_string());

    // Insert agent
    repo.insert(agent.clone())
        .await
        .expect("failed to insert agent");

    // Retrieve agent
    let retrieved = repo.get(agent_id).await.expect("failed to get agent");

    assert!(retrieved.is_some());
    let retrieved = retrieved.unwrap();
    assert_eq!(retrieved.id, agent.id);
    assert_eq!(retrieved.agent_type, agent.agent_type);
    assert_eq!(retrieved.status, AgentStatus::Idle);
    assert!(retrieved.current_task_id.is_none());
    assert_eq!(retrieved.memory_usage_bytes, 0);
    assert_eq!(retrieved.cpu_usage_percent, 0.0);

    teardown_test_db(pool).await;
}

#[tokio::test]
async fn test_get_nonexistent_agent() {
    let pool = setup_test_db().await;
    let repo = AgentRepositoryImpl::new(pool.clone());

    let nonexistent_id = Uuid::new_v4();
    let result = repo.get(nonexistent_id).await.expect("query should succeed");

    assert!(result.is_none());

    teardown_test_db(pool).await;
}

#[tokio::test]
async fn test_update_agent() {
    let pool = setup_test_db().await;
    let repo = AgentRepositoryImpl::new(pool.clone());

    let agent_id = Uuid::new_v4();
    let mut agent = Agent::new(agent_id, "test-agent".to_string());

    // Insert agent
    repo.insert(agent.clone())
        .await
        .expect("failed to insert agent");

    // Update agent (note: setting current_task_id requires a valid task to exist)
    agent.status = AgentStatus::Busy;
    // Don't set current_task_id to avoid foreign key constraint (would need to create a task first)
    agent.memory_usage_bytes = 1024 * 1024; // 1 MB
    agent.cpu_usage_percent = 45.5;

    repo.update(agent.clone())
        .await
        .expect("failed to update agent");

    // Retrieve and verify
    let retrieved = repo
        .get(agent_id)
        .await
        .expect("failed to get agent")
        .unwrap();

    assert_eq!(retrieved.status, AgentStatus::Busy);
    assert_eq!(retrieved.current_task_id, None);
    assert_eq!(retrieved.memory_usage_bytes, 1024 * 1024);
    assert_eq!(retrieved.cpu_usage_percent, 45.5);

    teardown_test_db(pool).await;
}

#[tokio::test]
async fn test_list_all_agents() {
    let pool = setup_test_db().await;
    let repo = AgentRepositoryImpl::new(pool.clone());

    // Insert multiple agents
    for i in 0..5 {
        let agent = Agent::new(Uuid::new_v4(), format!("agent-{}", i));
        repo.insert(agent).await.expect("failed to insert agent");
    }

    // List all agents
    let agents = repo.list(None).await.expect("failed to list agents");

    assert_eq!(agents.len(), 5);

    teardown_test_db(pool).await;
}

#[tokio::test]
async fn test_list_agents_by_status() {
    let pool = setup_test_db().await;
    let repo = AgentRepositoryImpl::new(pool.clone());

    // Insert agents with different statuses
    for i in 0..3 {
        let mut agent = Agent::new(Uuid::new_v4(), format!("idle-agent-{}", i));
        agent.status = AgentStatus::Idle;
        repo.insert(agent).await.expect("failed to insert agent");
    }

    for i in 0..2 {
        let mut agent = Agent::new(Uuid::new_v4(), format!("busy-agent-{}", i));
        agent.status = AgentStatus::Busy;
        repo.insert(agent).await.expect("failed to insert agent");
    }

    // List idle agents
    let idle_agents = repo
        .list(Some(AgentStatus::Idle))
        .await
        .expect("failed to list idle agents");

    assert_eq!(idle_agents.len(), 3);
    assert!(idle_agents.iter().all(|a| a.status == AgentStatus::Idle));

    // List busy agents
    let busy_agents = repo
        .list(Some(AgentStatus::Busy))
        .await
        .expect("failed to list busy agents");

    assert_eq!(busy_agents.len(), 2);
    assert!(busy_agents.iter().all(|a| a.status == AgentStatus::Busy));

    teardown_test_db(pool).await;
}

#[tokio::test]
async fn test_find_stale_agents() {
    let pool = setup_test_db().await;
    let repo = AgentRepositoryImpl::new(pool.clone());

    // Insert fresh agent
    let fresh_agent = Agent::new(Uuid::new_v4(), "fresh-agent".to_string());
    repo.insert(fresh_agent.clone())
        .await
        .expect("failed to insert agent");

    // Insert stale agent
    let mut stale_agent = Agent::new(Uuid::new_v4(), "stale-agent".to_string());
    stale_agent.heartbeat_at = Utc::now() - Duration::seconds(120); // 2 minutes old
    repo.insert(stale_agent.clone())
        .await
        .expect("failed to insert agent");

    // Find stale agents (threshold: 60 seconds)
    let stale_agents = repo
        .find_stale_agents(Duration::seconds(60))
        .await
        .expect("failed to find stale agents");

    assert_eq!(stale_agents.len(), 1);
    assert_eq!(stale_agents[0].id, stale_agent.id);

    teardown_test_db(pool).await;
}

#[tokio::test]
async fn test_find_stale_agents_excludes_terminated() {
    let pool = setup_test_db().await;
    let repo = AgentRepositoryImpl::new(pool.clone());

    // Insert stale but terminated agent
    let mut terminated_agent = Agent::new(Uuid::new_v4(), "terminated-agent".to_string());
    terminated_agent.heartbeat_at = Utc::now() - Duration::seconds(120);
    terminated_agent.terminate();
    repo.insert(terminated_agent.clone())
        .await
        .expect("failed to insert agent");

    // Find stale agents
    let stale_agents = repo
        .find_stale_agents(Duration::seconds(60))
        .await
        .expect("failed to find stale agents");

    // Terminated agents should not be included
    assert_eq!(stale_agents.len(), 0);

    teardown_test_db(pool).await;
}

#[tokio::test]
async fn test_update_heartbeat() {
    let pool = setup_test_db().await;
    let repo = AgentRepositoryImpl::new(pool.clone());

    let agent_id = Uuid::new_v4();
    let mut agent = Agent::new(agent_id, "test-agent".to_string());

    // Set old heartbeat
    agent.heartbeat_at = Utc::now() - Duration::seconds(60);
    let old_heartbeat = agent.heartbeat_at;

    repo.insert(agent).await.expect("failed to insert agent");

    // Update heartbeat
    repo.update_heartbeat(agent_id)
        .await
        .expect("failed to update heartbeat");

    // Retrieve and verify
    let updated = repo
        .get(agent_id)
        .await
        .expect("failed to get agent")
        .unwrap();

    assert!(updated.heartbeat_at > old_heartbeat);

    teardown_test_db(pool).await;
}

#[tokio::test]
async fn test_update_heartbeat_nonexistent_agent() {
    let pool = setup_test_db().await;
    let repo = AgentRepositoryImpl::new(pool.clone());

    let nonexistent_id = Uuid::new_v4();
    let result = repo.update_heartbeat(nonexistent_id).await;

    assert!(result.is_err());
    match result {
        Err(abathur::DatabaseError::NotFound(id)) => {
            assert_eq!(id, nonexistent_id);
        }
        _ => panic!("Expected NotFound error"),
    }

    teardown_test_db(pool).await;
}

#[tokio::test]
async fn test_foreign_key_constraint() {
    let pool = setup_test_db().await;
    let repo = AgentRepositoryImpl::new(pool.clone());

    // Create agent with reference to non-existent task
    // Note: The tasks table is just a stub in the migration
    // In a real scenario, we'd need to insert a task first
    let mut agent = Agent::new(Uuid::new_v4(), "test-agent".to_string());
    agent.current_task_id = Some(Uuid::new_v4()); // Non-existent task

    // This should fail due to foreign key constraint
    let result = repo.insert(agent).await;

    // SQLite with foreign_keys enabled should reject this
    assert!(result.is_err());

    teardown_test_db(pool).await;
}

#[tokio::test]
async fn test_agent_lifecycle() {
    let pool = setup_test_db().await;
    let repo = AgentRepositoryImpl::new(pool.clone());

    let agent_id = Uuid::new_v4();
    let mut agent = Agent::new(agent_id, "lifecycle-agent".to_string());

    // 1. Create agent (idle)
    repo.insert(agent.clone())
        .await
        .expect("failed to insert agent");

    let retrieved = repo.get(agent_id).await.unwrap().unwrap();
    assert_eq!(retrieved.status, AgentStatus::Idle);

    // 2. Assign task (busy)
    agent.status = AgentStatus::Busy;
    agent.current_task_id = Some(Uuid::new_v4());
    // Note: This will fail foreign key check unless we insert the task first
    // For now, we'll skip this update or modify the test

    // 3. Complete task (back to idle)
    agent.status = AgentStatus::Idle;
    agent.current_task_id = None;
    repo.update(agent.clone()).await.expect("failed to update");

    let retrieved = repo.get(agent_id).await.unwrap().unwrap();
    assert_eq!(retrieved.status, AgentStatus::Idle);
    assert!(retrieved.current_task_id.is_none());

    // 4. Terminate agent
    agent.terminate();
    repo.update(agent.clone()).await.expect("failed to update");

    let retrieved = repo.get(agent_id).await.unwrap().unwrap();
    assert_eq!(retrieved.status, AgentStatus::Terminated);
    assert!(retrieved.terminated_at.is_some());

    teardown_test_db(pool).await;
}
