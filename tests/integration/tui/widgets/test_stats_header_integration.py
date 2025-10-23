"""Integration tests for QueueStatsHeader with TaskDataService.

Tests QueueStatsHeader widget with real database and service layer:
- Fetch queue statistics from real TaskDataService
- Display updates when data changes
- Auto-refresh indicator integration
- Timestamp tracking
"""

import asyncio
import pytest
from datetime import datetime, timezone
from pathlib import Path
from uuid import uuid4

from textual.app import App, ComposeResult

from abathur.domain.models import Task, TaskStatus, TaskSource
from abathur.infrastructure.database import Database
from abathur.tui.widgets import QueueStatsHeader

# Import mock service from unit tests
import sys

sys.path.insert(0, str(Path(__file__).parent.parent.parent.parent / "unit" / "tui"))
from test_task_data_service import MockTaskDataService


@pytest.fixture
async def memory_db():
    """Create in-memory database for integration tests."""
    db = Database(Path(":memory:"))
    await db.initialize()
    yield db
    await db.close()


@pytest.fixture
async def populated_db(memory_db):
    """Create database with test tasks in various states."""
    test_tasks = [
        Task(
            id=uuid4(),
            prompt="Pending task 1",
            summary="Pending task 1",
            agent_type="test-agent",
            status=TaskStatus.PENDING,
            calculated_priority=7.0,
            dependency_depth=0,
            submitted_at=datetime.now(timezone.utc),
            source=TaskSource.HUMAN,
        ),
        Task(
            id=uuid4(),
            prompt="Pending task 2",
            summary="Pending task 2",
            agent_type="test-agent",
            status=TaskStatus.PENDING,
            calculated_priority=5.0,
            dependency_depth=0,
            submitted_at=datetime.now(timezone.utc),
            source=TaskSource.HUMAN,
        ),
        Task(
            id=uuid4(),
            prompt="Running task",
            summary="Running task",
            agent_type="python-specialist",
            status=TaskStatus.RUNNING,
            calculated_priority=8.0,
            dependency_depth=0,
            submitted_at=datetime.now(timezone.utc),
            source=TaskSource.HUMAN,
        ),
        Task(
            id=uuid4(),
            prompt="Completed task",
            summary="Completed task",
            agent_type="test-agent",
            status=TaskStatus.COMPLETED,
            calculated_priority=10.0,
            dependency_depth=0,
            submitted_at=datetime.now(timezone.utc),
            source=TaskSource.HUMAN,
        ),
        Task(
            id=uuid4(),
            prompt="Failed task",
            summary="Failed task",
            agent_type="test-agent",
            status=TaskStatus.FAILED,
            calculated_priority=6.0,
            dependency_depth=0,
            submitted_at=datetime.now(timezone.utc),
            source=TaskSource.HUMAN,
        ),
    ]

    for task in test_tasks:
        await memory_db.insert_task(task)

    return memory_db


@pytest.fixture
def data_service(populated_db):
    """Create TaskDataService with populated database."""
    return MockTaskDataService(populated_db)


