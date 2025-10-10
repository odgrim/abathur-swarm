"""OAuth authentication provider for Claude Code integration."""

from datetime import datetime, timezone
from typing import TYPE_CHECKING, Literal

from abathur.domain.ports.auth_provider import AuthProvider
from abathur.infrastructure.logger import get_logger

if TYPE_CHECKING:
    from abathur.infrastructure.config import ConfigManager

logger = get_logger(__name__)


class OAuthAuthProvider(AuthProvider):
    """OAuth authentication provider for Claude Code integration.

    This provider manages OAuth tokens obtained from Claude Code.
    Token refresh is handled by Claude Code itself - this provider
    simply uses the tokens as-is.

    Attributes:
        access_token: Current OAuth access token
        refresh_token: OAuth refresh token (stored but not used for refresh)
        expires_at: Token expiry timestamp (UTC)
        config_manager: ConfigManager instance for token persistence
    """

    def __init__(
        self,
        access_token: str,
        refresh_token: str,
        expires_at: datetime,
        config_manager: "ConfigManager",
    ):
        """Initialize OAuth authentication provider.

        Args:
            access_token: OAuth access token
            refresh_token: OAuth refresh token
            expires_at: Token expiry timestamp (must be timezone-aware UTC)
            config_manager: ConfigManager instance for token persistence
        """
        self.access_token = access_token
        self.refresh_token = refresh_token
        self.expires_at = expires_at
        self.config_manager = config_manager

        # Ensure expires_at is timezone-aware UTC
        if self.expires_at.tzinfo is None:
            logger.warning(
                "oauth_expires_at_no_timezone",
                message="expires_at has no timezone, assuming UTC",
            )
            self.expires_at = self.expires_at.replace(tzinfo=timezone.utc)

    async def get_credentials(self) -> dict[str, str]:
        """Get OAuth credentials.

        Returns the current access token without any refresh logic.
        Token refresh is handled externally by Claude Code.

        Returns:
            Dict with:
            - 'type': 'bearer'
            - 'value': access token
            - 'expires_at': ISO timestamp
        """
        return {
            "type": "bearer",
            "value": self.access_token,
            "expires_at": self.expires_at.isoformat(),
        }

    async def refresh_credentials(self, force: bool = False) -> bool:
        """No-op for OAuth provider.

        Token refresh is handled by Claude Code externally.
        This method exists to satisfy the AuthProvider interface.

        Args:
            force: Ignored

        Returns:
            Always returns True (no refresh needed)
        """
        logger.debug(
            "oauth_refresh_noop",
            message="Token refresh delegated to Claude Code",
        )
        return True

    def is_valid(self) -> bool:
        """Check if current credentials exist.

        Note: Does not check expiry - assumes Claude Code manages token freshness.

        Returns:
            True if access token exists
        """
        return bool(self.access_token)

    def get_auth_method(self) -> Literal["oauth"]:
        """Get authentication method.

        Returns:
            "oauth"
        """
        return "oauth"

    def get_context_limit(self) -> int:
        """Get context window limit for OAuth authentication.

        Returns:
            200,000 tokens (200K token context window for OAuth)
        """
        return 200_000
