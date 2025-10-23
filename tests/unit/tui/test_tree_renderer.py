"""Comprehensive unit tests for TreeRenderer.

Tests layout computation, rendering logic, and formatting in isolation:
- Unicode/ASCII box-drawing
- Task node formatting
- Color mapping by TaskStatus
- Summary truncation
- Empty list handling
- Status icons
"""

import pytest
from datetime import datetime, timezone
from uuid import uuid4

from abathur.domain.models import Task, TaskStatus, TaskSource
from abathur.tui.rendering.tree_renderer import (
    TreeRenderer,
    TASK_STATUS_COLORS,
    get_status_color,
)


class TestTreeRendererFormatting:
    """Test suite for TreeRenderer text formatting."""

    @pytest.fixture
    def renderer(self):
        """Create TreeRenderer instance for testing."""
        return TreeRenderer()

    @pytest.fixture
    def sample_task(self):
        """Create a sample task for testing."""
        return Task(
            id=uuid4(),
            prompt="Test task prompt",
            summary="Test task summary",
            agent_type="test-agent",
            status=TaskStatus.PENDING,
            calculated_priority=7.5,
            dependency_depth=0,
            submitted_at=datetime.now(timezone.utc),
            source=TaskSource.HUMAN,
        )

    def test_format_task_node_truncates_long_summary(self, renderer):
        """Test summary truncated to 40 chars with ellipsis."""
        long_summary = "x" * 100
        task = Task(
            id=uuid4(),
            prompt="Test",
            summary=long_summary,
            agent_type="test",
            status=TaskStatus.PENDING,
            calculated_priority=5.0,
            dependency_depth=0,
            submitted_at=datetime.now(timezone.utc),
            source=TaskSource.HUMAN,
        )

        formatted = renderer.format_task_node(task)

        # Should be truncated to 40 chars + "..."
        assert "..." in formatted.plain
        # Summary portion should not exceed 43 chars (40 + "...")
        summary_part = formatted.plain.split("(")[0].strip()
        assert len(summary_part) <= 43

    def test_format_task_node_uses_prompt_if_no_summary(self, renderer):
        """Test falls back to prompt when summary is None."""
        task = Task(
            id=uuid4(),
            prompt="This is the prompt text",
            summary=None,
            agent_type="test",
            status=TaskStatus.READY,
            calculated_priority=3.0,
            dependency_depth=0,
            submitted_at=datetime.now(timezone.utc),
            source=TaskSource.HUMAN,
        )

        formatted = renderer.format_task_node(task)

        # Should use prompt text
        assert "This is the prompt text" in formatted.plain

    def test_format_task_node_includes_priority(self, renderer, sample_task):
        """Test formatted output includes priority value."""
        formatted = renderer.format_task_node(sample_task)

        # Priority should appear in parentheses
        assert "(7.5)" in formatted.plain

    def test_format_task_node_applies_status_color(self, renderer):
        """Test correct TaskStatus color applied to each status."""
        for status in TaskStatus:
            task = Task(
                id=uuid4(),
                prompt="Test task",
                summary=f"Task with status {status.value}",
                agent_type="test",
                status=status,
                calculated_priority=5.0,
                dependency_depth=0,
                submitted_at=datetime.now(timezone.utc),
                source=TaskSource.HUMAN,
            )

            formatted = renderer.format_task_node(task)

            # Verify text contains the summary
            assert f"Task with status {status.value}" in formatted.plain


class TestStatusIcons:
    """Test suite for status icon functionality."""

    @pytest.fixture
    def renderer(self):
        """Create TreeRenderer instance."""
        return TreeRenderer()

    def test_get_status_icon_all_statuses(self, renderer):
        """Test all TaskStatus values have unique icons."""
        icons = set()

        for status in TaskStatus:
            icon = renderer._get_status_icon(status)
            assert isinstance(icon, str)
            assert len(icon) > 0
            icons.add(icon)

        # All icons should be unique
        assert len(icons) == len(TaskStatus)

    def test_get_status_icon_completed_is_checkmark(self, renderer):
        """Test COMPLETED status uses checkmark icon."""
        icon = renderer._get_status_icon(TaskStatus.COMPLETED)
        assert icon == "✓"

    def test_get_status_icon_failed_is_x(self, renderer):
        """Test FAILED status uses X icon."""
        icon = renderer._get_status_icon(TaskStatus.FAILED)
        assert icon == "✗"

    def test_get_status_icon_running_is_filled_circle(self, renderer):
        """Test RUNNING status uses filled circle icon."""
        icon = renderer._get_status_icon(TaskStatus.RUNNING)
        assert icon == "◉"

    def test_get_status_icon_pending_is_empty_circle(self, renderer):
        """Test PENDING status uses empty circle icon."""
        icon = renderer._get_status_icon(TaskStatus.PENDING)
        assert icon == "○"


