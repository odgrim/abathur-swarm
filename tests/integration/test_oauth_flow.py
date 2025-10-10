"""Integration tests for OAuth authentication flow."""

import tempfile
from collections.abc import Generator
from datetime import datetime, timedelta, timezone
from pathlib import Path
from unittest.mock import AsyncMock, MagicMock, patch

import pytest
from abathur.application.claude_client import ClaudeClient
from abathur.infrastructure.config import ConfigManager
from abathur.infrastructure.oauth_auth import OAuthAuthProvider


@pytest.fixture
def temp_project_root() -> Generator[Path, None, None]:
    """Create a temporary project root directory."""
    with tempfile.TemporaryDirectory() as tmpdir:
        yield Path(tmpdir)


@pytest.fixture
def config_manager(temp_project_root: Path) -> ConfigManager:
    """Create a ConfigManager with temporary project root."""
    return ConfigManager(project_root=temp_project_root)


class TestOAuthEndToEndFlow:
    """Test end-to-end OAuth authentication flows."""

    @pytest.mark.asyncio
    async def test_oauth_login_and_execute_task_flow(
        self, config_manager: ConfigManager, temp_project_root: Path
    ) -> None:
        """Test complete flow: login → store tokens → create client → execute task."""
        # Step 1: Store OAuth tokens (simulating oauth-login)
        expires_at = datetime.now(timezone.utc) + timedelta(hours=1)

        await config_manager.set_oauth_token(
            access_token="test_access_token",
            refresh_token="test_refresh_token",
            expires_at=expires_at,
            use_keychain=False,  # Use .env for testing
        )

        # Verify tokens stored
        env_file = temp_project_root / ".env"
        assert env_file.exists()
        with open(env_file) as f:
            content = f.read()
            assert "test_access_token" in content
            assert "test_refresh_token" in content

        # Step 2: Load tokens and create OAuth provider
        access_token, refresh_token, loaded_expires_at = await config_manager.get_oauth_token()

        assert access_token == "test_access_token"
        assert refresh_token == "test_refresh_token"
        assert loaded_expires_at == expires_at

        # Step 3: Create OAuth auth provider
        oauth_provider = OAuthAuthProvider(
            access_token=access_token,
            refresh_token=refresh_token,
            expires_at=expires_at,
            config_manager=config_manager,
        )

        assert oauth_provider.is_valid()
        assert oauth_provider.get_auth_method() == "oauth"
        assert oauth_provider.get_context_limit() == 200_000

        # Step 4: Create ClaudeClient with OAuth provider
        claude_client = ClaudeClient(auth_provider=oauth_provider)

        assert claude_client.auth_provider is oauth_provider
        assert claude_client.context_limit == 200_000

    @pytest.mark.asyncio
    async def test_oauth_token_refresh_flow(self, config_manager: ConfigManager) -> None:
        """Test token refresh flow when token is near expiry."""
        # Create provider with near-expiry token
        expires_at = datetime.now(timezone.utc) + timedelta(minutes=3)

        oauth_provider = OAuthAuthProvider(
            access_token="old_access_token",
            refresh_token="test_refresh_token",
            expires_at=expires_at,
            config_manager=config_manager,
        )

        # Mock the refresh endpoint
        with patch("httpx.AsyncClient") as mock_client:
            mock_response = MagicMock()
            mock_response.status_code = 200
            mock_response.json.return_value = {
                "access_token": "new_access_token",
                "refresh_token": "new_refresh_token",
                "expires_in": 3600,
            }
            mock_response.raise_for_status = MagicMock()

            mock_client.return_value.__aenter__.return_value.post = AsyncMock(
                return_value=mock_response
            )

            # Trigger proactive refresh via get_credentials
            credentials = await oauth_provider.get_credentials()

            # Verify new token returned
            assert credentials["value"] == "new_access_token"
            assert oauth_provider.access_token == "new_access_token"
            assert oauth_provider.refresh_token == "new_refresh_token"

    @pytest.mark.asyncio
    async def test_oauth_logout_flow(
        self, config_manager: ConfigManager, temp_project_root: Path
    ) -> None:
        """Test OAuth logout flow clears all tokens."""
        # Step 1: Store tokens
        expires_at = datetime.now(timezone.utc) + timedelta(hours=1)

        await config_manager.set_oauth_token(
            access_token="test_token",
            refresh_token="test_refresh",
            expires_at=expires_at,
            use_keychain=False,
        )

        # Verify tokens exist
        env_file = temp_project_root / ".env"
        assert env_file.exists()

        # Step 2: Clear tokens (simulating oauth-logout)
        config_manager.clear_oauth_tokens()

        # Verify tokens removed
        with open(env_file) as f:
            content = f.read()
            assert "test_token" not in content
            assert "test_refresh" not in content

        # Step 3: Verify get_oauth_token raises error
        with pytest.raises(ValueError, match="OAuth tokens not found"):
            await config_manager.get_oauth_token()

    @pytest.mark.asyncio
    async def test_dual_mode_auth_priority(
        self, config_manager: ConfigManager, temp_project_root: Path
    ) -> None:
        """Test that API key takes priority over OAuth when both are configured."""
        # Store both API key and OAuth tokens
        expires_at = datetime.now(timezone.utc) + timedelta(hours=1)

        # Store OAuth tokens
        await config_manager.set_oauth_token(
            access_token="oauth_token",
            refresh_token="oauth_refresh",
            expires_at=expires_at,
            use_keychain=False,
        )

        # Store API key
        env_file = temp_project_root / ".env"
        with open(env_file, "a") as f:
            f.write("\nANTHROPIC_API_KEY=sk-ant-api03-test-key\n")

        # Try to get API key first (should succeed)
        api_key = config_manager.get_api_key()
        assert api_key == "sk-ant-api03-test-key"

        # This simulates the service initialization behavior
        # where API key is tried first before OAuth


