//! SQLite implementation of the RefinementRepository for evolution loop persistence.
//!
//! Stores refinement requests so that InProgress items can be recovered after
//! a process restart via `recover_in_progress_refinements()` on startup.

use async_trait::async_trait;
use chrono::Utc;
use sqlx::SqlitePool;
use uuid::Uuid;

use crate::services::evolution_loop::{
    EvolutionTrigger, RefinementRepository, RefinementRequest, RefinementSeverity,
    RefinementStatus, TemplateStats,
};

/// SQLite-backed persistence for evolution loop refinement requests.
#[derive(Clone)]
pub struct SqliteRefinementRepository {
    pool: SqlitePool,
}

impl SqliteRefinementRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

/// SQLite row mapping for the `refinement_requests` table.
#[derive(sqlx::FromRow)]
struct RefinementRequestRow {
    id: String,
    template_name: String,
    template_version: i64,
    severity: String,
    trigger: String,
    stats_json: String,
    failed_task_ids_json: String,
    status: String,
    created_at: String,
    #[allow(dead_code)]
    updated_at: String,
}

impl TryFrom<RefinementRequestRow> for RefinementRequest {
    type Error = Box<dyn std::error::Error + Send + Sync>;

    fn try_from(row: RefinementRequestRow) -> Result<Self, Self::Error> {
        let id = Uuid::parse_str(&row.id)?;
        let template_version = row.template_version as u32;
        let severity = severity_from_str(&row.severity)?;
        let trigger = trigger_from_str(&row.trigger)?;
        let stats: TemplateStats = serde_json::from_str(&row.stats_json)?;
        let failed_task_ids: Vec<Uuid> = serde_json::from_str(&row.failed_task_ids_json)?;
        let status = status_from_str(&row.status)?;
        let created_at = chrono::DateTime::parse_from_rfc3339(&row.created_at)
            .map(|dt| dt.with_timezone(&chrono::Utc))?;

        Ok(RefinementRequest {
            id,
            template_name: row.template_name,
            template_version,
            severity,
            trigger,
            stats,
            failed_task_ids,
            created_at,
            status,
        })
    }
}

// ── string ↔ enum conversions ──────────────────────────────────────────────

fn severity_to_str(s: &RefinementSeverity) -> &'static str {
    match s {
        RefinementSeverity::Minor => "Minor",
        RefinementSeverity::Major => "Major",
        RefinementSeverity::Immediate => "Immediate",
    }
}

fn severity_from_str(
    s: &str,
) -> Result<RefinementSeverity, Box<dyn std::error::Error + Send + Sync>> {
    match s {
        "Minor" => Ok(RefinementSeverity::Minor),
        "Major" => Ok(RefinementSeverity::Major),
        "Immediate" => Ok(RefinementSeverity::Immediate),
        other => Err(format!("Unknown severity: '{}'", other).into()),
    }
}

fn trigger_to_str(t: &EvolutionTrigger) -> &'static str {
    match t {
        EvolutionTrigger::LowSuccessRate => "LowSuccessRate",
        EvolutionTrigger::VeryLowSuccessRate => "VeryLowSuccessRate",
        EvolutionTrigger::GoalViolations => "GoalViolations",
        EvolutionTrigger::DownstreamImpact => "DownstreamImpact",
        EvolutionTrigger::Regression => "Regression",
    }
}

fn trigger_from_str(
    s: &str,
) -> Result<EvolutionTrigger, Box<dyn std::error::Error + Send + Sync>> {
    match s {
        "LowSuccessRate" => Ok(EvolutionTrigger::LowSuccessRate),
        "VeryLowSuccessRate" => Ok(EvolutionTrigger::VeryLowSuccessRate),
        "GoalViolations" => Ok(EvolutionTrigger::GoalViolations),
        "DownstreamImpact" => Ok(EvolutionTrigger::DownstreamImpact),
        "Regression" => Ok(EvolutionTrigger::Regression),
        other => Err(format!("Unknown trigger: '{}'", other).into()),
    }
}

fn status_to_str(s: &RefinementStatus) -> &'static str {
    match s {
        RefinementStatus::Pending => "Pending",
        RefinementStatus::InProgress => "InProgress",
        RefinementStatus::Completed => "Completed",
        RefinementStatus::Failed => "Failed",
    }
}

fn status_from_str(
    s: &str,
) -> Result<RefinementStatus, Box<dyn std::error::Error + Send + Sync>> {
    match s {
        "Pending" => Ok(RefinementStatus::Pending),
        "InProgress" => Ok(RefinementStatus::InProgress),
        "Completed" => Ok(RefinementStatus::Completed),
        "Failed" => Ok(RefinementStatus::Failed),
        other => Err(format!("Unknown status: '{}'", other).into()),
    }
}

// ── RefinementRepository implementation ────────────────────────────────────

