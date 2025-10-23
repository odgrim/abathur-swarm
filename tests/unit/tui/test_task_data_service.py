"""Unit tests for TaskDataService with mocked dependencies.

Tests caching, filtering, and data transformation logic in isolation.
All database and service dependencies are mocked.
"""

import pytest
from datetime import datetime, timezone, timedelta
from uuid import uuid4
from unittest.mock import Mock, AsyncMock, patch

from abathur.domain.models import Task, TaskStatus, TaskSource


class MockTaskDataService:
    """Mock TaskDataService for testing (implementation TBD)."""

    def __init__(self, db, task_queue_service=None, dependency_resolver=None):
        self.db = db
        self.task_queue_service = task_queue_service
        self.dependency_resolver = dependency_resolver
        self._cache = {}
        self._cache_timestamp = None
        self.cache_ttl_seconds = 30
        self.last_refresh = None

    async def fetch_tasks(self, force_refresh=False, filter_state=None):
        """Fetch tasks from database with caching."""
        # Check cache
        if not force_refresh and self._is_cache_valid():
            return self._cache.get("tasks", [])

        # Fetch from database
        tasks = await self.db.list_tasks()

        # Apply filters
        if filter_state:
            tasks = [t for t in tasks if filter_state.matches(t)]

        # Update cache
        self._cache["tasks"] = tasks
        self._cache_timestamp = datetime.now(timezone.utc)
        self.last_refresh = self._cache_timestamp

        return tasks

    def _is_cache_valid(self):
        """Check if cache is still valid within TTL."""
        if self._cache_timestamp is None:
            return False

        elapsed = (datetime.now(timezone.utc) - self._cache_timestamp).total_seconds()
        return elapsed < self.cache_ttl_seconds

    async def get_dependency_graph(self):
        """Build dependency graph adjacency list."""
        tasks = await self.fetch_tasks()

        graph = {}
        for task in tasks:
            graph[str(task.id)] = [str(dep_id) for dep_id in task.dependencies]

        return graph

    async def get_queue_status(self):
        """Get queue statistics."""
        tasks = await self.fetch_tasks()

        status_counts = {}
        for task in tasks:
            status_counts[task.status] = status_counts.get(task.status, 0) + 1

        total = len(tasks)
        avg_priority = sum(t.calculated_priority for t in tasks) / total if total > 0 else 0

        return Mock(
            total_tasks=total,
            pending_count=status_counts.get(TaskStatus.PENDING, 0),
            running_count=status_counts.get(TaskStatus.RUNNING, 0),
            completed_count=status_counts.get(TaskStatus.COMPLETED, 0),
            failed_count=status_counts.get(TaskStatus.FAILED, 0),
            avg_priority=avg_priority,
        )


class MockFilterState:
    """Mock FilterState for testing (implementation TBD)."""

    def __init__(
        self,
        status=None,
        agent_type=None,
        feature_branch=None,
        text_search=None,
    ):
        self.status = status
        self.agent_type = agent_type
        self.feature_branch = feature_branch
        self.text_search = text_search

    def matches(self, task: Task) -> bool:
        """Check if task matches all active filters."""
        if self.status and task.status != self.status:
            return False

        if self.agent_type and task.agent_type != self.agent_type:
            return False

        if self.feature_branch and task.feature_branch != self.feature_branch:
            return False

        if self.text_search:
            search_text = self.text_search.lower()
            if search_text not in (task.summary or "").lower():
                if search_text not in (task.prompt or "").lower():
                    return False

        return True

    def is_active(self) -> bool:
        """Check if any filters are active."""
        return any(
            [
                self.status is not None,
                self.agent_type is not None,
                self.feature_branch is not None,
                self.text_search is not None,
            ]
        )


