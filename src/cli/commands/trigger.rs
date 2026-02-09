//! Trigger rule CLI commands.

use anyhow::{Context, Result};
use clap::{Args, Subcommand};
use std::sync::Arc;

use crate::adapters::sqlite::{initialize_default_database, SqliteTriggerRuleRepository};
use crate::cli::id_resolver::resolve_trigger_rule_id;
use crate::cli::output::{output, truncate, CommandOutput};
use crate::domain::ports::TriggerRuleRepository;
use crate::services::trigger_rules::TriggerRule;

#[derive(Args, Debug)]
pub struct TriggerArgs {
    #[command(subcommand)]
    pub command: TriggerCommands,
}

#[derive(Subcommand, Debug)]
pub enum TriggerCommands {
    /// List all trigger rules
    List {
        /// Only show enabled rules
        #[arg(long)]
        enabled_only: bool,
    },
    /// Show trigger rule details
    Show {
        /// Rule ID or name
        id_or_name: String,
    },
    /// Enable a trigger rule
    Enable {
        /// Rule ID or name
        id_or_name: String,
    },
    /// Disable a trigger rule
    Disable {
        /// Rule ID or name
        id_or_name: String,
    },
    /// Delete a trigger rule
    Delete {
        /// Rule ID or name
        id_or_name: String,
    },
    /// Seed built-in trigger rules into the database
    Seed,
}

#[derive(Debug, serde::Serialize)]
pub struct TriggerRuleOutput {
    pub id: String,
    pub name: String,
    pub description: String,
    pub enabled: bool,
    pub fire_count: u64,
    pub last_fired: Option<String>,
}

impl From<&TriggerRule> for TriggerRuleOutput {
    fn from(rule: &TriggerRule) -> Self {
        Self {
            id: rule.id.to_string(),
            name: rule.name.clone(),
            description: truncate(&rule.description, 40),
            enabled: rule.enabled,
            fire_count: rule.fire_count,
            last_fired: rule.last_fired.map(|t| t.to_rfc3339()),
        }
    }
}

#[derive(Debug, serde::Serialize)]
pub struct TriggerListOutput {
    pub rules: Vec<TriggerRuleOutput>,
    pub total: usize,
}

impl CommandOutput for TriggerListOutput {
    fn to_human(&self) -> String {
        if self.rules.is_empty() {
            return "No trigger rules found.".to_string();
        }

        let mut lines = vec![format!("Found {} trigger rule(s):\n", self.total)];
        lines.push(format!(
            "{:<12} {:<30} {:<8} {:<8} {:<30}",
            "ID", "NAME", "ENABLED", "FIRED", "DESCRIPTION"
        ));
        lines.push("-".repeat(90));

        for rule in &self.rules {
            lines.push(format!(
                "{:<12} {:<30} {:<8} {:<8} {:<30}",
                &rule.id[..8],
                truncate(&rule.name, 28),
                if rule.enabled { "yes" } else { "no" },
                rule.fire_count,
                rule.description,
            ));
        }

        lines.join("\n")
    }

    fn to_json(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or_default()
    }
}

#[derive(Debug, serde::Serialize)]
pub struct TriggerDetailOutput {
    pub rule: TriggerRuleOutput,
    pub filter: String,
    pub condition: String,
    pub action: String,
    pub cooldown_secs: Option<u64>,
}

impl CommandOutput for TriggerDetailOutput {
    fn to_human(&self) -> String {
        let mut lines = vec![
            format!("Trigger Rule: {}", self.rule.name),
            format!("ID: {}", self.rule.id),
            format!("Description: {}", self.rule.description),
            format!("Enabled: {}", self.rule.enabled),
            format!("Fire Count: {}", self.rule.fire_count),
        ];

        if let Some(ref last) = self.rule.last_fired {
            lines.push(format!("Last Fired: {}", last));
        }

        if let Some(cooldown) = self.cooldown_secs {
            lines.push(format!("Cooldown: {}s", cooldown));
        }

        lines.push(format!("\nFilter: {}", self.filter));
        lines.push(format!("Condition: {}", self.condition));
        lines.push(format!("Action: {}", self.action));

        lines.join("\n")
    }

