"""MCP server lifecycle management and integration with Claude Agent SDK."""

import asyncio
from dataclasses import dataclass
from datetime import datetime, timezone
from enum import Enum
from typing import Any
from uuid import UUID, uuid4

from abathur.infrastructure.logger import get_logger
from abathur.infrastructure.mcp_config import MCPConfigLoader, MCPServer

logger = get_logger(__name__)


class MCPServerState(str, Enum):
    """MCP server states."""

    STOPPED = "stopped"
    STARTING = "starting"
    RUNNING = "running"
    STOPPING = "stopping"
    FAILED = "failed"


@dataclass
class MCPServerProcess:
    """Running MCP server process."""

    id: UUID
    server_config: MCPServer
    state: MCPServerState
    process: asyncio.subprocess.Process | None
    started_at: datetime | None
    last_health_check: datetime | None
    error_message: str | None = None


class MCPManager:
    """Manages MCP server lifecycle and integration with Claude Agent SDK."""

    def __init__(self, config_loader: MCPConfigLoader | None = None):
        """Initialize MCP manager.

        Args:
            config_loader: MCP configuration loader (default: new instance)
        """
        self.config_loader = config_loader or MCPConfigLoader()
        self.servers: dict[str, MCPServer] = {}
        self.running_processes: dict[str, MCPServerProcess] = {}
        self._health_check_task: asyncio.Task | None = None

    async def initialize(self) -> None:
        """Initialize MCP manager and load server configurations."""
        logger.info("mcp_manager_initializing")

        # Load MCP configurations
        self.servers = self.config_loader.load_mcp_config()

        # Validate configurations
        errors = self.config_loader.validate_mcp_config(self.servers)
        if errors:
            for server_name, server_errors in errors.items():
                logger.error(
                    "mcp_server_validation_failed",
                    server=server_name,
                    errors=server_errors,
                )

        logger.info(
            "mcp_manager_initialized",
            server_count=len(self.servers),
        )

    async def start_server(self, server_name: str) -> bool:
        """Start an MCP server.

        Args:
            server_name: Name of server to start

        Returns:
            True if server started successfully
        """
        if server_name not in self.servers:
            logger.error("mcp_server_not_found", server=server_name)
            return False

        if server_name in self.running_processes:
            logger.warning("mcp_server_already_running", server=server_name)
            return True

        server_config = self.servers[server_name]

        try:
            logger.info(
                "mcp_server_starting",
                server=server_name,
                command=server_config.command,
            )

            # Create server process entry
            server_process = MCPServerProcess(
                id=uuid4(),
                server_config=server_config,
                state=MCPServerState.STARTING,
                process=None,
                started_at=None,
                last_health_check=None,
            )

            self.running_processes[server_name] = server_process

            # Start subprocess
            env = {**server_config.env}  # Copy environment
            process = await asyncio.create_subprocess_exec(
                server_config.command,
                *server_config.args,
                stdin=asyncio.subprocess.PIPE,
                stdout=asyncio.subprocess.PIPE,
                stderr=asyncio.subprocess.PIPE,
                env=env,
            )

            # Update process info
            server_process.process = process
            server_process.state = MCPServerState.RUNNING
            server_process.started_at = datetime.now(timezone.utc)

            logger.info(
                "mcp_server_started",
                server=server_name,
                pid=process.pid,
            )

            # Start health check monitoring
            if not self._health_check_task:
                await self.start_health_monitoring()

            return True

        except Exception as e:
            logger.error(
                "mcp_server_start_failed",
                server=server_name,
                error=str(e),
            )

            if server_name in self.running_processes:
                self.running_processes[server_name].state = MCPServerState.FAILED
                self.running_processes[server_name].error_message = str(e)

            return False

    async def stop_server(self, server_name: str) -> bool:
        """Stop an MCP server.

        Args:
            server_name: Name of server to stop

        Returns:
            True if server stopped successfully
        """
        if server_name not in self.running_processes:
            logger.warning("mcp_server_not_running", server=server_name)
            return True

        server_process = self.running_processes[server_name]

        try:
            logger.info("mcp_server_stopping", server=server_name)

            server_process.state = MCPServerState.STOPPING

            if server_process.process:
                # Try graceful shutdown first
                server_process.process.terminate()

                try:
                    await asyncio.wait_for(server_process.process.wait(), timeout=5.0)
                except asyncio.TimeoutError:
                    # Force kill if not responding
                    logger.warning("mcp_server_force_killing", server=server_name)
                    server_process.process.kill()
                    await server_process.process.wait()

            server_process.state = MCPServerState.STOPPED
            del self.running_processes[server_name]

            logger.info("mcp_server_stopped", server=server_name)
            return True

        except Exception as e:
            logger.error(
                "mcp_server_stop_failed",
                server=server_name,
                error=str(e),
            )
            return False

    async def start_all_servers(self) -> dict[str, bool]:
        """Start all configured MCP servers.

        Returns:
            Dictionary mapping server names to start success status
        """
        results = {}

        for server_name in self.servers.keys():
            success = await self.start_server(server_name)
            results[server_name] = success

        return results

    async def stop_all_servers(self) -> dict[str, bool]:
        """Stop all running MCP servers.

        Returns:
            Dictionary mapping server names to stop success status
        """
        results = {}

        # Stop health monitoring first
        await self.stop_health_monitoring()

        for server_name in list(self.running_processes.keys()):
            success = await self.stop_server(server_name)
            results[server_name] = success

        return results

    async def restart_server(self, server_name: str) -> bool:
        """Restart an MCP server.

        Args:
            server_name: Name of server to restart

        Returns:
            True if server restarted successfully
        """
        logger.info("mcp_server_restarting", server=server_name)

        await self.stop_server(server_name)
        await asyncio.sleep(1.0)  # Brief pause before restart
        return await self.start_server(server_name)

    async def start_health_monitoring(self, check_interval: float = 30.0) -> None:
        """Start background health monitoring for MCP servers.

        Args:
            check_interval: Interval between health checks in seconds
        """
        if self._health_check_task is None or self._health_check_task.done():
            self._health_check_task = asyncio.create_task(self._health_check_loop(check_interval))
            logger.info("mcp_health_monitoring_started", interval=check_interval)

    async def stop_health_monitoring(self) -> None:
        """Stop background health monitoring."""
        if self._health_check_task and not self._health_check_task.done():
            self._health_check_task.cancel()
            try:
                await self._health_check_task
            except asyncio.CancelledError:
                pass
            logger.info("mcp_health_monitoring_stopped")

    async def _health_check_loop(self, check_interval: float) -> None:
        """Background task to check health of MCP servers."""
        try:
            while True:
                await asyncio.sleep(check_interval)
                await self._check_server_health()

        except asyncio.CancelledError:
            logger.info("mcp_health_check_loop_cancelled")
        except Exception as e:
            logger.error("mcp_health_check_error", error=str(e))

    async def _check_server_health(self) -> None:
        """Check health of all running MCP servers."""
        for server_name, server_process in list(self.running_processes.items()):
            if server_process.state != MCPServerState.RUNNING:
                continue

            if not server_process.process:
                continue

            # Check if process is still alive
            return_code = server_process.process.returncode

            if return_code is not None:
                # Process has terminated
                logger.error(
                    "mcp_server_died",
                    server=server_name,
                    return_code=return_code,
                )

                server_process.state = MCPServerState.FAILED
                server_process.error_message = f"Process exited with code {return_code}"

                # Attempt restart
                logger.info("mcp_server_auto_restarting", server=server_name)
                await self.restart_server(server_name)

            else:
                # Process is alive, update health check timestamp
                server_process.last_health_check = datetime.now(timezone.utc)

    def get_server_status(self, server_name: str) -> dict[str, Any] | None:
        """Get status of an MCP server.

        Args:
            server_name: Name of server

        Returns:
            Status dictionary or None if server not found
        """
        if server_name not in self.servers:
            return None

        server_config = self.servers[server_name]
        server_process = self.running_processes.get(server_name)

        status: dict[str, Any] = {
            "name": server_name,
            "command": server_config.command,
            "state": server_process.state.value if server_process else "stopped",
        }

        if server_process:
            status.update(
                {
                    "pid": server_process.process.pid if server_process.process else None,
                    "started_at": server_process.started_at.isoformat()
                    if server_process.started_at
                    else None,
                    "last_health_check": server_process.last_health_check.isoformat()
                    if server_process.last_health_check
                    else None,
                    "error_message": server_process.error_message,
                }
            )

        return status

    def get_all_server_status(self) -> dict[str, dict[str, Any]]:
        """Get status of all MCP servers.

        Returns:
            Dictionary mapping server names to status dictionaries
        """
        return {
            name: status
            for name in self.servers.keys()
            if (status := self.get_server_status(name)) is not None
        }

    def get_sdk_config(self) -> dict[str, Any]:
        """Get Claude Agent SDK configuration for MCP servers.

        Returns:
            Configuration dictionary for Claude Agent SDK
        """
        return self.config_loader.get_sdk_config(self.servers)

    def bind_agent_to_servers(self, agent_id: UUID, server_names: list[str]) -> dict[str, bool]:
        """Bind an agent to specific MCP servers.

        Args:
            agent_id: Agent ID to bind
            server_names: List of server names to bind to

        Returns:
            Dictionary mapping server names to bind success status
        """
        results = {}

        for server_name in server_names:
            if server_name not in self.servers:
                logger.error(
                    "mcp_server_not_found_for_binding",
                    agent_id=str(agent_id),
                    server=server_name,
                )
                results[server_name] = False
                continue

            if server_name not in self.running_processes:
                logger.warning(
                    "mcp_server_not_running_for_binding",
                    agent_id=str(agent_id),
                    server=server_name,
                )
                results[server_name] = False
                continue

            # In a real implementation, this would configure the agent
            # to use the MCP server via the Claude Agent SDK
            logger.info(
                "agent_bound_to_mcp_server",
                agent_id=str(agent_id),
                server=server_name,
            )
            results[server_name] = True

        return results

    async def shutdown(self) -> None:
        """Shutdown MCP manager and stop all servers."""
        logger.info("mcp_manager_shutting_down")

        await self.stop_all_servers()

        logger.info("mcp_manager_shutdown_complete")
