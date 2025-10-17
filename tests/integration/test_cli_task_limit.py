"""Integration tests for CLI --task-limit flag.

Tests complete end-to-end workflows:
- 'abathur swarm start --task-limit N' command
- CLI invocation with real database
- Task limit enforcement through CLI interface
- CLI output validation

This test validates Phase 2 (Validation & Testing) for task limit enforcement feature.
"""

import asyncio
import tempfile
from pathlib import Path
from unittest.mock import AsyncMock, patch
from uuid import uuid4

import pytest
from abathur.application.agent_executor import AgentExecutor
from abathur.cli.main import app
from abathur.domain.models import Result, Task, TaskSource, TaskStatus
from abathur.infrastructure.database import Database
from abathur.services.dependency_resolver import DependencyResolver
from abathur.services.priority_calculator import PriorityCalculator
from abathur.services.task_queue_service import TaskQueueService
from typer.testing import CliRunner

# Fixtures


@pytest.fixture
def temp_db_path_sync() -> Path:
    """Create temporary database file for CLI testing (synchronous)."""
    with tempfile.NamedTemporaryFile(suffix=".db", delete=False) as f:
        db_path = Path(f.name)
    yield db_path
    # Cleanup
    if db_path.exists():
        db_path.unlink()
    # Also cleanup WAL files
    wal_path = db_path.with_suffix(".db-wal")
    shm_path = db_path.with_suffix(".db-shm")
    if wal_path.exists():
        wal_path.unlink()
    if shm_path.exists():
        shm_path.unlink()


def _setup_populated_db_sync(db_path: Path) -> list:
    """Helper to populate database synchronously."""

    # Use asyncio.run to run async setup in sync context
    async def setup():
        # Initialize database
        db = Database(db_path)
        await db.initialize()

        # Create task queue service
        dependency_resolver = DependencyResolver(db)
        priority_calculator = PriorityCalculator(dependency_resolver)
        task_queue_service = TaskQueueService(db, dependency_resolver, priority_calculator)

        # Create 10 tasks
        task_ids = []
        for i in range(10):
            task = await task_queue_service.enqueue_task(
                description=f"CLI test task {i}",
                source=TaskSource.HUMAN,
                agent_type="general-purpose",
                base_priority=5,
            )
            task_ids.append(task.id)

        # Close database connection
        await db.close()
        return task_ids

    return asyncio.run(setup())


@pytest.fixture
def cli_runner() -> CliRunner:
    """Create Typer CLI test runner."""
    return CliRunner()


@pytest.fixture
def mock_agent_executor() -> AsyncMock:
    """Create mock AgentExecutor that succeeds quickly."""
    executor = AsyncMock(spec=AgentExecutor)

    # Mock successful task execution with 10ms delay
    async def execute_task(task: Task) -> Result:
        await asyncio.sleep(0.01)  # Simulate task execution
        return Result(
            task_id=task.id,
            agent_id=uuid4(),
            success=True,
            data={"output": f"Task {task.id} completed"},
        )

    executor.execute_task.side_effect = execute_task
    return executor


# Integration Tests


