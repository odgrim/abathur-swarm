"""Unit tests for ViewMode organization strategies.

Tests different view mode strategies for organizing tasks:
- Tree view (hierarchical by parent_task_id)
- Dependency view (by prerequisites)
- Timeline view (chronological)
- Feature branch view (grouped by feature_branch)
- Flat list view (priority-sorted)
"""

import pytest
from datetime import datetime, timezone, timedelta
from uuid import uuid4

from abathur.domain.models import Task, TaskStatus, TaskSource


# Mock ViewMode strategies (implementation TBD)
class TreeViewMode:
    """Hierarchical view organized by parent_task_id."""

    @staticmethod
    def organize(tasks: list[Task]) -> list[Task]:
        """Group tasks hierarchically by parent_task_id and dependency_depth."""
        # Sort by dependency_depth first, then by calculated_priority within each level
        return sorted(
            tasks,
            key=lambda t: (t.dependency_depth, -t.calculated_priority),
        )


class DependencyViewMode:
    """Dependency-focused view organized by prerequisites."""

    @staticmethod
    def organize(tasks: list[Task]) -> list[Task]:
        """Organize by prerequisite relationships (topological sort)."""
        # Tasks with no dependencies first, then ordered by dependency count
        return sorted(
            tasks,
            key=lambda t: (len(t.dependencies), -t.calculated_priority),
        )


class TimelineViewMode:
    """Chronological view sorted by submission time."""

    @staticmethod
    def organize(tasks: list[Task]) -> list[Task]:
        """Sort by submitted_at timestamp (newest first)."""
        return sorted(tasks, key=lambda t: t.submitted_at, reverse=True)


class FeatureBranchViewMode:
    """Feature branch view grouped by feature_branch."""

    @staticmethod
    def organize(tasks: list[Task]) -> list[Task]:
        """Group by feature_branch, then by priority within each branch."""
        # Tasks without feature_branch come first
        def sort_key(t):
            branch = t.feature_branch or ""
            return (branch, -t.calculated_priority)

        return sorted(tasks, key=sort_key)


class FlatListViewMode:
    """Flat list view sorted by calculated_priority."""

    @staticmethod
    def organize(tasks: list[Task]) -> list[Task]:
        """Sort by calculated_priority descending."""
        return sorted(tasks, key=lambda t: -t.calculated_priority)


class TestTreeViewMode:
    """Test suite for hierarchical tree view mode."""

    @pytest.fixture
    def sample_tasks(self):
        """Create tasks with hierarchical structure."""
        parent_id = uuid4()

        return [
            Task(
                id=parent_id,
                prompt="Parent task",
                summary="Parent task",
                agent_type="test",
                status=TaskStatus.COMPLETED,
                calculated_priority=10.0,
                dependency_depth=0,
                submitted_at=datetime.now(timezone.utc),
                source=TaskSource.HUMAN,
                parent_task_id=None,
            ),
            Task(
                id=uuid4(),
                prompt="Child task 1",
                summary="Child task 1",
                agent_type="test",
                status=TaskStatus.RUNNING,
                calculated_priority=8.0,
                dependency_depth=1,
                submitted_at=datetime.now(timezone.utc),
                source=TaskSource.HUMAN,
                parent_task_id=parent_id,
            ),
            Task(
                id=uuid4(),
                prompt="Child task 2",
                summary="Child task 2",
                agent_type="test",
                status=TaskStatus.PENDING,
                calculated_priority=9.0,
                dependency_depth=1,
                submitted_at=datetime.now(timezone.utc),
                source=TaskSource.HUMAN,
                parent_task_id=parent_id,
            ),
        ]

    def test_organizes_by_dependency_depth(self, sample_tasks):
        """Test tasks grouped by dependency_depth."""
        # Act
        organized = TreeViewMode.organize(sample_tasks)

        # Assert - parent (depth 0) comes first
        assert organized[0].dependency_depth == 0
        assert organized[1].dependency_depth == 1
        assert organized[2].dependency_depth == 1

    def test_sorts_by_priority_within_same_depth(self, sample_tasks):
        """Test tasks at same depth sorted by priority."""
        # Act
        organized = TreeViewMode.organize(sample_tasks)

        # Assert - within depth 1, higher priority (9.0) comes before lower (8.0)
        depth_1_tasks = [t for t in organized if t.dependency_depth == 1]
        assert depth_1_tasks[0].calculated_priority == 9.0
        assert depth_1_tasks[1].calculated_priority == 8.0

    def test_preserves_all_tasks(self, sample_tasks):
        """Test all tasks preserved after organization."""
        # Act
        organized = TreeViewMode.organize(sample_tasks)

        # Assert
        assert len(organized) == len(sample_tasks)


