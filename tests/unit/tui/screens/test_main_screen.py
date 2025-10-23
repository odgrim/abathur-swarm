"""Unit tests for MainScreen.

Tests the main screen composition, layout, and placeholder widgets.
"""

import pytest
from textual.widgets import Footer, Header, Static

from abathur.tui.app import TaskQueueTUI
from abathur.tui.screens.main_screen import MainScreen


@pytest.mark.asyncio
async def test_main_screen_composition():
    """Test that MainScreen composes all required widgets."""
    app = TaskQueueTUI()

    async with app.run_test() as pilot:
        # Get the main screen
        screen = app.screen
        assert isinstance(screen, MainScreen)

        # Verify Header widget is present
        headers = screen.query(Header)
        assert len(headers) == 1, "Should have exactly one Header"

        # Verify Footer widget is present
        footers = screen.query(Footer)
        assert len(footers) == 1, "Should have exactly one Footer"

        # Verify stats header placeholder
        stats_header = screen.query_one("#stats-header", Static)
        assert stats_header is not None
        assert "Queue Stats" in stats_header.renderable
        assert "Phase 3" in stats_header.renderable

        # Verify tree panel placeholder
        tree_panel = screen.query_one("#tree-panel", Static)
        assert tree_panel is not None
        assert "Task Tree" in tree_panel.renderable
        assert "Phase 2" in tree_panel.renderable

        # Verify detail panel placeholder
        detail_panel = screen.query_one("#detail-panel", Static)
        assert detail_panel is not None
        assert "Task Details" in detail_panel.renderable
        assert "Phase 3" in detail_panel.renderable


@pytest.mark.asyncio
async def test_main_screen_layout():
    """Test that MainScreen has correct layout structure."""
    app = TaskQueueTUI()

    async with app.run_test() as pilot:
        screen = app.screen

        # Verify content container exists
        content = screen.query_one("#content")
        assert content is not None

        # Verify horizontal split panels exist
        tree_panel = screen.query_one("#tree-panel")
        detail_panel = screen.query_one("#detail-panel")
        assert tree_panel is not None
        assert detail_panel is not None


@pytest.mark.asyncio
async def test_main_screen_mount():
    """Test that MainScreen on_mount sets app title and subtitle."""
    app = TaskQueueTUI()

    async with app.run_test() as pilot:
        # Check that app title and subtitle are set
        assert app.title == "Abathur Task Queue Visualizer"
        assert app.sub_title == "Phase 1 - Foundation"


@pytest.mark.asyncio
async def test_main_screen_refresh_data():
    """Test that refresh_data method exists and is callable.

    In Phase 1, this is a placeholder that does nothing.
    """
    app = TaskQueueTUI()

    async with app.run_test() as pilot:
        screen = app.screen
        assert isinstance(screen, MainScreen)

        # Should not raise an error
        await screen.refresh_data()


@pytest.mark.asyncio
async def test_main_screen_apply_filter():
    """Test that apply_filter method exists and is callable.

    In Phase 1, this is a placeholder that does nothing.
    """
    app = TaskQueueTUI()

    async with app.run_test() as pilot:
        screen = app.screen
        assert isinstance(screen, MainScreen)

        # Should not raise an error
        screen.apply_filter({"status": "pending"})


@pytest.mark.asyncio
async def test_main_screen_css_classes():
    """Test that MainScreen applies correct CSS classes."""
    app = TaskQueueTUI()

    async with app.run_test() as pilot:
        screen = app.screen

        # Verify stats header has correct ID
        stats_header = screen.query_one("#stats-header")
        assert stats_header.id == "stats-header"

        # Verify content container has correct ID
        content = screen.query_one("#content")
        assert content.id == "content"

        # Verify panels have correct IDs
        tree_panel = screen.query_one("#tree-panel")
        assert tree_panel.id == "tree-panel"

        detail_panel = screen.query_one("#detail-panel")
        assert detail_panel.id == "detail-panel"