    fn to_json(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or_default()
    }
}

#[derive(Debug, serde::Serialize)]
pub struct TriggerActionOutput {
    pub success: bool,
    pub message: String,
}

impl CommandOutput for TriggerActionOutput {
    fn to_human(&self) -> String {
        self.message.clone()
    }

    fn to_json(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or_default()
    }
}

pub async fn execute(args: TriggerArgs, json_mode: bool) -> Result<()> {
    let pool = initialize_default_database()
        .await
        .context("Failed to initialize database. Run 'abathur init' first.")?;

    let repo = Arc::new(SqliteTriggerRuleRepository::new(pool.clone()));

    match args.command {
        TriggerCommands::List { enabled_only } => {
            let rules = if enabled_only {
                repo.list_enabled().await?
            } else {
                repo.list().await?
            };

            let out = TriggerListOutput {
                total: rules.len(),
                rules: rules.iter().map(TriggerRuleOutput::from).collect(),
            };
            output(&out, json_mode);
        }

        TriggerCommands::Show { id_or_name } => {
            let rule = find_rule(&repo, &pool, &id_or_name).await?;

            let out = TriggerDetailOutput {
                rule: TriggerRuleOutput::from(&rule),
                filter: serde_json::to_string_pretty(&rule.filter).unwrap_or_default(),
                condition: serde_json::to_string_pretty(&rule.condition).unwrap_or_default(),
                action: serde_json::to_string_pretty(&rule.action).unwrap_or_default(),
                cooldown_secs: rule.cooldown.map(|d| d.as_secs()),
            };
            output(&out, json_mode);
        }

        TriggerCommands::Enable { id_or_name } => {
            let mut rule = find_rule(&repo, &pool, &id_or_name).await?;
            rule.enabled = true;
            repo.update(&rule).await?;

            let out = TriggerActionOutput {
                success: true,
                message: format!("Trigger rule enabled: {}", rule.name),
            };
            output(&out, json_mode);
        }

        TriggerCommands::Disable { id_or_name } => {
            let mut rule = find_rule(&repo, &pool, &id_or_name).await?;
            rule.enabled = false;
            repo.update(&rule).await?;

            let out = TriggerActionOutput {
                success: true,
                message: format!("Trigger rule disabled: {}", rule.name),
            };
            output(&out, json_mode);
        }

        TriggerCommands::Delete { id_or_name } => {
            let rule = find_rule(&repo, &pool, &id_or_name).await?;
            let name = rule.name.clone();
            repo.delete(rule.id).await?;

            let out = TriggerActionOutput {
                success: true,
                message: format!("Trigger rule deleted: {}", name),
            };
            output(&out, json_mode);
        }

        TriggerCommands::Seed => {
            let builtin = crate::services::trigger_rules::builtin_trigger_rules();
            let mut seeded = 0;

            for rule in builtin {
                // Skip if rule with same name already exists
                if repo.get_by_name(&rule.name).await?.is_none() {
                    repo.create(&rule).await?;
                    seeded += 1;
                }
            }

            let out = TriggerActionOutput {
                success: true,
                message: format!("Seeded {} built-in trigger rule(s)", seeded),
            };
            output(&out, json_mode);
        }
    }

    Ok(())
}

async fn find_rule(
    repo: &Arc<SqliteTriggerRuleRepository>,
    pool: &sqlx::SqlitePool,
    id_or_name: &str,
) -> Result<TriggerRule> {
    // Try by name first
    if let Some(rule) = repo.get_by_name(id_or_name).await? {
        return Ok(rule);
    }

    // Try by UUID (prefix match)
    if let Ok(uuid) = resolve_trigger_rule_id(pool, id_or_name).await {
        if let Some(rule) = repo.get(uuid).await? {
            return Ok(rule);
        }
    }

    anyhow::bail!("Trigger rule not found: {}", id_or_name)
}
