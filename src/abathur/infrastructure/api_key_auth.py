"""API key authentication provider for backward compatibility."""

from typing import Literal

from abathur.domain.ports.auth_provider import AuthProvider
from abathur.infrastructure.exceptions import APIKeyInvalidError


class APIKeyAuthProvider(AuthProvider):
    """API key authentication provider.

    This provider wraps existing API key authentication logic,
    maintaining 100% backward compatibility with existing workflows.

    API keys do not expire, so refresh operations are no-ops.
    Context window limit is 1,000,000 tokens.

    Attributes:
        api_key: Anthropic API key (must start with sk-ant-api)
    """

    def __init__(self, api_key: str):
        """Initialize API key authentication provider.

        Args:
            api_key: Anthropic API key

        Raises:
            APIKeyInvalidError: If API key is missing or has invalid format
        """
        if not api_key:
            raise APIKeyInvalidError("API key cannot be empty")

        if not api_key.startswith("sk-ant-api"):
            raise APIKeyInvalidError(
                f"Invalid API key format (must start with sk-ant-api): {api_key[:15]}..."
            )

        self.api_key = api_key

    async def get_credentials(self) -> dict[str, str]:
        """Get API key credentials.

        Returns:
            Dict with:
            - 'type': 'api_key'
            - 'value': API key string
        """
        return {"type": "api_key", "value": self.api_key}

    async def refresh_credentials(self) -> bool:
        """Refresh credentials (no-op for API keys).

        API keys do not expire, so this always returns True.

        Returns:
            True (API keys don't need refreshing)
        """
        return True

    def is_valid(self) -> bool:
        """Check if API key is valid.

        Returns:
            True if API key exists and is non-empty
        """
        return bool(self.api_key)

    def get_auth_method(self) -> Literal["api_key"]:
        """Get authentication method.

        Returns:
            "api_key"
        """
        return "api_key"

    def get_context_limit(self) -> int:
        """Get context window limit for API key authentication.

        Returns:
            1,000,000 tokens (1M token context window)
        """
        return 1_000_000
