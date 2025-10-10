"""Claude API client wrapper with retry logic and rate limiting."""

import asyncio
import os
from collections.abc import AsyncIterator
from typing import TYPE_CHECKING, Any

from anthropic import Anthropic, AsyncAnthropic

from abathur.infrastructure.logger import get_logger

if TYPE_CHECKING:
    from abathur.domain.ports.auth_provider import AuthProvider

logger = get_logger(__name__)


class ClaudeClient:
    """Wrapper for Anthropic Claude API with retry logic and rate limiting."""

    def __init__(
        self,
        api_key: str | None = None,
        auth_provider: "AuthProvider | None" = None,
        model: str = "claude-sonnet-4-20250514",
        max_retries: int = 3,
        timeout: int = 300,
    ):
        """Initialize Claude client.

        Args:
            api_key: Anthropic API key (deprecated, use auth_provider instead)
            auth_provider: Authentication provider (AuthProvider instance)
            model: Default model to use
            max_retries: Maximum retry attempts for transient errors
            timeout: Request timeout in seconds

        Raises:
            ValueError: If no authentication is provided

        Note:
            For backward compatibility, api_key parameter is still supported.
            If both api_key and auth_provider are provided, auth_provider takes precedence.
        """
        # Initialize auth provider (with backward compatibility)
        if auth_provider:
            self.auth_provider = auth_provider
        elif api_key:
            # Backward compatibility: wrap API key in provider
            from abathur.infrastructure.api_key_auth import APIKeyAuthProvider

            self.auth_provider = APIKeyAuthProvider(api_key)
        else:
            # Try environment variable
            env_api_key = os.getenv("ANTHROPIC_API_KEY")
            if env_api_key:
                from abathur.infrastructure.api_key_auth import APIKeyAuthProvider

                self.auth_provider = APIKeyAuthProvider(env_api_key)
            else:
                raise ValueError(
                    "Authentication required. Provide api_key, auth_provider, "
                    "or set ANTHROPIC_API_KEY environment variable."
                )

        self.model = model
        self.max_retries = max_retries
        self.timeout = timeout
        self.context_limit = self.auth_provider.get_context_limit()

        logger.info(
            "claude_client_initialized",
            auth_method=self.auth_provider.get_auth_method(),
            context_limit=self.context_limit,
        )

        # Initialize SDK clients (will be configured with auth on first use)
        self.client: Anthropic | None = None
        self.async_client: AsyncAnthropic | None = None

    async def _configure_sdk_auth(self) -> None:
        """Configure SDK authentication from AuthProvider.

        This method sets the appropriate environment variable based on
        the authentication type and reinitializes the SDK clients.
        """
        credentials = await self.auth_provider.get_credentials()

        # Clear existing auth env vars
        for var in ["ANTHROPIC_API_KEY", "ANTHROPIC_AUTH_TOKEN"]:
            if var in os.environ:
                del os.environ[var]

        # Set appropriate env var based on credential type
        if credentials["type"] == "api_key":
            os.environ["ANTHROPIC_API_KEY"] = credentials["value"]
        elif credentials["type"] == "bearer":
            os.environ["ANTHROPIC_AUTH_TOKEN"] = credentials["value"]

        # Initialize or reinitialize SDK clients
        self.async_client = AsyncAnthropic(max_retries=self.max_retries)
        self.client = Anthropic(max_retries=self.max_retries)

    def _estimate_tokens(self, system_prompt: str, user_message: str) -> int:
        """Estimate token count using character approximation.

        Uses a simple heuristic: 1 token ≈ 4 characters for English text.

        Args:
            system_prompt: System prompt text
            user_message: User message text

        Returns:
            Estimated token count
        """
        total_chars = len(system_prompt) + len(user_message)
        estimated_tokens = total_chars // 4
        overhead = 10  # Message formatting overhead
        return estimated_tokens + overhead

    async def execute_task(
        self,
        system_prompt: str,
        user_message: str,
        max_tokens: int = 8000,
        temperature: float = 0.7,
        model: str | None = None,
    ) -> dict[str, Any]:
        """Execute a task using Claude with automatic token refresh on 401.

        Args:
            system_prompt: System prompt defining agent behavior
            user_message: User message/task to execute
            max_tokens: Maximum tokens in response
            temperature: Sampling temperature
            model: Model to use (overrides default)

        Returns:
            Dictionary with:
                - success: bool
                - content: str (response text)
                - stop_reason: str
                - usage: dict (token usage stats)
                - error: Optional[str]
        """
        model_to_use = model or self.model

        # Context window validation (warn if approaching limit)
        estimated_tokens = self._estimate_tokens(system_prompt, user_message)
        if estimated_tokens > self.context_limit * 0.9:  # 90% threshold
            logger.warning(
                "context_window_warning",
                estimated_tokens=estimated_tokens,
                limit=self.context_limit,
                auth_method=self.auth_provider.get_auth_method(),
                percentage=round(estimated_tokens / self.context_limit * 100, 1),
            )

        # Retry loop for 401 errors (OAuth token refresh)
        # Track if we've already attempted refresh to detect refresh failures
        refresh_attempted = False
        consecutive_401s = 0

        for attempt in range(self.max_retries):
            try:
                # Configure SDK with current credentials (lazy initialization)
                await self._configure_sdk_auth()
                assert self.async_client is not None

                logger.info(
                    "executing_claude_task",
                    model=model_to_use,
                    auth_method=self.auth_provider.get_auth_method(),
                    attempt=attempt + 1,
                )

                response = await self.async_client.messages.create(
                    model=model_to_use,
                    max_tokens=max_tokens,
                    temperature=temperature,
                    system=system_prompt,
                    messages=[{"role": "user", "content": user_message}],
                    timeout=self.timeout,
                )

                # Extract text content
                content_text = ""
                for block in response.content:
                    if hasattr(block, "text"):
                        content_text += block.text

                result = {
                    "success": True,
                    "content": content_text,
                    "stop_reason": response.stop_reason,
                    "usage": {
                        "input_tokens": response.usage.input_tokens,
                        "output_tokens": response.usage.output_tokens,
                    },
                    "error": None,
                }

                logger.info(
                    "claude_task_completed",
                    tokens_used=response.usage.input_tokens + response.usage.output_tokens,
                    stop_reason=response.stop_reason,
                )

                return result

            except Exception as e:
                # Check for 401 Unauthorized (OAuth token expired)
                error_str = str(e).lower()
                is_401 = "401" in error_str or "unauthorized" in error_str

                if is_401:
                    consecutive_401s += 1

                    # If we get multiple 401s in a row after refresh, the token is truly invalid
                    if consecutive_401s > 1 and refresh_attempted:
                        logger.error(
                            "repeated_auth_failures_after_refresh",
                            consecutive_401s=consecutive_401s,
                            message="Token refresh succeeded but token still invalid",
                        )
                        from abathur.infrastructure.exceptions import OAuthRefreshError

                        return {
                            "success": False,
                            "content": "",
                            "stop_reason": "error",
                            "usage": {"input_tokens": 0, "output_tokens": 0},
                            "error": f"Authentication failed after refresh: {str(OAuthRefreshError())}",
                        }

                    if attempt < self.max_retries - 1:
                        logger.warning(
                            "auth_failed_attempting_refresh",
                            attempt=attempt + 1,
                            max_retries=self.max_retries,
                            consecutive_401s=consecutive_401s,
                        )

                        # Attempt to refresh credentials
                        refresh_attempted = True

                        # Force refresh if this is a repeat 401 (don't trust "already refreshed" status)
                        force_refresh = consecutive_401s > 1

                        if force_refresh:
                            logger.warning(
                                "forcing_token_refresh",
                                reason="repeated_401_after_claimed_refresh",
                                consecutive_401s=consecutive_401s,
                            )
                            # Add small delay to avoid rate limiting
                            await asyncio.sleep(0.5)

                        if await self.auth_provider.refresh_credentials(force=force_refresh):
                            logger.info("credentials_refreshed_retrying")
                            # Add small delay to ensure token propagates
                            await asyncio.sleep(0.1)
                            continue
                        else:
                            # Refresh failed - return error
                            from abathur.infrastructure.exceptions import OAuthRefreshError

                            logger.error("credential_refresh_failed")
                            return {
                                "success": False,
                                "content": "",
                                "stop_reason": "error",
                                "usage": {"input_tokens": 0, "output_tokens": 0},
                                "error": f"Authentication failed: {str(OAuthRefreshError())}",
                            }
                    else:
                        # Last attempt, can't refresh anymore
                        logger.error("claude_task_failed", error=str(e), is_auth_error=is_401)
                        return {
                            "success": False,
                            "content": "",
                            "stop_reason": "error",
                            "usage": {"input_tokens": 0, "output_tokens": 0},
                            "error": str(e),
                        }
                else:
                    # Non-401 error
                    logger.error("claude_task_failed", error=str(e), is_auth_error=is_401)
                    return {
                        "success": False,
                        "content": "",
                        "stop_reason": "error",
                        "usage": {"input_tokens": 0, "output_tokens": 0},
                        "error": str(e),
                    }

        # Should never reach here, but satisfy mypy
        return {
            "success": False,
            "content": "",
            "stop_reason": "error",
            "usage": {"input_tokens": 0, "output_tokens": 0},
            "error": "Max retries exceeded",
        }

    async def stream_task(
        self,
        system_prompt: str,
        user_message: str,
        max_tokens: int = 8000,
        temperature: float = 0.7,
        model: str | None = None,
    ) -> AsyncIterator[str]:
        """Stream a task execution using Claude.

        Args:
            system_prompt: System prompt defining agent behavior
            user_message: User message/task to execute
            max_tokens: Maximum tokens in response
            temperature: Sampling temperature
            model: Model to use (overrides default)

        Yields:
            Text chunks as they arrive
        """
        model_to_use = model or self.model

        try:
            # Configure SDK with current credentials
            await self._configure_sdk_auth()
            assert self.async_client is not None

            logger.info(
                "streaming_claude_task",
                model=model_to_use,
                auth_method=self.auth_provider.get_auth_method(),
            )

            async with self.async_client.messages.stream(
                model=model_to_use,
                max_tokens=max_tokens,
                temperature=temperature,
                system=system_prompt,
                messages=[{"role": "user", "content": user_message}],
                timeout=self.timeout,
            ) as stream:
                async for text in stream.text_stream:
                    yield text

        except Exception as e:
            logger.error("claude_stream_failed", error=str(e))
            raise

    async def validate_authentication(self) -> bool:
        """Validate authentication by making a test request.

        Returns:
            True if authentication is valid, False otherwise

        Note:
            This method makes an actual API request to validate credentials.
            For OAuth, this will also trigger token refresh if needed.
        """
        try:
            # Configure SDK with current credentials
            await self._configure_sdk_auth()
            assert self.async_client is not None

            # Make a minimal test request
            await self.async_client.messages.create(
                model="claude-3-haiku-20240307",  # Use smallest/fastest model
                max_tokens=10,
                messages=[{"role": "user", "content": "test"}],
            )
            logger.info(
                "authentication_validated",
                auth_method=self.auth_provider.get_auth_method(),
            )
            return True
        except Exception as e:
            logger.error(
                "authentication_validation_failed",
                error=str(e),
                auth_method=self.auth_provider.get_auth_method(),
            )
            return False

    def validate_api_key(self) -> bool:
        """Validate API key by making a test request.

        Deprecated: Use validate_authentication() instead.

        Returns:
            True if API key is valid, False otherwise
        """
        import asyncio

        logger.warning(
            "validate_api_key_deprecated",
            message="validate_api_key() is deprecated, use validate_authentication() instead",
        )
        return asyncio.run(self.validate_authentication())

    async def batch_execute(
        self,
        tasks: list[dict[str, str]],
        max_concurrent: int = 5,
    ) -> list[dict[str, Any]]:
        """Execute multiple tasks concurrently with rate limiting.

        Args:
            tasks: List of task dictionaries with 'system_prompt' and 'user_message'
            max_concurrent: Maximum number of concurrent requests

        Returns:
            List of result dictionaries
        """
        semaphore = asyncio.Semaphore(max_concurrent)

        async def execute_with_semaphore(task: dict[str, Any]) -> dict[str, Any]:
            async with semaphore:
                return await self.execute_task(
                    system_prompt=task["system_prompt"],
                    user_message=task["user_message"],
                    max_tokens=task.get("max_tokens", 8000),
                    temperature=task.get("temperature", 0.7),
                    model=task.get("model"),
                )

        logger.info("batch_executing_tasks", count=len(tasks), max_concurrent=max_concurrent)
        results = await asyncio.gather(*[execute_with_semaphore(task) for task in tasks])
        logger.info(
            "batch_execution_complete", success_count=sum(1 for r in results if r["success"])
        )

        return results