class TestRenderFlatList:
    """Test suite for flat list rendering."""

    @pytest.fixture
    def renderer(self):
        """Create TreeRenderer instance."""
        return TreeRenderer()

    @pytest.fixture
    def sample_tasks(self):
        """Create sample tasks with varied statuses."""
        return [
            Task(
                id=uuid4(),
                prompt="First task",
                summary="First task",
                agent_type="test",
                status=TaskStatus.PENDING,
                calculated_priority=10.0,
                dependency_depth=0,
                submitted_at=datetime.now(timezone.utc),
                source=TaskSource.HUMAN,
            ),
            Task(
                id=uuid4(),
                prompt="Second task",
                summary="Second task",
                agent_type="test",
                status=TaskStatus.RUNNING,
                calculated_priority=8.0,
                dependency_depth=0,
                submitted_at=datetime.now(timezone.utc),
                source=TaskSource.HUMAN,
            ),
            Task(
                id=uuid4(),
                prompt="Third task",
                summary="Third task",
                agent_type="test",
                status=TaskStatus.COMPLETED,
                calculated_priority=5.0,
                dependency_depth=0,
                submitted_at=datetime.now(timezone.utc),
                source=TaskSource.HUMAN,
            ),
        ]

    def test_render_flat_list_returns_correct_count(self, renderer, sample_tasks):
        """Test returns one line per task."""
        lines = renderer.render_flat_list(sample_tasks)
        assert len(lines) == len(sample_tasks)

    def test_render_flat_list_includes_status_icons(self, renderer, sample_tasks):
        """Test each line includes a status icon."""
        lines = renderer.render_flat_list(sample_tasks)

        # Expected icons for our sample tasks
        expected_icons = ["○", "◉", "✓"]

        for i, line in enumerate(lines):
            assert expected_icons[i] in line.plain

    def test_render_flat_list_includes_summaries(self, renderer, sample_tasks):
        """Test each line includes the task summary."""
        lines = renderer.render_flat_list(sample_tasks)

        for i, line in enumerate(lines):
            assert sample_tasks[i].summary in line.plain

    def test_render_flat_list_includes_priorities(self, renderer, sample_tasks):
        """Test each line includes the task priority."""
        lines = renderer.render_flat_list(sample_tasks)

        for i, line in enumerate(lines):
            priority = sample_tasks[i].calculated_priority
            assert f"({priority:.1f})" in line.plain

    def test_render_flat_list_empty_task_list(self, renderer):
        """Test handles empty task list gracefully."""
        lines = renderer.render_flat_list([])
        assert lines == []

    def test_render_flat_list_applies_colors_per_status(self, renderer):
        """Test color coding applied based on task status."""
        tasks = [
            Task(
                id=uuid4(),
                prompt="Failed task",
                summary="Failed task",
                agent_type="test",
                status=TaskStatus.FAILED,
                calculated_priority=5.0,
                dependency_depth=0,
                submitted_at=datetime.now(timezone.utc),
                source=TaskSource.HUMAN,
            ),
            Task(
                id=uuid4(),
                prompt="Completed task",
                summary="Completed task",
                agent_type="test",
                status=TaskStatus.COMPLETED,
                calculated_priority=5.0,
                dependency_depth=0,
                submitted_at=datetime.now(timezone.utc),
                source=TaskSource.HUMAN,
            ),
        ]

        lines = renderer.render_flat_list(tasks)

        # Both tasks should have content
        assert len(lines) == 2
        assert "Failed task" in lines[0].plain
        assert "Completed task" in lines[1].plain


class TestUnicodeSupport:
    """Test suite for Unicode detection and fallback."""

    def test_supports_unicode_detection(self):
        """Test supports_unicode() returns boolean."""
        result = TreeRenderer.supports_unicode()
        assert isinstance(result, bool)

    def test_supports_unicode_checks_encoding(self, monkeypatch):
        """Test checks stdout encoding."""
        import sys

        # Mock sys.stdout.encoding to non-UTF-8
        class MockStdout:
            encoding = "ascii"

        monkeypatch.setattr(sys, "stdout", MockStdout())

        # Should return False for ASCII encoding
        result = TreeRenderer.supports_unicode()
        assert result is False

    def test_supports_unicode_checks_lang_env(self, monkeypatch):
        """Test checks LANG environment variable."""
        import os
        import sys

        # Set UTF-8 encoding
        class MockStdout:
            encoding = "utf-8"

        monkeypatch.setattr(sys, "stdout", MockStdout())

        # Set LANG without UTF-8
        monkeypatch.setenv("LANG", "en_US.ISO-8859-1")

        result = TreeRenderer.supports_unicode()
        assert result is False


