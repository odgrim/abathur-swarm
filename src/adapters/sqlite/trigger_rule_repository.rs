//! SQLite implementation of the TriggerRuleRepository.

use async_trait::async_trait;
use sqlx::SqlitePool;
use std::time::Duration;
use uuid::Uuid;

use crate::adapters::sqlite::{parse_optional_datetime, parse_uuid};
use crate::domain::errors::{DomainError, DomainResult};
use crate::domain::ports::TriggerRuleRepository;
use crate::services::trigger_rules::{
    SerializableEventFilter, TriggerAction, TriggerCondition, TriggerRule,
};

#[derive(Clone)]
pub struct SqliteTriggerRuleRepository {
    pool: SqlitePool,
}

impl SqliteTriggerRuleRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

#[derive(Debug, sqlx::FromRow)]
struct TriggerRuleRow {
    id: String,
    name: String,
    description: String,
    filter_json: String,
    condition_type: String,
    condition_data: Option<String>,
    action_type: String,
    action_data: Option<String>,
    cooldown_secs: Option<i64>,
    enabled: i32,
    last_fired: Option<String>,
    fire_count: i64,
    created_at: String,
}

fn row_to_rule(row: TriggerRuleRow) -> DomainResult<TriggerRule> {
    let id = parse_uuid(&row.id)?;
    let filter: SerializableEventFilter = serde_json::from_str(&row.filter_json)
        .map_err(|e| DomainError::SerializationError(e.to_string()))?;

    let condition = match row.condition_type.as_str() {
        "always" => TriggerCondition::Always,
        "count_threshold" | "absence" => {
            let data = row.condition_data.unwrap_or_default();
            serde_json::from_str(&data)
                .map_err(|e| DomainError::SerializationError(e.to_string()))?
        }
        other => {
            return Err(DomainError::SerializationError(format!(
                "Unknown condition type: {}",
                other
            )));
        }
    };

    let action = match row.action_type.as_str() {
        "emit_event" | "issue_command" | "emit_and_command" => {
            let data = row.action_data.unwrap_or_default();
            serde_json::from_str(&data)
                .map_err(|e| DomainError::SerializationError(e.to_string()))?
        }
        other => {
            return Err(DomainError::SerializationError(format!(
                "Unknown action type: {}",
                other
            )));
        }
    };

    let last_fired = parse_optional_datetime(row.last_fired)?;
    let created_at = chrono::DateTime::parse_from_rfc3339(&row.created_at)
        .map(|dt| dt.with_timezone(&chrono::Utc))
        .map_err(|e| DomainError::SerializationError(e.to_string()))?;

    Ok(TriggerRule {
        id,
        name: row.name,
        description: row.description,
        filter,
        condition,
        action,
        cooldown: row.cooldown_secs.map(|s| Duration::from_secs(s as u64)),
        enabled: row.enabled != 0,
        last_fired,
        fire_count: row.fire_count as u64,
        created_at,
    })
}

