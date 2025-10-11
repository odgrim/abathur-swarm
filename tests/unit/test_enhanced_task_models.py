"""Unit tests for enhanced task queue models."""

from datetime import datetime, timezone
from uuid import uuid4

import pytest
from abathur.domain.models import (
    DependencyType,
    Task,
    TaskDependency,
    TaskSource,
    TaskStatus,
)
from pydantic import ValidationError


class TestTaskStatus:
    """Test enhanced TaskStatus enum."""

    def test_all_statuses_defined(self) -> None:
        """All required statuses are defined."""
        assert TaskStatus.PENDING.value == "pending"
        assert TaskStatus.BLOCKED.value == "blocked"
        assert TaskStatus.READY.value == "ready"
        assert TaskStatus.RUNNING.value == "running"
        assert TaskStatus.COMPLETED.value == "completed"
        assert TaskStatus.FAILED.value == "failed"
        assert TaskStatus.CANCELLED.value == "cancelled"

    def test_status_count(self) -> None:
        """Verify we have exactly 7 statuses."""
        assert len(TaskStatus) == 7


class TestTaskSource:
    """Test TaskSource enum."""

    def test_all_sources_defined(self) -> None:
        """All required sources are defined."""
        assert TaskSource.HUMAN.value == "human"
        assert TaskSource.AGENT_REQUIREMENTS.value == "agent_requirements"
        assert TaskSource.AGENT_PLANNER.value == "agent_planner"
        assert TaskSource.AGENT_IMPLEMENTATION.value == "agent_implementation"

    def test_source_count(self) -> None:
        """Verify we have exactly 4 sources."""
        assert len(TaskSource) == 4


class TestDependencyType:
    """Test DependencyType enum."""

    def test_all_types_defined(self) -> None:
        """All required dependency types are defined."""
        assert DependencyType.SEQUENTIAL.value == "sequential"
        assert DependencyType.PARALLEL.value == "parallel"

    def test_type_count(self) -> None:
        """Verify we have exactly 2 dependency types."""
        assert len(DependencyType) == 2


class TestTaskModel:
    """Test enhanced Task model."""

    def test_task_with_defaults(self) -> None:
        """Task can be created with minimal fields."""
        task = Task(prompt="Test task")
        assert task.id is not None
        assert task.prompt == "Test task"
        assert task.source == TaskSource.HUMAN
        assert task.dependency_type == DependencyType.SEQUENTIAL
        assert task.calculated_priority == 5.0
        assert task.deadline is None
        assert task.estimated_duration_seconds is None
        assert task.dependency_depth == 0
        assert task.status == TaskStatus.PENDING
        assert task.priority == 5

    def test_task_with_source(self) -> None:
        """Task can be created with specific source."""
        task = Task(prompt="Agent task", source=TaskSource.AGENT_PLANNER)
        assert task.source == TaskSource.AGENT_PLANNER

    def test_task_with_priority_fields(self) -> None:
        """Task can be created with priority calculation fields."""
        deadline = datetime.now(timezone.utc)
        task = Task(
            prompt="Urgent task",
            calculated_priority=8.5,
            deadline=deadline,
            estimated_duration_seconds=3600,
            dependency_depth=2,
        )
        assert task.calculated_priority == 8.5
        assert task.deadline == deadline
        assert task.estimated_duration_seconds == 3600
        assert task.dependency_depth == 2

    def test_task_with_dependencies(self) -> None:
        """Task can be created with dependencies."""
        dep1 = uuid4()
        dep2 = uuid4()
        task = Task(
            prompt="Dependent task",
            dependencies=[dep1, dep2],
            dependency_type=DependencyType.PARALLEL,
        )
        assert len(task.dependencies) == 2
        assert dep1 in task.dependencies
        assert dep2 in task.dependencies
        assert task.dependency_type == DependencyType.PARALLEL

    def test_task_json_serialization(self) -> None:
        """Task can be serialized to JSON."""
        task = Task(
            prompt="Serializable task",
            source=TaskSource.AGENT_IMPLEMENTATION,
            calculated_priority=7.5,
            dependency_depth=1,
        )
        # Verify model_dump works
        data = task.model_dump()
        assert data["prompt"] == "Serializable task"
        assert data["source"] == "agent_implementation"
        assert data["calculated_priority"] == 7.5
        assert data["dependency_depth"] == 1


