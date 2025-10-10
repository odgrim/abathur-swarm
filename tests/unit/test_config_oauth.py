"""Unit tests for ConfigManager OAuth methods."""

import os
import tempfile
from collections.abc import Generator
from datetime import datetime, timedelta, timezone
from pathlib import Path
from unittest.mock import patch

import pytest
from abathur.infrastructure.config import ConfigManager


@pytest.fixture
def temp_project_root() -> Generator[Path, None, None]:
    """Create a temporary project root directory."""
    with tempfile.TemporaryDirectory() as tmpdir:
        yield Path(tmpdir)


@pytest.fixture
def config_manager(temp_project_root: Path) -> ConfigManager:
    """Create a ConfigManager with temporary project root."""
    return ConfigManager(project_root=temp_project_root)


class TestDetectAuthMethod:
    """Test authentication method detection."""

    def test_detect_api_key(self, config_manager: ConfigManager) -> None:
        """Test detection of API key format."""
        assert config_manager.detect_auth_method("sk-ant-api03-test-key") == "api_key"
        assert config_manager.detect_auth_method("sk-ant-api-test-key") == "api_key"
        assert config_manager.detect_auth_method("sk-ant-api01-another-key") == "api_key"

    def test_detect_oauth_token(self, config_manager: ConfigManager) -> None:
        """Test detection of OAuth token format."""
        # OAuth tokens are long and don't start with sk-
        long_token = "a" * 60
        assert config_manager.detect_auth_method(long_token) == "oauth"

    def test_detect_invalid_format_raises(self, config_manager: ConfigManager) -> None:
        """Test that invalid format raises ValueError."""
        with pytest.raises(ValueError, match="Unrecognized credential format"):
            config_manager.detect_auth_method("invalid-short")

        with pytest.raises(ValueError, match="Unrecognized credential format"):
            config_manager.detect_auth_method("sk-other-format")


class TestGetOAuthToken:
    """Test OAuth token retrieval."""

    @pytest.mark.asyncio
    async def test_get_from_environment_variables(self, config_manager: ConfigManager) -> None:
        """Test retrieving OAuth tokens from environment variables."""
        expires_at = datetime.now(timezone.utc) + timedelta(hours=1)

        with patch.dict(
            os.environ,
            {
                "ANTHROPIC_AUTH_TOKEN": "env_access_token",
                "ANTHROPIC_OAUTH_REFRESH_TOKEN": "env_refresh_token",
                "ANTHROPIC_OAUTH_EXPIRES_AT": expires_at.isoformat(),
            },
        ):
            access, refresh, exp = await config_manager.get_oauth_token()

            assert access == "env_access_token"
            assert refresh == "env_refresh_token"
            assert exp == expires_at

    @pytest.mark.asyncio
    async def test_get_from_keychain(self, config_manager: ConfigManager) -> None:
        """Test retrieving OAuth tokens from keychain."""
        expires_at = datetime.now(timezone.utc) + timedelta(hours=1)

        def mock_get_password(service: str, key: str) -> str | None:
            if key == "anthropic_oauth_access_token":
                return "keychain_access_token"
            elif key == "anthropic_oauth_refresh_token":
                return "keychain_refresh_token"
            elif key == "anthropic_oauth_expires_at":
                return expires_at.isoformat()
            return None

        with patch.dict(os.environ, {}, clear=True), patch(
            "keyring.get_password", side_effect=mock_get_password
        ):
            access, refresh, exp = await config_manager.get_oauth_token()

            assert access == "keychain_access_token"
            assert refresh == "keychain_refresh_token"
            assert exp == expires_at

    @pytest.mark.asyncio
    async def test_get_from_env_file(
        self, config_manager: ConfigManager, temp_project_root: Path
    ) -> None:
        """Test retrieving OAuth tokens from .env file."""
        expires_at = datetime.now(timezone.utc) + timedelta(hours=1)

        # Create .env file
        env_file = temp_project_root / ".env"
        with open(env_file, "w") as f:
            f.write("ANTHROPIC_AUTH_TOKEN=env_file_access_token\n")
            f.write("ANTHROPIC_OAUTH_REFRESH_TOKEN=env_file_refresh_token\n")
            f.write(f"ANTHROPIC_OAUTH_EXPIRES_AT={expires_at.isoformat()}\n")

        with patch.dict(os.environ, {}, clear=True), patch(
            "keyring.get_password", return_value=None
        ):
            access, refresh, exp = await config_manager.get_oauth_token()

            assert access == "env_file_access_token"
            assert refresh == "env_file_refresh_token"
            assert exp == expires_at

    @pytest.mark.asyncio
    async def test_priority_order_env_vars_first(
        self, config_manager: ConfigManager, temp_project_root: Path
    ) -> None:
        """Test that environment variables have highest priority."""
        expires_at = datetime.now(timezone.utc) + timedelta(hours=1)

        # Create .env file with different tokens
        env_file = temp_project_root / ".env"
        with open(env_file, "w") as f:
            f.write("ANTHROPIC_AUTH_TOKEN=file_token\n")
            f.write("ANTHROPIC_OAUTH_REFRESH_TOKEN=file_refresh\n")
            f.write(f"ANTHROPIC_OAUTH_EXPIRES_AT={expires_at.isoformat()}\n")

        # Set environment variables
        with patch.dict(
            os.environ,
            {
                "ANTHROPIC_AUTH_TOKEN": "env_token",
                "ANTHROPIC_OAUTH_REFRESH_TOKEN": "env_refresh",
                "ANTHROPIC_OAUTH_EXPIRES_AT": expires_at.isoformat(),
            },
        ):
            access, refresh, exp = await config_manager.get_oauth_token()

            # Should use env vars, not file
            assert access == "env_token"
            assert refresh == "env_refresh"

    @pytest.mark.asyncio
    async def test_raises_when_not_found(self, config_manager: ConfigManager) -> None:
        """Test that ValueError is raised when tokens not found."""
        with patch.dict(os.environ, {}, clear=True), patch(
            "keyring.get_password", return_value=None
        ):
            with pytest.raises(ValueError, match="OAuth tokens not found"):
                await config_manager.get_oauth_token()


