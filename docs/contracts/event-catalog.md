# Event Catalog

> Complete reference of every `EventPayload` variant, organized by category.

For each event: what triggers it, who handles it, and what invariants hold.

---

## Orchestrator Events

### `OrchestratorStarted`
- **Emitted by:** `SwarmOrchestrator::start()`
- **Handled by:** `StartupCatchUpHandler` — replays missed events, fixes orphaned tasks, reevaluates goals
- **Severity:** Info

### `OrchestratorPaused` / `OrchestratorResumed` / `OrchestratorStopped`
- **Emitted by:** `SwarmOrchestrator` lifecycle methods
- **Handled by:** Status display, logging
- **Severity:** Info/Warning

### `StatusUpdate(SwarmStatsPayload)`
- **Emitted by:** `StatsUpdateHandler` (periodic)
- **Handled by:** SSE/WebSocket clients
- **Invariant:** Contains snapshot of current `SwarmStats`

---

## Goal Events

### `GoalStarted { goal_id, goal_name }`
- **Emitted by:** `GoalService::create_goal()`
- **Handled by:** `GoalCreatedHandler` — refreshes active goals cache
- **Severity:** Info
- **Postcondition:** Goal exists in repository with `Active` status

### `GoalDecomposed { goal_id, task_count }`
- **Emitted by:** `SwarmOrchestrator::goal_processing` after LLM decomposition
- **Handled by:** Logging, stats
- **Precondition:** Goal exists and is Active
- **Postcondition:** `task_count` tasks have been submitted as children

### `GoalIterationCompleted { goal_id, tasks_completed }`
- **Emitted by:** `SwarmOrchestrator` after intent verification
- **Handled by:** `GoalEvaluationHandler`
- **Postcondition:** Iteration results are recorded

### `GoalPaused { goal_id, reason }`
- **Emitted by:** `GoalService::transition_status()`
- **Handled by:** Reconciliation handlers
- **Postcondition:** Goal status is `Paused`

### `GoalStatusChanged { goal_id, from_status, to_status }`
- **Emitted by:** `GoalService::transition_status()`
- **Handled by:** Cache refresh handlers
- **Severity:** Info

### `ConvergenceCompleted { goal_id, converged, iterations, final_satisfaction }`
- **Emitted by:** `SwarmOrchestrator` convergent execution
- **Handled by:** `GoalConvergenceCheckHandler`
- **Postcondition:** Goal convergence loop is terminated

### `SemanticDriftDetected { goal_id, recurring_gaps, iterations }`
- **Emitted by:** Intent verification loop
- **Handled by:** `ObstacleEscalationHandler` — may escalate to human
- **Severity:** Warning

### `GoalConstraintViolated { goal_id, constraint_name, violation }`
- **Emitted by:** Constraint checking during task execution
- **Handled by:** Logging, escalation
- **Severity:** Warning

### `GoalDeleted { goal_id, goal_name }`
- **Emitted by:** `GoalService::delete_goal()`
- **Precondition:** Goal has no children
- **Severity:** Warning

### `GoalDomainsUpdated { goal_id, old_domains, new_domains }`
- **Emitted by:** `GoalService::update_domains()`
- **Severity:** Info

### `GoalDescriptionUpdated { goal_id, reason }`
- **Emitted by:** Overmind specification amendment
- **Severity:** Info

### `GoalAlignmentEvaluated { task_id, overall_score, passes }`
- **Emitted by:** Alignment check after task completion
- **Severity:** Info/Warning (based on `passes`)

### `GoalConstraintsUpdated { goal_id }`
- **Emitted by:** `GoalService`
- **Severity:** Info

---

## Task Lifecycle Events

### `TaskSubmitted { task_id, task_title, goal_id }`
- **Emitted by:** `TaskService::submit_task()`
- **Handled by:** Logging, stats
- **Postcondition:** Task exists in repository; status is Pending, Ready, or Blocked
- **Severity:** Info

### `TaskReady { task_id, task_title }`
- **Emitted by:** `TaskService::submit_task()` (immediate readiness) or `TaskService::transition_to_ready()`
- **Handled by:** `ReadyTaskPollingHandler` — polls queue, assigns to agents
- **Precondition:** All dependencies are Complete
- **Postcondition:** Task status is `Ready`
- **Severity:** Debug