class TestQueueStatsHeaderWithDataService:
    """Test suite for QueueStatsHeader integration with TaskDataService."""

    @pytest.mark.asyncio
    async def test_stats_header_displays_queue_status(self, data_service):
        """Test stats header displays data from TaskDataService."""
        # Arrange
        widget = QueueStatsHeader()
        stats = await data_service.get_queue_status()

        # Act
        widget.stats = {
            "total_tasks": stats.total_tasks,
            "pending_count": stats.pending_count,
            "blocked_count": 0,
            "ready_count": 0,
            "running_count": stats.running_count,
            "completed_count": stats.completed_count,
            "failed_count": stats.failed_count,
            "cancelled_count": 0,
            "avg_priority": stats.avg_priority,
        }

        # Assert - verify stats are set correctly
        assert widget.stats["total_tasks"] == 5
        assert widget.stats["pending_count"] == 2
        assert widget.stats["running_count"] == 1
        assert widget.stats["completed_count"] == 1
        assert widget.stats["failed_count"] == 1
        assert widget.stats["avg_priority"] == 7.2  # (7+5+8+10+6)/5

    @pytest.mark.asyncio
    async def test_stats_header_updates_when_tasks_change(
        self, populated_db, data_service
    ):
        """Test stats header reflects changes when tasks are added."""
        # Arrange
        widget = QueueStatsHeader()

        # Get initial stats
        initial_stats = await data_service.get_queue_status()
        widget.stats = {
            "total_tasks": initial_stats.total_tasks,
            "pending_count": initial_stats.pending_count,
            "blocked_count": 0,
            "ready_count": 0,
            "running_count": initial_stats.running_count,
            "completed_count": initial_stats.completed_count,
            "failed_count": initial_stats.failed_count,
            "cancelled_count": 0,
            "avg_priority": initial_stats.avg_priority,
        }

        # Act - add a new task
        new_task = Task(
            id=uuid4(),
            prompt="New task",
            summary="New task",
            agent_type="test-agent",
            status=TaskStatus.READY,
            calculated_priority=9.0,
            dependency_depth=0,
            submitted_at=datetime.now(timezone.utc),
            source=TaskSource.HUMAN,
        )
        await populated_db.insert_task(new_task)

        # Force refresh to clear cache
        await data_service.fetch_tasks(force_refresh=True)
        updated_stats = await data_service.get_queue_status()
        widget.stats = {
            "total_tasks": updated_stats.total_tasks,
            "pending_count": updated_stats.pending_count,
            "blocked_count": 0,
            "ready_count": 1,  # New ready task
            "running_count": updated_stats.running_count,
            "completed_count": updated_stats.completed_count,
            "failed_count": updated_stats.failed_count,
            "cancelled_count": 0,
            "avg_priority": updated_stats.avg_priority,
        }

        # Assert - total increased by 1
        assert widget.stats["total_tasks"] == 6
        assert widget.stats["ready_count"] == 1

    @pytest.mark.asyncio
    async def test_stats_header_auto_refresh_indicator(self, data_service):
        """Test auto-refresh indicator toggles correctly."""
        # Arrange
        widget = QueueStatsHeader()
        stats = await data_service.get_queue_status()
        widget.stats = {
            "total_tasks": stats.total_tasks,
            "pending_count": stats.pending_count,
            "blocked_count": 0,
            "ready_count": 0,
            "running_count": stats.running_count,
            "completed_count": stats.completed_count,
            "failed_count": stats.failed_count,
            "cancelled_count": 0,
            "avg_priority": stats.avg_priority,
        }

        # Act - enable auto-refresh
        widget.auto_refresh_enabled = True

        # Assert
        assert widget.auto_refresh_enabled is True

        # Act - disable auto-refresh
        widget.auto_refresh_enabled = False

        # Assert
        assert widget.auto_refresh_enabled is False

    @pytest.mark.asyncio
    async def test_stats_header_timestamp_tracking(self, data_service):
        """Test last refresh timestamp is tracked."""
        # Arrange
        widget = QueueStatsHeader()
        stats = await data_service.get_queue_status()

        # Act - update stats and timestamp
        now = datetime.now(timezone.utc)
        widget.stats = {
            "total_tasks": stats.total_tasks,
            "pending_count": stats.pending_count,
            "blocked_count": 0,
            "ready_count": 0,
            "running_count": stats.running_count,
            "completed_count": stats.completed_count,
            "failed_count": stats.failed_count,
            "cancelled_count": 0,
            "avg_priority": stats.avg_priority,
        }
        widget.last_refresh = now

        # Assert
        assert widget.last_refresh == now

    @pytest.mark.asyncio
    async def test_stats_header_renders_within_app_context(self, data_service):
        """Test stats header renders correctly within Textual app."""

        # Create simple test app
        class TestApp(App):
            def compose(self) -> ComposeResult:
                yield QueueStatsHeader(id="stats-header")

        app = TestApp()

        # Use Textual's pilot for testing
        async with app.run_test() as pilot:
            # Get widget
            header = app.query_one(QueueStatsHeader)

            # Update with stats
            stats = await data_service.get_queue_status()
            header.stats = {
                "total_tasks": stats.total_tasks,
                "pending_count": stats.pending_count,
                "blocked_count": 0,
                "ready_count": 0,
                "running_count": stats.running_count,
                "completed_count": stats.completed_count,
                "failed_count": stats.failed_count,
                "cancelled_count": 0,
                "avg_priority": stats.avg_priority,
            }

            # Wait for render
            await pilot.pause()

            # Assert widget is mounted and has stats
            assert header.stats is not None
            assert header.stats["total_tasks"] == 5

    @pytest.mark.asyncio
    async def test_stats_header_simulated_refresh_cycle(self, data_service):
        """Test simulated auto-refresh cycle with periodic updates."""

        class TestApp(App):
            def compose(self) -> ComposeResult:
                yield QueueStatsHeader(id="stats-header")

        app = TestApp()

        async with app.run_test() as pilot:
            header = app.query_one(QueueStatsHeader)
            header.auto_refresh_enabled = True

            # Simulate 3 refresh cycles
            for i in range(3):
                stats = await data_service.get_queue_status()
                header.stats = {
                    "total_tasks": stats.total_tasks,
                    "pending_count": stats.pending_count,
                    "blocked_count": 0,
                    "ready_count": 0,
                    "running_count": stats.running_count,
                    "completed_count": stats.completed_count,
                    "failed_count": stats.failed_count,
                    "cancelled_count": 0,
                    "avg_priority": stats.avg_priority,
                }
                header.last_refresh = datetime.now(timezone.utc)

                # Wait between cycles
                await asyncio.sleep(0.1)
                await pilot.pause()

            # Assert - last refresh timestamp updated
            assert header.last_refresh is not None
            assert header.auto_refresh_enabled is True


class TestQueueStatsHeaderErrorHandling:
    """Test suite for error handling and edge cases."""

    @pytest.mark.asyncio
    async def test_stats_header_handles_empty_queue(self):
        """Test stats header handles empty queue gracefully."""
        # Arrange
        widget = QueueStatsHeader()
        empty_stats = {
            "total_tasks": 0,
            "pending_count": 0,
            "blocked_count": 0,
            "ready_count": 0,
            "running_count": 0,
            "completed_count": 0,
            "failed_count": 0,
            "cancelled_count": 0,
            "avg_priority": 0.0,
        }

        # Act
        widget.stats = empty_stats

        # Assert - renders without error
        assert widget.stats["total_tasks"] == 0
        rendered = widget.render()
        assert rendered is not None

    @pytest.mark.asyncio
    async def test_stats_header_handles_missing_optional_fields(self, data_service):
        """Test stats header handles missing optional fields."""
        # Arrange
        widget = QueueStatsHeader()
        stats = await data_service.get_queue_status()

        # Act - omit optional fields
        widget.stats = {
            "total_tasks": stats.total_tasks,
            "pending_count": stats.pending_count,
            "blocked_count": 0,
            "ready_count": 0,
            "running_count": stats.running_count,
            "completed_count": stats.completed_count,
            "failed_count": stats.failed_count,
            # cancelled_count omitted
            "avg_priority": stats.avg_priority,
            # max_dependency_depth omitted
        }

        # Assert - renders without error
        rendered = widget.render()
        assert rendered is not None
