//! Memory CLI commands.

use anyhow::{Context, Result};
use clap::{Args, Subcommand};
use std::sync::Arc;

use crate::adapters::sqlite::{SqliteMemoryRepository, initialize_default_database};
use crate::cli::command_dispatcher::CliCommandDispatcher;
use crate::cli::display::{
    CommandOutput, DetailView, action_success, colorize_memory_tier, list_table, output,
    relative_time_str, render_list, short_id, truncate_ellipsis,
};
use crate::cli::id_resolver::resolve_memory_id;
use crate::domain::models::{AccessorId, Memory, MemoryQuery, MemoryTier, MemoryType};
use crate::services::MemoryService;
use crate::services::command_bus::{CommandResult, DomainCommand, MemoryCommand};

#[derive(Args, Debug)]
pub struct MemoryArgs {
    #[command(subcommand)]
    pub command: MemoryCommands,
}

#[derive(Subcommand, Debug)]
pub enum MemoryCommands {
    /// Create a new memory
    #[command(visible_alias = "store")]
    Create {
        /// Memory key
        key: String,
        /// Memory content
        content: String,
        /// Namespace
        #[arg(short, long, default_value = "default")]
        namespace: String,
        /// Tier (working, episodic, semantic)
        #[arg(short, long, default_value = "working")]
        tier: String,
        /// Type (fact, code, decision, error, pattern, reference, context)
        #[arg(long, default_value = "fact")]
        memory_type: String,
    },
    /// Show a memory by ID or key
    #[command(visible_alias = "recall")]
    Show {
        /// Memory ID or key
        id_or_key: String,
        /// Namespace (required if using key)
        #[arg(short, long)]
        namespace: Option<String>,
    },
    /// Search memories
    Search {
        /// Search query
        query: String,
        /// Namespace filter
        #[arg(short, long)]
        namespace: Option<String>,
        /// Maximum results
        #[arg(short, long, default_value = "10")]
        limit: usize,
    },
    /// List memories
    List {
        /// Filter by namespace
        #[arg(short, long)]
        namespace: Option<String>,
        /// Filter by tier
        #[arg(short, long)]
        tier: Option<String>,
        /// Filter by type (fact, code, decision, error, pattern, reference, context)
        #[arg(long = "type")]
        memory_type: Option<String>,
        /// Maximum results
        #[arg(short, long, default_value = "20")]
        limit: usize,
    },
    /// Update a memory's content or metadata
    Update {
        /// Memory ID
        id: String,
        /// New content
        #[arg(short, long)]
        content: Option<String>,
        /// New namespace
        #[arg(short, long)]
        namespace: Option<String>,
        /// New tier (working, episodic, semantic)
        #[arg(short, long)]
        tier: Option<String>,
    },
    /// Delete a memory
    #[command(visible_alias = "forget")]
    Delete {
        /// Memory ID
        id: String,
    },
    /// Run maintenance (prune expired and decayed)
    Prune {
        /// Only prune expired (skip decay check)
        #[arg(long)]
        expired_only: bool,
    },
    /// Show memory statistics
    Stats,
}

#[derive(Debug, serde::Serialize)]
pub struct MemoryOutput {
    pub id: String,
    pub key: String,
    pub namespace: String,
    pub tier: String,
    pub memory_type: String,
    pub access_count: u32,
    pub decay_factor: f32,
    pub content_preview: String,
}

impl From<&Memory> for MemoryOutput {
    fn from(mem: &Memory) -> Self {
        Self {
            id: mem.id.to_string(),
            key: mem.key.clone(),
            namespace: mem.namespace.clone(),
            tier: mem.tier.as_str().to_string(),
            memory_type: mem.memory_type.as_str().to_string(),
            access_count: mem.access_count,
            decay_factor: mem.decay_factor(),
            content_preview: truncate_ellipsis(&mem.content, 50),
        }
    }
}

#[derive(Debug, serde::Serialize)]
pub struct MemoryListOutput {
    pub memories: Vec<MemoryOutput>,
    pub total: usize,
}

