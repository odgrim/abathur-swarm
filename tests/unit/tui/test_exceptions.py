"""Unit tests for TUI exception classes.

Tests verify:
- Exception instantiation with required and optional parameters
- Exception message formatting
- Exception inheritance hierarchy
- Exception chaining with 'from' syntax
- String representation (__str__) methods
"""

import pytest

from abathur.tui.exceptions import (
    TUIConfigError,
    TUIDataError,
    TUIError,
    TUIRenderError,
)


class TestTUIError:
    """Test cases for TUIError base exception."""

    def test_instantiation(self):
        """Test TUIError can be instantiated with a message."""
        error = TUIError("Test error message")
        assert error.message == "Test error message"
        assert str(error) == "Test error message"

    def test_inheritance(self):
        """Test TUIError inherits from Exception."""
        error = TUIError("Test error")
        assert isinstance(error, Exception)

    def test_can_be_raised(self):
        """Test TUIError can be raised and caught."""
        with pytest.raises(TUIError) as exc_info:
            raise TUIError("Test error")
        assert str(exc_info.value) == "Test error"

    def test_catches_subclasses(self):
        """Test TUIError can catch all TUI exception subclasses."""
        with pytest.raises(TUIError):
            raise TUIDataError("Data error")

        with pytest.raises(TUIError):
            raise TUIRenderError("Render error", component_name="TestWidget")

        with pytest.raises(TUIError):
            raise TUIConfigError("Config error", config_key="test_key")


class TestTUIDataError:
    """Test cases for TUIDataError exception."""

    def test_instantiation_without_original_exception(self):
        """Test TUIDataError can be instantiated with just a message."""
        error = TUIDataError("Failed to fetch tasks")
        assert error.message == "Failed to fetch tasks"
        assert error.original_exception is None
        assert str(error) == "Failed to fetch tasks"

    def test_instantiation_with_original_exception(self):
        """Test TUIDataError can include an original exception."""
        original = ValueError("Invalid data format")
        error = TUIDataError("Failed to parse tasks", original_exception=original)
        assert error.message == "Failed to parse tasks"
        assert error.original_exception is original
        assert "Failed to parse tasks" in str(error)
        assert "Invalid data format" in str(error)

    def test_inheritance(self):
        """Test TUIDataError inherits from TUIError."""
        error = TUIDataError("Test error")
        assert isinstance(error, TUIError)
        assert isinstance(error, Exception)

    def test_exception_chaining(self):
        """Test TUIDataError can be chained with 'from' syntax."""
        original = ConnectionError("Network timeout")
        try:
            try:
                raise original
            except ConnectionError as e:
                raise TUIDataError("Failed to fetch data", original_exception=e) from e
        except TUIDataError as caught:
            assert caught.original_exception is original
            assert caught.__cause__ is original

    def test_string_representation_formatting(self):
        """Test TUIDataError __str__ includes original exception details."""
        original = RuntimeError("Database connection lost")
        error = TUIDataError("Data fetch failed", original_exception=original)
        error_str = str(error)
        assert "Data fetch failed" in error_str
        assert "caused by:" in error_str
        assert "Database connection lost" in error_str


class TestTUIRenderError:
    """Test cases for TUIRenderError exception."""

    def test_instantiation(self):
        """Test TUIRenderError can be instantiated with message and component name."""
        error = TUIRenderError("Widget failed to mount", component_name="TaskTree")
        assert error.message == "Widget failed to mount"
        assert error.component_name == "TaskTree"

    def test_string_representation(self):
        """Test TUIRenderError __str__ includes component name."""
        error = TUIRenderError("Layout calculation failed", component_name="MainScreen")
        error_str = str(error)
        assert "[MainScreen]" in error_str
        assert "Layout calculation failed" in error_str

    def test_inheritance(self):
        """Test TUIRenderError inherits from TUIError."""
        error = TUIRenderError("Test error", component_name="TestWidget")
        assert isinstance(error, TUIError)
        assert isinstance(error, Exception)

    def test_exception_chaining(self):
        """Test TUIRenderError can be chained with 'from' syntax."""
        original = AttributeError("'NoneType' object has no attribute 'render'")
        try:
            try:
                raise original
            except AttributeError as e:
                raise TUIRenderError(
                    "Failed to render component", component_name="TaskList"
                ) from e
        except TUIRenderError as caught:
            assert caught.__cause__ is original
            assert caught.component_name == "TaskList"

    def test_component_name_in_message(self):
        """Test component name is properly formatted in error message."""
        error = TUIRenderError("Style application failed", component_name="StatusBar")
        assert str(error) == "[StatusBar] Style application failed"


