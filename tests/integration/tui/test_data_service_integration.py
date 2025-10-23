"""Integration tests for TUI data service with real database.

Tests TaskDataService integration with SQLite database:
- Fetch tasks from real database
- Build dependency graphs with real data
- Calculate queue statistics with real data
- Apply filtering to real task data
- Auto-refresh cycle simulation
- Error recovery with database failures
"""

import asyncio
import pytest
from collections.abc import AsyncGenerator
from pathlib import Path
from datetime import datetime, timezone, timedelta
from uuid import uuid4

from abathur.domain.models import Task, TaskStatus, TaskSource
from abathur.infrastructure.database import Database


# Import mock service from unit tests (to be implemented in actual code)
import sys
sys.path.insert(0, str(Path(__file__).parent.parent.parent / "unit" / "tui"))
from test_task_data_service import MockTaskDataService, MockFilterState


@pytest.fixture
async def memory_db() -> AsyncGenerator[Database, None]:
    """Create in-memory database for fast integration tests."""
    db = Database(Path(":memory:"))
    await db.initialize()
    yield db
    await db.close()


@pytest.fixture
async def populated_db(memory_db: Database) -> Database:
    """Create database with test task data."""
    # Insert test tasks with various states
    test_tasks = [
        Task(
            id=uuid4(),
            prompt="Test task 1",
            summary="Test task 1",
            agent_type="test-agent",
            status=TaskStatus.PENDING,
            calculated_priority=10.0,
            dependency_depth=0,
            submitted_at=datetime.now(timezone.utc),
            source=TaskSource.HUMAN,
            feature_branch="feature/test",
        ),
        Task(
            id=uuid4(),
            prompt="Test task 2",
            summary="Test task 2",
            agent_type="python-specialist",
            status=TaskStatus.RUNNING,
            calculated_priority=8.0,
            dependency_depth=0,
            submitted_at=datetime.now(timezone.utc),
            source=TaskSource.HUMAN,
            feature_branch="feature/test",
        ),
        Task(
            id=uuid4(),
            prompt="Test task 3",
            summary="Test task 3",
            agent_type="test-agent",
            status=TaskStatus.COMPLETED,
            calculated_priority=5.0,
            dependency_depth=0,
            submitted_at=datetime.now(timezone.utc),
            source=TaskSource.HUMAN,
            feature_branch="feature/other",
        ),
    ]

    for task in test_tasks:
        await memory_db.insert_task(task)

    return memory_db


@pytest.fixture
def data_service(populated_db: Database) -> MockTaskDataService:
    """Create TaskDataService with populated database."""
    return MockTaskDataService(populated_db)


class TestFetchTasksFromDatabase:
    """Test suite for fetching tasks from real database."""

    @pytest.mark.asyncio
    async def test_fetch_tasks_returns_all_tasks(self, data_service):
        """Test fetching all tasks from database."""
        # Act
        tasks = await data_service.fetch_tasks()

        # Assert
        assert len(tasks) == 3
        assert all(isinstance(t, Task) for t in tasks)

    @pytest.mark.asyncio
    async def test_fetch_tasks_with_empty_database(self, memory_db):
        """Test fetching from empty database."""
        # Arrange
        service = MockTaskDataService(memory_db)

        # Act
        tasks = await service.fetch_tasks()

        # Assert
        assert tasks == []

    @pytest.mark.asyncio
    async def test_fetch_tasks_preserves_task_attributes(self, data_service):
        """Test all task attributes preserved from database."""
        # Act
        tasks = await data_service.fetch_tasks()

        # Assert - verify attributes preserved
        task = tasks[0]
        assert task.id is not None
        assert task.summary is not None
        assert task.agent_type is not None
        assert task.status in TaskStatus
        assert isinstance(task.calculated_priority, float)


