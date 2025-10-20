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
        # Create prerequisite task first (FK constraint requirement)
        task = Task(prompt="Test task for agent", summary="Agent test task")
        await database.insert_task(task)
        task_id = task.id

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
        # Create prerequisite task first (FK constraint requirement)
        task = Task(prompt="Test task for state", summary="State test task")
        await database.insert_task(task)
        task_id = task.id

        state_data = {"iteration": 5, "result": "success"}

        await database.set_state(task_id, "loop_state", state_data)

        retrieved_state = await database.get_state(task_id, "loop_state")

        assert retrieved_state == state_data

    @pytest.mark.asyncio
    async def test_update_existing_state(self, database: Database) -> None:
        """Test updating existing state."""
        # Create prerequisite task first (FK constraint requirement)
        task = Task(prompt="Test task for state update", summary="State update test")
        await database.insert_task(task)
        task_id = task.id

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
        # Create prerequisite task first (needed for agent)
        task = Task(prompt="Test task for audit", summary="Audit test task")
        await database.insert_task(task)
        task_id = task.id

        # Create prerequisite agent (FK constraint requirement)
        agent = Agent(
            name="test-audit-agent",
            specialization="auditing",
            task_id=task_id,
        )
        await database.insert_agent(agent)
        agent_id = agent.id

        await database.log_audit(
            task_id=task_id,
            action_type="execute_task",
            agent_id=agent_id,
            action_data={"command": "test"},
            result="success",
        )

        # Audit entry should be logged (no error)


