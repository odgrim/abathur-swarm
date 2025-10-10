"""Custom exception hierarchy for Abathur authentication and errors."""


class AbathurError(Exception):
    """Base exception for all Abathur errors."""

    pass


class AuthenticationError(AbathurError):
    """Base authentication error with optional remediation guidance.

    Attributes:
        message: Error message describing what went wrong
        remediation: Optional guidance on how to fix the issue
    """

    def __init__(self, message: str, remediation: str | None = None):
        """Initialize authentication error.

        Args:
            message: Error message
            remediation: Optional remediation guidance
        """
        super().__init__(message)
        self.remediation = remediation

    def __str__(self) -> str:
        """Return formatted error message with remediation if available."""
        if self.remediation:
            return f"{self.args[0]}\n\nRemediation: {self.remediation}"
        return str(self.args[0])


class APIKeyInvalidError(AuthenticationError):
    """API key is invalid, malformed, or not configured.

    This error indicates that the provided API key does not match
    the expected format or is missing entirely.
    """

    def __init__(self, message: str = "API key invalid or malformed"):
        """Initialize API key invalid error.

        Args:
            message: Optional custom error message
        """
        super().__init__(
            message=message,
            remediation="Check key format (should start with sk-ant-api) or generate new key at console.anthropic.com",
        )


class ContextWindowExceededError(AbathurError):
    """Task input exceeds context window limit for authentication method.

    This error is raised when a task's input (system prompt + user message)
    exceeds the context window limit for the current authentication method:
    - API Key: 1,000,000 tokens
    - Claude CLI: 200,000 tokens

    Attributes:
        tokens: Estimated token count of the task input
        limit: Context window limit for the auth method
        auth_method: Authentication method being used
    """

    def __init__(self, tokens: int, limit: int, auth_method: str):
        """Initialize context window exceeded error.

        Args:
            tokens: Estimated token count
            limit: Context window limit
            auth_method: Authentication method ("api_key" or "claude_cli")
        """
        message = (
            f"Task input ({tokens:,} tokens) exceeds {auth_method} "
            f"context limit ({limit:,} tokens)"
        )
        super().__init__(message)
        self.tokens = tokens
        self.limit = limit
        self.auth_method = auth_method