class TestDependencyGraphConstruction:
    """Test suite for dependency graph building with real data."""

    @pytest.fixture
    async def db_with_dependencies(self, memory_db: Database) -> Database:
        """Create database with tasks having dependency relationships."""
        task1_id = uuid4()
        task2_id = uuid4()
        task3_id = uuid4()

        tasks = [
            Task(
                id=task1_id,
                prompt="Independent task",
                summary="Independent task",
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
                prompt="Task depends on task1",
                summary="Task depends on task1",
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
                prompt="Task depends on task1 and task2",
                summary="Task depends on task1 and task2",
                agent_type="test",
                status=TaskStatus.PENDING,
                calculated_priority=5.0,
                dependency_depth=2,
                submitted_at=datetime.now(timezone.utc),
                source=TaskSource.HUMAN,
                dependencies=[task1_id, task2_id],
            ),
        ]

        for task in tasks:
            await memory_db.insert_task(task)

        return memory_db

    @pytest.mark.asyncio
    async def test_get_dependency_graph_with_real_data(self, db_with_dependencies):
        """Test building dependency graph from real database."""
        # Arrange
        service = MockTaskDataService(db_with_dependencies)

        # Act
        graph = await service.get_dependency_graph()

        # Assert
        assert isinstance(graph, dict)
        assert len(graph) == 3

        # Verify graph structure
        tasks = await db_with_dependencies.list_tasks()
        task1_id = str(tasks[0].id)
        task2_id = str(tasks[1].id)
        task3_id = str(tasks[2].id)

        # Independent task has no dependencies
        assert graph[task1_id] == []

        # Task2 depends on task1
        assert task1_id in graph[task2_id]

        # Task3 depends on both
        assert task1_id in graph[task3_id]
        assert task2_id in graph[task3_id]


class TestQueueStatistics:
    """Test suite for queue statistics calculation with real data."""

    @pytest.mark.asyncio
    async def test_get_queue_status_with_real_data(self, data_service):
        """Test queue statistics calculated from real database."""
        # Act
        status = await data_service.get_queue_status()

        # Assert
        assert status.total_tasks == 3
        assert status.pending_count == 1
        assert status.running_count == 1
        assert status.completed_count == 1
        assert status.failed_count == 0

    @pytest.mark.asyncio
    async def test_get_queue_status_calculates_average_priority(self, data_service):
        """Test average priority calculation."""
        # Act
        status = await data_service.get_queue_status()

        # Assert - average of 10.0, 8.0, 5.0 = 7.67
        assert 7.0 < status.avg_priority < 8.0

    @pytest.mark.asyncio
    async def test_get_queue_status_with_empty_queue(self, memory_db):
        """Test queue status with no tasks."""
        # Arrange
        service = MockTaskDataService(memory_db)

        # Act
        status = await service.get_queue_status()

        # Assert
        assert status.total_tasks == 0
        assert status.avg_priority == 0


