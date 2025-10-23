"""Unit tests for tree validation and ordering logic.

Tests validation methods used for recursive task tree deletion:
- _build_tree_structure: Adjacency list construction
- _validate_tree_deletability: Status validation for tree deletion
- _order_tasks_by_depth: Topological sorting for deletion order

These tests ensure FR003 (partial tree preservation) is correctly implemented.
"""

import pytest
from datetime import datetime, timezone
from pathlib import Path
from uuid import UUID, uuid4

from abathur.domain.models import Task, TaskStatus, TaskSource
from abathur.infrastructure.database import Database
from abathur.tui.models import TreeNode


@pytest.fixture
async def db_with_validation_methods():
    """Create in-memory database with validation helper methods.

    NOTE: This fixture assumes the validation methods (_build_tree_structure,
    _validate_tree_deletability, _order_tasks_by_depth) have been implemented
    in the Database class as part of Phase 2 Tasks 1-3.
    """
    db = Database(Path(":memory:"))
    await db.initialize()
    yield db
    await db.close()


def create_task(
    task_id: UUID | None = None,
    status: TaskStatus = TaskStatus.COMPLETED,
    parent_task_id: UUID | None = None,
    dependency_depth: int = 0,
    summary: str = "Test task",
) -> Task:
    """Helper to create Task object for testing."""
    return Task(
        id=task_id or uuid4(),
        prompt="Test task description",
        summary=summary,
        agent_type="test-agent",
        priority=5,
        status=status,
        source=TaskSource.HUMAN,
        parent_task_id=parent_task_id,
        calculated_priority=5.0,
        dependency_depth=dependency_depth,
        submitted_at=datetime.now(timezone.utc),
        last_updated_at=datetime.now(timezone.utc),
    )


def create_tree_node(
    task: Task,
    children: list[UUID] | None = None,
    level: int = 0,
) -> TreeNode:
    """Helper to create TreeNode object for testing."""
    return TreeNode(
        task_id=task.id,
        task=task,
        children=children or [],
        level=level,
        position=0,
    )


