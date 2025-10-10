"""Unit tests for AuthProvider interface and implementations."""

import pytest
from abathur.infrastructure.api_key_auth import APIKeyAuthProvider
from abathur.infrastructure.exceptions import APIKeyInvalidError


class TestAPIKeyAuthProvider:
    """Test cases for APIKeyAuthProvider."""

    @pytest.mark.asyncio
    async def test_get_credentials_returns_api_key_type(self) -> None:
        """Test that get_credentials returns correct type and value."""
        api_key = "sk-ant-api03-test-key"
        provider = APIKeyAuthProvider(api_key)

        credentials = await provider.get_credentials()

        assert credentials["type"] == "api_key"
        assert credentials["value"] == api_key

    @pytest.mark.asyncio
    async def test_refresh_credentials_always_succeeds(self) -> None:
        """Test that API keys don't need refresh (always returns True)."""
        provider = APIKeyAuthProvider("sk-ant-api03-test-key")

        result = await provider.refresh_credentials()

        assert result is True

    def test_is_valid_returns_true_for_valid_key(self) -> None:
        """Test that is_valid returns True for valid API key."""
        provider = APIKeyAuthProvider("sk-ant-api03-test-key")

        assert provider.is_valid() is True

    def test_get_auth_method_returns_api_key(self) -> None:
        """Test that get_auth_method returns 'api_key'."""
        provider = APIKeyAuthProvider("sk-ant-api03-test-key")

        assert provider.get_auth_method() == "api_key"

    def test_get_context_limit_returns_1_million(self) -> None:
        """Test that API key context limit is 1M tokens."""
        provider = APIKeyAuthProvider("sk-ant-api03-test-key")

        assert provider.get_context_limit() == 1_000_000

    def test_init_raises_on_empty_key(self) -> None:
        """Test that empty API key raises APIKeyInvalidError."""
        with pytest.raises(APIKeyInvalidError, match="API key cannot be empty"):
            APIKeyAuthProvider("")

    def test_init_raises_on_invalid_prefix(self) -> None:
        """Test that invalid API key prefix raises APIKeyInvalidError."""
        with pytest.raises(APIKeyInvalidError, match="Invalid API key format"):
            APIKeyAuthProvider("invalid-key-format")

    def test_init_accepts_valid_prefix(self) -> None:
        """Test that valid API key prefix is accepted."""
        valid_keys = [
            "sk-ant-api03-test",
            "sk-ant-api-test",
            "sk-ant-api01-test",
        ]
        for key in valid_keys:
            provider = APIKeyAuthProvider(key)
            assert provider.api_key == key
