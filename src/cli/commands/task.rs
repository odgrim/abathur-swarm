use anyhow::{Context, Result};

use crate::cli::models::TaskStatus;
use crate::cli::output::table::{format_queue_stats_table, format_task_table};
use crate::cli::service::TaskQueueServiceAdapter;

/// Handle task submit command
pub async fn handle_submit(
    service: &TaskQueueServiceAdapter,
    description: String,
    agent_type: String,
    summary: Option<String>,
    priority: u8,
    dependencies: Vec<String>,
    json: bool,
) -> Result<()> {
    // Resolve dependency prefixes to full UUIDs
    let mut resolved_deps = Vec::new();
    for dep_prefix in &dependencies {
        let dep_id = service
            .resolve_task_id_prefix(dep_prefix)
            .await
            .context(format!("Failed to resolve dependency '{}'", dep_prefix))?;
        resolved_deps.push(dep_id);
    }

    // Generate summary from description if not provided
    // Take first 140 characters (max summary length)
    let task_summary = summary.unwrap_or_else(|| {
        if description.len() <= 140 {
            description.clone()
        } else {
            format!("{}...", &description[..137])
        }
    });

    let task_id = service
        .submit_task(
            task_summary.clone(),
            description.clone(),
            agent_type.clone(),
            priority,
            resolved_deps.clone(),
        )
        .await
        .context("Failed to submit task")?;

    if json {
        let output = serde_json::json!({
            "task_id": task_id,
            "summary": task_summary,
            "description": description,
            "agent_type": agent_type,
            "priority": priority,
            "dependencies": resolved_deps,
        });
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        println!("Task submitted successfully!");
        println!("  Task ID: {}", task_id);
        println!("  Summary: {}", task_summary);
        println!("  Description: {}", description);
        println!("  Agent type: {}", agent_type);
        println!("  Priority: {}", priority);
        if !dependencies.is_empty() {
            println!("  Dependencies: {} task(s)", dependencies.len());
        }
    }

    Ok(())
}

/// Handle task list command
pub async fn handle_list(
    service: &TaskQueueServiceAdapter,
    status_filter: Option<TaskStatus>,
    limit: usize,
    json: bool,
) -> Result<()> {
    let tasks = service
        .list_tasks(status_filter, limit)
        .await
        .context("Failed to list tasks")?;

    if json {
        println!("{}", serde_json::to_string_pretty(&tasks)?);
    } else {
        if tasks.is_empty() {
            println!("No tasks found.");
            return Ok(());
        }

        println!("Tasks:");
        println!("{}", format_task_table(&tasks));
        println!("\nShowing {} task(s)", tasks.len());
    }

    Ok(())
}

/// Handle task show command
pub async fn handle_show(service: &TaskQueueServiceAdapter, task_id_prefix: String, json: bool) -> Result<()> {
    // Resolve task ID prefix
    let task_id = service
        .resolve_task_id_prefix(&task_id_prefix)
        .await
        .context(format!("Failed to resolve task ID '{}'", task_id_prefix))?;

    let task = service
        .get_task(task_id)
        .await
        .context("Failed to retrieve task")?
        .ok_or_else(|| {
            anyhow::anyhow!(
                "Task {} not found. Use 'abathur task list' to see available tasks.",
                task_id
            )
        })?;

    if json {
        println!("{}", serde_json::to_string_pretty(&task)?);
    } else {
        println!("Task Details:");
        println!("  ID: {}", task.id);
        println!("  Status: {}", task.status);
        println!("  Summary: {}", task.summary);
        println!("  Description: {}", task.description);
        println!("  Agent type: {}", task.agent_type);
        println!(
            "  Priority: {} (computed: {:.1})",
            task.base_priority, task.computed_priority
        );
        println!(
            "  Created at: {}",
            task.created_at.format("%Y-%m-%d %H:%M:%S UTC")
        );
        println!(
            "  Updated at: {}",
            task.updated_at.format("%Y-%m-%d %H:%M:%S UTC")
        );

        if let Some(started_at) = task.started_at {
            println!(
                "  Started at: {}",
                started_at.format("%Y-%m-%d %H:%M:%S UTC")
            );
        }

        if let Some(completed_at) = task.completed_at {
            println!(
                "  Completed at: {}",
                completed_at.format("%Y-%m-%d %H:%M:%S UTC")
            );
        }

        if !task.dependencies.is_empty() {
            println!("  Dependencies:");
            for dep in &task.dependencies {
                println!("    - {}", dep);
            }
        }
    }

    Ok(())
}