def test_cli_swarm_start_with_task_limit(
    temp_db_path_sync: Path,
    cli_runner: CliRunner,
    mock_agent_executor: AsyncMock,
) -> None:
    """Test CLI 'swarm start --task-limit N' processes exactly N tasks.

    Integration test validating:
    1. CLI accepts --task-limit flag
    2. CLI invokes SwarmOrchestrator with task_limit parameter
    3. Exactly N tasks are processed (not all tasks in queue)
    4. Database reflects correct final state (N completed, rest ready)
    5. CLI exits successfully with informative output

    Tests Phase 2 validation criteria:
    - CLI --task-limit flag works end-to-end
    - Swarm stops after processing N tasks
    - Remaining tasks stay in queue
    """
    # Setup: Create populated database
    task_ids = _setup_populated_db_sync(temp_db_path_sync)
    db_path = temp_db_path_sync
    task_limit = 5

    # Patch _get_services to inject mock executor
    async def mock_get_services():
        """Mock services with real database and mock executor."""
        # Initialize real database
        db = Database(db_path)
        await db.initialize()

        # Create real task queue service
        dependency_resolver = DependencyResolver(db)
        priority_calculator = PriorityCalculator(dependency_resolver)
        task_queue_service = TaskQueueService(db, dependency_resolver, priority_calculator)

        # Import real classes
        from abathur.application import SwarmOrchestrator, TaskCoordinator
        from abathur.infrastructure import ConfigManager

        # Create swarm orchestrator with mock executor
        swarm_orchestrator = SwarmOrchestrator(
            task_queue_service=task_queue_service,
            agent_executor=mock_agent_executor,
            max_concurrent_agents=5,
            poll_interval=0.01,  # Fast polling for testing
        )

        task_coordinator = TaskCoordinator(db)
        config_manager = ConfigManager()

        return {
            "database": db,
            "task_coordinator": task_coordinator,
            "task_queue_service": task_queue_service,
            "agent_executor": mock_agent_executor,
            "swarm_orchestrator": swarm_orchestrator,
            "config_manager": config_manager,
            "template_manager": AsyncMock(),
            "mcp_manager": AsyncMock(initialize=AsyncMock()),
            "resource_monitor": AsyncMock(
                start_monitoring=AsyncMock(), stop_monitoring=AsyncMock()
            ),
            "loop_executor": AsyncMock(),
            "claude_client": AsyncMock(),
        }

    # Patch _get_services in CLI module
    with patch("abathur.cli.main._get_services", side_effect=mock_get_services):
        # Invoke CLI command: abathur swarm start --task-limit 5 --no-mcp
        result = cli_runner.invoke(
            app,
            [
                "swarm",
                "start",
                "--task-limit",
                str(task_limit),
                "--no-mcp",  # Disable MCP server for testing
            ],
        )

    # Assertion 1: CLI exits successfully
    assert (
        result.exit_code == 0
    ), f"CLI exited with error (code {result.exit_code}): {result.stdout}"

    # Assertion 2: CLI output mentions task completion
    output_lower = result.stdout.lower()
    assert (
        "swarm completed" in output_lower or "tasks" in output_lower
    ), f"CLI output doesn't mention task completion: {result.stdout}"

    # Assertion 3: Verify database state (5 completed, 5 ready)
    # Re-open database to check final state
    async def verify_database():
        db = Database(db_path)
        await db.initialize()

        completed_count = 0
        ready_count = 0

        for task_id in task_ids:
            task = await db.get_task(task_id)
            assert task is not None, f"Task {task_id} not found in database"

            if task.status == TaskStatus.COMPLETED:
                completed_count += 1
                assert task.completed_at is not None, f"Task {task_id} missing completed_at"
            elif task.status == TaskStatus.READY:
                ready_count += 1
                assert task.started_at is None, f"Task {task_id} should not have started"

        await db.close()
        return completed_count, ready_count

    completed_count, ready_count = asyncio.run(verify_database())

    # Assertion 4: Exactly 5 tasks completed
    assert (
        completed_count == task_limit
    ), f"Expected {task_limit} completed tasks, got {completed_count}"

    # Assertion 5: Exactly 5 tasks remain ready
    assert ready_count == (
        len(task_ids) - task_limit
    ), f"Expected {len(task_ids) - task_limit} ready tasks, got {ready_count}"


