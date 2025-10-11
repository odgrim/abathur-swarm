"""Helper for managing the Abathur Memory MCP server lifecycle."""

import asyncio
import sys
from pathlib import Path
from typing import Any

from abathur.infrastructure.logger import get_logger

logger = get_logger(__name__)


class MemoryServerManager:
    """Manages the Abathur Memory MCP server lifecycle."""

    def __init__(self, db_path: Path):
        """Initialize memory server manager.

        Args:
            db_path: Path to SQLite database
        """
        self.db_path = db_path
        self.process: asyncio.subprocess.Process | None = None
        self._running = False

    async def start(self) -> bool:
        """Start the memory MCP server in background.

        Returns:
            True if server started successfully
        """
        if self._running and self.process:
            logger.warning("memory_mcp_server_already_running")
            return True

        try:
            logger.info("starting_memory_mcp_server", db_path=str(self.db_path))

            # Start server process
            self.process = await asyncio.create_subprocess_exec(
                sys.executable,
                "-m",
                "abathur.mcp.memory_server",
                "--db-path",
                str(self.db_path),
                stdin=asyncio.subprocess.PIPE,
                stdout=asyncio.subprocess.PIPE,
                stderr=asyncio.subprocess.PIPE,
            )

            self._running = True
            logger.info(
                "memory_mcp_server_started", pid=self.process.pid, db_path=str(self.db_path)
            )

            return True

        except Exception as e:
            logger.error("memory_mcp_server_start_failed", error=str(e))
            self._running = False
            return False

    async def stop(self) -> bool:
        """Stop the memory MCP server.

        Returns:
            True if server stopped successfully
        """
        if not self._running or not self.process:
            logger.warning("memory_mcp_server_not_running")
            return True

        try:
            logger.info("stopping_memory_mcp_server", pid=self.process.pid)

            # Graceful shutdown
            self.process.terminate()

            try:
                await asyncio.wait_for(self.process.wait(), timeout=5.0)
            except asyncio.TimeoutError:
                # Force kill if not responding
                logger.warning("memory_mcp_server_force_killing")
                self.process.kill()
                await self.process.wait()

            self._running = False
            self.process = None

            logger.info("memory_mcp_server_stopped")
            return True

        except Exception as e:
            logger.error("memory_mcp_server_stop_failed", error=str(e))
            return False

    async def is_running(self) -> bool:
        """Check if server is running.

        Returns:
            True if server is running
        """
        if not self._running or not self.process:
            return False

        # Check if process is alive
        return_code = self.process.returncode

        if return_code is not None:
            # Process died
            self._running = False
            self.process = None
            return False

        return True

    def get_status(self) -> dict[str, Any]:
        """Get server status.

        Returns:
            Status dictionary
        """
        return {
            "running": self._running,
            "pid": self.process.pid if self.process else None,
            "db_path": str(self.db_path),
        }