/// Handle task update command
pub async fn handle_update(
    service: &TaskQueueServiceAdapter,
    task_id_prefixes: Vec<String>,
    status: Option<String>,
    priority: Option<u8>,
    agent_type: Option<String>,
    add_dependency: Vec<String>,
    remove_dependency: Vec<String>,
    retry: bool,
    cancel: bool,
    json: bool,
) -> Result<()> {
    // Validate that at least one update operation is specified
    if status.is_none()
        && priority.is_none()
        && agent_type.is_none()
        && add_dependency.is_empty()
        && remove_dependency.is_empty()
        && !retry
        && !cancel
    {
        return Err(anyhow::anyhow!(
            "At least one update operation must be specified (--status, --priority, --agent-type, --add-dependency, --remove-dependency, --retry, or --cancel)"
        ));
    }

    // Resolve all task ID prefixes
    let mut task_ids = Vec::new();
    for prefix in &task_id_prefixes {
        let task_id = service
            .resolve_task_id_prefix(prefix)
            .await
            .context(format!("Failed to resolve task ID '{}'", prefix))?;
        task_ids.push(task_id);
    }

    // Resolve dependency prefixes
    let mut resolved_add_deps = Vec::new();
    for dep_prefix in &add_dependency {
        let dep_id = service
            .resolve_task_id_prefix(dep_prefix)
            .await
            .context(format!("Failed to resolve add-dependency '{}'", dep_prefix))?;
        resolved_add_deps.push(dep_id);
    }

    let mut resolved_remove_deps = Vec::new();
    for dep_prefix in &remove_dependency {
        let dep_id = service
            .resolve_task_id_prefix(dep_prefix)
            .await
            .context(format!("Failed to resolve remove-dependency '{}'", dep_prefix))?;
        resolved_remove_deps.push(dep_id);
    }

    let mut results = Vec::new();
    let mut errors = Vec::new();

    // Update each task
    for task_id in &task_ids {
        let result = service
            .update_task(
                *task_id,
                status.as_deref(),
                priority,
                agent_type.clone(),
                resolved_add_deps.clone(),
                resolved_remove_deps.clone(),
                retry,
                cancel,
            )
            .await;

        match result {
            Ok(()) => results.push(*task_id),
            Err(e) => errors.push((*task_id, e)),
        }
    }

    if json {
        let output = serde_json::json!({
            "successful": results,
            "failed": errors.iter().map(|(id, e)| {
                serde_json::json!({
                    "task_id": id,
                    "error": e.to_string()
                })
            }).collect::<Vec<_>>(),
        });
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        if !results.is_empty() {
            println!("Successfully updated {} task(s):", results.len());
            for task_id in results {
                println!("  - {}", task_id);
            }
        }

        if !errors.is_empty() {
            println!("\nFailed to update {} task(s):", errors.len());
            for (task_id, error) in errors {
                println!("  - {}: {}", task_id, error);
            }
        }
    }

    Ok(())
}

/// Handle task status command
pub async fn handle_status(service: &TaskQueueServiceAdapter, json: bool) -> Result<()> {
    let stats = service
        .get_queue_stats()
        .await
        .context("Failed to get queue statistics")?;

    if json {
        println!("{}", serde_json::to_string_pretty(&stats)?);
    } else {
        println!("Queue Status:");
        println!("{}", format_queue_stats_table(&stats));
    }

    Ok(())
}

/// Handle task resolve command
///
/// Resolves dependencies for all Pending/Blocked tasks and updates them to Ready
/// if their dependencies are satisfied.
pub async fn handle_resolve(service: &TaskQueueServiceAdapter, json: bool) -> Result<()> {
    let count = service
        .resolve_dependencies()
        .await
        .context("Failed to resolve task dependencies")?;

    if json {
        let output = serde_json::json!({
            "status": "success",
            "tasks_updated": count,
            "message": format!("{} task(s) updated to Ready status", count)
        });
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        println!("Task Dependency Resolution");
        println!("=========================");
        println!("Tasks updated to Ready: {}", count);

        if count > 0 {
            println!("\nRun 'abathur task list --status ready' to view ready tasks.");
        } else {
            println!("\nNo tasks were ready to be updated.");
            println!("Check 'abathur task list --status pending' or '--status blocked' for pending tasks.");
        }
    }

    Ok(())
}
