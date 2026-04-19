//! Composable middleware chains for pre-spawn and post-completion lifecycle.
//!
//! Two traits define extension points that were previously hardcoded inline
//! in the orchestrator:
//!
//! - [`PreSpawnMiddleware`] runs before an agent is spawned for a ready task.
//!   It can enrich the context (mutate through `&mut`), short-circuit the
//!   spawn with [`PreSpawnDecision::Skip`], or fail hard via `DomainError`.
//! - [`PostCompletionMiddleware`] runs after a task reaches a terminal state
//!   (Complete or Failed). Each middleware is an independent side effect; a
//!   failure in one does not block later middleware.
//!
//! The built-in implementations live in `middleware/` submodules and preserve
//! the exact semantics of the previous inline logic — the only visible change
//! is that external callers can register additional middleware via the
//! orchestrator builder.
//!
//! Inspiration: the `EventReactor`/`EventHandler` design in `event_reactor.rs`,
//! plus the hook-pack pattern from openclaw's `internal-hooks.ts`.

use async_trait::async_trait;
use std::sync::Arc;
use tokio::sync::Semaphore;

use crate::domain::errors::{DomainError, DomainResult};
use crate::domain::models::Task;
use crate::domain::models::workflow_template::OutputDelivery;
use crate::domain::ports::{
    AgentRepository, GoalRepository, MergeRequestRepository, TaskRepository, WorktreeRepository,
};
use crate::services::event_bus::EventBus;
use crate::services::{
    AuditLogService, CircuitBreakerService, Guardrails, budget_tracker::BudgetTracker,
    cost_window_service::CostWindowService,
};

// ============================================================================
// Pre-spawn chain
// ============================================================================

/// Decision returned by a [`PreSpawnMiddleware`].
#[derive(Debug)]
pub enum PreSpawnDecision {
    /// Continue to the next middleware, and ultimately to substrate invocation.
    Continue,
    /// Do not spawn. The optional reason is emitted for audit/tracing.
    Skip { reason: String },
}

/// Context threaded through the pre-spawn middleware chain.
///
/// Fields are pragmatically designed: we pass `Arc<dyn Trait>` for the
/// handles middleware actually need rather than the full orchestrator, and
/// use owned data for task metadata so the struct doesn't leak lifetimes.
///
/// Middleware can mutate fields (e.g. set `agent_type` after routing) so
/// downstream middleware sees the enriched state.
pub struct PreSpawnContext {
    // -- Inputs (owned so there is no lifetime bleed) --
    /// The task being considered for spawn.
    pub task: Task,

    // -- Enriched by middleware --
    /// Resolved agent template name; populated by `RouteTaskMiddleware`.
    pub agent_type: Option<String>,

    // -- Services / repositories the middleware may read --
    pub task_repo: Arc<dyn TaskRepository>,
    pub agent_repo: Arc<dyn AgentRepository>,
    pub goal_repo: Arc<dyn GoalRepository>,
    pub audit_log: Arc<AuditLogService>,
    pub circuit_breaker: Arc<CircuitBreakerService>,
    pub guardrails: Arc<Guardrails>,
    pub cost_window_service: Option<Arc<CostWindowService>>,
    pub budget_tracker: Option<Arc<BudgetTracker>>,
    pub agent_semaphore: Arc<Semaphore>,
    pub max_agents: usize,

    // -- Optional extension points --
    /// Running count of "federation-priority" bumps applied by middleware.
    /// Currently purely advisory — provided so a future federation-signal
    /// middleware can record when it bumps priority, without needing to
    /// mutate `Task` directly.
    pub federation_priority_bumps: u32,
}

impl PreSpawnContext {
    /// Short, human-readable task identifier for logs.
    pub fn task_id_str(&self) -> String {
        self.task.id.to_string()
    }
}

/// Pre-spawn middleware trait.
///
/// Runs before an agent is spawned for a ready task. Can enrich the context,
/// short-circuit the chain, or fail hard.
#[async_trait]
pub trait PreSpawnMiddleware: Send + Sync {
    /// Stable name used in logs / audit entries.
    fn name(&self) -> &'static str;

    async fn handle(&self, ctx: &mut PreSpawnContext) -> DomainResult<PreSpawnDecision>;
}

/// Ordered chain of [`PreSpawnMiddleware`] implementations.
///
/// Iterates in registration order. Stops at the first [`PreSpawnDecision::Skip`]
/// and returns it to the caller; propagates any [`DomainError`] immediately.
pub struct PreSpawnChain {
    middleware: Vec<Arc<dyn PreSpawnMiddleware>>,
}

impl PreSpawnChain {
    pub fn new() -> Self {
        Self {
            middleware: Vec::new(),
        }
    }