class TestTaskDataServiceCaching:
    """Test suite for TaskDataService caching behavior."""

    @pytest.fixture
    def mock_db(self):
        """Create mock database."""
        db = AsyncMock()
        db.list_tasks = AsyncMock(return_value=[])
        return db

    @pytest.fixture
    def service(self, mock_db):
        """Create TaskDataService with mocked database."""
        return MockTaskDataService(mock_db)

    @pytest.mark.asyncio
    async def test_fetch_tasks_calls_database_on_first_fetch(self, service, mock_db):
        """Test first fetch queries database."""
        # Arrange
        sample_tasks = [
            Task(
                id=uuid4(),
                prompt="Task 1",
                summary="Task 1",
                agent_type="test",
                status=TaskStatus.PENDING,
                calculated_priority=5.0,
                dependency_depth=0,
                submitted_at=datetime.now(timezone.utc),
                source=TaskSource.HUMAN,
            )
        ]
        mock_db.list_tasks.return_value = sample_tasks

        # Act
        tasks = await service.fetch_tasks()

        # Assert
        mock_db.list_tasks.assert_called_once()
        assert tasks == sample_tasks

    @pytest.mark.asyncio
    async def test_fetch_tasks_uses_cache_within_ttl(self, service, mock_db):
        """Test second fetch within TTL uses cache."""
        # Arrange
        sample_tasks = [
            Task(
                id=uuid4(),
                prompt="Task 1",
                summary="Task 1",
                agent_type="test",
                status=TaskStatus.PENDING,
                calculated_priority=5.0,
                dependency_depth=0,
                submitted_at=datetime.now(timezone.utc),
                source=TaskSource.HUMAN,
            )
        ]
        mock_db.list_tasks.return_value = sample_tasks

        # Act - first fetch
        tasks1 = await service.fetch_tasks()

        # Act - second fetch (should hit cache)
        tasks2 = await service.fetch_tasks()

        # Assert - database called only once
        assert mock_db.list_tasks.call_count == 1
        assert tasks1 is tasks2  # Same object reference

    @pytest.mark.asyncio
    async def test_fetch_tasks_force_refresh_bypasses_cache(self, service, mock_db):
        """Test force_refresh parameter bypasses cache."""
        # Arrange
        task1 = Task(
            id=uuid4(),
            prompt="Task 1",
            summary="Task 1",
            agent_type="test",
            status=TaskStatus.PENDING,
            calculated_priority=5.0,
            dependency_depth=0,
            submitted_at=datetime.now(timezone.utc),
            source=TaskSource.HUMAN,
        )
        task2 = Task(
            id=uuid4(),
            prompt="Task 2",
            summary="Task 2",
            agent_type="test",
            status=TaskStatus.RUNNING,
            calculated_priority=8.0,
            dependency_depth=0,
            submitted_at=datetime.now(timezone.utc),
            source=TaskSource.HUMAN,
        )

        # First fetch returns task1, second fetch returns task2
        mock_db.list_tasks.side_effect = [[task1], [task2]]

        # Act - first fetch
        tasks1 = await service.fetch_tasks()

        # Act - force refresh
        tasks2 = await service.fetch_tasks(force_refresh=True)

        # Assert - database called twice
        assert mock_db.list_tasks.call_count == 2
        assert len(tasks1) == 1
        assert len(tasks2) == 1
        assert tasks1[0].id != tasks2[0].id

    @pytest.mark.asyncio
    async def test_is_cache_valid_returns_false_when_no_cache(self, service):
        """Test _is_cache_valid returns False when cache empty."""
        assert service._is_cache_valid() is False

    @pytest.mark.asyncio
    async def test_is_cache_valid_returns_true_within_ttl(self, service, mock_db):
        """Test _is_cache_valid returns True within TTL."""
        # Arrange - populate cache
        mock_db.list_tasks.return_value = []
        await service.fetch_tasks()

        # Act & Assert
        assert service._is_cache_valid() is True

    @pytest.mark.asyncio
    async def test_is_cache_valid_returns_false_after_ttl(self, service, mock_db):
        """Test _is_cache_valid returns False after TTL expires."""
        # Arrange - populate cache
        mock_db.list_tasks.return_value = []
        await service.fetch_tasks()

        # Simulate time passage beyond TTL
        service._cache_timestamp = datetime.now(timezone.utc) - timedelta(seconds=31)

        # Act & Assert
        assert service._is_cache_valid() is False


