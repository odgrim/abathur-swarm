"""Custom exception classes for Abathur TUI components.

This module defines a hierarchy of exceptions for handling TUI-specific errors.
All exceptions inherit from TUIError, which provides a base for catching any
TUI-related error.

Exception Hierarchy:
    TUIError (base)
    ├── TUIDataError (data fetching failures)
    ├── TUIRenderError (widget/component rendering failures)
    └── TUIConfigError (configuration issues)
"""

from typing import Any


class TUIError(Exception):
    """Base exception for all TUI-specific errors.

    This is the parent class for all TUI exceptions. Use this to catch
    any TUI-related error without needing to know the specific type.

    Examples:
        >>> try:
        ...     raise TUIDataError("Failed to fetch tasks")
        ... except TUIError as e:
        ...     print(f"TUI error occurred: {e}")
        TUI error occurred: Failed to fetch tasks
    """

    def __init__(self, message: str) -> None:
        """Initialize TUIError with a message.

        Args:
            message: Human-readable error description
        """
        self.message = message
        super().__init__(message)

    def __str__(self) -> str:
        """Return string representation of the error."""
        return self.message


class TUIDataError(TUIError):
    """Raised when data fetching or processing fails.

    Use this exception when:
    - Failed to fetch data from the task queue service
    - Failed to parse or validate fetched data
    - Network or connectivity issues prevent data access
    - Data corruption or unexpected format is encountered

    Attributes:
        message: Human-readable error description
        original_exception: The underlying exception that caused this error (optional)

    Examples:
        >>> try:
        ...     result = await service.get_tasks()
        ... except Exception as e:
        ...     raise TUIDataError("Failed to fetch tasks", original_exception=e) from e

        >>> # Without original exception
        >>> raise TUIDataError("Task data is corrupted")
    """

    def __init__(
        self,
        message: str,
        original_exception: Exception | None = None,
    ) -> None:
        """Initialize TUIDataError with message and optional original exception.

        Args:
            message: Human-readable error description
            original_exception: The underlying exception that caused this error
        """
        super().__init__(message)
        self.original_exception = original_exception

    def __str__(self) -> str:
        """Return string representation including original exception if present."""
        if self.original_exception:
            return f"{self.message} (caused by: {self.original_exception})"
        return self.message


class TUIRenderError(TUIError):
    """Raised when widget or component rendering fails.

    Use this exception when:
    - Widget initialization fails
    - Layout calculation fails
    - Component mounting or composition fails
    - Screen transition or navigation fails
    - Style or theme application fails

    Attributes:
        message: Human-readable error description
        component_name: Name of the component that failed to render

    Examples:
        >>> raise TUIRenderError(
        ...     "Failed to render task tree",
        ...     component_name="TaskTreeWidget"
        ... )

        >>> # In a widget class
        >>> try:
        ...     self.compose_tree()
        ... except Exception as e:
        ...     raise TUIRenderError(
        ...         f"Tree composition failed: {e}",
        ...         component_name=self.__class__.__name__
        ...     ) from e
    """

    def __init__(
        self,
        message: str,
        component_name: str,
    ) -> None:
        """Initialize TUIRenderError with message and component name.

        Args:
            message: Human-readable error description
            component_name: Name of the component that failed to render
        """
        super().__init__(message)
        self.component_name = component_name

    def __str__(self) -> str:
        """Return string representation including component name."""
        return f"[{self.component_name}] {self.message}"


class TUIConfigError(TUIError):
    """Raised for configuration-related issues.

    Use this exception when:
    - Required configuration is missing
    - Configuration value is invalid or out of range
    - Configuration file cannot be read or parsed
    - Environment variables are missing or invalid

    Attributes:
        message: Human-readable error description
        config_key: The configuration key that caused the error

    Examples:
        >>> raise TUIConfigError(
        ...     "Refresh interval must be positive",
        ...     config_key="refresh_interval"
        ... )

        >>> # When loading config
        >>> if not config.get("database_path"):
        ...     raise TUIConfigError(
        ...         "Database path is required",
        ...         config_key="database_path"
        ...     )
    """

    def __init__(
        self,
        message: str,
        config_key: str,
    ) -> None:
        """Initialize TUIConfigError with message and config key.

        Args:
            message: Human-readable error description
            config_key: The configuration key that caused the error
        """
        super().__init__(message)
        self.config_key = config_key

    def __str__(self) -> str:
        """Return string representation including config key."""
        return f"Configuration error for '{self.config_key}': {self.message}"
