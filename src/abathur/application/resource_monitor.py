"""Resource monitoring for tracking system and agent resource usage."""

import asyncio
from dataclasses import dataclass

import psutil

from abathur.infrastructure.logger import get_logger

logger = get_logger(__name__)


@dataclass
class ResourceSnapshot:
    """Snapshot of system resource usage."""

    timestamp: float
    cpu_percent: float
    memory_mb: float
    memory_percent: float
    available_memory_mb: float
    agent_count: int


@dataclass
class ResourceLimits:
    """Resource limits configuration."""

    max_memory_per_agent_mb: int = 512
    max_total_memory_mb: int = 4096
    max_cpu_percent: float = 80.0
    warning_memory_percent: float = 80.0


class ResourceMonitor:
    """Monitors system and agent resource usage."""

    def __init__(
        self,
        limits: ResourceLimits | None = None,
        check_interval: float = 5.0,
    ):
        """Initialize resource monitor.

        Args:
            limits: Resource limits configuration
            check_interval: Interval between resource checks in seconds
        """
        self.limits = limits or ResourceLimits()
        self.check_interval = check_interval
        self.snapshots: list[ResourceSnapshot] = []
        self.max_snapshots = 100  # Keep last 100 snapshots
        self._monitor_task: asyncio.Task | None = None
        self._process = psutil.Process()

    async def start_monitoring(self) -> None:
        """Start background resource monitoring."""
        if self._monitor_task is None or self._monitor_task.done():
            self._monitor_task = asyncio.create_task(self._monitor_loop())
            logger.info("resource_monitoring_started")

    async def stop_monitoring(self) -> None:
        """Stop background resource monitoring."""
        if self._monitor_task and not self._monitor_task.done():
            self._monitor_task.cancel()
            try:
                await self._monitor_task
            except asyncio.CancelledError:
                pass
            logger.info("resource_monitoring_stopped")

    async def _monitor_loop(self) -> None:
        """Background task to monitor resource usage."""
        try:
            while True:
                await asyncio.sleep(self.check_interval)
                snapshot = await self.get_snapshot()
                self._check_limits(snapshot)

        except asyncio.CancelledError:
            logger.info("resource_monitor_cancelled")
        except Exception as e:
            logger.error("resource_monitor_error", error=str(e))

    async def get_snapshot(self, agent_count: int = 0) -> ResourceSnapshot:
        """Get current resource usage snapshot.

        Args:
            agent_count: Number of active agents

        Returns:
            ResourceSnapshot with current usage
        """
        try:
            # Get system-wide metrics
            cpu_percent = psutil.cpu_percent(interval=0.1)
            memory = psutil.virtual_memory()

            # Get process memory
            process_memory = self._process.memory_info().rss / (1024 * 1024)  # MB

            snapshot = ResourceSnapshot(
                timestamp=asyncio.get_event_loop().time(),
                cpu_percent=cpu_percent,
                memory_mb=process_memory,
                memory_percent=memory.percent,
                available_memory_mb=memory.available / (1024 * 1024),
                agent_count=agent_count,
            )

            # Store snapshot
            self.snapshots.append(snapshot)
            if len(self.snapshots) > self.max_snapshots:
                self.snapshots.pop(0)

            return snapshot

        except Exception as e:
            logger.error("snapshot_error", error=str(e))
            raise

    def _check_limits(self, snapshot: ResourceSnapshot) -> None:
        """Check if resource usage exceeds limits and log warnings.

        Args:
            snapshot: Resource snapshot to check
        """
        # Check memory usage
        if snapshot.memory_percent >= self.limits.warning_memory_percent:
            logger.warning(
                "high_memory_usage",
                percent=snapshot.memory_percent,
                mb=snapshot.memory_mb,
            )

        if snapshot.memory_mb >= self.limits.max_total_memory_mb:
            logger.error(
                "memory_limit_exceeded",
                current_mb=snapshot.memory_mb,
                limit_mb=self.limits.max_total_memory_mb,
            )

        # Check CPU usage
        if snapshot.cpu_percent >= self.limits.max_cpu_percent:
            logger.warning(
                "high_cpu_usage",
                percent=snapshot.cpu_percent,
                limit=self.limits.max_cpu_percent,
            )

        # Check per-agent memory estimate
        if snapshot.agent_count > 0:
            per_agent_mb = snapshot.memory_mb / snapshot.agent_count
            if per_agent_mb >= self.limits.max_memory_per_agent_mb:
                logger.warning(
                    "high_per_agent_memory",
                    per_agent_mb=per_agent_mb,
                    limit_mb=self.limits.max_memory_per_agent_mb,
                )

    def can_spawn_agent(self, agent_count: int) -> bool:
        """Check if resources allow spawning another agent.

        Args:
            agent_count: Current number of agents

        Returns:
            True if resources allow spawning another agent
        """
        if not self.snapshots:
            return True  # No data yet, allow spawn

        latest = self.snapshots[-1]

        # Check memory
        estimated_new_memory = latest.memory_mb + self.limits.max_memory_per_agent_mb
        if estimated_new_memory >= self.limits.max_total_memory_mb:
            logger.warning(
                "cannot_spawn_agent_memory",
                current_mb=latest.memory_mb,
                estimated_mb=estimated_new_memory,
                limit_mb=self.limits.max_total_memory_mb,
            )
            return False

        # Check CPU (soft limit)
        if latest.cpu_percent >= self.limits.max_cpu_percent:
            logger.warning(
                "cannot_spawn_agent_cpu",
                current_percent=latest.cpu_percent,
                limit_percent=self.limits.max_cpu_percent,
            )
            return False

        return True

    def get_stats(self) -> dict:
        """Get resource usage statistics.

        Returns:
            Dictionary with statistics
        """
        if not self.snapshots:
            return {
                "snapshots_count": 0,
                "current": None,
                "average": None,
            }

        latest = self.snapshots[-1]
        avg_cpu = sum(s.cpu_percent for s in self.snapshots) / len(self.snapshots)
        avg_memory = sum(s.memory_mb for s in self.snapshots) / len(self.snapshots)

        return {
            "snapshots_count": len(self.snapshots),
            "current": {
                "cpu_percent": latest.cpu_percent,
                "memory_mb": latest.memory_mb,
                "memory_percent": latest.memory_percent,
                "available_memory_mb": latest.available_memory_mb,
                "agent_count": latest.agent_count,
            },
            "average": {
                "cpu_percent": avg_cpu,
                "memory_mb": avg_memory,
            },
            "limits": {
                "max_memory_per_agent_mb": self.limits.max_memory_per_agent_mb,
                "max_total_memory_mb": self.limits.max_total_memory_mb,
                "max_cpu_percent": self.limits.max_cpu_percent,
            },
        }

    def clear_history(self) -> None:
        """Clear snapshot history."""
        self.snapshots.clear()
        logger.info("resource_history_cleared")
