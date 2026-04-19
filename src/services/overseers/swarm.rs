//! SwarmOverseer — treats child swarm convergence as an overseer signal.
//!
//! Polls a child swarm's A2A task status and converts convergence artifacts
//! into OverseerSignals that the parent's convergence engine can use.

use std::sync::Arc;

use async_trait::async_trait;

use crate::adapters::a2a::client::A2AClient;
use crate::domain::models::a2a_protocol::A2APart;
use crate::domain::models::convergence::Overseer;
use crate::domain::models::convergence::{
    ArtifactReference, BuildResult, CustomCheckResult, OverseerCost, OverseerResult,
    OverseerSignalUpdate, TestResults,
};

// ---------------------------------------------------------------------------
// SwarmOverseer
// ---------------------------------------------------------------------------

/// An overseer that polls a child swarm's A2A task status and converts
/// convergence artifacts into [`OverseerResult`]s for the parent convergence
/// engine.
///
/// The child swarm is expected to publish convergence signal data as a
/// JSON [`A2APart::Data`] part in its task artifacts. The expected JSON
/// shape is:
///
/// ```json
/// {
///   "build_passing": true,
///   "test_pass_rate": 0.95,
///   "convergence_level": 0.87,
///   "type_check_clean": true,
///   "security_issues": 0
/// }
/// ```
pub struct SwarmOverseer {
    cerebrate_id: String,
    remote_task_id: String,
    a2a_client: Arc<dyn A2AClient>,
    remote_url: String,
}

impl SwarmOverseer {
    /// Create a new `SwarmOverseer`.
    ///
    /// # Arguments
    ///
    /// * `cerebrate_id` — identifier of the child cerebrate whose task we poll.
    /// * `remote_task_id` — the A2A task ID on the remote cerebrate.
    /// * `remote_url` — base URL for the remote cerebrate's A2A endpoint.
    /// * `a2a_client` — shared A2A client used to poll the task.
    pub fn new(
        cerebrate_id: impl Into<String>,
        remote_task_id: impl Into<String>,
        remote_url: impl Into<String>,
        a2a_client: Arc<dyn A2AClient>,
    ) -> Self {
        Self {
            cerebrate_id: cerebrate_id.into(),
            remote_task_id: remote_task_id.into(),
            remote_url: remote_url.into(),
            a2a_client,
        }
    }
}

#[async_trait]
impl Overseer for SwarmOverseer {
    fn name(&self) -> &str {
        &self.cerebrate_id
    }

    fn cost(&self) -> OverseerCost {
        OverseerCost::Cheap
    }

