"""Main Textual TUI application for Abathur Task Queue.

This module contains the TaskQueueTUI app class which manages the application
lifecycle, screens, and global state.
"""

from textual.app import App
from textual.binding import Binding
from textual.reactive import var

from .screens.main_screen import MainScreen


class TaskQueueTUI(App):
    """Main Textual TUI application for task queue visualization.

    This app manages the application lifecycle, global keybindings,
    and screen navigation.
    """

    # Global keybindings
    BINDINGS = [
        Binding("q", "quit", "Quit", priority=True),
        Binding("r", "refresh", "Refresh"),
        Binding("f", "filter", "Filter"),
        Binding("v", "cycle_view", "View Mode"),
        Binding("?", "help", "Help"),
    ]

    # Reactive state (automatically triggers UI updates)
    selected_task_id: var[str | None] = var(None)
    current_view_mode: var[str] = var("tree")
    auto_refresh_enabled: var[bool] = var(True)

    def __init__(
        self,
        refresh_interval: float | None = 2.0,
        initial_view_mode: str = "tree",
        **kwargs,
    ):
        """Initialize TUI with configuration.

        Args:
            refresh_interval: Auto-refresh interval in seconds (None to disable)
            initial_view_mode: Initial view mode (tree, dependency, timeline, etc.)
            **kwargs: Additional arguments for App base class
        """
        super().__init__(**kwargs)
        self.refresh_interval = refresh_interval
        self.current_view_mode = initial_view_mode
        self._refresh_timer = None

    def on_mount(self) -> None:
        """Called when app starts - setup initial state."""
        # Install main screen
        self.push_screen(MainScreen())

        # TODO: Phase 1 - Start auto-refresh when TaskDataService is available
        # if self.refresh_interval:
        #     self.start_auto_refresh()

    def start_auto_refresh(self) -> None:
        """Start periodic refresh timer.

        This will be implemented in Phase 2 when TaskDataService is available.
        """
        if self.refresh_interval:
            self._refresh_timer = self.set_interval(
                self.refresh_interval,
                self.action_refresh,
            )

    def stop_auto_refresh(self) -> None:
        """Stop periodic refresh timer."""
        if self._refresh_timer:
            self._refresh_timer.stop()
            self._refresh_timer = None

    # Action methods (invoked by keybindings)
    def action_refresh(self) -> None:
        """Refresh data from services.

        This will be implemented in Phase 2 when TaskDataService is available.
        """
        # TODO: Phase 2 - Implement refresh
        pass

    def action_filter(self) -> None:
        """Open filter modal screen.

        This will be implemented in Phase 2 when FilterScreen is available.
        """
        # TODO: Phase 2 - Implement filter screen
        pass

    def action_cycle_view(self) -> None:
        """Cycle through view modes.

        Cycles: tree -> dependency -> timeline -> feature_branch -> flat_list
        """
        modes = ["tree", "dependency", "timeline", "feature_branch", "flat_list"]
        try:
            current_idx = modes.index(self.current_view_mode)
        except ValueError:
            current_idx = 0
        next_idx = (current_idx + 1) % len(modes)
        self.current_view_mode = modes[next_idx]

    def action_help(self) -> None:
        """Show help screen.

        This will be implemented in Phase 3.
        """
        # TODO: Phase 3 - Implement help screen
        pass
