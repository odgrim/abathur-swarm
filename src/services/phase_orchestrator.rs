//! Phase Orchestrator service.
//!
//! A deterministic Rust state machine that drives workflow execution
//! phase-by-phase, invoking the overmind only at discrete decision points
//! (decomposition, recovery, escalation). Each phase bounds context usage
//! by running through the existing `DagExecutor` with fresh agent sessions.

use std::sync::Arc;

use tokio::sync::{mpsc, RwLock};
use tracing::{debug, error, info, warn};
use uuid::Uuid;

use crate::domain::errors::{DomainError, DomainResult};
use crate::domain::models::{
    Task, TaskDag, TaskSource, TaskStatus, IntentSatisfaction,
    workflow::{
        PhaseDefinition, PhaseStatus, PhaseType,
        WorkflowDefinition, WorkflowInstance, WorkflowStatus,
    },
    overmind::{StuckStateRecoveryRequest, GoalContext, FailureRecord},
};
use crate::domain::ports::{
    AgentRepository, GoalRepository, Substrate, TaskRepository, WorkflowRepository,
};
use crate::services::dag_executor::{DagExecutor, ExecutionEvent, ExecutorConfig};
use crate::services::event_bus::{EventBus, EventPayload, EventSeverity};
use crate::services::event_factory::workflow_event;
use crate::services::intent_verifier::IntentVerifierService;
use crate::services::OvermindService;

/// Configuration for the phase orchestrator.
#[derive(Debug, Clone)]
pub struct PhaseOrchestratorConfig {
    /// Default executor config for phases.
    pub executor_config: ExecutorConfig,
    /// Maximum retries per phase.
    pub max_phase_retries: u32,
    /// Whether to invoke overmind for recovery on phase failure.
    pub enable_overmind_recovery: bool,
}

impl Default for PhaseOrchestratorConfig {
    fn default() -> Self {
        Self {
            executor_config: ExecutorConfig::default(),
            max_phase_retries: 2,
            enable_overmind_recovery: true,
        }
    }
}

/// The phase orchestrator service.
///
/// Drives workflow execution as a deterministic state machine. It does NOT
/// hold LLM sessions. It reacts to events and drives phases forward.
pub struct PhaseOrchestrator<T, A, G>
where
    T: TaskRepository + 'static,
    A: AgentRepository + 'static,
    G: GoalRepository + 'static,
{
    task_repo: Arc<T>,
    agent_repo: Arc<A>,
    goal_repo: Arc<G>,
    substrate: Arc<dyn Substrate>,
    workflow_repo: Arc<dyn WorkflowRepository>,
    event_bus: Arc<EventBus>,
    overmind: Option<Arc<OvermindService>>,
    intent_verifier: Option<Arc<IntentVerifierService<G, T>>>,
    config: PhaseOrchestratorConfig,
    /// Active workflow instances being tracked.
    active_workflows: Arc<RwLock<Vec<(WorkflowDefinition, WorkflowInstance)>>>,
}

