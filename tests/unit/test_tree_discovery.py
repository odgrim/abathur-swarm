"""Unit tests for TreeDiscoveryNode and _discover_task_tree().

Tests individual components in isolation:
- TreeDiscoveryNode construction from database rows
- TreeDiscoveryNode helper methods (is_leaf, matches_status)
- Tree discovery with WITH RECURSIVE CTE
- Edge cases and error scenarios
"""

import pytest
from uuid import uuid4
from pathlib import Path
from collections.abc import AsyncGenerator

from abathur.domain.models import TaskStatus
from abathur.infrastructure.database import Database, TreeDiscoveryNode


# Test fixtures
@pytest.fixture
async def memory_db() -> AsyncGenerator[Database, None]:
    """Create in-memory database for fast unit tests."""
    db = Database(Path(":memory:"))
    await db.initialize()
    yield db
    await db.close()


# TreeDiscoveryNode Unit Tests (4 tests)
class TestTreeDiscoveryNode:
    """Unit tests for TreeDiscoveryNode data model."""

    def test_treenode_from_row(self):
        """Test TreeDiscoveryNode construction from database row dictionary."""
        # Arrange
        task_id = uuid4()
        parent_id = uuid4()
        row_dict = {
            "id": str(task_id),
            "parent_id": str(parent_id),
            "status": "completed",
            "depth": 2
        }

        # Act
        node = TreeDiscoveryNode.from_row(row_dict)

        # Assert
        assert node.id == task_id
        assert node.parent_id == parent_id
        assert node.status == TaskStatus.COMPLETED
        assert node.depth == 2
        assert node.children_ids == []

    def test_treenode_from_row_without_parent(self):
        """Test TreeDiscoveryNode construction for root node (no parent)."""
        # Arrange
        task_id = uuid4()
        row_dict = {
            "id": str(task_id),
            "parent_id": None,
            "status": "completed",
            "depth": 0
        }

        # Act
        node = TreeDiscoveryNode.from_row(row_dict)

        # Assert
        assert node.id == task_id
        assert node.parent_id is None
        assert node.status == TaskStatus.COMPLETED
        assert node.depth == 0

    def test_treenode_is_leaf(self):
        """Test leaf detection (empty children_ids list)."""
        # Arrange - leaf node (no children)
        leaf_node = TreeDiscoveryNode(
            id=uuid4(),
            parent_id=uuid4(),
            status=TaskStatus.COMPLETED,
            depth=1,
            children_ids=[]
        )

        # Arrange - non-leaf node (has children)
        non_leaf_node = TreeDiscoveryNode(
            id=uuid4(),
            parent_id=uuid4(),
            status=TaskStatus.COMPLETED,
            depth=0,
            children_ids=[uuid4(), uuid4()]
        )

        # Act & Assert
        assert leaf_node.is_leaf() is True
        assert non_leaf_node.is_leaf() is False

    def test_treenode_matches_status(self):
        """Test status matching against allowed status list."""
        # Arrange
        node = TreeDiscoveryNode(
            id=uuid4(),
            parent_id=None,
            status=TaskStatus.COMPLETED,
            depth=0,
            children_ids=[]
        )

        # Act & Assert - status in allowed list
        assert node.matches_status([TaskStatus.COMPLETED, TaskStatus.FAILED]) is True

        # Act & Assert - status not in allowed list
        assert node.matches_status([TaskStatus.PENDING, TaskStatus.RUNNING]) is False

        # Act & Assert - empty allowed list
        assert node.matches_status([]) is False


