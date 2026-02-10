//! Event CLI commands for replay, gap detection, and reconciliation.

use anyhow::{Context, Result};
use clap::{Args, Subcommand};
use std::sync::Arc;

use crate::adapters::sqlite::{initialize_default_database, SqliteEventRepository};
use crate::cli::id_resolver::resolve_dlq_id;
use crate::cli::output::{output, CommandOutput};
use crate::services::event_store::{EventQuery, EventStore};

#[derive(Args, Debug)]
pub struct EventArgs {
    #[command(subcommand)]
    pub command: EventCommands,
}

#[derive(Subcommand, Debug)]
pub enum EventCommands {
    /// Show event store statistics
    Stats,
    /// Detect sequence gaps in the event store
    Gaps {
        /// How far back to scan (number of events)
        #[arg(short, long, default_value = "10000")]
        window: u64,
    },
    /// List recent events
    List {
        /// Maximum number of events to show
        #[arg(short, long, default_value = "20")]
        limit: u32,
        /// Filter by category (task, goal, memory, scheduler, etc.)
        #[arg(short, long)]
        category: Option<String>,
    },
    /// Manage dead letter queue entries
    Dlq {
        #[command(subcommand)]
        command: DlqCommands,
    },
}

#[derive(Subcommand, Debug)]
pub enum DlqCommands {
    /// List unresolved dead letter entries
    List {
        /// Filter by handler name
        #[arg(long)]
        handler: Option<String>,
        /// Maximum number of entries to show
        #[arg(short, long, default_value = "20")]
        limit: u32,
    },
    /// Retry a specific dead letter entry
    Retry {
        /// Dead letter entry ID
        id: String,
    },
    /// Retry all unresolved entries, optionally filtered by handler
    RetryAll {
        /// Filter by handler name
        #[arg(long)]
        handler: Option<String>,
    },
    /// Purge resolved entries older than the specified duration (e.g., "7d", "24h")
    Purge {
        /// Duration threshold (e.g., "7d", "24h", "1h")
        #[arg(long, default_value = "7d")]
        older_than: String,
    },
}

#[derive(Debug, serde::Serialize)]
pub struct EventStatsOutput {
    pub total_events: u64,
    pub latest_sequence: Option<u64>,
    pub oldest_event: Option<String>,
    pub newest_event: Option<String>,
}

impl CommandOutput for EventStatsOutput {
    fn to_human(&self) -> String {
        let mut lines = vec!["Event Store Statistics:".to_string()];
        lines.push(format!("  Total events:     {}", self.total_events));
        lines.push(format!(
            "  Latest sequence:  {}",
            self.latest_sequence
                .map(|s| s.to_string())
                .unwrap_or_else(|| "none".to_string())
        ));
        if let Some(ref oldest) = self.oldest_event {
            lines.push(format!("  Oldest event:     {}", oldest));
        }
        if let Some(ref newest) = self.newest_event {
            lines.push(format!("  Newest event:     {}", newest));
        }
        lines.join("\n")
    }

    fn to_json(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or_default()
    }
}

#[derive(Debug, serde::Serialize)]
pub struct GapReport {
    pub gaps: Vec<GapEntry>,
    pub total_gaps: usize,
    pub scan_from: u64,
    pub scan_to: u64,
}

#[derive(Debug, serde::Serialize)]
pub struct GapEntry {
    pub start: u64,
    pub end: u64,
    pub missing: u64,
}

impl CommandOutput for GapReport {
    fn to_human(&self) -> String {
        if self.gaps.is_empty() {
            return format!(
                "No sequence gaps found in range [{}, {}].",
                self.scan_from, self.scan_to
            );
        }

        let mut lines = vec![format!(
            "Found {} gap(s) in range [{}, {}]:\n",
            self.total_gaps, self.scan_from, self.scan_to
        )];
        lines.push(format!("{:<12} {:<12} {:<10}", "START", "END", "MISSING"));
        lines.push("-".repeat(34));
        for gap in &self.gaps {
            lines.push(format!(
                "{:<12} {:<12} {:<10}",
                gap.start, gap.end, gap.missing
            ));
        }
        lines.join("\n")
    }

    fn to_json(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or_default()
    }
}

#[derive(Debug, serde::Serialize)]
pub struct EventListOutput {
    pub events: Vec<EventEntry>,
    pub total: usize,
}

#[derive(Debug, serde::Serialize)]
pub struct EventEntry {
    pub sequence: u64,
    pub timestamp: String,
    pub category: String,
    pub severity: String,
    pub payload_type: String,
}

