"""Unit tests for recursive query logic in Database class.

Tests individual components in isolation:
- TreeNode model (is_leaf, add_child)
- get_task_tree_with_status (tree traversal, status filtering, max_depth)
- check_tree_all_match_status (all match, some mismatch, validation)
- PruneFilters.recursive field validation
- RecursivePruneResult model validation
"""

from datetime import datetime, timezone
from uuid import UUID, uuid4

import pytest
from pydantic import ValidationError

from abathur.domain.models import TaskStatus
from abathur.infrastructure.database import (
    Database,
    PruneFilters,
    RecursivePruneResult,
    TreeNode,
)


# ============================================================================
# TreeNode Model Tests
# ============================================================================


class TestTreeNode:
    """Unit tests for TreeNode model."""

    def test_treenode_with_valid_data(self):
        """Test TreeNode accepts valid data and validates correctly."""
        # Arrange
        task_id = uuid4()
        parent_id = uuid4()
        valid_data = {
            "id": task_id,
            "parent_id": parent_id,
            "status": TaskStatus.COMPLETED,
            "depth": 2,
            "children_ids": [],
        }

        # Act
        node = TreeNode(**valid_data)

        # Assert
        assert node.id == task_id
        assert node.parent_id == parent_id
        assert node.status == TaskStatus.COMPLETED
        assert node.depth == 2
        assert node.children_ids == []

    def test_treenode_with_no_parent(self):
        """Test TreeNode works with parent_id=None (root node)."""
        # Arrange
        task_id = uuid4()
        data = {
            "id": task_id,
            "parent_id": None,
            "status": TaskStatus.PENDING,
            "depth": 0,
        }

        # Act
        node = TreeNode(**data)

        # Assert
        assert node.id == task_id
        assert node.parent_id is None
        assert node.depth == 0
        assert node.is_leaf() is True

    def test_treenode_depth_validation_valid_boundary(self):
        """Test depth validation at valid boundaries (0 and 100)."""
        # Arrange - depth=0 (minimum)
        data_min = {
            "id": uuid4(),
            "status": TaskStatus.READY,
            "depth": 0,
        }

        # Act
        node_min = TreeNode(**data_min)

        # Assert
        assert node_min.depth == 0

        # Arrange - depth=100 (maximum)
        data_max = {
            "id": uuid4(),
            "status": TaskStatus.READY,
            "depth": 100,
        }

        # Act
        node_max = TreeNode(**data_max)

        # Assert
        assert node_max.depth == 100

    def test_treenode_depth_validation_negative(self):
        """Test depth validation rejects negative values."""
        # Arrange
        invalid_data = {
            "id": uuid4(),
            "status": TaskStatus.PENDING,
            "depth": -1,
        }

        # Act & Assert
        with pytest.raises(ValidationError) as exc_info:
            TreeNode(**invalid_data)

        # Verify error mentions constraint
        assert "greater_than_equal" in str(exc_info.value).lower()

    def test_treenode_depth_validation_exceeds_max(self):
        """Test depth validation rejects values > 100."""
        # Arrange
        invalid_data = {
            "id": uuid4(),
            "status": TaskStatus.PENDING,
            "depth": 101,
        }

        # Act & Assert
        with pytest.raises(ValidationError) as exc_info:
            TreeNode(**invalid_data)

        # Verify error mentions constraint
        assert "less_than_equal" in str(exc_info.value).lower()

    def test_treenode_is_leaf_returns_true_for_no_children(self):
        """Test is_leaf() returns True when children_ids is empty."""
        # Arrange
        node = TreeNode(
            id=uuid4(),
            status=TaskStatus.COMPLETED,
            depth=1,
            children_ids=[],
        )

        # Act
        result = node.is_leaf()

        # Assert
        assert result is True

    def test_treenode_is_leaf_returns_false_with_children(self):
        """Test is_leaf() returns False when children_ids has items."""
        # Arrange
        node = TreeNode(
            id=uuid4(),
            status=TaskStatus.RUNNING,
            depth=1,
            children_ids=[uuid4(), uuid4()],
        )

        # Act
        result = node.is_leaf()

        # Assert
        assert result is False

    def test_treenode_add_child_appends_to_children_list(self):
        """Test add_child() appends child ID to children_ids list."""
        # Arrange
        node = TreeNode(
            id=uuid4(),
            status=TaskStatus.PENDING,
            depth=0,
            children_ids=[],
        )
        child_id = uuid4()

        # Act
        node.add_child(child_id)

        # Assert
        assert len(node.children_ids) == 1
        assert node.children_ids[0] == child_id
        assert node.is_leaf() is False

    def test_treenode_add_child_multiple_children(self):
        """Test add_child() can add multiple children sequentially."""
        # Arrange
        node = TreeNode(
            id=uuid4(),
            status=TaskStatus.READY,
            depth=0,
        )
        child1 = uuid4()
        child2 = uuid4()
        child3 = uuid4()

        # Act
        node.add_child(child1)
        node.add_child(child2)
        node.add_child(child3)

        # Assert
        assert len(node.children_ids) == 3
        assert node.children_ids == [child1, child2, child3]

    def test_treenode_serialization_includes_all_fields(self):
        """Test TreeNode serializes to dict with all fields present."""
        # Arrange
        task_id = uuid4()
        parent_id = uuid4()
        child_id = uuid4()
        node = TreeNode(
            id=task_id,
            parent_id=parent_id,
            status=TaskStatus.FAILED,
            depth=5,
            children_ids=[child_id],
        )

        # Act
        serialized = node.model_dump()

        # Assert
        assert "id" in serialized
        assert "parent_id" in serialized
        assert "status" in serialized
        assert "depth" in serialized
        assert "children_ids" in serialized
        assert serialized["id"] == task_id
        assert serialized["parent_id"] == parent_id
        assert serialized["status"] == TaskStatus.FAILED
        assert serialized["depth"] == 5
        assert serialized["children_ids"] == [child_id]