impl CommandOutput for MemoryListOutput {
    fn to_human(&self) -> String {
        if self.memories.is_empty() {
            return "No memories found.".to_string();
        }

        let mut table = list_table(&["ID", "Key", "Namespace", "Tier", "Type"]);

        for mem in &self.memories {
            table.add_row(vec![
                short_id(&mem.id).to_string(),
                truncate_ellipsis(&mem.key, 40),
                truncate_ellipsis(&mem.namespace, 15),
                colorize_memory_tier(&mem.tier).to_string(),
                mem.memory_type.clone(),
            ]);
        }

        render_list("memory", table, self.total)
    }

    fn to_json(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or_default()
    }
}

#[derive(Debug, serde::Serialize)]
pub struct MemoryDetailOutput {
    pub memory: MemoryOutput,
    pub content: String,
    pub created_at: String,
    pub last_accessed: String,
    pub expires_at: Option<String>,
    pub tags: Vec<String>,
}

impl CommandOutput for MemoryDetailOutput {
    fn to_human(&self) -> String {
        let mut view = DetailView::new(&self.memory.key)
            .field("ID", &self.memory.id)
            .field("Namespace", &self.memory.namespace)
            .field("Tier", &colorize_memory_tier(&self.memory.tier).to_string())
            .field("Type", &self.memory.memory_type)
            .field("Accesses", &self.memory.access_count.to_string())
            .field("Decay", &format!("{:.2}", self.memory.decay_factor))
            .section("Content")
            .item(&self.content);

        if !self.tags.is_empty() {
            view = view.section("Tags").item(&self.tags.join(", "));
        }

        view = view
            .section("Timing")
            .field(
                "Created",
                &format!(
                    "{} ({})",
                    relative_time_str(&self.created_at),
                    &self.created_at
                ),
            )
            .field(
                "Accessed",
                &format!(
                    "{} ({})",
                    relative_time_str(&self.last_accessed),
                    &self.last_accessed
                ),
            )
            .field(
                "Expires",
                &self
                    .expires_at
                    .as_deref()
                    .map(|s| format!("{} ({})", relative_time_str(s), s))
                    .unwrap_or_else(|| "-".to_string()),
            );

        view.render()
    }

    fn to_json(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or_default()
    }
}

#[derive(Debug, serde::Serialize)]
pub struct MemoryActionOutput {
    pub success: bool,
    pub message: String,
    pub memory: Option<MemoryOutput>,
}

impl CommandOutput for MemoryActionOutput {
    fn to_human(&self) -> String {
        if self.success {
            action_success(&self.message)
        } else {
            crate::cli::display::action_failure(&self.message)
        }
    }

    fn to_json(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or_default()
    }
}

#[derive(Debug, serde::Serialize)]
pub struct MemoryStatsOutput {
    pub working: u64,
    pub episodic: u64,
    pub semantic: u64,
    pub total: u64,
}

impl CommandOutput for MemoryStatsOutput {
    fn to_human(&self) -> String {
        use colored::Colorize;
        DetailView::new("Memory Statistics")
            .field(
                &colorize_memory_tier("Working").to_string(),
                &self.working.to_string(),
            )
            .field(
                &colorize_memory_tier("Episodic").to_string(),
                &self.episodic.to_string(),
            )
            .field(
                &colorize_memory_tier("Semantic").to_string(),
                &self.semantic.to_string(),
            )
            .field("Total", &self.total.to_string().bold().to_string())
            .render()
    }

    fn to_json(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or_default()
    }
}

#[derive(Debug, serde::Serialize)]
pub struct PruneOutput {
    pub expired_pruned: u64,
    pub decayed_pruned: u64,
    pub promoted: u64,
    pub conflicts_resolved: u64,
}

impl CommandOutput for PruneOutput {
    fn to_human(&self) -> String {
        action_success(&format!(
            "Maintenance complete: {} expired, {} decayed, {} promoted, {} conflicts resolved",
            self.expired_pruned, self.decayed_pruned, self.promoted, self.conflicts_resolved
        ))
    }

    fn to_json(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or_default()
    }
}

