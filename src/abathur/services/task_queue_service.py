"""Task queue service for managing task lifecycle with dependency resolution and priority scheduling.

This module implements the core task queue orchestration by integrating:
- Schema and domain models (Phase 1)
- Dependency resolution (Phase 2)
- Priority calculation (Phase 3)

Features:
- Task enqueue with dependency validation and priority calculation
- Priority-based task dequeuing with dependency awareness
- Automatic dependency resolution and task unblocking
- Failure propagation and cascade cancellation
- Queue statistics and execution planning

Performance targets:
- Task enqueue: <10ms (including validation + priority calculation)
- Get next task: <5ms (single indexed query)
- Complete task: <50ms (including cascade for 10 dependents)
- Queue status: <20ms (aggregate queries)
- Execution plan: <30ms (100-task graph)
"""

import logging
from datetime import datetime, timezone
from typing import Any
from uuid import UUID, uuid4

from abathur.domain.models import (
    DependencyType,
    Task,
    TaskDependency,
    TaskSource,
    TaskStatus,
)
from abathur.infrastructure.database import Database
from abathur.services.dependency_resolver import (
    CircularDependencyError,
    DependencyResolver,
)
from abathur.services.priority_calculator import PriorityCalculator

logger = logging.getLogger(__name__)


class TaskQueueError(Exception):
    """Base exception for task queue errors."""

    pass


class TaskNotFoundError(TaskQueueError):
    """Raised when task ID doesn't exist."""

    pass


class InvalidTransitionError(TaskQueueError):
    """Raised when invalid status transition attempted."""

    pass


