"""Unit tests for TaskQueueService summary parameter handling.

Tests verify that:
1. summary parameter is accepted by enqueue_task
2. summary is passed through to Task model
3. Pydantic validation is triggered correctly
4. Backward compatibility is maintained (summary=None works)
"""

from pathlib import Path

import pytest
from abathur.domain.models import TaskSource
from abathur.infrastructure.database import Database
from abathur.services.dependency_resolver import DependencyResolver
from abathur.services.priority_calculator import PriorityCalculator
from abathur.services.task_queue_service import TaskQueueError, TaskQueueService


class TestTaskQueueServiceSummary:
    """Test TaskQueueService summary parameter handling."""

    @pytest.mark.asyncio
    async def test_enqueue_task_with_summary(self):
        """Test enqueue_task accepts summary parameter and creates task correctly."""
        # Setup
        db = Database(Path(":memory:"))
        await db.initialize()
        dependency_resolver = DependencyResolver(db)
        priority_calculator = PriorityCalculator(dependency_resolver)
        service = TaskQueueService(db, dependency_resolver, priority_calculator)

        # Act
        task = await service.enqueue_task(
            description="Test task with summary", source=TaskSource.HUMAN, summary="Test summary"
        )

        # Assert
        assert task is not None
        assert task.summary == "Test summary"
        assert task.prompt == "Test task with summary"
        assert task.source == TaskSource.HUMAN

        # Verify task persisted with summary
        retrieved = await db.get_task(task.id)
        assert retrieved is not None
        assert retrieved.summary == "Test summary"

    @pytest.mark.asyncio
    async def test_enqueue_task_without_summary(self):
        """Test enqueue_task works without summary (backward compatibility)."""
        # Setup
        db = Database(Path(":memory:"))
        await db.initialize()
        dependency_resolver = DependencyResolver(db)
        priority_calculator = PriorityCalculator(dependency_resolver)
        service = TaskQueueService(db, dependency_resolver, priority_calculator)

        # Act
        task = await service.enqueue_task(
            description="Test task without summary", source=TaskSource.HUMAN
        )

        # Assert
        assert task is not None
        assert task.summary is None
        assert task.prompt == "Test task without summary"

        # Verify task persisted with null summary
        retrieved = await db.get_task(task.id)
        assert retrieved is not None
        assert retrieved.summary is None

    @pytest.mark.asyncio
    async def test_enqueue_task_summary_max_length(self):
        """Test Pydantic validates summary max_length constraint."""
        # Setup
        db = Database(Path(":memory:"))
        await db.initialize()
        dependency_resolver = DependencyResolver(db)
        priority_calculator = PriorityCalculator(dependency_resolver)
        service = TaskQueueService(db, dependency_resolver, priority_calculator)

        # Valid: 500 characters (max)
        summary_500 = "x" * 500
        task = await service.enqueue_task(
            description="Test with 500 char summary", source=TaskSource.HUMAN, summary=summary_500
        )
        assert task.summary == summary_500
        assert len(task.summary) == 500

        # Invalid: 501 characters (exceeds max)
        summary_501 = "x" * 501
        with pytest.raises(TaskQueueError) as exc_info:
            await service.enqueue_task(
                description="Test with 501 char summary",
                source=TaskSource.HUMAN,
                summary=summary_501,
            )

        # Verify validation error message mentions max_length
        error_msg = str(exc_info.value)
        assert "String should have at most 500 characters" in error_msg

    @pytest.mark.asyncio
    async def test_enqueue_task_summary_with_prerequisites(self):
        """Test summary works correctly with task dependencies."""
        # Setup
        db = Database(Path(":memory:"))
        await db.initialize()
        dependency_resolver = DependencyResolver(db)
        priority_calculator = PriorityCalculator(dependency_resolver)
        service = TaskQueueService(db, dependency_resolver, priority_calculator)

        # Create prerequisite task
        prereq = await service.enqueue_task(
            description="Prerequisite task", source=TaskSource.HUMAN, summary="Prereq summary"
        )

        # Create dependent task with summary
        dependent = await service.enqueue_task(
            description="Dependent task",
            source=TaskSource.HUMAN,
            summary="Dependent summary",
            prerequisites=[prereq.id],
        )

        # Assert both tasks have summaries
        assert prereq.summary == "Prereq summary"
        assert dependent.summary == "Dependent summary"

        # Verify both persisted correctly
        retrieved_prereq = await db.get_task(prereq.id)
        retrieved_dependent = await db.get_task(dependent.id)
        assert retrieved_prereq.summary == "Prereq summary"
        assert retrieved_dependent.summary == "Dependent summary"

    @pytest.mark.asyncio
    async def test_enqueue_task_empty_summary(self):
        """Test empty string summary is allowed."""
        # Setup
        db = Database(Path(":memory:"))
        await db.initialize()
        dependency_resolver = DependencyResolver(db)
        priority_calculator = PriorityCalculator(dependency_resolver)
        service = TaskQueueService(db, dependency_resolver, priority_calculator)

        # Act
        task = await service.enqueue_task(
            description="Test task with empty summary", source=TaskSource.HUMAN, summary=""
        )

        # Assert
        assert task is not None
        assert task.summary == ""

        # Verify persisted
        retrieved = await db.get_task(task.id)
        assert retrieved is not None
        assert retrieved.summary == ""
