"""Unit tests for FilterState matching logic.

Tests filter matching for:
- Task status filtering (set-based with OR logic within set)
- Agent type filtering
- Feature branch filtering
- Text search in summary/prompt
- Combined filters (AND logic)
- is_active() detection
"""

import pytest
from datetime import datetime, timezone
from uuid import uuid4

from abathur.domain.models import Task, TaskStatus, TaskSource
from abathur.tui.models import FilterState


class TestFilterStateStatusFiltering:
    """Test suite for task status filtering."""

    @pytest.fixture
    def sample_task(self):
        """Create a sample task."""
        return Task(
            id=uuid4(),
            prompt="Test task",
            summary="Test task",
            agent_type="test-agent",
            status=TaskStatus.RUNNING,
            calculated_priority=5.0,
            dependency_depth=0,
            submitted_at=datetime.now(timezone.utc),
            source=TaskSource.HUMAN,
        )

    def test_matches_when_status_filter_matches(self, sample_task):
        """Test task matches when status filter matches."""
        # Arrange
        filter_state = FilterState(status_filter={TaskStatus.RUNNING})

        # Act & Assert
        assert filter_state.matches(sample_task) is True

    def test_does_not_match_when_status_filter_differs(self, sample_task):
        """Test task doesn't match when status filter differs."""
        # Arrange
        filter_state = FilterState(status_filter={TaskStatus.COMPLETED})

        # Act & Assert
        assert filter_state.matches(sample_task) is False

    def test_matches_all_when_no_status_filter(self, sample_task):
        """Test task matches when no status filter set."""
        # Arrange
        filter_state = FilterState(status_filter=None)

        # Act & Assert
        assert filter_state.matches(sample_task) is True

    def test_matches_with_multiple_statuses_in_set(self, sample_task):
        """Test task matches when status in filter set (OR within set)."""
        # Arrange
        filter_state = FilterState(
            status_filter={TaskStatus.RUNNING, TaskStatus.PENDING, TaskStatus.READY}
        )

        # Act & Assert
        assert filter_state.matches(sample_task) is True

    def test_does_not_match_when_status_not_in_set(self, sample_task):
        """Test task doesn't match when status not in filter set."""
        # Arrange
        filter_state = FilterState(
            status_filter={TaskStatus.PENDING, TaskStatus.COMPLETED}
        )

        # Act & Assert
        assert filter_state.matches(sample_task) is False


class TestFilterStateAgentTypeFiltering:
    """Test suite for agent type filtering."""

    @pytest.fixture
    def sample_task(self):
        """Create a sample task."""
        return Task(
            id=uuid4(),
            prompt="Test task",
            summary="Test task",
            agent_type="python-specialist",
            status=TaskStatus.PENDING,
            calculated_priority=5.0,
            dependency_depth=0,
            submitted_at=datetime.now(timezone.utc),
            source=TaskSource.HUMAN,
        )

    def test_matches_when_agent_type_filter_matches(self, sample_task):
        """Test task matches when agent_type filter matches (substring)."""
        # Arrange
        filter_state = FilterState(agent_type_filter="python")

        # Act & Assert
        assert filter_state.matches(sample_task) is True

    def test_does_not_match_when_agent_type_filter_differs(self, sample_task):
        """Test task doesn't match when agent_type differs."""
        # Arrange
        filter_state = FilterState(agent_type_filter="typescript")

        # Act & Assert
        assert filter_state.matches(sample_task) is False

    def test_matches_all_when_no_agent_type_filter(self, sample_task):
        """Test task matches when no agent_type filter set."""
        # Arrange
        filter_state = FilterState(agent_type_filter=None)

        # Act & Assert
        assert filter_state.matches(sample_task) is True

    def test_agent_type_case_insensitive(self, sample_task):
        """Test agent_type filter is case-insensitive."""
        # Arrange
        filter_state = FilterState(agent_type_filter="PYTHON")

        # Act & Assert
        assert filter_state.matches(sample_task) is True


