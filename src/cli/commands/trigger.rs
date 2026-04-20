//! Trigger rule CLI commands.

use anyhow::{Context, Result};
use chrono::Utc;
use clap::{Args, Subcommand};
use std::str::FromStr;
use std::sync::Arc;

use crate::adapters::sqlite::{SqliteTriggerRuleRepository, initialize_default_database};
use crate::cli::display::{
    CommandOutput, DetailView, action_failure, action_success, list_table, output,
    relative_time_str, render_list, short_id, truncate_ellipsis,
};
use crate::cli::id_resolver::resolve_trigger_rule_id;
use crate::domain::ports::TriggerRuleRepository;
use crate::services::event_bus::EventCategory;
use crate::services::trigger_rules::{
    SerializableDomainCommand, SerializableEventFilter, TriggerAction, TriggerCondition,
    TriggerRule, normalize_cron_expression, validate_cron_expression,
};

#[derive(Args, Debug)]
pub struct TriggerArgs {
    #[command(subcommand)]
    pub command: TriggerCommands,
}

/// Arguments for `trigger create`. Extracted into a named [`Args`] struct so
/// the large payload lives behind a pointer once — keeps `TriggerCommands`
/// lean for pattern matches and codegen.
#[derive(Args, Debug)]
pub struct TriggerCreateArgs {
    /// Trigger name (unique identifier)
    #[arg(long)]
    pub name: String,

    /// Description of the trigger
    #[arg(long, default_value = "")]
    pub description: String,

    /// Cron expression (5-field: min hour dom month dow)
    #[arg(long, group = "trigger_type")]
    pub cron: Option<String>,

    /// Event type to trigger on (e.g., "TaskFailed", "GoalStarted")
    #[arg(long, group = "trigger_type")]
    pub on_event: Option<String>,

    /// Event category filter (task, goal, memory, scheduler, etc.)
    #[arg(long)]
    pub category: Option<String>,

    /// Prompt / task description to run when triggered
    #[arg(long, alias = "task-description")]
    pub prompt: String,

    /// Title for created tasks (defaults to trigger name)
    #[arg(long)]
    pub task_title: Option<String>,

    /// Priority for created tasks (low, normal, high, critical)
    #[arg(long, default_value = "normal")]
    pub priority: String,

    /// Agent type to assign to created tasks
    #[arg(long)]
    pub agent_type: Option<String>,

    /// Minimum seconds between firings
    #[arg(long)]
    pub cooldown: Option<u64>,
}

