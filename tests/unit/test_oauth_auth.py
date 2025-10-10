"""Unit tests for OAuthAuthProvider."""

import asyncio
from datetime import datetime, timedelta, timezone
from typing import Any
from unittest.mock import AsyncMock, MagicMock, patch

import httpx
import pytest
from abathur.infrastructure.oauth_auth import OAuthAuthProvider


@pytest.fixture
def mock_config_manager() -> MagicMock:
    """Create a mock ConfigManager."""
    manager = MagicMock()
    manager.set_oauth_token = AsyncMock()
    return manager


@pytest.fixture
def valid_oauth_provider(mock_config_manager: MagicMock) -> OAuthAuthProvider:
    """Create a valid OAuthAuthProvider with future expiry."""
    expires_at = datetime.now(timezone.utc) + timedelta(hours=1)
    return OAuthAuthProvider(
        access_token="test_access_token",
        refresh_token="test_refresh_token",
        expires_at=expires_at,
        config_manager=mock_config_manager,
    )


@pytest.fixture
def near_expiry_provider(mock_config_manager: MagicMock) -> OAuthAuthProvider:
    """Create OAuthAuthProvider with token expiring in 3 minutes."""
    expires_at = datetime.now(timezone.utc) + timedelta(minutes=3)
    return OAuthAuthProvider(
        access_token="test_access_token",
        refresh_token="test_refresh_token",
        expires_at=expires_at,
        config_manager=mock_config_manager,
    )


@pytest.fixture
def expired_provider(mock_config_manager: MagicMock) -> OAuthAuthProvider:
    """Create OAuthAuthProvider with expired token."""
    expires_at = datetime.now(timezone.utc) - timedelta(hours=1)
    return OAuthAuthProvider(
        access_token="test_access_token",
        refresh_token="test_refresh_token",
        expires_at=expires_at,
        config_manager=mock_config_manager,
    )


class TestOAuthAuthProviderInit:
    """Test OAuthAuthProvider initialization."""

    def test_init_with_timezone_aware_datetime(self, mock_config_manager: MagicMock) -> None:
        """Test initialization with timezone-aware datetime."""
        expires_at = datetime.now(timezone.utc) + timedelta(hours=1)
        provider = OAuthAuthProvider(
            access_token="test_token",
            refresh_token="test_refresh",
            expires_at=expires_at,
            config_manager=mock_config_manager,
        )

        assert provider.access_token == "test_token"
        assert provider.refresh_token == "test_refresh"
        assert provider.expires_at == expires_at
        assert provider.expires_at.tzinfo is not None

    def test_init_with_naive_datetime_assumes_utc(self, mock_config_manager: MagicMock) -> None:
        """Test that naive datetime is converted to UTC."""
        expires_at_naive = datetime.now() + timedelta(hours=1)
        provider = OAuthAuthProvider(
            access_token="test_token",
            refresh_token="test_refresh",
            expires_at=expires_at_naive,
            config_manager=mock_config_manager,
        )

        assert provider.expires_at.tzinfo == timezone.utc


class TestOAuthAuthProviderMethods:
    """Test OAuthAuthProvider basic methods."""

    @pytest.mark.asyncio
    async def test_get_credentials_returns_bearer_type(
        self, valid_oauth_provider: OAuthAuthProvider
    ) -> None:
        """Test that get_credentials returns bearer type."""
        credentials = await valid_oauth_provider.get_credentials()

        assert credentials["type"] == "bearer"
        assert credentials["value"] == "test_access_token"
        assert "expires_at" in credentials

    def test_is_valid_returns_true_for_valid_token(
        self, valid_oauth_provider: OAuthAuthProvider
    ) -> None:
        """Test that is_valid returns True for non-expired token."""
        assert valid_oauth_provider.is_valid() is True

    def test_is_valid_returns_false_for_expired_token(
        self, expired_provider: OAuthAuthProvider
    ) -> None:
        """Test that is_valid returns False for expired token."""
        assert expired_provider.is_valid() is False

    def test_get_auth_method_returns_oauth(self, valid_oauth_provider: OAuthAuthProvider) -> None:
        """Test that get_auth_method returns 'oauth'."""
        assert valid_oauth_provider.get_auth_method() == "oauth"

    def test_get_context_limit_returns_200k(self, valid_oauth_provider: OAuthAuthProvider) -> None:
        """Test that OAuth context limit is 200K tokens."""
        assert valid_oauth_provider.get_context_limit() == 200_000

    def test_is_expired_detects_expired_token(self, expired_provider: OAuthAuthProvider) -> None:
        """Test _is_expired correctly identifies expired tokens."""
        assert expired_provider._is_expired() is True

    def test_is_near_expiry_detects_soon_expiring_token(
        self, near_expiry_provider: OAuthAuthProvider
    ) -> None:
        """Test _is_near_expiry detects tokens expiring within buffer."""
        assert near_expiry_provider._is_near_expiry() is True

    def test_is_near_expiry_false_for_distant_expiry(
        self, valid_oauth_provider: OAuthAuthProvider
    ) -> None:
        """Test _is_near_expiry returns False for tokens expiring later."""
        assert valid_oauth_provider._is_near_expiry() is False