#[async_trait]
impl RefinementRepository for SqliteRefinementRepository {
    async fn create(
        &self,
        request: &RefinementRequest,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let stats_json = serde_json::to_string(&request.stats)?;
        let failed_task_ids_json = serde_json::to_string(&request.failed_task_ids)?;
        let now = Utc::now().to_rfc3339();

        sqlx::query(
            r#"INSERT INTO refinement_requests
               (id, template_name, template_version, severity, trigger, stats_json,
                failed_task_ids_json, status, created_at, updated_at)
               VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"#,
        )
        .bind(request.id.to_string())
        .bind(&request.template_name)
        .bind(request.template_version as i64)
        .bind(severity_to_str(&request.severity))
        .bind(trigger_to_str(&request.trigger))
        .bind(&stats_json)
        .bind(&failed_task_ids_json)
        .bind(status_to_str(&request.status))
        .bind(request.created_at.to_rfc3339())
        .bind(&now)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    async fn get_pending(
        &self,
    ) -> Result<Vec<RefinementRequest>, Box<dyn std::error::Error + Send + Sync>> {
        let rows: Vec<RefinementRequestRow> = sqlx::query_as(
            "SELECT * FROM refinement_requests WHERE status = 'Pending' ORDER BY created_at ASC",
        )
        .fetch_all(&self.pool)
        .await?;

        rows.into_iter()
            .map(RefinementRequest::try_from)
            .collect()
    }

    async fn update_status(
        &self,
        id: Uuid,
        status: RefinementStatus,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let now = Utc::now().to_rfc3339();
        sqlx::query(
            "UPDATE refinement_requests SET status = ?, updated_at = ? WHERE id = ?",
        )
        .bind(status_to_str(&status))
        .bind(&now)
        .bind(id.to_string())
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    async fn reset_in_progress_to_pending(
        &self,
    ) -> Result<Vec<RefinementRequest>, Box<dyn std::error::Error + Send + Sync>> {
        // Fetch all InProgress rows before updating so we know which ones were recovered
        let rows: Vec<RefinementRequestRow> = sqlx::query_as(
            "SELECT * FROM refinement_requests WHERE status = 'InProgress'",
        )
        .fetch_all(&self.pool)
        .await?;

        if !rows.is_empty() {
            let now = Utc::now().to_rfc3339();
            sqlx::query(
                "UPDATE refinement_requests SET status = 'Pending', updated_at = ? \
                 WHERE status = 'InProgress'",
            )
            .bind(&now)
            .execute(&self.pool)
            .await?;
        }

        // Return the recovered rows with status overridden to Pending
        rows.into_iter()
            .map(|mut row| {
                row.status = "Pending".to_string();
                RefinementRequest::try_from(row)
            })
            .collect()
    }
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::sqlite::create_migrated_test_pool;

    fn make_request(template_name: &str) -> RefinementRequest {
        RefinementRequest::new(
            template_name.to_string(),
            1,
            RefinementSeverity::Minor,
            EvolutionTrigger::LowSuccessRate,
            TemplateStats::new(template_name.to_string(), 1),
            vec![],
        )
    }

    #[tokio::test]
    async fn test_create_and_get_pending() {
        let pool = create_migrated_test_pool().await.unwrap();
        let repo = SqliteRefinementRepository::new(pool);

        let request = make_request("test-agent");
        repo.create(&request).await.unwrap();

        let pending = repo.get_pending().await.unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].template_name, "test-agent");
        assert_eq!(pending[0].status, RefinementStatus::Pending);
    }

    #[tokio::test]
    async fn test_update_status_to_in_progress() {
        let pool = create_migrated_test_pool().await.unwrap();
        let repo = SqliteRefinementRepository::new(pool);

        let request = make_request("test-agent");
        let id = request.id;
        repo.create(&request).await.unwrap();

        repo.update_status(id, RefinementStatus::InProgress)
            .await
            .unwrap();

        // InProgress requests must not appear in the pending list
        let pending = repo.get_pending().await.unwrap();
        assert!(
            pending.is_empty(),
            "InProgress request should not appear in pending list"
        );
    }

    #[tokio::test]
    async fn test_update_status_to_completed() {
        let pool = create_migrated_test_pool().await.unwrap();
        let repo = SqliteRefinementRepository::new(pool);

        let request = make_request("test-agent");
        let id = request.id;
        repo.create(&request).await.unwrap();

        repo.update_status(id, RefinementStatus::Completed)
            .await
            .unwrap();

        let pending = repo.get_pending().await.unwrap();
        assert!(
            pending.is_empty(),
            "Completed request should not appear in pending list"
        );
    }

    #[tokio::test]
    async fn test_reset_in_progress_to_pending() {
        let pool = create_migrated_test_pool().await.unwrap();
        let repo = SqliteRefinementRepository::new(pool);

        // Create two requests
        let req1 = make_request("agent-a");
        let req2 = make_request("agent-b");
        repo.create(&req1).await.unwrap();
        repo.create(&req2).await.unwrap();

        // Move one to InProgress
        repo.update_status(req1.id, RefinementStatus::InProgress)
            .await
            .unwrap();

        // Reset InProgress back to Pending; should return only the recovered one
        let recovered = repo.reset_in_progress_to_pending().await.unwrap();
        assert_eq!(recovered.len(), 1, "only the InProgress request should be recovered");
        assert_eq!(
            recovered[0].id, req1.id,
            "req1 (previously InProgress) should be in recovered list"
        );
        assert_eq!(
            recovered[0].status,
            RefinementStatus::Pending,
            "recovered request must have Pending status"
        );

        // Both requests should now be visible as Pending
        let pending = repo.get_pending().await.unwrap();
        assert_eq!(pending.len(), 2, "both requests should now be Pending");
    }

    #[tokio::test]
    async fn test_no_duplicate_pending_on_completed() {
        let pool = create_migrated_test_pool().await.unwrap();
        let repo = SqliteRefinementRepository::new(pool);

        let request = make_request("test-agent");
        let id = request.id;
        repo.create(&request).await.unwrap();
        repo.update_status(id, RefinementStatus::Completed)
            .await
            .unwrap();

        // reset_in_progress_to_pending should not touch or return Completed items
        let recovered = repo.reset_in_progress_to_pending().await.unwrap();
        assert!(
            !recovered.iter().any(|r| r.id == id),
            "Completed request must not appear in recovered list"
        );
    }
}