### `TaskClaimed { task_id, agent_type }`
- **Emitted by:** `TaskService::claim_task()`
- **Precondition:** Task was `Ready`
- **Postcondition:** Task status is `Running`, `agent_type` is set
- **Severity:** Info

### `TaskSpawned { task_id, task_title, agent_type }`
- **Emitted by:** `SwarmOrchestrator` when creating agent subprocess
- **Handled by:** `SpecialistCheckHandler`
- **Severity:** Info

### `TaskStarted { task_id, task_title }`
- **Emitted by:** Agent execution start
- **Severity:** Info

### `TaskCompleted { task_id, tokens_used }`
- **Emitted by:** `TaskService::complete_task()`
- **Handled by (System priority):** `TaskCompletedReadinessHandler` — cascades readiness to dependents
- **Handled by (High priority):** `WorkflowSubtaskCompletionHandler` — advances workflow
- **Handled by (Normal priority):** `ConvergenceCoordinationHandler`, `TaskCompletionLearningHandler`, `TaskOutcomeMemoryHandler`
- **Postcondition:** Task status is `Complete`; `completed_at` is set
- **Severity:** Info
- **Critical handler note:** `TaskCompletedReadinessHandler` retries on `ConcurrencyConflict`

### `TaskCompletedWithResult { task_id, result: TaskResultPayload }`
- **Emitted by:** Substrate execution wrapper (contains full result data)
- **Handled by:** Same as `TaskCompleted` plus result-specific handlers
- **Severity:** Info

### `TaskFailed { task_id, error, retry_count }`
- **Emitted by:** `TaskService::fail_task()`
- **Handled by (System priority):** `TaskFailedBlockHandler` — blocks dependents if retries exhausted
- **Handled by (System priority):** `AgentTerminationHandler` — kills agent subprocess
- **Handled by (Normal priority):** `TaskFailedRetryHandler` — requeues if retries remain
- **Postcondition:** Task status is `Failed`; error stored in context hints
- **Severity:** Error
- **Critical invariant:** If `retry_count >= max_retries`, dependents are blocked. Otherwise, task may be retried.

### `TaskRetrying { task_id, attempt, max_attempts }`
- **Emitted by:** `TaskService::retry_task()`
- **Precondition:** Task was `Failed` with `retry_count < max_retries`
- **Postcondition:** Task status is `Ready`; `retry_count` incremented
- **Severity:** Warning

### `TaskCanceled { task_id, reason }`
- **Emitted by:** `TaskService::cancel_task()`
- **Handled by (System priority):** `TaskFailedBlockHandler` — blocks all dependents
- **Handled by (System priority):** `AgentTerminationHandler` — kills agent subprocess
- **Postcondition:** Task status is `Canceled` (terminal)
- **Severity:** Warning

### `TaskValidating { task_id }`
- **Emitted by:** Workflow engine when entering verification
- **Postcondition:** Task status is `Validating`
- **Severity:** Info

### `TaskVerified { task_id, passed, checks_passed, checks_total }`
- **Emitted by:** `IntegrationVerifier::verify_task()`
- **Severity:** Info

### `TaskQueuedForMerge { task_id, stage }`
- **Emitted by:** Post-completion workflow
- **Handled by:** `EgressRoutingHandler`
- **Severity:** Info

### `PullRequestCreated { task_id, pr_url, branch }`
- **Emitted by:** Worktree merge flow
- **Severity:** Info

### `TaskMerged { task_id, commit_sha }`
- **Emitted by:** Worktree merge flow
- **Severity:** Info

### `WorktreeCreated { task_id, path }`
- **Emitted by:** `WorktreeService::create_worktree()`
- **Severity:** Info

### `WorktreeDestroyed { worktree_id, task_id, reason }`
- **Emitted by:** Worktree cleanup
- **Severity:** Info

### `TaskDependencyChanged { task_id, added, removed }`
- **Emitted by:** DAG restructure operations
- **Severity:** Info

### `TaskPriorityChanged { task_id, from, to, reason }`
- **Emitted by:** `PriorityAgingHandler` or manual override
- **Severity:** Info

