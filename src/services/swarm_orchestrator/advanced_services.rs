//! `AdvancedServices` — progressive-enhancement features. Every field is
//! independently `Option`, gated by a `with_*()` builder method on
//! `SwarmOrchestrator`.
//!
//! Part of the T11 god-object decomposition (see
//! `specs/T11-swarm-orchestrator-decomposition.md`). Generic over the
//! repository types because [`memory_repo`](Self::memory_repo) and
//! [`intent_verifier`](Self::intent_verifier) carry the orchestrator's
//! generic parameters; everything else is dynamic-dispatched and could in
//! principle be generic-free, but keeping the bundle on the same generics
//! as the orchestrator lets a single `with_*()` method mutate it directly.

use std::sync::Arc;

use sqlx::SqlitePool;
use tokio::sync::RwLock;

use crate::adapters::mcp::FederationClient;
use crate::domain::ports::{
    AgentRepository, GoalRepository, MemoryRepository, MergeRequestRepository, OutboxRepository,
    TaskRepository, TrajectoryRepository, TriggerRuleRepository, WorktreeRepository,
};
use crate::services::{
    IntentVerifierService, OvermindService,
    adapter_registry::AdapterRegistry,
    budget_tracker::BudgetTracker,
    command_bus::CommandBus,
    cost_window_service::CostWindowService,
    federation::FederationService,
    overseers::OverseerClusterService,
};

/// Optional, progressive-enhancement subsystems. Each field is independently
/// optional; multi-field features (e.g. convergent execution requires
/// `trajectory_repo` + `overseer_cluster` + `intent_verifier` + `memory_repo`)
/// are validated up-front by `SwarmOrchestrator::validate_dependencies()`.
// dead_code: introduced in T11 step 1; fields wired in steps 2-7.
#[allow(dead_code)]
pub(super) struct AdvancedServices<G, T, W, A, M>
where
    G: GoalRepository + 'static,
    T: TaskRepository + 'static,
    W: WorktreeRepository + 'static,
    A: AgentRepository + 'static,
    M: MemoryRepository + 'static,
{
    pub(super) memory_repo: Option<Arc<M>>,
    pub(super) intent_verifier: Option<Arc<IntentVerifierService<G, T>>>,
    pub(super) overmind: Option<Arc<OvermindService>>,
    pub(super) command_bus: Arc<RwLock<Option<Arc<CommandBus>>>>,
    /// Optional DB pool for services that need persistence (absence timers,
    /// command dedup, evolution refinements, event outbox).
    pub(super) pool: Option<SqlitePool>,
    pub(super) outbox_repo: Option<Arc<dyn OutboxRepository>>,
    pub(super) trigger_rule_repo: Option<Arc<dyn TriggerRuleRepository>>,
    pub(super) merge_request_repo: Option<Arc<dyn MergeRequestRepository>>,
    pub(super) adapter_registry: Option<Arc<AdapterRegistry>>,
    pub(super) budget_tracker: Option<Arc<BudgetTracker>>,
    pub(super) cost_window_service: Option<Arc<CostWindowService>>,

    pub(super) federation_client: Option<Arc<FederationClient>>,
    pub(super) federation_service: Option<Arc<FederationService>>,

    pub(super) overseer_cluster: Option<Arc<OverseerClusterService>>,
    pub(super) trajectory_repo: Option<Arc<dyn TrajectoryRepository>>,
    pub(super) convergence_engine_config:
        Option<crate::domain::models::convergence::ConvergenceEngineConfig>,

    // Phantom marker to consume the W generic, which is needed to keep this
    // bundle on the same generics as `SwarmOrchestrator` (so a single
    // `with_*()` method can mutate it without juggling parameters) but isn't
    // used by any field directly. `A` is unused too — same rationale.
    pub(super) _marker: std::marker::PhantomData<(W, A)>,
}

#[allow(dead_code)]
impl<G, T, W, A, M> AdvancedServices<G, T, W, A, M>
where
    G: GoalRepository + 'static,
    T: TaskRepository + 'static,
    W: WorktreeRepository + 'static,
    A: AgentRepository + 'static,
    M: MemoryRepository + 'static,
{
    /// Construct an empty bundle — every optional service is `None`. Use
    /// the `with_*()` builders on `SwarmOrchestrator` to populate.
    pub(super) fn new() -> Self {
        Self {
            memory_repo: None,
            intent_verifier: None,
            overmind: None,
            command_bus: Arc::new(RwLock::new(None)),
            pool: None,
            outbox_repo: None,
            trigger_rule_repo: None,
            merge_request_repo: None,
            adapter_registry: None,
            budget_tracker: None,
            cost_window_service: None,
            federation_client: None,
            federation_service: None,
            overseer_cluster: None,
            trajectory_repo: None,
            convergence_engine_config: None,
            _marker: std::marker::PhantomData,
        }
    }
}
