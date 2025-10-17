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
from abathur.services.task_queue_service import TaskQueueService


class TestTaskQueueServiceSummary:
    """Test TaskQueueService summary parameter handling."""

    @pytest.mark.asyncio
    async def test_enqueue_task_with_summary(self) -> None:
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
    async def test_enqueue_task_without_summary(self) -> None:
        """Test enqueue_task auto-generates summary when not provided."""
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
        assert task.summary == "User Prompt: Test task without summary"
        assert task.prompt == "Test task without summary"

        # Verify task persisted with auto-generated summary
        retrieved = await db.get_task(task.id)
        assert retrieved is not None
        assert retrieved.summary == "User Prompt: Test task without summary"

    @pytest.mark.asyncio
    async def test_enqueue_task_summary_max_length(self) -> None:
        """Test Pydantic auto-truncates summary to max_length (140 chars)."""
        # Setup
        db = Database(Path(":memory:"))
        await db.initialize()
        dependency_resolver = DependencyResolver(db)
        priority_calculator = PriorityCalculator(dependency_resolver)
        service = TaskQueueService(db, dependency_resolver, priority_calculator)

        # Valid: 140 characters (max after stripping)
        summary_140 = "x" * 140
        task = await service.enqueue_task(
            description="Test with 140 char summary", source=TaskSource.HUMAN, summary=summary_140
        )
        assert task.summary == summary_140
        assert len(task.summary) == 140

        # 141 characters - should be auto-truncated to 140 by Pydantic field validator
        summary_141 = "x" * 141
        task_truncated = await service.enqueue_task(
            description="Test with 141 char summary",
            source=TaskSource.HUMAN,
            summary=summary_141,
        )

        # Verify summary was truncated to exactly 140 characters
        assert task_truncated.summary is not None
        assert len(task_truncated.summary) == 140
        assert task_truncated.summary == "x" * 140

    @pytest.mark.asyncio
    async def test_enqueue_task_summary_with_prerequisites(self) -> None:
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
        assert retrieved_prereq is not None
        assert retrieved_dependent is not None
        assert retrieved_prereq.summary == "Prereq summary"
        assert retrieved_dependent.summary == "Dependent summary"

    @pytest.mark.asyncio
    async def test_enqueue_task_empty_summary(self) -> None:
        """Test empty string summary triggers auto-generation."""
        # Setup
        db = Database(Path(":memory:"))
        await db.initialize()
        dependency_resolver = DependencyResolver(db)
        priority_calculator = PriorityCalculator(dependency_resolver)
        service = TaskQueueService(db, dependency_resolver, priority_calculator)

        # Act - empty summary should trigger auto-generation
        task = await service.enqueue_task(
            description="Test task with empty summary", source=TaskSource.HUMAN, summary=""
        )

        # Assert - should auto-generate from description
        assert task is not None
        assert task.summary == "User Prompt: Test task with empty summary"

        # Verify persisted
        retrieved = await db.get_task(task.id)
        assert retrieved is not None
        assert retrieved.summary == "User Prompt: Test task with empty summary"

    @pytest.mark.asyncio
    async def test_enqueue_task_summary_empty_description_human(self) -> None:
        """Test empty description for human tasks generates 'Task'."""
        # Setup
        db = Database(Path(":memory:"))
        await db.initialize()
        dependency_resolver = DependencyResolver(db)
        priority_calculator = PriorityCalculator(dependency_resolver)
        service = TaskQueueService(db, dependency_resolver, priority_calculator)

        # Act - empty description should generate "Task"
        task = await service.enqueue_task(description="", source=TaskSource.HUMAN)

        # Assert
        assert task is not None
        assert task.summary == "Task"  # Not "User Prompt: "

        # Verify persisted
        retrieved = await db.get_task(task.id)
        assert retrieved is not None
        assert retrieved.summary == "Task"

    @pytest.mark.asyncio
    async def test_enqueue_task_summary_empty_description_agent(self) -> None:
        """Test empty description for agent tasks generates 'Task'."""
        # Setup
        db = Database(Path(":memory:"))
        await db.initialize()
        dependency_resolver = DependencyResolver(db)
        priority_calculator = PriorityCalculator(dependency_resolver)
        service = TaskQueueService(db, dependency_resolver, priority_calculator)

        # Act - empty description should generate "Task"
        task = await service.enqueue_task(description="", source=TaskSource.AGENT_REQUIREMENTS)

        # Assert
        assert task is not None
        assert task.summary == "Task"

        # Verify persisted
        retrieved = await db.get_task(task.id)
        assert retrieved is not None
        assert retrieved.summary == "Task"

    @pytest.mark.asyncio
    async def test_enqueue_task_summary_whitespace_description(self) -> None:
        """Test whitespace-only description generates 'Task'."""
        # Setup
        db = Database(Path(":memory:"))
        await db.initialize()
        dependency_resolver = DependencyResolver(db)
        priority_calculator = PriorityCalculator(dependency_resolver)
        service = TaskQueueService(db, dependency_resolver, priority_calculator)

        # Act - whitespace-only description should generate "Task"
        task = await service.enqueue_task(description="   ", source=TaskSource.HUMAN)

        # Assert
        assert task is not None
        assert task.summary == "Task"

        # Verify persisted
        retrieved = await db.get_task(task.id)
        assert retrieved is not None
        assert retrieved.summary == "Task"

    @pytest.mark.asyncio
    async def test_enqueue_task_summary_auto_gen_human(self) -> None:
        """Test auto-generated summary for human tasks includes prefix."""
        # Setup
        db = Database(Path(":memory:"))
        await db.initialize()
        dependency_resolver = DependencyResolver(db)
        priority_calculator = PriorityCalculator(dependency_resolver)
        service = TaskQueueService(db, dependency_resolver, priority_calculator)

        # Act
        task = await service.enqueue_task(description="Do something", source=TaskSource.HUMAN)

        # Assert
        assert task is not None
        assert task.summary == "User Prompt: Do something"

        # Verify persisted
        retrieved = await db.get_task(task.id)
        assert retrieved is not None
        assert retrieved.summary == "User Prompt: Do something"

    @pytest.mark.asyncio
    async def test_enqueue_task_summary_auto_gen_agent(self) -> None:
        """Test auto-generated summary for agent tasks has no prefix."""
        # Setup
        db = Database(Path(":memory:"))
        await db.initialize()
        dependency_resolver = DependencyResolver(db)
        priority_calculator = PriorityCalculator(dependency_resolver)
        service = TaskQueueService(db, dependency_resolver, priority_calculator)

        # Act
        task = await service.enqueue_task(
            description="Do something", source=TaskSource.AGENT_REQUIREMENTS
        )

        # Assert
        assert task is not None
        assert task.summary == "Do something"  # No prefix for agent tasks

        # Verify persisted
        retrieved = await db.get_task(task.id)
        assert retrieved is not None
        assert retrieved.summary == "Do something"
