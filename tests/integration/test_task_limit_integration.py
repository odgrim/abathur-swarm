"""Integration tests for SwarmOrchestrator task_limit enforcement.

Tests the fix for race condition in task_limit enforcement:
- Realistic workload with slow concurrent tasks (no over-spawning)
- Performance regression testing (fast tasks)
- Backward compatibility (task_limit=None)
- Edge cases (limit == concurrent)

These tests validate that the counter increment occurs BEFORE task spawning,
preventing race conditions where more tasks are spawned than the limit allows.
"""

import asyncio
import time
from collections.abc import AsyncGenerator
from pathlib import Path
from uuid import UUID

import pytest
from abathur.application.agent_executor import AgentExecutor
from abathur.application.swarm_orchestrator import SwarmOrchestrator
from abathur.domain.models import Result, Task, TaskStatus
from abathur.infrastructure.database import Database
from abathur.services.task_queue_service import TaskQueueService


@pytest.fixture
async def memory_db() -> AsyncGenerator[Database, None]:
    """Create in-memory database for fast integration tests."""
    db = Database(Path(":memory:"))
    await db.initialize()
    yield db
    await db.close()


@pytest.fixture
async def task_queue_service(memory_db: Database) -> TaskQueueService:
    """Create TaskQueueService with in-memory database and required dependencies."""
    from abathur.services.dependency_resolver import DependencyResolver
    from abathur.services.priority_calculator import PriorityCalculator

    dependency_resolver = DependencyResolver(memory_db)
    priority_calculator = PriorityCalculator(dependency_resolver)
    return TaskQueueService(memory_db, dependency_resolver, priority_calculator)


class MockAgentExecutor(AgentExecutor):
    """Mock agent executor for integration testing.

    Allows configurable task execution duration and success/failure simulation.
    """

    def __init__(self, execution_duration: float = 0.01, fail_tasks: set[UUID] | None = None):
        """Initialize mock executor.

        Args:
            execution_duration: How long each task execution should take (seconds)
            fail_tasks: Set of task IDs that should fail (default: all succeed)
        """
        self.execution_duration = execution_duration
        self.fail_tasks = fail_tasks or set()
        self.executed_tasks: list[UUID] = []

    async def execute_task(self, task: Task) -> Result:
        """Execute task with configurable duration and success/failure."""
        # Simulate task execution time
        await asyncio.sleep(self.execution_duration)

        # Track execution
        self.executed_tasks.append(task.id)

        # Determine success/failure
        success = task.id not in self.fail_tasks

        return Result(
            task_id=task.id,
            agent_id=UUID(int=0),
            success=success,
            error=None if success else "Mock task failure",
            data={"executed": True} if success else None,
            execution_time_seconds=self.execution_duration,
        )


@pytest.fixture
def create_test_task():
    """Factory fixture for creating test tasks."""

    def _create(task_id: str, prompt: str = "Test task") -> Task:
        return Task(
            id=UUID(task_id) if len(task_id) == 36 else UUID(int=int(task_id)),
            prompt=prompt,
            agent_type="test-agent",
            status=TaskStatus.READY,
        )

    return _create


# ==============================================================================
# Scenario 1: Realistic Workload Test (Slow Concurrent Tasks)
# ==============================================================================