    /// Register a middleware. Registration order is preserved.
    pub fn register(&mut self, mw: Arc<dyn PreSpawnMiddleware>) {
        self.middleware.push(mw);
    }

    /// Number of registered middleware (primarily for testing / introspection).
    pub fn len(&self) -> usize {
        self.middleware.len()
    }

    pub fn is_empty(&self) -> bool {
        self.middleware.is_empty()
    }

    /// Run the chain. Returns `Continue` if every middleware allowed the
    /// spawn; returns `Skip { reason }` from the first middleware that
    /// rejected it. Errors short-circuit and bubble up.
    pub async fn run(&self, ctx: &mut PreSpawnContext) -> DomainResult<PreSpawnDecision> {
        for mw in &self.middleware {
            match mw.handle(ctx).await? {
                PreSpawnDecision::Continue => {}
                skip @ PreSpawnDecision::Skip { .. } => {
                    tracing::debug!(
                        middleware = mw.name(),
                        task_id = %ctx.task.id,
                        decision = ?skip,
                        "PreSpawnChain: middleware requested skip"
                    );
                    return Ok(skip);
                }
            }
        }
        Ok(PreSpawnDecision::Continue)
    }
}

impl Default for PreSpawnChain {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Post-completion chain
// ============================================================================

/// Context threaded through the post-completion middleware chain.
///
/// Carries the terminal task, repos/services the middleware may need, and a
/// small set of flags that earlier middleware set for later middleware to
/// short-circuit on (preserving the early-return semantics the previous
/// inline workflow had).
pub struct PostCompletionContext {
    pub task_id: uuid::Uuid,

    // -- Repositories / services --
    pub task_repo: Arc<dyn TaskRepository>,
    pub goal_repo: Arc<dyn GoalRepository>,
    pub worktree_repo: Arc<dyn WorktreeRepository>,
    pub merge_request_repo: Option<Arc<dyn MergeRequestRepository>>,
    pub audit_log: Arc<AuditLogService>,
    pub event_bus: Arc<EventBus>,
    pub event_tx: tokio::sync::mpsc::Sender<super::types::SwarmEvent>,

    // -- Workflow config (as captured at spawn time) --
    pub verify_on_completion: bool,
    pub use_merge_queue: bool,
    pub prefer_pull_requests: bool,
    pub require_commits: bool,
    pub intent_satisfied: bool,
    pub output_delivery: OutputDelivery,
    pub repo_path: std::path::PathBuf,
    pub default_base_ref: String,
    pub fetch_on_sync: bool,

    // -- Carried state set by earlier middleware --
    /// Set by verification middleware once it has run; downstream middleware
    /// that depend on the verification outcome read this.
    pub verification_passed: bool,
    /// Set by middleware that has handled per-task follow-up (e.g. subtask
    /// merge-back or autoship on a parent/root) to inform later middleware
    /// (PR / merge-queue) to stand down — matching the previous
    /// early-return behaviour.
    pub tree_handled: bool,
}

/// Post-completion middleware trait.
///
/// Runs after a task enters a terminal state. Side-effect oriented; a failure
/// in one middleware does not block the next — the chain runner logs the
/// error via `tracing` and the event bus and continues.
#[async_trait]
pub trait PostCompletionMiddleware: Send + Sync {
    /// Stable name used in logs / audit entries.
    fn name(&self) -> &'static str;

    async fn handle(&self, ctx: &mut PostCompletionContext) -> DomainResult<()>;
}

/// Ordered chain of [`PostCompletionMiddleware`] implementations.
///
/// Iterates in registration order. Does NOT stop on error; each middleware is
/// an independent side effect. Errors are surfaced as `SubsystemError` on
/// the event bus and via `tracing::error!`.
pub struct PostCompletionChain {
    middleware: Vec<Arc<dyn PostCompletionMiddleware>>,
}

impl PostCompletionChain {
    pub fn new() -> Self {
        Self {
            middleware: Vec::new(),
        }
    }

    pub fn register(&mut self, mw: Arc<dyn PostCompletionMiddleware>) {
        self.middleware.push(mw);
    }

    pub fn len(&self) -> usize {
        self.middleware.len()
    }

    pub fn is_empty(&self) -> bool {
        self.middleware.is_empty()
    }

