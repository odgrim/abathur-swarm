//! Task execution service.
//!
//! Owns the per-task execution lifecycle that previously lived inside the
//! `tokio::spawn` closure of `goal_processing::spawn_task_agent`. Coordinates
//! the Direct and Convergent execution paths and dispatches to the existing
//! post-completion middleware chain.
//!
//! ## Intent-gap retry flow
//!
//! When convergent execution returns
//! [`ConvergentOutcome::IntentGapsFound`](super::convergent_execution::ConvergentOutcome::IntentGapsFound),
//! the orchestrator must (a) annotate the failing task with structured gap
//! context for the next attempt, and (b) for *standalone* tasks (no parent
//! workflow), create an explicit retry task seeded with the gap descriptions.
//! Workflow subtasks rely on the workflow engine to re-enqueue.
//!
//! Both halves of that flow are owned by [`handle_intent_gaps_with_retry`] so
//! future maintainers can find the full flow in one place. The risk-mitigation
//! note for this lives in spec T10 §6 Risk 4.
//!
//! ## Risk 1 (deadlock) mitigation
//!
//! Every Arc/handle the spawn-block needs is captured into [`ExecutionConfig`]
//! and [`TaskExecutionParams`] **before** the orchestrator calls
//! `tokio::spawn`. No `Arc<RwLock<>>` or `Arc<Mutex<>>` is constructed inside
//! `execute_task()`. See spec T10 §6 Risk 1 for the rationale.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use tokio::sync::{RwLock, Semaphore, mpsc};

use crate::domain::models::workflow_template::WorkflowTemplate;
use crate::domain::models::{
    AgentTier, ExecutionMode, OutputDelivery, SessionStatus, SubstrateConfig, SubstrateRequest,
    Task, TaskStatus,
};
use crate::domain::models::convergence::ConvergenceEngineConfig;
use crate::domain::ports::{
    GoalRepository, MergeRequestRepository, Substrate, TaskRepository, TrajectoryRepository,
    WorktreeRepository,
};
use crate::services::command_bus::{CommandBus, CommandEnvelope, CommandSource, DomainCommand, TaskCommand};
use crate::services::event_bus::EventBus;
use crate::services::evolution_loop::EvolutionLoop;
use crate::services::guardrails::Guardrails;
use crate::services::{
    AgentTierHint, AuditAction, AuditActor, AuditCategory, AuditEntry, AuditLevel, CircuitScope,
    ModelRouter, TaskExecution, TaskOutcome,
};

use super::agent_prep::AgentMetadata;
use super::convergent_execution::{ConvergentIntentVerifier, ConvergentOutcome};
use super::goal_processing::{
    can_safely_auto_complete, is_max_turns_auto_completable, replay_gate_rejection_event,
};
use super::helpers::{
    PostCompletionWorkflowParams, auto_commit_worktree, run_post_completion_workflow,
};
use super::middleware::PostCompletionChain;
use super::types::SwarmEvent;

/// Static configuration captured before spawning the per-task worker.
///
/// Risk 3 mitigation: `post_completion_chain` is part of this struct so the
/// spawn block always has access to the verify/merge/PR middleware. A unit
/// test asserts the chain fires for completed direct-mode tasks.
pub struct ExecutionConfig {
    pub repo_path: PathBuf,
    pub default_base_ref: String,
    /// The orchestrator owns the semaphore; permits are acquired before
    /// `execute_task` is called and travel via `TaskExecutionParams::permit`.
    /// Kept here so the orchestrator can pass through any future construction
    /// helpers without rewiring callsites.
    // reason: held but not read on the spawn path; permits already flow via
    // `TaskExecutionParams::permit`. Kept on `ExecutionConfig` so the
    // orchestrator can hand it to future helpers without rewiring callsites.
    #[allow(dead_code)]
    pub agent_semaphore: Arc<Semaphore>,
    pub guardrails: Arc<Guardrails>,
    pub require_commits: bool,
    pub verify_on_completion: bool,
    pub use_merge_queue: bool,
    pub prefer_pull_requests: bool,
    pub track_evolution: bool,
    pub evolution_loop: Arc<EvolutionLoop>,
    pub fetch_on_sync: bool,
    pub output_delivery: OutputDelivery,
    pub merge_request_repo: Option<Arc<dyn MergeRequestRepository>>,
    pub post_completion_chain: Arc<RwLock<PostCompletionChain>>,
}

/// Parameters captured for a single task execution. Owns every Arc/clone the
/// spawn body needs so the orchestrator can hand it off to `tokio::spawn`
/// without holding any references back to itself (Risk 1).
pub struct TaskExecutionParams {
    pub task: Task,
    pub task_id: uuid::Uuid,
    pub agent_type: String,
    pub system_prompt: String,
    pub task_description: String,
    pub effective_mode: ExecutionMode,
    pub is_convergent: bool,
    pub max_turns: u32,
    pub agent_meta: AgentMetadata,
    pub worktree_path: Option<String>,
    pub all_workflows: Vec<WorkflowTemplate>,
    pub circuit_scope: CircuitScope,
    pub agent_unique_id: String,
    pub template_version: u32,
    pub agent_type_for_evolution: String,

    // Per-spawn dependency clones
    pub substrate: Arc<dyn Substrate>,
    pub task_repo: Arc<dyn TaskRepository>,
    pub worktree_repo: Arc<dyn WorktreeRepository>,
    pub goal_repo: Arc<dyn GoalRepository>,
    pub event_bus: Arc<EventBus>,
    pub event_tx: mpsc::Sender<SwarmEvent>,
    pub audit_log: Arc<crate::services::AuditLogService>,
    pub circuit_breaker: Arc<crate::services::CircuitBreakerService>,
    pub command_bus: Option<Arc<CommandBus>>,
    pub total_tokens: Arc<AtomicU64>,
    pub permit: tokio::sync::OwnedSemaphorePermit,

    // Convergence infrastructure (None when not configured)
    pub overseer_cluster: Option<Arc<crate::services::overseers::OverseerClusterService>>,
    pub trajectory_repo: Option<Arc<dyn TrajectoryRepository>>,
    pub convergence_engine_config: Option<ConvergenceEngineConfig>,
    pub memory_repo: Option<Arc<dyn crate::domain::ports::MemoryRepository>>,
    pub intent_verifier: Option<Arc<dyn ConvergentIntentVerifier>>,

    pub config: ExecutionConfig,
}

