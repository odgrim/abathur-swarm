use anyhow::{Context, Result};
use uuid::Uuid;

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
    dependencies: Vec<Uuid>,
    json: bool,
) -> Result<()> {
    let task_id = service
        .submit_task(
            description.clone(),
            agent_type.clone(),
            priority,
            dependencies.clone(),
        )
        .await
        .context("Failed to submit task")?;

    if json {
        let mut output = serde_json::json!({
            "task_id": task_id,
            "description": description,
            "agent_type": agent_type,
            "priority": priority,
            "dependencies": dependencies,
        });
        if let Some(summary_text) = &summary {
            output["summary"] = serde_json::json!(summary_text);
        }
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        println!("Task submitted successfully!");
        println!("  Task ID: {}", task_id);
        println!("  Description: {}", description);
        if let Some(summary_text) = &summary {
            println!("  Summary: {}", summary_text);
        }
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
pub async fn handle_show(service: &TaskQueueServiceAdapter, task_id: Uuid, json: bool) -> Result<()> {
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
    task_ids: Vec<Uuid>,
    status: Option<String>,
    priority: Option<u8>,
    agent_type: Option<String>,
    add_dependency: Vec<Uuid>,
    remove_dependency: Vec<Uuid>,
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
                add_dependency.clone(),
                remove_dependency.clone(),
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
