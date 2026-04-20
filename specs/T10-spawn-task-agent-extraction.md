# T10 — Refactor `spawn_task_agent` into Cohesive Services

> Design spec for review. Produced by a planner agent reading current main.
> Implementation should follow the migration plan in §5; each commit must compile + pass tests.

## 1. Summary

`spawn_task_agent` (lines 263–2256 in `src/services/swarm_orchestrator/goal_processing.rs`, ~1993 lines) is a monolithic spawner that orchestrates the entire task execution lifecycle. It handles middleware pre-spawn validation, agent template resolution, capability registration, worktree provisioning, context assembly (goals + memory), file I/O (CLAUDE.md + settings.json), execution mode routing (Direct vs. Convergent), and multi-branch completion workflows including post-task verification, merge queuing, and intent gap retry creation.

The function is too large to reason about, mutates shared state ~150+ times via clones, has 5+ levels of nesting in the convergence branch, and mixes unrelated concerns: workspace setup, context loading, execution routing, and completion handling. This makes changes risky and makes testing individual phases impossible.

## 2. Proposed Services

### Service 1: `AgentPreparationService`
**Purpose:** Resolve agent template, validate pre-spawn constraints, extract capabilities.

```rust
pub struct AgentPreparationService {
    agent_repo: Arc<dyn AgentRepository>,
    task_repo: Arc<dyn TaskRepository>,
    audit_log: Arc<AuditLogService>,
    circuit_breaker: Arc<CircuitBreakerService>,
}

pub async fn prepare_agent(
    &self,
    agent_type: &str,
) -> DomainResult<AgentMetadata>;

pub struct AgentMetadata {
    pub version: u32,
    pub capabilities: Vec<String>,
    pub cli_tools: Vec<String>,
    pub can_write: bool,
    pub is_read_only: bool,
    pub max_turns: u32,
    pub preferred_model: Option<String>,
    pub tier: AgentTier,
    pub is_read_only_role: bool, // name-based heuristic + template check
}
```

Private helpers:
- `determine_read_only_role()` — heuristic for legacy agents
- `map_template_tools_to_cli()` — existing helper, moved here

### Service 2: `TaskContextService`
**Purpose:** Load goal + memory context, assemble task description.

```rust
pub struct TaskContextService {
    goal_repo: Arc<dyn GoalRepository>,
    memory_repo: Option<Arc<dyn MemoryRepository>>,
}

pub async fn load_task_context(
    &self,
    task: &Task,
) -> DomainResult<TaskContext>;

pub struct TaskContext {
    pub goal_context: Option<String>,
    pub memory_context: Option<String>,
    pub intent_gap_context: Option<String>,
    pub combined_description: String, // goal + memory + intent_gap + task.description
}
```

Private helpers: `load_goal_context()`, `load_memory_context()` (budget-aware), `assemble_description()`.

### Service 3: `WorkspaceProvisioningService`
**Purpose:** Provision worktree, write CLAUDE.md and settings.json.

```rust
pub struct WorkspaceProvisioningService {
    worktree_repo: Arc<dyn WorktreeRepository>,
}

pub async fn provision_workspace(
    &self,
    task_id: uuid::Uuid,
    workspace_kind: WorkspaceKind,
) -> DomainResult<Option<String>>; // Returns worktree_path or None

pub async fn write_agent_config(
    &self,
    worktree_path: &str,
    agent_metadata: &AgentMetadata,
) -> DomainResult<()>;
```

Private helpers: `write_claude_md()`, `write_settings_json()`, `ensure_claude_dir()`.

### Service 4: `ExecutionModeResolverService`
**Purpose:** Determine effective execution mode (Direct vs. Convergent), apply runtime upgrades.

```rust
pub struct ExecutionModeResolverService {
    convergence_enabled: bool,
}

pub fn resolve_mode(
    &self,
    stored_mode: ExecutionMode,
    agent_metadata: &AgentMetadata,
) -> (ExecutionMode, bool); // (effective_mode, is_convergent_final)
```

Private helpers: `should_upgrade_to_convergent()`.

### Service 5: `TaskExecutionService`
**Purpose:** Spawn and manage task execution (Direct or Convergent), handle outcomes.

```rust
pub struct TaskExecutionService {
    substrate: Arc<dyn Substrate>,
    task_repo: Arc<dyn TaskRepository>,
    worktree_repo: Arc<dyn WorktreeRepository>,
    event_bus: Arc<EventBus>,
    audit_log: Arc<AuditLogService>,
    circuit_breaker: Arc<CircuitBreakerService>,
    command_bus: Arc<RwLock<Option<Arc<CommandBus>>>>,
    // Convergence infrastructure (optional)
    overseer_cluster: Option<Arc<OverseerClusterService>>,
    trajectory_repo: Option<Arc<dyn TrajectoryRepository>>,
    convergence_engine_config: Option<ConvergenceEngineConfig>,
    memory_repo: Option<Arc<dyn MemoryRepository>>,
    intent_verifier: Option<Arc<dyn ConvergentIntentVerifier>>,
}

pub async fn execute_task(
    &self,
    task: &Task,
    agent_metadata: &AgentMetadata,
    task_context: &TaskContext,
    worktree_path: Option<&str>,
    effective_mode: ExecutionMode,
    max_turns: u32,
    config: &ExecutionConfig,
) -> DomainResult<TaskExecutionOutcome>;

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
}

pub enum TaskExecutionOutcome {
    Completed { task_id: uuid::Uuid },
    Failed { task_id: uuid::Uuid, error: String },
    Retrying { task_id: uuid::Uuid, retry_task_id: uuid::Uuid },
}
```