### `TaskDescriptionUpdated { task_id, reason }`
- **Emitted by:** Specification amendment during convergence
- **Severity:** Info

### `SubtaskMergedToFeature { task_id, feature_branch }`
- **Emitted by:** Subtask merge flow
- **Severity:** Info

### `TaskExecutionRecorded { task_id, execution_mode, complexity, succeeded, tokens_used }`
- **Emitted by:** `TaskService::complete_task()` and `TaskService::fail_task()`
- **Handled by:** `TaskCompletionLearningHandler` — ML feedback loop
- **Severity:** Debug
- **Category:** Memory (not Task)

---

## Execution Events

### `ExecutionStarted { total_tasks, wave_count }`
- **Emitted by:** `DagExecutor` when beginning DAG execution
- **Severity:** Info

### `ExecutionCompleted { status, results }`
- **Emitted by:** `DagExecutor` when all waves complete
- **Severity:** Info

### `WaveStarted { wave_number, task_count }` / `WaveCompleted { wave_number, succeeded, failed }`
- **Emitted by:** `DagExecutor` per-wave
- **Severity:** Info

### `RestructureTriggered { task_id, decision }` / `RestructureDecision { task_id, decision }`
- **Emitted by:** DAG restructure service
- **Severity:** Info/Warning

### `ReviewLoopTriggered { failed_review_task_id, iteration, max_iterations, new_plan_task_id, new_review_task_id }`
- **Emitted by:** Review failure handler
- **Handled by:** `ReviewFailureLoopHandler`
- **Severity:** Warning

---

## Agent Events

### `AgentCreated { agent_type, tier }` / `AgentInstanceSpawned { instance_id, template_name, tier }`
- **Emitted by:** Agent lifecycle service
- **Severity:** Info

### `AgentInstanceAssigned { instance_id, task_id, template_name }`
- **Emitted by:** Task dispatch
- **Severity:** Info

### `AgentInstanceCompleted { instance_id, task_id, tokens_used }` / `AgentInstanceFailed { instance_id, task_id, template_name }`
- **Emitted by:** Substrate execution wrapper
- **Severity:** Info/Error

### `SpecialistSpawned { specialist_type, trigger, task_id }`
- **Emitted by:** Specialist trigger system
- **Severity:** Info

### `EvolutionTriggered { template_name, trigger }`
- **Emitted by:** Agent evolution loop
- **Severity:** Info

### `SpawnLimitExceeded { parent_task_id, limit_type, current_value, limit_value }`
- **Emitted by:** `TaskService::check_spawn_limits()`
- **Severity:** Warning

### `AgentTemplateRegistered { template_name, tier, version }` / `AgentTemplateStatusChanged { template_name, from_status, to_status }`
- **Emitted by:** Agent template management
- **Severity:** Info

---

## Verification Events

### `IntentVerificationStarted { goal_id, iteration }` / `IntentVerificationCompleted { goal_id, satisfaction, confidence, gaps_count, iteration, will_retry }`
- **Emitted by:** Intent verification service
- **Severity:** Info

### `IntentVerificationRequested { goal_id, completed_task_ids }` / `IntentVerificationResult { satisfaction, confidence, gaps_count, iteration, should_continue }`
- **Emitted by:** Goal evaluation loop
- **Severity:** Info

### `WaveVerificationRequested / WaveVerificationResult`
- **Emitted by:** Wave-level verification
- **Severity:** Info

### `BranchVerificationStarted / BranchVerificationCompleted`
- **Emitted by:** Branch-level verification
- **Severity:** Info

---

## Memory Events

### `MemoryStored { memory_id, key, namespace, tier, memory_type }`
- **Emitted by:** Memory service on write
- **Severity:** Debug

### `MemoryPromoted { memory_id, key, from_tier, to_tier }`
- **Emitted by:** Memory maintenance daemon
- **Severity:** Info

### `MemoryPruned { count, reason }`
- **Emitted by:** Memory maintenance daemon
- **Severity:** Info

### `MemoryAccessed { memory_id, key, access_count, accessor, distinct_accessor_count }`
- **Emitted by:** Memory service on read
- **Handled by:** `MemoryInformedDecompositionHandler`
- **Severity:** Debug