class TestBuildTreeStructure:
    """Unit tests for _build_tree_structure() method.

    Tests adjacency list construction from flat TreeNode list.
    """

    def test_build_tree_structure_simple_parent_child(self):
        """Test building adjacency list with simple parent-child relationship."""
        # Arrange
        parent_id = uuid4()
        child1_id = uuid4()
        child2_id = uuid4()

        parent_task = create_task(task_id=parent_id, parent_task_id=None)
        child1_task = create_task(task_id=child1_id, parent_task_id=parent_id)
        child2_task = create_task(task_id=child2_id, parent_task_id=parent_id)

        tree_nodes = [
            create_tree_node(parent_task),
            create_tree_node(child1_task),
            create_tree_node(child2_task),
        ]

        # Act
        db = Database(Path(":memory:"))  # Don't need initialized db for pure helper
        adjacency = db._build_tree_structure(tree_nodes)

        # Assert - adjacency list has parent -> children mapping
        assert parent_id in adjacency
        assert set(adjacency[parent_id]) == {child1_id, child2_id}

        # Assert - children field populated in TreeNode objects
        parent_node = next(n for n in tree_nodes if n.task_id == parent_id)
        assert set(parent_node.children) == {child1_id, child2_id}

        # Assert - child nodes have empty children lists
        child1_node = next(n for n in tree_nodes if n.task_id == child1_id)
        child2_node = next(n for n in tree_nodes if n.task_id == child2_id)
        assert child1_node.children == []
        assert child2_node.children == []

    def test_build_tree_structure_deep_hierarchy(self):
        """Test building adjacency list with multi-level hierarchy."""
        # Arrange - create 3-level tree: root -> mid -> leaf
        root_id = uuid4()
        mid_id = uuid4()
        leaf_id = uuid4()

        root_task = create_task(task_id=root_id, parent_task_id=None)
        mid_task = create_task(task_id=mid_id, parent_task_id=root_id)
        leaf_task = create_task(task_id=leaf_id, parent_task_id=mid_id)

        tree_nodes = [
            create_tree_node(root_task),
            create_tree_node(mid_task),
            create_tree_node(leaf_task),
        ]

        # Act
        db = Database(Path(":memory:"))
        adjacency = db._build_tree_structure(tree_nodes)

        # Assert - each level has correct children
        assert adjacency[root_id] == [mid_id]
        assert adjacency[mid_id] == [leaf_id]
        assert leaf_id not in adjacency  # Leaf has no children

        # Assert - TreeNode children populated correctly
        root_node = next(n for n in tree_nodes if n.task_id == root_id)
        mid_node = next(n for n in tree_nodes if n.task_id == mid_id)
        leaf_node = next(n for n in tree_nodes if n.task_id == leaf_id)

        assert root_node.children == [mid_id]
        assert mid_node.children == [leaf_id]
        assert leaf_node.children == []

    def test_build_tree_structure_multiple_roots(self):
        """Test building adjacency list with multiple root nodes."""
        # Arrange - two separate trees
        root1_id = uuid4()
        root2_id = uuid4()
        child1_id = uuid4()
        child2_id = uuid4()

        root1_task = create_task(task_id=root1_id, parent_task_id=None)
        root2_task = create_task(task_id=root2_id, parent_task_id=None)
        child1_task = create_task(task_id=child1_id, parent_task_id=root1_id)
        child2_task = create_task(task_id=child2_id, parent_task_id=root2_id)

        tree_nodes = [
            create_tree_node(root1_task),
            create_tree_node(root2_task),
            create_tree_node(child1_task),
            create_tree_node(child2_task),
        ]

        # Act
        db = Database(Path(":memory:"))
        adjacency = db._build_tree_structure(tree_nodes)

        # Assert - both roots have their children
        assert adjacency[root1_id] == [child1_id]
        assert adjacency[root2_id] == [child2_id]

        # Assert - roots don't appear as children
        all_children = [child for children in adjacency.values() for child in children]
        assert root1_id not in all_children
        assert root2_id not in all_children

    def test_build_tree_structure_empty_list(self):
        """Test building adjacency list with empty tree node list."""
        # Arrange
        tree_nodes = []

        # Act
        db = Database(Path(":memory:"))
        adjacency = db._build_tree_structure(tree_nodes)

        # Assert
        assert adjacency == {}

    def test_build_tree_structure_single_root_node(self):
        """Test building adjacency list with single root node (no children)."""
        # Arrange
        root_id = uuid4()
        root_task = create_task(task_id=root_id, parent_task_id=None)
        tree_nodes = [create_tree_node(root_task)]

        # Act
        db = Database(Path(":memory:"))
        adjacency = db._build_tree_structure(tree_nodes)

        # Assert - root has no children in adjacency list
        assert root_id not in adjacency or adjacency[root_id] == []

        # Assert - TreeNode children is empty
        assert tree_nodes[0].children == []