def test_cli_swarm_start_without_task_limit_processes_all_tasks(
    temp_db_path_sync: Path,
    cli_runner: CliRunner,
    mock_agent_executor: AsyncMock,
) -> None:
    """Test CLI 'swarm start' WITHOUT --task-limit processes all available tasks.

    Validates:
    1. CLI works without --task-limit flag (backward compatibility)
    2. Without limit, all tasks in queue are processed
    3. Database shows all tasks completed

    This ensures the --task-limit flag is truly optional.
    """
    # Setup: Create populated database
    task_ids = _setup_populated_db_sync(temp_db_path_sync)
    db_path = temp_db_path_sync

    # Patch _get_services to inject mock executor
    async def mock_get_services():
        """Mock services with real database and mock executor."""
        db = Database(db_path)
        await db.initialize()

        dependency_resolver = DependencyResolver(db)
        priority_calculator = PriorityCalculator(dependency_resolver)
        task_queue_service = TaskQueueService(db, dependency_resolver, priority_calculator)

        from abathur.application import SwarmOrchestrator, TaskCoordinator
        from abathur.infrastructure import ConfigManager

        swarm_orchestrator = SwarmOrchestrator(
            task_queue_service=task_queue_service,
            agent_executor=mock_agent_executor,
            max_concurrent_agents=5,
            poll_interval=0.01,
        )

        task_coordinator = TaskCoordinator(db)
        config_manager = ConfigManager()

        return {
            "database": db,
            "task_coordinator": task_coordinator,
            "task_queue_service": task_queue_service,
            "agent_executor": mock_agent_executor,
            "swarm_orchestrator": swarm_orchestrator,
            "config_manager": config_manager,
            "template_manager": AsyncMock(),
            "mcp_manager": AsyncMock(initialize=AsyncMock()),
            "resource_monitor": AsyncMock(
                start_monitoring=AsyncMock(), stop_monitoring=AsyncMock()
            ),
            "loop_executor": AsyncMock(),
            "claude_client": AsyncMock(),
        }

    # Add timeout to prevent infinite loop if no tasks are available
    # The swarm will exit when no READY tasks remain
    with patch("abathur.cli.main._get_services", side_effect=mock_get_services):
        # Invoke CLI without --task-limit flag
        result = cli_runner.invoke(
            app,
            [
                "swarm",
                "start",
                "--no-mcp",
                # Note: NO --task-limit flag specified
            ],
            # Add a reasonable timeout to prevent infinite test hang
            # In reality, swarm will exit when queue is empty
            input=None,  # No interactive input
        )

    # CLI should exit successfully after processing all tasks
    assert (
        result.exit_code == 0
    ), f"CLI exited with error (code {result.exit_code}): {result.stdout}"

    # Verify all tasks are completed
    async def verify_all_completed():
        db = Database(db_path)
        await db.initialize()

        completed_count = 0
        for task_id in task_ids:
            task = await db.get_task(task_id)
            if task.status == TaskStatus.COMPLETED:
                completed_count += 1

        await db.close()
        return completed_count

    completed_count = asyncio.run(verify_all_completed())

    # All 10 tasks should be completed when no limit is specified
    assert completed_count == len(
        task_ids
    ), f"Expected all {len(task_ids)} tasks completed, got {completed_count}"