@pytest.mark.asyncio
async def test_realistic_workload_no_over_spawning(
    memory_db: Database,
    task_queue_service: TaskQueueService,
    create_test_task,
):
    """Verify task_limit prevents over-spawning with slow concurrent tasks.

    This is the CRITICAL test that validates the race condition fix.

    Setup:
        - task_limit = 10
        - max_concurrent_agents = 20
        - Task duration: 5 seconds each
        - 100 tasks queued (more than limit)

    Expected behavior:
        - Exactly 10 tasks spawned (not 20, not 100)
        - No over-spawning despite high concurrency
        - All spawned tasks complete successfully

    Before fix:
        - Race condition allowed up to 20 tasks to spawn
        - Counter incremented in finally block (after task completion)

    After fix:
        - Counter increments at line 140 (before asyncio.create_task)
        - Limit check at line 124 sees accurate count
        - Exactly 10 tasks spawned
    """
    # Arrange: Create 100 tasks (more than limit)
    tasks = [create_test_task(str(i), f"Slow task {i}") for i in range(100)]

    # Enqueue all tasks
    for task in tasks:
        await task_queue_service.enqueue_task(
            description=task.prompt,
            agent_type=task.agent_type,
            source="human",  # type: ignore
        )

    # Create mock executor with 0.5-second execution time (simulates slow tasks)
    # Reduced from 5.0s for faster test execution while still testing concurrent behavior
    mock_executor = MockAgentExecutor(execution_duration=0.5)

    # Create orchestrator with high concurrency but low limit
    orchestrator = SwarmOrchestrator(
        task_queue_service=task_queue_service,
        agent_executor=mock_executor,
        max_concurrent_agents=20,  # High concurrency
        poll_interval=0.1,  # Fast polling for test speed
    )

    # Act: Run swarm with limit=10
    start_time = time.time()
    results = await orchestrator.start_swarm(task_limit=10)
    elapsed_time = time.time() - start_time

    # Assert: Exactly 10 tasks executed (critical validation)
    assert len(results) == 10, f"Expected exactly 10 tasks, but got {len(results)}"
    assert (
        len(mock_executor.executed_tasks) == 10
    ), f"Expected executor to run exactly 10 tasks, but it ran {len(mock_executor.executed_tasks)}"

    # Assert: All tasks succeeded
    assert all(r.success for r in results), "All tasks should succeed"

    # Assert: Tasks executed concurrently (not sequentially)
    # 10 tasks * 0.5s / 20 concurrent = ~0.25s minimum
    # Allow 20x overhead for test environment = 5s max
    assert elapsed_time < 5.0, (
        f"Tasks should execute concurrently, but took {elapsed_time:.2f}s "
        f"(expected <5s for 10 tasks @ 0.5s each with 20 concurrent slots)"
    )


# ==============================================================================
# Scenario 2: Performance Regression Test (Fast Tasks)
# ==============================================================================


@pytest.mark.asyncio
async def test_performance_no_regression(
    memory_db: Database,
    task_queue_service: TaskQueueService,
    create_test_task,
):
    """Verify counter increment location change has no performance impact.

    Setup:
        - task_limit = 100
        - max_concurrent_agents = 20
        - Task duration: 10ms each (fast tasks)

    Expected behavior:
        - All 100 tasks execute successfully
        - No performance regression from counter location change
        - Execution time within expected bounds

    Performance baseline:
        - 100 tasks * 0.01s / 20 concurrent = ~0.05s minimum
        - Allow 20x overhead for test environment = 1.0s max
    """
    # Arrange: Create 100 fast tasks
    tasks = [create_test_task(str(i), f"Fast task {i}") for i in range(100)]

    # Enqueue all tasks
    for task in tasks:
        await task_queue_service.enqueue_task(
            description=task.prompt,
            agent_type=task.agent_type,
            source="human",  # type: ignore
        )

    # Create mock executor with fast execution (10ms per task)
    mock_executor = MockAgentExecutor(execution_duration=0.01)

    # Create orchestrator
    orchestrator = SwarmOrchestrator(
        task_queue_service=task_queue_service,
        agent_executor=mock_executor,
        max_concurrent_agents=20,
        poll_interval=0.01,  # Fast polling
    )

    # Act: Run swarm with limit=100
    start_time = time.time()
    results = await orchestrator.start_swarm(task_limit=100)
    elapsed_time = time.time() - start_time

    # Assert: All 100 tasks executed
    assert len(results) == 100, f"Expected 100 tasks, got {len(results)}"
    assert len(mock_executor.executed_tasks) == 100

    # Assert: All tasks succeeded
    assert all(r.success for r in results), "All tasks should succeed"

    # Assert: No performance regression
    # 100 tasks * 0.01s / 20 concurrent = ~0.05s minimum
    # Allow generous 20x overhead for test environment
    assert elapsed_time < 1.0, (
        f"Performance regression detected: took {elapsed_time:.2f}s "
        f"(expected <1.0s for 100 fast tasks)"
    )


