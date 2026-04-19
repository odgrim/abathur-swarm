//! Infrastructure subsystem for the swarm orchestrator.
//!
//! Manages cold start, memory decay daemon, MCP server lifecycle,
//! worktree creation, task verification, and statistics tracking.

use std::sync::Arc;
use std::sync::atomic::Ordering;
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::domain::errors::DomainResult;
use crate::domain::models::workflow_template::WorkspaceKind;
use crate::domain::models::{GoalStatus, TaskStatus};
use crate::domain::ports::{
    AgentRepository, GoalRepository, MemoryRepository, TaskRepository, WorktreeRepository,
};
use crate::services::{
    AuditAction, AuditActor, AuditCategory, AuditEntry, AuditLevel, ColdStartConfig,
    ColdStartReport, ColdStartService, DecayDaemonConfig, IntegrationVerifierService,
    MemoryDecayDaemon, MemoryMaintenanceService, MemoryService, VerificationResult,
    VerifierConfig, WorktreeConfig,
    WorktreeService,
    command_bus::{CommandEnvelope, CommandSource, DomainCommand, TaskCommand},
    supervise,
};

use super::SwarmOrchestrator;
use super::types::{OrchestratorStatus, SwarmEvent, SwarmStats};

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
        let exe_ok = std::env::current_exe().map(|p| p.exists()).unwrap_or(false);
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
            tracing::warn!(
                "Database not found at {:?} — MCP stdio servers cannot launch",
                db_path
            );
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
                    tracing::warn!(
                        "A2A gateway at {} returned status {}",
                        a2a_url,
                        resp.status()
                    );
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
                self.audit_log
                    .info(
                        AuditCategory::System,
                        AuditAction::SwarmStarted,
                        format!(
                            "All MCP servers healthy (attempt {}/{})",
                            attempt, max_attempts
                        ),
                    )
                    .await;
                return Ok(());
            }

            tracing::info!(
                "Waiting for MCP servers... (attempt {}/{})",
                attempt,
                max_attempts
            );
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        }

        self.audit_log
            .log(AuditEntry::new(
                AuditLevel::Error,
                AuditCategory::System,
                AuditAction::SwarmStarted,
                AuditActor::System,
                format!("MCP servers not ready after {} attempts", max_attempts),
            ))
            .await;

        Err(crate::domain::errors::DomainError::TimeoutError {
            operation: "mcp_readiness".to_string(),
            limit_secs: max_attempts as u64,
        })
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
            self.audit_log
                .info(
                    AuditCategory::Task,
                    AuditAction::TaskCompleted,
                    format!(
                        "Task {} passed verification: {}/{} checks",
                        task_id, checks_passed, checks_total
                    ),
                )
                .await;
        } else {
            self.audit_log
                .log(
                    AuditEntry::new(
                        AuditLevel::Warning,
                        AuditCategory::Task,
                        AuditAction::TaskFailed,
                        AuditActor::System,
                        format!(
                            "Task {} failed verification: {}",
                            task_id,
                            result.failures_summary.clone().unwrap_or_default()
                        ),
                    )
                    .with_entity(task_id, "task"),
                )
                .await;
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
            self.audit_log
                .info(
                    AuditCategory::System,
                    AuditAction::SwarmStarted,
                    format!(
                        "Skipping cold start - {} existing memories found",
                        total_memories
                    ),
                )
                .await;
            return Ok(None);
        }

        // Run cold start
        self.audit_log
            .info(
                AuditCategory::System,
                AuditAction::SwarmStarted,
                "Running cold start analysis...",
            )
            .await;

        let cold_start_config = ColdStartConfig {
            project_root: self.config.repo_path.clone(),
            use_llm_analysis: self.overmind.is_some(),
            ..Default::default()
        };
        let cold_start_service = ColdStartService::new(memory_service, cold_start_config)
            .with_event_bus(self.event_bus.clone());
        let cold_start_service = if self.overmind.is_some() {
            cold_start_service.with_substrate(self.substrate.clone())
        } else {
            cold_start_service
        };

        let report = cold_start_service.gather_context().await?;

        self.audit_log
            .info(
                AuditCategory::Memory,
                AuditAction::MemoryStored,
                format!(
                    "Cold start complete: {} memories created, project type: {}",
                    report.memories_created, report.project_type
                ),
            )
            .await;

        Ok(Some(report))
    }

    /// Check whether an 'origin' remote is configured for the repository.
    ///
    /// Logs a prominent warning when no remote is found so operators are
    /// aware the swarm is running in local-only mode.  Remote sync and push
    /// operations will individually short-circuit when they detect no remote,
    /// but this startup check makes the situation immediately visible.
    pub fn check_remote_at_startup(&self) {
        if !crate::services::worktree_service::check_remote_available(&self.config.repo_path) {
            tracing::warn!(
                path = %self.config.repo_path.display(),
                "No 'origin' remote configured for repository at {} — \
                 operating in local-only mode. Remote sync and push operations will be skipped.",
                self.config.repo_path.display()
            );
        }
    }

    /// Run startup codebase triage if no codebase profile exists in memory.
    ///
    /// Returns `Ok(true)` if triage ran and stored a profile, `Ok(false)` if
    /// a profile already existed (idempotency check), or an error on failure.
    ///
    /// This is a blocking startup step — the swarm should not accept user tasks
    /// until triage completes.
    ///
    /// Creates a real task in the task repository so the triage is visible in
    /// the task queue alongside user tasks.
    pub async fn run_startup_triage(&self) -> DomainResult<bool>
    where
        M: MemoryRepository + Send + Sync + 'static,
    {
        use crate::domain::models::SessionStatus;
        use crate::domain::models::task::{Task, TaskSource, TaskStatus, TaskType};

        let Some(ref memory_repo) = self.memory_repo else {
            return Ok(false);
        };

        // Idempotency check: skip if codebase-profile already exists
        if let Ok(Some(_)) = memory_repo.get_by_key("codebase-profile", "triage").await {
            self.audit_log
                .info(
                    AuditCategory::System,
                    AuditAction::SwarmStarted,
                    "Skipping startup triage — codebase-profile already exists in memory",
                )
                .await;
            return Ok(false);
        }

        self.audit_log
            .info(
                AuditCategory::System,
                AuditAction::SwarmStarted,
                "Running startup codebase triage...",
            )
            .await;

        // Create a real task in the task repository
        let mut task = Task::with_title(
            "Startup codebase triage",
            "Profile this workspace and store a codebase-profile in memory. \
             Follow the steps in your system prompt exactly.",
        )
        .with_source(TaskSource::System)
        .with_task_type(TaskType::Research)
        .with_agent("codebase-triage");

        let _ = task.transition_to(TaskStatus::Ready);
        let _ = task.transition_to(TaskStatus::Running);

        self.task_repo.create(&task).await?;

        // Build MCP stdio config so the triage agent can access memory and task tools.
        // Same pattern as goal_processing.rs — uses absolute paths so the MCP server
        // finds the DB regardless of the agent's working directory.
        let triage_template = crate::domain::models::specialist_templates::create_triage_agent();
        let abathur_exe =
            std::env::current_exe().unwrap_or_else(|_| std::path::PathBuf::from("abathur"));
        let db_path = std::env::current_dir()
            .unwrap_or_else(|_| self.config.repo_path.clone())
            .join(".abathur")
            .join("abathur.db");
        let mcp_config = serde_json::json!({
            "mcpServers": {
                "abathur": {
                    "command": abathur_exe.to_string_lossy(),
                    "args": [
                        "mcp", "stdio",
                        "--db-path", db_path.to_string_lossy(),
                        "--task-id", task.id.to_string()
                    ]
                }
            }
        });

        let user_prompt = format!(
            "Your task ID is `{}`. Use this ID when calling task_update_status.\n\n{}",
            task.id, task.description
        );

        let request = crate::domain::models::SubstrateRequest::new(
            task.id,
            &triage_template.name,
            &triage_template.system_prompt,
            &user_prompt,
        )
        .with_config(crate::domain::models::SubstrateConfig {
            max_turns: 12,
            working_dir: Some(self.config.repo_path.to_string_lossy().to_string()),
            model: triage_template.preferred_model.clone(),
            mcp_servers: vec![mcp_config.to_string()],
            ..Default::default()
        });

        match self.substrate.execute(request).await {
            Ok(session) if session.status == SessionStatus::Completed => {
                tracing::info!(
                    session_id = %session.id,
                    turns = session.turns_completed,
                    task_id = %task.id,
                    "Startup codebase triage completed"
                );
                let _ = task.transition_to(TaskStatus::Complete);
                let _ = self.task_repo.update(&task).await;
                Ok(true)
            }
            Ok(session) => {
                // Agent ran but didn't complete (max_turns, error, etc.)
                let error = session.error.unwrap_or_else(|| "unknown".to_string());
                tracing::warn!(
                    status = ?session.status,
                    turns = session.turns_completed,
                    task_id = %task.id,
                    "Startup codebase triage agent did not complete: {}", error
                );
                let _ = task.transition_to(TaskStatus::Failed);
                let _ = self.task_repo.update(&task).await;
                Err(crate::domain::errors::DomainError::SubstrateError(format!(
                    "Triage agent did not complete: {}",
                    error
                )))
            }
            Err(e) => {
                tracing::warn!("Startup codebase triage failed: {}", e);
                let _ = task.transition_to(TaskStatus::Failed);
                let _ = self.task_repo.update(&task).await;
                Err(e)
            }
        }
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

    /// Start the convergence polling daemon for federation.
    ///
    /// Requires an A2A client and a federated goal repository. If the
    /// federation service is not configured this is a no-op.
    pub async fn start_convergence_poller(
        &self,
        a2a_client: Arc<dyn crate::adapters::a2a::A2AClient>,
        federated_goal_repo: Arc<dyn crate::domain::ports::FederatedGoalRepository>,
    ) -> DomainResult<()> {
        let Some(ref federation_service) = self.federation_service else {
            return Ok(());
        };

        use crate::services::federation::convergence_poller::{
            ConvergencePollerConfig, ConvergencePollingDaemon,
        };

        let daemon = ConvergencePollingDaemon::new(
            federation_service.clone(),
            a2a_client,
            federated_goal_repo,
            self.event_bus.clone(),
            ConvergencePollerConfig::default(),
        );

        let handle = daemon.start();

        {
            let mut stored = self.convergence_poller_handle.write().await;
            *stored = Some(handle);
        }

        self.audit_log
            .info(
                AuditCategory::System,
                AuditAction::SwarmStarted,
                "Convergence polling daemon started",
            )
            .await;

        Ok(())
    }

    /// Start the convergence publisher daemon for federation (Cerebrate role).
    ///
    /// The publisher periodically snapshots local convergence state and attaches
    /// it to federated A2A tasks so the parent Overmind can poll them. Only starts
    /// when the federation service is configured and the role is Cerebrate.
    pub async fn start_convergence_publisher(
        &self,
        a2a_tasks: Arc<
            tokio::sync::RwLock<
                std::collections::HashMap<String, crate::adapters::mcp::a2a_http::InMemoryTask>,
            >,
        >,
    ) -> DomainResult<()> {
        let Some(ref federation_service) = self.federation_service else {
            return Ok(());
        };

        use crate::services::federation::config::FederationRole;

        if !matches!(federation_service.config().role, FederationRole::Cerebrate) {
            return Ok(());
        }

        use crate::services::federation::convergence_publisher::ConvergencePublisher;

        let mut publisher =
            ConvergencePublisher::new(a2a_tasks, std::time::Duration::from_secs(30));

        if let Some(ref trajectory_repo) = self.trajectory_repo {
            publisher = publisher.with_trajectory_repo(trajectory_repo.clone());
        }

        let handle = publisher.spawn();

        {
            let mut stored = self.convergence_publisher_handle.write().await;
            *stored = Some(handle);
        }

        self.audit_log
            .info(
                AuditCategory::System,
                AuditAction::SwarmStarted,
                "Convergence publisher daemon started (Cerebrate role)",
            )
            .await;

        Ok(())
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
        let maintenance_service =
            Arc::new(MemoryMaintenanceService::from_memory_service(memory_service));
        let daemon = MemoryDecayDaemon::new(maintenance_service, DecayDaemonConfig::default())
            .with_event_bus(self.event_bus.clone());

        // Get the handle before running
        let handle = daemon.handle();

        // Store the handle
        {
            let mut daemon_handle = self.decay_daemon_handle.write().await;
            *daemon_handle = Some(handle);
        }

        // Run daemon and log events in background, publishing to EventBus
        let audit_log = self.audit_log.clone();
        let event_bus = self.event_bus.clone();
        supervise("memory_decay_event_listener", async move {
            let mut event_rx = daemon.run().await;
            while let Some(event) = event_rx.recv().await {
                match event {
                    crate::services::DecayDaemonEvent::Started => {
                        audit_log
                            .info(
                                AuditCategory::System,
                                AuditAction::SwarmStarted,
                                "Memory decay daemon started",
                            )
                            .await;
                    }
                    crate::services::DecayDaemonEvent::MaintenanceStarted { run_number } => {
                        tracing::debug!(run_number, "Memory maintenance cycle starting");
                    }
                    crate::services::DecayDaemonEvent::MaintenanceCompleted {
                        run_number,
                        report,
                        ..
                    } => {
                        audit_log
                            .info(
                                AuditCategory::Memory,
                                AuditAction::MemoryPruned,
                                format!(
                                    "Memory maintenance #{}: {} expired, {} decayed, {} promoted",
                                    run_number,
                                    report.expired_pruned,
                                    report.decayed_pruned,
                                    report.promoted
                                ),
                            )
                            .await;
                    }
                    crate::services::DecayDaemonEvent::MaintenanceFailed {
                        run_number,
                        error,
                        consecutive_failures,
                        max_consecutive_failures,
                    } => {
                        tracing::warn!(
                            run_number,
                            consecutive_failures,
                            max_consecutive_failures,
                            "Memory maintenance failed: {}",
                            error,
                        );
                        // Publish to EventBus so handlers/monitors can react
                        event_bus
                            .publish(crate::services::event_factory::make_event(
                                crate::services::event_bus::EventSeverity::Warning,
                                crate::services::event_bus::EventCategory::Memory,
                                None,
                                None,
                                crate::services::event_bus::EventPayload::MemoryMaintenanceFailed {
                                    run_number,
                                    error,
                                    consecutive_failures,
                                    max_consecutive_failures,
                                },
                            ))
                            .await;
                    }
                    crate::services::DecayDaemonEvent::FailureThresholdWarning {
                        consecutive_failures,
                        max_consecutive_failures,
                        latest_error,
                    } => {
                        tracing::error!(
                            consecutive_failures,
                            max_consecutive_failures,
                            "Memory daemon DEGRADED — approaching failure limit: {}",
                            latest_error,
                        );
                        audit_log
                            .log(AuditEntry::new(
                                AuditLevel::Error,
                                AuditCategory::System,
                                AuditAction::SwarmStopped,
                                AuditActor::System,
                                format!(
                                    "Memory daemon degraded: {}/{} consecutive failures — {}",
                                    consecutive_failures, max_consecutive_failures, latest_error
                                ),
                            ))
                            .await;
                        event_bus
                            .publish(crate::services::event_factory::make_event(
                                crate::services::event_bus::EventSeverity::Error,
                                crate::services::event_bus::EventCategory::Memory,
                                None,
                                None,
                                crate::services::event_bus::EventPayload::MemoryDaemonDegraded {
                                    consecutive_failures,
                                    max_consecutive_failures,
                                    latest_error,
                                },
                            ))
                            .await;
                    }
                    crate::services::DecayDaemonEvent::Stopped { reason } => {
                        let reason_str = format!("{:?}", reason);
                        let severity = if reason == crate::services::StopReason::TooManyFailures {
                            AuditLevel::Error
                        } else {
                            AuditLevel::Info
                        };
                        audit_log
                            .log(AuditEntry::new(
                                severity,
                                AuditCategory::System,
                                AuditAction::SwarmStopped,
                                AuditActor::System,
                                format!("Memory decay daemon stopped: {}", reason_str),
                            ))
                            .await;
                        event_bus
                            .publish(crate::services::event_factory::make_event(
                                if reason == crate::services::StopReason::TooManyFailures {
                                    crate::services::event_bus::EventSeverity::Critical
                                } else {
                                    crate::services::event_bus::EventSeverity::Info
                                },
                                crate::services::event_bus::EventCategory::Memory,
                                None,
                                None,
                                crate::services::event_bus::EventPayload::MemoryDaemonStopped {
                                    reason: reason_str,
                                },
                            ))
                            .await;
                    }
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

    /// Start the outbox poller background daemon.
    ///
    /// Reads unpublished events from the outbox table and publishes them
    /// to the EventBus. Only starts if an outbox repository is configured
    /// (i.e., a pool was provided via `with_pool`).
    pub async fn start_outbox_poller(&self) {
        let Some(ref outbox) = self.outbox_repo else {
            return;
        };

        let poller = crate::services::outbox_poller::OutboxPoller::new(
            outbox.clone(),
            self.event_bus.clone(),
            crate::services::outbox_poller::OutboxPollerConfig::default(),
        );

        let handle = poller.start();

        {
            let mut stored = self.outbox_poller_handle.write().await;
            *stored = Some(handle);
        }

        tracing::info!("Outbox poller started");
    }

    /// Stop the outbox poller.
    pub async fn stop_outbox_poller(&self) {
        let handle = self.outbox_poller_handle.read().await;
        if let Some(ref h) = *handle {
            h.stop();
        }
    }

    /// Stop the convergence poller daemon.
    pub async fn stop_convergence_poller(&self) {
        let handle = self.convergence_poller_handle.read().await;
        if let Some(ref h) = *handle {
            h.stop();
        }
    }

    /// Stop the convergence publisher daemon.
    pub async fn stop_convergence_publisher(&self) {
        let handle = self.convergence_publisher_handle.read().await;
        if let Some(ref h) = *handle {
            h.stop();
        }
    }

    /// Create or reuse a worktree for task execution.
    ///
    /// On retries the worktree from the previous attempt still exists in the DB
    /// (and on disk).  Instead of failing with a UNIQUE constraint error, we
    /// detect the existing worktree and reuse its path.
    ///
    /// For subtasks (tasks with a parent_id), the worktree branches from the
    /// root ancestor's feature branch instead of the default base ref. This
    /// enables the feature-branch aggregation pattern where all subtask work
    /// is merged back into a single feature branch for a combined PR.
    pub(super) async fn create_worktree_for_task(&self, task_id: Uuid) -> DomainResult<String> {
        // Fast path: if a worktree already exists for this task (retry scenario),
        // reuse it instead of trying to create a duplicate.
        if let Ok(Some(existing)) = self.worktree_repo.get_by_task(task_id).await {
            tracing::info!(
                "Reusing existing worktree for task {} at {}",
                task_id,
                existing.path
            );
            return Ok(existing.path);
        }

        // If this is a subtask, branch from the root ancestor's feature branch
        let parent_base_ref = if let Ok(Some(task)) = self.task_repo.get(task_id).await {
            if let Some(parent_id) = task.parent_id {
                let root_id = self.find_root_ancestor(parent_id).await;
                match self.worktree_repo.get_by_task(root_id).await {
                    Ok(Some(root_wt)) if !root_wt.status.is_terminal() => {
                        Some(root_wt.branch.clone())
                    }
                    _ => None, // Root has no active worktree; use default
                }
            } else {
                None
            }
        } else {
            None
        };

        let worktree_config = WorktreeConfig {
            base_path: self.config.worktree_base_path.clone(),
            repo_path: self.config.repo_path.clone(),
            default_base_ref: self.config.default_base_ref.clone(),
            auto_cleanup: true,
            fetch_on_sync: self.config.fetch_on_sync,
        };

        let worktree_service = WorktreeService::new(self.worktree_repo.clone(), worktree_config);

        // Pass parent branch as base_ref when available
        let worktree = worktree_service
            .create_worktree(task_id, parent_base_ref.as_deref())
            .await?;

        // Publish via EventBus (bridge forwards to event_tx automatically)
        self.event_bus
            .publish(crate::services::event_factory::task_event(
                crate::services::event_bus::EventSeverity::Info,
                None,
                task_id,
                crate::services::event_bus::EventPayload::WorktreeCreated {
                    task_id,
                    path: worktree.path.clone(),
                },
            ))
            .await;

        Ok(worktree.path)
    }

    /// Provision a workspace for task execution based on the workflow's `WorkspaceKind`.
    ///
    /// - `WorkspaceKind::Worktree` → create (or reuse) a git worktree via
    ///   [`create_worktree_for_task`].
    /// - `WorkspaceKind::TempDir` → create an isolated temporary directory that
    ///   is *not* a git repository.
    /// - `WorkspaceKind::None` → no workspace; returns `None`.
    ///
    /// Returns `None` if the workspace kind is `None` or if provisioning fails.
    /// The caller should treat a `None` result as "work in-process without an
    /// isolated directory" — agents will run with the default substrate CWD.
    pub(super) async fn provision_workspace_for_task(
        &self,
        task_id: Uuid,
        workspace_kind: WorkspaceKind,
    ) -> Option<String> {
        // When worktrees are disabled globally, downgrade Worktree requests
        // so agents work directly in the swarm's working directory.
        let effective_kind =
            if !self.config.use_worktrees && workspace_kind == WorkspaceKind::Worktree {
                tracing::info!(
                    "Worktrees disabled — task {} will use swarm working directory",
                    task_id
                );
                WorkspaceKind::None
            } else {
                workspace_kind
            };

        match effective_kind {
            WorkspaceKind::Worktree => match self.create_worktree_for_task(task_id).await {
                Ok(path) => Some(path),
                Err(e) => {
                    tracing::warn!("Failed to create worktree for task {}: {}", task_id, e);
                    None
                }
            },
            WorkspaceKind::TempDir => {
                let tmp = std::env::temp_dir().join(format!("abathur-task-{}", task_id));
                match std::fs::create_dir_all(&tmp) {
                    Ok(()) => {
                        tracing::debug!(
                            "Provisioned temp directory for task {} at {:?}",
                            task_id,
                            tmp
                        );
                        Some(tmp.to_string_lossy().to_string())
                    }
                    Err(e) => {
                        tracing::warn!(
                            "Failed to create temp directory for task {}: {}",
                            task_id,
                            e
                        );
                        None
                    }
                }
            }
            WorkspaceKind::None => {
                tracing::debug!(
                    "WorkspaceKind::None for task {} — no workspace provisioned",
                    task_id
                );
                None
            }
        }
    }

    /// Find the root ancestor of a task using a single recursive CTE query.
    pub(super) async fn find_root_ancestor(&self, task_id: Uuid) -> Uuid {
        self.task_repo
            .find_root_task_id(task_id)
            .await
            .unwrap_or(task_id)
    }

    /// Update statistics.
    pub(super) async fn update_stats(
        &self,
        event_tx: &mpsc::Sender<SwarmEvent>,
    ) -> DomainResult<()> {
        let task_counts = self.task_repo.count_by_status().await?;
        let active_worktrees = self.worktree_repo.list_active().await?.len();

        let stats = SwarmStats {
            active_goals: self
                .goal_repo
                .list(crate::domain::ports::GoalFilter {
                    status: Some(GoalStatus::Active),
                    ..Default::default()
                })
                .await?
                .len(),
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
        let cb = self.command_bus.read().await.clone();

        // 1. Fail stale Running tasks (started_at older than threshold).
        //    On restart, any task that was Running has lost its agent.
        let running_tasks = self
            .task_repo
            .list(crate::domain::ports::TaskFilter {
                status: Some(TaskStatus::Running),
                ..Default::default()
            })
            .await?;

        for task in &running_tasks {
            tracing::info!(
                "Startup reconciliation: failing stale running task {} ('{}')",
                task.id,
                task.title
            );
            if let Some(ref cb) = cb {
                let envelope = CommandEnvelope::new(
                    CommandSource::System,
                    DomainCommand::Task(TaskCommand::Fail {
                        task_id: task.id,
                        error: Some(
                            "Stale running task detected during startup reconciliation".to_string(),
                        ),
                    }),
                );
                match cb.dispatch(envelope).await {
                    Ok(_) => {
                        corrections += 1;
                    }
                    Err(e) => {
                        tracing::warn!(
                            "CommandBus failed to reconcile task {}, falling back to direct write: {}",
                            task.id,
                            e
                        );
                        let mut task = task.clone();
                        task.status = TaskStatus::Failed;
                        if let Err(e) = self.task_repo.update(&task).await {
                            tracing::warn!("Failed to reconcile task {}: {}", task.id, e);
                        } else {
                            corrections += 1;
                        }
                    }
                }
            } else {
                let mut task = task.clone();
                task.status = TaskStatus::Failed;
                if let Err(e) = self.task_repo.update(&task).await {
                    tracing::warn!("Failed to reconcile task {}: {}", task.id, e);
                } else {
                    corrections += 1;
                }
            }
        }

        // 2. Check Ready tasks with incomplete dependencies -> move back to Pending
        let ready_tasks = self
            .task_repo
            .list(crate::domain::ports::TaskFilter {
                status: Some(TaskStatus::Ready),
                ..Default::default()
            })
            .await?;

        for task in &ready_tasks {
            if !task.depends_on.is_empty() {
                let mut all_deps_complete = true;
                for dep_id in &task.depends_on {
                    if let Ok(Some(dep)) = self.task_repo.get(*dep_id).await
                        && dep.status != TaskStatus::Complete
                    {
                        all_deps_complete = false;
                        break;
                    }
                }
                if !all_deps_complete {
                    tracing::info!(
                        "Startup reconciliation: moving task {} ('{}') back to Pending (incomplete deps)",
                        task.id,
                        task.title
                    );
                    if let Some(ref cb) = cb {
                        let envelope = CommandEnvelope::new(
                            CommandSource::System,
                            DomainCommand::Task(TaskCommand::Transition {
                                task_id: task.id,
                                new_status: TaskStatus::Pending,
                            }),
                        );
                        match cb.dispatch(envelope).await {
                            Ok(_) => {
                                corrections += 1;
                            }
                            Err(e) => {
                                tracing::warn!(
                                    "CommandBus failed to reconcile task {}, falling back to direct write: {}",
                                    task.id,
                                    e
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
                    } else {
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
        }

        // 3. Check Pending tasks with all dependencies complete -> transition to Ready
        let pending_tasks = self
            .task_repo
            .list(crate::domain::ports::TaskFilter {
                status: Some(TaskStatus::Pending),
                ..Default::default()
            })
            .await?;

        for task in &pending_tasks {
            let should_promote = if task.depends_on.is_empty() {
                true
            } else {
                let mut all_complete = true;
                for dep_id in &task.depends_on {
                    if let Ok(Some(dep)) = self.task_repo.get(*dep_id).await
                        && dep.status != TaskStatus::Complete
                    {
                        all_complete = false;
                        break;
                    }
                }
                all_complete
            };

            if should_promote {
                tracing::info!(
                    "Startup reconciliation: promoting task {} ('{}') to Ready",
                    task.id,
                    task.title
                );
                if let Some(ref cb) = cb {
                    let envelope = CommandEnvelope::new(
                        CommandSource::System,
                        DomainCommand::Task(TaskCommand::Transition {
                            task_id: task.id,
                            new_status: TaskStatus::Ready,
                        }),
                    );
                    match cb.dispatch(envelope).await {
                        Ok(_) => {
                            corrections += 1;
                        }
                        Err(e) => {
                            tracing::warn!(
                                "CommandBus failed to reconcile task {}, falling back to direct write: {}",
                                task.id,
                                e
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
                } else {
                    let mut task = task.clone();
                    task.status = TaskStatus::Ready;
                    if let Err(e) = self.task_repo.update(&task).await {
                        tracing::warn!("Failed to reconcile task {}: {}", task.id, e);
                    } else {
                        corrections += 1;
                    }
                }
            }
        }

        // 4. Fix stale Validating tasks based on workflow_state and staleness.
        //    Uses updated_at against stale_validating_timeout_secs (default 1800s / 30min).
        //    - Stale + PhaseReady → transition to Running (overmind can retry)
        //    - Stale + terminal workflow_state → force-transition to match terminal state
        //    - Stale + no workflow_state → fail (standalone task lost its validator)
        //    - Stale + Verifying → leave alone (ReconciliationHandler handles at runtime)
        //    - Not stale → leave alone
        //    Inconsistent states (e.g. Validating + PhaseReady) are always fixed
        //    regardless of staleness, since they represent deadlocks.
        let validating_tasks = self
            .task_repo
            .list(crate::domain::ports::TaskFilter {
                status: Some(TaskStatus::Validating),
                ..Default::default()
            })
            .await?;

        let now = chrono::Utc::now();
        // Default 1800s (30 minutes); matches ReconciliationHandler default
        let stale_validating_timeout = chrono::Duration::seconds(1800);

        for task in &validating_tasks {
            use crate::domain::models::workflow_state::WorkflowState;

            let elapsed = now - task.updated_at;
            let is_stale = elapsed > stale_validating_timeout;

            let ws = task.workflow_state();

            match ws {
                Some(WorkflowState::Verifying { .. }) => {
                    // Verifying is the expected state during validation — leave alone.
                    // The ReconciliationHandler will handle re-triggering if stale at runtime.
                    continue;
                }

                Some(ref ws) if ws.is_terminal() => {
                    // Terminal workflow state but task stuck in Validating — always fix (deadlock).
                    let target_status = match ws {
                        WorkflowState::Completed { .. } => TaskStatus::Complete,
                        WorkflowState::Failed { .. } | WorkflowState::Rejected { .. } => {
                            TaskStatus::Failed
                        }
                        _ => TaskStatus::Failed,
                    };
                    tracing::warn!(
                        "Startup reconciliation: Validating task {} ('{}') has terminal workflow_state {:?} — force-transitioning to {:?}",
                        task.id,
                        task.title,
                        ws,
                        target_status
                    );
                    if let Some(ref cb) = cb {
                        let envelope = CommandEnvelope::new(
                            CommandSource::System,
                            DomainCommand::Task(TaskCommand::ForceTransition {
                                task_id: task.id,
                                new_status: target_status,
                                reason: format!(
                                    "Startup reconciliation: terminal workflow_state {:?} but task stuck in Validating",
                                    ws
                                ),
                            }),
                        );
                        match cb.dispatch(envelope).await {
                            Ok(_) => {
                                corrections += 1;
                            }
                            Err(e) => {
                                tracing::warn!(
                                    "CommandBus failed to reconcile task {}, falling back to direct write: {}",
                                    task.id,
                                    e
                                );
                                let mut task = task.clone();
                                task.status = target_status;
                                if let Err(e) = self.task_repo.update(&task).await {
                                    tracing::warn!("Failed to reconcile task {}: {}", task.id, e);
                                } else {
                                    corrections += 1;
                                }
                            }
                        }
                    } else {
                        let mut task = task.clone();
                        task.status = target_status;
                        if let Err(e) = self.task_repo.update(&task).await {
                            tracing::warn!("Failed to reconcile task {}: {}", task.id, e);
                        } else {
                            corrections += 1;
                        }
                    }
                }

                Some(WorkflowState::PhaseReady { .. }) => {
                    // PhaseReady + Validating is always a deadlock — fix regardless of staleness.
                    // Transition to Running so the overmind can resume driving the workflow.
                    tracing::warn!(
                        "Startup reconciliation: Validating task {} ('{}') has WorkflowState::PhaseReady — transitioning to Running (deadlock fix)",
                        task.id,
                        task.title
                    );
                    if let Some(ref cb) = cb {
                        let envelope = CommandEnvelope::new(
                            CommandSource::System,
                            DomainCommand::Task(TaskCommand::ForceTransition {
                                task_id: task.id,
                                new_status: TaskStatus::Running,
                                reason: "Startup reconciliation: Validating+PhaseReady deadlock — resuming as Running".to_string(),
                            }),
                        );
                        match cb.dispatch(envelope).await {
                            Ok(_) => {
                                corrections += 1;
                            }
                            Err(e) => {
                                tracing::warn!(
                                    "CommandBus failed to reconcile task {}, falling back to direct write: {}",
                                    task.id,
                                    e
                                );
                                let mut task = task.clone();
                                task.status = TaskStatus::Running;
                                if let Err(e) = self.task_repo.update(&task).await {
                                    tracing::warn!("Failed to reconcile task {}: {}", task.id, e);
                                } else {
                                    corrections += 1;
                                }
                            }
                        }
                    } else {
                        let mut task = task.clone();
                        task.status = TaskStatus::Running;
                        if let Err(e) = self.task_repo.update(&task).await {
                            tracing::warn!("Failed to reconcile task {}: {}", task.id, e);
                        } else {
                            corrections += 1;
                        }
                    }
                }

                None if is_stale => {
                    // No workflow state (standalone task) and stale — fail it.
                    tracing::warn!(
                        "Startup reconciliation: stale Validating task {} ('{}') has no workflow_state — failing (stale {}s)",
                        task.id,
                        task.title,
                        elapsed.num_seconds()
                    );
                    if let Some(ref cb) = cb {
                        let envelope = CommandEnvelope::new(
                            CommandSource::System,
                            DomainCommand::Task(TaskCommand::Fail {
                                task_id: task.id,
                                error: Some("Stale validating task with no workflow_state detected during startup reconciliation".to_string()),
                            }),
                        );
                        match cb.dispatch(envelope).await {
                            Ok(_) => {
                                corrections += 1;
                            }
                            Err(e) => {
                                tracing::warn!(
                                    "CommandBus failed to reconcile task {}, falling back to direct write: {}",
                                    task.id,
                                    e
                                );
                                let mut task = task.clone();
                                task.status = TaskStatus::Failed;
                                if let Err(e) = self.task_repo.update(&task).await {
                                    tracing::warn!("Failed to reconcile task {}: {}", task.id, e);
                                } else {
                                    corrections += 1;
                                }
                            }
                        }
                    } else {
                        let mut task = task.clone();
                        task.status = TaskStatus::Failed;
                        if let Err(e) = self.task_repo.update(&task).await {
                            tracing::warn!("Failed to reconcile task {}: {}", task.id, e);
                        } else {
                            corrections += 1;
                        }
                    }
                }

                Some(_) if is_stale => {
                    // Other non-terminal workflow states (PhaseRunning, FanningOut, etc.) and stale —
                    // transition to Running so the overmind can resume.
                    let ws_ref = task.workflow_state();
                    tracing::warn!(
                        "Startup reconciliation: stale Validating task {} ('{}') has non-terminal workflow_state {:?} — transitioning to Running (stale {}s)",
                        task.id,
                        task.title,
                        ws_ref,
                        elapsed.num_seconds()
                    );
                    if let Some(ref cb) = cb {
                        let envelope = CommandEnvelope::new(
                            CommandSource::System,
                            DomainCommand::Task(TaskCommand::ForceTransition {
                                task_id: task.id,
                                new_status: TaskStatus::Running,
                                reason: format!(
                                    "Startup reconciliation: stale Validating with workflow_state {:?} — resuming as Running",
                                    ws_ref
                                ),
                            }),
                        );
                        match cb.dispatch(envelope).await {
                            Ok(_) => {
                                corrections += 1;
                            }
                            Err(e) => {
                                tracing::warn!(
                                    "CommandBus failed to reconcile task {}, falling back to direct write: {}",
                                    task.id,
                                    e
                                );
                                let mut task = task.clone();
                                task.status = TaskStatus::Running;
                                if let Err(e) = self.task_repo.update(&task).await {
                                    tracing::warn!("Failed to reconcile task {}: {}", task.id, e);
                                } else {
                                    corrections += 1;
                                }
                            }
                        }
                    } else {
                        let mut task = task.clone();
                        task.status = TaskStatus::Running;
                        if let Err(e) = self.task_repo.update(&task).await {
                            tracing::warn!("Failed to reconcile task {}: {}", task.id, e);
                        } else {
                            corrections += 1;
                        }
                    }
                }

                _ => {
                    // Not stale, no inconsistency — leave alone
                    tracing::debug!(
                        "Startup reconciliation: Validating task {} ('{}') is not stale — leaving alone",
                        task.id,
                        task.title
                    );
                }
            }
        }

        // Recover InProgress refinement requests from previous process run
        self.evolution_loop.recover_in_progress_refinements().await;

        // Restore persisted template stats and version changes
        self.evolution_loop.load_persisted_state().await;

        Ok(corrections)
    }
}
