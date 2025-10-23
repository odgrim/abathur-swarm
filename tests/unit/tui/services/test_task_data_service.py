"""Unit tests for TaskDataService with comprehensive caching tests."""

import asyncio
from datetime import datetime, timedelta
from unittest.mock import AsyncMock, MagicMock, patch
from uuid import UUID, uuid4

import pytest

from abathur.domain.models import Task, TaskSource, TaskStatus
from abathur.tui.exceptions import TUIDataError
from abathur.tui.models import CachedData, FilterState, QueueStatus
from abathur.tui.services.task_data_service import TaskDataService


@pytest.fixture
def mock_db():
    """Create mock Database."""
    db = MagicMock()
    db.list_tasks = AsyncMock(return_value=[])
    db.get_feature_branch_summary = AsyncMock(return_value={
        "feature_branch": "test-branch",
        "total_tasks": 5,
        "status_breakdown": {"completed": 3, "pending": 2},
        "progress": {"completed": 3, "completion_rate": 60.0},
        "agent_breakdown": [],
        "timestamps": {"earliest_task": None, "latest_activity": None},
    })
    return db


@pytest.fixture
def mock_task_service():
    """Create mock TaskQueueService."""
    service = MagicMock()
    service._db = MagicMock()
    service._db.list_tasks = AsyncMock(return_value=[])
    service.get_queue_status = AsyncMock(return_value={
        "total_tasks": 10,
        "pending": 2,
        "blocked": 1,
        "ready": 3,
        "running": 1,
        "completed": 2,
        "failed": 1,
        "cancelled": 0,
        "avg_priority": 5.5,
        "max_depth": 3,
    })
    service.get_task_execution_plan = AsyncMock(return_value=[
        [UUID("00000000-0000-0000-0000-000000000001")],
        [UUID("00000000-0000-0000-0000-000000000002")],
    ])
    return service


@pytest.fixture
def mock_dependency_resolver():
    """Create mock DependencyResolver."""
    resolver = MagicMock()
    resolver._build_dependency_graph = AsyncMock(return_value={})
    return resolver


@pytest.fixture
def service(mock_db, mock_task_service, mock_dependency_resolver):
    """Create TaskDataService with mocked dependencies."""
    return TaskDataService(
        db=mock_db,
        task_service=mock_task_service,
        dependency_resolver=mock_dependency_resolver,
        default_ttl=2.0,
    )


@pytest.fixture
def sample_tasks():
    """Create sample tasks for testing."""
    return [
        Task(
            id=uuid4(),
            prompt="Task 1",
            summary="Task 1",
            status=TaskStatus.PENDING,
            agent_type="test-agent",
            source=TaskSource.HUMAN,
        ),
        Task(
            id=uuid4(),
            prompt="Task 2",
            summary="Task 2",
            status=TaskStatus.COMPLETED,
            agent_type="test-agent",
            source=TaskSource.HUMAN,
        ),
    ]


# Cache hit/miss tests


@pytest.mark.asyncio
async def test_cache_miss_fetches_fresh_data(service, mock_task_service, sample_tasks):
    """Test cache miss triggers data fetch."""
    # Arrange
    mock_task_service._db.list_tasks.return_value = sample_tasks

    # Act
    result = await service.fetch_tasks()

    # Assert
    assert result == sample_tasks
    mock_task_service._db.list_tasks.assert_called_once()
    assert service._tasks_cache is not None


@pytest.mark.asyncio
async def test_cache_hit_does_not_fetch(service, mock_task_service, sample_tasks):
    """Test cache hit does not trigger fetch."""
    # Arrange - pre-populate cache
    service._tasks_cache = CachedData(data=sample_tasks, ttl_seconds=10.0)

    # Act
    result = await service.fetch_tasks()

    # Assert
    assert result == sample_tasks
    mock_task_service._db.list_tasks.assert_not_called()


@pytest.mark.asyncio
async def test_expired_cache_refetches(service, mock_task_service, sample_tasks):
    """Test expired cache triggers refetch."""
    # Arrange - create expired cache
    old_task = Task(
        id=uuid4(),
        prompt="Old Task",
        summary="Old Task",
        status=TaskStatus.PENDING,
        source=TaskSource.HUMAN,
    )
    expired_cache = CachedData(data=[old_task], ttl_seconds=0.1)
    service._tasks_cache = expired_cache

    # Wait for cache to expire
    await asyncio.sleep(0.2)

    new_tasks = sample_tasks
    mock_task_service._db.list_tasks.return_value = new_tasks

    # Act
    result = await service.fetch_tasks()

    # Assert
    assert result == new_tasks
    mock_task_service._db.list_tasks.assert_called_once()


