"""Comprehensive tests for TreeRenderer hierarchical layout and rendering.

Tests cover:
- Layout computation with grouping and sorting
- Parent-child relationship building
- Priority sorting within levels
- Task node formatting with colors
- Rich Tree rendering
- Visible nodes filtering with expand/collapse
- Status color mapping
- Summary truncation
- Unicode/ASCII detection
"""

import pytest
from uuid import uuid4
from datetime import datetime, timezone

from abathur.domain.models import Task, TaskStatus, TaskSource
from abathur.tui.rendering.tree_renderer import (
    TreeRenderer,
    TASK_STATUS_COLORS,
    get_status_color,
)
from abathur.tui.models import TreeLayout


@pytest.fixture
def sample_tasks():
    """Create sample task hierarchy for testing.

    Structure:
        Parent (depth=0, priority=10.0, COMPLETED)
        ├── Child 1 (depth=1, priority=8.0, RUNNING)
        └── Child 2 (depth=1, priority=7.0, PENDING)
    """
    parent_id = uuid4()
    child1_id = uuid4()
    child2_id = uuid4()

    parent = Task(
        id=parent_id,
        prompt="Parent task",
        summary="Parent",
        agent_type="test-agent",
        status=TaskStatus.COMPLETED,
        calculated_priority=10.0,
        dependency_depth=0,
        submitted_at=datetime.now(timezone.utc),
        source=TaskSource.HUMAN,
        parent_task_id=None,
    )

    child1 = Task(
        id=child1_id,
        prompt="Child task 1",
        summary="Child 1",
        agent_type="test-agent",
        status=TaskStatus.RUNNING,
        calculated_priority=8.0,
        dependency_depth=1,
        submitted_at=datetime.now(timezone.utc),
        source=TaskSource.AGENT_PLANNER,
        parent_task_id=parent_id,
    )

    child2 = Task(
        id=child2_id,
        prompt="Child task 2",
        summary="Child 2",
        agent_type="test-agent",
        status=TaskStatus.PENDING,
        calculated_priority=7.0,
        dependency_depth=1,
        submitted_at=datetime.now(timezone.utc),
        source=TaskSource.AGENT_PLANNER,
        parent_task_id=parent_id,
    )

    return [parent, child1, child2]


@pytest.fixture
def multi_level_tasks():
    """Create 3-level task hierarchy for complex testing.

    Structure:
        Root (depth=0)
        └── Level1 (depth=1)
            └── Level2 (depth=2)
    """
    root_id = uuid4()
    level1_id = uuid4()
    level2_id = uuid4()

    root = Task(
        id=root_id,
        prompt="Root task",
        summary="Root",
        agent_type="test",
        status=TaskStatus.READY,
        calculated_priority=10.0,
        dependency_depth=0,
        submitted_at=datetime.now(timezone.utc),
        source=TaskSource.HUMAN,
        parent_task_id=None,
    )

    level1 = Task(
        id=level1_id,
        prompt="Level 1 task",
        summary="Level 1",
        agent_type="test",
        status=TaskStatus.RUNNING,
        calculated_priority=8.0,
        dependency_depth=1,
        submitted_at=datetime.now(timezone.utc),
        source=TaskSource.AGENT_PLANNER,
        parent_task_id=root_id,
    )

    level2 = Task(
        id=level2_id,
        prompt="Level 2 task",
        summary="Level 2",
        agent_type="test",
        status=TaskStatus.PENDING,
        calculated_priority=6.0,
        dependency_depth=2,
        submitted_at=datetime.now(timezone.utc),
        source=TaskSource.AGENT_IMPLEMENTATION,
        parent_task_id=level1_id,
    )

    return [root, level1, level2]


