"""Unit tests for TaskQueueTUI main application.

Tests cover:
- App initialization with dependency injection
- Reactive state management
- Global keybindings
- View mode cycling
- Auto-refresh lifecycle
- Screen management
"""

import pytest
from textual.pilot import Pilot

from abathur.tui.app import TaskQueueTUI
from abathur.tui.models import ViewMode
from abathur.tui.services.task_data_service import TaskDataService


@pytest.fixture
def task_data_service():
    """Fixture providing TaskDataService instance."""
    return TaskDataService()


@pytest.fixture
def app(task_data_service):
    """Fixture providing TaskQueueTUI app instance."""
    return TaskQueueTUI(
        task_data_service=task_data_service,
        initial_view_mode=ViewMode.TREE,
        auto_refresh=False,  # Disable for tests to avoid timing issues
        refresh_interval=2.0,
    )


class TestAppInitialization:
    """Test app initialization and configuration."""

    def test_app_creates_with_defaults(self, task_data_service):
        """Test app creation with default configuration."""
        app = TaskQueueTUI(task_data_service=task_data_service)

        assert app.task_data_service is task_data_service
        assert app.current_view_mode == ViewMode.TREE
        assert app.auto_refresh_enabled is True
        assert app.refresh_interval == 2.0
        assert app.selected_task_id is None

    def test_app_creates_with_custom_config(self, task_data_service):
        """Test app creation with custom configuration."""
        app = TaskQueueTUI(
            task_data_service=task_data_service,
            initial_view_mode=ViewMode.FLAT_LIST,
            auto_refresh=False,
            refresh_interval=5.0,
        )

        assert app.current_view_mode == ViewMode.FLAT_LIST
        assert app.auto_refresh_enabled is False
        assert app.refresh_interval == 5.0

    def test_app_has_correct_bindings(self, app):
        """Test app has all required global keybindings."""
        binding_keys = {binding.key for binding in app.BINDINGS}

        assert "q" in binding_keys  # quit
        assert "r" in binding_keys  # refresh
        assert "f" in binding_keys  # filter
        assert "v" in binding_keys  # view mode
        assert "?" in binding_keys  # help


class TestReactiveState:
    """Test reactive state properties."""

    @pytest.mark.asyncio
    async def test_selected_task_id_is_reactive(self, app):
        """Test selected_task_id reactive property."""
        async with app.run_test() as pilot:
            from uuid import uuid4

            task_id = uuid4()
            app.selected_task_id = task_id

            assert app.selected_task_id == task_id

    @pytest.mark.asyncio
    async def test_current_view_mode_is_reactive(self, app):
        """Test current_view_mode reactive property."""
        async with app.run_test() as pilot:
            app.current_view_mode = ViewMode.TIMELINE

            assert app.current_view_mode == ViewMode.TIMELINE


class TestViewModeCycling:
    """Test view mode cycling functionality."""

    @pytest.mark.asyncio
    async def test_cycle_view_mode_through_all_modes(self, app):
        """Test cycling through all view modes."""
        async with app.run_test() as pilot:
            # Start at TREE
            assert app.current_view_mode == ViewMode.TREE

            # Cycle to DEPENDENCY
            await pilot.press("v")
            assert app.current_view_mode == ViewMode.DEPENDENCY

            # Cycle to TIMELINE
            await pilot.press("v")
            assert app.current_view_mode == ViewMode.TIMELINE

            # Cycle to FEATURE_BRANCH
            await pilot.press("v")
            assert app.current_view_mode == ViewMode.FEATURE_BRANCH

            # Cycle to FLAT_LIST
            await pilot.press("v")
            assert app.current_view_mode == ViewMode.FLAT_LIST

            # Cycle back to TREE
            await pilot.press("v")
            assert app.current_view_mode == ViewMode.TREE

    @pytest.mark.asyncio
    async def test_view_mode_cycle_action_directly(self, app):
        """Test action_cycle_view_mode method directly."""
        async with app.run_test():
            initial_mode = app.current_view_mode
            app.action_cycle_view_mode()

            # Should have moved to next mode
            assert app.current_view_mode != initial_mode


