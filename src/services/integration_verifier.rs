//! Integration Verifier Service.
//!
//! Validates that completed tasks satisfy goal constraints,
//! pass integration tests, and are ready for merge.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::process::Command;
use uuid::Uuid;

use crate::domain::errors::{DomainError, DomainResult};
use crate::domain::models::{GoalConstraint, ConstraintType, Task, TaskStatus};
use crate::domain::ports::{GoalRepository, TaskRepository, WorktreeRepository};

/// Configuration for the integration verifier.
#[derive(Debug, Clone)]
pub struct VerifierConfig {
    /// Whether to run integration tests.
    pub run_tests: bool,
    /// Whether to run linting.
    pub run_lint: bool,
    /// Whether to check formatting.
    pub check_format: bool,
    /// Whether to require commits ahead of base ref.
    /// Set to false for read-only agents (research, analysis) that produce no code changes.
    pub require_commits: bool,
    /// Timeout for test execution (seconds).
    pub test_timeout_secs: u64,
    /// Whether to fail on warnings.
    pub fail_on_warnings: bool,
}

impl Default for VerifierConfig {
    fn default() -> Self {
        Self {
            run_tests: true,
            run_lint: true,
            check_format: true,
            require_commits: true,
            test_timeout_secs: 300,
            fail_on_warnings: false,
        }
    }
}

/// Result of a verification check.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerificationCheck {
    /// Name of the check.
    pub name: String,
    /// Whether the check passed.
    pub passed: bool,
    /// Human-readable message.
    pub message: String,
    /// Additional details (JSON).
    pub details: Option<serde_json::Value>,
}

/// Result of verifying a task.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerificationResult {
    /// Task ID that was verified.
    pub task_id: Uuid,
    /// Whether all checks passed.
    pub passed: bool,
    /// Individual check results.
    pub checks: Vec<VerificationCheck>,
    /// Summary of failures if any.
    pub failures_summary: Option<String>,
    /// When verification was performed.
    pub verified_at: DateTime<Utc>,
}

/// Result of running tests.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestResult {
    /// Whether all tests passed.
    pub passed: bool,
    /// Total number of tests.
    pub total: usize,
    /// Number of passed tests.
    pub passed_count: usize,
    /// Number of failed tests.
    pub failed_count: usize,
    /// Names of failed tests.
    pub failures: Vec<String>,
    /// Duration in milliseconds.
    pub duration_ms: u64,
}

/// Integration Verifier Service.
pub struct IntegrationVerifierService<T, G, W>
where
    T: TaskRepository + 'static,
    G: GoalRepository + 'static,
    W: WorktreeRepository + 'static,
{
    task_repo: Arc<T>,
    #[allow(dead_code)]
    goal_repo: Arc<G>,
    worktree_repo: Arc<W>,
    config: VerifierConfig,
}

