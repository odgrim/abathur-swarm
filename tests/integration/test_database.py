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
            prompt="Test task prompt",
            agent_type="test-agent",
            input_data={"key": "value"},
            priority=7,
        )

        await database.insert_task(task)

        # Retrieve the task
        retrieved_task = await database.get_task(task.id)

        assert retrieved_task is not None
        assert retrieved_task.id == task.id
        assert retrieved_task.prompt == "Test task prompt"
        assert retrieved_task.agent_type == "test-agent"
        assert retrieved_task.priority == 7
        assert retrieved_task.input_data == {"key": "value"}
        assert retrieved_task.status == TaskStatus.PENDING

    @pytest.mark.asyncio
    async def test_update_task_status(self, database: Database) -> None:
        """Test updating task status."""
        task = Task(
            prompt="Test task",
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
        task1 = Task(prompt="Task 1", priority=5)
        task2 = Task(prompt="Task 2", priority=8)
        task3 = Task(prompt="Task 3", priority=3)

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
        task1 = Task(prompt="Task 1")
        task2 = Task(prompt="Task 2")

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
        task1 = Task(prompt="Task 1", priority=5)
        task2 = Task(prompt="Task 2", priority=8)
        task3 = Task(prompt="Task 3", priority=3)

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
        parent_task = Task(prompt="Parent task")
        await database.insert_task(parent_task)

        child_task = Task(
            prompt="Child task",
            parent_task_id=parent_task.id,
        )
        await database.insert_task(child_task)

        retrieved_child = await database.get_task(child_task.id)
        assert retrieved_child is not None
        assert retrieved_child.parent_task_id == parent_task.id

    @pytest.mark.asyncio
    async def test_delete_task_by_id_success(self, database: Database) -> None:
        """Test successful deletion of a task by ID."""
        task = Task(prompt="Task to delete", summary="Delete test task")
        await database.insert_task(task)

        # Verify task exists
        retrieved_task = await database.get_task(task.id)
        assert retrieved_task is not None

        # Delete the task
        result = await database.delete_task_by_id(task.id)
        assert result is True

        # Verify task is deleted
        deleted_task = await database.get_task(task.id)
        assert deleted_task is None

    @pytest.mark.asyncio
    async def test_delete_task_by_id_not_found(self, database: Database) -> None:
        """Test deletion of non-existent task returns False."""
        non_existent_id = uuid4()

        # Try to delete non-existent task
        result = await database.delete_task_by_id(non_existent_id)
        assert result is False

    @pytest.mark.asyncio
    async def test_delete_task_by_id_cascade(self, database: Database) -> None:
        """Verify CASCADE deletes agents and checkpoints."""
        # Create a task
        task = Task(prompt="Task with dependencies", summary="CASCADE test task")
        await database.insert_task(task)

        # Create an agent for the task
        agent = Agent(
            name="test-agent",
            specialization="testing",
            task_id=task.id,
        )
        await database.insert_agent(agent)

        # Create a checkpoint for the task
        async with database._get_connection() as conn:
            await conn.execute(
                """
                INSERT INTO checkpoints (task_id, iteration, state, created_at)
                VALUES (?, ?, ?, ?)
                """,
                (str(task.id), 1, '{"test": "data"}', "2024-01-01T00:00:00"),
            )
            await conn.commit()

        # Verify task, agent, and checkpoint exist
        retrieved_task = await database.get_task(task.id)
        assert retrieved_task is not None

        async with database._get_connection() as conn:
            # Check agent exists
            cursor = await conn.execute(
                "SELECT COUNT(*) FROM agents WHERE task_id = ?",
                (str(task.id),),
            )
            agent_count = (await cursor.fetchone())[0]
            assert agent_count == 1

            # Check checkpoint exists
            cursor = await conn.execute(
                "SELECT COUNT(*) FROM checkpoints WHERE task_id = ?",
                (str(task.id),),
            )
            checkpoint_count = (await cursor.fetchone())[0]
            assert checkpoint_count == 1

        # Delete the task
        result = await database.delete_task_by_id(task.id)
        assert result is True

        # Verify CASCADE deleted agents and checkpoints
        async with database._get_connection() as conn:
            # Check agent was deleted
            cursor = await conn.execute(
                "SELECT COUNT(*) FROM agents WHERE task_id = ?",
                (str(task.id),),
            )
            agent_count = (await cursor.fetchone())[0]
            assert agent_count == 0

            # Check checkpoint was deleted
            cursor = await conn.execute(
                "SELECT COUNT(*) FROM checkpoints WHERE task_id = ?",
                (str(task.id),),
            )
            checkpoint_count = (await cursor.fetchone())[0]
            assert checkpoint_count == 0

    @pytest.mark.asyncio
    async def test_delete_tasks_by_status_success(self, database: Database) -> None:
        """Test successful deletion of multiple tasks by status."""
        # Create tasks with different statuses
        task1 = Task(prompt="Task 1", summary="Task 1 summary")
        task2 = Task(prompt="Task 2", summary="Task 2 summary")
        task3 = Task(prompt="Task 3", summary="Task 3 summary")

        await database.insert_task(task1)
        await database.insert_task(task2)
        await database.insert_task(task3)

        # Update task2 and task3 to completed
        await database.update_task_status(task2.id, TaskStatus.COMPLETED)
        await database.update_task_status(task3.id, TaskStatus.COMPLETED)

        # Delete all completed tasks
        deleted_count = await database.delete_tasks_by_status(TaskStatus.COMPLETED)
        assert deleted_count == 2

        # Verify tasks are deleted
        retrieved_task2 = await database.get_task(task2.id)
        retrieved_task3 = await database.get_task(task3.id)
        assert retrieved_task2 is None
        assert retrieved_task3 is None

        # Verify task1 still exists
        retrieved_task1 = await database.get_task(task1.id)
        assert retrieved_task1 is not None
        assert retrieved_task1.status == TaskStatus.PENDING

    @pytest.mark.asyncio
    async def test_delete_tasks_by_status_no_tasks(self, database: Database) -> None:
        """Test deletion when no tasks match status (returns 0)."""
        # Create a pending task
        task = Task(prompt="Task 1", summary="Task 1 summary")
        await database.insert_task(task)

        # Try to delete completed tasks (none exist)
        deleted_count = await database.delete_tasks_by_status(TaskStatus.COMPLETED)
        assert deleted_count == 0

        # Verify task still exists
        retrieved_task = await database.get_task(task.id)
        assert retrieved_task is not None

    @pytest.mark.asyncio
    async def test_delete_tasks_by_status_cascade(self, database: Database) -> None:
        """Verify CASCADE deletes agents and checkpoints for all tasks."""
        # Create multiple tasks
        task1 = Task(prompt="Task 1", summary="Task 1 summary")
        task2 = Task(prompt="Task 2", summary="Task 2 summary")

        await database.insert_task(task1)
        await database.insert_task(task2)

        # Update both to completed
        await database.update_task_status(task1.id, TaskStatus.COMPLETED)
        await database.update_task_status(task2.id, TaskStatus.COMPLETED)

        # Create agents for both tasks
        agent1 = Agent(name="agent-1", specialization="test", task_id=task1.id)
        agent2 = Agent(name="agent-2", specialization="test", task_id=task2.id)

        await database.insert_agent(agent1)
        await database.insert_agent(agent2)

        # Create checkpoints for both tasks
        async with database._get_connection() as conn:
            await conn.execute(
                """
                INSERT INTO checkpoints (task_id, iteration, state, created_at)
                VALUES (?, ?, ?, ?)
                """,
                (str(task1.id), 1, '{"test": "data1"}', "2024-01-01T00:00:00"),
            )
            await conn.execute(
                """
                INSERT INTO checkpoints (task_id, iteration, state, created_at)
                VALUES (?, ?, ?, ?)
                """,
                (str(task2.id), 1, '{"test": "data2"}', "2024-01-01T00:00:00"),
            )
            await conn.commit()

        # Verify agents and checkpoints exist
        async with database._get_connection() as conn:
            cursor = await conn.execute("SELECT COUNT(*) FROM agents")
            agent_count = (await cursor.fetchone())[0]
            assert agent_count == 2

            cursor = await conn.execute("SELECT COUNT(*) FROM checkpoints")
            checkpoint_count = (await cursor.fetchone())[0]
            assert checkpoint_count == 2

        # Delete all completed tasks
        deleted_count = await database.delete_tasks_by_status(TaskStatus.COMPLETED)
        assert deleted_count == 2

        # Verify CASCADE deleted all agents and checkpoints
        async with database._get_connection() as conn:
            cursor = await conn.execute("SELECT COUNT(*) FROM agents")
            agent_count = (await cursor.fetchone())[0]
            assert agent_count == 0

            cursor = await conn.execute("SELECT COUNT(*) FROM checkpoints")
            checkpoint_count = (await cursor.fetchone())[0]
            assert checkpoint_count == 0


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