#[derive(Subcommand, Debug)]
pub enum TriggerCommands {
    /// Create a new trigger rule
    Create(Box<TriggerCreateArgs>),
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
    /// Update a trigger rule's properties
    Update {
        /// Rule ID or name
        id_or_name: String,
        /// New description
        #[arg(long)]
        description: Option<String>,
        /// New cron expression (5-field: min hour dom month dow)
        #[arg(long)]
        cron: Option<String>,
        /// New cooldown in seconds
        #[arg(long)]
        cooldown: Option<u64>,
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
    pub condition_type: String,
    pub enabled: bool,
    pub fire_count: u64,
    pub last_fired: Option<String>,
    pub next_fire_at: Option<String>,
}

/// Compute the next fire time for a cron trigger condition.
fn compute_trigger_next_fire(condition: &TriggerCondition) -> Option<String> {
    if let TriggerCondition::Cron { expression } = condition {
        let normalized = normalize_cron_expression(expression);
        let sched = cron::Schedule::from_str(&normalized).ok()?;
        let next = sched.upcoming(Utc).next()?;
        Some(next.to_rfc3339())
    } else {
        None
    }
}

impl From<&TriggerRule> for TriggerRuleOutput {
    fn from(rule: &TriggerRule) -> Self {
        let condition_type = match &rule.condition {
            TriggerCondition::Always => "always".to_string(),
            TriggerCondition::CountThreshold { .. } => "count_threshold".to_string(),
            TriggerCondition::Absence { .. } => "absence".to_string(),
            TriggerCondition::Cron { expression } => format!("cron({})", expression),
        };
        let next_fire_at = if rule.enabled {
            compute_trigger_next_fire(&rule.condition)
        } else {
            None
        };
        Self {
            id: rule.id.to_string(),
            name: rule.name.clone(),
            description: truncate_ellipsis(&rule.description, 40),
            condition_type,
            enabled: rule.enabled,
            fire_count: rule.fire_count,
            last_fired: rule.last_fired.map(|t| t.to_rfc3339()),
            next_fire_at,
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

        let mut table = list_table(&[
            "ID",
            "Name",
            "Type",
            "Enabled",
            "Next Fire",
            "Fires",
            "Description",
        ]);

        for rule in &self.rules {
            let next_fire = rule
                .next_fire_at
                .as_deref()
                .map(relative_time_str)
                .unwrap_or_else(|| "-".to_string());
            table.add_row(vec![
                short_id(&rule.id).to_string(),
                truncate_ellipsis(&rule.name, 28),
                truncate_ellipsis(&rule.condition_type, 20),
                if rule.enabled {
                    "yes".to_string()
                } else {
                    "no".to_string()
                },
                next_fire,
                rule.fire_count.to_string(),
                truncate_ellipsis(&rule.description, 35),
            ]);
        }

        render_list("trigger rule", table, self.total)
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
        let mut view = DetailView::new(&self.rule.name)
            .field("ID", &self.rule.id)
            .field("Description", &self.rule.description)
            .field("Enabled", if self.rule.enabled { "yes" } else { "no" })
            .field("Fires", &self.rule.fire_count.to_string());

        if let Some(ref next) = self.rule.next_fire_at {
            view = view.field("Next Fire", &relative_time_str(next));
        }

        if let Some(ref last) = self.rule.last_fired {
            view = view.field("Last Fired", &relative_time_str(last));
        }

        if let Some(cooldown) = self.cooldown_secs {
            view = view.field("Cooldown", &format!("{}s", cooldown));
        }

        view = view
            .section("Configuration")
            .field("Filter", &self.filter)
            .field("Condition", &self.condition)
            .field("Action", &self.action);

        view.render()
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
        if self.success {
            action_success(&self.message)
        } else {
            action_failure(&self.message)
        }
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
        TriggerCommands::Create(args) => {
            let TriggerCreateArgs {
                name,
                description,
                cron,
                on_event,
                category,
                prompt,
                task_title,
                priority,
                agent_type,
                cooldown,
            } = *args;
            // Validate and normalize cron expression if provided
            let cron = if let Some(expr) = cron {
                validate_cron_expression(&expr).map_err(|e| anyhow::anyhow!(e))?;
                Some(normalize_cron_expression(&expr))
            } else {
                None
            };

            // Build the filter
            let filter = if cron.is_some() {
                // Cron triggers listen for their own ScheduledEventFired events
                TriggerRule::cron_event_filter()
            } else if let Some(ref event_type) = on_event {
                // Event-driven trigger: filter by payload type
                let cats = if let Some(ref cat) = category {
                    vec![parse_event_category(cat)?]
                } else {
                    vec![] // match all categories
                };
                SerializableEventFilter {
                    categories: cats,
                    min_severity: None,
                    payload_types: vec![event_type.clone()],
                    goal_id: None,
                    task_id: None,
                }
            } else {
                anyhow::bail!("Must specify either --cron or --on-event");
            };

            // Build the condition
            let condition = if let Some(ref expr) = cron {
                TriggerCondition::Cron {
                    expression: expr.to_string(),
                }
            } else {
                TriggerCondition::Always
            };

            // Build the action — always SubmitTask for CLI-created triggers
            let task_title_str = task_title.unwrap_or_else(|| name.clone());
            let action = TriggerAction::IssueCommand {
                command: SerializableDomainCommand::SubmitTask {
                    title: task_title_str,
                    description: prompt,
                    priority,
                    agent_type,
                },
            };

            // Build the rule
            let mut rule = TriggerRule::new(name, filter, action)
                .with_description(description)
                .with_condition(condition);

            if let Some(cd) = cooldown {
                rule = rule.with_cooldown(cd);
            }

            // Check for duplicate name
            if repo.get_by_name(&rule.name).await?.is_some() {
                anyhow::bail!("A trigger rule with name '{}' already exists", rule.name);
            }

            // Persist
            repo.create(&rule).await?;

            let out = TriggerActionOutput {
                success: true,
                message: format!(
                    "Trigger rule created: {} ({})",
                    rule.name,
                    if cron.is_some() { "cron" } else { "event" }
                ),
            };
            output(&out, json_mode);
        }

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

        TriggerCommands::Update {
            id_or_name,
            description,
            cron,
            cooldown,
        } => {
            let mut rule = find_rule(&repo, &pool, &id_or_name).await?;

            if let Some(desc) = description {
                rule.description = desc;
            }
            if let Some(expr) = cron {
                validate_cron_expression(&expr).map_err(|e| anyhow::anyhow!(e))?;
                let normalized = normalize_cron_expression(&expr);
                rule.condition = TriggerCondition::Cron {
                    expression: normalized,
                };
                rule.filter = TriggerRule::cron_event_filter();
            }
            if let Some(cd) = cooldown {
                rule.cooldown = Some(std::time::Duration::from_secs(cd));
            }

            repo.update(&rule).await?;

            let out = TriggerActionOutput {
                success: true,
                message: format!("Trigger rule updated: {}", rule.name),
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

fn parse_event_category(s: &str) -> Result<EventCategory> {
    match s.to_lowercase().as_str() {
        "orchestrator" => Ok(EventCategory::Orchestrator),
        "goal" => Ok(EventCategory::Goal),
        "task" => Ok(EventCategory::Task),
        "execution" => Ok(EventCategory::Execution),
        "agent" => Ok(EventCategory::Agent),
        "verification" => Ok(EventCategory::Verification),
        "escalation" => Ok(EventCategory::Escalation),
        "memory" => Ok(EventCategory::Memory),
        "scheduler" => Ok(EventCategory::Scheduler),
        "convergence" => Ok(EventCategory::Convergence),
        "workflow" => Ok(EventCategory::Workflow),
        "adapter" => Ok(EventCategory::Adapter),
        "budget" => Ok(EventCategory::Budget),
        other => anyhow::bail!(
            "Invalid event category: '{}'. Valid categories: orchestrator, goal, task, execution, agent, verification, escalation, memory, scheduler, convergence, workflow, adapter, budget",
            other
        ),
    }
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
    if let Ok(uuid) = resolve_trigger_rule_id(pool, id_or_name).await
        && let Some(rule) = repo.get(uuid).await?
    {
        return Ok(rule);
    }

    anyhow::bail!("Trigger rule not found: {}", id_or_name)
}
