//! Convergent execution path for the swarm orchestrator.
//!
//! When a task has `ExecutionMode::Convergent`, the orchestrator enters a
//! convergence loop instead of a single-shot substrate invocation. The loop
//! uses the convergence engine's granular primitives with substrate execution
//! injected between strategy selection and overseer measurement.
//!
//! # Design
//!
//! The orchestrator owns the outer loop; the engine owns the inner logic.
//! The engine's granular primitives (`iterate_once`, `select_strategy`,
//! `initialize_bandit`, `finalize`) are used because the orchestrator must
//! inject substrate execution between strategy selection and overseer
//! measurement.
//!
//! The flow per iteration is:
//! 1. Check cancellation token
//! 2. Select strategy (bandit + eligibility filter)
//! 3. If FreshStart, reset worktree to base branch state
//! 4. Build prompt (bridge: task + trajectory + strategy -> prompt string)
//! 5. Execute substrate (agent runtime produces artifact)
//! 6. Collect artifact reference from worktree
//! 7. Measure with overseers via engine
//! 8. Record observation, classify attractor, update bandit via `iterate_once`
//! 9. Act on `LoopControl` to continue, converge, decompose, etc.

use std::sync::Arc;
use std::time::Instant;

use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::domain::errors::{DomainError, DomainResult};
use crate::domain::models::convergence::*;
use crate::domain::models::task::Task;
use crate::domain::models::{SubstrateConfig, SubstrateRequest};
use crate::domain::ports::{MemoryRepository, Substrate, TaskRepository, TrajectoryRepository};
use crate::services::convergence_bridge;
use crate::services::convergence_engine::{ConvergenceEngine, OverseerMeasurer};
use crate::services::event_bus::{EventBus, EventPayload, EventSeverity};
use crate::services::event_factory;

// ---------------------------------------------------------------------------
// ConvergentOutcome
// ---------------------------------------------------------------------------

/// Outcome of convergent execution, consumed by the orchestrator to decide
/// the task's terminal status.
#[derive(Debug)]
pub enum ConvergentOutcome {
    /// The trajectory converged to a satisfactory result.
    Converged,
    /// Budget exhausted, but the best observation was above the partial
    /// acceptance threshold. The orchestrator should treat this as success.
    PartialAccepted,
    /// The engine determined the task should be decomposed into subtasks.
    /// The orchestrator creates child tasks from the trajectory's
    /// decomposition plan.
    Decomposed(Trajectory),
    /// Convergence failed. The message describes the terminal condition
    /// (trapped, exhausted, budget denied, etc.).
    Failed(String),
    /// The convergence loop was cancelled via the cancellation token.
    /// The trajectory has been persisted in its current state. The caller
    /// handles the task status transition (typically to Canceled).
    Cancelled,
}

// ---------------------------------------------------------------------------
// run_convergent_execution
// ---------------------------------------------------------------------------

