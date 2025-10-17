"""Claude CLI authentication provider for fallback when no API key is available."""

import asyncio
import json
import shutil
from typing import Literal

from abathur.domain.ports.auth_provider import AuthProvider
from abathur.infrastructure.exceptions import AuthenticationError
from abathur.infrastructure.logger import get_logger

logger = get_logger(__name__)


class ClaudeCLIAuthProvider(AuthProvider):
    """Claude CLI authentication provider.

    This provider uses the `claude` CLI binary for authentication
    and API calls. It's used as a fallback when no API key is available.

    The provider checks for the Claude CLI binary and uses it to
    make API requests directly, bypassing the SDK.

    Attributes:
        cli_path: Path to the claude CLI binary
    """

    def __init__(self) -> None:
        """Initialize Claude CLI authentication provider.

        Raises:
            AuthenticationError: If claude CLI is not found in PATH
        """
        # Check if claude CLI is available
        cli_path = shutil.which("claude")
        if not cli_path:
            raise AuthenticationError(
                "Claude CLI not found in PATH. "
                "Please install Claude CLI or set ANTHROPIC_API_KEY. "
                "Visit https://docs.anthropic.com/claude/docs/quickstart "
                "for installation instructions."
            )

        self.cli_path: str = cli_path
        logger.debug("claude_cli_initialized", cli_path=self.cli_path)

    async def get_credentials(self) -> dict[str, str]:
        """Get Claude CLI credentials.

        Returns:
            Dict with:
            - 'type': 'claude_cli'
            - 'value': path to claude CLI binary
        """
        return {"type": "claude_cli", "value": self.cli_path}

    async def refresh_credentials(self, force: bool = False) -> bool:
        """Refresh credentials (no-op for Claude CLI).

        Claude CLI handles authentication internally.

        Args:
            force: Ignored for Claude CLI

        Returns:
            True if CLI is still available
        """
        # Check if CLI is still available
        return self.is_valid()

    def is_valid(self) -> bool:
        """Check if Claude CLI is available.

        Returns:
            True if claude CLI binary exists and is executable
        """
        return shutil.which("claude") is not None

    def get_auth_method(self) -> Literal["api_key"]:
        """Get authentication method.

        Returns:
            "api_key" (for compatibility with existing code)

        Note:
            We return "api_key" instead of "claude_cli" to maintain
            compatibility with existing code that checks auth methods.
        """
        return "api_key"

    def get_context_limit(self) -> int:
        """Get context window limit for Claude CLI.

        Returns:
            200,000 tokens (Claude CLI uses Claude Code authentication)
        """
        return 200_000

    async def execute_prompt(
        self,
        system_prompt: str,
        user_message: str,
        max_tokens: int = 8000,
        temperature: float = 0.7,
        model: str | None = None,
    ) -> dict[str, str]:
        """Execute a prompt using Claude CLI.

        Args:
            system_prompt: System prompt defining agent behavior
            user_message: User message/task to execute
            max_tokens: Maximum tokens in response
            temperature: Sampling temperature
            model: Model to use (ignored, CLI uses default)

        Returns:
            Dict with:
            - 'content': response text (always present, may be empty string)
            - 'error': error message if failed, empty string otherwise

        Raises:
            subprocess.CalledProcessError: If claude CLI fails
        """
        try:
            # Construct the prompt with system instructions
            full_prompt = f"{system_prompt}\n\n{user_message}"

            # Execute claude CLI
            # Note: Claude CLI may not support all parameters,
            # so we use a simple prompt-based approach
            logger.info(
                "executing_claude_cli",
                model=model or "default",
                prompt_length=len(full_prompt),
            )

            # Import Path for cwd resolution
            from pathlib import Path

            # Pass current working directory to subprocess so it can access
            # project-specific MCP server configurations and permissions
            project_cwd = Path.cwd()

            process = await asyncio.create_subprocess_exec(
                self.cli_path,
                "--print",
                "--output-format",
                "json",
                "--dangerously-skip-permissions",
                stdin=asyncio.subprocess.PIPE,
                stdout=asyncio.subprocess.PIPE,
                stderr=asyncio.subprocess.PIPE,
                cwd=str(project_cwd),
            )

            stdout, stderr = await process.communicate(input=full_prompt.encode())

            if process.returncode != 0:
                error_msg = stderr.decode().strip() if stderr else "Unknown error"
                stdout_msg = stdout.decode().strip() if stdout else ""

                logger.error(
                    "claude_cli_failed",
                    error=error_msg,
                    stdout=stdout_msg,
                    return_code=process.returncode,
                    cli_path=self.cli_path,
                    prompt_length=len(full_prompt),
                )

                # Provide more detailed error message
                detailed_error = f"Claude CLI failed with exit code {process.returncode}"
                if error_msg:
                    detailed_error += f": {error_msg}"
                if stdout_msg and not error_msg:
                    # Sometimes error details are in stdout
                    detailed_error += f": {stdout_msg}"

                return {"content": "", "error": detailed_error}

            # Parse JSON response
            try:
                response = json.loads(stdout.decode())
                content = response.get("content", response.get("text", ""))

                logger.info("claude_cli_completed", response_length=len(content))

                return {"content": content, "error": ""}
            except json.JSONDecodeError:
                # If JSON parsing fails, use raw stdout
                content = stdout.decode()
                return {"content": content, "error": ""}

        except Exception as e:
            logger.error(
                "claude_cli_execution_failed",
                error=str(e),
                error_type=type(e).__name__,
                cli_path=self.cli_path,
            )
            return {
                "content": "",
                "error": f"Claude CLI execution failed ({type(e).__name__}): {str(e)}",
            }