    async fn measure(&self, _artifact: &ArtifactReference) -> anyhow::Result<OverseerResult> {
        // Poll the child swarm's task status via A2A.
        let task = match self
            .a2a_client
            .get_task(&self.remote_url, &self.remote_task_id, None)
            .await
        {
            Ok(task) => task,
            Err(err) => {
                // On A2A client error, return a failing custom check.
                return Ok(OverseerResult {
                    pass: false,
                    signal: OverseerSignalUpdate::CustomCheck(CustomCheckResult {
                        name: format!("swarm_{}", self.cerebrate_id),
                        passed: false,
                        details: format!("A2A poll failed: {err}"),
                    }),
                });
            }
        };

        // Find the latest artifact with a Data part containing convergence signals.
        let data_value = task
            .artifacts
            .iter()
            .rev()
            .flat_map(|a| a.parts.iter())
            .find_map(|part| {
                if let A2APart::Data { data, .. } = part {
                    // Only accept objects that look like convergence signals.
                    if data.get("convergence_level").is_some() {
                        Some(data.clone())
                    } else {
                        None
                    }
                } else {
                    None
                }
            });

        let data = match data_value {
            Some(d) => d,
            None => {
                // No convergence data found — report as a non-passing custom check.
                return Ok(OverseerResult {
                    pass: false,
                    signal: OverseerSignalUpdate::CustomCheck(CustomCheckResult {
                        name: format!("swarm_{}", self.cerebrate_id),
                        passed: false,
                        details: "No convergence signal data found in task artifacts".to_string(),
                    }),
                });
            }
        };

        // Extract signal values from the JSON data, using sensible defaults.
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

        // Determine overall pass: all signals must be good.
        let all_good = build_passing
            && type_check_clean
            && test_pass_rate >= 0.99
            && convergence_level >= 0.8
            && security_issues == 0;

        // Choose the most important signal update to return.
        // Priority: build failure > test failure > convergence check.
        let signal = if !build_passing {
            OverseerSignalUpdate::BuildResult(BuildResult {
                success: false,
                error_count: 0,
                errors: vec![],
            })
        } else if test_pass_rate < 0.99 {
            let passed = (test_pass_rate * 100.0) as u32;
            let failed = ((1.0 - test_pass_rate) * 100.0) as u32;
            OverseerSignalUpdate::TestResults(TestResults {
                passed,
                failed,
                skipped: 0,
                total: 100,
                regression_count: 0,
                failing_test_names: vec![],
            })
        } else {
            // Convergence level custom check (also covers the all-good case).
            OverseerSignalUpdate::CustomCheck(CustomCheckResult {
                name: format!("swarm_{}", self.cerebrate_id),
                passed: convergence_level >= 0.8,
                details: format!("convergence_level={convergence_level:.2}"),
            })
        };

        Ok(OverseerResult {
            pass: all_good,
            signal,
        })
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::pin::Pin;

    use async_trait::async_trait;
    use futures::stream::Stream;

    use crate::adapters::a2a::client::{A2AClient, A2AWireError};
    use crate::domain::models::a2a_protocol::{
        A2AProtocolArtifact, A2AStandardAgentCard, A2AStreamEvent, A2ATask, A2ATaskState,
        A2ATaskStatus, TaskSendParams,
    };

    // -- Mock A2A client -----------------------------------------------------

    struct MockA2AClient {
        task: Result<A2ATask, String>,
    }

    #[async_trait]
    impl A2AClient for MockA2AClient {
        async fn discover(&self, _url: &str) -> Result<A2AStandardAgentCard, A2AWireError> {
            unreachable!(
                "MockA2AClient::discover called — tests in overseers/swarm.rs only invoke get_task, so reaching this method indicates the code under test changed to call unexpected A2A methods"
            )
        }

        async fn send_message(
            &self,
            _url: &str,
            _params: TaskSendParams,
        ) -> Result<A2ATask, A2AWireError> {
            unreachable!(
                "MockA2AClient::send_message called — tests in overseers/swarm.rs only invoke get_task, so reaching this method indicates the code under test changed to call unexpected A2A methods"
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
                "MockA2AClient::send_streaming called — tests in overseers/swarm.rs only invoke get_task, so reaching this method indicates the code under test changed to call unexpected A2A methods"
            )
        }

        async fn get_task(
            &self,
            _url: &str,
            _task_id: &str,
            _history_length: Option<u32>,
        ) -> Result<A2ATask, A2AWireError> {
            match &self.task {
                Ok(task) => Ok(task.clone()),
                Err(msg) => Err(A2AWireError::TaskNotFound(msg.clone())),
            }
        }

        async fn cancel_task(&self, _url: &str, _task_id: &str) -> Result<A2ATask, A2AWireError> {
            unreachable!(
                "MockA2AClient::cancel_task called — tests in overseers/swarm.rs only invoke get_task, so reaching this method indicates the code under test changed to call unexpected A2A methods"
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
                "MockA2AClient::subscribe_to_task called — tests in overseers/swarm.rs only invoke get_task, so reaching this method indicates the code under test changed to call unexpected A2A methods"
            )
        }
    }

    fn test_artifact() -> ArtifactReference {
        ArtifactReference::new("/test/path", "hash123")
    }

    fn make_task_with_data(data: serde_json::Value) -> A2ATask {
        A2ATask {
            id: "task-1".to_string(),
            context_id: None,
            status: A2ATaskStatus {
                state: A2ATaskState::Completed,
                message: None,
                timestamp: None,
            },
            history: None,
            artifacts: vec![A2AProtocolArtifact {
                artifact_id: "art-1".to_string(),
                name: Some("convergence-signals".to_string()),
                description: None,
                parts: vec![A2APart::Data {
                    data,
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

    #[tokio::test]
    async fn all_good_signals_pass() {
        let task = make_task_with_data(serde_json::json!({
            "build_passing": true,
            "test_pass_rate": 1.0,
            "convergence_level": 0.95,
            "type_check_clean": true,
            "security_issues": 0
        }));

        let client = Arc::new(MockA2AClient { task: Ok(task) });
        let overseer = SwarmOverseer::new("child-1", "task-1", "http://localhost:8080", client);

        let result = overseer.measure(&test_artifact()).await.unwrap();
        assert!(result.pass);
    }

    #[tokio::test]
    async fn build_failure_returns_build_result() {
        let task = make_task_with_data(serde_json::json!({
            "build_passing": false,
            "test_pass_rate": 1.0,
            "convergence_level": 0.95,
            "type_check_clean": true,
            "security_issues": 0
        }));

        let client = Arc::new(MockA2AClient { task: Ok(task) });
        let overseer = SwarmOverseer::new("child-1", "task-1", "http://localhost:8080", client);

        let result = overseer.measure(&test_artifact()).await.unwrap();
        assert!(!result.pass);
        assert!(matches!(
            result.signal,
            OverseerSignalUpdate::BuildResult(ref b) if !b.success
        ));
    }

    #[tokio::test]
    async fn low_test_rate_returns_test_results() {
        let task = make_task_with_data(serde_json::json!({
            "build_passing": true,
            "test_pass_rate": 0.75,
            "convergence_level": 0.95,
            "type_check_clean": true,
            "security_issues": 0
        }));

        let client = Arc::new(MockA2AClient { task: Ok(task) });
        let overseer = SwarmOverseer::new("child-1", "task-1", "http://localhost:8080", client);

        let result = overseer.measure(&test_artifact()).await.unwrap();
        assert!(!result.pass);
        assert!(matches!(
            result.signal,
            OverseerSignalUpdate::TestResults(ref t) if t.passed == 75 && t.failed == 25
        ));
    }

    #[tokio::test]
    async fn a2a_error_returns_custom_check_failure() {
        let client = Arc::new(MockA2AClient {
            task: Err("not found".to_string()),
        });
        let overseer = SwarmOverseer::new("child-1", "task-1", "http://localhost:8080", client);

        let result = overseer.measure(&test_artifact()).await.unwrap();
        assert!(!result.pass);
        assert!(matches!(
            result.signal,
            OverseerSignalUpdate::CustomCheck(ref c) if c.details.contains("A2A poll failed")
        ));
    }

    #[tokio::test]
    async fn no_convergence_data_returns_custom_check_failure() {
        let task = A2ATask {
            id: "task-1".to_string(),
            context_id: None,
            status: A2ATaskStatus {
                state: A2ATaskState::Completed,
                message: None,
                timestamp: None,
            },
            history: None,
            artifacts: vec![],
            metadata: None,
        };

        let client = Arc::new(MockA2AClient { task: Ok(task) });
        let overseer = SwarmOverseer::new("child-1", "task-1", "http://localhost:8080", client);

        let result = overseer.measure(&test_artifact()).await.unwrap();
        assert!(!result.pass);
        assert!(matches!(
            result.signal,
            OverseerSignalUpdate::CustomCheck(ref c) if c.details.contains("No convergence signal data")
        ));
    }

    #[tokio::test]
    async fn low_convergence_returns_custom_check() {
        let task = make_task_with_data(serde_json::json!({
            "build_passing": true,
            "test_pass_rate": 1.0,
            "convergence_level": 0.5,
            "type_check_clean": true,
            "security_issues": 0
        }));

        let client = Arc::new(MockA2AClient { task: Ok(task) });
        let overseer = SwarmOverseer::new("child-1", "task-1", "http://localhost:8080", client);

        let result = overseer.measure(&test_artifact()).await.unwrap();
        assert!(!result.pass);
        assert!(matches!(
            result.signal,
            OverseerSignalUpdate::CustomCheck(ref c) if !c.passed && c.details.contains("0.50")
        ));
    }

    #[test]
    fn name_returns_cerebrate_id() {
        let client = Arc::new(MockA2AClient {
            task: Err("unused".to_string()),
        });
        let overseer = SwarmOverseer::new("my-cerebrate", "task-1", "http://localhost", client);
        assert_eq!(overseer.name(), "my-cerebrate");
    }

    #[test]
    fn cost_is_cheap() {
        let client = Arc::new(MockA2AClient {
            task: Err("unused".to_string()),
        });
        let overseer = SwarmOverseer::new("c1", "t1", "http://localhost", client);
        assert_eq!(overseer.cost(), OverseerCost::Cheap);
    }
}
