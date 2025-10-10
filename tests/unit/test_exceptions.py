"""Unit tests for custom exception hierarchy."""

from abathur.infrastructure.exceptions import (
    AbathurError,
    APIKeyInvalidError,
    AuthenticationError,
    ContextWindowExceededError,
    OAuthRefreshError,
    OAuthTokenExpiredError,
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

    def test_oauth_token_expired_error_has_default_message(self) -> None:
        """Test OAuthTokenExpiredError has appropriate default message."""
        error = OAuthTokenExpiredError()

        assert "OAuth token expired" in str(error)
        assert "abathur config oauth-login" in str(error)
        assert isinstance(error, AuthenticationError)

    def test_oauth_token_expired_error_custom_message(self) -> None:
        """Test OAuthTokenExpiredError with custom message."""
        error = OAuthTokenExpiredError("Custom expired message")

        assert "Custom expired message" in str(error)
        assert "abathur config oauth-login" in str(error)

    def test_oauth_refresh_error_has_default_message(self) -> None:
        """Test OAuthRefreshError has appropriate default message."""
        error = OAuthRefreshError()

        assert "Token refresh failed" in str(error)
        assert "Check network connection" in str(error)
        assert isinstance(error, AuthenticationError)

    def test_api_key_invalid_error_has_default_message(self) -> None:
        """Test APIKeyInvalidError has appropriate default message."""
        error = APIKeyInvalidError()

        assert "API key invalid" in str(error)
        assert "console.anthropic.com" in str(error)
        assert isinstance(error, AuthenticationError)

    def test_context_window_exceeded_error_attributes(self) -> None:
        """Test ContextWindowExceededError stores attributes correctly."""
        error = ContextWindowExceededError(tokens=210_000, limit=200_000, auth_method="oauth")

        assert error.tokens == 210_000
        assert error.limit == 200_000
        assert error.auth_method == "oauth"
        assert "210,000 tokens" in str(error)
        assert "200,000 tokens" in str(error)
        assert "oauth" in str(error)
        assert isinstance(error, AbathurError)

    def test_all_auth_errors_have_remediation(self) -> None:
        """Test that all authentication errors provide remediation."""
        errors = [
            OAuthTokenExpiredError(),
            OAuthRefreshError(),
            APIKeyInvalidError(),
        ]

        for error in errors:
            assert error.remediation is not None
            assert len(error.remediation) > 0