/// Per-task worker entry point. Drives Direct or Convergent execution and
/// dispatches to the post-completion middleware chain on success.
///
/// This function takes ownership of every dependency and is intended to be
/// called from inside `tokio::spawn`. Invariant: no `Arc<RwLock<>>` or
/// `Arc<Mutex<>>` is constructed in this function (Risk 1 mitigation).
pub async fn execute_task(params: TaskExecutionParams) {
    let TaskExecutionParams {
        task: task_clone,
        task_id,
        agent_type,
        system_prompt,
        task_description,
        effective_mode,
        is_convergent,
        max_turns,
        agent_meta,
        worktree_path,
        all_workflows,
        circuit_scope,
        agent_unique_id,
        template_version,
        agent_type_for_evolution,
        substrate,
        task_repo,
        worktree_repo,
        goal_repo,
        event_bus,
        event_tx,
        audit_log,
        circuit_breaker,
        command_bus,
        total_tokens,
        permit,
        overseer_cluster,
        trajectory_repo,
        convergence_engine_config,
        memory_repo,
        intent_verifier,
        config,
    } = params;

    let _permit = permit;
    let template_version_for_evolution = template_version;
    let template_preferred_model = agent_meta.preferred_model.clone();
    let template_tier = agent_meta.tier;
    let cli_tools = agent_meta.cli_tools.clone();
    let post_task_repo: Arc<dyn TaskRepository> = task_repo.clone();
    let post_goal_repo: Arc<dyn GoalRepository> = goal_repo;
    let post_worktree_repo: Arc<dyn WorktreeRepository> = worktree_repo.clone();
    let post_completion_chain = config.post_completion_chain.clone();
    let repo_path = config.repo_path;
    let default_base_ref = config.default_base_ref;
    let verify_on_completion = config.verify_on_completion;
    let use_merge_queue = config.use_merge_queue;
    let prefer_pull_requests = config.prefer_pull_requests;
    let track_evolution = config.track_evolution;
    let evolution_loop = config.evolution_loop;
    let require_commits = config.require_commits;
    let fetch_on_sync = config.fetch_on_sync;
    let output_delivery = config.output_delivery;
    let merge_request_repo = config.merge_request_repo;
    let guardrails = config.guardrails;

    // Task is already Running (claimed atomically before spawn).

    // -----------------------------------------------------------------
    // Convergent execution path (Phase 3)
    // -----------------------------------------------------------------
    if is_convergent {
        let converge_components = match (
            overseer_cluster.clone(),
            trajectory_repo.clone(),
            memory_repo.clone(),
            intent_verifier.clone(),
        ) {
            (Some(oc), Some(tr), Some(mr), Some(iv)) => Some((oc, tr, mr, iv)),
            _ => None,
        };

        if let Some((overseer_cluster, trajectory_repo_arc, memory_repo, convergent_intent_verifier)) =
            converge_components
        {
            let trajectory_repo_wrapped = Arc::new(
                crate::services::convergence_bridge::DynTrajectoryRepository(trajectory_repo_arc),
            );
            let memory_repo_wrapped = Arc::new(
                crate::services::convergence_bridge::DynMemoryRepository(memory_repo),
            );

            let engine_config = convergence_engine_config.unwrap_or_else(|| {
                crate::services::convergence_bridge::build_engine_config_from_defaults()
            });

            let engine = crate::services::convergence_engine::ConvergenceEngine::new(
                trajectory_repo_wrapped.clone(),
                memory_repo_wrapped,
                overseer_cluster,
                engine_config,
            );

            let goal_id: Option<uuid::Uuid> = None;

            let convergent_worktree_path = if worktree_path.is_some() {
                match worktree_repo.get_by_task(task_id).await {
                    Ok(Some(wt)) => Some(wt.path.clone()),
                    _ => worktree_path.clone(),
                }
            } else {
                None
            };

            audit_log
                .log(
                    AuditEntry::new(
                        AuditLevel::Info,
                        AuditCategory::Execution,
                        AuditAction::TaskCompleted,
                        AuditActor::System,
                        format!(
                            "Task {} entering convergent execution (mode: {:?})",
                            task_id, task_clone.execution_mode
                        ),
                    )
                    .with_entity(task_id, "task"),
                )
                .await;

            let cancellation_token = tokio_util::sync::CancellationToken::new();
            let deadline = task_clone.deadline;

            let outcome = if let ExecutionMode::Convergent {
                parallel_samples: Some(n),
            } = &effective_mode
            {
                if worktree_path.is_some() {
                    super::convergent_execution::run_parallel_convergent_execution(
                        &task_clone,
                        goal_id,
                        &substrate,
                        &task_repo,
                        &trajectory_repo_wrapped,
                        &engine,
                        &event_bus,
                        &agent_type,
                        &system_prompt,
                        max_turns,
                        cancellation_token,
                        deadline,
                        *n,
                        &default_base_ref,
                        &format!(
                            "{}/convergent_parallel_{}",
                            repo_path.display(),
                            task_id
                        ),
                        convergent_intent_verifier.clone(),
                    )
                    .await
                } else {
                    tracing::warn!(
                        task_id = %task_id,
                        parallel_samples = n,
                        "Parallel convergent mode requested but worktrees disabled; falling back to sequential"
                    );
                    super::convergent_execution::run_convergent_execution(
                        &task_clone,
                        goal_id,
                        &substrate,
                        &task_repo,
                        &trajectory_repo_wrapped,
                        &engine,
                        &event_bus,
                        &agent_type,
                        &system_prompt,
                        convergent_worktree_path.as_deref(),
                        max_turns,
                        cancellation_token,
                        deadline,
                        convergent_intent_verifier.clone(),
                    )
                    .await
                }
            } else {
                super::convergent_execution::run_convergent_execution(
                    &task_clone,
                    goal_id,
                    &substrate,
                    &task_repo,
                    &trajectory_repo_wrapped,
                    &engine,
                    &event_bus,
                    &agent_type,
                    &system_prompt,
                    convergent_worktree_path.as_deref(),
                    max_turns,
                    cancellation_token,
                    deadline,
                    convergent_intent_verifier,
                )
                .await
            };

            if let Some(ref wt_path) = convergent_worktree_path {
                let _ = auto_commit_worktree(wt_path, task_id).await;
            }

            // Persist convergent outcome on task context for downstream
            // workflow verification.
            {
                let outcome_str = match &outcome {
                    Ok(ConvergentOutcome::Converged) => "converged",
                    Ok(ConvergentOutcome::IndeterminateAccepted) => "indeterminate_accepted",
                    Ok(ConvergentOutcome::PartialAccepted) => "partial_accepted",
                    Ok(ConvergentOutcome::IntentGapsFound(_)) => "intent_gaps_found",
                    Ok(ConvergentOutcome::Decomposed(_)) => "decomposed",
                    Ok(ConvergentOutcome::Failed(_)) => "failed",
                    Ok(ConvergentOutcome::Cancelled) => "cancelled",
                    Err(_) => "error",
                };
                if let Ok(Some(mut t)) = task_repo.get(task_id).await {
                    t.context.custom.insert(
                        "convergence_outcome".to_string(),
                        serde_json::json!(outcome_str),
                    );
                    let _ = task_repo.update(&t).await;
                }
            }

            match outcome {
                Ok(ref convergent_outcome @ ConvergentOutcome::Converged)
                | Ok(ref convergent_outcome @ ConvergentOutcome::IndeterminateAccepted)
                | Ok(ref convergent_outcome @ ConvergentOutcome::PartialAccepted) => {
                    let intent_satisfied =
                        !matches!(convergent_outcome, ConvergentOutcome::IndeterminateAccepted);

                    let current_task = task_repo.get(task_id).await.ok().flatten();
                    let already_terminal = current_task
                        .as_ref()
                        .is_some_and(|t| t.status.is_terminal());

                    if !already_terminal {
                        let target_status = if verify_on_completion && !intent_satisfied {
                            if current_task
                                .as_ref()
                                .is_some_and(|t| !can_safely_auto_complete(t))
                            {
                                tracing::warn!(
                                    task_id = %task_id,
                                    "Overmind exhausted turns mid-workflow — failing instead of auto-completing to Validating"
                                );
                                TaskStatus::Failed
                            } else {
                                TaskStatus::Validating
                            }
                        } else {
                            TaskStatus::Complete
                        };

                        if let Some(ref cb) = command_bus {
                            let envelope = CommandEnvelope::new(
                                CommandSource::System,
                                DomainCommand::Task(TaskCommand::Transition {
                                    task_id,
                                    new_status: target_status,
                                }),
                            );
                            if let Err(e) = cb.dispatch(envelope).await {
                                tracing::warn!(
                                    "Failed to complete convergent task {} via CommandBus: {}",
                                    task_id, e
                                );
                                if let Ok(Some(mut t)) = task_repo.get(task_id).await
                                    && !t.status.is_terminal()
                                {
                                    let _ = t.transition_to(target_status);
                                    let _ = task_repo.update(&t).await;
                                }
                                if target_status == TaskStatus::Complete {
                                    event_bus
                                        .publish(crate::services::event_factory::task_event(
                                            crate::services::event_bus::EventSeverity::Info,
                                            None,
                                            task_id,
                                            crate::services::event_bus::EventPayload::TaskCompleted {
                                                task_id,
                                                tokens_used: 0,
                                            },
                                        ))
                                        .await;
                                }
                            }
                        } else if let Ok(Some(mut t)) = task_repo.get(task_id).await {
                            if !t.status.is_terminal() {
                                let _ = t.transition_to(target_status);
                                let _ = task_repo.update(&t).await;
                            }
                            if target_status == TaskStatus::Complete {
                                event_bus
                                    .publish(crate::services::event_factory::task_event(
                                        crate::services::event_bus::EventSeverity::Info,
                                        None,
                                        task_id,
                                        crate::services::event_bus::EventPayload::TaskCompleted {
                                            task_id,
                                            tokens_used: 0,
                                        },
                                    ))
                                    .await;
                            }
                        }
                    } else {
                        tracing::debug!(
                            task_id = %task_id,
                            status = ?current_task.as_ref().map(|t| t.status),
                            "Skipping convergent task transition — already terminal (completed via MCP)"
                        );
                    }

                    circuit_breaker.record_success(circuit_scope.clone()).await;

                    if worktree_path.is_some()
                        && let Ok(Some(mut wt)) = worktree_repo.get_by_task(task_id).await
                    {
                        wt.complete();
                        let _ = worktree_repo.update(&wt).await;
                    }

                    if verify_on_completion || use_merge_queue || prefer_pull_requests {
                        let _ = run_post_completion_workflow(PostCompletionWorkflowParams {
                            task_id,
                            task_repo: post_task_repo.clone(),
                            goal_repo: post_goal_repo.clone(),
                            worktree_repo: post_worktree_repo.clone(),
                            event_tx: &event_tx,
                            event_bus: &event_bus,
                            audit_log: &audit_log,
                            verify_on_completion,
                            use_merge_queue,
                            prefer_pull_requests,
                            repo_path: &repo_path,
                            default_base_ref: &default_base_ref,
                            require_commits,
                            intent_satisfied,
                            output_delivery: output_delivery.clone(),
                            merge_request_repo: merge_request_repo.clone(),
                            fetch_on_sync,
                            post_completion_chain: post_completion_chain.clone(),
                        })
                        .await;
                    }

                    if track_evolution {
                        let execution = TaskExecution {
                            task_id,
                            template_name: agent_type_for_evolution.clone(),
                            template_version: template_version_for_evolution,
                            outcome: TaskOutcome::Success,
                            executed_at: chrono::Utc::now(),
                            turns_used: 0,
                            tokens_used: 0,
                            downstream_tasks: vec![],
                        };
                        evolution_loop.record_execution(execution).await;
                    }

                    audit_log
                        .log(
                            AuditEntry::new(
                                AuditLevel::Info,
                                AuditCategory::Task,
                                AuditAction::TaskCompleted,
                                AuditActor::System,
                                format!("Convergent task {} completed successfully", task_id),
                            )
                            .with_entity(task_id, "task"),
                        )
                        .await;
                }

                Ok(ConvergentOutcome::IntentGapsFound(ivr)) => {
                    handle_intent_gaps_with_retry(
                        *ivr,
                        task_id,
                        &task_repo,
                        &event_bus,
                        &audit_log,
                    )
                    .await;
                }

                Ok(ConvergentOutcome::Decomposed(trajectory)) => {
                    let spec = &trajectory.specification.effective;
                    let criteria = &spec.success_criteria;

                    let child_count = if criteria.is_empty() { 1 } else { criteria.len() };

                    audit_log
                        .log(
                            AuditEntry::new(
                                AuditLevel::Info,
                                AuditCategory::Task,
                                AuditAction::TaskCompleted,
                                AuditActor::System,
                                format!(
                                    "Convergent task {} decomposed into {} subtask(s) (trajectory {})",
                                    task_id, child_count, trajectory.id,
                                ),
                            )
                            .with_entity(task_id, "task"),
                        )
                        .await;

                    if criteria.is_empty() {
                        let mut child = Task::with_title(
                            format!("Decomposed from {}", task_id),
                            &spec.content,
                        );
                        child.parent_id = Some(task_id);
                        child.execution_mode = ExecutionMode::Direct;
                        let _ = child.transition_to(TaskStatus::Ready);
                        if let Err(e) = task_repo.create(&child).await {
                            tracing::warn!(
                                "Failed to create decomposed subtask for {}: {}",
                                task_id, e
                            );
                        }
                    } else {
                        for (i, criterion) in criteria.iter().enumerate() {
                            let title = format!(
                                "Subtask {}/{} of {}",
                                i + 1,
                                criteria.len(),
                                task_id,
                            );
                            let description =
                                format!("{}\n\nFocus: {}", spec.content, criterion);
                            let mut child = Task::with_title(&title, &description);
                            child.parent_id = Some(task_id);
                            child.execution_mode = ExecutionMode::Direct;
                            let _ = child.transition_to(TaskStatus::Ready);
                            if let Err(e) = task_repo.create(&child).await {
                                tracing::warn!(
                                    "Failed to create decomposed subtask {} for {}: {}",
                                    i + 1,
                                    task_id,
                                    e
                                );
                            }
                        }
                    }
                }

                Ok(ConvergentOutcome::Failed(msg)) => {
                    let current_task = task_repo.get(task_id).await.ok().flatten();
                    let already_terminal = current_task
                        .as_ref()
                        .is_some_and(|t| t.status.is_terminal());

                    if already_terminal {
                        tracing::warn!(
                            task_id = %task_id,
                            status = ?current_task.as_ref().map(|t| t.status),
                            error = %msg,
                            "Skipping convergent task failure — already terminal (completed via MCP)"
                        );
                    } else {
                        if let Some(ref cb) = command_bus {
                            let envelope = CommandEnvelope::new(
                                CommandSource::System,
                                DomainCommand::Task(TaskCommand::Fail {
                                    task_id,
                                    error: Some(msg.clone()),
                                }),
                            );
                            if let Err(e) = cb.dispatch(envelope).await {
                                tracing::warn!(
                                    "Failed to fail convergent task {} via CommandBus: {}",
                                    task_id, e
                                );
                                if let Ok(Some(mut t)) = task_repo.get(task_id).await
                                    && !t.status.is_terminal()
                                {
                                    let _ = t.transition_to(TaskStatus::Failed);
                                    let _ = task_repo.update(&t).await;
                                }
                            }
                        } else if let Ok(Some(mut t)) = task_repo.get(task_id).await
                            && !t.status.is_terminal()
                        {
                            let _ = t.transition_to(TaskStatus::Failed);
                            let _ = task_repo.update(&t).await;
                        }

                        let current_retry_count = task_repo
                            .get(task_id)
                            .await
                            .ok()
                            .flatten()
                            .map(|t| t.retry_count)
                            .unwrap_or(0);
                        event_bus
                            .publish(crate::services::event_factory::task_event(
                                crate::services::event_bus::EventSeverity::Warning,
                                None,
                                task_id,
                                crate::services::event_bus::EventPayload::TaskFailed {
                                    task_id,
                                    error: msg.clone(),
                                    retry_count: current_retry_count,
                                },
                            ))
                            .await;
                    }

                    circuit_breaker
                        .record_failure(circuit_scope.clone(), &msg)
                        .await;

                    if worktree_path.is_some()
                        && let Ok(Some(mut wt)) = worktree_repo.get_by_task(task_id).await
                    {
                        wt.fail(msg.clone());
                        let _ = worktree_repo.update(&wt).await;
                    }

                    if track_evolution {
                        let execution = TaskExecution {
                            task_id,
                            template_name: agent_type_for_evolution.clone(),
                            template_version: template_version_for_evolution,
                            outcome: TaskOutcome::Failure,
                            executed_at: chrono::Utc::now(),
                            turns_used: 0,
                            tokens_used: 0,
                            downstream_tasks: vec![],
                        };
                        evolution_loop.record_execution(execution).await;
                    }

                    audit_log
                        .log(
                            AuditEntry::new(
                                AuditLevel::Warning,
                                AuditCategory::Task,
                                AuditAction::TaskFailed,
                                AuditActor::System,
                                format!("Convergent task {} failed: {}", task_id, msg),
                            )
                            .with_entity(task_id, "task"),
                        )
                        .await;
                }

                Ok(ConvergentOutcome::Cancelled) => {
                    if let Ok(Some(mut t)) = task_repo.get(task_id).await {
                        let _ = t.transition_to(TaskStatus::Canceled);
                        let _ = task_repo.update(&t).await;
                    }

                    if worktree_path.is_some()
                        && let Ok(Some(mut wt)) = worktree_repo.get_by_task(task_id).await
                    {
                        wt.fail("cancelled".to_string());
                        let _ = worktree_repo.update(&wt).await;
                    }

                    audit_log
                        .log(
                            AuditEntry::new(
                                AuditLevel::Info,
                                AuditCategory::Task,
                                AuditAction::TaskFailed,
                                AuditActor::System,
                                format!("Convergent task {} cancelled", task_id),
                            )
                            .with_entity(task_id, "task"),
                        )
                        .await;
                }

                Err(e) => {
                    let error_msg = format!("Convergent execution error: {}", e);

                    if let Some(ref cb) = command_bus {
                        let envelope = CommandEnvelope::new(
                            CommandSource::System,
                            DomainCommand::Task(TaskCommand::Fail {
                                task_id,
                                error: Some(error_msg.clone()),
                            }),
                        );
                        let _ = cb.dispatch(envelope).await;
                    } else if let Ok(Some(mut t)) = task_repo.get(task_id).await {
                        let _ = t.transition_to(TaskStatus::Failed);
                        let _ = task_repo.update(&t).await;
                    }

                    circuit_breaker
                        .record_failure(circuit_scope.clone(), &error_msg)
                        .await;

                    if worktree_path.is_some()
                        && let Ok(Some(mut wt)) = worktree_repo.get_by_task(task_id).await
                    {
                        wt.fail(error_msg.clone());
                        let _ = worktree_repo.update(&wt).await;
                    }

                    audit_log
                        .log(
                            AuditEntry::new(
                                AuditLevel::Error,
                                AuditCategory::Task,
                                AuditAction::TaskFailed,
                                AuditActor::System,
                                error_msg,
                            )
                            .with_entity(task_id, "task"),
                        )
                        .await;
                }
            }

            guardrails.register_agent_end(&agent_unique_id).await;
            return;
        } else {
            tracing::warn!(
                "Task {} has convergent execution mode but convergence infrastructure \
                 is not fully configured (overseer_cluster={}, trajectory_repo={}, \
                 memory_repo={}, intent_verifier={}). Falling back to direct execution.",
                task_id,
                overseer_cluster.is_some(),
                trajectory_repo.is_some(),
                memory_repo.is_some(),
                intent_verifier.is_some(),
            );
            audit_log
                .log(
                    AuditEntry::new(
                        AuditLevel::Warning,
                        AuditCategory::Execution,
                        AuditAction::TaskFailed,
                        AuditActor::System,
                        format!(
                            "Task {} requested convergent execution but infrastructure not configured; using direct mode",
                            task_id
                        ),
                    )
                    .with_entity(task_id, "task"),
                )
                .await;
        }
    }

    // -----------------------------------------------------------------
    // Direct execution path (single-shot substrate invocation)
    // -----------------------------------------------------------------

    let mut substrate_config = SubstrateConfig::default().with_max_turns(max_turns);
    if let Some(ref wt_path) = worktree_path {
        substrate_config = substrate_config.with_working_dir(wt_path);
    }

    if let Some(ref model) = template_preferred_model {
        substrate_config.model = Some(model.clone());
    } else {
        let tier_hint = match template_tier {
            AgentTier::Architect => AgentTierHint::Architect,
            AgentTier::Specialist => AgentTierHint::Specialist,
            AgentTier::Worker => AgentTierHint::Worker,
        };
        let selection = ModelRouter::with_defaults().select_model(
            task_clone.routing_hints.complexity,
            Some(tier_hint),
            task_clone.retry_count,
        );
        tracing::debug!(
            task_id = %task_id,
            %agent_type,
            model = %selection.model,
            reason = %selection.reason,
            "ModelRouter selected model for direct execution"
        );
        substrate_config.model = Some(selection.model);
    }

    if !cli_tools.is_empty() {
        substrate_config = substrate_config.with_allowed_tools(cli_tools);
    }

    let abathur_exe =
        std::env::current_exe().unwrap_or_else(|_| std::path::PathBuf::from("abathur"));
    let db_path = std::env::current_dir()
        .unwrap_or_else(|_| repo_path.clone())
        .join(".abathur")
        .join("abathur.db");

    let mut mcp_args = vec![
        "mcp".to_string(),
        "stdio".to_string(),
        "--db-path".to_string(),
        db_path.to_string_lossy().to_string(),
        "--task-id".to_string(),
        task_id.to_string(),
    ];
    if agent_type.to_lowercase() == "overmind" {
        mcp_args.push("--workflow-session".to_string());
    }
    let mcp_config = serde_json::json!({
        "mcpServers": {
            "abathur": {
                "command": abathur_exe.to_string_lossy(),
                "args": mcp_args
            }
        }
    });
    substrate_config = substrate_config.with_mcp_server(mcp_config.to_string());

    let request =
        SubstrateRequest::new(task_id, &agent_type, &system_prompt, &task_description)
            .with_config(substrate_config);

    let result = substrate.execute(request).await;

    if let Some(ref wt_path) = worktree_path {
        let _ = auto_commit_worktree(wt_path, task_id).await;
    }

    if let Ok(Some(mut completed_task)) = task_repo.get(task_id).await {
        match result {
            Ok(session) if session.status == SessionStatus::Completed => {
                let tokens = session.total_tokens();
                let turns = session.turns_completed;
                total_tokens.fetch_add(tokens, Ordering::Relaxed);

                if !completed_task.status.is_terminal() {
                    let target_status = if verify_on_completion
                        && can_safely_auto_complete(&completed_task)
                    {
                        TaskStatus::Validating
                    } else if verify_on_completion
                        && !can_safely_auto_complete(&completed_task)
                    {
                        tracing::warn!(
                            task_id = %task_id,
                            "Skipping Validating transition for task with active workflow — completing directly"
                        );
                        TaskStatus::Complete
                    } else {
                        TaskStatus::Complete
                    };
                    if let Some(ref cb) = command_bus {
                        let envelope = CommandEnvelope::new(
                            CommandSource::System,
                            DomainCommand::Task(TaskCommand::Transition {
                                task_id,
                                new_status: target_status,
                            }),
                        );
                        if let Err(e) = cb.dispatch(envelope).await {
                            tracing::warn!(
                                "Failed to complete task {} via CommandBus, using non-atomic fallback: {}",
                                task_id, e
                            );
                            if let Ok(Some(mut t)) = task_repo.get(task_id).await
                                && !t.status.is_terminal()
                            {
                                let _ = t.transition_to(target_status);
                                let _ = task_repo.update(&t).await;
                            }
                            if target_status == TaskStatus::Complete {
                                event_bus
                                    .publish(crate::services::event_factory::task_event(
                                        crate::services::event_bus::EventSeverity::Info,
                                        None,
                                        task_id,
                                        crate::services::event_bus::EventPayload::TaskCompleted {
                                            task_id,
                                            tokens_used: tokens,
                                        },
                                    ))
                                    .await;
                            }
                        }
                    } else {
                        tracing::warn!(
                            "CommandBus not available for task {} completion, using non-atomic fallback",
                            task_id
                        );
                        let _ = completed_task.transition_to(target_status);
                        let _ = task_repo.update(&completed_task).await;
                        if target_status == TaskStatus::Complete {
                            event_bus
                                .publish(crate::services::event_factory::task_event(
                                    crate::services::event_bus::EventSeverity::Info,
                                    None,
                                    task_id,
                                    crate::services::event_bus::EventPayload::TaskCompleted {
                                        task_id,
                                        tokens_used: tokens,
                                    },
                                ))
                                .await;
                        }
                    }
                } else {
                    tracing::debug!(
                        task_id = %task_id,
                        status = ?completed_task.status,
                        "Skipping task transition — already terminal (completed via MCP)"
                    );
                    replay_gate_rejection_event(&completed_task, &event_bus, &all_workflows).await;
                }

                circuit_breaker.record_success(circuit_scope.clone()).await;

                audit_log
                    .log(
                        AuditEntry::new(
                            AuditLevel::Info,
                            AuditCategory::Task,
                            AuditAction::TaskCompleted,
                            AuditActor::System,
                            format!(
                                "Task completed: {} tokens used, {} turns",
                                tokens, turns
                            ),
                        )
                        .with_entity(task_id, "task"),
                    )
                    .await;

                if worktree_path.is_some()
                    && let Ok(Some(mut wt)) = worktree_repo.get_by_task(task_id).await
                {
                    wt.complete();
                    let _ = worktree_repo.update(&wt).await;

                    if let Ok(Some(mut task)) = task_repo.get(task_id).await {
                        let artifact = crate::domain::models::ArtifactRef {
                            uri: format!("worktree://{}/{}", task_id, wt.branch),
                            artifact_type: crate::domain::models::ArtifactType::Code,
                            checksum: wt.merge_commit.clone(),
                        };
                        task.artifacts.push(artifact);
                        task.worktree_path = Some(wt.path.clone());
                        let _ = task_repo.update(&task).await;
                    }
                }

                if verify_on_completion || use_merge_queue || prefer_pull_requests {
                    let workflow_result =
                        run_post_completion_workflow(PostCompletionWorkflowParams {
                            task_id,
                            task_repo: post_task_repo.clone(),
                            goal_repo: post_goal_repo.clone(),
                            worktree_repo: post_worktree_repo.clone(),
                            event_tx: &event_tx,
                            event_bus: &event_bus,
                            audit_log: &audit_log,
                            verify_on_completion,
                            use_merge_queue,
                            prefer_pull_requests,
                            repo_path: &repo_path,
                            default_base_ref: &default_base_ref,
                            require_commits,
                            intent_satisfied: false,
                            output_delivery: output_delivery.clone(),
                            merge_request_repo: merge_request_repo.clone(),
                            fetch_on_sync,
                            post_completion_chain: post_completion_chain.clone(),
                        })
                        .await;

                    if let Err(e) = workflow_result {
                        audit_log
                            .log(
                                AuditEntry::new(
                                    AuditLevel::Warning,
                                    AuditCategory::Task,
                                    AuditAction::TaskFailed,
                                    AuditActor::System,
                                    format!(
                                        "Post-completion workflow error for task {}: {}",
                                        task_id, e
                                    ),
                                )
                                .with_entity(task_id, "task"),
                            )
                            .await;
                    }
                }

                if track_evolution {
                    let outcome = if let Ok(Some(post_task)) = task_repo.get(task_id).await {
                        if post_task.status == TaskStatus::Failed {
                            TaskOutcome::Failure
                        } else {
                            TaskOutcome::Success
                        }
                    } else {
                        TaskOutcome::Success
                    };
                    let execution = TaskExecution {
                        task_id,
                        template_name: agent_type_for_evolution.clone(),
                        template_version: template_version_for_evolution,
                        outcome,
                        executed_at: chrono::Utc::now(),
                        turns_used: turns,
                        tokens_used: tokens,
                        downstream_tasks: vec![],
                    };
                    evolution_loop.record_execution(execution).await;
                }
            }
            Ok(session) => {
                let tokens = session.total_tokens();
                let turns = session.turns_completed;
                total_tokens.fetch_add(tokens, Ordering::Relaxed);

                let error_msg = session
                    .error
                    .clone()
                    .unwrap_or_else(|| "Unknown error".to_string());

                let auto_completed = if completed_task.status.is_terminal() {
                    tracing::warn!(
                        task_id = %task_id,
                        status = ?completed_task.status,
                        error = %error_msg,
                        "Skipping task failure — already terminal (completed via MCP)"
                    );
                    replay_gate_rejection_event(&completed_task, &event_bus, &all_workflows).await;
                    false
                } else if is_max_turns_auto_completable(&error_msg) {
                    if !can_safely_auto_complete(&completed_task) {
                        tracing::warn!(
                            task_id = %task_id,
                            error = %error_msg,
                            "Overmind exhausted turns mid-workflow — failing instead of auto-completing"
                        );
                        false
                    } else {
                        tracing::warn!(
                            task_id = %task_id,
                            error = %error_msg,
                            "Auto-completing task — agent exhausted turns but reported completion"
                        );
                        let target_status = if verify_on_completion {
                            TaskStatus::Validating
                        } else {
                            TaskStatus::Complete
                        };
                        if let Some(ref cb) = command_bus {
                            let envelope = CommandEnvelope::new(
                                CommandSource::System,
                                DomainCommand::Task(TaskCommand::Transition {
                                    task_id,
                                    new_status: target_status,
                                }),
                            );
                            if let Err(e) = cb.dispatch(envelope).await {
                                tracing::warn!(
                                    "Failed to auto-complete task {} via CommandBus, using non-atomic fallback: {}",
                                    task_id, e
                                );
                                if let Ok(Some(mut t)) = task_repo.get(task_id).await
                                    && !t.status.is_terminal()
                                {
                                    let _ = t.transition_to(target_status);
                                    let _ = task_repo.update(&t).await;
                                }
                                if target_status == TaskStatus::Complete {
                                    event_bus
                                        .publish(crate::services::event_factory::task_event(
                                            crate::services::event_bus::EventSeverity::Info,
                                            None,
                                            task_id,
                                            crate::services::event_bus::EventPayload::TaskCompleted {
                                                task_id,
                                                tokens_used: tokens,
                                            },
                                        ))
                                        .await;
                                }
                            }
                        } else {
                            tracing::warn!(
                                "CommandBus not available for task {} auto-completion, using non-atomic fallback",
                                task_id
                            );
                            let _ = completed_task.transition_to(target_status);
                            let _ = task_repo.update(&completed_task).await;
                            if target_status == TaskStatus::Complete {
                                event_bus
                                    .publish(crate::services::event_factory::task_event(
                                        crate::services::event_bus::EventSeverity::Info,
                                        None,
                                        task_id,
                                        crate::services::event_bus::EventPayload::TaskCompleted {
                                            task_id,
                                            tokens_used: tokens,
                                        },
                                    ))
                                    .await;
                            }
                        }
                        true
                    }
                } else {
                    if let Some(ref cb) = command_bus {
                        let envelope = CommandEnvelope::new(
                            CommandSource::System,
                            DomainCommand::Task(TaskCommand::Fail {
                                task_id,
                                error: Some(error_msg.clone()),
                            }),
                        );
                        if let Err(e) = cb.dispatch(envelope).await {
                            tracing::warn!(
                                "Failed to fail task {} via CommandBus, using non-atomic fallback: {}",
                                task_id, e
                            );
                            if let Ok(Some(mut t)) = task_repo.get(task_id).await
                                && !t.status.is_terminal()
                            {
                                let _ = t.transition_to(TaskStatus::Failed);
                                let _ = task_repo.update(&t).await;
                            }
                            event_bus
                                .publish(crate::services::event_factory::task_event(
                                    crate::services::event_bus::EventSeverity::Warning,
                                    None,
                                    task_id,
                                    crate::services::event_bus::EventPayload::TaskFailed {
                                        task_id,
                                        error: error_msg.clone(),
                                        retry_count: completed_task.retry_count,
                                    },
                                ))
                                .await;
                        }
                    } else {
                        tracing::warn!(
                            "CommandBus not available for task {} failure, using non-atomic fallback",
                            task_id
                        );
                        let _ = completed_task.transition_to(TaskStatus::Failed);
                        let _ = task_repo.update(&completed_task).await;
                        event_bus
                            .publish(crate::services::event_factory::task_event(
                                crate::services::event_bus::EventSeverity::Warning,
                                None,
                                task_id,
                                crate::services::event_bus::EventPayload::TaskFailed {
                                    task_id,
                                    error: error_msg.clone(),
                                    retry_count: completed_task.retry_count,
                                },
                            ))
                            .await;
                    }
                    false
                };

                if !auto_completed {
                    circuit_breaker
                        .record_failure(circuit_scope.clone(), &error_msg)
                        .await;
                }

                if track_evolution {
                    let execution = TaskExecution {
                        task_id,
                        template_name: agent_type_for_evolution.clone(),
                        template_version: template_version_for_evolution,
                        outcome: if auto_completed {
                            TaskOutcome::Success
                        } else {
                            TaskOutcome::Failure
                        },
                        executed_at: chrono::Utc::now(),
                        turns_used: turns,
                        tokens_used: tokens,
                        downstream_tasks: vec![],
                    };
                    evolution_loop.record_execution(execution).await;
                }

                if auto_completed {
                    audit_log
                        .log(
                            AuditEntry::new(
                                AuditLevel::Warning,
                                AuditCategory::Task,
                                AuditAction::TaskCompleted,
                                AuditActor::System,
                                format!(
                                    "Task auto-completed (max_turns with completion signal): {}",
                                    error_msg,
                                ),
                            )
                            .with_entity(task_id, "task"),
                        )
                        .await;

                    if worktree_path.is_some()
                        && let Ok(Some(mut wt)) = worktree_repo.get_by_task(task_id).await
                    {
                        wt.complete();
                        let _ = worktree_repo.update(&wt).await;
                    }
                } else {
                    let consecutive_budget = completed_task
                        .context
                        .custom
                        .get("consecutive_budget_failures")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0);
                    audit_log
                        .log(
                            AuditEntry::new(
                                AuditLevel::Warning,
                                AuditCategory::Task,
                                AuditAction::TaskFailed,
                                AuditActor::System,
                                format!(
                                    "Task failed: {} (retry {}/{}, consecutive_budget_failures: {})",
                                    error_msg,
                                    completed_task.retry_count,
                                    completed_task.max_retries,
                                    consecutive_budget,
                                ),
                            )
                            .with_entity(task_id, "task"),
                        )
                        .await;

                    if worktree_path.is_some()
                        && let Ok(Some(mut wt)) = worktree_repo.get_by_task(task_id).await
                    {
                        wt.fail(error_msg.clone());
                        let _ = worktree_repo.update(&wt).await;
                    }
                }
            }
            Err(e) => {
                let error_msg = e.to_string();

                if completed_task.status.is_terminal() {
                    tracing::warn!(
                        task_id = %task_id,
                        status = ?completed_task.status,
                        error = %error_msg,
                        "Skipping task failure — already terminal (completed via MCP)"
                    );
                    replay_gate_rejection_event(&completed_task, &event_bus, &all_workflows).await;
                } else if let Some(ref cb) = command_bus {
                    let envelope = CommandEnvelope::new(
                        CommandSource::System,
                        DomainCommand::Task(TaskCommand::Fail {
                            task_id,
                            error: Some(error_msg.clone()),
                        }),
                    );
                    if let Err(e) = cb.dispatch(envelope).await {
                        tracing::warn!(
                            "Failed to fail task {} via CommandBus, using non-atomic fallback: {}",
                            task_id, e
                        );
                        if let Ok(Some(mut t)) = task_repo.get(task_id).await
                            && !t.status.is_terminal()
                        {
                            let _ = t.transition_to(TaskStatus::Failed);
                            let _ = task_repo.update(&t).await;
                        }
                        event_bus
                            .publish(crate::services::event_factory::task_event(
                                crate::services::event_bus::EventSeverity::Warning,
                                None,
                                task_id,
                                crate::services::event_bus::EventPayload::TaskFailed {
                                    task_id,
                                    error: error_msg.clone(),
                                    retry_count: completed_task.retry_count,
                                },
                            ))
                            .await;
                    }
                } else {
                    tracing::warn!(
                        "CommandBus not available for task {} failure, using non-atomic fallback",
                        task_id
                    );
                    let _ = completed_task.transition_to(TaskStatus::Failed);
                    let _ = task_repo.update(&completed_task).await;
                    event_bus
                        .publish(crate::services::event_factory::task_event(
                            crate::services::event_bus::EventSeverity::Warning,
                            None,
                            task_id,
                            crate::services::event_bus::EventPayload::TaskFailed {
                                task_id,
                                error: error_msg.clone(),
                                retry_count: completed_task.retry_count,
                            },
                        ))
                        .await;
                }

                circuit_breaker
                    .record_failure(circuit_scope.clone(), &error_msg)
                    .await;

                if track_evolution {
                    let execution = TaskExecution {
                        task_id,
                        template_name: agent_type_for_evolution.clone(),
                        template_version: template_version_for_evolution,
                        outcome: TaskOutcome::Failure,
                        executed_at: chrono::Utc::now(),
                        turns_used: 0,
                        tokens_used: 0,
                        downstream_tasks: vec![],
                    };
                    evolution_loop.record_execution(execution).await;
                }

                audit_log
                    .log(
                        AuditEntry::new(
                            AuditLevel::Error,
                            AuditCategory::Task,
                            AuditAction::TaskFailed,
                            AuditActor::System,
                            format!("Task execution error: {}", error_msg),
                        )
                        .with_entity(task_id, "task"),
                    )
                    .await;

                if worktree_path.is_some()
                    && let Ok(Some(mut wt)) = worktree_repo.get_by_task(task_id).await
                {
                    wt.fail(error_msg.clone());
                    let _ = worktree_repo.update(&wt).await;
                }
            }
        }
    }

    guardrails.register_agent_end(&agent_unique_id).await;
}

