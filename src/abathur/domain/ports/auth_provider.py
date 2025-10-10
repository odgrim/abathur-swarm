"""Abstract authentication provider for Claude API."""

from abc import ABC, abstractmethod
from typing import Literal


class AuthProvider(ABC):
    """Abstract authentication provider for Claude API.

    This interface defines the contract for authentication providers,
    allowing different authentication methods (API key, OAuth) to be
    used interchangeably throughout the application.
    """

    @abstractmethod
    async def get_credentials(self) -> dict[str, str]:
        """Get credentials for API requests.

        This method may trigger automatic token refresh if credentials
        are near expiry (for OAuth providers).

        Returns:
            Dict with:
            - 'type': 'api_key' | 'bearer'
            - 'value': credential value
            - 'expires_at': ISO timestamp (for OAuth only, optional)

        Raises:
            AuthenticationError: If credentials cannot be retrieved or refreshed
        """
        pass

    @abstractmethod
    async def refresh_credentials(self, force: bool = False) -> bool:
        """Refresh expired credentials.

        For OAuth providers using Claude Code integration, this is a no-op
        as token refresh is handled externally by Claude Code.
        For API key providers, this is a no-op (always returns True).

        Args:
            force: Ignored for OAuth providers (Claude Code handles refresh)

        Returns:
            Always returns True (no-op)

        Note:
            This method exists for interface compatibility but does not
            perform any actual token refresh operations.
        """
        pass

    @abstractmethod
    def is_valid(self) -> bool:
        """Check if current credentials are valid and not expired.

        Returns:
            True if credentials exist and are valid, False otherwise

        Note:
            This is a synchronous method that performs local validation
            without making network requests.
        """
        pass

    @abstractmethod
    def get_auth_method(self) -> Literal["api_key", "oauth"]:
        """Get authentication method type.

        Returns:
            "api_key" for API key authentication
            "oauth" for OAuth token authentication
        """
        pass

    @abstractmethod
    def get_context_limit(self) -> int:
        """Get context window token limit for this auth method.

        Returns:
            Maximum tokens allowed in context window:
            - 1,000,000 for API key authentication
            - 200,000 for OAuth authentication
        """
        pass
