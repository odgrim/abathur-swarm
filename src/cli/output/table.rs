use comfy_table::{Cell, Color, Table};

use crate::cli::models::{QueueStats, Task, TaskStatus};

/// Format tasks as a table
pub fn format_task_table(tasks: &[Task]) -> Table {
    let mut table = Table::new();
    table.set_header(vec!["ID", "Status", "Description", "Agent", "Priority"]);

    for task in tasks {
        let status_cell = Cell::new(task.status.to_string()).fg(status_color(task.status));

        table.add_row(vec![
            Cell::new(truncate_uuid(&task.id)),
            status_cell,
            Cell::new(truncate_description(&task.description, 40)),
            Cell::new(&task.agent_type),
            Cell::new(format!(
                "{} ({:.1})",
                task.base_priority, task.computed_priority
            )),
        ]);
    }

    table
}

/// Format queue statistics as a table
pub fn format_queue_stats_table(stats: &QueueStats) -> Table {
    let mut table = Table::new();
    table.set_header(vec!["Status", "Count"]);

    table.add_row(vec![Cell::new("Total"), Cell::new(stats.total.to_string())]);

    if stats.pending > 0 {
        table.add_row(vec![
            Cell::new("Pending").fg(Color::Yellow),
            Cell::new(stats.pending.to_string()),
        ]);
    }

    if stats.blocked > 0 {
        table.add_row(vec![
            Cell::new("Blocked").fg(Color::Magenta),
            Cell::new(stats.blocked.to_string()),
        ]);
    }

    if stats.ready > 0 {
        table.add_row(vec![
            Cell::new("Ready").fg(Color::Cyan),
            Cell::new(stats.ready.to_string()),
        ]);
    }

    if stats.running > 0 {
        table.add_row(vec![
            Cell::new("Running").fg(Color::Blue),
            Cell::new(stats.running.to_string()),
        ]);
    }

    if stats.completed > 0 {
        table.add_row(vec![
            Cell::new("Completed").fg(Color::Green),
            Cell::new(stats.completed.to_string()),
        ]);
    }

    if stats.failed > 0 {
        table.add_row(vec![
            Cell::new("Failed").fg(Color::Red),
            Cell::new(stats.failed.to_string()),
        ]);
    }

    if stats.cancelled > 0 {
        table.add_row(vec![
            Cell::new("Cancelled").fg(Color::DarkGrey),
            Cell::new(stats.cancelled.to_string()),
        ]);
    }

    table
}

/// Get color for task status
fn status_color(status: TaskStatus) -> Color {
    match status {
        TaskStatus::Pending => Color::Yellow,
        TaskStatus::Blocked => Color::Magenta,
        TaskStatus::Ready => Color::Cyan,
        TaskStatus::Running => Color::Blue,
        TaskStatus::Completed => Color::Green,
        TaskStatus::Failed => Color::Red,
        TaskStatus::Cancelled => Color::DarkGrey,
    }
}

/// Truncate UUID to first 8 characters
fn truncate_uuid(uuid: &uuid::Uuid) -> String {
    uuid.to_string()[..8].to_string()
}

/// Truncate description to max length
fn truncate_description(desc: &str, max_len: usize) -> String {
    if desc.len() <= max_len {
        desc.to_string()
    } else {
        format!("{}...", &desc[..max_len - 3])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_truncate_uuid() {
        let uuid = uuid::Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap();
        assert_eq!(truncate_uuid(&uuid), "550e8400");
    }

    #[test]
    fn test_truncate_description() {
        assert_eq!(truncate_description("Short", 40), "Short");
        assert_eq!(
            truncate_description("This is a very long description that needs truncating", 20),
            "This is a very lo..."
        );
    }
}
