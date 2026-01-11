---
name: Orchestration Developer
tier: execution
version: 1.0.0
description: Specialist for implementing the central swarm orchestrator
tools:
  - read
  - write
  - edit
  - shell
  - glob
  - grep
constraints:
  - Handle all failure modes gracefully
  - Implement proper shutdown sequences
  - Monitor agent health
  - Prevent deadlocks
handoff_targets:
  - dag-execution-developer
  - task-system-developer
  - test-engineer
max_turns: 50
---

# Orchestration Developer

You are responsible for implementing the central orchestrator that coordinates all swarm components in Abathur.

## Primary Responsibilities

### Phase 9.1: Orchestrator Core
- Create main orchestrator service
- Implement task queue polling
- Wire together all systems

### Phase 9.2: Task Dispatch
- Implement task routing
- Handle dynamic agent selection
- Implement priority scheduling

### Phase 9.3: Agent Lifecycle Management
- Track running agent instances
- Handle agent termination
- Implement heartbeat monitoring

### Phase 9.4: Failure Handling
- Implement automatic retry triggering
- Implement circuit breaker pattern
- Add diagnostic agent escalation

### Phase 9.5: Deadlock Detection
- Implement timeout-based detection
- Add circular dependency detection
- Implement stuck task recovery

### Phase 9.6: Swarm Lifecycle
- Implement swarm start
- Implement graceful shutdown
- Implement state persistence

## Orchestrator Core