class TestValidateTreeDeletability:
    """Unit tests for _validate_tree_deletability() method.

    Tests FR003: Partial tree preservation when not all descendants match status.
    """

    def test_validate_tree_all_match(self):
        """Test validation when all nodes match allowed status -> all deletable."""
        # Arrange - tree with all COMPLETED nodes
        root_id = uuid4()
        child1_id = uuid4()
        child2_id = uuid4()

        tree = {
            root_id: create_tree_node(
                create_task(task_id=root_id, status=TaskStatus.COMPLETED),
                children=[child1_id, child2_id],
            ),
            child1_id: create_tree_node(
                create_task(task_id=child1_id, status=TaskStatus.COMPLETED, parent_task_id=root_id)
            ),
            child2_id: create_tree_node(
                create_task(task_id=child2_id, status=TaskStatus.COMPLETED, parent_task_id=root_id)
            ),
        }

        allowed_statuses = [TaskStatus.COMPLETED, TaskStatus.FAILED, TaskStatus.CANCELLED]

        # Act
        db = Database(Path(":memory:"))
        result = db._validate_tree_deletability(tree, root_id, allowed_statuses)

        # Assert - all nodes are deletable
        assert result == {root_id, child1_id, child2_id}

    def test_validate_tree_root_match_child_no_match(self):
        """Test FR003: Root matches, child doesn't -> preserve both (partial tree)."""
        # Arrange - root is COMPLETED, child is RUNNING (not deletable)
        root_id = uuid4()
        child_id = uuid4()

        tree = {
            root_id: create_tree_node(
                create_task(task_id=root_id, status=TaskStatus.COMPLETED),
                children=[child_id],
            ),
            child_id: create_tree_node(
                create_task(task_id=child_id, status=TaskStatus.RUNNING, parent_task_id=root_id)
            ),
        }

        allowed_statuses = [TaskStatus.COMPLETED, TaskStatus.FAILED, TaskStatus.CANCELLED]

        # Act
        db = Database(Path(":memory:"))
        result = db._validate_tree_deletability(tree, root_id, allowed_statuses)

        # Assert - FR003: preserve both root and child (empty set = no deletion)
        assert result == set()

    def test_validate_tree_root_no_match_child_match(self):
        """Test FR003: Root doesn't match, child matches -> delete child only."""
        # Arrange - root is RUNNING, child is COMPLETED
        root_id = uuid4()
        child_id = uuid4()

        tree = {
            root_id: create_tree_node(
                create_task(task_id=root_id, status=TaskStatus.RUNNING),
                children=[child_id],
            ),
            child_id: create_tree_node(
                create_task(task_id=child_id, status=TaskStatus.COMPLETED, parent_task_id=root_id)
            ),
        }

        allowed_statuses = [TaskStatus.COMPLETED, TaskStatus.FAILED, TaskStatus.CANCELLED]

        # Act
        db = Database(Path(":memory:"))
        result = db._validate_tree_deletability(tree, root_id, allowed_statuses)

        # Assert - FR003: delete child only, preserve root
        assert result == {child_id}

    def test_validate_tree_mixed_status_partial_deletion(self):
        """Test mixed status tree with partial deletions."""
        # Arrange - complex tree:
        #   root (COMPLETED)
        #   ├─ child1 (COMPLETED)
        #   │  └─ grandchild1 (COMPLETED) - deletable subtree
        #   └─ child2 (RUNNING)
        #      └─ grandchild2 (COMPLETED) - deletable, but parent not
        root_id = uuid4()
        child1_id = uuid4()
        child2_id = uuid4()
        grandchild1_id = uuid4()
        grandchild2_id = uuid4()

        tree = {
            root_id: create_tree_node(
                create_task(task_id=root_id, status=TaskStatus.COMPLETED),
                children=[child1_id, child2_id],
            ),
            child1_id: create_tree_node(
                create_task(task_id=child1_id, status=TaskStatus.COMPLETED, parent_task_id=root_id),
                children=[grandchild1_id],
            ),
            child2_id: create_tree_node(
                create_task(task_id=child2_id, status=TaskStatus.RUNNING, parent_task_id=root_id),
                children=[grandchild2_id],
            ),
            grandchild1_id: create_tree_node(
                create_task(task_id=grandchild1_id, status=TaskStatus.COMPLETED, parent_task_id=child1_id)
            ),
            grandchild2_id: create_tree_node(
                create_task(task_id=grandchild2_id, status=TaskStatus.COMPLETED, parent_task_id=child2_id)
            ),
        }

        allowed_statuses = [TaskStatus.COMPLETED, TaskStatus.FAILED, TaskStatus.CANCELLED]

        # Act
        db = Database(Path(":memory:"))
        result = db._validate_tree_deletability(tree, root_id, allowed_statuses)

        # Assert - FR003: cannot delete root because child2 is RUNNING
        # Expected: empty set (preserve entire tree due to non-matching descendant)
        assert result == set()

    def test_validate_tree_multiple_allowed_statuses(self):
        """Test validation with multiple allowed statuses (COMPLETED, FAILED, CANCELLED)."""
        # Arrange - tree with different terminal statuses (all allowed)
        root_id = uuid4()
        child1_id = uuid4()
        child2_id = uuid4()
        child3_id = uuid4()

        tree = {
            root_id: create_tree_node(
                create_task(task_id=root_id, status=TaskStatus.COMPLETED),
                children=[child1_id, child2_id, child3_id],
            ),
            child1_id: create_tree_node(
                create_task(task_id=child1_id, status=TaskStatus.COMPLETED, parent_task_id=root_id)
            ),
            child2_id: create_tree_node(
                create_task(task_id=child2_id, status=TaskStatus.FAILED, parent_task_id=root_id)
            ),
            child3_id: create_tree_node(
                create_task(task_id=child3_id, status=TaskStatus.CANCELLED, parent_task_id=root_id)
            ),
        }

        allowed_statuses = [TaskStatus.COMPLETED, TaskStatus.FAILED, TaskStatus.CANCELLED]

        # Act
        db = Database(Path(":memory:"))
        result = db._validate_tree_deletability(tree, root_id, allowed_statuses)

        # Assert - all nodes match allowed statuses -> all deletable
        assert result == {root_id, child1_id, child2_id, child3_id}

    def test_validate_tree_single_node_tree(self):
        """Test validation with single-node tree (root only)."""
        # Arrange - single root node with COMPLETED status
        root_id = uuid4()
        tree = {
            root_id: create_tree_node(create_task(task_id=root_id, status=TaskStatus.COMPLETED)),
        }

        allowed_statuses = [TaskStatus.COMPLETED]

        # Act
        db = Database(Path(":memory:"))
        result = db._validate_tree_deletability(tree, root_id, allowed_statuses)

        # Assert - single node matches -> deletable
        assert result == {root_id}

    def test_validate_tree_root_not_in_tree(self):
        """Test validation when root_id doesn't exist in tree (edge case)."""
        # Arrange - empty tree or tree without the specified root
        root_id = uuid4()
        other_id = uuid4()

        tree = {
            other_id: create_tree_node(create_task(task_id=other_id, status=TaskStatus.COMPLETED)),
        }

        allowed_statuses = [TaskStatus.COMPLETED]

        # Act
        db = Database(Path(":memory:"))
        result = db._validate_tree_deletability(tree, root_id, allowed_statuses)

        # Assert - root not found -> return empty set
        assert result == set()

    def test_validate_tree_child_not_in_tree(self):
        """Test validation when child node referenced but not in tree (edge case)."""
        # Arrange - root has child_id in children list, but child not in tree dict
        root_id = uuid4()
        missing_child_id = uuid4()

        root_task = create_task(task_id=root_id, status=TaskStatus.COMPLETED)
        root_node = create_tree_node(root_task)
        # Manually set children to include missing child
        root_node.children = [missing_child_id]

        tree = {
            root_id: root_node,
            # missing_child_id NOT in tree
        }

        allowed_statuses = [TaskStatus.COMPLETED]

        # Act
        db = Database(Path(":memory:"))
        result = db._validate_tree_deletability(tree, root_id, allowed_statuses)

        # Assert - root matches, missing child treated as "matches" -> root deletable
        assert result == {root_id}