/// Handle the `IntentGapsFound` outcome from convergent execution: store
/// structured gap context on the failing task, transition it to Failed, and
/// for standalone tasks (no parent workflow) create an explicit retry task
/// seeded with the gap descriptions.
///
/// This consolidation is the spec T10 §6 Risk 4 mitigation: gap-context
/// storage and retry-task creation live in one helper so future maintainers
/// can find the full intent-gap retry flow in a single place.
async fn handle_intent_gaps_with_retry(
    ivr: crate::domain::models::intent_verification::IntentVerificationResult,
    task_id: uuid::Uuid,
    task_repo: &Arc<dyn TaskRepository>,
    event_bus: &Arc<EventBus>,
    audit_log: &Arc<crate::services::AuditLogService>,
) {
    let total_gaps = ivr.gaps.len() + ivr.implicit_gaps.len();
    let gap_descriptions: Vec<String> = ivr
        .all_gaps()
        .map(|g| {
            let action = g.suggested_action.as_deref().unwrap_or("(no suggestion)");
            format!("- [{}] {}: {}", g.severity.as_str(), g.description, action)
        })
        .collect();
    let gap_context = format!(
        "## Intent Verification Gaps (from previous attempt)\n\
         Satisfaction: {} (confidence: {:.2})\n\
         Accomplishment: {}\n\n\
         ### Gaps to address:\n{}",
        ivr.satisfaction.as_str(),
        ivr.confidence,
        ivr.accomplishment_summary,
        gap_descriptions.join("\n"),
    );

    audit_log
        .log(
            AuditEntry::new(
                AuditLevel::Warning,
                AuditCategory::Task,
                AuditAction::TaskFailed,
                AuditActor::System,
                format!(
                    "Task {} overseer-converged but intent unsatisfied ({}, {} gaps). Failing with gap context for retry.",
                    task_id,
                    ivr.satisfaction.as_str(),
                    total_gaps,
                ),
            )
            .with_entity(task_id, "task"),
        )
        .await;

    if let Ok(Some(mut t)) = task_repo.get(task_id).await {
        t.context.custom.insert(
            "intent_gaps".to_string(),
            serde_json::json!({
                "satisfaction": ivr.satisfaction.as_str(),
                "confidence": ivr.confidence,
                "accomplishment": ivr.accomplishment_summary,
                "gaps": ivr.gaps.iter().map(|g| serde_json::json!({
                    "description": g.description,
                    "severity": g.severity.as_str(),
                    "category": g.category.as_str(),
                    "suggested_action": g.suggested_action,
                })).collect::<Vec<_>>(),
                "implicit_gaps": ivr.implicit_gaps.iter().map(|g| serde_json::json!({
                    "description": g.description,
                    "severity": g.severity.as_str(),
                    "category": g.category.as_str(),
                    "suggested_action": g.suggested_action,
                })).collect::<Vec<_>>(),
            }),
        );
        t.set_intent_gap_context(gap_context.clone());

        let is_workflow_subtask = t.is_workflow_phase_subtask();

        if !t.status.is_terminal() {
            let _ = t.transition_to(TaskStatus::Failed);
        }
        let retry_count = t.retry_count;
        let _ = task_repo.update(&t).await;

        event_bus
            .publish(crate::services::event_factory::task_event(
                crate::services::event_bus::EventSeverity::Warning,
                None,
                task_id,
                crate::services::event_bus::EventPayload::TaskFailed {
                    task_id,
                    error: format!(
                        "Intent verification found {} gap(s); {}",
                        total_gaps,
                        if is_workflow_subtask {
                            "workflow engine will retry with gap context"
                        } else {
                            "creating retry task with gap context"
                        },
                    ),
                    retry_count,
                },
            ))
            .await;

        if !is_workflow_subtask {
            let new_description = format!("{}\n\n{}", t.description, gap_context);
            let mut retry_task = Task::with_title(format!("[retry] {}", t.title), &new_description);
            retry_task.parent_id = t.parent_id;
            retry_task.task_type = t.task_type;
            retry_task.priority = t.priority;
            retry_task.source = t.source;
            retry_task.context.custom.insert(
                "intent_gaps".to_string(),
                t.context
                    .custom
                    .get("intent_gaps")
                    .cloned()
                    .unwrap_or_default(),
            );
            retry_task.context.custom.insert(
                "retry_reason".to_string(),
                serde_json::json!("intent_gaps_found"),
            );
            retry_task.context.custom.insert(
                "previous_task_id".to_string(),
                serde_json::json!(task_id.to_string()),
            );
            let _ = retry_task.transition_to(TaskStatus::Ready);

            match task_repo.create(&retry_task).await {
                Ok(_) => {
                    tracing::info!(
                        original_task_id = %task_id,
                        retry_task_id = %retry_task.id,
                        gaps = total_gaps,
                        "Created standalone retry task with intent gap context"
                    );
                    event_bus
                        .publish(crate::services::event_factory::task_event(
                            crate::services::event_bus::EventSeverity::Info,
                            None,
                            retry_task.id,
                            crate::services::event_bus::EventPayload::TaskReady {
                                task_id: retry_task.id,
                                task_title: retry_task.title.clone(),
                            },
                        ))
                        .await;
                }
                Err(e) => {
                    tracing::error!(
                        task_id = %task_id,
                        error = %e,
                        "Failed to create retry task for intent gaps"
                    );
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::sqlite::test_support;
    use crate::adapters::substrates::MockSubstrate;
    use crate::services::guardrails::Guardrails;

    /// Risk 3 mitigation test: a minimal post-completion middleware fires
    /// when the spawn block calls `run_post_completion_workflow` for a
    /// completed direct-mode task.
    #[tokio::test]
    async fn test_post_completion_chain_runs_for_completed_direct_task() {
        use std::sync::atomic::{AtomicBool, Ordering as AtomOrdering};

        struct FlagMiddleware {
            fired: Arc<AtomicBool>,
        }

        #[async_trait::async_trait]
        impl super::super::middleware::PostCompletionMiddleware for FlagMiddleware {
            fn name(&self) -> &'static str {
                "flag"
            }
            async fn handle(
                &self,
                _ctx: &mut super::super::middleware::PostCompletionContext,
            ) -> crate::domain::errors::DomainResult<()> {
                self.fired.store(true, AtomOrdering::Relaxed);
                Ok(())
            }
        }

        let fired = Arc::new(AtomicBool::new(false));
        let mut chain = PostCompletionChain::new();
        chain.register(Arc::new(FlagMiddleware {
            fired: fired.clone(),
        }));
        let chain_arc = Arc::new(RwLock::new(chain));

        // Build the minimal context needed to invoke the chain directly,
        // matching the call shape used inside execute_task.
        let (goal_repo, task_repo, worktree_repo, _agent_repo, _mem_repo) =
            test_support::setup_all_repos().await;
        let task_repo_dyn: Arc<dyn TaskRepository> = task_repo;
        let goal_repo_dyn: Arc<dyn GoalRepository> = goal_repo;
        let worktree_repo_dyn: Arc<dyn WorktreeRepository> = worktree_repo;
        let event_bus = Arc::new(EventBus::new(
            crate::services::event_bus::EventBusConfig::default(),
        ));
        let audit_log = Arc::new(crate::services::AuditLogService::with_defaults());
        let (event_tx, _event_rx) = mpsc::channel(8);

        let task_id = uuid::Uuid::new_v4();
        let _ = run_post_completion_workflow(PostCompletionWorkflowParams {
            task_id,
            task_repo: task_repo_dyn,
            goal_repo: goal_repo_dyn,
            worktree_repo: worktree_repo_dyn,
            event_tx: &event_tx,
            event_bus: &event_bus,
            audit_log: &audit_log,
            verify_on_completion: true,
            use_merge_queue: false,
            prefer_pull_requests: false,
            repo_path: std::path::Path::new("."),
            default_base_ref: "main",
            require_commits: false,
            intent_satisfied: false,
            output_delivery: OutputDelivery::PullRequest,
            merge_request_repo: None,
            fetch_on_sync: false,
            post_completion_chain: chain_arc.clone(),
        })
        .await;

        assert!(
            fired.load(AtomOrdering::Relaxed),
            "post-completion middleware must fire for verify_on_completion=true paths"
        );

        // Touch substrate import.
        let _: Arc<dyn Substrate> = Arc::new(MockSubstrate::new());
        let _g = Guardrails::with_defaults();
    }
}