Private helpers: `run_direct_execution()`, `run_convergent_execution()`, `handle_convergent_outcome()`, `handle_direct_outcome()`, `bump_max_turns_on_retry()`.

## 3. The Slimmed `spawn_task_agent` (Pseudocode)

```rust
pub(super) async fn spawn_task_agent(
    &self,
    task: &Task,
    event_tx: &mpsc::Sender<SwarmEvent>,
) -> DomainResult<()> {
    use super::middleware::{PreSpawnContext, PreSpawnDecision};

    // Phase 1: Pre-spawn middleware validation
    let mut ctx = PreSpawnContext { /* build as before */ };
    let decision = { let chain = self.pre_spawn_chain.read().await; chain.run(&mut ctx).await? };
    if let PreSpawnDecision::Skip { reason } = decision { return Ok(()); }
    let agent_type = ctx.agent_type.clone().ok_or(/* ... */)?;

    // Phase 2: Atomically claim task
    let scope = CircuitScope::agent(&agent_type);
    if self.agent_semaphore.clone().try_acquire_owned().is_err() { return Ok(()); }
    match self.task_repo.claim_task_atomic(task.id, &agent_type).await? {
        Ok(None) => return Ok(()),
        Ok(Some(_)) => { /* publish TaskClaimed event */ },
        Err(_) => return Ok(()),
    }

    // Phase 3: Prepare agent
    let agent_metadata = self.agent_prep_svc.prepare_agent(&agent_type).await?;

    // Phase 4: Register capabilities with A2A if needed
    if self.config.mcp_servers.a2a_gateway.is_some() {
        let _ = self.register_agent_capabilities(&agent_type, agent_metadata.capabilities.clone()).await;
    }

    // Phase 5: Resolve execution mode
    let (effective_mode, is_convergent) = self.exec_mode_resolver.resolve_mode(
        task.execution_mode.clone(),
        &agent_metadata,
    );

    // Phase 6: Provision workspace + write config
    let worktree_path = self.workspace_svc.provision_workspace(
        task.id, task_workflow.workspace_kind
    ).await?;
    if let Some(ref path) = worktree_path {
        self.workspace_svc.write_agent_config(path, &agent_metadata).await?;
    }

    // Phase 7: Load task context (goals + memory)
    let task_context = self.context_svc.load_task_context(task).await?;

    // Phase 8: Spawn task execution
    let config = ExecutionConfig { /* populate from self.config */ };
    let _outcome = self.exec_svc.execute_task(
        task,
        &agent_metadata,
        &task_context,
        worktree_path.as_deref(),
        effective_mode,
        max_turns,
        &config,
    ).await?;

    Ok(())
}
```

## 4. Data Flow

