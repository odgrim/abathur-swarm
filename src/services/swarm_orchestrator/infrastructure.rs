//! Infrastructure subsystem for the swarm orchestrator.
//!
//! Manages cold start, memory decay daemon, MCP server lifecycle,
//! worktree creation, task verification, and statistics tracking.

use std::sync::Arc;
use std::sync::atomic::Ordering;
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::domain::errors::DomainResult;
use crate::domain::models::{GoalStatus, TaskStatus};
use crate::domain::ports::{AgentRepository, GoalRepository, MemoryRepository, TaskRepository, WorktreeRepository};
use crate::services::{
    AuditAction, AuditActor, AuditCategory, AuditEntry, AuditLevel,
    ColdStartConfig, ColdStartReport, ColdStartService,
    DecayDaemonConfig, IntegrationVerifierService, MemoryDecayDaemon, MemoryService,
    VerificationResult, VerifierConfig, WorktreeConfig, WorktreeService,
};

use super::types::{OrchestratorStatus, SwarmEvent, SwarmStats};
use super::SwarmOrchestrator;

impl<G, T, W, A, M> SwarmOrchestrator<G, T, W, A, M>
where
    G: GoalRepository + 'static,
    T: TaskRepository + 'static,
    W: WorktreeRepository + 'static,
    A: AgentRepository + 'static,
    M: MemoryRepository + 'static,
{
    /// Check if the MCP infrastructure is ready.
    ///
    /// With MCP stdio servers, agents get their own server process via --mcp,
    /// so there are no external servers to health-check. Instead, verify that
    /// the abathur binary and database file exist so stdio servers can launch.
    /// Falls back to HTTP health checks for any configured HTTP servers (A2A gateway).
    pub async fn check_mcp_readiness(&self) -> bool {
        // Check abathur binary exists (needed by MCP stdio servers)
        let exe_ok = std::env::current_exe()
            .map(|p| p.exists())
            .unwrap_or(false);
        if !exe_ok {
            tracing::warn!("Abathur binary not found — MCP stdio servers cannot launch");
            return false;
        }

        // Check database file exists — use absolute path consistent with agent MCP configs
        let db_path = std::env::current_dir()
            .unwrap_or_else(|_| self.config.repo_path.clone())
            .join(".abathur")
            .join("abathur.db");
        if !db_path.exists() {
            tracing::warn!("Database not found at {:?} — MCP stdio servers cannot launch", db_path);
            return false;
        }

        // Health-check any HTTP servers that are still configured (e.g., A2A gateway)
        if let Some(ref a2a_url) = self.config.mcp_servers.a2a_gateway {
            let client = reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(2))
                .build()
                .unwrap_or_default();

            let health_url = format!("{}/health", a2a_url.trim_end_matches('/'));
            match client.get(&health_url).send().await {
                Ok(resp) if resp.status().is_success() => {}
                Ok(resp) => {
                    tracing::warn!("A2A gateway at {} returned status {}", a2a_url, resp.status());
                    return false;
                }
                Err(e) => {
                    tracing::warn!("A2A gateway at {} unreachable: {}", a2a_url, e);
                    return false;
                }
            }
        }

        true
    }

    /// Wait for all configured MCP servers to become healthy.
    ///
    /// Retries up to 30 times with 1-second intervals (30s total).
    /// Used at startup to ensure infrastructure is ready before processing tasks.
    pub async fn await_mcp_readiness(&self) -> DomainResult<()> {
        let max_attempts = 30u32;

        for attempt in 1..=max_attempts {
            if self.check_mcp_readiness().await {
                self.audit_log.info(
                    AuditCategory::System,
                    AuditAction::SwarmStarted,
                    format!("All MCP servers healthy (attempt {}/{})", attempt, max_attempts),
                ).await;
                return Ok(());
            }

            tracing::info!("Waiting for MCP servers... (attempt {}/{})", attempt, max_attempts);
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        }

        self.audit_log.log(
            AuditEntry::new(
                AuditLevel::Error,
                AuditCategory::System,
                AuditAction::SwarmStarted,
                AuditActor::System,
                format!("MCP servers not ready after {} attempts", max_attempts),
            ),
        ).await;

        Err(crate::domain::errors::DomainError::ExecutionFailed(
            format!("MCP servers not ready after {} attempts", max_attempts),
        ))
    }

    /// Verify a completed task using the IntegrationVerifier.
    ///
    /// Returns the verification result if verification is enabled and passes.
    /// Uses lightweight config (no code checks) - code quality is verified at merge time.
    pub async fn verify_task(&self, task_id: Uuid) -> DomainResult<Option<VerificationResult>> {
        if !self.config.verify_on_completion {
            return Ok(None);
        }

        let verifier = IntegrationVerifierService::new(
            self.task_repo.clone(),
            self.goal_repo.clone(),
            self.worktree_repo.clone(),
            VerifierConfig {
                run_tests: false,
                run_lint: false,
                check_format: false,
                ..VerifierConfig::default()
            },
        );

        let result = verifier.verify_task(task_id).await?;

        // Compute check statistics
        let checks_total = result.checks.len();
        let checks_passed = result.checks.iter().filter(|c| c.passed).count();

        // Log verification result
        if result.passed {
            self.audit_log.info(
                AuditCategory::Task,
                AuditAction::TaskCompleted,
                format!(
                    "Task {} passed verification: {}/{} checks",
                    task_id, checks_passed, checks_total
                ),
            ).await;
        } else {
            self.audit_log.log(
                AuditEntry::new(
                    AuditLevel::Warning,
                    AuditCategory::Task,
                    AuditAction::TaskFailed,
                    AuditActor::System,
                    format!(
                        "Task {} failed verification: {}",
                        task_id, result.failures_summary.clone().unwrap_or_default()
                    ),
                )
                .with_entity(task_id, "task"),
            ).await;
        }

        Ok(Some(result))
    }

    /// Run cold start analysis if memory is empty.
    pub async fn cold_start(&self) -> DomainResult<Option<ColdStartReport>>
    where
        M: MemoryRepository + Send + Sync + 'static,
    {
        let Some(ref memory_repo) = self.memory_repo else {
            return Ok(None);
        };

        // Create memory service
        let memory_service = MemoryService::new(memory_repo.clone());

        // Check if we have any existing memories
        let stats = memory_service.get_stats().await?;
        let total_memories = stats.total();
        if total_memories > 0 {
            self.audit_log.info(
                AuditCategory::System,
                AuditAction::SwarmStarted,
                format!("Skipping cold start - {} existing memories found", total_memories),
            ).await;
            return Ok(None);
        }

        // Run cold start
        self.audit_log.info(
            AuditCategory::System,
            AuditAction::SwarmStarted,
            "Running cold start analysis...",
        ).await;

        let cold_start_config = ColdStartConfig {
            project_root: self.config.repo_path.clone(),
            use_llm_analysis: self.overmind.is_some(),
            ..Default::default()
        };
        let cold_start_service = ColdStartService::new(
            memory_service,
            cold_start_config,
        );
        let cold_start_service = if self.overmind.is_some() {
            cold_start_service.with_substrate(self.substrate.clone())
        } else {
            cold_start_service
        };

        let report = cold_start_service.gather_context().await?;

        self.audit_log.info(
            AuditCategory::Memory,
            AuditAction::MemoryStored,
            format!(
                "Cold start complete: {} memories created, project type: {}",
                report.memories_created, report.project_type
            ),
        ).await;

        Ok(Some(report))
    }

    /// Store MCP server shutdown handle for external management.
    pub async fn set_mcp_shutdown_handle(&self, tx: tokio::sync::broadcast::Sender<()>) {
        let mut handle = self.mcp_shutdown_tx.write().await;
        *handle = Some(tx);
    }

    /// Stop embedded MCP servers if a shutdown handle was set.
    pub async fn stop_embedded_mcp_servers(&self) {
        let handle = self.mcp_shutdown_tx.read().await;
        if let Some(ref tx) = *handle {
            let _ = tx.send(());
        }
    }

    /// Start the memory decay daemon.
    pub async fn start_decay_daemon(&self) -> DomainResult<()>
    where
        M: MemoryRepository + Send + Sync + 'static,
    {
        let Some(ref memory_repo) = self.memory_repo else {
            return Ok(());
        };

        let memory_service = Arc::new(MemoryService::new(memory_repo.clone()));
        let daemon = MemoryDecayDaemon::new(memory_service, DecayDaemonConfig::default());

        // Get the handle before running
        let handle = daemon.handle();

        // Store the handle
        {
            let mut daemon_handle = self.decay_daemon_handle.write().await;
            *daemon_handle = Some(handle);
        }

        // Run daemon and log events in background
        let audit_log = self.audit_log.clone();
        tokio::spawn(async move {
            let mut event_rx = daemon.run().await;
            while let Some(event) = event_rx.recv().await {
                match event {
                    crate::services::DecayDaemonEvent::Started => {
                        audit_log.info(
                            AuditCategory::System,
                            AuditAction::SwarmStarted,
                            "Memory decay daemon started",
                        ).await;
                    }
                    crate::services::DecayDaemonEvent::MaintenanceCompleted { run_number, report, .. } => {
                        audit_log.info(
                            AuditCategory::Memory,
                            AuditAction::MemoryPruned,
                            format!(
                                "Memory maintenance #{}: {} expired, {} decayed, {} promoted",
                                run_number, report.expired_pruned, report.decayed_pruned, report.promoted
                            ),
                        ).await;
                    }
                    crate::services::DecayDaemonEvent::Stopped { reason } => {
                        audit_log.info(
                            AuditCategory::System,
                            AuditAction::SwarmStopped,
                            format!("Memory decay daemon stopped: {:?}", reason),
                        ).await;
                    }
                    _ => {}
                }
            }
        });

        Ok(())
    }

    /// Stop the memory decay daemon.
    pub async fn stop_decay_daemon(&self) {
        let daemon_handle = self.decay_daemon_handle.read().await;
        if let Some(ref handle) = *daemon_handle {
            handle.stop();
        }
    }

    /// Create a worktree for task execution.
    pub(super) async fn create_worktree_for_task(
        &self,
        task_id: Uuid,
        event_tx: &mpsc::Sender<SwarmEvent>,
    ) -> DomainResult<String> {
        let worktree_config = WorktreeConfig {
            base_path: self.config.worktree_base_path.clone(),
            repo_path: self.config.repo_path.clone(),
            default_base_ref: self.config.default_base_ref.clone(),
            auto_cleanup: true,
        };

        let worktree_service = WorktreeService::new(
            self.worktree_repo.clone(),
            worktree_config,
        );

        let worktree = worktree_service.create_worktree(task_id, None).await?;

        let _ = event_tx.send(SwarmEvent::WorktreeCreated {
            task_id,
            path: worktree.path.clone(),
        }).await;

        Ok(worktree.path)
    }

    /// Update statistics.
    pub(super) async fn update_stats(&self, event_tx: &mpsc::Sender<SwarmEvent>) -> DomainResult<()> {
        let task_counts = self.task_repo.count_by_status().await?;
        let active_worktrees = self.worktree_repo.list_active().await?.len();

        let stats = SwarmStats {
            active_goals: self.goal_repo.list(crate::domain::ports::GoalFilter {
                status: Some(GoalStatus::Active),
                ..Default::default()
            }).await?.len(),
            pending_tasks: *task_counts.get(&TaskStatus::Pending).unwrap_or(&0) as usize,
            ready_tasks: *task_counts.get(&TaskStatus::Ready).unwrap_or(&0) as usize,
            running_tasks: *task_counts.get(&TaskStatus::Running).unwrap_or(&0) as usize,
            completed_tasks: *task_counts.get(&TaskStatus::Complete).unwrap_or(&0) as usize,
            failed_tasks: *task_counts.get(&TaskStatus::Failed).unwrap_or(&0) as usize,
            active_agents: self.config.max_agents - self.agent_semaphore.available_permits(),
            active_worktrees,
            total_tokens_used: self.total_tokens.load(Ordering::Relaxed),
        };

        {
            let mut s = self.stats.write().await;
            *s = stats.clone();
        }

        let _ = event_tx.send(SwarmEvent::StatusUpdate(stats)).await;
        Ok(())
    }

    /// Get current status.
    pub async fn status(&self) -> OrchestratorStatus {
        self.status.read().await.clone()
    }

    /// Get current stats.
    pub async fn stats(&self) -> SwarmStats {
        self.stats.read().await.clone()
    }

    /// Pause the orchestrator.
    pub async fn pause(&self) {
        let mut status = self.status.write().await;
        if *status == OrchestratorStatus::Running {
            *status = OrchestratorStatus::Paused;
        }
    }

    /// Resume the orchestrator.
    pub async fn resume(&self) {
        let mut status = self.status.write().await;
        if *status == OrchestratorStatus::Paused {
            *status = OrchestratorStatus::Running;
        }
    }

    /// Stop the orchestrator gracefully.
    pub async fn stop(&self) {
        let mut status = self.status.write().await;
        *status = OrchestratorStatus::ShuttingDown;
    }

    /// Get total tokens used.
    pub fn total_tokens(&self) -> u64 {
        self.total_tokens.load(Ordering::Relaxed)
    }

    /// Run startup reconciliation to fix inconsistent state after a crash or restart.
    ///
    /// Checks for:
    /// - Tasks stuck in `Running` status (stale agents) -> fail them
    /// - Tasks in `Ready` status with incomplete dependencies -> move back to `Pending`
    /// - Tasks in `Pending` status with all dependencies complete -> transition to `Ready`
    pub async fn run_startup_reconciliation(&self) -> DomainResult<u64> {
        let mut corrections: u64 = 0;

        // 1. Fail stale Running tasks (started_at older than threshold).
        //    On restart, any task that was Running has lost its agent.
        let running_tasks = self.task_repo.list(crate::domain::ports::TaskFilter {
            status: Some(TaskStatus::Running),
            ..Default::default()
        }).await?;

        for task in &running_tasks {
            tracing::info!(
                "Startup reconciliation: failing stale running task {} ('{}')",
                task.id, task.title
            );
            let mut task = task.clone();
            task.status = TaskStatus::Failed;
            if let Err(e) = self.task_repo.update(&task).await {
                tracing::warn!("Failed to reconcile task {}: {}", task.id, e);
            } else {
                corrections += 1;
            }
        }

        // 2. Check Ready tasks with incomplete dependencies -> move back to Pending
        let ready_tasks = self.task_repo.list(crate::domain::ports::TaskFilter {
            status: Some(TaskStatus::Ready),
            ..Default::default()
        }).await?;

        for task in &ready_tasks {
            if !task.depends_on.is_empty() {
                let mut all_deps_complete = true;
                for dep_id in &task.depends_on {
                    if let Ok(Some(dep)) = self.task_repo.get(*dep_id).await {
                        if dep.status != TaskStatus::Complete {
                            all_deps_complete = false;
                            break;
                        }
                    }
                }
                if !all_deps_complete {
                    tracing::info!(
                        "Startup reconciliation: moving task {} ('{}') back to Pending (incomplete deps)",
                        task.id, task.title
                    );
                    let mut task = task.clone();
                    task.status = TaskStatus::Pending;
                    if let Err(e) = self.task_repo.update(&task).await {
                        tracing::warn!("Failed to reconcile task {}: {}", task.id, e);
                    } else {
                        corrections += 1;
                    }
                }
            }
        }

        // 3. Check Pending tasks with all dependencies complete -> transition to Ready
        let pending_tasks = self.task_repo.list(crate::domain::ports::TaskFilter {
            status: Some(TaskStatus::Pending),
            ..Default::default()
        }).await?;

        for task in &pending_tasks {
            let should_promote = if task.depends_on.is_empty() {
                true
            } else {
                let mut all_complete = true;
                for dep_id in &task.depends_on {
                    if let Ok(Some(dep)) = self.task_repo.get(*dep_id).await {
                        if dep.status != TaskStatus::Complete {
                            all_complete = false;
                            break;
                        }
                    }
                }
                all_complete
            };

            if should_promote {
                tracing::info!(
                    "Startup reconciliation: promoting task {} ('{}') to Ready",
                    task.id, task.title
                );
                let mut task = task.clone();
                task.status = TaskStatus::Ready;
                if let Err(e) = self.task_repo.update(&task).await {
                    tracing::warn!("Failed to reconcile task {}: {}", task.id, e);
                } else {
                    corrections += 1;
                }
            }
        }

        Ok(corrections)
    }
}
