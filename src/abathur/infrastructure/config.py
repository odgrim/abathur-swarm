"""Configuration management with hierarchical loading."""

import os
from datetime import datetime
from pathlib import Path
from typing import Any, Literal

import keyring
import yaml
from pydantic import BaseModel, Field

from abathur.infrastructure.logger import get_logger

logger = get_logger(__name__)


class QueueConfig(BaseModel):
    """Queue configuration."""

    max_size: int = Field(default=1000, ge=1)
    default_priority: int = Field(default=5, ge=0, le=10)
    retry_attempts: int = Field(default=3, ge=0)
    retry_backoff_initial: str = "10s"
    retry_backoff_max: str = "5m"


class SwarmConfig(BaseModel):
    """Swarm orchestration configuration."""

    max_concurrent_agents: int = Field(default=10, ge=1)
    agent_spawn_timeout: str = "5s"
    agent_idle_timeout: str = "5m"
    hierarchical_depth_limit: int = Field(default=3, ge=1)


class LoopConfig(BaseModel):
    """Loop execution configuration."""

    max_iterations: int = Field(default=10, ge=1)
    default_timeout: str = "1h"
    checkpoint_interval: int = Field(default=1, ge=1)


class ResourceConfig(BaseModel):
    """Resource limits configuration."""

    max_memory_per_agent: str = "512MB"
    max_total_memory: str = "4GB"
    adaptive_cpu: bool = True


class MonitoringConfig(BaseModel):
    """Monitoring and logging configuration."""

    log_rotation_days: int = Field(default=30, ge=1)
    audit_retention_days: int = Field(default=90, ge=1)
    metrics_enabled: bool = True


class AuthConfig(BaseModel):
    """Authentication configuration."""

    mode: Literal["auto", "api_key", "oauth"] = "auto"
    oauth_token_storage: Literal["keychain", "env"] = "keychain"
    auto_refresh: bool = True
    refresh_retries: int = Field(default=3, ge=1, le=10)
    context_window_handling: Literal["warn", "block", "ignore"] = "warn"


class Config(BaseModel):
    """Main configuration model."""

    version: str = "0.1.0"
    log_level: str = "INFO"
    queue: QueueConfig = Field(default_factory=QueueConfig)
    swarm: SwarmConfig = Field(default_factory=SwarmConfig)
    loop: LoopConfig = Field(default_factory=LoopConfig)
    resources: ResourceConfig = Field(default_factory=ResourceConfig)
    monitoring: MonitoringConfig = Field(default_factory=MonitoringConfig)
    auth: AuthConfig = Field(default_factory=AuthConfig)


