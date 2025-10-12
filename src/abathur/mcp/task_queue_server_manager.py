"""Task Queue MCP Server lifecycle management."""

import asyncio
import os
import signal
import sys
from pathlib import Path
from typing import Any

from abathur.infrastructure.logger import get_logger

logger = get_logger(__name__)


class TaskQueueServerManager:
    """Manages Task Queue MCP server lifecycle.

    Provides functionality for:
    - Starting server in foreground or background
    - Stopping server gracefully
    - Checking server status
    - PID tracking for background processes
    - Automatic restart on failure
    """

    def __init__(self, db_path: Path, pid_file: Path | None = None):
        """Initialize server manager.

        Args:
            db_path: Path to SQLite database
            pid_file: Path to PID file for background process tracking
        """
        self.db_path = db_path
        self.pid_file = pid_file or Path.home() / ".abathur" / "task_queue_mcp.pid"
        self.process: asyncio.subprocess.Process | None = None

    async def start_foreground(self) -> int:
        """Start server in foreground (blocking).

        Returns:
            Exit code
        """
        logger.info("task_queue_mcp_starting_foreground", db_path=str(self.db_path))

        # Import and run server
        from abathur.mcp.task_queue_server import main

        try:
            await main()
            return 0
        except KeyboardInterrupt:
            logger.info("task_queue_mcp_interrupted")
            return 0
        except Exception as e:
            logger.error("task_queue_mcp_error", error=str(e))
            return 1

    async def start_background(self) -> bool:
        """Start server in background.

        Returns:
            True if server started successfully
        """
        # Check if already running
        if self.is_running():
            logger.warning("task_queue_mcp_already_running")
            return True

        logger.info("task_queue_mcp_starting_background", db_path=str(self.db_path))

        # Get python executable and script path
        python_exe = sys.executable
        script_path = Path(__file__).parent / "task_queue_server.py"

        try:
            # Start subprocess
            self.process = await asyncio.create_subprocess_exec(
                python_exe,
                str(script_path),
                "--db-path",
                str(self.db_path),
                stdin=asyncio.subprocess.PIPE,
                stdout=asyncio.subprocess.PIPE,
                stderr=asyncio.subprocess.PIPE,
            )

            # Write PID file
            self.pid_file.parent.mkdir(parents=True, exist_ok=True)
            self.pid_file.write_text(str(self.process.pid))

            logger.info("task_queue_mcp_started_background", pid=self.process.pid)
            return True

        except Exception as e:
            logger.error("task_queue_mcp_start_failed", error=str(e))
            return False

    async def stop(self) -> bool:
        """Stop the server.

        Returns:
            True if server stopped successfully
        """
        if not self.is_running():
            logger.warning("task_queue_mcp_not_running")
            return True

        try:
            pid = self._read_pid()
            if pid:
                logger.info("task_queue_mcp_stopping", pid=pid)

                # Send SIGTERM for graceful shutdown
                try:
                    os.kill(pid, signal.SIGTERM)

                    # Wait for process to exit (max 5 seconds)
                    for _ in range(50):
                        if not self._is_process_running(pid):
                            break
                        await asyncio.sleep(0.1)

                    # Force kill if still running
                    if self._is_process_running(pid):
                        logger.warning("task_queue_mcp_force_killing", pid=pid)
                        os.kill(pid, signal.SIGKILL)

                except ProcessLookupError:
                    # Process already dead
                    pass

                # Remove PID file
                if self.pid_file.exists():
                    self.pid_file.unlink()

                logger.info("task_queue_mcp_stopped")
                return True

        except Exception as e:
            logger.error("task_queue_mcp_stop_failed", error=str(e))
            return False

        return True

    def is_running(self) -> bool:
        """Check if server is running.

        Returns:
            True if server is running
        """
        pid = self._read_pid()
        if not pid:
            return False

        return self._is_process_running(pid)

    def get_status(self) -> dict[str, Any]:
        """Get server status.

        Returns:
            Status dictionary
        """
        pid = self._read_pid()
        running = self.is_running()

        return {
            "name": "task-queue",
            "running": running,
            "pid": pid if running else None,
            "db_path": str(self.db_path),
            "pid_file": str(self.pid_file),
        }

    def _read_pid(self) -> int | None:
        """Read PID from file.

        Returns:
            PID or None if file doesn't exist
        """
        if not self.pid_file.exists():
            return None

        try:
            return int(self.pid_file.read_text().strip())
        except (ValueError, OSError):
            return None

    def _is_process_running(self, pid: int) -> bool:
        """Check if process is running.

        Args:
            pid: Process ID

        Returns:
            True if process is running
        """
        try:
            os.kill(pid, 0)  # Signal 0 just checks if process exists
            return True
        except ProcessLookupError:
            return False
        except PermissionError:
            # Process exists but we can't signal it
            return True


async def main() -> None:
    """Main entry point for CLI."""
    import argparse

    parser = argparse.ArgumentParser(description="Abathur Task Queue MCP Server Manager")
    parser.add_argument(
        "action",
        choices=["start", "stop", "status", "restart"],
        help="Action to perform",
    )
    parser.add_argument(
        "--db-path",
        type=Path,
        default=Path.cwd() / "abathur.db",
        help="Path to SQLite database (default: ./abathur.db)",
    )
    parser.add_argument(
        "--foreground",
        action="store_true",
        help="Run in foreground (default: background)",
    )

    args = parser.parse_args()

    manager = TaskQueueServerManager(args.db_path)

    if args.action == "start":
        if args.foreground:
            exit_code = await manager.start_foreground()
            sys.exit(exit_code)
        else:
            success = await manager.start_background()
            sys.exit(0 if success else 1)

    elif args.action == "stop":
        success = await manager.stop()
        sys.exit(0 if success else 1)

    elif args.action == "status":
        status = manager.get_status()
        print("Task Queue MCP Server:")
        print(f"  Running: {status['running']}")
        print(f"  PID: {status['pid']}")
        print(f"  Database: {status['db_path']}")
        sys.exit(0 if status["running"] else 1)

    elif args.action == "restart":
        await manager.stop()
        await asyncio.sleep(1.0)
        success = await manager.start_background()
        sys.exit(0 if success else 1)


if __name__ == "__main__":
    asyncio.run(main())