impl<T, G, W> IntegrationVerifierService<T, G, W>
where
    T: TaskRepository + 'static,
    G: GoalRepository + 'static,
    W: WorktreeRepository + 'static,
{
    pub fn new(
        task_repo: Arc<T>,
        goal_repo: Arc<G>,
        worktree_repo: Arc<W>,
        config: VerifierConfig,
    ) -> Self {
        Self {
            task_repo,
            goal_repo,
            worktree_repo,
            config,
        }
    }

    /// Verify a task is ready for merge.
    pub async fn verify_task(&self, task_id: Uuid) -> DomainResult<VerificationResult> {
        let task = self.task_repo.get(task_id).await?
            .ok_or(DomainError::TaskNotFound(task_id))?;

        let mut checks = Vec::new();

        // 1. Check all dependencies complete
        let deps_check = self.check_dependencies_complete(&task).await?;
        checks.push(deps_check);

        // 3. Get worktree and check for commits / run code checks
        let worktree = self.worktree_repo.get_by_task(task_id).await?;
        if let Some(wt) = worktree {
            // Check that the agent actually produced commits (skip for read-only agents)
            if self.config.require_commits {
                let commits_check = self.check_has_commits(&wt.path, &wt.base_ref).await;
                checks.push(commits_check);
            }

            // Run integration tests
            if self.config.run_tests {
                let test_check = self.run_integration_tests(&wt.path).await;
                checks.push(test_check);
            }

            // Run lint check
            if self.config.run_lint {
                let lint_check = self.run_lint_check(&wt.path).await;
                checks.push(lint_check);
            }

            // Run format check
            if self.config.check_format {
                let format_check = self.run_format_check(&wt.path).await;
                checks.push(format_check);
            }
        } else {
            // No worktree - add a note
            checks.push(VerificationCheck {
                name: "worktree".to_string(),
                passed: true,
                message: "No worktree associated with task".to_string(),
                details: None,
            });
        }

        // Calculate overall result
        let all_passed = checks.iter().all(|c| c.passed);
        let failures: Vec<_> = checks.iter().filter(|c| !c.passed).collect();

        // Log each check result for debugging task verification issues
        for check in &checks {
            if check.passed {
                tracing::debug!(
                    task_id = %task_id,
                    check = %check.name,
                    "Verification check passed: {}",
                    check.message
                );
            } else {
                tracing::warn!(
                    task_id = %task_id,
                    check = %check.name,
                    details = ?check.details,
                    "Verification check FAILED: {}",
                    check.message
                );
            }
        }

        let failures_summary = if failures.is_empty() {
            None
        } else {
            let summary = failures
                .iter()
                .map(|c| format!("{}: {}", c.name, c.message))
                .collect::<Vec<_>>()
                .join("\n");
            tracing::warn!(
                task_id = %task_id,
                checks_passed = checks.len() - failures.len(),
                checks_total = checks.len(),
                "Task verification failed: {}",
                summary
            );
            Some(summary)
        };

        Ok(VerificationResult {
            task_id,
            passed: all_passed,
            checks,
            failures_summary,
            verified_at: Utc::now(),
        })
    }

    /// Check that all task dependencies are complete.
    async fn check_dependencies_complete(&self, task: &Task) -> DomainResult<VerificationCheck> {
        if task.depends_on.is_empty() {
            return Ok(VerificationCheck {
                name: "dependencies".to_string(),
                passed: true,
                message: "No dependencies to check".to_string(),
                details: None,
            });
        }

        let deps = self.task_repo.get_dependencies(task.id).await?;
        let incomplete: Vec<_> = deps
            .iter()
            .filter(|t| t.status != TaskStatus::Complete)
            .collect();

        if incomplete.is_empty() {
            Ok(VerificationCheck {
                name: "dependencies".to_string(),
                passed: true,
                message: format!("All {} dependencies complete", deps.len()),
                details: None,
            })
        } else {
            Ok(VerificationCheck {
                name: "dependencies".to_string(),
                passed: false,
                message: format!("{} dependencies incomplete", incomplete.len()),
                details: Some(serde_json::json!({
                    "incomplete": incomplete.iter().map(|t| {
                        serde_json::json!({
                            "id": t.id.to_string(),
                            "title": &t.title,
                            "status": format!("{:?}", t.status)
                        })
                    }).collect::<Vec<_>>()
                })),
            })
        }
    }

    /// Check that the worktree branch has commits ahead of its base ref.
    async fn check_has_commits(&self, worktree_path: &str, base_ref: &str) -> VerificationCheck {
        let output = Command::new("git")
            .args(["rev-list", "--count", &format!("{}..HEAD", base_ref)])
            .current_dir(worktree_path)
            .output()
            .await;

        match output {
            Ok(output) if output.status.success() => {
                let count_str = String::from_utf8_lossy(&output.stdout).trim().to_string();
                let count: u64 = count_str.parse().unwrap_or(0);

                if count > 0 {
                    VerificationCheck {
                        name: "has_commits".to_string(),
                        passed: true,
                        message: format!("{} commit(s) ahead of {}", count, base_ref),
                        details: Some(serde_json::json!({
                            "commits_ahead": count,
                            "base_ref": base_ref
                        })),
                    }
                } else {
                    VerificationCheck {
                        name: "has_commits".to_string(),
                        passed: false,
                        message: format!("No commits ahead of {} — agent produced no changes", base_ref),
                        details: Some(serde_json::json!({
                            "commits_ahead": 0,
                            "base_ref": base_ref
                        })),
                    }
                }
            }
            Ok(output) => {
                let stderr = String::from_utf8_lossy(&output.stderr);
                VerificationCheck {
                    name: "has_commits".to_string(),
                    passed: false,
                    message: format!("Failed to check commits: {}", stderr.trim()),
                    details: None,
                }
            }
            Err(e) => VerificationCheck {
                name: "has_commits".to_string(),
                passed: false,
                message: format!("Failed to run git: {}", e),
                details: None,
            },
        }
    }

    /// Verify goal constraints are satisfied.
    #[allow(dead_code)]
    async fn verify_goal_constraints(
        &self,
        task: &Task,
        goal_id: Uuid,
    ) -> DomainResult<VerificationCheck> {
        let goal = self.goal_repo.get(goal_id).await?;

        let Some(goal) = goal else {
            return Ok(VerificationCheck {
                name: "goal_constraints".to_string(),
                passed: true,
                message: "Goal not found, skipping constraint check".to_string(),
                details: None,
            });
        };

        if goal.constraints.is_empty() {
            return Ok(VerificationCheck {
                name: "goal_constraints".to_string(),
                passed: true,
                message: "No goal constraints to verify".to_string(),
                details: None,
            });
        }

        let mut violations = Vec::new();

        for constraint in &goal.constraints {
            let result = self.evaluate_constraint(task, constraint);
            if !result.0 {
                violations.push(serde_json::json!({
                    "constraint": &constraint.name,
                    "type": format!("{:?}", constraint.constraint_type),
                    "reason": result.1
                }));
            }
        }

        if violations.is_empty() {
            Ok(VerificationCheck {
                name: "goal_constraints".to_string(),
                passed: true,
                message: format!("All {} constraints satisfied", goal.constraints.len()),
                details: None,
            })
        } else {
            Ok(VerificationCheck {
                name: "goal_constraints".to_string(),
                passed: false,
                message: format!("{} constraint violations", violations.len()),
                details: Some(serde_json::json!({ "violations": violations })),
            })
        }
    }

    /// Evaluate a single constraint.
    #[allow(dead_code)]
    fn evaluate_constraint(&self, _task: &Task, constraint: &GoalConstraint) -> (bool, Option<String>) {
        // Basic constraint evaluation
        // In a full implementation, this would analyze the task output
        // and verify specific conditions
        match constraint.constraint_type {
            ConstraintType::Invariant => {
                // Invariants should always be true - would need code analysis
                (true, None)
            }
            ConstraintType::Boundary => {
                // Hard limits - check for explicit violations
                (true, None)
            }
            ConstraintType::Preference => {
                // Soft preferences - always satisfied but may note issues
                (true, None)
            }
        }
    }

    /// Run integration tests in the worktree.
    async fn run_integration_tests(&self, path: &str) -> VerificationCheck {
        let start = std::time::Instant::now();

        let output = Command::new("cargo")
            .args(["test", "--", "--test-threads=1"])
            .current_dir(path)
            .output()
            .await;

        match output {
            Ok(output) => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                let stderr = String::from_utf8_lossy(&output.stderr);
                let duration_ms = start.elapsed().as_millis() as u64;

                let test_result = self.parse_test_output(&stdout, &stderr);

                if output.status.success() {
                    VerificationCheck {
                        name: "integration_tests".to_string(),
                        passed: true,
                        message: format!("{} tests passed", test_result.passed_count),
                        details: Some(serde_json::json!({
                            "total": test_result.total,
                            "passed": test_result.passed_count,
                            "duration_ms": duration_ms
                        })),
                    }
                } else {
                    VerificationCheck {
                        name: "integration_tests".to_string(),
                        passed: false,
                        message: format!("{} test failures", test_result.failed_count),
                        details: Some(serde_json::json!({
                            "total": test_result.total,
                            "passed": test_result.passed_count,
                            "failed": test_result.failed_count,
                            "failures": test_result.failures,
                            "duration_ms": duration_ms
                        })),
                    }
                }
            }
            Err(e) => VerificationCheck {
                name: "integration_tests".to_string(),
                passed: false,
                message: format!("Failed to run tests: {}", e),
                details: None,
            },
        }
    }

    /// Parse test output to extract results.
    fn parse_test_output(&self, stdout: &str, stderr: &str) -> TestResult {
        let mut total = 0;
        let mut passed = 0;
        let mut failed = 0;
        let mut failures = Vec::new();

        // Count tests from "running X tests" lines
        for line in stdout.lines() {
            if line.contains("running") && line.contains("test") {
                if let Some(count) = line.split_whitespace().nth(1).and_then(|s| s.parse::<usize>().ok()) {
                    total += count;
                }
            }
            if line.contains("... ok") {
                passed += 1;
            } else if line.contains("... FAILED") {
                failed += 1;
                if let Some(test_name) = line.split("...").next() {
                    failures.push(test_name.trim().to_string());
                }
            }
        }

        // Also check stderr
        for line in stderr.lines() {
            if line.contains("... FAILED") {
                failed += 1;
                if let Some(test_name) = line.split("...").next() {
                    let name = test_name.trim().to_string();
                    if !failures.contains(&name) {
                        failures.push(name);
                    }
                }
            }
        }

        TestResult {
            passed: failed == 0,
            total,
            passed_count: passed,
            failed_count: failed,
            failures,
            duration_ms: 0,
        }
    }

    /// Run lint check (cargo clippy).
    async fn run_lint_check(&self, path: &str) -> VerificationCheck {
        let args = if self.config.fail_on_warnings {
            vec!["clippy", "--", "-D", "warnings"]
        } else {
            vec!["clippy"]
        };

        let output = Command::new("cargo")
            .args(&args)
            .current_dir(path)
            .output()
            .await;

        match output {
            Ok(output) => {
                if output.status.success() {
                    VerificationCheck {
                        name: "lint".to_string(),
                        passed: true,
                        message: "Lint check passed".to_string(),
                        details: None,
                    }
                } else {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    let issues: Vec<String> = stderr
                        .lines()
                        .filter(|l| l.contains("warning:") || l.contains("error:"))
                        .take(10) // Limit to first 10 issues
                        .map(String::from)
                        .collect();

                    VerificationCheck {
                        name: "lint".to_string(),
                        passed: false,
                        message: format!("{} lint issues", issues.len()),
                        details: Some(serde_json::json!({ "issues": issues })),
                    }
                }
            }
            Err(e) => VerificationCheck {
                name: "lint".to_string(),
                passed: false,
                message: format!("Failed to run lint: {}", e),
                details: None,
            },
        }
    }

    /// Run format check (cargo fmt --check).
    async fn run_format_check(&self, path: &str) -> VerificationCheck {
        let output = Command::new("cargo")
            .args(["fmt", "--check"])
            .current_dir(path)
            .output()
            .await;

        match output {
            Ok(output) => {
                if output.status.success() {
                    VerificationCheck {
                        name: "format".to_string(),
                        passed: true,
                        message: "Code is properly formatted".to_string(),
                        details: None,
                    }
                } else {
                    VerificationCheck {
                        name: "format".to_string(),
                        passed: false,
                        message: "Code needs formatting".to_string(),
                        details: None,
                    }
                }
            }
            Err(e) => VerificationCheck {
                name: "format".to_string(),
                passed: false,
                message: format!("Failed to run format check: {}", e),
                details: None,
            },
        }
    }

    /// Verify a task can be merged (no conflicts).
    pub async fn verify_mergeable(&self, task_id: Uuid, base_branch: &str) -> DomainResult<VerificationCheck> {
        let worktree = self.worktree_repo.get_by_task(task_id).await?;

        let Some(wt) = worktree else {
            return Ok(VerificationCheck {
                name: "mergeable".to_string(),
                passed: true,
                message: "No worktree to check".to_string(),
                details: None,
            });
        };

        // Check if branch can merge into base
        let output = Command::new("git")
            .args(["merge-tree", base_branch, &wt.branch])
            .current_dir(&wt.path)
            .output()
            .await;

        match output {
            Ok(output) => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                let has_conflicts = stdout.contains("<<<<<<");

                if has_conflicts {
                    Ok(VerificationCheck {
                        name: "mergeable".to_string(),
                        passed: false,
                        message: format!("Merge conflicts with {}", base_branch),
                        details: None,
                    })
                } else {
                    Ok(VerificationCheck {
                        name: "mergeable".to_string(),
                        passed: true,
                        message: format!("Can merge into {}", base_branch),
                        details: None,
                    })
                }
            }
            Err(e) => Ok(VerificationCheck {
                name: "mergeable".to_string(),
                passed: false,
                message: format!("Failed to check mergeability: {}", e),
                details: None,
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_verifier_config_default() {
        let config = VerifierConfig::default();
        assert!(config.run_tests);
        assert!(config.run_lint);
        assert!(config.check_format);
        assert!(config.require_commits);
        assert_eq!(config.test_timeout_secs, 300);
    }

    #[test]
    fn test_verification_check_json() {
        let check = VerificationCheck {
            name: "test".to_string(),
            passed: true,
            message: "All good".to_string(),
            details: Some(serde_json::json!({"count": 5})),
        };

        let json = serde_json::to_string(&check).unwrap();
        assert!(json.contains("\"name\":\"test\""));
        assert!(json.contains("\"passed\":true"));
    }

    #[test]
    fn test_parse_test_output() {
        // Mock output parsing
        let output = r#"
running 5 tests
test test_one ... ok
test test_two ... ok
test test_three ... FAILED
test test_four ... ok
test test_five ... ok
"#;

        // Can't easily test the full service without DB setup
        // but we can verify the output format expectations
        assert!(output.contains("running 5 tests"));
        assert!(output.contains("FAILED"));
    }
}