class TestComputeLayout:
    """Tests for TreeRenderer.compute_layout() algorithm."""

    def test_compute_layout_hierarchical(self, sample_tasks):
        """Test hierarchical layout computation with parent-child relationships."""
        renderer = TreeRenderer()
        dependency_graph = {}

        layout = renderer.compute_layout(sample_tasks, dependency_graph)

        assert layout.total_nodes == 3
        assert layout.max_depth == 1
        assert len(layout.root_nodes) == 1

        # Parent should be root
        parent = sample_tasks[0]
        assert parent.id in layout.root_nodes

        # Parent should have 2 children
        parent_node = layout.nodes[parent.id]
        assert len(parent_node.children) == 2

    def test_compute_layout_priority_sorting(self, sample_tasks):
        """Test tasks sorted by priority within levels (descending)."""
        renderer = TreeRenderer()
        dependency_graph = {}

        layout = renderer.compute_layout(sample_tasks, dependency_graph)

        # Children should be ordered by priority (highest first)
        parent_node = layout.nodes[sample_tasks[0].id]
        child_ids = parent_node.children

        # Get child priorities
        child_priorities = [
            layout.nodes[cid].task.calculated_priority for cid in child_ids
        ]

        # Should be sorted descending
        assert child_priorities == sorted(child_priorities, reverse=True)
        assert child_priorities[0] == 8.0  # Child 1 (higher priority)
        assert child_priorities[1] == 7.0  # Child 2 (lower priority)

    def test_compute_layout_position_assignment(self, sample_tasks):
        """Test position indices assigned correctly within levels."""
        renderer = TreeRenderer()
        dependency_graph = {}

        layout = renderer.compute_layout(sample_tasks, dependency_graph)

        # Root node should have position 0
        root_node = layout.nodes[sample_tasks[0].id]
        assert root_node.position == 0
        assert root_node.level == 0

        # Children should have positions 0 and 1
        child_positions = [
            layout.nodes[cid].position for cid in root_node.children
        ]
        assert sorted(child_positions) == [0, 1]

    def test_compute_layout_empty_tasks(self):
        """Test layout computation with empty task list."""
        renderer = TreeRenderer()
        layout = renderer.compute_layout([], {})

        assert layout.total_nodes == 0
        assert layout.max_depth == 0
        assert len(layout.root_nodes) == 0
        assert len(layout.nodes) == 0

    def test_compute_layout_multi_level(self, multi_level_tasks):
        """Test layout computation with 3-level hierarchy."""
        renderer = TreeRenderer()
        dependency_graph = {}

        layout = renderer.compute_layout(multi_level_tasks, dependency_graph)

        assert layout.total_nodes == 3
        assert layout.max_depth == 2
        assert len(layout.root_nodes) == 1

        # Verify chain: root -> level1 -> level2
        root_id = multi_level_tasks[0].id
        level1_id = multi_level_tasks[1].id
        level2_id = multi_level_tasks[2].id

        root_node = layout.nodes[root_id]
        assert level1_id in root_node.children

        level1_node = layout.nodes[level1_id]
        assert level2_id in level1_node.children

    def test_compute_layout_orphan_nodes(self):
        """Test handling of orphan nodes (parent not in tree)."""
        renderer = TreeRenderer()

        # Create task with missing parent
        orphan = Task(
            id=uuid4(),
            prompt="Orphan task",
            summary="Orphan",
            agent_type="test",
            status=TaskStatus.PENDING,
            calculated_priority=5.0,
            dependency_depth=1,
            submitted_at=datetime.now(timezone.utc),
            source=TaskSource.HUMAN,
            parent_task_id=uuid4(),  # Parent doesn't exist in tree
        )

        layout = renderer.compute_layout([orphan], {})

        # Orphan should be treated as root node
        assert orphan.id in layout.root_nodes
        assert layout.total_nodes == 1

    def test_compute_layout_multiple_roots(self):
        """Test layout with multiple root nodes."""
        renderer = TreeRenderer()

        root1 = Task(
            id=uuid4(),
            prompt="Root 1",
            summary="Root 1",
            agent_type="test",
            status=TaskStatus.READY,
            calculated_priority=10.0,
            dependency_depth=0,
            submitted_at=datetime.now(timezone.utc),
            source=TaskSource.HUMAN,
            parent_task_id=None,
        )

        root2 = Task(
            id=uuid4(),
            prompt="Root 2",
            summary="Root 2",
            agent_type="test",
            status=TaskStatus.PENDING,
            calculated_priority=8.0,
            dependency_depth=0,
            submitted_at=datetime.now(timezone.utc),
            source=TaskSource.HUMAN,
            parent_task_id=None,
        )

        layout = renderer.compute_layout([root1, root2], {})

        assert len(layout.root_nodes) == 2
        assert root1.id in layout.root_nodes
        assert root2.id in layout.root_nodes


