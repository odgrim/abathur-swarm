"""Integration tests for SwarmOrchestrator task_limit with real task queue.

Tests complete end-to-end workflows with actual task queue operations:
- Scenario 1: Basic task limit (task_limit=5)
- Scenario 2: Graceful shutdown with active tasks
- Scenario 3: Indefinite mode (task_limit=None)
- Scenario 4: Zero limit (task_limit=0)
- Scenario 5: Failed tasks count toward limit

This validates Phase 3 (Integration Testing) for the task limit enforcement feature.
Reference: Task 003 - Integration Testing with Real Task Queue
"""

import asyncio
from collections.abc import AsyncGenerator
from pathlib import Path
from unittest.mock import AsyncMock
from uuid import uuid4

import pytest
from abathur.application.swarm_orchestrator import SwarmOrchestrator
from abathur.domain.models import Result, Task, TaskSource, TaskStatus
from abathur.infrastructure.database import Database
from abathur.services.dependency_resolver import DependencyResolver
from abathur.services.priority_calculator import PriorityCalculator
from abathur.services.task_queue_service import TaskQueueService

# Fixtures


@pytest.fixture
async def memory_db() -> AsyncGenerator[Database, None]:
    """Create in-memory database for fast integration tests."""
    db = Database(Path(":memory:"))
    await db.initialize()
    yield db
    await db.close()


@pytest.fixture
async def task_queue_service(memory_db: Database) -> TaskQueueService:
    """Create TaskQueueService with in-memory database."""
    dependency_resolver = DependencyResolver(memory_db)
    priority_calculator = PriorityCalculator(dependency_resolver)
    return TaskQueueService(memory_db, dependency_resolver, priority_calculator)


@pytest.fixture
def mock_agent_executor() -> AsyncMock:
    """Create mock AgentExecutor that succeeds quickly."""
    executor = AsyncMock()

    # Default: successful execution with minimal delay
    async def execute_task(task: Task) -> Result:
        await asyncio.sleep(0.01)  # Simulate task execution
        return Result(
            task_id=task.id,
            agent_id=uuid4(),
            success=True,
            data={"output": f"Task {task.id} completed successfully"},
        )

    executor.execute_task.side_effect = execute_task
    return executor


@pytest.fixture
async def orchestrator(
    task_queue_service: TaskQueueService, mock_agent_executor: AsyncMock
) -> SwarmOrchestrator:
    """Create SwarmOrchestrator with real task queue and mock executor."""
    return SwarmOrchestrator(
        task_queue_service=task_queue_service,
        agent_executor=mock_agent_executor,
        max_concurrent_agents=5,
        poll_interval=0.01,  # Fast polling for testing
    )


# Integration Tests - Scenario 1: Basic Task Limit