# Force refresh tests


@pytest.mark.asyncio
async def test_force_refresh_bypasses_cache(service, mock_task_service, sample_tasks):
    """Test force_refresh bypasses valid cache."""
    # Arrange - pre-populate cache
    cached_tasks = [
        Task(
            id=uuid4(),
            prompt="Cached",
            summary="Cached",
            status=TaskStatus.PENDING,
            source=TaskSource.HUMAN,
        )
    ]
    service._tasks_cache = CachedData(data=cached_tasks, ttl_seconds=10.0)

    mock_task_service._db.list_tasks.return_value = sample_tasks

    # Act
    result = await service.fetch_tasks(force_refresh=True)

    # Assert
    assert result == sample_tasks
    mock_task_service._db.list_tasks.assert_called_once()


# Error handling tests


@pytest.mark.asyncio
async def test_error_serves_stale_cache(service, mock_task_service, sample_tasks):
    """Test fetch error serves stale cache with warning."""
    # Arrange - expired cache exists
    service._tasks_cache = CachedData(data=sample_tasks, ttl_seconds=0.1)
    await asyncio.sleep(0.2)  # Expire cache

    # Mock fetch to raise error
    mock_task_service._db.list_tasks.side_effect = Exception("DB error")

    # Act
    result = await service.fetch_tasks()

    # Assert - should return stale cache
    assert result == sample_tasks


@pytest.mark.asyncio
async def test_error_no_cache_raises_exception(service, mock_task_service):
    """Test fetch error with no cache raises TUIDataError."""
    # Arrange - no cache
    mock_task_service._db.list_tasks.side_effect = Exception("DB error")

    # Act & Assert
    with pytest.raises(TUIDataError) as exc_info:
        await service.fetch_tasks()

    assert "Failed to fetch tasks" in str(exc_info.value)


# Filter tests


@pytest.mark.asyncio
async def test_filters_applied_after_caching(service, mock_task_service, sample_tasks):
    """Test filters are applied to cached data."""
    # Arrange
    service._tasks_cache = CachedData(data=sample_tasks, ttl_seconds=10.0)

    filter_pending = FilterState(statuses=[TaskStatus.PENDING])

    # Act
    result = await service.fetch_tasks(filters=filter_pending)

    # Assert
    assert len(result) == 1
    assert result[0].status == TaskStatus.PENDING
    mock_task_service._db.list_tasks.assert_not_called()


@pytest.mark.asyncio
async def test_text_search_filter(service, sample_tasks):
    """Test text search filter in summary and prompt."""
    # Arrange
    service._tasks_cache = CachedData(data=sample_tasks, ttl_seconds=10.0)
    filter_text = FilterState(text_search="Task 1")

    # Act
    result = await service.fetch_tasks(filters=filter_text)

    # Assert
    assert len(result) == 1
    assert "Task 1" in result[0].summary


# Queue status tests


@pytest.mark.asyncio
async def test_get_queue_status_caches_data(service, mock_task_service):
    """Test queue status is cached."""
    # Act
    result1 = await service.get_queue_status()
    result2 = await service.get_queue_status()

    # Assert
    assert result1.total_tasks == 10
    assert result2.total_tasks == 10
    # Should only call once (cached)
    assert mock_task_service.get_queue_status.call_count == 1


@pytest.mark.asyncio
async def test_get_queue_status_error_handling(service, mock_task_service):
    """Test queue status error handling with no cache."""
    # Arrange
    mock_task_service.get_queue_status.side_effect = Exception("DB error")

    # Act & Assert
    with pytest.raises(TUIDataError) as exc_info:
        await service.get_queue_status()

    assert "Failed to fetch queue status" in str(exc_info.value)


# Dependency graph tests


@pytest.mark.asyncio
async def test_get_dependency_graph_caches_data(
    service, mock_dependency_resolver
):
    """Test dependency graph is cached."""
    # Arrange
    graph_data = {
        uuid4(): [uuid4(), uuid4()],
    }
    mock_dependency_resolver._build_dependency_graph.return_value = graph_data

    # Act
    result1 = await service.get_dependency_graph()
    result2 = await service.get_dependency_graph()

    # Assert
    assert len(result1) == len(graph_data)
    # Should only call once (cached)
    assert mock_dependency_resolver._build_dependency_graph.call_count == 1


