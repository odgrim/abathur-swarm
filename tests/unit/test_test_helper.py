"""Unit tests for test_helper module.

Tests the calculate_sum function with various scenarios:
- Basic addition
- Zero handling
- Negative numbers
- Mixed signs
"""

import pytest
from src.utils.test_helper import calculate_sum


class TestCalculateSum:
    """Unit tests for calculate_sum function."""

    def test_basic_addition(self):
        """Test basic addition of two positive integers."""
        # Arrange
        a = 2
        b = 3

        # Act
        result = calculate_sum(a, b)

        # Assert
        assert result == 5

    def test_zero_handling(self):
        """Test addition with zero (identity element)."""
        # Arrange
        a = 0
        b = 5

        # Act
        result = calculate_sum(a, b)

        # Assert
        assert result == 5

    def test_negative_numbers(self):
        """Test addition of two negative integers."""
        # Arrange
        a = -2
        b = -3

        # Act
        result = calculate_sum(a, b)

        # Assert
        assert result == -5

    def test_mixed_signs(self):
        """Test addition of negative and positive integers."""
        # Arrange
        a = -5
        b = 10

        # Act
        result = calculate_sum(a, b)

        # Assert
        assert result == 5

    def test_zero_plus_zero(self):
        """Test addition of zero plus zero."""
        # Arrange
        a = 0
        b = 0

        # Act
        result = calculate_sum(a, b)

        # Assert
        assert result == 0

    def test_large_positive_numbers(self):
        """Test addition of large positive integers."""
        # Arrange
        a = 1000000
        b = 2000000

        # Act
        result = calculate_sum(a, b)

        # Assert
        assert result == 3000000

    def test_large_negative_numbers(self):
        """Test addition of large negative integers."""
        # Arrange
        a = -1000000
        b = -2000000

        # Act
        result = calculate_sum(a, b)

        # Assert
        assert result == -3000000

    def test_commutative_property(self):
        """Test that addition is commutative (a + b = b + a)."""
        # Arrange
        a = 7
        b = 3

        # Act
        result1 = calculate_sum(a, b)
        result2 = calculate_sum(b, a)

        # Assert
        assert result1 == result2
        assert result1 == 10


# Parametrized test for multiple scenarios
@pytest.mark.parametrize(
    "a,b,expected",
    [
        (2, 3, 5),           # Basic addition
        (0, 5, 5),           # Zero handling
        (-2, -3, -5),        # Negative numbers
        (-5, 10, 5),         # Mixed signs
        (0, 0, 0),           # Both zero
        (100, 200, 300),     # Larger numbers
        (-100, 100, 0),      # Cancellation
        (1, -1, 0),          # Small cancellation
    ],
)
def test_calculate_sum_parametrized(a, b, expected):
    """Parametrized test covering multiple scenarios."""
    assert calculate_sum(a, b) == expected
