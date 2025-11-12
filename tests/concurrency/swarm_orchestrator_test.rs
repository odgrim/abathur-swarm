//! Concurrency tests for SwarmOrchestrator
//!
//! Tests the async concurrency patterns using tokio primitives.

use abathur::application::resource_monitor::{ResourceLimits, ResourceMonitor};
use abathur::application::swarm_orchestrator::{SwarmOrchestrator, SwarmState};
use abathur::application::task_coordinator::TaskCoordinator;
use abathur::application::AgentExecutor;
use abathur::domain::models::{Config, DependencyType, Task, TaskSource, TaskStatus};
use abathur::domain::ports::{
    ClaudeClient, ClaudeError, ClaudeRequest, ClaudeResponse, ContentBlock, McpClient, McpError,
    McpToolRequest, McpToolResponse, MessageRequest, MessageResponse, PriorityCalculator,
    ResourceContent, ResourceInfo, TaskQueueService, ToolInfo, TokenUsage, Usage,
};
use abathur::services::DependencyResolver;
use anyhow::Result;
use async_trait::async_trait;
use chrono::Utc;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;
use uuid::Uuid;

// ========================
// Mock Implementations
// ========================

struct MockTaskQueue {
    tasks: Arc<StdMutex<HashMap<Uuid, Task>>>,
    get_count: Arc<AtomicUsize>,
}

impl MockTaskQueue {
    fn new() -> Self {
        Self {
            tasks: Arc::new(StdMutex::new(HashMap::new())),
            get_count: Arc::new(AtomicUsize::new(0)),
        }
    }

    fn add_task(&self, task: Task) {
        let mut tasks = self.tasks.lock().unwrap();
        tasks.insert(task.id, task);
    }

    fn get_call_count(&self) -> usize {
        self.get_count.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl TaskQueueService for MockTaskQueue {
    async fn get_task(&self, task_id: Uuid) -> Result<Task> {
        self.get_count.fetch_add(1, Ordering::SeqCst);
        let tasks = self.tasks.lock().unwrap();
        tasks
            .get(&task_id)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("Task not found"))
    }

    async fn get_tasks_by_status(&self, status: TaskStatus) -> Result<Vec<Task>> {
        let tasks = self.tasks.lock().unwrap();
        Ok(tasks
            .values()
            .filter(|t| t.status == status)
            .cloned()
            .collect())
    }

    async fn get_dependent_tasks(&self, _task_id: Uuid) -> Result<Vec<Task>> {
        Ok(vec![])
    }

    async fn get_children_by_parent(&self, _parent_id: Uuid) -> Result<Vec<Task>> {
        Ok(vec![])
    }

    async fn update_task_status(&self, task_id: Uuid, status: TaskStatus) -> Result<()> {
        let mut tasks = self.tasks.lock().unwrap();
        if let Some(task) = tasks.get_mut(&task_id) {
            task.status = status;
            Ok(())
        } else {
            Err(anyhow::anyhow!("Task not found"))
        }
    }

    async fn update_task_priority(&self, task_id: Uuid, priority: f64) -> Result<()> {
        let mut tasks = self.tasks.lock().unwrap();
        if let Some(task) = tasks.get_mut(&task_id) {
            task.calculated_priority = priority;
            Ok(())
        } else {
            Err(anyhow::anyhow!("Task not found"))
        }
    }

    async fn update_task(&self, task: &Task) -> Result<()> {
        let mut tasks = self.tasks.lock().unwrap();
        tasks.insert(task.id, task.clone());
        Ok(())
    }

    async fn mark_task_failed(&self, task_id: Uuid, error_message: String) -> Result<()> {
        let mut tasks = self.tasks.lock().unwrap();
        if let Some(task) = tasks.get_mut(&task_id) {
            task.status = TaskStatus::Failed;
            task.error_message = Some(error_message);
            Ok(())
        } else {
            Err(anyhow::anyhow!("Task not found"))
        }
    }

    async fn get_next_ready_task(&self) -> Result<Option<Task>> {
        let mut tasks = self.tasks.lock().unwrap();

        // Find and remove the first ready task
        let ready_task_id = tasks
            .values()
            .find(|t| t.status == TaskStatus::Ready)
            .map(|t| t.id);

        if let Some(task_id) = ready_task_id {
            if let Some(task) = tasks.get_mut(&task_id) {
                // Mark as pending to prevent re-selection
                task.status = TaskStatus::Pending;
                return Ok(Some(task.clone()));
            }
        }

        Ok(None)
    }
}

struct MockPriorityCalculator;

#[async_trait]
impl PriorityCalculator for MockPriorityCalculator {
    async fn calculate_priority(&self, task: &Task) -> Result<f64> {
        Ok(f64::from(task.priority))
    }