impl<T, A, G> PhaseOrchestrator<T, A, G>
where
    T: TaskRepository + 'static,
    A: AgentRepository + 'static,
    G: GoalRepository + 'static,
{
    pub fn new(
        task_repo: Arc<T>,
        agent_repo: Arc<A>,
        goal_repo: Arc<G>,
        substrate: Arc<dyn Substrate>,
        workflow_repo: Arc<dyn WorkflowRepository>,
        event_bus: Arc<EventBus>,
        config: PhaseOrchestratorConfig,
    ) -> Self {
        Self {
            task_repo,
            agent_repo,
            goal_repo,
            substrate,
            workflow_repo,
            event_bus,
            overmind: None,
            intent_verifier: None,
            config,
            active_workflows: Arc::new(RwLock::new(Vec::new())),
        }
    }

    /// Set the overmind service for recovery decisions.
    pub fn with_overmind(mut self, overmind: Arc<OvermindService>) -> Self {
        self.overmind = Some(overmind);
        self
    }

    /// Set the intent verifier service for verification phases.
    pub fn with_intent_verifier(mut self, verifier: Arc<IntentVerifierService<G, T>>) -> Self {
        self.intent_verifier = Some(verifier);
        self
    }

    // ========================================================================
    // Workflow Lifecycle
    // ========================================================================

    /// Start executing a workflow from a definition.
    ///
    /// Creates a WorkflowInstance, persists it, and begins advancing.
    pub async fn execute_workflow(
        &self,
        definition: WorkflowDefinition,
    ) -> DomainResult<WorkflowInstance> {
        // Validate the DAG
        definition
            .validate_dag()
            .map_err(|e| DomainError::ValidationFailed(e))?;

        // Persist definition
        self.workflow_repo.save_definition(&definition).await?;

        // Create instance
        let mut instance = WorkflowInstance::new(&definition);
        instance.status = WorkflowStatus::Running;
        instance.updated_at = chrono::Utc::now();

        // Persist instance
        self.workflow_repo.save_instance(&instance).await?;

        // Track actively
        {
            let mut active = self.active_workflows.write().await;
            active.push((definition.clone(), instance.clone()));
        }

        // Emit workflow started event
        self.event_bus
            .publish(workflow_event(
                EventSeverity::Info,
                Some(definition.goal_id),
                EventPayload::WorkflowStarted {
                    workflow_instance_id: instance.id,
                    workflow_name: definition.name.clone(),
                    goal_id: definition.goal_id,
                    phase_count: definition.phases.len(),
                },
            ))
            .await;

        info!(
            workflow_id = %instance.id,
            goal_id = %definition.goal_id,
            phases = definition.phases.len(),
            "Workflow started: {}",
            definition.name,
        );

        // Start advancing
        self.advance_workflow(&mut instance, &definition).await?;

        // Persist updated state
        self.workflow_repo.update_instance(&instance).await?;
        self.update_active_instance(&instance).await;

        Ok(instance)
    }

    /// Resume a running workflow (e.g., after crash recovery).
    pub async fn resume_workflow(&self, instance_id: Uuid) -> DomainResult<()> {
        let instance = self
            .workflow_repo
            .get_instance(instance_id)
            .await?
            .ok_or(DomainError::WorkflowNotFound(instance_id))?;

        if instance.status != WorkflowStatus::Running {
            return Ok(()); // Nothing to resume
        }

        let definition = self
            .workflow_repo
            .get_definition(instance.workflow_id)
            .await?
            .ok_or(DomainError::WorkflowNotFound(instance.workflow_id))?;

        let mut instance = instance;
        self.advance_workflow(&mut instance, &definition).await?;
        self.workflow_repo.update_instance(&instance).await?;
        self.update_active_instance(&instance).await;

        Ok(())
    }

    /// Recover running workflows on startup.
    pub async fn recover_running_workflows(&self) -> DomainResult<usize> {
        let running = self
            .workflow_repo
            .get_instances_by_status(WorkflowStatus::Running)
            .await?;

        let count = running.len();
        for instance in running {
            info!(workflow_id = %instance.id, "Recovering running workflow");
            if let Err(e) = self.resume_workflow(instance.id).await {
                error!(
                    workflow_id = %instance.id,
                    error = %e,
                    "Failed to recover workflow"
                );
            }
        }

        Ok(count)
    }

    // ========================================================================
    // Core State Machine
    // ========================================================================

    /// Advance the workflow: compute ready phases, start them, check completion.
    async fn advance_workflow(
        &self,
        instance: &mut WorkflowInstance,
        definition: &WorkflowDefinition,
    ) -> DomainResult<()> {
        loop {
            let completed = instance.completed_phases();
            let ready_phase_ids = definition.ready_phases(&completed);

            // Filter to phases not already running or terminal
            let actionable: Vec<Uuid> = ready_phase_ids
                .into_iter()
                .filter(|id| {
                    instance
                        .phase_instances
                        .get(id)
                        .map(|pi| pi.status == PhaseStatus::Pending || pi.status == PhaseStatus::Ready)
                        .unwrap_or(false)
                })
                .collect();

            if actionable.is_empty() {
                // Check if workflow is complete
                if instance.all_phases_terminal() {
                    self.complete_workflow(instance, definition).await?;
                }
                // No more phases to start right now -- return and wait for events
                return Ok(());
            }

            // Start all actionable phases
            for phase_id in actionable {
                if let Err(e) = self.start_phase(instance, definition, phase_id).await {
                    error!(
                        workflow_id = %instance.id,
                        phase_id = %phase_id,
                        error = %e,
                        "Failed to start phase"
                    );
                    // Mark the phase as failed
                    if let Some(pi) = instance.phase_instances.get_mut(&phase_id) {
                        pi.status = PhaseStatus::Failed;
                        pi.error = Some(e.to_string());
                        pi.completed_at = Some(chrono::Utc::now());
                    }

                    if definition.config.fail_fast {
                        self.fail_workflow(instance, definition, &e.to_string())
                            .await?;
                        return Ok(());
                    }
                }
            }

            // After starting phases, the workflow must wait for them to complete
            // (event-driven via on_phase_tasks_completed). Don't loop.
            return Ok(());
        }
    }

    /// Start a single phase: create tasks, build TaskDag, dispatch to DagExecutor.
    async fn start_phase(
        &self,
        instance: &mut WorkflowInstance,
        definition: &WorkflowDefinition,
        phase_id: Uuid,
    ) -> DomainResult<()> {
        let phase_def = definition
            .phases
            .iter()
            .find(|p| p.id == phase_id)
            .ok_or_else(|| {
                DomainError::ValidationFailed(format!("Phase {} not found in definition", phase_id))
            })?
            .clone();

        // Update phase instance status
        {
            let pi = instance
                .phase_instances
                .get_mut(&phase_id)
                .ok_or_else(|| {
                    DomainError::ValidationFailed(format!(
                        "Phase instance {} not found",
                        phase_id
                    ))
                })?;
            pi.status = PhaseStatus::Running;
            pi.started_at = Some(chrono::Utc::now());
        }

        info!(
            workflow_id = %instance.id,
            phase_id = %phase_id,
            phase_name = %phase_def.name,
            phase_type = ?phase_def.phase_type,
            "Starting phase"
        );

        match &phase_def.phase_type {
            PhaseType::Execute | PhaseType::Iterative { .. } => {
                self.start_execute_phase(instance, &phase_def).await?;
            }
            PhaseType::Verify => {
                self.run_verify_phase(instance, &phase_def).await?;
            }
            PhaseType::Decompose | PhaseType::Decision => {
                let pi = instance.phase_instances.get_mut(&phase_id).unwrap();
                pi.status = PhaseStatus::AwaitingDecision;
                debug!(phase_id = %phase_id, "Phase awaiting overmind decision");
            }
            PhaseType::FanOut | PhaseType::Aggregate | PhaseType::SubWorkflow => {
                let pi = instance.phase_instances.get_mut(&phase_id).unwrap();
                pi.status = PhaseStatus::Completed;
                pi.completed_at = Some(chrono::Utc::now());
                debug!(phase_id = %phase_id, "Phase type not yet implemented, skipping");
            }
        }

        // Read task_count after phase has been started
        let task_count = instance
            .phase_instances
            .get(&phase_id)
            .map(|pi| pi.task_ids.len())
            .unwrap_or(0);

        // Emit phase started event
        self.event_bus
            .publish(workflow_event(
                EventSeverity::Info,
                Some(instance.goal_id),
                EventPayload::PhaseStarted {
                    workflow_instance_id: instance.id,
                    phase_id,
                    phase_name: phase_def.name.clone(),
                    task_count,
                },
            ))
            .await;

        Ok(())
    }

    /// Start an Execute phase: create tasks, build DAG, run with DagExecutor.
    async fn start_execute_phase(
        &self,
        instance: &mut WorkflowInstance,
        phase_def: &PhaseDefinition,
    ) -> DomainResult<()> {
        let phase_id = phase_def.id;

        if phase_def.task_definitions.is_empty() {
            // No tasks -- mark as completed
            let pi = instance.phase_instances.get_mut(&phase_id).unwrap();
            pi.status = PhaseStatus::Completed;
            pi.completed_at = Some(chrono::Utc::now());
            return Ok(());
        }

        // Create Task domain objects from TaskDefinitions
        let mut tasks = Vec::new();
        let mut title_to_id = std::collections::HashMap::new();

        for ptd in &phase_def.task_definitions {
            let td = &ptd.task_def;
            let mut task = Task::with_title(&td.title, &td.description);
            task.priority = td.priority.clone();
            if let Some(ref agent_type) = td.agent_type {
                task = task.with_agent(agent_type);
            }
            task = task.with_source(TaskSource::System);
            task.parent_id = Some(instance.goal_id);

            title_to_id.insert(td.title.clone(), task.id);
            tasks.push((task, td.depends_on.clone()));
        }

        // Resolve title-based dependencies to task IDs
        for (task, dep_titles) in &mut tasks {
            for dep_title in dep_titles.iter() {
                if let Some(&dep_id) = title_to_id.get(dep_title) {
                    task.depends_on.push(dep_id);
                }
            }
        }

        // Persist tasks and collect IDs
        let mut task_ids = Vec::new();
        for (task, _) in &tasks {
            self.task_repo.create(task).await?;
            task_ids.push(task.id);

            // Add dependencies
            for dep_id in &task.depends_on {
                self.task_repo.add_dependency(task.id, *dep_id).await?;
            }
        }

        // Store task IDs on the phase instance
        {
            let pi = instance.phase_instances.get_mut(&phase_id).unwrap();
            pi.task_ids = task_ids.clone();
        }

        // Mark tasks with no dependencies as Ready
        for (task, _) in &tasks {
            if task.depends_on.is_empty() {
                let mut t = task.clone();
                t.status = TaskStatus::Ready;
                self.task_repo.update(&t).await?;
            }
        }

        // Build TaskDag and execute
        let all_tasks: Vec<Task> = {
            let mut result = Vec::new();
            for id in &task_ids {
                if let Some(t) = self.task_repo.get(*id).await? {
                    result.push(t);
                }
            }
            result
        };

        let dag = TaskDag::from_tasks(all_tasks);

        // Spawn DagExecutor in background
        let task_repo = self.task_repo.clone();
        let agent_repo = self.agent_repo.clone();
        let goal_repo = self.goal_repo.clone();
        let substrate = self.substrate.clone();
        let event_bus = self.event_bus.clone();
        let executor_config = self.config.executor_config.clone();
        let workflow_instance_id = instance.id;
        let phase_id = phase_def.id;
        let phase_name = phase_def.name.clone();
        let goal_id = instance.goal_id;

        tokio::spawn(async move {
            let executor = DagExecutor::new(
                task_repo,
                agent_repo,
                substrate,
                executor_config,
            )
            .with_goal_repo(goal_repo)
            .with_event_bus(event_bus.clone());

            let (event_tx, mut event_rx) = mpsc::channel::<ExecutionEvent>(256);

            // Forward execution events to the event bus
            let bus_clone = event_bus.clone();
            let fwd_handle = tokio::spawn(async move {
                while let Some(exec_event) = event_rx.recv().await {
                    let unified: crate::services::event_bus::UnifiedEvent = exec_event.into();
                    bus_clone.publish(unified).await;
                }
            });

            let result = executor.execute_with_events(&dag, event_tx).await;
            fwd_handle.abort();

            // Determine phase outcome based on execution result
            match result {
                Ok(results) => {
                    let status = if results.failed_tasks == 0 {
                        "completed"
                    } else if results.completed_tasks > 0 {
                        "partial_success"
                    } else {
                        "failed"
                    };

                    // Emit phase completion/failure event
                    if results.failed_tasks == 0 {
                        event_bus
                            .publish(workflow_event(
                                EventSeverity::Info,
                                Some(goal_id),
                                EventPayload::PhaseCompleted {
                                    workflow_instance_id,
                                    phase_id,
                                    phase_name: phase_name.clone(),
                                },
                            ))
                            .await;
                    } else {
                        event_bus
                            .publish(workflow_event(
                                EventSeverity::Error,
                                Some(goal_id),
                                EventPayload::PhaseFailed {
                                    workflow_instance_id,
                                    phase_id,
                                    phase_name: phase_name.clone(),
                                    error: format!(
                                        "{} tasks failed out of {}",
                                        results.failed_tasks, results.total_tasks
                                    ),
                                },
                            ))
                            .await;
                    }

                    debug!(
                        workflow_id = %workflow_instance_id,
                        phase_id = %phase_id,
                        status = status,
                        completed = results.completed_tasks,
                        failed = results.failed_tasks,
                        "Phase execution finished"
                    );
                }
                Err(e) => {
                    event_bus
                        .publish(workflow_event(
                            EventSeverity::Error,
                            Some(goal_id),
                            EventPayload::PhaseFailed {
                                workflow_instance_id,
                                phase_id,
                                phase_name,
                                error: e.to_string(),
                            },
                        ))
                        .await;
                }
            }
        });

        Ok(())
    }

    /// Run a verification phase inline.
    async fn run_verify_phase(
        &self,
        instance: &mut WorkflowInstance,
        phase_def: &PhaseDefinition,
    ) -> DomainResult<()> {
        let phase_id = phase_def.id;

        // Use IntentVerifier when configured, otherwise auto-pass (backwards compatible).
        let passed = if let Some(ref verifier) = self.intent_verifier {
            // Clone task_ids from an immutable borrow before the mutable borrow below
            let task_ids: Vec<Uuid> = instance
                .phase_instances
                .get(&phase_id)
                .map(|pi| pi.task_ids.clone())
                .unwrap_or_default();

            let mut tasks = Vec::new();
            for task_id in &task_ids {
                if let Ok(Some(task)) = self.task_repo.get(*task_id).await {
                    if task.status == TaskStatus::Complete {
                        tasks.push(task);
                    }
                }
            }

            match verifier.verify_task_batch(&tasks, &phase_def.name, 1).await {
                Ok(result) => result.satisfaction == IntentSatisfaction::Satisfied,
                Err(e) => {
                    warn!(
                        phase_id = %phase_id,
                        error = %e,
                        "Intent verification failed, defaulting to pass"
                    );
                    true // Graceful degradation: pass on verification error
                }
            }
        } else {
            // No verifier configured — auto-pass (backwards compatible)
            true
        };

        {
            let pi = instance.phase_instances.get_mut(&phase_id).ok_or_else(|| {
                DomainError::ValidationFailed(format!("Phase instance {} not found", phase_id))
            })?;
            pi.verification_result = Some(passed);

            if passed {
                pi.status = PhaseStatus::Completed;
                pi.completed_at = Some(chrono::Utc::now());
            } else if phase_def
                .verification
                .as_ref()
                .map(|v| v.is_blocking)
                .unwrap_or(false)
            {
                pi.status = PhaseStatus::Failed;
                pi.error = Some("Verification failed".to_string());
                pi.completed_at = Some(chrono::Utc::now());
            } else {
                // Non-blocking verification failure -- continue
                pi.status = PhaseStatus::Completed;
                pi.completed_at = Some(chrono::Utc::now());
                warn!(
                    phase_id = %phase_id,
                    "Non-blocking verification failed, continuing"
                );
            }
        }

        self.event_bus
            .publish(workflow_event(
                EventSeverity::Info,
                Some(instance.goal_id),
                EventPayload::PhaseVerificationCompleted {
                    workflow_instance_id: instance.id,
                    phase_id,
                    passed,
                },
            ))
            .await;

        Ok(())
    }

    // ========================================================================
    // Event Handlers (called from SwarmOrchestrator event system)
    // ========================================================================

    /// Handle notification that all tasks in a phase have completed.
    ///
    /// Called by the event system when a TaskCompleted/TaskFailed event
    /// is received for a task that belongs to a workflow phase.
    pub async fn on_phase_tasks_completed(
        &self,
        workflow_instance_id: Uuid,
        phase_id: Uuid,
    ) -> DomainResult<()> {
        let mut instance = self
            .workflow_repo
            .get_instance(workflow_instance_id)
            .await?
            .ok_or(DomainError::WorkflowNotFound(workflow_instance_id))?;

        let definition = self
            .workflow_repo
            .get_definition(instance.workflow_id)
            .await?
            .ok_or(DomainError::WorkflowNotFound(instance.workflow_id))?;

        // Check all tasks in this phase
        let pi = instance
            .phase_instances
            .get(&phase_id)
            .ok_or_else(|| {
                DomainError::ValidationFailed(format!(
                    "Phase instance {} not found",
                    phase_id
                ))
            })?
            .clone();

        if pi.status != PhaseStatus::Running {
            return Ok(()); // Phase not running, ignore
        }

        // Check if all tasks are terminal
        let mut all_done = true;
        let mut any_failed = false;
        for task_id in &pi.task_ids {
            if let Some(task) = self.task_repo.get(*task_id).await? {
                if !task.status.is_terminal() {
                    all_done = false;
                    break;
                }
                if task.status == TaskStatus::Failed {
                    any_failed = true;
                }
            }
        }

        if !all_done {
            return Ok(()); // Still waiting for tasks
        }

        // All tasks done -- update phase status
        if any_failed {
            self.on_phase_failed(&mut instance, &definition, phase_id, "One or more tasks failed")
                .await?;
        } else {
            // Check if phase has verification
            let phase_def = definition.phases.iter().find(|p| p.id == phase_id).cloned();
            if let Some(pd) = phase_def {
                if pd.verification.is_some() {
                    instance.phase_instances.get_mut(&phase_id).unwrap().status = PhaseStatus::Verifying;
                    self.run_verify_phase_by_id(&mut instance, &definition, phase_id)
                        .await?;
                } else {
                    let pi_mut = instance.phase_instances.get_mut(&phase_id).unwrap();
                    pi_mut.status = PhaseStatus::Completed;
                    pi_mut.completed_at = Some(chrono::Utc::now());

                    self.event_bus
                        .publish(workflow_event(
                            EventSeverity::Info,
                            Some(instance.goal_id),
                            EventPayload::PhaseCompleted {
                                workflow_instance_id: instance.id,
                                phase_id,
                                phase_name: pd.name.clone(),
                            },
                        ))
                        .await;
                }
            }
        }

        // Try to advance the workflow
        self.advance_workflow(&mut instance, &definition).await?;

        // Persist updated state
        self.workflow_repo.update_instance(&instance).await?;
        self.update_active_instance(&instance).await;

        Ok(())
    }

    /// Handle a phase verification result.
    pub async fn on_phase_verified(
        &self,
        workflow_instance_id: Uuid,
        phase_id: Uuid,
        passed: bool,
    ) -> DomainResult<()> {
        let mut instance = self
            .workflow_repo
            .get_instance(workflow_instance_id)
            .await?
            .ok_or(DomainError::WorkflowNotFound(workflow_instance_id))?;

        let definition = self
            .workflow_repo
            .get_definition(instance.workflow_id)
            .await?
            .ok_or(DomainError::WorkflowNotFound(instance.workflow_id))?;

        let phase_def = definition.phases.iter().find(|p| p.id == phase_id);

        if let Some(pi) = instance.phase_instances.get_mut(&phase_id) {
            pi.verification_result = Some(passed);

            if passed {
                pi.status = PhaseStatus::Completed;
                pi.completed_at = Some(chrono::Utc::now());
            } else {
                // Check if iterative and can retry
                if let Some(pd) = phase_def {
                    if let PhaseType::Iterative { max_iterations } = &pd.phase_type {
                        if pi.iteration_count < *max_iterations {
                            pi.iteration_count += 1;
                            pi.status = PhaseStatus::Ready; // Will be re-started on next advance
                            debug!(
                                phase_id = %phase_id,
                                iteration = pi.iteration_count,
                                "Iterative phase looping back"
                            );
                        } else {
                            pi.status = PhaseStatus::Failed;
                            pi.error = Some("Max iterations reached".to_string());
                            pi.completed_at = Some(chrono::Utc::now());
                        }
                    } else {
                        let is_blocking = pd
                            .verification
                            .as_ref()
                            .map(|v| v.is_blocking)
                            .unwrap_or(true);
                        if is_blocking {
                            pi.status = PhaseStatus::Failed;
                            pi.error = Some("Blocking verification failed".to_string());
                            pi.completed_at = Some(chrono::Utc::now());
                        } else {
                            pi.status = PhaseStatus::Completed;
                            pi.completed_at = Some(chrono::Utc::now());
                        }
                    }
                }
            }
        }

        self.advance_workflow(&mut instance, &definition).await?;
        self.workflow_repo.update_instance(&instance).await?;
        self.update_active_instance(&instance).await;

        Ok(())
    }

    /// Handle a phase failure with optional recovery.
    async fn on_phase_failed(
        &self,
        instance: &mut WorkflowInstance,
        definition: &WorkflowDefinition,
        phase_id: Uuid,
        error: &str,
    ) -> DomainResult<()> {
        let pi = instance
            .phase_instances
            .get_mut(&phase_id)
            .ok_or_else(|| {
                DomainError::ValidationFailed(format!(
                    "Phase instance {} not found",
                    phase_id
                ))
            })?;

        let max_retries = definition.config.max_phase_retries;
        let phase_name = definition
            .phases
            .iter()
            .find(|p| p.id == phase_id)
            .map(|p| p.name.clone())
            .unwrap_or_else(|| "unknown".to_string());

        if pi.retry_count < max_retries {
            // Retry the phase
            pi.retry_count += 1;
            pi.status = PhaseStatus::Ready;
            pi.error = None;
            pi.task_ids.clear(); // Will be re-created

            self.event_bus
                .publish(workflow_event(
                    EventSeverity::Warning,
                    Some(instance.goal_id),
                    EventPayload::PhaseRecoveryStarted {
                        workflow_instance_id: instance.id,
                        phase_id,
                        retry_count: pi.retry_count,
                    },
                ))
                .await;

            info!(
                workflow_id = %instance.id,
                phase_id = %phase_id,
                retry = pi.retry_count,
                max_retries = max_retries,
                "Retrying failed phase"
            );
        } else {
            // Retries exhausted
            pi.status = PhaseStatus::Failed;
            pi.error = Some(error.to_string());
            pi.completed_at = Some(chrono::Utc::now());

            // Capture values from pi before dropping the mutable borrow
            let phase_retry_count = pi.retry_count;
            let phase_task_ids = pi.task_ids.clone();

            self.event_bus
                .publish(workflow_event(
                    EventSeverity::Error,
                    Some(instance.goal_id),
                    EventPayload::PhaseFailed {
                        workflow_instance_id: instance.id,
                        phase_id,
                        phase_name,
                        error: error.to_string(),
                    },
                ))
                .await;

            // Try overmind recovery if enabled
            if self.config.enable_overmind_recovery {
                if let Some(ref overmind) = self.overmind {
                    info!(
                        workflow_id = %instance.id,
                        phase_id = %phase_id,
                        "Invoking overmind for phase recovery"
                    );

                    // Gather context for recovery request
                    let goal = self.goal_repo.get(instance.goal_id).await.ok().flatten();

                    // Find a representative failed task from the phase
                    let mut failed_task: Option<Task> = None;
                    for tid in &phase_task_ids {
                        if let Ok(Some(t)) = self.task_repo.get(*tid).await {
                            if t.status == TaskStatus::Failed {
                                failed_task = Some(t);
                                break;
                            }
                        }
                    }

                    let representative = failed_task.unwrap_or_else(|| {
                        Task::with_title(
                            format!("Phase '{}' recovery", definition.phases.iter()
                                .find(|p| p.id == phase_id)
                                .map(|p| p.name.as_str())
                                .unwrap_or("unknown")),
                            error.to_string(),
                        )
                    });

                    let request = StuckStateRecoveryRequest {
                        task_id: representative.id,
                        task_title: representative.title.clone(),
                        task_description: representative.description.clone(),
                        goal_context: GoalContext {
                            goal_id: instance.goal_id,
                            goal_name: goal.as_ref().map(|g| g.name.clone()).unwrap_or_default(),
                            goal_description: goal.as_ref().map(|g| g.description.clone()).unwrap_or_default(),
                            other_tasks_status: format!("{} tasks in phase", phase_task_ids.len()),
                        },
                        failure_history: vec![FailureRecord {
                            attempt: phase_retry_count,
                            timestamp: chrono::Utc::now(),
                            error: error.to_string(),
                            agent_type: representative.agent_type.clone().unwrap_or_default(),
                            turns_used: 0,
                        }],
                        previous_recovery_attempts: vec![],
                        available_approaches: vec![],
                    };

                    match overmind.recover_from_stuck(request).await {
                        Ok(decision) => {
                            info!(
                                workflow_id = %instance.id,
                                phase_id = %phase_id,
                                root_cause = ?decision.root_cause.category,
                                "Overmind recovery decision received"
                            );
                            // Recovery decision is logged but not automatically applied
                            // to workflow state — the orchestrator respects fail_fast below
                        }
                        Err(e) => {
                            warn!(
                                workflow_id = %instance.id,
                                phase_id = %phase_id,
                                error = %e,
                                "Overmind recovery failed"
                            );
                        }
                    }
                }
            }

            if definition.config.fail_fast {
                self.fail_workflow(instance, definition, error).await?;
            }
        }

        Ok(())
    }

    // ========================================================================
    // Workflow Completion
    // ========================================================================

    /// Mark a workflow as completed.
    async fn complete_workflow(
        &self,
        instance: &mut WorkflowInstance,
        definition: &WorkflowDefinition,
    ) -> DomainResult<()> {
        let now = chrono::Utc::now();

        if instance.any_phase_failed() {
            instance.status = WorkflowStatus::Failed;
        } else {
            instance.status = WorkflowStatus::Completed;
        }
        instance.completed_at = Some(now);
        instance.updated_at = now;

        self.event_bus
            .publish(workflow_event(
                EventSeverity::Info,
                Some(instance.goal_id),
                EventPayload::WorkflowCompleted {
                    workflow_instance_id: instance.id,
                    goal_id: instance.goal_id,
                    status: instance.status.to_string(),
                    tokens_consumed: instance.tokens_consumed,
                },
            ))
            .await;

        info!(
            workflow_id = %instance.id,
            status = %instance.status,
            "Workflow completed: {}",
            definition.name,
        );

        // Remove from active list
        {
            let mut active = self.active_workflows.write().await;
            active.retain(|(_, inst)| inst.id != instance.id);
        }

        Ok(())
    }

    /// Mark a workflow as failed.
    async fn fail_workflow(
        &self,
        instance: &mut WorkflowInstance,
        definition: &WorkflowDefinition,
        error: &str,
    ) -> DomainResult<()> {
        let now = chrono::Utc::now();
        instance.status = WorkflowStatus::Failed;
        instance.completed_at = Some(now);
        instance.updated_at = now;

        self.event_bus
            .publish(workflow_event(
                EventSeverity::Error,
                Some(instance.goal_id),
                EventPayload::WorkflowCompleted {
                    workflow_instance_id: instance.id,
                    goal_id: instance.goal_id,
                    status: "failed".to_string(),
                    tokens_consumed: instance.tokens_consumed,
                },
            ))
            .await;

        error!(
            workflow_id = %instance.id,
            error = error,
            "Workflow failed: {}",
            definition.name,
        );

        // Remove from active list
        {
            let mut active = self.active_workflows.write().await;
            active.retain(|(_, inst)| inst.id != instance.id);
        }

        Ok(())
    }

    // ========================================================================
    // Helpers
    // ========================================================================

    /// Run verification for a phase by ID (delegates to run_verify_phase).
    async fn run_verify_phase_by_id(
        &self,
        instance: &mut WorkflowInstance,
        definition: &WorkflowDefinition,
        phase_id: Uuid,
    ) -> DomainResult<()> {
        let phase_def = definition
            .phases
            .iter()
            .find(|p| p.id == phase_id)
            .ok_or_else(|| {
                DomainError::ValidationFailed(format!("Phase {} not found", phase_id))
            })?
            .clone();

        self.run_verify_phase(instance, &phase_def).await
    }

    /// Update the active workflow instance in the in-memory list.
    async fn update_active_instance(&self, instance: &WorkflowInstance) {
        let mut active = self.active_workflows.write().await;
        if let Some(entry) = active.iter_mut().find(|(_, inst)| inst.id == instance.id) {
            entry.1 = instance.clone();
        }
    }

    /// Look up which workflow instance and phase a task belongs to.
    pub async fn find_workflow_for_task(
        &self,
        task_id: Uuid,
    ) -> Option<(Uuid, Uuid)> {
        let active = self.active_workflows.read().await;
        for (_, instance) in active.iter() {
            if let Some(phase_id) = instance.phase_for_task(task_id) {
                return Some((instance.id, phase_id));
            }
        }
        None
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::models::workflow::*;

    #[test]
    fn test_phase_orchestrator_config_defaults() {
        let config = PhaseOrchestratorConfig::default();
        assert_eq!(config.max_phase_retries, 2);
        assert!(config.enable_overmind_recovery);
    }
}