impl CommandOutput for EventListOutput {
    fn to_human(&self) -> String {
        if self.events.is_empty() {
            return "No events found.".to_string();
        }

        let mut lines = vec![format!("Showing {} event(s):\n", self.total)];
        lines.push(format!(
            "{:<8} {:<24} {:<14} {:<10} {:<30}",
            "SEQ", "TIMESTAMP", "CATEGORY", "SEVERITY", "PAYLOAD"
        ));
        lines.push("-".repeat(86));
        for e in &self.events {
            lines.push(format!(
                "{:<8} {:<24} {:<14} {:<10} {:<30}",
                e.sequence, e.timestamp, e.category, e.severity, e.payload_type
            ));
        }
        lines.join("\n")
    }

    fn to_json(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or_default()
    }
}

#[derive(Debug, serde::Serialize)]
pub struct DlqListOutput {
    pub entries: Vec<DlqEntry>,
    pub total: usize,
}

#[derive(Debug, serde::Serialize)]
pub struct DlqEntry {
    pub id: String,
    pub event_sequence: u64,
    pub handler_name: String,
    pub error_message: String,
    pub retry_count: u32,
    pub max_retries: u32,
    pub created_at: String,
}

impl CommandOutput for DlqListOutput {
    fn to_human(&self) -> String {
        if self.entries.is_empty() {
            return "No dead letter entries found.".to_string();
        }

        let mut lines = vec![format!("Showing {} DLQ entry(ies):\n", self.total)];
        lines.push(format!(
            "{:<38} {:<8} {:<20} {:<8} {:<40}",
            "ID", "SEQ", "HANDLER", "RETRIES", "ERROR"
        ));
        lines.push("-".repeat(114));
        for e in &self.entries {
            let error_truncated = if e.error_message.len() > 40 {
                format!("{}...", &e.error_message[..37])
            } else {
                e.error_message.clone()
            };
            lines.push(format!(
                "{:<38} {:<8} {:<20} {}/{:<5} {:<40}",
                e.id, e.event_sequence, e.handler_name, e.retry_count, e.max_retries, error_truncated
            ));
        }
        lines.join("\n")
    }

    fn to_json(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or_default()
    }
}

#[derive(Debug, serde::Serialize)]
pub struct DlqActionOutput {
    pub message: String,
    pub count: u64,
}

impl CommandOutput for DlqActionOutput {
    fn to_human(&self) -> String {
        self.message.clone()
    }

    fn to_json(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or_default()
    }
}

fn parse_duration(s: &str) -> Result<std::time::Duration> {
    let s = s.trim();
    if s.is_empty() {
        anyhow::bail!("Empty duration string");
    }
    let (num_str, suffix) = if s.ends_with('d') {
        (&s[..s.len() - 1], "d")
    } else if s.ends_with('h') {
        (&s[..s.len() - 1], "h")
    } else if s.ends_with('m') {
        (&s[..s.len() - 1], "m")
    } else {
        anyhow::bail!("Duration must end with 'd', 'h', or 'm' (e.g., '7d', '24h', '30m')");
    };
    let num: u64 = num_str.parse().context("Invalid number in duration")?;
    match suffix {
        "d" => Ok(std::time::Duration::from_secs(num * 86400)),
        "h" => Ok(std::time::Duration::from_secs(num * 3600)),
        "m" => Ok(std::time::Duration::from_secs(num * 60)),
        _ => unreachable!(),
    }
}