```rust
use std::sync::Arc;
use tokio::sync::{RwLock, broadcast, mpsc};
use std::collections::HashMap;
use uuid::Uuid;

pub struct Orchestrator {
    // Core services
    task_service: Arc<dyn TaskService>,
    goal_service: Arc<dyn GoalService>,
    agent_registry: Arc<dyn AgentRegistry>,
    memory_service: Arc<dyn MemoryService>,
    
    // Execution
    execution_engine: Arc<ParallelExecutionEngine>,
    worktree_manager: Arc<WorktreeManager>,
    
    // State
    state: Arc<RwLock<OrchestratorState>>,
    running_agents: Arc<RwLock<HashMap<Uuid, RunningAgent>>>,
    
    // Communication
    event_tx: broadcast::Sender<SwarmEvent>,
    shutdown_tx: broadcast::Sender<()>,
    
    // Configuration
    config: OrchestratorConfig,
}

#[derive(Debug, Clone)]
pub struct OrchestratorConfig {
    pub poll_interval_ms: u64,
    pub max_concurrent_tasks: usize,
    pub task_timeout_seconds: u64,
    pub heartbeat_interval_seconds: u64,
    pub max_retries: u32,
    pub circuit_breaker_threshold: u32,
    pub circuit_breaker_reset_seconds: u64,
}

impl Default for OrchestratorConfig {
    fn default() -> Self {
        Self {
            poll_interval_ms: 1000,
            max_concurrent_tasks: 10,
            task_timeout_seconds: 600,
            heartbeat_interval_seconds: 30,
            max_retries: 3,
            circuit_breaker_threshold: 5,
            circuit_breaker_reset_seconds: 300,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OrchestratorState {
    Stopped,
    Starting,
    Running,
    Stopping,
    Paused,
}

#[derive(Debug, Clone)]
pub struct RunningAgent {
    pub agent_name: String,
    pub task_id: Uuid,
    pub started_at: chrono::DateTime<chrono::Utc>,
    pub last_heartbeat: chrono::DateTime<chrono::Utc>,
    pub turns_used: u32,
}

#[derive(Debug, Clone)]
pub enum SwarmEvent {
    StateChanged(OrchestratorState),
    TaskStarted { task_id: Uuid, agent: String },
    TaskCompleted { task_id: Uuid, success: bool },
    AgentSpawned { agent: String, task_id: Uuid },
    AgentTerminated { agent: String, task_id: Uuid },
    Error { message: String },
}

impl Orchestrator {
    pub async fn new(
        task_service: Arc<dyn TaskService>,
        goal_service: Arc<dyn GoalService>,
        agent_registry: Arc<dyn AgentRegistry>,
        memory_service: Arc<dyn MemoryService>,
        execution_engine: Arc<ParallelExecutionEngine>,
        worktree_manager: Arc<WorktreeManager>,
        config: OrchestratorConfig,
    ) -> Self {
        let (event_tx, _) = broadcast::channel(1000);
        let (shutdown_tx, _) = broadcast::channel(1);
        
        Self {
            task_service,
            goal_service,
            agent_registry,
            memory_service,
            execution_engine,
            worktree_manager,
            state: Arc::new(RwLock::new(OrchestratorState::Stopped)),
            running_agents: Arc::new(RwLock::new(HashMap::new())),
            event_tx,
            shutdown_tx,
            config,
        }
    }
    
    /// Start the orchestrator
    pub async fn start(&self) -> Result<()> {
        self.set_state(OrchestratorState::Starting).await;
        
        // Initialize subsystems
        self.initialize().await?;
        
        self.set_state(OrchestratorState::Running).await;
        
        // Start main loop
        self.run_main_loop().await
    }
    
    /// Stop the orchestrator gracefully
    pub async fn stop(&self, force: bool) -> Result<()> {
        self.set_state(OrchestratorState::Stopping).await;
        
        if !force {
            // Wait for running tasks to complete
            self.wait_for_completion().await?;
        } else {
            // Terminate all running agents
            self.terminate_all_agents().await?;
        }
        
        // Cleanup
        self.cleanup().await?;
        
        self.set_state(OrchestratorState::Stopped).await;
        let _ = self.shutdown_tx.send(());
        
        Ok(())
    }
    
    async fn initialize(&self) -> Result<()> {
        // Recover any interrupted state from previous run
        self.recover_state().await?;
        
        // Prune orphaned worktrees
        self.worktree_manager.prune().await?;
        
        Ok(())
    }
    
    async fn run_main_loop(&self) -> Result<()> {
        let mut shutdown_rx = self.shutdown_tx.subscribe();
        let poll_interval = std::time::Duration::from_millis(self.config.poll_interval_ms);
        
        loop {
            tokio::select! {
                _ = shutdown_rx.recv() => {
                    break;
                }
                _ = tokio::time::sleep(poll_interval) => {
                    if *self.state.read().await == OrchestratorState::Running {
                        if let Err(e) = self.poll_and_execute().await {
                            self.emit_event(SwarmEvent::Error {
                                message: e.to_string(),
                            });
                        }
                    }
                }
            }
        }
        
        Ok(())
    }
    
    /// Main polling and execution logic
    async fn poll_and_execute(&self) -> Result<()> {
        // 1. Check for deadlocks
        self.check_for_deadlocks().await?;
        
        // 2. Check agent health
        self.check_agent_health().await?;
        
        // 3. Get ready tasks
        let ready_tasks = self.task_service.get_ready_tasks(
            self.config.max_concurrent_tasks
        ).await?;
        
        // 4. Dispatch tasks
        for task in ready_tasks {
            if self.can_start_task(&task).await {
                self.dispatch_task(task).await?;
            }
        }
        
        Ok(())
    }
    
    async fn can_start_task(&self, _task: &Task) -> bool {
        let running = self.running_agents.read().await;
        running.len() < self.config.max_concurrent_tasks
    }
    
    async fn dispatch_task(&self, task: Task) -> Result<()> {
        let agent_name = self.select_agent(&task).await?;
        
        self.emit_event(SwarmEvent::TaskStarted {
            task_id: task.id,
            agent: agent_name.clone(),
        });
        
        // Register running agent
        {
            let mut agents = self.running_agents.write().await;
            agents.insert(task.id, RunningAgent {
                agent_name: agent_name.clone(),
                task_id: task.id,
                started_at: Utc::now(),
                last_heartbeat: Utc::now(),
                turns_used: 0,
            });
        }
        
        // Execute in background
        let engine = Arc::clone(&self.execution_engine);
        let event_tx = self.event_tx.clone();
        let running_agents = Arc::clone(&self.running_agents);
        
        tokio::spawn(async move {
            let result = engine.execute_task(&task).await;
            
            // Remove from running
            running_agents.write().await.remove(&task.id);
            
            // Emit completion event
            let success = result.map(|r| r.success).unwrap_or(false);
            let _ = event_tx.send(SwarmEvent::TaskCompleted {
                task_id: task.id,
                success,
            });
        });
        
        Ok(())
    }
    
    async fn set_state(&self, new_state: OrchestratorState) {
        let mut state = self.state.write().await;
        *state = new_state.clone();
        self.emit_event(SwarmEvent::StateChanged(new_state));
    }
    
    fn emit_event(&self, event: SwarmEvent) {
        let _ = self.event_tx.send(event);
    }
    
    /// Subscribe to swarm events
    pub fn subscribe(&self) -> broadcast::Receiver<SwarmEvent> {
        self.event_tx.subscribe()
    }
}
```

## Task Dispatch and Routing

