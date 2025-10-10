"""Unit tests for custom exception hierarchy."""

from abathur.infrastructure.exceptions import (
    AbathurError,
    APIKeyInvalidError,
    AuthenticationError,
    ContextWindowExceededError,
)


class TestExceptionHierarchy:
    """Test custom exception hierarchy."""

    def test_abathur_error_is_base_exception(self) -> None:
        """Test that AbathurError is the base exception."""
        error = AbathurError("Test error")
        assert isinstance(error, Exception)
        assert str(error) == "Test error"

    def test_authentication_error_inherits_from_abathur_error(self) -> None:
        """Test that AuthenticationError inherits from AbathurError."""
        error = AuthenticationError("Auth failed")
        assert isinstance(error, AbathurError)

    def test_authentication_error_with_remediation(self) -> None:
        """Test AuthenticationError with remediation message."""
        error = AuthenticationError("Auth failed", remediation="Run: abathur config")

        assert error.remediation == "Run: abathur config"
        assert "Remediation:" in str(error)
        assert "Run: abathur config" in str(error)

    def test_authentication_error_without_remediation(self) -> None:
        """Test AuthenticationError without remediation message."""
        error = AuthenticationError("Auth failed")

        assert error.remediation is None
        assert "Remediation:" not in str(error)

    def test_api_key_invalid_error_has_default_message(self) -> None:
        """Test APIKeyInvalidError has appropriate default message."""
        error = APIKeyInvalidError()

        assert "API key invalid" in str(error)
        assert "console.anthropic.com" in str(error)
        assert isinstance(error, AuthenticationError)

    def test_context_window_exceeded_error_attributes(self) -> None:
        """Test ContextWindowExceededError stores attributes correctly."""
        error = ContextWindowExceededError(tokens=210_000, limit=200_000, auth_method="claude_cli")

        assert error.tokens == 210_000
        assert error.limit == 200_000
        assert error.auth_method == "claude_cli"
        assert "210,000 tokens" in str(error)
        assert "200,000 tokens" in str(error)
        assert "claude_cli" in str(error)
        assert isinstance(error, AbathurError)

    def test_api_key_invalid_error_has_remediation(self) -> None:
        """Test that APIKeyInvalidError provides remediation."""
        error = APIKeyInvalidError()

        assert error.remediation is not None
        assert len(error.remediation) > 0