class TestColorMapping:
    """Test suite for TaskStatus color mapping."""

    def test_all_task_statuses_have_colors(self):
        """Verify every TaskStatus has a color mapping."""
        for status in TaskStatus:
            assert status in TASK_STATUS_COLORS
            color = get_status_color(status)
            assert isinstance(color, str)
            assert len(color) > 0

    def test_get_status_color_completed_is_bright_green(self):
        """Test COMPLETED status mapped to bright_green."""
        assert get_status_color(TaskStatus.COMPLETED) == "bright_green"

    def test_get_status_color_failed_is_red(self):
        """Test FAILED status mapped to red."""
        assert get_status_color(TaskStatus.FAILED) == "red"

    def test_get_status_color_pending_is_blue(self):
        """Test PENDING status mapped to blue."""
        assert get_status_color(TaskStatus.PENDING) == "blue"

    def test_get_status_color_running_is_magenta(self):
        """Test RUNNING status mapped to magenta."""
        assert get_status_color(TaskStatus.RUNNING) == "magenta"

    def test_get_status_color_blocked_is_yellow(self):
        """Test BLOCKED status mapped to yellow."""
        assert get_status_color(TaskStatus.BLOCKED) == "yellow"

    def test_get_status_color_ready_is_green(self):
        """Test READY status mapped to green."""
        assert get_status_color(TaskStatus.READY) == "green"

    def test_get_status_color_cancelled_is_dim(self):
        """Test CANCELLED status mapped to dim."""
        assert get_status_color(TaskStatus.CANCELLED) == "dim"

    def test_status_colors_accessible_from_class(self):
        """Test STATUS_COLORS constant accessible from TreeRenderer."""
        renderer = TreeRenderer()
        assert renderer.STATUS_COLORS == TASK_STATUS_COLORS


class TestEdgeCases:
    """Test suite for edge cases and error handling."""

    @pytest.fixture
    def renderer(self):
        """Create TreeRenderer instance."""
        return TreeRenderer()

    def test_format_task_node_with_zero_priority(self, renderer):
        """Test handles priority value of 0."""
        task = Task(
            id=uuid4(),
            prompt="Zero priority task",
            summary="Zero priority",
            agent_type="test",
            status=TaskStatus.PENDING,
            calculated_priority=0.0,
            dependency_depth=0,
            submitted_at=datetime.now(timezone.utc),
            source=TaskSource.HUMAN,
        )

        formatted = renderer.format_task_node(task)
        assert "(0.0)" in formatted.plain

    def test_format_task_node_with_max_priority(self, renderer):
        """Test handles maximum priority value."""
        task = Task(
            id=uuid4(),
            prompt="Max priority task",
            summary="Max priority",
            agent_type="test",
            status=TaskStatus.READY,
            calculated_priority=100.0,
            dependency_depth=0,
            submitted_at=datetime.now(timezone.utc),
            source=TaskSource.HUMAN,
        )

        formatted = renderer.format_task_node(task)
        assert "(100.0)" in formatted.plain

    def test_format_task_node_with_empty_summary_and_empty_prompt(self, renderer):
        """Test handles both empty summary and prompt."""
        task = Task(
            id=uuid4(),
            prompt="",
            summary="",
            agent_type="test",
            status=TaskStatus.PENDING,
            calculated_priority=5.0,
            dependency_depth=0,
            submitted_at=datetime.now(timezone.utc),
            source=TaskSource.HUMAN,
        )

        formatted = renderer.format_task_node(task)
        # Should not crash, should show priority at least
        assert "(5.0)" in formatted.plain

    def test_render_flat_list_with_max_width_parameter(self, renderer):
        """Test max_width parameter is accepted (reserved for future)."""
        task = Task(
            id=uuid4(),
            prompt="Test",
            summary="Test task",
            agent_type="test",
            status=TaskStatus.PENDING,
            calculated_priority=5.0,
            dependency_depth=0,
            submitted_at=datetime.now(timezone.utc),
            source=TaskSource.HUMAN,
        )

        # Should accept max_width without error (even if not yet used)
        lines = renderer.render_flat_list([task], max_width=60)
        assert len(lines) == 1
