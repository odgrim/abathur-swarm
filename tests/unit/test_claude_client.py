"""Unit tests for ClaudeClient with AuthProvider integration."""

import asyncio
import os
from unittest.mock import AsyncMock, MagicMock, patch

import pytest
from abathur.application.claude_client import ClaudeClient
from abathur.infrastructure.api_key_auth import APIKeyAuthProvider
from abathur.infrastructure.exceptions import APIKeyInvalidError


class TestClaudeClientBackwardCompatibility:
    """Test backward compatibility with API key parameter."""

    def test_init_with_api_key_parameter(self) -> None:
        """Test that API key parameter still works (backward compatibility)."""
        client = ClaudeClient(api_key="sk-ant-api03-test-key")

        assert client.auth_provider is not None
        assert client.auth_provider.get_auth_method() == "api_key"
        assert client.context_limit == 1_000_000

    def test_init_with_env_var_fallback(self) -> None:
        """Test that ANTHROPIC_API_KEY environment variable is used as fallback."""
        with patch.dict(os.environ, {"ANTHROPIC_API_KEY": "sk-ant-api03-env-key"}):
            client = ClaudeClient()

            assert client.auth_provider is not None
            assert client.auth_provider.get_auth_method() == "api_key"

    def test_init_raises_without_auth(self) -> None:
        """Test that initialization raises without any authentication."""
        with patch.dict(os.environ, {}, clear=True):
            with pytest.raises(ValueError, match="Authentication required"):
                ClaudeClient()

    def test_init_with_invalid_api_key(self) -> None:
        """Test that invalid API key raises error during initialization."""
        with pytest.raises(APIKeyInvalidError):
            ClaudeClient(api_key="invalid-key")


class TestClaudeClientWithAuthProvider:
    """Test ClaudeClient with AuthProvider parameter."""

    def test_init_with_auth_provider(self) -> None:
        """Test initialization with explicit AuthProvider."""
        provider = APIKeyAuthProvider("sk-ant-api03-test-key")
        client = ClaudeClient(auth_provider=provider)

        assert client.auth_provider is provider
        assert client.context_limit == 1_000_000

    def test_auth_provider_takes_precedence_over_api_key(self) -> None:
        """Test that auth_provider parameter takes precedence over api_key."""
        provider = APIKeyAuthProvider("sk-ant-api03-provider-key")
        client = ClaudeClient(api_key="sk-ant-api03-param-key", auth_provider=provider)

        assert client.auth_provider is provider
        # Verify the provider key is used, not the param key
        credentials = asyncio.run(provider.get_credentials())
        assert credentials["value"] == "sk-ant-api03-provider-key"

    @pytest.mark.asyncio
    async def test_configure_sdk_auth_sets_api_key_env_var(self) -> None:
        """Test that _configure_sdk_auth sets ANTHROPIC_API_KEY for API key auth."""
        provider = APIKeyAuthProvider("sk-ant-api03-test-key")
        client = ClaudeClient(auth_provider=provider)

        # Clear existing env vars
        for var in ["ANTHROPIC_API_KEY", "ANTHROPIC_AUTH_TOKEN"]:
            os.environ.pop(var, None)

        await client._configure_sdk_auth()

        assert os.environ.get("ANTHROPIC_API_KEY") == "sk-ant-api03-test-key"
        assert "ANTHROPIC_AUTH_TOKEN" not in os.environ

    @pytest.mark.asyncio
    async def test_execute_task_calls_configure_sdk_auth(self) -> None:
        """Test that execute_task configures SDK auth before making request."""
        provider = APIKeyAuthProvider("sk-ant-api03-test-key")
        client = ClaudeClient(auth_provider=provider)

        # Mock the async_client to avoid actual API calls
        mock_response = MagicMock()
        mock_response.content = [MagicMock(text="Test response")]
        mock_response.stop_reason = "end_turn"
        mock_response.usage = MagicMock(input_tokens=10, output_tokens=5)

        client.async_client = AsyncMock()
        client.async_client.messages.create = AsyncMock(return_value=mock_response)

        result = await client.execute_task(system_prompt="Test system", user_message="Test message")

        assert result["success"] is True
        assert client.async_client.messages.create.called

    def test_context_limit_set_from_auth_provider(self) -> None:
        """Test that context limit is set from AuthProvider."""
        provider = APIKeyAuthProvider("sk-ant-api03-test-key")
        client = ClaudeClient(auth_provider=provider)

        assert client.context_limit == provider.get_context_limit()
        assert client.context_limit == 1_000_000


class TestClaudeClientMigration:
    """Test migration from old to new API."""

    def test_old_style_initialization_still_works(self) -> None:
        """Test that old-style API key initialization still works."""
        # This is how users currently initialize ClaudeClient
        client = ClaudeClient(api_key="sk-ant-api03-test-key")

        # Should work without any changes
        assert client is not None
        assert client.auth_provider.get_auth_method() == "api_key"

    def test_new_style_initialization(self) -> None:
        """Test new-style initialization with AuthProvider."""
        # This is the new way to initialize ClaudeClient
        provider = APIKeyAuthProvider("sk-ant-api03-test-key")
        client = ClaudeClient(auth_provider=provider)

        assert client is not None
        assert client.auth_provider is provider
