<<<<<<< HEAD
//! Table output formatting for CLI commands
//!
//! Provides formatted table output for tasks, agents, and MCP servers using comfy-table.
//! Supports color-coded cells, automatic column sizing, and accessibility features.

use crate::domain::models::{Agent, AgentStatus, McpServerConfig, Task, TaskStatus};
use comfy_table::{presets, Attribute, Cell, Color, ContentArrangement, Table};
use std::env;

/// Table formatter for CLI output
pub struct TableFormatter {
    /// Whether to use colors in output
    use_colors: bool,
    /// Maximum width for tables (None = auto)
    max_width: Option<usize>,
}

impl TableFormatter {
    /// Create a new table formatter
    pub fn new() -> Self {
        Self {
            use_colors: supports_color(),
            max_width: None,
        }
    }

    /// Create a new table formatter with custom settings
    pub fn with_config(use_colors: bool, max_width: Option<usize>) -> Self {
        Self {
            use_colors,
            max_width,
        }
    }

    /// Format a list of tasks as a table
    pub fn format_tasks(&self, tasks: &[Task]) -> String {
        let mut table = self.create_base_table();

        // Header row
        table.set_header(vec![
            Cell::new("ID").add_attribute(Attribute::Bold),
            Cell::new("Summary").add_attribute(Attribute::Bold),
            Cell::new("Status").add_attribute(Attribute::Bold),
            Cell::new("Priority").add_attribute(Attribute::Bold),
            Cell::new("Agent").add_attribute(Attribute::Bold),
            Cell::new("Branch").add_attribute(Attribute::Bold),
        ]);

        // Data rows
        for task in tasks {
            let id_short = &task.id.to_string()[..8];
            let summary = truncate_text(&task.summary, 40);

            let status_cell = if self.use_colors {
                Cell::new(&task.status.to_string())
                    .fg(status_color(&task.status))
            } else {
                Cell::new(&format!("{} {}", status_icon(&task.status), task.status))
            };

            let priority_cell = if self.use_colors {
                Cell::new(task.priority.to_string())
                    .fg(priority_color(task.priority))
            } else {
                Cell::new(task.priority.to_string())
            };

            let branch = task.task_branch.as_deref().unwrap_or("-");

            table.add_row(vec![
                Cell::new(id_short),
                Cell::new(&summary),
                status_cell,
                priority_cell,
                Cell::new(&task.agent_type),
                Cell::new(truncate_text(branch, 30)),
            ]);
        }

        table.to_string()
    }

    /// Format a list of agents as a table
    pub fn format_agents(&self, agents: &[Agent]) -> String {
        let mut table = self.create_base_table();

        // Header row
        table.set_header(vec![
            Cell::new("ID").add_attribute(Attribute::Bold),
            Cell::new("Type").add_attribute(Attribute::Bold),
            Cell::new("Status").add_attribute(Attribute::Bold),
            Cell::new("Current Task").add_attribute(Attribute::Bold),
            Cell::new("Memory (MB)").add_attribute(Attribute::Bold),
            Cell::new("CPU %").add_attribute(Attribute::Bold),
        ]);

        // Data rows
        for agent in agents {
            let id_short = &agent.id.to_string()[..8];

            let status_cell = if self.use_colors {
                Cell::new(&agent.status.to_string())
                    .fg(agent_status_color(&agent.status))
            } else {
                Cell::new(&format!("{} {}", agent_status_icon(&agent.status), agent.status))
            };

            let task_id = agent.current_task_id
                .map(|id| id.to_string()[..8].to_string())
                .unwrap_or_else(|| "-".to_string());

            let memory_mb = agent.memory_usage_bytes / (1024 * 1024);
            let cpu_str = format!("{:.1}", agent.cpu_usage_percent);

            table.add_row(vec![
                Cell::new(id_short),
                Cell::new(&agent.agent_type),
                status_cell,
                Cell::new(&task_id),
                Cell::new(memory_mb.to_string()),
                Cell::new(&cpu_str),
            ]);
        }

        table.to_string()
    }

