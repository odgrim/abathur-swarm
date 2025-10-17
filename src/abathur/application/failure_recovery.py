"""Failure recovery mechanisms for task and agent failures."""

import asyncio
from dataclasses import dataclass
from datetime import datetime, timedelta, timezone
from uuid import UUID

from abathur.application.task_coordinator import TaskCoordinator
from abathur.domain.models import Task, TaskStatus
from abathur.infrastructure.database import Database
from abathur.infrastructure.logger import get_logger

logger = get_logger(__name__)


@dataclass
class RetryPolicy:
    """Retry policy configuration."""

    max_retries: int = 3
    initial_backoff_seconds: float = 10.0
    max_backoff_seconds: float = 300.0  # 5 minutes
    backoff_multiplier: float = 2.0
    jitter: bool = True


@dataclass
class FailureStats:
    """Failure statistics."""

    total_failures: int = 0
    permanent_failures: int = 0
    transient_failures: int = 0
    retried_tasks: int = 0
    recovered_tasks: int = 0


class FailureRecovery:
    """Manages failure detection and recovery for tasks and agents."""

    def __init__(
        self,
        task_coordinator: TaskCoordinator,
        database: Database,
        retry_policy: RetryPolicy | None = None,
    ):
        """Initialize failure recovery.

        Args:
            task_coordinator: Task coordinator for task management
            database: Database for state persistence
            retry_policy: Retry policy configuration
        """
        self.task_coordinator = task_coordinator
        self.database = database
        self.retry_policy = retry_policy or RetryPolicy()
        self.stats = FailureStats()
        self._recovery_task: asyncio.Task | None = None

    async def start_recovery_monitor(self, check_interval: float = 60.0) -> None:
        """Start background task to monitor and recover failed tasks.

        Args:
            check_interval: Interval between recovery checks in seconds
        """
        if self._recovery_task is None or self._recovery_task.done():
            self._recovery_task = asyncio.create_task(self._recovery_loop(check_interval))
            logger.info("recovery_monitor_started", interval=check_interval)

    async def stop_recovery_monitor(self) -> None:
        """Stop background recovery monitoring."""
        if self._recovery_task and not self._recovery_task.done():
            self._recovery_task.cancel()
            try:
                await self._recovery_task
            except asyncio.CancelledError:
                pass
            logger.info("recovery_monitor_stopped")

    async def _recovery_loop(self, check_interval: float) -> None:
        """Background task to check for and recover failed tasks."""
        try:
            while True:
                await asyncio.sleep(check_interval)
                await self._check_failed_tasks()
                await self._check_stalled_tasks()

        except asyncio.CancelledError:
            logger.info("recovery_loop_cancelled")
        except Exception as e:
            logger.error("recovery_loop_error", error=str(e))

    async def _check_failed_tasks(self) -> None:
        """Check for failed or cancelled tasks that can be retried."""
        failed_tasks = await self.task_coordinator.list_tasks(status=TaskStatus.FAILED, limit=100)
        cancelled_tasks = await self.task_coordinator.list_tasks(
            status=TaskStatus.CANCELLED, limit=100
        )

        for task in failed_tasks + cancelled_tasks:
            if await self._should_retry(task):
                await self.retry_task(task.id)

    async def _check_stalled_tasks(self) -> None:
        """Check for tasks that have been running too long (stalled)."""
        running_tasks = await self.task_coordinator.list_tasks(status=TaskStatus.RUNNING, limit=100)

        now = datetime.now(timezone.utc)
        stall_threshold = timedelta(hours=1)  # Tasks stalled after 1 hour

        for task in running_tasks:
            if task.started_at:
                running_time = now - task.started_at
                if running_time > stall_threshold:
                    logger.warning(
                        "task_stalled",
                        task_id=str(task.id),
                        running_time_seconds=running_time.total_seconds(),
                    )
                    # Mark as failed and retry
                    await self.task_coordinator.update_task_status(
                        task.id,
                        TaskStatus.FAILED,
                        error_message="Task stalled - exceeded time limit",
                    )
                    self.stats.transient_failures += 1

    async def _should_retry(self, task: Task) -> bool:
        """Determine if a failed or cancelled task should be retried.

        Args:
            task: Task to check

        Returns:
            True if task should be retried
        """
        if task.retry_count >= task.max_retries:
            logger.info(
                "task_max_retries_reached",
                task_id=str(task.id),
                retry_count=task.retry_count,
            )
            self.stats.permanent_failures += 1
            return False

        # Calculate backoff time
        backoff = self._calculate_backoff(task.retry_count)

        # Check if enough time has passed since failure
        if task.completed_at:
            time_since_failure = (datetime.now(timezone.utc) - task.completed_at).total_seconds()
            if time_since_failure < backoff:
                return False

        return True

    def _calculate_backoff(self, retry_count: int) -> float:
        """Calculate exponential backoff time.

        Args:
            retry_count: Number of retries so far

        Returns:
            Backoff time in seconds
        """
        backoff = min(
            self.retry_policy.initial_backoff_seconds
            * (self.retry_policy.backoff_multiplier**retry_count),
            self.retry_policy.max_backoff_seconds,
        )

        # Add jitter
        if self.retry_policy.jitter:
            import random

            jitter = backoff * 0.2 * random.random()  # Up to 20% jitter
            backoff += jitter

        return backoff

    async def retry_task(self, task_id: UUID) -> bool:
        """Retry a failed task.

        Args:
            task_id: Task ID to retry

        Returns:
            True if task was queued for retry
        """
        success = await self.task_coordinator.retry_task(task_id)

        if success:
            self.stats.retried_tasks += 1
            logger.info("task_retried", task_id=str(task_id))

        return success

    async def handle_agent_failure(self, agent_id: UUID, task_id: UUID, error: str) -> None:
        """Handle an agent failure.

        Args:
            agent_id: ID of failed agent
            task_id: ID of task that failed
            error: Error message
        """
        logger.error(
            "agent_failure",
            agent_id=str(agent_id),
            task_id=str(task_id),
            error=error,
        )

        self.stats.total_failures += 1

        # Mark task as failed
        await self.task_coordinator.update_task_status(
            task_id, TaskStatus.FAILED, error_message=error
        )

        # Determine if error is transient or permanent
        if self._is_transient_error(error):
            self.stats.transient_failures += 1
            # Will be retried by recovery loop
        else:
            # Check if max retries exceeded
            task = await self.task_coordinator.get_task(task_id)
            if task and task.retry_count >= task.max_retries:
                self.stats.permanent_failures += 1

    def _is_transient_error(self, error: str) -> bool:
        """Determine if an error is transient (retriable).

        Args:
            error: Error message

        Returns:
            True if error is likely transient
        """
        transient_indicators = [
            "timeout",
            "rate limit",
            "connection",
            "network",
            "temporary",
            "service unavailable",
            "503",
            "429",
        ]

        error_lower = error.lower()
        return any(indicator in error_lower for indicator in transient_indicators)

    def get_stats(self) -> dict:
        """Get failure recovery statistics.

        Returns:
            Dictionary with statistics
        """
        return {
            "total_failures": self.stats.total_failures,
            "permanent_failures": self.stats.permanent_failures,
            "transient_failures": self.stats.transient_failures,
            "retried_tasks": self.stats.retried_tasks,
            "recovered_tasks": self.stats.recovered_tasks,
        }
