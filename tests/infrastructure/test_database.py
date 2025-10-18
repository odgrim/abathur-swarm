"""Unit tests for Database deletion methods.

Tests individual database deletion operations in isolation:
- delete_task() - single task deletion
- delete_tasks_by_status() - bulk deletion by status filter
- CASCADE DELETE behavior on task_dependencies
- Edge cases and error scenarios
"""

from collections.abc import AsyncGenerator
from pathlib import Path
from tempfile import TemporaryDirectory
from uuid import uuid4

import pytest
from abathur.domain.models import DependencyType, Task, TaskDependency, TaskStatus
from abathur.infrastructure.database import Database


@pytest.fixture
async def database() -> AsyncGenerator[Database, None]:
    """Create a test database."""
    with TemporaryDirectory() as tmpdir:
        db_path = Path(tmpdir) / "test.db"
        db = Database(db_path)
        await db.initialize()
        yield db


class TestDeleteTask:
    """Unit tests for Database.delete_task() method."""

    @pytest.mark.asyncio
    async def test_delete_task_success(self, database: Database) -> None:
        """Test successful deletion of existing task."""
        # Arrange - create a task
        task = Task(prompt="Task to delete", summary="Delete test")
        await database.insert_task(task)

        # Verify task exists
        retrieved = await database.get_task(task.id)
        assert retrieved is not None

        # Act - delete the task
        result = await database.delete_task(task.id)

        # Assert - returns True and task is deleted
        assert result is True
        assert await database.get_task(task.id) is None

    @pytest.mark.asyncio
    async def test_delete_task_not_found(self, database: Database) -> None:
        """Test deletion of non-existent task returns False."""
        # Arrange - non-existent UUID
        non_existent_id = uuid4()

        # Act - try to delete non-existent task
        result = await database.delete_task(non_existent_id)

        # Assert - returns False
        assert result is False

    @pytest.mark.asyncio
    async def test_delete_task_cascade_dependencies(self, database: Database) -> None:
        """Test CASCADE DELETE removes task_dependencies rows when foreign keys enabled.

        NOTE: delete_task() does NOT explicitly enable foreign keys, so CASCADE DELETE
        behavior depends on connection settings. This test verifies the behavior when
        foreign keys ARE enabled. For guaranteed CASCADE behavior, use delete_task_by_id()
        or delete_tasks_by_status() which explicitly enable foreign keys.
        """
        # Arrange - create tasks with dependencies
        task1 = Task(prompt="Task 1", summary="Prerequisite")
        task2 = Task(prompt="Task 2", summary="Dependent")
        await database.insert_task(task1)
        await database.insert_task(task2)

        # Create dependency: task2 depends on task1
        dep = TaskDependency(
            dependent_task_id=task2.id,
            prerequisite_task_id=task1.id,
            dependency_type=DependencyType.SEQUENTIAL,
        )
        await database.insert_task_dependency(dep)

        # Verify dependency exists
        async with database._get_connection() as conn:
            cursor = await conn.execute("SELECT COUNT(*) FROM task_dependencies WHERE id = ?", (str(dep.id),))
            row = await cursor.fetchone()
            assert row is not None
            assert row[0] == 1

        # Act - delete task1 (prerequisite)
        # Note: Must manually enable foreign keys for CASCADE to work
        async with database._get_connection() as conn:
            await conn.execute("PRAGMA foreign_keys = ON")
            cursor = await conn.execute("DELETE FROM tasks WHERE id = ?", (str(task1.id),))
            await conn.commit()
            deleted = cursor.rowcount > 0

        # Assert - task1 deleted, CASCADE removes dependency
        assert deleted is True
        assert await database.get_task(task1.id) is None

        # Verify dependency was CASCADE deleted
        async with database._get_connection() as conn:
            cursor = await conn.execute("SELECT COUNT(*) FROM task_dependencies WHERE id = ?", (str(dep.id),))
            row = await cursor.fetchone()
            assert row is not None
            assert row[0] == 0  # Dependency was CASCADE deleted