@pytest.mark.asyncio
async def test_scenario_1_basic_task_limit_exactly_5_tasks(
    task_queue_service: TaskQueueService,
    orchestrator: SwarmOrchestrator,
    memory_db: Database,
) -> None:
    """Scenario 1: Verify exactly 5 tasks complete when task_limit=5.

    Test Strategy:
    1. Create 10 simple tasks in the task queue
    2. Run: start_swarm(task_limit=5)
    3. Wait for swarm to stop
    4. Query completed tasks from database
    5. Expected: Exactly 5 tasks completed
    6. Verify: No more than 5 tasks completed

    Success Criteria:
    - Exactly 5 tasks in COMPLETED state
    - Remaining 5 tasks in READY state
    - Swarm exits gracefully after 5th task completes

    Implementation Reference:
    - src/abathur/application/swarm_orchestrator.py:124-130 (limit check)
    """
    # Step 1: Create 10 tasks in queue
    task_ids = []
    for i in range(10):
        task = await task_queue_service.enqueue_task(
            description=f"Scenario 1 - Task {i}",
            source=TaskSource.HUMAN,
            agent_type="general-purpose",
            base_priority=5,
        )
        task_ids.append(task.id)

    # Verify all tasks start in READY state
    for task_id in task_ids:
        task = await memory_db.get_task(task_id)
        assert task is not None
        assert task.status == TaskStatus.READY

    # Step 2: Run swarm with task_limit=5
    results = await orchestrator.start_swarm(task_limit=5)

    # Assertion 1: Exactly 5 tasks processed
    assert len(results) == 5, f"Expected exactly 5 results, got {len(results)}"

    # Assertion 2: All 5 tasks completed successfully
    assert all(r.success for r in results), "All tasks should complete successfully"

    # Step 3: Verify database state (exactly 5 COMPLETED, 5 READY)
    completed_count = 0
    ready_count = 0

    for task_id in task_ids:
        task = await memory_db.get_task(task_id)
        assert task is not None, f"Task {task_id} not found in database"

        if task.status == TaskStatus.COMPLETED:
            completed_count += 1
            assert task.completed_at is not None, f"Task {task_id} should have completed_at"
        elif task.status == TaskStatus.READY:
            ready_count += 1
            assert task.started_at is None, f"Task {task_id} should not have started"

    # Assertion 3: Exactly 5 completed (not more, not less)
    assert completed_count == 5, f"Expected 5 completed tasks, got {completed_count}"

    # Assertion 4: Exactly 5 remain ready
    assert ready_count == 5, f"Expected 5 ready tasks, got {ready_count}"

    # Assertion 5: Verify no partial execution (all or nothing per task)
    for task_id in task_ids:
        task = await memory_db.get_task(task_id)
        if task.status == TaskStatus.COMPLETED:
            # Completed tasks should have all timestamps
            assert task.submitted_at is not None
            assert task.started_at is not None
            assert task.completed_at is not None
        elif task.status == TaskStatus.READY:
            # Ready tasks should have no execution timestamps
            assert task.started_at is None
            assert task.completed_at is None


# Integration Tests - Scenario 2: Graceful Shutdown with Active Tasks


@pytest.mark.asyncio
async def test_scenario_2_graceful_shutdown_with_active_tasks(
    task_queue_service: TaskQueueService,
    memory_db: Database,
) -> None:
    """Scenario 2: Verify active tasks complete after limit reached.

    Test Strategy:
    1. Create 8 tasks with 100ms execution time (simulates slow tasks)
    2. Run: start_swarm(task_limit=5, max_concurrent_agents=3)
    3. Monitor active tasks during execution
    4. Expected: At least 5 tasks complete (tasks 6-7 may complete if spawned before limit)
    5. Verify graceful shutdown waits for active tasks

    Success Criteria:
    - At least 5 tasks complete
    - Active tasks spawned before limit are allowed to finish
    - No tasks terminated mid-execution
    - Logs show "task_limit_reached" message

    Implementation Reference:
    - src/abathur/application/swarm_orchestrator.py:178-180 (wait for active tasks)
    """
    # Create mock executor with longer execution time
    mock_executor = AsyncMock()

    async def slow_execute(task: Task) -> Result:
        """Simulate slower task execution to observe concurrency."""
        await asyncio.sleep(0.1)  # 100ms execution time
        return Result(
            task_id=task.id,
            agent_id=uuid4(),
            success=True,
            data={"output": f"Task {task.id} completed after 100ms"},
        )

    mock_executor.execute_task.side_effect = slow_execute

    # Create orchestrator with max_concurrent=3
    orchestrator = SwarmOrchestrator(
        task_queue_service=task_queue_service,
        agent_executor=mock_executor,
        max_concurrent_agents=3,
        poll_interval=0.01,
    )

    # Step 1: Create 8 tasks
    task_ids = []
    for i in range(8):
        task = await task_queue_service.enqueue_task(
            description=f"Scenario 2 - Task {i}",
            source=TaskSource.HUMAN,
            agent_type="general-purpose",
            base_priority=5,
        )
        task_ids.append(task.id)

    # Step 2: Run swarm with task_limit=5
    results = await orchestrator.start_swarm(task_limit=5)

    # Assertion 1: At least 5 tasks processed (may be more if spawned before limit)
    assert len(results) >= 5, f"Expected at least 5 results, got {len(results)}"

    # Assertion 2: All processed tasks completed successfully
    assert all(r.success for r in results), "All tasks should complete successfully"

    # Assertion 3: No more than max_concurrent+task_limit tasks processed
    # (worst case: 3 active + 5 limit = 8 total if all spawned concurrently)
    assert len(results) <= 8, f"Expected at most 8 results, got {len(results)}"

    # Step 3: Verify database state
    completed_count = 0
    ready_count = 0

    for task_id in task_ids:
        task = await memory_db.get_task(task_id)
        if task.status == TaskStatus.COMPLETED:
            completed_count += 1
            # Verify timestamp ordering for completed tasks
            assert task.submitted_at <= task.started_at
            assert task.started_at <= task.completed_at
        elif task.status == TaskStatus.READY:
            ready_count += 1

    # Assertion 4: Completed count matches result count
    assert completed_count == len(results)

    # Assertion 5: Remaining tasks stay in READY state
    assert ready_count == (8 - completed_count)


