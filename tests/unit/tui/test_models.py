"""Unit tests for TUI data models."""

from datetime import datetime, timedelta, timezone
from uuid import UUID, uuid4

import pytest

from abathur.domain.models import Task, TaskStatus
from abathur.tui.models import (
    CachedTaskData,
    FeatureBranchSummary,
    FilterState,
    NavigationState,
    TreeLayout,
    TreeNode,
    ViewMode,
)


class TestViewMode:
    """Tests for ViewMode enum."""

    def test_view_mode_values(self) -> None:
        """Test that all view modes have correct string values."""
        assert ViewMode.TREE.value == "tree"
        assert ViewMode.DEPENDENCY.value == "dependency"
        assert ViewMode.TIMELINE.value == "timeline"
        assert ViewMode.FEATURE_BRANCH.value == "feature_branch"
        assert ViewMode.FLAT_LIST.value == "flat_list"


class TestFilterState:
    """Tests for FilterState model."""

    def test_is_active_with_no_filters(self) -> None:
        """Test is_active returns False when no filters are set."""
        filter_state = FilterState()
        assert not filter_state.is_active()

    def test_is_active_with_status_filter(self) -> None:
        """Test is_active returns True when status filter is set."""
        filter_state = FilterState(status_filter={TaskStatus.PENDING})
        assert filter_state.is_active()

    def test_is_active_with_agent_type_filter(self) -> None:
        """Test is_active returns True when agent type filter is set."""
        filter_state = FilterState(agent_type_filter="python-specialist")
        assert filter_state.is_active()

    def test_is_active_with_feature_branch_filter(self) -> None:
        """Test is_active returns True when feature branch filter is set."""
        filter_state = FilterState(feature_branch_filter="feature/tui")
        assert filter_state.is_active()

    def test_is_active_with_text_search(self) -> None:
        """Test is_active returns True when text search is set."""
        filter_state = FilterState(text_search="bug fix")
        assert filter_state.is_active()

    def test_is_active_with_multiple_filters(self) -> None:
        """Test is_active returns True when multiple filters are set."""
        filter_state = FilterState(
            status_filter={TaskStatus.RUNNING},
            agent_type_filter="python-specialist",
        )
        assert filter_state.is_active()

    def test_matches_no_filters_always_true(self) -> None:
        """Test matches returns True for any task when no filters are set."""
        filter_state = FilterState()
        task = Task(prompt="Test task")
        assert filter_state.matches(task)

    def test_matches_status_filter_match(self) -> None:
        """Test matches returns True when task status matches filter."""
        filter_state = FilterState(status_filter={TaskStatus.PENDING})
        task = Task(prompt="Test task", status=TaskStatus.PENDING)
        assert filter_state.matches(task)

    def test_matches_status_filter_no_match(self) -> None:
        """Test matches returns False when task status doesn't match filter."""
        filter_state = FilterState(status_filter={TaskStatus.RUNNING})
        task = Task(prompt="Test task", status=TaskStatus.PENDING)
        assert not filter_state.matches(task)

    def test_matches_status_filter_multiple_values(self) -> None:
        """Test matches handles multiple status values in filter."""
        filter_state = FilterState(
            status_filter={TaskStatus.PENDING, TaskStatus.RUNNING}
        )
        task1 = Task(prompt="Test 1", status=TaskStatus.PENDING)
        task2 = Task(prompt="Test 2", status=TaskStatus.RUNNING)
        task3 = Task(prompt="Test 3", status=TaskStatus.COMPLETED)
        assert filter_state.matches(task1)
        assert filter_state.matches(task2)
        assert not filter_state.matches(task3)

    def test_matches_agent_type_filter_match(self) -> None:
        """Test matches returns True when agent type matches filter."""
        filter_state = FilterState(agent_type_filter="python-specialist")
        task = Task(prompt="Test task", agent_type="python-specialist")
        assert filter_state.matches(task)

    def test_matches_agent_type_filter_no_match(self) -> None:
        """Test matches returns False when agent type doesn't match filter."""
        filter_state = FilterState(agent_type_filter="python-specialist")
        task = Task(prompt="Test task", agent_type="general-purpose")
        assert not filter_state.matches(task)

    def test_matches_feature_branch_filter_match(self) -> None:
        """Test matches returns True when feature branch matches filter."""
        filter_state = FilterState(feature_branch_filter="feature/tui")
        task = Task(prompt="Test task", feature_branch="feature/tui")
        assert filter_state.matches(task)

    def test_matches_feature_branch_filter_no_match(self) -> None:
        """Test matches returns False when feature branch doesn't match."""
        filter_state = FilterState(feature_branch_filter="feature/tui")
        task = Task(prompt="Test task", feature_branch="feature/api")
        assert not filter_state.matches(task)

    def test_matches_feature_branch_filter_none_value(self) -> None:
        """Test matches handles None feature_branch in task."""
        filter_state = FilterState(feature_branch_filter="feature/tui")
        task = Task(prompt="Test task", feature_branch=None)
        assert not filter_state.matches(task)

    def test_matches_text_search_in_summary(self) -> None:
        """Test matches finds text in task summary (case-insensitive)."""
        filter_state = FilterState(text_search="BUG")
        task = Task(prompt="Fix issue", summary="Fix bug in parser")
        assert filter_state.matches(task)

    def test_matches_text_search_in_prompt(self) -> None:
        """Test matches finds text in task prompt (case-insensitive)."""
        filter_state = FilterState(text_search="parser")
        task = Task(prompt="Fix parser bug", summary="Fix issue")
        assert filter_state.matches(task)

    def test_matches_text_search_no_match(self) -> None:
        """Test matches returns False when text not found."""
        filter_state = FilterState(text_search="database")
        task = Task(prompt="Fix parser", summary="Fix bug")
        assert not filter_state.matches(task)

    def test_matches_text_search_case_insensitive(self) -> None:
        """Test matches performs case-insensitive text search."""
        filter_state = FilterState(text_search="BUG")
        task = Task(prompt="fix bug", summary="Bug fix")
        assert filter_state.matches(task)

    def test_matches_all_filters_and_logic(self) -> None:
        """Test matches uses AND logic for multiple filters."""
        filter_state = FilterState(
            status_filter={TaskStatus.PENDING},
            agent_type_filter="python-specialist",
            text_search="bug",
        )
        # Task matches all filters
        task1 = Task(
            prompt="Fix bug in parser",
            status=TaskStatus.PENDING,
            agent_type="python-specialist",
        )
        assert filter_state.matches(task1)

        # Task fails status filter
        task2 = Task(
            prompt="Fix bug in parser",
            status=TaskStatus.RUNNING,
            agent_type="python-specialist",
        )
        assert not filter_state.matches(task2)

        # Task fails agent type filter
        task3 = Task(
            prompt="Fix bug in parser",
            status=TaskStatus.PENDING,
            agent_type="general-purpose",
        )
        assert not filter_state.matches(task3)

        # Task fails text search filter
        task4 = Task(
            prompt="Add feature",
            status=TaskStatus.PENDING,
            agent_type="python-specialist",
        )
        assert not filter_state.matches(task4)


