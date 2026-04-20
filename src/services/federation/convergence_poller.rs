//! Convergence polling daemon for federated goals.
//!
//! Periodically polls each active `FederatedGoal`, measures child swarm state
//! via the A2A client, evaluates the convergence contract, transitions goal
//! state, and emits lifecycle events.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

#[cfg(test)]
use chrono::Utc;
use tokio::sync::RwLock;
use uuid::Uuid;

use crate::adapters::a2a::client::A2AClient;
use crate::domain::models::a2a_protocol::A2APart;
use crate::domain::models::goal_federation::{
    ConvergenceSignalSnapshot, FederatedGoal, FederatedGoalState, TaskStatusSummary,
};
use crate::domain::ports::FederatedGoalRepository;
use crate::services::clock::{DynClock, system_clock};
use crate::services::event_bus::{EventBus, EventPayload, EventSeverity};
use crate::services::event_factory;
use crate::services::federation::service::FederationService;
use crate::services::supervise;

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// Configuration for the convergence polling daemon.
#[derive(Debug, Clone)]
pub struct ConvergencePollerConfig {
    /// Base interval between polling ticks.
    pub poll_interval: Duration,
    /// Number of consecutive poll failures before a goal is marked Failed.
    pub max_consecutive_misses: u32,
}

impl Default for ConvergencePollerConfig {
    fn default() -> Self {
        Self {
            poll_interval: Duration::from_secs(60),
            max_consecutive_misses: 5,
        }
    }
}

// ---------------------------------------------------------------------------
// Daemon handle
// ---------------------------------------------------------------------------

/// Handle to control the convergence polling daemon from outside.
pub struct ConvergencePollerHandle {
    stop_flag: Arc<AtomicBool>,
}

impl ConvergencePollerHandle {
    /// Request the daemon to stop.
    pub fn stop(&self) {
        self.stop_flag.store(true, Ordering::Release);
    }

    /// Check if stop was requested.
    pub fn is_stop_requested(&self) -> bool {
        self.stop_flag.load(Ordering::Acquire)
    }
}

// ---------------------------------------------------------------------------
// Daemon
// ---------------------------------------------------------------------------

/// Background daemon that periodically polls federated goals for convergence.
pub struct ConvergencePollingDaemon {
    federation_service: Arc<FederationService>,
    a2a_client: Arc<dyn A2AClient>,
    federated_goal_repo: Arc<dyn FederatedGoalRepository>,
    event_bus: Arc<EventBus>,
    config: ConvergencePollerConfig,
    stop_flag: Arc<AtomicBool>,
    /// Per-goal consecutive miss counters.
    miss_counters: Arc<RwLock<HashMap<Uuid, u32>>>,
    clock: DynClock,
}

impl ConvergencePollingDaemon {
    /// Create a new convergence polling daemon.
    pub fn new(
        federation_service: Arc<FederationService>,
        a2a_client: Arc<dyn A2AClient>,
        federated_goal_repo: Arc<dyn FederatedGoalRepository>,
        event_bus: Arc<EventBus>,
        config: ConvergencePollerConfig,
    ) -> Self {
        Self {
            federation_service,
            a2a_client,
            federated_goal_repo,
            event_bus,
            config,
            stop_flag: Arc::new(AtomicBool::new(false)),
            miss_counters: Arc::new(RwLock::new(HashMap::new())),
            clock: system_clock(),
        }
    }

    /// Inject a custom clock (for deterministic testing).
    pub fn with_clock(mut self, clock: DynClock) -> Self {
        self.clock = clock;
        self
    }

    /// Get a handle to control the daemon externally.
    pub fn handle(&self) -> ConvergencePollerHandle {
        ConvergencePollerHandle {
            stop_flag: self.stop_flag.clone(),
        }
    }

    /// Start the daemon. Spawns a background task and returns immediately.
    pub fn start(self) -> ConvergencePollerHandle {
        let handle = self.handle();
        supervise("convergence_poller", async move {
            self.run_loop().await;
        });
        handle
    }

    /// Main polling loop.
    async fn run_loop(&self) {
        tracing::info!(
            poll_interval_secs = self.config.poll_interval.as_secs(),
            max_consecutive_misses = self.config.max_consecutive_misses,
            "Convergence polling daemon started"
        );

        let mut interval = tokio::time::interval(self.config.poll_interval);

        loop {
            interval.tick().await;

            if self.stop_flag.load(Ordering::Acquire) {
                tracing::info!("Convergence polling daemon stopping (requested)");
                break;
            }

            if let Err(e) = self.poll_all_goals().await {
                tracing::warn!("Convergence poll tick failed: {}", e);
            }
        }
    }

