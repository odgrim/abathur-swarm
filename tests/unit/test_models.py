"""Unit tests for domain models."""

from datetime import datetime, timezone
from uuid import uuid4

import pytest
from pydantic import ValidationError
from abathur.domain.models import Agent, AgentState, Task, TaskStatus


class TestTask:
    """Tests for Task model."""

    def test_create_task_with_defaults(self) -> None:
        """Test creating a task with default values."""
        task = Task(
            prompt="Test task prompt",
            input_data={"key": "value"},
        )

        assert task.id is not None
        assert task.prompt == "Test task prompt"
        assert task.agent_type == "requirements-gatherer"  # Default agent type
        assert task.priority == 5
        assert task.status == TaskStatus.PENDING
        assert task.input_data == {"key": "value"}
        assert task.result_data is None
        assert task.error_message is None
        assert task.retry_count == 0
        assert task.max_retries == 3
        assert isinstance(task.submitted_at, datetime)

    def test_create_task_with_custom_priority(self) -> None:
        """Test creating a task with custom priority."""
        task = Task(
            prompt="Test task with custom priority",
            priority=8,
        )

        assert task.priority == 8

    def test_task_priority_validation(self) -> None:
        """Test that task priority is validated."""
        with pytest.raises(ValueError):
            Task(
                prompt="Test task",
                priority=11,  # Invalid: > 10
            )

        with pytest.raises(ValueError):
            Task(
                prompt="Test task",
                priority=-1,  # Invalid: < 0
            )

    def test_task_with_parent(self) -> None:
        """Test creating a child task with parent reference."""
        parent_id = uuid4()
        task = Task(
            prompt="Child task prompt",
            parent_task_id=parent_id,
        )

        assert task.parent_task_id == parent_id

    def test_task_with_dependencies(self) -> None:
        """Test creating a task with dependencies."""
        dep1 = uuid4()
        dep2 = uuid4()
        task = Task(
            prompt="Task with dependencies",
            dependencies=[dep1, dep2],
        )

        assert len(task.dependencies) == 2
        assert dep1 in task.dependencies
        assert dep2 in task.dependencies

    # ===== Summary Field Tests =====

    def test_task_with_summary(self) -> None:
        """Test Task model accepts summary field with valid data."""
        # Arrange
        valid_summary = "This is a test task summary"

        # Act
        task = Task(
            prompt="Test task with summary",
            summary=valid_summary,
        )

        # Assert
        assert task.summary == valid_summary
        assert task.prompt == "Test task with summary"

        # Verify serialization includes summary
        serialized = task.model_dump()
        assert "summary" in serialized
        assert serialized["summary"] == valid_summary

    def test_task_without_summary(self) -> None:
        """Test Task model works with summary=None (backward compatibility)."""
        # Arrange & Act - create task without providing summary
        task = Task(
            prompt="Test task without summary",
        )

        # Assert - summary defaults to None
        assert task.summary is None
        assert task.prompt == "Test task without summary"

        # Verify serialization includes summary field as None
        serialized = task.model_dump()
        assert "summary" in serialized
        assert serialized["summary"] is None

    def test_task_summary_none_explicit(self) -> None:
        """Test Task model accepts explicit summary=None."""
        # Arrange & Act - explicitly set summary to None
        task = Task(
            prompt="Test task with explicit None",
            summary=None,
        )

        # Assert
        assert task.summary is None

        # Verify no validation error
        serialized = task.model_dump()
        assert serialized["summary"] is None

    def test_task_summary_max_length_at_boundary(self) -> None:
        """Test summary field validation at exact boundary (max_length=500)."""
        # Arrange - create summary with exactly 500 characters
        summary_500_chars = "x" * 500

        # Act
        task = Task(
            prompt="Test task with 500 char summary",
            summary=summary_500_chars,
        )

        # Assert - should accept exactly 500 characters
        assert task.summary == summary_500_chars
        assert len(task.summary) == 500

    def test_task_summary_exceeds_max_length(self) -> None:
        """Test Pydantic enforces max_length=500 constraint on summary field."""
        # Arrange - create summary with 501 characters (exceeds limit)
        summary_501_chars = "x" * 501

        # Act & Assert - should raise ValidationError
        with pytest.raises(ValidationError) as exc_info:
            Task(
                prompt="Test task with too long summary",
                summary=summary_501_chars,
            )

        # Verify error message mentions max_length constraint
        error_str = str(exc_info.value).lower()
        assert "max_length" in error_str or "maximum" in error_str or "500" in error_str

    def test_task_summary_empty_string(self) -> None:
        """Test summary field accepts empty string."""
        # Arrange & Act
        task = Task(
            prompt="Test task with empty summary",
            summary="",
        )

        # Assert - empty string is valid
        assert task.summary == ""
        assert task.summary is not None  # Not None, but empty string

    def test_task_summary_serialization_includes_all_fields(self) -> None:
        """Test Task model serializes to dict with summary field present."""
        # Arrange
        task_with_summary = Task(
            prompt="Task with summary",
            summary="Test summary",
            agent_type="test-agent",
            priority=7,
        )

        # Act
        serialized = task_with_summary.model_dump()

        # Assert - verify summary is in serialized output
        assert "summary" in serialized
        assert serialized["summary"] == "Test summary"
        assert serialized["prompt"] == "Task with summary"
        assert serialized["agent_type"] == "test-agent"
        assert serialized["priority"] == 7

    def test_task_summary_with_special_characters(self) -> None:
        """Test summary field accepts special characters and unicode."""
        # Arrange - test with unicode, emojis, and special chars
        special_summary = "Task: Fix bug 🐛 - Update API → v2.0 (高优先级)"

        # Act
        task = Task(
            prompt="Test task with special chars",
            summary=special_summary,
        )

        # Assert - special characters should be preserved
        assert task.summary == special_summary
        assert "🐛" in task.summary
        assert "→" in task.summary
        assert "高优先级" in task.summary

    def test_task_summary_whitespace_handling(self) -> None:
        """Test summary field preserves whitespace."""
        # Arrange - test with leading/trailing/internal whitespace
        summary_with_whitespace = "  Task summary with  spaces  "

        # Act
        task = Task(
            prompt="Test task with whitespace",
            summary=summary_with_whitespace,
        )

        # Assert - whitespace should be preserved
        assert task.summary == summary_with_whitespace

    def test_task_summary_json_encoding(self) -> None:
        """Test Task with summary can be JSON-encoded correctly."""
        # Arrange
        task = Task(
            prompt="Test JSON encoding",
            summary="Summary for JSON test",
        )

        # Act - use model_dump_json for JSON serialization
        json_str = task.model_dump_json()

        # Assert - should contain summary field
        assert "summary" in json_str
        assert "Summary for JSON test" in json_str

    def test_task_summary_multiple_tasks_independence(self) -> None:
        """Test summary field is independent across multiple Task instances."""
        # Arrange & Act - create multiple tasks with different summaries
        task1 = Task(prompt="Task 1", summary="Summary 1")
        task2 = Task(prompt="Task 2", summary="Summary 2")
        task3 = Task(prompt="Task 3")  # No summary

        # Assert - each task has correct summary
        assert task1.summary == "Summary 1"
        assert task2.summary == "Summary 2"
        assert task3.summary is None

        # Verify no cross-contamination
        assert task1.summary != task2.summary
        assert task1.summary != task3.summary