class TestNavigationState:
    """Tests for NavigationState model."""

    def test_default_values(self) -> None:
        """Test NavigationState has correct default values."""
        nav_state = NavigationState()
        assert nav_state.selected_task_id is None
        assert nav_state.expanded_nodes == set()
        assert nav_state.scroll_position == 0
        assert nav_state.focus_widget == "tree"

    def test_custom_values(self) -> None:
        """Test NavigationState accepts custom values."""
        task_id = uuid4()
        expanded = {uuid4(), uuid4()}
        nav_state = NavigationState(
            selected_task_id=task_id,
            expanded_nodes=expanded,
            scroll_position=10,
            focus_widget="filter_panel",
        )
        assert nav_state.selected_task_id == task_id
        assert nav_state.expanded_nodes == expanded
        assert nav_state.scroll_position == 10
        assert nav_state.focus_widget == "filter_panel"

    def test_negative_scroll_position_rejected(self) -> None:
        """Test NavigationState rejects negative scroll position."""
        with pytest.raises(ValueError):
            NavigationState(scroll_position=-1)


class TestCachedTaskData:
    """Tests for CachedTaskData model."""

    def test_is_expired_not_expired(self) -> None:
        """Test is_expired returns False for valid cache."""
        now = datetime.now(timezone.utc)
        cache = CachedTaskData(
            tasks=[],
            dependency_graph={},
            cached_at=now,
            ttl_seconds=2.0,
        )
        assert not cache.is_expired()

    def test_is_expired_just_expired(self) -> None:
        """Test is_expired returns True when cache just expired."""
        past = datetime.now(timezone.utc) - timedelta(seconds=2.1)
        cache = CachedTaskData(
            tasks=[],
            dependency_graph={},
            cached_at=past,
            ttl_seconds=2.0,
        )
        assert cache.is_expired()

    def test_is_expired_exact_ttl(self) -> None:
        """Test is_expired returns True when cache is exactly at TTL."""
        past = datetime.now(timezone.utc) - timedelta(seconds=2.0)
        cache = CachedTaskData(
            tasks=[],
            dependency_graph={},
            cached_at=past,
            ttl_seconds=2.0,
        )
        assert cache.is_expired()

    def test_is_expired_custom_ttl(self) -> None:
        """Test is_expired works with custom TTL values."""
        past = datetime.now(timezone.utc) - timedelta(seconds=5.5)
        cache = CachedTaskData(
            tasks=[],
            dependency_graph={},
            cached_at=past,
            ttl_seconds=5.0,
        )
        assert cache.is_expired()

    def test_cache_with_tasks(self) -> None:
        """Test CachedTaskData stores tasks correctly."""
        task1 = Task(prompt="Task 1")
        task2 = Task(prompt="Task 2")
        cache = CachedTaskData(
            tasks=[task1, task2],
            dependency_graph={task1.id: [], task2.id: [task1.id]},
            cached_at=datetime.now(timezone.utc),
        )
        assert len(cache.tasks) == 2
        assert cache.dependency_graph[task2.id] == [task1.id]