    async fn recalculate_priorities(&self, tasks: &[Task]) -> Result<Vec<(Uuid, f64)>> {
        Ok(tasks
            .iter()
            .map(|t| (t.id, f64::from(t.priority)))
            .collect())
    }
}

struct MockClaudeClient {
    call_count: Arc<AtomicU32>,
}

impl MockClaudeClient {
    fn new() -> Self {
        Self {
            call_count: Arc::new(AtomicU32::new(0)),
        }
    }
}

#[async_trait]
impl ClaudeClient for MockClaudeClient {
    async fn execute(&self, request: ClaudeRequest) -> Result<ClaudeResponse, ClaudeError> {
        self.call_count.fetch_add(1, Ordering::SeqCst);

        // Simulate some work
        tokio::time::sleep(Duration::from_millis(50)).await;

        Ok(ClaudeResponse {
            task_id: request.task_id,
            content: "Mock response".to_string(),
            stop_reason: "end_turn".to_string(),
            usage: TokenUsage {
                input_tokens: 100,
                output_tokens: 50,
            },
        })
    }

    async fn send_message(&self, _request: MessageRequest) -> Result<MessageResponse, ClaudeError> {
        Ok(MessageResponse {
            id: "mock-msg-id".to_string(),
            content: vec![ContentBlock {
                content_type: "text".to_string(),
                text: Some("Mock message response".to_string()),
            }],
            stop_reason: Some("end_turn".to_string()),
            usage: Usage {
                input_tokens: 100,
                output_tokens: 50,
            },
        })
    }

    async fn stream_message(
        &self,
        _request: MessageRequest,
    ) -> Result<
        Box<dyn futures::Stream<Item = Result<abathur::domain::ports::MessageChunk, ClaudeError>> + Send + Unpin>,
        ClaudeError,
    > {
        use abathur::domain::ports::MessageChunk;
        use futures::stream;

        let chunks = vec![Ok(MessageChunk {
            delta: Some("Mock streaming response".to_string()),
            stop_reason: Some("end_turn".to_string()),
        })];

        Ok(Box::new(stream::iter(chunks)))
    }

    async fn health_check(&self) -> Result<(), ClaudeError> {
        Ok(())
    }
}

struct MockMcpClient;

#[async_trait]
impl McpClient for MockMcpClient {
    async fn invoke_tool(&self, request: McpToolRequest) -> Result<McpToolResponse, McpError> {
        Ok(McpToolResponse {
            task_id: request.task_id,
            result: serde_json::json!({"success": true}),
            is_error: false,
        })
    }

    async fn call_tool(
        &self,
        _server: &str,
        _tool: &str,
        _args: serde_json::Value,
    ) -> Result<serde_json::Value, McpError> {
        Ok(serde_json::json!({"success": true}))
    }

    async fn list_tools(&self, _server_name: &str) -> Result<Vec<ToolInfo>, McpError> {
        Ok(vec![])
    }

    async fn read_resource(&self, _server: &str, uri: &str) -> Result<ResourceContent, McpError> {
        Ok(ResourceContent {
            uri: uri.to_string(),
            mime_type: Some("text/plain".to_string()),
            text: Some("Mock resource content".to_string()),
            blob: None,
        })
    }

    async fn list_resources(&self, _server: &str) -> Result<Vec<ResourceInfo>, McpError> {
        Ok(vec![])
    }

