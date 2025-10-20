"""Utility functions for CLI argument parsing and validation.

Duration Approximations
-----------------------
The parse_duration_to_days function uses simplified approximations for
time unit conversions to provide a convenient CLI interface:

- Months: 30 days (actual months vary from 28-31 days)
- Years: 365 days (does not account for leap years with 366 days)

These approximations are intentional design decisions prioritizing:
1. Simplicity: Easy mental math for users (e.g., "6m" = 180 days)
2. Consistency: Predictable behavior regardless of current date
3. CLI Convenience: Quick estimates without calendar complexity

Trade-offs:
- A "1m" duration means exactly 30 days, not "1 calendar month"
- A "1y" duration means exactly 365 days, not "1 calendar year"
- Users should be aware these are approximations for long durations

Examples:
    "6m" → 180 days (not 181-184 days depending on months)
    "1y" → 365 days (not 365-366 days depending on leap year)
"""

import re


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

MAX_DURATION_DAYS = 36_500  # ~100 years, reasonable upper limit


def parse_duration_to_days(duration_str: str) -> int:
    """
    Parse a duration string (e.g., '7d', '2w', '1m', '1y') into number of days.

    Args:
        duration_str: Duration string with format: number + unit
                     Units: 'd' (days), 'w' (weeks), 'm' (months), 'y' (years)

    Returns:
        Number of days as integer

    Raises:
        ValueError: If duration string format is invalid, unit is unsupported,
                   duration is zero (not allowed), or duration exceeds
                   maximum allowed duration (~100 years)

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
    match = re.match(r'^(\d+)([dwmy])$', duration_str.lower().strip())
    if not match:
        raise ValueError(
            f"Invalid duration format: '{duration_str}'. "
            "Expected format: <number><unit> (e.g., '7d', '2w', '1m', '1y')"
        )

    value = int(match.group(1))

    # Validate non-zero positive value
    if value == 0:
        raise ValueError(
            f"Duration must be positive: '{duration_str}'. "
            "Zero duration is not allowed."
        )

    unit = match.group(2)

    # Validate unit and calculate days
    if unit not in DURATION_MULTIPLIERS:
        raise ValueError(f"Unsupported time unit: '{unit}'. Supported: d, w, m, y")

    result = value * DURATION_MULTIPLIERS[unit]

    # Validate maximum duration
    if result > MAX_DURATION_DAYS:
        raise ValueError(
            f"Duration exceeds maximum allowed: '{duration_str}' = {result} days. "
            f"Maximum allowed: {MAX_DURATION_DAYS} days (~100 years)."
        )

    return result