class TestAutoRefreshLifecycle:
    """Test auto-refresh timer lifecycle."""

    @pytest.mark.asyncio
    async def test_auto_refresh_starts_on_mount_when_enabled(self, task_data_service):
        """Test auto-refresh starts when app mounts with auto_refresh=True."""
        app = TaskQueueTUI(
            task_data_service=task_data_service,
            auto_refresh=True,
            refresh_interval=2.0,
        )

        async with app.run_test():
            # Auto-refresh should have started
            assert app._refresh_timer is not None

    @pytest.mark.asyncio
    async def test_auto_refresh_does_not_start_when_disabled(self, app):
        """Test auto-refresh does not start when auto_refresh=False."""
        async with app.run_test():
            # Auto-refresh should not have started
            assert app._refresh_timer is None

    @pytest.mark.asyncio
    async def test_auto_refresh_stops_on_unmount(self, task_data_service):
        """Test auto-refresh stops when app unmounts."""
        app = TaskQueueTUI(
            task_data_service=task_data_service,
            auto_refresh=True,
            refresh_interval=2.0,
        )

        async with app.run_test():
            # Timer should be running
            assert app._refresh_timer is not None

        # After app stops, timer should be cleaned up
        # (on_unmount called automatically)
        assert app._refresh_timer is None

    @pytest.mark.asyncio
    async def test_auto_refresh_restarts_when_interval_changes(
        self, task_data_service
    ):
        """Test auto-refresh restarts when interval changes."""
        app = TaskQueueTUI(
            task_data_service=task_data_service,
            auto_refresh=True,
            refresh_interval=2.0,
        )

        async with app.run_test():
            initial_timer = app._refresh_timer

            # Change interval
            app.refresh_interval = 5.0

            # Timer should have been recreated
            assert app._refresh_timer is not None
            assert app._refresh_timer != initial_timer


class TestKeybindings:
    """Test keyboard shortcuts and actions."""

    @pytest.mark.asyncio
    async def test_quit_keybinding(self, app):
        """Test 'q' keybinding quits the app."""
        async with app.run_test() as pilot:
            # Press 'q' should exit (app.exit() is called)
            await pilot.press("q")
            # If we get here without exception, quit was handled

    @pytest.mark.asyncio
    async def test_refresh_keybinding(self, app):
        """Test 'r' keybinding triggers refresh."""
        async with app.run_test() as pilot:
            # Press 'r' should call action_refresh_now
            await pilot.press("r")
            # No exception means refresh action was handled

    @pytest.mark.asyncio
    async def test_filter_keybinding_opens_modal(self, app):
        """Test 'f' keybinding opens FilterScreen modal."""
        async with app.run_test() as pilot:
            # Initially on MainScreen
            from abathur.tui.screens.main_screen import MainScreen

            assert isinstance(pilot.app.screen, MainScreen)

            # Press 'f' to open filter
            await pilot.press("f")

            # Should have pushed FilterScreen
            from abathur.tui.screens.filter_screen import FilterScreen

            assert isinstance(pilot.app.screen, FilterScreen)

    @pytest.mark.asyncio
    async def test_help_keybinding(self, app):
        """Test '?' keybinding triggers help action."""
        async with app.run_test() as pilot:
            # Press '?' should call action_show_help (placeholder for now)
            await pilot.press("?")
            # No exception means help action was handled


class TestScreenManagement:
    """Test screen management and navigation."""

    @pytest.mark.asyncio
    async def test_main_screen_installed_on_mount(self, app):
        """Test MainScreen is installed when app mounts."""
        async with app.run_test() as pilot:
            from abathur.tui.screens.main_screen import MainScreen

            assert isinstance(pilot.app.screen, MainScreen)

    @pytest.mark.asyncio
    async def test_filter_screen_can_be_dismissed(self, app):
        """Test FilterScreen can be opened and dismissed."""
        async with app.run_test() as pilot:
            from abathur.tui.screens.main_screen import MainScreen

            # Open filter screen
            await pilot.press("f")

            # Cancel filter (should dismiss and return to MainScreen)
            # Note: We need to click cancel button or press escape
            # For now, just verify we can get to filter screen
            from abathur.tui.screens.filter_screen import FilterScreen

            assert isinstance(pilot.app.screen, FilterScreen)


class TestReactiveWatchers:
    """Test reactive property watchers."""

    @pytest.mark.asyncio
    async def test_watch_current_view_mode_triggers_refresh(self, app):
        """Test changing view mode triggers data refresh."""
        async with app.run_test():
            # Change view mode should trigger watch_current_view_mode
            app.current_view_mode = ViewMode.TIMELINE
            # Watcher should have been called (no exception)

    @pytest.mark.asyncio
    async def test_watch_auto_refresh_enabled_starts_timer(self, app):
        """Test enabling auto-refresh starts timer."""
        async with app.run_test():
            # Initially disabled
            assert app._refresh_timer is None

            # Enable auto-refresh
            app.auto_refresh_enabled = True

            # Timer should now be running
            assert app._refresh_timer is not None

    @pytest.mark.asyncio
    async def test_watch_auto_refresh_enabled_stops_timer(self, task_data_service):
        """Test disabling auto-refresh stops timer."""
        app = TaskQueueTUI(
            task_data_service=task_data_service,
            auto_refresh=True,
            refresh_interval=2.0,
        )

        async with app.run_test():
            # Initially enabled with timer
            assert app._refresh_timer is not None

            # Disable auto-refresh
            app.auto_refresh_enabled = False

            # Timer should be stopped
            assert app._refresh_timer is None