class TestTaskDependencyModel:
    """Test TaskDependency model."""

    def test_dependency_creation(self) -> None:
        """TaskDependency can be created."""
        dep_id = uuid4()
        prereq_id = uuid4()

        dependency = TaskDependency(
            dependent_task_id=dep_id,
            prerequisite_task_id=prereq_id,
            dependency_type=DependencyType.SEQUENTIAL,
        )

        assert dependency.id is not None
        assert dependency.dependent_task_id == dep_id
        assert dependency.prerequisite_task_id == prereq_id
        assert dependency.dependency_type == DependencyType.SEQUENTIAL
        assert dependency.resolved_at is None
        assert dependency.created_at is not None

    def test_dependency_resolution(self) -> None:
        """TaskDependency can be resolved."""
        dependency = TaskDependency(
            dependent_task_id=uuid4(),
            prerequisite_task_id=uuid4(),
            dependency_type=DependencyType.PARALLEL,
        )

        resolved_time = datetime.now(timezone.utc)
        dependency.resolved_at = resolved_time

        assert dependency.resolved_at == resolved_time

    def test_dependency_types(self) -> None:
        """TaskDependency supports both dependency types."""
        seq_dep = TaskDependency(
            dependent_task_id=uuid4(),
            prerequisite_task_id=uuid4(),
            dependency_type=DependencyType.SEQUENTIAL,
        )
        assert seq_dep.dependency_type == DependencyType.SEQUENTIAL

        par_dep = TaskDependency(
            dependent_task_id=uuid4(),
            prerequisite_task_id=uuid4(),
            dependency_type=DependencyType.PARALLEL,
        )
        assert par_dep.dependency_type == DependencyType.PARALLEL

    def test_dependency_json_serialization(self) -> None:
        """TaskDependency can be serialized to JSON."""
        dependency = TaskDependency(
            dependent_task_id=uuid4(),
            prerequisite_task_id=uuid4(),
            dependency_type=DependencyType.SEQUENTIAL,
        )
        data = dependency.model_dump()
        assert "dependent_task_id" in data
        assert "prerequisite_task_id" in data
        assert data["dependency_type"] == "sequential"
        assert "created_at" in data


class TestTaskModelValidation:
    """Test Task model validation rules."""

    def test_priority_bounds(self) -> None:
        """Task priority must be between 0 and 10."""
        # Valid priorities
        task1 = Task(prompt="Test", priority=0)
        assert task1.priority == 0

        task2 = Task(prompt="Test", priority=10)
        assert task2.priority == 10

        # Invalid priorities should raise validation error
        with pytest.raises(ValidationError):
            Task(prompt="Test", priority=-1)

        with pytest.raises(ValidationError):
            Task(prompt="Test", priority=11)

    def test_calculated_priority_non_negative(self) -> None:
        """Calculated priority can be any non-negative float."""
        task = Task(prompt="Test", calculated_priority=0.0)
        assert task.calculated_priority == 0.0

        task2 = Task(prompt="Test", calculated_priority=100.5)
        assert task2.calculated_priority == 100.5

    def test_dependency_depth_non_negative(self) -> None:
        """Dependency depth must be non-negative."""
        task = Task(prompt="Test", dependency_depth=0)
        assert task.dependency_depth == 0

        task2 = Task(prompt="Test", dependency_depth=10)
        assert task2.dependency_depth == 10


class TestModelDefaults:
    """Test that model defaults match specifications."""

    def test_task_defaults(self) -> None:
        """Verify all Task field defaults."""
        task = Task(prompt="Test")

        # Core fields
        assert task.agent_type == "general"
        assert task.priority == 5
        assert task.status == TaskStatus.PENDING
        assert task.input_data == {}
        assert task.result_data is None
        assert task.retry_count == 0
        assert task.max_retries == 3
        assert task.max_execution_timeout_seconds == 3600

        # Enhanced fields
        assert task.source == TaskSource.HUMAN
        assert task.dependency_type == DependencyType.SEQUENTIAL
        assert task.calculated_priority == 5.0
        assert task.deadline is None
        assert task.estimated_duration_seconds is None
        assert task.dependency_depth == 0
        assert task.dependencies == []

    def test_task_dependency_defaults(self) -> None:
        """Verify TaskDependency field defaults."""
        dep = TaskDependency(
            dependent_task_id=uuid4(),
            prerequisite_task_id=uuid4(),
            dependency_type=DependencyType.SEQUENTIAL,
        )
        assert dep.resolved_at is None
        assert dep.created_at is not None
        assert dep.id is not None
