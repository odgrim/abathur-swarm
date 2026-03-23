//! Child Convergence Signal Publisher.
//!
//! A background daemon running on the child swarm that periodically snapshots
//! local convergence state and attaches it as an `A2AArtifact` on the federated
//! `InMemoryTask`. The parent's `SwarmOverseer` can then poll the task to read
//! the latest convergence signal.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use serde_json::{json, Value};
use tokio::sync::RwLock;
use tokio::time::interval;
use uuid::Uuid;

use crate::adapters::mcp::a2a_http::{
    A2AArtifact, A2ATaskState, InMemoryTask, MessagePart,
};
use crate::domain::ports::TrajectoryRepository;

/// Handle to stop the convergence publisher daemon.
pub struct ConvergencePublisherHandle {
    stop_flag: Arc<AtomicBool>,
}

impl ConvergencePublisherHandle {
    /// Request the publisher to stop.
    pub fn stop(&self) {
        self.stop_flag.store(true, Ordering::Release);
    }

    /// Check if stop was requested.
    pub fn is_stop_requested(&self) -> bool {
        self.stop_flag.load(Ordering::Acquire)
    }
}

/// Background daemon that publishes convergence signal artifacts on
/// federated `goal_delegate` tasks so the parent swarm can poll them.
pub struct ConvergencePublisher {
    tasks: Arc<RwLock<HashMap<String, InMemoryTask>>>,
    trajectory_repo: Option<Arc<dyn TrajectoryRepository>>,
    poll_interval: Duration,
    stop_flag: Arc<AtomicBool>,
}

/// The convergence signal snapshot that gets serialized into the artifact.
#[derive(Debug, Clone)]
pub struct ConvergenceSnapshot {
    pub convergence_level: f64,
    pub build_passing: bool,
    pub test_pass_rate: f64,
    pub type_check_clean: bool,
    pub security_issues: u64,
}

impl Default for ConvergenceSnapshot {
    fn default() -> Self {
        Self {
            convergence_level: 0.0,
            build_passing: false,
            test_pass_rate: 0.0,
            type_check_clean: false,
            security_issues: 0,
        }
    }
}

impl ConvergenceSnapshot {
    /// Serialize to the JSON format that `SwarmOverseer` expects.
    pub fn to_json(&self) -> Value {
        json!({
            "convergence_level": self.convergence_level,
            "build_passing": self.build_passing,
            "test_pass_rate": self.test_pass_rate,
            "type_check_clean": self.type_check_clean,
            "security_issues": self.security_issues,
        })
    }
}

