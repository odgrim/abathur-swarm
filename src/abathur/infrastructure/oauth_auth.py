"""OAuth authentication provider with automatic token refresh."""

import asyncio
from datetime import datetime, timedelta, timezone
from typing import TYPE_CHECKING, Literal

import httpx

from abathur.domain.ports.auth_provider import AuthProvider
from abathur.infrastructure.exceptions import OAuthRefreshError
from abathur.infrastructure.logger import get_logger

if TYPE_CHECKING:
    from abathur.infrastructure.config import ConfigManager

logger = get_logger(__name__)


class OAuthAuthProvider(AuthProvider):
    """OAuth authentication provider with automatic token refresh.

    This provider manages OAuth tokens with automatic refresh capabilities:
    - Proactive refresh: Refreshes 5 minutes before expiry
    - Reactive refresh: Retries on 401 Unauthorized errors
    - Token rotation: Handles refresh token rotation
    - Persistent storage: Saves tokens to ConfigManager

    Attributes:
        access_token: Current OAuth access token
        refresh_token: OAuth refresh token for obtaining new access tokens
        expires_at: Token expiry timestamp (UTC)
        config_manager: ConfigManager instance for token persistence
    """

    # Token refresh endpoint (verified working)
    TOKEN_REFRESH_URL = "https://console.anthropic.com/v1/oauth/token"

    # Claude Code client ID (from research)
    CLIENT_ID = "9d1c250a-e61b-44d9-88ed-5944d1962f5e"

    # Refresh 5 minutes before expiry to account for clock skew
    REFRESH_BUFFER_MINUTES = 5

    # Lock for preventing concurrent refresh requests
    _refresh_lock = asyncio.Lock()

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
        """Get OAuth credentials with automatic proactive refresh.

        This method checks if the token is near expiry and proactively
        refreshes it before returning credentials.

        Returns:
            Dict with:
            - 'type': 'bearer'
            - 'value': access token
            - 'expires_at': ISO timestamp

        Raises:
            OAuthRefreshError: If token is expired and refresh fails
        """
        # Proactive refresh if near expiry
        if self._is_near_expiry():
            logger.info(
                "proactive_token_refresh",
                expires_at=self.expires_at.isoformat(),
                time_until_expiry=str(self.expires_at - datetime.now(timezone.utc)),
            )
            await self.refresh_credentials()

        # Verify token is still valid
        if not self.is_valid():
            raise OAuthRefreshError("Token expired and refresh failed")

        return {
            "type": "bearer",
            "value": self.access_token,
            "expires_at": self.expires_at.isoformat(),
        }

    async def refresh_credentials(self, force: bool = False) -> bool:
        """Refresh OAuth token with retry logic.

        This method implements:
        - 3-retry logic with exponential backoff for transient errors
        - Respect for 429 Retry-After headers
        - Token rotation handling
        - Persistent storage of new tokens

        Args:
            force: If True, force refresh even if token appears valid (for handling
                   cases where API returns 401 despite token appearing fresh)

        Returns:
            True if refresh succeeded, False otherwise
        """
        # Use lock to prevent concurrent refresh requests
        async with self._refresh_lock:
            # Double-check expiry inside lock (another task may have refreshed)
            # But also verify the token was refreshed recently (within last 10 seconds)
            # to ensure we're not using a potentially stale/invalid token
            if not force and not self._is_expired() and not self._is_near_expiry():
                # Add extra validation: if this appears to be a fresh token
                # (expires_at is in the future and was recently set), accept it
                if self.expires_at > datetime.now(timezone.utc) + timedelta(minutes=1):
                    logger.debug(
                        "token_already_refreshed",
                        message="Token refreshed by another task",
                        expires_at=self.expires_at.isoformat(),
                    )
                    return True
                else:
                    # Token expiry is suspicious (too close or in past), force refresh
                    logger.warning(
                        "token_expiry_suspicious",
                        expires_at=self.expires_at.isoformat(),
                        forcing_refresh=True,
                    )
            elif force:
                logger.info(
                    "forced_token_refresh",
                    message="Forcing token refresh despite apparent validity",
                )

            # Retry up to 3 times
            for attempt in range(3):
                try:
                    async with httpx.AsyncClient() as client:
                        response = await client.post(
                            self.TOKEN_REFRESH_URL,
                            json={
                                "grant_type": "refresh_token",
                                "refresh_token": self.refresh_token,
                                "client_id": self.CLIENT_ID,
                            },
                            timeout=30.0,
                        )

                        # 401 means refresh token expired - no retry
                        if response.status_code == 401:
                            logger.error(
                                "refresh_token_expired",
                                message="Refresh token expired or revoked",
                            )
                            return False

                        # Raise for other HTTP errors (will be caught below)
                        response.raise_for_status()

                        # Parse response
                        data = response.json()

                        # Update tokens (handle rotation)
                        self.access_token = data["access_token"]
                        self.refresh_token = data.get("refresh_token", self.refresh_token)
                        self.expires_at = datetime.now(timezone.utc) + timedelta(
                            seconds=data["expires_in"]
                        )

                        # Persist new tokens
                        await self.config_manager.set_oauth_token(
                            self.access_token, self.refresh_token, self.expires_at
                        )

                        logger.info(
                            "oauth_token_refreshed",
                            expires_at=self.expires_at.isoformat(),
                            rotated=data.get("refresh_token") is not None,
                        )
                        return True

                except httpx.HTTPStatusError as e:
                    # Handle 429 rate limiting with Retry-After
                    if e.response.status_code == 429 and attempt < 2:
                        retry_after = int(e.response.headers.get("Retry-After", 60))
                        logger.warning(
                            "token_refresh_rate_limited",
                            retry_after=retry_after,
                            attempt=attempt + 1,
                        )
                        await asyncio.sleep(retry_after)
                        continue

                    logger.error(
                        "token_refresh_http_error",
                        status=e.response.status_code,
                        attempt=attempt + 1,
                    )

                    # Last attempt failed
                    if attempt == 2:
                        return False

                except Exception as e:
                    logger.error(
                        "token_refresh_error",
                        error=str(e),
                        error_type=type(e).__name__,
                        attempt=attempt + 1,
                    )

                    # Last attempt failed
                    if attempt == 2:
                        return False

                    # Exponential backoff for transient errors
                    await asyncio.sleep(2**attempt)

        return False

    def is_valid(self) -> bool:
        """Check if current credentials are valid and not expired.

        Returns:
            True if access token exists and is not expired
        """
        return bool(self.access_token) and not self._is_expired()

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

    def _is_expired(self) -> bool:
        """Check if token is expired.

        Returns:
            True if current time >= expiry time
        """
        now = datetime.now(timezone.utc)
        return now >= self.expires_at

    def _is_near_expiry(self) -> bool:
        """Check if token is near expiry (within buffer window).

        Returns:
            True if token expires within REFRESH_BUFFER_MINUTES
        """
        now = datetime.now(timezone.utc)
        buffer = timedelta(minutes=self.REFRESH_BUFFER_MINUTES)
        return now >= (self.expires_at - buffer)