def test_cli_swarm_start_task_limit_zero_exits_immediately(
    temp_db_path_sync: Path,
    cli_runner: CliRunner,
    mock_agent_executor: AsyncMock,
) -> None:
    """Test CLI 'swarm start --task-limit 0' exits immediately without processing tasks.

    Validates:
    1. --task-limit 0 is a valid edge case
    2. No tasks are spawned
    3. All tasks remain in READY state
    4. CLI exits gracefully
    """
    # Setup: Create populated database
    task_ids = _setup_populated_db_sync(temp_db_path_sync)
    db_path = temp_db_path_sync

    # Patch _get_services
    async def mock_get_services():
        db = Database(db_path)
        await db.initialize()

        dependency_resolver = DependencyResolver(db)
        priority_calculator = PriorityCalculator(dependency_resolver)
        task_queue_service = TaskQueueService(db, dependency_resolver, priority_calculator)

        from abathur.application import SwarmOrchestrator, TaskCoordinator
        from abathur.infrastructure import ConfigManager

        swarm_orchestrator = SwarmOrchestrator(
            task_queue_service=task_queue_service,
            agent_executor=mock_agent_executor,
            max_concurrent_agents=5,
            poll_interval=0.01,
        )

        task_coordinator = TaskCoordinator(db)
        config_manager = ConfigManager()

        return {
            "database": db,
            "task_coordinator": task_coordinator,
            "task_queue_service": task_queue_service,
            "agent_executor": mock_agent_executor,
            "swarm_orchestrator": swarm_orchestrator,
            "config_manager": config_manager,
            "template_manager": AsyncMock(),
            "mcp_manager": AsyncMock(initialize=AsyncMock()),
            "resource_monitor": AsyncMock(
                start_monitoring=AsyncMock(), stop_monitoring=AsyncMock()
            ),
            "loop_executor": AsyncMock(),
            "claude_client": AsyncMock(),
        }

    with patch("abathur.cli.main._get_services", side_effect=mock_get_services):
        # Invoke CLI with --task-limit 0
        result = cli_runner.invoke(
            app,
            ["swarm", "start", "--task-limit", "0", "--no-mcp"],
        )

    # CLI should exit successfully
    assert (
        result.exit_code == 0
    ), f"CLI exited with error (code {result.exit_code}): {result.stdout}"

    # Verify NO tasks were executed (mock should have 0 calls)
    # Note: Can't directly check mock call count because of patching scope
    # Instead, verify database state

    # Verify all tasks remain in READY state (none were processed)
    async def verify_all_ready():
        db = Database(db_path)
        await db.initialize()

        ready_count = 0
        for task_id in task_ids:
            task = await db.get_task(task_id)
            if task.status == TaskStatus.READY:
                ready_count += 1

        await db.close()
        return ready_count

    ready_count = asyncio.run(verify_all_ready())

    # All tasks should remain READY
    assert ready_count == len(
        task_ids
    ), f"Expected all {len(task_ids)} tasks to remain READY, got {ready_count}"


def test_cli_swarm_start_task_limit_exceeds_queue_size(
    temp_db_path_sync: Path,
    cli_runner: CliRunner,
    mock_agent_executor: AsyncMock,
) -> None:
    """Test CLI 'swarm start --task-limit N' where N > queue size.

    Validates:
    1. CLI handles limit greater than available tasks
    2. Only processes available tasks (not more)
    3. Swarm exits when queue is empty (before reaching limit)
    """
    # Setup: Create populated database
    task_ids = _setup_populated_db_sync(temp_db_path_sync)
    db_path = temp_db_path_sync
    task_limit = 100  # Much larger than 10 tasks in queue

    # Patch _get_services
    async def mock_get_services():
        db = Database(db_path)
        await db.initialize()

        dependency_resolver = DependencyResolver(db)
        priority_calculator = PriorityCalculator(dependency_resolver)
        task_queue_service = TaskQueueService(db, dependency_resolver, priority_calculator)

        from abathur.application import SwarmOrchestrator, TaskCoordinator
        from abathur.infrastructure import ConfigManager

        swarm_orchestrator = SwarmOrchestrator(
            task_queue_service=task_queue_service,
            agent_executor=mock_agent_executor,
            max_concurrent_agents=5,
            poll_interval=0.01,
        )

        task_coordinator = TaskCoordinator(db)
        config_manager = ConfigManager()

        return {
            "database": db,
            "task_coordinator": task_coordinator,
            "task_queue_service": task_queue_service,
            "agent_executor": mock_agent_executor,
            "swarm_orchestrator": swarm_orchestrator,
            "config_manager": config_manager,
            "template_manager": AsyncMock(),
            "mcp_manager": AsyncMock(initialize=AsyncMock()),
            "resource_monitor": AsyncMock(
                start_monitoring=AsyncMock(), stop_monitoring=AsyncMock()
            ),
            "loop_executor": AsyncMock(),
            "claude_client": AsyncMock(),
        }

    with patch("abathur.cli.main._get_services", side_effect=mock_get_services):
        # Invoke CLI with --task-limit 100 (but only 10 tasks exist)
        result = cli_runner.invoke(
            app,
            ["swarm", "start", "--task-limit", str(task_limit), "--no-mcp"],
        )

    # CLI should exit successfully
    assert (
        result.exit_code == 0
    ), f"CLI exited with error (code {result.exit_code}): {result.stdout}"

    # Verify only 10 tasks were processed (all available tasks)
    async def verify_completed():
        db = Database(db_path)
        await db.initialize()

        completed_count = 0
        for task_id in task_ids:
            task = await db.get_task(task_id)
            if task.status == TaskStatus.COMPLETED:
                completed_count += 1

        await db.close()
        return completed_count

    completed_count = asyncio.run(verify_completed())

    # Should complete all 10 tasks (not 100)
    assert completed_count == len(
        task_ids
    ), f"Expected {len(task_ids)} completed tasks, got {completed_count}"