### `MemoryConflictDetected { memory_a, memory_b, key, similarity }` / `MemoryConflictResolved { memory_a, memory_b, resolution_type }`
- **Emitted by:** Memory maintenance
- **Handled by:** `MemoryConflictEscalationHandler` — escalates if unresolvable
- **Severity:** Warning/Info

### `MemoryMaintenanceCompleted / MemoryMaintenanceFailed / MemoryDaemonDegraded / MemoryDaemonStopped`
- **Emitted by:** Memory decay daemon
- **Severity:** Info/Error/Critical

---

## Escalation Events

### `HumanEscalationRequired { goal_id, task_id, reason, urgency, questions, is_blocking }`
- **Emitted by:** Various services when human input is needed
- **Severity:** Warning/Critical (based on `urgency`)
- **Invariant:** If `is_blocking`, the task/goal is paused until response

### `HumanResponseReceived { escalation_id, decision, allows_continuation }`
- **Emitted by:** Escalation handler when human responds
- **Handled by:** `ConvergenceEscalationFeedbackHandler`
- **Severity:** Info

### `HumanEscalationExpired { task_id, goal_id, default_action }`
- **Emitted by:** `EscalationTimeoutHandler` (periodic)
- **Postcondition:** `default_action` is applied
- **Severity:** Warning

---

## Scheduler Events

### `ScheduledEventRegistered { schedule_id, name, schedule_type }`
- **Emitted by:** `EventScheduler::register()`
- **Severity:** Info

### `ScheduledEventFired { schedule_id, name }`
- **Emitted by:** `EventScheduler` tick loop
- **Handled by:** All periodic handlers (stats, reconciliation, pruning, polling, etc.)
- **Invariant:** At-least-once delivery; handlers must be idempotent

### `ScheduledEventCanceled { schedule_id, name }`
- **Emitted by:** `EventScheduler::cancel()`
- **Severity:** Info

---

## Convergence Events (EventPayload)

### `ConvergenceStarted { task_id, trajectory_id, estimated_iterations, basin_width, convergence_mode }`
- **Emitted by:** Convergence engine setup phase
- **Severity:** Info

### `ConvergenceIteration { task_id, trajectory_id, iteration, strategy, convergence_delta, convergence_level, attractor_type, budget_remaining_fraction }`
- **Emitted by:** Convergence engine per-iteration
- **Severity:** Debug

### `ConvergenceAttractorTransition { task_id, trajectory_id, from, to, confidence }`
- **Emitted by:** Attractor classifier
- **Severity:** Info

### `ConvergenceBudgetExtension { task_id, trajectory_id, granted, additional_iterations, additional_tokens }`
- **Emitted by:** Budget extension handler
- **Severity:** Info/Warning

### `ConvergenceFreshStart { task_id, trajectory_id, fresh_start_number, reason }`
- **Emitted by:** Convergence engine when resetting context
- **Severity:** Warning

### `ConvergenceTerminated { task_id, trajectory_id, outcome, total_iterations, total_tokens, final_convergence_level }`
- **Emitted by:** Convergence engine resolve phase
- **Handled by:** `ConvergenceMemoryHandler` — records metrics for learning
- **Severity:** Info

---

## Workflow Events

### `WorkflowEnrolled { task_id, workflow_name }`
- **Emitted by:** `TaskService::submit_task()` during auto-enrollment
- **Severity:** Info

### `WorkflowPhaseStarted { task_id, phase_index, phase_name, subtask_ids }`
- **Emitted by:** `WorkflowEngine::fan_out()`
- **Severity:** Info

### `WorkflowPhaseReady { task_id, phase_index, phase_name }`
- **Emitted by:** `WorkflowEngine::advance()`
- **Handled by:** `WorkflowVerificationHandler` (if applicable)
- **Severity:** Info

### `WorkflowGateReached { task_id, phase_index, phase_name }`
- **Emitted by:** Workflow engine when entering gate phase
- **Severity:** Info

### `WorkflowGateVerdict { task_id, phase_index, verdict }`
- **Emitted by:** Overmind gate decision
- **Severity:** Info

### `WorkflowAdvanced { task_id, from_phase, to_phase }`
- **Emitted by:** `WorkflowEngine::advance()`
- **Severity:** Info

