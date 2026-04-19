//! Post-completion middleware: run integration verification for tasks
//! currently sitting in `Validating`.
//!
//! Runs the lightweight `IntegrationVerifierService` (no tests/lint/format —
//! those happen at merge time), publishes `TaskVerified`, and transitions
//! `Validating -> Complete` or `Validating -> Failed` depending on the
//! outcome and `intent_satisfied` flag.

use async_trait::async_trait;

use crate::domain::errors::DomainResult;
use crate::domain::models::TaskStatus;
use crate::services::{
    AuditAction, AuditActor, AuditCategory, AuditEntry, AuditLevel,
    IntegrationVerifierService, VerifierConfig,
};
use crate::services::swarm_orchestrator::types::SwarmEvent;

use super::{PostCompletionContext, PostCompletionMiddleware};

pub struct VerificationMiddleware;

impl VerificationMiddleware {
    pub fn new() -> Self {
        Self
    }
}

impl Default for VerificationMiddleware {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl PostCompletionMiddleware for VerificationMiddleware {
    fn name(&self) -> &'static str {
        "verification"
    }

    async fn handle(&self, ctx: &mut PostCompletionContext) -> DomainResult<()> {
        // When the tree is already handled (e.g. MemoryOnly short-circuit),
        // preserve the early-exit semantics.
        if ctx.tree_handled {
            return Ok(());
        }

        let task_id = ctx.task_id;
        let intent_satisfied = ctx.intent_satisfied;
        let require_commits = ctx.require_commits;

        // Convergence-verified tasks can ship without commits; suppress the
        // commits gate inside the verifier in that case.
        let require_commits_for_verification = require_commits && !intent_satisfied;
        if intent_satisfied && require_commits {
            tracing::info!(
                task_id = %task_id,
                "Intent verified as satisfied — overriding require_commits to false"
            );
        }

        if !ctx.verify_on_completion {
            // Verification skipped — assume passed (matches previous behaviour).
            ctx.verification_passed = true;
            return Ok(());
        }

        let verifier = IntegrationVerifierService::new(
            ctx.task_repo.clone(),
            ctx.goal_repo.clone(),
            ctx.worktree_repo.clone(),
            VerifierConfig {
                run_tests: false,
                run_lint: false,
                check_format: false,
                require_commits: require_commits_for_verification,
                ..VerifierConfig::default()
            },
        );

        match verifier.verify_task(task_id).await {
            Ok(result) => {
                let checks_total = result.checks.len();
                let checks_passed = result.checks.iter().filter(|c| c.passed).count();

                let _ = ctx
                    .event_tx
                    .send(SwarmEvent::TaskVerified {
                        task_id,
                        passed: result.passed,
                        checks_passed,
                        checks_total,
                        failures_summary: result.failures_summary.clone(),
                    })
                    .await;

                if result.passed {
                    if let Ok(Some(mut task)) = ctx.task_repo.get(task_id).await
                        && task.status == TaskStatus::Validating
                    {
                        let _ = task.transition_to(TaskStatus::Complete);
                        let _ = ctx.task_repo.update(&task).await;
                        let _ = ctx
                            .event_tx
                            .send(SwarmEvent::TaskCompleted {
                                task_id,
                                tokens_used: 0,
                            })
                            .await;
                        ctx.event_bus
                            .publish(crate::services::event_factory::task_event(
                                crate::services::event_bus::EventSeverity::Info,
                                None,
                                task_id,
                                crate::services::event_bus::EventPayload::TaskCompleted {
                                    task_id,
                                    tokens_used: 0,
                                },
                            ))
                            .await;
                    }

                    ctx.audit_log
                        .info(
                            AuditCategory::Task,
                            AuditAction::TaskCompleted,
                            format!(
                                "Task {} passed verification: {}/{} checks",
                                task_id, checks_passed, checks_total
                            ),
                        )
                        .await;
                } else if intent_satisfied {
                    tracing::warn!(
                        task_id = %task_id,
                        failures = ?result.failures_summary,
                        "Integration verifier failed but intent is satisfied — \
                         proceeding to Complete (advisory mode)"
                    );

                    if let Ok(Some(mut task)) = ctx.task_repo.get(task_id).await
                        && task.status == TaskStatus::Validating
                    {
                        let _ = task.transition_to(TaskStatus::Complete);
                        let _ = ctx.task_repo.update(&task).await;
                        let _ = ctx
                            .event_tx
                            .send(SwarmEvent::TaskCompleted {
                                task_id,
                                tokens_used: 0,
                            })
                            .await;
                        ctx.event_bus
                            .publish(crate::services::event_factory::task_event(
                                crate::services::event_bus::EventSeverity::Info,
                                None,
                                task_id,
                                crate::services::event_bus::EventPayload::TaskCompleted {
                                    task_id,
                                    tokens_used: 0,
                                },
                            ))
                            .await;
                    }

                    ctx.audit_log
                        .log(
                            AuditEntry::new(
                                AuditLevel::Warning,
                                AuditCategory::Task,
                                AuditAction::TaskCompleted,
                                AuditActor::System,
                                format!(
                                    "Task {} completed (intent satisfied) despite integration \
                                     verifier failures: {}",
                                    task_id,
                                    result.failures_summary.clone().unwrap_or_default()
                                ),
                            )
                            .with_entity(task_id, "task"),
                        )
                        .await;
                } else {
                    let retry_count = if let Ok(Some(mut task)) = ctx.task_repo.get(task_id).await {
                        if task.status == TaskStatus::Validating {
                            task.retry_count += 1;
                            let _ = task.transition_to(TaskStatus::Failed);
                            let _ = ctx.task_repo.update(&task).await;
                        }
                        task.retry_count
                    } else {
                        0
                    };

                    let failure_msg = format!(
                        "verification-failed: {}",
                        result.failures_summary.clone().unwrap_or_default()
                    );
                    let _ = ctx
                        .event_tx
                        .send(SwarmEvent::TaskFailed {
                            task_id,
                            error: failure_msg,
                            retry_count,
                        })
                        .await;

                    if let Ok(Some(mut wt)) = ctx.worktree_repo.get_by_task(task_id).await {
                        wt.fail("Verification failed".to_string());
                        let _ = ctx.worktree_repo.update(&wt).await;
                    }

                    ctx.audit_log
                        .log(
                            AuditEntry::new(
                                AuditLevel::Warning,
                                AuditCategory::Task,
                                AuditAction::TaskFailed,
                                AuditActor::System,
                                format!(
                                    "Task {} failed verification (retry_count={}): {}",
                                    task_id,
                                    retry_count,
                                    result.failures_summary.clone().unwrap_or_default()
                                ),
                            )
                            .with_entity(task_id, "task"),
                        )
                        .await;
                }

                // When intent is satisfied, treat as passed even if integration
                // checks failed (advisory mode).
                ctx.verification_passed = result.passed || intent_satisfied;
            }
            Err(e) => {
                if intent_satisfied {
                    tracing::warn!(
                        task_id = %task_id,
                        error = %e,
                        "Integration verification error but intent satisfied — \
                         proceeding to Complete"
                    );

                    if let Ok(Some(mut task)) = ctx.task_repo.get(task_id).await
                        && task.status == TaskStatus::Validating
                    {
                        let _ = task.transition_to(TaskStatus::Complete);
                        let _ = ctx.task_repo.update(&task).await;
                        let _ = ctx
                            .event_tx
                            .send(SwarmEvent::TaskCompleted {
                                task_id,
                                tokens_used: 0,
                            })
                            .await;
                        ctx.event_bus
                            .publish(crate::services::event_factory::task_event(
                                crate::services::event_bus::EventSeverity::Info,
                                None,
                                task_id,
                                crate::services::event_bus::EventPayload::TaskCompleted {
                                    task_id,
                                    tokens_used: 0,
                                },
                            ))
                            .await;
                    }

                    ctx.audit_log
                        .log(
                            AuditEntry::new(
                                AuditLevel::Warning,
                                AuditCategory::Task,
                                AuditAction::TaskCompleted,
                                AuditActor::System,
                                format!(
                                    "Task {} completed (intent satisfied) despite verification error: {}",
                                    task_id, e
                                ),
                            )
                            .with_entity(task_id, "task"),
                        )
                        .await;
                    ctx.verification_passed = true;
                } else {
                    let retry_count = if let Ok(Some(mut task)) = ctx.task_repo.get(task_id).await {
                        if task.status == TaskStatus::Validating {
                            task.retry_count += 1;
                            let _ = task.transition_to(TaskStatus::Failed);
                            let _ = ctx.task_repo.update(&task).await;
                        }
                        task.retry_count
                    } else {
                        0
                    };

                    let _ = ctx
                        .event_tx
                        .send(SwarmEvent::TaskFailed {
                            task_id,
                            error: format!("verification-error: {}", e),
                            retry_count,
                        })
                        .await;

                    if let Ok(Some(mut wt)) = ctx.worktree_repo.get_by_task(task_id).await {
                        wt.fail(format!("Verification error: {}", e));
                        let _ = ctx.worktree_repo.update(&wt).await;
                    }

                    ctx.audit_log
                        .log(
                            AuditEntry::new(
                                AuditLevel::Warning,
                                AuditCategory::Task,
                                AuditAction::TaskFailed,
                                AuditActor::System,
                                format!(
                                    "Task {} verification error (retry_count={}): {}",
                                    task_id, retry_count, e
                                ),
                            )
                            .with_entity(task_id, "task"),
                        )
                        .await;
                    ctx.verification_passed = false;
                }
            }
        }

        Ok(())
    }
}