class TestDependencyViewMode:
    """Test suite for dependency-focused view mode."""

    @pytest.fixture
    def sample_tasks(self):
        """Create tasks with varied dependency counts."""
        task1_id = uuid4()
        task2_id = uuid4()

        return [
            Task(
                id=uuid4(),
                prompt="Dependent task",
                summary="Dependent task",
                agent_type="test",
                status=TaskStatus.PENDING,
                calculated_priority=5.0,
                dependency_depth=2,
                dependencies=[task1_id, task2_id],
                submitted_at=datetime.now(timezone.utc),
                source=TaskSource.HUMAN,
            ),
            Task(
                id=task1_id,
                prompt="Independent task 1",
                summary="Independent task 1",
                agent_type="test",
                status=TaskStatus.COMPLETED,
                calculated_priority=10.0,
                dependency_depth=0,
                dependencies=[],
                submitted_at=datetime.now(timezone.utc),
                source=TaskSource.HUMAN,
            ),
            Task(
                id=task2_id,
                prompt="Independent task 2",
                summary="Independent task 2",
                agent_type="test",
                status=TaskStatus.RUNNING,
                calculated_priority=8.0,
                dependency_depth=0,
                dependencies=[],
                submitted_at=datetime.now(timezone.utc),
                source=TaskSource.HUMAN,
            ),
        ]

    def test_organizes_independent_tasks_first(self, sample_tasks):
        """Test tasks with no dependencies come first."""
        # Act
        organized = DependencyViewMode.organize(sample_tasks)

        # Assert - first two tasks have no dependencies
        assert len(organized[0].dependencies) == 0
        assert len(organized[1].dependencies) == 0
        assert len(organized[2].dependencies) == 2

    def test_sorts_by_priority_within_same_dependency_count(self, sample_tasks):
        """Test tasks with same dependency count sorted by priority."""
        # Act
        organized = DependencyViewMode.organize(sample_tasks)

        # Assert - among independent tasks, higher priority first
        independent_tasks = [t for t in organized if len(t.dependencies) == 0]
        assert independent_tasks[0].calculated_priority >= independent_tasks[1].calculated_priority


class TestTimelineViewMode:
    """Test suite for chronological timeline view mode."""

    @pytest.fixture
    def sample_tasks(self):
        """Create tasks with different submission times."""
        now = datetime.now(timezone.utc)

        return [
            Task(
                id=uuid4(),
                prompt="Oldest task",
                summary="Oldest task",
                agent_type="test",
                status=TaskStatus.COMPLETED,
                calculated_priority=5.0,
                dependency_depth=0,
                submitted_at=now - timedelta(hours=2),
                source=TaskSource.HUMAN,
            ),
            Task(
                id=uuid4(),
                prompt="Newest task",
                summary="Newest task",
                agent_type="test",
                status=TaskStatus.PENDING,
                calculated_priority=8.0,
                dependency_depth=0,
                submitted_at=now,
                source=TaskSource.HUMAN,
            ),
            Task(
                id=uuid4(),
                prompt="Middle task",
                summary="Middle task",
                agent_type="test",
                status=TaskStatus.RUNNING,
                calculated_priority=7.0,
                dependency_depth=0,
                submitted_at=now - timedelta(hours=1),
                source=TaskSource.HUMAN,
            ),
        ]

    def test_organizes_by_submission_time_newest_first(self, sample_tasks):
        """Test tasks sorted by submitted_at descending."""
        # Act
        organized = TimelineViewMode.organize(sample_tasks)

        # Assert - newest first
        assert organized[0].summary == "Newest task"
        assert organized[1].summary == "Middle task"
        assert organized[2].summary == "Oldest task"

    def test_respects_chronological_order(self, sample_tasks):
        """Test chronological ordering is correct."""
        # Act
        organized = TimelineViewMode.organize(sample_tasks)

        # Assert - each task submitted after the next
        for i in range(len(organized) - 1):
            assert organized[i].submitted_at >= organized[i + 1].submitted_at


class TestFeatureBranchViewMode:
    """Test suite for feature branch view mode."""

    @pytest.fixture
    def sample_tasks(self):
        """Create tasks with different feature branches."""
        return [
            Task(
                id=uuid4(),
                prompt="Feature A task 1",
                summary="Feature A task 1",
                agent_type="test",
                status=TaskStatus.PENDING,
                calculated_priority=8.0,
                dependency_depth=0,
                submitted_at=datetime.now(timezone.utc),
                source=TaskSource.HUMAN,
                feature_branch="feature/a",
            ),
            Task(
                id=uuid4(),
                prompt="Feature B task 1",
                summary="Feature B task 1",
                agent_type="test",
                status=TaskStatus.RUNNING,
                calculated_priority=10.0,
                dependency_depth=0,
                submitted_at=datetime.now(timezone.utc),
                source=TaskSource.HUMAN,
                feature_branch="feature/b",
            ),
            Task(
                id=uuid4(),
                prompt="Feature A task 2",
                summary="Feature A task 2",
                agent_type="test",
                status=TaskStatus.COMPLETED,
                calculated_priority=5.0,
                dependency_depth=0,
                submitted_at=datetime.now(timezone.utc),
                source=TaskSource.HUMAN,
                feature_branch="feature/a",
            ),
            Task(
                id=uuid4(),
                prompt="No branch task",
                summary="No branch task",
                agent_type="test",
                status=TaskStatus.PENDING,
                calculated_priority=7.0,
                dependency_depth=0,
                submitted_at=datetime.now(timezone.utc),
                source=TaskSource.HUMAN,
                feature_branch=None,
            ),
        ]

    def test_groups_by_feature_branch(self, sample_tasks):
        """Test tasks grouped by feature_branch value."""
        # Act
        organized = FeatureBranchViewMode.organize(sample_tasks)

        # Assert - tasks from same branch are adjacent
        # Find feature/a tasks
        feature_a_indices = [
            i for i, t in enumerate(organized) if t.feature_branch == "feature/a"
        ]

        # Check they are consecutive
        assert feature_a_indices[1] - feature_a_indices[0] == 1

    def test_sorts_by_priority_within_same_branch(self, sample_tasks):
        """Test tasks within same branch sorted by priority."""
        # Act
        organized = FeatureBranchViewMode.organize(sample_tasks)

        # Get feature/a tasks
        feature_a_tasks = [t for t in organized if t.feature_branch == "feature/a"]

        # Assert - higher priority first
        assert feature_a_tasks[0].calculated_priority >= feature_a_tasks[1].calculated_priority

    def test_places_no_branch_tasks_first(self, sample_tasks):
        """Test tasks with no feature_branch come first."""
        # Act
        organized = FeatureBranchViewMode.organize(sample_tasks)

        # Assert - first task has no feature_branch
        assert organized[0].feature_branch is None or organized[0].feature_branch == ""


