"""Main screen for Abathur Task Queue TUI.

This screen provides the primary interface for viewing task queues,
displaying a tree view of tasks on the left and detailed information
on the right.
"""

from textual.app import ComposeResult
from textual.containers import Horizontal, Vertical
from textual.screen import Screen
from textual.widgets import Footer, Header, Static


class MainScreen(Screen):
    """Primary screen with task tree, detail panel, stats header, and footer.

    Layout:
    - Header: Queue statistics (Phase 3)
    - Body: Horizontal split
        - Left (60%): Task tree view (Phase 2)
        - Right (40%): Task detail panel (Phase 3)
    - Footer: Keybinding help text
    """

    CSS = """
    MainScreen {
        layout: vertical;
    }

    #stats-header {
        height: 3;
        background: $panel;
        border: solid $primary;
        content-align: center middle;
        text-style: bold;
    }

    #content {
        layout: horizontal;
        height: 1fr;
    }

    #tree-panel {
        width: 60%;
        border-right: solid $primary;
        padding: 1 2;
        content-align: center middle;
    }

    #detail-panel {
        width: 40%;
        padding: 1 2;
        content-align: center middle;
    }
    """

    def compose(self) -> ComposeResult:
        """Create child widgets for the main screen.

        Yields:
            Widgets that compose the main screen layout.
        """
        # Header
        yield Header()

        # Stats header (placeholder for Phase 3)
        yield Static(
            "Queue Stats (Coming in Phase 3)",
            id="stats-header",
        )

        # Main content area with horizontal split
        with Vertical(id="content"):
            with Horizontal():
                # Left panel: Task tree (placeholder for Phase 2)
                yield Static(
                    "Task Tree\n(Coming in Phase 2)",
                    id="tree-panel",
                )

                # Right panel: Task details (placeholder for Phase 3)
                yield Static(
                    "Task Details\n(Coming in Phase 3)",
                    id="detail-panel",
                )

        # Footer with keybinding hints
        yield Footer()

    def on_mount(self) -> None:
        """Called when screen is mounted - setup initial state."""
        # Set footer help text
        self.app.title = "Abathur Task Queue Visualizer"
        self.app.sub_title = "Phase 1 - Foundation"

    async def refresh_data(self) -> None:
        """Refresh data from services.

        This will be implemented in Phase 2 when TaskDataService is available.
        For Phase 1, this is a placeholder.
        """
        # TODO: Phase 2 - Implement data refresh from TaskDataService
        pass

    def apply_filter(self, filter_state) -> None:
        """Apply filter to displayed data.

        Args:
            filter_state: Filter criteria to apply

        This will be implemented in Phase 2 when filtering is active.
        For Phase 1, this is a placeholder.
        """
        # TODO: Phase 2 - Implement filter application
        pass