class TestAgent:
    """Tests for Agent model."""

    def test_create_agent_with_defaults(self) -> None:
        """Test creating an agent with default values."""
        task_id = uuid4()
        agent = Agent(
            name="test-agent",
            specialization="testing",
            task_id=task_id,
        )

        assert agent.id is not None
        assert agent.name == "test-agent"
        assert agent.specialization == "testing"
        assert agent.task_id == task_id
        assert agent.state == AgentState.SPAWNING
        assert agent.model == "claude-sonnet-4-5-20250929"
        assert isinstance(agent.spawned_at, datetime)
        assert agent.terminated_at is None
        assert agent.resource_usage == {}

    def test_agent_state_transitions(self) -> None:
        """Test agent state transitions."""
        agent = Agent(
            name="test-agent",
            specialization="testing",
            task_id=uuid4(),
        )

        # Initial state
        assert agent.state == AgentState.SPAWNING

        # Update to idle
        agent.state = AgentState.IDLE
        assert agent.state == AgentState.IDLE

        # Update to busy
        agent.state = AgentState.BUSY
        assert agent.state == AgentState.BUSY

        # Update to terminating
        agent.state = AgentState.TERMINATING
        assert agent.state == AgentState.TERMINATING

        # Update to terminated
        agent.state = AgentState.TERMINATED
        agent.terminated_at = datetime.now(timezone.utc)
        assert agent.state == AgentState.TERMINATED
        assert agent.terminated_at is not None

    def test_agent_with_custom_model(self) -> None:
        """Test creating an agent with custom model."""
        agent = Agent(
            name="test-agent",
            specialization="testing",
            task_id=uuid4(),
            model="claude-opus-4-20250514",
        )

        assert agent.model == "claude-opus-4-20250514"

    def test_agent_with_resource_usage(self) -> None:
        """Test agent with resource usage tracking."""
        agent = Agent(
            name="test-agent",
            specialization="testing",
            task_id=uuid4(),
            resource_usage={
                "memory_mb": 256,
                "tokens_used": 1000,
                "execution_time_seconds": 12.5,
            },
        )

        assert agent.resource_usage["memory_mb"] == 256
        assert agent.resource_usage["tokens_used"] == 1000
        assert agent.resource_usage["execution_time_seconds"] == 12.5