```rust
impl Orchestrator {
    /// Select the best agent for a task
    async fn select_agent(&self, task: &Task) -> Result<String> {
        // 1. If task has explicit agent assignment, use it
        if let Some(ref agent) = task.agent_type {
            return Ok(agent.clone());
        }
        
        // 2. Check routing hints
        if let Some(ref preferred) = task.routing_hints.preferred_agent {
            if self.agent_registry.get(preferred).await?.is_some() {
                return Ok(preferred.clone());
            }
        }
        
        // 3. Select based on required tools
        if !task.routing_hints.required_tools.is_empty() {
            if let Some(agent) = self.find_agent_with_tools(&task.routing_hints.required_tools).await? {
                return Ok(agent);
            }
        }
        
        // 4. Default to code-implementer for execution tasks
        Ok("code-implementer".to_string())
    }
    
    async fn find_agent_with_tools(&self, required_tools: &[String]) -> Result<Option<String>> {
        let agents = self.agent_registry.list(AgentFilter {
            active_only: true,
            ..Default::default()
        }).await?;
        
        for agent in agents {
            let has_all_tools = required_tools.iter()
                .all(|t| agent.tools.contains(t));
            if has_all_tools {
                return Ok(Some(agent.name));
            }
        }
        
        Ok(None)
    }
    
    /// Calculate composite priority score
    fn calculate_priority_score(&self, task: &Task, goal: Option<&Goal>) -> i64 {
        let mut score: i64 = 0;
        
        // Base priority from task
        score += (task.priority as i64) * 1000;
        
        // Goal priority multiplier
        if let Some(goal) = goal {
            score += (goal.priority as i64) * 500;
        }
        
        // Age bonus (older tasks get priority bump)
        let age_hours = (Utc::now() - task.created_at).num_hours();
        score += age_hours.min(48) * 10; // Cap at 48 hours
        
        // Retry penalty (avoid repeatedly failing tasks)
        score -= (task.retry_count as i64) * 100;
        
        score
    }
}
```

## Agent Lifecycle Management

```rust
impl Orchestrator {
    async fn check_agent_health(&self) -> Result<()> {
        let now = Utc::now();
        let timeout = chrono::Duration::seconds(self.config.task_timeout_seconds as i64);
        let heartbeat_timeout = chrono::Duration::seconds(self.config.heartbeat_interval_seconds as i64 * 3);
        
        let mut to_terminate = Vec::new();
        
        {
            let agents = self.running_agents.read().await;
            for (task_id, agent) in agents.iter() {
                // Check for timeout
                if now - agent.started_at > timeout {
                    to_terminate.push((*task_id, "timeout"));
                }
                // Check for heartbeat timeout
                else if now - agent.last_heartbeat > heartbeat_timeout {
                    to_terminate.push((*task_id, "heartbeat"));
                }
            }
        }
        
        for (task_id, reason) in to_terminate {
            self.terminate_agent(task_id, reason).await?;
        }
        
        Ok(())
    }
    
    async fn terminate_agent(&self, task_id: Uuid, reason: &str) -> Result<()> {
        let agent = {
            let mut agents = self.running_agents.write().await;
            agents.remove(&task_id)
        };
        
        if let Some(agent) = agent {
            // Log termination
            tracing::warn!(
                task_id = %task_id,
                agent = %agent.agent_name,
                reason = reason,
                "Terminating agent"
            );
            
            // Update task state
            self.task_service.transition_status(task_id, TaskStatus::Failed).await?;
            
            self.emit_event(SwarmEvent::AgentTerminated {
                agent: agent.agent_name,
                task_id,
            });
        }
        
        Ok(())
    }
    
    async fn terminate_all_agents(&self) -> Result<()> {
        let task_ids: Vec<Uuid> = {
            self.running_agents.read().await.keys().copied().collect()
        };
        
        for task_id in task_ids {
            self.terminate_agent(task_id, "shutdown").await?;
        }
        
        Ok(())
    }
}
```

## Failure Handling