    /// Poll all active federated goals once.
    async fn poll_all_goals(&self) -> Result<(), String> {
        let goals = self
            .federated_goal_repo
            .get_active()
            .await
            .map_err(|e| format!("Failed to fetch active goals: {e}"))?;

        if goals.is_empty() {
            return Ok(());
        }

        tracing::debug!(count = goals.len(), "Polling active federated goals");

        for goal in goals {
            if self.stop_flag.load(Ordering::Acquire) {
                break;
            }
            self.poll_goal(&goal).await;
        }

        Ok(())
    }

    /// Poll a single federated goal and update its state.
    async fn poll_goal(&self, goal: &FederatedGoal) {
        // We need a remote_task_id and cerebrate URL to poll.
        let remote_task_id = match &goal.remote_task_id {
            Some(id) => id.clone(),
            None => {
                tracing::debug!(
                    goal_id = %goal.id,
                    "Skipping goal without remote_task_id"
                );
                return;
            }
        };

        let cerebrate_url = match self.get_cerebrate_url(&goal.cerebrate_id).await {
            Some(url) => url,
            None => {
                tracing::warn!(
                    goal_id = %goal.id,
                    cerebrate_id = %goal.cerebrate_id,
                    "Cannot poll: cerebrate URL not available"
                );
                self.record_miss(goal, "Cerebrate URL not available").await;
                return;
            }
        };

        // Poll the child swarm's A2A task.
        let task_result = self
            .a2a_client
            .get_task(&cerebrate_url, &remote_task_id, None)
            .await;

        let task = match task_result {
            Ok(t) => {
                // Successful poll — reset miss counter.
                self.reset_miss_counter(goal.id).await;
                t
            }
            Err(e) => {
                tracing::warn!(
                    goal_id = %goal.id,
                    cerebrate_id = %goal.cerebrate_id,
                    error = %e,
                    "A2A poll failed"
                );
                self.record_miss(goal, &format!("A2A poll failed: {e}"))
                    .await;
                return;
            }
        };

        // Build convergence signal snapshot from the task data.
        let snapshot = self.build_snapshot_from_task(&task);

        // Persist the signals.
        if let Err(e) = self
            .federated_goal_repo
            .update_signals(goal.id, snapshot.clone())
            .await
        {
            tracing::warn!(
                goal_id = %goal.id,
                "Failed to update signals: {}",
                e
            );
        }

        // Emit progress event.
        self.event_bus
            .publish(event_factory::federation_event(
                EventSeverity::Info,
                Some(goal.id),
                EventPayload::FederatedGoalProgress {
                    federation_goal_id: goal.id,
                    convergence_level: snapshot.convergence_level,
                    signals: snapshot.signals.clone(),
                },
            ))
            .await;

        // Evaluate state transitions.
        self.evaluate_transition(goal, &snapshot).await;
    }

    /// Build a `ConvergenceSignalSnapshot` by parsing A2A task artifact data,
    /// using the same logic as `SwarmOverseer`.
    fn build_snapshot_from_task(
        &self,
        task: &crate::domain::models::a2a_protocol::A2ATask,
    ) -> ConvergenceSignalSnapshot {
        // Find the latest artifact with convergence data.
        let data_value = task
            .artifacts
            .iter()
            .rev()
            .flat_map(|a| a.parts.iter())
            .find_map(|part| {
                if let A2APart::Data { data, .. } = part {
                    if data.get("convergence_level").is_some() {
                        Some(data.clone())
                    } else {
                        None
                    }
                } else {
                    None
                }
            });

        match data_value {
            Some(data) => {
                let build_passing = data
                    .get("build_passing")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);

                let test_pass_rate = data
                    .get("test_pass_rate")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(0.0);

                let convergence_level = data
                    .get("convergence_level")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(0.0);

                let type_check_clean = data
                    .get("type_check_clean")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);

                let security_issues = data
                    .get("security_issues")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);

                let mut signals = HashMap::new();
                signals.insert(
                    "build_passing".to_string(),
                    if build_passing { 1.0 } else { 0.0 },
                );
                signals.insert("test_pass_rate".to_string(), test_pass_rate);
                signals.insert(
                    "type_check_clean".to_string(),
                    if type_check_clean { 1.0 } else { 0.0 },
                );
                signals.insert("security_issues".to_string(), security_issues as f64);

                // Extract task summary from metadata if available.
                let task_summary = self.extract_task_summary(&data);