# ============================================================================
# get_task_tree_with_status Tests
# ============================================================================


class TestGetTaskTreeWithStatus:
    """Unit tests for Database.get_task_tree_with_status() method."""

    @pytest.mark.asyncio
    async def test_empty_root_task_ids_raises_valueerror(self, memory_db: Database):
        """Test that empty root_task_ids raises ValueError."""
        # Arrange
        empty_list: list[UUID] = []

        # Act & Assert
        with pytest.raises(ValueError, match="root_task_ids cannot be empty"):
            await memory_db.get_task_tree_with_status(
                root_task_ids=empty_list,
                filter_statuses=None,
                max_depth=100,
            )

    @pytest.mark.asyncio
    async def test_invalid_max_depth_zero_raises_valueerror(
        self, memory_db: Database
    ):
        """Test that max_depth=0 raises ValueError."""
        # Arrange
        root_id = uuid4()

        # Act & Assert
        with pytest.raises(ValueError, match="max_depth must be between 1 and 1000"):
            await memory_db.get_task_tree_with_status(
                root_task_ids=[root_id],
                filter_statuses=None,
                max_depth=0,
            )

    @pytest.mark.asyncio
    async def test_invalid_max_depth_negative_raises_valueerror(
        self, memory_db: Database
    ):
        """Test that negative max_depth raises ValueError."""
        # Arrange
        root_id = uuid4()

        # Act & Assert
        with pytest.raises(ValueError, match="max_depth must be between 1 and 1000"):
            await memory_db.get_task_tree_with_status(
                root_task_ids=[root_id],
                filter_statuses=None,
                max_depth=-1,
            )

    @pytest.mark.asyncio
    async def test_invalid_max_depth_exceeds_1000_raises_valueerror(
        self, memory_db: Database
    ):
        """Test that max_depth > 1000 raises ValueError."""
        # Arrange
        root_id = uuid4()

        # Act & Assert
        with pytest.raises(ValueError, match="max_depth must be between 1 and 1000"):
            await memory_db.get_task_tree_with_status(
                root_task_ids=[root_id],
                filter_statuses=None,
                max_depth=1001,
            )

    @pytest.mark.asyncio
    async def test_valid_max_depth_boundary_values(self, memory_db: Database):
        """Test that max_depth boundary values (1 and 1000) are accepted."""
        # Arrange
        root_id = uuid4()

        # Act & Assert - max_depth=1 should not raise
        try:
            await memory_db.get_task_tree_with_status(
                root_task_ids=[root_id],
                filter_statuses=None,
                max_depth=1,
            )
        except ValueError:
            pytest.fail("max_depth=1 should be valid")

        # Act & Assert - max_depth=1000 should not raise
        try:
            await memory_db.get_task_tree_with_status(
                root_task_ids=[root_id],
                filter_statuses=None,
                max_depth=1000,
            )
        except ValueError:
            pytest.fail("max_depth=1000 should be valid")

    @pytest.mark.asyncio
    async def test_basic_tree_traversal_single_root_no_children(
        self, memory_db: Database
    ):
        """Test basic traversal with single root task (no children)."""
        # Arrange - create root task
        from abathur.domain.models import Task

        root_task = Task(
            summary="Root task",

            prompt="Root task description",
            status=TaskStatus.COMPLETED,
        )
        await memory_db.insert_task(root_task)

        # Act
        result = await memory_db.get_task_tree_with_status(
            root_task_ids=[root_task.id],
            filter_statuses=None,
            max_depth=100,
        )

        # Assert
        assert len(result) == 1
        assert root_task.id in result
        node = result[root_task.id]
        assert node.id == root_task.id
        assert node.status == TaskStatus.COMPLETED
        assert node.depth == 0
        assert node.is_leaf() is True

    @pytest.mark.asyncio
    async def test_basic_tree_traversal_with_children(self, memory_db: Database):
        """Test traversal with root and child tasks."""
        # Arrange - create task tree: root -> child1, child2
        from abathur.domain.models import Task

        root_task = Task(
            summary="Root task",

            prompt="Root task description",
            status=TaskStatus.COMPLETED,
        )
        await memory_db.insert_task(root_task)

        child1 = Task(
            summary="Child 1",

            prompt="Child 1 description",
            parent_task_id=root_task.id,
            status=TaskStatus.COMPLETED,
        )
        await memory_db.insert_task(child1)

        child2 = Task(
            summary="Child 2",

            prompt="Child 2 description",
            parent_task_id=root_task.id,
            status=TaskStatus.FAILED,
        )
        await memory_db.insert_task(child2)

        # Act
        result = await memory_db.get_task_tree_with_status(
            root_task_ids=[root_task.id],
            filter_statuses=None,
            max_depth=100,
        )

        # Assert - should return all 3 tasks
        assert len(result) == 3
        assert root_task.id in result
        assert child1.id in result
        assert child2.id in result

        # Assert root node
        root_node = result[root_task.id]
        assert root_node.depth == 0
        assert len(root_node.children_ids) == 2
        assert child1.id in root_node.children_ids
        assert child2.id in root_node.children_ids

        # Assert child nodes
        child1_node = result[child1.id]
        assert child1_node.depth == 1
        assert child1_node.parent_id == root_task.id
        assert child1_node.is_leaf() is True

        child2_node = result[child2.id]
        assert child2_node.depth == 1
        assert child2_node.parent_id == root_task.id
        assert child2_node.is_leaf() is True

    @pytest.mark.asyncio
    async def test_status_filtering_returns_only_matching_statuses(
        self, memory_db: Database
    ):
        """Test status filtering returns only tasks with specified statuses."""
        # Arrange - create mixed status tree
        from abathur.domain.models import Task

        root = Task(summary="Root",
 prompt="Root description", status=TaskStatus.COMPLETED)
        await memory_db.insert_task(root)

        child_completed = Task(
            summary="Child completed",

            prompt="Child completed",
            parent_task_id=root.id,
            status=TaskStatus.COMPLETED,
        )
        await memory_db.insert_task(child_completed)

        child_failed = Task(
            summary="Child failed",

            prompt="Child failed",
            parent_task_id=root.id,
            status=TaskStatus.FAILED,
        )
        await memory_db.insert_task(child_failed)

        # Act - filter for COMPLETED only
        result = await memory_db.get_task_tree_with_status(
            root_task_ids=[root.id],
            filter_statuses=[TaskStatus.COMPLETED],
            max_depth=100,
        )

        # Assert - should only return COMPLETED tasks
        assert len(result) == 2
        assert root.id in result
        assert child_completed.id in result
        assert child_failed.id not in result

    @pytest.mark.asyncio
    async def test_status_filtering_with_multiple_statuses(self, memory_db: Database):
        """Test filtering with multiple allowed statuses."""
        # Arrange
        from abathur.domain.models import Task

        root = Task(summary="Root",
 prompt="Root description", status=TaskStatus.COMPLETED)
        await memory_db.insert_task(root)

        child1 = Task(
            summary="Child 1",

            prompt="Child 1",
            parent_task_id=root.id,
            status=TaskStatus.FAILED,
        )
        await memory_db.insert_task(child1)

        child2 = Task(
            summary="Child 2",

            prompt="Child 2",
            parent_task_id=root.id,
            status=TaskStatus.PENDING,
        )
        await memory_db.insert_task(child2)

        # Act - filter for COMPLETED and FAILED
        result = await memory_db.get_task_tree_with_status(
            root_task_ids=[root.id],
            filter_statuses=[TaskStatus.COMPLETED, TaskStatus.FAILED],
            max_depth=100,
        )

        # Assert
        assert len(result) == 2
        assert root.id in result
        assert child1.id in result
        assert child2.id not in result

    @pytest.mark.asyncio
    async def test_max_depth_enforcement_limits_traversal(self, memory_db: Database):
        """Test max_depth parameter limits tree traversal depth."""
        # Arrange - create deep tree: root(0) -> child(1) -> grandchild(2) -> great_grandchild(3)
        from abathur.domain.models import Task

        root = Task(summary="Root",
 prompt="Root description", status=TaskStatus.COMPLETED)
        await memory_db.insert_task(root)

        child = Task(
            summary="Child",
            prompt="Child description",
            parent_task_id=root.id,
            status=TaskStatus.COMPLETED,
        )
        await memory_db.insert_task(child)

        grandchild = Task(
            summary="Grandchild",
            prompt="Grandchild description",
            parent_task_id=child.id,
            status=TaskStatus.COMPLETED,
        )
        await memory_db.insert_task(grandchild)

        great_grandchild = Task(
            summary="Great-grandchild",
            prompt="Great-grandchild description",
            parent_task_id=grandchild.id,
            status=TaskStatus.COMPLETED,
        )
        await memory_db.insert_task(great_grandchild)

        # Act - use max_depth=5 to retrieve all 4 levels
        # Then use max_depth=3 to test limiting (should only get 3 levels)
        result_all = await memory_db.get_task_tree_with_status(
            root_task_ids=[root.id],
            filter_statuses=None,
            max_depth=5,
        )

        # Assert - all 4 tasks returned with max_depth=5
        assert len(result_all) == 4

        # Test limiting with max_depth=3
        # SQL WHERE parent.depth < 3 means depths 0,1,2,3 are generated
        # Then RuntimeError raised when max_observed_depth (3) >= max_depth (3)
        # So this should raise RuntimeError
        with pytest.raises(RuntimeError, match="Tree depth exceeded"):
            await memory_db.get_task_tree_with_status(
                root_task_ids=[root.id],
                filter_statuses=None,
                max_depth=3,
            )

    @pytest.mark.asyncio
    async def test_multiple_root_tasks(self, memory_db: Database):
        """Test traversal with multiple root tasks."""
        # Arrange - create two separate trees
        from abathur.domain.models import Task

        root1 = Task(summary="Root 1",
 prompt="Root 1 description", status=TaskStatus.COMPLETED)
        await memory_db.insert_task(root1)

        child1 = Task(
            summary="Child of root 1",

            prompt="Child of root 1",
            parent_task_id=root1.id,
            status=TaskStatus.COMPLETED,
        )
        await memory_db.insert_task(child1)

        root2 = Task(summary="Root 2",
 prompt="Root 2 description", status=TaskStatus.FAILED)
        await memory_db.insert_task(root2)

        child2 = Task(
            summary="Child of root 2",

            prompt="Child of root 2",
            parent_task_id=root2.id,
            status=TaskStatus.FAILED,
        )
        await memory_db.insert_task(child2)

        # Act
        result = await memory_db.get_task_tree_with_status(
            root_task_ids=[root1.id, root2.id],
            filter_statuses=None,
            max_depth=100,
        )

        # Assert - should return all 4 tasks
        assert len(result) == 4
        assert root1.id in result
        assert child1.id in result
        assert root2.id in result
        assert child2.id in result