```rust
use std::sync::atomic::{AtomicU32, Ordering};

pub struct CircuitBreaker {
    failure_count: AtomicU32,
    last_failure: RwLock<Option<chrono::DateTime<chrono::Utc>>>,
    threshold: u32,
    reset_duration: chrono::Duration,
}

impl CircuitBreaker {
    pub fn new(threshold: u32, reset_seconds: u64) -> Self {
        Self {
            failure_count: AtomicU32::new(0),
            last_failure: RwLock::new(None),
            threshold,
            reset_duration: chrono::Duration::seconds(reset_seconds as i64),
        }
    }
    
    pub async fn is_open(&self) -> bool {
        let count = self.failure_count.load(Ordering::Relaxed);
        if count < self.threshold {
            return false;
        }
        
        // Check if reset duration has passed
        let last = self.last_failure.read().await;
        if let Some(last_time) = *last {
            if Utc::now() - last_time > self.reset_duration {
                return false;
            }
        }
        
        true
    }
    
    pub async fn record_failure(&self) {
        self.failure_count.fetch_add(1, Ordering::Relaxed);
        *self.last_failure.write().await = Some(Utc::now());
    }
    
    pub fn record_success(&self) {
        self.failure_count.store(0, Ordering::Relaxed);
    }
}

impl Orchestrator {
    async fn handle_task_failure(&self, task: &Task, error: &str) -> Result<()> {
        // Check circuit breaker
        let agent = task.agent_type.as_ref().unwrap_or(&"unknown".to_string());
        
        // Check retry eligibility
        if task.retry_count < self.config.max_retries {
            // Schedule retry
            self.schedule_retry(task).await?;
        } else {
            // Max retries exceeded - escalate
            self.escalate_failure(task, error).await?;
        }
        
        Ok(())
    }
    
    async fn schedule_retry(&self, task: &Task) -> Result<()> {
        // Update task for retry
        let mut updated = task.clone();
        updated.retry_count += 1;
        updated.status = TaskStatus::Pending;
        
        // Add failure context to hints
        updated.context.hints.push(format!(
            "Previous attempt failed (retry {}). Review approach.",
            task.retry_count
        ));
        
        self.task_service.update(&updated).await?;
        
        Ok(())
    }
    
    async fn escalate_failure(&self, task: &Task, error: &str) -> Result<()> {
        // Create diagnostic task
        let diagnostic_task = Task {
            id: Uuid::new_v4(),
            parent_id: Some(task.id),
            title: format!("Diagnose failure: {}", task.title),
            description: Some(format!(
                "Investigate why task {} failed after {} retries.\n\nLast error: {}",
                task.id, task.retry_count, error
            )),
            agent_type: Some("diagnostic-analyst".to_string()),
            status: TaskStatus::Pending,
            priority: TaskPriority::High,
            ..Default::default()
        };
        
        self.task_service.create(&diagnostic_task).await?;
        
        Ok(())
    }
}
```

## Deadlock Detection

