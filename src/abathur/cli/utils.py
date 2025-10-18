"""CLI utility functions for Abathur."""

import re
from typing import Optional


# Duration conversion constants
# Note: Months and years use approximations for CLI convenience
DAYS_PER_DAY = 1
DAYS_PER_WEEK = 7
DAYS_PER_MONTH_APPROX = 30  # Approximation: actual months vary 28-31 days
DAYS_PER_YEAR_APPROX = 365   # Approximation: does not account for leap years (366 days)

DURATION_MULTIPLIERS = {
    "d": DAYS_PER_DAY,
    "w": DAYS_PER_WEEK,
    "m": DAYS_PER_MONTH_APPROX,
    "y": DAYS_PER_YEAR_APPROX,
}


def parse_duration_to_days(duration_str: str) -> int:
    """
    Parse a duration string (e.g., '7d', '2w', '1m', '1y') into number of days.

    Args:
        duration_str: Duration string with format: number + unit
                     Units: 'd' (days), 'w' (weeks), 'm' (months), 'y' (years)

    Returns:
        Number of days as integer

    Raises:
        ValueError: If duration string format is invalid or unit is unsupported

    Examples:
        >>> parse_duration_to_days('7d')
        7
        >>> parse_duration_to_days('2w')
        14
        >>> parse_duration_to_days('1m')
        30
        >>> parse_duration_to_days('1y')
        365
    """
    # Validate format
    match = re.match(r'^(\d+)([dwmy])$', duration_str.lower())
    if not match:
        raise ValueError(
            f"Invalid duration format: '{duration_str}'. "
            "Expected format: <number><unit> (e.g., '7d', '2w', '1m', '1y')"
        )

    value = int(match.group(1))
    unit = match.group(2)

    # Validate unit and calculate days
    if unit not in DURATION_MULTIPLIERS:
        raise ValueError(f"Unsupported time unit: '{unit}'. Supported: d, w, m, y")

    return value * DURATION_MULTIPLIERS[unit]