pub async fn execute(args: EventArgs, json_mode: bool) -> Result<()> {
    let pool = initialize_default_database()
        .await
        .context("Failed to initialize database. Run 'abathur init' first.")?;

    let store = Arc::new(SqliteEventRepository::new(pool.clone()));

    match args.command {
        EventCommands::Stats => {
            let stats = store
                .stats()
                .await
                .map_err(|e| anyhow::anyhow!("Failed to get stats: {}", e))?;

            let out = EventStatsOutput {
                total_events: stats.total_events,
                latest_sequence: stats.latest_sequence.map(|s| s.0),
                oldest_event: stats.oldest_event.map(|t| t.to_rfc3339()),
                newest_event: stats.newest_event.map(|t| t.to_rfc3339()),
            };
            output(&out, json_mode);
        }

        EventCommands::Gaps { window } => {
            let latest = store
                .latest_sequence()
                .await
                .map_err(|e| anyhow::anyhow!("Failed to get latest sequence: {}", e))?;

            let to = latest.map(|s| s.0).unwrap_or(0);
            let from = to.saturating_sub(window);

            let raw_gaps = store
                .detect_sequence_gaps(from, to)
                .await
                .map_err(|e| anyhow::anyhow!("Failed to detect gaps: {}", e))?;

            let gaps: Vec<GapEntry> = raw_gaps
                .into_iter()
                .map(|(start, end)| GapEntry {
                    start,
                    end,
                    missing: end - start + 1,
                })
                .collect();

            let out = GapReport {
                total_gaps: gaps.len(),
                gaps,
                scan_from: from,
                scan_to: to,
            };
            output(&out, json_mode);
        }

        EventCommands::List { limit, category } => {
            use crate::services::event_bus::EventCategory;

            let mut query = EventQuery::new().limit(limit).descending();

            if let Some(cat) = category {
                let parsed = match cat.to_lowercase().as_str() {
                    "orchestrator" => Some(EventCategory::Orchestrator),
                    "goal" => Some(EventCategory::Goal),
                    "task" => Some(EventCategory::Task),
                    "execution" => Some(EventCategory::Execution),
                    "agent" => Some(EventCategory::Agent),
                    "verification" => Some(EventCategory::Verification),
                    "escalation" => Some(EventCategory::Escalation),
                    "memory" => Some(EventCategory::Memory),
                    "scheduler" => Some(EventCategory::Scheduler),
                    _ => None,
                };
                if let Some(c) = parsed {
                    query = query.category(c);
                }
            }

            let events = store
                .query(query)
                .await
                .map_err(|e| anyhow::anyhow!("Failed to query events: {}", e))?;

            let entries: Vec<EventEntry> = events
                .iter()
                .map(|e| EventEntry {
                    sequence: e.sequence.0,
                    timestamp: e.timestamp.format("%Y-%m-%d %H:%M:%S").to_string(),
                    category: format!("{}", e.category),
                    severity: format!("{}", e.severity),
                    payload_type: e.payload.variant_name().to_string(),
                })
                .collect();

            let out = EventListOutput {
                total: entries.len(),
                events: entries,
            };
            output(&out, json_mode);
        }

        EventCommands::Dlq { command } => match command {
            DlqCommands::List { handler, limit } => {
                let entries = store
                    .list_dead_letters(handler.as_deref(), limit)
                    .await
                    .map_err(|e| anyhow::anyhow!("Failed to list DLQ entries: {}", e))?;

                let dlq_entries: Vec<DlqEntry> = entries
                    .iter()
                    .map(|e| DlqEntry {
                        id: e.id.clone(),
                        event_sequence: e.event_sequence,
                        handler_name: e.handler_name.clone(),
                        error_message: e.error_message.clone(),
                        retry_count: e.retry_count,
                        max_retries: e.max_retries,
                        created_at: e.created_at.format("%Y-%m-%d %H:%M:%S").to_string(),
                    })
                    .collect();

                let out = DlqListOutput {
                    total: dlq_entries.len(),
                    entries: dlq_entries,
                };
                output(&out, json_mode);
            }
            DlqCommands::Retry { id } => {
                let resolved_id = resolve_dlq_id(&pool, &id).await?;
                store
                    .resolve_dead_letter(&resolved_id)
                    .await
                    .map_err(|e| anyhow::anyhow!("Failed to resolve DLQ entry: {}", e))?;

                let out = DlqActionOutput {
                    message: format!("Resolved DLQ entry {}", resolved_id),
                    count: 1,
                };
                output(&out, json_mode);
            }
            DlqCommands::RetryAll { handler } => {
                let entries = store
                    .list_dead_letters(handler.as_deref(), 1000)
                    .await
                    .map_err(|e| anyhow::anyhow!("Failed to list DLQ entries: {}", e))?;

                let mut resolved = 0u64;
                for entry in &entries {
                    if let Err(e) = store.resolve_dead_letter(&entry.id).await {
                        tracing::warn!("Failed to resolve DLQ entry {}: {}", entry.id, e);
                    } else {
                        resolved += 1;
                    }
                }

                let out = DlqActionOutput {
                    message: format!("Resolved {} DLQ entries", resolved),
                    count: resolved,
                };
                output(&out, json_mode);
            }
            DlqCommands::Purge { older_than } => {
                let duration = parse_duration(&older_than)?;
                let purged = store
                    .purge_dead_letters(duration)
                    .await
                    .map_err(|e| anyhow::anyhow!("Failed to purge DLQ entries: {}", e))?;

                let out = DlqActionOutput {
                    message: format!("Purged {} resolved DLQ entries", purged),
                    count: purged,
                };
                output(&out, json_mode);
            }
        },
    }

    Ok(())
}
