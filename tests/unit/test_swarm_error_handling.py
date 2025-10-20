"""Tests for swarm orchestrator error handling."""

from unittest.mock import AsyncMock
from uuid import uuid4

import pytest
from abathur.application.swarm_orchestrator import SwarmOrchestrator
from abathur.domain.models import Result, Task, TaskStatus


class TestSwarmErrorHandling:
    """Test error handling in swarm orchestrator."""

    @pytest.mark.asyncio
    async def test_task_marked_failed_on_error(self) -> None:
        """Test that tasks are marked as FAILED when execution fails."""
        # Create mock dependencies
        task_queue_service = AsyncMock()
        agent_executor = AsyncMock()

        # Create a test task
        task_id = uuid4()
        task = Task(
            id=task_id,
            prompt="Test task",
            agent_type="general",
            priority=5,
            status=TaskStatus.RUNNING,
            max_retries=0,  # No retries to simplify test
        )

        # Mock agent executor to return a failed result
        failed_result = Result(
            task_id=task_id,
            agent_id=uuid4(),
            success=False,
            error="Claude CLI error: test error message",
        )
        agent_executor.execute_task.return_value = failed_result

        # Mock task queue service methods
        task_queue_service.fail_task = AsyncMock()

        # Create orchestrator
        orchestrator = SwarmOrchestrator(
            task_queue_service=task_queue_service,
            agent_executor=agent_executor,
            max_concurrent_agents=1,
        )

        # Execute task
        result = await orchestrator._execute_with_semaphore(task)

        # Verify result
        assert result.success is False
        assert result.error == "Claude CLI error: test error message"

        # Verify fail_task was called with error message
        task_queue_service.fail_task.assert_called_once_with(
            task_id, error_message="Claude CLI error: test error message"
        )

    @pytest.mark.asyncio
    async def test_task_marked_failed_then_retry_on_error_with_retries_available(self) -> None:
        """Test that tasks are marked FAILED first, then retried if retries are available."""
        # Create mock dependencies
        task_queue_service = AsyncMock()
        agent_executor = AsyncMock()

        # Create a test task with retries available
        task_id = uuid4()
        task = Task(
            id=task_id,
            prompt="Test task",
            agent_type="general",
            priority=5,
            status=TaskStatus.RUNNING,
            retry_count=0,
            max_retries=3,  # Retries available
        )

        # Mock agent executor to return a failed result
        failed_result = Result(
            task_id=task_id,
            agent_id=uuid4(),
            success=False,
            error="Claude CLI error: temporary failure",
        )
        agent_executor.execute_task.return_value = failed_result

        # Mock task queue service methods
        task_queue_service.fail_task = AsyncMock()

        # Create orchestrator
        orchestrator = SwarmOrchestrator(
            task_queue_service=task_queue_service,
            agent_executor=agent_executor,
            max_concurrent_agents=1,
        )

        # Execute task
        result = await orchestrator._execute_with_semaphore(task)

        # Verify result
        assert result.success is False

        # Verify fail_task was called with error message
        task_queue_service.fail_task.assert_called_once_with(
            task_id, error_message="Claude CLI error: temporary failure"
        )

    @pytest.mark.asyncio
    async def test_task_marked_completed_on_success(self) -> None:
        """Test that tasks are marked as COMPLETED when execution succeeds."""
        # Create mock dependencies
        task_queue_service = AsyncMock()
        agent_executor = AsyncMock()

        # Create a test task
        task_id = uuid4()
        task = Task(
            id=task_id,
            prompt="Test task",
            agent_type="general",
            priority=5,
            status=TaskStatus.RUNNING,
        )

        # Mock agent executor to return a successful result
        success_result = Result(
            task_id=task_id,
            agent_id=uuid4(),
            success=True,
            data={"output": "Task completed successfully"},
        )
        agent_executor.execute_task.return_value = success_result

        # Mock task queue service methods
        task_queue_service.complete_task = AsyncMock()

        # Create orchestrator
        orchestrator = SwarmOrchestrator(
            task_queue_service=task_queue_service,
            agent_executor=agent_executor,
            max_concurrent_agents=1,
        )

        # Execute task
        result = await orchestrator._execute_with_semaphore(task)

        # Verify result
        assert result.success is True

        # Verify complete_task was called
        task_queue_service.complete_task.assert_called_once_with(task_id)