class TestTaskDataServiceFiltering:
    """Test suite for filtering functionality."""

    @pytest.fixture
    def mock_db(self):
        """Create mock database with sample tasks."""
        db = AsyncMock()

        sample_tasks = [
            Task(
                id=uuid4(),
                prompt="Task 1",
                summary="Task 1",
                agent_type="agent-a",
                status=TaskStatus.PENDING,
                calculated_priority=5.0,
                dependency_depth=0,
                submitted_at=datetime.now(timezone.utc),
                source=TaskSource.HUMAN,
                feature_branch="feature/test",
            ),
            Task(
                id=uuid4(),
                prompt="Task 2",
                summary="Task 2",
                agent_type="agent-b",
                status=TaskStatus.RUNNING,
                calculated_priority=8.0,
                dependency_depth=0,
                submitted_at=datetime.now(timezone.utc),
                source=TaskSource.HUMAN,
                feature_branch="feature/test",
            ),
            Task(
                id=uuid4(),
                prompt="Task 3",
                summary="Task 3",
                agent_type="agent-a",
                status=TaskStatus.COMPLETED,
                calculated_priority=10.0,
                dependency_depth=0,
                submitted_at=datetime.now(timezone.utc),
                source=TaskSource.HUMAN,
                feature_branch="feature/other",
            ),
        ]

        db.list_tasks.return_value = sample_tasks
        return db

    @pytest.fixture
    def service(self, mock_db):
        """Create TaskDataService with mocked database."""
        return MockTaskDataService(mock_db)

    @pytest.mark.asyncio
    async def test_fetch_tasks_with_status_filter(self, service):
        """Test filtering by task status."""
        # Arrange
        filter_state = MockFilterState(status=TaskStatus.PENDING)

        # Act
        tasks = await service.fetch_tasks(filter_state=filter_state)

        # Assert - only pending tasks
        assert len(tasks) == 1
        assert tasks[0].status == TaskStatus.PENDING

    @pytest.mark.asyncio
    async def test_fetch_tasks_with_agent_type_filter(self, service):
        """Test filtering by agent type."""
        # Arrange
        filter_state = MockFilterState(agent_type="agent-a")

        # Act
        tasks = await service.fetch_tasks(filter_state=filter_state)

        # Assert - only agent-a tasks
        assert len(tasks) == 2
        assert all(t.agent_type == "agent-a" for t in tasks)

    @pytest.mark.asyncio
    async def test_fetch_tasks_with_feature_branch_filter(self, service):
        """Test filtering by feature branch."""
        # Arrange
        filter_state = MockFilterState(feature_branch="feature/test")

        # Act
        tasks = await service.fetch_tasks(filter_state=filter_state)

        # Assert - only feature/test tasks
        assert len(tasks) == 2
        assert all(t.feature_branch == "feature/test" for t in tasks)

    @pytest.mark.asyncio
    async def test_fetch_tasks_with_text_search_filter(self, service, mock_db):
        """Test filtering by text search in summary/prompt."""
        # Arrange
        mock_db.list_tasks.return_value = [
            Task(
                id=uuid4(),
                prompt="Implement authentication",
                summary="Implement authentication",
                agent_type="test",
                status=TaskStatus.PENDING,
                calculated_priority=5.0,
                dependency_depth=0,
                submitted_at=datetime.now(timezone.utc),
                source=TaskSource.HUMAN,
            ),
            Task(
                id=uuid4(),
                prompt="Write tests",
                summary="Write tests",
                agent_type="test",
                status=TaskStatus.PENDING,
                calculated_priority=5.0,
                dependency_depth=0,
                submitted_at=datetime.now(timezone.utc),
                source=TaskSource.HUMAN,
            ),
        ]

        filter_state = MockFilterState(text_search="auth")

        # Act
        tasks = await service.fetch_tasks(filter_state=filter_state)

        # Assert - only tasks with "auth" in summary/prompt
        assert len(tasks) == 1
        assert "authentication" in tasks[0].summary.lower()

    @pytest.mark.asyncio
    async def test_fetch_tasks_with_combined_filters(self, service):
        """Test combining multiple filters (AND logic)."""
        # Arrange
        filter_state = MockFilterState(
            status=TaskStatus.PENDING, agent_type="agent-a"
        )

        # Act
        tasks = await service.fetch_tasks(filter_state=filter_state)

        # Assert - tasks matching both filters
        assert len(tasks) == 1
        assert tasks[0].status == TaskStatus.PENDING
        assert tasks[0].agent_type == "agent-a"