class TestFilteringWithRealData:
    """Test suite for applying filters to real task data."""

    @pytest.mark.asyncio
    async def test_filter_by_status(self, data_service):
        """Test filtering tasks by status."""
        # Arrange
        filter_state = MockFilterState(status=TaskStatus.PENDING)

        # Act
        tasks = await data_service.fetch_tasks(filter_state=filter_state)

        # Assert - only pending tasks
        assert len(tasks) == 1
        assert all(t.status == TaskStatus.PENDING for t in tasks)

    @pytest.mark.asyncio
    async def test_filter_by_agent_type(self, data_service):
        """Test filtering tasks by agent type."""
        # Arrange
        filter_state = MockFilterState(agent_type="test-agent")

        # Act
        tasks = await data_service.fetch_tasks(filter_state=filter_state)

        # Assert - only test-agent tasks
        assert len(tasks) == 2
        assert all(t.agent_type == "test-agent" for t in tasks)

    @pytest.mark.asyncio
    async def test_filter_by_feature_branch(self, data_service):
        """Test filtering tasks by feature branch."""
        # Arrange
        filter_state = MockFilterState(feature_branch="feature/test")

        # Act
        tasks = await data_service.fetch_tasks(filter_state=filter_state)

        # Assert - only feature/test tasks
        assert len(tasks) == 2
        assert all(t.feature_branch == "feature/test" for t in tasks)

    @pytest.mark.asyncio
    async def test_filter_by_text_search(self, populated_db):
        """Test filtering by text search in real data."""
        # Arrange
        # Add a task with specific text
        await populated_db.insert_task(
            Task(
                id=uuid4(),
                prompt="Implement authentication module",
                summary="Implement authentication",
                agent_type="test",
                status=TaskStatus.PENDING,
                calculated_priority=5.0,
                dependency_depth=0,
                submitted_at=datetime.now(timezone.utc),
                source=TaskSource.HUMAN,
            )
        )

        service = MockTaskDataService(populated_db)
        filter_state = MockFilterState(text_search="authentication")

        # Act
        tasks = await service.fetch_tasks(filter_state=filter_state)

        # Assert - only tasks with "authentication"
        assert len(tasks) == 1
        assert "authentication" in tasks[0].summary.lower()

    @pytest.mark.asyncio
    async def test_combined_filters(self, data_service):
        """Test combining multiple filters with AND logic."""
        # Arrange
        filter_state = MockFilterState(
            status=TaskStatus.PENDING,
            feature_branch="feature/test",
        )

        # Act
        tasks = await data_service.fetch_tasks(filter_state=filter_state)

        # Assert - tasks matching both filters
        assert len(tasks) == 1
        assert tasks[0].status == TaskStatus.PENDING
        assert tasks[0].feature_branch == "feature/test"


class TestCachingWithRealDatabase:
    """Test suite for caching behavior with real database."""

    @pytest.mark.asyncio
    async def test_cache_hit_uses_cached_data(self, data_service, populated_db):
        """Test second fetch within TTL uses cached data."""
        # Act - first fetch
        tasks1 = await data_service.fetch_tasks()

        # Modify database (add new task)
        await populated_db.insert_task(
            Task(
                id=uuid4(),
                prompt="New task",
                summary="New task",
                agent_type="test",
                status=TaskStatus.PENDING,
                calculated_priority=5.0,
                dependency_depth=0,
                submitted_at=datetime.now(timezone.utc),
                source=TaskSource.HUMAN,
            )
        )

        # Act - second fetch (should use cache)
        tasks2 = await data_service.fetch_tasks()

        # Assert - cached data, doesn't include new task
        assert len(tasks1) == len(tasks2)
        assert tasks1 is tasks2  # Same object reference

    @pytest.mark.asyncio
    async def test_force_refresh_updates_cache(self, data_service, populated_db):
        """Test force refresh fetches latest data."""
        # Act - first fetch
        tasks1 = await data_service.fetch_tasks()

        # Modify database
        await populated_db.insert_task(
            Task(
                id=uuid4(),
                prompt="New task",
                summary="New task",
                agent_type="test",
                status=TaskStatus.PENDING,
                calculated_priority=5.0,
                dependency_depth=0,
                submitted_at=datetime.now(timezone.utc),
                source=TaskSource.HUMAN,
            )
        )

        # Act - force refresh
        tasks2 = await data_service.fetch_tasks(force_refresh=True)

        # Assert - new data includes added task
        assert len(tasks2) == len(tasks1) + 1

    @pytest.mark.asyncio
    async def test_cache_expiration_triggers_refresh(self, data_service, populated_db):
        """Test cache expiration causes database refresh."""
        # Arrange - set short TTL
        data_service.cache_ttl_seconds = 0.1

        # Act - first fetch
        tasks1 = await data_service.fetch_tasks()

        # Wait for cache expiration
        await asyncio.sleep(0.15)

        # Modify database
        await populated_db.insert_task(
            Task(
                id=uuid4(),
                prompt="New task",
                summary="New task",
                agent_type="test",
                status=TaskStatus.PENDING,
                calculated_priority=5.0,
                dependency_depth=0,
                submitted_at=datetime.now(timezone.utc),
                source=TaskSource.HUMAN,
            )
        )

        # Act - fetch after expiration
        tasks2 = await data_service.fetch_tasks()

        # Assert - new data fetched
        assert len(tasks2) == len(tasks1) + 1


