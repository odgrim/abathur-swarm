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
    RefinementStatus, TaskExecution, TaskOutcome, TemplateStats, VersionChangeRecord,
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
        EvolutionTrigger::StaleTimeout => "StaleTimeout",
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
        "StaleTimeout" => Ok(EvolutionTrigger::StaleTimeout),
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

// ── Row types for template stats persistence ───────────────────────────────

#[derive(sqlx::FromRow)]
struct TemplateStatsRow {
    template_name: String,
    template_version: i64,
    total_tasks: i64,
    successful_tasks: i64,
    failed_tasks: i64,
    goal_violations: i64,
    success_rate: f64,
    avg_turns: f64,
    avg_tokens: f64,
    first_execution: Option<String>,
    last_execution: Option<String>,
    #[allow(dead_code)]
    updated_at: String,
}

impl From<TemplateStatsRow> for TemplateStats {
    fn from(row: TemplateStatsRow) -> Self {
        Self {
            template_name: row.template_name,
            template_version: row.template_version as u32,
            total_tasks: row.total_tasks as usize,
            successful_tasks: row.successful_tasks as usize,
            failed_tasks: row.failed_tasks as usize,
            goal_violations: row.goal_violations as usize,
            success_rate: row.success_rate,
            avg_turns: row.avg_turns,
            avg_tokens: row.avg_tokens,
            first_execution: row
                .first_execution
                .and_then(|s| chrono::DateTime::parse_from_rfc3339(&s).ok())
                .map(|dt| dt.with_timezone(&chrono::Utc)),
            last_execution: row
                .last_execution
                .and_then(|s| chrono::DateTime::parse_from_rfc3339(&s).ok())
                .map(|dt| dt.with_timezone(&chrono::Utc)),
        }
    }
}

#[derive(sqlx::FromRow)]
struct TemplateExecutionRow {
    #[allow(dead_code)]
    id: String,
    task_id: String,
    template_name: String,
    template_version: i64,
    outcome: String,
    executed_at: String,
    turns_used: i64,
    tokens_used: i64,
    downstream_tasks_json: String,
    #[allow(dead_code)]
    created_at: String,
}

impl TryFrom<TemplateExecutionRow> for TaskExecution {
    type Error = Box<dyn std::error::Error + Send + Sync>;

    fn try_from(row: TemplateExecutionRow) -> Result<Self, Self::Error> {
        let task_id = Uuid::parse_str(&row.task_id)?;
        let outcome = outcome_from_str(&row.outcome)?;
        let executed_at = chrono::DateTime::parse_from_rfc3339(&row.executed_at)
            .map(|dt| dt.with_timezone(&chrono::Utc))?;
        let downstream_tasks: Vec<Uuid> = serde_json::from_str(&row.downstream_tasks_json)?;

        Ok(TaskExecution {
            task_id,
            template_name: row.template_name,
            template_version: row.template_version as u32,
            outcome,
            executed_at,
            turns_used: row.turns_used as u32,
            tokens_used: row.tokens_used as u64,
            downstream_tasks,
        })
    }
}

#[derive(sqlx::FromRow)]
struct VersionChangeRow {
    #[allow(dead_code)]
    id: i64,
    template_name: String,
    from_version: i64,
    to_version: i64,
    previous_stats_json: String,
    changed_at: String,
    #[allow(dead_code)]
    created_at: String,
}

impl TryFrom<VersionChangeRow> for VersionChangeRecord {
    type Error = Box<dyn std::error::Error + Send + Sync>;

    fn try_from(row: VersionChangeRow) -> Result<Self, Self::Error> {
        let previous_stats: TemplateStats = serde_json::from_str(&row.previous_stats_json)?;
        let changed_at = chrono::DateTime::parse_from_rfc3339(&row.changed_at)
            .map(|dt| dt.with_timezone(&chrono::Utc))?;

        Ok(VersionChangeRecord {
            template_name: row.template_name,
            from_version: row.from_version as u32,
            to_version: row.to_version as u32,
            previous_stats,
            changed_at,
        })
    }
}