# Execution plan tests (no caching)


@pytest.mark.asyncio
async def test_get_execution_plan_not_cached(service, mock_task_service):
    """Test execution plan is not cached (parameterized query)."""
    # Arrange
    task_ids = [uuid4(), uuid4()]

    # Act
    result = await service.get_execution_plan(task_ids)

    # Assert
    assert result.total_tasks == len(task_ids)
    assert result.total_batches == 2
    mock_task_service.get_task_execution_plan.assert_called_once_with(task_ids)


# Feature branch summary tests (no caching)


@pytest.mark.asyncio
async def test_get_feature_branch_summary_not_cached(service, mock_db):
    """Test feature branch summary is not cached."""
    # Act
    result = await service.get_feature_branch_summary("test-branch")

    # Assert
    assert result.feature_branch == "test-branch"
    assert result.total_tasks == 5
    mock_db.get_feature_branch_summary.assert_called_once_with("test-branch")


# Auto-refresh tests


@pytest.mark.asyncio
async def test_auto_refresh_updates_cache(service, mock_task_service, sample_tasks):
    """Test auto-refresh periodically updates cache."""
    # Arrange
    refresh_count = 0

    def callback():
        nonlocal refresh_count
        refresh_count += 1

    mock_task_service._db.list_tasks.return_value = sample_tasks

    # Act
    service.start_auto_refresh(callback, interval=0.1)
    await asyncio.sleep(0.35)  # Wait for ~3 refreshes
    service.stop_auto_refresh()

    # Assert
    assert refresh_count >= 2  # At least 2 callbacks
    assert service._tasks_cache is not None


@pytest.mark.asyncio
async def test_stop_auto_refresh_cancels_task(service):
    """Test stop_auto_refresh cancels background task."""
    # Arrange
    service.start_auto_refresh(lambda: None, interval=1.0)
    assert service._refresh_task is not None

    # Act
    service.stop_auto_refresh()

    # Assert
    assert service._refresh_task is None
    assert service._refresh_callback is None


# Manual cache invalidation tests


@pytest.mark.asyncio
async def test_manual_cache_invalidation(service, sample_tasks):
    """Test manual cache invalidation clears cache."""
    # Arrange
    service._tasks_cache = CachedData(data=sample_tasks, ttl_seconds=10.0)
    service._graph_cache = CachedData(data={}, ttl_seconds=10.0)

    # Act
    service.invalidate_cache()

    # Assert
    assert service._tasks_cache is None
    assert service._graph_cache is None


@pytest.mark.asyncio
async def test_selective_cache_invalidation(service, sample_tasks):
    """Test selective cache invalidation."""
    # Arrange
    service._tasks_cache = CachedData(data=sample_tasks, ttl_seconds=10.0)
    service._graph_cache = CachedData(data={}, ttl_seconds=10.0)

    # Act
    service.invalidate_cache("_tasks_cache")

    # Assert
    assert service._tasks_cache is None
    assert service._graph_cache is not None  # Should remain


@pytest.mark.asyncio
async def test_refresh_all_invalidates_and_refetches(
    service, mock_task_service, mock_dependency_resolver, sample_tasks
):
    """Test refresh_all invalidates and refetches all caches."""
    # Arrange
    mock_task_service._db.list_tasks.return_value = sample_tasks
    mock_dependency_resolver._build_dependency_graph.return_value = {}

    # Act
    await service.refresh_all()

    # Assert
    assert service._tasks_cache is not None
    assert service._graph_cache is not None
    assert service._status_cache is not None


# TTL and time_remaining tests


def test_cached_data_is_expired():
    """Test CachedData.is_expired() method."""
    # Arrange
    cache = CachedData(data=[], ttl_seconds=0.1)

    # Assert - initially not expired
    assert not cache.is_expired()

    # Wait and check expiration
    import time

    time.sleep(0.2)
    assert cache.is_expired()


def test_cached_data_time_remaining():
    """Test CachedData.time_remaining() method."""
    # Arrange
    cache = CachedData(data=[], ttl_seconds=2.0)

    # Assert
    remaining = cache.time_remaining()
    assert 1.5 < remaining <= 2.0

    # Expired cache
    import time

    time.sleep(2.1)
    assert cache.time_remaining() == 0.0