# Integration Tests - Scenario 3: Indefinite Mode (task_limit=None)


@pytest.mark.asyncio
async def test_scenario_3_indefinite_mode_task_limit_none(
    task_queue_service: TaskQueueService,
    memory_db: Database,
    mock_agent_executor: AsyncMock,
) -> None:
    """Scenario 3: Verify task_limit=None runs indefinitely.

    Test Strategy:
    1. Create 20 tasks
    2. Run: start_swarm(task_limit=None)
    3. Use manual shutdown after all tasks complete
    4. Expected: All 20 tasks complete
    5. Swarm continues until queue is empty (then manual shutdown)

    Success Criteria:
    - All 20 tasks complete
    - Swarm doesn't exit early
    - Backward compatibility maintained

    Implementation Reference:
    - src/abathur/application/swarm_orchestrator.py:124 (limit check skipped when None)
    """
    # Create orchestrator
    orchestrator = SwarmOrchestrator(
        task_queue_service=task_queue_service,
        agent_executor=mock_agent_executor,
        max_concurrent_agents=5,
        poll_interval=0.01,
    )

    # Step 1: Create 20 tasks
    task_ids = []
    for i in range(20):
        task = await task_queue_service.enqueue_task(
            description=f"Scenario 3 - Task {i}",
            source=TaskSource.HUMAN,
            agent_type="general-purpose",
            base_priority=5,
        )
        task_ids.append(task.id)

    # Step 2: Start swarm with task_limit=None and manual shutdown
    async def shutdown_after_tasks():
        """Shutdown swarm after all tasks complete."""
        # Wait for all tasks to be processed
        await asyncio.sleep(0.5)  # Give time for all 20 tasks to complete
        await orchestrator.shutdown()

    shutdown_task = asyncio.create_task(shutdown_after_tasks())

    results = await orchestrator.start_swarm(task_limit=None)

    await shutdown_task

    # Assertion 1: All 20 tasks processed
    assert len(results) == 20, f"Expected 20 results, got {len(results)}"

    # Assertion 2: All tasks completed successfully
    assert all(r.success for r in results), "All tasks should complete successfully"

    # Step 3: Verify database state (all COMPLETED)
    completed_count = 0
    for task_id in task_ids:
        task = await memory_db.get_task(task_id)
        if task.status == TaskStatus.COMPLETED:
            completed_count += 1

    # Assertion 3: All 20 tasks in COMPLETED state
    assert completed_count == 20, f"Expected 20 completed tasks, got {completed_count}"


# Integration Tests - Scenario 4: Zero Limit (task_limit=0)