class TestErrorRecovery:
    """Test suite for error handling with database failures."""

    @pytest.mark.asyncio
    async def test_handles_closed_database(self, memory_db):
        """Test error handling when database connection closed."""
        # Arrange
        service = MockTaskDataService(memory_db)

        # Close database
        await memory_db.close()

        # Act & Assert - should handle gracefully
        with pytest.raises(Exception):  # Could be specific TUIDataError
            await service.fetch_tasks()

    @pytest.mark.asyncio
    async def test_handles_invalid_task_data(self, memory_db):
        """Test error handling with corrupted task data."""
        # This would test error handling for database returning invalid data
        # Implementation depends on error handling strategy
        pass


class TestAutoRefreshSimulation:
    """Test suite for auto-refresh cycle simulation."""

    @pytest.mark.asyncio
    async def test_last_refresh_timestamp_updated(self, data_service):
        """Test last_refresh timestamp updated on fetch."""
        # Arrange
        initial_timestamp = data_service.last_refresh

        # Act
        await data_service.fetch_tasks()

        # Assert
        assert data_service.last_refresh is not None
        assert data_service.last_refresh != initial_timestamp

    @pytest.mark.asyncio
    async def test_multiple_refreshes_update_timestamp(self, data_service):
        """Test timestamp updates on each refresh."""
        # Act - first refresh
        await data_service.fetch_tasks()
        timestamp1 = data_service.last_refresh

        # Small delay
        await asyncio.sleep(0.01)

        # Act - second refresh
        await data_service.fetch_tasks(force_refresh=True)
        timestamp2 = data_service.last_refresh

        # Assert - timestamps differ
        assert timestamp2 > timestamp1


class TestComplexQueries:
    """Test suite for complex database queries and edge cases."""

    @pytest.mark.asyncio
    async def test_fetches_tasks_with_all_statuses(self, memory_db):
        """Test fetching tasks with all possible statuses."""
        # Arrange - insert task for each status
        for status in TaskStatus:
            await memory_db.insert_task(
                Task(
                    id=uuid4(),
                    prompt=f"Task with status {status.value}",
                    summary=f"Task {status.value}",
                    agent_type="test",
                    status=status,
                    calculated_priority=5.0,
                    dependency_depth=0,
                    submitted_at=datetime.now(timezone.utc),
                    source=TaskSource.HUMAN,
                )
            )

        service = MockTaskDataService(memory_db)

        # Act
        tasks = await service.fetch_tasks()

        # Assert - all 7 statuses represented
        statuses = {t.status for t in tasks}
        assert len(statuses) == len(TaskStatus)

    @pytest.mark.asyncio
    async def test_handles_large_dependency_chains(self, memory_db):
        """Test handling deeply nested dependency chains."""
        # Arrange - create chain of 10 dependent tasks
        task_ids = [uuid4() for _ in range(10)]

        for i, task_id in enumerate(task_ids):
            dependencies = [task_ids[i - 1]] if i > 0 else []

            await memory_db.insert_task(
                Task(
                    id=task_id,
                    prompt=f"Task {i}",
                    summary=f"Task {i}",
                    agent_type="test",
                    status=TaskStatus.PENDING,
                    calculated_priority=10 - i,
                    dependency_depth=i,
                    dependencies=dependencies,
                    submitted_at=datetime.now(timezone.utc),
                    source=TaskSource.HUMAN,
                )
            )

        service = MockTaskDataService(memory_db)

        # Act
        graph = await service.get_dependency_graph()

        # Assert - verify chain structure
        assert len(graph) == 10

        # Verify each task depends on previous (except first)
        for i in range(1, 10):
            task_id = str(task_ids[i])
            prev_id = str(task_ids[i - 1])
            assert prev_id in graph[task_id]
