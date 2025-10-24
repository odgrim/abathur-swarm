"""Unit tests for TaskCoordinator.

Tests the TaskCoordinator application layer, specifically the list_tasks() method
with the exclude_status parameter for filtering tasks.

This test suite verifies:
- Parameter pass-through from application layer to database layer
- Backward compatibility with existing code (exclude_status defaults to None)
- Clean architecture: application layer doesn't validate, just passes parameters
"""

from unittest.mock import AsyncMock, Mock
from uuid import uuid4

import pytest
from abathur.application.task_coordinator import TaskCoordinator
from abathur.domain.models import Task, TaskStatus
from abathur.infrastructure.database import Database


@pytest.fixture
def mock_database():
    """Mock database for testing TaskCoordinator.

    Creates a mock Database with AsyncMock for list_tasks method.
    This isolates TaskCoordinator from the database layer.
    """
    mock_db = Mock(spec=Database)
    mock_db.list_tasks = AsyncMock(return_value=[])
    return mock_db


@pytest.fixture
def task_coordinator(mock_database):
    """Create TaskCoordinator with mocked database.

    Args:
        mock_database: Mocked database fixture

    Returns:
        TaskCoordinator instance with mocked dependencies
    """
    return TaskCoordinator(database=mock_database)


class TestTaskCoordinatorListTasks:
    """Unit tests for TaskCoordinator.list_tasks() method."""

    @pytest.mark.asyncio
    async def test_list_tasks_with_exclude_status(
        self, task_coordinator: TaskCoordinator, mock_database: Mock
    ):
        """Test TaskCoordinator.list_tasks() passes exclude_status to database layer.

        TEST-APP-01: Verify exclude_status parameter is correctly passed through
        from application layer to database layer.

        This test ensures the TaskCoordinator acts as a clean pass-through,
        forwarding the exclude_status parameter without modification.
        """
        # Arrange
        mock_database.list_tasks.return_value = [
            Mock(spec=Task, id=uuid4(), status=TaskStatus.RUNNING),
            Mock(spec=Task, id=uuid4(), status=TaskStatus.PENDING),
        ]

        # Act
        result = await task_coordinator.list_tasks(exclude_status=TaskStatus.COMPLETED)

        # Assert - verify database.list_tasks() called with correct parameters
        mock_database.list_tasks.assert_called_once_with(
            status=None, exclude_status=TaskStatus.COMPLETED, limit=100
        )

        # Assert - verify results returned correctly
        assert len(result) == 2
        assert all(task.status != TaskStatus.COMPLETED for task in result)

    @pytest.mark.asyncio
    async def test_list_tasks_exclude_status_none_default(
        self, task_coordinator: TaskCoordinator, mock_database: Mock
    ):
        """Test TaskCoordinator.list_tasks() defaults exclude_status to None for backward compatibility.

        TEST-APP-02: Verify backward compatibility when exclude_status is not provided.

        This test ensures existing code that calls list_tasks() without exclude_status
        continues to work correctly, with exclude_status defaulting to None.
        """
        # Arrange
        mock_database.list_tasks.return_value = [
            Mock(spec=Task, id=uuid4(), status=TaskStatus.RUNNING),
            Mock(spec=Task, id=uuid4(), status=TaskStatus.COMPLETED),
        ]

        # Act - call without exclude_status parameter
        result = await task_coordinator.list_tasks(status=TaskStatus.RUNNING)

        # Assert - verify database.list_tasks() called with exclude_status=None (default)
        mock_database.list_tasks.assert_called_once_with(
            status=TaskStatus.RUNNING, exclude_status=None, limit=100
        )

        # Assert - verify results returned correctly
        assert len(result) == 2

    @pytest.mark.asyncio
    async def test_list_tasks_with_both_filters(
        self, task_coordinator: TaskCoordinator, mock_database: Mock
    ):
        """Test TaskCoordinator passes both status and exclude_status to database layer.

        Note: Application layer doesn't validate mutual exclusivity of parameters.
        That validation happens at the CLI layer. The application layer is responsible
        only for passing parameters through to the database layer.

        This test demonstrates clean separation of concerns:
        - CLI layer: Validates user input and parameter combinations
        - Application layer: Passes parameters through without validation
        - Database layer: Executes the query with given parameters
        """
        # Arrange
        mock_database.list_tasks.return_value = [
            Mock(spec=Task, id=uuid4(), status=TaskStatus.RUNNING),
        ]

        # Act - pass both status and exclude_status (validation happens at CLI layer)
        result = await task_coordinator.list_tasks(
            status=TaskStatus.RUNNING, exclude_status=TaskStatus.COMPLETED
        )

        # Assert - verify both parameters passed through to database
        mock_database.list_tasks.assert_called_once_with(
            status=TaskStatus.RUNNING, exclude_status=TaskStatus.COMPLETED, limit=100
        )

        # Assert - verify results returned correctly
        assert len(result) == 1
        assert result[0].status == TaskStatus.RUNNING

    @pytest.mark.asyncio
    async def test_list_tasks_with_custom_limit(
        self, task_coordinator: TaskCoordinator, mock_database: Mock
    ):
        """Test TaskCoordinator passes custom limit parameter correctly.

        Additional test to verify the limit parameter works alongside exclude_status.
        """
        # Arrange
        mock_database.list_tasks.return_value = []

        # Act
        await task_coordinator.list_tasks(exclude_status=TaskStatus.FAILED, limit=50)

        # Assert
        mock_database.list_tasks.assert_called_once_with(
            status=None, exclude_status=TaskStatus.FAILED, limit=50
        )

    @pytest.mark.asyncio
    async def test_list_tasks_no_parameters(
        self, task_coordinator: TaskCoordinator, mock_database: Mock
    ):
        """Test TaskCoordinator with default parameters (all defaults).

        Verifies backward compatibility when called with no parameters at all.
        """
        # Arrange
        mock_database.list_tasks.return_value = [
            Mock(spec=Task, id=uuid4(), status=TaskStatus.PENDING),
            Mock(spec=Task, id=uuid4(), status=TaskStatus.RUNNING),
            Mock(spec=Task, id=uuid4(), status=TaskStatus.COMPLETED),
        ]

        # Act - call with all defaults
        result = await task_coordinator.list_tasks()

        # Assert - verify default parameters passed to database
        mock_database.list_tasks.assert_called_once_with(
            status=None, exclude_status=None, limit=100
        )

        # Assert - verify all tasks returned
        assert len(result) == 3
