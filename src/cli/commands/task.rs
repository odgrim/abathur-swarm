use anyhow::{Context, Result};
use uuid::Uuid;

use crate::cli::models::TaskStatus;
use crate::cli::output::table::{format_queue_stats_table, format_task_table};
use crate::cli::service::TaskQueueService;

/// Handle task submit command
pub async fn handle_submit(
    service: &TaskQueueService,
    description: String,
    agent_type: String,
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
        let output = serde_json::json!({
            "task_id": task_id,
            "description": description,
            "agent_type": agent_type,
            "priority": priority,
            "dependencies": dependencies,
        });
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        println!("Task submitted successfully!");
        println!("  Task ID: {}", task_id);
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
    service: &TaskQueueService,
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
pub async fn handle_show(service: &TaskQueueService, task_id: Uuid, json: bool) -> Result<()> {
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

/// Handle task cancel command
pub async fn handle_cancel(service: &TaskQueueService, task_id: Uuid, json: bool) -> Result<()> {
    service
        .cancel_task(task_id)
        .await
        .context(format!("Failed to cancel task {}", task_id))?;

    if json {
        let output = serde_json::json!({
            "task_id": task_id,
            "status": "cancelled",
        });
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        println!("Task {} cancelled successfully.", task_id);
    }

    Ok(())
}

/// Handle task retry command
pub async fn handle_retry(service: &TaskQueueService, task_id: Uuid, json: bool) -> Result<()> {
    let new_task_id = service
        .retry_task(task_id)
        .await
        .context(format!("Failed to retry task {}", task_id))?;

    if json {
        let output = serde_json::json!({
            "original_task_id": task_id,
            "new_task_id": new_task_id,
        });
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        println!("Task {} retried successfully.", task_id);
        println!("  New task ID: {}", new_task_id);
    }

    Ok(())
}

/// Handle task status command
pub async fn handle_status(service: &TaskQueueService, json: bool) -> Result<()> {
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
