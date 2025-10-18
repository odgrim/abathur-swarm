"""Unit tests for CLI utility functions."""

import pytest

from abathur.cli.utils import parse_duration_to_days


class TestParseDurationToDays:
    """Test suite for parse_duration_to_days function."""

    # Valid format tests
    def test_parse_days(self):
        """Test parsing days format."""
        assert parse_duration_to_days("30d") == 30
        assert parse_duration_to_days("1d") == 1
        assert parse_duration_to_days("365d") == 365

    def test_parse_weeks(self):
        """Test parsing weeks format."""
        assert parse_duration_to_days("2w") == 14  # 2 * 7
        assert parse_duration_to_days("1w") == 7
        assert parse_duration_to_days("12w") == 84  # 12 * 7

    def test_parse_months(self):
        """Test parsing months format (30 days/month approximation)."""
        assert parse_duration_to_days("3m") == 90  # 3 * 30
        assert parse_duration_to_days("1m") == 30
        assert parse_duration_to_days("6m") == 180  # 6 * 30

    def test_parse_years(self):
        """Test parsing years format (365 days/year approximation)."""
        assert parse_duration_to_days("1y") == 365
        assert parse_duration_to_days("2y") == 730  # 2 * 365

    # Case insensitivity tests
    def test_parse_case_insensitive(self):
        """Test that parsing is case-insensitive."""
        assert parse_duration_to_days("30D") == 30
        assert parse_duration_to_days("2W") == 14
        assert parse_duration_to_days("3M") == 90
        assert parse_duration_to_days("1Y") == 365

    # Whitespace handling tests
    def test_parse_with_whitespace(self):
        """Test that leading/trailing whitespace is handled."""
        assert parse_duration_to_days(" 30d ") == 30
        assert parse_duration_to_days("  2w") == 14
        assert parse_duration_to_days("3m  ") == 90

    # Large values tests
    def test_parse_large_values(self):
        """Test parsing large duration values."""
        assert parse_duration_to_days("999d") == 999
        assert parse_duration_to_days("100w") == 700  # 100 * 7
        assert parse_duration_to_days("24m") == 720  # 24 * 30
        assert parse_duration_to_days("10y") == 3650  # 10 * 365

    # Invalid format tests
    def test_parse_invalid_format_no_unit(self):
        """Test that format without unit raises ValueError."""
        with pytest.raises(ValueError, match="Invalid duration format"):
            parse_duration_to_days("30")

    def test_parse_invalid_format_no_number(self):
        """Test that format without number raises ValueError."""
        with pytest.raises(ValueError, match="Invalid duration format"):
            parse_duration_to_days("d")

    def test_parse_invalid_format_wrong_order(self):
        """Test that wrong order (unit before number) raises ValueError."""
        with pytest.raises(ValueError, match="Invalid duration format"):
            parse_duration_to_days("d30")

    def test_parse_invalid_unit(self):
        """Test that unsupported unit raises ValueError."""
        with pytest.raises(ValueError, match="Invalid duration format"):
            parse_duration_to_days("30x")
        with pytest.raises(ValueError, match="Invalid duration format"):
            parse_duration_to_days("30h")  # hours not supported

    def test_parse_invalid_alphabetic(self):
        """Test that pure alphabetic string raises ValueError."""
        with pytest.raises(ValueError, match="Invalid duration format"):
            parse_duration_to_days("invalid")
        with pytest.raises(ValueError, match="Invalid duration format"):
            parse_duration_to_days("abc")

    def test_parse_empty_string(self):
        """Test that empty string raises ValueError."""
        with pytest.raises(ValueError, match="Invalid duration format"):
            parse_duration_to_days("")

    def test_parse_whitespace_only(self):
        """Test that whitespace-only string raises ValueError."""
        with pytest.raises(ValueError, match="Invalid duration format"):
            parse_duration_to_days("   ")

    # Edge case: zero duration
    def test_parse_zero_duration(self):
        """Test that zero duration raises ValueError."""
        with pytest.raises(ValueError, match="Duration must be positive"):
            parse_duration_to_days("0d")
        with pytest.raises(ValueError, match="Duration must be positive"):
            parse_duration_to_days("0w")
        with pytest.raises(ValueError, match="Duration must be positive"):
            parse_duration_to_days("0m")
        with pytest.raises(ValueError, match="Duration must be positive"):
            parse_duration_to_days("0y")

    # Edge case: negative values (should be rejected by regex pattern)
    def test_parse_negative_duration(self):
        """Test that negative duration raises ValueError."""
        with pytest.raises(ValueError, match="Invalid duration format"):
            parse_duration_to_days("-5d")
        with pytest.raises(ValueError, match="Invalid duration format"):
            parse_duration_to_days("-1w")

    # Edge case: decimal values (should be rejected)
    def test_parse_decimal_value(self):
        """Test that decimal values raise ValueError."""
        with pytest.raises(ValueError, match="Invalid duration format"):
            parse_duration_to_days("3.5d")
        with pytest.raises(ValueError, match="Invalid duration format"):
            parse_duration_to_days("1.5w")

    # Edge case: multiple units (should be rejected)
    def test_parse_multiple_units(self):
        """Test that multiple units raise ValueError."""
        with pytest.raises(ValueError, match="Invalid duration format"):
            parse_duration_to_days("30d2w")
        with pytest.raises(ValueError, match="Invalid duration format"):
            parse_duration_to_days("1y6m")

    # Boundary value tests
    def test_parse_boundary_values(self):
        """Test parsing at common boundary values."""
        assert parse_duration_to_days("1d") == 1  # Minimum valid
        assert parse_duration_to_days("7d") == 7  # Week in days
        assert parse_duration_to_days("30d") == 30  # Month approximation
        assert parse_duration_to_days("90d") == 90  # Quarter
        assert parse_duration_to_days("365d") == 365  # Year approximation