# ==============================================================================
# Scenario 3: Backward Compatibility Test (task_limit=None)
# ==============================================================================


@pytest.mark.asyncio
async def test_backward_compatibility_large_limit(
    memory_db: Database,
    task_queue_service: TaskQueueService,
    create_test_task,
):
    """Verify large task_limit processes all tasks (effectively no limit).

    Setup:
        - task_limit = 1000 (much larger than task count)
        - max_concurrent_agents = 10
        - 50 tasks total

    Expected behavior:
        - All 50 tasks execute (limit not reached)
        - Backward compatible with task_limit=None behavior
        - No over-spawning occurs

    This validates that high task limits work correctly and don't over-spawn.
    Note: task_limit=None runs indefinitely, so we test with a large limit instead.
    """
    # Arrange: Create 50 tasks
    num_tasks = 50
    tasks = [create_test_task(str(i), f"Task {i}") for i in range(num_tasks)]

    # Enqueue all tasks
    for task in tasks:
        await task_queue_service.enqueue_task(
            description=task.prompt,
            agent_type=task.agent_type,
            source="human",  # type: ignore
        )

    # Create mock executor
    mock_executor = MockAgentExecutor(execution_duration=0.01)

    # Create orchestrator
    orchestrator = SwarmOrchestrator(
        task_queue_service=task_queue_service,
        agent_executor=mock_executor,
        max_concurrent_agents=10,
        poll_interval=0.01,
    )

    # Act: Run swarm with large limit (1000 >> 50 tasks)
    results = await orchestrator.start_swarm(task_limit=1000)

    # Assert: All 50 tasks should execute (limit not reached)
    assert len(results) == num_tasks, (
        f"Expected all {num_tasks} tasks to execute with large limit (1000), "
        f"but only {len(results)} executed"
    )

    # Assert: All tasks succeeded
    assert all(r.success for r in results), "All tasks should succeed"


# ==============================================================================
# Scenario 4: Edge Case - Limit Equals Concurrent
# ==============================================================================


@pytest.mark.asyncio
async def test_edge_case_limit_equals_concurrent(
    memory_db: Database,
    task_queue_service: TaskQueueService,
    create_test_task,
):
    """Verify behavior when task_limit == max_concurrent_agents.

    Setup:
        - task_limit = 5
        - max_concurrent_agents = 5
        - 20 tasks total

    Expected behavior:
        - Exactly 5 tasks execute
        - No over-spawning when limit equals concurrency
        - Remaining 15 tasks stay in queue

    This edge case validates the fix works when limit == concurrency.
    """
    # Arrange: Create 20 tasks (more than limit)
    num_tasks = 20
    limit = 5
    tasks = [create_test_task(str(i), f"Task {i}") for i in range(num_tasks)]

    # Enqueue all tasks
    for task in tasks:
        await task_queue_service.enqueue_task(
            description=task.prompt,
            agent_type=task.agent_type,
            source="human",  # type: ignore
        )

    # Create mock executor
    mock_executor = MockAgentExecutor(execution_duration=0.1)

    # Create orchestrator where limit == max_concurrent
    orchestrator = SwarmOrchestrator(
        task_queue_service=task_queue_service,
        agent_executor=mock_executor,
        max_concurrent_agents=limit,  # Same as limit
        poll_interval=0.01,
    )

    # Act: Run swarm with limit=5
    results = await orchestrator.start_swarm(task_limit=limit)

    # Assert: Exactly 5 tasks executed
    assert len(results) == limit, (
        f"Expected exactly {limit} tasks when limit == max_concurrent, " f"but got {len(results)}"
    )

    # Assert: All tasks succeeded
    assert all(r.success for r in results), "All tasks should succeed"