    /// Run the chain. Each middleware runs independently — errors are logged
    /// (tracing + event_bus SubsystemError) and the chain continues.
    pub async fn run(&self, ctx: &mut PostCompletionContext) -> DomainResult<()> {
        for mw in &self.middleware {
            let name = mw.name();
            match mw.handle(ctx).await {
                Ok(()) => {}
                Err(e) => {
                    tracing::error!(
                        middleware = name,
                        task_id = %ctx.task_id,
                        error = %e,
                        "PostCompletionChain: middleware error (continuing chain)"
                    );
                    // Scoped SubsystemError so observability picks it up.
                    ctx.event_bus
                        .publish(crate::services::event_factory::orchestrator_event(
                            crate::services::event_bus::EventSeverity::Error,
                            crate::services::event_bus::EventPayload::SubsystemError {
                                subsystem: format!("post_completion::{}", name),
                                error: e.to_string(),
                            },
                        ))
                        .await;
                }
            }
        }
        Ok(())
    }
}

impl Default for PostCompletionChain {
    fn default() -> Self {
        Self::new()
    }
}

// Silence an unused import lint when no implementor references DomainError yet.
#[allow(dead_code)]
fn _assert_domain_error_used(_: DomainError) {}

pub mod autoship;
pub mod budget;
pub mod circuit_breaker;
pub mod federation_priority;
pub mod guardrails_check;
pub mod mcp_readiness;
pub mod memory_only;
pub mod merge_queue;
pub mod pull_request;
pub mod quiet_window;
pub mod route_task;
pub mod subtask_merge;
pub mod verification;

pub use autoship::AutoshipMiddleware;
pub use budget::{BudgetConcurrencyMiddleware, BudgetDispatchMiddleware};
pub use circuit_breaker::CircuitBreakerMiddleware;
pub use federation_priority::FederationPriorityMiddleware;
pub use guardrails_check::GuardrailsMiddleware;
pub use mcp_readiness::McpReadinessMiddleware;
pub use memory_only::MemoryOnlyShortCircuitMiddleware;
pub use merge_queue::MergeQueueMiddleware;
pub use pull_request::PullRequestMiddleware;
pub use quiet_window::QuietWindowMiddleware;
pub use route_task::RouteTaskMiddleware;
pub use subtask_merge::SubtaskMergeBackMiddleware;
pub use verification::VerificationMiddleware;

// ============================================================================
// Tests for the chain infrastructure itself
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::models::Task;
    use std::sync::atomic::{AtomicU32, Ordering};

    fn sample_task() -> Task {
        Task::with_title("test-task", "test description")
    }

    // -- shared test plumbing -------------------------------------------------

    fn make_pre_spawn_ctx() -> PreSpawnContext {
        use crate::adapters::sqlite::test_support;
        // Build minimal in-memory repos that satisfy the trait objects.
        // The middleware under test in this module doesn't actually exercise
        // the repos; we only need Arc<dyn ...> instances that typecheck.
        let (task_repo, agent_repo, goal_repo) = test_support::lazy_dyn_repos_minimal();
        PreSpawnContext {
            task: sample_task(),
            agent_type: None,
            task_repo,
            agent_repo,
            goal_repo,
            audit_log: Arc::new(AuditLogService::with_defaults()),
            circuit_breaker: Arc::new(CircuitBreakerService::with_defaults()),
            guardrails: Arc::new(Guardrails::with_defaults()),
            cost_window_service: None,
            budget_tracker: None,
            agent_semaphore: Arc::new(Semaphore::new(4)),
            max_agents: 4,
            federation_priority_bumps: 0,
        }
    }

    // -- mock middleware ------------------------------------------------------

    struct OrderRecording {
        name_: &'static str,
        slot: Arc<std::sync::Mutex<Vec<&'static str>>>,
        decision: PreSpawnDecision,
    }

    impl OrderRecording {
        fn cont(name: &'static str, slot: Arc<std::sync::Mutex<Vec<&'static str>>>) -> Arc<Self> {
            Arc::new(Self {
                name_: name,
                slot,
                decision: PreSpawnDecision::Continue,
            })
        }

        fn skip(
            name: &'static str,
            slot: Arc<std::sync::Mutex<Vec<&'static str>>>,
            reason: &'static str,
        ) -> Arc<Self> {
            Arc::new(Self {
                name_: name,
                slot,
                decision: PreSpawnDecision::Skip {
                    reason: reason.to_string(),
                },
            })
        }
    }

    #[async_trait]
    impl PreSpawnMiddleware for OrderRecording {
        fn name(&self) -> &'static str {
            self.name_
        }

