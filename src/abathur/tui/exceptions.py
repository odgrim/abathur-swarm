"""TUI-specific exceptions.

This module contains exception classes for TUI-specific error conditions
and error handling.
"""


class TUIError(Exception):
    """Base exception for TUI-related errors."""

    pass


class RenderingError(TUIError):
    """Exception raised when rendering fails."""

    pass


class ViewModeError(TUIError):
    """Exception raised for view mode related errors."""

    pass