class TestSetOAuthToken:
    """Test OAuth token storage."""

    @pytest.mark.asyncio
    async def test_set_in_keychain(self, config_manager: ConfigManager) -> None:
        """Test storing OAuth tokens in keychain."""
        expires_at = datetime.now(timezone.utc) + timedelta(hours=1)
        stored_values: dict[str, str] = {}

        def mock_set_password(service: str, key: str, value: str) -> None:
            stored_values[key] = value

        with patch("keyring.set_password", side_effect=mock_set_password):
            await config_manager.set_oauth_token(
                "new_access_token", "new_refresh_token", expires_at, use_keychain=True
            )

            assert stored_values["anthropic_oauth_access_token"] == "new_access_token"
            assert stored_values["anthropic_oauth_refresh_token"] == "new_refresh_token"
            assert stored_values["anthropic_oauth_expires_at"] == expires_at.isoformat()

    @pytest.mark.asyncio
    async def test_set_in_env_file(
        self, config_manager: ConfigManager, temp_project_root: Path
    ) -> None:
        """Test storing OAuth tokens in .env file."""
        expires_at = datetime.now(timezone.utc) + timedelta(hours=1)

        await config_manager.set_oauth_token(
            "file_access_token", "file_refresh_token", expires_at, use_keychain=False
        )

        # Verify file created
        env_file = temp_project_root / ".env"
        assert env_file.exists()

        # Verify permissions
        assert oct(env_file.stat().st_mode)[-3:] == "600"

        # Verify content
        with open(env_file) as f:
            content = f.read()
            assert "ANTHROPIC_AUTH_TOKEN=file_access_token" in content
            assert "ANTHROPIC_OAUTH_REFRESH_TOKEN=file_refresh_token" in content
            assert f"ANTHROPIC_OAUTH_EXPIRES_AT={expires_at.isoformat()}" in content

    @pytest.mark.asyncio
    async def test_set_fallback_to_env_on_keychain_failure(
        self, config_manager: ConfigManager, temp_project_root: Path
    ) -> None:
        """Test fallback to .env file when keychain fails."""
        expires_at = datetime.now(timezone.utc) + timedelta(hours=1)

        with patch("keyring.set_password", side_effect=Exception("Keychain not available")):
            await config_manager.set_oauth_token(
                "fallback_access", "fallback_refresh", expires_at, use_keychain=True
            )

            # Should have fallen back to .env file
            env_file = temp_project_root / ".env"
            assert env_file.exists()

            with open(env_file) as f:
                content = f.read()
                assert "fallback_access" in content

    @pytest.mark.asyncio
    async def test_set_overwrites_existing_tokens_in_env_file(
        self, config_manager: ConfigManager, temp_project_root: Path
    ) -> None:
        """Test that setting tokens overwrites existing ones in .env."""
        # Create .env with existing content
        env_file = temp_project_root / ".env"
        with open(env_file, "w") as f:
            f.write("SOME_OTHER_VAR=value\n")
            f.write("ANTHROPIC_AUTH_TOKEN=old_token\n")
            f.write("ANTHROPIC_OAUTH_REFRESH_TOKEN=old_refresh\n")
            f.write("ANTHROPIC_OAUTH_EXPIRES_AT=2025-01-01T00:00:00+00:00\n")

        expires_at = datetime.now(timezone.utc) + timedelta(hours=1)

        await config_manager.set_oauth_token(
            "new_token", "new_refresh", expires_at, use_keychain=False
        )

        with open(env_file) as f:
            content = f.read()
            # Other vars preserved
            assert "SOME_OTHER_VAR=value" in content
            # Old tokens removed
            assert "old_token" not in content
            assert "old_refresh" not in content
            # New tokens added
            assert "new_token" in content
            assert "new_refresh" in content


