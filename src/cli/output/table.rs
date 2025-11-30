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
            Cell::new("Chain").add_attribute(Attribute::Bold),
            Cell::new("Agent").add_attribute(Attribute::Bold),
            Cell::new("Branch").add_attribute(Attribute::Bold),
        ]);

        // Data rows
        for task in tasks {
            let id_short = &task.id.to_string()[..8];
            let summary = truncate_text(&task.summary, 40);

            let status_cell = if self.use_colors {
                Cell::new(task.status.to_string())
                    .fg(status_color(&task.status))
            } else {
                Cell::new(format!("{} {}", status_icon(&task.status), task.status))
            };

            // Format chain_id - show first 8 chars if present, otherwise show "-"
            let chain_display = task.chain_id
                .as_ref()
                .map(|id| {
                    if id.len() > 8 {
                        format!("{}...", &id[..8])
                    } else {
                        id.clone()
                    }
                })
                .unwrap_or_else(|| "-".to_string());

            let chain_cell = if self.use_colors && task.chain_id.is_some() {
                Cell::new(&chain_display).fg(Color::Cyan)
            } else {
                Cell::new(&chain_display)
            };

            // Display task branch if set, otherwise fall back to feature branch
            let branch = task.branch.as_deref()
                .or(task.feature_branch.as_deref())
                .unwrap_or("-");

            table.add_row(vec![
                Cell::new(id_short),
                Cell::new(&summary),
                status_cell,
                chain_cell,
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
                Cell::new(agent.status.to_string())
                    .fg(agent_status_color(&agent.status))
            } else {
                Cell::new(format!("{} {}", agent_status_icon(&agent.status), agent.status))
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
        TaskStatus::AwaitingValidation => Color::Yellow,
        TaskStatus::ValidationRunning => Color::Cyan,
        TaskStatus::ValidationFailed => Color::Red,
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
        TaskStatus::AwaitingValidation => "⧗",
        TaskStatus::ValidationRunning => "⟳",
        TaskStatus::ValidationFailed => "✗",
        TaskStatus::Failed => "✗",
        TaskStatus::Cancelled => "⊘",
        TaskStatus::Ready => "●",
        TaskStatus::Blocked => "⊗",
        TaskStatus::Pending => "○",
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
    }
}

/// Format a task list as a table (convenience function for backward compatibility)
pub fn format_task_table(tasks: &[crate::cli::models::Task]) -> String {
    // Convert CLI tasks to domain tasks for formatting
    let formatter = TableFormatter::new();
    let domain_tasks: Vec<Task> = tasks.iter().map(|t| Task {
        id: t.id,
        summary: t.summary.clone(),
        description: t.description.clone(),
        agent_type: t.agent_type.clone(),
        priority: t.base_priority,
        calculated_priority: t.computed_priority,
        status: match t.status {
            crate::cli::models::TaskStatus::Pending => TaskStatus::Pending,
            crate::cli::models::TaskStatus::Blocked => TaskStatus::Blocked,
            crate::cli::models::TaskStatus::Ready => TaskStatus::Ready,
            crate::cli::models::TaskStatus::Running => TaskStatus::Running,
            crate::cli::models::TaskStatus::Completed => TaskStatus::Completed,
            crate::cli::models::TaskStatus::Failed => TaskStatus::Failed,
            crate::cli::models::TaskStatus::Cancelled => TaskStatus::Cancelled,
        },
        dependencies: None,
        dependency_type: crate::domain::models::task::DependencyType::Sequential,
        dependency_depth: 0,
        input_data: None,
        result_data: None,
        error_message: None,
        retry_count: 0,
        max_retries: 3,
        max_execution_timeout_seconds: 3600,
        submitted_at: t.created_at,
        started_at: t.started_at,
        completed_at: t.completed_at,
        last_updated_at: t.updated_at,
        created_by: None,
        parent_task_id: None,
        session_id: None,
        source: crate::domain::models::task::TaskSource::Human,
        deadline: None,
        estimated_duration_seconds: None,
        branch: t.branch.clone(),
        feature_branch: t.feature_branch.clone(),
        worktree_path: None,
        validation_requirement: crate::domain::models::task::ValidationRequirement::None,
        validation_task_id: None,
        validating_task_id: None,
        remediation_count: 0,
        is_remediation: false,
        workflow_state: None,
        workflow_expectations: None,
        chain_id: t.chain_id.clone(),
        chain_step_index: 0,
        idempotency_key: None,
        version: 1,
    }).collect();

    formatter.format_tasks(&domain_tasks)
}

/// Format queue stats as a table (convenience function for backward compatibility)
pub fn format_queue_stats_table(stats: &crate::cli::models::QueueStats) -> String {
    let mut table = Table::new();
    table.load_preset(presets::UTF8_FULL);

    table.set_header(vec![
        Cell::new("Metric").add_attribute(Attribute::Bold),
        Cell::new("Count").add_attribute(Attribute::Bold),
    ]);

    table.add_row(vec!["Total", &stats.total.to_string()]);
    table.add_row(vec!["Pending", &stats.pending.to_string()]);
    table.add_row(vec!["Blocked", &stats.blocked.to_string()]);
    table.add_row(vec!["Ready", &stats.ready.to_string()]);
    table.add_row(vec!["Running", &stats.running.to_string()]);
    table.add_row(vec!["Completed", &stats.completed.to_string()]);
    table.add_row(vec!["Failed", &stats.failed.to_string()]);
    table.add_row(vec!["Cancelled", &stats.cancelled.to_string()]);

    table.to_string()
}

/// Format a memory list as a table (convenience function for CLI)
pub fn format_memory_table(memories: &[crate::cli::models::Memory]) -> String {
    let mut table = Table::new();
    table.load_preset(presets::UTF8_FULL);
    table.set_content_arrangement(ContentArrangement::Dynamic);

    table.set_header(vec![
        Cell::new("Namespace").add_attribute(Attribute::Bold),
        Cell::new("Key").add_attribute(Attribute::Bold),
        Cell::new("Type").add_attribute(Attribute::Bold),
        Cell::new("Created By").add_attribute(Attribute::Bold),
        Cell::new("Updated").add_attribute(Attribute::Bold),
    ]);

    for memory in memories {
        let updated_str = format_relative_time(&memory.updated_at);

        table.add_row(vec![
            Cell::new(truncate_text(&memory.namespace, 30)),
            Cell::new(truncate_text(&memory.key, 20)),
            Cell::new(format!("{}", memory.memory_type)),
            Cell::new(truncate_text(&memory.created_by, 15)),
            Cell::new(updated_str),
        ]);
    }

    table.to_string()
}

/// Format relative time (e.g., "2 hours ago")
fn format_relative_time(datetime: &chrono::DateTime<chrono::Utc>) -> String {
    let now = chrono::Utc::now();
    let duration = now.signed_duration_since(*datetime);

    if duration.num_seconds() < 60 {
        "just now".to_string()
    } else if duration.num_minutes() < 60 {
        let mins = duration.num_minutes();
        format!("{} min{} ago", mins, if mins == 1 { "" } else { "s" })
    } else if duration.num_hours() < 24 {
        let hours = duration.num_hours();
        format!("{} hour{} ago", hours, if hours == 1 { "" } else { "s" })
    } else if duration.num_days() < 30 {
        let days = duration.num_days();
        format!("{} day{} ago", days, if days == 1 { "" } else { "s" })
    } else {
        datetime.format("%Y-%m-%d").to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
            branch: Some("test-branch".to_string()),
            worktree_path: None,
            validation_requirement: crate::domain::models::task::ValidationRequirement::None,
            validation_task_id: None,
            validating_task_id: None,
            remediation_count: 0,
            is_remediation: false,
            workflow_state: None,
            workflow_expectations: None,
            chain_id: None,
            chain_step_index: 0,
            idempotency_key: None,
            version: 1,
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
    }
}