class ConfigManager:
    """Manage configuration loading from multiple sources with hierarchy."""

    def __init__(self, project_root: Path | None = None) -> None:
        """Initialize config manager.

        Args:
            project_root: Root directory of the project (default: current directory)
        """
        self.project_root = project_root or Path.cwd()
        self._config: Config | None = None

    def load_config(self) -> Config:
        """Load configuration from all sources in hierarchy order.

        Configuration hierarchy (highest priority last):
        1. System defaults (embedded in Config model)
        2. Template defaults (.abathur/config.yaml)
        3. User overrides (~/.abathur/config.yaml)
        4. Project overrides (.abathur/local.yaml)
        5. Environment variables (ABATHUR_* prefix)

        Returns:
            Merged configuration
        """
        if self._config is not None:
            return self._config

        # Start with system defaults
        config_dict: dict[str, Any] = {}

        # Load template defaults
        template_config_path = self.project_root / ".abathur" / "config.yaml"
        if template_config_path.exists():
            config_dict = self._merge_dicts(config_dict, self._load_yaml(template_config_path))

        # Load user overrides
        user_config_path = Path.home() / ".abathur" / "config.yaml"
        if user_config_path.exists():
            config_dict = self._merge_dicts(config_dict, self._load_yaml(user_config_path))

        # Load project overrides
        local_config_path = self.project_root / ".abathur" / "local.yaml"
        if local_config_path.exists():
            config_dict = self._merge_dicts(config_dict, self._load_yaml(local_config_path))

        # Apply environment variables
        config_dict = self._apply_env_vars(config_dict)

        # Create and validate config
        self._config = Config(**config_dict)
        return self._config

    def _load_yaml(self, path: Path) -> dict[str, Any]:
        """Load YAML configuration file."""
        with open(path) as f:
            return yaml.safe_load(f) or {}

    def _merge_dicts(self, base: dict[str, Any], override: dict[str, Any]) -> dict[str, Any]:
        """Recursively merge two dictionaries."""
        result = base.copy()
        for key, value in override.items():
            if key in result and isinstance(result[key], dict) and isinstance(value, dict):
                result[key] = self._merge_dicts(result[key], value)
            else:
                result[key] = value
        return result

    def _apply_env_vars(self, config_dict: dict[str, Any]) -> dict[str, Any]:
        """Apply environment variables with ABATHUR_ prefix."""
        # Map of env var names to config paths
        env_mappings = {
            "ABATHUR_LOG_LEVEL": ["log_level"],
            "ABATHUR_QUEUE_MAX_SIZE": ["queue", "max_size"],
            "ABATHUR_MAX_CONCURRENT_AGENTS": ["swarm", "max_concurrent_agents"],
            "ABATHUR_MAX_ITERATIONS": ["loop", "max_iterations"],
        }

        for env_var, path in env_mappings.items():
            value = os.getenv(env_var)
            if value is not None:
                # Navigate to the nested dict
                current = config_dict
                for key in path[:-1]:
                    if key not in current:
                        current[key] = {}
                    current = current[key]
                # Set the value (convert to int if needed)
                try:
                    current[path[-1]] = int(value)
                except ValueError:
                    current[path[-1]] = value

        return config_dict

    def get_api_key(self) -> str:
        """Get Anthropic API key from environment, keychain, or .env file.

        Priority:
        1. ANTHROPIC_API_KEY environment variable
        2. System keychain
        3. .env file

        Returns:
            API key

        Raises:
            ValueError: If API key not found
        """
        # 1. Environment variable
        if key := os.getenv("ANTHROPIC_API_KEY"):
            return key

        # 2. System keychain
        try:
            key = keyring.get_password("abathur", "anthropic_api_key")
            if key:
                return key
        except Exception:
            pass

        # 3. .env file
        env_file = self.project_root / ".env"
        if env_file.exists():
            with open(env_file) as f:
                for line in f:
                    line = line.strip()
                    if line.startswith("ANTHROPIC_API_KEY="):
                        return line.split("=", 1)[1].strip().strip('"').strip("'")

        raise ValueError(
            "ANTHROPIC_API_KEY not found. Set it via:\n"
            "  1. Environment variable: export ANTHROPIC_API_KEY=your-key\n"
            "  2. Keychain: abathur config set-key\n"
            "  3. .env file: echo 'ANTHROPIC_API_KEY=your-key' > .env"
        )

    def set_api_key(self, api_key: str, use_keychain: bool = True) -> None:
        """Store API key in keychain or .env file.

        Args:
            api_key: The API key to store
            use_keychain: If True, store in keychain; otherwise in .env file
        """
        if use_keychain:
            try:
                keyring.set_password("abathur", "anthropic_api_key", api_key)
                return
            except Exception as e:
                raise ValueError(f"Failed to store API key in keychain: {e}") from e
        else:
            # Store in .env file
            env_file = self.project_root / ".env"
            with open(env_file, "a") as f:
                f.write(f"\nANTHROPIC_API_KEY={api_key}\n")

    def get_database_path(self) -> Path:
        """Get path to SQLite database."""
        db_dir = self.project_root / ".abathur"
        db_dir.mkdir(exist_ok=True)
        return db_dir / "abathur.db"

    def get_log_dir(self) -> Path:
        """Get path to log directory."""
        log_dir = self.project_root / ".abathur" / "logs"
        log_dir.mkdir(parents=True, exist_ok=True)
        return log_dir

    def detect_auth_method(self, credential: str) -> Literal["api_key", "oauth"]:
        """Detect authentication method from credential format.

        Args:
            credential: The credential string to analyze

        Returns:
            "api_key" if credential is an Anthropic API key
            "oauth" if credential appears to be an OAuth token

        Raises:
            ValueError: If credential format is unrecognized
        """
        if credential.startswith("sk-ant-api"):
            return "api_key"
        elif len(credential) > 50 and not credential.startswith("sk-"):
            # OAuth tokens are typically longer and don't start with sk-
            return "oauth"
        else:
            raise ValueError(
                f"Unrecognized credential format: {credential[:15]}...\n"
                "Expected: API key (sk-ant-api...) or OAuth token"
            )

    async def get_oauth_token(self) -> tuple[str, str, datetime]:
        """Get OAuth tokens from storage.

        Priority:
        1. ANTHROPIC_AUTH_TOKEN + ANTHROPIC_OAUTH_REFRESH_TOKEN environment variables
        2. System keychain
        3. .env file

        Returns:
            Tuple of (access_token, refresh_token, expires_at)

        Raises:
            ValueError: If OAuth tokens not found in any location
        """
        # 1. Environment variables (highest priority)
        access_token = os.getenv("ANTHROPIC_AUTH_TOKEN")
        refresh_token = os.getenv("ANTHROPIC_OAUTH_REFRESH_TOKEN")
        expires_at_str = os.getenv("ANTHROPIC_OAUTH_EXPIRES_AT")

        if access_token and refresh_token and expires_at_str:
            expires_at = datetime.fromisoformat(expires_at_str)
            logger.info("oauth_tokens_loaded", source="environment_variables")
            return access_token, refresh_token, expires_at

        # 2. System keychain
        try:
            access_token = keyring.get_password("abathur", "anthropic_oauth_access_token")
            refresh_token = keyring.get_password("abathur", "anthropic_oauth_refresh_token")
            expires_at_str = keyring.get_password("abathur", "anthropic_oauth_expires_at")

            if access_token and refresh_token and expires_at_str:
                expires_at = datetime.fromisoformat(expires_at_str)
                logger.info("oauth_tokens_loaded", source="keychain")
                return access_token, refresh_token, expires_at
        except Exception as e:
            logger.debug("keychain_read_failed", error=str(e))

        # 3. .env file (fallback)
        env_file = self.project_root / ".env"
        if env_file.exists():
            env_tokens = {}
            with open(env_file) as f:
                for line in f:
                    line = line.strip()
                    if line.startswith("ANTHROPIC_AUTH_TOKEN="):
                        env_tokens["access"] = line.split("=", 1)[1].strip().strip('"').strip("'")
                    elif line.startswith("ANTHROPIC_OAUTH_REFRESH_TOKEN="):
                        env_tokens["refresh"] = line.split("=", 1)[1].strip().strip('"').strip("'")
                    elif line.startswith("ANTHROPIC_OAUTH_EXPIRES_AT="):
                        env_tokens["expires"] = line.split("=", 1)[1].strip().strip('"').strip("'")

            if all(k in env_tokens for k in ["access", "refresh", "expires"]):
                expires_at = datetime.fromisoformat(env_tokens["expires"])
                logger.info("oauth_tokens_loaded", source="env_file")
                return env_tokens["access"], env_tokens["refresh"], expires_at

        raise ValueError(
            "OAuth tokens not found. Please authenticate with: abathur config oauth-login"
        )

    async def set_oauth_token(
        self,
        access_token: str,
        refresh_token: str,
        expires_at: datetime,
        use_keychain: bool = True,
    ) -> None:
        """Store OAuth tokens securely.

        Args:
            access_token: OAuth access token
            refresh_token: OAuth refresh token
            expires_at: Token expiry timestamp
            use_keychain: If True, store in keychain; otherwise in .env file

        Raises:
            ValueError: If storage fails
        """
        if use_keychain:
            try:
                keyring.set_password("abathur", "anthropic_oauth_access_token", access_token)
                keyring.set_password("abathur", "anthropic_oauth_refresh_token", refresh_token)
                keyring.set_password(
                    "abathur", "anthropic_oauth_expires_at", expires_at.isoformat()
                )
                logger.info("oauth_tokens_stored", storage="keychain")
            except Exception as e:
                logger.warning(
                    "keychain_storage_failed",
                    error=str(e),
                    message="Falling back to .env file",
                )
                # Fallback to .env file
                use_keychain = False

        if not use_keychain:
            # Store in .env file
            env_file = self.project_root / ".env"

            # Read existing content
            existing_lines = []
            if env_file.exists():
                with open(env_file) as f:
                    existing_lines = [
                        line
                        for line in f.readlines()
                        if not any(
                            line.startswith(prefix)
                            for prefix in [
                                "ANTHROPIC_AUTH_TOKEN=",
                                "ANTHROPIC_OAUTH_REFRESH_TOKEN=",
                                "ANTHROPIC_OAUTH_EXPIRES_AT=",
                            ]
                        )
                    ]

            # Write all lines including new OAuth tokens
            with open(env_file, "w") as f:
                f.writelines(existing_lines)
                f.write(f"\nANTHROPIC_AUTH_TOKEN={access_token}\n")
                f.write(f"ANTHROPIC_OAUTH_REFRESH_TOKEN={refresh_token}\n")
                f.write(f"ANTHROPIC_OAUTH_EXPIRES_AT={expires_at.isoformat()}\n")

            # Set restrictive permissions (user read/write only)
            env_file.chmod(0o600)

            logger.info("oauth_tokens_stored", storage="env_file")

    def clear_oauth_tokens(self, clear_env_file: bool = True) -> None:
        """Clear stored OAuth tokens from all locations.

        Args:
            clear_env_file: If True, also remove tokens from .env file

        Note:
            This method clears tokens from:
            - System keychain
            - Environment variables (current process only)
            - .env file (if clear_env_file=True)
        """
        # Clear keychain
        for key in [
            "anthropic_oauth_access_token",
            "anthropic_oauth_refresh_token",
            "anthropic_oauth_expires_at",
        ]:
            try:
                keyring.delete_password("abathur", key)
            except Exception:
                pass  # Key may not exist

        # Clear environment variables
        for var in [
            "ANTHROPIC_AUTH_TOKEN",
            "ANTHROPIC_OAUTH_REFRESH_TOKEN",
            "ANTHROPIC_OAUTH_EXPIRES_AT",
        ]:
            os.environ.pop(var, None)

        # Clear .env file
        if clear_env_file:
            env_file = self.project_root / ".env"
            if env_file.exists():
                # Read existing lines and filter out OAuth tokens
                with open(env_file) as f:
                    lines = [
                        line
                        for line in f.readlines()
                        if not any(
                            line.startswith(prefix)
                            for prefix in [
                                "ANTHROPIC_AUTH_TOKEN=",
                                "ANTHROPIC_OAUTH_REFRESH_TOKEN=",
                                "ANTHROPIC_OAUTH_EXPIRES_AT=",
                            ]
                        )
                    ]

                # Write back filtered lines
                with open(env_file, "w") as f:
                    f.writelines(lines)

        logger.info("oauth_tokens_cleared")
