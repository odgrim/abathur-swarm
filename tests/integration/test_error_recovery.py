"""Integration tests for error recovery in delete operations.

Tests error recovery scenarios for database delete_tasks() operations:
- Partial deletion success when some tasks deletable, others blocked
- Proper reporting of blocked deletions with parent-child relationships
- DeleteResult structure validation for mixed success/failure cases
"""

import asyncio
from collections.abc import AsyncGenerator
from datetime import datetime, timezone
from pathlib import Path
from uuid import UUID, uuid4

import pytest
from abathur.domain.models import (
    DependencyType,
    Task,
    TaskDependency,
    TaskSource,
    TaskStatus,
)
from abathur.infrastructure.database import Database


@pytest.fixture
async def memory_db() -> AsyncGenerator[Database, None]:
    """Create in-memory database for fast integration tests."""
    db = Database(Path(":memory:"))
    await db.initialize()
    yield db
    await db.close()


class TestPartialDeletionFailureHandling:
    """Test error recovery for partial deletion scenarios."""

    @pytest.mark.asyncio
    async def test_delete_tasks_partial_failure_handling(
        self, memory_db: Database
    ):
        """Test error recovery when attempting to delete mix of deletable and blocked tasks.

        Scenario:
        1. Create parent task with child (cannot delete parent)
        2. Create standalone task (can delete individually)
        3. Attempt to delete both parent and standalone together
        4. Verify all-or-nothing behavior: ALL deletions blocked due to parent
        5. Verify DeleteResult structure reports blocked deletions correctly

        Expected result (current implementation - all-or-nothing):
        - deleted_count == 0 (no deletions when any task has children)
        - blocked_deletions contains parent with child_ids
        - errors contains appropriate error message
        - Both tasks still exist in database
        """
        # Step 1: Create parent task with child
        parent_task = Task(
            id=uuid4(),
            prompt="Parent task",
            agent_type="test-agent",
            priority=5,
            status=TaskStatus.COMPLETED,
            input_data={},
            submitted_at=datetime.now(timezone.utc),
            last_updated_at=datetime.now(timezone.utc),
            source=TaskSource.HUMAN,
            dependency_type=DependencyType.SEQUENTIAL,
            summary="Parent task with child",
        )

        child_task = Task(
            id=uuid4(),
            prompt="Child task",
            agent_type="test-agent",
            priority=5,
            status=TaskStatus.PENDING,
            input_data={},
            parent_task_id=parent_task.id,  # Link to parent
            submitted_at=datetime.now(timezone.utc),
            last_updated_at=datetime.now(timezone.utc),
            source=TaskSource.HUMAN,
            dependency_type=DependencyType.SEQUENTIAL,
            summary="Child task of parent",
        )

        await memory_db.insert_task(parent_task)
        await memory_db.insert_task(child_task)

        # Step 2: Create standalone task (no children, can be deleted)
        standalone_task = Task(
            id=uuid4(),
            prompt="Standalone task",
            agent_type="test-agent",
            priority=5,
            status=TaskStatus.COMPLETED,
            input_data={},
            submitted_at=datetime.now(timezone.utc),
            last_updated_at=datetime.now(timezone.utc),
            source=TaskSource.HUMAN,
            dependency_type=DependencyType.SEQUENTIAL,
            summary="Standalone task - no children",
        )

        await memory_db.insert_task(standalone_task)

        # Step 3: Attempt to delete both parent and standalone
        result = await memory_db.delete_tasks([parent_task.id, standalone_task.id])

        # Step 4: Verify all-or-nothing behavior - NO tasks deleted when ANY has children
        assert result["deleted_count"] == 0, \
            "All deletions should be blocked when any task has children"

        # Step 5: Verify blocked_deletions contains parent
        assert len(result["blocked_deletions"]) == 1, "Parent should be in blocked list"
        blocked = result["blocked_deletions"][0]

        # Step 6: Verify blocked deletion structure
        assert "task_id" in blocked, "Blocked deletion must contain task_id"
        assert "child_ids" in blocked, "Blocked deletion must contain child_ids"
        assert blocked["task_id"] == str(parent_task.id), "Parent task should be blocked"
        assert str(child_task.id) in blocked["child_ids"], "Child ID should be listed"
        assert len(blocked["child_ids"]) == 1, "Should report exactly 1 child"

        # Step 7: Verify errors list contains appropriate message
        assert len(result["errors"]) > 0, "Should have error message for blocked deletion"
        error_message = result["errors"][0].lower()
        assert "child" in error_message or "dependent" in error_message, \
            "Error message should mention child tasks"

        # Step 8: Verify standalone was NOT deleted (all-or-nothing behavior)
        standalone_retrieved = await memory_db.get_task(standalone_task.id)
        assert standalone_retrieved is not None, \
            "Standalone task should still exist (all deletions blocked)"

        # Step 9: Verify parent still exists (was not deleted)
        parent_retrieved = await memory_db.get_task(parent_task.id)
        assert parent_retrieved is not None, "Parent task should still exist"

        # Step 10: Verify child still exists (was not deleted)
        child_retrieved = await memory_db.get_task(child_task.id)
        assert child_retrieved is not None, "Child task should still exist"
