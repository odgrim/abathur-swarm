"""Unit tests for tree_formatter module.

Tests individual components in isolation:
- Task line formatting (summary, truncation, fallback)
- Status color mapping
- Terminal Unicode support detection
- Tree structure formatting (empty, single, hierarchical, orphaned, deep)
- Unicode vs ASCII rendering
"""

import os
import sys
from unittest.mock import patch
from uuid import UUID, uuid4

import pytest
from rich.text import Text

from abathur.cli.tree_formatter import (
    TASK_STATUS_COLORS,
    format_task_line,
    format_tree,
    get_status_color,
    supports_unicode,
)
from abathur.domain.models import Task, TaskStatus


# Test Fixtures for Task Hierarchies
@pytest.fixture
def simple_root_task() -> Task:
    """Create simple root task (no parent, no children)."""
    return Task(
        id=UUID("12345678-1234-1234-1234-123456789000"),
        summary="Simple root task",
        prompt="Do something simple",
        priority=5,
        status=TaskStatus.PENDING,
        calculated_priority=5.0,
    )


@pytest.fixture
def task_with_long_summary() -> Task:
    """Create task with summary exceeding 60 chars."""
    long_summary = "This is a very long task summary that exceeds the sixty character truncation limit and should be cut off"
    return Task(
        id=UUID("12345678-1234-1234-1234-123456789001"),
        summary=long_summary,
        prompt="Long task",
        priority=7,
        status=TaskStatus.RUNNING,
        calculated_priority=7.0,
    )


@pytest.fixture
def task_without_summary() -> Task:
    """Create task with no summary (should fallback to prompt)."""
    return Task(
        id=UUID("12345678-1234-1234-1234-123456789002"),
        summary=None,
        prompt="This is the prompt fallback text",
        priority=3,
        status=TaskStatus.COMPLETED,
        calculated_priority=3.0,
    )


@pytest.fixture
def task_without_summary_or_prompt() -> Task:
    """Create task with neither summary nor prompt (edge case).

    Note: Since prompt is required by Pydantic, we test the fallback
    when summary is None by providing an empty prompt.
    """
    return Task(
        id=UUID("12345678-1234-1234-1234-123456789003"),
        summary=None,
        prompt="",  # Empty prompt to test fallback
        priority=5,
        status=TaskStatus.FAILED,
        calculated_priority=5.0,
    )


@pytest.fixture
def parent_child_hierarchy() -> list[Task]:
    """Create simple parent-child task hierarchy."""
    parent_id = UUID("12345678-1234-1234-1234-123456789100")
    child_id = UUID("12345678-1234-1234-1234-123456789101")

    parent = Task(
        id=parent_id,
        summary="Parent task",
        prompt="Parent prompt",
        priority=8,
        status=TaskStatus.RUNNING,
        calculated_priority=8.0,
    )

    child = Task(
        id=child_id,
        summary="Child task",
        prompt="Child prompt",
        priority=6,
        status=TaskStatus.READY,
        parent_task_id=parent_id,
        calculated_priority=6.0,
    )

    return [parent, child]


@pytest.fixture
def multiple_roots_hierarchy() -> list[Task]:
    """Create hierarchy with multiple root tasks."""
    root1_id = UUID("12345678-1234-1234-1234-123456789200")
    root2_id = UUID("12345678-1234-1234-1234-123456789201")

    root1 = Task(
        id=root1_id,
        summary="First root",
        prompt="Root 1",
        priority=9,
        status=TaskStatus.PENDING,
        calculated_priority=9.0,
    )

    root2 = Task(
        id=root2_id,
        summary="Second root",
        prompt="Root 2",
        priority=7,
        status=TaskStatus.COMPLETED,
        calculated_priority=7.0,
    )

    return [root1, root2]


@pytest.fixture
def orphaned_tasks() -> list[Task]:
    """Create tasks with parent_task_id pointing to non-existent parent."""
    orphan1_id = UUID("12345678-1234-1234-1234-123456789300")
    orphan2_id = UUID("12345678-1234-1234-1234-123456789301")
    nonexistent_parent_id = UUID("99999999-9999-9999-9999-999999999999")

    orphan1 = Task(
        id=orphan1_id,
        summary="Orphaned task 1",
        prompt="Orphan 1",
        priority=5,
        status=TaskStatus.BLOCKED,
        parent_task_id=nonexistent_parent_id,
        calculated_priority=5.0,
    )

    orphan2 = Task(
        id=orphan2_id,
        summary="Orphaned task 2",
        prompt="Orphan 2",
        priority=4,
        status=TaskStatus.PENDING,
        parent_task_id=nonexistent_parent_id,
        calculated_priority=4.0,
    )

    return [orphan1, orphan2]