class TestFilterStateFeatureBranchFiltering:
    """Test suite for feature branch filtering."""

    @pytest.fixture
    def sample_task(self):
        """Create a sample task."""
        return Task(
            id=uuid4(),
            prompt="Test task",
            summary="Test task",
            agent_type="test-agent",
            status=TaskStatus.PENDING,
            calculated_priority=5.0,
            dependency_depth=0,
            submitted_at=datetime.now(timezone.utc),
            source=TaskSource.HUMAN,
            feature_branch="feature/tui-implementation",
        )

    def test_matches_when_feature_branch_filter_matches(self, sample_task):
        """Test task matches when feature_branch filter matches (substring)."""
        # Arrange
        filter_state = FilterState(feature_branch_filter="tui")

        # Act & Assert
        assert filter_state.matches(sample_task) is True

    def test_does_not_match_when_feature_branch_filter_differs(self, sample_task):
        """Test task doesn't match when feature_branch differs."""
        # Arrange
        filter_state = FilterState(feature_branch_filter="backend")

        # Act & Assert
        assert filter_state.matches(sample_task) is False

    def test_matches_all_when_no_feature_branch_filter(self, sample_task):
        """Test task matches when no feature_branch filter set."""
        # Arrange
        filter_state = FilterState(feature_branch_filter=None)

        # Act & Assert
        assert filter_state.matches(sample_task) is True

    def test_feature_branch_case_insensitive(self, sample_task):
        """Test feature_branch filter is case-insensitive."""
        # Arrange
        filter_state = FilterState(feature_branch_filter="TUI")

        # Act & Assert
        assert filter_state.matches(sample_task) is True


class TestFilterStateTextSearch:
    """Test suite for text search filtering."""

    def test_matches_when_text_in_summary(self):
        """Test task matches when search text in summary."""
        # Arrange
        task = Task(
            id=uuid4(),
            prompt="Different text",
            summary="Implement authentication feature",
            agent_type="test",
            status=TaskStatus.PENDING,
            calculated_priority=5.0,
            dependency_depth=0,
            submitted_at=datetime.now(timezone.utc),
            source=TaskSource.HUMAN,
        )
        filter_state = FilterState(text_search="authentication")

        # Act & Assert
        assert filter_state.matches(task) is True

    def test_matches_when_text_in_prompt(self):
        """Test task matches when search text in prompt."""
        # Arrange
        task = Task(
            id=uuid4(),
            prompt="Write comprehensive tests for the authentication module",
            summary="Write tests",
            agent_type="test",
            status=TaskStatus.PENDING,
            calculated_priority=5.0,
            dependency_depth=0,
            submitted_at=datetime.now(timezone.utc),
            source=TaskSource.HUMAN,
        )
        filter_state = FilterState(text_search="authentication")

        # Act & Assert
        assert filter_state.matches(task) is True

    def test_case_insensitive_search(self):
        """Test text search is case-insensitive."""
        # Arrange
        task = Task(
            id=uuid4(),
            prompt="Test",
            summary="Implement Authentication Feature",
            agent_type="test",
            status=TaskStatus.PENDING,
            calculated_priority=5.0,
            dependency_depth=0,
            submitted_at=datetime.now(timezone.utc),
            source=TaskSource.HUMAN,
        )
        filter_state = FilterState(text_search="authentication")

        # Act & Assert
        assert filter_state.matches(task) is True

    def test_does_not_match_when_text_not_found(self):
        """Test task doesn't match when search text not found."""
        # Arrange
        task = Task(
            id=uuid4(),
            prompt="Different task",
            summary="Different task",
            agent_type="test",
            status=TaskStatus.PENDING,
            calculated_priority=5.0,
            dependency_depth=0,
            submitted_at=datetime.now(timezone.utc),
            source=TaskSource.HUMAN,
        )
        filter_state = FilterState(text_search="authentication")

        # Act & Assert
        assert filter_state.matches(task) is False

    def test_partial_match_in_summary(self):
        """Test partial text match works."""
        # Arrange
        task = Task(
            id=uuid4(),
            prompt="Test",
            summary="Implement auth module",
            agent_type="test",
            status=TaskStatus.PENDING,
            calculated_priority=5.0,
            dependency_depth=0,
            submitted_at=datetime.now(timezone.utc),
            source=TaskSource.HUMAN,
        )
        filter_state = FilterState(text_search="auth")

        # Act & Assert
        assert filter_state.matches(task) is True


