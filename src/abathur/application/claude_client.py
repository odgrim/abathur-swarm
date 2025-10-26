"""Claude API client wrapper for task execution."""

import asyncio
import os
from collections.abc import AsyncIterator
from typing import TYPE_CHECKING, Any

from anthropic import Anthropic, AsyncAnthropic

from abathur.infrastructure.logger import get_logger

if TYPE_CHECKING:
    from abathur.domain.ports.auth_provider import AuthProvider

logger = get_logger(__name__)


def _is_claude_cli_provider(auth_provider: "AuthProvider") -> bool:
    """Check if the auth provider is Claude CLI based.

    Args:
        auth_provider: Authentication provider to check

    Returns:
        True if provider is Claude CLI, False otherwise
    """
    from abathur.infrastructure.claude_cli_auth import ClaudeCLIAuthProvider

    return isinstance(auth_provider, ClaudeCLIAuthProvider)


class ClaudeClient:
    """Wrapper for Anthropic Claude API."""

    def __init__(
        self,
        api_key: str | None = None,
        auth_provider: "AuthProvider | None" = None,
        model: str = "claude-sonnet-4-5-20250929",
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

        logger.debug(
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
        tools: list[dict[str, Any]] | None = None,
        tool_executor: Any = None,
    ) -> dict[str, Any]:
        """Execute a task using Claude.

        Args:
            system_prompt: System prompt defining agent behavior
            user_message: User message/task to execute
            max_tokens: Maximum tokens in response
            temperature: Sampling temperature
            model: Model to use (overrides default)
            tools: Optional list of tools (from MCP or other sources)
            tool_executor: Optional callable to execute tool calls

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

        # Check if using Claude CLI provider
        if _is_claude_cli_provider(self.auth_provider):
            from abathur.infrastructure.claude_cli_auth import ClaudeCLIAuthProvider

            cli_provider = self.auth_provider
            assert isinstance(cli_provider, ClaudeCLIAuthProvider)

            result = await cli_provider.execute_prompt(
                system_prompt=system_prompt,
                user_message=user_message,
                max_tokens=max_tokens,
                temperature=temperature,
                model=model_to_use,
            )

            error_msg = result.get("error", "")
            content = result.get("content", "")

            if error_msg:
                return {
                    "success": False,
                    "content": "",
                    "stop_reason": "error",
                    "usage": {"input_tokens": 0, "output_tokens": 0},
                    "error": error_msg,
                }

            return {
                "success": True,
                "content": content,
                "stop_reason": "end_turn",
                "usage": {
                    "input_tokens": estimated_tokens,
                    "output_tokens": len(content) // 4,
                },
                "error": None,
            }

        # Standard SDK-based execution
        try:
            # Configure SDK with current credentials
            await self._configure_sdk_auth()
            assert self.async_client is not None

            logger.info(
                "executing_claude_task",
                model=model_to_use,
                auth_method=self.auth_provider.get_auth_method(),
                tools_count=len(tools) if tools else 0,
            )

            # Build messages list
            messages = [{"role": "user", "content": user_message}]

            # Track token usage across multiple turns
            total_input_tokens = 0
            total_output_tokens = 0

            # Agentic loop: handle tool calls and continue conversation
            max_turns = 10  # Prevent infinite loops
            final_text = []

            for turn in range(max_turns):
                # Make API call with tools if provided
                api_kwargs = {
                    "model": model_to_use,
                    "max_tokens": max_tokens,
                    "temperature": temperature,
                    "system": system_prompt,
                    "messages": messages,
                    "timeout": self.timeout,
                }

                if tools:
                    api_kwargs["tools"] = tools

                response = await self.async_client.messages.create(**api_kwargs)

                # Track token usage
                total_input_tokens += response.usage.input_tokens
                total_output_tokens += response.usage.output_tokens

                # Process response content
                assistant_content = []
                has_tool_use = False

                for block in response.content:
                    if hasattr(block, "text"):
                        final_text.append(block.text)
                        assistant_content.append({"type": "text", "text": block.text})
                    elif hasattr(block, "type") and block.type == "tool_use":
                        has_tool_use = True
                        tool_block = {
                            "type": "tool_use",
                            "id": block.id,
                            "name": block.name,
                            "input": block.input,
                        }
                        assistant_content.append(tool_block)

                        # Execute tool if executor provided
                        if tool_executor:
                            logger.info(
                                "executing_tool",
                                tool_name=block.name,
                                tool_input=block.input,
                            )

                            try:
                                tool_result = await tool_executor(block.name, block.input)
                                logger.info(
                                    "tool_executed",
                                    tool_name=block.name,
                                    success=True,
                                )
                            except Exception as e:
                                logger.error(
                                    "tool_execution_failed",
                                    tool_name=block.name,
                                    error=str(e),
                                )
                                tool_result = {
                                    "error": str(e),
                                    "type": "error",
                                }

                # If no tool use, we're done
                if not has_tool_use or response.stop_reason == "end_turn":
                    logger.info(
                        "claude_task_completed",
                        turns=turn + 1,
                        tokens_used=total_input_tokens + total_output_tokens,
                        stop_reason=response.stop_reason,
                    )

                    return {
                        "success": True,
                        "content": "\n".join(final_text),
                        "stop_reason": response.stop_reason,
                        "usage": {
                            "input_tokens": total_input_tokens,
                            "output_tokens": total_output_tokens,
                        },
                        "error": None,
                    }

                # Add assistant response to messages
                messages.append({"role": "assistant", "content": assistant_content})

                # Add tool results to messages
                if has_tool_use and tool_executor:
                    tool_results = []
                    for block in response.content:
                        if hasattr(block, "type") and block.type == "tool_use":
                            try:
                                tool_result = await tool_executor(block.name, block.input)
                                tool_results.append({
                                    "type": "tool_result",
                                    "tool_use_id": block.id,
                                    "content": str(tool_result),
                                })
                            except Exception as e:
                                tool_results.append({
                                    "type": "tool_result",
                                    "tool_use_id": block.id,
                                    "content": f"Error: {e}",
                                    "is_error": True,
                                })

                    messages.append({"role": "user", "content": tool_results})

            # Max turns reached
            logger.warning("max_turns_reached", turns=max_turns)
            return {
                "success": True,
                "content": "\n".join(final_text),
                "stop_reason": "max_turns",
                "usage": {
                    "input_tokens": total_input_tokens,
                    "output_tokens": total_output_tokens,
                },
                "error": None,
            }

        except Exception as e:
            logger.error("claude_task_failed", error=str(e))
            return {
                "success": False,
                "content": "",
                "stop_reason": "error",
                "usage": {"input_tokens": 0, "output_tokens": 0},
                "error": str(e),
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
        """
        try:
            # Configure SDK with current credentials
            await self._configure_sdk_auth()
            assert self.async_client is not None

            # Make a minimal test request
            await self.async_client.messages.create(
                model="claude-3-5-haiku-20241022",  # Use smallest/fastest model
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