class TestDeleteTasksByStatus:
    """Unit tests for Database.delete_tasks_by_status() method."""

    @pytest.mark.asyncio
    async def test_delete_tasks_by_status_completed(self, database: Database) -> None:
        """Test deletion of all completed tasks."""
        # Arrange - create tasks with different statuses
        task1 = Task(prompt="Task 1", summary="Completed 1")
        task2 = Task(prompt="Task 2", summary="Completed 2")
        task3 = Task(prompt="Task 3", summary="Pending")

        await database.insert_task(task1)
        await database.insert_task(task2)
        await database.insert_task(task3)

        # Update task1 and task2 to completed
        await database.update_task_status(task1.id, TaskStatus.COMPLETED)
        await database.update_task_status(task2.id, TaskStatus.COMPLETED)

        # Act - delete all completed tasks
        result = await database.delete_tasks_by_status(TaskStatus.COMPLETED)

        # Assert - returns 2, both completed tasks deleted
        assert result.deleted_tasks == 2

        # Verify completed tasks are deleted
        assert await database.get_task(task1.id) is None
        assert await database.get_task(task2.id) is None

        # Verify pending task still exists
        assert await database.get_task(task3.id) is not None

    @pytest.mark.asyncio
    async def test_delete_tasks_by_status_failed(self, database: Database) -> None:
        """Test deletion of all failed tasks."""
        # Arrange - create tasks with different statuses
        task1 = Task(prompt="Task 1", summary="Failed 1")
        task2 = Task(prompt="Task 2", summary="Failed 2")
        task3 = Task(prompt="Task 3", summary="Completed")

        await database.insert_task(task1)
        await database.insert_task(task2)
        await database.insert_task(task3)

        # Update task1 and task2 to failed
        await database.update_task_status(task1.id, TaskStatus.FAILED, error_message="Test error")
        await database.update_task_status(task2.id, TaskStatus.FAILED, error_message="Another error")

        # Update task3 to completed
        await database.update_task_status(task3.id, TaskStatus.COMPLETED)

        # Act - delete all failed tasks
        result = await database.delete_tasks_by_status(TaskStatus.FAILED)

        # Assert - returns 2, both failed tasks deleted
        assert result.deleted_tasks == 2

        # Verify failed tasks are deleted
        assert await database.get_task(task1.id) is None
        assert await database.get_task(task2.id) is None

        # Verify completed task still exists
        assert await database.get_task(task3.id) is not None

    @pytest.mark.asyncio
    async def test_delete_tasks_by_status_empty_result(self, database: Database) -> None:
        """Test deletion when no tasks match status (returns 0)."""
        # Arrange - create only pending tasks
        task = Task(prompt="Pending task", summary="Pending")
        await database.insert_task(task)

        # Act - try to delete completed tasks (none exist)
        result = await database.delete_tasks_by_status(TaskStatus.COMPLETED)

        # Assert - returns 0
        assert result.deleted_tasks == 0

        # Verify pending task still exists
        assert await database.get_task(task.id) is not None

    @pytest.mark.asyncio
    async def test_delete_tasks_by_status_cascade_dependencies(self, database: Database) -> None:
        """Test CASCADE DELETE removes task_dependencies for all deleted tasks."""
        # Arrange - create tasks with dependencies
        task1 = Task(prompt="Task 1", summary="Completed 1")
        task2 = Task(prompt="Task 2", summary="Completed 2")
        task3 = Task(prompt="Task 3", summary="Pending")

        await database.insert_task(task1)
        await database.insert_task(task2)
        await database.insert_task(task3)

        # Update task1 and task2 to completed
        await database.update_task_status(task1.id, TaskStatus.COMPLETED)
        await database.update_task_status(task2.id, TaskStatus.COMPLETED)

        # Create dependencies
        dep1 = TaskDependency(
            dependent_task_id=task2.id,
            prerequisite_task_id=task1.id,
            dependency_type=DependencyType.SEQUENTIAL,
        )
        dep2 = TaskDependency(
            dependent_task_id=task3.id,
            prerequisite_task_id=task2.id,
            dependency_type=DependencyType.SEQUENTIAL,
        )

        await database.insert_task_dependency(dep1)
        await database.insert_task_dependency(dep2)

        # Verify both dependencies exist
        async with database._get_connection() as conn:
            cursor = await conn.execute("SELECT COUNT(*) FROM task_dependencies")
            row = await cursor.fetchone()
            assert row is not None
            assert row[0] == 2

        # Act - delete all completed tasks (task1 and task2)
        result = await database.delete_tasks_by_status(TaskStatus.COMPLETED)

        # Assert - 2 tasks deleted
        assert result.deleted_tasks == 2

        # Verify CASCADE deleted both dependencies (dep1 and dep2)
        # dep1: task2 -> task1 (both deleted)
        # dep2: task3 -> task2 (task2 deleted, so dependency removed)
        async with database._get_connection() as conn:
            cursor = await conn.execute("SELECT COUNT(*) FROM task_dependencies")
            row = await cursor.fetchone()
            assert row is not None
            assert row[0] == 0  # All dependencies CASCADE deleted

    @pytest.mark.asyncio
    async def test_delete_tasks_by_status_multiple_statuses(self, database: Database) -> None:
        """Test deletion only affects tasks with exact status match."""
        # Arrange - create tasks with various statuses
        task_pending = Task(prompt="Pending", summary="Pending")
        task_running = Task(prompt="Running", summary="Running")
        task_completed = Task(prompt="Completed", summary="Completed")
        task_failed = Task(prompt="Failed", summary="Failed")

        await database.insert_task(task_pending)
        await database.insert_task(task_running)
        await database.insert_task(task_completed)
        await database.insert_task(task_failed)

        # Update statuses
        await database.update_task_status(task_running.id, TaskStatus.RUNNING)
        await database.update_task_status(task_completed.id, TaskStatus.COMPLETED)
        await database.update_task_status(task_failed.id, TaskStatus.FAILED)

        # Act - delete only completed tasks
        result = await database.delete_tasks_by_status(TaskStatus.COMPLETED)

        # Assert - only 1 task deleted
        assert result.deleted_tasks == 1

        # Verify only completed task is deleted
        assert await database.get_task(task_completed.id) is None

        # Verify all other tasks still exist
        assert await database.get_task(task_pending.id) is not None
        assert await database.get_task(task_running.id) is not None
        assert await database.get_task(task_failed.id) is not None

    @pytest.mark.asyncio
    async def test_delete_tasks_by_status_with_agents_and_checkpoints(self, database: Database) -> None:
        """Test CASCADE deletes agents and checkpoints for deleted tasks."""
        # Arrange - create completed tasks with related records
        from abathur.domain.models import Agent

        task1 = Task(prompt="Task 1", summary="Completed with relations")
        task2 = Task(prompt="Task 2", summary="Pending")

        await database.insert_task(task1)
        await database.insert_task(task2)

        # Update task1 to completed
        await database.update_task_status(task1.id, TaskStatus.COMPLETED)

        # Create agent for task1
        agent = Agent(
            name="test-agent",
            specialization="testing",
            task_id=task1.id,
        )
        await database.insert_agent(agent)

        # Create checkpoint for task1
        async with database._get_connection() as conn:
            await conn.execute(
                """
                INSERT INTO checkpoints (task_id, iteration, state, created_at)
                VALUES (?, ?, ?, ?)
                """,
                (str(task1.id), 1, '{"test": "data"}', "2024-01-01T00:00:00"),
            )
            await conn.commit()

        # Verify agent and checkpoint exist
        async with database._get_connection() as conn:
            cursor = await conn.execute("SELECT COUNT(*) FROM agents WHERE task_id = ?", (str(task1.id),))
            row = await cursor.fetchone()
            assert row is not None
            assert row[0] == 1

            cursor = await conn.execute("SELECT COUNT(*) FROM checkpoints WHERE task_id = ?", (str(task1.id),))
            row = await cursor.fetchone()
            assert row is not None
            assert row[0] == 1

        # Act - delete all completed tasks
        result = await database.delete_tasks_by_status(TaskStatus.COMPLETED)

        # Assert - 1 task deleted
        assert result.deleted_tasks == 1
        assert await database.get_task(task1.id) is None

        # Verify CASCADE deleted agent and checkpoint
        async with database._get_connection() as conn:
            cursor = await conn.execute("SELECT COUNT(*) FROM agents WHERE task_id = ?", (str(task1.id),))
            row = await cursor.fetchone()
            assert row is not None
            assert row[0] == 0  # Agent CASCADE deleted

            cursor = await conn.execute("SELECT COUNT(*) FROM checkpoints WHERE task_id = ?", (str(task1.id),))
            row = await cursor.fetchone()
            assert row is not None
            assert row[0] == 0  # Checkpoint CASCADE deleted