| Phase | Input | Output | Handler |
|-------|-------|--------|---------|
| Pre-spawn validation | Task + Agent type hint | `PreSpawnDecision` (Continue/Skip) | Pre-spawn middleware chain |
| Agent preparation | Agent type string | `AgentMetadata` | `AgentPreparationService` |
| Execution mode resolution | `ExecutionMode` + `AgentMetadata` | Effective `ExecutionMode` + `is_convergent` | `ExecutionModeResolverService` |
| Workspace provisioning | `WorkspaceKind` | `Option<String>` (worktree path) + .claude/*.md files | `WorkspaceProvisioningService` |
| Context loading | Task | `TaskContext` | `TaskContextService` |
| Task execution | All above + permits + SLA | `TaskExecutionOutcome` | `TaskExecutionService` (spawned) |
| Post-completion | `TaskExecutionOutcome` | Status transitions + event bus + worktree cleanup | Post-completion middleware chain (existing) |

State handoff structs: `AgentMetadata`, `TaskContext`, `ExecutionConfig`, `TaskExecutionOutcome`.

## 5. Migration Plan

Each step compiles + passes tests independently.

1. **Introduce service structs (no behavior change).** Add `agent_prep.rs`, `task_context.rs`, `workspace.rs`, `exec_mode.rs`, `task_exec.rs` modules. Implement service structs with constructor + stub methods. Wire into `SwarmOrchestrator` as Arc fields.
2. **Migrate `AgentPreparationService`.** Move `map_template_tools_to_cli()`. Implement `prepare_agent()` with logic from current lines 369–422. Replace inline logic in `spawn_task_agent`.
3. **Migrate `ExecutionModeResolverService`.** Extract lines 666–697 into `resolve_mode()`. Replace inline logic.
4. **Migrate `WorkspaceProvisioningService`.** Extract worktree provisioning + CLAUDE.md/settings.json I/O (lines 512–581). Replace inline.
5. **Migrate `TaskContextService`.** Extract goal/memory context loading (lines 583–656).
6. **Migrate `TaskExecutionService::execute_task` — Direct path.** Extract lines 1573–2100. Delegate to existing `run_post_completion_workflow()`.
7. **Migrate `TaskExecutionService::execute_task` — Convergent path.** Extract lines 814–1544. Delegate to existing `run_convergent_execution()` + `run_parallel_convergent_execution()`. Implement intent-gap + decomposition + failure outcome handling.
8. **Delete inlined code.** Remove all extracted inline logic. Verify `spawn_task_agent` is now ~60–80 lines. All tests must pass with no behavior change.
9. **Update constructor + builder.** Wire new services into `SwarmOrchestrator::new()`. Lazy-construct services where appropriate.

## 6. Risks

**Risk 1: Deadlock in spawned task path.** Current code moves large Arcs into `tokio::spawn` (lines 805–2253). If a service holds an orchestrator-field reference and a clone is missed, the spawned task may deadlock waiting for a lock held by the main loop.
*Mitigation:* All services receive `Arc<dyn Trait>` clones BEFORE the spawn. Verify no `Arc<RwLock<>>` or `Arc<Mutex<>>` is constructed inside `execute_task()`.

**Risk 2: Incomplete state hand-off to spawned task.** `execute_task()` is called before `tokio::spawn`; closure must receive all needed state. New service struct fields may be forgotten when constructing `ExecutionConfig`.
*Mitigation:* `ExecutionConfig` struct enforces all fields. Code review checkpoints comparing field usage before/after.

**Risk 3: Post-completion middleware breakage.** Post-completion chain runs inside the spawned task (line 1120: `run_post_completion_workflow()`). If we move `TaskExecutionService` but forget to pass `post_completion_chain`, middleware silently won't fire and verifications are skipped.
*Mitigation:* `ExecutionConfig` includes `post_completion_chain` Arc field. Add a unit test that verifies `run_post_completion_workflow` is called in the direct path.

**Risk 4: Intent-gap retry creation visibility lost.** Intent gap context creation (lines 1182–1231) and retry task creation (lines 1264–1316) move into `TaskExecutionService::handle_convergent_outcome()`. Future maintainers won't know where gap handling lives if split poorly.
*Mitigation:* Consolidate into a single helper `handle_intent_gaps_with_retry()` inside `TaskExecutionService`. Document the full flow in a module-level docstring.

**Risk 5: Model selection logic duplication.** ModelRouter call (lines 1597–1607) is deep in direct execution. Moving into `TaskExecutionService` means logic is no longer inline; future changes may miss the branch.
*Mitigation:* Extract into a dedicated method `resolve_model_for_execution()` in `AgentPreparationService` or `ExecutionConfig` builder. Resolve model early; store result.

## 7. Test Strategy

Existing tests (must pass with zero changes):
- `test_spawn_task_agent_validates_pre_spawn_chain`
- `test_spawn_task_agent_claims_task_atomically`
- `test_spawn_task_agent_with_convergent_mode`
- `test_spawn_task_agent_direct_mode_with_post_completion`
- `test_spawn_task_agent_intent_gap_retry_creation`
- All orchestrator integration tests using real substrate + event bus

New unit tests per extracted service (add in each commit):
- **AgentPreparationService**: `test_prepare_agent_resolves_template_metadata`, `test_prepare_agent_maps_capabilities_to_cli_tools`, `test_prepare_agent_detects_read_only_roles`.
- **ExecutionModeResolverService**: `test_mode_direct_unchanged_for_read_only_agent`, `test_mode_upgrades_direct_to_convergent_for_write_capable`, `test_mode_respects_convergence_disabled_flag`.
- **WorkspaceProvisioningService**: `test_workspace_provision_creates_worktree`, `test_workspace_writes_claude_md_with_tool_restrictions`, `test_workspace_writes_settings_json_with_allowed_tools`.
- **TaskContextService**: `test_context_loads_goal_guidance`, `test_context_loads_memory_with_budget_limit`, `test_context_assembles_description_in_priority_order`.
- **TaskExecutionService**: `test_execute_direct_invokes_substrate_and_transitions_status`, `test_execute_convergent_delegates_to_engine`, `test_execute_handles_intent_gaps_and_creates_retry_task`, `test_execute_publishes_TaskCompleted_event_on_success`.

## 8. Out of Scope

1. Refactor `run_post_completion_workflow()` helper — well-isolated; only move its call site.
2. Move `PreSpawnContext` building into a factory — still needs orchestrator fields; leave inline.
3. Extract middleware chain initialization — already cleanly separated.
4. Unify Direct and Convergent outcome handling — outcomes differ too much; keep separate.
5. Consolidate model selection logic across the codebase — `ModelRouter` lives elsewhere.
6. Optimize Arc cloning in spawned task — current cloning is necessary; revisit if measured.