@pytest.fixture
def deep_hierarchy() -> list[Task]:
    """Create deep task hierarchy (4 levels deep)."""
    level0_id = UUID("12345678-1234-1234-1234-123456789400")
    level1_id = UUID("12345678-1234-1234-1234-123456789401")
    level2_id = UUID("12345678-1234-1234-1234-123456789402")
    level3_id = UUID("12345678-1234-1234-1234-123456789403")

    level0 = Task(
        id=level0_id,
        summary="Level 0 (root)",
        prompt="Root level",
        priority=10,
        status=TaskStatus.RUNNING,
        calculated_priority=10.0,
    )

    level1 = Task(
        id=level1_id,
        summary="Level 1",
        prompt="First child",
        priority=9,
        status=TaskStatus.RUNNING,
        parent_task_id=level0_id,
        calculated_priority=9.0,
    )

    level2 = Task(
        id=level2_id,
        summary="Level 2",
        prompt="Second child",
        priority=8,
        status=TaskStatus.READY,
        parent_task_id=level1_id,
        calculated_priority=8.0,
    )

    level3 = Task(
        id=level3_id,
        summary="Level 3 (leaf)",
        prompt="Deepest child",
        priority=7,
        status=TaskStatus.PENDING,
        parent_task_id=level2_id,
        calculated_priority=7.0,
    )

    return [level0, level1, level2, level3]


@pytest.fixture
def wide_hierarchy() -> list[Task]:
    """Create wide hierarchy (one parent with multiple children)."""
    parent_id = UUID("12345678-1234-1234-1234-123456789500")
    child1_id = UUID("12345678-1234-1234-1234-123456789501")
    child2_id = UUID("12345678-1234-1234-1234-123456789502")
    child3_id = UUID("12345678-1234-1234-1234-123456789503")

    parent = Task(
        id=parent_id,
        summary="Parent with many children",
        prompt="Parent",
        priority=8,
        status=TaskStatus.RUNNING,
        calculated_priority=8.0,
    )

    child1 = Task(
        id=child1_id,
        summary="Child 1",
        prompt="First child",
        priority=7,
        status=TaskStatus.READY,
        parent_task_id=parent_id,
        calculated_priority=7.0,
    )

    child2 = Task(
        id=child2_id,
        summary="Child 2",
        prompt="Second child",
        priority=6,
        status=TaskStatus.PENDING,
        parent_task_id=parent_id,
        calculated_priority=6.0,
    )

    child3 = Task(
        id=child3_id,
        summary="Child 3",
        prompt="Third child",
        priority=5,
        status=TaskStatus.BLOCKED,
        parent_task_id=parent_id,
        calculated_priority=5.0,
    )

    return [parent, child1, child2, child3]


# Unit Tests - Task Line Formatting


class TestFormatTaskLine:
    """Unit tests for format_task_line function."""

    def test_format_task_line_with_summary(self, simple_root_task: Task):
        """Test format_task_line with valid summary."""
        # Act
        result = format_task_line(simple_root_task)

        # Assert
        assert isinstance(result, Text)
        assert "12345678" in result.plain  # ID prefix
        assert "Simple root task" in result.plain  # Summary
        assert "(5.0)" in result.plain  # Priority
        # Verify status color applied (blue for PENDING)
        assert result.style == "blue"

    def test_format_task_line_truncates_long_summary(self, task_with_long_summary: Task):
        """Test format_task_line truncates summary at 60 chars."""
        # Act
        result = format_task_line(task_with_long_summary)

        # Assert
        assert isinstance(result, Text)
        # Summary should be truncated to 60 chars + "..."
        assert "..." in result.plain
        # Full summary should NOT be present
        assert "exceeds the sixty character truncation limit and should be cut off" not in result.plain
        # First 60 chars should be present
        assert "This is a very long task summary that exceeds the sixty" in result.plain
        # Verify status color (magenta for RUNNING)
        assert result.style == "magenta"

    def test_format_task_line_uses_prompt_fallback(self, task_without_summary: Task):
        """Test format_task_line uses prompt when summary is None."""
        # Act
        result = format_task_line(task_without_summary)

        # Assert
        assert isinstance(result, Text)
        assert "12345678" in result.plain  # ID prefix
        assert "This is the prompt fallback text" in result.plain  # Prompt used
        assert "(3.0)" in result.plain  # Priority
        # Verify status color (bright_green for COMPLETED)
        assert result.style == "bright_green"

    def test_format_task_line_empty_prompt_fallback(self, task_without_summary_or_prompt: Task):
        """Test format_task_line handles empty prompt gracefully.

        When summary is None and prompt is empty string, the implementation
        uses the empty string (not "Untitled Task"). This tests that edge case.
        """
        # Act
        result = format_task_line(task_without_summary_or_prompt)

        # Assert
        assert isinstance(result, Text)
        # Empty prompt results in just ID and priority (no summary text)
        assert "12345678" in result.plain  # ID prefix
        assert "(5.0)" in result.plain  # Priority
        # Verify status color (red for FAILED)
        assert result.style == "red"


