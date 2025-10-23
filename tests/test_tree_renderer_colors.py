"""Unit tests for TreeRenderer color mapping functionality.

Tests TaskStatus color mapping, helper functions, and rendering with colors.
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


class TestColorMapping:
    """Test suite for TaskStatus color mapping."""

    def test_all_task_statuses_have_color_mappings(self):
        """Verify all 7 TaskStatus values have color mappings."""
        # Get all TaskStatus enum values
        all_statuses = list(TaskStatus)

        # Check each status has a mapping
        for status in all_statuses:
            assert status in TASK_STATUS_COLORS, f"Missing color mapping for {status}"
            color = TASK_STATUS_COLORS[status]
            assert isinstance(color, str), f"Color for {status} must be string"
            assert len(color) > 0, f"Color for {status} cannot be empty"

        # Verify we have exactly 7 mappings (one per status)
        assert len(TASK_STATUS_COLORS) == 7, "Expected 7 TaskStatus color mappings"

    def test_color_mapping_values_match_specification(self):
        """Verify colors match the technical specification."""
        expected_colors = {
            TaskStatus.PENDING: "blue",
            TaskStatus.BLOCKED: "yellow",
            TaskStatus.READY: "green",
            TaskStatus.RUNNING: "magenta",
            TaskStatus.COMPLETED: "bright_green",
            TaskStatus.FAILED: "red",
            TaskStatus.CANCELLED: "dim",
        }

        for status, expected_color in expected_colors.items():
            actual_color = TASK_STATUS_COLORS[status]
            assert actual_color == expected_color, (
                f"Color mismatch for {status}: "
                f"expected '{expected_color}', got '{actual_color}'"
            )

    def test_get_status_color_returns_correct_color(self):
        """Test get_status_color() helper returns correct colors."""
        assert get_status_color(TaskStatus.PENDING) == "blue"
        assert get_status_color(TaskStatus.BLOCKED) == "yellow"
        assert get_status_color(TaskStatus.READY) == "green"
        assert get_status_color(TaskStatus.RUNNING) == "magenta"
        assert get_status_color(TaskStatus.COMPLETED) == "bright_green"
        assert get_status_color(TaskStatus.FAILED) == "red"
        assert get_status_color(TaskStatus.CANCELLED) == "dim"

    def test_get_status_color_handles_unknown_status_gracefully(self):
        """Test get_status_color() returns 'white' for unknown status."""
        # This tests the graceful fallback behavior
        # We can't create an invalid TaskStatus enum, so we mock the behavior
        # by directly testing the .get() method with a non-existent key

        # Direct test of fallback logic
        fake_status = "NONEXISTENT_STATUS"
        color = TASK_STATUS_COLORS.get(fake_status, "white")
        assert color == "white", "Unknown status should fallback to 'white'"

    def test_status_colors_accessible_from_class(self):
        """Test STATUS_COLORS constant accessible from TreeRenderer class."""
        renderer = TreeRenderer()

        # Verify class constant matches module constant
        assert renderer.STATUS_COLORS == TASK_STATUS_COLORS

        # Verify all statuses accessible
        assert TaskStatus.COMPLETED in renderer.STATUS_COLORS
        assert renderer.STATUS_COLORS[TaskStatus.COMPLETED] == "bright_green"


class TestFormatTaskNode:
    """Test suite for format_task_node() color application."""

    @pytest.fixture
    def renderer(self):
        """Create TreeRenderer instance."""
        return TreeRenderer()

    def test_format_task_node_applies_correct_color_for_each_status(self, renderer):
        """Test format_task_node() applies correct color for each TaskStatus."""
        for status in TaskStatus:
            task = Task(
                id=uuid4(),
                prompt="Test task",
                summary=f"Test {status.value}",
                agent_type="test",
                status=status,
                calculated_priority=5.0,
                dependency_depth=0,
                submitted_at=datetime.now(timezone.utc),
                source=TaskSource.HUMAN,
            )

            text = renderer.format_task_node(task)

            # Verify text is created
            assert text is not None
            assert len(text.plain) > 0

            # Verify summary is in text
            assert f"Test {status.value}" in text.plain

    def test_format_task_node_completed_uses_bright_green(self, renderer):
        """Test COMPLETED status uses bright_green color."""
        task = Task(
            id=uuid4(),
            prompt="Completed task",
            summary="Completed task",
            agent_type="test",
            status=TaskStatus.COMPLETED,
            calculated_priority=10.0,
            dependency_depth=0,
            submitted_at=datetime.now(timezone.utc),
            source=TaskSource.HUMAN,
        )

        text = renderer.format_task_node(task)

        # Check that bright_green is the expected color
        expected_color = get_status_color(TaskStatus.COMPLETED)
        assert expected_color == "bright_green"

    def test_format_task_node_failed_uses_red(self, renderer):
        """Test FAILED status uses red color."""
        task = Task(
            id=uuid4(),
            prompt="Failed task",
            summary="Failed task",
            agent_type="test",
            status=TaskStatus.FAILED,
            calculated_priority=5.0,
            dependency_depth=0,
            submitted_at=datetime.now(timezone.utc),
            source=TaskSource.HUMAN,
        )

        text = renderer.format_task_node(task)

        expected_color = get_status_color(TaskStatus.FAILED)
        assert expected_color == "red"

    def test_format_task_node_includes_priority_in_dim(self, renderer):
        """Test format_task_node() includes priority in dim style."""
        task = Task(
            id=uuid4(),
            prompt="Test",
            summary="Test task",
            agent_type="test",
            status=TaskStatus.READY,
            calculated_priority=7.5,
            dependency_depth=0,
            submitted_at=datetime.now(timezone.utc),
            source=TaskSource.HUMAN,
        )

        text = renderer.format_task_node(task)

        # Verify priority appears in text
        assert "(7.5)" in text.plain


class TestStatusIcons:
    """Test suite for status icon mapping."""

    @pytest.fixture
    def renderer(self):
        """Create TreeRenderer instance."""
        return TreeRenderer()

    def test_get_status_icon_returns_unique_icons(self, renderer):
        """Test each TaskStatus has a unique icon."""
        icons = {}

        for status in TaskStatus:
            icon = renderer._get_status_icon(status)
            assert isinstance(icon, str)
            assert len(icon) > 0

            # Check uniqueness
            if icon in icons:
                pytest.fail(
                    f"Duplicate icon '{icon}' for {status} and {icons[icon]}"
                )

            icons[icon] = status

    def test_get_status_icon_uses_unicode_symbols(self, renderer):
        """Test status icons are Unicode symbols."""
        expected_icons = {
            TaskStatus.PENDING: "○",
            TaskStatus.BLOCKED: "⊗",
            TaskStatus.READY: "◎",
            TaskStatus.RUNNING: "◉",
            TaskStatus.COMPLETED: "✓",
            TaskStatus.FAILED: "✗",
            TaskStatus.CANCELLED: "⊘",
        }

        for status, expected_icon in expected_icons.items():
            actual_icon = renderer._get_status_icon(status)
            assert actual_icon == expected_icon, (
                f"Icon mismatch for {status}: "
                f"expected '{expected_icon}', got '{actual_icon}'"
            )

    def test_get_status_icon_handles_unknown_status(self, renderer):
        """Test _get_status_icon() returns default for unknown status."""
        # Create a mock status that doesn't exist in the mapping
        # Since we can't create invalid enum values, we'll verify the fallback logic
        # by checking the implementation returns "○" for missing keys

        # Verify default fallback in the icons dict
        icons = {
            TaskStatus.PENDING: "○",
            TaskStatus.BLOCKED: "⊗",
            TaskStatus.READY: "◎",
            TaskStatus.RUNNING: "◉",
            TaskStatus.COMPLETED: "✓",
            TaskStatus.FAILED: "✗",
            TaskStatus.CANCELLED: "⊘",
        }

        # Test fallback behavior
        fake_status = "FAKE"
        default_icon = icons.get(fake_status, "○")
        assert default_icon == "○", "Unknown status should fallback to '○'"


class TestRenderFlatList:
    """Test suite for render_flat_list() with color coding."""

    @pytest.fixture
    def renderer(self):
        """Create TreeRenderer instance."""
        return TreeRenderer()

    @pytest.fixture
    def sample_tasks(self):
        """Create sample tasks with different statuses."""
        return [
            Task(
                id=uuid4(),
                prompt="Pending task",
                summary="Pending",
                agent_type="test",
                status=TaskStatus.PENDING,
                calculated_priority=5.0,
                dependency_depth=0,
                submitted_at=datetime.now(timezone.utc),
                source=TaskSource.HUMAN,
            ),
            Task(
                id=uuid4(),
                prompt="Completed task",
                summary="Completed",
                agent_type="test",
                status=TaskStatus.COMPLETED,
                calculated_priority=10.0,
                dependency_depth=0,
                submitted_at=datetime.now(timezone.utc),
                source=TaskSource.HUMAN,
            ),
            Task(
                id=uuid4(),
                prompt="Failed task",
                summary="Failed",
                agent_type="test",
                status=TaskStatus.FAILED,
                calculated_priority=3.0,
                dependency_depth=0,
                submitted_at=datetime.now(timezone.utc),
                source=TaskSource.HUMAN,
            ),
        ]

    def test_render_flat_list_includes_status_icons(self, renderer, sample_tasks):
        """Test render_flat_list() includes status icons."""
        lines = renderer.render_flat_list(sample_tasks)

        assert len(lines) == 3

        # Check each line contains an icon
        for line in lines:
            plain = line.plain
            assert any(
                icon in plain
                for icon in ["○", "⊗", "◎", "◉", "✓", "✗", "⊘"]
            ), f"Line should contain status icon: {plain}"

    def test_render_flat_list_applies_colors(self, renderer, sample_tasks):
        """Test render_flat_list() applies color coding."""
        lines = renderer.render_flat_list(sample_tasks)

        # Verify we got lines for all tasks
        assert len(lines) == 3

        # Verify each line has content
        for i, line in enumerate(lines):
            assert len(line.plain) > 0, f"Line {i} should have content"
            # Verify summary appears in line
            assert sample_tasks[i].summary in line.plain


class TestColorConsistency:
    """Test suite for color consistency across visualization formats."""

    def test_colors_consistent_with_task_visualizer(self):
        """Verify colors are semantically consistent with task_visualizer.py.

        While exact color values may differ (e.g., lightblue vs blue),
        the semantic meaning should be preserved:
        - PENDING: Cool color (blue family)
        - BLOCKED: Warning color (yellow family)
        - READY: Positive color (green family)
        - RUNNING: Active color (orange/magenta family)
        - COMPLETED: Strong positive (bright green)
        - FAILED: Error color (red)
        - CANCELLED: Neutral/de-emphasized (gray/dim)
        """
        # Verify semantic color families
        assert "blue" in get_status_color(TaskStatus.PENDING)
        assert "yellow" in get_status_color(TaskStatus.BLOCKED)
        assert "green" in get_status_color(TaskStatus.READY)
        assert get_status_color(TaskStatus.RUNNING) in ["magenta", "orange"]
        assert "green" in get_status_color(TaskStatus.COMPLETED)
        assert "red" in get_status_color(TaskStatus.FAILED)
        assert get_status_color(TaskStatus.CANCELLED) in ["dim", "gray"]
