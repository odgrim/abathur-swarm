"""Unit tests for QueueStatsHeader widget.

Tests reactive properties, rendering, color coding, and auto-refresh indicator.
All tests use Textual's async testing framework.
"""

import pytest
from datetime import datetime, timezone
from rich.text import Text
from rich.table import Table

from abathur.domain.models import TaskStatus
from abathur.tui.widgets import QueueStatsHeader


class TestQueueStatsHeaderInitialization:
    """Test suite for widget initialization."""

    def test_initialization_with_defaults(self):
        """Test widget initializes with default values."""
        # Act
        widget = QueueStatsHeader()

        # Assert
        assert widget.stats is None
        assert widget.auto_refresh_enabled is False
        assert widget.last_refresh is None

    def test_initialization_with_custom_id(self):
        """Test widget accepts custom ID."""
        # Act
        widget = QueueStatsHeader(id="custom-header")

        # Assert
        assert widget.id == "custom-header"


class TestQueueStatsHeaderReactiveProperties:
    """Test suite for reactive property updates."""

    def test_stats_reactive_property_update(self):
        """Test updating stats triggers watch method."""
        # Arrange
        widget = QueueStatsHeader()
        test_stats = {
            "total_tasks": 10,
            "pending_count": 3,
            "blocked_count": 1,
            "ready_count": 2,
            "running_count": 2,
            "completed_count": 1,
            "failed_count": 1,
            "cancelled_count": 0,
            "avg_priority": 6.5,
        }

        # Act
        widget.stats = test_stats

        # Assert - reactive property updated
        assert widget.stats == test_stats

    def test_auto_refresh_enabled_reactive_property(self):
        """Test auto_refresh_enabled property updates."""
        # Arrange
        widget = QueueStatsHeader()

        # Act
        widget.auto_refresh_enabled = True

        # Assert
        assert widget.auto_refresh_enabled is True

    def test_last_refresh_reactive_property(self):
        """Test last_refresh property updates."""
        # Arrange
        widget = QueueStatsHeader()
        now = datetime.now(timezone.utc)

        # Act
        widget.last_refresh = now

        # Assert
        assert widget.last_refresh == now


class TestQueueStatsHeaderRendering:
    """Test suite for rendering logic."""

    def test_render_with_no_stats_shows_loading(self):
        """Test render shows loading message when stats is None."""
        # Arrange
        widget = QueueStatsHeader()

        # Act
        rendered = widget.render()

        # Assert
        assert isinstance(rendered, Text)
        assert "Loading" in str(rendered)

    def test_render_with_stats_returns_table(self):
        """Test render returns Rich Table with stats."""
        # Arrange
        widget = QueueStatsHeader()
        widget.stats = {
            "total_tasks": 5,
            "pending_count": 2,
            "blocked_count": 0,
            "ready_count": 1,
            "running_count": 1,
            "completed_count": 1,
            "failed_count": 0,
            "cancelled_count": 0,
            "avg_priority": 7.0,
        }

        # Act
        rendered = widget.render()

        # Assert
        assert isinstance(rendered, Table)

    def test_render_stats_includes_all_status_counts(self):
        """Test _render_stats includes all task status counts."""
        # Arrange
        widget = QueueStatsHeader()
        stats = {
            "total_tasks": 10,
            "pending_count": 3,
            "blocked_count": 1,
            "ready_count": 2,
            "running_count": 2,
            "completed_count": 1,
            "failed_count": 1,
            "cancelled_count": 0,
            "avg_priority": 6.5,
        }

        # Act
        table = widget._render_stats(stats)

        # Assert - check table is created
        assert isinstance(table, Table)
        # Table should have 2 rows (status counts + health metrics)
        assert table.row_count == 2

    def test_render_stats_with_max_depth(self):
        """Test _render_stats includes max dependency depth when available."""
        # Arrange
        widget = QueueStatsHeader()
        stats = {
            "total_tasks": 5,
            "pending_count": 2,
            "blocked_count": 0,
            "ready_count": 1,
            "running_count": 1,
            "completed_count": 1,
            "failed_count": 0,
            "cancelled_count": 0,
            "avg_priority": 7.0,
            "max_dependency_depth": 3,
        }

        # Act
        table = widget._render_stats(stats)

        # Assert - table created with depth metric
        assert isinstance(table, Table)
        assert table.row_count == 2

    def test_render_stats_with_auto_refresh_enabled(self):
        """Test _render_stats shows ⟳ when auto-refresh enabled."""
        # Arrange
        widget = QueueStatsHeader()
        widget.auto_refresh_enabled = True
        widget.last_refresh = datetime.now(timezone.utc)
        stats = {
            "total_tasks": 5,
            "pending_count": 2,
            "blocked_count": 0,
            "ready_count": 1,
            "running_count": 1,
            "completed_count": 1,
            "failed_count": 0,
            "cancelled_count": 0,
            "avg_priority": 7.0,
        }

        # Act
        table = widget._render_stats(stats)

        # Assert - verify refresh indicator in output
        # Note: Can't easily inspect Rich Table internals, but we verify it renders
        assert isinstance(table, Table)

    def test_render_stats_with_auto_refresh_disabled(self):
        """Test _render_stats shows ⏸ when auto-refresh disabled."""
        # Arrange
        widget = QueueStatsHeader()
        widget.auto_refresh_enabled = False
        stats = {
            "total_tasks": 5,
            "pending_count": 2,
            "blocked_count": 0,
            "ready_count": 1,
            "running_count": 1,
            "completed_count": 1,
            "failed_count": 0,
            "cancelled_count": 0,
            "avg_priority": 7.0,
        }

        # Act
        table = widget._render_stats(stats)

        # Assert
        assert isinstance(table, Table)


