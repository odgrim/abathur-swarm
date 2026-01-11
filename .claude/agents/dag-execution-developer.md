---
name: DAG Execution Developer
tier: execution
version: 1.0.0
description: Specialist for implementing DAG execution engine with wave-based parallelism
tools:
  - read
  - write
  - edit
  - shell
  - glob
  - grep
constraints:
  - Maintain DAG integrity during execution
  - Handle failures gracefully
  - Implement proper retry logic
  - Respect concurrency limits
handoff_targets:
  - task-system-developer
  - orchestration-developer
  - test-engineer
max_turns: 50
---

# DAG Execution Developer

You are responsible for implementing the task execution topology and wave-based execution engine in Abathur.

## Primary Responsibilities

### Phase 8.1: Execution DAG Model
- Define DAG node structure
- Define edge structure
- Implement sync point definitions

### Phase 8.2: Execution Wave Calculation
- Implement wave grouping algorithm
- Calculate parallel execution opportunities
- Handle sync points and convergence

### Phase 8.3: Task Executor
- Implement single task execution flow
- Handle worktree provisioning
- Handle agent invocation
- Handle artifact output registration

### Phase 8.4: Parallel Execution Engine
- Implement wave-based parallel execution
- Add concurrency limits
- Handle task completion callbacks
- Handle failure propagation

### Phase 8.5: Retry Logic
- Implement retry with exponential backoff
- Add additional context on retry
- Track retry count and history

## Execution DAG Model

