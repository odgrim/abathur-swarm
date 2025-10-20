"""Queue statistics header widget for TUI.

Displays real-time task queue metrics:
- Task counts by status (Total, Pending, Blocked, Ready, Running, Completed, Failed, Cancelled)
- Queue health metrics (Average priority, Max dependency depth)
- Refresh info (Last update timestamp, Auto-refresh indicator)
"""

from datetime import datetime
from typing import Any

from rich.table import Table
from rich.text import Text
from textual.reactive import reactive
from textual.widgets import Static

from abathur.domain.models import TaskStatus


class QueueStatsHeader(Static):
    """Header displaying real-time queue statistics.

    Reactive Properties:
        stats: Queue statistics data from TaskDataService.get_queue_status()
        auto_refresh_enabled: Whether auto-refresh is active (shows ⟳ indicator)
        last_refresh: Timestamp of last data update

    Example:
        ```python
        header = QueueStatsHeader()
        header.stats = await task_service.get_queue_status()
        header.auto_refresh_enabled = True
        ```
    """

    # CSS styling
    DEFAULT_CSS = """
    QueueStatsHeader {
        height: 3;
        border: solid $primary;
        padding: 0 1;
        background: $surface;
    }

    QueueStatsHeader:focus {
        border: solid $accent;
    }
    """

    # Reactive properties
    stats: reactive[dict[str, Any] | None] = reactive(None)
    auto_refresh_enabled: reactive[bool] = reactive(False)
    last_refresh: reactive[datetime | None] = reactive(None)

    # Status color mapping (matching TaskStatus enum values)
    STATUS_COLORS = {
        TaskStatus.PENDING: "blue",
        TaskStatus.BLOCKED: "yellow",
        TaskStatus.READY: "cyan",
        TaskStatus.RUNNING: "magenta",
        TaskStatus.COMPLETED: "green",
        TaskStatus.FAILED: "red",
        TaskStatus.CANCELLED: "dim",
    }

    def __init__(self, **kwargs: Any) -> None:
        """Initialize QueueStatsHeader widget."""
        super().__init__(**kwargs)

    def watch_stats(
        self, old_stats: dict[str, Any] | None, new_stats: dict[str, Any] | None
    ) -> None:
        """Update display when stats change.

        Args:
            old_stats: Previous statistics data
            new_stats: New statistics data to render
        """
        if new_stats is not None:
            self.update(self._render_stats(new_stats))

    def watch_auto_refresh_enabled(self, old_value: bool, new_value: bool) -> None:
        """Update display when auto-refresh state changes.

        Args:
            old_value: Previous auto-refresh state
            new_value: New auto-refresh state
        """
        # Trigger re-render to show/hide refresh indicator
        if self.stats is not None:
            self.update(self._render_stats(self.stats))

    def watch_last_refresh(
        self, old_time: datetime | None, new_time: datetime | None
    ) -> None:
        """Update display when last refresh timestamp changes.

        Args:
            old_time: Previous refresh timestamp
            new_time: New refresh timestamp
        """
        # Trigger re-render to update timestamp
        if self.stats is not None:
            self.update(self._render_stats(self.stats))

    def _render_stats(self, stats: dict[str, Any]) -> Table:
        """Render queue statistics as Rich table.

        Args:
            stats: Dictionary containing queue metrics with keys:
                - total_tasks: int
                - pending_count: int
                - blocked_count: int
                - ready_count: int
                - running_count: int
                - completed_count: int
                - failed_count: int
                - cancelled_count: int (optional)
                - avg_priority: float
                - max_dependency_depth: int (optional)

        Returns:
            Rich Table renderable with formatted statistics
        """
        table = Table.grid(padding=(0, 2))

        # Row 1: Status counts
        row_parts = []

        # Total (bold white)
        total = stats.get("total_tasks", 0)
        row_parts.append(Text(f"Total: {total}", style="bold white"))

        # Status counts with color coding
        status_metrics = [
            ("Pending", TaskStatus.PENDING, stats.get("pending_count", 0)),
            ("Blocked", TaskStatus.BLOCKED, stats.get("blocked_count", 0)),
            ("Ready", TaskStatus.READY, stats.get("ready_count", 0)),
            ("Running", TaskStatus.RUNNING, stats.get("running_count", 0)),
            ("Completed", TaskStatus.COMPLETED, stats.get("completed_count", 0)),
            ("Failed", TaskStatus.FAILED, stats.get("failed_count", 0)),
            ("Cancelled", TaskStatus.CANCELLED, stats.get("cancelled_count", 0)),
        ]

        for label, status, count in status_metrics:
            color = self.STATUS_COLORS.get(status, "white")
            row_parts.append(Text(f"{label}: {count}", style=color))

        # Row 2: Health metrics
        health_parts = []

        # Average priority
        avg_priority = stats.get("avg_priority", 0.0)
        health_parts.append(Text(f"Avg Priority: {avg_priority:.1f}", style="cyan"))

        # Max dependency depth (if available)
        max_depth = stats.get("max_dependency_depth")
        if max_depth is not None:
            health_parts.append(
                Text(f"Max Depth: {max_depth}", style="yellow")
            )

        # Refresh info
        refresh_parts = []

        # Auto-refresh indicator
        if self.auto_refresh_enabled:
            refresh_parts.append(Text("⟳", style="green bold"))
        else:
            refresh_parts.append(Text("⏸", style="dim"))

        # Last refresh timestamp
        if self.last_refresh:
            time_str = self.last_refresh.strftime("%H:%M:%S")
            refresh_parts.append(Text(f"Updated: {time_str}", style="dim"))

        # Assemble table rows
        # First row: status counts
        status_row = Text(" | ").join(row_parts)
        table.add_row(status_row)

        # Second row: health metrics + refresh info
        health_row = Text(" | ").join(health_parts + refresh_parts)
        table.add_row(health_row)

        return table

    def render(self) -> Text | Table:
        """Render widget content.

        Returns:
            Rich renderable (Text or Table) for display
        """
        if self.stats is None:
            return Text("Loading queue statistics...", style="dim italic")

        return self._render_stats(self.stats)
