"""Integration tests for CLI mem show command."""

import asyncio
import pytest
from pathlib import Path
from typer.testing import CliRunner
from uuid import uuid4

from abathur.cli.main import app
from abathur.domain.models import Task, TaskSource, TaskStatus
from abathur.infrastructure import Database


runner = CliRunner()


@pytest.fixture(scope="function")
def database(cli_test_db_path: Path, mock_cli_database_path):
    """Create a test database at .abathur/test.db with mocked path."""
    # Create test database (path already cleaned by cli_test_db_path fixture)
    db = Database(cli_test_db_path)
    asyncio.run(db.initialize())
    return db


@pytest.fixture(scope="function")
def sample_task_with_memory(database):
    """Create a task and associated memory entries for testing."""
    from abathur.application import TaskCoordinator
    from abathur.services.memory_service import MemoryService

    coordinator = TaskCoordinator(database)
    memory_service = MemoryService(database)

    async def create_task_and_memories():
        # Create a test task
        task = Task(
            prompt="Test task for memory",
            summary="Test task with memories",
            agent_type="test-agent",
            source=TaskSource.HUMAN,
            status=TaskStatus.COMPLETED,
        )
        task_id = await coordinator.submit_task(task)

        # Add memory entries for this task
        await memory_service.add_memory(
            namespace=f"task:{task_id}:requirements",
            key="functional_requirements",
            value={"requirements": ["req1", "req2", "req3"]},
            memory_type="semantic",
            created_by="test-agent",
        )

        await memory_service.add_memory(
            namespace=f"task:{task_id}:requirements",
            key="constraints",
            value={"constraints": ["constraint1", "constraint2"]},
            memory_type="semantic",
            created_by="test-agent",
        )

        await memory_service.add_memory(
            namespace=f"task:{task_id}:workflow",
            key="status",
            value={"status": "in_progress"},
            memory_type="episodic",
            created_by="test-agent",
        )

        return task_id

    return asyncio.run(create_task_and_memories())


def test_mem_show_with_task_prefix(database, sample_task_with_memory):
    """Test showing memories with task: namespace prefix."""
    task_id = str(sample_task_with_memory)
    prefix = f"task:{task_id}"

    result = runner.invoke(app, ["mem", "show", prefix])

    assert result.exit_code == 0
    assert "Found 3 memory entries" in result.stdout
    assert "functional_requirements" in result.stdout
    assert "constraints" in result.stdout
    assert "workflow" in result.stdout


def test_mem_show_with_task_id_only(database, sample_task_with_memory):
    """Test showing memories with bare task ID (auto-resolved)."""
    task_id = str(sample_task_with_memory)

    result = runner.invoke(app, ["mem", "show", task_id])

    assert result.exit_code == 0
    assert "Found 3 memory entries" in result.stdout
    assert "functional_requirements" in result.stdout


def test_mem_show_with_task_id_prefix(database, sample_task_with_memory):
    """Test showing memories with task ID prefix (auto-resolved)."""
    task_id = str(sample_task_with_memory)
    prefix = task_id[:8]  # Use first 8 characters

    result = runner.invoke(app, ["mem", "show", prefix])

    assert result.exit_code == 0
    assert "Found 3 memory entries" in result.stdout


def test_mem_show_with_specific_namespace(database, sample_task_with_memory):
    """Test showing memories with specific namespace."""
    task_id = str(sample_task_with_memory)
    namespace = f"task:{task_id}:requirements"

    result = runner.invoke(app, ["mem", "show", namespace])

    assert result.exit_code == 0
    assert "Found 2 memory entries" in result.stdout
    assert "functional_requirements" in result.stdout
    assert "constraints" in result.stdout
    # Should NOT show workflow memory
    assert "workflow" not in result.stdout


def test_mem_show_no_memories_found(database):
    """Test showing memories when no memories match the prefix."""
    fake_task_id = str(uuid4())
    prefix = f"task:{fake_task_id}"

    result = runner.invoke(app, ["mem", "show", prefix])

    assert result.exit_code == 0
    assert f"No memories found with prefix '{prefix}'" in result.stdout


def test_mem_show_custom_namespace(database):
    """Test showing memories with custom (non-task) namespace."""
    from abathur.services.memory_service import MemoryService

    memory_service = MemoryService(database)

    async def create_custom_memories():
        await memory_service.add_memory(
            namespace="project:test-project:config",
            key="settings",
            value={"setting1": "value1"},
            memory_type="semantic",
            created_by="test-agent",
        )

        await memory_service.add_memory(
            namespace="project:test-project:data",
            key="metrics",
            value={"metric1": 100},
            memory_type="semantic",
            created_by="test-agent",
        )

    asyncio.run(create_custom_memories())

    result = runner.invoke(app, ["mem", "show", "project:test-project"])

    assert result.exit_code == 0
    assert "Found 2 memory entries" in result.stdout
    assert "config" in result.stdout
    assert "data" in result.stdout


def test_mem_show_displays_version_info(database, sample_task_with_memory):
    """Test that mem show displays version information."""
    task_id = str(sample_task_with_memory)
    prefix = f"task:{task_id}"

    result = runner.invoke(app, ["mem", "show", prefix])

    assert result.exit_code == 0
    assert "Version 1:" in result.stdout
    assert "Created By:" in result.stdout
    assert "test-agent" in result.stdout


def test_mem_show_displays_json_values(database, sample_task_with_memory):
    """Test that mem show displays JSON values."""
    task_id = str(sample_task_with_memory)
    prefix = f"task:{task_id}"

    result = runner.invoke(app, ["mem", "show", prefix])

    assert result.exit_code == 0
    # Should display pretty-printed JSON
    assert "req1" in result.stdout
    assert "req2" in result.stdout
    assert "constraint1" in result.stdout
