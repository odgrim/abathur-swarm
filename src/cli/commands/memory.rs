use anyhow::{Context, Result};

use crate::cli::models::Memory as CliMemory;
use crate::cli::output::table::format_memory_table;
use crate::cli::service::MemoryServiceAdapter;
use crate::domain::models::MemoryType;

/// Handle memory list command
pub async fn handle_list(
    service: &MemoryServiceAdapter,
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

/// Handle memory show command with fuzzy matching support
/// If exact match is not found, attempts fuzzy matching on namespace and key
pub async fn handle_show(
    service: &MemoryServiceAdapter,
    namespace: String,
    key: String,
    json: bool,
) -> Result<()> {
    // First try exact match
    let memory = service
        .get(&namespace, &key)
        .await
        .context("Failed to retrieve memory")?;

    let memory = if let Some(mem) = memory {
        mem
    } else {
        // Exact match failed, try fuzzy matching
        fuzzy_find_memory(service, &namespace, &key).await?
    };

    if json {
        println!("{}", serde_json::to_string_pretty(&memory)?);
    } else {
        println!("\nMemory Details:");
        println!("─────────────────────────────────────────");
        println!("Namespace:   {}", memory.namespace);
        println!("Key:         {}", memory.key);
        println!("Type:        {}", memory.memory_type);
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

/// Fuzzy find a memory by namespace and key patterns
/// Returns error if no match or multiple matches found
async fn fuzzy_find_memory(
    service: &MemoryServiceAdapter,
    namespace_pattern: &str,
    key_pattern: &str,
) -> Result<crate::domain::models::Memory> {
    // Search for memories with namespace prefix
    let all_memories = service
        .search("", None, Some(1000))
        .await
        .context("Failed to search memories for fuzzy matching")?;

    // Filter memories that match both namespace and key patterns
    let matches: Vec<_> = all_memories
        .into_iter()
        .filter(|m| {
            m.namespace.contains(namespace_pattern) && m.key.contains(key_pattern)
        })
        .collect();

    match matches.len() {
        0 => Err(anyhow::anyhow!(
            "No memory found matching namespace pattern '{}' and key pattern '{}'",
            namespace_pattern,
            key_pattern
        )),
        1 => {
            let memory = &matches[0];
            eprintln!(
                "Found unique match: {}:{}",
                memory.namespace, memory.key
            );
            Ok(matches.into_iter().next().unwrap())
        }
        n => {
            let mut error_msg = format!(
                "Found {} memories matching namespace pattern '{}' and key pattern '{}'.\n\nAmbiguous matches:\n",
                n, namespace_pattern, key_pattern
            );
            for m in &matches {
                error_msg.push_str(&format!("  - {}:{}\n", m.namespace, m.key));
            }
            error_msg.push_str("\nPlease provide more specific namespace and key patterns.");
            Err(anyhow::anyhow!(error_msg))
        }
    }
}

/// Handle memory count command
pub async fn handle_count(
    service: &MemoryServiceAdapter,
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
