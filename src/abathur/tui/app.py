"""Main TaskQueueTUI Textual Application.

This module implements the main TUI app class for Abathur task queue visualization.
"""

from uuid import UUID
from typing import Any

from textual.app import App
from textual.binding import Binding
from textual.reactive import var
from textual.timer import Timer

from .models import ViewMode
from .screens.main_screen import MainScreen
from .screens.filter_screen import FilterScreen
from .services.task_data_service import TaskDataService


class TaskQueueTUI(App[None]):
    """Main Textual TUI application for task queue visualization.

    This app provides an interactive terminal interface for viewing and
    managing the Abathur task queue with features including:
    - Hierarchical task tree visualization with multiple view modes
    - Real-time auto-refresh of task data
    - Task filtering by status, agent, branch, and text
    - Detailed task metadata display
    - Queue statistics and metrics

    Attributes:
        selected_task_id: Currently selected task UUID (reactive)
        current_view_mode: Active view mode for task visualization (reactive)
        auto_refresh_enabled: Whether auto-refresh is active (reactive)
        refresh_interval: Auto-refresh interval in seconds (reactive)
    """

    # Global CSS styling (can be moved to .tcss file in later phases)
    CSS = """
    Screen {
        background: $surface;
    }

    #content {
        height: 1fr;
        background: $panel;
    }
    """

    # Global keybindings
    BINDINGS = [
        Binding("q", "quit", "Quit", priority=True),
        Binding("r", "refresh_now", "Refresh"),
        Binding("f", "show_filter_screen", "Filter"),
        Binding("v", "cycle_view_mode", "View Mode"),
        Binding("?", "show_help", "Help"),
    ]

    # Reactive state properties
    selected_task_id: var[UUID | None] = var(None)
    current_view_mode: var[ViewMode] = var(ViewMode.TREE)
    auto_refresh_enabled: var[bool] = var(True)
    refresh_interval: var[float] = var(2.0)

    def __init__(
        self,
        task_data_service: TaskDataService,
        initial_view_mode: ViewMode = ViewMode.TREE,
        auto_refresh: bool = True,
        refresh_interval: float = 2.0,
        **kwargs: Any,
    ) -> None:
        """Initialize TaskQueueTUI application.

        Args:
            task_data_service: Service for task data fetching and caching
            initial_view_mode: Initial view mode (default: TREE)
            auto_refresh: Enable auto-refresh on startup (default: True)
            refresh_interval: Auto-refresh interval in seconds (default: 2.0)
            **kwargs: Additional arguments passed to App base class
        """
        super().__init__(**kwargs)

        # Dependency injection
        self.task_data_service = task_data_service

        # Configuration
        self.current_view_mode = initial_view_mode
        self.auto_refresh_enabled = auto_refresh
        self.refresh_interval = refresh_interval

        # Internal state
        self._refresh_timer: Timer | None = None

    def on_mount(self) -> None:
        """Called when app starts - setup initial state.

        Lifecycle sequence:
        1. Install MainScreen as default screen
        2. Start auto-refresh timer if enabled
        3. Trigger initial data load
        """
        # Install main screen
        self.push_screen(MainScreen())

        # Start auto-refresh if enabled
        if self.auto_refresh_enabled:
            self._start_auto_refresh()

    def on_unmount(self) -> None:
        """Called when app shuts down - cleanup resources.

        Ensures proper cleanup:
        - Stop auto-refresh timer
        - Release service resources
        """
        self._stop_auto_refresh()

    def _start_auto_refresh(self) -> None:
        """Start periodic auto-refresh timer.

        Uses Textual's set_interval to trigger refresh_now action
        at the configured interval.
        """
        if self.refresh_interval > 0 and not self._refresh_timer:
            self._refresh_timer = self.set_interval(
                self.refresh_interval,
                self.action_refresh_now,
            )

    def _stop_auto_refresh(self) -> None:
        """Stop periodic auto-refresh timer.

        Safely stops and clears the refresh timer if active.
        """
        if self._refresh_timer:
            self._refresh_timer.stop()
            self._refresh_timer = None

    # Action methods (invoked by keybindings)

    async def action_quit(self) -> None:
        """Handle quit keybinding (q).

        Cleanly exits the application by calling Textual's exit method.
        """
        self.exit()

    def action_refresh_now(self) -> None:
        """Handle refresh keybinding (r).

        Triggers manual refresh of task data from services.
        Also called periodically by auto-refresh timer.
        """
        # Trigger refresh on current screen
        if hasattr(self.screen, "refresh_data"):
            self.screen.refresh_data()

    def action_show_filter_screen(self) -> None:
        """Handle filter keybinding (f).

        Opens FilterScreen modal for user to set filter criteria.
        Applies filter via callback when modal is dismissed.
        """
        self.push_screen(FilterScreen(), callback=self._on_filter_applied)

    def action_cycle_view_mode(self) -> None:
        """Handle view mode keybinding (v).

        Cycles through available view modes:
        TREE -> DEPENDENCY -> TIMELINE -> FEATURE_BRANCH -> FLAT_LIST -> TREE
        """
        modes = [
            ViewMode.TREE,
            ViewMode.DEPENDENCY,
            ViewMode.TIMELINE,
            ViewMode.FEATURE_BRANCH,
            ViewMode.FLAT_LIST,
        ]

        current_idx = modes.index(self.current_view_mode)
        next_idx = (current_idx + 1) % len(modes)
        self.current_view_mode = modes[next_idx]

    def action_show_help(self) -> None:
        """Handle help keybinding (?).

        Shows help overlay with keyboard shortcuts and usage information.

        Phase 1 placeholder: This will be fully implemented in later phases
        with a proper help modal screen showing all keybindings and features.
        """
        # Placeholder - will show HelpScreen modal in later phases
        pass

    # Event handlers

    def _on_filter_applied(self, filter_state: dict[str, Any] | None) -> None:
        """Handle filter application from FilterScreen modal.

        Args:
            filter_state: Filter criteria dict or None if cancelled

        Applies the filter to the current screen's data view.
        """
        if filter_state and hasattr(self.screen, "apply_filter"):
            self.screen.apply_filter(filter_state)

    # Reactive watchers

    def watch_current_view_mode(
        self, old_mode: ViewMode, new_mode: ViewMode
    ) -> None:
        """Called when view mode changes.

        Args:
            old_mode: Previous view mode
            new_mode: New view mode

        Triggers re-render of task visualization in the new mode.
        """
        # Guard: Only trigger refresh if app is mounted with screens
        if not self.is_running or not self._screen_stack:
            return

        # Trigger re-render in new view mode
        if hasattr(self.screen, "refresh_data"):
            self.screen.refresh_data()

    def watch_auto_refresh_enabled(
        self, old_enabled: bool, new_enabled: bool
    ) -> None:
        """Called when auto-refresh is enabled/disabled.

        Args:
            old_enabled: Previous auto-refresh state
            new_enabled: New auto-refresh state

        Starts or stops the auto-refresh timer accordingly.
        """
        # Guard: Only manage timers if app is mounted
        if not self.is_running:
            return

        if new_enabled and not old_enabled:
            self._start_auto_refresh()
        elif not new_enabled and old_enabled:
            self._stop_auto_refresh()

    def watch_refresh_interval(
        self, old_interval: float, new_interval: float
    ) -> None:
        """Called when refresh interval changes.

        Args:
            old_interval: Previous refresh interval
            new_interval: New refresh interval

        Restarts auto-refresh timer with new interval if active.
        """
        # Guard: Only restart timer if app is mounted and auto-refresh is enabled
        if not self.is_running or not self.auto_refresh_enabled:
            return

        self._stop_auto_refresh()
        self._start_auto_refresh()