class TestTreeNode:
    """Tests for TreeNode model."""

    def test_tree_node_creation(self) -> None:
        """Test TreeNode can be created with required fields."""
        task = Task(prompt="Test task")
        node = TreeNode(
            task_id=task.id,
            task=task,
            level=0,
            position=0,
        )
        assert node.task_id == task.id
        assert node.task == task
        assert node.children == []
        assert node.level == 0
        assert node.is_expanded is True
        assert node.position == 0

    def test_tree_node_with_children(self) -> None:
        """Test TreeNode can store child task IDs."""
        task = Task(prompt="Parent task")
        child1_id = uuid4()
        child2_id = uuid4()
        node = TreeNode(
            task_id=task.id,
            task=task,
            children=[child1_id, child2_id],
            level=0,
            position=0,
        )
        assert len(node.children) == 2
        assert child1_id in node.children
        assert child2_id in node.children

    def test_tree_node_collapsed(self) -> None:
        """Test TreeNode can be collapsed."""
        task = Task(prompt="Test task")
        node = TreeNode(
            task_id=task.id,
            task=task,
            level=0,
            position=0,
            is_expanded=False,
        )
        assert node.is_expanded is False


class TestTreeLayout:
    """Tests for TreeLayout model."""

    def test_empty_tree_layout(self) -> None:
        """Test TreeLayout can be created empty."""
        layout = TreeLayout(
            nodes={},
            root_nodes=[],
            max_depth=0,
            total_nodes=0,
        )
        assert layout.nodes == {}
        assert layout.root_nodes == []
        assert layout.max_depth == 0
        assert layout.total_nodes == 0

    def test_get_visible_nodes_all_expanded(self) -> None:
        """Test get_visible_nodes returns all nodes when fully expanded."""
        # Create tree: root -> child1 -> grandchild
        root_task = Task(prompt="Root", parent_task_id=None)
        child_task = Task(prompt="Child", parent_task_id=root_task.id)
        grandchild_task = Task(prompt="Grandchild", parent_task_id=child_task.id)

        root_node = TreeNode(
            task_id=root_task.id,
            task=root_task,
            children=[child_task.id],
            level=0,
            position=0,
        )
        child_node = TreeNode(
            task_id=child_task.id,
            task=child_task,
            children=[grandchild_task.id],
            level=1,
            position=0,
        )
        grandchild_node = TreeNode(
            task_id=grandchild_task.id,
            task=grandchild_task,
            children=[],
            level=2,
            position=0,
        )

        layout = TreeLayout(
            nodes={
                root_task.id: root_node,
                child_task.id: child_node,
                grandchild_task.id: grandchild_node,
            },
            root_nodes=[root_task.id],
            max_depth=2,
            total_nodes=3,
        )

        # All nodes expanded - should see all nodes
        expanded = {root_task.id, child_task.id, grandchild_task.id}
        visible = layout.get_visible_nodes(expanded)
        assert len(visible) == 3

    def test_get_visible_nodes_root_collapsed(self) -> None:
        """Test get_visible_nodes hides children when root is collapsed."""
        # Create tree: root -> child
        root_task = Task(prompt="Root", parent_task_id=None)
        child_task = Task(prompt="Child", parent_task_id=root_task.id)

        root_node = TreeNode(
            task_id=root_task.id,
            task=root_task,
            children=[child_task.id],
            level=0,
            position=0,
        )
        child_node = TreeNode(
            task_id=child_task.id,
            task=child_task,
            children=[],
            level=1,
            position=0,
        )

        layout = TreeLayout(
            nodes={
                root_task.id: root_node,
                child_task.id: child_node,
            },
            root_nodes=[root_task.id],
            max_depth=1,
            total_nodes=2,
        )

        # Root collapsed - should only see root
        expanded: set[UUID] = set()
        visible = layout.get_visible_nodes(expanded)
        assert len(visible) == 1
        assert visible[0].task_id == root_task.id

    def test_get_visible_nodes_partial_collapse(self) -> None:
        """Test get_visible_nodes with partially collapsed tree."""
        # Create tree: root -> child1, child2 (child1 -> grandchild)
        root_task = Task(prompt="Root", parent_task_id=None)
        child1_task = Task(prompt="Child1", parent_task_id=root_task.id)
        child2_task = Task(prompt="Child2", parent_task_id=root_task.id)
        grandchild_task = Task(prompt="Grandchild", parent_task_id=child1_task.id)

        root_node = TreeNode(
            task_id=root_task.id,
            task=root_task,
            children=[child1_task.id, child2_task.id],
            level=0,
            position=0,
        )
        child1_node = TreeNode(
            task_id=child1_task.id,
            task=child1_task,
            children=[grandchild_task.id],
            level=1,
            position=0,
        )
        child2_node = TreeNode(
            task_id=child2_task.id,
            task=child2_task,
            children=[],
            level=1,
            position=1,
        )
        grandchild_node = TreeNode(
            task_id=grandchild_task.id,
            task=grandchild_task,
            children=[],
            level=2,
            position=0,
        )

        layout = TreeLayout(
            nodes={
                root_task.id: root_node,
                child1_task.id: child1_node,
                child2_task.id: child2_node,
                grandchild_task.id: grandchild_node,
            },
            root_nodes=[root_task.id],
            max_depth=2,
            total_nodes=4,
        )

        # Root expanded, child1 collapsed - should see root, child1, child2
        expanded = {root_task.id}
        visible = layout.get_visible_nodes(expanded)
        assert len(visible) == 3
        visible_ids = {node.task_id for node in visible}
        assert root_task.id in visible_ids
        assert child1_task.id in visible_ids
        assert child2_task.id in visible_ids
        assert grandchild_task.id not in visible_ids

    def test_find_node_path_root_node(self) -> None:
        """Test find_node_path returns single-element path for root."""
        root_task = Task(prompt="Root", parent_task_id=None)
        root_node = TreeNode(
            task_id=root_task.id,
            task=root_task,
            children=[],
            level=0,
            position=0,
        )

        layout = TreeLayout(
            nodes={root_task.id: root_node},
            root_nodes=[root_task.id],
            max_depth=0,
            total_nodes=1,
        )

        path = layout.find_node_path(root_task.id)
        assert path == [root_task.id]

    def test_find_node_path_nested_node(self) -> None:
        """Test find_node_path returns full path for nested node."""
        # Create tree: root -> child -> grandchild
        root_task = Task(prompt="Root", parent_task_id=None)
        child_task = Task(prompt="Child", parent_task_id=root_task.id)
        grandchild_task = Task(prompt="Grandchild", parent_task_id=child_task.id)

        root_node = TreeNode(
            task_id=root_task.id,
            task=root_task,
            children=[child_task.id],
            level=0,
            position=0,
        )
        child_node = TreeNode(
            task_id=child_task.id,
            task=child_task,
            children=[grandchild_task.id],
            level=1,
            position=0,
        )
        grandchild_node = TreeNode(
            task_id=grandchild_task.id,
            task=grandchild_task,
            children=[],
            level=2,
            position=0,
        )

        layout = TreeLayout(
            nodes={
                root_task.id: root_node,
                child_task.id: child_node,
                grandchild_task.id: grandchild_node,
            },
            root_nodes=[root_task.id],
            max_depth=2,
            total_nodes=3,
        )

        path = layout.find_node_path(grandchild_task.id)
        assert path == [root_task.id, child_task.id, grandchild_task.id]

    def test_find_node_path_not_found(self) -> None:
        """Test find_node_path returns empty list for unknown task."""
        layout = TreeLayout(
            nodes={},
            root_nodes=[],
            max_depth=0,
            total_nodes=0,
        )

        path = layout.find_node_path(uuid4())
        assert path == []