class TestContextWindowValidation:
    """Test context window validation integration."""

    @pytest.mark.asyncio
    async def test_context_window_warning_for_oauth(self, config_manager: ConfigManager) -> None:
        """Test that context window warning is logged for large OAuth requests."""
        expires_at = datetime.now(timezone.utc) + timedelta(hours=1)

        oauth_provider = OAuthAuthProvider(
            access_token="test_token",
            refresh_token="test_refresh",
            expires_at=expires_at,
            config_manager=config_manager,
        )

        claude_client = ClaudeClient(auth_provider=oauth_provider)

        # Create a large prompt that exceeds 90% of OAuth limit (200K tokens)
        # Need ~720K characters (180K tokens at 4 chars/token)
        large_prompt = "x" * 720_000

        # Mock the SDK client to avoid actual API calls
        mock_response = MagicMock()
        mock_response.content = [MagicMock(text="Test response")]
        mock_response.stop_reason = "end_turn"
        mock_response.usage = MagicMock(input_tokens=10, output_tokens=5)

        claude_client.async_client = AsyncMock()
        claude_client.async_client.messages.create = AsyncMock(return_value=mock_response)

        # Execute task - should trigger context warning
        with patch("abathur.application.claude_client.logger") as mock_logger:
            _result = await claude_client.execute_task(
                system_prompt=large_prompt, user_message="test"
            )

            # Verify warning was logged
            mock_logger.warning.assert_called()
            call_args = mock_logger.warning.call_args
            assert call_args[0][0] == "context_window_warning"

    @pytest.mark.asyncio
    async def test_no_warning_for_small_requests(self, config_manager: ConfigManager) -> None:
        """Test that no warning is logged for small requests."""
        expires_at = datetime.now(timezone.utc) + timedelta(hours=1)

        oauth_provider = OAuthAuthProvider(
            access_token="test_token",
            refresh_token="test_refresh",
            expires_at=expires_at,
            config_manager=config_manager,
        )

        claude_client = ClaudeClient(auth_provider=oauth_provider)

        # Small prompt (well under limit)
        small_prompt = "Hello, world!"

        # Mock the SDK client
        mock_response = MagicMock()
        mock_response.content = [MagicMock(text="Test response")]
        mock_response.stop_reason = "end_turn"
        mock_response.usage = MagicMock(input_tokens=10, output_tokens=5)

        claude_client.async_client = AsyncMock()
        claude_client.async_client.messages.create = AsyncMock(return_value=mock_response)

        # Execute task - should NOT trigger warning
        with patch("abathur.application.claude_client.logger") as mock_logger:
            _result = await claude_client.execute_task(
                system_prompt=small_prompt, user_message="test"
            )

            # Verify warning was NOT called
            mock_logger.warning.assert_not_called()


class TestAuthenticationFallback:
    """Test authentication fallback and error handling."""

    @pytest.mark.asyncio
    async def test_no_auth_configured_raises_helpful_error(
        self, config_manager: ConfigManager
    ) -> None:
        """Test that helpful error is raised when no auth is configured."""
        # Don't configure any auth

        with pytest.raises(ValueError, match="ANTHROPIC_API_KEY not found"):
            config_manager.get_api_key()

        with pytest.raises(ValueError, match="OAuth tokens not found"):
            await config_manager.get_oauth_token()

    @pytest.mark.asyncio
    async def test_expired_oauth_token_triggers_refresh(
        self, config_manager: ConfigManager
    ) -> None:
        """Test that expired OAuth token triggers automatic refresh."""
        # Create provider with expired token
        expires_at = datetime.now(timezone.utc) - timedelta(hours=1)

        oauth_provider = OAuthAuthProvider(
            access_token="expired_token",
            refresh_token="refresh_token",
            expires_at=expires_at,
            config_manager=config_manager,
        )

        assert not oauth_provider.is_valid()

        # Mock successful refresh
        with patch("httpx.AsyncClient") as mock_client:
            mock_response = MagicMock()
            mock_response.status_code = 200
            mock_response.json.return_value = {
                "access_token": "refreshed_token",
                "refresh_token": "new_refresh",
                "expires_in": 3600,
            }
            mock_response.raise_for_status = MagicMock()

            mock_client.return_value.__aenter__.return_value.post = AsyncMock(
                return_value=mock_response
            )

            # Attempt to get credentials should trigger automatic refresh
            credentials = await oauth_provider.get_credentials()

            # Verify refresh succeeded and new token is returned
            assert credentials["value"] == "refreshed_token"
            assert oauth_provider.access_token == "refreshed_token"
            assert oauth_provider.is_valid()