class TaskQueueService:
    """Task queue orchestration service integrating dependency resolution and priority calculation.

    This service coordinates all Phase 1-3 components to provide a complete task queue
    with dependency management, priority-based scheduling, and state management.

    Usage:
        db = Database(Path("abathur.db"))
        await db.initialize()

        dependency_resolver = DependencyResolver(db)
        priority_calculator = PriorityCalculator(dependency_resolver)
        service = TaskQueueService(db, dependency_resolver, priority_calculator)

        # Enqueue tasks
        task_a = await service.enqueue_task("Task A", TaskSource.HUMAN)
        task_b = await service.enqueue_task("Task B", TaskSource.HUMAN, prerequisites=[task_a.id])

        # Dequeue and execute
        next_task = await service.get_next_task()
        await service.complete_task(next_task.id)

    State Transitions:
        PENDING → READY (all dependencies met)
        BLOCKED → READY (last dependency completed)
        READY → RUNNING (dequeued by get_next_task)
        RUNNING → COMPLETED (successful execution)
        RUNNING → FAILED (error during execution)
        RUNNING → CANCELLED (user cancellation)
        BLOCKED → CANCELLED (prerequisite failed/cancelled)
        COMPLETED/FAILED/CANCELLED → Terminal (no further transitions)
    """

    def __init__(
        self,
        database: Database,
        dependency_resolver: DependencyResolver,
        priority_calculator: PriorityCalculator,
    ):
        """Initialize task queue service.

        Args:
            database: Database instance for task storage
            dependency_resolver: DependencyResolver for dependency graph operations
            priority_calculator: PriorityCalculator for priority scoring
        """
        self._db = database
        self._dependency_resolver = dependency_resolver
        self._priority_calculator = priority_calculator

        logger.info("TaskQueueService initialized")

    async def enqueue_task(
        self,
        description: str,
        source: TaskSource,
        summary: str | None = None,
        parent_task_id: UUID | None = None,
        prerequisites: list[UUID] | None = None,
        base_priority: int = 5,
        deadline: datetime | None = None,
        estimated_duration_seconds: int | None = None,
        agent_type: str = "requirements-gatherer",
        session_id: str | None = None,
        input_data: dict[str, Any] | None = None,
        feature_branch: str | None = None,
        task_branch: str | None = None,
    ) -> Task:
        """Enqueue a new task with dependency validation and priority calculation.

        Steps:
        1. Validate prerequisites exist in database
        2. Check for circular dependencies (DependencyResolver.validate_new_dependency)
        3. Calculate dependency depth (DependencyResolver.calculate_dependency_depth)
        4. Determine initial status (READY if no prerequisites or all completed, BLOCKED otherwise)
        5. Calculate initial priority (PriorityCalculator.calculate_priority)
        6. Insert task into database (atomic transaction)
        7. Insert task dependencies (same transaction)
        8. Return created task

        Args:
            description: Task description/instruction
            source: Task source (HUMAN or AGENT_*)
            summary: Brief human-readable task summary, max 200 chars (optional)
            parent_task_id: Parent task ID (for hierarchical tasks)
            prerequisites: List of prerequisite task IDs
            base_priority: User-specified priority (0-10, default 5)
            deadline: Task deadline (optional)
            estimated_duration_seconds: Estimated execution time in seconds (optional)
            agent_type: Agent type to execute task (default "requirements-gatherer")
            session_id: Session ID for memory context (optional)
            input_data: Additional input data (optional)
            feature_branch: Feature branch that task changes get merged into (optional)
            task_branch: Individual task branch for isolated work, merges into feature_branch (optional)

        Returns:
            Created task with calculated priority and initial status

        Raises:
            ValueError: If prerequisites don't exist or circular dependency detected
            TaskQueueError: If database transaction fails
        """
        try:
            prerequisites = prerequisites or []
            input_data = input_data or {}

            # Generate summary if not provided
            if summary is None:
                summary = description[:100].strip()
                if not summary:
                    summary = "Task"

            # Validate base_priority range
            if not 0 <= base_priority <= 10:
                raise ValueError(f"base_priority must be in range [0, 10], got {base_priority}")

            # Step 1: Validate prerequisites exist
            if prerequisites:
                await self._validate_prerequisites_exist(prerequisites)

            # Step 2 & 3: Check circular dependencies and calculate depth
            task_id = uuid4()

            if prerequisites:
                for prereq_id in prerequisites:
                    try:
                        # Validate no circular dependency
                        await self._dependency_resolver.detect_circular_dependencies(
                            [prereq_id], task_id
                        )
                    except CircularDependencyError as e:
                        logger.error(f"Circular dependency detected for task {task_id}: {e}")
                        raise ValueError(f"Cannot add dependency - creates cycle: {e}") from e

            # Calculate dependency depth (after task inserted with dependencies)
            # We'll calculate it after creating task dependencies

            # Step 4: Determine initial status
            if not prerequisites:
                initial_status = TaskStatus.READY
            else:
                unmet = await self._get_unmet_prerequisites(prerequisites)
                initial_status = TaskStatus.BLOCKED if unmet else TaskStatus.READY

            # Step 5: Create task object
            task = Task(
                id=task_id,
                prompt=description,
                summary=summary,
                agent_type=agent_type,
                priority=base_priority,
                status=initial_status,
                source=source,
                parent_task_id=parent_task_id,
                session_id=session_id,
                input_data=input_data,
                deadline=deadline,
                estimated_duration_seconds=estimated_duration_seconds,
                calculated_priority=0.0,  # Will be calculated after depth known
                dependency_depth=0,  # Will be calculated after dependencies inserted
                feature_branch=feature_branch,
                task_branch=task_branch,
                submitted_at=datetime.now(timezone.utc),
                last_updated_at=datetime.now(timezone.utc),
            )

            # Step 6 & 7: Insert task and dependencies in transaction
            async with self._db._get_connection() as conn:
                # Insert task
                await conn.execute(
                    """
                    INSERT INTO tasks (
                        id, prompt, agent_type, priority, status, input_data,
                        result_data, error_message, retry_count, max_retries,
                        max_execution_timeout_seconds,
                        submitted_at, started_at, completed_at, last_updated_at,
                        created_by, parent_task_id, dependencies, session_id,
                        source, dependency_type, calculated_priority, deadline,
                        estimated_duration_seconds, dependency_depth, feature_branch, task_branch, summary
                    ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                    """,
                    (
                        str(task.id),
                        task.prompt,
                        task.agent_type,
                        task.priority,
                        task.status.value,
                        "{}",  # input_data as JSON
                        None,  # result_data
                        None,  # error_message
                        0,  # retry_count
                        3,  # max_retries
                        3600,  # max_execution_timeout_seconds
                        task.submitted_at.isoformat(),
                        None,  # started_at
                        None,  # completed_at
                        task.last_updated_at.isoformat(),
                        None,  # created_by
                        str(parent_task_id) if parent_task_id else None,
                        "[]",  # dependencies as JSON array
                        session_id,
                        task.source.value,
                        DependencyType.SEQUENTIAL.value,
                        0.0,  # calculated_priority (will update after depth)
                        task.deadline.isoformat() if task.deadline else None,
                        task.estimated_duration_seconds,
                        0,  # dependency_depth (will update after calculation)
                        feature_branch,
                        task_branch,
                        summary,
                    ),
                )

                # Insert task dependencies
                for prereq_id in prerequisites:
                    dependency = TaskDependency(
                        id=uuid4(),
                        dependent_task_id=task.id,
                        prerequisite_task_id=prereq_id,
                        dependency_type=DependencyType.SEQUENTIAL,
                        created_at=datetime.now(timezone.utc),
                    )

                    await conn.execute(
                        """
                        INSERT INTO task_dependencies (
                            id, dependent_task_id, prerequisite_task_id,
                            dependency_type, created_at, resolved_at
                        ) VALUES (?, ?, ?, ?, ?, ?)
                        """,
                        (
                            str(dependency.id),
                            str(dependency.dependent_task_id),
                            str(dependency.prerequisite_task_id),
                            dependency.dependency_type.value,
                            dependency.created_at.isoformat(),
                            None,  # resolved_at
                        ),
                    )

                await conn.commit()

            # Invalidate dependency resolver cache after adding dependencies
            self._dependency_resolver.invalidate_cache()

            # Calculate dependency depth
            depth = await self._dependency_resolver.calculate_dependency_depth(task.id)
            task.dependency_depth = depth

            # Calculate initial priority
            priority = await self._priority_calculator.calculate_priority(task)
            task.calculated_priority = priority

            # Update task with calculated depth and priority
            await self._update_task_priority_and_depth(task.id, priority, depth)

            logger.info(
                f"Enqueued task {task.id}: status={task.status}, "
                f"priority={priority:.2f}, depth={depth}, prerequisites={len(prerequisites)}"
            )

            return task

        except CircularDependencyError as e:
            logger.error(f"Failed to enqueue task: circular dependency - {e}")
            raise ValueError(f"Cannot enqueue task: {e}") from e
        except Exception as e:
            logger.error(f"Failed to enqueue task: {e}", exc_info=True)
            raise TaskQueueError(f"Failed to enqueue task: {e}") from e

    async def get_next_task(self) -> Task | None:
        """Return highest priority READY task and mark it as RUNNING.

        Query:
            SELECT * FROM tasks
            WHERE status = 'ready'
            ORDER BY calculated_priority DESC, submitted_at ASC
            LIMIT 1

        Uses idx_tasks_ready_priority index for optimal performance.

        Steps:
        1. Query for highest priority READY task
        2. If found, update status to RUNNING (atomic)
        3. Update started_at timestamp
        4. Return task
        5. If not found, return None

        Returns:
            Next task to execute, or None if queue empty

        Performance:
            Target: <5ms query time using composite index
        """
        try:
            async with self._db._get_connection() as conn:
                cursor = await conn.execute(
                    """
                    SELECT * FROM tasks
                    WHERE status = ?
                    ORDER BY calculated_priority DESC, submitted_at ASC
                    LIMIT 1
                    """,
                    (TaskStatus.READY.value,),
                )
                row = await cursor.fetchone()

                if not row:
                    logger.debug("No ready tasks in queue")
                    return None

                task = self._db._row_to_task(row)

                # Update status to RUNNING atomically
                now = datetime.now(timezone.utc)
                await conn.execute(
                    "UPDATE tasks SET status = ?, started_at = ?, last_updated_at = ? WHERE id = ?",
                    (TaskStatus.RUNNING.value, now.isoformat(), now.isoformat(), str(task.id)),
                )
                await conn.commit()

                task.status = TaskStatus.RUNNING
                task.started_at = now
                task.last_updated_at = now

                logger.info(
                    f"Dequeued task {task.id}: priority={task.calculated_priority:.2f}, "
                    f"source={task.source}"
                )

                return task

        except Exception as e:
            logger.error(f"Failed to get next task: {e}", exc_info=True)
            raise TaskQueueError(f"Failed to get next task: {e}") from e

    async def complete_task(self, task_id: UUID) -> list[UUID]:
        """Mark task as COMPLETED and unblock dependent tasks.

        Steps:
        1. Update task status to COMPLETED
        2. Update completed_at timestamp
        3. Resolve dependencies in task_dependencies table (set resolved_at)
        4. Get all tasks that were blocked by this one
        5. For each blocked task:
           a. Check if ALL its prerequisites are now met
           b. If yes, update status from BLOCKED → READY
           c. Recalculate priority (may have changed with new depth)
           d. Update calculated_priority in database
        6. Return list of newly-unblocked task IDs

        Args:
            task_id: Task ID to mark as completed

        Returns:
            List of task IDs that were unblocked (BLOCKED → READY)

        Raises:
            TaskNotFoundError: If task doesn't exist
            TaskQueueError: If database transaction fails
        """
        try:
            async with self._db._get_connection() as conn:
                # Step 1 & 2: Update task status to COMPLETED
                now = datetime.now(timezone.utc)
                cursor = await conn.execute(
                    """
                    UPDATE tasks
                    SET status = ?, completed_at = ?, last_updated_at = ?
                    WHERE id = ?
                    """,
                    (TaskStatus.COMPLETED.value, now.isoformat(), now.isoformat(), str(task_id)),
                )

                if cursor.rowcount == 0:
                    raise TaskNotFoundError(f"Task {task_id} not found")

                # Step 3: Mark dependencies as resolved
                await conn.execute(
                    """
                    UPDATE task_dependencies
                    SET resolved_at = ?
                    WHERE prerequisite_task_id = ? AND resolved_at IS NULL
                    """,
                    (now.isoformat(), str(task_id)),
                )

                # Step 4: Get all tasks that depend on this one
                cursor = await conn.execute(
                    """
                    SELECT DISTINCT dependent_task_id FROM task_dependencies
                    WHERE prerequisite_task_id = ?
                    """,
                    (str(task_id),),
                )
                dependent_rows = await cursor.fetchall()
                dependent_ids = [UUID(row[0]) for row in dependent_rows]

                await conn.commit()

            # Invalidate dependency resolver cache after resolving dependencies
            self._dependency_resolver.invalidate_cache()

            # Step 5: Check each dependent task and unblock if ready
            unblocked_ids: list[UUID] = []

            for dependent_id in dependent_ids:
                # Check if all dependencies are now met
                all_met = await self._dependency_resolver.are_all_dependencies_met(dependent_id)

                if all_met:
                    # Update status to READY
                    async with self._db._get_connection() as conn:
                        await conn.execute(
                            """
                            UPDATE tasks
                            SET status = ?, last_updated_at = ?
                            WHERE id = ? AND status = ?
                            """,
                            (
                                TaskStatus.READY.value,
                                datetime.now(timezone.utc).isoformat(),
                                str(dependent_id),
                                TaskStatus.BLOCKED.value,
                            ),
                        )
                        await conn.commit()

                    # Recalculate priority
                    await self._update_task_priority(dependent_id)

                    unblocked_ids.append(dependent_id)
                    logger.info(f"Unblocked task {dependent_id} (all dependencies met)")

            logger.info(f"Completed task {task_id}: unblocked {len(unblocked_ids)} dependent tasks")

            return unblocked_ids

        except TaskNotFoundError:
            raise
        except Exception as e:
            logger.error(f"Failed to complete task {task_id}: {e}", exc_info=True)
            raise TaskQueueError(f"Failed to complete task: {e}") from e

    async def fail_task(self, task_id: UUID, error_message: str) -> list[UUID]:
        """Mark task as FAILED and cascade cancellation to dependent tasks.

        Steps:
        1. Update task status to FAILED
        2. Set error_message field
        3. Get all tasks that depend on this one (recursively)
        4. Update all dependent tasks to CANCELLED status
        5. Return list of cancelled task IDs

        Rationale:
        If a task fails, tasks depending on it cannot proceed.
        Cascading cancellation prevents orphaned blocked tasks.

        Args:
            task_id: Task ID to mark as failed
            error_message: Error message describing failure

        Returns:
            List of task IDs that were cancelled due to failure

        Raises:
            TaskNotFoundError: If task doesn't exist
            TaskQueueError: If database transaction fails
        """
        try:
            async with self._db._get_connection() as conn:
                # Step 1 & 2: Update task status to FAILED
                now = datetime.now(timezone.utc)
                cursor = await conn.execute(
                    """
                    UPDATE tasks
                    SET status = ?, error_message = ?, completed_at = ?, last_updated_at = ?
                    WHERE id = ?
                    """,
                    (
                        TaskStatus.FAILED.value,
                        error_message,
                        now.isoformat(),
                        now.isoformat(),
                        str(task_id),
                    ),
                )

                if cursor.rowcount == 0:
                    raise TaskNotFoundError(f"Task {task_id} not found")

                await conn.commit()

            # Step 3: Get all dependent tasks recursively
            dependent_ids = await self._get_dependent_tasks_recursive(task_id)

            # Step 4: Update all dependent tasks to CANCELLED
            if dependent_ids:
                async with self._db._get_connection() as conn:
                    placeholders = ",".join(["?" for _ in dependent_ids])
                    await conn.execute(
                        f"""
                        UPDATE tasks
                        SET status = ?, last_updated_at = ?
                        WHERE id IN ({placeholders})
                        """,
                        [TaskStatus.CANCELLED.value, datetime.now(timezone.utc).isoformat()]
                        + [str(dep_id) for dep_id in dependent_ids],
                    )
                    await conn.commit()

            logger.info(
                f"Failed task {task_id}: cancelled {len(dependent_ids)} dependent tasks. "
                f"Error: {error_message}"
            )

            return dependent_ids

        except TaskNotFoundError:
            raise
        except Exception as e:
            logger.error(f"Failed to fail task {task_id}: {e}", exc_info=True)
            raise TaskQueueError(f"Failed to fail task: {e}") from e

    async def cancel_task(self, task_id: UUID) -> list[UUID]:
        """Cancel task and cascade cancellation to dependents.

        Similar to fail_task but without error message.
        Used for user-initiated cancellations.

        Steps:
        1. Update task status to CANCELLED
        2. Get all dependent tasks (recursively)
        3. Update dependents to CANCELLED
        4. Return list of cancelled task IDs

        Args:
            task_id: Task ID to cancel

        Returns:
            List of task IDs that were cancelled (including this task)

        Raises:
            TaskNotFoundError: If task doesn't exist
            TaskQueueError: If database transaction fails
        """
        try:
            async with self._db._get_connection() as conn:
                # Step 1: Update task status to CANCELLED
                now = datetime.now(timezone.utc)
                cursor = await conn.execute(
                    """
                    UPDATE tasks
                    SET status = ?, last_updated_at = ?
                    WHERE id = ?
                    """,
                    (TaskStatus.CANCELLED.value, now.isoformat(), str(task_id)),
                )

                if cursor.rowcount == 0:
                    raise TaskNotFoundError(f"Task {task_id} not found")

                await conn.commit()

            # Step 2: Get all dependent tasks recursively
            dependent_ids = await self._get_dependent_tasks_recursive(task_id)

            # Step 3: Update all dependent tasks to CANCELLED
            if dependent_ids:
                async with self._db._get_connection() as conn:
                    placeholders = ",".join(["?" for _ in dependent_ids])
                    await conn.execute(
                        f"""
                        UPDATE tasks
                        SET status = ?, last_updated_at = ?
                        WHERE id IN ({placeholders})
                        """,
                        [TaskStatus.CANCELLED.value, datetime.now(timezone.utc).isoformat()]
                        + [str(dep_id) for dep_id in dependent_ids],
                    )
                    await conn.commit()

            logger.info(f"Cancelled task {task_id}: cancelled {len(dependent_ids)} dependent tasks")

            # Return all cancelled IDs (including the original task)
            return [task_id] + dependent_ids

        except TaskNotFoundError:
            raise
        except Exception as e:
            logger.error(f"Failed to cancel task {task_id}: {e}", exc_info=True)
            raise TaskQueueError(f"Failed to cancel task: {e}") from e

    async def get_queue_status(self) -> dict[str, Any]:
        """Return queue statistics for monitoring.

        Returns:
        {
            "total_tasks": int,
            "pending": int,
            "blocked": int,
            "ready": int,
            "running": int,
            "completed": int,
            "failed": int,
            "cancelled": int,
            "avg_priority": float,
            "max_depth": int,
            "oldest_pending": datetime | None,
            "newest_task": datetime | None,
        }

        Query:
        SELECT status, COUNT(*) as count, AVG(calculated_priority) as avg_priority
        FROM tasks
        GROUP BY status

        Performance:
            Target: <20ms query time using aggregate queries
        """
        try:
            async with self._db._get_connection() as conn:
                # Get status counts and average priority
                cursor = await conn.execute(
                    """
                    SELECT
                        status,
                        COUNT(*) as count,
                        AVG(calculated_priority) as avg_priority
                    FROM tasks
                    GROUP BY status
                    """
                )
                rows = await cursor.fetchall()

                # Initialize counts
                status_counts = {
                    "pending": 0,
                    "blocked": 0,
                    "ready": 0,
                    "running": 0,
                    "completed": 0,
                    "failed": 0,
                    "cancelled": 0,
                }
                total_tasks = 0
                total_priority = 0.0
                count_for_avg = 0

                for row in rows:
                    status = row[0]
                    count = row[1]
                    avg_priority = row[2] or 0.0

                    status_counts[status] = count
                    total_tasks += count
                    total_priority += avg_priority * count
                    count_for_avg += count

                # Calculate overall average priority
                avg_priority = total_priority / count_for_avg if count_for_avg > 0 else 0.0

                # Get max depth
                cursor = await conn.execute("SELECT MAX(dependency_depth) FROM tasks")
                max_depth_row = await cursor.fetchone()
                max_depth = max_depth_row[0] if max_depth_row and max_depth_row[0] else 0

                # Get oldest pending task
                cursor = await conn.execute(
                    """
                    SELECT submitted_at FROM tasks
                    WHERE status IN (?, ?)
                    ORDER BY submitted_at ASC
                    LIMIT 1
                    """,
                    (TaskStatus.PENDING.value, TaskStatus.BLOCKED.value),
                )
                oldest_row = await cursor.fetchone()
                oldest_pending = (
                    datetime.fromisoformat(oldest_row[0]) if oldest_row and oldest_row[0] else None
                )

                # Get newest task
                cursor = await conn.execute(
                    """
                    SELECT submitted_at FROM tasks
                    ORDER BY submitted_at DESC
                    LIMIT 1
                    """
                )
                newest_row = await cursor.fetchone()
                newest_task = (
                    datetime.fromisoformat(newest_row[0]) if newest_row and newest_row[0] else None
                )

            result = {
                "total_tasks": total_tasks,
                **status_counts,
                "avg_priority": round(avg_priority, 2),
                "max_depth": max_depth,
                "oldest_pending": oldest_pending,
                "newest_task": newest_task,
            }

            logger.debug(f"Queue status: {result}")
            return result

        except Exception as e:
            logger.error(f"Failed to get queue status: {e}", exc_info=True)
            raise TaskQueueError(f"Failed to get queue status: {e}") from e

    async def get_task_execution_plan(self, task_ids: list[UUID]) -> list[list[UUID]]:
        """Return execution plan as list of batches (topological sort).

        Uses DependencyResolver.get_execution_order() to compute topological sort.
        Tasks in same batch can execute in parallel (no dependencies between them).

        Example:
        Input: [A, B, C, D] where A→B, A→C, B→D, C→D
        Output: [[A], [B, C], [D]]

        Batch 0: [A] - no dependencies, execute first
        Batch 1: [B, C] - depend on A, can execute in parallel
        Batch 2: [D] - depends on B and C, execute after both complete

        Args:
            task_ids: List of task IDs to sort

        Returns:
            List of batches, each batch is list of task IDs that can execute in parallel

        Raises:
            CircularDependencyError: If graph contains cycles
            TaskQueueError: If execution order calculation fails
        """
        try:
            if not task_ids:
                return []

            # Get topological sort from DependencyResolver
            ordered_ids = await self._dependency_resolver.get_execution_order(task_ids)

            # Group tasks by depth level for parallel execution
            depth_map: dict[int, list[UUID]] = {}

            for task_id in ordered_ids:
                depth = await self._dependency_resolver.calculate_dependency_depth(task_id)
                if depth not in depth_map:
                    depth_map[depth] = []
                depth_map[depth].append(task_id)

            # Convert to sorted list of batches
            max_depth = max(depth_map.keys()) if depth_map else 0
            batches = [depth_map.get(i, []) for i in range(max_depth + 1)]

            logger.debug(
                f"Execution plan for {len(task_ids)} tasks: {len(batches)} batches, "
                f"max_depth={max_depth}"
            )

            return batches

        except CircularDependencyError as e:
            logger.error(f"Circular dependency in execution plan: {e}")
            raise
        except Exception as e:
            logger.error(f"Failed to get execution plan: {e}", exc_info=True)
            raise TaskQueueError(f"Failed to get execution plan: {e}") from e

    # Helper methods

    async def _validate_prerequisites_exist(self, prerequisite_ids: list[UUID]) -> None:
        """Validate all prerequisite task IDs exist in database.

        Args:
            prerequisite_ids: List of prerequisite task IDs to validate

        Raises:
            ValueError: If any prerequisite doesn't exist
        """
        async with self._db._get_connection() as conn:
            placeholders = ",".join(["?" for _ in prerequisite_ids])
            cursor = await conn.execute(
                f"SELECT id FROM tasks WHERE id IN ({placeholders})",
                [str(pid) for pid in prerequisite_ids],
            )
            rows = await cursor.fetchall()
            found_ids = {UUID(row[0]) for row in rows}

            missing = set(prerequisite_ids) - found_ids
            if missing:
                raise ValueError(f"Prerequisites not found: {missing}")

    async def _get_unmet_prerequisites(self, prerequisite_ids: list[UUID]) -> list[UUID]:
        """Get list of prerequisite tasks that are not yet completed.

        Args:
            prerequisite_ids: List of prerequisite task IDs

        Returns:
            List of prerequisite task IDs that are not in COMPLETED status
        """
        async with self._db._get_connection() as conn:
            placeholders = ",".join(["?" for _ in prerequisite_ids])
            cursor = await conn.execute(
                f"""
                SELECT id FROM tasks
                WHERE id IN ({placeholders})
                AND status != ?
                """,
                [str(pid) for pid in prerequisite_ids] + [TaskStatus.COMPLETED.value],
            )
            rows = await cursor.fetchall()
            return [UUID(row[0]) for row in rows]

    async def _get_dependent_tasks_recursive(self, task_id: UUID) -> list[UUID]:
        """Get all tasks that transitively depend on this task.

        Uses iterative BFS to find all descendants in dependency graph.

        Args:
            task_id: Root task ID

        Returns:
            List of all dependent task IDs (transitive closure)
        """
        visited: set[UUID] = set()
        queue = [task_id]
        result: list[UUID] = []

        async with self._db._get_connection() as conn:
            while queue:
                current = queue.pop(0)

                if current in visited:
                    continue

                visited.add(current)

                # Get direct dependents
                cursor = await conn.execute(
                    """
                    SELECT dependent_task_id FROM task_dependencies
                    WHERE prerequisite_task_id = ?
                    """,
                    (str(current),),
                )
                rows = await cursor.fetchall()

                for row in rows:
                    dependent_id = UUID(row[0])
                    if dependent_id not in visited:
                        queue.append(dependent_id)
                        result.append(dependent_id)

        return result

    async def _update_task_priority(self, task_id: UUID) -> float:
        """Recalculate and update task priority.

        Args:
            task_id: Task ID to update

        Returns:
            New priority value
        """
        task = await self._db.get_task(task_id)
        if not task:
            logger.warning(f"Task {task_id} not found during priority update")
            return 0.0

        new_priority = await self._priority_calculator.calculate_priority(task)

        async with self._db._get_connection() as conn:
            await conn.execute(
                "UPDATE tasks SET calculated_priority = ? WHERE id = ?",
                (new_priority, str(task_id)),
            )
            await conn.commit()

        return new_priority

    async def _update_task_priority_and_depth(
        self, task_id: UUID, priority: float, depth: int
    ) -> None:
        """Update task calculated_priority and dependency_depth.

        Args:
            task_id: Task ID to update
            priority: New priority value
            depth: New depth value
        """
        async with self._db._get_connection() as conn:
            await conn.execute(
                "UPDATE tasks SET calculated_priority = ?, dependency_depth = ? WHERE id = ?",
                (priority, depth, str(task_id)),
            )
            await conn.commit()