pub async fn execute(args: MemoryArgs, json_mode: bool) -> Result<()> {
    let pool = initialize_default_database()
        .await
        .context("Failed to initialize database. Run 'abathur init' first.")?;

    let repo = Arc::new(SqliteMemoryRepository::new(pool.clone()));
    let event_bus = crate::cli::event_helpers::create_persistent_event_bus(pool.clone()).await;
    let service = MemoryService::new(repo);
    let dispatcher = CliCommandDispatcher::new(pool.clone(), event_bus);

    match args.command {
        MemoryCommands::Create {
            key,
            content,
            namespace,
            tier,
            memory_type,
        } => {
            let tier = MemoryTier::from_str(&tier)
                .ok_or_else(|| anyhow::anyhow!("Invalid tier: {}", tier))?;
            let mtype = MemoryType::from_str(&memory_type)
                .ok_or_else(|| anyhow::anyhow!("Invalid memory type: {}", memory_type))?;

            let cmd = DomainCommand::Memory(MemoryCommand::Store {
                key,
                content,
                namespace,
                tier,
                memory_type: mtype,
                metadata: None,
            });

            let result = dispatcher
                .dispatch(cmd)
                .await
                .map_err(|e| anyhow::anyhow!("{}", e))?;

            let memory = match result {
                CommandResult::Memory(m) => m,
                _ => anyhow::bail!("Unexpected command result"),
            };

            let out = MemoryActionOutput {
                success: true,
                message: format!(
                    "Memory created: {} (tier: {})",
                    memory.id,
                    memory.tier.as_str()
                ),
                memory: Some(MemoryOutput::from(&memory)),
            };
            output(&out, json_mode);
        }

        MemoryCommands::Show {
            id_or_key,
            namespace,
        } => {
            let cmd = if let Ok(uuid) = resolve_memory_id(&pool, &id_or_key).await {
                DomainCommand::Memory(MemoryCommand::Recall {
                    id: uuid,
                    accessor: AccessorId::system("cli"),
                })
            } else {
                let ns = namespace.unwrap_or_else(|| "default".to_string());
                DomainCommand::Memory(MemoryCommand::RecallByKey {
                    key: id_or_key.clone(),
                    namespace: ns,
                    accessor: AccessorId::system("cli"),
                })
            };

            let result = dispatcher
                .dispatch(cmd)
                .await
                .map_err(|e| anyhow::anyhow!("{}", e))?;

            match result {
                CommandResult::MemoryOpt(Some(mem)) => {
                    let out = MemoryDetailOutput {
                        memory: MemoryOutput::from(&mem),
                        content: mem.content.clone(),
                        created_at: mem.created_at.to_rfc3339(),
                        last_accessed: mem.last_accessed.to_rfc3339(),
                        expires_at: mem.expires_at.map(|t| t.to_rfc3339()),
                        tags: mem.metadata.tags.clone(),
                    };
                    output(&out, json_mode);
                }
                CommandResult::MemoryOpt(None) => {
                    let out = MemoryActionOutput {
                        success: false,
                        message: format!("Memory not found: {}", id_or_key),
                        memory: None,
                    };
                    output(&out, json_mode);
                }
                _ => anyhow::bail!("Unexpected command result"),
            }
        }

        MemoryCommands::Search {
            query,
            namespace,
            limit,
        } => {
            let memories = service.search(&query, namespace.as_deref(), limit).await?;

            let out = MemoryListOutput {
                total: memories.len(),
                memories: memories.iter().map(MemoryOutput::from).collect(),
            };
            output(&out, json_mode);
        }

        MemoryCommands::List {
            namespace,
            tier,
            memory_type,
            limit,
        } => {
            let query = MemoryQuery {
                namespace,
                tier: tier.as_ref().and_then(|t| MemoryTier::from_str(t)),
                memory_type: memory_type.as_ref().and_then(|t| MemoryType::from_str(t)),
                limit: Some(limit),
                ..Default::default()
            };

            let memories = service.query(query).await?;

            let out = MemoryListOutput {
                total: memories.len(),
                memories: memories.iter().map(MemoryOutput::from).collect(),
            };
            output(&out, json_mode);
        }

        MemoryCommands::Update {
            id,
            content,
            namespace,
            tier,
        } => {
            let uuid = resolve_memory_id(&pool, &id).await?;
            let tier = tier
                .map(|t| {
                    MemoryTier::from_str(&t).ok_or_else(|| anyhow::anyhow!("Invalid tier: {}", t))
                })
                .transpose()?;

            let cmd = DomainCommand::Memory(MemoryCommand::Update {
                id: uuid,
                content,
                namespace,
                tier,
            });

            let result = dispatcher
                .dispatch(cmd)
                .await
                .map_err(|e| anyhow::anyhow!("{}", e))?;

            let memory = match result {
                CommandResult::Memory(m) => m,
                _ => anyhow::bail!("Unexpected command result"),
            };

            let out = MemoryActionOutput {
                success: true,
                message: format!("Memory updated: {}", memory.id),
                memory: Some(MemoryOutput::from(&memory)),
            };
            output(&out, json_mode);
        }

        MemoryCommands::Delete { id } => {
            let uuid = resolve_memory_id(&pool, &id).await?;

            let cmd = DomainCommand::Memory(MemoryCommand::Forget { id: uuid });

            dispatcher
                .dispatch(cmd)
                .await
                .map_err(|e| anyhow::anyhow!("{}", e))?;

            let out = MemoryActionOutput {
                success: true,
                message: format!("Memory deleted: {}", id),
                memory: None,
            };
            output(&out, json_mode);
        }

        MemoryCommands::Prune { expired_only } => {
            let cmd = if expired_only {
                DomainCommand::Memory(MemoryCommand::PruneExpired)
            } else {
                DomainCommand::Memory(MemoryCommand::RunMaintenance)
            };

            let result = dispatcher
                .dispatch(cmd)
                .await
                .map_err(|e| anyhow::anyhow!("{}", e))?;

            let report = match result {
                CommandResult::MaintenanceReport(r) => r,
                CommandResult::PruneCount(count) => crate::services::MaintenanceReport {
                    expired_pruned: count,
                    decayed_pruned: 0,
                    promoted: 0,
                    conflicts_resolved: 0,
                },
                _ => anyhow::bail!("Unexpected command result"),
            };

            let out = PruneOutput {
                expired_pruned: report.expired_pruned,
                decayed_pruned: report.decayed_pruned,
                promoted: report.promoted,
                conflicts_resolved: report.conflicts_resolved,
            };
            output(&out, json_mode);
        }

        MemoryCommands::Stats => {
            let stats = service.get_stats().await?;

            let out = MemoryStatsOutput {
                working: stats.working_count,
                episodic: stats.episodic_count,
                semantic: stats.semantic_count,
                total: stats.total(),
            };
            output(&out, json_mode);
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    /// Wrapper to parse MemoryArgs from CLI tokens.
    #[derive(Parser, Debug)]
    struct Cli {
        #[command(subcommand)]
        command: MemoryCommands,
    }

    #[test]
    fn parse_list_with_type_flag() {
        let cli = Cli::parse_from(["memory", "list", "--type", "decision"]);
        match cli.command {
            MemoryCommands::List { memory_type, .. } => {
                assert_eq!(memory_type.as_deref(), Some("decision"));
            }
            _ => panic!("Expected List command"),
        }
    }

    #[test]
    fn parse_list_without_type_flag() {
        let cli = Cli::parse_from(["memory", "list"]);
        match cli.command {
            MemoryCommands::List { memory_type, .. } => {
                assert!(memory_type.is_none());
            }
            _ => panic!("Expected List command"),
        }
    }

    #[test]
    fn parse_list_with_all_filters() {
        let cli = Cli::parse_from([
            "memory",
            "list",
            "--namespace",
            "test-ns",
            "--tier",
            "semantic",
            "--type",
            "error",
            "--limit",
            "5",
        ]);
        match cli.command {
            MemoryCommands::List {
                namespace,
                tier,
                memory_type,
                limit,
            } => {
                assert_eq!(namespace.as_deref(), Some("test-ns"));
                assert_eq!(tier.as_deref(), Some("semantic"));
                assert_eq!(memory_type.as_deref(), Some("error"));
                assert_eq!(limit, 5);
            }
            _ => panic!("Expected List command"),
        }
    }
}