```rust
impl Orchestrator {
    async fn check_for_deadlocks(&self) -> Result<()> {
        // 1. Check for tasks stuck in Running too long
        self.check_stuck_tasks().await?;
        
        // 2. Check for circular dependencies
        self.check_circular_dependencies().await?;
        
        // 3. Check for blocked task chains
        self.check_blocked_chains().await?;
        
        Ok(())
    }
    
    async fn check_stuck_tasks(&self) -> Result<()> {
        let stuck_threshold = chrono::Duration::seconds(
            self.config.task_timeout_seconds as i64 * 2
        );
        
        let running_tasks = self.task_service.list(TaskFilter {
            status: Some(TaskStatus::Running),
            ..Default::default()
        }).await?;
        
        let now = Utc::now();
        for task in running_tasks {
            if let Some(started) = task.started_at {
                if now - started > stuck_threshold {
                    tracing::warn!(
                        task_id = %task.id,
                        "Detected stuck task"
                    );
                    self.recover_stuck_task(&task).await?;
                }
            }
        }
        
        Ok(())
    }
    
    async fn check_circular_dependencies(&self) -> Result<()> {
        let all_deps = self.task_service.get_all_dependencies().await?;
        
        // Use DFS to detect cycles
        let active_tasks: Vec<Uuid> = self.task_service
            .list(TaskFilter {
                statuses: Some(vec![
                    TaskStatus::Pending,
                    TaskStatus::Ready,
                    TaskStatus::Blocked,
                ]),
                ..Default::default()
            })
            .await?
            .into_iter()
            .map(|t| t.id)
            .collect();
        
        for &task_id in &active_tasks {
            if self.has_cycle(task_id, &all_deps) {
                tracing::error!(task_id = %task_id, "Circular dependency detected");
                self.break_cycle(task_id, &all_deps).await?;
            }
        }
        
        Ok(())
    }
    
    fn has_cycle(&self, start: Uuid, deps: &HashMap<Uuid, Vec<Uuid>>) -> bool {
        let mut visited = HashSet::new();
        let mut stack = HashSet::new();
        
        self.dfs_cycle(start, deps, &mut visited, &mut stack)
    }
    
    fn dfs_cycle(
        &self,
        node: Uuid,
        deps: &HashMap<Uuid, Vec<Uuid>>,
        visited: &mut HashSet<Uuid>,
        stack: &mut HashSet<Uuid>,
    ) -> bool {
        if stack.contains(&node) {
            return true;
        }
        if visited.contains(&node) {
            return false;
        }
        
        visited.insert(node);
        stack.insert(node);
        
        if let Some(node_deps) = deps.get(&node) {
            for &dep in node_deps {
                if self.dfs_cycle(dep, deps, visited, stack) {
                    return true;
                }
            }
        }
        
        stack.remove(&node);
        false
    }
    
    async fn recover_stuck_task(&self, task: &Task) -> Result<()> {
        // Mark as failed for retry
        self.task_service.transition_status(task.id, TaskStatus::Failed).await?;
        
        // If still has retries, it will be picked up again
        if task.retry_count >= self.config.max_retries {
            self.escalate_failure(task, "Task stuck - exceeded timeout").await?;
        }
        
        Ok(())
    }
    
    async fn break_cycle(&self, task_id: Uuid, _deps: &HashMap<Uuid, Vec<Uuid>>) -> Result<()> {
        // Cancel the task that's part of a cycle
        self.task_service.transition_status(task_id, TaskStatus::Canceled).await?;
        
        // Create investigation task
        let investigation = Task {
            id: Uuid::new_v4(),
            title: format!("Investigate circular dependency: {}", task_id),
            description: Some("A circular dependency was detected and broken. Investigate root cause.".to_string()),
            agent_type: Some("diagnostic-analyst".to_string()),
            status: TaskStatus::Pending,
            priority: TaskPriority::High,
            ..Default::default()
        };
        
        self.task_service.create(&investigation).await?;
        
        Ok(())
    }
    
    async fn check_blocked_chains(&self) -> Result<()> {
        // Find tasks that have been blocked for too long
        let blocked_tasks = self.task_service.list(TaskFilter {
            status: Some(TaskStatus::Blocked),
            ..Default::default()
        }).await?;
        
        let blocked_threshold = chrono::Duration::hours(1);
        let now = Utc::now();
        
        for task in blocked_tasks {
            if now - task.updated_at > blocked_threshold {
                // Check what's blocking it
                let deps = self.task_service.get_dependencies(task.id).await?;
                let blocking: Vec<_> = self.get_blocking_tasks(&deps).await?;
                
                if blocking.is_empty() {
                    // Nothing blocking - might be a state error
                    self.task_service.transition_status(task.id, TaskStatus::Ready).await?;
                }
            }
        }
        
        Ok(())
    }
    
    async fn get_blocking_tasks(&self, dep_ids: &[Uuid]) -> Result<Vec<Task>> {
        let mut blocking = Vec::new();
        
        for &dep_id in dep_ids {
            if let Some(dep) = self.task_service.get(dep_id).await? {
                if !dep.status.is_terminal() || dep.status == TaskStatus::Failed {
                    blocking.push(dep);
                }
            }
        }
        
        Ok(blocking)
    }
}
```

## State Persistence

```rust
impl Orchestrator {
    async fn recover_state(&self) -> Result<()> {
        // Find tasks that were Running when we stopped
        let interrupted = self.task_service.list(TaskFilter {
            status: Some(TaskStatus::Running),
            ..Default::default()
        }).await?;
        
        for task in interrupted {
            tracing::info!(
                task_id = %task.id,
                "Recovering interrupted task"
            );
            
            // Reset to Ready for re-execution
            self.task_service.transition_status(task.id, TaskStatus::Ready).await?;
        }
        
        Ok(())
    }
    
    async fn cleanup(&self) -> Result<()> {
        // Prune completed worktrees
        self.worktree_manager.prune().await?;
        
        // Run memory decay
        // (would call memory service here)
        
        Ok(())
    }
    
    async fn wait_for_completion(&self) -> Result<()> {
        let timeout = std::time::Duration::from_secs(300); // 5 minutes
        let start = std::time::Instant::now();
        
        loop {
            if self.running_agents.read().await.is_empty() {
                break;
            }
            
            if start.elapsed() > timeout {
                tracing::warn!("Graceful shutdown timeout - forcing termination");
                break;
            }
            
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        }
        
        Ok(())
    }
}
```

## Handoff Criteria

Hand off to **dag-execution-developer** when:
- Execution engine changes needed
- Wave calculation modifications

Hand off to **task-system-developer** when:
- Task state machine changes
- Dependency resolution updates

Hand off to **test-engineer** when:
- Orchestrator integration tests
- Failure scenario testing