# ============================================================================
# check_tree_all_match_status Tests
# ============================================================================


class TestCheckTreeAllMatchStatus:
    """Unit tests for Database.check_tree_all_match_status() method."""

    @pytest.mark.asyncio
    async def test_empty_root_task_ids_raises_valueerror(self, memory_db: Database):
        """Test that empty root_task_ids raises ValueError."""
        # Arrange
        empty_list: list[UUID] = []

        # Act & Assert
        with pytest.raises(ValueError, match="root_task_ids cannot be empty"):
            await memory_db.check_tree_all_match_status(
                root_task_ids=empty_list,
                allowed_statuses=[TaskStatus.COMPLETED],
            )

    @pytest.mark.asyncio
    async def test_empty_allowed_statuses_raises_valueerror(self, memory_db: Database):
        """Test that empty allowed_statuses raises ValueError."""
        # Arrange
        root_id = uuid4()
        empty_statuses: list[TaskStatus] = []

        # Act & Assert
        with pytest.raises(ValueError, match="allowed_statuses cannot be empty"):
            await memory_db.check_tree_all_match_status(
                root_task_ids=[root_id],
                allowed_statuses=empty_statuses,
            )

    @pytest.mark.asyncio
    async def test_all_match_returns_true(self, memory_db: Database):
        """Test returns True when all descendants match allowed_statuses."""
        # Arrange - create tree where all tasks are COMPLETED
        from abathur.domain.models import Task

        root = Task(summary="Root",
 prompt="Root description", status=TaskStatus.COMPLETED)
        await memory_db.insert_task(root)

        child1 = Task(
            summary="Child 1",

            prompt="Child 1",
            parent_task_id=root.id,
            status=TaskStatus.COMPLETED,
        )
        await memory_db.insert_task(child1)

        child2 = Task(
            summary="Child 2",

            prompt="Child 2",
            parent_task_id=root.id,
            status=TaskStatus.COMPLETED,
        )
        await memory_db.insert_task(child2)

        # Act
        result = await memory_db.check_tree_all_match_status(
            root_task_ids=[root.id],
            allowed_statuses=[TaskStatus.COMPLETED],
        )

        # Assert
        assert len(result) == 1
        assert root.id in result
        assert result[root.id] is True

    @pytest.mark.asyncio
    async def test_some_mismatch_returns_false(self, memory_db: Database):
        """Test returns False when some descendants do not match."""
        # Arrange - create tree with mixed statuses
        from abathur.domain.models import Task

        root = Task(summary="Root",
 prompt="Root description", status=TaskStatus.COMPLETED)
        await memory_db.insert_task(root)

        child_completed = Task(
            summary="Child completed",

            prompt="Child completed",
            parent_task_id=root.id,
            status=TaskStatus.COMPLETED,
        )
        await memory_db.insert_task(child_completed)

        child_pending = Task(
            summary="Child pending",

            prompt="Child pending",
            parent_task_id=root.id,
            status=TaskStatus.PENDING,
        )
        await memory_db.insert_task(child_pending)

        # Act
        result = await memory_db.check_tree_all_match_status(
            root_task_ids=[root.id],
            allowed_statuses=[TaskStatus.COMPLETED],
        )

        # Assert
        assert len(result) == 1
        assert root.id in result
        assert result[root.id] is False

    @pytest.mark.asyncio
    async def test_multiple_allowed_statuses_all_match(self, memory_db: Database):
        """Test with multiple allowed statuses where all tasks match."""
        # Arrange
        from abathur.domain.models import Task

        root = Task(summary="Root",
 prompt="Root description", status=TaskStatus.COMPLETED)
        await memory_db.insert_task(root)

        child1 = Task(
            summary="Child 1",

            prompt="Child 1",
            parent_task_id=root.id,
            status=TaskStatus.FAILED,
        )
        await memory_db.insert_task(child1)

        child2 = Task(
            summary="Child 2",

            prompt="Child 2",
            parent_task_id=root.id,
            status=TaskStatus.CANCELLED,
        )
        await memory_db.insert_task(child2)

        # Act
        result = await memory_db.check_tree_all_match_status(
            root_task_ids=[root.id],
            allowed_statuses=[
                TaskStatus.COMPLETED,
                TaskStatus.FAILED,
                TaskStatus.CANCELLED,
            ],
        )

        # Assert
        assert result[root.id] is True

    @pytest.mark.asyncio
    async def test_multiple_root_tasks_separate_results(self, memory_db: Database):
        """Test returns separate result for each root task."""
        # Arrange - two trees: one all-match, one mismatch
        from abathur.domain.models import Task

        root1 = Task(summary="Root 1",
 prompt="Root 1 description", status=TaskStatus.COMPLETED)
        await memory_db.insert_task(root1)

        child1 = Task(
            summary="Child 1",

            prompt="Child 1",
            parent_task_id=root1.id,
            status=TaskStatus.COMPLETED,
        )
        await memory_db.insert_task(child1)

        root2 = Task(summary="Root 2",
 prompt="Root 2 description", status=TaskStatus.COMPLETED)
        await memory_db.insert_task(root2)

        child2 = Task(
            summary="Child 2",

            prompt="Child 2",
            parent_task_id=root2.id,
            status=TaskStatus.PENDING,
        )
        await memory_db.insert_task(child2)

        # Act
        result = await memory_db.check_tree_all_match_status(
            root_task_ids=[root1.id, root2.id],
            allowed_statuses=[TaskStatus.COMPLETED],
        )

        # Assert
        assert len(result) == 2
        assert result[root1.id] is True
        assert result[root2.id] is False

    @pytest.mark.asyncio
    async def test_nonexistent_root_task(self, memory_db: Database):
        """Test behavior with non-existent root task ID."""
        # Arrange
        nonexistent_id = uuid4()

        # Act
        result = await memory_db.check_tree_all_match_status(
            root_task_ids=[nonexistent_id],
            allowed_statuses=[TaskStatus.COMPLETED],
        )

        # Assert - should return False for non-existent task
        assert len(result) == 1
        assert result[nonexistent_id] is False


