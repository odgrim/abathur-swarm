"""Task detail panel widget displaying comprehensive task metadata.

This module provides the TaskDetailPanel, a Textual Static widget that displays
all 28 fields of a Task model with:
- Reactive updates when task selection changes
- Rich text formatting with color coding
- JSON syntax highlighting for complex fields
- Organized sections for readability
- Graceful handling of None/missing values
"""

import json
from uuid import UUID
from datetime import datetime

from textual.reactive import reactive
from textual.widgets import Static
from rich.text import Text
from rich.table import Table
from rich.panel import Panel
from rich.syntax import Syntax

from abathur.domain.models import Task, TaskStatus, TaskSource, DependencyType


class TaskDetailPanel(Static):
    """Panel displaying detailed task metadata for selected task.

    This widget watches for task selection changes and renders all 28 fields
    of the Task model organized into logical sections. It provides rich formatting,
    color-coded status indicators, and JSON syntax highlighting.

    Reactive Properties:
        selected_task_id: UUID of the task to display (None for no selection)
        task_data: Full Task object for the selected task (None if not loaded)

    Sections Displayed:
        1. Identification: task_id, summary
        2. Status: status (color-coded), agent_type, source
        3. Priority: base_priority, calculated_priority, dependency_depth
        4. Branches: feature_branch, task_branch, worktree_path
        5. Timestamps: submitted_at, started_at, completed_at, last_updated_at
        6. Dependencies: prerequisites count, dependents count, dependency_type
        7. Execution: retry_count, max_retries, max_execution_timeout_seconds
        8. Results: result_data (JSON), error_message

    Example:
        >>> detail_panel = TaskDetailPanel()
        >>> # In parent screen/app with TaskTreeWidget:
        >>> @on(TaskTreeWidget.TaskSelected)
        >>> def handle_task_selected(self, message: TaskTreeWidget.TaskSelected):
        >>>     detail_panel.selected_task_id = message.task_id
    """

    # Reactive properties
    selected_task_id: reactive[UUID | None] = reactive(None)
    task_data: reactive[Task | None] = reactive(None)

    # Status color mapping (matches TaskTreeWidget colors)
    STATUS_COLORS = {
        TaskStatus.PENDING: "blue",
        TaskStatus.BLOCKED: "yellow",
        TaskStatus.READY: "green",
        TaskStatus.RUNNING: "magenta",
        TaskStatus.COMPLETED: "bright_green",
        TaskStatus.FAILED: "red",
        TaskStatus.CANCELLED: "dim white",
    }

    # CSS styling
    DEFAULT_CSS = """
    TaskDetailPanel {
        height: 100%;
        border: solid $primary;
        padding: 1;
        overflow-y: auto;
    }

    TaskDetailPanel:focus {
        border: solid $accent;
    }

    TaskDetailPanel .section-title {
        color: $accent;
        text-style: bold;
    }
    """

    def watch_selected_task_id(
        self, old_id: UUID | None, new_id: UUID | None
    ) -> None:
        """Called automatically when selected_task_id changes.

        This watch method fetches the full task data and triggers a re-render.
        If new_id is None, displays "No task selected" message.

        Args:
            old_id: Previous task ID (unused)
            new_id: New task ID to display (or None)
        """
        if new_id is None:
            self.task_data = None
            self.update(self._render_empty_state())
        else:
            # TODO: Fetch task from service layer when integrated
            # For now, we'll need task_data to be set externally
            # In real implementation: self.run_worker(self._load_task(new_id))
            pass

    def watch_task_data(self, old_task: Task | None, new_task: Task | None) -> None:
        """Called automatically when task_data changes.

        Renders the task details when data is available.

        Args:
            old_task: Previous task data (unused)
            new_task: New task data to render (or None)
        """
        if new_task is None:
            self.update(self._render_empty_state())
        else:
            self.update(self._render_task(new_task))

    def render(self) -> Panel:
        """Render the panel with current task data.

        Returns:
            Rich Panel containing task details or empty state message
        """
        if self.task_data is None:
            return self._render_empty_state()
        return self._render_task(self.task_data)

    def _render_empty_state(self) -> Panel:
        """Render empty state when no task is selected.

        Returns:
            Panel with "No task selected" message
        """
        message = Text("No task selected", style="dim italic")
        return Panel(
            message,
            title="Task Details",
            border_style="blue",
            padding=(1, 2),
        )

    def _render_task(self, task: Task) -> Panel:
        """Render complete task details with all 28 fields.

        Organizes fields into logical sections with proper formatting,
        color coding, and syntax highlighting.

        Args:
            task: Task object to render

        Returns:
            Rich Panel containing formatted task details
        """
        # Create main table with grid layout (no borders between rows)
        table = Table.grid(padding=(0, 2), expand=True)
        table.add_column(justify="right", style="cyan", no_wrap=True)
        table.add_column(style="white")

        # Section 1: Identification
        self._add_section_header(table, "Identification")
        table.add_row("ID:", self._format_uuid(task.id))
        table.add_row("Summary:", self._format_summary(task.summary, task.prompt))

        # Section 2: Status
        self._add_section_header(table, "Status")
        table.add_row("Status:", self._format_status(task.status))
        table.add_row("Agent Type:", task.agent_type)
        table.add_row("Source:", self._format_source(task.source))

        # Section 3: Priority
        self._add_section_header(table, "Priority")
        table.add_row("Base Priority:", str(task.priority))
        table.add_row("Calculated Priority:", f"{task.calculated_priority:.2f}")
        table.add_row("Dependency Depth:", str(task.dependency_depth))

        # Section 4: Branches
        self._add_section_header(table, "Branches")
        table.add_row("Feature Branch:", self._format_optional(task.feature_branch))
        table.add_row("Task Branch:", self._format_optional(task.task_branch))
        table.add_row("Worktree Path:", self._format_optional(task.worktree_path))

        # Section 5: Timestamps
        self._add_section_header(table, "Timestamps")
        table.add_row("Submitted At:", self._format_datetime(task.submitted_at))
        table.add_row("Started At:", self._format_datetime(task.started_at))
        table.add_row("Completed At:", self._format_datetime(task.completed_at))
        table.add_row("Last Updated:", self._format_datetime(task.last_updated_at))

        # Section 6: Dependencies
        self._add_section_header(table, "Dependencies")
        table.add_row("Prerequisites:", str(len(task.dependencies)))
        table.add_row("Parent Task ID:", self._format_optional_uuid(task.parent_task_id))
        table.add_row("Dependency Type:", self._format_dependency_type(task.dependency_type))

        # Section 7: Execution
        self._add_section_header(table, "Execution")
        table.add_row("Retry Count:", f"{task.retry_count} / {task.max_retries}")
        table.add_row(
            "Timeout:",
            self._format_timeout(task.max_execution_timeout_seconds),
        )
        table.add_row("Deadline:", self._format_datetime(task.deadline))
        table.add_row(
            "Est. Duration:",
            self._format_duration(task.estimated_duration_seconds),
        )

        # Section 8: Context
        self._add_section_header(table, "Context")
        table.add_row("Session ID:", self._format_optional(task.session_id))
        table.add_row("Created By:", self._format_optional(task.created_by))
        table.add_row("Input Data:", self._format_dict_size(task.input_data))

        # Section 9: Results (only show if completed or failed)
        if task.status in (TaskStatus.COMPLETED, TaskStatus.FAILED):
            self._add_section_header(table, "Results")

            if task.result_data:
                table.add_row("Result Data:", "")
                # Add JSON syntax highlighted result on next row
                json_syntax = self._format_json(task.result_data)
                # Create a nested table for indented JSON
                json_table = Table.grid(padding=(0, 4))
                json_table.add_column()
                json_table.add_row(json_syntax)
                table.add_row("", json_table)

            if task.error_message:
                table.add_row("Error:", self._format_error(task.error_message))

        # Wrap in panel with colored border based on status
        border_style = self.STATUS_COLORS.get(task.status, "white")
        title = f"Task Details - {task.status.value.upper()}"

        return Panel(
            table,
            title=title,
            border_style=border_style,
            padding=(1, 2),
        )

    def _add_section_header(self, table: Table, title: str) -> None:
        """Add a section header to the table.

        Args:
            table: Table to add header to
            title: Section title text
        """
        # Add empty row for spacing (except at start)
        if len(table.rows) > 0:
            table.add_row("", "")
        # Add section title row
        header = Text(f"─── {title} ", style="bold cyan")
        header.append("─" * (40 - len(title)), style="dim cyan")
        table.add_row("", header)

    def _format_uuid(self, uuid: UUID) -> Text:
        """Format UUID with monospace font.

        Args:
            uuid: UUID to format

        Returns:
            Formatted Text object
        """
        return Text(str(uuid), style="yellow dim")

    def _format_optional_uuid(self, uuid: UUID | None) -> Text:
        """Format optional UUID field.

        Args:
            uuid: UUID to format or None

        Returns:
            Formatted Text object
        """
        if uuid is None:
            return Text("—", style="dim")
        return self._format_uuid(uuid)

    def _format_summary(self, summary: str | None, prompt: str) -> Text:
        """Format summary field, showing prompt preview if summary is None.

        Args:
            summary: Task summary (may be None)
            prompt: Task prompt (fallback)

        Returns:
            Formatted Text object
        """
        if summary:
            return Text(summary, style="white")
        # Show first 60 chars of prompt as fallback
        preview = prompt[:60] + "..." if len(prompt) > 60 else prompt
        return Text(preview, style="dim italic")

    def _format_status(self, status: TaskStatus) -> Text:
        """Format status with color coding.

        Args:
            status: TaskStatus enum value

        Returns:
            Colored Text object
        """
        color = self.STATUS_COLORS.get(status, "white")
        icon = self._get_status_icon(status)
        return Text(f"{icon} {status.value.upper()}", style=f"bold {color}")

    def _get_status_icon(self, status: TaskStatus) -> str:
        """Get icon for task status.

        Args:
            status: TaskStatus enum value

        Returns:
            Unicode icon character
        """
        icons = {
            TaskStatus.PENDING: "○",
            TaskStatus.BLOCKED: "◐",
            TaskStatus.READY: "◉",
            TaskStatus.RUNNING: "◎",
            TaskStatus.COMPLETED: "✓",
            TaskStatus.FAILED: "✗",
            TaskStatus.CANCELLED: "⊗",
        }
        return icons.get(status, "•")

    def _format_source(self, source: TaskSource) -> Text:
        """Format task source with appropriate styling.

        Args:
            source: TaskSource enum value

        Returns:
            Formatted Text object
        """
        colors = {
            TaskSource.HUMAN: "green",
            TaskSource.AGENT_REQUIREMENTS: "cyan",
            TaskSource.AGENT_PLANNER: "magenta",
            TaskSource.AGENT_IMPLEMENTATION: "yellow",
        }
        color = colors.get(source, "white")
        return Text(source.value, style=color)

    def _format_dependency_type(self, dep_type: DependencyType) -> Text:
        """Format dependency type.

        Args:
            dep_type: DependencyType enum value

        Returns:
            Formatted Text object
        """
        return Text(dep_type.value, style="cyan")

    def _format_optional(self, value: str | None) -> Text:
        """Format optional string field.

        Args:
            value: String value or None

        Returns:
            Formatted Text object
        """
        if value is None:
            return Text("—", style="dim")
        return Text(value, style="white")

    def _format_datetime(self, dt: datetime | None) -> Text:
        """Format datetime in ISO format with relative time.

        Args:
            dt: Datetime object or None

        Returns:
            Formatted Text object
        """
        if dt is None:
            return Text("—", style="dim")

        # Format as ISO string
        iso_str = dt.strftime("%Y-%m-%d %H:%M:%S UTC")

        # Calculate relative time (basic implementation)
        # TODO: Use a proper relative time library for production
        relative = self._get_relative_time(dt)

        return Text(f"{iso_str} ({relative})", style="white")

    def _get_relative_time(self, dt: datetime) -> str:
        """Get relative time string (e.g., "2 hours ago").

        Args:
            dt: Datetime object

        Returns:
            Relative time string
        """
        from datetime import timezone

        now = datetime.now(timezone.utc)
        delta = now - dt

        seconds = int(delta.total_seconds())
        if seconds < 60:
            return f"{seconds}s ago"
        minutes = seconds // 60
        if minutes < 60:
            return f"{minutes}m ago"
        hours = minutes // 60
        if hours < 24:
            return f"{hours}h ago"
        days = hours // 24
        return f"{days}d ago"

    def _format_timeout(self, seconds: int) -> Text:
        """Format timeout in human-readable format.

        Args:
            seconds: Timeout in seconds

        Returns:
            Formatted Text object
        """
        if seconds < 60:
            return Text(f"{seconds}s", style="white")
        minutes = seconds // 60
        if minutes < 60:
            return Text(f"{minutes}m", style="white")
        hours = minutes // 60
        return Text(f"{hours}h", style="white")

    def _format_duration(self, seconds: int | None) -> Text:
        """Format estimated duration.

        Args:
            seconds: Duration in seconds or None

        Returns:
            Formatted Text object
        """
        if seconds is None:
            return Text("—", style="dim")
        return self._format_timeout(seconds)

    def _format_dict_size(self, data: dict) -> Text:
        """Format dictionary size indicator.

        Args:
            data: Dictionary to measure

        Returns:
            Formatted Text object
        """
        if not data:
            return Text("empty", style="dim")
        count = len(data)
        return Text(f"{count} {'key' if count == 1 else 'keys'}", style="cyan")

    def _format_json(self, data: dict) -> Syntax:
        """Format JSON with syntax highlighting.

        Args:
            data: Dictionary to format as JSON

        Returns:
            Rich Syntax object with highlighted JSON
        """
        json_str = json.dumps(data, indent=2, default=str)
        return Syntax(
            json_str,
            "json",
            theme="monokai",
            line_numbers=False,
            word_wrap=True,
        )

    def _format_error(self, error: str) -> Text:
        """Format error message with red styling.

        Args:
            error: Error message string

        Returns:
            Formatted Text object
        """
        # Truncate long error messages
        if len(error) > 200:
            error = error[:200] + "..."
        return Text(error, style="bold red")
