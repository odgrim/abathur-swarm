"""Unit tests for OAuthAuthProvider."""

from datetime import datetime, timedelta, timezone
from unittest.mock import MagicMock

import pytest
from abathur.infrastructure.oauth_auth import OAuthAuthProvider


@pytest.fixture
def mock_config_manager() -> MagicMock:
    """Create a mock ConfigManager."""
    return MagicMock()


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
        """Test that is_valid returns True when token exists."""
        assert valid_oauth_provider.is_valid() is True

    def test_is_valid_returns_true_for_expired_token(
        self, expired_provider: OAuthAuthProvider
    ) -> None:
        """Test that is_valid returns True even for expired tokens.

        Token expiry checking is delegated to Claude Code, so we only
        check if the token exists, not if it's expired.
        """
        assert expired_provider.is_valid() is True

    def test_is_valid_returns_false_for_empty_token(self, mock_config_manager: MagicMock) -> None:
        """Test that is_valid returns False when token is empty."""
        provider = OAuthAuthProvider(
            access_token="",
            refresh_token="test_refresh",
            expires_at=datetime.now(timezone.utc) + timedelta(hours=1),
            config_manager=mock_config_manager,
        )
        assert provider.is_valid() is False

    def test_get_auth_method_returns_oauth(self, valid_oauth_provider: OAuthAuthProvider) -> None:
        """Test that get_auth_method returns 'oauth'."""
        assert valid_oauth_provider.get_auth_method() == "oauth"

    def test_get_context_limit_returns_200k(self, valid_oauth_provider: OAuthAuthProvider) -> None:
        """Test that OAuth context limit is 200K tokens."""
        assert valid_oauth_provider.get_context_limit() == 200_000

    @pytest.mark.asyncio
    async def test_refresh_credentials_is_noop(
        self, valid_oauth_provider: OAuthAuthProvider
    ) -> None:
        """Test that refresh_credentials is a no-op and always returns True."""
        result = await valid_oauth_provider.refresh_credentials()
        assert result is True

    @pytest.mark.asyncio
    async def test_refresh_credentials_with_force_is_noop(
        self, valid_oauth_provider: OAuthAuthProvider
    ) -> None:
        """Test that refresh_credentials with force=True is still a no-op."""
        result = await valid_oauth_provider.refresh_credentials(force=True)
        assert result is True