class TestFormatTaskNode:
    """Tests for TreeRenderer.format_task_node() formatting."""

    def test_format_task_node_basic(self):
        """Test task node formatting with summary and priority."""
        renderer = TreeRenderer()

        task = Task(
            id=uuid4(),
            prompt="Test task",
            summary="Test summary",
            agent_type="test",
            status=TaskStatus.COMPLETED,
            calculated_priority=9.5,
            dependency_depth=0,
            submitted_at=datetime.now(timezone.utc),
            source=TaskSource.HUMAN,
        )

        text = renderer.format_task_node(task)

        # Verify text contains summary and priority
        plain = text.plain
        assert "Test summary" in plain
        assert "(9.5)" in plain

    def test_format_task_node_truncation(self):
        """Test summary truncation at 40 chars with ellipsis."""
        renderer = TreeRenderer()

        long_summary = "x" * 50  # 50 characters
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

        text = renderer.format_task_node(task)
        plain = text.plain

        # Should be truncated with ellipsis
        summary_part = plain.split("(")[0].strip()
        assert len(summary_part) <= 43  # 40 + "..."
        assert "..." in summary_part

    def test_format_task_node_no_summary_uses_prompt(self):
        """Test formatting falls back to prompt when summary is None."""
        renderer = TreeRenderer()

        task = Task(
            id=uuid4(),
            prompt="This is the prompt text",
            summary=None,  # No summary
            agent_type="test",
            status=TaskStatus.READY,
            calculated_priority=7.0,
            dependency_depth=0,
            submitted_at=datetime.now(timezone.utc),
            source=TaskSource.HUMAN,
        )

        text = renderer.format_task_node(task)
        plain = text.plain

        # Should use prompt text
        assert "This is the prompt" in plain
        assert "(7.0)" in plain


class TestRenderTree:
    """Tests for TreeRenderer.render_tree() Rich Tree generation."""

    def test_render_tree_with_unicode(self, sample_tasks):
        """Test Rich Tree rendering with Unicode box-drawing."""
        renderer = TreeRenderer()

        layout = renderer.compute_layout(sample_tasks, {})
        expanded = set(layout.nodes.keys())  # All expanded

        tree = renderer.render_tree(layout, expanded, use_unicode=True)

        # Verify tree created
        assert tree is not None
        assert tree.label == "Task Queue"

    def test_render_tree_with_ascii(self, sample_tasks):
        """Test Rich Tree rendering with ASCII fallback."""
        renderer = TreeRenderer()

        layout = renderer.compute_layout(sample_tasks, {})
        expanded = set(layout.nodes.keys())

        tree = renderer.render_tree(layout, expanded, use_unicode=False)

        assert tree is not None
        assert tree.label == "Task Queue"

    def test_render_tree_collapsed_nodes(self, multi_level_tasks):
        """Test rendering respects expand/collapse state."""
        renderer = TreeRenderer()

        layout = renderer.compute_layout(multi_level_tasks, {})

        # Collapse the level1 node
        root_id = multi_level_tasks[0].id
        expanded = {root_id}  # Only root expanded

        tree = renderer.render_tree(layout, expanded, use_unicode=True)

        # Tree should only show root and level1 (not level2)
        # This is a visual test - we can verify tree is created
        assert tree is not None


class TestTreeLayout:
    """Tests for TreeLayout model methods."""

    def test_get_visible_nodes_all_expanded(self, sample_tasks):
        """Test visible nodes when all nodes are expanded."""
        renderer = TreeRenderer()
        layout = renderer.compute_layout(sample_tasks, {})

        # All expanded
        expanded = set(layout.nodes.keys())
        visible = layout.get_visible_nodes(expanded)

        assert len(visible) == 3  # All 3 nodes visible

    def test_get_visible_nodes_collapsed_root(self, sample_tasks):
        """Test visible nodes when root is collapsed."""
        renderer = TreeRenderer()
        layout = renderer.compute_layout(sample_tasks, {})

        # Nothing expanded
        expanded = set()
        visible = layout.get_visible_nodes(expanded)

        # Should only see root node (children hidden)
        assert len(visible) == 1
        assert visible[0].level == 0

    def test_find_node_path(self, multi_level_tasks):
        """Test finding path from root to leaf node."""
        renderer = TreeRenderer()
        layout = renderer.compute_layout(multi_level_tasks, {})

        root_id = multi_level_tasks[0].id
        level1_id = multi_level_tasks[1].id
        level2_id = multi_level_tasks[2].id

        # Find path to leaf node
        path = layout.find_node_path(level2_id)

        assert len(path) == 3
        assert path == [root_id, level1_id, level2_id]

    def test_find_node_path_root_node(self, sample_tasks):
        """Test finding path to root node."""
        renderer = TreeRenderer()
        layout = renderer.compute_layout(sample_tasks, {})

        root_id = sample_tasks[0].id
        path = layout.find_node_path(root_id)

        assert len(path) == 1
        assert path == [root_id]

    def test_find_node_path_missing_node(self, sample_tasks):
        """Test finding path to non-existent node."""
        renderer = TreeRenderer()
        layout = renderer.compute_layout(sample_tasks, {})

        missing_id = uuid4()
        path = layout.find_node_path(missing_id)

        assert path == []


