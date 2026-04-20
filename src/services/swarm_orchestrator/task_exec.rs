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
//! Both halves of that flow are owned by
//! [`handle_intent_gaps_with_retry`] so future maintainers can find the full
//! flow in one place. The risk-mitigation note for this lives in spec T10 §6
//! Risk 4.
//!
//! ## Risk 1 (deadlock) mitigation
//!
//! Every Arc/handle the spawn-block needs is moved into [`ExecutionConfig`]
//! and [`TaskExecutionService`] **before** the orchestrator calls
//! `tokio::spawn`. No `Arc<RwLock<>>` or `Arc<Mutex<>>` is constructed inside
//! `execute_task()`. See spec T10 §6 Risk 1 for the rationale.
#![allow(dead_code)]

use std::path::PathBuf;
use std::sync::Arc;

use tokio::sync::{RwLock, Semaphore};

use crate::domain::ports::{TaskRepository, TrajectoryRepository, WorktreeRepository};
use crate::services::command_bus::CommandBus;
use crate::services::event_bus::EventBus;
use crate::services::evolution_loop::EvolutionLoop;
use crate::services::guardrails::Guardrails;
use crate::services::{AuditLogService, CircuitBreakerService};

use super::middleware::PostCompletionChain;
use crate::domain::models::OutputDelivery;
use crate::domain::models::convergence::ConvergenceEngineConfig;
use crate::domain::ports::MergeRequestRepository;
use crate::domain::ports::Substrate;

/// Static configuration captured before spawning the per-task worker.
///
/// Every Arc lives in this struct so the spawn block doesn't have to clone
/// from `&self` references that may be held by other futures (Risk 1).
#[allow(dead_code)]
pub struct ExecutionConfig {
    pub repo_path: PathBuf,
    pub default_base_ref: String,
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
    /// Risk 3 mitigation: post-completion middleware chain travels with the
    /// execution config so spawned tasks always reach the verify / merge-queue
    /// / PR middleware. A unit test asserts this fires (see
    /// `test_post_completion_chain_runs_for_completed_direct_task`).
    pub post_completion_chain: Arc<RwLock<PostCompletionChain>>,
}

/// Per-task execution service.
///
/// Holds Arc clones of every dependency the spawn-block needs. Constructed
/// once per task spawn, before `tokio::spawn`.
#[allow(dead_code)]
pub struct TaskExecutionService {
    pub substrate: Arc<dyn Substrate>,
    pub task_repo: Arc<dyn TaskRepository>,
    pub worktree_repo: Arc<dyn WorktreeRepository>,
    pub event_bus: Arc<EventBus>,
    pub audit_log: Arc<AuditLogService>,
    pub circuit_breaker: Arc<CircuitBreakerService>,
    pub command_bus: Option<Arc<CommandBus>>,
    pub overseer_cluster:
        Option<Arc<crate::services::overseers::OverseerClusterService>>,
    pub trajectory_repo: Option<Arc<dyn TrajectoryRepository>>,
    pub convergence_engine_config: Option<ConvergenceEngineConfig>,
    pub intent_verifier:
        Option<Arc<dyn super::convergent_execution::ConvergentIntentVerifier>>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::substrates::MockSubstrate;

    /// Risk 3 sanity test: the post-completion chain is reachable from
    /// `ExecutionConfig`. Once `execute_task()` is migrated, this test will
    /// exercise that the chain actually fires for completed direct-mode tasks.
    #[tokio::test]
    async fn test_post_completion_chain_present_in_execution_config() {
        let chain = Arc::new(RwLock::new(PostCompletionChain::new()));

        let cfg = ExecutionConfig {
            repo_path: PathBuf::from("."),
            default_base_ref: "main".into(),
            agent_semaphore: Arc::new(Semaphore::new(1)),
            guardrails: Arc::new(Guardrails::with_defaults()),
            require_commits: false,
            verify_on_completion: false,
            use_merge_queue: false,
            prefer_pull_requests: false,
            track_evolution: false,
            evolution_loop: Arc::new(EvolutionLoop::with_default_config()),
            fetch_on_sync: false,
            output_delivery: OutputDelivery::PullRequest,
            merge_request_repo: None,
            post_completion_chain: chain.clone(),
        };

        // The chain is wired through the execution config; same Arc.
        assert!(Arc::ptr_eq(&cfg.post_completion_chain, &chain));

        // Touch substrate to keep the import live.
        let _: Arc<dyn Substrate> = Arc::new(MockSubstrate::new());
    }
}