    /// Format a list of MCP servers as a table
    pub fn format_mcp_servers(&self, servers: &[McpServerConfig]) -> String {
        let mut table = self.create_base_table();

        // Header row
        table.set_header(vec![
            Cell::new("Name").add_attribute(Attribute::Bold),
            Cell::new("Command").add_attribute(Attribute::Bold),
            Cell::new("Args").add_attribute(Attribute::Bold),
            Cell::new("Env Vars").add_attribute(Attribute::Bold),
        ]);

        // Data rows
        for server in servers {
            let args_str = if server.args.is_empty() {
                "-".to_string()
            } else {
                server.args.join(" ")
            };

            let env_count = if server.env.is_empty() {
                "-".to_string()
            } else {
                format!("{} vars", server.env.len())
            };

            table.add_row(vec![
                Cell::new(&server.name),
                Cell::new(truncate_text(&server.command, 30)),
                Cell::new(truncate_text(&args_str, 40)),
                Cell::new(&env_count),
            ]);
        }

        table.to_string()
    }

    /// Create a base table with common settings
    fn create_base_table(&self) -> Table {
        let mut table = Table::new();

        // Use UTF-8 preset for nice borders
        table.load_preset(presets::UTF8_FULL)
            .set_content_arrangement(ContentArrangement::Dynamic);

        // Apply max width if set
        if let Some(width) = self.max_width {
            table.set_width(width as u16);
        }

        table
    }
}

impl Default for TableFormatter {
    fn default() -> Self {
        Self::new()
    }
}

/// Check if color output is supported
fn supports_color() -> bool {
    // Respect NO_COLOR environment variable
    if env::var("NO_COLOR").is_ok() {
        return false;
    }

    // Check for dumb terminal
    if let Ok(term) = env::var("TERM") {
        if term == "dumb" {
            return false;
        }
    }

    true
}

/// Map task status to color
fn status_color(status: &TaskStatus) -> Color {
    match status {
        TaskStatus::Completed => Color::Green,
        TaskStatus::Running => Color::Cyan,
        TaskStatus::Failed => Color::Red,
        TaskStatus::Cancelled => Color::DarkGrey,
        TaskStatus::Ready => Color::Yellow,
        TaskStatus::Blocked => Color::Magenta,
        TaskStatus::Pending => Color::White,
    }
}

/// Map task status to icon
fn status_icon(status: &TaskStatus) -> &'static str {
    match status {
        TaskStatus::Completed => "✓",
        TaskStatus::Running => "⟳",
        TaskStatus::Failed => "✗",
        TaskStatus::Cancelled => "⊘",
        TaskStatus::Ready => "●",
        TaskStatus::Blocked => "⊗",
        TaskStatus::Pending => "○",
    }
}

/// Map priority to color (high = red, low = blue)
fn priority_color(priority: u8) -> Color {
    match priority {
        8..=10 => Color::Red,
        5..=7 => Color::Yellow,
        _ => Color::Blue,
    }
}

/// Map agent status to color
fn agent_status_color(status: &AgentStatus) -> Color {
    match status {
        AgentStatus::Idle => Color::Green,
        AgentStatus::Busy => Color::Cyan,
        AgentStatus::Terminated => Color::DarkGrey,
    }
}

/// Map agent status to icon
fn agent_status_icon(status: &AgentStatus) -> &'static str {
    match status {
        AgentStatus::Idle => "○",
        AgentStatus::Busy => "●",
        AgentStatus::Terminated => "✗",
    }
}

/// Truncate text to max length with ellipsis
fn truncate_text(text: &str, max_len: usize) -> String {
    if text.len() <= max_len {
        text.to_string()
    } else {
        format!("{}...", &text[..max_len.saturating_sub(3)])
=======
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
>>>>>>> task_cli-structure_20251025-210033
    }
}

#[cfg(test)]
mod tests {
    use super::*;
<<<<<<< HEAD
    use chrono::Utc;
    use uuid::Uuid;

    #[test]
    fn test_table_formatter_new() {
        let formatter = TableFormatter::new();
        assert_eq!(formatter.max_width, None);
    }

    #[test]
    fn test_table_formatter_with_config() {
        let formatter = TableFormatter::with_config(false, Some(120));
        assert!(!formatter.use_colors);
        assert_eq!(formatter.max_width, Some(120));
    }

