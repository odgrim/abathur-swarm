# Swarm Orchestrator Architecture Design Document

## Executive Summary

This document provides the complete architectural design for Abathur's swarm orchestrator system. The design enables a functional, scalable, and resilient task execution system that manages a pool of worker agents to process tasks from the queue automatically.

## Table of Contents

1. [System Overview](#system-overview)
2. [Architecture Components](#architecture-components)
3. [Component Interaction Diagrams](#component-interaction-diagrams)
4. [Data Flow Diagrams](#data-flow-diagrams)
5. [Implementation Recommendations](#implementation-recommendations)
6. [State Management](#state-management)
7. [Risk Assessment](#risk-assessment)
8. [Testing Strategy](#testing-strategy)

## System Overview

### Current State
- Basic swarm commands (start/stop/status) implemented
- State persistence to `~/.abathur/swarm_state.json` for CLI continuity
- Infrastructure components available (TaskCoordinator, AgentExecutor, etc.)
- Database schema supports agents and tasks
- Missing: actual orchestration loop and agent pool management

### Target Architecture
A fully functional swarm orchestrator that:
- Manages a dynamic pool of worker agents
- Continuously processes tasks from the queue
- Handles failures gracefully with retry logic
- Scales based on workload and resource constraints
- Provides real-time monitoring and status updates

## Architecture Components

### Core Components

#### 1. SwarmOrchestrator (Enhanced)
**Purpose**: Central orchestration controller managing the entire swarm lifecycle

**Responsibilities**:
- Initialize and manage the agent pool
- Start/stop background processing loop
- Coordinate with all subsystems
- Handle graceful shutdown
- Persist and restore state

**Key Interfaces**:
```rust
pub struct SwarmOrchestrator {
    agent_pool: Arc<RwLock<AgentPool>>,
    task_dispatcher: Arc<TaskDispatcher>,
    resource_monitor: Arc<ResourceMonitor>,
    state_manager: Arc<StateManager>,
    background_handle: Option<JoinHandle<()>>,
}
```

#### 2. AgentPool
**Purpose**: Manages the lifecycle of worker agents

**Responsibilities**:
- Create and destroy agents dynamically
- Track agent states (Idle/Busy/Terminated)
- Handle agent health checks and heartbeats
- Manage agent assignment to tasks
- Scale pool based on demand

**Key Interfaces**:
```rust
pub struct AgentPool {
    agents: HashMap<Uuid, Arc<RwLock<AgentWorker>>>,
    max_agents: usize,
    min_agents: usize,
    agent_repo: Arc<dyn AgentRepository>,
}
```

#### 3. AgentWorker
**Purpose**: Individual worker agent executing tasks

**Responsibilities**:
- Execute assigned tasks
- Report status and progress
- Handle task timeouts
- Manage local resources
- Send heartbeats

**Key Interfaces**:
```rust
pub struct AgentWorker {
    id: Uuid,
    agent_type: String,
    status: Arc<RwLock<AgentStatus>>,
    current_task: Arc<RwLock<Option<Task>>>,
    executor: Arc<AgentExecutor>,
    shutdown_rx: broadcast::Receiver<()>,
}
```

#### 4. TaskDispatcher
**Purpose**: Assigns tasks to available agents

**Responsibilities**:
- Poll for ready tasks from queue
- Match tasks to appropriate agents
- Handle task assignment logic
- Track task execution status
- Manage retry queue

**Key Interfaces**:
```rust
pub struct TaskDispatcher {
    task_queue: Arc<dyn TaskQueueService>,
    agent_pool: Arc<RwLock<AgentPool>>,
    assignment_strategy: Box<dyn AssignmentStrategy>,
    retry_queue: Arc<RwLock<VecDeque<Task>>>,
}
```

#### 5. StateManager
**Purpose**: Manages persistent and runtime state

**Responsibilities**:
- Bridge file-based state with in-memory state
- Handle state synchronization
- Support crash recovery
- Track metrics and statistics

**Key Interfaces**:
```rust
pub struct StateManager {
    file_state: Arc<RwLock<SwarmStateFile>>,
    runtime_state: Arc<RwLock<RuntimeState>>,
    state_repo: Arc<dyn StateRepository>,
}
```

#### 6. BackgroundProcessor
**Purpose**: Main processing loop running in background

**Responsibilities**:
- Continuous task polling and dispatch
- Agent health monitoring
- Resource monitoring integration
- Metrics collection
- Graceful shutdown handling

## Component Interaction Diagrams

### System Architecture Overview
```
┌─────────────────────────────────────────────────────────────────┐
│                         CLI Invocation                          │
│  (abathur swarm start/stop/status)                             │
└─────────────────┬───────────────────────────────────────────────┘
                  │
                  ▼
┌─────────────────────────────────────────────────────────────────┐
│                       SwarmService (CLI)                        │
│  • Read/Write ~/.abathur/swarm_state.json                      │
│  • Communicate with SwarmOrchestrator via IPC/Socket           │
└─────────────────┬───────────────────────────────────────────────┘
                  │
                  ▼
┌─────────────────────────────────────────────────────────────────┐
│                     SwarmOrchestrator                           │
│  ┌──────────────┬──────────────┬──────────────┬──────────────┐ │
│  │ AgentPool    │TaskDispatcher│ResourceMonitor│StateManager  │ │
│  └──────┬───────┴──────┬───────┴──────┬───────┴──────┬───────┘ │
│         │              │              │              │          │
│  ┌──────▼──────────────▼──────────────▼──────────────▼───────┐ │
│  │              BackgroundProcessor Loop                      │ │
│  │  • Poll tasks  • Dispatch  • Monitor  • Update state      │ │
│  └─────────────────────────────────────────────────────────┘ │
└─────────────────────────────────────────────────────────────────┘
                  │
    ┌─────────────┼─────────────┬──────────────┬────────────────┐
    ▼             ▼             ▼              ▼                ▼
┌─────────┐ ┌─────────┐ ┌──────────────┐ ┌──────────┐ ┌─────────────┐
│Agent #1 │ │Agent #2 │ │TaskQueue DB  │ │State DB  │ │Agent DB     │
└─────────┘ └─────────┘ └──────────────┘ └──────────┘ └─────────────┘
```

### Task Processing Flow
```
┌────────────┐     ┌──────────────┐     ┌──────────────┐
│Task Queue  │────▶│TaskDispatcher│────▶│ Agent Pool   │
│  (Ready)   │     │   (Polling)  │     │  (Matching)  │
└────────────┘     └──────────────┘     └──────────────┘
                           │                    │
                           ▼                    ▼
                   ┌──────────────┐     ┌──────────────┐
                   │Task Assignment│────▶│Agent Worker  │
                   │   Strategy    │     │ (Execution)  │
                   └──────────────┘     └──────────────┘
                                               │
                                               ▼
                                        ┌──────────────┐
                                        │Task Complete │
                                        │   /Failed    │
                                        └──────────────┘
                                               │
                           ┌───────────────────┼───────────┐
                           ▼                   ▼           ▼
                    ┌──────────────┐  ┌──────────┐ ┌───────────┐
                    │Update Status │  │  Retry   │ │ Trigger   │
                    │   in DB      │  │  Queue   │ │Dependents │
                    └──────────────┘  └──────────┘ └───────────┘
```

### Agent Lifecycle
```
     ┌─────────┐
     │ Created │
     └────┬────┘
          │
          ▼
     ┌─────────┐     Task      ┌─────────┐
     │  Idle   │◄──────────────▶│  Busy   │
     └────┬────┘   Assignment   └────┬────┘
          │                          │
          │      Heartbeat           │ Task Complete
          ▼       Timeout            ▼
     ┌─────────┐               ┌─────────┐
     │  Stale  │               │  Idle   │
     └────┬────┘               └─────────┘
          │
          ▼
     ┌──────────┐
     │Terminated│
     └──────────┘
```

## Data Flow Diagrams

### State Synchronization Flow
```
File State                 Runtime State              Database State
(~/.abathur/)              (In-Memory)               (SQLite)
     │                          │                          │
     │    Read on Start         │                          │
     ├─────────────────────────▶│                          │
     │                          │                          │
     │                          │     Sync Agents          │
     │                          │◄─────────────────────────┤
     │                          │                          │
     │    Periodic Write        │     Update Tasks         │
     │◄─────────────────────────┤─────────────────────────▶│
     │                          │                          │
     │                          │     Query Ready Tasks    │
     │                          │◄─────────────────────────┤
     │                          │                          │
```

### Task Assignment Flow
```
1. Poll Ready Tasks
   TaskQueue ──▶ TaskDispatcher

2. Check Agent Availability
   TaskDispatcher ──▶ AgentPool

3. Apply Assignment Strategy
   • Agent Type Matching
   • Load Balancing
   • Resource Constraints

4. Assign Task to Agent
   TaskDispatcher ──▶ AgentWorker

5. Update Task Status
   AgentWorker ──▶ TaskQueue (Running)

6. Execute Task
   AgentWorker ──▶ AgentExecutor ──▶ Claude API/MCP Tools

7. Report Result
   AgentWorker ──▶ TaskQueue (Completed/Failed)

8. Trigger Dependencies
   TaskCoordinator ──▶ DependencyResolver ──▶ TaskQueue
```

## Implementation Recommendations

### TODO #1: Agent Pool Management

**Location**: `src/application/agent_pool.rs` (new file)

```rust
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use uuid::Uuid;

pub struct AgentPool {
    agents: Arc<RwLock<HashMap<Uuid, Arc<AgentWorker>>>>,
    max_agents: usize,
    min_agents: usize,
    agent_repo: Arc<dyn AgentRepository>,
    agent_executor: Arc<AgentExecutor>,
}

impl AgentPool {
    pub async fn initialize(&self) -> Result<()> {
        // 1. Load existing agents from DB
        let existing_agents = self.agent_repo.list(None).await?;

        // 2. Terminate stale agents
        for agent in existing_agents {
            if agent.is_stale(Duration::seconds(60)) {
                self.agent_repo.update(agent.terminate()).await?;
            }
        }

        // 3. Create minimum number of agents
        for _ in 0..self.min_agents {
            self.spawn_agent().await?;
        }

        Ok(())
    }

    pub async fn spawn_agent(&self) -> Result<Uuid> {
        let agent_id = Uuid::new_v4();
        let agent = Agent::new(agent_id, "general-purpose".to_string());

        // Save to database
        self.agent_repo.insert(agent.clone()).await?;

        // Create worker
        let worker = Arc::new(AgentWorker::new(
            agent_id,
            self.agent_executor.clone(),
        ));

        // Start worker background task
        worker.start().await?;

        // Add to pool
        let mut agents = self.agents.write().await;
        agents.insert(agent_id, worker);

        Ok(agent_id)
    }

    pub async fn get_idle_agent(&self) -> Option<Arc<AgentWorker>> {
        let agents = self.agents.read().await;
        for (_, agent) in agents.iter() {
            if agent.is_idle().await {
                return Some(agent.clone());
            }
        }
        None
    }

    pub async fn scale_pool(&self, pending_tasks: usize) -> Result<()> {
        let current_size = self.agents.read().await.len();

        // Scale up if needed
        if pending_tasks > current_size && current_size < self.max_agents {
            let spawn_count = (pending_tasks - current_size)
                .min(self.max_agents - current_size);

            for _ in 0..spawn_count {
                self.spawn_agent().await?;
            }
        }

        // Scale down if over-provisioned
        if pending_tasks < current_size / 2 && current_size > self.min_agents {
            // Terminate idle agents
            self.terminate_idle_agents(current_size - self.min_agents).await?;
        }

        Ok(())
    }
}
```

### TODO #2: Task Distribution

**Location**: `src/application/task_dispatcher.rs` (new file)

```rust
use std::sync::Arc;
use tokio::sync::RwLock;
use std::collections::VecDeque;

pub struct TaskDispatcher {
    task_queue: Arc<dyn TaskQueueService>,
    task_coordinator: Arc<TaskCoordinator>,
    agent_pool: Arc<RwLock<AgentPool>>,
    retry_queue: Arc<RwLock<VecDeque<RetryTask>>>,
}

impl TaskDispatcher {
    pub async fn dispatch_loop(&self, mut shutdown_rx: broadcast::Receiver<()>) -> Result<()> {
        let mut poll_interval = interval(Duration::from_secs(1));

        loop {
            select! {
                _ = poll_interval.tick() => {
                    // 1. Check retry queue first
                    if let Some(task) = self.process_retry_queue().await? {
                        self.dispatch_task(task).await?;
                        continue;
                    }

                    // 2. Get next ready task
                    if let Some(task) = self.get_next_ready_task().await? {
                        // 3. Find suitable agent
                        if let Some(agent) = self.find_suitable_agent(&task).await {
                            // 4. Assign task
                            self.assign_task_to_agent(task, agent).await?;
                        } else {
                            // No available agent, task stays in queue
                            debug!("No available agent for task {}", task.id);
                        }
                    }
                }

                _ = shutdown_rx.recv() => {
                    info!("Task dispatcher shutting down");
                    break;
                }
            }
        }

        Ok(())
    }

    async fn get_next_ready_task(&self) -> Result<Option<Task>> {
        let filters = TaskFilters {
            status: Some(vec![TaskStatus::Ready]),
            limit: Some(1),
            order_by: Some("calculated_priority DESC".to_string()),
        };

        let tasks = self.task_queue.list(filters).await?;
        Ok(tasks.into_iter().next())
    }

    async fn find_suitable_agent(&self, task: &Task) -> Option<Arc<AgentWorker>> {
        let pool = self.agent_pool.read().await;

        // Strategy 1: Type matching
        if let Some(agent) = pool.get_agent_by_type(&task.agent_type).await {
            if agent.is_idle().await {
                return Some(agent);
            }
        }

        // Strategy 2: Any idle agent
        pool.get_idle_agent().await
    }

    async fn assign_task_to_agent(&self, mut task: Task, agent: Arc<AgentWorker>) -> Result<()> {
        // 1. Mark task as running
        task.status = TaskStatus::Running;
        task.started_at = Some(Utc::now());
        self.task_queue.update(&task).await?;

        // 2. Assign to agent
        agent.assign_task(task.clone()).await?;

        // 3. Update agent status in DB
        let mut agent_model = self.agent_pool.get_agent_model(agent.id).await?;
        agent_model.status = AgentStatus::Busy;
        agent_model.current_task_id = Some(task.id);
        self.agent_repo.update(agent_model).await?;

        info!("Assigned task {} to agent {}", task.id, agent.id);
        Ok(())
    }

    async fn handle_task_completion(&self, task_id: Uuid, result: TaskResult) -> Result<()> {
        let mut task = self.task_queue.get(task_id).await?.unwrap();

        match result {
            TaskResult::Success(data) => {
                task.status = TaskStatus::Completed;
                task.result_data = Some(data);
                task.completed_at = Some(Utc::now());
                self.task_queue.update(&task).await?;

                // Trigger dependent tasks
                self.task_coordinator.trigger_dependents(task_id).await?;
            }
            TaskResult::Failure(error) => {
                task.retry_count += 1;

                if task.retry_count < task.max_retries {
                    // Add to retry queue
                    let retry = RetryTask {
                        task: task.clone(),
                        retry_at: Utc::now() + Duration::seconds(60 * task.retry_count),
                    };
                    self.retry_queue.write().await.push_back(retry);
                } else {
                    // Max retries exceeded
                    task.status = TaskStatus::Failed;
                    task.error_message = Some(error);
                    task.completed_at = Some(Utc::now());
                    self.task_queue.update(&task).await?;

                    // Cascade failure to dependents
                    self.task_coordinator.cascade_failure(task_id).await?;
                }
            }
        }

        Ok(())
    }
}
```

### TODO #3: Background Processing Loop

**Location**: Enhance `src/application/swarm_orchestrator.rs`

```rust
impl SwarmOrchestrator {
    pub async fn start(&mut self) -> Result<()> {
        // ... existing state transition code ...

        // Initialize infrastructure
        self.initialize_infrastructure().await?;

        // Start background processing
        let handle = self.start_background_processing().await?;
        self.background_handle = Some(handle);

        // ... rest of existing code ...
    }

    async fn initialize_infrastructure(&self) -> Result<()> {
        // 1. Initialize agent pool
        self.agent_pool.initialize().await?;

        // 2. Start resource monitor
        let monitor_handle = self.resource_monitor
            .start(Duration::from_secs(5))
            .await?;
        self.monitor_handle = Some(monitor_handle);

        // 3. Restore runtime state from DB
        self.state_manager.restore_state().await?;

        Ok(())
    }

    async fn start_background_processing(&self) -> Result<JoinHandle<()>> {
        let dispatcher = self.task_dispatcher.clone();
        let agent_pool = self.agent_pool.clone();
        let resource_monitor = self.resource_monitor.clone();
        let state_manager = self.state_manager.clone();
        let shutdown_tx = self.shutdown_tx.clone();

        let handle = tokio::spawn(async move {
            let mut shutdown_rx = shutdown_tx.subscribe();
            let mut health_check_interval = interval(Duration::from_secs(30));
            let mut state_sync_interval = interval(Duration::from_secs(10));

            // Start dispatcher loop in separate task
            let dispatcher_handle = tokio::spawn({
                let dispatcher = dispatcher.clone();
                let mut shutdown_rx = shutdown_tx.subscribe();
                async move {
                    dispatcher.dispatch_loop(shutdown_rx).await
                }
            });

            loop {
                select! {
                    // Health check agents
                    _ = health_check_interval.tick() => {
                        if let Err(e) = agent_pool.health_check_all().await {
                            error!("Agent health check failed: {}", e);
                        }
                    }

                    // Sync state to disk
                    _ = state_sync_interval.tick() => {
                        if let Err(e) = state_manager.sync_to_disk().await {
                            error!("State sync failed: {}", e);
                        }
                    }

                    // Check resource limits and scale
                    _ = async {
                        if resource_monitor.should_throttle().await {
                            agent_pool.reduce_concurrency().await
                        } else {
                            // Check if we should scale up
                            let queue_size = get_queue_size().await?;
                            agent_pool.scale_pool(queue_size).await
                        }
                    } => {}

                    // Shutdown signal
                    _ = shutdown_rx.recv() => {
                        info!("Background processor shutting down");
                        dispatcher_handle.abort();
                        break;
                    }
                }
            }

            // Cleanup
            let _ = dispatcher_handle.await;
            Ok::<(), anyhow::Error>(())
        });

        Ok(handle)
    }
}
```

### TODO #4: State Persistence

**Location**: `src/application/state_manager.rs` (new file)

```rust
pub struct StateManager {
    file_path: PathBuf,
    runtime_state: Arc<RwLock<RuntimeState>>,
    db_state: Arc<dyn StateRepository>,
}

#[derive(Serialize, Deserialize)]
struct RuntimeState {
    swarm_status: SwarmState,
    max_agents: usize,
    tasks_processed: u64,
    tasks_failed: u64,
    agent_stats: HashMap<Uuid, AgentStats>,
    last_checkpoint: DateTime<Utc>,
}

impl StateManager {
    pub async fn restore_state(&self) -> Result<()> {
        // 1. Read file state
        let file_state = self.read_file_state().await?;

        // 2. Query database state
        let db_agents = self.db_state.get_all_agents().await?;
        let db_metrics = self.db_state.get_metrics().await?;

        // 3. Reconcile states
        let mut runtime = self.runtime_state.write().await;
        runtime.swarm_status = file_state.state;
        runtime.max_agents = file_state.max_agents;
        runtime.tasks_processed = db_metrics.tasks_processed;
        runtime.tasks_failed = db_metrics.tasks_failed;

        // 4. Handle crash recovery
        if file_state.state == SwarmState::Running {
            // System crashed while running, clean up
            warn!("Detected unclean shutdown, performing recovery");

            // Mark all running tasks as failed
            self.recover_running_tasks().await?;

            // Terminate stale agents
            self.cleanup_stale_agents().await?;
        }

        Ok(())
    }

    pub async fn sync_to_disk(&self) -> Result<()> {
        let runtime = self.runtime_state.read().await;

        let file_state = SwarmStateFile {
            state: runtime.swarm_status.to_string(),
            max_agents: runtime.max_agents,
            tasks_processed: runtime.tasks_processed,
            tasks_failed: runtime.tasks_failed,
            last_checkpoint: Utc::now(),
        };

        self.write_file_state(&file_state).await?;
        Ok(())
    }

    async fn recover_running_tasks(&self) -> Result<()> {
        let filters = TaskFilters {
            status: Some(vec![TaskStatus::Running]),
            ..Default::default()
        };

        let running_tasks = self.task_queue.list(filters).await?;

        for mut task in running_tasks {
            warn!("Recovering task {} from unclean shutdown", task.id);

            // Reset to ready state for retry
            task.status = TaskStatus::Ready;
            task.started_at = None;
            self.task_queue.update(&task).await?;
        }

        Ok(())
    }
}
```

### TODO #5: Integration Points

**Location**: `src/cli/service/swarm_service.rs` (enhance existing)

```rust
impl SwarmService {
    pub async fn start(&self, max_agents: usize) -> Result<()> {
        // 1. Update file state (existing)
        let mut state = Self::read_state()?;
        state.state = "Running".to_string();
        state.max_agents = max_agents;
        Self::write_state(&state)?;

        // 2. NEW: Start daemon process if not running
        if !self.is_daemon_running().await? {
            self.start_daemon_process().await?;
        }

        // 3. NEW: Send start command to daemon via IPC
        let client = self.connect_to_daemon().await?;
        client.send_command(DaemonCommand::Start { max_agents }).await?;

        Ok(())
    }

    async fn start_daemon_process(&self) -> Result<()> {
        // Fork daemon process that runs SwarmOrchestrator
        let daemon_path = env::current_exe()?
            .parent()
            .unwrap()
            .join("abathur-daemon");

        Command::new(daemon_path)
            .arg("--daemon")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?;

        // Wait for daemon to be ready
        tokio::time::sleep(Duration::from_secs(2)).await;

        Ok(())
    }

    async fn connect_to_daemon(&self) -> Result<DaemonClient> {
        // Connect via Unix socket or TCP
        let socket_path = dirs::home_dir()
            .unwrap()
            .join(".abathur/daemon.sock");

        DaemonClient::connect(socket_path).await
    }
}
```

## State Management

### State Layers

1. **File State** (`~/.abathur/swarm_state.json`)
   - Persists across CLI invocations
   - Minimal state: running/stopped, max_agents, basic stats
   - Human-readable for debugging

2. **Runtime State** (In-memory)
   - Full agent pool state
   - Task assignment mappings
   - Performance metrics
   - Resource usage

3. **Database State** (SQLite)
   - Authoritative source for agents and tasks
   - Audit trail
   - Metrics history

### State Synchronization Strategy

```
CLI Start Command
    ↓
Read File State → Start/Connect Daemon → Load DB State
                        ↓
                  Initialize Runtime State
                        ↓
                  Begin Processing Loop
                        ↓
              Periodic State Sync (every 10s)
                   ├─→ Update File State
                   └─→ Update DB State

Crash Recovery:
File State + DB State → Reconstruct Runtime State
```

## Risk Assessment

### Risk Matrix

| Risk | Likelihood | Impact | Mitigation Strategy |
|------|------------|--------|-------------------|
| **Agent Deadlock** | Medium | High | Implement timeout and health checks |
| **Resource Exhaustion** | Medium | High | Resource monitoring with throttling |
| **Task Queue Starvation** | Low | Medium | Priority-based scheduling |
| **Database Corruption** | Low | Critical | WAL mode, regular backups |
| **Daemon Process Crash** | Medium | High | Systemd/launchd integration for auto-restart |
| **Network Partition** | Low | Medium | Retry logic with exponential backoff |
| **Memory Leak** | Medium | High | Periodic agent recycling |
| **Circular Dependencies** | Low | High | Dependency validation at submission |

### Detailed Mitigation Strategies

#### 1. Agent Deadlock Prevention
```rust
// Implement task timeout
impl AgentWorker {
    async fn execute_task_with_timeout(&self, task: Task) -> Result<TaskResult> {
        let timeout_duration = Duration::from_secs(task.max_execution_timeout_seconds);

        match timeout(timeout_duration, self.executor.execute(task)).await {
            Ok(result) => result,
            Err(_) => {
                error!("Task {} timed out after {}s", task.id, timeout_duration.as_secs());
                TaskResult::Failure("Execution timeout".to_string())
            }
        }
    }
}
```

#### 2. Resource Exhaustion Prevention
```rust
// Adaptive agent pool sizing
impl AgentPool {
    async fn check_resource_pressure(&self) -> ResourcePressure {
        let status = self.resource_monitor.get_status().await?;

        if status.cpu_percent > 90.0 || status.memory_mb > self.memory_limit * 0.9 {
            ResourcePressure::Critical
        } else if status.should_throttle {
            ResourcePressure::High
        } else {
            ResourcePressure::Normal
        }
    }

    async fn adapt_pool_size(&self, pressure: ResourcePressure) -> Result<()> {
        match pressure {
            ResourcePressure::Critical => {
                // Stop spawning new agents
                self.freeze_pool().await?;
                // Terminate idle agents
                self.terminate_idle_agents(self.agents.len() / 2).await?;
            }
            ResourcePressure::High => {
                // Reduce pool size gradually
                self.terminate_idle_agents(1).await?;
            }
            ResourcePressure::Normal => {
                // Normal scaling logic
                self.scale_based_on_queue().await?;
            }
        }
        Ok(())
    }
}
```

#### 3. Crash Recovery
```rust
impl SwarmOrchestrator {
    async fn recover_from_crash(&self) -> Result<()> {
        info!("Performing crash recovery");

        // 1. Clean up stale agents
        let stale_threshold = Duration::seconds(60);
        let stale_agents = self.agent_repo
            .find_stale_agents(stale_threshold)
            .await?;

        for agent in stale_agents {
            warn!("Terminating stale agent {}", agent.id);
            self.agent_repo.terminate(agent.id).await?;
        }

        // 2. Reset running tasks
        let running_tasks = self.task_queue
            .list(TaskFilters {
                status: Some(vec![TaskStatus::Running]),
                ..Default::default()
            })
            .await?;

        for mut task in running_tasks {
            warn!("Resetting task {} to ready state", task.id);
            task.status = TaskStatus::Ready;
            task.started_at = None;
            self.task_queue.update(&task).await?;
        }

        // 3. Rebuild agent pool
        self.agent_pool.initialize().await?;

        Ok(())
    }
}
```

## Testing Strategy

### Unit Tests

1. **Agent Pool Tests**
   - Test agent creation and termination
   - Test scaling logic
   - Test health check mechanism
   - Test agent assignment

2. **Task Dispatcher Tests**
   - Test task polling
   - Test assignment strategies
   - Test retry logic
   - Test failure handling

3. **State Manager Tests**
   - Test state persistence
   - Test crash recovery
   - Test state reconciliation

### Integration Tests

1. **End-to-End Task Processing**
```rust
#[tokio::test]
async fn test_task_processing_e2e() {
    // Setup
    let orchestrator = setup_test_orchestrator().await;
    orchestrator.start().await.unwrap();

    // Submit task
    let task = create_test_task();
    let task_id = task_queue.submit(task).await.unwrap();

    // Wait for completion
    let result = wait_for_task_completion(task_id, Duration::from_secs(30)).await;

    assert_eq!(result.status, TaskStatus::Completed);

    // Cleanup
    orchestrator.stop().await.unwrap();
}
```

2. **Failure Recovery Test**
```rust
#[tokio::test]
async fn test_crash_recovery() {
    // Start orchestrator
    let orchestrator = setup_test_orchestrator().await;
    orchestrator.start().await.unwrap();

    // Submit tasks
    let task_ids = submit_test_tasks(5).await;

    // Simulate crash
    orchestrator.force_shutdown().await;

    // Restart
    let recovered = setup_test_orchestrator().await;
    recovered.start().await.unwrap();

    // Verify tasks are reprocessed
    for task_id in task_ids {
        let task = task_queue.get(task_id).await.unwrap();
        assert_ne!(task.status, TaskStatus::Running);
    }
}
```

3. **Resource Pressure Test**
```rust
#[tokio::test]
async fn test_resource_throttling() {
    let orchestrator = setup_test_orchestrator().await;
    orchestrator.start().await.unwrap();

    // Submit many tasks to create pressure
    submit_test_tasks(100).await;

    // Monitor agent pool size
    let initial_size = orchestrator.agent_pool.size().await;

    // Simulate high resource usage
    simulate_high_cpu_usage().await;

    // Pool should reduce
    tokio::time::sleep(Duration::from_secs(10)).await;
    let reduced_size = orchestrator.agent_pool.size().await;

    assert!(reduced_size < initial_size);
}
```

### Performance Tests

1. **Throughput Test**
   - Measure tasks/second processing rate
   - Test with various pool sizes
   - Benchmark against requirements

2. **Latency Test**
   - Measure task pickup latency
   - Measure end-to-end processing time
   - Test under various load conditions

3. **Scalability Test**
   - Test with 1, 10, 100, 1000 tasks
   - Measure resource usage growth
   - Validate linear scaling

### Chaos Engineering Tests

1. **Random Agent Failures**
   - Kill random agents during processing
   - Verify task recovery and completion

2. **Database Connection Loss**
   - Simulate database unavailability
   - Verify graceful degradation

3. **Resource Exhaustion**
   - Limit available memory/CPU
   - Verify throttling and recovery

## Implementation Timeline

### Phase 1: Core Infrastructure (Week 1)
- [ ] Implement AgentPool with basic lifecycle
- [ ] Implement AgentWorker with task execution
- [ ] Basic TaskDispatcher without retry logic
- [ ] Simple StateManager with file persistence

### Phase 2: Robustness (Week 2)
- [ ] Add health checks and heartbeats
- [ ] Implement retry logic and failure handling
- [ ] Add resource monitoring integration
- [ ] Implement crash recovery

### Phase 3: Optimization (Week 3)
- [ ] Dynamic scaling logic
- [ ] Assignment strategies
- [ ] Performance monitoring
- [ ] Metrics collection

### Phase 4: Production Readiness (Week 4)
- [ ] Systemd/launchd integration
- [ ] Comprehensive testing
- [ ] Documentation
- [ ] Deployment tooling

## Conclusion

This architecture provides a robust, scalable, and fault-tolerant swarm orchestrator for Abathur. The design separates concerns clearly, handles failures gracefully, and provides multiple layers of state management for reliability. The implementation can be done incrementally, with each phase adding more sophistication while maintaining backward compatibility.

Key advantages:
- **Resilient**: Handles crashes, failures, and resource constraints
- **Scalable**: Dynamic agent pool sizing based on workload
- **Observable**: Comprehensive monitoring and metrics
- **Maintainable**: Clear separation of concerns and modular design
- **Testable**: Each component can be tested in isolation

The system is ready for implementation following the detailed recommendations provided for each TODO section.