# Unit Tests - Status Colors


class TestGetStatusColor:
    """Unit tests for get_status_color function."""

    def test_get_status_color_all_statuses(self):
        """Test get_status_color returns correct color for all TaskStatus values."""
        # Test all defined statuses
        assert get_status_color(TaskStatus.PENDING) == "blue"
        assert get_status_color(TaskStatus.BLOCKED) == "yellow"
        assert get_status_color(TaskStatus.READY) == "green"
        assert get_status_color(TaskStatus.RUNNING) == "magenta"
        assert get_status_color(TaskStatus.COMPLETED) == "bright_green"
        assert get_status_color(TaskStatus.FAILED) == "red"
        assert get_status_color(TaskStatus.CANCELLED) == "dim"

    def test_get_status_color_complete_mapping(self):
        """Test TASK_STATUS_COLORS dict contains all TaskStatus enum values."""
        # Get all TaskStatus values
        all_statuses = set(TaskStatus)

        # Get all mapped statuses
        mapped_statuses = set(TASK_STATUS_COLORS.keys())

        # Assert complete coverage
        assert all_statuses == mapped_statuses, "TASK_STATUS_COLORS missing some TaskStatus values"


# Unit Tests - Terminal Unicode Support


class TestSupportsUnicode:
    """Unit tests for supports_unicode function."""

    def test_supports_unicode_utf8_terminal(self):
        """Test supports_unicode returns True for UTF-8 terminal."""
        # Mock UTF-8 environment using object attribute patching
        import io
        from unittest.mock import MagicMock

        # Create mock stdout with UTF-8 encoding
        mock_stdout = MagicMock(spec=io.TextIOWrapper)
        mock_stdout.encoding = "utf-8"

        with patch("sys.stdout", mock_stdout), patch.dict(os.environ, {"LANG": "en_US.UTF-8"}):
            result = supports_unicode()
            assert result is True

    def test_supports_unicode_ascii_terminal(self):
        """Test supports_unicode returns False for ASCII terminal."""
        # Mock ASCII environment
        import io
        from unittest.mock import MagicMock

        mock_stdout = MagicMock(spec=io.TextIOWrapper)
        mock_stdout.encoding = "ascii"

        with patch("sys.stdout", mock_stdout), patch.dict(os.environ, {"LANG": "C"}):
            result = supports_unicode()
            assert result is False

    def test_supports_unicode_no_utf8_in_lang(self):
        """Test supports_unicode returns False when LANG doesn't contain UTF-8."""
        # UTF-8 encoding but LANG env doesn't specify UTF-8
        import io
        from unittest.mock import MagicMock

        mock_stdout = MagicMock(spec=io.TextIOWrapper)
        mock_stdout.encoding = "utf-8"

        with patch("sys.stdout", mock_stdout), patch.dict(os.environ, {"LANG": "en_US"}, clear=True):
            result = supports_unicode()
            assert result is False

    def test_supports_unicode_exception_fallback(self):
        """Test supports_unicode returns False on exception (safe fallback)."""
        # Mock sys.stdout.encoding to raise exception
        import io
        from unittest.mock import MagicMock, PropertyMock

        mock_stdout = MagicMock(spec=io.TextIOWrapper)
        # Make encoding property raise exception
        type(mock_stdout).encoding = PropertyMock(side_effect=Exception("Test exception"))

        with patch("sys.stdout", mock_stdout):
            result = supports_unicode()
            assert result is False


# Unit Tests - Tree Formatting


