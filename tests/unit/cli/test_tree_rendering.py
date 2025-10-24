"""Unit tests for CLI tree rendering functions.

Tests the new box-drawing character tree visualization:
- _build_tree_string() function with Unicode and ASCII modes
- Edge cases: empty trees, single nodes, deep hierarchies
- Connector character validation
- Depth truncation behavior
"""

import pytest
from rich.text import Text
from uuid import UUID, uuid4

from abathur.cli.main import _build_tree_string, _format_node_label
from abathur.domain.models import Task, TaskSource, TaskStatus
from abathur.tui.models import TreeNode


class TestBuildTreeString:
    """Unit tests for _build_tree_string() helper function."""

    def test_empty_tree_returns_empty_list(self):
        """Test that empty tree returns empty list."""
        # Arrange
        nodes = []

        # Act
        result = _build_tree_string(nodes, max_depth=5, use_unicode=True)

        # Assert
        assert result == []

    def test_single_root_node_unicode(self):
        """Test single root node with Unicode connectors."""
        # Arrange
        task = Task(
            prompt="Test task",
            summary="Test summary",
            agent_type="test-agent",
            source=TaskSource.HUMAN,
            status=TaskStatus.PENDING,
        )
        task.id = uuid4()
        node = TreeNode(task_id=task.id, task=task, children=[], level=0, position=0)
        nodes = [node]

        # Act
        result = _build_tree_string(nodes, max_depth=5, use_unicode=True)

        # Assert
        assert len(result) == 1
        assert isinstance(result[0], Text)
        # Should have last connector (└──) since it's the only root
        assert "└──" in result[0].plain

    def test_single_root_node_ascii(self):
        """Test single root node with ASCII connectors."""
        # Arrange
        task = Task(
            prompt="Test task",
            summary="Test summary",
            agent_type="test-agent",
            source=TaskSource.HUMAN,
            status=TaskStatus.PENDING,
        )
        task.id = uuid4()
        node = TreeNode(task_id=task.id, task=task, children=[], level=0, position=0)
        nodes = [node]

        # Act
        result = _build_tree_string(nodes, max_depth=5, use_unicode=False)

        # Assert
        assert len(result) == 1
        assert isinstance(result[0], Text)
        # Should have ASCII last connector (`--)
        assert "`--" in result[0].plain

    def test_two_root_nodes_unicode_connectors(self):
        """Test two root nodes use correct connectors (├── and └──)."""
        # Arrange
        task1 = Task(
            prompt="Task 1",
            summary="First task",
            agent_type="test-agent",
            source=TaskSource.HUMAN,
            status=TaskStatus.PENDING,
        )
        task1.id = uuid4()
        task2 = Task(
            prompt="Task 2",
            summary="Second task",
            agent_type="test-agent",
            source=TaskSource.HUMAN,
            status=TaskStatus.PENDING,
        )
        task2.id = uuid4()

        node1 = TreeNode(task_id=task1.id, task=task1, children=[], level=0, position=0)
        node2 = TreeNode(task_id=task2.id, task=task2, children=[], level=0, position=1)
        nodes = [node1, node2]

        # Act
        result = _build_tree_string(nodes, max_depth=5, use_unicode=True)

        # Assert
        assert len(result) == 2
        # First root should have mid connector (├──)
        assert "├──" in result[0].plain
        # Last root should have last connector (└──)
        assert "└──" in result[1].plain

    def test_parent_with_children_unicode(self):
        """Test parent-child relationship with Unicode vertical lines."""
        # Arrange
        parent_task = Task(
            prompt="Parent",
            summary="Parent task",
            agent_type="test-agent",
            source=TaskSource.HUMAN,
            status=TaskStatus.RUNNING,
        )
        parent_task.id = uuid4()

        child_task = Task(
            prompt="Child",
            summary="Child task",
            agent_type="test-agent",
            source=TaskSource.HUMAN,
            status=TaskStatus.PENDING,
            parent_task_id=parent_task.id,
        )
        child_task.id = uuid4()

        parent_node = TreeNode(task_id=parent_task.id, task=parent_task, children=[child_task.id], level=0, position=0)
        child_node = TreeNode(task_id=child_task.id, task=child_task, children=[], level=1, position=0)
        nodes = [parent_node, child_node]

        # Act
        result = _build_tree_string(nodes, max_depth=5, use_unicode=True)

        # Assert
        assert len(result) == 2
        # Parent should have last connector (only root)
        assert "└──" in result[0].plain
        # Child should be indented with spaces (parent is last)
        assert "    └──" in result[1].plain  # 4 spaces + connector

    def test_parent_with_multiple_children_unicode(self):
        """Test parent with multiple children shows correct connectors and vertical lines."""
        # Arrange
        parent_task = Task(
            prompt="Parent",
            summary="Parent task",
            agent_type="test-agent",
            source=TaskSource.HUMAN,
            status=TaskStatus.RUNNING,
        )
        parent_task.id = uuid4()

        child1_task = Task(
            prompt="Child 1",
            summary="First child",
            agent_type="test-agent",
            source=TaskSource.HUMAN,
            status=TaskStatus.PENDING,
            parent_task_id=parent_task.id,
        )
        child1_task.id = uuid4()

        child2_task = Task(
            prompt="Child 2",
            summary="Second child",
            agent_type="test-agent",
            source=TaskSource.HUMAN,
            status=TaskStatus.PENDING,
            parent_task_id=parent_task.id,
        )
        child2_task.id = uuid4()

        parent_node = TreeNode(task_id=parent_task.id, task=parent_task, children=[child1_task.id, child2_task.id], level=0, position=0)
        child1_node = TreeNode(task_id=child1_task.id, task=child1_task, children=[], level=0, position=0)
        child2_node = TreeNode(task_id=child2_task.id, task=child2_task, children=[], level=0, position=0)
        nodes = [parent_node, child1_node, child2_node]

        # Act
        result = _build_tree_string(nodes, max_depth=5, use_unicode=True)

        # Assert
        assert len(result) == 3
        # Parent
        assert "└──" in result[0].plain
        # First child should have mid connector (├──) with spaces prefix (parent is last)
        assert "    ├──" in result[1].plain
        # Second child should have last connector (└──)
        assert "    └──" in result[2].plain

    def test_depth_truncation_at_max_depth(self):
        """Test that tree truncates at max_depth with '...' indicator."""
        # Arrange - create deep hierarchy (5 levels)
        tasks = []
        nodes = []
        for i in range(5):
            task = Task(
                prompt=f"Level {i}",
                summary=f"Task at level {i}",
                agent_type="test-agent",
                source=TaskSource.HUMAN,
                status=TaskStatus.PENDING,
            )
            task.id = uuid4()
            if i > 0:
                task.parent_task_id = tasks[i - 1].id
            tasks.append(task)

        # Build nodes with parent-child relationships
        for i, task in enumerate(tasks):
            if i < len(tasks) - 1:
                children = [tasks[i + 1].id]
            else:
                children = []
            node = TreeNode(task_id=task.id, task=task, children=children, level=i, position=0)
            nodes.append(node)

        # Act - truncate at depth 3
        result = _build_tree_string(nodes, max_depth=3, use_unicode=True)

        # Assert
        # Should have 3 task lines + 1 truncation line
        assert len(result) == 4
        # Last line should be truncation indicator
        assert "..." in result[3].plain
        assert "more items" in result[3].plain

    def test_ascii_mode_uses_correct_characters(self):
        """Test ASCII mode uses |-- `-- | instead of Unicode box-drawing."""
        # Arrange
        parent_task = Task(
            prompt="Parent",
            summary="Parent",
            agent_type="test-agent",
            source=TaskSource.HUMAN,
            status=TaskStatus.PENDING,
        )
        parent_task.id = uuid4()

        child_task = Task(
            prompt="Child",
            summary="Child",
            agent_type="test-agent",
            source=TaskSource.HUMAN,
            status=TaskStatus.PENDING,
            parent_task_id=parent_task.id,
        )
        child_task.id = uuid4()

        parent_node = TreeNode(task_id=parent_task.id, task=parent_task, children=[child_task.id], level=0, position=0)
        child_node = TreeNode(task_id=child_task.id, task=child_task, children=[], level=1, position=0)
        nodes = [parent_node, child_node]

        # Act
        result = _build_tree_string(nodes, max_depth=5, use_unicode=False)

        # Assert
        # Parent should use ASCII last connector
        assert "`--" in result[0].plain
        # Child should use ASCII last connector with spaces
        assert "    `--" in result[1].plain
        # Should NOT contain Unicode characters
        assert "├" not in result[0].plain
        assert "└" not in result[0].plain
        assert "│" not in result[0].plain

    def test_wide_tree_many_siblings(self):
        """Test tree with many siblings (10+) renders correctly."""
        # Arrange
        parent_task = Task(
            prompt="Parent",
            summary="Parent with many children",
            agent_type="test-agent",
            source=TaskSource.HUMAN,
            status=TaskStatus.RUNNING,
        )
        parent_task.id = uuid4()

        child_ids = []
        child_nodes = []
        for i in range(15):
            child_task = Task(
                prompt=f"Child {i}",
                summary=f"Child {i}",
                agent_type="test-agent",
                source=TaskSource.HUMAN,
                status=TaskStatus.PENDING,
                parent_task_id=parent_task.id,
            )
            child_task.id = uuid4()
            child_ids.append(child_task.id)
            child_node = TreeNode(task_id=child_task.id, task=child_task, children=[], level=1, position=i)
            child_nodes.append(child_node)

        parent_node = TreeNode(task_id=parent_task.id, task=parent_task, children=child_ids, level=0, position=0)
        nodes = [parent_node] + child_nodes

        # Act
        result = _build_tree_string(nodes, max_depth=5, use_unicode=True)

        # Assert
        assert len(result) == 16  # 1 parent + 15 children
        # Parent
        assert "└──" in result[0].plain
        # First 14 children should have mid connector (├──)
        for i in range(1, 15):
            assert "├──" in result[i].plain
        # Last child should have last connector (└──)
        assert "└──" in result[15].plain

    def test_orphan_node_treated_as_root(self):
        """Test node with missing parent is treated as root."""
        # Arrange
        orphan_task = Task(
            prompt="Orphan",
            summary="Orphan task",
            agent_type="test-agent",
            source=TaskSource.HUMAN,
            status=TaskStatus.PENDING,
            parent_task_id=uuid4(),  # Parent doesn't exist in nodes list
        )
        orphan_task.id = uuid4()

        orphan_node = TreeNode(task_id=orphan_task.id, task=orphan_task, children=[], level=0, position=0)
        nodes = [orphan_node]

        # Act
        result = _build_tree_string(nodes, max_depth=5, use_unicode=True)

        # Assert
        # Should render as root (no error)
        assert len(result) == 1
        assert "└──" in result[0].plain

    def test_mixed_depth_branches(self):
        """Test tree with branches of varying depth."""
        # Arrange
        # Root with 2 branches: one shallow (1 level), one deep (3 levels)
        root = Task(
            prompt="Root",
            summary="Root",
            agent_type="test-agent",
            source=TaskSource.HUMAN,
            status=TaskStatus.RUNNING,
        )
        root.id = uuid4()

        # Shallow branch (just child1)
        child1 = Task(
            prompt="Child 1",
            summary="Shallow branch",
            agent_type="test-agent",
            source=TaskSource.HUMAN,
            status=TaskStatus.PENDING,
            parent_task_id=root.id,
        )
        child1.id = uuid4()

        # Deep branch (child2 -> grandchild -> great-grandchild)
        child2 = Task(
            prompt="Child 2",
            summary="Deep branch",
            agent_type="test-agent",
            source=TaskSource.HUMAN,
            status=TaskStatus.RUNNING,
            parent_task_id=root.id,
        )
        child2.id = uuid4()

        grandchild = Task(
            prompt="Grandchild",
            summary="Level 2",
            agent_type="test-agent",
            source=TaskSource.HUMAN,
            status=TaskStatus.PENDING,
            parent_task_id=child2.id,
        )
        grandchild.id = uuid4()

        great_grandchild = Task(
            prompt="Great-grandchild",
            summary="Level 3",
            agent_type="test-agent",
            source=TaskSource.HUMAN,
            status=TaskStatus.PENDING,
            parent_task_id=grandchild.id,
        )
        great_grandchild.id = uuid4()

        # Build nodes
        root_node = TreeNode(task_id=root.id, task=root, children=[child1.id, child2.id], level=0, position=0)
        child1_node = TreeNode(task_id=child1.id, task=child1, children=[], level=1, position=0)
        child2_node = TreeNode(task_id=child2.id, task=child2, children=[grandchild.id], level=1, position=1)
        grandchild_node = TreeNode(task_id=grandchild.id, task=grandchild, children=[great_grandchild.id], level=2, position=0)
        great_grandchild_node = TreeNode(task_id=great_grandchild.id, task=great_grandchild, children=[], level=3, position=0)

        nodes = [root_node, child1_node, child2_node, grandchild_node, great_grandchild_node]

        # Act
        result = _build_tree_string(nodes, max_depth=5, use_unicode=True)

        # Assert
        # Should have 5 lines (no truncation)
        assert len(result) == 5
        # Verify no truncation indicator
        assert not any("..." in line.plain for line in result)

    def test_connector_spacing_alignment(self):
        """Test that connector spacing is correct (3 spaces after vertical line)."""
        # Arrange
        parent = Task(
            prompt="Parent",
            summary="Parent",
            agent_type="test-agent",
            source=TaskSource.HUMAN,
            status=TaskStatus.RUNNING,
        )
        parent.id = uuid4()

        child = Task(
            prompt="Child",
            summary="Child",
            agent_type="test-agent",
            source=TaskSource.HUMAN,
            status=TaskStatus.PENDING,
            parent_task_id=parent.id,
        )
        child.id = uuid4()

        parent_node = TreeNode(task_id=parent.id, task=parent, children=[child.id], level=0, position=0)
        child_node = TreeNode(task_id=child.id, task=child, children=[], level=0, position=0)
        nodes = [parent_node, child_node]

        # Act
        result_unicode = _build_tree_string(nodes, max_depth=5, use_unicode=True)
        result_ascii = _build_tree_string(nodes, max_depth=5, use_unicode=False)

        # Assert
        # Unicode: parent is last, so child prefix should be 4 spaces + connector
        assert result_unicode[1].plain.startswith("    ")  # 4 spaces
        # ASCII: same spacing
        assert result_ascii[1].plain.startswith("    ")  # 4 spaces

    def test_returns_rich_text_objects(self):
        """Test that function returns list of Rich Text objects."""
        # Arrange
        task = Task(
            prompt="Test",
            summary="Test",
            agent_type="test-agent",
            source=TaskSource.HUMAN,
            status=TaskStatus.PENDING,
        )
        task.id = uuid4()
        node = TreeNode(task_id=task.id, task=task, children=[], level=0, position=0)
        nodes = [node]

        # Act
        result = _build_tree_string(nodes, max_depth=5, use_unicode=True)

        # Assert
        assert isinstance(result, list)
        assert all(isinstance(line, Text) for line in result)

    def test_preserves_status_colors_from_format_node_label(self):
        """Test that status colors from _format_node_label are preserved."""
        # Arrange
        completed_task = Task(
            prompt="Completed",
            summary="Completed task",
            agent_type="test-agent",
            source=TaskSource.HUMAN,
            status=TaskStatus.COMPLETED,
        )
        completed_task.id = uuid4()

        failed_task = Task(
            prompt="Failed",
            summary="Failed task",
            agent_type="test-agent",
            source=TaskSource.HUMAN,
            status=TaskStatus.FAILED,
        )
        failed_task.id = uuid4()

        completed_node = TreeNode(task_id=completed_task.id, task=completed_task, children=[], level=0, position=0)
        failed_node = TreeNode(task_id=failed_task.id, task=failed_task, children=[], level=0, position=0)
        nodes = [completed_node, failed_node]

        # Act
        result = _build_tree_string(nodes, max_depth=5, use_unicode=True)

        # Assert
        assert len(result) == 2
        # Both should have Rich Text formatting (styles preserved)
        assert result[0]._spans  # Rich Text has styling spans
        assert result[1]._spans  # Rich Text has styling spans
        # Verify task summaries are in output
        assert "Completed task" in result[0].plain
        assert "Failed task" in result[1].plain
