//! Pre-spawn middleware: skip if required MCP servers aren't ready.
//!
//! Preserves the runtime safety net previously in `spawn_task_agent`: if the
//! Abathur binary or DB file are missing, or if a configured A2A gateway
//! won't respond, the task stays `Ready` and will be retried on the next
//! poll cycle.

use async_trait::async_trait;
use std::path::PathBuf;

use crate::domain::errors::DomainResult;
use crate::services::{AuditAction, AuditActor, AuditCategory, AuditEntry, AuditLevel};

use super::{PreSpawnContext, PreSpawnDecision, PreSpawnMiddleware};

/// Checks local MCP infrastructure before allowing spawn.
pub struct McpReadinessMiddleware {
    /// Path to the repository root; used to locate the `.abathur/` dir.
    repo_path: PathBuf,
    /// Optional A2A gateway base URL to probe via `/health`.
    a2a_gateway: Option<String>,
}

impl McpReadinessMiddleware {
    pub fn new(repo_path: PathBuf, a2a_gateway: Option<String>) -> Self {
        Self { repo_path, a2a_gateway }
    }

    async fn check(&self) -> bool {
        // Abathur binary must exist — MCP stdio servers are forked children.
        let exe_ok = std::env::current_exe()
            .map(|p| p.exists())
            .unwrap_or(false);
        if !exe_ok {
            tracing::warn!("Abathur binary not found — MCP stdio servers cannot launch");
            return false;
        }

        // DB file — absolute path consistent with agent MCP configs.
        let db_path = std::env::current_dir()
            .unwrap_or_else(|_| self.repo_path.clone())
            .join(".abathur")
            .join("abathur.db");
        if !db_path.exists() {
            tracing::warn!(
                "Database not found at {:?} — MCP stdio servers cannot launch",
                db_path
            );
            return false;
        }

        // A2A gateway health probe (if configured).
        if let Some(ref a2a_url) = self.a2a_gateway {
            let client = reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(2))
                .build()
                .unwrap_or_default();

            let health_url = format!("{}/health", a2a_url.trim_end_matches('/'));
            match client.get(&health_url).send().await {
                Ok(resp) if resp.status().is_success() => {}
                Ok(resp) => {
                    tracing::warn!(
                        "A2A gateway at {} returned status {}",
                        a2a_url,
                        resp.status()
                    );
                    return false;
                }
                Err(e) => {
                    tracing::warn!("A2A gateway at {} unreachable: {}", a2a_url, e);
                    return false;
                }
            }
        }

        true
    }
}

#[async_trait]
impl PreSpawnMiddleware for McpReadinessMiddleware {
    fn name(&self) -> &'static str {
        "mcp-readiness"
    }

    async fn handle(
        &self,
        ctx: &mut PreSpawnContext,
    ) -> DomainResult<PreSpawnDecision> {
        if self.check().await {
            return Ok(PreSpawnDecision::Continue);
        }

        ctx.audit_log
            .log(
                AuditEntry::new(
                    AuditLevel::Warning,
                    AuditCategory::Execution,
                    AuditAction::TaskFailed,
                    AuditActor::System,
                    format!(
                        "Skipping spawn for task {} - MCP servers not ready (will retry next cycle)",
                        ctx.task.id
                    ),
                )
                .with_entity(ctx.task.id, "task"),
            )
            .await;

        Ok(PreSpawnDecision::Skip {
            reason: "mcp-not-ready".to_string(),
        })
    }
}