fn outcome_to_str(o: &TaskOutcome) -> &'static str {
    match o {
        TaskOutcome::Success => "Success",
        TaskOutcome::Failure => "Failure",
        TaskOutcome::GoalViolation => "GoalViolation",
    }
}

fn outcome_from_str(
    s: &str,
) -> Result<TaskOutcome, Box<dyn std::error::Error + Send + Sync>> {
    match s {
        "Success" => Ok(TaskOutcome::Success),
        "Failure" => Ok(TaskOutcome::Failure),
        "GoalViolation" => Ok(TaskOutcome::GoalViolation),
        other => Err(format!("Unknown outcome: '{}'", other).into()),
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

    async fn save_stats(
        &self,
        stats: &TemplateStats,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let now = Utc::now().to_rfc3339();
        let first_exec = stats.first_execution.map(|dt| dt.to_rfc3339());
        let last_exec = stats.last_execution.map(|dt| dt.to_rfc3339());

        sqlx::query(
            r#"INSERT INTO template_stats
               (template_name, template_version, total_tasks, successful_tasks,
                failed_tasks, goal_violations, success_rate, avg_turns, avg_tokens,
                first_execution, last_execution, updated_at)
               VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
               ON CONFLICT(template_name) DO UPDATE SET
                template_version = excluded.template_version,
                total_tasks = excluded.total_tasks,
                successful_tasks = excluded.successful_tasks,
                failed_tasks = excluded.failed_tasks,
                goal_violations = excluded.goal_violations,
                success_rate = excluded.success_rate,
                avg_turns = excluded.avg_turns,
                avg_tokens = excluded.avg_tokens,
                first_execution = excluded.first_execution,
                last_execution = excluded.last_execution,
                updated_at = excluded.updated_at"#,
        )
        .bind(&stats.template_name)
        .bind(stats.template_version as i64)
        .bind(stats.total_tasks as i64)
        .bind(stats.successful_tasks as i64)
        .bind(stats.failed_tasks as i64)
        .bind(stats.goal_violations as i64)
        .bind(stats.success_rate)
        .bind(stats.avg_turns)
        .bind(stats.avg_tokens)
        .bind(&first_exec)
        .bind(&last_exec)
        .bind(&now)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    async fn load_all_stats(
        &self,
    ) -> Result<Vec<TemplateStats>, Box<dyn std::error::Error + Send + Sync>> {
        let rows: Vec<TemplateStatsRow> =
            sqlx::query_as("SELECT * FROM template_stats")
                .fetch_all(&self.pool)
                .await?;

        Ok(rows.into_iter().map(TemplateStats::from).collect())
    }

    async fn save_execution(
        &self,
        execution: &TaskExecution,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let downstream_json = serde_json::to_string(&execution.downstream_tasks)?;
        let outcome_str = outcome_to_str(&execution.outcome);

        sqlx::query(
            r#"INSERT OR IGNORE INTO template_executions
               (id, task_id, template_name, template_version, outcome,
                executed_at, turns_used, tokens_used, downstream_tasks_json)
               VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)"#,
        )
        .bind(Uuid::new_v4().to_string()) // execution row ID
        .bind(execution.task_id.to_string())
        .bind(&execution.template_name)
        .bind(execution.template_version as i64)
        .bind(outcome_str)
        .bind(execution.executed_at.to_rfc3339())
        .bind(execution.turns_used as i64)
        .bind(execution.tokens_used as i64)
        .bind(&downstream_json)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    async fn load_executions(
        &self,
        template_name: &str,
    ) -> Result<Vec<TaskExecution>, Box<dyn std::error::Error + Send + Sync>> {
        let rows: Vec<TemplateExecutionRow> = sqlx::query_as(
            "SELECT * FROM template_executions WHERE template_name = ? ORDER BY executed_at ASC",
        )
        .bind(template_name)
        .fetch_all(&self.pool)
        .await?;

        rows.into_iter()
            .map(TaskExecution::try_from)
            .collect()
    }

    async fn save_version_change(
        &self,
        template_name: &str,
        from_version: u32,
        to_version: u32,
        previous_stats: &TemplateStats,
        changed_at: chrono::DateTime<Utc>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let stats_json = serde_json::to_string(previous_stats)?;

        sqlx::query(
            r#"INSERT INTO template_version_changes
               (template_name, from_version, to_version, previous_stats_json, changed_at)
               VALUES (?, ?, ?, ?, ?)"#,
        )
        .bind(template_name)
        .bind(from_version as i64)
        .bind(to_version as i64)
        .bind(&stats_json)
        .bind(changed_at.to_rfc3339())
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    async fn load_version_changes(
        &self,
    ) -> Result<Vec<VersionChangeRecord>, Box<dyn std::error::Error + Send + Sync>> {
        let rows: Vec<VersionChangeRow> = sqlx::query_as(
            "SELECT * FROM template_version_changes ORDER BY changed_at DESC",
        )
        .fetch_all(&self.pool)
        .await?;

        rows.into_iter()
            .map(VersionChangeRecord::try_from)
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

    #[tokio::test]
    async fn test_stats_survive_restart() {
        let pool = create_migrated_test_pool().await.unwrap();
        let repo = SqliteRefinementRepository::new(pool.clone());

        // Save stats
        let mut stats = TemplateStats::new("persist-agent".to_string(), 1);
        stats.total_tasks = 10;
        stats.successful_tasks = 7;
        stats.failed_tasks = 3;
        stats.success_rate = 0.7;
        stats.avg_turns = 5.0;
        stats.avg_tokens = 500.0;
        repo.save_stats(&stats).await.unwrap();

        // Simulate restart: create a new repo instance against the same pool
        let repo2 = SqliteRefinementRepository::new(pool);
        let loaded = repo2.load_all_stats().await.unwrap();

        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].template_name, "persist-agent");
        assert_eq!(loaded[0].template_version, 1);
        assert_eq!(loaded[0].total_tasks, 10);
        assert_eq!(loaded[0].successful_tasks, 7);
        assert_eq!(loaded[0].failed_tasks, 3);
        assert!((loaded[0].success_rate - 0.7).abs() < 0.001);
        assert!((loaded[0].avg_turns - 5.0).abs() < 0.001);
        assert!((loaded[0].avg_tokens - 500.0).abs() < 0.001);
    }

    #[tokio::test]
    async fn test_version_change_survives_restart() {
        let pool = create_migrated_test_pool().await.unwrap();
        let repo = SqliteRefinementRepository::new(pool.clone());

        let prev_stats = TemplateStats::new("version-agent".to_string(), 1);
        let changed_at = Utc::now();
        repo.save_version_change("version-agent", 1, 2, &prev_stats, changed_at)
            .await
            .unwrap();

        // Simulate restart
        let repo2 = SqliteRefinementRepository::new(pool);
        let changes = repo2.load_version_changes().await.unwrap();

        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].template_name, "version-agent");
        assert_eq!(changes[0].from_version, 1);
        assert_eq!(changes[0].to_version, 2);
        assert_eq!(changes[0].previous_stats.template_name, "version-agent");
    }

    #[tokio::test]
    async fn test_persistence_failure_non_fatal() {
        // Use a repo with a pool that has had its tables dropped to simulate failure.
        // The default no-op trait methods should silently succeed (returning Ok).
        // For the actual implementation, we test that save_stats on a broken pool
        // returns an error rather than panicking.
        use crate::adapters::sqlite::create_test_pool;

        // create_test_pool gives us a pool without migrations — tables don't exist
        let pool = create_test_pool().await.unwrap();
        let repo = SqliteRefinementRepository::new(pool);

        let stats = TemplateStats::new("broken-agent".to_string(), 1);
        let result = repo.save_stats(&stats).await;
        assert!(
            result.is_err(),
            "save_stats should return Err on missing table, not panic"
        );

        let result = repo.load_all_stats().await;
        assert!(
            result.is_err(),
            "load_all_stats should return Err on missing table, not panic"
        );
    }
}