class TestClearOAuthTokens:
    """Test OAuth token cleanup."""

    def test_clear_from_keychain(self, config_manager: ConfigManager) -> None:
        """Test clearing tokens from keychain."""
        deleted_keys: list[str] = []

        def mock_delete_password(service: str, key: str) -> None:
            deleted_keys.append(key)

        with patch("keyring.delete_password", side_effect=mock_delete_password), patch.dict(
            os.environ,
            {
                "ANTHROPIC_AUTH_TOKEN": "test",
                "ANTHROPIC_OAUTH_REFRESH_TOKEN": "test",
                "ANTHROPIC_OAUTH_EXPIRES_AT": "test",
            },
        ):
            config_manager.clear_oauth_tokens(clear_env_file=False)

            # Verify keychain keys deleted
            assert "anthropic_oauth_access_token" in deleted_keys
            assert "anthropic_oauth_refresh_token" in deleted_keys
            assert "anthropic_oauth_expires_at" in deleted_keys

            # Verify env vars cleared
            assert "ANTHROPIC_AUTH_TOKEN" not in os.environ
            assert "ANTHROPIC_OAUTH_REFRESH_TOKEN" not in os.environ

    def test_clear_from_env_file(
        self, config_manager: ConfigManager, temp_project_root: Path
    ) -> None:
        """Test clearing tokens from .env file."""
        # Create .env with OAuth tokens and other vars
        env_file = temp_project_root / ".env"
        with open(env_file, "w") as f:
            f.write("SOME_OTHER_VAR=value\n")
            f.write("ANTHROPIC_AUTH_TOKEN=token_to_remove\n")
            f.write("ANTHROPIC_OAUTH_REFRESH_TOKEN=refresh_to_remove\n")
            f.write("ANTHROPIC_OAUTH_EXPIRES_AT=2025-01-01T00:00:00+00:00\n")
            f.write("ANOTHER_VAR=another_value\n")

        with patch("keyring.delete_password"):
            config_manager.clear_oauth_tokens(clear_env_file=True)

        # Verify OAuth tokens removed but other vars preserved
        with open(env_file) as f:
            content = f.read()
            assert "SOME_OTHER_VAR=value" in content
            assert "ANOTHER_VAR=another_value" in content
            assert "token_to_remove" not in content
            assert "refresh_to_remove" not in content

    def test_clear_handles_missing_keychain_gracefully(self, config_manager: ConfigManager) -> None:
        """Test that clearing handles missing keychain entries gracefully."""
        with patch("keyring.delete_password", side_effect=Exception("Key not found")):
            # Should not raise - just log and continue
            config_manager.clear_oauth_tokens(clear_env_file=False)