# ==============================================================================
# Scenario 5: Task Limit with Failed Tasks
# ==============================================================================


@pytest.mark.asyncio
async def test_task_limit_with_failures(
    memory_db: Database,
    task_queue_service: TaskQueueService,
    create_test_task,
):
    """Verify failed tasks count toward task_limit.

    Setup:
        - task_limit = 10
        - max_concurrent_agents = 5
        - 20 tasks total
        - Tasks 3, 5, 7 configured to fail

    Expected behavior:
        - Exactly 10 tasks execute (including failures)
        - Failed tasks count toward limit
        - 7 tasks succeed, 3 tasks fail

    This validates that task_limit counts ALL spawned tasks, not just successful ones.
    """
    # Arrange: Create 20 tasks
    num_tasks = 20
    limit = 10
    tasks = [create_test_task(str(i), f"Task {i}") for i in range(num_tasks)]

    # Enqueue all tasks
    task_ids = []
    for task in tasks:
        queued_task = await task_queue_service.enqueue_task(
            description=task.prompt,
            agent_type=task.agent_type,
            source="human",  # type: ignore
        )
        task_ids.append(queued_task.id)

    # Configure tasks 3, 5, 7 to fail (indices in executed order)
    # We'll fail every 3rd task
    fail_task_ids = {task_ids[i] for i in [2, 4, 6]}  # 0-indexed, so positions 3, 5, 7

    # Create mock executor that fails specific tasks
    mock_executor = MockAgentExecutor(
        execution_duration=0.01,
        fail_tasks=fail_task_ids,
    )

    # Create orchestrator
    orchestrator = SwarmOrchestrator(
        task_queue_service=task_queue_service,
        agent_executor=mock_executor,
        max_concurrent_agents=5,
        poll_interval=0.01,
    )

    # Act: Run swarm with limit=10
    results = await orchestrator.start_swarm(task_limit=limit)

    # Assert: Exactly 10 tasks executed (including failures)
    assert (
        len(results) == limit
    ), f"Expected exactly {limit} tasks (including failures), got {len(results)}"

    # Assert: Correct number of successes and failures
    successes = [r for r in results if r.success]
    failures = [r for r in results if not r.success]

    assert len(failures) == 3, f"Expected 3 failures, got {len(failures)}"
    assert len(successes) == 7, f"Expected 7 successes, got {len(successes)}"


# ==============================================================================
# Scenario 6: Task Limit Zero (Immediate Exit)
# ==============================================================================


@pytest.mark.asyncio
async def test_task_limit_zero(
    memory_db: Database,
    task_queue_service: TaskQueueService,
    create_test_task,
):
    """Verify task_limit=0 exits immediately without processing any tasks.

    Setup:
        - task_limit = 0
        - 10 tasks queued

    Expected behavior:
        - Zero tasks execute
        - Swarm exits immediately
        - All tasks remain in READY state
    """
    # Arrange: Create 10 tasks
    num_tasks = 10
    tasks = [create_test_task(str(i), f"Task {i}") for i in range(num_tasks)]

    # Enqueue all tasks
    for task in tasks:
        await task_queue_service.enqueue_task(
            description=task.prompt,
            agent_type=task.agent_type,
            source="human",  # type: ignore
        )

    # Create mock executor
    mock_executor = MockAgentExecutor(execution_duration=0.01)

    # Create orchestrator
    orchestrator = SwarmOrchestrator(
        task_queue_service=task_queue_service,
        agent_executor=mock_executor,
        max_concurrent_agents=5,
        poll_interval=0.01,
    )

    # Act: Run swarm with limit=0
    results = await orchestrator.start_swarm(task_limit=0)

    # Assert: Zero tasks executed
    assert len(results) == 0, f"Expected 0 tasks with limit=0, got {len(results)}"
    assert len(mock_executor.executed_tasks) == 0, "No tasks should be executed"
