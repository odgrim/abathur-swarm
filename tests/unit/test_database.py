"""Unit tests for Database.delete_tasks() bulk deletion method with child task validation."""

from collections.abc import AsyncGenerator
from pathlib import Path
from tempfile import TemporaryDirectory
from uuid import uuid4

import pytest
from abathur.domain.models import DependencyType, Task, TaskDependency
from abathur.infrastructure.database import Database


@pytest.fixture
async def database() -> AsyncGenerator[Database, None]:
    """Create a test database."""
    with TemporaryDirectory() as tmpdir:
        db_path = Path(tmpdir) / "test.db"
        db = Database(db_path)
        await db.initialize()
        yield db


class TestDeleteTasks:
    """Tests for Database.delete_tasks() bulk deletion method."""

    @pytest.mark.asyncio
    async def test_delete_single_task(self, database: Database) -> None:
        """Test delete_tasks() with a single task."""
        task = Task(prompt="Task to delete", summary="Single delete test")
        await database.insert_task(task)

        # Verify task exists
        retrieved_task = await database.get_task(task.id)
        assert retrieved_task is not None

        # Delete the task
        result = await database.delete_tasks([task.id])
        assert result["deleted_count"] == 1
        assert result["blocked_deletions"] == []
        assert result["errors"] == []

        # Verify task is deleted
        deleted_task = await database.get_task(task.id)
        assert deleted_task is None

    @pytest.mark.asyncio
    async def test_delete_multiple_tasks(self, database: Database) -> None:
        """Test delete_tasks() with multiple tasks in bulk."""
        # Create 15 tasks
        tasks = [Task(prompt=f"Task {i}", summary=f"Bulk test {i}") for i in range(15)]
        for task in tasks:
            await database.insert_task(task)

        # Verify all tasks exist
        all_tasks = await database.list_tasks(limit=20)
        assert len(all_tasks) == 15

        # Delete all tasks in single transaction
        task_ids = [task.id for task in tasks]
        result = await database.delete_tasks(task_ids)
        assert result["deleted_count"] == 15
        assert result["blocked_deletions"] == []
        assert result["errors"] == []

        # Verify all tasks are deleted
        remaining_tasks = await database.list_tasks(limit=20)
        assert len(remaining_tasks) == 0

    @pytest.mark.asyncio
    async def test_delete_tasks_empty_list(self, database: Database) -> None:
        """Test delete_tasks() raises ValueError for empty list."""
        with pytest.raises(ValueError, match="task_ids cannot be empty"):
            await database.delete_tasks([])

    @pytest.mark.asyncio
    async def test_delete_nonexistent_task_succeeds(self, database: Database) -> None:
        """Test delete_tasks() with non-existent UUIDs returns 0."""
        non_existent_ids = [uuid4(), uuid4(), uuid4()]

        # Delete non-existent tasks
        result = await database.delete_tasks(non_existent_ids)
        assert result["deleted_count"] == 0
        assert result["blocked_deletions"] == []
        assert result["errors"] == []

    @pytest.mark.asyncio
    async def test_delete_tasks_mixed_existent_nonexistent(self, database: Database) -> None:
        """Test delete_tasks() with mix of existent and non-existent UUIDs."""
        # Create 3 tasks
        task1 = Task(prompt="Task 1", summary="Mixed test 1")
        task2 = Task(prompt="Task 2", summary="Mixed test 2")
        task3 = Task(prompt="Task 3", summary="Mixed test 3")

        await database.insert_task(task1)
        await database.insert_task(task2)
        await database.insert_task(task3)

        # Mix existent and non-existent IDs
        task_ids = [task1.id, uuid4(), task2.id, uuid4(), task3.id]

        # Delete should only delete the 3 existing tasks
        result = await database.delete_tasks(task_ids)
        assert result["deleted_count"] == 3
        assert result["blocked_deletions"] == []
        assert result["errors"] == []

        # Verify only the 3 existing tasks were deleted
        assert await database.get_task(task1.id) is None
        assert await database.get_task(task2.id) is None
        assert await database.get_task(task3.id) is None

    @pytest.mark.asyncio
    async def test_delete_blocks_parent_with_children(self, database: Database) -> None:
        """Test delete_tasks() blocks deletion of parent tasks with child tasks."""
        # Create parent task
        parent = Task(prompt="Parent task", summary="Parent")
        await database.insert_task(parent)

        # Create child tasks with parent_task_id
        child1 = Task(prompt="Child 1", summary="Child 1", parent_task_id=parent.id)
        child2 = Task(prompt="Child 2", summary="Child 2", parent_task_id=parent.id)
        await database.insert_task(child1)
        await database.insert_task(child2)

        # Try to delete parent - should be blocked
        result = await database.delete_tasks([parent.id])
        assert result["deleted_count"] == 0
        assert len(result["blocked_deletions"]) == 2
        assert result["errors"] == ["Cannot delete tasks with child tasks. Delete children first."]

        # Verify parent still exists
        assert await database.get_task(parent.id) is not None

        # Verify child tasks are in blocked_deletions
        blocked_ids = {item["task_id"] for item in result["blocked_deletions"]}
        assert str(child1.id) in blocked_ids
        assert str(child2.id) in blocked_ids

    @pytest.mark.asyncio
    async def test_delete_cascades_to_dependencies(self, database: Database) -> None:
        """Test delete_tasks() CASCADE deletes task_dependencies."""
        # Create tasks
        task1 = Task(prompt="Task 1", summary="CASCADE dep test 1")
        task2 = Task(prompt="Task 2", summary="CASCADE dep test 2")
        task3 = Task(prompt="Task 3", summary="CASCADE dep test 3")

        await database.insert_task(task1)
        await database.insert_task(task2)
        await database.insert_task(task3)

        # Create dependencies: task2 depends on task1, task3 depends on task2
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

        # Verify dependencies exist
        async with database._get_connection() as conn:
            cursor = await conn.execute("SELECT COUNT(*) FROM task_dependencies")
            row = await cursor.fetchone()
            assert row is not None
            dep_count = row[0]
            assert dep_count == 2

        # Delete task1 - should CASCADE delete dep1 (where task1 is prerequisite)
        result = await database.delete_tasks([task1.id])
        assert result["deleted_count"] == 1

        # Verify only dep1 was CASCADE deleted (dep2 should remain since task2 still exists)
        async with database._get_connection() as conn:
            cursor = await conn.execute("SELECT COUNT(*) FROM task_dependencies")
            row = await cursor.fetchone()
            assert row is not None
            dep_count = row[0]
            assert dep_count == 1  # Only dep2 remains

        # Now delete all remaining tasks (task2 and task3)
        result = await database.delete_tasks([task2.id, task3.id])
        assert result["deleted_count"] == 2

        # Verify all dependencies were CASCADE deleted
        async with database._get_connection() as conn:
            cursor = await conn.execute("SELECT COUNT(*) FROM task_dependencies")
            row = await cursor.fetchone()
            assert row is not None
            dep_count = row[0]
            assert dep_count == 0  # All dependencies deleted

    @pytest.mark.asyncio
    async def test_delete_returns_correct_counts(self, database: Database) -> None:
        """Test delete_tasks() returns accurate deleted_count."""
        # Create 10 tasks
        tasks = [Task(prompt=f"Task {i}", summary=f"Count test {i}") for i in range(10)]
        for task in tasks:
            await database.insert_task(task)

        # Delete all tasks
        task_ids = [task.id for task in tasks]
        result = await database.delete_tasks(task_ids)
        assert result["deleted_count"] == 10
        assert result["blocked_deletions"] == []
        assert result["errors"] == []

        # Verify all deleted
        for task_id in task_ids:
            assert await database.get_task(task_id) is None
