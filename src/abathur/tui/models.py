"""TUI-specific data models for caching and filtering."""

from datetime import datetime
from typing import Generic, TypeVar
from uuid import UUID

from pydantic import BaseModel, Field

from abathur.domain.models import Task, TaskStatus

T = TypeVar("T")


class CachedData(BaseModel, Generic[T]):
    """Generic cache entry with TTL support.

    Attributes:
        data: The cached data of any type
        cached_at: Timestamp when data was cached
        ttl_seconds: Time-to-live in seconds
    """

    data: T
    cached_at: datetime = Field(default_factory=datetime.now)
    ttl_seconds: float = Field(default=2.0)

    def is_expired(self) -> bool:
        """Check if cache entry has exceeded TTL.

        Returns:
            True if cache has expired, False otherwise
        """
        elapsed = (datetime.now() - self.cached_at).total_seconds()
        return elapsed > self.ttl_seconds

    def time_remaining(self) -> float:
        """Calculate seconds remaining before expiration.

        Returns:
            Seconds remaining (0 if already expired)
        """
        elapsed = (datetime.now() - self.cached_at).total_seconds()
        return max(0.0, self.ttl_seconds - elapsed)


class FilterState(BaseModel):
    """Task filtering criteria for TUI.

    Attributes:
        statuses: Filter by task statuses (None = all statuses)
        agent_types: Filter by agent types (None = all agents)
        feature_branches: Filter by feature branches (None = all branches)
        text_search: Search text in summary or prompt (None = no text filter)
    """

    statuses: list[TaskStatus] | None = Field(
        default=None, description="Filter by task statuses"
    )
    agent_types: list[str] | None = Field(
        default=None, description="Filter by agent types"
    )
    feature_branches: list[str] | None = Field(
        default=None, description="Filter by feature branches"
    )
    text_search: str | None = Field(
        default=None, description="Search text in summary or prompt"
    )

    def matches(self, task: Task) -> bool:
        """Check if task matches all active filters.

        Args:
            task: Task to check against filters

        Returns:
            True if task matches all filters, False otherwise
        """
        # Status filter
        if self.statuses and task.status not in self.statuses:
            return False

        # Agent type filter
        if self.agent_types and task.agent_type not in self.agent_types:
            return False

        # Feature branch filter
        if self.feature_branches:
            if not task.feature_branch or task.feature_branch not in self.feature_branches:
                return False

        # Text search filter (case-insensitive, search in summary and prompt)
        if self.text_search:
            search_lower = self.text_search.lower()
            summary_match = (
                task.summary and search_lower in task.summary.lower()
            )
            prompt_match = search_lower in task.prompt.lower()
            if not (summary_match or prompt_match):
                return False

        return True


class QueueStatus(BaseModel):
    """Queue statistics for TUI header display.

    Attributes:
        total_tasks: Total number of tasks in queue
        pending: Number of pending tasks
        blocked: Number of blocked tasks
        ready: Number of ready tasks
        running: Number of running tasks
        completed: Number of completed tasks
        failed: Number of failed tasks
        cancelled: Number of cancelled tasks
        avg_priority: Average calculated priority
        max_depth: Maximum dependency depth
    """

    total_tasks: int
    pending: int
    blocked: int
    ready: int
    running: int
    completed: int
    failed: int
    cancelled: int
    avg_priority: float
    max_depth: int


class ExecutionPlan(BaseModel):
    """Execution plan with parallel batches.

    Attributes:
        task_ids: Original list of task IDs requested
        batches: List of batches, each batch contains tasks that can run in parallel
        total_batches: Total number of batches
        total_tasks: Total number of tasks in plan
    """

    task_ids: list[UUID]
    batches: list[list[UUID]]
    total_batches: int
    total_tasks: int


class FeatureBranchSummary(BaseModel):
    """Summary of tasks for a specific feature branch.

    Attributes:
        feature_branch: Feature branch name
        total_tasks: Total number of tasks
        status_breakdown: Task counts by status
        progress: Progress metrics (completed, failed, etc.)
        agent_breakdown: Task counts by agent type
        timestamps: Earliest task and latest activity timestamps
    """

    feature_branch: str
    total_tasks: int
    status_breakdown: dict[str, int]
    progress: dict[str, int | float]
    agent_breakdown: list[dict[str, int | str]]
    timestamps: dict[str, str | None]