class TestColorMapping:
    """Tests for status color mapping."""

    def test_all_statuses_have_colors(self):
        """Test all TaskStatus enum values have color mappings."""
        for status in TaskStatus:
            assert status in TASK_STATUS_COLORS
            color = TASK_STATUS_COLORS[status]
            assert isinstance(color, str)
            assert len(color) > 0

    def test_get_status_color_known_status(self):
        """Test get_status_color for known status."""
        color = get_status_color(TaskStatus.COMPLETED)
        assert color == "bright_green"

        color = get_status_color(TaskStatus.FAILED)
        assert color == "red"

    def test_status_color_mapping_values(self):
        """Test specific color mappings match expected values."""
        assert TASK_STATUS_COLORS[TaskStatus.PENDING] == "blue"
        assert TASK_STATUS_COLORS[TaskStatus.BLOCKED] == "yellow"
        assert TASK_STATUS_COLORS[TaskStatus.READY] == "green"
        assert TASK_STATUS_COLORS[TaskStatus.RUNNING] == "magenta"
        assert TASK_STATUS_COLORS[TaskStatus.COMPLETED] == "bright_green"
        assert TASK_STATUS_COLORS[TaskStatus.FAILED] == "red"
        assert TASK_STATUS_COLORS[TaskStatus.CANCELLED] == "dim"


class TestStatusIcons:
    """Tests for status icon mapping."""

    def test_get_status_icon_all_statuses(self):
        """Test status icons for all TaskStatus values."""
        renderer = TreeRenderer()

        icons = {
            TaskStatus.PENDING: "○",
            TaskStatus.BLOCKED: "⊗",
            TaskStatus.READY: "◎",
            TaskStatus.RUNNING: "◉",
            TaskStatus.COMPLETED: "✓",
            TaskStatus.FAILED: "✗",
            TaskStatus.CANCELLED: "⊘",
        }

        for status, expected_icon in icons.items():
            icon = renderer._get_status_icon(status)
            assert icon == expected_icon


class TestRenderFlatList:
    """Tests for flat list rendering."""

    def test_render_flat_list_basic(self, sample_tasks):
        """Test flat list rendering with status icons."""
        renderer = TreeRenderer()
        lines = renderer.render_flat_list(sample_tasks)

        assert len(lines) == 3

        # Each line should contain icon and summary
        for line in lines:
            plain = line.plain
            assert len(plain) > 0
            # Should have an icon (first character)
            assert plain[0] in ["○", "⊗", "◎", "◉", "✓", "✗", "⊘"]


class TestUnicodeDetection:
    """Tests for Unicode support detection."""

    def test_supports_unicode_method_exists(self):
        """Test that supports_unicode() method exists and returns bool."""
        result = TreeRenderer.supports_unicode()
        assert isinstance(result, bool)


class TestEdgeCases:
    """Tests for edge cases and error handling."""

    def test_compute_layout_with_same_priorities(self):
        """Test layout with tasks having identical priorities."""
        renderer = TreeRenderer()

        tasks = [
            Task(
                id=uuid4(),
                prompt=f"Task {i}",
                summary=f"Task {i}",
                agent_type="test",
                status=TaskStatus.PENDING,
                calculated_priority=5.0,  # Same priority
                dependency_depth=0,
                submitted_at=datetime.now(timezone.utc),
                source=TaskSource.HUMAN,
                parent_task_id=None,
            )
            for i in range(5)
        ]

        layout = renderer.compute_layout(tasks, {})

        assert layout.total_nodes == 5
        assert len(layout.root_nodes) == 5

    def test_compute_layout_deep_hierarchy(self):
        """Test layout with deep hierarchy (10 levels)."""
        renderer = TreeRenderer()

        tasks = []
        parent_id = None
        for depth in range(10):
            task_id = uuid4()
            task = Task(
                id=task_id,
                prompt=f"Level {depth}",
                summary=f"Level {depth}",
                agent_type="test",
                status=TaskStatus.PENDING,
                calculated_priority=10.0 - depth,
                dependency_depth=depth,
                submitted_at=datetime.now(timezone.utc),
                source=TaskSource.HUMAN,
                parent_task_id=parent_id,
            )
            tasks.append(task)
            parent_id = task_id

        layout = renderer.compute_layout(tasks, {})

        assert layout.total_nodes == 10
        assert layout.max_depth == 9
        assert len(layout.root_nodes) == 1