class TestOrderTasksByDepth:
    """Unit tests for _order_tasks_by_depth() method.

    Tests topological sorting for deletion order (deepest first).
    """

    def test_order_tasks_deepest_first(self):
        """Test ordering tasks by depth, deepest first for safe deletion."""
        # Arrange - 3-level tree with depth tracking
        root_id = uuid4()
        mid_id = uuid4()
        leaf_id = uuid4()

        tasks_with_depth = [
            (root_id, 0),  # Root at depth 0
            (mid_id, 1),   # Middle at depth 1
            (leaf_id, 2),  # Leaf at depth 2
        ]

        # Act
        db = Database(Path(":memory:"))
        ordered = db._order_tasks_by_depth(tasks_with_depth)

        # Assert - deepest first (leaf -> mid -> root)
        assert ordered == [leaf_id, mid_id, root_id]

    def test_order_tasks_single_level(self):
        """Test ordering with tasks at same depth level."""
        # Arrange - all tasks at depth 0 (siblings)
        task1_id = uuid4()
        task2_id = uuid4()
        task3_id = uuid4()

        tasks_with_depth = [
            (task1_id, 0),
            (task2_id, 0),
            (task3_id, 0),
        ]

        # Act
        db = Database(Path(":memory:"))
        ordered = db._order_tasks_by_depth(tasks_with_depth)

        # Assert - all at same depth, order within level doesn't matter
        # Just verify all tasks are present
        assert set(ordered) == {task1_id, task2_id, task3_id}
        assert len(ordered) == 3

    def test_order_tasks_deep_hierarchy(self):
        """Test ordering with deep hierarchy (10 levels)."""
        # Arrange - create 10-level deep tree
        task_ids = [uuid4() for _ in range(10)]
        tasks_with_depth = [(task_id, depth) for depth, task_id in enumerate(task_ids)]

        # Act
        db = Database(Path(":memory:"))
        ordered = db._order_tasks_by_depth(tasks_with_depth)

        # Assert - verify deepest-first ordering
        # Depth 9 should come first, depth 0 last
        assert ordered[0] == task_ids[9]  # Deepest (depth 9)
        assert ordered[-1] == task_ids[0]  # Shallowest (depth 0)

        # Verify complete ordering is reversed
        assert ordered == list(reversed(task_ids))

    def test_order_tasks_mixed_depths(self):
        """Test ordering with tasks at various mixed depths."""
        # Arrange - realistic tree:
        #   depth 0: root
        #   depth 1: child1, child2
        #   depth 2: grandchild1 (under child1)
        #   depth 3: greatgrandchild1 (under grandchild1)
        root_id = uuid4()
        child1_id = uuid4()
        child2_id = uuid4()
        grandchild1_id = uuid4()
        greatgrandchild1_id = uuid4()

        tasks_with_depth = [
            (root_id, 0),
            (child1_id, 1),
            (child2_id, 1),
            (grandchild1_id, 2),
            (greatgrandchild1_id, 3),
        ]

        # Act
        db = Database(Path(":memory:"))
        ordered = db._order_tasks_by_depth(tasks_with_depth)

        # Assert - verify deepest-first grouping
        # Depth 3 first
        assert ordered[0] == greatgrandchild1_id

        # Depth 2 next
        assert ordered[1] == grandchild1_id

        # Depth 1 (child1, child2) - order within level may vary
        assert set(ordered[2:4]) == {child1_id, child2_id}

        # Depth 0 last
        assert ordered[4] == root_id

    def test_order_tasks_empty_list(self):
        """Test ordering with empty task list."""
        # Arrange
        tasks_with_depth = []

        # Act
        db = Database(Path(":memory:"))
        ordered = db._order_tasks_by_depth(tasks_with_depth)

        # Assert
        assert ordered == []

    def test_order_tasks_preserves_all_tasks(self):
        """Test that ordering preserves all input tasks (no duplicates or losses)."""
        # Arrange - varied depths
        task_ids = [uuid4() for _ in range(20)]
        tasks_with_depth = [
            (task_ids[i], i % 5)  # Distribute across 5 depth levels
            for i in range(20)
        ]

        # Act
        db = Database(Path(":memory:"))
        ordered = db._order_tasks_by_depth(tasks_with_depth)

        # Assert - all tasks present, no duplicates
        assert len(ordered) == 20
        assert set(ordered) == set(task_ids)
        assert len(set(ordered)) == 20  # No duplicates
