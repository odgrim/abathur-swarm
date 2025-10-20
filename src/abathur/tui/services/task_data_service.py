"""Async data service with intelligent caching for TUI.

This module provides the TaskDataService class that manages task data fetching,
caching with TTL expiration, and optional auto-refresh mechanisms.
"""

import asyncio
import logging
from typing import Any, Awaitable, Callable, TypeVar, cast
from uuid import UUID

T = TypeVar("T")

from abathur.domain.models import Task
from abathur.infrastructure.database import Database
from abathur.services.dependency_resolver import DependencyResolver
from abathur.services.task_queue_service import TaskQueueService
from abathur.tui.exceptions import TUIDataError
from abathur.tui.models import (
    CachedData,
    ExecutionPlan,
    FeatureBranchSummary,
    FilterState,
    QueueStatus,
)

logger = logging.getLogger(__name__)


class TaskDataService:
    """Async data service with intelligent caching for TUI.

    Provides cached access to task queue data with automatic TTL-based
    invalidation and optional auto-refresh capabilities.

    Attributes:
        db: Database instance for data access
        task_service: TaskQueueService for queue operations
        dependency_resolver: DependencyResolver for graph operations
        default_ttl: Default TTL for cache entries (seconds)
    """

    def __init__(
        self,
        db: Database,
        task_service: TaskQueueService,
        dependency_resolver: DependencyResolver,
        default_ttl: float = 2.0,
    ):
        """Initialize service with dependencies.

        Args:
            db: Database instance
            task_service: TaskQueueService instance
            dependency_resolver: DependencyResolver instance
            default_ttl: Default cache TTL in seconds (default: 2.0)
        """
        self.db = db
        self.task_service = task_service
        self.dependency_resolver = dependency_resolver
        self.default_ttl = default_ttl

        # Cache storage (in-memory dict)
        self._tasks_cache: CachedData[list[Task]] | None = None
        self._graph_cache: CachedData[dict[UUID, list[UUID]]] | None = None
        self._status_cache: CachedData[QueueStatus] | None = None

        # Auto-refresh state
        self._refresh_task: asyncio.Task[None] | None = None
        self._refresh_callback: Callable[[], None] | None = None
        self._refresh_interval: float = 2.0

    async def fetch_tasks(
        self, filters: FilterState | None = None, force_refresh: bool = False
    ) -> list[Task]:
        """Fetch all tasks with optional filtering (cached).

        Args:
            filters: Optional filter criteria
            force_refresh: Force refresh bypassing cache

        Returns:
            List of tasks matching filters

        Note:
            Filtering is applied AFTER caching. Cache stores all tasks,
            filtering is applied on retrieval.
        """
        try:
            # Force refresh invalidates cache
            if force_refresh:
                self._tasks_cache = None

            async def _fetch() -> list[Task]:
                return await self.task_service._db.list_tasks(limit=1000)

            all_tasks = await self._get_cached_or_fetch("_tasks_cache", _fetch)

            # Apply filters if provided
            if filters:
                return [t for t in all_tasks if filters.matches(t)]
            return all_tasks

        except Exception as e:
            # If we have ANY cache (even expired), return it
            if self._tasks_cache:
                logger.warning(
                    f"Failed to fetch tasks, serving stale cache: {e}",
                    exc_info=True,
                )
                if filters:
                    return [t for t in self._tasks_cache.data if filters.matches(t)]
                return self._tasks_cache.data

            # No cache available - raise TUIDataError
            raise TUIDataError(
                f"Failed to fetch tasks and no cache available: {e}"
            ) from e

    async def get_dependency_graph(
        self, force_refresh: bool = False
    ) -> dict[UUID, list[UUID]]:
        """Get task dependency graph (cached).

        Args:
            force_refresh: Force refresh bypassing cache

        Returns:
            Dictionary mapping task IDs to prerequisite task IDs
        """
        try:
            # Force refresh invalidates cache
            if force_refresh:
                self._graph_cache = None

            async def _fetch() -> dict[UUID, list[UUID]]:
                # Build dependency graph from resolver's internal method
                graph_dict = await self.dependency_resolver._build_dependency_graph()
                # Convert to list format for consistency
                return {k: list(v) for k, v in graph_dict.items()}

            return await self._get_cached_or_fetch("_graph_cache", _fetch)

        except Exception as e:
            # Serve stale cache on error
            if self._graph_cache:
                logger.warning(
                    f"Failed to fetch dependency graph, serving stale cache: {e}",
                    exc_info=True,
                )
                return self._graph_cache.data

            # No cache available
            raise TUIDataError(
                f"Failed to fetch dependency graph and no cache available: {e}"
            ) from e

    async def get_queue_status(self, force_refresh: bool = False) -> QueueStatus:
        """Get queue statistics (cached).

        Args:
            force_refresh: Force refresh bypassing cache

        Returns:
            QueueStatus object with task counts and metrics
        """
        try:
            # Force refresh invalidates cache
            if force_refresh:
                self._status_cache = None

            async def _fetch() -> QueueStatus:
                # Fetch from task_service
                status_dict = await self.task_service.get_queue_status()
                return QueueStatus(
                    total_tasks=status_dict["total_tasks"],
                    pending=status_dict["pending"],
                    blocked=status_dict["blocked"],
                    ready=status_dict["ready"],
                    running=status_dict["running"],
                    completed=status_dict["completed"],
                    failed=status_dict["failed"],
                    cancelled=status_dict["cancelled"],
                    avg_priority=status_dict["avg_priority"],
                    max_depth=status_dict["max_depth"],
                )

            return await self._get_cached_or_fetch("_status_cache", _fetch)

        except Exception as e:
            # Serve stale cache on error
            if self._status_cache:
                logger.warning(
                    f"Failed to fetch queue status, serving stale cache: {e}",
                    exc_info=True,
                )
                return self._status_cache.data

            # No cache available
            raise TUIDataError(
                f"Failed to fetch queue status and no cache available: {e}"
            ) from e

    async def get_execution_plan(self, task_ids: list[UUID]) -> ExecutionPlan:
        """Get execution plan for tasks (NOT cached - specific query).

        Args:
            task_ids: List of task IDs to plan

        Returns:
            ExecutionPlan with parallel batches

        Note:
            Not cached because it's a specific query parameter.
            Only cache data that's frequently accessed with same params.
        """
        try:
            # Get execution plan from task service
            batches = await self.task_service.get_task_execution_plan(task_ids)

            return ExecutionPlan(
                task_ids=task_ids,
                batches=batches,
                total_batches=len(batches),
                total_tasks=len(task_ids),
            )

        except Exception as e:
            raise TUIDataError(f"Failed to get execution plan: {e}") from e

    async def get_feature_branch_summary(
        self, branch: str
    ) -> FeatureBranchSummary:
        """Get feature branch summary (NOT cached - specific query).

        Args:
            branch: Feature branch name

        Returns:
            FeatureBranchSummary for the specified branch

        Note:
            Not cached because it's a specific query parameter.
            Only cache data that's frequently accessed with same params.
        """
        try:
            # Fetch from database
            summary_dict = await self.db.get_feature_branch_summary(branch)

            return FeatureBranchSummary(
                feature_branch=summary_dict["feature_branch"],
                total_tasks=summary_dict["total_tasks"],
                status_breakdown=summary_dict["status_breakdown"],
                progress=summary_dict["progress"],
                agent_breakdown=summary_dict["agent_breakdown"],
                timestamps=summary_dict["timestamps"],
            )

        except Exception as e:
            raise TUIDataError(
                f"Failed to get feature branch summary for '{branch}': {e}"
            ) from e

    def start_auto_refresh(
        self, callback: Callable[[], None], interval: float = 2.0
    ) -> None:
        """Start automatic cache refresh in background.

        Args:
            callback: Function to call after each refresh
            interval: Refresh interval in seconds (default: 2.0)
        """
        # Stop existing refresh task if any
        self.stop_auto_refresh()

        self._refresh_callback = callback
        self._refresh_interval = interval

        # Create background task
        self._refresh_task = asyncio.create_task(self._auto_refresh_loop())
        logger.info(f"Started auto-refresh with {interval}s interval")

    def stop_auto_refresh(self) -> None:
        """Stop automatic cache refresh."""
        if self._refresh_task:
            self._refresh_task.cancel()
            self._refresh_task = None
        self._refresh_callback = None
        logger.info("Stopped auto-refresh")

    def invalidate_cache(self, cache_name: str | None = None) -> None:
        """Invalidate specific cache or all caches.

        Args:
            cache_name: Name of cache to invalidate, or None for all
        """
        if cache_name:
            setattr(self, cache_name, None)
            logger.debug(f"Invalidated cache: {cache_name}")
        else:
            # Invalidate all caches
            self._tasks_cache = None
            self._graph_cache = None
            self._status_cache = None
            logger.debug("Invalidated all caches")

    async def refresh_all(self) -> None:
        """Force refresh all cached data."""
        self.invalidate_cache()
        # Pre-populate caches in parallel
        await asyncio.gather(
            self.fetch_tasks(force_refresh=True),
            self.get_dependency_graph(force_refresh=True),
            self.get_queue_status(force_refresh=True),
            return_exceptions=True,  # Don't fail if one refresh fails
        )
        logger.debug("Refreshed all caches")

    async def _get_cached_or_fetch(
        self,
        cache_attr: str,
        fetch_fn: Callable[[], Awaitable[T]],
        ttl: float | None = None,
    ) -> T:
        """Generic cache-or-fetch pattern with TTL.

        Args:
            cache_attr: Name of cache attribute (e.g., "_tasks_cache")
            fetch_fn: Async function to fetch fresh data
            ttl: TTL in seconds (uses default_ttl if None)

        Returns:
            Cached data if valid, otherwise fresh fetched data
        """
        ttl = ttl or self.default_ttl
        cache: CachedData[Any] | None = getattr(self, cache_attr)

        # Check cache validity
        if cache is not None and not cache.is_expired():
            logger.debug(f"Cache hit: {cache_attr}")
            return cast(T, cache.data)

        # Cache miss or expired - fetch fresh data
        logger.debug(f"Cache miss or expired: {cache_attr}")
        try:
            fresh_data = await fetch_fn()
            cached_entry: CachedData[Any] = CachedData(
                data=fresh_data, ttl_seconds=ttl
            )
            setattr(self, cache_attr, cached_entry)
            return fresh_data
        except Exception as e:
            # If fetch fails but we have stale cache, return it with warning
            if cache is not None:
                logger.warning(
                    f"Fetch failed for {cache_attr}, serving stale cache: {e}"
                )
                return cast(T, cache.data)
            raise  # No cache available, propagate error

    async def _auto_refresh_loop(self) -> None:
        """Background task for periodic cache refresh.

        Continuously refreshes cache at specified interval and
        invokes callback after each refresh.
        """
        while True:
            try:
                await asyncio.sleep(self._refresh_interval)
                await self.refresh_all()

                # Invoke callback if registered
                if self._refresh_callback:
                    self._refresh_callback()

            except asyncio.CancelledError:
                # Task was cancelled, exit cleanly
                logger.debug("Auto-refresh loop cancelled")
                break
            except Exception as e:
                # Log error but continue refreshing
                logger.error(f"Auto-refresh error: {e}", exc_info=True)
                # Consider exponential backoff here in production