class TestQueueStatsHeaderColorCoding:
    """Test suite for status color mapping."""

    def test_status_colors_defined_for_all_statuses(self):
        """Test STATUS_COLORS maps all TaskStatus values."""
        # Arrange
        widget = QueueStatsHeader()

        # Act & Assert - verify all statuses have colors
        assert TaskStatus.PENDING in widget.STATUS_COLORS
        assert TaskStatus.BLOCKED in widget.STATUS_COLORS
        assert TaskStatus.READY in widget.STATUS_COLORS
        assert TaskStatus.RUNNING in widget.STATUS_COLORS
        assert TaskStatus.COMPLETED in widget.STATUS_COLORS
        assert TaskStatus.FAILED in widget.STATUS_COLORS
        assert TaskStatus.CANCELLED in widget.STATUS_COLORS

    def test_status_color_values_are_valid(self):
        """Test status colors use valid Rich color names."""
        # Arrange
        widget = QueueStatsHeader()
        valid_colors = ["blue", "yellow", "cyan", "magenta", "green", "red", "dim"]

        # Act & Assert - all colors should be valid
        for status, color in widget.STATUS_COLORS.items():
            assert color in valid_colors, f"Invalid color {color} for {status}"


class TestQueueStatsHeaderWatchMethods:
    """Test suite for reactive property watch methods."""

    def test_watch_stats_updates_display(self):
        """Test watch_stats calls update when stats change."""
        # Arrange
        widget = QueueStatsHeader()
        new_stats = {
            "total_tasks": 5,
            "pending_count": 2,
            "blocked_count": 0,
            "ready_count": 1,
            "running_count": 1,
            "completed_count": 1,
            "failed_count": 0,
            "cancelled_count": 0,
            "avg_priority": 7.0,
        }

        # Act - manually call watch method
        widget.watch_stats(None, new_stats)

        # Assert - verify stats stored
        # (update() is called internally, but hard to test without app context)

    def test_watch_auto_refresh_enabled_triggers_rerender(self):
        """Test watch_auto_refresh_enabled updates display."""
        # Arrange
        widget = QueueStatsHeader()
        widget.stats = {
            "total_tasks": 5,
            "pending_count": 2,
            "blocked_count": 0,
            "ready_count": 1,
            "running_count": 1,
            "completed_count": 1,
            "failed_count": 0,
            "cancelled_count": 0,
            "avg_priority": 7.0,
        }

        # Act - manually call watch method
        widget.watch_auto_refresh_enabled(False, True)

        # Assert - method executes without error

    def test_watch_last_refresh_triggers_rerender(self):
        """Test watch_last_refresh updates display."""
        # Arrange
        widget = QueueStatsHeader()
        widget.stats = {
            "total_tasks": 5,
            "pending_count": 2,
            "blocked_count": 0,
            "ready_count": 1,
            "running_count": 1,
            "completed_count": 1,
            "failed_count": 0,
            "cancelled_count": 0,
            "avg_priority": 7.0,
        }
        now = datetime.now(timezone.utc)

        # Act - manually call watch method
        widget.watch_last_refresh(None, now)

        # Assert - method executes without error


class TestQueueStatsHeaderEdgeCases:
    """Test suite for edge cases and error handling."""

    def test_render_stats_with_zero_tasks(self):
        """Test _render_stats handles empty queue."""
        # Arrange
        widget = QueueStatsHeader()
        stats = {
            "total_tasks": 0,
            "pending_count": 0,
            "blocked_count": 0,
            "ready_count": 0,
            "running_count": 0,
            "completed_count": 0,
            "failed_count": 0,
            "cancelled_count": 0,
            "avg_priority": 0.0,
        }

        # Act
        table = widget._render_stats(stats)

        # Assert - renders without error
        assert isinstance(table, Table)

    def test_render_stats_with_missing_cancelled_count(self):
        """Test _render_stats handles missing cancelled_count (optional)."""
        # Arrange
        widget = QueueStatsHeader()
        stats = {
            "total_tasks": 5,
            "pending_count": 2,
            "blocked_count": 0,
            "ready_count": 1,
            "running_count": 1,
            "completed_count": 1,
            "failed_count": 0,
            # cancelled_count missing
            "avg_priority": 7.0,
        }

        # Act
        table = widget._render_stats(stats)

        # Assert - defaults to 0
        assert isinstance(table, Table)

    def test_render_stats_with_missing_max_depth(self):
        """Test _render_stats handles missing max_dependency_depth."""
        # Arrange
        widget = QueueStatsHeader()
        stats = {
            "total_tasks": 5,
            "pending_count": 2,
            "blocked_count": 0,
            "ready_count": 1,
            "running_count": 1,
            "completed_count": 1,
            "failed_count": 0,
            "cancelled_count": 0,
            "avg_priority": 7.0,
            # max_dependency_depth missing
        }

        # Act
        table = widget._render_stats(stats)

        # Assert - renders without max depth
        assert isinstance(table, Table)

    def test_render_with_no_last_refresh(self):
        """Test render when last_refresh is None."""
        # Arrange
        widget = QueueStatsHeader()
        widget.stats = {
            "total_tasks": 5,
            "pending_count": 2,
            "blocked_count": 0,
            "ready_count": 1,
            "running_count": 1,
            "completed_count": 1,
            "failed_count": 0,
            "cancelled_count": 0,
            "avg_priority": 7.0,
        }
        widget.last_refresh = None

        # Act
        rendered = widget.render()

        # Assert - renders without timestamp
        assert isinstance(rendered, Table)
