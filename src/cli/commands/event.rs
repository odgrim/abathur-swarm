//! Event CLI commands for replay, gap detection, and reconciliation.

use anyhow::{Context, Result};
use clap::{Args, Subcommand};
use std::sync::Arc;

use crate::adapters::sqlite::{initialize_default_database, SqliteEventRepository};
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

pub async fn execute(args: EventArgs, json_mode: bool) -> Result<()> {
    let pool = initialize_default_database()
        .await
        .context("Failed to initialize database. Run 'abathur init' first.")?;

    let store = Arc::new(SqliteEventRepository::new(pool));

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
    }

    Ok(())
}
