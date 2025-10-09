"""Task coordinator for managing task queue and lifecycle."""

from uuid import UUID

from abathur.domain.models import Task, TaskStatus
from abathur.infrastructure.database import Database
from abathur.infrastructure.logger import get_logger

logger = get_logger(__name__)


class TaskCoordinator:
    """Coordinates task queue management and lifecycle."""

    def __init__(self, database: Database):
        """Initialize task coordinator.

        Args:
            database: Database instance for task persistence
        """
        self.database = database

    async def submit_task(self, task: Task) -> UUID:
        """Submit a new task to the queue.

        Args:
            task: Task to submit

        Returns:
            Task ID

        Raises:
            RuntimeError: If submission fails
        """
        try:
            await self.database.insert_task(task)
            await self.database.log_audit(
                task_id=task.id,
                action_type="task_submitted",
                action_data={
                    "template": task.template_name,
                    "priority": task.priority,
                },
                result="success",
            )
            logger.info("task_submitted", task_id=str(task.id), template=task.template_name)
            return task.id
        except Exception as e:
            logger.error("task_submit_failed", error=str(e))
            raise RuntimeError(f"Failed to submit task: {e}") from e

    async def get_next_task(self) -> Task | None:
        """Get the next highest priority pending task.

        Returns:
            Next task to execute, or None if queue is empty
        """
        task = await self.database.dequeue_next_task()
        if task:
            logger.info("task_dequeued", task_id=str(task.id), priority=task.priority)
        return task

    async def update_task_status(
        self, task_id: UUID, status: TaskStatus, error_message: str | None = None
    ) -> None:
        """Update task status.

        Args:
            task_id: Task ID
            status: New status
            error_message: Optional error message for failed tasks
        """
        await self.database.update_task_status(task_id, status, error_message)
        await self.database.log_audit(
            task_id=task_id,
            action_type="task_status_updated",
            action_data={"status": status.value, "error": error_message},
            result="success",
        )
        logger.info("task_status_updated", task_id=str(task_id), status=status.value)

    async def cancel_task(self, task_id: UUID) -> bool:
        """Cancel a pending task.

        Args:
            task_id: Task ID to cancel

        Returns:
            True if cancelled, False if task not found or already running/completed
        """
        task = await self.database.get_task(task_id)
        if not task:
            logger.warning("task_not_found", task_id=str(task_id))
            return False

        if task.status not in (TaskStatus.PENDING,):
            logger.warning(
                "task_cannot_cancel",
                task_id=str(task_id),
                status=task.status.value,
            )
            return False

        await self.update_task_status(task_id, TaskStatus.CANCELLED)
        return True

    async def get_task(self, task_id: UUID) -> Task | None:
        """Get task by ID.

        Args:
            task_id: Task ID

        Returns:
            Task if found, None otherwise
        """
        return await self.database.get_task(task_id)

    async def list_tasks(self, status: TaskStatus | None = None, limit: int = 100) -> list[Task]:
        """List tasks with optional status filter.

        Args:
            status: Filter by status (if None, return all)
            limit: Maximum number of tasks to return

        Returns:
            List of tasks
        """
        return await self.database.list_tasks(status=status, limit=limit)

    async def retry_task(self, task_id: UUID) -> bool:
        """Retry a failed task.

        Args:
            task_id: Task ID to retry

        Returns:
            True if task was reset to pending, False otherwise
        """
        task = await self.database.get_task(task_id)
        if not task:
            return False

        if task.status != TaskStatus.FAILED:
            logger.warning(
                "task_cannot_retry",
                task_id=str(task_id),
                status=task.status.value,
            )
            return False

        if task.retry_count >= task.max_retries:
            logger.warning(
                "task_max_retries_exceeded",
                task_id=str(task_id),
                retry_count=task.retry_count,
            )
            return False

        # Reset to pending for retry
        await self.update_task_status(task_id, TaskStatus.PENDING)
        logger.info("task_retry_scheduled", task_id=str(task_id))
        return True