### `WorkflowCompleted { task_id }`
- **Emitted by:** `WorkflowEngine::advance()` when all phases done
- **Postcondition:** Parent task status transitions to `Complete`
- **Severity:** Info

### `WorkflowVerificationRequested / WorkflowVerificationCompleted`
- **Emitted by:** Verification handler
- **Severity:** Info

### `WorkflowPhaseRetried { task_id, phase_index, phase_name, retry_count }`
- **Emitted by:** Workflow engine when retrying failed verification
- **Severity:** Warning

### `WorkflowPhaseFailed { task_id, phase_index, phase_name, reason }`
- **Emitted by:** Workflow engine when phase fails with exhausted retries
- **Postcondition:** Parent task transitions to `Failed`
- **Severity:** Error

---

## Adapter Events

### `AdapterIngestionCompleted { adapter_name, items_found, tasks_created }` / `AdapterIngestionFailed { adapter_name, error }`
- **Emitted by:** Adapter polling loop
- **Severity:** Info/Error

### `AdapterEgressCompleted { adapter_name, task_id, action, success }` / `AdapterEgressFailed { adapter_name, task_id, error }`
- **Emitted by:** Egress routing handler
- **Severity:** Info/Error

### `AdapterTaskIngested { task_id, adapter_name }`
- **Emitted by:** Adapter ingestion
- **Severity:** Info

---

## Budget Events

### `BudgetPressureChanged { previous_level, new_level, consumed_pct, window_id }`
- **Emitted by:** `BudgetTracker::report_budget_signal()`
- **Postcondition:** Agent concurrency limits adjusted per pressure level
- **Severity:** Info/Warning/Critical

### `BudgetOpportunityDetected { window_id, remaining_tokens, time_to_reset_secs, opportunity_score }`
- **Emitted by:** Budget tracker when utilization is low
- **Severity:** Info

---

## Federation Events

### `FederationCerebrateConnected / Disconnected / TaskDelegated / TaskAccepted / TaskRejected`
- **Emitted by:** Federation service
- **Severity:** Info/Warning

### `FederationProgressReceived / FederationResultReceived`
- **Emitted by:** Federation polling
- **Severity:** Info

### `FederationHeartbeatMissed / FederationCerebrateUnreachable / FederationStallDetected`
- **Emitted by:** Federation health monitor
- **Severity:** Warning/Error

### `FederationReactionEmitted { reaction_type, description, goal_id, task_id }`
- **Emitted by:** Federation event reactor
- **Severity:** Info

---

## SLA & Runtime Events

### `TaskSLAWarning { task_id, deadline, remaining_secs }` / `TaskSLACritical` / `TaskSLABreached`
- **Emitted by:** `TaskSLAEnforcementHandler` (periodic)
- **Handled by:** `ConvergenceSLAPressureHandler` — adds hints to convergence
- **Severity:** Warning/Error/Critical

### `TaskRunningLong { task_id, runtime_secs }` / `TaskRunningCritical { task_id, runtime_secs }`
- **Emitted by:** Reconciliation handler (periodic)
- **Severity:** Warning/Error

---

## System Events

### `HandlerError { handler_name, event_sequence, error, circuit_breaker_tripped }`
- **Emitted by:** Event reactor on handler failure
- **Severity:** Error

### `CriticalHandlerDegraded { handler_name, error, failure_count, backoff_attempt }`
- **Emitted by:** Event reactor when critical handler fails repeatedly
- **Severity:** Critical

### `ReconciliationCompleted { corrections_made }`
- **Emitted by:** Reconciliation handler
- **Severity:** Info

### `StartupCatchUpCompleted { orphaned_tasks_fixed, missed_events_replayed, goals_reevaluated, duration_ms }`
- **Emitted by:** `StartupCatchUpHandler`
- **Severity:** Info

### `TriggerRuleCreated / TriggerRuleToggled / TriggerRuleDeleted`
- **Emitted by:** Trigger rule management
- **Severity:** Info

### `MemoryInformedGoal { goal_id, memory_id, memory_key }`
- **Emitted by:** Memory-informed decomposition handler
- **Severity:** Debug