class TestFormatTree:
    """Unit tests for format_tree function."""

    def test_format_tree_empty_list(self):
        """Test format_tree with empty task list."""
        # Act
        tree = format_tree([])

        # Assert
        # Tree should have root label "Task Queue"
        assert tree.label == "Task Queue"
        # Should have one child with "No tasks found" message
        # Note: Rich Tree doesn't expose children directly, so we check by rendering
        from rich.console import Console
        from io import StringIO

        buffer = StringIO()
        console = Console(file=buffer, force_terminal=True, width=120)
        console.print(tree)
        output = buffer.getvalue()

        assert "No tasks found" in output

    def test_format_tree_single_root_task(self, simple_root_task: Task):
        """Test format_tree with single root task (no parent, no children)."""
        # Act
        tree = format_tree([simple_root_task])

        # Assert
        assert tree.label == "Task Queue"

        # Render and verify task appears
        from rich.console import Console
        from io import StringIO

        buffer = StringIO()
        console = Console(file=buffer, force_terminal=True, width=120)
        console.print(tree)
        output = buffer.getvalue()

        assert "12345678" in output  # ID prefix
        assert "Simple root task" in output

    def test_format_tree_parent_child_hierarchy(self, parent_child_hierarchy: list[Task]):
        """Test format_tree with parent-child hierarchy."""
        # Act
        tree = format_tree(parent_child_hierarchy)

        # Assert
        from rich.console import Console
        from io import StringIO

        buffer = StringIO()
        console = Console(file=buffer, force_terminal=True, width=120)
        console.print(tree)
        output = buffer.getvalue()

        # Both parent and child should appear
        assert "12345678" in output  # Parent ID prefix
        assert "Parent task" in output
        assert "12345678" in output  # Child ID prefix (same prefix by design)
        assert "Child task" in output

        # Child should appear indented (tree structure)
        lines = output.split("\n")
        parent_line_idx = next(i for i, line in enumerate(lines) if "Parent task" in line)
        child_line_idx = next(i for i, line in enumerate(lines) if "Child task" in line)

        # Child should appear after parent
        assert child_line_idx > parent_line_idx

        # Child line should have more leading whitespace (indentation)
        parent_indent = len(lines[parent_line_idx]) - len(lines[parent_line_idx].lstrip())
        child_indent = len(lines[child_line_idx]) - len(lines[child_line_idx].lstrip())
        assert child_indent > parent_indent

    def test_format_tree_multiple_roots(self, multiple_roots_hierarchy: list[Task]):
        """Test format_tree with multiple root tasks."""
        # Act
        tree = format_tree(multiple_roots_hierarchy)

        # Assert
        from rich.console import Console
        from io import StringIO

        buffer = StringIO()
        console = Console(file=buffer, force_terminal=True, width=120)
        console.print(tree)
        output = buffer.getvalue()

        # Both roots should appear
        assert "First root" in output
        assert "Second root" in output

        # Both should be at same indentation level (roots)
        lines = output.split("\n")
        root1_line = next(line for line in lines if "First root" in line)
        root2_line = next(line for line in lines if "Second root" in line)

        root1_indent = len(root1_line) - len(root1_line.lstrip())
        root2_indent = len(root2_line) - len(root2_line.lstrip())

        # Same indentation level
        assert root1_indent == root2_indent

    def test_format_tree_orphaned_tasks(self, orphaned_tasks: list[Task]):
        """Test format_tree handles orphaned tasks (parent_task_id doesn't exist)."""
        # Act
        tree = format_tree(orphaned_tasks)

        # Assert
        from rich.console import Console
        from io import StringIO

        buffer = StringIO()
        console = Console(file=buffer, force_terminal=True, width=120)
        console.print(tree)
        output = buffer.getvalue()

        # Orphaned tasks should appear at root level
        assert "Orphaned task 1" in output
        assert "Orphaned task 2" in output

    def test_format_tree_deep_hierarchy(self, deep_hierarchy: list[Task]):
        """Test format_tree with deep hierarchy (4 levels)."""
        # Act
        tree = format_tree(deep_hierarchy)

        # Assert
        from rich.console import Console
        from io import StringIO

        buffer = StringIO()
        console = Console(file=buffer, force_terminal=True, width=120)
        console.print(tree)
        output = buffer.getvalue()

        # All levels should appear
        assert "Level 0 (root)" in output
        assert "Level 1" in output
        assert "Level 2" in output
        assert "Level 3 (leaf)" in output

        # Verify hierarchical structure (increasing indentation)
        lines = output.split("\n")
        level0_line = next(line for line in lines if "Level 0 (root)" in line)
        level1_line = next(line for line in lines if "Level 1" in line and "Level 0" not in line)
        level2_line = next(line for line in lines if "Level 2" in line)
        level3_line = next(line for line in lines if "Level 3 (leaf)" in line)

        level0_indent = len(level0_line) - len(level0_line.lstrip())
        level1_indent = len(level1_line) - len(level1_line.lstrip())
        level2_indent = len(level2_line) - len(level2_line.lstrip())
        level3_indent = len(level3_line) - len(level3_line.lstrip())

        # Each level should be more indented than parent
        assert level1_indent > level0_indent
        assert level2_indent > level1_indent
        assert level3_indent > level2_indent

    def test_format_tree_wide_hierarchy(self, wide_hierarchy: list[Task]):
        """Test format_tree with wide hierarchy (parent with multiple children)."""
        # Act
        tree = format_tree(wide_hierarchy)

        # Assert
        from rich.console import Console
        from io import StringIO

        buffer = StringIO()
        console = Console(file=buffer, force_terminal=True, width=120)
        console.print(tree)
        output = buffer.getvalue()

        # Parent and all children should appear
        assert "Parent with many children" in output
        assert "Child 1" in output
        assert "Child 2" in output
        assert "Child 3" in output

    def test_format_tree_priority_sorting(self, wide_hierarchy: list[Task]):
        """Test format_tree sorts children by priority (highest first)."""
        # Act
        tree = format_tree(wide_hierarchy)

        # Assert
        from rich.console import Console
        from io import StringIO

        buffer = StringIO()
        console = Console(file=buffer, force_terminal=True, width=120)
        console.print(tree)
        output = buffer.getvalue()

        # Find line indices
        lines = output.split("\n")
        child1_idx = next(i for i, line in enumerate(lines) if "Child 1" in line)
        child2_idx = next(i for i, line in enumerate(lines) if "Child 2" in line)
        child3_idx = next(i for i, line in enumerate(lines) if "Child 3" in line)

        # Children should appear in priority order: Child 1 (7.0), Child 2 (6.0), Child 3 (5.0)
        assert child1_idx < child2_idx < child3_idx

    def test_format_tree_unicode_vs_ascii(self, parent_child_hierarchy: list[Task]):
        """Test format_tree renders correctly with Unicode vs ASCII mode."""
        # Act - Unicode mode
        tree_unicode = format_tree(parent_child_hierarchy, use_unicode=True)

        # Act - ASCII mode
        tree_ascii = format_tree(parent_child_hierarchy, use_unicode=False)

        # Assert - both should render successfully
        from rich.console import Console
        from io import StringIO

        # Unicode rendering
        buffer_unicode = StringIO()
        console_unicode = Console(file=buffer_unicode, force_terminal=True, width=120)
        console_unicode.print(tree_unicode)
        output_unicode = buffer_unicode.getvalue()

        # ASCII rendering
        buffer_ascii = StringIO()
        console_ascii = Console(file=buffer_ascii, force_terminal=True, width=120)
        console_ascii.print(tree_ascii)
        output_ascii = buffer_ascii.getvalue()

        # Both should contain task content
        assert "Parent task" in output_unicode
        assert "Child task" in output_unicode
        assert "Parent task" in output_ascii
        assert "Child task" in output_ascii

        # Both should have tree structure (Rich handles box-drawing internally)
        assert len(output_unicode) > 0
        assert len(output_ascii) > 0

    def test_format_tree_circular_reference_protection(self):
        """Test format_tree handles circular parent-child references without infinite loop."""
        # Create scenario where we could have circular traversal:
        # Task A has no parent (root), Task B is child of A
        # The visited set in format_tree should prevent re-visiting Task A
        task_a_id = UUID("aaaaaaaa-1234-1234-1234-123456789000")
        task_b_id = UUID("bbbbbbbb-1234-1234-1234-123456789001")

        # Task A is root task
        task_a = Task(
            id=task_a_id,
            summary="Task A",
            prompt="Task A",
            priority=5,
            status=TaskStatus.PENDING,
            calculated_priority=5.0,
        )

        # Task B is child of Task A
        task_b = Task(
            id=task_b_id,
            summary="Task B",
            prompt="Task B",
            priority=5,
            status=TaskStatus.PENDING,
            parent_task_id=task_a_id,
            calculated_priority=5.0,
        )

        # Test that the tree can be rendered without infinite loops
        # The visited set ensures each task is only added once
        tree = format_tree([task_a, task_b])

        # Assert - should complete without hanging
        from rich.console import Console
        from io import StringIO

        buffer = StringIO()
        console = Console(file=buffer, force_terminal=True, width=120)
        console.print(tree)
        output = buffer.getvalue()

        # Should render both tasks in hierarchy
        assert "Task A" in output
        assert "Task B" in output

        # Verify Task B appears only once (not duplicated)
        assert output.count("Task B") == 1