class TestFlatListViewMode:
    """Test suite for flat priority-sorted list view mode."""

    @pytest.fixture
    def sample_tasks(self):
        """Create tasks with varied priorities."""
        return [
            Task(
                id=uuid4(),
                prompt="Low priority",
                summary="Low priority",
                agent_type="test",
                status=TaskStatus.PENDING,
                calculated_priority=3.0,
                dependency_depth=0,
                submitted_at=datetime.now(timezone.utc),
                source=TaskSource.HUMAN,
            ),
            Task(
                id=uuid4(),
                prompt="High priority",
                summary="High priority",
                agent_type="test",
                status=TaskStatus.PENDING,
                calculated_priority=10.0,
                dependency_depth=0,
                submitted_at=datetime.now(timezone.utc),
                source=TaskSource.HUMAN,
            ),
            Task(
                id=uuid4(),
                prompt="Medium priority",
                summary="Medium priority",
                agent_type="test",
                status=TaskStatus.RUNNING,
                calculated_priority=7.0,
                dependency_depth=0,
                submitted_at=datetime.now(timezone.utc),
                source=TaskSource.HUMAN,
            ),
        ]

    def test_sorts_by_priority_descending(self, sample_tasks):
        """Test tasks sorted by calculated_priority highest first."""
        # Act
        organized = FlatListViewMode.organize(sample_tasks)

        # Assert - highest priority first
        assert organized[0].calculated_priority == 10.0
        assert organized[1].calculated_priority == 7.0
        assert organized[2].calculated_priority == 3.0

    def test_ignores_hierarchical_structure(self, sample_tasks):
        """Test flat view ignores parent_task_id and dependencies."""
        # Modify tasks to have hierarchical structure
        sample_tasks[1].parent_task_id = sample_tasks[0].id
        sample_tasks[2].dependencies = [sample_tasks[0].id]

        # Act
        organized = FlatListViewMode.organize(sample_tasks)

        # Assert - still sorted by priority, not hierarchy
        assert organized[0].calculated_priority == 10.0


class TestViewModeSwitching:
    """Test suite for view mode transitions."""

    @pytest.fixture
    def sample_tasks(self):
        """Create sample tasks for testing transitions."""
        return [
            Task(
                id=uuid4(),
                prompt="Task 1",
                summary="Task 1",
                agent_type="test",
                status=TaskStatus.PENDING,
                calculated_priority=5.0,
                dependency_depth=0,
                submitted_at=datetime.now(timezone.utc),
                source=TaskSource.HUMAN,
            ),
            Task(
                id=uuid4(),
                prompt="Task 2",
                summary="Task 2",
                agent_type="test",
                status=TaskStatus.RUNNING,
                calculated_priority=8.0,
                dependency_depth=1,
                submitted_at=datetime.now(timezone.utc),
                source=TaskSource.HUMAN,
            ),
        ]

    def test_switching_preserves_all_tasks(self, sample_tasks):
        """Test switching view modes preserves all tasks."""
        # Act - organize in different modes
        tree_view = TreeViewMode.organize(sample_tasks)
        flat_view = FlatListViewMode.organize(sample_tasks)
        timeline_view = TimelineViewMode.organize(sample_tasks)

        # Assert - all have same count
        assert len(tree_view) == len(sample_tasks)
        assert len(flat_view) == len(sample_tasks)
        assert len(timeline_view) == len(sample_tasks)

    def test_switching_changes_order(self, sample_tasks):
        """Test different view modes produce different orderings."""
        # Act
        tree_view = TreeViewMode.organize(sample_tasks)
        flat_view = FlatListViewMode.organize(sample_tasks)

        # Assert - orderings differ (flat view sorts by priority)
        tree_order = [t.id for t in tree_view]
        flat_order = [t.id for t in flat_view]

        # They should differ (flat view puts higher priority first)
        assert tree_order != flat_order
