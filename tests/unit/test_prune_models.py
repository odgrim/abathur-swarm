"""Unit tests for PruneFilters and PruneResult Pydantic models."""

from datetime import datetime, timezone

import pytest
from pydantic import ValidationError

from abathur.domain.models import TaskStatus
from abathur.infrastructure.database import PruneFilters, PruneResult


class TestPruneFilters:
    """Tests for PruneFilters validation."""

    def test_valid_older_than_days(self):
        """Test valid configuration with older_than_days."""
        filters = PruneFilters(older_than_days=30)
        assert filters.older_than_days == 30
        assert filters.before_date is None
        assert filters.dry_run is False
        assert filters.limit is None
        assert set(filters.statuses) == {
            TaskStatus.COMPLETED,
            TaskStatus.FAILED,
            TaskStatus.CANCELLED,
        }

    def test_valid_before_date(self):
        """Test valid configuration with before_date."""
        cutoff_date = datetime(2025, 1, 1, 0, 0, 0, tzinfo=timezone.utc)
        filters = PruneFilters(before_date=cutoff_date)
        assert filters.before_date == cutoff_date
        assert filters.older_than_days is None
        assert filters.dry_run is False

    def test_valid_both_filters(self):
        """Test valid configuration with both time filters."""
        cutoff_date = datetime(2025, 1, 1, 0, 0, 0, tzinfo=timezone.utc)
        filters = PruneFilters(older_than_days=30, before_date=cutoff_date)
        assert filters.older_than_days == 30
        assert filters.before_date == cutoff_date

    def test_valid_with_limit(self):
        """Test valid configuration with limit."""
        filters = PruneFilters(older_than_days=30, limit=100)
        assert filters.limit == 100

    def test_valid_with_dry_run(self):
        """Test valid configuration with dry_run enabled."""
        filters = PruneFilters(older_than_days=30, dry_run=True)
        assert filters.dry_run is True

    def test_valid_custom_statuses(self):
        """Test valid configuration with custom statuses."""
        filters = PruneFilters(
            older_than_days=30, statuses=[TaskStatus.COMPLETED, TaskStatus.FAILED]
        )
        assert set(filters.statuses) == {TaskStatus.COMPLETED, TaskStatus.FAILED}

    def test_requires_time_filter(self):
        """Test that at least one time filter is required."""
        with pytest.raises(ValueError, match="At least one of"):
            PruneFilters()

    def test_forbids_pending_status(self):
        """Test that PENDING status cannot be pruned."""
        with pytest.raises(ValueError, match="Cannot prune tasks"):
            PruneFilters(older_than_days=30, statuses=[TaskStatus.PENDING])

    def test_forbids_blocked_status(self):
        """Test that BLOCKED status cannot be pruned."""
        with pytest.raises(ValueError, match="Cannot prune tasks"):
            PruneFilters(older_than_days=30, statuses=[TaskStatus.BLOCKED])

    def test_forbids_ready_status(self):
        """Test that READY status cannot be pruned."""
        with pytest.raises(ValueError, match="Cannot prune tasks"):
            PruneFilters(older_than_days=30, statuses=[TaskStatus.READY])

    def test_forbids_running_status(self):
        """Test that RUNNING status cannot be pruned."""
        with pytest.raises(ValueError, match="Cannot prune tasks"):
            PruneFilters(older_than_days=30, statuses=[TaskStatus.RUNNING])

    def test_forbids_mixed_statuses(self):
        """Test that mixing forbidden and allowed statuses fails."""
        with pytest.raises(ValueError, match="Cannot prune tasks"):
            PruneFilters(
                older_than_days=30,
                statuses=[TaskStatus.COMPLETED, TaskStatus.PENDING],
            )

    def test_older_than_days_minimum(self):
        """Test that older_than_days must be >= 1."""
        with pytest.raises(ValidationError):
            PruneFilters(older_than_days=0)

    def test_older_than_days_negative(self):
        """Test that older_than_days cannot be negative."""
        with pytest.raises(ValidationError):
            PruneFilters(older_than_days=-1)

    def test_limit_minimum(self):
        """Test that limit must be >= 1."""
        with pytest.raises(ValidationError):
            PruneFilters(older_than_days=30, limit=0)

    def test_limit_negative(self):
        """Test that limit cannot be negative."""
        with pytest.raises(ValidationError):
            PruneFilters(older_than_days=30, limit=-1)

    def test_default_statuses(self):
        """Test that default statuses are COMPLETED, FAILED, CANCELLED."""
        filters = PruneFilters(older_than_days=30)
        assert len(filters.statuses) == 3
        assert TaskStatus.COMPLETED in filters.statuses
        assert TaskStatus.FAILED in filters.statuses
        assert TaskStatus.CANCELLED in filters.statuses


