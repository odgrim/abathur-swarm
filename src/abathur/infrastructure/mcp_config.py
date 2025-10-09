"""MCP (Model Context Protocol) configuration loading."""

import json
from dataclasses import dataclass
from pathlib import Path
from typing import Any

from abathur.infrastructure.logger import get_logger

logger = get_logger(__name__)


@dataclass
class MCPServer:
    """MCP Server configuration."""

    name: str
    command: str
    args: list[str]
    env: dict[str, str]


class MCPConfigLoader:
    """Loads MCP server configurations."""

    def __init__(self, project_root: Path | None = None):
        """Initialize MCP config loader.

        Args:
            project_root: Project root directory (default: current directory)
        """
        self.project_root = project_root or Path.cwd()

    def load_mcp_config(self) -> dict[str, MCPServer]:
        """Load MCP server configuration.

        Checks for configuration in:
        1. .mcp.json (project root)
        2. .claude/mcp.json

        Returns:
            Dictionary mapping server names to MCPServer objects
        """
        # Try project root first
        mcp_paths = [
            self.project_root / ".mcp.json",
            self.project_root / ".claude" / "mcp.json",
        ]

        for mcp_path in mcp_paths:
            if mcp_path.exists():
                logger.info("loading_mcp_config", path=str(mcp_path))
                return self._parse_mcp_config(mcp_path)

        logger.warning("no_mcp_config_found")
        return {}

    def _parse_mcp_config(self, config_path: Path) -> dict[str, MCPServer]:
        """Parse MCP configuration file.

        Args:
            config_path: Path to MCP config file

        Returns:
            Dictionary of MCP servers
        """
        try:
            with open(config_path) as f:
                config = json.load(f)

            servers = {}
            mcp_servers = config.get("mcpServers", {})

            for name, server_config in mcp_servers.items():
                # Expand environment variables in env values
                env = {}
                for key, value in server_config.get("env", {}).items():
                    # Simple ${VAR} expansion
                    if isinstance(value, str) and value.startswith("${") and value.endswith("}"):
                        import os

                        env_var = value[2:-1]
                        env[key] = os.getenv(env_var, "")
                    else:
                        env[key] = value

                servers[name] = MCPServer(
                    name=name,
                    command=server_config.get("command", ""),
                    args=server_config.get("args", []),
                    env=env,
                )

                logger.info("mcp_server_loaded", name=name, command=server_config.get("command"))

            return servers

        except Exception as e:
            logger.error("mcp_config_parse_error", path=str(config_path), error=str(e))
            return {}

    def validate_mcp_config(self, servers: dict[str, MCPServer]) -> dict[str, list[str]]:
        """Validate MCP server configurations.

        Args:
            servers: Dictionary of MCP servers to validate

        Returns:
            Dictionary mapping server names to lists of validation errors
        """
        errors = {}

        for name, server in servers.items():
            server_errors = []

            if not server.command:
                server_errors.append("Missing command")

            # Check if required environment variables are set
            for key, value in server.env.items():
                if not value:
                    server_errors.append(f"Environment variable {key} is not set")

            if server_errors:
                errors[name] = server_errors

        return errors

    def get_sdk_config(self, servers: dict[str, MCPServer]) -> dict[str, Any]:
        """Convert MCP servers to Claude Agent SDK configuration format.

        Args:
            servers: Dictionary of MCP servers

        Returns:
            Configuration dictionary for Claude Agent SDK
        """
        # Format for Claude Agent SDK (future use)
        sdk_config = {}

        for name, server in servers.items():
            sdk_config[name] = {
                "type": "stdio",
                "command": server.command,
                "args": server.args,
                "env": server.env,
            }

        return sdk_config