        async fn handle(&self, _ctx: &mut PreSpawnContext) -> DomainResult<PreSpawnDecision> {
            self.slot.lock().unwrap().push(self.name_);
            Ok(match &self.decision {
                PreSpawnDecision::Continue => PreSpawnDecision::Continue,
                PreSpawnDecision::Skip { reason } => PreSpawnDecision::Skip {
                    reason: reason.clone(),
                },
            })
        }
    }

    /// Mutating middleware that stamps agent_type so we can verify enrichment.
    struct AgentStamp(&'static str);

    #[async_trait]
    impl PreSpawnMiddleware for AgentStamp {
        fn name(&self) -> &'static str {
            "agent-stamp"
        }

        async fn handle(&self, ctx: &mut PreSpawnContext) -> DomainResult<PreSpawnDecision> {
            ctx.agent_type = Some(self.0.to_string());
            Ok(PreSpawnDecision::Continue)
        }
    }

    #[tokio::test]
    async fn pre_spawn_chain_runs_middleware_in_registration_order_and_mutates_context() {
        let order = Arc::new(std::sync::Mutex::new(Vec::<&'static str>::new()));
        let mut chain = PreSpawnChain::new();
        chain.register(OrderRecording::cont("first", order.clone()));
        chain.register(Arc::new(AgentStamp("my-agent")));
        chain.register(OrderRecording::cont("third", order.clone()));

        let mut ctx = make_pre_spawn_ctx();
        assert!(ctx.agent_type.is_none());

        let decision = chain.run(&mut ctx).await.unwrap();
        assert!(matches!(decision, PreSpawnDecision::Continue));

        assert_eq!(*order.lock().unwrap(), vec!["first", "third"]);
        assert_eq!(ctx.agent_type.as_deref(), Some("my-agent"));
    }

    #[tokio::test]
    async fn pre_spawn_chain_stops_at_first_skip() {
        let order = Arc::new(std::sync::Mutex::new(Vec::<&'static str>::new()));
        let mut chain = PreSpawnChain::new();
        chain.register(OrderRecording::cont("first", order.clone()));
        chain.register(OrderRecording::skip("gate", order.clone(), "budget"));
        // This one must NOT run.
        chain.register(OrderRecording::cont("never", order.clone()));

        let mut ctx = make_pre_spawn_ctx();
        let decision = chain.run(&mut ctx).await.unwrap();

        match decision {
            PreSpawnDecision::Skip { reason } => assert_eq!(reason, "budget"),
            _ => panic!("expected Skip"),
        }
        assert_eq!(*order.lock().unwrap(), vec!["first", "gate"]);
    }

    // -- post-completion chain tests -----------------------------------------

    struct Counter(AtomicU32, &'static str);

    #[async_trait]
    impl PostCompletionMiddleware for Counter {
        fn name(&self) -> &'static str {
            self.1
        }
        async fn handle(&self, _ctx: &mut PostCompletionContext) -> DomainResult<()> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    struct Fail(&'static str);
    #[async_trait]
    impl PostCompletionMiddleware for Fail {
        fn name(&self) -> &'static str {
            self.0
        }
        async fn handle(&self, _ctx: &mut PostCompletionContext) -> DomainResult<()> {
            Err(DomainError::ExecutionFailed("boom".to_string()))
        }
    }

    fn make_post_ctx() -> PostCompletionContext {
        use crate::adapters::sqlite::test_support;
        use crate::services::event_bus::{EventBus, EventBusConfig};

        let (task_repo, goal_repo, worktree_repo) = test_support::lazy_dyn_repos_post_completion();
        let event_bus = Arc::new(EventBus::new(EventBusConfig::default()));
        let (tx, _rx) = tokio::sync::mpsc::channel(16);

        PostCompletionContext {
            task_id: uuid::Uuid::new_v4(),
            task_repo,
            goal_repo,
            worktree_repo,
            merge_request_repo: None,
            audit_log: Arc::new(AuditLogService::with_defaults()),
            event_bus,
            event_tx: tx,
            verify_on_completion: false,
            use_merge_queue: false,
            prefer_pull_requests: false,
            require_commits: false,
            intent_satisfied: false,
            output_delivery: OutputDelivery::PullRequest,
            repo_path: std::path::PathBuf::from("/tmp"),
            default_base_ref: "main".to_string(),
            fetch_on_sync: false,
            verification_passed: true,
            tree_handled: false,
        }
    }

    #[tokio::test]
    async fn post_completion_chain_continues_past_middleware_errors() {
        let first_ran = Arc::new(Counter(AtomicU32::new(0), "first"));
        let after_failure = Arc::new(Counter(AtomicU32::new(0), "third"));
        let mut chain = PostCompletionChain::new();
        chain.register(first_ran.clone());
        chain.register(Arc::new(Fail("exploding")));
        chain.register(after_failure.clone());

        let mut ctx = make_post_ctx();
        chain.run(&mut ctx).await.unwrap();

        assert_eq!(first_ran.0.load(Ordering::SeqCst), 1);
        // The key invariant: later middleware still ran despite the failure.
        assert_eq!(after_failure.0.load(Ordering::SeqCst), 1);
    }
}
