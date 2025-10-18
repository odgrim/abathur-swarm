"""Utility functions for CLI argument parsing and validation."""

import re


def parse_duration_to_days(duration_str: str) -> int:
    """
    Parse duration string to integer days.

    Supported formats:
    - "30d" = 30 days
    - "2w" = 14 days (2 weeks × 7 days)
    - "6m" = ~180 days (6 months × 30 days)
    - "1y" = 365 days

    Args:
        duration_str: Duration string with unit suffix (d/w/m/y)

    Returns:
        Integer number of days

    Raises:
        ValueError: If format is invalid, unsupported unit, or value is non-positive

    Examples:
        >>> parse_duration_to_days("30d")
        30
        >>> parse_duration_to_days("2w")
        14
        >>> parse_duration_to_days("6m")
        180
        >>> parse_duration_to_days("1y")
        365
    """
    # Regex pattern to match number + unit
    # Captures: integer value and single-character unit (d/w/m/y)
    pattern = r"^(\d+)([dwmy])$"
    match = re.match(pattern, duration_str.lower().strip())

    if not match:
        raise ValueError(
            f"Invalid duration format: '{duration_str}'. "
            f"Expected format: <number><unit> (e.g., '30d', '2w', '6m', '1y')"
        )

    value = int(match.group(1))
    unit = match.group(2)

    # Validate non-zero positive value
    if value == 0:
        raise ValueError(
            f"Duration must be positive: '{duration_str}'. "
            f"Zero duration is not allowed."
        )

    # Convert to days
    # Note: months and years use approximations
    multipliers = {
        "d": 1,  # days
        "w": 7,  # weeks
        "m": 30,  # months (approximation: 30 days/month)
        "y": 365,  # years (approximation: 365 days/year, no leap year)
    }

    return value * multipliers[unit]