    async fn health_check(&self, _server_name: &str) -> Result<(), McpError> {
        Ok(())
    }
}

fn create_test_task(status: TaskStatus) -> Task {
    Task {
        id: Uuid::new_v4(),
        summary: "Test task".to_string(),
        description: "Test description".to_string(),
        agent_type: "test-agent".to_string(),
        priority: 5,
        calculated_priority: 5.0,
        status,
        dependencies: None,
        dependency_type: DependencyType::Sequential,
        dependency_depth: 0,
        input_data: None,
        result_data: None,
        error_message: None,
        retry_count: 0,
        max_retries: 3,
        max_execution_timeout_seconds: 3600,
        submitted_at: Utc::now(),
        started_at: None,
        completed_at: None,
        last_updated_at: Utc::now(),
        created_by: None,
        parent_task_id: None,
        session_id: None,
        source: TaskSource::Human,
        deadline: None,
        estimated_duration_seconds: None,
        feature_branch: None,
        task_branch: None,
        worktree_path: None,
    }
}

// ========================
// Tests
// ========================

#[tokio::test]
async fn test_swarm_orchestrator_start_stop() {
    let task_queue = Arc::new(MockTaskQueue::new());
    let dependency_resolver = Arc::new(DependencyResolver::new());
    let priority_calc = Arc::new(MockPriorityCalculator);

    let task_coordinator = Arc::new(TaskCoordinator::new(
        task_queue,
        dependency_resolver,
        priority_calc,
    ));

    let claude_client = Arc::new(MockClaudeClient::new());
    let mcp_client = Arc::new(MockMcpClient);
    let agent_executor = Arc::new(AgentExecutor::new(claude_client, mcp_client));

    let limits = ResourceLimits::default();
    let resource_monitor = Arc::new(ResourceMonitor::new(limits));

    let config = Config::default();

    let mut orchestrator = SwarmOrchestrator::new(
        5,
        task_coordinator,
        agent_executor,
        resource_monitor,
        config,
    );

    // Initially stopped
    assert_eq!(orchestrator.get_state().await, SwarmState::Stopped);

    // Start orchestrator
    orchestrator.start().await.unwrap();
    assert_eq!(orchestrator.get_state().await, SwarmState::Running);

    // Let it run briefly
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Stop orchestrator
    orchestrator.stop().await.unwrap();
    assert_eq!(orchestrator.get_state().await, SwarmState::Stopped);
}

#[tokio::test]
async fn test_swarm_respects_max_concurrency() {
    let task_queue = Arc::new(MockTaskQueue::new());

    // Add multiple ready tasks
    for _ in 0..20 {
        let task = create_test_task(TaskStatus::Ready);
        task_queue.add_task(task);
    }

    let dependency_resolver = Arc::new(DependencyResolver::new());
    let priority_calc = Arc::new(MockPriorityCalculator);

    let task_coordinator = Arc::new(TaskCoordinator::new(
        Arc::clone(&task_queue) as Arc<dyn TaskQueueService>,
        dependency_resolver,
        priority_calc,
    ));

    let claude_client = Arc::new(MockClaudeClient::new());
    let mcp_client = Arc::new(MockMcpClient);
    let agent_executor = Arc::new(AgentExecutor::new(claude_client, mcp_client));

    let limits = ResourceLimits::default();
    let resource_monitor = Arc::new(ResourceMonitor::new(limits));

    let config = Config::default();

    let max_agents = 3;
    let mut orchestrator = SwarmOrchestrator::new(
        max_agents,
        task_coordinator,
        agent_executor,
        resource_monitor,
        config,
    );

    orchestrator.start().await.unwrap();

    // Wait for tasks to start processing
    tokio::time::sleep(Duration::from_secs(2)).await;

    let stats = orchestrator.get_stats().await;

    // Should have processed some tasks
    assert!(stats.tasks_processed > 0 || stats.active_agents > 0);

    // Should respect max concurrency
    assert!(stats.active_agents <= max_agents);

    orchestrator.stop().await.unwrap();
}

#[tokio::test]
async fn test_graceful_shutdown_waits_for_agents() {
    let task_queue = Arc::new(MockTaskQueue::new());

    // Add a few ready tasks
    for _ in 0..3 {
        let task = create_test_task(TaskStatus::Ready);
        task_queue.add_task(task);
    }

    let dependency_resolver = Arc::new(DependencyResolver::new());
    let priority_calc = Arc::new(MockPriorityCalculator);

    let task_coordinator = Arc::new(TaskCoordinator::new(
        Arc::clone(&task_queue) as Arc<dyn TaskQueueService>,
        dependency_resolver,
        priority_calc,
    ));

    let claude_client = Arc::new(MockClaudeClient::new());
    let mcp_client = Arc::new(MockMcpClient);
    let agent_executor = Arc::new(AgentExecutor::new(claude_client, mcp_client));

    let limits = ResourceLimits::default();
    let resource_monitor = Arc::new(ResourceMonitor::new(limits));

    let config = Config::default();

    let mut orchestrator = SwarmOrchestrator::new(
        2,
        task_coordinator,
        agent_executor,
        resource_monitor,
        config,
    );

    orchestrator.start().await.unwrap();

    // Let agents start processing
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Shutdown should wait for completion (with timeout)
    let shutdown_start = std::time::Instant::now();
    orchestrator.stop().await.unwrap();
    let shutdown_duration = shutdown_start.elapsed();

    // Should have waited at least a little (agents need to finish)
    // but not hit the 30s timeout
    assert!(shutdown_duration < Duration::from_secs(30));

    let stats = orchestrator.get_stats().await;
    assert_eq!(stats.active_agents, 0);
}

#[tokio::test]
async fn test_swarm_stats_tracking() {
    let task_queue = Arc::new(MockTaskQueue::new());
    let dependency_resolver = Arc::new(DependencyResolver::new());
    let priority_calc = Arc::new(MockPriorityCalculator);

    let task_coordinator = Arc::new(TaskCoordinator::new(
        task_queue,
        dependency_resolver,
        priority_calc,
    ));

    let claude_client = Arc::new(MockClaudeClient::new());
    let mcp_client = Arc::new(MockMcpClient);
    let agent_executor = Arc::new(AgentExecutor::new(claude_client, mcp_client));

    let limits = ResourceLimits::default();
    let resource_monitor = Arc::new(ResourceMonitor::new(limits));

    let config = Config::default();

    let max_agents = 5;
    let orchestrator = SwarmOrchestrator::new(
        max_agents,
        task_coordinator,
        agent_executor,
        resource_monitor,
        config,
    );

    let stats = orchestrator.get_stats().await;
    assert_eq!(stats.state, SwarmState::Stopped);
    assert_eq!(stats.max_agents, max_agents);
    assert_eq!(stats.active_agents, 0);
    assert_eq!(stats.tasks_processed, 0);
    assert_eq!(stats.tasks_failed, 0);
}
