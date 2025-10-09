"""Swarm orchestrator for coordinating multiple concurrent agents."""

import asyncio
from typing import Any
from uuid import UUID

from abathur.application.agent_executor import AgentExecutor
from abathur.application.task_coordinator import TaskCoordinator
from abathur.domain.models import Result, Task, TaskStatus
from abathur.infrastructure.logger import get_logger

logger = get_logger(__name__)


class SwarmOrchestrator:
    """Orchestrates concurrent execution of multiple agents in a swarm."""

    def __init__(
        self,
        task_coordinator: TaskCoordinator,
        agent_executor: AgentExecutor,
        max_concurrent_agents: int = 10,
        agent_spawn_timeout: float = 5.0,
    ):
        """Initialize swarm orchestrator.

        Args:
            task_coordinator: Task coordinator for queue management
            agent_executor: Agent executor for running tasks
            max_concurrent_agents: Maximum number of concurrent agents
            agent_spawn_timeout: Timeout for agent spawning in seconds
        """
        self.task_coordinator = task_coordinator
        self.agent_executor = agent_executor
        self.max_concurrent_agents = max_concurrent_agents
        self.agent_spawn_timeout = agent_spawn_timeout
        self.semaphore = asyncio.Semaphore(max_concurrent_agents)
        self.active_agents: dict[UUID, Task] = {}
        self.results: list[Result] = []

    async def start_swarm(self, task_limit: int | None = None) -> list[Result]:
        """Start the swarm and process tasks from the queue.

        Args:
            task_limit: Maximum number of tasks to process (None for unlimited)

        Returns:
            List of all execution results
        """
        logger.info(
            "starting_swarm",
            max_concurrent=self.max_concurrent_agents,
            task_limit=task_limit,
        )

        tasks_processed = 0
        active_tasks: list[Any] = []

        try:
            while task_limit is None or tasks_processed < task_limit:
                # Get next task
                next_task = await self.task_coordinator.get_next_task()
                if not next_task:
                    # No more tasks, wait for active tasks to complete
                    if active_tasks:
                        logger.info("no_pending_tasks_waiting_for_active")
                        await asyncio.gather(*active_tasks, return_exceptions=True)
                    break

                # Spawn agent for task
                task_coroutine = asyncio.create_task(self._execute_with_semaphore(next_task))
                active_tasks.append(task_coroutine)
                tasks_processed += 1

                logger.info(
                    "task_spawned",
                    task_id=str(next_task.id),
                    active_count=len(self.active_agents),
                )

            # Wait for all remaining tasks
            if active_tasks:
                logger.info("waiting_for_remaining_tasks", count=len(active_tasks))
                await asyncio.gather(*active_tasks, return_exceptions=True)

            logger.info(
                "swarm_complete",
                tasks_processed=tasks_processed,
                results_count=len(self.results),
            )

            return self.results

        except Exception as e:
            logger.error("swarm_error", error=str(e))
            # Cancel all active tasks
            for task in active_tasks:
                if not task.done():
                    task.cancel()
            raise

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
                    await self.task_coordinator.update_task_status(task.id, TaskStatus.COMPLETED)
                else:
                    # Check if we should retry
                    task_obj = await self.task_coordinator.get_task(task.id)
                    if task_obj and task_obj.retry_count < task_obj.max_retries:
                        logger.info(
                            "task_will_retry",
                            task_id=str(task.id),
                            retry_count=task_obj.retry_count,
                        )
                        await self.task_coordinator.retry_task(task.id)
                    else:
                        await self.task_coordinator.update_task_status(
                            task.id, TaskStatus.FAILED, error_message=result.error
                        )

                self.results.append(result)
                return result

            except Exception as e:
                logger.error("agent_execution_exception", task_id=str(task.id), error=str(e))
                await self.task_coordinator.update_task_status(
                    task.id, TaskStatus.FAILED, error_message=str(e)
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

    async def execute_batch(self, tasks: list[Task]) -> list[Result]:
        """Execute a batch of tasks concurrently.

        Args:
            tasks: List of tasks to execute

        Returns:
            List of execution results
        """
        logger.info("executing_batch", task_count=len(tasks))

        # Submit all tasks to queue
        for task in tasks:
            await self.task_coordinator.submit_task(task)

        # Process the batch
        return await self.start_swarm(task_limit=len(tasks))

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

        # Wait for active agents to complete with timeout
        if self.active_agents:
            logger.warning("active_agents_during_shutdown", count=len(self.active_agents))

        self.active_agents.clear()
        self.results.clear()

    def reset(self) -> None:
        """Reset swarm state (for testing or re-initialization)."""
        self.active_agents.clear()
        self.results.clear()
        logger.info("swarm_reset")