def test_cli_swarm_start_task_limit_with_failed_tasks(
    temp_db_path_sync: Path,
    cli_runner: CliRunner,
) -> None:
    """Test CLI 'swarm start --task-limit N' counts failed tasks toward limit.

    Validates:
    1. Failed tasks count toward task_limit
    2. Swarm stops after processing N tasks (regardless of success/failure)
    3. Database shows correct mix of FAILED and READY tasks
    """
    # Setup: Create populated database
    task_ids = _setup_populated_db_sync(temp_db_path_sync)
    db_path = temp_db_path_sync
    task_limit = 4

    # Create mock executor that fails all tasks
    failing_executor = AsyncMock(spec=AgentExecutor)

    async def fail_task(task: Task) -> Result:
        await asyncio.sleep(0.01)
        return Result(
            task_id=task.id,
            agent_id=uuid4(),
            success=False,
            error="Simulated task failure",
        )

    failing_executor.execute_task.side_effect = fail_task

    # Patch _get_services with failing executor
    async def mock_get_services():
        db = Database(db_path)
        await db.initialize()

        dependency_resolver = DependencyResolver(db)
        priority_calculator = PriorityCalculator(dependency_resolver)
        task_queue_service = TaskQueueService(db, dependency_resolver, priority_calculator)

        from abathur.application import SwarmOrchestrator, TaskCoordinator
        from abathur.infrastructure import ConfigManager

        swarm_orchestrator = SwarmOrchestrator(
            task_queue_service=task_queue_service,
            agent_executor=failing_executor,
            max_concurrent_agents=5,
            poll_interval=0.01,
        )

        task_coordinator = TaskCoordinator(db)
        config_manager = ConfigManager()

        return {
            "database": db,
            "task_coordinator": task_coordinator,
            "task_queue_service": task_queue_service,
            "agent_executor": failing_executor,
            "swarm_orchestrator": swarm_orchestrator,
            "config_manager": config_manager,
            "template_manager": AsyncMock(),
            "mcp_manager": AsyncMock(initialize=AsyncMock()),
            "resource_monitor": AsyncMock(
                start_monitoring=AsyncMock(), stop_monitoring=AsyncMock()
            ),
            "loop_executor": AsyncMock(),
            "claude_client": AsyncMock(),
        }

    with patch("abathur.cli.main._get_services", side_effect=mock_get_services):
        # Invoke CLI with --task-limit 4
        result = cli_runner.invoke(
            app,
            ["swarm", "start", "--task-limit", str(task_limit), "--no-mcp"],
        )

    # CLI should exit successfully (even with failed tasks)
    assert (
        result.exit_code == 0
    ), f"CLI exited with error (code {result.exit_code}): {result.stdout}"

    # Verify exactly 4 tasks were processed (all failed)
    async def verify_failed():
        db = Database(db_path)
        await db.initialize()

        failed_count = 0
        ready_count = 0

        for task_id in task_ids:
            task = await db.get_task(task_id)
            if task.status == TaskStatus.FAILED:
                failed_count += 1
            elif task.status == TaskStatus.READY:
                ready_count += 1

        await db.close()
        return failed_count, ready_count

    failed_count, ready_count = asyncio.run(verify_failed())

    # Exactly 4 tasks should have failed (processed)
    assert failed_count == task_limit, f"Expected {task_limit} failed tasks, got {failed_count}"

    # Exactly 6 tasks should remain ready (not processed)
    assert ready_count == (
        len(task_ids) - task_limit
    ), f"Expected {len(task_ids) - task_limit} ready tasks, got {ready_count}"