#[async_trait]
impl TriggerRuleRepository for SqliteTriggerRuleRepository {
    async fn create(&self, rule: &TriggerRule) -> DomainResult<()> {
        let filter_json = serde_json::to_string(&rule.filter)?;
        let (condition_type, condition_data) = serialize_condition(&rule.condition)?;
        let (action_type, action_data) = serialize_action(&rule.action)?;
        let cooldown_secs = rule.cooldown.map(|d| d.as_secs() as i64);

        sqlx::query(
            r#"INSERT INTO trigger_rules
               (id, name, description, filter_json, condition_type, condition_data,
                action_type, action_data, cooldown_secs, enabled, last_fired, fire_count, created_at)
               VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"#,
        )
        .bind(rule.id.to_string())
        .bind(&rule.name)
        .bind(&rule.description)
        .bind(&filter_json)
        .bind(&condition_type)
        .bind(&condition_data)
        .bind(&action_type)
        .bind(&action_data)
        .bind(cooldown_secs)
        .bind(if rule.enabled { 1i32 } else { 0i32 })
        .bind(rule.last_fired.map(|t| t.to_rfc3339()))
        .bind(rule.fire_count as i64)
        .bind(rule.created_at.to_rfc3339())
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    async fn get(&self, id: Uuid) -> DomainResult<Option<TriggerRule>> {
        let row: Option<TriggerRuleRow> =
            sqlx::query_as("SELECT * FROM trigger_rules WHERE id = ?")
                .bind(id.to_string())
                .fetch_optional(&self.pool)
                .await?;

        row.map(row_to_rule).transpose()
    }

    async fn get_by_name(&self, name: &str) -> DomainResult<Option<TriggerRule>> {
        let row: Option<TriggerRuleRow> =
            sqlx::query_as("SELECT * FROM trigger_rules WHERE name = ?")
                .bind(name)
                .fetch_optional(&self.pool)
                .await?;

        row.map(row_to_rule).transpose()
    }

    async fn update(&self, rule: &TriggerRule) -> DomainResult<()> {
        let filter_json = serde_json::to_string(&rule.filter)?;
        let (condition_type, condition_data) = serialize_condition(&rule.condition)?;
        let (action_type, action_data) = serialize_action(&rule.action)?;
        let cooldown_secs = rule.cooldown.map(|d| d.as_secs() as i64);

        sqlx::query(
            r#"UPDATE trigger_rules SET
               name = ?, description = ?, filter_json = ?,
               condition_type = ?, condition_data = ?,
               action_type = ?, action_data = ?,
               cooldown_secs = ?, enabled = ?,
               last_fired = ?, fire_count = ?,
               updated_at = datetime('now')
               WHERE id = ?"#,
        )
        .bind(&rule.name)
        .bind(&rule.description)
        .bind(&filter_json)
        .bind(&condition_type)
        .bind(&condition_data)
        .bind(&action_type)
        .bind(&action_data)
        .bind(cooldown_secs)
        .bind(if rule.enabled { 1i32 } else { 0i32 })
        .bind(rule.last_fired.map(|t| t.to_rfc3339()))
        .bind(rule.fire_count as i64)
        .bind(rule.id.to_string())
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    async fn delete(&self, id: Uuid) -> DomainResult<()> {
        sqlx::query("DELETE FROM trigger_rules WHERE id = ?")
            .bind(id.to_string())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn list(&self) -> DomainResult<Vec<TriggerRule>> {
        let rows: Vec<TriggerRuleRow> =
            sqlx::query_as("SELECT * FROM trigger_rules ORDER BY name")
                .fetch_all(&self.pool)
                .await?;

        rows.into_iter().map(row_to_rule).collect()
    }

    async fn list_enabled(&self) -> DomainResult<Vec<TriggerRule>> {
        let rows: Vec<TriggerRuleRow> =
            sqlx::query_as("SELECT * FROM trigger_rules WHERE enabled = 1 ORDER BY name")
                .fetch_all(&self.pool)
                .await?;

        rows.into_iter().map(row_to_rule).collect()
    }
}

fn serialize_condition(cond: &TriggerCondition) -> DomainResult<(String, Option<String>)> {
    match cond {
        TriggerCondition::Always => Ok(("always".to_string(), None)),
        TriggerCondition::CountThreshold { .. } => {
            let data = serde_json::to_string(cond)?;
            Ok(("count_threshold".to_string(), Some(data)))
        }
        TriggerCondition::Absence { .. } => {
            let data = serde_json::to_string(cond)?;
            Ok(("absence".to_string(), Some(data)))
        }
    }
}

fn serialize_action(action: &TriggerAction) -> DomainResult<(String, Option<String>)> {
    let type_str = match action {
        TriggerAction::EmitEvent { .. } => "emit_event",
        TriggerAction::IssueCommand { .. } => "issue_command",
        TriggerAction::EmitAndCommand { .. } => "emit_and_command",
    };
    let data = serde_json::to_string(action)?;
    Ok((type_str.to_string(), Some(data)))
}
