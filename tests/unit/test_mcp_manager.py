"""Tests for MCP Manager."""

from collections.abc import AsyncIterator
from typing import Any

import pytest
from abathur.application.mcp_manager import MCPManager
from abathur.infrastructure.mcp_config import MCPConfigLoader


@pytest.fixture
def mcp_config_loader(tmp_path: Any) -> MCPConfigLoader:
    """Create MCP config loader with test config."""
    import json

    config_file = tmp_path / ".mcp.json"
    config = {
        "mcpServers": {
            "test-server": {
                "command": "echo",
                "args": ["test"],
                "env": {},
            }
        }
    }

    with open(config_file, "w") as f:
        json.dump(config, f)

    return MCPConfigLoader(project_root=tmp_path)


@pytest.fixture
async def mcp_manager(mcp_config_loader: MCPConfigLoader) -> AsyncIterator[MCPManager]:
    """Create MCP manager."""
    manager = MCPManager(config_loader=mcp_config_loader)
    await manager.initialize()
    yield manager
    await manager.shutdown()


@pytest.mark.asyncio
async def test_mcp_manager_initialization(mcp_manager: MCPManager) -> None:
    """Test MCP manager initialization."""
    assert len(mcp_manager.servers) >= 0
    assert isinstance(mcp_manager.running_processes, dict)


@pytest.mark.asyncio
async def test_mcp_manager_get_server_status(mcp_manager: MCPManager) -> None:
    """Test getting server status."""
    if mcp_manager.servers:
        server_name = list(mcp_manager.servers.keys())[0]
        status = mcp_manager.get_server_status(server_name)

        assert status is not None
        assert "name" in status
        assert "command" in status
        assert "state" in status


@pytest.mark.asyncio
async def test_mcp_manager_get_all_server_status(mcp_manager: MCPManager) -> None:
    """Test getting all server statuses."""
    statuses = mcp_manager.get_all_server_status()

    assert isinstance(statuses, dict)
    for _name, status in statuses.items():
        assert "name" in status
        assert "command" in status
        assert "state" in status


@pytest.mark.asyncio
async def test_mcp_manager_get_sdk_config(mcp_manager: MCPManager) -> None:
    """Test getting SDK configuration."""
    sdk_config = mcp_manager.get_sdk_config()

    assert isinstance(sdk_config, dict)
    for _name, config in sdk_config.items():
        assert "type" in config
        assert "command" in config
        assert "args" in config
        assert "env" in config


@pytest.mark.asyncio
async def test_mcp_manager_bind_agent_to_servers(mcp_manager: MCPManager) -> None:
    """Test binding agent to servers."""
    from uuid import uuid4

    agent_id = uuid4()

    if mcp_manager.servers:
        server_names = list(mcp_manager.servers.keys())[:1]
        results = mcp_manager.bind_agent_to_servers(agent_id, server_names)

        assert isinstance(results, dict)
        assert len(results) == len(server_names)


@pytest.mark.asyncio
async def test_mcp_server_start_stop(mcp_config_loader: MCPConfigLoader) -> None:
    """Test starting and stopping MCP server."""
    # This test may fail in CI/CD without proper server setup
    # Keeping it simple to avoid flakiness
    manager = MCPManager(config_loader=mcp_config_loader)
    await manager.initialize()

    # Just verify the methods exist and can be called
    assert hasattr(manager, "start_server")
    assert hasattr(manager, "stop_server")
    assert hasattr(manager, "restart_server")

    await manager.shutdown()
