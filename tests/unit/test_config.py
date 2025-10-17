"""Unit tests for configuration management."""

import os
from pathlib import Path
from tempfile import TemporaryDirectory

import pytest
from abathur.infrastructure.config import Config, ConfigManager, QueueConfig, TemplateRepo


class TestConfig:
    """Tests for Config model."""

    def test_default_config(self) -> None:
        """Test creating a config with defaults."""
        config = Config()

        assert config.version == "0.1.0"
        assert config.log_level == "INFO"
        assert config.queue.max_size == 1000
        assert config.queue.default_priority == 5
        assert config.swarm.max_concurrent_agents == 10
        assert config.loop.max_iterations == 10
        assert config.monitoring.metrics_enabled is True
        # Check default template repo
        assert len(config.template_repos) == 1
        assert config.template_repos[0].url == "https://github.com/odgrim/abathur-claude-template.git"
        assert config.template_repos[0].version == "main"

    def test_custom_config_values(self) -> None:
        """Test creating a config with custom values."""
        config = Config(
            log_level="DEBUG",
            queue=QueueConfig(max_size=500, default_priority=3),
        )

        assert config.log_level == "DEBUG"
        assert config.queue.max_size == 500
        assert config.queue.default_priority == 3

    def test_custom_template_repos(self) -> None:
        """Test creating a config with custom template repos."""
        config = Config(
            template_repos=[
                TemplateRepo(url="https://github.com/org/template1.git", version="v1.0"),
                TemplateRepo(url="https://github.com/org/template2.git", version="main"),
            ]
        )

        assert len(config.template_repos) == 2
        assert config.template_repos[0].url == "https://github.com/org/template1.git"
        assert config.template_repos[0].version == "v1.0"
        assert config.template_repos[1].url == "https://github.com/org/template2.git"
        assert config.template_repos[1].version == "main"


class TestConfigManager:
    """Tests for ConfigManager."""

    def test_load_default_config(self) -> None:
        """Test loading config with no files present."""
        with TemporaryDirectory() as tmpdir:
            config_manager = ConfigManager(project_root=Path(tmpdir))
            config = config_manager.load_config()

            # Should return default values
            assert config.version == "0.1.0"
            assert config.log_level == "INFO"

    def test_load_template_config(self) -> None:
        """Test loading config from template file."""
        with TemporaryDirectory() as tmpdir:
            project_root = Path(tmpdir)
            config_dir = project_root / ".abathur"
            config_dir.mkdir()

            # Create template config
            config_file = config_dir / "config.yaml"
            config_file.write_text(
                """
version: "1.0.0"
log_level: DEBUG
queue:
  max_size: 500
                """
            )

            config_manager = ConfigManager(project_root=project_root)
            config = config_manager.load_config()

            assert config.version == "1.0.0"
            assert config.log_level == "DEBUG"
            assert config.queue.max_size == 500

    def test_config_hierarchy(self) -> None:
        """Test configuration hierarchy with multiple sources."""
        with TemporaryDirectory() as tmpdir:
            project_root = Path(tmpdir)
            config_dir = project_root / ".abathur"
            config_dir.mkdir()

            # Create template config (lower priority)
            template_config = config_dir / "config.yaml"
            template_config.write_text(
                """
log_level: INFO
queue:
  max_size: 1000
  default_priority: 5
                """
            )

            # Create local config (higher priority)
            local_config = config_dir / "local.yaml"
            local_config.write_text(
                """
log_level: DEBUG
queue:
  max_size: 2000
                """
            )

            config_manager = ConfigManager(project_root=project_root)
            config = config_manager.load_config()

            # Local config should override template config
            assert config.log_level == "DEBUG"
            assert config.queue.max_size == 2000
            # But unset values should come from template
            assert config.queue.default_priority == 5

    def test_env_var_override(self) -> None:
        """Test that environment variables override config files."""
        with TemporaryDirectory() as tmpdir:
            project_root = Path(tmpdir)
            config_dir = project_root / ".abathur"
            config_dir.mkdir()

            # Create template config
            config_file = config_dir / "config.yaml"
            config_file.write_text(
                """
log_level: INFO
queue:
  max_size: 1000
                """
            )

            # Set environment variable
            os.environ["ABATHUR_LOG_LEVEL"] = "ERROR"
            os.environ["ABATHUR_QUEUE_MAX_SIZE"] = "3000"

            try:
                config_manager = ConfigManager(project_root=project_root)
                config = config_manager.load_config()

                # Environment variables should override file config
                assert config.log_level == "ERROR"
                assert config.queue.max_size == 3000
            finally:
                # Cleanup
                del os.environ["ABATHUR_LOG_LEVEL"]
                del os.environ["ABATHUR_QUEUE_MAX_SIZE"]

    def test_get_database_path(self) -> None:
        """Test getting database path."""
        with TemporaryDirectory() as tmpdir:
            project_root = Path(tmpdir)
            config_manager = ConfigManager(project_root=project_root)

            db_path = config_manager.get_database_path()

            assert db_path == project_root / ".abathur" / "abathur.db"
            assert db_path.parent.exists()  # Directory should be created

    def test_get_log_dir(self) -> None:
        """Test getting log directory."""
        with TemporaryDirectory() as tmpdir:
            project_root = Path(tmpdir)
            config_manager = ConfigManager(project_root=project_root)

            log_dir = config_manager.get_log_dir()

            assert log_dir == project_root / ".abathur" / "logs"
            assert log_dir.exists()  # Directory should be created

    def test_get_api_key_from_env(self) -> None:
        """Test getting API key from environment variable."""
        with TemporaryDirectory() as tmpdir:
            project_root = Path(tmpdir)
            config_manager = ConfigManager(project_root=project_root)

            # Set environment variable
            os.environ["ANTHROPIC_API_KEY"] = "test-key-123"

            try:
                api_key = config_manager.get_api_key()
                assert api_key == "test-key-123"
            finally:
                del os.environ["ANTHROPIC_API_KEY"]

    def test_get_api_key_not_found(self) -> None:
        """Test error when API key not found."""
        with TemporaryDirectory() as tmpdir:
            project_root = Path(tmpdir)
            config_manager = ConfigManager(project_root=project_root)

            # Ensure no API key is set
            if "ANTHROPIC_API_KEY" in os.environ:
                del os.environ["ANTHROPIC_API_KEY"]

            with pytest.raises(ValueError, match="ANTHROPIC_API_KEY not found"):
                config_manager.get_api_key()
