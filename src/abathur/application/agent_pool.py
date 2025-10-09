"""Agent pool for managing concurrent agent lifecycle."""

import asyncio
from dataclasses import dataclass
from datetime import datetime, timezone
from uuid import UUID

from abathur.domain.models import Agent, AgentState
from abathur.infrastructure.database import Database
from abathur.infrastructure.logger import get_logger

logger = get_logger(__name__)


@dataclass
class PoolStats:
    """Agent pool statistics."""

    max_size: int
    active_count: int
    idle_count: int
    spawning_count: int
    terminating_count: int
    total_spawned: int
    total_terminated: int


class AgentPool:
    """Manages a pool of agents with lifecycle control."""

    def __init__(
        self,
        database: Database,
        max_pool_size: int = 10,
        idle_timeout: float = 300.0,  # 5 minutes
        health_check_interval: float = 30.0,
    ):
        """Initialize agent pool.

        Args:
            database: Database for agent state persistence
            max_pool_size: Maximum number of agents in pool
            idle_timeout: Timeout for idle agents in seconds
            health_check_interval: Interval for health checks in seconds
        """
        self.database = database
        self.max_pool_size = max_pool_size
        self.idle_timeout = idle_timeout
        self.health_check_interval = health_check_interval
        self.semaphore = asyncio.Semaphore(max_pool_size)
        self.agents: dict[UUID, Agent] = {}
        self.agent_last_activity: dict[UUID, datetime] = {}
        self.total_spawned = 0
        self.total_terminated = 0
        self._health_check_task: asyncio.Task | None = None

    async def acquire_agent(self, agent: Agent) -> bool:
        """Acquire a slot in the pool for an agent.

        Args:
            agent: Agent to acquire slot for

        Returns:
            True if slot acquired, False if pool is full
        """
        try:
            # Non-blocking acquire
            if self.semaphore.locked() and len(self.agents) >= self.max_pool_size:
                logger.warning("agent_pool_full", max_size=self.max_pool_size)
                return False

            await self.semaphore.acquire()
            self.agents[agent.id] = agent
            self.agent_last_activity[agent.id] = datetime.now(timezone.utc)
            self.total_spawned += 1

            await self.database.insert_agent(agent)
            await self.database.update_agent_state(agent.id, AgentState.IDLE)

            logger.info(
                "agent_acquired",
                agent_id=str(agent.id),
                pool_size=len(self.agents),
            )

            return True

        except Exception as e:
            logger.error("agent_acquire_failed", error=str(e))
            return False

    async def release_agent(self, agent_id: UUID) -> None:
        """Release an agent slot from the pool.

        Args:
            agent_id: ID of agent to release
        """
        if agent_id not in self.agents:
            logger.warning("agent_not_in_pool", agent_id=str(agent_id))
            return

        try:
            await self.database.update_agent_state(agent_id, AgentState.TERMINATING)
            await self.database.update_agent_state(agent_id, AgentState.TERMINATED)

            del self.agents[agent_id]
            if agent_id in self.agent_last_activity:
                del self.agent_last_activity[agent_id]

            self.semaphore.release()
            self.total_terminated += 1

            logger.info(
                "agent_released",
                agent_id=str(agent_id),
                pool_size=len(self.agents),
            )

        except Exception as e:
            logger.error("agent_release_failed", agent_id=str(agent_id), error=str(e))

    async def update_activity(self, agent_id: UUID) -> None:
        """Update the last activity timestamp for an agent.

        Args:
            agent_id: ID of agent to update
        """
        if agent_id in self.agent_last_activity:
            self.agent_last_activity[agent_id] = datetime.now(timezone.utc)

    async def start_health_monitoring(self) -> None:
        """Start background health monitoring task."""
        if self._health_check_task is None or self._health_check_task.done():
            self._health_check_task = asyncio.create_task(self._health_check_loop())
            logger.info("health_monitoring_started")

    async def stop_health_monitoring(self) -> None:
        """Stop background health monitoring task."""
        if self._health_check_task and not self._health_check_task.done():
            self._health_check_task.cancel()
            try:
                await self._health_check_task
            except asyncio.CancelledError:
                pass
            logger.info("health_monitoring_stopped")

    async def _health_check_loop(self) -> None:
        """Background task to monitor agent health and terminate idle agents."""
        try:
            while True:
                await asyncio.sleep(self.health_check_interval)
                await self._check_idle_agents()

        except asyncio.CancelledError:
            logger.info("health_check_cancelled")
        except Exception as e:
            logger.error("health_check_error", error=str(e))

    async def _check_idle_agents(self) -> None:
        """Check for and terminate idle agents."""
        now = datetime.now(timezone.utc)
        idle_agents = []

        for agent_id, last_activity in self.agent_last_activity.items():
            idle_time = (now - last_activity).total_seconds()
            if idle_time > self.idle_timeout:
                idle_agents.append(agent_id)

        if idle_agents:
            logger.info("terminating_idle_agents", count=len(idle_agents))
            for agent_id in idle_agents:
                await self.release_agent(agent_id)

    def get_stats(self) -> PoolStats:
        """Get current pool statistics.

        Returns:
            PoolStats object with current statistics
        """
        state_counts: dict[AgentState, int] = {}
        for agent in self.agents.values():
            state_counts[agent.state] = state_counts.get(agent.state, 0) + 1

        return PoolStats(
            max_size=self.max_pool_size,
            active_count=len(self.agents),
            idle_count=state_counts.get(AgentState.IDLE, 0),
            spawning_count=state_counts.get(AgentState.SPAWNING, 0),
            terminating_count=state_counts.get(AgentState.TERMINATING, 0),
            total_spawned=self.total_spawned,
            total_terminated=self.total_terminated,
        )

    async def shutdown(self) -> None:
        """Shutdown the agent pool and terminate all agents."""
        logger.info("shutting_down_agent_pool", active_agents=len(self.agents))

        # Stop health monitoring
        await self.stop_health_monitoring()

        # Terminate all agents
        agent_ids = list(self.agents.keys())
        for agent_id in agent_ids:
            await self.release_agent(agent_id)

        logger.info("agent_pool_shutdown_complete")

    def get_available_capacity(self) -> int:
        """Get number of available agent slots.

        Returns:
            Number of available slots
        """
        return self.max_pool_size - len(self.agents)

    def is_full(self) -> bool:
        """Check if agent pool is full.

        Returns:
            True if pool is at capacity
        """
        return len(self.agents) >= self.max_pool_size