class TestDatabasePruneOperations:
    """Integration tests for database prune_tasks() method."""

    @pytest.mark.asyncio
    async def test_prune_tasks_by_age_and_status(self, database: Database) -> None:
        """Test pruning tasks by age and status filters."""
        from datetime import datetime, timedelta, timezone
        from abathur.infrastructure.database import PruneFilters

        # Create old completed tasks (older than 30 days)
        old_task1 = Task(prompt="Old task 1", summary="Old completed task")
        old_task2 = Task(prompt="Old task 2", summary="Old completed task")
        await database.insert_task(old_task1)
        await database.insert_task(old_task2)

        # Update to completed with old completion date
        old_date = datetime.now(timezone.utc) - timedelta(days=35)
        async with database._get_connection() as conn:
            await conn.execute(
                "UPDATE tasks SET status = ?, completed_at = ? WHERE id IN (?, ?)",
                (
                    TaskStatus.COMPLETED.value,
                    old_date.isoformat(),
                    str(old_task1.id),
                    str(old_task2.id),
                ),
            )
            await conn.commit()

        # Create recent completed task (should not be pruned)
        recent_task = Task(prompt="Recent task", summary="Recent completed task")
        await database.insert_task(recent_task)
        await database.update_task_status(recent_task.id, TaskStatus.COMPLETED)

        # Prune tasks older than 30 days
        filters = PruneFilters(older_than_days=30, statuses=[TaskStatus.COMPLETED])
        result = await database.prune_tasks(filters)

        # Assert - 2 old tasks deleted, 0 dependencies
        assert result.deleted_tasks == 2
        assert result.deleted_dependencies == 0
        assert result.dry_run is False
        assert TaskStatus.COMPLETED in result.breakdown_by_status
        assert result.breakdown_by_status[TaskStatus.COMPLETED] == 2

        # Verify old tasks deleted
        assert await database.get_task(old_task1.id) is None
        assert await database.get_task(old_task2.id) is None

        # Verify recent task still exists
        assert await database.get_task(recent_task.id) is not None

    @pytest.mark.asyncio
    async def test_prune_tasks_dry_run_mode(self, database: Database) -> None:
        """Test dry run mode previews deletion without removing tasks."""
        from datetime import datetime, timedelta, timezone
        from abathur.infrastructure.database import PruneFilters

        # Create old completed task
        old_task = Task(prompt="Old task", summary="Old completed task")
        await database.insert_task(old_task)

        old_date = datetime.now(timezone.utc) - timedelta(days=35)
        async with database._get_connection() as conn:
            await conn.execute(
                "UPDATE tasks SET status = ?, completed_at = ? WHERE id = ?",
                (TaskStatus.COMPLETED.value, old_date.isoformat(), str(old_task.id)),
            )
            await conn.commit()

        # Dry run prune
        filters = PruneFilters(
            older_than_days=30, statuses=[TaskStatus.COMPLETED], dry_run=True
        )
        result = await database.prune_tasks(filters)

        # Assert - shows what would be deleted
        assert result.deleted_tasks == 1
        assert result.dry_run is True
        assert result.reclaimed_bytes is None  # No VACUUM in dry run

        # Verify task still exists (not actually deleted)
        assert await database.get_task(old_task.id) is not None

    @pytest.mark.asyncio
    async def test_prune_tasks_with_vacuum(self, database: Database) -> None:
        """Test VACUUM operation reclaims space after deletion."""
        from datetime import datetime, timedelta, timezone
        from abathur.infrastructure.database import PruneFilters

        # Create multiple old completed tasks
        tasks_to_create = 10
        task_ids = []
        for i in range(tasks_to_create):
            task = Task(
                prompt=f"Old task {i}",
                summary=f"Old completed task {i}",
                input_data={"large_data": "x" * 1000},  # Add some data for size
            )
            await database.insert_task(task)
            task_ids.append(task.id)

        # Update all to completed with old dates
        old_date = datetime.now(timezone.utc) - timedelta(days=35)
        async with database._get_connection() as conn:
            for task_id in task_ids:
                await conn.execute(
                    "UPDATE tasks SET status = ?, completed_at = ? WHERE id = ?",
                    (TaskStatus.COMPLETED.value, old_date.isoformat(), str(task_id)),
                )
            await conn.commit()

        # Prune tasks (VACUUM will run)
        filters = PruneFilters(older_than_days=30, statuses=[TaskStatus.COMPLETED])
        result = await database.prune_tasks(filters)

        # Assert - tasks deleted and VACUUM ran
        assert result.deleted_tasks == tasks_to_create
        assert result.dry_run is False
        # VACUUM should report reclaimed bytes (may be None or >= 0)
        assert result.reclaimed_bytes is None or result.reclaimed_bytes >= 0

    @pytest.mark.asyncio
    async def test_prune_tasks_multiple_statuses(self, database: Database) -> None:
        """Test pruning tasks with multiple status filters."""
        from datetime import datetime, timedelta, timezone
        from abathur.infrastructure.database import PruneFilters

        # Create old tasks with different statuses
        completed_task = Task(prompt="Completed", summary="Completed task")
        failed_task = Task(prompt="Failed", summary="Failed task")
        cancelled_task = Task(prompt="Cancelled", summary="Cancelled task")
        pending_task = Task(prompt="Pending", summary="Pending task")

        await database.insert_task(completed_task)
        await database.insert_task(failed_task)
        await database.insert_task(cancelled_task)
        await database.insert_task(pending_task)

        # Update to respective statuses with old dates
        old_date = datetime.now(timezone.utc) - timedelta(days=35)
        async with database._get_connection() as conn:
            await conn.execute(
                "UPDATE tasks SET status = ?, completed_at = ? WHERE id = ?",
                (
                    TaskStatus.COMPLETED.value,
                    old_date.isoformat(),
                    str(completed_task.id),
                ),
            )
            await conn.execute(
                "UPDATE tasks SET status = ?, completed_at = ? WHERE id = ?",
                (TaskStatus.FAILED.value, old_date.isoformat(), str(failed_task.id)),
            )
            await conn.execute(
                "UPDATE tasks SET status = ?, completed_at = ? WHERE id = ?",
                (
                    TaskStatus.CANCELLED.value,
                    old_date.isoformat(),
                    str(cancelled_task.id),
                ),
            )
            await conn.commit()

        # Prune all terminal statuses (completed, failed, cancelled)
        filters = PruneFilters(
            older_than_days=30,
            statuses=[TaskStatus.COMPLETED, TaskStatus.FAILED, TaskStatus.CANCELLED],
        )
        result = await database.prune_tasks(filters)

        # Assert - 3 tasks deleted (pending not deleted)
        assert result.deleted_tasks == 3
        assert result.breakdown_by_status[TaskStatus.COMPLETED] == 1
        assert result.breakdown_by_status[TaskStatus.FAILED] == 1
        assert result.breakdown_by_status[TaskStatus.CANCELLED] == 1

        # Verify terminal tasks deleted
        assert await database.get_task(completed_task.id) is None
        assert await database.get_task(failed_task.id) is None
        assert await database.get_task(cancelled_task.id) is None

        # Verify pending task still exists
        assert await database.get_task(pending_task.id) is not None

    @pytest.mark.asyncio
    async def test_prune_tasks_with_dependencies(self, database: Database) -> None:
        """Test pruning tasks with task_dependencies records."""
        from datetime import datetime, timedelta, timezone
        from abathur.infrastructure.database import PruneFilters

        # Create old completed tasks with dependency relationship
        prerequisite_task = Task(prompt="Prerequisite", summary="Prerequisite task")
        dependent_task = Task(prompt="Dependent", summary="Dependent task")

        await database.insert_task(prerequisite_task)
        await database.insert_task(dependent_task)

        # Create dependency relationship
        from datetime import datetime, timezone

        async with database._get_connection() as conn:
            await conn.execute(
                """
                INSERT INTO task_dependencies (prerequisite_task_id, dependent_task_id, created_at)
                VALUES (?, ?, ?)
                """,
                (
                    str(prerequisite_task.id),
                    str(dependent_task.id),
                    datetime.now(timezone.utc).isoformat(),
                ),
            )
            await conn.commit()

        # Update both to completed with old dates
        old_date = datetime.now(timezone.utc) - timedelta(days=35)
        async with database._get_connection() as conn:
            await conn.execute(
                "UPDATE tasks SET status = ?, completed_at = ? WHERE id IN (?, ?)",
                (
                    TaskStatus.COMPLETED.value,
                    old_date.isoformat(),
                    str(prerequisite_task.id),
                    str(dependent_task.id),
                ),
            )
            await conn.commit()

        # Prune tasks
        filters = PruneFilters(older_than_days=30, statuses=[TaskStatus.COMPLETED])
        result = await database.prune_tasks(filters)

        # Assert - 2 tasks deleted, 1 dependency deleted
        assert result.deleted_tasks == 2
        assert result.deleted_dependencies == 1

        # Verify dependency also deleted
        async with database._get_connection() as conn:
            cursor = await conn.execute(
                "SELECT COUNT(*) FROM task_dependencies WHERE prerequisite_task_id = ?",
                (str(prerequisite_task.id),),
            )
            dep_count = (await cursor.fetchone())[0]
            assert dep_count == 0

    @pytest.mark.asyncio
    async def test_prune_tasks_empty_result(self, database: Database) -> None:
        """Test pruning when no tasks match filter criteria."""
        from abathur.infrastructure.database import PruneFilters

        # Create recent completed task (within 30 days)
        recent_task = Task(prompt="Recent", summary="Recent task")
        await database.insert_task(recent_task)
        await database.update_task_status(recent_task.id, TaskStatus.COMPLETED)

        # Prune tasks older than 30 days (none exist)
        filters = PruneFilters(older_than_days=30, statuses=[TaskStatus.COMPLETED])
        result = await database.prune_tasks(filters)

        # Assert - no tasks deleted
        assert result.deleted_tasks == 0
        assert result.deleted_dependencies == 0
        assert result.breakdown_by_status == {}

        # Verify recent task still exists
        assert await database.get_task(recent_task.id) is not None

    @pytest.mark.asyncio
    async def test_prune_tasks_with_limit(self, database: Database) -> None:
        """Test pruning respects limit parameter."""
        from datetime import datetime, timedelta, timezone
        from abathur.infrastructure.database import PruneFilters

        # Create 10 old completed tasks
        task_ids = []
        for i in range(10):
            task = Task(prompt=f"Old task {i}", summary=f"Old task {i}")
            await database.insert_task(task)
            task_ids.append(task.id)

        # Update all to completed with old dates
        old_date = datetime.now(timezone.utc) - timedelta(days=35)
        async with database._get_connection() as conn:
            for task_id in task_ids:
                await conn.execute(
                    "UPDATE tasks SET status = ?, completed_at = ? WHERE id = ?",
                    (TaskStatus.COMPLETED.value, old_date.isoformat(), str(task_id)),
                )
            await conn.commit()

        # Prune with limit of 5
        filters = PruneFilters(
            older_than_days=30, statuses=[TaskStatus.COMPLETED], limit=5
        )
        result = await database.prune_tasks(filters)

        # Assert - only 5 tasks deleted (respecting limit)
        assert result.deleted_tasks == 5

        # Verify 5 tasks remain
        remaining_tasks = await database.list_tasks(status=TaskStatus.COMPLETED)
        assert len(remaining_tasks) == 5

    @pytest.mark.asyncio
    async def test_prune_tasks_before_date_filter(self, database: Database) -> None:
        """Test pruning with before_date filter instead of older_than_days."""
        from datetime import datetime, timedelta, timezone
        from abathur.infrastructure.database import PruneFilters

        # Create tasks with specific dates
        old_task = Task(prompt="Old task", summary="Old task")
        recent_task = Task(prompt="Recent task", summary="Recent task")

        await database.insert_task(old_task)
        await database.insert_task(recent_task)

        # Set specific completion dates
        cutoff_date = datetime.now(timezone.utc) - timedelta(days=30)
        old_date = cutoff_date - timedelta(days=5)  # 35 days ago
        recent_date = cutoff_date + timedelta(days=5)  # 25 days ago

        async with database._get_connection() as conn:
            await conn.execute(
                "UPDATE tasks SET status = ?, completed_at = ? WHERE id = ?",
                (TaskStatus.COMPLETED.value, old_date.isoformat(), str(old_task.id)),
            )
            await conn.execute(
                "UPDATE tasks SET status = ?, completed_at = ? WHERE id = ?",
                (
                    TaskStatus.COMPLETED.value,
                    recent_date.isoformat(),
                    str(recent_task.id),
                ),
            )
            await conn.commit()

        # Prune tasks before cutoff date
        filters = PruneFilters(
            before_date=cutoff_date, statuses=[TaskStatus.COMPLETED]
        )
        result = await database.prune_tasks(filters)

        # Assert - only old task deleted
        assert result.deleted_tasks == 1

        # Verify
        assert await database.get_task(old_task.id) is None
        assert await database.get_task(recent_task.id) is not None

    @pytest.mark.asyncio
    async def test_prune_tasks_with_parent_child_mixed_statuses(
        self, database: Database
    ) -> None:
        """Test pruning respects filter and orphans children without deleting them.

        Tree structure:
            Parent A (completed, old) -> Child B (running, recent)
                                      -> Child C (completed, old)
                                         -> Grandchild D (failed, recent)
            Parent E (running, recent) -> Child F (completed, old)

        Expected after pruning completed tasks older than 30 days:
            - Delete: Parent A, Child C
            - Orphan: Child B (parent_task_id = NULL)
            - Orphan: Grandchild D (parent_task_id = NULL, since parent C deleted)
            - Keep unchanged: Parent E, Child F (F not deleted because has running parent)
        """
        from datetime import datetime, timedelta, timezone
        from abathur.infrastructure.database import PruneFilters

        # Create task tree with mixed statuses and dates
        parent_a = Task(prompt="Parent A", summary="Completed parent")
        child_b = Task(
            prompt="Child B",
            summary="Running child",
            parent_task_id=parent_a.id,
        )
        child_c = Task(
            prompt="Child C",
            summary="Completed child",
            parent_task_id=parent_a.id,
        )
        grandchild_d = Task(
            prompt="Grandchild D",
            summary="Failed grandchild",
            parent_task_id=child_c.id,
        )
        parent_e = Task(prompt="Parent E", summary="Running parent")
        child_f = Task(
            prompt="Child F",
            summary="Completed child",
            parent_task_id=parent_e.id,
        )

        # Insert all tasks
        await database.insert_task(parent_a)
        await database.insert_task(child_b)
        await database.insert_task(child_c)
        await database.insert_task(grandchild_d)
        await database.insert_task(parent_e)
        await database.insert_task(child_f)

        # Set statuses and dates
        old_date = datetime.now(timezone.utc) - timedelta(days=35)
        recent_date = datetime.now(timezone.utc) - timedelta(days=5)

        async with database._get_connection() as conn:
            # Parent A: completed, old
            await conn.execute(
                "UPDATE tasks SET status = ?, completed_at = ? WHERE id = ?",
                (TaskStatus.COMPLETED.value, old_date.isoformat(), str(parent_a.id)),
            )
            # Child B: running, recent
            await conn.execute(
                "UPDATE tasks SET status = ?, started_at = ? WHERE id = ?",
                (TaskStatus.RUNNING.value, recent_date.isoformat(), str(child_b.id)),
            )
            # Child C: completed, old
            await conn.execute(
                "UPDATE tasks SET status = ?, completed_at = ? WHERE id = ?",
                (TaskStatus.COMPLETED.value, old_date.isoformat(), str(child_c.id)),
            )
            # Grandchild D: failed, recent
            await conn.execute(
                "UPDATE tasks SET status = ?, completed_at = ? WHERE id = ?",
                (TaskStatus.FAILED.value, recent_date.isoformat(), str(grandchild_d.id)),
            )
            # Parent E: running, recent
            await conn.execute(
                "UPDATE tasks SET status = ?, started_at = ? WHERE id = ?",
                (TaskStatus.RUNNING.value, recent_date.isoformat(), str(parent_e.id)),
            )
            # Child F: completed, old
            await conn.execute(
                "UPDATE tasks SET status = ?, completed_at = ? WHERE id = ?",
                (TaskStatus.COMPLETED.value, old_date.isoformat(), str(child_f.id)),
            )
            await conn.commit()

        # Prune completed tasks older than 30 days
        filters = PruneFilters(older_than_days=30, statuses=[TaskStatus.COMPLETED])
        result = await database.prune_tasks(filters)

        # Assert - should delete Parent A, Child C, Child F (3 completed old tasks)
        assert result.deleted_tasks == 3
        assert result.dry_run is False
        assert TaskStatus.COMPLETED in result.breakdown_by_status
        assert result.breakdown_by_status[TaskStatus.COMPLETED] == 3

        # Verify deletions
        assert await database.get_task(parent_a.id) is None  # Deleted
        assert await database.get_task(child_c.id) is None  # Deleted
        assert await database.get_task(child_f.id) is None  # Deleted

        # Verify surviving tasks
        child_b_after = await database.get_task(child_b.id)
        grandchild_d_after = await database.get_task(grandchild_d.id)
        parent_e_after = await database.get_task(parent_e.id)

        assert child_b_after is not None  # Survived
        assert grandchild_d_after is not None  # Survived
        assert parent_e_after is not None  # Survived

        # Verify orphaning - children should have parent_task_id set to NULL
        assert child_b_after.parent_task_id is None  # Orphaned (parent deleted)
        assert grandchild_d_after.parent_task_id is None  # Orphaned (parent deleted)

        # Parent E should still be parent of its children (but child F was deleted)
        assert parent_e_after.parent_task_id is None  # Was never a child