class TestPruneResult:
    """Tests for PruneResult model."""

    def test_valid_result(self):
        """Test valid PruneResult creation."""
        result = PruneResult(
            deleted_tasks=10,
            deleted_dependencies=15,
            reclaimed_bytes=1024,
            dry_run=False,
            breakdown_by_status={TaskStatus.COMPLETED: 8, TaskStatus.FAILED: 2},
        )
        assert result.deleted_tasks == 10
        assert result.deleted_dependencies == 15
        assert result.reclaimed_bytes == 1024
        assert result.dry_run is False
        assert result.breakdown_by_status == {TaskStatus.COMPLETED: 8, TaskStatus.FAILED: 2}

    def test_valid_without_reclaimed_bytes(self):
        """Test valid PruneResult without reclaimed_bytes."""
        result = PruneResult(
            deleted_tasks=5, deleted_dependencies=3, dry_run=True
        )
        assert result.deleted_tasks == 5
        assert result.deleted_dependencies == 3
        assert result.reclaimed_bytes is None
        assert result.dry_run is True
        assert result.breakdown_by_status == {}

    def test_deleted_tasks_non_negative(self):
        """Test that deleted_tasks must be non-negative."""
        with pytest.raises(ValidationError):
            PruneResult(deleted_tasks=-1, deleted_dependencies=0, dry_run=False)

    def test_deleted_dependencies_non_negative(self):
        """Test that deleted_dependencies must be non-negative."""
        with pytest.raises(ValidationError):
            PruneResult(deleted_tasks=0, deleted_dependencies=-1, dry_run=False)

    def test_reclaimed_bytes_non_negative(self):
        """Test that reclaimed_bytes must be non-negative when provided."""
        with pytest.raises(ValidationError):
            PruneResult(
                deleted_tasks=0,
                deleted_dependencies=0,
                reclaimed_bytes=-1,
                dry_run=False,
            )

    def test_zero_deleted_tasks(self):
        """Test that zero deleted_tasks is valid."""
        result = PruneResult(deleted_tasks=0, deleted_dependencies=0, dry_run=True)
        assert result.deleted_tasks == 0
        assert result.deleted_dependencies == 0

    def test_breakdown_by_status_default(self):
        """Test that breakdown_by_status defaults to empty dict."""
        result = PruneResult(deleted_tasks=0, deleted_dependencies=0, dry_run=True)
        assert result.breakdown_by_status == {}
        assert isinstance(result.breakdown_by_status, dict)

    def test_breakdown_by_status_non_negative_values(self):
        """Test that breakdown_by_status values must be non-negative."""
        with pytest.raises(ValidationError, match="must be non-negative"):
            PruneResult(
                deleted_tasks=0,
                deleted_dependencies=0,
                dry_run=False,
                breakdown_by_status={TaskStatus.COMPLETED: -1},
            )

    def test_breakdown_by_status_with_multiple_statuses(self):
        """Test breakdown_by_status with multiple task statuses."""
        result = PruneResult(
            deleted_tasks=20,
            deleted_dependencies=10,
            dry_run=False,
            breakdown_by_status={
                TaskStatus.COMPLETED: 15,
                TaskStatus.FAILED: 3,
                TaskStatus.CANCELLED: 2,
            },
        )
        assert result.breakdown_by_status[TaskStatus.COMPLETED] == 15
        assert result.breakdown_by_status[TaskStatus.FAILED] == 3
        assert result.breakdown_by_status[TaskStatus.CANCELLED] == 2
        assert len(result.breakdown_by_status) == 3
