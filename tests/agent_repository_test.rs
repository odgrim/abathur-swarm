mod helpers;

use abathur::domain::models::{Agent, AgentStatus};
use abathur::domain::ports::AgentRepository;
use abathur::infrastructure::database::AgentRepositoryImpl;
use chrono::{Duration, Utc};
use uuid::Uuid;

use helpers::database::{setup_test_db, teardown_test_db};

fn create_test_agent(agent_type: &str, status: AgentStatus) -> Agent {
    Agent {
        id: Uuid::new_v4(),
        agent_type: agent_type.to_string(),
        status,
        current_task_id: None,
        heartbeat_at: Utc::now(),
        memory_usage_bytes: 1024 * 1024, // 1 MB
        cpu_usage_percent: 25.0,
        created_at: Utc::now(),
        terminated_at: None,
    }
}

#[tokio::test]
async fn test_insert_and_get_agent() {
    let pool = setup_test_db().await;
    let repo = AgentRepositoryImpl::new(pool.clone());

    let agent = create_test_agent("rust-specialist", AgentStatus::Idle);
    let agent_id = agent.id;

    // Insert agent
    repo.insert(agent.clone())
        .await
        .expect("failed to insert agent");

    // Get agent
    let retrieved = repo.get(agent_id).await.expect("failed to get agent");
    assert!(retrieved.is_some());

    let retrieved = retrieved.unwrap();
    assert_eq!(retrieved.id, agent_id);
    assert_eq!(retrieved.agent_type, "rust-specialist");
    assert_eq!(retrieved.status, AgentStatus::Idle);
    assert_eq!(retrieved.memory_usage_bytes, 1024 * 1024);
    assert_eq!(retrieved.cpu_usage_percent, 25.0);

    teardown_test_db(pool).await;
}

#[tokio::test]
async fn test_get_nonexistent_agent() {
    let pool = setup_test_db().await;
    let repo = AgentRepositoryImpl::new(pool.clone());

    let nonexistent_id = Uuid::new_v4();
    let result = repo.get(nonexistent_id).await.expect("failed to query");
    assert!(result.is_none());

    teardown_test_db(pool).await;
}

#[tokio::test]
async fn test_update_agent() {
    let pool = setup_test_db().await;
    let repo = AgentRepositoryImpl::new(pool.clone());

    let mut agent = create_test_agent("test-agent", AgentStatus::Idle);
    repo.insert(agent.clone())
        .await
        .expect("failed to insert agent");

    // Update agent
    agent.status = AgentStatus::Busy;
    agent.current_task_id = Some(Uuid::new_v4());
    agent.memory_usage_bytes = 2 * 1024 * 1024; // 2 MB
    agent.cpu_usage_percent = 75.0;

    repo.update(agent.clone())
        .await
        .expect("failed to update agent");

    // Verify update
    let retrieved = repo.get(agent.id).await.expect("failed to get agent").unwrap();
    assert_eq!(retrieved.status, AgentStatus::Busy);
    assert_eq!(retrieved.current_task_id, agent.current_task_id);
    assert_eq!(retrieved.memory_usage_bytes, 2 * 1024 * 1024);
    assert_eq!(retrieved.cpu_usage_percent, 75.0);

    teardown_test_db(pool).await;
}

#[tokio::test]
async fn test_list_all_agents() {
    let pool = setup_test_db().await;
    let repo = AgentRepositoryImpl::new(pool.clone());

    // Insert multiple agents
    let agent1 = create_test_agent("rust-specialist", AgentStatus::Idle);
    let agent2 = create_test_agent("python-specialist", AgentStatus::Busy);
    let agent3 = create_test_agent("general-purpose", AgentStatus::Terminating);

    repo.insert(agent1).await.expect("failed to insert agent1");
    repo.insert(agent2).await.expect("failed to insert agent2");
    repo.insert(agent3).await.expect("failed to insert agent3");

    // List all agents
    let all_agents = repo.list(None).await.expect("failed to list agents");
    assert_eq!(all_agents.len(), 3);

    teardown_test_db(pool).await;
}

#[tokio::test]
async fn test_list_agents_by_status() {
    let pool = setup_test_db().await;
    let repo = AgentRepositoryImpl::new(pool.clone());

    // Insert agents with different statuses
    let agent1 = create_test_agent("agent1", AgentStatus::Idle);
    let agent2 = create_test_agent("agent2", AgentStatus::Idle);
    let agent3 = create_test_agent("agent3", AgentStatus::Busy);
    let agent4 = create_test_agent("agent4", AgentStatus::Terminating);

    repo.insert(agent1).await.expect("failed to insert agent1");
    repo.insert(agent2).await.expect("failed to insert agent2");
    repo.insert(agent3).await.expect("failed to insert agent3");
    repo.insert(agent4).await.expect("failed to insert agent4");

    // List idle agents
    let idle_agents = repo
        .list(Some(AgentStatus::Idle))
        .await
        .expect("failed to list idle agents");
    assert_eq!(idle_agents.len(), 2);

    // List busy agents
    let busy_agents = repo
        .list(Some(AgentStatus::Busy))
        .await
        .expect("failed to list busy agents");
    assert_eq!(busy_agents.len(), 1);
    assert_eq!(busy_agents[0].agent_type, "agent3");

    teardown_test_db(pool).await;
}