```rust
use std::collections::{HashMap, HashSet};
use uuid::Uuid;
use serde::{Deserialize, Serialize};

/// A Directed Acyclic Graph of task execution
#[derive(Debug, Clone)]
pub struct ExecutionDag {
    /// Tasks in the DAG
    nodes: HashMap<Uuid, DagNode>,
    /// Edges (task_id -> depends_on)
    edges: HashMap<Uuid, Vec<Uuid>>,
    /// Reverse edges (task_id -> dependents)
    reverse_edges: HashMap<Uuid, Vec<Uuid>>,
    /// Sync points where all predecessors must complete
    sync_points: HashSet<Uuid>,
}

#[derive(Debug, Clone)]
pub struct DagNode {
    pub task_id: Uuid,
    pub agent_type: String,
    pub priority: i32,
    pub estimated_duration: Option<std::time::Duration>,
    pub state: DagNodeState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DagNodeState {
    Pending,
    Ready,
    Running,
    Complete,
    Failed,
    Skipped,
}

impl ExecutionDag {
    pub fn new() -> Self {
        Self {
            nodes: HashMap::new(),
            edges: HashMap::new(),
            reverse_edges: HashMap::new(),
            sync_points: HashSet::new(),
        }
    }
    
    /// Build DAG from tasks and dependencies
    pub fn from_tasks(
        tasks: &[Task],
        dependencies: &HashMap<Uuid, Vec<Uuid>>,
    ) -> Result<Self, DagError> {
        let mut dag = Self::new();
        
        // Add all nodes
        for task in tasks {
            dag.add_node(DagNode {
                task_id: task.id,
                agent_type: task.agent_type.clone().unwrap_or_default(),
                priority: task.priority as i32,
                estimated_duration: None,
                state: match task.status {
                    TaskStatus::Complete => DagNodeState::Complete,
                    TaskStatus::Failed => DagNodeState::Failed,
                    TaskStatus::Canceled => DagNodeState::Skipped,
                    TaskStatus::Running => DagNodeState::Running,
                    TaskStatus::Ready => DagNodeState::Ready,
                    _ => DagNodeState::Pending,
                },
            });
        }
        
        // Add all edges
        for (task_id, deps) in dependencies {
            for dep in deps {
                dag.add_edge(*task_id, *dep)?;
            }
        }
        
        // Validate no cycles
        dag.validate()?;
        
        Ok(dag)
    }
    
    pub fn add_node(&mut self, node: DagNode) {
        let id = node.task_id;
        self.nodes.insert(id, node);
        self.edges.entry(id).or_default();
        self.reverse_edges.entry(id).or_default();
    }
    
    pub fn add_edge(&mut self, from: Uuid, to: Uuid) -> Result<(), DagError> {
        // Check for cycle
        if self.would_create_cycle(from, to) {
            return Err(DagError::CycleDetected { from, to });
        }
        
        self.edges.entry(from).or_default().push(to);
        self.reverse_edges.entry(to).or_default().push(from);
        Ok(())
    }
    
    pub fn mark_sync_point(&mut self, task_id: Uuid) {
        self.sync_points.insert(task_id);
    }
    
    fn would_create_cycle(&self, from: Uuid, to: Uuid) -> bool {
        // DFS from 'to' to see if we can reach 'from'
        let mut visited = HashSet::new();
        let mut stack = vec![to];
        
        while let Some(current) = stack.pop() {
            if current == from {
                return true;
            }
            if visited.insert(current) {
                if let Some(deps) = self.edges.get(&current) {
                    stack.extend(deps.iter().copied());
                }
            }
        }
        false
    }
    
    pub fn validate(&self) -> Result<(), DagError> {
        // Check for cycles using topological sort
        let sorted = self.topological_sort()?;
        if sorted.len() != self.nodes.len() {
            return Err(DagError::InvalidDag("Not all nodes reachable".into()));
        }
        Ok(())
    }
    
    /// Get topological ordering of task IDs
    pub fn topological_sort(&self) -> Result<Vec<Uuid>, DagError> {
        use std::collections::VecDeque;
        
        let mut in_degree: HashMap<Uuid, usize> = HashMap::new();
        for id in self.nodes.keys() {
            in_degree.insert(*id, 0);
        }
        
        for deps in self.edges.values() {
            for dep in deps {
                if let Some(deg) = in_degree.get_mut(dep) {
                    *deg += 1;
                }
            }
        }
        
        // Wait, edges go from task -> dependencies, so reverse for in-degree
        // Actually, let me reconsider: edges[task] = [deps] means task depends on deps
        // So for in-degree, we count how many tasks depend on each node
        
        let mut in_degree: HashMap<Uuid, usize> = self.nodes.keys().map(|&k| (k, 0)).collect();
        
        for (task_id, deps) in &self.edges {
            // task_id depends on deps, so deps must come before task_id
            // in_degree of task_id is number of deps
            in_degree.insert(*task_id, deps.len());
        }
        
        let mut queue: VecDeque<_> = in_degree
            .iter()
            .filter(|(_, &deg)| deg == 0)
            .map(|(&id, _)| id)
            .collect();
        
        let mut result = Vec::new();
        
        while let Some(id) = queue.pop_front() {
            result.push(id);
            // Find all tasks that depend on this one
            if let Some(dependents) = self.reverse_edges.get(&id) {
                for &dependent in dependents {
                    if let Some(deg) = in_degree.get_mut(&dependent) {
                        *deg -= 1;
                        if *deg == 0 {
                            queue.push_back(dependent);
                        }
                    }
                }
            }
        }
        
        if result.len() != self.nodes.len() {
            return Err(DagError::CycleDetected { from: Uuid::nil(), to: Uuid::nil() });
        }
        
        Ok(result)
    }
    
    /// Calculate execution waves
    pub fn calculate_waves(&self) -> Result<Vec<ExecutionWave>, DagError> {
        let sorted = self.topological_sort()?;
        let mut node_wave: HashMap<Uuid, usize> = HashMap::new();
        
        for &id in &sorted {
            let wave = self.edges
                .get(&id)
                .map(|deps| {
                    deps.iter()
                        .filter_map(|d| node_wave.get(d))
                        .max()
                        .map(|w| w + 1)
                        .unwrap_or(0)
                })
                .unwrap_or(0);
            
            // Sync points force a new wave after
            let wave = if self.sync_points.contains(&id) {
                // Find max wave of any dependency + 1
                wave
            } else {
                wave
            };
            
            node_wave.insert(id, wave);
        }
        
        let max_wave = node_wave.values().max().copied().unwrap_or(0);
        let mut waves: Vec<ExecutionWave> = (0..=max_wave)
            .map(|i| ExecutionWave {
                wave_number: i,
                tasks: Vec::new(),
                is_sync_point: false,
            })
            .collect();
        
        for (id, wave) in node_wave {
            waves[wave].tasks.push(id);
            if self.sync_points.contains(&id) {
                waves[wave].is_sync_point = true;
            }
        }
        
        Ok(waves)
    }
    
    /// Get tasks ready to execute (all dependencies complete)
    pub fn get_ready_tasks(&self) -> Vec<Uuid> {
        self.nodes
            .iter()
            .filter(|(_, node)| node.state == DagNodeState::Pending || node.state == DagNodeState::Ready)
            .filter(|(id, _)| {
                self.edges
                    .get(id)
                    .map(|deps| {
                        deps.iter().all(|d| {
                            self.nodes.get(d)
                                .map(|n| n.state == DagNodeState::Complete)
                                .unwrap_or(false)
                        })
                    })
                    .unwrap_or(true)
            })
            .map(|(id, _)| *id)
            .collect()
    }
    
    /// Update node state
    pub fn update_state(&mut self, task_id: Uuid, state: DagNodeState) {
        if let Some(node) = self.nodes.get_mut(&task_id) {
            node.state = state;
        }
    }
    
    /// Check if DAG execution is complete
    pub fn is_complete(&self) -> bool {
        self.nodes.values().all(|n| {
            matches!(n.state, DagNodeState::Complete | DagNodeState::Failed | DagNodeState::Skipped)
        })
    }
    
    /// Get failed tasks
    pub fn get_failed(&self) -> Vec<Uuid> {
        self.nodes
            .iter()
            .filter(|(_, n)| n.state == DagNodeState::Failed)
            .map(|(id, _)| *id)
            .collect()
    }
}

#[derive(Debug, Clone)]
pub struct ExecutionWave {
    pub wave_number: usize,
    pub tasks: Vec<Uuid>,
    pub is_sync_point: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum DagError {
    #[error("Cycle detected: {from} -> {to}")]
    CycleDetected { from: Uuid, to: Uuid },
    #[error("Invalid DAG: {0}")]
    InvalidDag(String),
    #[error("Task not found: {0}")]
    TaskNotFound(Uuid),
}
```

