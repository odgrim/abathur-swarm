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

        Automatically sets task status to READY if it has no dependencies,
        aligning with TaskQueueService behavior.

        Args:
            task: Task to submit

        Returns:
            Task ID

        Raises:
            RuntimeError: If submission fails
        """
        try:
            # Auto-transition PENDING → READY if no dependencies
            # This aligns with TaskQueueService.enqueue_task() behavior
            if task.status == TaskStatus.PENDING and not task.dependencies:
                task.status = TaskStatus.READY
                logger.debug(
                    "task_auto_transitioned_to_ready",
                    task_id=str(task.id),
                    reason="no_dependencies",
                )

            await self.database.insert_task(task)
            await self.database.log_audit(
                task_id=task.id,
                action_type="task_submitted",
                action_data={
                    "agent_type": task.agent_type,
                    "priority": task.priority,
                    "status": task.status.value,
                },
                result="success",
            )
            logger.info(
                "task_submitted",
                task_id=str(task.id),
                agent_type=task.agent_type,
                status=task.status.value,
            )
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

    async def get_task(self, task_id: UUID) -> Task | None:
        """Get task by ID.

        Args:
            task_id: Task ID

        Returns:
            Task if found, None otherwise
        """
        return await self.database.get_task(task_id)

    async def list_tasks(
        self,
        status: TaskStatus | None = None,
        exclude_status: TaskStatus | None = None,
        limit: int = 100,
    ) -> list[Task]:
        """List tasks with optional status filter.

        Args:
            status: Filter by status (if None, return all)
            exclude_status: Exclude tasks with this status (if None, no exclusion)
            limit: Maximum number of tasks to return

        Returns:
            List of tasks
        """
        return await self.database.list_tasks(
            status=status, exclude_status=exclude_status, limit=limit
        )

    async def handle_stale_tasks(self) -> list[UUID]:
        """Detect and handle stale running tasks that have exceeded their timeout.

        Tasks that have exceeded their timeout will be:
        - Marked as FAILED if they've exceeded max retries
        - Marked as PENDING for retry if retries are available

        Returns:
            List of task IDs that were handled
        """
        stale_tasks = await self.database.get_stale_running_tasks()
        handled_task_ids = []

        for task in stale_tasks:
            logger.warning(
                "stale_task_detected",
                task_id=str(task.id),
                started_at=task.started_at.isoformat() if task.started_at else None,
                last_updated_at=task.last_updated_at.isoformat(),
                timeout_seconds=task.max_execution_timeout_seconds,
            )

            # Increment retry count
            await self.database.increment_task_retry_count(task.id)

            # Check if max retries exceeded (task.retry_count is the old value, now it's +1)
            if task.retry_count + 1 >= task.max_retries:
                # Max retries exceeded, mark as failed
                await self.update_task_status(
                    task.id,
                    TaskStatus.FAILED,
                    error_message=f"Task exceeded timeout ({task.max_execution_timeout_seconds}s) and max retries",
                )
                logger.error(
                    "task_timeout_failed",
                    task_id=str(task.id),
                    retry_count=task.retry_count + 1,
                )
            else:
                # Reset to pending for retry
                await self.update_task_status(task.id, TaskStatus.PENDING, error_message=None)
                logger.info(
                    "task_timeout_retry",
                    task_id=str(task.id),
                    retry_count=task.retry_count + 1,
                )

            handled_task_ids.append(task.id)

        if handled_task_ids:
            logger.info(
                "stale_tasks_handled",
                count=len(handled_task_ids),
                task_ids=[str(tid) for tid in handled_task_ids],
            )

        return handled_task_ids