class TestOAuthProactiveRefresh:
    """Test proactive token refresh functionality."""

    @pytest.mark.asyncio
    async def test_get_credentials_triggers_proactive_refresh(
        self, near_expiry_provider: OAuthAuthProvider, mock_config_manager: MagicMock
    ) -> None:
        """Test that get_credentials triggers proactive refresh when near expiry."""
        # Mock successful refresh
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

            credentials = await near_expiry_provider.get_credentials()

            # Verify new token returned
            assert credentials["value"] == "new_access_token"
            # Verify tokens were persisted
            assert mock_config_manager.set_oauth_token.called

    @pytest.mark.asyncio
    async def test_no_proactive_refresh_for_distant_expiry(
        self, valid_oauth_provider: OAuthAuthProvider
    ) -> None:
        """Test that no refresh occurs for tokens with distant expiry."""
        # Don't mock httpx - if it tries to make a request, test will fail
        credentials = await valid_oauth_provider.get_credentials()

        # Should return existing token without refresh
        assert credentials["value"] == "test_access_token"


class TestOAuthTokenRefresh:
    """Test token refresh logic and retry mechanisms."""

    @pytest.mark.asyncio
    async def test_refresh_credentials_success(
        self, near_expiry_provider: OAuthAuthProvider, mock_config_manager: MagicMock
    ) -> None:
        """Test successful token refresh."""
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

            result = await near_expiry_provider.refresh_credentials()

            assert result is True
            assert near_expiry_provider.access_token == "new_access_token"
            assert near_expiry_provider.refresh_token == "new_refresh_token"
            assert mock_config_manager.set_oauth_token.called

    @pytest.mark.asyncio
    async def test_refresh_handles_token_rotation(
        self, near_expiry_provider: OAuthAuthProvider, mock_config_manager: MagicMock
    ) -> None:
        """Test that refresh handles refresh token rotation."""
        with patch("httpx.AsyncClient") as mock_client:
            mock_response = MagicMock()
            mock_response.status_code = 200
            # Server returns new refresh token
            mock_response.json.return_value = {
                "access_token": "new_access_token",
                "refresh_token": "rotated_refresh_token",
                "expires_in": 3600,
            }
            mock_response.raise_for_status = MagicMock()

            mock_client.return_value.__aenter__.return_value.post = AsyncMock(
                return_value=mock_response
            )

            await near_expiry_provider.refresh_credentials()

            # Verify both tokens updated
            assert near_expiry_provider.access_token == "new_access_token"
            assert near_expiry_provider.refresh_token == "rotated_refresh_token"

    @pytest.mark.asyncio
    async def test_refresh_401_returns_false(self, near_expiry_provider: OAuthAuthProvider) -> None:
        """Test that 401 on refresh returns False (no retry)."""
        with patch("httpx.AsyncClient") as mock_client:
            mock_response = MagicMock()
            mock_response.status_code = 401

            mock_client.return_value.__aenter__.return_value.post = AsyncMock(
                return_value=mock_response
            )

            result = await near_expiry_provider.refresh_credentials()

            assert result is False

    @pytest.mark.asyncio
    async def test_refresh_retries_on_429(self, near_expiry_provider: OAuthAuthProvider) -> None:
        """Test that refresh retries on 429 with Retry-After."""
        with patch("httpx.AsyncClient") as mock_client, patch("asyncio.sleep") as mock_sleep:
            # First call returns 429, second succeeds
            mock_429 = MagicMock()
            mock_429.status_code = 429
            mock_429.headers = {"Retry-After": "5"}

            mock_success = MagicMock()
            mock_success.status_code = 200
            mock_success.json.return_value = {
                "access_token": "new_token",
                "expires_in": 3600,
            }
            mock_success.raise_for_status = MagicMock()

            # Create HTTPStatusError for 429
            error = httpx.HTTPStatusError("429", request=MagicMock(), response=mock_429)

            # First call raises 429, second succeeds
            post_mock = AsyncMock(side_effect=[error, mock_success])
            mock_client.return_value.__aenter__.return_value.post = post_mock

            result = await near_expiry_provider.refresh_credentials()

            # Verify retry occurred
            assert result is True
            assert post_mock.call_count == 2
            mock_sleep.assert_called_once_with(5)

    @pytest.mark.asyncio
    async def test_refresh_max_retries_exceeded(
        self, near_expiry_provider: OAuthAuthProvider
    ) -> None:
        """Test that refresh gives up after 3 failed attempts."""
        with patch("httpx.AsyncClient") as mock_client, patch("asyncio.sleep"):
            # All attempts return 500
            error = httpx.HTTPStatusError(
                "500", request=MagicMock(), response=MagicMock(status_code=500)
            )
            mock_client.return_value.__aenter__.return_value.post = AsyncMock(side_effect=error)

            result = await near_expiry_provider.refresh_credentials()

            assert result is False


class TestOAuthConcurrency:
    """Test concurrent refresh handling."""

    @pytest.mark.asyncio
    async def test_concurrent_refresh_only_refreshes_once(
        self, near_expiry_provider: OAuthAuthProvider, mock_config_manager: MagicMock
    ) -> None:
        """Test that concurrent refresh requests only refresh once."""
        refresh_count = 0

        async def mock_refresh(*args: Any, **kwargs: Any) -> MagicMock:
            nonlocal refresh_count
            refresh_count += 1
            await asyncio.sleep(0.1)  # Simulate network delay
            return MagicMock(
                status_code=200,
                json=lambda: {"access_token": "new_token", "expires_in": 3600},
                raise_for_status=MagicMock(),
            )

        with patch("httpx.AsyncClient") as mock_client:
            mock_client.return_value.__aenter__.return_value.post = mock_refresh

            # Trigger multiple concurrent refreshes
            results = await asyncio.gather(
                *[near_expiry_provider.refresh_credentials() for _ in range(5)]
            )

            # All should succeed
            assert all(results)
            # But only one actual refresh should occur (due to lock)
            assert refresh_count == 1