#[tokio::test]
async fn test_update_heartbeat() {
    let pool = setup_test_db().await;
    let repo = AgentRepositoryImpl::new(pool.clone());

    let agent = create_test_agent("test-agent", AgentStatus::Idle);
    let agent_id = agent.id;
    let original_heartbeat = agent.heartbeat_at;

    repo.insert(agent)
        .await
        .expect("failed to insert agent");

    // Wait a moment to ensure time difference
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    // Update heartbeat
    repo.update_heartbeat(agent_id)
        .await
        .expect("failed to update heartbeat");

    // Verify heartbeat was updated
    let retrieved = repo.get(agent_id).await.expect("failed to get agent").unwrap();
    assert!(retrieved.heartbeat_at > original_heartbeat);

    teardown_test_db(pool).await;
}

#[tokio::test]
async fn test_find_stale_agents() {
    let pool = setup_test_db().await;
    let repo = AgentRepositoryImpl::new(pool.clone());

    // Insert an agent with recent heartbeat
    let fresh_agent = create_test_agent("fresh-agent", AgentStatus::Busy);
    repo.insert(fresh_agent.clone())
        .await
        .expect("failed to insert fresh agent");

    // Insert an agent with old heartbeat
    let mut stale_agent = create_test_agent("stale-agent", AgentStatus::Busy);
    stale_agent.heartbeat_at = Utc::now() - Duration::hours(2);
    repo.insert(stale_agent.clone())
        .await
        .expect("failed to insert stale agent");

    // Find stale agents (older than 1 hour)
    let threshold = Duration::hours(1);
    let stale_agents = repo
        .find_stale_agents(threshold)
        .await
        .expect("failed to find stale agents");

    assert_eq!(stale_agents.len(), 1);
    assert_eq!(stale_agents[0].id, stale_agent.id);
    assert_eq!(stale_agents[0].agent_type, "stale-agent");

    teardown_test_db(pool).await;
}

#[tokio::test]
async fn test_agent_with_current_task() {
    let pool = setup_test_db().await;
    let repo = AgentRepositoryImpl::new(pool.clone());

    let task_id = Uuid::new_v4();
    let mut agent = create_test_agent("test-agent", AgentStatus::Busy);
    agent.current_task_id = Some(task_id);

    repo.insert(agent.clone())
        .await
        .expect("failed to insert agent");

    let retrieved = repo.get(agent.id).await.expect("failed to get agent").unwrap();
    assert_eq!(retrieved.current_task_id, Some(task_id));

    teardown_test_db(pool).await;
}

#[tokio::test]
async fn test_agent_termination() {
    let pool = setup_test_db().await;
    let repo = AgentRepositoryImpl::new(pool.clone());

    let mut agent = create_test_agent("test-agent", AgentStatus::Idle);
    repo.insert(agent.clone())
        .await
        .expect("failed to insert agent");

    // Terminate agent
    agent.status = AgentStatus::Terminated;
    agent.terminated_at = Some(Utc::now());

    repo.update(agent.clone())
        .await
        .expect("failed to update agent");

    // Verify termination
    let retrieved = repo.get(agent.id).await.expect("failed to get agent").unwrap();
    assert_eq!(retrieved.status, AgentStatus::Terminated);
    assert!(retrieved.terminated_at.is_some());

    teardown_test_db(pool).await;
}

#[tokio::test]
async fn test_agent_resource_usage() {
    let pool = setup_test_db().await;
    let repo = AgentRepositoryImpl::new(pool.clone());

    let mut agent = create_test_agent("test-agent", AgentStatus::Busy);
    agent.memory_usage_bytes = 512 * 1024 * 1024; // 512 MB
    agent.cpu_usage_percent = 95.5;

    repo.insert(agent.clone())
        .await
        .expect("failed to insert agent");

    let retrieved = repo.get(agent.id).await.expect("failed to get agent").unwrap();
    assert_eq!(retrieved.memory_usage_bytes, 512 * 1024 * 1024);
    assert_eq!(retrieved.cpu_usage_percent, 95.5);

    teardown_test_db(pool).await;
}

#[tokio::test]
async fn test_multiple_agent_types() {
    let pool = setup_test_db().await;
    let repo = AgentRepositoryImpl::new(pool.clone());

    let agent_types = vec![
        "rust-specialist",
        "python-specialist",
        "javascript-specialist",
        "general-purpose",
        "code-reviewer",
    ];

    for agent_type in &agent_types {
        let agent = create_test_agent(agent_type, AgentStatus::Idle);
        repo.insert(agent)
            .await
            .expect("failed to insert agent");
    }

    let all_agents = repo.list(None).await.expect("failed to list agents");
    assert_eq!(all_agents.len(), agent_types.len());

    let agent_types_retrieved: Vec<String> = all_agents
        .into_iter()
        .map(|a| a.agent_type)
        .collect();

    for agent_type in agent_types {
        assert!(agent_types_retrieved.contains(&agent_type.to_string()));
    }

    teardown_test_db(pool).await;
}