class TestTaskDataServiceDependencyGraph:
    """Test suite for dependency graph construction."""

    @pytest.fixture
    def mock_db(self):
        """Create mock database with tasks having dependencies."""
        db = AsyncMock()

        task1_id = uuid4()
        task2_id = uuid4()
        task3_id = uuid4()

        sample_tasks = [
            Task(
                id=task1_id,
                prompt="Task 1",
                summary="Task 1",
                agent_type="test",
                status=TaskStatus.COMPLETED,
                calculated_priority=10.0,
                dependency_depth=0,
                submitted_at=datetime.now(timezone.utc),
                source=TaskSource.HUMAN,
                dependencies=[],
            ),
            Task(
                id=task2_id,
                prompt="Task 2",
                summary="Task 2",
                agent_type="test",
                status=TaskStatus.RUNNING,
                calculated_priority=8.0,
                dependency_depth=1,
                submitted_at=datetime.now(timezone.utc),
                source=TaskSource.HUMAN,
                dependencies=[task1_id],
            ),
            Task(
                id=task3_id,
                prompt="Task 3",
                summary="Task 3",
                agent_type="test",
                status=TaskStatus.PENDING,
                calculated_priority=5.0,
                dependency_depth=2,
                submitted_at=datetime.now(timezone.utc),
                source=TaskSource.HUMAN,
                dependencies=[task1_id, task2_id],
            ),
        ]

        db.list_tasks.return_value = sample_tasks
        return db

    @pytest.fixture
    def service(self, mock_db):
        """Create TaskDataService with mocked database."""
        return MockTaskDataService(mock_db)

    @pytest.mark.asyncio
    async def test_get_dependency_graph_builds_adjacency_list(self, service, mock_db):
        """Test dependency graph constructed as adjacency list."""
        # Act
        graph = await service.get_dependency_graph()

        # Assert - verify structure
        assert isinstance(graph, dict)
        assert len(graph) == 3

        # Verify task1 has no dependencies
        task1_id = str(mock_db.list_tasks.return_value[0].id)
        assert graph[task1_id] == []

        # Verify task2 depends on task1
        task2_id = str(mock_db.list_tasks.return_value[1].id)
        assert task1_id in graph[task2_id]

        # Verify task3 depends on task1 and task2
        task3_id = str(mock_db.list_tasks.return_value[2].id)
        assert task1_id in graph[task3_id]
        assert task2_id in graph[task3_id]


class TestTaskDataServiceQueueStatus:
    """Test suite for queue statistics."""

    @pytest.fixture
    def mock_db(self):
        """Create mock database with varied task statuses."""
        db = AsyncMock()

        sample_tasks = [
            Task(
                id=uuid4(),
                prompt="Task 1",
                summary="Task 1",
                agent_type="test",
                status=TaskStatus.PENDING,
                calculated_priority=5.0,
                dependency_depth=0,
                submitted_at=datetime.now(timezone.utc),
                source=TaskSource.HUMAN,
            ),
            Task(
                id=uuid4(),
                prompt="Task 2",
                summary="Task 2",
                agent_type="test",
                status=TaskStatus.PENDING,
                calculated_priority=7.0,
                dependency_depth=0,
                submitted_at=datetime.now(timezone.utc),
                source=TaskSource.HUMAN,
            ),
            Task(
                id=uuid4(),
                prompt="Task 3",
                summary="Task 3",
                agent_type="test",
                status=TaskStatus.RUNNING,
                calculated_priority=10.0,
                dependency_depth=0,
                submitted_at=datetime.now(timezone.utc),
                source=TaskSource.HUMAN,
            ),
            Task(
                id=uuid4(),
                prompt="Task 4",
                summary="Task 4",
                agent_type="test",
                status=TaskStatus.COMPLETED,
                calculated_priority=8.0,
                dependency_depth=0,
                submitted_at=datetime.now(timezone.utc),
                source=TaskSource.HUMAN,
            ),
        ]

        db.list_tasks.return_value = sample_tasks
        return db

    @pytest.fixture
    def service(self, mock_db):
        """Create TaskDataService with mocked database."""
        return MockTaskDataService(mock_db)

    @pytest.mark.asyncio
    async def test_get_queue_status_calculates_total_tasks(self, service):
        """Test total task count."""
        # Act
        status = await service.get_queue_status()

        # Assert
        assert status.total_tasks == 4

    @pytest.mark.asyncio
    async def test_get_queue_status_counts_by_status(self, service):
        """Test status-specific counts."""
        # Act
        status = await service.get_queue_status()

        # Assert
        assert status.pending_count == 2
        assert status.running_count == 1
        assert status.completed_count == 1
        assert status.failed_count == 0

    @pytest.mark.asyncio
    async def test_get_queue_status_calculates_average_priority(self, service):
        """Test average priority calculation."""
        # Act
        status = await service.get_queue_status()

        # Assert - average of 5.0, 7.0, 10.0, 8.0 = 7.5
        assert status.avg_priority == 7.5

    @pytest.mark.asyncio
    async def test_get_queue_status_handles_empty_queue(self, mock_db):
        """Test queue status with no tasks."""
        # Arrange
        mock_db.list_tasks.return_value = []
        service = MockTaskDataService(mock_db)

        # Act
        status = await service.get_queue_status()

        # Assert
        assert status.total_tasks == 0
        assert status.avg_priority == 0