class TestTUIConfigError:
    """Test cases for TUIConfigError exception."""

    def test_instantiation(self):
        """Test TUIConfigError can be instantiated with message and config key."""
        error = TUIConfigError("Value must be positive", config_key="refresh_interval")
        assert error.message == "Value must be positive"
        assert error.config_key == "refresh_interval"

    def test_string_representation(self):
        """Test TUIConfigError __str__ includes config key."""
        error = TUIConfigError("Missing required field", config_key="database_path")
        error_str = str(error)
        assert "database_path" in error_str
        assert "Missing required field" in error_str
        assert "Configuration error" in error_str

    def test_inheritance(self):
        """Test TUIConfigError inherits from TUIError."""
        error = TUIConfigError("Test error", config_key="test_key")
        assert isinstance(error, TUIError)
        assert isinstance(error, Exception)

    def test_exception_chaining(self):
        """Test TUIConfigError can be chained with 'from' syntax."""
        original = FileNotFoundError("Config file not found")
        try:
            try:
                raise original
            except FileNotFoundError as e:
                raise TUIConfigError(
                    "Failed to load configuration", config_key="config_file"
                ) from e
        except TUIConfigError as caught:
            assert caught.__cause__ is original
            assert caught.config_key == "config_file"

    def test_config_key_formatting(self):
        """Test config key is properly formatted in error message."""
        error = TUIConfigError("Invalid value type", config_key="max_workers")
        assert str(error) == "Configuration error for 'max_workers': Invalid value type"


class TestExceptionHierarchy:
    """Test cases for exception hierarchy and catching behavior."""

    def test_catch_all_with_base_exception(self):
        """Test catching TUIError catches all subclasses."""
        exceptions_to_test = [
            TUIDataError("data error"),
            TUIRenderError("render error", component_name="Widget"),
            TUIConfigError("config error", config_key="key"),
        ]

        for exc in exceptions_to_test:
            with pytest.raises(TUIError):
                raise exc

    def test_specific_exception_catching(self):
        """Test specific exception types can be caught individually."""
        with pytest.raises(TUIDataError):
            raise TUIDataError("data error")

        with pytest.raises(TUIRenderError):
            raise TUIRenderError("render error", component_name="Widget")

        with pytest.raises(TUIConfigError):
            raise TUIConfigError("config error", config_key="key")

    def test_multiple_exception_handlers(self):
        """Test multiple exception handlers work correctly."""

        def raise_data_error():
            raise TUIDataError("data error")

        def raise_render_error():
            raise TUIRenderError("render error", component_name="Widget")

        # Test specific handler is used
        try:
            raise_data_error()
        except TUIDataError as e:
            assert "data error" in str(e)
        except TUIError:
            pytest.fail("Should have caught TUIDataError specifically")

        # Test fallback to base handler
        try:
            raise_render_error()
        except TUIDataError:
            pytest.fail("Should not catch TUIDataError")
        except TUIError as e:
            assert "render error" in str(e)


class TestRealWorldScenarios:
    """Test cases for real-world usage scenarios."""

    def test_data_fetch_error_with_chaining(self):
        """Test typical data fetch error scenario with exception chaining."""

        async def fetch_tasks():
            """Simulate fetching tasks with error."""
            raise ConnectionError("Database unavailable")

        async def handle_fetch():
            """Simulate error handling wrapper."""
            try:
                await fetch_tasks()
            except ConnectionError as e:
                raise TUIDataError(
                    "Failed to fetch tasks from database", original_exception=e
                ) from e

        with pytest.raises(TUIDataError) as exc_info:
            import asyncio

            asyncio.run(handle_fetch())

        error = exc_info.value
        assert isinstance(error.original_exception, ConnectionError)
        assert "Failed to fetch tasks" in str(error)
        assert "Database unavailable" in str(error)

    def test_render_error_in_widget(self):
        """Test typical widget render error scenario."""

        class MockWidget:
            """Mock widget that fails to render."""

            def render(self):
                """Simulate rendering failure."""
                raise AttributeError("Missing required attribute")

        widget = MockWidget()

        with pytest.raises(TUIRenderError) as exc_info:
            try:
                widget.render()
            except AttributeError as e:
                raise TUIRenderError(
                    f"Widget render failed: {e}", component_name="MockWidget"
                ) from e

        error = exc_info.value
        assert error.component_name == "MockWidget"
        assert "[MockWidget]" in str(error)

    def test_config_validation_error(self):
        """Test typical configuration validation error scenario."""

        def validate_config(config: dict):
            """Validate configuration with specific rules."""
            if "refresh_interval" not in config:
                raise TUIConfigError(
                    "Missing required configuration", config_key="refresh_interval"
                )

            if config["refresh_interval"] <= 0:
                raise TUIConfigError(
                    "Must be a positive integer", config_key="refresh_interval"
                )

        with pytest.raises(TUIConfigError) as exc_info:
            validate_config({})

        assert exc_info.value.config_key == "refresh_interval"

        with pytest.raises(TUIConfigError) as exc_info:
            validate_config({"refresh_interval": -1})

        assert "positive integer" in str(exc_info.value)
