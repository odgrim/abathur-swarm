"""TUI-specific exceptions."""


class TUIDataError(Exception):
    """Exception raised for TUI data service errors.

    Used when data fetching or caching operations fail and
    there's no valid fallback (stale cache) available.
    """

    pass
