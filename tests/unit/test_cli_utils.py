"""Unit tests for CLI utility functions."""

import pytest

from abathur.cli.utils import (
    DAYS_PER_DAY,
    DAYS_PER_WEEK,
    DAYS_PER_MONTH_APPROX,
    DAYS_PER_YEAR_APPROX,
    DURATION_MULTIPLIERS,
    parse_duration_to_days,
)


class TestDurationConstants:
    """Tests for duration conversion constants."""

    def test_days_per_day_value(self):
        """Test DAYS_PER_DAY constant value."""
        assert DAYS_PER_DAY == 1

    def test_days_per_week_value(self):
        """Test DAYS_PER_WEEK constant value."""
        assert DAYS_PER_WEEK == 7

    def test_days_per_month_approx_value(self):
        """Test DAYS_PER_MONTH_APPROX constant value."""
        assert DAYS_PER_MONTH_APPROX == 30

    def test_days_per_year_approx_value(self):
        """Test DAYS_PER_YEAR_APPROX constant value."""
        assert DAYS_PER_YEAR_APPROX == 365

    def test_duration_multipliers_dict(self):
        """Test DURATION_MULTIPLIERS dictionary structure."""
        assert "d" in DURATION_MULTIPLIERS
        assert "w" in DURATION_MULTIPLIERS
        assert "m" in DURATION_MULTIPLIERS
        assert "y" in DURATION_MULTIPLIERS

    def test_duration_multipliers_uses_constants(self):
        """Test that DURATION_MULTIPLIERS uses the named constants."""
        assert DURATION_MULTIPLIERS["d"] == DAYS_PER_DAY
        assert DURATION_MULTIPLIERS["w"] == DAYS_PER_WEEK
        assert DURATION_MULTIPLIERS["m"] == DAYS_PER_MONTH_APPROX
        assert DURATION_MULTIPLIERS["y"] == DAYS_PER_YEAR_APPROX


class TestParseDurationToDays:
    """Tests for parse_duration_to_days function."""

    def test_parse_days(self):
        """Test parsing days duration."""
        assert parse_duration_to_days("1d") == 1
        assert parse_duration_to_days("7d") == 7
        assert parse_duration_to_days("30d") == 30

    def test_parse_weeks(self):
        """Test parsing weeks duration."""
        assert parse_duration_to_days("1w") == 7
        assert parse_duration_to_days("2w") == 14
        assert parse_duration_to_days("4w") == 28

    def test_parse_months(self):
        """Test parsing months duration (approximation)."""
        assert parse_duration_to_days("1m") == 30
        assert parse_duration_to_days("2m") == 60
        assert parse_duration_to_days("12m") == 360

    def test_parse_years(self):
        """Test parsing years duration (approximation)."""
        assert parse_duration_to_days("1y") == 365
        assert parse_duration_to_days("2y") == 730
        assert parse_duration_to_days("5y") == 1825

    def test_parse_case_insensitive(self):
        """Test that parsing is case-insensitive."""
        assert parse_duration_to_days("7D") == 7
        assert parse_duration_to_days("2W") == 14
        assert parse_duration_to_days("1M") == 30
        assert parse_duration_to_days("1Y") == 365

    def test_invalid_format_no_unit(self):
        """Test error handling for duration without unit."""
        with pytest.raises(ValueError, match="Invalid duration format"):
            parse_duration_to_days("7")

    def test_invalid_format_no_number(self):
        """Test error handling for duration without number."""
        with pytest.raises(ValueError, match="Invalid duration format"):
            parse_duration_to_days("d")

    def test_invalid_format_invalid_unit(self):
        """Test error handling for unsupported unit."""
        with pytest.raises(ValueError, match="Invalid duration format"):
            parse_duration_to_days("7x")

    def test_invalid_format_spaces(self):
        """Test error handling for duration with spaces."""
        with pytest.raises(ValueError, match="Invalid duration format"):
            parse_duration_to_days("7 d")

    def test_invalid_format_negative(self):
        """Test error handling for negative duration."""
        with pytest.raises(ValueError, match="Invalid duration format"):
            parse_duration_to_days("-7d")

    def test_invalid_format_decimal(self):
        """Test error handling for decimal duration."""
        with pytest.raises(ValueError, match="Invalid duration format"):
            parse_duration_to_days("7.5d")

    def test_zero_duration(self):
        """Test parsing zero duration."""
        assert parse_duration_to_days("0d") == 0
        assert parse_duration_to_days("0w") == 0
        assert parse_duration_to_days("0m") == 0
        assert parse_duration_to_days("0y") == 0

    def test_large_duration(self):
        """Test parsing large duration values."""
        assert parse_duration_to_days("999d") == 999
        assert parse_duration_to_days("100w") == 700
        assert parse_duration_to_days("100m") == 3000
        assert parse_duration_to_days("10y") == 3650