# ============================================================================
# PruneFilters.recursive Field Tests
# ============================================================================


class TestPruneFiltersRecursiveField:
    """Unit tests for PruneFilters.recursive field validation."""

    def test_recursive_field_defaults_to_false(self):
        """Test recursive field defaults to False."""
        # Arrange
        filters = PruneFilters(
            older_than_days=30,
        )

        # Act & Assert
        assert filters.recursive is False

    def test_recursive_field_accepts_true(self):
        """Test recursive field accepts True value."""
        # Arrange
        filters = PruneFilters(
            older_than_days=30,
            recursive=True,
        )

        # Act & Assert
        assert filters.recursive is True

    def test_recursive_field_accepts_false_explicitly(self):
        """Test recursive field accepts False value explicitly."""
        # Arrange
        filters = PruneFilters(
            older_than_days=30,
            recursive=False,
        )

        # Act & Assert
        assert filters.recursive is False

    def test_recursive_field_serialization(self):
        """Test recursive field is included in serialization."""
        # Arrange
        filters = PruneFilters(
            task_ids=[uuid4()],
            recursive=True,
        )

        # Act
        serialized = filters.model_dump()

        # Assert
        assert "recursive" in serialized
        assert serialized["recursive"] is True

    def test_recursive_with_task_ids_filter(self):
        """Test recursive can be combined with task_ids filter."""
        # Arrange
        task_ids = [uuid4(), uuid4()]
        filters = PruneFilters(
            task_ids=task_ids,
            recursive=True,
        )

        # Act & Assert
        assert filters.task_ids == task_ids
        assert filters.recursive is True

    def test_recursive_with_status_filter(self):
        """Test recursive can be combined with status filter."""
        # Arrange
        filters = PruneFilters(
            older_than_days=7,
            statuses=[TaskStatus.COMPLETED, TaskStatus.FAILED],
            recursive=True,
        )

        # Act & Assert
        assert filters.statuses == [TaskStatus.COMPLETED, TaskStatus.FAILED]
        assert filters.recursive is True

    def test_recursive_with_time_filter(self):
        """Test recursive can be combined with time-based filter."""
        # Arrange
        before = datetime(2025, 1, 1, 0, 0, 0, tzinfo=timezone.utc)
        filters = PruneFilters(
            before_date=before,
            recursive=True,
        )

        # Act & Assert
        assert filters.before_date == before
        assert filters.recursive is True