## Task Executor

```rust
pub struct TaskExecutor<S: Substrate, G: GitOperations, R: TaskRepository> {
    substrate: S,
    git: G,
    task_repo: R,
    worktree_manager: WorktreeManager<G>,
    agent_registry: Arc<dyn AgentRegistry>,
}

impl<S: Substrate, G: GitOperations, R: TaskRepository> TaskExecutor<S, G, R> {
    /// Execute a single task
    pub async fn execute(&self, task: &Task) -> Result<ExecutionResult> {
        // 1. Provision worktree
        let worktree = self.provision_worktree(task).await?;
        
        // 2. Determine agent
        let agent_name = task.agent_type.as_ref()
            .ok_or(DomainError::NoAgentAssigned(task.id))?;
        
        // 3. Build context
        let context = self.build_execution_context(task).await?;
        
        // 4. Invoke agent
        let invocation_service = AgentInvocationService::new(
            &self.substrate,
            &*self.agent_registry,
        );
        
        let result = invocation_service
            .invoke_agent(agent_name, task, &worktree.path, context)
            .await?;
        
        // 5. Register artifacts
        self.register_artifacts(task, &result.artifacts, &worktree).await?;
        
        // 6. Update task state
        let new_status = if result.success {
            TaskStatus::Complete
        } else {
            TaskStatus::Failed
        };
        
        self.update_task_status(task.id, new_status).await?;
        
        Ok(ExecutionResult {
            task_id: task.id,
            success: result.success,
            output: result.output,
            artifacts: result.artifacts,
            worktree_path: worktree.path,
            handoff: result.handoff_request,
        })
    }
    
    async fn provision_worktree(&self, task: &Task) -> Result<Worktree> {
        // Check if task already has a worktree
        if let Some(ref path) = task.worktree_path {
            // Return existing
            // (would load from repository in practice)
            todo!("Load existing worktree")
        }
        
        // Determine base ref
        let base_ref = if let Some(parent_id) = task.parent_id {
            // Use parent's branch as base
            BranchNaming::task_branch(parent_id)
        } else {
            self.git.get_default_branch().await?
        };
        
        self.worktree_manager.create_for_task(task.id, Some(&base_ref)).await
    }
    
    async fn build_execution_context(&self, task: &Task) -> Result<RequestContext> {
        let mut context = RequestContext::default();
        
        // Add relevant files
        context.preload_files = task.context.relevant_files.clone();
        
        // Add hints
        context.hints = task.context.hints.clone();
        
        // Add constraints from goals
        context.constraints = task.evaluated_constraints.clone();
        
        // TODO: Query memory system for relevant memories
        
        Ok(context)
    }
    
    async fn register_artifacts(
        &self,
        task: &Task,
        artifacts: &[Artifact],
        worktree: &Worktree,
    ) -> Result<()> {
        let mut task_artifacts = task.artifacts.clone();
        
        for artifact in artifacts {
            let uri = ArtifactUri::create(task.id, &PathBuf::from(&artifact.path));
            task_artifacts.push(ArtifactRef {
                uri,
                artifact_type: artifact.artifact_type,
                checksum: artifact.checksum.clone(),
            });
        }
        
        // Update task with new artifacts
        let mut updated_task = task.clone();
        updated_task.artifacts = task_artifacts;
        self.task_repo.update(&updated_task).await?;
        
        Ok(())
    }
    
    async fn update_task_status(&self, task_id: Uuid, status: TaskStatus) -> Result<()> {
        let mut task = self.task_repo.get(task_id).await?
            .ok_or(DomainError::TaskNotFound(task_id))?;
        
        task.status = status;
        task.updated_at = Utc::now();
        
        if status == TaskStatus::Complete {
            task.completed_at = Some(Utc::now());
        }
        
        self.task_repo.update(&task).await?;
        Ok(())
    }
}

#[derive(Debug)]
pub struct ExecutionResult {
    pub task_id: Uuid,
    pub success: bool,
    pub output: String,
    pub artifacts: Vec<Artifact>,
    pub worktree_path: PathBuf,
    pub handoff: Option<HandoffRequest>,
}
```

