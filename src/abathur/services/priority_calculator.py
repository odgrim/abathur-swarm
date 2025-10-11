"""Priority calculation service for dynamic task priority scoring.

This module implements the Chapter 20 prioritization patterns with weighted
multi-factor scoring to dynamically calculate task priorities based on:
- Base priority (user-specified): 30% weight
- Dependency depth (deeper = higher priority): 25% weight
- Deadline urgency (closer deadline = higher priority): 25% weight
- Blocking impact (more blocked tasks = higher priority): 15% weight
- Task source (HUMAN > AGENT priorities): 5% weight

Performance targets:
- Single calculation: <5ms
- Batch calculation (100 tasks): <50ms
"""

import logging
import math
from datetime import datetime, timezone
from typing import TYPE_CHECKING
from uuid import UUID

from abathur.domain.models import Task, TaskSource, TaskStatus

if TYPE_CHECKING:
    from abathur.infrastructure.database import Database
    from abathur.services.dependency_resolver import DependencyResolver

logger = logging.getLogger(__name__)


class PriorityCalculator:
    """Calculates dynamic task priorities based on multiple factors.

    Implements Chapter 20 prioritization patterns with weighted multi-factor scoring:
    - Base priority (user-specified): 30% weight
    - Dependency depth (deeper = higher): 25% weight
    - Deadline urgency (closer = higher): 25% weight
    - Blocking impact (more blocked tasks = higher): 15% weight
    - Task source (HUMAN > AGENT): 5% weight

    Priority Formula:
        priority = (
            base_score * 0.30 +
            depth_score * 0.25 +
            urgency_score * 0.25 +
            blocking_score * 0.15 +
            source_score * 0.05
        )

    All factor scores are normalized to [0, 100] range.
    Final priority is clamped to [0, 100].

    Performance targets:
    - Single calculation: <5ms
    - Batch calculation (100 tasks): <50ms
    """

    def __init__(
        self,
        dependency_resolver: "DependencyResolver",
        base_weight: float = 0.30,
        depth_weight: float = 0.25,
        urgency_weight: float = 0.25,
        blocking_weight: float = 0.15,
        source_weight: float = 0.05,
    ):
        """Initialize priority calculator with tunable weights.

        Args:
            dependency_resolver: DependencyResolver instance for depth calculations
            base_weight: Weight for user-specified base priority (default: 0.30)
            depth_weight: Weight for dependency depth score (default: 0.25)
            urgency_weight: Weight for deadline urgency score (default: 0.25)
            blocking_weight: Weight for blocking tasks count (default: 0.15)
            source_weight: Weight for task source priority (default: 0.05)

        Raises:
            ValueError: If weights don't sum to 1.0 (within tolerance)
        """
        self._dependency_resolver = dependency_resolver
        self._base_weight = base_weight
        self._depth_weight = depth_weight
        self._urgency_weight = urgency_weight
        self._blocking_weight = blocking_weight
        self._source_weight = source_weight

        # Validate weights sum to 1.0 (within floating point tolerance)
        total_weight = base_weight + depth_weight + urgency_weight + blocking_weight + source_weight
        if not math.isclose(total_weight, 1.0, rel_tol=1e-6):
            raise ValueError(
                f"Weights must sum to 1.0, got {total_weight:.6f}. "
                f"Weights: base={base_weight}, depth={depth_weight}, "
                f"urgency={urgency_weight}, blocking={blocking_weight}, source={source_weight}"
            )

        logger.debug(
            f"PriorityCalculator initialized with weights: "
            f"base={base_weight}, depth={depth_weight}, urgency={urgency_weight}, "
            f"blocking={blocking_weight}, source={source_weight}"
        )

    async def calculate_priority(self, task: Task) -> float:
        """Calculate dynamic priority score (0.0-100.0) for a task.

        Priority Formula:
            priority = (
                base_score * base_weight +
                depth_score * depth_weight +
                urgency_score * urgency_weight +
                blocking_score * blocking_weight +
                source_score * source_weight
            )

        All factor scores are normalized to [0, 100] range.

        Args:
            task: Task to calculate priority for

        Returns:
            Priority score (0.0-100.0), clamped to valid range

        Performance:
            Target: <5ms per calculation
        """
        try:
            # 1. Base priority score (0-10 scale, normalize to 0-100)
            base_score = float(task.priority) * 10.0  # Scale 0-10 to 0-100

            # 2. Dependency depth score (0-100)
            depth_score = await self._calculate_depth_score(task)

            # 3. Urgency score based on deadline (0-100)
            urgency_score = self._calculate_urgency_score(
                task.deadline, task.estimated_duration_seconds
            )

            # 4. Blocking impact score (0-100)
            blocking_score = await self._calculate_blocking_score(task)

            # 5. Source priority score (0-100)
            source_score = self._calculate_source_score(task.source)

            # Weighted sum
            priority = (
                base_score * self._base_weight
                + depth_score * self._depth_weight
                + urgency_score * self._urgency_weight
                + blocking_score * self._blocking_weight
                + source_score * self._source_weight
            )

            # Clamp to [0, 100]
            clamped_priority = max(0.0, min(100.0, priority))

            logger.debug(
                f"Priority calculated for task {task.id}: {clamped_priority:.2f} "
                f"(base={base_score:.1f}, depth={depth_score:.1f}, "
                f"urgency={urgency_score:.1f}, blocking={blocking_score:.1f}, "
                f"source={source_score:.1f})"
            )

            return clamped_priority

        except Exception as e:
            logger.error(f"Error calculating priority for task {task.id}: {e}", exc_info=True)
            # Return neutral priority on error
            return 50.0

    async def recalculate_priorities(
        self, affected_task_ids: list[UUID], db: "Database"
    ) -> dict[UUID, float]:
        """Recalculate priorities for multiple tasks (batch operation).

        Used after state changes (task completion, new task submission) to
        update priorities for affected tasks.

        Only recalculates priorities for tasks in PENDING, BLOCKED, or READY status.
        Tasks in other statuses (RUNNING, COMPLETED, etc.) are skipped.

        Args:
            affected_task_ids: List of task IDs to recalculate
            db: Database instance for fetching tasks

        Returns:
            Mapping of task_id -> new_priority (only for recalculated tasks)

        Performance:
            Target: <50ms for 100 tasks
        """
        results: dict[UUID, float] = {}

        for task_id in affected_task_ids:
            try:
                task = await db.get_task(task_id)
                if task is None:
                    logger.warning(f"Task {task_id} not found during priority recalculation")
                    continue

                # Only recalculate for tasks in active states
                if task.status in [TaskStatus.PENDING, TaskStatus.BLOCKED, TaskStatus.READY]:
                    new_priority = await self.calculate_priority(task)
                    results[task_id] = new_priority
                else:
                    logger.debug(
                        f"Skipping priority recalculation for task {task_id} "
                        f"(status: {task.status})"
                    )

            except Exception as e:
                logger.error(f"Error recalculating priority for task {task_id}: {e}", exc_info=True)
                # Continue with other tasks on error
                continue

        logger.info(f"Recalculated priorities for {len(results)}/{len(affected_task_ids)} tasks")
        return results

    async def _calculate_depth_score(self, task: Task) -> float:
        """Calculate priority score based on dependency depth.

        Deeper tasks (more prerequisites completed) get higher priority.
        This encourages completion of task chains.

        Scoring (linear scaling):
        - Depth 0 (root): 0 points
        - Depth 1: 10 points
        - Depth 2: 20 points
        - ...
        - Depth 10+: 100 points (capped)

        Formula: min(depth * 10, 100)

        Args:
            task: Task to score

        Returns:
            Depth score (0-100)
        """
        try:
            depth = await self._dependency_resolver.calculate_dependency_depth(task.id)
            score = min(100.0, float(depth) * 10.0)
            return score
        except Exception as e:
            logger.warning(f"Error calculating depth score for task {task.id}: {e}")
            return 0.0  # Root task default on error

    def _calculate_urgency_score(
        self, deadline: datetime | None, estimated_duration: int | None
    ) -> float:
        """Calculate urgency score based on deadline proximity.

        Tasks with approaching deadlines get higher urgency scores.
        Considers estimated duration to detect "too late" scenarios.

        Scoring:
        - No deadline: 50 points (neutral)
        - Past deadline: 100 points (urgent)
        - Insufficient time: 100 points (time_remaining < estimated_duration)
        - Exponential decay: 100 * exp(-time_remaining / (estimated_duration * 2))

        For tasks without estimated_duration, uses simpler thresholds:
        - > 1 week: 10 points
        - 1 week: 30 points
        - 1 day: 50 points
        - 1 hour: 80 points
        - < 1 minute: 100 points

        Args:
            deadline: Task deadline (None if no deadline)
            estimated_duration: Estimated execution time in seconds (None if unknown)

        Returns:
            Urgency score (0-100)
        """
        if deadline is None:
            return 50.0  # Neutral priority for tasks without deadlines

        now = datetime.now(timezone.utc)
        time_remaining = (deadline - now).total_seconds()

        # Past deadline or negative time
        if time_remaining <= 0:
            return 100.0

        # Check if not enough time to complete
        if estimated_duration is not None and time_remaining < estimated_duration:
            return 100.0  # Urgent - may miss deadline

        # Exponential urgency scaling with estimated duration
        if estimated_duration is not None:
            # Exponential decay: score = 100 * exp(-t / (2 * duration))
            # This gives high urgency when time_remaining approaches estimated_duration
            decay_factor = time_remaining / (estimated_duration * 2.0)
            score = 100.0 * math.exp(-decay_factor)
            return min(100.0, score)

        # Simple threshold-based scoring without estimated duration
        if time_remaining < 60:  # < 1 minute
            return 100.0
        elif time_remaining < 3600:  # < 1 hour
            return 80.0
        elif time_remaining < 86400:  # < 1 day
            return 50.0
        elif time_remaining < 604800:  # < 1 week
            return 30.0
        else:  # > 1 week
            return 10.0

    async def _calculate_blocking_score(self, task: Task) -> float:
        """Calculate score based on number of tasks blocked by this one.

        Tasks that are blocking other tasks get priority boost to unblock
        the dependency chain.

        Scoring (logarithmic scaling):
        - 0 blocked tasks: 0 points
        - 1-2 blocked: 20 points
        - 3-5 blocked: 40 points
        - 6-10 blocked: 60 points
        - 11-20 blocked: 80 points
        - 20+ blocked: 100 points

        Formula: min(log10(blocked_count + 1) * 33.33, 100)

        Uses logarithmic scaling to prevent extreme priority inflation.

        Args:
            task: Task to score

        Returns:
            Blocking score (0-100)
        """
        try:
            # Get tasks blocked by this one
            blocked_task_ids = await self._dependency_resolver.get_blocked_tasks(task.id)
            num_blocked = len(blocked_task_ids)

            if num_blocked == 0:
                return 0.0

            # Logarithmic scaling: log10(n+1) * 33.33
            # This gives: 1->10, 10->33, 100->67, 1000->100
            score = math.log10(num_blocked + 1) * 33.33
            return min(100.0, score)

        except Exception as e:
            logger.warning(f"Error calculating blocking score for task {task.id}: {e}")
            return 0.0  # No blocking boost on error

    def _calculate_source_score(self, source: TaskSource) -> float:
        """Calculate priority score based on task source.

        Human-submitted tasks get higher priority than agent-generated subtasks.
        This ensures user requests are prioritized over internal agent work.

        Scoring:
        - HUMAN: 100 points
        - AGENT_REQUIREMENTS: 75 points
        - AGENT_PLANNER: 50 points
        - AGENT_IMPLEMENTATION: 25 points

        Args:
            source: Task source enum

        Returns:
            Source priority score (0-100)
        """
        if source == TaskSource.HUMAN:
            return 100.0
        elif source == TaskSource.AGENT_REQUIREMENTS:
            return 75.0
        elif source == TaskSource.AGENT_PLANNER:
            return 50.0
        elif source == TaskSource.AGENT_IMPLEMENTATION:
            return 25.0
        else:
            logger.warning(f"Unknown task source: {source}, defaulting to 0")
            return 0.0