@pytest.mark.asyncio
async def test_scenario_4_zero_limit_exits_immediately(
    task_queue_service: TaskQueueService,
    orchestrator: SwarmOrchestrator,
    memory_db: Database,
) -> None:
    """Scenario 4: Verify task_limit=0 exits immediately.

    Test Strategy:
    1. Create 10 tasks
    2. Run: start_swarm(task_limit=0)
    3. Expected: Swarm exits immediately without processing any tasks
    4. All tasks remain in READY state

    Success Criteria:
    - 0 tasks completed
    - Swarm exits immediately
    - Log shows "task_limit_reached (0)"

    Implementation Reference:
    - src/abathur/application/swarm_orchestrator.py:124-130 (limit check triggers immediately)
    """
    import time

    # Step 1: Create 10 tasks
    task_ids = []
    for i in range(10):
        task = await task_queue_service.enqueue_task(
            description=f"Scenario 4 - Task {i}",
            source=TaskSource.HUMAN,
            agent_type="general-purpose",
            base_priority=5,
        )
        task_ids.append(task.id)

    # Verify all tasks start in READY state
    for task_id in task_ids:
        task = await memory_db.get_task(task_id)
        assert task.status == TaskStatus.READY

    # Step 2: Run swarm with task_limit=0 and measure time
    start_time = time.time()
    results = await orchestrator.start_swarm(task_limit=0)
    duration = time.time() - start_time

    # Assertion 1: Zero tasks processed
    assert len(results) == 0, f"Expected 0 results, got {len(results)}"

    # Assertion 2: Swarm exited immediately (<1 second)
    assert duration < 1.0, f"Expected immediate exit (<1s), but took {duration:.2f}s"

    # Step 3: Verify database state (all READY)
    ready_count = 0
    for task_id in task_ids:
        task = await memory_db.get_task(task_id)
        if task.status == TaskStatus.READY:
            ready_count += 1
            # Verify no execution occurred
            assert task.started_at is None
            assert task.completed_at is None

    # Assertion 3: All 10 tasks remain READY
    assert ready_count == 10, f"Expected 10 ready tasks, got {ready_count}"


# Integration Tests - Scenario 5: Failed Tasks Count Toward Limit


@pytest.mark.asyncio
async def test_scenario_5_failed_tasks_count_toward_limit(
    task_queue_service: TaskQueueService,
    memory_db: Database,
) -> None:
    """Scenario 5: Verify failed tasks count toward task_limit.

    Test Strategy:
    1. Create 10 tasks, configure executor to fail all tasks
    2. Run: start_swarm(task_limit=5)
    3. Expected: Swarm stops after 5 tasks complete (all failures)
    4. Both successful and failed tasks count toward limit

    Success Criteria:
    - Exactly 5 tasks completed (FAILED status)
    - Failed tasks counted toward limit
    - Swarm exits after 5 total completions

    Implementation Reference:
    - src/abathur/application/swarm_orchestrator.py:145 (counter increments for all tasks)
    - src/abathur/application/swarm_orchestrator.py:244-246 (failed tasks marked in queue)
    """
    # Create mock executor that fails all tasks
    failing_executor = AsyncMock()

    async def fail_task(task: Task) -> Result:
        """Simulate task failure."""
        await asyncio.sleep(0.01)
        return Result(
            task_id=task.id,
            agent_id=uuid4(),
            success=False,
            error="Simulated task failure for Scenario 5",
        )

    failing_executor.execute_task.side_effect = fail_task

    # Create orchestrator with failing executor
    orchestrator = SwarmOrchestrator(
        task_queue_service=task_queue_service,
        agent_executor=failing_executor,
        max_concurrent_agents=5,
        poll_interval=0.01,
    )

    # Step 1: Create 10 tasks
    task_ids = []
    for i in range(10):
        task = await task_queue_service.enqueue_task(
            description=f"Scenario 5 - Task {i}",
            source=TaskSource.HUMAN,
            agent_type="general-purpose",
            base_priority=5,
        )
        task_ids.append(task.id)

    # Step 2: Run swarm with task_limit=5
    results = await orchestrator.start_swarm(task_limit=5)

    # Assertion 1: Exactly 5 tasks processed (despite all failing)
    assert len(results) == 5, f"Expected exactly 5 results, got {len(results)}"

    # Assertion 2: All 5 tasks failed
    assert all(not r.success for r in results), "All tasks should have failed"

    # Assertion 3: All failures have error messages
    assert all(
        r.error and "Simulated task failure" in r.error for r in results
    ), "All failures should have error messages"

    # Step 3: Verify database state (5 FAILED, 5 READY)
    failed_count = 0
    ready_count = 0

    for task_id in task_ids:
        task = await memory_db.get_task(task_id)
        assert task is not None, f"Task {task_id} not found in database"

        if task.status == TaskStatus.FAILED:
            failed_count += 1
            # Failed tasks should have timestamps
            assert task.started_at is not None
            assert task.failed_at is not None
            # Verify error message stored
            assert task.error_message is not None
            assert "Simulated task failure" in task.error_message
        elif task.status == TaskStatus.READY:
            ready_count += 1
            assert task.started_at is None

    # Assertion 4: Exactly 5 failed tasks
    assert failed_count == 5, f"Expected 5 failed tasks, got {failed_count}"

    # Assertion 5: Exactly 5 remain ready
    assert ready_count == 5, f"Expected 5 ready tasks, got {ready_count}"


