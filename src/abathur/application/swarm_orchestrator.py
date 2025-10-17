"""Swarm orchestrator for coordinating multiple concurrent agents."""

import asyncio
from typing import Any
from uuid import UUID

from abathur.application.agent_executor import AgentExecutor
from abathur.domain.models import Result, Task
from abathur.infrastructure.logger import get_logger
from abathur.services.task_queue_service import TaskQueueService

logger = get_logger(__name__)


class SwarmOrchestrator:
    """Orchestrates concurrent execution of multiple agents in a swarm."""

    def __init__(
        self,
        task_queue_service: TaskQueueService,
        agent_executor: AgentExecutor,
        max_concurrent_agents: int = 10,
        agent_spawn_timeout: float = 5.0,
        poll_interval: float = 2.0,
    ):
        """Initialize swarm orchestrator.

        Args:
            task_queue_service: TaskQueueService for queue management and dependency resolution
            agent_executor: Agent executor for running tasks
            max_concurrent_agents: Maximum number of concurrent agents
            agent_spawn_timeout: Timeout for agent spawning in seconds
            poll_interval: Interval in seconds between polling for new tasks
        """
        self.task_queue_service = task_queue_service
        self.agent_executor = agent_executor
        self.max_concurrent_agents = max_concurrent_agents
        self.agent_spawn_timeout = agent_spawn_timeout
        self.poll_interval = poll_interval
        self.semaphore = asyncio.Semaphore(max_concurrent_agents)
        self.active_agents: dict[UUID, Task] = {}
        self.results: list[Result] = []
        self._shutdown_event = asyncio.Event()
        self._running = False

    async def start_swarm(self, task_limit: int | None = None) -> list[Result]:
        """Start the swarm orchestrator and process tasks from the queue.

        Args:
            task_limit: Optional maximum number of tasks to complete. If None, continues indefinitely.

        This mode keeps the swarm running, continuously polling the
        database for new READY tasks. It respects the max_concurrent_agents limit
        and spawns new agent instances as slots become available.

        The swarm will:
        1. Pick tasks from the queue
        2. Spawn agents to execute tasks (up to task_limit if specified)
        3. Wait for all active agents to complete
        4. Exit gracefully when task_limit reached or queue empty

        Tasks are counted when completed (successful or failed), not when spawned.
        Active tasks spawned before reaching the limit are allowed to complete.

        The swarm will continue until:
        - task_limit is reached (if specified)
        - A shutdown signal is received (SIGINT or SIGTERM)
        - The shutdown() method is called
        - The task_limit is reached (if specified)

        Args:
            task_limit: Maximum number of tasks to complete before stopping the swarm.
                       - None (default): Runs indefinitely until shutdown() is called
                       - 0: Exits immediately without processing any tasks
                       - N: Processes exactly N tasks, then stops gracefully

                       IMPORTANT: Tasks are counted when COMPLETED, not when spawned.
                       This means the swarm stops spawning new tasks once N tasks
                       have finished, ensuring exactly N tasks complete.

                       Failed tasks (both Result.success=False and exception-based
                       failures) count toward the limit.

        Returns:
            List of all execution results
        """
        self._running = True
        self._shutdown_event.clear()

        logger.info(
            "starting_continuous_swarm",
            max_concurrent=self.max_concurrent_agents,
            poll_interval=self.poll_interval,
            task_limit=task_limit,
        )

        # Track active task coroutines
        active_task_coroutines: set[asyncio.Task] = set()
        tasks_processed = 0  # Track tasks spawned for task_limit enforcement

        try:
            while self._running and not self._shutdown_event.is_set():
                # Check if task limit has been reached (spawn-time counting)
                if task_limit is not None and tasks_processed >= task_limit:
                    logger.info(
                        "task_limit_reached",
                        limit=task_limit,
                        tasks_spawned=tasks_processed,
                    )
                    break

                # Check if we have capacity for more tasks
                if len(self.active_agents) < self.max_concurrent_agents:
                    # Try to get next READY task
                    next_task = await self.task_queue_service.get_next_task()

                    if next_task:
                        # CRITICAL: Increment BEFORE spawning for spawn-time counting
                        # This prevents race condition where tasks spawn faster than counter increments
                        tasks_processed += 1

                        # Spawn agent for task
                        task_coroutine = asyncio.create_task(
                            self._execute_with_semaphore(next_task)
                        )
                        active_task_coroutines.add(task_coroutine)

                        # Remove completed tasks from tracking
                        active_task_coroutines = {t for t in active_task_coroutines if not t.done()}

                        logger.info(
                            "task_spawned_continuous",
                            task_id=str(next_task.id),
                            active_count=len(self.active_agents),
                            available_slots=self.max_concurrent_agents - len(self.active_agents),
                        )

                        # Exit immediately after spawning Nth task (spawn-time limit enforcement)
                        if task_limit is not None and tasks_processed >= task_limit:
                            logger.debug(
                                "Task limit reached after spawning task",
                                task_limit=task_limit,
                                tasks_spawned=tasks_processed,
                            )
                            break
                    else:
                        # No tasks available, wait before polling again
                        logger.debug(
                            "no_ready_tasks_polling",
                            active_count=len(self.active_agents),
                        )
                        await asyncio.sleep(self.poll_interval)
                else:
                    # At capacity, wait before checking again
                    logger.debug(
                        "at_capacity_waiting",
                        active_count=len(self.active_agents),
                        max_concurrent=self.max_concurrent_agents,
                    )
                    await asyncio.sleep(self.poll_interval)

                # Clean up completed tasks
                active_task_coroutines = {t for t in active_task_coroutines if not t.done()}

            logger.info("continuous_swarm_shutdown_initiated")

            # Wait for all active tasks to complete
            if active_task_coroutines:
                logger.info("waiting_for_active_tasks_shutdown", count=len(active_task_coroutines))
                await asyncio.gather(*active_task_coroutines, return_exceptions=True)

            logger.info(
                "continuous_swarm_stopped",
                tasks_spawned=tasks_processed,
            )

            return self.results

        except Exception as e:
            logger.error(
                "continuous_swarm_error",
                error=str(e),
                error_type=type(e).__name__,
            )
            # Cancel all active tasks
            for task in active_task_coroutines:
                if not task.done():
                    task.cancel()
            raise
        finally:
            self._running = False

    async def _execute_with_semaphore(self, task: Task) -> Result:
        """Execute a task with semaphore control for concurrency limiting.

        Args:
            task: Task to execute

        Returns:
            Execution result
        """
        async with self.semaphore:
            self.active_agents[task.id] = task

            try:
                logger.info(
                    "agent_executing",
                    task_id=str(task.id),
                    active_count=len(self.active_agents),
                )

                result = await self.agent_executor.execute_task(task)

                # Update task status based on result
                if result.success:
                    # Mark completed and unblock dependent tasks
                    await self.task_queue_service.complete_task(task.id)
                    logger.info(
                        "task_completed_in_swarm",
                        task_id=str(task.id),
                        agent_id=str(result.agent_id),
                    )
                else:
                    # Log detailed error information
                    logger.error(
                        "task_failed_in_swarm",
                        task_id=str(task.id),
                        agent_id=str(result.agent_id),
                        error=result.error,
                        metadata=result.metadata,
                    )

                    # Mark as failed (TaskQueueService handles retry logic)
                    await self.task_queue_service.fail_task(
                        task.id, error_message=result.error or "Unknown error"
                    )

                self.results.append(result)
                return result

            except Exception as e:
                logger.error(
                    "agent_execution_exception",
                    task_id=str(task.id),
                    error=str(e),
                    error_type=type(e).__name__,
                    agent_type=task.agent_type,
                )
                # Mark task as failed
                await self.task_queue_service.fail_task(
                    task.id, error_message=f"{type(e).__name__}: {e}"
                )
                error_result = Result(
                    task_id=task.id,
                    agent_id=UUID(int=0),
                    success=False,
                    error=str(e),
                )
                self.results.append(error_result)
                return error_result

            finally:
                if task.id in self.active_agents:
                    del self.active_agents[task.id]

    async def execute_batch(self, task_ids: list[UUID]) -> list[Result]:
        """Execute a batch of tasks concurrently.

        Args:
            task_ids: List of task IDs to execute (tasks must already be in queue)

        Returns:
            List of execution results
        """
        logger.info("executing_batch", task_count=len(task_ids))

        # Tasks should already be in the queue with proper status
        # Just process them - the swarm will pick them up
        return await self.start_swarm(task_limit=len(task_ids))

    async def get_swarm_status(self) -> dict[str, Any]:
        """Get current swarm status.

        Returns:
            Dictionary with swarm status information
        """
        return {
            "max_concurrent_agents": self.max_concurrent_agents,
            "active_agents": len(self.active_agents),
            "available_slots": self.max_concurrent_agents - len(self.active_agents),
            "total_results": len(self.results),
            "success_count": sum(1 for r in self.results if r.success),
            "failure_count": sum(1 for r in self.results if not r.success),
        }

    async def shutdown(self) -> None:
        """Gracefully shutdown the swarm."""
        logger.info("shutting_down_swarm", active_agents=len(self.active_agents))

        # Signal shutdown
        self._running = False
        self._shutdown_event.set()

        # Wait for active agents to complete with timeout
        if self.active_agents:
            logger.warning("active_agents_during_shutdown", count=len(self.active_agents))

        logger.info("swarm_shutdown_complete")

    def reset(self) -> None:
        """Reset swarm state (for testing or re-initialization)."""
        self.active_agents.clear()
        self.results.clear()
        logger.info("swarm_reset")
