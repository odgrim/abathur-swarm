"""Unit tests for PruneFilters.build_where_clause() method."""

from datetime import datetime, timezone


from abathur.domain.models import TaskStatus
from abathur.infrastructure.database import PruneFilters


class TestPruneFiltersBuildWhereClause:
    """Test WHERE clause generation for task filtering."""

    def test_older_than_days_generates_correct_clause(self):
        """Test WHERE clause for older_than_days filter."""
        filters = PruneFilters(
            older_than_days=30, statuses=[TaskStatus.COMPLETED]
        )

        where_sql, params = filters.build_where_clause()

        # Verify WHERE clause structure
        assert "(completed_at < date('now', ?" in where_sql
        assert "(completed_at IS NULL AND submitted_at < date('now', ?" in where_sql
        assert "status IN (?)" in where_sql
        assert " AND " in where_sql

        # Verify parameters
        assert params[0] == "-30 days"
        assert params[1] == "-30 days"
        assert params[2] == "completed"

    def test_before_date_generates_correct_clause(self):
        """Test WHERE clause for before_date filter."""
        before = datetime(2025, 1, 1, 0, 0, 0, tzinfo=timezone.utc)
        filters = PruneFilters(before_date=before, statuses=[TaskStatus.FAILED])

        where_sql, params = filters.build_where_clause()

        # Verify WHERE clause structure
        assert "(completed_at < ?" in where_sql
        assert "(completed_at IS NULL AND submitted_at < ?)" in where_sql
        assert "status IN (?)" in where_sql

        # Verify parameters
        assert params[0] == "2025-01-01T00:00:00+00:00"
        assert params[1] == "2025-01-01T00:00:00+00:00"
        assert params[2] == "failed"

    def test_multiple_statuses_generates_correct_placeholders(self):
        """Test WHERE clause with multiple status filters."""
        filters = PruneFilters(
            older_than_days=7,
            statuses=[
                TaskStatus.COMPLETED,
                TaskStatus.FAILED,
                TaskStatus.CANCELLED,
            ],
        )

        where_sql, params = filters.build_where_clause()

        # Verify status placeholders
        assert "status IN (?,?,?)" in where_sql

        # Verify all status values in parameters
        assert "completed" in params
        assert "failed" in params
        assert "cancelled" in params
        assert (
            len([p for p in params if p in ["completed", "failed", "cancelled"]])
            == 3
        )

    def test_single_status_generates_single_placeholder(self):
        """Test WHERE clause with single status filter."""
        filters = PruneFilters(
            older_than_days=1, statuses=[TaskStatus.COMPLETED]
        )

        where_sql, params = filters.build_where_clause()

        # Verify single placeholder
        assert "status IN (?)" in where_sql
        assert params[-1] == "completed"

    def test_where_clause_combines_with_and(self):
        """Test that WHERE clauses are combined with AND operator."""
        filters = PruneFilters(
            older_than_days=30, statuses=[TaskStatus.COMPLETED]
        )

        where_sql, params = filters.build_where_clause()

        # Time filter and status filter should be joined by AND
        # Note: There's also an AND within the time filter for NULL check
        # So we verify the top-level structure instead
        assert where_sql.count(" AND status IN") == 1

        # Time filter should come first, then status filter
        time_clause_end = where_sql.find("))")
        status_clause_start = where_sql.find("status IN")
        assert time_clause_end < status_clause_start

    def test_older_than_days_parameter_format(self):
        """Test that older_than_days generates negative day offset."""
        filters = PruneFilters(
            older_than_days=90, statuses=[TaskStatus.COMPLETED]
        )

        where_sql, params = filters.build_where_clause()

        # Should have "-90 days" format for SQLite date() function
        assert params[0] == "-90 days"
        assert params[1] == "-90 days"

    def test_before_date_iso_format(self):
        """Test that before_date is converted to ISO 8601 format."""
        before = datetime(2024, 12, 31, 23, 59, 59, tzinfo=timezone.utc)
        filters = PruneFilters(
            before_date=before, statuses=[TaskStatus.CANCELLED]
        )

        where_sql, params = filters.build_where_clause()

        # Should be ISO 8601 format with timezone
        assert params[0] == "2024-12-31T23:59:59+00:00"
        assert params[1] == "2024-12-31T23:59:59+00:00"

    def test_default_statuses_generate_three_placeholders(self):
        """Test WHERE clause with default statuses (COMPLETED, FAILED, CANCELLED)."""
        filters = PruneFilters(older_than_days=7)

        where_sql, params = filters.build_where_clause()

        # Default has 3 statuses
        assert "status IN (?,?,?)" in where_sql
        assert len([p for p in params if p in ["completed", "failed", "cancelled"]]) == 3

    def test_older_than_days_precedence_over_before_date(self):
        """Test that older_than_days is used when both time filters are present."""
        before = datetime(2025, 1, 1, 0, 0, 0, tzinfo=timezone.utc)
        filters = PruneFilters(
            older_than_days=30,
            before_date=before,
            statuses=[TaskStatus.COMPLETED],
        )

        where_sql, params = filters.build_where_clause()

        # Should use older_than_days (date('now', ?)) not before_date
        assert "date('now', ?" in where_sql
        assert params[0] == "-30 days"
        # before_date should NOT be in params
        assert "2025-01-01T00:00:00+00:00" not in params

    def test_parameter_count_matches_placeholders(self):
        """Test that parameter count matches placeholder count."""
        filters = PruneFilters(
            older_than_days=7,
            statuses=[
                TaskStatus.COMPLETED,
                TaskStatus.FAILED,
                TaskStatus.CANCELLED,
            ],
        )

        where_sql, params = filters.build_where_clause()

        # Count placeholders in SQL
        placeholder_count = where_sql.count("?")
        # Should be: 2 for time filter + 3 for statuses = 5
        assert placeholder_count == 5
        assert len(params) == 5

    def test_parameter_order_matches_placeholder_order(self):
        """Test that parameters are in correct order for placeholders."""
        before = datetime(2025, 6, 15, 12, 30, 0, tzinfo=timezone.utc)
        filters = PruneFilters(
            before_date=before,
            statuses=[TaskStatus.COMPLETED, TaskStatus.FAILED],
        )

        where_sql, params = filters.build_where_clause()

        # Parameters should be: [before_iso, before_iso, "completed", "failed"]
        assert len(params) == 4
        assert params[0] == "2025-06-15T12:30:00+00:00"
        assert params[1] == "2025-06-15T12:30:00+00:00"
        assert params[2] == "completed"
        assert params[3] == "failed"

    def test_completed_at_null_handling_in_time_filter(self):
        """Test that WHERE clause handles NULL completed_at with submitted_at fallback."""
        filters = PruneFilters(
            older_than_days=30, statuses=[TaskStatus.COMPLETED]
        )

        where_sql, params = filters.build_where_clause()

        # Should check both completed_at and submitted_at with OR
        assert "completed_at < date('now', ?)" in where_sql
        assert "completed_at IS NULL AND submitted_at < date('now', ?)" in where_sql
        assert " OR " in where_sql

    def test_where_clause_without_leading_where_keyword(self):
        """Test that WHERE clause does not include 'WHERE' keyword."""
        filters = PruneFilters(
            older_than_days=7, statuses=[TaskStatus.COMPLETED]
        )

        where_sql, params = filters.build_where_clause()

        # Should NOT start with 'WHERE'
        assert not where_sql.strip().upper().startswith("WHERE")
        # Should be a condition that can be used after 'WHERE'
        assert where_sql.startswith("(")