# ============================================================================
# RecursivePruneResult Model Tests
# ============================================================================


class TestRecursivePruneResult:
    """Unit tests for RecursivePruneResult model validation."""

    def test_recursive_prune_result_with_valid_data(self):
        """Test RecursivePruneResult accepts valid data."""
        # Arrange
        valid_data = {
            "deleted_tasks": 10,
            "deleted_dependencies": 0,
            "dry_run": False,
            "breakdown_by_status": {
                "completed": 7,
                "failed": 2,
                "cancelled": 1,
            },
            "tree_depth": 3,
            "deleted_by_depth": {
                0: 1,
                1: 3,
                2: 4,
                3: 2,
            },
            "trees_deleted": 2,
        }

        # Act
        result = RecursivePruneResult(**valid_data)

        # Assert
        assert result.deleted_tasks == 10
        assert result.tree_depth == 3
        assert result.trees_deleted == 2
        assert result.deleted_by_depth == {0: 1, 1: 3, 2: 4, 3: 2}

    def test_tree_depth_validation_accepts_zero(self):
        """Test tree_depth accepts 0 (single root task)."""
        # Arrange
        data = {
            "deleted_tasks": 1,
            "deleted_dependencies": 0,
            "dry_run": False,
            "breakdown_by_status": {"completed": 1},
            "tree_depth": 0,
            "deleted_by_depth": {0: 1},
            "trees_deleted": 1,
        }

        # Act
        result = RecursivePruneResult(**data)

        # Assert
        assert result.tree_depth == 0

    def test_tree_depth_validation_rejects_negative(self):
        """Test tree_depth validation rejects negative values."""
        # Arrange
        invalid_data = {
            "deleted_tasks": 5,
            "deleted_dependencies": 0,
            "dry_run": False,
            "breakdown_by_status": {"completed": 5},
            "tree_depth": -1,
            "deleted_by_depth": {0: 5},
            "trees_deleted": 1,
        }

        # Act & Assert
        with pytest.raises(ValidationError) as exc_info:
            RecursivePruneResult(**invalid_data)

        assert "greater_than_equal" in str(exc_info.value).lower()

    def test_trees_deleted_validation_accepts_zero(self):
        """Test trees_deleted accepts 0 (no trees matched criteria)."""
        # Arrange
        data = {
            "deleted_tasks": 0,
            "deleted_dependencies": 0,
            "dry_run": False,
            "breakdown_by_status": {},
            "tree_depth": 0,
            "deleted_by_depth": {},
            "trees_deleted": 0,
        }

        # Act
        result = RecursivePruneResult(**data)

        # Assert
        assert result.trees_deleted == 0

    def test_trees_deleted_validation_rejects_negative(self):
        """Test trees_deleted validation rejects negative values."""
        # Arrange
        invalid_data = {
            "deleted_tasks": 0,
            "deleted_dependencies": 0,
            "dry_run": False,
            "breakdown_by_status": {},
            "tree_depth": 0,
            "deleted_by_depth": {},
            "trees_deleted": -1,
        }

        # Act & Assert
        with pytest.raises(ValidationError) as exc_info:
            RecursivePruneResult(**invalid_data)

        assert "greater_than_equal" in str(exc_info.value).lower()

    def test_deleted_by_depth_empty_dict_valid(self):
        """Test deleted_by_depth accepts empty dict."""
        # Arrange
        data = {
            "deleted_tasks": 0,
            "deleted_dependencies": 0,
            "dry_run": False,
            "breakdown_by_status": {},
            "tree_depth": 0,
            "deleted_by_depth": {},
            "trees_deleted": 0,
        }

        # Act
        result = RecursivePruneResult(**data)

        # Assert
        assert result.deleted_by_depth == {}

    def test_deleted_by_depth_multiple_levels(self):
        """Test deleted_by_depth with multiple depth levels."""
        # Arrange
        data = {
            "deleted_tasks": 15,
            "deleted_dependencies": 0,
            "dry_run": False,
            "breakdown_by_status": {"completed": 15},
            "tree_depth": 5,
            "deleted_by_depth": {
                0: 1,
                1: 2,
                2: 4,
                3: 5,
                4: 2,
                5: 1,
            },
            "trees_deleted": 1,
        }

        # Act
        result = RecursivePruneResult(**data)

        # Assert
        assert result.deleted_by_depth[0] == 1
        assert result.deleted_by_depth[5] == 1
        assert sum(result.deleted_by_depth.values()) == 15

    def test_serialization_includes_all_fields(self):
        """Test RecursivePruneResult serializes with all fields."""
        # Arrange
        result = RecursivePruneResult(
            deleted_tasks=10,
            deleted_dependencies=0,
            dry_run=False,
            breakdown_by_status={"completed": 10},
            tree_depth=2,
            deleted_by_depth={0: 1, 1: 4, 2: 5},
            trees_deleted=1,
        )

        # Act
        serialized = result.model_dump()

        # Assert
        assert "deleted_tasks" in serialized
        assert "breakdown_by_status" in serialized
        assert "tree_depth" in serialized
        assert "deleted_by_depth" in serialized
        assert "trees_deleted" in serialized
        assert serialized["tree_depth"] == 2
        assert serialized["trees_deleted"] == 1

    def test_inherits_from_prune_result(self):
        """Test RecursivePruneResult inherits fields from PruneResult."""
        # Arrange
        from abathur.infrastructure.database import PruneResult

        result = RecursivePruneResult(
            deleted_tasks=5,
            deleted_dependencies=0,
            dry_run=False,
            breakdown_by_status={"completed": 5},
            tree_depth=1,
            deleted_by_depth={0: 1, 1: 4},
            trees_deleted=1,
        )

        # Act & Assert
        assert isinstance(result, PruneResult)
        assert hasattr(result, "deleted_tasks")
        assert hasattr(result, "breakdown_by_status")
        assert hasattr(result, "tree_depth")
        assert hasattr(result, "trees_deleted")
