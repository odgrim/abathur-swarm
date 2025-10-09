"""Claude API client wrapper with retry logic and rate limiting."""

import asyncio
import os
from collections.abc import AsyncIterator
from typing import Any

from anthropic import Anthropic, AsyncAnthropic

from abathur.infrastructure.logger import get_logger

logger = get_logger(__name__)


class ClaudeClient:
    """Wrapper for Anthropic Claude API with retry logic and rate limiting."""

    def __init__(
        self,
        api_key: str | None = None,
        model: str = "claude-sonnet-4-20250514",
        max_retries: int = 3,
        timeout: int = 300,
    ):
        """Initialize Claude client.

        Args:
            api_key: Anthropic API key (if None, reads from environment)
            model: Default model to use
            max_retries: Maximum retry attempts for transient errors
            timeout: Request timeout in seconds
        """
        self.api_key = api_key or os.getenv("ANTHROPIC_API_KEY")
        if not self.api_key:
            raise ValueError("ANTHROPIC_API_KEY must be provided or set in environment")

        self.model = model
        self.max_retries = max_retries
        self.timeout = timeout

        # Initialize sync and async clients
        self.client = Anthropic(api_key=self.api_key, max_retries=max_retries)
        self.async_client = AsyncAnthropic(api_key=self.api_key, max_retries=max_retries)

    async def execute_task(
        self,
        system_prompt: str,
        user_message: str,
        max_tokens: int = 8000,
        temperature: float = 0.7,
        model: str | None = None,
    ) -> dict[str, Any]:
        """Execute a task using Claude.

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

        try:
            logger.info("executing_claude_task", model=model_to_use)

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
            logger.info("streaming_claude_task", model=model_to_use)

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

    def validate_api_key(self) -> bool:
        """Validate API key by making a test request.

        Returns:
            True if API key is valid, False otherwise
        """
        try:
            # Make a minimal test request
            self.client.messages.create(
                model="claude-3-haiku-20240307",  # Use smallest/fastest model
                max_tokens=10,
                messages=[{"role": "user", "content": "test"}],
            )
            return True
        except Exception as e:
            logger.error("api_key_validation_failed", error=str(e))
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
