use anyhow::{Context, Result};

use crate::cli::models::Memory as CliMemory;
use crate::cli::output::table::format_memory_table;
use crate::cli::service::MemoryService;
use crate::domain::models::MemoryType;

/// Handle memory list command
pub async fn handle_list(
    service: &MemoryService,
    namespace_prefix: Option<String>,
    memory_type: Option<MemoryType>,
    limit: usize,
    json: bool,
) -> Result<()> {
    let prefix = namespace_prefix.as_deref().unwrap_or("");
    let memories = service
        .search(prefix, memory_type, Some(limit))
        .await
        .context("Failed to list memories")?;

    if json {
        println!("{}", serde_json::to_string_pretty(&memories)?);
    } else {
        if memories.is_empty() {
            println!("No memories found.");
            return Ok(());
        }

        // Convert domain memories to CLI models for display
        let cli_memories: Vec<CliMemory> = memories
            .into_iter()
            .map(|m| CliMemory {
                namespace: m.namespace,
                key: m.key,
                value: m.value,
                memory_type: match m.memory_type {
                    crate::domain::models::MemoryType::Semantic => {
                        crate::cli::models::MemoryType::Semantic
                    }
                    crate::domain::models::MemoryType::Episodic => {
                        crate::cli::models::MemoryType::Episodic
                    }
                    crate::domain::models::MemoryType::Procedural => {
                        crate::cli::models::MemoryType::Procedural
                    }
                },
                version: m.version,
                created_by: m.created_by,
                created_at: m.created_at,
                updated_at: m.updated_at,
            })
            .collect();

        println!("Memories:");
        println!("{}", format_memory_table(&cli_memories));
        println!("\nShowing {} memor{}", cli_memories.len(), if cli_memories.len() == 1 { "y" } else { "ies" });
    }

    Ok(())
}

/// Handle memory show command
pub async fn handle_show(
    service: &MemoryService,
    namespace: String,
    key: String,
    version: Option<u32>,
    json: bool,
) -> Result<()> {
    let memory = if let Some(v) = version {
        service
            .get_version(&namespace, &key, v)
            .await
            .context("Failed to retrieve memory version")?
    } else {
        service
            .get(&namespace, &key)
            .await
            .context("Failed to retrieve memory")?
    };

    let memory = memory.ok_or_else(|| {
        anyhow::anyhow!(
            "Memory not found at {}:{}{}",
            namespace,
            key,
            version.map(|v| format!(" (version {})", v)).unwrap_or_default()
        )
    })?;

    if json {
        println!("{}", serde_json::to_string_pretty(&memory)?);
    } else {
        println!("\nMemory Details:");
        println!("─────────────────────────────────────────");
        println!("Namespace:   {}", memory.namespace);
        println!("Key:         {}", memory.key);
        println!("Type:        {}", memory.memory_type);
        println!("Version:     {}", memory.version);
        println!("Created by:  {}", memory.created_by);
        println!("Updated by:  {}", memory.updated_by);
        println!("Created at:  {}", memory.created_at.format("%Y-%m-%d %H:%M:%S UTC"));
        println!("Updated at:  {}", memory.updated_at.format("%Y-%m-%d %H:%M:%S UTC"));
        println!("\nValue:");
        println!("{}", serde_json::to_string_pretty(&memory.value)?);

        if let Some(metadata) = &memory.metadata {
            println!("\nMetadata:");
            println!("{}", serde_json::to_string_pretty(metadata)?);
        }
    }

    Ok(())
}

/// Handle memory versions command
pub async fn handle_versions(
    service: &MemoryService,
    namespace: String,
    key: String,
    json: bool,
) -> Result<()> {
    let versions = service
        .list_versions(&namespace, &key)
        .await
        .context("Failed to list memory versions")?;

    if json {
        println!("{}", serde_json::to_string_pretty(&versions)?);
    } else {
        if versions.is_empty() {
            println!("No versions found for {}:{}", namespace, key);
            return Ok(());
        }

        println!("\nVersion History for {}:{}", namespace, key);
        println!("─────────────────────────────────────────");

        for memory in versions {
            let status = if memory.is_deleted { " [DELETED]" } else { "" };
            println!("\nVersion {}{}", memory.version, status);
            println!("  Updated by: {}", memory.updated_by);
            println!("  Updated at: {}", memory.updated_at.format("%Y-%m-%d %H:%M:%S UTC"));
            println!("  Value: {}", serde_json::to_string(&memory.value)?);
        }
    }

    Ok(())
}

/// Handle memory count command
pub async fn handle_count(
    service: &MemoryService,
    namespace_prefix: String,
    memory_type: Option<MemoryType>,
    json: bool,
) -> Result<()> {
    let count = service
        .count(&namespace_prefix, memory_type)
        .await
        .context("Failed to count memories")?;

    if json {
        let output = serde_json::json!({
            "namespace_prefix": namespace_prefix,
            "memory_type": memory_type.map(|t| t.to_string()),
            "count": count,
        });
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        let type_str = memory_type.map(|t| format!(" {} ", t)).unwrap_or_else(|| " ".to_string());
        println!("Found {}{}memories matching prefix '{}'", count, type_str, namespace_prefix);
    }

    Ok(())
}
