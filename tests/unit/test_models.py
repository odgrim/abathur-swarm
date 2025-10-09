"""Unit tests for domain models."""

from datetime import datetime, timezone
from uuid import uuid4

import pytest
from abathur.domain.models import Agent, AgentState, Task, TaskStatus


class TestTask:
    """Tests for Task model."""

    def test_create_task_with_defaults(self) -> None:
        """Test creating a task with default values."""
        task = Task(
            template_name="test-template",
            input_data={"key": "value"},
        )

        assert task.id is not None
        assert task.template_name == "test-template"
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
            template_name="test-template",
            input_data={"key": "value"},
            priority=8,
        )

        assert task.priority == 8

    def test_task_priority_validation(self) -> None:
        """Test that task priority is validated."""
        with pytest.raises(ValueError):
            Task(
                template_name="test-template",
                input_data={"key": "value"},
                priority=11,  # Invalid: > 10
            )

        with pytest.raises(ValueError):
            Task(
                template_name="test-template",
                input_data={"key": "value"},
                priority=-1,  # Invalid: < 0
            )

    def test_task_with_parent(self) -> None:
        """Test creating a child task with parent reference."""
        parent_id = uuid4()
        task = Task(
            template_name="child-template",
            input_data={},
            parent_task_id=parent_id,
        )

        assert task.parent_task_id == parent_id

    def test_task_with_dependencies(self) -> None:
        """Test creating a task with dependencies."""
        dep1 = uuid4()
        dep2 = uuid4()
        task = Task(
            template_name="test-template",
            input_data={},
            dependencies=[dep1, dep2],
        )

        assert len(task.dependencies) == 2
        assert dep1 in task.dependencies
        assert dep2 in task.dependencies


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
        assert agent.model == "claude-sonnet-4-20250514"
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