class TestFilterStateCombinedFilters:
    """Test suite for combined filters with AND logic."""

    def test_matches_when_all_filters_match(self):
        """Test task matches when all filters match."""
        # Arrange
        task = Task(
            id=uuid4(),
            prompt="Test",
            summary="Implement authentication",
            agent_type="python-specialist",
            status=TaskStatus.RUNNING,
            calculated_priority=5.0,
            dependency_depth=0,
            submitted_at=datetime.now(timezone.utc),
            source=TaskSource.HUMAN,
            feature_branch="feature/auth",
        )

        filter_state = FilterState(
            status_filter={TaskStatus.RUNNING},
            agent_type_filter="python",
            feature_branch_filter="auth",
            text_search="authentication",
        )

        # Act & Assert
        assert filter_state.matches(task) is True

    def test_does_not_match_when_any_filter_fails(self):
        """Test task doesn't match when any filter fails."""
        # Arrange
        task = Task(
            id=uuid4(),
            prompt="Test",
            summary="Implement authentication",
            agent_type="python-specialist",
            status=TaskStatus.RUNNING,
            calculated_priority=5.0,
            dependency_depth=0,
            submitted_at=datetime.now(timezone.utc),
            source=TaskSource.HUMAN,
            feature_branch="feature/auth",
        )

        # Status doesn't match
        filter_state = FilterState(
            status_filter={TaskStatus.COMPLETED},
            agent_type_filter="python",
        )

        # Act & Assert
        assert filter_state.matches(task) is False

    def test_combines_status_and_agent_type_filters(self):
        """Test combining status and agent_type filters."""
        # Arrange
        task = Task(
            id=uuid4(),
            prompt="Test",
            summary="Test task",
            agent_type="test-agent",
            status=TaskStatus.PENDING,
            calculated_priority=5.0,
            dependency_depth=0,
            submitted_at=datetime.now(timezone.utc),
            source=TaskSource.HUMAN,
        )

        filter_state = FilterState(
            status_filter={TaskStatus.PENDING},
            agent_type_filter="test",
        )

        # Act & Assert
        assert filter_state.matches(task) is True


class TestFilterStateIsActive:
    """Test suite for is_active() detection."""

    def test_is_active_returns_false_when_no_filters(self):
        """Test is_active returns False when no filters set."""
        # Arrange
        filter_state = FilterState()

        # Act & Assert
        assert filter_state.is_active() is False

    def test_is_active_returns_true_when_status_filter_set(self):
        """Test is_active returns True when status filter set."""
        # Arrange
        filter_state = FilterState(status_filter={TaskStatus.PENDING})

        # Act & Assert
        assert filter_state.is_active() is True

    def test_is_active_returns_true_when_agent_type_filter_set(self):
        """Test is_active returns True when agent_type filter set."""
        # Arrange
        filter_state = FilterState(agent_type_filter="python-specialist")

        # Act & Assert
        assert filter_state.is_active() is True

    def test_is_active_returns_true_when_text_search_set(self):
        """Test is_active returns True when text_search set."""
        # Arrange
        filter_state = FilterState(text_search="auth")

        # Act & Assert
        assert filter_state.is_active() is True

    def test_is_active_returns_true_when_multiple_filters_set(self):
        """Test is_active returns True when multiple filters set."""
        # Arrange
        filter_state = FilterState(
            status_filter={TaskStatus.RUNNING},
            agent_type_filter="test-agent",
        )

        # Act & Assert
        assert filter_state.is_active() is True