impl ConvergencePublisher {
    /// Create a new convergence publisher.
    pub fn new(
        tasks: Arc<RwLock<HashMap<String, InMemoryTask>>>,
        poll_interval: Duration,
    ) -> Self {
        Self {
            tasks,
            trajectory_repo: None,
            poll_interval,
            stop_flag: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Attach a trajectory repository for real convergence signal computation.
    ///
    /// Without this, the publisher falls back to default (zero) snapshots.
    pub fn with_trajectory_repo(mut self, repo: Arc<dyn TrajectoryRepository>) -> Self {
        self.trajectory_repo = Some(repo);
        self
    }

    /// Get a handle to control the publisher.
    pub fn handle(&self) -> ConvergencePublisherHandle {
        ConvergencePublisherHandle {
            stop_flag: self.stop_flag.clone(),
        }
    }

    /// Spawn the publisher as a background task. Returns a handle for stopping.
    pub fn spawn(self) -> ConvergencePublisherHandle {
        let handle = self.handle();
        tokio::spawn(async move {
            self.run_loop().await;
        });
        handle
    }

    /// Main loop: periodically scan tasks and publish convergence artifacts.
    async fn run_loop(self) {
        let mut timer = interval(self.poll_interval);

        loop {
            timer.tick().await;

            if self.stop_flag.load(Ordering::Acquire) {
                tracing::info!("ConvergencePublisher stopping (stop flag set)");
                break;
            }

            self.publish_tick().await;
        }
    }

    /// Single tick: scan all goal_delegate Working tasks and update their artifacts.
    async fn publish_tick(&self) {
        let mut tasks = self.tasks.write().await;

        let task_ids: Vec<String> = tasks
            .iter()
            .filter(|(_, task)| is_working_goal_delegate(task))
            .map(|(id, _)| id.clone())
            .collect();

        for task_id in task_ids {
            if let Some(task) = tasks.get_mut(&task_id) {
                let snapshot = compute_convergence_snapshot(task, self.trajectory_repo.as_ref()).await;
                let artifact = build_convergence_artifact(&snapshot);
                upsert_convergence_artifact(task, artifact);
                task.updated_at = Utc::now();
            }
        }
    }
}

/// Check whether a task is a `goal_delegate` in `Working` state.
pub fn is_working_goal_delegate(task: &InMemoryTask) -> bool {
    if !matches!(task.state, A2ATaskState::Working) {
        return false;
    }

    task.metadata
        .as_ref()
        .and_then(|m| m.get("abathur:federation"))
        .and_then(|fed| fed.get("intent"))
        .and_then(|v| v.as_str())
        .map(|intent| intent == "goal_delegate")
        .unwrap_or(false)
}

/// Extract the `local_goal_id` from a federation task's metadata.
fn extract_goal_id(task: &InMemoryTask) -> Option<String> {
    task.metadata
        .as_ref()?
        .get("abathur:federation")?
        .get("parent_goal_id")
        .or_else(|| task.metadata.as_ref()?.get("abathur:federation")?.get("goal_id"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

/// Compute the convergence snapshot for a task.
///
/// When a `TrajectoryRepository` is available, queries the latest trajectory
/// for the task's associated goal and extracts real overseer signals and
/// convergence metrics. Falls back to defaults if no trajectory exists yet
/// or no repository is provided.
pub async fn compute_convergence_snapshot(
    task: &InMemoryTask,
    trajectory_repo: Option<&Arc<dyn TrajectoryRepository>>,
) -> ConvergenceSnapshot {
    let Some(repo) = trajectory_repo else {
        return ConvergenceSnapshot::default();
    };

    let Some(goal_id) = extract_goal_id(task) else {
        tracing::debug!(task_id = %task.id, "No goal_id in federation metadata, using default snapshot");
        return ConvergenceSnapshot::default();
    };

    let trajectories = match repo.get_by_goal(&goal_id).await {
        Ok(t) => t,
        Err(e) => {
            tracing::warn!(goal_id = %goal_id, error = %e, "Failed to query trajectories");
            return ConvergenceSnapshot::default();
        }
    };

    // Find the most recent observation across all trajectories for this goal.
    let latest_obs = trajectories
        .iter()
        .flat_map(|t| t.observations.iter())
        .max_by_key(|o| o.timestamp);

    let Some(obs) = latest_obs else {
        return ConvergenceSnapshot::default();
    };

    let signals = &obs.overseer_signals;

    let build_passing = signals
        .build_result
        .as_ref()
        .map(|b| b.success)
        .unwrap_or(false);

    let test_pass_rate = signals
        .test_results
        .as_ref()
        .map(|t| {
            let total = t.passed + t.failed;
            if total == 0 { 0.0 } else { t.passed as f64 / total as f64 }
        })
        .unwrap_or(0.0);

    let type_check_clean = signals
        .type_check
        .as_ref()
        .map(|t| t.clean)
        .unwrap_or(false);

    let security_issues = signals
        .security_scan
        .as_ref()
        .map(|s| (s.critical_count + s.high_count + s.medium_count) as u64)
        .unwrap_or(0);

    let convergence_level = obs
        .metrics
        .as_ref()
        .map(|m| m.intent_blended_level.unwrap_or(m.convergence_level))
        .unwrap_or(0.0);

    ConvergenceSnapshot {
        convergence_level,
        build_passing,
        test_pass_rate,
        type_check_clean,
        security_issues,
    }
}

/// Build an `A2AArtifact` containing the convergence signal data.
pub fn build_convergence_artifact(snapshot: &ConvergenceSnapshot) -> A2AArtifact {
    let data = snapshot.to_json();

    A2AArtifact {
        id: Uuid::new_v4().to_string(),
        name: "convergence_signal".to_string(),
        description: Some("Child swarm convergence signal snapshot".to_string()),
        mime_type: "application/json".to_string(),
        parts: vec![MessagePart::Data {
            mime_type: "application/json".to_string(),
            data,
        }],
        metadata: Some(json!({ "signal_type": "convergence" })),
        index: 0,
        append: false,
        last_chunk: true,
    }
}

/// Insert or replace the convergence artifact on a task.
///
/// If the task already has an artifact named `"convergence_signal"`, it is
/// replaced in-place. Otherwise the new artifact is appended.
pub fn upsert_convergence_artifact(task: &mut InMemoryTask, artifact: A2AArtifact) {
    if let Some(pos) = task
        .artifacts
        .iter()
        .position(|a| a.name == "convergence_signal")
    {
        task.artifacts[pos] = artifact;
    } else {
        task.artifacts.push(artifact);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use serde_json::json;

    /// Helper to create a mock `InMemoryTask` with given state and metadata.
    fn make_task(state: A2ATaskState, metadata: Option<Value>) -> InMemoryTask {
        let now = Utc::now();
        InMemoryTask {
            id: Uuid::new_v4().to_string(),
            session_id: Uuid::new_v4().to_string(),
            state,
            history: vec![],
            artifacts: vec![],
            metadata,
            created_at: now,
            updated_at: now,
            push_config: None,
        }
    }

    fn goal_delegate_metadata() -> Value {
        json!({
            "abathur:federation": {
                "intent": "goal_delegate",
                "goal_id": "test-goal-1",
                "goal_name": "Test Goal",
                "goal_description": "A test goal",
                "priority": "normal",
                "constraints": [],
                "convergence_contract": {}
            }
        })
    }

    #[test]
    fn test_builds_correct_artifact_format() {
        let snapshot = ConvergenceSnapshot {
            convergence_level: 0.73,
            build_passing: true,
            test_pass_rate: 0.95,
            type_check_clean: true,
            security_issues: 0,
        };

        let artifact = build_convergence_artifact(&snapshot);

        assert_eq!(artifact.name, "convergence_signal");
        assert_eq!(artifact.mime_type, "application/json");
        assert_eq!(artifact.parts.len(), 1);
        assert!(artifact.last_chunk);

        match &artifact.parts[0] {
            MessagePart::Data { mime_type, data } => {
                assert_eq!(mime_type, "application/json");
                assert_eq!(data["convergence_level"], 0.73);
                assert_eq!(data["build_passing"], true);
                assert_eq!(data["test_pass_rate"], 0.95);
                assert_eq!(data["type_check_clean"], true);
                assert_eq!(data["security_issues"], 0);
            }
            _ => panic!("Expected MessagePart::Data"),
        }
    }

    #[test]
    fn test_only_processes_goal_delegate_working_tasks() {
        let working_delegate = make_task(
            A2ATaskState::Working,
            Some(goal_delegate_metadata()),
        );
        assert!(is_working_goal_delegate(&working_delegate));

        // Non-delegate working task
        let working_other = make_task(
            A2ATaskState::Working,
            Some(json!({ "abathur:federation": { "intent": "other" } })),
        );
        assert!(!is_working_goal_delegate(&working_other));

        // No metadata at all
        let working_no_meta = make_task(A2ATaskState::Working, None);
        assert!(!is_working_goal_delegate(&working_no_meta));
    }

    #[test]
    fn test_skips_non_working_states() {
        let meta = Some(goal_delegate_metadata());

        let completed = make_task(A2ATaskState::Completed, meta.clone());
        assert!(!is_working_goal_delegate(&completed));

        let failed = make_task(A2ATaskState::Failed, meta.clone());
        assert!(!is_working_goal_delegate(&failed));

        let submitted = make_task(A2ATaskState::Submitted, meta.clone());
        assert!(!is_working_goal_delegate(&submitted));

        let canceled = make_task(A2ATaskState::Canceled, meta);
        assert!(!is_working_goal_delegate(&canceled));
    }

    #[test]
    fn test_artifact_data_matches_swarm_overseer_expectations() {
        // SwarmOverseer looks for data.get("convergence_level").is_some()
        // and extracts: convergence_level, build_passing, test_pass_rate,
        // type_check_clean, security_issues
        let snapshot = ConvergenceSnapshot::default();
        let artifact = build_convergence_artifact(&snapshot);

        let data = match &artifact.parts[0] {
            MessagePart::Data { data, .. } => data,
            _ => panic!("Expected MessagePart::Data"),
        };

        // All required keys must be present
        assert!(data.get("convergence_level").is_some());
        assert!(data.get("build_passing").is_some());
        assert!(data.get("test_pass_rate").is_some());
        assert!(data.get("type_check_clean").is_some());
        assert!(data.get("security_issues").is_some());

        // Verify types
        assert!(data["convergence_level"].is_f64());
        assert!(data["build_passing"].is_boolean());
        assert!(data["test_pass_rate"].is_f64());
        assert!(data["type_check_clean"].is_boolean());
        assert!(data["security_issues"].is_number());
    }

    #[test]
    fn test_upsert_replaces_existing_convergence_artifact() {
        let mut task = make_task(
            A2ATaskState::Working,
            Some(goal_delegate_metadata()),
        );

        let snapshot1 = ConvergenceSnapshot {
            convergence_level: 0.3,
            ..Default::default()
        };
        let artifact1 = build_convergence_artifact(&snapshot1);
        upsert_convergence_artifact(&mut task, artifact1);
        assert_eq!(task.artifacts.len(), 1);

        let snapshot2 = ConvergenceSnapshot {
            convergence_level: 0.7,
            build_passing: true,
            ..Default::default()
        };
        let artifact2 = build_convergence_artifact(&snapshot2);
        upsert_convergence_artifact(&mut task, artifact2);

        // Should still be 1 artifact (replaced, not appended)
        assert_eq!(task.artifacts.len(), 1);

        match &task.artifacts[0].parts[0] {
            MessagePart::Data { data, .. } => {
                assert_eq!(data["convergence_level"], 0.7);
                assert_eq!(data["build_passing"], true);
            }
            _ => panic!("Expected MessagePart::Data"),
        }
    }

    #[test]
    fn test_snapshot_to_json_roundtrip() {
        let snapshot = ConvergenceSnapshot {
            convergence_level: 0.85,
            build_passing: true,
            test_pass_rate: 0.99,
            type_check_clean: true,
            security_issues: 2,
        };

        let json = snapshot.to_json();
        assert_eq!(json["convergence_level"].as_f64().unwrap(), 0.85);
        assert_eq!(json["build_passing"].as_bool().unwrap(), true);
        assert_eq!(json["test_pass_rate"].as_f64().unwrap(), 0.99);
        assert_eq!(json["type_check_clean"].as_bool().unwrap(), true);
        assert_eq!(json["security_issues"].as_u64().unwrap(), 2);
    }

    #[tokio::test]
    async fn test_publish_tick_updates_only_eligible_tasks() {
        let mut tasks_map = HashMap::new();

        // Working goal_delegate task — should get artifact
        let eligible = make_task(
            A2ATaskState::Working,
            Some(goal_delegate_metadata()),
        );
        let eligible_id = eligible.id.clone();
        tasks_map.insert(eligible_id.clone(), eligible);

        // Completed goal_delegate task — should NOT get artifact
        let completed = make_task(
            A2ATaskState::Completed,
            Some(goal_delegate_metadata()),
        );
        let completed_id = completed.id.clone();
        tasks_map.insert(completed_id.clone(), completed);

        // Working non-delegate task — should NOT get artifact
        let non_delegate = make_task(
            A2ATaskState::Working,
            Some(json!({ "something": "else" })),
        );
        let non_delegate_id = non_delegate.id.clone();
        tasks_map.insert(non_delegate_id.clone(), non_delegate);

        let tasks = Arc::new(RwLock::new(tasks_map));
        let publisher = ConvergencePublisher::new(tasks.clone(), Duration::from_secs(5));

        publisher.publish_tick().await;

        let tasks_guard = tasks.read().await;
        assert_eq!(tasks_guard[&eligible_id].artifacts.len(), 1);
        assert_eq!(tasks_guard[&completed_id].artifacts.len(), 0);
        assert_eq!(tasks_guard[&non_delegate_id].artifacts.len(), 0);
    }
}
