//! Memory CLI commands.

use anyhow::{Context, Result};
use clap::{Args, Subcommand};
use std::sync::Arc;

use crate::adapters::sqlite::{SqliteMemoryRepository, initialize_default_database};
use crate::cli::command_dispatcher::CliCommandDispatcher;
use crate::cli::id_resolver::resolve_memory_id;
use crate::cli::output::{output, truncate, CommandOutput};
use crate::domain::models::{Memory, MemoryQuery, MemoryTier, MemoryType};
use crate::services::command_bus::{CommandResult, DomainCommand, MemoryCommand};
use crate::services::MemoryService;

#[derive(Args, Debug)]
pub struct MemoryArgs {
    #[command(subcommand)]
    pub command: MemoryCommands,
}

#[derive(Subcommand, Debug)]
pub enum MemoryCommands {
    /// Store a memory
    Store {
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
    /// Recall a memory by ID or key
    Recall {
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
        /// Maximum results
        #[arg(short, long, default_value = "20")]
        limit: usize,
    },
    /// Delete a memory
    Forget {
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
            content_preview: truncate(&mem.content, 50),
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

        let mut lines = vec![format!("Found {} memory(ies):\n", self.total)];
        lines.push(format!(
            "{:<12} {:<15} {:<12} {:<10} {:<6} {:<30}",
            "ID", "KEY", "NAMESPACE", "TIER", "DECAY", "CONTENT"
        ));
        lines.push("-".repeat(90));

        for mem in &self.memories {
            lines.push(format!(
                "{:<12} {:<15} {:<12} {:<10} {:<6.2} {:<30}",
                &mem.id[..8],
                truncate(&mem.key, 13),
                truncate(&mem.namespace, 10),
                mem.tier,
                mem.decay_factor,
                mem.content_preview
            ));
        }

        lines.join("\n")
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
        let mut lines = vec![
            format!("Memory: {}", self.memory.key),
            format!("ID: {}", self.memory.id),
            format!("Namespace: {}", self.memory.namespace),
            format!("Tier: {}", self.memory.tier),
            format!("Type: {}", self.memory.memory_type),
            format!("Access Count: {}", self.memory.access_count),
            format!("Decay Factor: {:.2}", self.memory.decay_factor),
            format!("\nContent:\n{}", self.content),
            format!("\nCreated: {}", self.created_at),
            format!("Last Accessed: {}", self.last_accessed),
        ];

        if let Some(exp) = &self.expires_at {
            lines.push(format!("Expires: {}", exp));
        }

        if !self.tags.is_empty() {
            lines.push(format!("Tags: {}", self.tags.join(", ")));
        }

        lines.join("\n")
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
        self.message.clone()
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
        let mut lines = vec!["Memory Statistics:".to_string()];
        lines.push(format!("  Working:   {}", self.working));
        lines.push(format!("  Episodic:  {}", self.episodic));
        lines.push(format!("  Semantic:  {}", self.semantic));
        lines.push("  -----------".to_string());
        lines.push(format!("  Total:     {}", self.total));
        lines.join("\n")
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
        let mut lines = vec!["Maintenance complete:".to_string()];
        lines.push(format!("  Expired pruned:      {}", self.expired_pruned));
        lines.push(format!("  Decayed pruned:      {}", self.decayed_pruned));
        lines.push(format!("  Promoted:            {}", self.promoted));
        lines.push(format!("  Conflicts resolved:  {}", self.conflicts_resolved));
        lines.join("\n")
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
    let event_bus = crate::cli::event_helpers::create_persistent_event_bus(pool.clone());
    let service = MemoryService::new_with_event_bus(repo, event_bus.clone());
    let dispatcher = CliCommandDispatcher::new(pool.clone(), event_bus);

    match args.command {
        MemoryCommands::Store { key, content, namespace, tier, memory_type } => {
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

            let result = dispatcher.dispatch(cmd).await
                .map_err(|e| anyhow::anyhow!("{}", e))?;

            let memory = match result {
                CommandResult::Memory(m) => m,
                _ => anyhow::bail!("Unexpected command result"),
            };

            let out = MemoryActionOutput {
                success: true,
                message: format!("Memory stored: {} (tier: {})", memory.id, memory.tier.as_str()),
                memory: Some(MemoryOutput::from(&memory)),
            };
            output(&out, json_mode);
        }

        MemoryCommands::Recall { id_or_key, namespace } => {
            let cmd = if let Ok(uuid) = resolve_memory_id(&pool, &id_or_key).await {
                DomainCommand::Memory(MemoryCommand::Recall { id: uuid })
            } else {
                let ns = namespace.unwrap_or_else(|| "default".to_string());
                DomainCommand::Memory(MemoryCommand::RecallByKey { key: id_or_key.clone(), namespace: ns })
            };

            let result = dispatcher.dispatch(cmd).await
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

        MemoryCommands::Search { query, namespace, limit } => {
            let memories = service.search(&query, namespace.as_deref(), limit).await?;

            let out = MemoryListOutput {
                total: memories.len(),
                memories: memories.iter().map(MemoryOutput::from).collect(),
            };
            output(&out, json_mode);
        }

        MemoryCommands::List { namespace, tier, limit } => {
            let query = MemoryQuery {
                namespace,
                tier: tier.as_ref().and_then(|t| MemoryTier::from_str(t)),
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

        MemoryCommands::Forget { id } => {
            let uuid = resolve_memory_id(&pool, &id).await?;

            let cmd = DomainCommand::Memory(MemoryCommand::Forget { id: uuid });

            dispatcher.dispatch(cmd).await
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

            let result = dispatcher.dispatch(cmd).await
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