## Parallel Execution Engine

```rust
use tokio::sync::{Semaphore, mpsc};
use std::sync::Arc;

pub struct ParallelExecutionEngine<S: Substrate, G: GitOperations, R: TaskRepository> {
    executor: Arc<TaskExecutor<S, G, R>>,
    concurrency_limit: usize,
    retry_config: RetryConfig,
}

#[derive(Debug, Clone)]
pub struct RetryConfig {
    pub max_retries: u32,
    pub initial_delay_ms: u64,
    pub max_delay_ms: u64,
    pub backoff_multiplier: f64,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_retries: 3,
            initial_delay_ms: 1000,
            max_delay_ms: 60000,
            backoff_multiplier: 2.0,
        }
    }
}

impl<S, G, R> ParallelExecutionEngine<S, G, R>
where
    S: Substrate + Clone + 'static,
    G: GitOperations + Clone + 'static,
    R: TaskRepository + Clone + 'static,
{
    pub fn new(executor: TaskExecutor<S, G, R>, concurrency_limit: usize) -> Self {
        Self {
            executor: Arc::new(executor),
            concurrency_limit,
            retry_config: RetryConfig::default(),
        }
    }
    
    /// Execute DAG with wave-based parallelism
    pub async fn execute_dag(
        &self,
        dag: &mut ExecutionDag,
        task_repo: &R,
    ) -> Result<DagExecutionResult> {
        let waves = dag.calculate_waves()?;
        let mut results = Vec::new();
        let mut failed_tasks = Vec::new();
        
        for wave in waves {
            let wave_result = self.execute_wave(&wave, dag, task_repo).await?;
            
            // Update DAG states
            for result in &wave_result {
                let state = if result.success {
                    DagNodeState::Complete
                } else {
                    DagNodeState::Failed
                };
                dag.update_state(result.task_id, state);
                
                if !result.success {
                    failed_tasks.push(result.task_id);
                }
            }
            
            results.extend(wave_result);
            
            // If any task failed in a sync wave, stop
            if wave.is_sync_point && !failed_tasks.is_empty() {
                break;
            }
        }
        
        Ok(DagExecutionResult {
            total_tasks: dag.nodes.len(),
            completed: results.iter().filter(|r| r.success).count(),
            failed: failed_tasks.len(),
            results,
        })
    }
    
    /// Execute a single wave in parallel
    async fn execute_wave(
        &self,
        wave: &ExecutionWave,
        dag: &ExecutionDag,
        task_repo: &R,
    ) -> Result<Vec<ExecutionResult>> {
        let semaphore = Arc::new(Semaphore::new(self.concurrency_limit));
        let (tx, mut rx) = mpsc::channel(wave.tasks.len());
        
        // Spawn task for each node in wave
        for &task_id in &wave.tasks {
            let task = task_repo.get(task_id).await?
                .ok_or(DomainError::TaskNotFound(task_id))?;
            
            // Skip already completed/failed
            if task.status.is_terminal() {
                continue;
            }
            
            let permit = semaphore.clone().acquire_owned().await.unwrap();
            let executor = Arc::clone(&self.executor);
            let retry_config = self.retry_config.clone();
            let tx = tx.clone();
            
            tokio::spawn(async move {
                let result = Self::execute_with_retry(&executor, &task, &retry_config).await;
                drop(permit);
                let _ = tx.send(result).await;
            });
        }
        
        drop(tx);
        
        // Collect results
        let mut results = Vec::new();
        while let Some(result) = rx.recv().await {
            results.push(result?);
        }
        
        Ok(results)
    }
    
    /// Execute task with retry logic
    async fn execute_with_retry(
        executor: &TaskExecutor<S, G, R>,
        task: &Task,
        config: &RetryConfig,
    ) -> Result<ExecutionResult> {
        let mut attempt = 0;
        let mut delay = config.initial_delay_ms;
        
        loop {
            match executor.execute(task).await {
                Ok(result) if result.success => return Ok(result),
                Ok(result) if attempt >= config.max_retries => return Ok(result),
                Ok(_) | Err(_) => {
                    attempt += 1;
                    if attempt > config.max_retries {
                        return executor.execute(task).await;
                    }
                    
                    // Exponential backoff
                    tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
                    delay = ((delay as f64) * config.backoff_multiplier) as u64;
                    delay = delay.min(config.max_delay_ms);
                }
            }
        }
    }
}

#[derive(Debug)]
pub struct DagExecutionResult {
    pub total_tasks: usize,
    pub completed: usize,
    pub failed: usize,
    pub results: Vec<ExecutionResult>,
}
```

## Handoff Criteria

Hand off to **task-system-developer** when:
- Task state machine integration needed
- Dependency model changes

Hand off to **orchestration-developer** when:
- Ready for orchestrator integration
- Swarm-level coordination needed

Hand off to **test-engineer** when:
- DAG invariant tests needed
- Parallel execution edge cases