/// Run convergent execution for a task.
///
/// This replaces the single-shot `substrate.execute()` with an iterative
/// convergence loop guided by the convergence engine. The orchestrator owns
/// the outer loop and injects substrate invocations between the engine's
/// strategy selection and observation recording.
///
/// # Arguments
///
/// * `task` - The task being executed (must have `ExecutionMode::Convergent`).
/// * `goal_id` - Optional parent goal for event correlation.
/// * `substrate` - The agent runtime substrate (Claude Code CLI, etc.).
/// * `task_repo` - Task repository for persisting trajectory linkage.
/// * `trajectory_store` - Trajectory repository for loading/persisting trajectories.
/// * `engine` - The convergence engine providing primitives.
/// * `event_bus` - For emitting convergence lifecycle events.
/// * `agent_type` - The agent template name (e.g. "coder", "overmind").
/// * `system_prompt` - The agent's system prompt.
/// * `worktree_path` - Optional worktree path for task isolation.
/// * `max_turns` - Maximum turns per substrate invocation.
/// * `cancellation_token` - Token checked at the top of each iteration;
///   when cancelled, the trajectory is persisted and `Cancelled` is returned.
/// * `deadline` - Optional SLA deadline; caps the trajectory budget's
///   `max_wall_time` so convergent tasks never breach the SLA.
pub async fn run_convergent_execution<T, Tr, M, O>(
    task: &Task,
    goal_id: Option<Uuid>,
    substrate: &Arc<dyn Substrate>,
    task_repo: &Arc<T>,
    trajectory_store: &Arc<Tr>,
    engine: &ConvergenceEngine<Tr, M, O>,
    event_bus: &Arc<EventBus>,
    agent_type: &str,
    system_prompt: &str,
    worktree_path: Option<&str>,
    max_turns: u32,
    cancellation_token: CancellationToken,
    deadline: Option<chrono::DateTime<chrono::Utc>>,
) -> DomainResult<ConvergentOutcome>
where
    T: TaskRepository + 'static,
    Tr: TrajectoryRepository + 'static,
    M: MemoryRepository + 'static,
    O: OverseerMeasurer + 'static,
{
    // -----------------------------------------------------------------------
    // 1. PREPARE -- Create or resume a trajectory (Part 4.2)
    // -----------------------------------------------------------------------

    let (mut trajectory, _infrastructure, mut bandit) = if let Some(tid) = task.trajectory_id {
        // Resume existing trajectory on retry. The trajectory already
        // contains the full history of observations, attractor state, and
        // bandit learning. The retry continues from where it left off.
        let loaded = trajectory_store
            .get(&tid.to_string())
            .await?
            .ok_or_else(|| {
                DomainError::ExecutionFailed(format!(
                    "trajectory {} referenced by task {} not found",
                    tid, task.id
                ))
            })?;

        let bandit = engine.initialize_bandit(&loaded).await;
        (loaded, None, bandit)
    } else {
        // First run: prepare from scratch.
        let submission = convergence_bridge::task_to_submission(task, goal_id);
        let (trajectory, infrastructure) = engine.prepare(&submission).await?;

        // Link the trajectory back to the task for observability and retry
        if let Ok(Some(mut t)) = task_repo.get(task.id).await {
            t.trajectory_id = Some(trajectory.id);
            let _ = task_repo.update(&t).await;
        }

        let bandit = engine.initialize_bandit(&trajectory).await;
        (trajectory, Some(infrastructure), bandit)
    };

    // -----------------------------------------------------------------------
    // 1b. SLA Deadline -> Budget Ceiling (Part 8.1)
    // -----------------------------------------------------------------------

    if let Some(deadline) = deadline {
        let remaining = deadline - chrono::Utc::now();
        if remaining.num_seconds() > 0 {
            let cap = std::time::Duration::from_secs(remaining.num_seconds() as u64);
            trajectory.budget.max_wall_time = trajectory.budget.max_wall_time.min(cap);
        }
    }

    // Emit ConvergenceStarted event
    let estimated_iterations = trajectory.budget.max_iterations;
    event_bus.publish(event_factory::make_event(
        EventSeverity::Info,
        crate::services::event_bus::EventCategory::Convergence,
        goal_id,
        Some(task.id),
        EventPayload::ConvergenceStarted {
            task_id: task.id,
            trajectory_id: trajectory.id,
            estimated_iterations,
            basin_width: "standard".to_string(),
            convergence_mode: "sequential".to_string(),
        },
    )).await;

    // Transition to iterating phase
    trajectory.phase = ConvergencePhase::Iterating;

    // Part 7.1: Track previous attractor classification for transition detection
    let mut prev_attractor = trajectory.attractor_state.classification.clone();

    // -----------------------------------------------------------------------
    // 2. ITERATE -- Main convergence loop
    // -----------------------------------------------------------------------

    loop {
        // 2a. Check cancellation token (Part 4.4)
        //
        // Checked at the top of each iteration. If cancelled, persist the
        // trajectory in its current state and return Cancelled. The caller
        // handles the task status transition.
        if cancellation_token.is_cancelled() {
            trajectory_store.save(&trajectory).await?;
            emit_convergence_terminated(
                event_bus, task, goal_id, &trajectory, "cancelled",
            ).await;
            return Ok(ConvergentOutcome::Cancelled);
        }

        // 2a'. SLA pressure consumption (Part 8.2)
        //
        // The ConvergenceSLAPressureHandler adds "sla:warning" or "sla:critical"
        // hints to the persisted task. Re-read the task to check for these hints
        // and adjust the convergence policy accordingly.
        if let Ok(Some(current_task)) = task_repo.get(task.id).await {
            apply_sla_pressure(&current_task.context.hints, &mut trajectory.policy);
        }

        // 2b. Select strategy
        //
        // Use forced strategy if set (e.g. fresh start from context degradation),
        // otherwise run the eligibility filter + bandit selection.
        let strategy = if let Some(forced) = trajectory.forced_strategy.take() {
            forced
        } else {
            let eligible = eligible_strategies(
                &trajectory.strategy_log,
                &trajectory.attractor_state,
                &trajectory.budget,
                trajectory.total_fresh_starts,
                trajectory.policy.max_fresh_starts,
            );
            if eligible.is_empty() {
                // No strategies available -- trapped
                let outcome = ConvergenceOutcome::Trapped {
                    trajectory_id: trajectory.id.to_string(),
                    attractor_type: trajectory.attractor_state.classification.clone(),
                };
                engine.finalize(&mut trajectory, &outcome, &bandit).await?;
                emit_convergence_terminated(
                    event_bus, task, goal_id, &trajectory, "trapped",
                ).await;
                return Ok(ConvergentOutcome::Failed(format!(
                    "trapped in {:?} attractor -- no eligible escape strategies",
                    trajectory.attractor_state.classification
                )));
            }
            bandit.select(
                &trajectory.attractor_state.classification,
                &eligible,
                &trajectory.policy,
            )
        };

        // Part 7.1: Strategy escalation check
        // ArchitectReview and Decompose are high-impact strategies that
        // may warrant human oversight before execution.
        if matches!(&strategy, StrategyKind::ArchitectReview | StrategyKind::Decompose) {
            event_bus.publish(event_factory::make_event(
                EventSeverity::Warning,
                crate::services::event_bus::EventCategory::Convergence,
                goal_id,
                Some(task.id),
                EventPayload::HumanEscalationNeeded {
                    goal_id,
                    task_id: Some(task.id),
                    reason: format!(
                        "Convergence engine selected {} strategy for task {}",
                        strategy.kind_name(), task.id
                    ),
                    urgency: "medium".to_string(),
                    is_blocking: false,
                },
            )).await;
        }

        // 2c. FreshStart worktree reset (Part 3.7)
        //
        // When the selected strategy is FreshStart, reset the worktree to
        // the base branch state before substrate invocation. This provides
        // a clean slate while preserving the worktree allocation. The
        // carry-forward context in the prompt provides the agent with
        // learnings from previous attempts without polluting the working tree.
        if matches!(strategy, StrategyKind::FreshStart { .. }) {
            trajectory.total_fresh_starts += 1;
            if let Some(wt) = worktree_path {
                reset_worktree(wt).await?;
            }
            event_bus.publish(event_factory::make_event(
                EventSeverity::Info,
                crate::services::event_bus::EventCategory::Convergence,
                goal_id,
                Some(task.id),
                EventPayload::ConvergenceFreshStart {
                    task_id: task.id,
                    trajectory_id: trajectory.id,
                    fresh_start_number: trajectory.total_fresh_starts,
                    reason: "FreshStart strategy selected".to_string(),
                },
            )).await;
        }

        // 2d. Build a convergent prompt from task + trajectory + strategy
        let prompt = convergence_bridge::build_convergent_prompt(task, &trajectory, &strategy);

        // 2e. Execute one substrate invocation with wall-time tracking
        let mut config = SubstrateConfig::default().with_max_turns(max_turns);
        if let Some(wt) = worktree_path {
            config = config.with_working_dir(wt);
        }
        let request = SubstrateRequest::new(
            task.id,
            agent_type,
            system_prompt,
            &prompt,
        ).with_config(config);

        let iteration_start = Instant::now();
        let session = substrate.execute(request).await?;
        let wall_time_ms = iteration_start.elapsed().as_millis() as u64;

        // 2f. Collect artifact reference from the worktree
        let artifact = convergence_bridge::collect_artifact(
            worktree_path.unwrap_or("."),
            "", // content hash will be computed by overseers if needed
        );

        // 2g. Build the observation
        let tokens_used = session.total_tokens();
        let sequence = trajectory.observations.len() as u32;

        let observation = Observation::new(
            sequence,
            artifact,
            OverseerSignals::default(), // overseers run inside iterate_once via engine
            strategy.clone(),
            tokens_used,
            wall_time_ms,
        );

        // 2h. Delegate to the engine's iterate_once: computes metrics,
        // classifies attractor, updates bandit, persists trajectory,
        // and returns loop control.
        let loop_control = engine
            .iterate_once(&mut trajectory, &mut bandit, &strategy, observation)
            .await?;

        // Part 7.1: Attractor transition detection
        let current_attractor = &trajectory.attractor_state.classification;
        if attractor_type_label(current_attractor) != attractor_type_label(&prev_attractor) {
            event_bus.publish(event_factory::make_event(
                EventSeverity::Info,
                crate::services::event_bus::EventCategory::Convergence,
                goal_id,
                Some(task.id),
                EventPayload::ConvergenceAttractorTransition {
                    task_id: task.id,
                    trajectory_id: trajectory.id,
                    from: attractor_type_label(&prev_attractor).to_string(),
                    to: attractor_type_label(current_attractor).to_string(),
                    confidence: trajectory.attractor_state.confidence,
                },
            )).await;
            prev_attractor = current_attractor.clone();
        }

        // 2i. Emit iteration event
        let (delta, level) = trajectory
            .observations
            .last()
            .and_then(|o| o.metrics.as_ref())
            .map(|m| (m.convergence_delta, m.convergence_level))
            .unwrap_or((0.0, 0.0));

        event_bus.publish(event_factory::make_event(
            EventSeverity::Info,
            crate::services::event_bus::EventCategory::Convergence,
            goal_id,
            Some(task.id),
            EventPayload::ConvergenceIteration {
                task_id: task.id,
                trajectory_id: trajectory.id,
                iteration: sequence,
                strategy: strategy.kind_name().to_string(),
                convergence_delta: delta,
                convergence_level: level,
                attractor_type: attractor_type_label(
                    &trajectory.attractor_state.classification,
                ).to_string(),
                budget_remaining_fraction: trajectory.budget.remaining_fraction(),
            },
        )).await;

        // 2j. Act on loop control
        match loop_control {
            LoopControl::Continue => {
                // Keep iterating
                continue;
            }
            LoopControl::Converged => {
                let final_seq = trajectory
                    .observations
                    .last()
                    .map(|o| o.sequence)
                    .unwrap_or(0);
                let outcome = ConvergenceOutcome::Converged {
                    trajectory_id: trajectory.id.to_string(),
                    final_observation_sequence: final_seq,
                };
                engine.finalize(&mut trajectory, &outcome, &bandit).await?;
                emit_convergence_terminated(
                    event_bus, task, goal_id, &trajectory, "converged",
                ).await;
                return Ok(ConvergentOutcome::Converged);
            }
            LoopControl::Exhausted => {
                // Check partial acceptance policy
                let accept_partial = if trajectory.policy.partial_acceptance {
                    trajectory
                        .best_observation()
                        .and_then(|o| o.metrics.as_ref())
                        .map(|m| m.convergence_level >= trajectory.policy.partial_threshold)
                        .unwrap_or(false)
                } else {
                    false
                };

                let best_seq = trajectory.best_observation().map(|o| o.sequence);
                let outcome = if accept_partial {
                    ConvergenceOutcome::Converged {
                        trajectory_id: trajectory.id.to_string(),
                        final_observation_sequence: best_seq.unwrap_or(0),
                    }
                } else {
                    ConvergenceOutcome::Exhausted {
                        trajectory_id: trajectory.id.to_string(),
                        best_observation_sequence: best_seq,
                    }
                };
                engine.finalize(&mut trajectory, &outcome, &bandit).await?;
                emit_convergence_terminated(
                    event_bus, task, goal_id, &trajectory, "exhausted",
                ).await;
                return Ok(if accept_partial {
                    ConvergentOutcome::PartialAccepted
                } else {
                    ConvergentOutcome::Failed(
                        "convergence budget exhausted without reaching acceptance threshold"
                            .to_string(),
                    )
                });
            }
            LoopControl::Trapped => {
                let outcome = ConvergenceOutcome::Trapped {
                    trajectory_id: trajectory.id.to_string(),
                    attractor_type: trajectory.attractor_state.classification.clone(),
                };
                engine.finalize(&mut trajectory, &outcome, &bandit).await?;
                emit_convergence_terminated(
                    event_bus, task, goal_id, &trajectory, "trapped",
                ).await;
                return Ok(ConvergentOutcome::Failed(format!(
                    "trapped in {} attractor",
                    attractor_type_label(&trajectory.attractor_state.classification),
                )));
            }
            LoopControl::Decompose => {
                emit_convergence_terminated(
                    event_bus, task, goal_id, &trajectory, "decomposed",
                ).await;
                return Ok(ConvergentOutcome::Decomposed(trajectory));
            }
            LoopControl::RequestExtension => {
                let additional_iterations = 3u32;
                let additional_tokens = (trajectory.budget.max_tokens as f64 * 0.3) as u64;

                if engine.request_extension(&mut trajectory).await? {
                    // Extension granted -- emit event and continue iterating
                    event_bus.publish(event_factory::make_event(
                        EventSeverity::Info,
                        crate::services::event_bus::EventCategory::Convergence,
                        goal_id,
                        Some(task.id),
                        EventPayload::ConvergenceBudgetExtension {
                            task_id: task.id,
                            trajectory_id: trajectory.id,
                            granted: true,
                            additional_iterations,
                            additional_tokens,
                        },
                    )).await;
                    continue;
                } else {
                    event_bus.publish(event_factory::make_event(
                        EventSeverity::Warning,
                        crate::services::event_bus::EventCategory::Convergence,
                        goal_id,
                        Some(task.id),
                        EventPayload::ConvergenceBudgetExtension {
                            task_id: task.id,
                            trajectory_id: trajectory.id,
                            granted: false,
                            additional_iterations: 0,
                            additional_tokens: 0,
                        },
                    )).await;

                    let outcome = ConvergenceOutcome::BudgetDenied {
                        trajectory_id: trajectory.id.to_string(),
                    };
                    engine.finalize(&mut trajectory, &outcome, &bandit).await?;
                    emit_convergence_terminated(
                        event_bus, task, goal_id, &trajectory, "budget_denied",
                    ).await;
                    return Ok(ConvergentOutcome::Failed(
                        "budget extension denied".to_string(),
                    ));
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// run_parallel_convergent_execution (Parts 11.1-11.3)
// ---------------------------------------------------------------------------

/// Run parallel convergent execution for a task.
///
/// Parallel mode spawns N independent substrate invocations concurrently,
/// each with a different strategy and its own worktree, then selects the
/// best trajectory and continues sequential iteration on the winner.
///
/// This is most effective for narrow-basin tasks where the correct approach
/// is hard to find but easy to verify once found. The budget is partitioned:
/// Phase 1 consumes N iterations worth of budget (one per parallel sample),
/// Phase 2 uses the remaining budget for sequential iteration on the winner.
///
/// # Arguments
///
/// Same as `run_convergent_execution`, plus:
/// * `parallel_samples` - Number of parallel trajectories to spawn in Phase 1.
/// * `base_branch` - The git branch to create worktrees from.
/// * `worktree_base_dir` - Base directory under which parallel worktrees are created.
pub async fn run_parallel_convergent_execution<T, Tr, M, O>(
    task: &Task,
    goal_id: Option<Uuid>,
    substrate: &Arc<dyn Substrate>,
    task_repo: &Arc<T>,
    trajectory_store: &Arc<Tr>,
    engine: &ConvergenceEngine<Tr, M, O>,
    event_bus: &Arc<EventBus>,
    agent_type: &str,
    system_prompt: &str,
    max_turns: u32,
    cancellation_token: CancellationToken,
    deadline: Option<chrono::DateTime<chrono::Utc>>,
    parallel_samples: u32,
    base_branch: &str,
    worktree_base_dir: &str,
) -> DomainResult<ConvergentOutcome>
where
    T: TaskRepository + 'static,
    Tr: TrajectoryRepository + 'static,
    M: MemoryRepository + 'static,
    O: OverseerMeasurer + 'static,
{
    let n = parallel_samples.max(1) as usize;

    // -----------------------------------------------------------------------
    // 1. PREPARE -- Create base trajectory and budget partitioning
    // -----------------------------------------------------------------------

    let submission = convergence_bridge::task_to_submission(task, goal_id);
    let (mut base_trajectory, _infrastructure) = engine.prepare(&submission).await?;

    // Apply SLA deadline cap (Part 8.1)
    if let Some(deadline) = deadline {
        let remaining = deadline - chrono::Utc::now();
        if remaining.num_seconds() > 0 {
            let cap = std::time::Duration::from_secs(remaining.num_seconds() as u64);
            base_trajectory.budget.max_wall_time = base_trajectory.budget.max_wall_time.min(cap);
        }
    }

    // Link trajectory to task
    if task.trajectory_id.is_none() {
        if let Ok(Some(mut t)) = task_repo.get(task.id).await {
            t.trajectory_id = Some(base_trajectory.id);
            let _ = task_repo.update(&t).await;
        }
    }

    // Emit ConvergenceStarted event
    event_bus.publish(event_factory::make_event(
        EventSeverity::Info,
        crate::services::event_bus::EventCategory::Convergence,
        goal_id,
        Some(task.id),
        EventPayload::ConvergenceStarted {
            task_id: task.id,
            trajectory_id: base_trajectory.id,
            estimated_iterations: base_trajectory.budget.max_iterations,
            basin_width: "standard".to_string(),
            convergence_mode: format!("parallel({})", n),
        },
    )).await;

    // -----------------------------------------------------------------------
    // 2. PHASE 1 -- Create N worktrees and spawn N concurrent invocations
    // -----------------------------------------------------------------------

    // Create parallel worktrees
    let mut worktree_paths: Vec<String> = Vec::with_capacity(n);
    for i in 0..n {
        let wt_path = format!("{}/parallel_{}", worktree_base_dir, i);
        create_worktree(&wt_path, base_branch).await?;
        worktree_paths.push(wt_path);
    }

    // Create N sample trajectories with partitioned budgets.
    // Phase 1 gets 1 iteration per sample; Phase 2 gets the rest.
    let mut sample_trajectories: Vec<Trajectory> = Vec::with_capacity(n);
    let mut sample_bandits: Vec<StrategyBandit> = Vec::with_capacity(n);

    for _ in 0..n {
        let mut sample = base_trajectory.clone();
        sample.id = Uuid::new_v4();
        // Each sample gets 1 iteration's worth of budget in Phase 1
        sample.budget = base_trajectory.budget.scale(1.0 / n as f64);
        sample.phase = ConvergencePhase::Iterating;

        let bandit = engine.initialize_bandit(&sample).await;
        sample_trajectories.push(sample);
        sample_bandits.push(bandit);
    }

    // Spawn N concurrent substrate invocations with different strategies
    let mut handles = Vec::with_capacity(n);

    for i in 0..n {
        let substrate = Arc::clone(substrate);
        let task_id = task.id;
        let agent_type = agent_type.to_string();
        let system_prompt = system_prompt.to_string();
        let wt_path = worktree_paths[i].clone();
        let cancellation_token = cancellation_token.clone();

        // Select a strategy for this sample
        let eligible = eligible_strategies(
            &sample_trajectories[i].strategy_log,
            &sample_trajectories[i].attractor_state,
            &sample_trajectories[i].budget,
            sample_trajectories[i].total_fresh_starts,
            sample_trajectories[i].policy.max_fresh_starts,
        );
        let strategy = if eligible.is_empty() {
            StrategyKind::RetryWithFeedback // fallback
        } else {
            sample_bandits[i].select(
                &sample_trajectories[i].attractor_state.classification,
                &eligible,
                &sample_trajectories[i].policy,
            )
        };

        let prompt = convergence_bridge::build_convergent_prompt(
            task,
            &sample_trajectories[i],
            &strategy,
        );

        let strategy_clone = strategy.clone();

        handles.push(tokio::spawn(async move {
            if cancellation_token.is_cancelled() {
                return Err(DomainError::ExecutionFailed("cancelled".to_string()));
            }

            let config = SubstrateConfig::default()
                .with_max_turns(max_turns)
                .with_working_dir(&wt_path);
            let request = SubstrateRequest::new(
                task_id,
                &agent_type,
                &system_prompt,
                &prompt,
            ).with_config(config);

            let iteration_start = Instant::now();
            let session = substrate.execute(request).await?;
            let wall_time_ms = iteration_start.elapsed().as_millis() as u64;

            let artifact = convergence_bridge::collect_artifact(&wt_path, "");
            let tokens_used = session.total_tokens();

            Ok((strategy_clone, artifact, tokens_used, wall_time_ms))
        }));
    }

    // Collect results from all parallel invocations
    let mut results: Vec<Option<(usize, StrategyKind, ArtifactReference, u64, u64)>> =
        Vec::with_capacity(n);
    for (i, handle) in handles.into_iter().enumerate() {
        match handle.await {
            Ok(Ok((strategy, artifact, tokens, wall_time))) => {
                results.push(Some((i, strategy, artifact, tokens, wall_time)));
            }
            Ok(Err(e)) => {
                tracing::warn!(
                    sample = i,
                    error = %e,
                    "Parallel sample {} failed",
                    i
                );
                results.push(None);
            }
            Err(e) => {
                tracing::warn!(
                    sample = i,
                    error = %e,
                    "Parallel sample {} panicked",
                    i
                );
                results.push(None);
            }
        }
    }

    // -----------------------------------------------------------------------
    // 3. PHASE 1 -- Measure each artifact and record observations
    // -----------------------------------------------------------------------

    for result in &results {
        if let Some((idx, strategy, artifact, tokens_used, wall_time_ms)) = result {
            let sequence = sample_trajectories[*idx].observations.len() as u32;
            let observation = Observation::new(
                sequence,
                artifact.clone(),
                OverseerSignals::default(),
                strategy.clone(),
                *tokens_used,
                *wall_time_ms,
            );

            let _ = engine
                .iterate_once(
                    &mut sample_trajectories[*idx],
                    &mut sample_bandits[*idx],
                    strategy,
                    observation,
                )
                .await;
        }
    }

    // -----------------------------------------------------------------------
    // 4. PHASE 2 -- Select the best trajectory via convergence level
    // -----------------------------------------------------------------------

    let winner_idx = sample_trajectories
        .iter()
        .enumerate()
        .filter(|(_, t)| !t.observations.is_empty())
        .max_by(|(_, a), (_, b)| {
            let a_level = a
                .best_observation()
                .and_then(|o| o.metrics.as_ref())
                .map(|m| m.convergence_level)
                .unwrap_or(0.0);
            let b_level = b
                .best_observation()
                .and_then(|o| o.metrics.as_ref())
                .map(|m| m.convergence_level)
                .unwrap_or(0.0);
            a_level.partial_cmp(&b_level).unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|(idx, _)| idx);

    let winner_idx = match winner_idx {
        Some(idx) => idx,
        None => {
            // All parallel samples failed -- clean up worktrees
            for wt_path in &worktree_paths {
                let _ = destroy_worktree(wt_path).await;
            }
            let outcome = ConvergenceOutcome::Exhausted {
                trajectory_id: base_trajectory.id.to_string(),
                best_observation_sequence: None,
            };
            engine.finalize(&mut base_trajectory, &outcome, &sample_bandits.first().unwrap_or(&StrategyBandit::with_default_priors())).await?;
            emit_convergence_terminated(
                event_bus, task, goal_id, &base_trajectory, "exhausted",
            ).await;
            return Ok(ConvergentOutcome::Failed(
                "all parallel samples failed".to_string(),
            ));
        }
    };

    // -----------------------------------------------------------------------
    // 5. Destroy losing worktrees, keep the winner
    // -----------------------------------------------------------------------

    let winner_worktree = worktree_paths[winner_idx].clone();
    for (i, wt_path) in worktree_paths.iter().enumerate() {
        if i != winner_idx {
            let _ = destroy_worktree(wt_path).await;
        }
    }

    // Transfer the winning trajectory's state: promote the winning sample
    // to the base trajectory and grant it the remaining budget.
    let mut trajectory = sample_trajectories.swap_remove(winner_idx);
    let bandit = sample_bandits.swap_remove(winner_idx);

    // Grant remaining budget for Phase 2: the base budget minus what
    // Phase 1 consumed. Phase 1 used N iterations' worth of budget,
    // so Phase 2 gets the rest.
    let phase1_tokens: u64 = trajectory.budget.tokens_used;
    let remaining_token_budget = base_trajectory.budget.max_tokens.saturating_sub(phase1_tokens);
    let remaining_iterations = base_trajectory
        .budget
        .max_iterations
        .saturating_sub(n as u32);

    trajectory.budget.max_tokens = trajectory.budget.tokens_used + remaining_token_budget;
    trajectory.budget.max_iterations = trajectory.budget.iterations_used + remaining_iterations.max(1);
    trajectory.budget.max_wall_time = base_trajectory.budget.max_wall_time;

    // -----------------------------------------------------------------------
    // 6. PHASE 2 -- Continue sequential iteration on the winner
    // -----------------------------------------------------------------------

    // Delegate to the standard sequential loop for the remaining budget.
    // We pass the winning worktree path and let the sequential loop run.
    run_convergent_execution_inner(
        task,
        goal_id,
        substrate,
        task_repo,
        trajectory_store,
        engine,
        event_bus,
        agent_type,
        system_prompt,
        Some(&winner_worktree),
        max_turns,
        cancellation_token,
        trajectory,
        bandit,
    )
    .await
}

// ---------------------------------------------------------------------------
// run_convergent_execution_inner (shared sequential loop)
// ---------------------------------------------------------------------------

/// Inner sequential convergence loop, used by both the standard sequential
/// path (after Phase 1 in parallel mode) and the main `run_convergent_execution`.
///
/// This function takes an already-prepared trajectory and bandit and runs the
/// convergence loop until a terminal condition is reached.
async fn run_convergent_execution_inner<T2, Tr, M, O>(
    task: &Task,
    goal_id: Option<Uuid>,
    substrate: &Arc<dyn Substrate>,
    task_repo: &Arc<T2>,
    trajectory_store: &Arc<Tr>,
    engine: &ConvergenceEngine<Tr, M, O>,
    event_bus: &Arc<EventBus>,
    agent_type: &str,
    system_prompt: &str,
    worktree_path: Option<&str>,
    max_turns: u32,
    cancellation_token: CancellationToken,
    mut trajectory: Trajectory,
    mut bandit: StrategyBandit,
) -> DomainResult<ConvergentOutcome>
where
    T2: TaskRepository + 'static,
    Tr: TrajectoryRepository + 'static,
    M: MemoryRepository + 'static,
    O: OverseerMeasurer + 'static,
{
    // Part 7.1: Track previous attractor classification for transition detection
    let mut prev_attractor = trajectory.attractor_state.classification.clone();

    loop {
        // Check cancellation
        if cancellation_token.is_cancelled() {
            trajectory_store.save(&trajectory).await?;
            emit_convergence_terminated(
                event_bus, task, goal_id, &trajectory, "cancelled",
            ).await;
            return Ok(ConvergentOutcome::Cancelled);
        }

        // SLA pressure consumption (Part 8.2)
        if let Ok(Some(current_task)) = task_repo.get(task.id).await {
            apply_sla_pressure(&current_task.context.hints, &mut trajectory.policy);
        }

        // Select strategy
        let strategy = if let Some(forced) = trajectory.forced_strategy.take() {
            forced
        } else {
            let eligible = eligible_strategies(
                &trajectory.strategy_log,
                &trajectory.attractor_state,
                &trajectory.budget,
                trajectory.total_fresh_starts,
                trajectory.policy.max_fresh_starts,
            );
            if eligible.is_empty() {
                let outcome = ConvergenceOutcome::Trapped {
                    trajectory_id: trajectory.id.to_string(),
                    attractor_type: trajectory.attractor_state.classification.clone(),
                };
                engine.finalize(&mut trajectory, &outcome, &bandit).await?;
                emit_convergence_terminated(
                    event_bus, task, goal_id, &trajectory, "trapped",
                ).await;
                return Ok(ConvergentOutcome::Failed(format!(
                    "trapped in {:?} attractor -- no eligible escape strategies",
                    trajectory.attractor_state.classification
                )));
            }
            bandit.select(
                &trajectory.attractor_state.classification,
                &eligible,
                &trajectory.policy,
            )
        };

        // Part 7.1: Strategy escalation check
        // ArchitectReview and Decompose are high-impact strategies that
        // may warrant human oversight before execution.
        if matches!(&strategy, StrategyKind::ArchitectReview | StrategyKind::Decompose) {
            event_bus.publish(event_factory::make_event(
                EventSeverity::Warning,
                crate::services::event_bus::EventCategory::Convergence,
                goal_id,
                Some(task.id),
                EventPayload::HumanEscalationNeeded {
                    goal_id,
                    task_id: Some(task.id),
                    reason: format!(
                        "Convergence engine selected {} strategy for task {}",
                        strategy.kind_name(), task.id
                    ),
                    urgency: "medium".to_string(),
                    is_blocking: false,
                },
            )).await;
        }

        // FreshStart worktree reset (Part 3.7)
        if matches!(strategy, StrategyKind::FreshStart { .. }) {
            trajectory.total_fresh_starts += 1;
            if let Some(wt) = worktree_path {
                reset_worktree(wt).await?;
            }
            event_bus.publish(event_factory::make_event(
                EventSeverity::Info,
                crate::services::event_bus::EventCategory::Convergence,
                goal_id,
                Some(task.id),
                EventPayload::ConvergenceFreshStart {
                    task_id: task.id,
                    trajectory_id: trajectory.id,
                    fresh_start_number: trajectory.total_fresh_starts,
                    reason: "FreshStart strategy selected".to_string(),
                },
            )).await;
        }

        // Build prompt and execute substrate
        let prompt = convergence_bridge::build_convergent_prompt(task, &trajectory, &strategy);

        let mut config = SubstrateConfig::default().with_max_turns(max_turns);
        if let Some(wt) = worktree_path {
            config = config.with_working_dir(wt);
        }
        let request = SubstrateRequest::new(
            task.id,
            agent_type,
            system_prompt,
            &prompt,
        ).with_config(config);

        let iteration_start = Instant::now();
        let session = substrate.execute(request).await?;
        let wall_time_ms = iteration_start.elapsed().as_millis() as u64;

        let artifact = convergence_bridge::collect_artifact(
            worktree_path.unwrap_or("."),
            "",
        );

        let tokens_used = session.total_tokens();
        let sequence = trajectory.observations.len() as u32;

        let observation = Observation::new(
            sequence,
            artifact,
            OverseerSignals::default(),
            strategy.clone(),
            tokens_used,
            wall_time_ms,
        );

        let loop_control = engine
            .iterate_once(&mut trajectory, &mut bandit, &strategy, observation)
            .await?;

        // Part 7.1: Attractor transition detection
        let current_attractor = &trajectory.attractor_state.classification;
        if attractor_type_label(current_attractor) != attractor_type_label(&prev_attractor) {
            event_bus.publish(event_factory::make_event(
                EventSeverity::Info,
                crate::services::event_bus::EventCategory::Convergence,
                goal_id,
                Some(task.id),
                EventPayload::ConvergenceAttractorTransition {
                    task_id: task.id,
                    trajectory_id: trajectory.id,
                    from: attractor_type_label(&prev_attractor).to_string(),
                    to: attractor_type_label(current_attractor).to_string(),
                    confidence: trajectory.attractor_state.confidence,
                },
            )).await;
            prev_attractor = current_attractor.clone();
        }

        // Emit iteration event
        let (delta, level) = trajectory
            .observations
            .last()
            .and_then(|o| o.metrics.as_ref())
            .map(|m| (m.convergence_delta, m.convergence_level))
            .unwrap_or((0.0, 0.0));

        event_bus.publish(event_factory::make_event(
            EventSeverity::Info,
            crate::services::event_bus::EventCategory::Convergence,
            goal_id,
            Some(task.id),
            EventPayload::ConvergenceIteration {
                task_id: task.id,
                trajectory_id: trajectory.id,
                iteration: sequence,
                strategy: strategy.kind_name().to_string(),
                convergence_delta: delta,
                convergence_level: level,
                attractor_type: attractor_type_label(
                    &trajectory.attractor_state.classification,
                ).to_string(),
                budget_remaining_fraction: trajectory.budget.remaining_fraction(),
            },
        )).await;

        // Act on loop control
        match loop_control {
            LoopControl::Continue => continue,
            LoopControl::Converged => {
                let final_seq = trajectory
                    .observations
                    .last()
                    .map(|o| o.sequence)
                    .unwrap_or(0);
                let outcome = ConvergenceOutcome::Converged {
                    trajectory_id: trajectory.id.to_string(),
                    final_observation_sequence: final_seq,
                };
                engine.finalize(&mut trajectory, &outcome, &bandit).await?;
                emit_convergence_terminated(
                    event_bus, task, goal_id, &trajectory, "converged",
                ).await;
                return Ok(ConvergentOutcome::Converged);
            }
            LoopControl::Exhausted => {
                let accept_partial = if trajectory.policy.partial_acceptance {
                    trajectory
                        .best_observation()
                        .and_then(|o| o.metrics.as_ref())
                        .map(|m| m.convergence_level >= trajectory.policy.partial_threshold)
                        .unwrap_or(false)
                } else {
                    false
                };

                let best_seq = trajectory.best_observation().map(|o| o.sequence);
                let outcome = if accept_partial {
                    ConvergenceOutcome::Converged {
                        trajectory_id: trajectory.id.to_string(),
                        final_observation_sequence: best_seq.unwrap_or(0),
                    }
                } else {
                    ConvergenceOutcome::Exhausted {
                        trajectory_id: trajectory.id.to_string(),
                        best_observation_sequence: best_seq,
                    }
                };
                engine.finalize(&mut trajectory, &outcome, &bandit).await?;
                emit_convergence_terminated(
                    event_bus, task, goal_id, &trajectory, "exhausted",
                ).await;
                return Ok(if accept_partial {
                    ConvergentOutcome::PartialAccepted
                } else {
                    ConvergentOutcome::Failed(
                        "convergence budget exhausted without reaching acceptance threshold"
                            .to_string(),
                    )
                });
            }
            LoopControl::Trapped => {
                let outcome = ConvergenceOutcome::Trapped {
                    trajectory_id: trajectory.id.to_string(),
                    attractor_type: trajectory.attractor_state.classification.clone(),
                };
                engine.finalize(&mut trajectory, &outcome, &bandit).await?;
                emit_convergence_terminated(
                    event_bus, task, goal_id, &trajectory, "trapped",
                ).await;
                return Ok(ConvergentOutcome::Failed(format!(
                    "trapped in {} attractor",
                    attractor_type_label(&trajectory.attractor_state.classification),
                )));
            }
            LoopControl::Decompose => {
                emit_convergence_terminated(
                    event_bus, task, goal_id, &trajectory, "decomposed",
                ).await;
                return Ok(ConvergentOutcome::Decomposed(trajectory));
            }
            LoopControl::RequestExtension => {
                let additional_iterations = 3u32;
                let additional_tokens = (trajectory.budget.max_tokens as f64 * 0.3) as u64;

                if engine.request_extension(&mut trajectory).await? {
                    event_bus.publish(event_factory::make_event(
                        EventSeverity::Info,
                        crate::services::event_bus::EventCategory::Convergence,
                        goal_id,
                        Some(task.id),
                        EventPayload::ConvergenceBudgetExtension {
                            task_id: task.id,
                            trajectory_id: trajectory.id,
                            granted: true,
                            additional_iterations,
                            additional_tokens,
                        },
                    )).await;
                    continue;
                } else {
                    event_bus.publish(event_factory::make_event(
                        EventSeverity::Warning,
                        crate::services::event_bus::EventCategory::Convergence,
                        goal_id,
                        Some(task.id),
                        EventPayload::ConvergenceBudgetExtension {
                            task_id: task.id,
                            trajectory_id: trajectory.id,
                            granted: false,
                            additional_iterations: 0,
                            additional_tokens: 0,
                        },
                    )).await;

                    let outcome = ConvergenceOutcome::BudgetDenied {
                        trajectory_id: trajectory.id.to_string(),
                    };
                    engine.finalize(&mut trajectory, &outcome, &bandit).await?;
                    emit_convergence_terminated(
                        event_bus, task, goal_id, &trajectory, "budget_denied",
                    ).await;
                    return Ok(ConvergentOutcome::Failed(
                        "budget extension denied".to_string(),
                    ));
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Worktree management helpers (Parts 3.7, 11.2)
// ---------------------------------------------------------------------------

/// Reset a worktree to the base branch state (Part 3.7).
///
/// Used when the FreshStart strategy is selected. Runs `git checkout -- .`
/// followed by `git clean -fd` to restore the worktree to a pristine state
/// without destroying the worktree allocation itself. The carry-forward
/// context in the prompt provides the agent with learnings from previous
/// attempts.
async fn reset_worktree(worktree_path: &str) -> DomainResult<()> {
    let checkout = tokio::process::Command::new("git")
        .args(["checkout", "--", "."])
        .current_dir(worktree_path)
        .output()
        .await
        .map_err(|e| {
            DomainError::ExecutionFailed(format!(
                "failed to reset worktree {}: git checkout: {}",
                worktree_path, e
            ))
        })?;

    if !checkout.status.success() {
        let stderr = String::from_utf8_lossy(&checkout.stderr);
        return Err(DomainError::ExecutionFailed(format!(
            "git checkout -- . failed in {}: {}",
            worktree_path, stderr
        )));
    }

    let clean = tokio::process::Command::new("git")
        .args(["clean", "-fd"])
        .current_dir(worktree_path)
        .output()
        .await
        .map_err(|e| {
            DomainError::ExecutionFailed(format!(
                "failed to clean worktree {}: git clean: {}",
                worktree_path, e
            ))
        })?;

    if !clean.status.success() {
        let stderr = String::from_utf8_lossy(&clean.stderr);
        return Err(DomainError::ExecutionFailed(format!(
            "git clean -fd failed in {}: {}",
            worktree_path, stderr
        )));
    }

    Ok(())
}

/// Create a git worktree at the specified path from the given branch.
///
/// Used by parallel mode (Part 11.2) to create N independent worktrees
/// for concurrent substrate invocations.
async fn create_worktree(worktree_path: &str, branch: &str) -> DomainResult<()> {
    let output = tokio::process::Command::new("git")
        .args(["worktree", "add", worktree_path, branch])
        .output()
        .await
        .map_err(|e| {
            DomainError::ExecutionFailed(format!(
                "failed to create worktree at {}: {}",
                worktree_path, e
            ))
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(DomainError::ExecutionFailed(format!(
            "git worktree add failed for {}: {}",
            worktree_path, stderr
        )));
    }

    Ok(())
}

/// Destroy a git worktree at the specified path.
///
/// Used by parallel mode (Part 11.2) to clean up losing worktrees after
/// the best trajectory is selected.
async fn destroy_worktree(worktree_path: &str) -> DomainResult<()> {
    let output = tokio::process::Command::new("git")
        .args(["worktree", "remove", "--force", worktree_path])
        .output()
        .await
        .map_err(|e| {
            DomainError::ExecutionFailed(format!(
                "failed to destroy worktree at {}: {}",
                worktree_path, e
            ))
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        tracing::warn!(
            worktree = worktree_path,
            "git worktree remove failed: {}",
            stderr
        );
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Event emission helpers
// ---------------------------------------------------------------------------

/// Emit a ConvergenceTerminated event summarizing the final state.
async fn emit_convergence_terminated(
    event_bus: &Arc<EventBus>,
    task: &Task,
    goal_id: Option<Uuid>,
    trajectory: &Trajectory,
    outcome_label: &str,
) {
    let final_level = trajectory
        .observations
        .last()
        .and_then(|o| o.metrics.as_ref())
        .map(|m| m.convergence_level)
        .unwrap_or(0.0);

    event_bus.publish(event_factory::make_event(
        EventSeverity::Info,
        crate::services::event_bus::EventCategory::Convergence,
        goal_id,
        Some(task.id),
        EventPayload::ConvergenceTerminated {
            task_id: task.id,
            trajectory_id: trajectory.id,
            outcome: outcome_label.to_string(),
            total_iterations: trajectory.observations.len() as u32,
            total_tokens: trajectory.budget.tokens_used,
            final_convergence_level: final_level,
        },
    )).await;
}

/// Map an AttractorType to a human-readable label for event payloads.
fn attractor_type_label(attractor: &AttractorType) -> &'static str {
    match attractor {
        AttractorType::FixedPoint { .. } => "fixed_point",
        AttractorType::LimitCycle { .. } => "limit_cycle",
        AttractorType::Divergent { .. } => "divergent",
        AttractorType::Plateau { .. } => "plateau",
        AttractorType::Indeterminate { .. } => "indeterminate",
    }
}

/// Apply SLA pressure hints to the convergence policy (Part 8.2).
///
/// The ConvergenceSLAPressureHandler adds "sla:warning" or "sla:critical"
/// hints to the task's persisted context. When the convergence loop detects
/// these hints, it adjusts the trajectory policy to increase the likelihood
/// of convergence within the remaining time:
///
/// - **sla:warning** -- Lower the acceptance threshold to accept "good enough"
///   results and enable partial acceptance.
/// - **sla:critical** -- Aggressively lower thresholds and skip expensive
///   overseers to converge as quickly as possible.
fn apply_sla_pressure(hints: &[String], policy: &mut ConvergencePolicy) {
    if hints.iter().any(|h| h == "sla:critical") {
        policy.acceptance_threshold = policy.acceptance_threshold.min(0.80);
        policy.partial_acceptance = true;
        policy.partial_threshold = policy.partial_threshold.min(0.50);
        policy.skip_expensive_overseers = true;
    } else if hints.iter().any(|h| h == "sla:warning") {
        policy.acceptance_threshold = policy.acceptance_threshold.min(0.85);
        policy.partial_acceptance = true;
        policy.partial_threshold = policy.partial_threshold.min(0.60);
    }
}