class TestFeatureBranchSummary:
    """Tests for FeatureBranchSummary model."""

    def test_feature_branch_summary_creation(self) -> None:
        """Test FeatureBranchSummary can be created with valid data."""
        summary = FeatureBranchSummary(
            feature_branch="feature/tui",
            total_tasks=10,
            status_counts={
                TaskStatus.PENDING: 3,
                TaskStatus.RUNNING: 2,
                TaskStatus.COMPLETED: 5,
            },
            blockers=[],
            completion_rate=0.5,
            avg_priority=6.5,
        )
        assert summary.feature_branch == "feature/tui"
        assert summary.total_tasks == 10
        assert summary.completion_rate == 0.5
        assert summary.avg_priority == 6.5

    def test_feature_branch_summary_with_blockers(self) -> None:
        """Test FeatureBranchSummary can store blocker tasks."""
        blocked_task = Task(prompt="Blocked task", status=TaskStatus.BLOCKED)
        failed_task = Task(prompt="Failed task", status=TaskStatus.FAILED)

        summary = FeatureBranchSummary(
            feature_branch="feature/api",
            total_tasks=5,
            status_counts={TaskStatus.BLOCKED: 1, TaskStatus.FAILED: 1},
            blockers=[blocked_task, failed_task],
            completion_rate=0.0,
            avg_priority=5.0,
        )
        assert len(summary.blockers) == 2
        assert blocked_task in summary.blockers
        assert failed_task in summary.blockers

    def test_feature_branch_summary_validation(self) -> None:
        """Test FeatureBranchSummary validates constraints."""
        # Valid: completion_rate in range
        summary = FeatureBranchSummary(
            feature_branch="feature/test",
            total_tasks=1,
            status_counts={},
            completion_rate=0.75,
            avg_priority=5.0,
        )
        assert summary.completion_rate == 0.75

        # Invalid: completion_rate > 1.0
        with pytest.raises(ValueError):
            FeatureBranchSummary(
                feature_branch="feature/test",
                total_tasks=1,
                status_counts={},
                completion_rate=1.5,
                avg_priority=5.0,
            )

        # Invalid: avg_priority > 10.0
        with pytest.raises(ValueError):
            FeatureBranchSummary(
                feature_branch="feature/test",
                total_tasks=1,
                status_counts={},
                completion_rate=0.5,
                avg_priority=11.0,
            )
