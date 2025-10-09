"""Integration tests for database operations."""

from collections.abc import AsyncGenerator
from pathlib import Path
from tempfile import TemporaryDirectory
from uuid import uuid4

import pytest
from abathur.domain.models import Agent, AgentState, Task, TaskStatus
from abathur.infrastructure.database import Database


@pytest.fixture
async def database() -> AsyncGenerator[Database, None]:
    """Create a test database."""
    with TemporaryDirectory() as tmpdir:
        db_path = Path(tmpdir) / "test.db"
        db = Database(db_path)
        await db.initialize()
        yield db


class TestDatabaseTaskOperations:
    """Tests for database task operations."""

    @pytest.mark.asyncio
    async def test_insert_and_get_task(self, database: Database) -> None:
        """Test inserting and retrieving a task."""
        task = Task(
            template_name="test-template",
            input_data={"key": "value"},
            priority=7,
        )

        await database.insert_task(task)

        # Retrieve the task
        retrieved_task = await database.get_task(task.id)

        assert retrieved_task is not None
        assert retrieved_task.id == task.id
        assert retrieved_task.template_name == "test-template"
        assert retrieved_task.priority == 7
        assert retrieved_task.input_data == {"key": "value"}
        assert retrieved_task.status == TaskStatus.PENDING

    @pytest.mark.asyncio
    async def test_update_task_status(self, database: Database) -> None:
        """Test updating task status."""
        task = Task(
            template_name="test-template",
            input_data={},
        )

        await database.insert_task(task)

        # Update to running
        await database.update_task_status(task.id, TaskStatus.RUNNING)

        retrieved_task = await database.get_task(task.id)
        assert retrieved_task is not None
        assert retrieved_task.status == TaskStatus.RUNNING
        assert retrieved_task.started_at is not None

        # Update to completed
        await database.update_task_status(task.id, TaskStatus.COMPLETED)

        retrieved_task = await database.get_task(task.id)
        assert retrieved_task is not None
        assert retrieved_task.status == TaskStatus.COMPLETED
        assert retrieved_task.completed_at is not None

    @pytest.mark.asyncio
    async def test_list_tasks(self, database: Database) -> None:
        """Test listing tasks."""
        # Create several tasks
        task1 = Task(template_name="template1", input_data={}, priority=5)
        task2 = Task(template_name="template2", input_data={}, priority=8)
        task3 = Task(template_name="template3", input_data={}, priority=3)

        await database.insert_task(task1)
        await database.insert_task(task2)
        await database.insert_task(task3)

        # List all tasks
        tasks = await database.list_tasks()

        assert len(tasks) == 3
        # Should be ordered by priority DESC (8, 5, 3)
        assert tasks[0].id == task2.id
        assert tasks[1].id == task1.id
        assert tasks[2].id == task3.id

    @pytest.mark.asyncio
    async def test_list_tasks_by_status(self, database: Database) -> None:
        """Test listing tasks filtered by status."""
        # Create tasks with different statuses
        task1 = Task(template_name="template1", input_data={})
        task2 = Task(template_name="template2", input_data={})

        await database.insert_task(task1)
        await database.insert_task(task2)

        # Update task2 to running
        await database.update_task_status(task2.id, TaskStatus.RUNNING)

        # List only pending tasks
        pending_tasks = await database.list_tasks(status=TaskStatus.PENDING)
        assert len(pending_tasks) == 1
        assert pending_tasks[0].id == task1.id

        # List only running tasks
        running_tasks = await database.list_tasks(status=TaskStatus.RUNNING)
        assert len(running_tasks) == 1
        assert running_tasks[0].id == task2.id

    @pytest.mark.asyncio
    async def test_dequeue_next_task(self, database: Database) -> None:
        """Test dequeuing the next highest priority task."""
        # Create tasks with different priorities
        task1 = Task(template_name="template1", input_data={}, priority=5)
        task2 = Task(template_name="template2", input_data={}, priority=8)
        task3 = Task(template_name="template3", input_data={}, priority=3)

        await database.insert_task(task1)
        await database.insert_task(task2)
        await database.insert_task(task3)

        # Dequeue should return highest priority (8)
        next_task = await database.dequeue_next_task()

        assert next_task is not None
        assert next_task.id == task2.id
        assert next_task.status == TaskStatus.RUNNING

        # Dequeue again should return next highest (5)
        next_task = await database.dequeue_next_task()

        assert next_task is not None
        assert next_task.id == task1.id
        assert next_task.status == TaskStatus.RUNNING

    @pytest.mark.asyncio
    async def test_task_with_parent(self, database: Database) -> None:
        """Test task with parent relationship."""
        parent_task = Task(template_name="parent", input_data={})
        await database.insert_task(parent_task)

        child_task = Task(
            template_name="child",
            input_data={},
            parent_task_id=parent_task.id,
        )
        await database.insert_task(child_task)

        retrieved_child = await database.get_task(child_task.id)
        assert retrieved_child is not None
        assert retrieved_child.parent_task_id == parent_task.id


class TestDatabaseAgentOperations:
    """Tests for database agent operations."""

    @pytest.mark.asyncio
    async def test_insert_and_update_agent(self, database: Database) -> None:
        """Test inserting and updating agent state."""
        task_id = uuid4()
        agent = Agent(
            name="test-agent",
            specialization="testing",
            task_id=task_id,
        )

        await database.insert_agent(agent)

        # Update agent state
        await database.update_agent_state(agent.id, AgentState.IDLE)

        # Update again to terminated
        await database.update_agent_state(agent.id, AgentState.TERMINATED)


class TestDatabaseStateOperations:
    """Tests for database state operations."""

    @pytest.mark.asyncio
    async def test_set_and_get_state(self, database: Database) -> None:
        """Test setting and getting shared state."""
        task_id = uuid4()
        state_data = {"iteration": 5, "result": "success"}

        await database.set_state(task_id, "loop_state", state_data)

        retrieved_state = await database.get_state(task_id, "loop_state")

        assert retrieved_state == state_data

    @pytest.mark.asyncio
    async def test_update_existing_state(self, database: Database) -> None:
        """Test updating existing state."""
        task_id = uuid4()
        initial_state = {"iteration": 1}

        await database.set_state(task_id, "loop_state", initial_state)

        # Update the same key
        updated_state = {"iteration": 2}
        await database.set_state(task_id, "loop_state", updated_state)

        retrieved_state = await database.get_state(task_id, "loop_state")

        assert retrieved_state == updated_state

    @pytest.mark.asyncio
    async def test_get_nonexistent_state(self, database: Database) -> None:
        """Test getting state that doesn't exist."""
        task_id = uuid4()

        retrieved_state = await database.get_state(task_id, "nonexistent")

        assert retrieved_state is None


class TestDatabaseAuditOperations:
    """Tests for database audit operations."""

    @pytest.mark.asyncio
    async def test_log_audit_entry(self, database: Database) -> None:
        """Test logging an audit entry."""
        task_id = uuid4()
        agent_id = uuid4()

        await database.log_audit(
            task_id=task_id,
            action_type="execute_task",
            agent_id=agent_id,
            action_data={"command": "test"},
            result="success",
        )

        # Audit entry should be logged (no error)