                ConvergenceSignalSnapshot {
                    timestamp: self.clock.now(),
                    signals,
                    convergence_level,
                    task_summary,
                }
            }
            None => {
                // No convergence data found — return empty snapshot.
                ConvergenceSignalSnapshot {
                    timestamp: self.clock.now(),
                    signals: HashMap::new(),
                    convergence_level: 0.0,
                    task_summary: TaskStatusSummary::default(),
                }
            }
        }
    }

    /// Extract a `TaskStatusSummary` from the convergence data JSON.
    fn extract_task_summary(&self, data: &serde_json::Value) -> TaskStatusSummary {
        if let Some(summary) = data.get("task_summary") {
            TaskStatusSummary {
                total: summary.get("total").and_then(|v| v.as_u64()).unwrap_or(0) as u32,
                completed: summary
                    .get("completed")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0) as u32,
                failed: summary.get("failed").and_then(|v| v.as_u64()).unwrap_or(0) as u32,
                running: summary.get("running").and_then(|v| v.as_u64()).unwrap_or(0) as u32,
                pending: summary.get("pending").and_then(|v| v.as_u64()).unwrap_or(0) as u32,
            }
        } else {
            TaskStatusSummary::default()
        }
    }

    /// Evaluate and apply state transitions based on the convergence snapshot.
    async fn evaluate_transition(
        &self,
        goal: &FederatedGoal,
        snapshot: &ConvergenceSignalSnapshot,
    ) {
        let current = goal.state;

        // Determine the target state.
        let target = if self.config.max_consecutive_misses > 0
            && self.get_miss_count(goal.id).await >= self.config.max_consecutive_misses
        {
            // Too many misses — fail.
            FederatedGoalState::Failed
        } else if goal.convergence_contract.is_satisfied(snapshot) {
            // Contract satisfied — converge.
            FederatedGoalState::Converged
        } else if snapshot.convergence_level > 0.0 && !snapshot.signals.is_empty() {
            // Positive progress detected.
            match current {
                FederatedGoalState::Delegated => FederatedGoalState::Active,
                FederatedGoalState::Active => FederatedGoalState::Converging,
                FederatedGoalState::Converging => {
                    // Check for regression: if convergence_level dropped compared to
                    // last_signals, regress to Active.
                    if let Some(ref last) = goal.last_signals {
                        if snapshot.convergence_level < last.convergence_level {
                            FederatedGoalState::Active
                        } else {
                            // Still converging, no state change needed.
                            return;
                        }
                    } else {
                        // No prior signals, stay converging.
                        return;
                    }
                }
                _ => return, // No transition needed.
            }
        } else {
            // No progress signal yet.
            match current {
                FederatedGoalState::Delegated => FederatedGoalState::Active,
                _ => return,
            }
        };

        // Check if target is Converged but the path requires intermediate steps.
        // For example, Delegated cannot go directly to Converged; it must go
        // Delegated -> Active -> Converging -> Converged.
        // We handle this by stepping through valid transitions.
        if target == FederatedGoalState::Converged {
            self.step_to_converged(goal, snapshot).await;
            return;
        }

        if target == FederatedGoalState::Failed {
            self.transition_to_failed(goal, "Too many consecutive poll failures")
                .await;
            return;
        }

        // Apply single transition if valid.
        if current.can_transition_to(target) {
            self.apply_transition(goal, target).await;
        }
    }

    /// Step a goal through the state machine toward Converged, respecting
    /// valid transitions.
    async fn step_to_converged(&self, goal: &FederatedGoal, snapshot: &ConvergenceSignalSnapshot) {
        let mut current = goal.state;

        // Walk through the required intermediate states.
        let path: &[FederatedGoalState] = match current {
            FederatedGoalState::Delegated => &[
                FederatedGoalState::Active,
                FederatedGoalState::Converging,
                FederatedGoalState::Converged,
            ],
            FederatedGoalState::Active => &[
                FederatedGoalState::Converging,
                FederatedGoalState::Converged,
            ],
            FederatedGoalState::Converging => &[FederatedGoalState::Converged],
            _ => return,
        };

        for &next in path {
            if !current.can_transition_to(next) {
                tracing::warn!(
                    goal_id = %goal.id,
                    from = current.as_str(),
                    to = next.as_str(),
                    "Invalid transition in step_to_converged"
                );
                return;
            }

            if let Err(e) = self.federated_goal_repo.update_state(goal.id, next).await {
                tracing::warn!(goal_id = %goal.id, "Failed to update state: {}", e);
                return;
            }

            current = next;

            if next == FederatedGoalState::Converged {
                tracing::info!(
                    goal_id = %goal.id,
                    cerebrate_id = %goal.cerebrate_id,
                    convergence_level = snapshot.convergence_level,
                    "Federated goal converged"
                );

                self.event_bus
                    .publish(event_factory::federation_event(
                        EventSeverity::Info,
                        Some(goal.id),
                        EventPayload::FederatedGoalConverged {
                            federation_goal_id: goal.id,
                            cerebrate_id: goal.cerebrate_id.clone(),
                        },
                    ))
                    .await;
            }
        }
    }

    /// Transition a goal to Failed, stepping through valid transitions if needed.
    async fn transition_to_failed(&self, goal: &FederatedGoal, reason: &str) {
        let current = goal.state;

        if current.is_terminal() {
            return;
        }

        if !current.can_transition_to(FederatedGoalState::Failed) {
            tracing::warn!(
                goal_id = %goal.id,
                from = current.as_str(),
                "Cannot transition to Failed from current state"
            );
            return;
        }

        if let Err(e) = self
            .federated_goal_repo
            .update_state(goal.id, FederatedGoalState::Failed)
            .await
        {
            tracing::warn!(goal_id = %goal.id, "Failed to update state to Failed: {}", e);
            return;
        }

        tracing::warn!(
            goal_id = %goal.id,
            cerebrate_id = %goal.cerebrate_id,
            reason = reason,
            "Federated goal failed"
        );

        self.event_bus
            .publish(event_factory::federation_event(
                EventSeverity::Error,
                Some(goal.id),
                EventPayload::FederatedGoalFailed {
                    federation_goal_id: goal.id,
                    cerebrate_id: goal.cerebrate_id.clone(),
                    reason: reason.to_string(),
                },
            ))
            .await;
    }

    /// Apply a single valid state transition.
    async fn apply_transition(&self, goal: &FederatedGoal, target: FederatedGoalState) {
        if let Err(e) = self.federated_goal_repo.update_state(goal.id, target).await {
            tracing::warn!(
                goal_id = %goal.id,
                from = goal.state.as_str(),
                to = target.as_str(),
                "Failed to update state: {}",
                e
            );
        } else {
            tracing::debug!(
                goal_id = %goal.id,
                from = goal.state.as_str(),
                to = target.as_str(),
                "Federated goal state transitioned"
            );
        }
    }

    /// Get the URL for a cerebrate from the federation service.
    async fn get_cerebrate_url(&self, cerebrate_id: &str) -> Option<String> {
        let status = self.federation_service.get_cerebrate(cerebrate_id).await?;
        status.url
    }

    /// Record a poll miss for a goal. If the threshold is exceeded, transition
    /// the goal to Failed.
    async fn record_miss(&self, goal: &FederatedGoal, reason: &str) {
        let count = {
            let mut counters = self.miss_counters.write().await;
            let counter = counters.entry(goal.id).or_insert(0);
            *counter += 1;
            *counter
        };

        tracing::debug!(
            goal_id = %goal.id,
            miss_count = count,
            max = self.config.max_consecutive_misses,
            "Recorded poll miss"
        );

        if count >= self.config.max_consecutive_misses {
            self.transition_to_failed(goal, &format!("{} ({} consecutive misses)", reason, count))
                .await;
        }
    }

    /// Reset the miss counter for a goal after a successful poll.
    async fn reset_miss_counter(&self, goal_id: Uuid) {
        let mut counters = self.miss_counters.write().await;
        counters.remove(&goal_id);
    }

    /// Get the current miss count for a goal.
    async fn get_miss_count(&self, goal_id: Uuid) -> u32 {
        let counters = self.miss_counters.read().await;
        counters.get(&goal_id).copied().unwrap_or(0)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    use std::pin::Pin;
    use std::sync::Mutex;

    use async_trait::async_trait;
    use futures::stream::Stream;

    use crate::adapters::a2a::client::A2AWireError;
    use crate::domain::errors::{DomainError, DomainResult};
    use crate::domain::models::a2a_protocol::{
        A2APart, A2AProtocolArtifact, A2AStandardAgentCard, A2AStreamEvent, A2ATask, A2ATaskState,
        A2ATaskStatus, TaskSendParams,
    };
    use crate::domain::models::goal_federation::*;
    use crate::services::event_bus::{EventBus, EventBusConfig, EventPayload};
    use crate::services::federation::config::FederationConfig;

    // -- Mock A2A Client ------------------------------------------------------

    struct MockA2AClient {
        /// Returns Ok(task) or Err(error) for each call in sequence.
        /// If exhausted, repeats the last entry.
        responses: Mutex<Vec<Result<A2ATask, String>>>,
    }

    impl MockA2AClient {
        fn with_task(task: A2ATask) -> Self {
            Self {
                responses: Mutex::new(vec![Ok(task)]),
            }
        }

        fn with_error(msg: &str) -> Self {
            Self {
                responses: Mutex::new(vec![Err(msg.to_string())]),
            }
        }

        fn with_responses(responses: Vec<Result<A2ATask, String>>) -> Self {
            Self {
                responses: Mutex::new(responses),
            }
        }
    }

    #[async_trait]
    impl A2AClient for MockA2AClient {
        async fn discover(&self, _url: &str) -> Result<A2AStandardAgentCard, A2AWireError> {
            unreachable!(
                "MockA2AClient::discover called — tests in federation/convergence_poller.rs only invoke get_task, so reaching this method indicates the code under test changed to call unexpected A2A methods"
            )
        }

        async fn send_message(
            &self,
            _url: &str,
            _params: TaskSendParams,
        ) -> Result<A2ATask, A2AWireError> {
            unreachable!(
                "MockA2AClient::send_message called — tests in federation/convergence_poller.rs only invoke get_task, so reaching this method indicates the code under test changed to call unexpected A2A methods"
            )
        }

        async fn send_streaming(
            &self,
            _url: &str,
            _params: TaskSendParams,
        ) -> Result<
            Pin<Box<dyn Stream<Item = Result<A2AStreamEvent, A2AWireError>> + Send>>,
            A2AWireError,
        > {
            unreachable!(
                "MockA2AClient::send_streaming called — tests in federation/convergence_poller.rs only invoke get_task, so reaching this method indicates the code under test changed to call unexpected A2A methods"
            )
        }

        async fn get_task(
            &self,
            _url: &str,
            _task_id: &str,
            _history_length: Option<u32>,
        ) -> Result<A2ATask, A2AWireError> {
            let mut responses = self.responses.lock().unwrap();
            let response = if responses.len() > 1 {
                responses.remove(0)
            } else {
                responses
                    .first()
                    .cloned()
                    .unwrap_or(Err("no responses".to_string()))
            };
            match response {
                Ok(task) => Ok(task),
                Err(msg) => Err(A2AWireError::TaskNotFound(msg)),
            }
        }

        async fn cancel_task(&self, _url: &str, _task_id: &str) -> Result<A2ATask, A2AWireError> {
            unreachable!(
                "MockA2AClient::cancel_task called — tests in federation/convergence_poller.rs only invoke get_task, so reaching this method indicates the code under test changed to call unexpected A2A methods"
            )
        }

        async fn subscribe_to_task(
            &self,
            _url: &str,
            _task_id: &str,
        ) -> Result<
            Pin<Box<dyn Stream<Item = Result<A2AStreamEvent, A2AWireError>> + Send>>,
            A2AWireError,
        > {
            unreachable!(
                "MockA2AClient::subscribe_to_task called — tests in federation/convergence_poller.rs only invoke get_task, so reaching this method indicates the code under test changed to call unexpected A2A methods"
            )
        }
    }

    // -- Mock FederatedGoalRepository -----------------------------------------

    struct MockFederatedGoalRepo {
        goals: Mutex<Vec<FederatedGoal>>,
    }

    impl MockFederatedGoalRepo {
        fn new(goals: Vec<FederatedGoal>) -> Self {
            Self {
                goals: Mutex::new(goals),
            }
        }

        fn get_goal(&self, id: Uuid) -> Option<FederatedGoal> {
            let goals = self.goals.lock().unwrap();
            goals.iter().find(|g| g.id == id).cloned()
        }
    }

    #[async_trait]
    impl FederatedGoalRepository for MockFederatedGoalRepo {
        async fn save(&self, goal: &FederatedGoal) -> DomainResult<()> {
            let mut goals = self.goals.lock().unwrap();
            if let Some(existing) = goals.iter_mut().find(|g| g.id == goal.id) {
                *existing = goal.clone();
            } else {
                goals.push(goal.clone());
            }
            Ok(())
        }

        async fn get(&self, id: Uuid) -> DomainResult<Option<FederatedGoal>> {
            let goals = self.goals.lock().unwrap();
            Ok(goals.iter().find(|g| g.id == id).cloned())
        }

        async fn get_by_local_goal(&self, local_goal_id: Uuid) -> DomainResult<Vec<FederatedGoal>> {
            let goals = self.goals.lock().unwrap();
            Ok(goals
                .iter()
                .filter(|g| g.local_goal_id == local_goal_id)
                .cloned()
                .collect())
        }

        async fn get_by_cerebrate(&self, cerebrate_id: &str) -> DomainResult<Vec<FederatedGoal>> {
            let goals = self.goals.lock().unwrap();
            Ok(goals
                .iter()
                .filter(|g| g.cerebrate_id == cerebrate_id)
                .cloned()
                .collect())
        }

        async fn get_active(&self) -> DomainResult<Vec<FederatedGoal>> {
            let goals = self.goals.lock().unwrap();
            Ok(goals
                .iter()
                .filter(|g| !g.state.is_terminal())
                .cloned()
                .collect())
        }

        async fn update_state(&self, id: Uuid, state: FederatedGoalState) -> DomainResult<()> {
            let mut goals = self.goals.lock().unwrap();
            if let Some(goal) = goals.iter_mut().find(|g| g.id == id) {
                goal.state = state;
                goal.updated_at = Utc::now();
                Ok(())
            } else {
                Err(DomainError::GoalNotFound(id))
            }
        }

        async fn update_signals(
            &self,
            id: Uuid,
            signals: ConvergenceSignalSnapshot,
        ) -> DomainResult<()> {
            let mut goals = self.goals.lock().unwrap();
            if let Some(goal) = goals.iter_mut().find(|g| g.id == id) {
                goal.last_signals = Some(signals);
                goal.updated_at = Utc::now();
                Ok(())
            } else {
                Err(DomainError::GoalNotFound(id))
            }
        }

        async fn delete(&self, id: Uuid) -> DomainResult<()> {
            let mut goals = self.goals.lock().unwrap();
            goals.retain(|g| g.id != id);
            Ok(())
        }
    }

    // -- Helpers --------------------------------------------------------------

    fn make_convergence_task(
        convergence_level: f64,
        build_passing: bool,
        test_pass_rate: f64,
    ) -> A2ATask {
        A2ATask {
            id: "task-1".to_string(),
            context_id: None,
            status: A2ATaskStatus {
                state: A2ATaskState::Working,
                message: None,
                timestamp: None,
            },
            history: None,
            artifacts: vec![A2AProtocolArtifact {
                artifact_id: "art-1".to_string(),
                name: Some("convergence-signals".to_string()),
                description: None,
                parts: vec![A2APart::Data {
                    data: serde_json::json!({
                        "build_passing": build_passing,
                        "test_pass_rate": test_pass_rate,
                        "convergence_level": convergence_level,
                        "type_check_clean": true,
                        "security_issues": 0
                    }),
                    metadata: None,
                }],
                metadata: None,
                index: None,
                append: None,
                last_chunk: None,
            }],
            metadata: None,
        }
    }

    fn make_active_goal(cerebrate_id: &str) -> FederatedGoal {
        let mut goal = FederatedGoal::new(Uuid::new_v4(), cerebrate_id, "Test intent");
        goal.state = FederatedGoalState::Active;
        goal.remote_task_id = Some("task-1".to_string());
        goal.convergence_contract = ConvergenceContract {
            required_signals: vec![
                ContractSignal::BuildPassing,
                ContractSignal::ConvergenceLevel { min_level: 0.8 },
            ],
            poll_interval_secs: 30,
        };
        goal
    }

    fn make_federation_service() -> Arc<FederationService> {
        let config = FederationConfig::default();
        let event_bus = Arc::new(EventBus::new(EventBusConfig::default()));
        Arc::new(FederationService::new(config, event_bus))
    }

    async fn make_daemon_with_goal(
        goal: FederatedGoal,
        client: Arc<dyn A2AClient>,
    ) -> (
        ConvergencePollingDaemon,
        Arc<MockFederatedGoalRepo>,
        Arc<EventBus>,
    ) {
        let repo = Arc::new(MockFederatedGoalRepo::new(vec![goal]));
        let event_bus = Arc::new(EventBus::new(EventBusConfig::default()));
        let fed_service = make_federation_service();

        // Register cerebrate so get_cerebrate_url works.
        fed_service
            .register_cerebrate("test-cerebrate", "Test Cerebrate", "http://localhost:9090")
            .await;

        let daemon = ConvergencePollingDaemon::new(
            fed_service,
            client,
            repo.clone(),
            event_bus.clone(),
            ConvergencePollerConfig {
                poll_interval: Duration::from_millis(50),
                max_consecutive_misses: 5,
            },
        );

        (daemon, repo, event_bus)
    }

    // -- Tests ----------------------------------------------------------------

    #[tokio::test]
    async fn test_poll_goal_transitions_to_converged() {
        // Full convergence: build passing, convergence_level = 0.95
        let task = make_convergence_task(0.95, true, 1.0);
        let client = Arc::new(MockA2AClient::with_task(task));

        let goal = make_active_goal("test-cerebrate");
        let goal_id = goal.id;

        let (daemon, repo, event_bus) = make_daemon_with_goal(goal, client).await;
        let mut subscriber = event_bus.subscribe();

        // Poll once.
        daemon.poll_all_goals().await.unwrap();

        // Goal should be Converged (Active -> Converging -> Converged).
        let updated = repo.get_goal(goal_id).unwrap();
        assert_eq!(
            updated.state,
            FederatedGoalState::Converged,
            "Goal should be Converged after contract is satisfied"
        );

        // Check for the FederatedGoalConverged event.
        let mut saw_converged = false;
        while let Ok(event) = subscriber.try_recv() {
            if let EventPayload::FederatedGoalConverged {
                federation_goal_id,
                cerebrate_id,
            } = event.payload
            {
                assert_eq!(federation_goal_id, goal_id);
                assert_eq!(cerebrate_id, "test-cerebrate");
                saw_converged = true;
            }
        }
        assert!(
            saw_converged,
            "Should have emitted FederatedGoalConverged event"
        );
    }

    #[tokio::test]
    async fn test_poll_goal_stays_active_when_contract_not_satisfied() {
        // Low convergence: build passing, convergence_level = 0.3 (below 0.8 threshold)
        let task = make_convergence_task(0.3, true, 1.0);
        let client = Arc::new(MockA2AClient::with_task(task));

        let goal = make_active_goal("test-cerebrate");
        let goal_id = goal.id;

        let (daemon, repo, _event_bus) = make_daemon_with_goal(goal, client).await;

        daemon.poll_all_goals().await.unwrap();

        // Should transition Active -> Converging (positive progress, convergence_level > 0)
        // but NOT to Converged since contract is not satisfied.
        let updated = repo.get_goal(goal_id).unwrap();
        assert_eq!(
            updated.state,
            FederatedGoalState::Converging,
            "Goal should be Converging (positive progress but contract not satisfied)"
        );
    }

    #[tokio::test]
    async fn test_poll_errors_cause_failure_after_threshold() {
        let client = Arc::new(MockA2AClient::with_error("connection refused"));

        let goal = make_active_goal("test-cerebrate");
        let goal_id = goal.id;

        let repo = Arc::new(MockFederatedGoalRepo::new(vec![goal]));
        let event_bus = Arc::new(EventBus::new(EventBusConfig::default()));
        let fed_service = make_federation_service();

        fed_service
            .register_cerebrate("test-cerebrate", "Test Cerebrate", "http://localhost:9090")
            .await;

        let daemon = ConvergencePollingDaemon::new(
            fed_service,
            client,
            repo.clone(),
            event_bus.clone(),
            ConvergencePollerConfig {
                poll_interval: Duration::from_millis(50),
                max_consecutive_misses: 3, // Fail after 3 misses.
            },
        );

        let mut subscriber = event_bus.subscribe();

        // Poll 3 times to exceed the miss threshold.
        for _ in 0..3 {
            daemon.poll_all_goals().await.unwrap();
        }

        let updated = repo.get_goal(goal_id).unwrap();
        assert_eq!(
            updated.state,
            FederatedGoalState::Failed,
            "Goal should be Failed after {} consecutive misses",
            3
        );

        // Check for the FederatedGoalFailed event.
        let mut saw_failed = false;
        while let Ok(event) = subscriber.try_recv() {
            if let EventPayload::FederatedGoalFailed {
                federation_goal_id,
                reason,
                ..
            } = event.payload
            {
                assert_eq!(federation_goal_id, goal_id);
                assert!(reason.contains("consecutive misses"));
                saw_failed = true;
            }
        }
        assert!(saw_failed, "Should have emitted FederatedGoalFailed event");
    }

    #[tokio::test]
    async fn test_state_transitions_respect_can_transition_to() {
        // Goal in Delegated state should go to Active first, not directly to Converging.
        let task = make_convergence_task(0.5, true, 1.0);
        let client = Arc::new(MockA2AClient::with_task(task));

        let mut goal = make_active_goal("test-cerebrate");
        goal.state = FederatedGoalState::Delegated;
        let goal_id = goal.id;

        let (daemon, repo, _event_bus) = make_daemon_with_goal(goal, client).await;

        // First poll: Delegated -> Active.
        daemon.poll_all_goals().await.unwrap();
        let updated = repo.get_goal(goal_id).unwrap();
        assert_eq!(
            updated.state,
            FederatedGoalState::Active,
            "Delegated should transition to Active first"
        );
    }

    #[tokio::test]
    async fn test_converging_to_converged_when_contract_satisfied() {
        let task = make_convergence_task(0.95, true, 1.0);
        let client = Arc::new(MockA2AClient::with_task(task));

        let mut goal = make_active_goal("test-cerebrate");
        goal.state = FederatedGoalState::Converging;
        let goal_id = goal.id;

        let (daemon, repo, _event_bus) = make_daemon_with_goal(goal, client).await;

        daemon.poll_all_goals().await.unwrap();

        let updated = repo.get_goal(goal_id).unwrap();
        assert_eq!(
            updated.state,
            FederatedGoalState::Converged,
            "Converging should transition to Converged when contract satisfied"
        );
    }

    #[tokio::test]
    async fn test_regression_transitions_converging_to_active() {
        let task = make_convergence_task(0.3, true, 1.0);
        let client = Arc::new(MockA2AClient::with_task(task));

        let mut goal = make_active_goal("test-cerebrate");
        goal.state = FederatedGoalState::Converging;
        // Set previous signals with higher convergence level.
        goal.last_signals = Some(ConvergenceSignalSnapshot {
            timestamp: Utc::now(),
            signals: HashMap::from([
                ("build_passing".to_string(), 1.0),
                ("test_pass_rate".to_string(), 1.0),
            ]),
            convergence_level: 0.7,
            task_summary: TaskStatusSummary::default(),
        });
        let goal_id = goal.id;

        let (daemon, repo, _event_bus) = make_daemon_with_goal(goal, client).await;

        daemon.poll_all_goals().await.unwrap();

        let updated = repo.get_goal(goal_id).unwrap();
        assert_eq!(
            updated.state,
            FederatedGoalState::Active,
            "Should regress to Active when convergence_level drops"
        );
    }

    #[tokio::test]
    async fn test_miss_counter_resets_on_success() {
        // First two calls fail, third succeeds.
        let task = make_convergence_task(0.5, true, 1.0);
        let responses = vec![
            Err("timeout".to_string()),
            Err("timeout".to_string()),
            Ok(task),
        ];
        let client = Arc::new(MockA2AClient::with_responses(responses));

        let goal = make_active_goal("test-cerebrate");
        let goal_id = goal.id;

        let repo = Arc::new(MockFederatedGoalRepo::new(vec![goal]));
        let event_bus = Arc::new(EventBus::new(EventBusConfig::default()));
        let fed_service = make_federation_service();

        fed_service
            .register_cerebrate("test-cerebrate", "Test Cerebrate", "http://localhost:9090")
            .await;

        let daemon = ConvergencePollingDaemon::new(
            fed_service,
            client,
            repo.clone(),
            event_bus,
            ConvergencePollerConfig {
                poll_interval: Duration::from_millis(50),
                max_consecutive_misses: 5,
            },
        );

        // Poll twice (failures).
        daemon.poll_all_goals().await.unwrap();
        assert_eq!(daemon.get_miss_count(goal_id).await, 1);
        daemon.poll_all_goals().await.unwrap();
        assert_eq!(daemon.get_miss_count(goal_id).await, 2);

        // Third poll succeeds, counter should reset.
        daemon.poll_all_goals().await.unwrap();
        assert_eq!(
            daemon.get_miss_count(goal_id).await,
            0,
            "Miss counter should reset to 0 after successful poll"
        );

        // Goal should not be Failed.
        let updated = repo.get_goal(goal_id).unwrap();
        assert_ne!(updated.state, FederatedGoalState::Failed);
    }

    #[tokio::test]
    async fn test_terminal_goals_are_not_polled() {
        let task = make_convergence_task(0.5, true, 1.0);
        let client = Arc::new(MockA2AClient::with_task(task));

        let mut goal = make_active_goal("test-cerebrate");
        goal.state = FederatedGoalState::Converged; // Terminal state.
        let goal_id = goal.id;

        let (daemon, repo, _event_bus) = make_daemon_with_goal(goal, client).await;

        // get_active() should filter out terminal goals.
        daemon.poll_all_goals().await.unwrap();

        let updated = repo.get_goal(goal_id).unwrap();
        assert_eq!(
            updated.state,
            FederatedGoalState::Converged,
            "Terminal goal should not be polled or modified"
        );
    }

    #[tokio::test]
    async fn test_goal_without_remote_task_id_is_skipped() {
        let task = make_convergence_task(0.5, true, 1.0);
        let client = Arc::new(MockA2AClient::with_task(task));

        let mut goal = make_active_goal("test-cerebrate");
        goal.remote_task_id = None; // No remote task ID.
        let goal_id = goal.id;

        let (daemon, repo, _event_bus) = make_daemon_with_goal(goal, client).await;

        daemon.poll_all_goals().await.unwrap();

        // Goal should remain Active (not transitioned).
        let updated = repo.get_goal(goal_id).unwrap();
        assert_eq!(updated.state, FederatedGoalState::Active);
    }

    #[tokio::test]
    async fn test_progress_event_emitted_on_poll() {
        let task = make_convergence_task(0.5, true, 0.9);
        let client = Arc::new(MockA2AClient::with_task(task));

        let goal = make_active_goal("test-cerebrate");
        let goal_id = goal.id;

        let (daemon, _repo, event_bus) = make_daemon_with_goal(goal, client).await;
        let mut subscriber = event_bus.subscribe();

        daemon.poll_all_goals().await.unwrap();

        let mut saw_progress = false;
        while let Ok(event) = subscriber.try_recv() {
            if let EventPayload::FederatedGoalProgress {
                federation_goal_id,
                convergence_level,
                signals,
            } = event.payload
            {
                assert_eq!(federation_goal_id, goal_id);
                assert!((convergence_level - 0.5).abs() < 0.01);
                assert!(signals.contains_key("build_passing"));
                saw_progress = true;
            }
        }
        assert!(
            saw_progress,
            "Should have emitted FederatedGoalProgress event"
        );
    }

    #[tokio::test]
    async fn test_build_snapshot_with_no_convergence_data() {
        let task = A2ATask {
            id: "task-1".to_string(),
            context_id: None,
            status: A2ATaskStatus {
                state: A2ATaskState::Working,
                message: None,
                timestamp: None,
            },
            history: None,
            artifacts: vec![],
            metadata: None,
        };

        let daemon = ConvergencePollingDaemon::new(
            make_federation_service(),
            Arc::new(MockA2AClient::with_task(task.clone())),
            Arc::new(MockFederatedGoalRepo::new(vec![])),
            Arc::new(EventBus::new(EventBusConfig::default())),
            ConvergencePollerConfig::default(),
        );

        let snapshot = daemon.build_snapshot_from_task(&task);
        assert!(snapshot.signals.is_empty());
        assert_eq!(snapshot.convergence_level, 0.0);
    }
}