# Tree Discovery Tests (7 tests)
class TestDiscoverTaskTree:
    """Unit tests for _discover_task_tree() CTE method."""

    @pytest.mark.asyncio
    async def test_discover_task_tree_single_level(self, memory_db: Database):
        """Test tree discovery with single root and 2 children."""
        # Arrange - create task hierarchy
        root_id = uuid4()
        child1_id = uuid4()
        child2_id = uuid4()

        async with memory_db._get_connection() as conn:
            # Insert root task
            await conn.execute(
                """INSERT INTO tasks (id, parent_task_id, status, summary, prompt,
                                     agent_type, priority, source, submitted_at)
                   VALUES (?, NULL, 'completed', 'Root', 'Root task', 'test-agent', 5,
                           'human', datetime('now'))""",
                (str(root_id),)
            )
            # Insert child tasks
            await conn.execute(
                """INSERT INTO tasks (id, parent_task_id, status, summary, prompt,
                                     agent_type, priority, source, submitted_at)
                   VALUES (?, ?, 'completed', 'Child1', 'Child 1', 'test-agent', 5,
                           'human', datetime('now'))""",
                (str(child1_id), str(root_id))
            )
            await conn.execute(
                """INSERT INTO tasks (id, parent_task_id, status, summary, description,
                                     agent_type, priority, source, submitted_at)
                   VALUES (?, ?, 'completed', 'Child2', 'Child 2', 'test-agent', 5,
                           'human', datetime('now'))""",
                (str(child2_id), str(root_id))
            )
            await conn.commit()

            # Act - discover tree
            nodes = await memory_db._discover_task_tree(conn, [root_id], max_depth=100)

        # Assert - verify all nodes returned
        assert len(nodes) == 3
        node_ids = {node.id for node in nodes}
        assert root_id in node_ids
        assert child1_id in node_ids
        assert child2_id in node_ids

        # Assert - verify depth calculation
        root_node = next(n for n in nodes if n.id == root_id)
        child1_node = next(n for n in nodes if n.id == child1_id)
        assert root_node.depth == 0
        assert child1_node.depth == 1

        # Assert - verify ordered by depth DESC (deepest first)
        assert nodes[0].depth >= nodes[-1].depth

    @pytest.mark.asyncio
    async def test_discover_task_tree_deep(self, memory_db: Database):
        """Test tree discovery with deep hierarchy (10 levels)."""
        # Arrange - create 10-level deep tree
        task_ids = [uuid4() for _ in range(10)]

        async with memory_db._get_connection() as conn:
            # Insert root (level 0)
            await conn.execute(
                """INSERT INTO tasks (id, parent_task_id, status, summary, description,
                                     agent_type, priority, source, submitted_at)
                   VALUES (?, NULL, 'completed', 'Level 0', 'Root', 'test-agent', 5,
                           'human', datetime('now'))""",
                (str(task_ids[0]),)
            )

            # Insert levels 1-9
            for i in range(1, 10):
                await conn.execute(
                    """INSERT INTO tasks (id, parent_task_id, status, summary, prompt,
                                         agent_type, priority, source, submitted_at)
                       VALUES (?, ?, 'completed', ?, 'Child', 'test-agent', 5,
                               'human', datetime('now'))""",
                    (str(task_ids[i]), str(task_ids[i-1]), f'Level {i}')
                )
            await conn.commit()

            # Act - discover tree
            nodes = await memory_db._discover_task_tree(conn, [task_ids[0]], max_depth=100)

        # Assert - verify all 10 levels discovered
        assert len(nodes) == 10

        # Assert - verify depth accuracy
        depths = {node.id: node.depth for node in nodes}
        for i, task_id in enumerate(task_ids):
            assert depths[task_id] == i

        # Assert - deepest node has depth=9
        deepest = nodes[0]  # First in DESC order
        assert deepest.depth == 9
        assert deepest.id == task_ids[9]

    @pytest.mark.asyncio
    async def test_discover_task_tree_wide(self, memory_db: Database):
        """Test tree discovery with wide tree (1 root, 100 children)."""
        # Arrange - create wide tree
        root_id = uuid4()
        child_ids = [uuid4() for _ in range(100)]

        async with memory_db._get_connection() as conn:
            # Insert root
            await conn.execute(
                """INSERT INTO tasks (id, parent_task_id, status, summary, prompt,
                                     agent_type, priority, source, submitted_at)
                   VALUES (?, NULL, 'completed', 'Root', 'Root task', 'test-agent', 5,
                           'human', datetime('now'))""",
                (str(root_id),)
            )

            # Insert 100 children
            for i, child_id in enumerate(child_ids):
                await conn.execute(
                    """INSERT INTO tasks (id, parent_task_id, status, summary, prompt,
                                         agent_type, priority, source, submitted_at)
                       VALUES (?, ?, 'completed', ?, 'Child task', 'test-agent', 5,
                               'human', datetime('now'))""",
                    (str(child_id), str(root_id), f'Child {i}')
                )
            await conn.commit()

            # Act - discover tree
            nodes = await memory_db._discover_task_tree(conn, [root_id], max_depth=100)

        # Assert - verify all 101 nodes returned (1 root + 100 children)
        assert len(nodes) == 101

        # Assert - verify all children have depth=1
        children = [n for n in nodes if n.id in child_ids]
        assert len(children) == 100
        assert all(c.depth == 1 for c in children)

        # Assert - verify root has depth=0
        root_node = next(n for n in nodes if n.id == root_id)
        assert root_node.depth == 0

    @pytest.mark.asyncio
    async def test_discover_task_tree_depth_calculation(self, memory_db: Database):
        """Test depth calculation accuracy in complex tree."""
        # Arrange - create tree with multiple branches at different depths
        #     root
        #    /    \
        #   a1     a2
        #   |      |  \
        #   b1     b2  b3
        #          |
        #          c1

        root_id, a1_id, a2_id, b1_id, b2_id, b3_id, c1_id = [uuid4() for _ in range(7)]

        async with memory_db._get_connection() as conn:
            # Insert all tasks
            tasks = [
                (root_id, None, 0),
                (a1_id, root_id, 1),
                (a2_id, root_id, 1),
                (b1_id, a1_id, 2),
                (b2_id, a2_id, 2),
                (b3_id, a2_id, 2),
                (c1_id, b2_id, 3),
            ]

            for task_id, parent_id, expected_depth in tasks:
                await conn.execute(
                    """INSERT INTO tasks (id, parent_task_id, status, summary, prompt,
                                         agent_type, priority, source, submitted_at)
                       VALUES (?, ?, 'completed', ?, 'Task', 'test-agent', 5,
                               'human', datetime('now'))""",
                    (str(task_id), str(parent_id) if parent_id else None, f'Task {task_id}')
                )
            await conn.commit()

            # Act - discover tree
            nodes = await memory_db._discover_task_tree(conn, [root_id], max_depth=100)

        # Assert - verify all nodes discovered
        assert len(nodes) == 7

        # Assert - verify exact depth for each node
        depth_map = {node.id: node.depth for node in nodes}
        assert depth_map[root_id] == 0
        assert depth_map[a1_id] == 1
        assert depth_map[a2_id] == 1
        assert depth_map[b1_id] == 2
        assert depth_map[b2_id] == 2
        assert depth_map[b3_id] == 2
        assert depth_map[c1_id] == 3

    @pytest.mark.asyncio
    async def test_discover_task_tree_max_depth(self, memory_db: Database):
        """Test max_depth enforcement prevents infinite traversal."""
        # Arrange - create deep tree (10 levels)
        task_ids = [uuid4() for _ in range(10)]

        async with memory_db._get_connection() as conn:
            # Insert root
            await conn.execute(
                """INSERT INTO tasks (id, parent_task_id, status, summary, description,
                                     agent_type, priority, source, submitted_at)
                   VALUES (?, NULL, 'completed', 'Root', 'Root', 'test-agent', 5,
                           'human', datetime('now'))""",
                (str(task_ids[0]),)
            )

            # Insert levels 1-9
            for i in range(1, 10):
                await conn.execute(
                    """INSERT INTO tasks (id, parent_task_id, status, summary, prompt,
                                         agent_type, priority, source, submitted_at)
                       VALUES (?, ?, 'completed', ?, 'Child', 'test-agent', 5,
                               'human', datetime('now'))""",
                    (str(task_ids[i]), str(task_ids[i-1]), f'Level {i}')
                )
            await conn.commit()

            # Act - discover tree with max_depth=5
            nodes = await memory_db._discover_task_tree(conn, [task_ids[0]], max_depth=5)

        # Assert - only 6 levels returned (0 through 5)
        assert len(nodes) == 6

        # Assert - deepest node has depth=5
        max_depth_found = max(n.depth for n in nodes)
        assert max_depth_found == 5

        # Assert - levels 6-9 not included
        node_ids = {node.id for node in nodes}
        for i in range(6, 10):
            assert task_ids[i] not in node_ids

    @pytest.mark.asyncio
    async def test_discover_task_tree_empty_roots(self, memory_db: Database):
        """Test ValueError raised when root_task_ids is empty."""
        # Arrange
        async with memory_db._get_connection() as conn:
            # Act & Assert - empty list raises ValueError
            with pytest.raises(ValueError, match="root_task_ids cannot be empty"):
                await memory_db._discover_task_tree(conn, [], max_depth=100)

    @pytest.mark.asyncio
    async def test_discover_task_tree_ordered_by_depth(self, memory_db: Database):
        """Test nodes returned in DESC order by depth (deepest first)."""
        # Arrange - create simple 3-level tree
        root_id, child_id, grandchild_id = uuid4(), uuid4(), uuid4()

        async with memory_db._get_connection() as conn:
            await conn.execute(
                """INSERT INTO tasks (id, parent_task_id, status, summary, description,
                                     agent_type, priority, source, submitted_at)
                   VALUES (?, NULL, 'completed', 'Root', 'Root', 'test-agent', 5,
                           'human', datetime('now'))""",
                (str(root_id),)
            )
            await conn.execute(
                """INSERT INTO tasks (id, parent_task_id, status, summary, description,
                                     agent_type, priority, source, submitted_at)
                   VALUES (?, ?, 'completed', 'Child', 'Child', 'test-agent', 5,
                           'human', datetime('now'))""",
                (str(child_id), str(root_id))
            )
            await conn.execute(
                """INSERT INTO tasks (id, parent_task_id, status, summary, description,
                                     agent_type, priority, source, submitted_at)
                   VALUES (?, ?, 'completed', 'Grandchild', 'Grandchild', 'test-agent', 5,
                           'human', datetime('now'))""",
                (str(grandchild_id), str(child_id))
            )
            await conn.commit()

            # Act
            nodes = await memory_db._discover_task_tree(conn, [root_id], max_depth=100)

        # Assert - nodes ordered by depth DESC
        assert len(nodes) == 3
        assert nodes[0].depth >= nodes[1].depth >= nodes[2].depth

        # Assert - first node is deepest (grandchild), last is root
        assert nodes[0].id == grandchild_id
        assert nodes[0].depth == 2
        assert nodes[-1].id == root_id
        assert nodes[-1].depth == 0