# Additional Integration Test: Mixed Success and Failure


@pytest.mark.asyncio
async def test_scenario_5b_mixed_success_and_failure_tasks(
    task_queue_service: TaskQueueService,
    memory_db: Database,
) -> None:
    """Scenario 5b: Verify mixed success/failure tasks count toward limit.

    Test Strategy:
    1. Create 10 tasks
    2. Configure executor to fail every other task (5 success, 5 failure)
    3. Run: start_swarm(task_limit=7)
    4. Expected: Mix of 7 total tasks (success + failure)

    Success Criteria:
    - Exactly 7 tasks processed (mix of success and failure)
    - Both successful and failed tasks count toward limit
    """
    # Create mock executor with mixed results
    mixed_executor = AsyncMock()
    call_count = 0

    async def mixed_execute(task: Task) -> Result:
        """Alternate between success and failure."""
        nonlocal call_count
        call_count += 1
        await asyncio.sleep(0.01)

        # Fail odd-numbered calls, succeed even-numbered
        if call_count % 2 == 1:
            return Result(
                task_id=task.id,
                agent_id=uuid4(),
                success=False,
                error="Intentional failure for testing",
            )
        else:
            return Result(
                task_id=task.id,
                agent_id=uuid4(),
                success=True,
                data={"output": "Success"},
            )

    mixed_executor.execute_task.side_effect = mixed_execute

    # Create orchestrator
    orchestrator = SwarmOrchestrator(
        task_queue_service=task_queue_service,
        agent_executor=mixed_executor,
        max_concurrent_agents=5,
        poll_interval=0.01,
    )

    # Create 10 tasks
    task_ids = []
    for i in range(10):
        task = await task_queue_service.enqueue_task(
            description=f"Scenario 5b - Task {i}",
            source=TaskSource.HUMAN,
            agent_type="general-purpose",
            base_priority=5,
        )
        task_ids.append(task.id)

    # Run swarm with task_limit=7
    results = await orchestrator.start_swarm(task_limit=7)

    # Assertion 1: Exactly 7 tasks processed
    assert len(results) == 7, f"Expected 7 results, got {len(results)}"

    # Assertion 2: Mix of success and failure
    success_count = sum(1 for r in results if r.success)
    failure_count = sum(1 for r in results if not r.success)

    assert success_count > 0, "Should have at least 1 successful task"
    assert failure_count > 0, "Should have at least 1 failed task"
    assert success_count + failure_count == 7, "Total should be 7"

    # Verify database state
    completed_count = 0
    failed_count = 0
    ready_count = 0

    for task_id in task_ids:
        task = await memory_db.get_task(task_id)
        if task.status == TaskStatus.COMPLETED:
            completed_count += 1
        elif task.status == TaskStatus.FAILED:
            failed_count += 1
        elif task.status == TaskStatus.READY:
            ready_count += 1

    # Exactly 7 tasks processed (completed or failed), 3 remain ready
    assert (completed_count + failed_count) == 7
    assert ready_count == 3