    #[test]
    fn test_format_tasks() {
        let task = Task {
            id: Uuid::new_v4(),
            summary: "Test task".to_string(),
            description: "Test description".to_string(),
            agent_type: "test-agent".to_string(),
            priority: 5,
            calculated_priority: 5.0,
            status: TaskStatus::Pending,
            dependencies: None,
            dependency_type: crate::domain::models::task::DependencyType::Sequential,
            dependency_depth: 0,
            input_data: None,
            result_data: None,
            error_message: None,
            retry_count: 0,
            max_retries: 3,
            max_execution_timeout_seconds: 3600,
            submitted_at: Utc::now(),
            started_at: None,
            completed_at: None,
            last_updated_at: Utc::now(),
            created_by: None,
            parent_task_id: None,
            session_id: None,
            source: crate::domain::models::task::TaskSource::Human,
            deadline: None,
            estimated_duration_seconds: None,
            feature_branch: None,
            task_branch: Some("test-branch".to_string()),
            worktree_path: None,
        };

        let formatter = TableFormatter::with_config(false, None);
        let output = formatter.format_tasks(&[task]);

        assert!(output.contains("Test task"));
        assert!(output.contains("test-agent"));
        assert!(output.contains("test-branch"));
    }

    #[test]
    fn test_format_agents() {
        let agent = Agent::new(Uuid::new_v4(), "test-agent".to_string());

        let formatter = TableFormatter::with_config(false, None);
        let output = formatter.format_agents(&[agent]);

        assert!(output.contains("test-agent"));
        assert!(output.contains("idle"));
    }

    #[test]
    fn test_format_mcp_servers() {
        let server = McpServerConfig {
            name: "test-server".to_string(),
            command: "test-command".to_string(),
            args: vec!["arg1".to_string(), "arg2".to_string()],
            env: std::collections::HashMap::new(),
        };

        let formatter = TableFormatter::with_config(false, None);
        let output = formatter.format_mcp_servers(&[server]);

        assert!(output.contains("test-server"));
        assert!(output.contains("test-command"));
        assert!(output.contains("arg1 arg2"));
    }

    #[test]
    fn test_status_icon_mapping() {
        assert_eq!(status_icon(&TaskStatus::Completed), "✓");
        assert_eq!(status_icon(&TaskStatus::Failed), "✗");
        assert_eq!(status_icon(&TaskStatus::Running), "⟳");
        assert_eq!(status_icon(&TaskStatus::Pending), "○");
    }

    #[test]
    fn test_status_color_mapping() {
        assert_eq!(status_color(&TaskStatus::Completed), Color::Green);
        assert_eq!(status_color(&TaskStatus::Failed), Color::Red);
        assert_eq!(status_color(&TaskStatus::Running), Color::Cyan);
    }

    #[test]
    fn test_priority_color_mapping() {
        assert_eq!(priority_color(10), Color::Red);
        assert_eq!(priority_color(5), Color::Yellow);
        assert_eq!(priority_color(1), Color::Blue);
    }

    #[test]
    fn test_agent_status_icon_mapping() {
        assert_eq!(agent_status_icon(&AgentStatus::Idle), "○");
        assert_eq!(agent_status_icon(&AgentStatus::Busy), "●");
        assert_eq!(agent_status_icon(&AgentStatus::Terminated), "✗");
    }

    #[test]
    fn test_agent_status_color_mapping() {
        assert_eq!(agent_status_color(&AgentStatus::Idle), Color::Green);
        assert_eq!(agent_status_color(&AgentStatus::Busy), Color::Cyan);
        assert_eq!(agent_status_color(&AgentStatus::Terminated), Color::DarkGrey);
    }

    #[test]
    fn test_truncate_text() {
        assert_eq!(truncate_text("short", 10), "short");
        assert_eq!(truncate_text("this is a very long text", 10), "this is...");
        assert_eq!(truncate_text("exactly10!", 10), "exactly10!");
    }

    #[test]
    fn test_truncate_text_edge_cases() {
        assert_eq!(truncate_text("", 10), "");
        assert_eq!(truncate_text("abc", 3), "abc");
        assert_eq!(truncate_text("abcd", 3), "...");
=======

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
>>>>>>> task_cli-structure_20251025-210033
    }
}