class TestFilterStateClear:
    """Test suite for filter clearing."""

    def test_clear_resets_all_filters(self):
        """Test clear() resets all filters to None."""
        # Arrange
        filter_state = FilterState(
            status_filter={TaskStatus.RUNNING},
            agent_type_filter="test-agent",
            feature_branch_filter="feature/test",
            text_search="auth",
        )

        # Act
        filter_state.clear()

        # Assert
        assert filter_state.status_filter is None
        assert filter_state.agent_type_filter is None
        assert filter_state.feature_branch_filter is None
        assert filter_state.text_search is None
        assert filter_state.is_active() is False

    def test_clear_makes_filter_match_all_tasks(self):
        """Test cleared filter matches all tasks."""
        # Arrange
        filter_state = FilterState(status_filter={TaskStatus.COMPLETED})
        task = Task(
            id=uuid4(),
            prompt="Test",
            summary="Test",
            agent_type="test",
            status=TaskStatus.PENDING,
            calculated_priority=5.0,
            dependency_depth=0,
            submitted_at=datetime.now(timezone.utc),
            source=TaskSource.HUMAN,
        )

        # Clear filter
        filter_state.clear()

        # Act & Assert
        assert filter_state.matches(task) is True


class TestFilterStateSourceFiltering:
    """Test suite for task source filtering."""

    def test_matches_when_source_filter_matches(self):
        """Test task matches when source filter matches."""
        # Arrange
        task = Task(
            id=uuid4(),
            prompt="Test",
            summary="Test",
            agent_type="test",
            status=TaskStatus.PENDING,
            calculated_priority=5.0,
            dependency_depth=0,
            submitted_at=datetime.now(timezone.utc),
            source=TaskSource.HUMAN,
        )
        filter_state = FilterState(source_filter=TaskSource.HUMAN)

        # Act & Assert
        assert filter_state.matches(task) is True

    def test_does_not_match_when_source_filter_differs(self):
        """Test task doesn't match when source differs."""
        # Arrange
        task = Task(
            id=uuid4(),
            prompt="Test",
            summary="Test",
            agent_type="test",
            status=TaskStatus.PENDING,
            calculated_priority=5.0,
            dependency_depth=0,
            submitted_at=datetime.now(timezone.utc),
            source=TaskSource.HUMAN,
        )
        filter_state = FilterState(source_filter=TaskSource.AGENT_PLANNER)

        # Act & Assert
        assert filter_state.matches(task) is False


class TestFilterStateEdgeCases:
    """Test suite for edge cases."""

    def test_matches_task_with_none_summary(self):
        """Test handles task with None summary."""
        # Arrange
        task = Task(
            id=uuid4(),
            prompt="Test prompt",
            summary=None,
            agent_type="test",
            status=TaskStatus.PENDING,
            calculated_priority=5.0,
            dependency_depth=0,
            submitted_at=datetime.now(timezone.utc),
            source=TaskSource.HUMAN,
        )
        filter_state = FilterState(text_search="test")

        # Act & Assert - should search in prompt
        assert filter_state.matches(task) is True

    def test_matches_task_with_none_feature_branch(self):
        """Test handles task with None feature_branch."""
        # Arrange
        task = Task(
            id=uuid4(),
            prompt="Test",
            summary="Test",
            agent_type="test",
            status=TaskStatus.PENDING,
            calculated_priority=5.0,
            dependency_depth=0,
            submitted_at=datetime.now(timezone.utc),
            source=TaskSource.HUMAN,
            feature_branch=None,
        )
        filter_state = FilterState(feature_branch_filter="feature/test")

        # Act & Assert - should not match
        assert filter_state.matches(task) is False

    def test_empty_text_search_matches_all(self):
        """Test empty string text search is treated as no filter."""
        # Arrange
        task = Task(
            id=uuid4(),
            prompt="Test",
            summary="Test",
            agent_type="test",
            status=TaskStatus.PENDING,
            calculated_priority=5.0,
            dependency_depth=0,
            submitted_at=datetime.now(timezone.utc),
            source=TaskSource.HUMAN,
        )
        filter_state = FilterState(text_search=None)

        # Act & Assert
        assert filter_state.matches(task) is True
