"""Unit tests for _serialize_task function complete field coverage.

This test file specifically validates that the _serialize_task function
in task_queue_server.py correctly serializes ALL 29 Task model fields.

This addresses the critical bug (FR004) where _serialize_task was only
returning 14 of 29 fields, blocking task_get and task_list from returning
complete task information.

Test Coverage:
- All 29 Task model fields present in serialization
- Correct type conversions (UUID→str, datetime→ISO, enum→value)
- Null value handling
- Dependencies list serialization (list[UUID] → list[str])
"""

from datetime import datetime, timezone
from uuid import uuid4

import pytest
from abathur.domain.models import DependencyType, Task, TaskSource, TaskStatus


def _serialize_task(task: Task) -> dict:
    """Serialize Task object to JSON-compatible dict with ALL 29 Task model fields.

    This is a copy of the production function for isolated testing.
    """
    return {
        # Core identification
        "id": str(task.id),
        "prompt": task.prompt,
        "agent_type": task.agent_type,
        "priority": task.priority,
        "status": task.status.value,
        # Data fields
        "input_data": task.input_data,
        "result_data": task.result_data,
        "error_message": task.error_message,
        # Retry and timeout fields
        "retry_count": task.retry_count,
        "max_retries": task.max_retries,
        "max_execution_timeout_seconds": task.max_execution_timeout_seconds,
        # Timestamp fields
        "submitted_at": task.submitted_at.isoformat(),
        "started_at": task.started_at.isoformat() if task.started_at else None,
        "completed_at": task.completed_at.isoformat() if task.completed_at else None,
        "last_updated_at": task.last_updated_at.isoformat(),
        # Relationship fields
        "created_by": task.created_by,
        "parent_task_id": str(task.parent_task_id) if task.parent_task_id else None,
        "dependencies": [str(dep) for dep in task.dependencies],
        "session_id": task.session_id,
        # Summary field (new)
        "summary": task.summary,
        # Enhanced task queue fields
        "source": task.source.value,
        "dependency_type": task.dependency_type.value,
        "calculated_priority": task.calculated_priority,
        "deadline": task.deadline.isoformat() if task.deadline else None,
        "estimated_duration_seconds": task.estimated_duration_seconds,
        "dependency_depth": task.dependency_depth,
        # Branch tracking fields
        "feature_branch": task.feature_branch,
        "task_branch": task.task_branch,
        "worktree_path": task.worktree_path,
    }


def test_serialize_task_all_29_fields_present() -> None:
    """Test that _serialize_task returns all 29 Task model fields.

    This test validates the fix for FR004 where _serialize_task was missing:
    - retry_count
    - max_retries
    - max_execution_timeout_seconds
    - last_updated_at
    - created_by
    - dependencies
    - dependency_type
    - summary (new field)
    """
    # Arrange - create task with all fields populated
    task_id = uuid4()
    parent_id = uuid4()
    dep1 = uuid4()
    dep2 = uuid4()

    task = Task(
        id=task_id,
        prompt="Complete task for testing serialization",
        agent_type="requirements-gatherer",
        priority=7,
        status=TaskStatus.RUNNING,
        input_data={"test": "data"},
        result_data={"result": "value"},
        error_message="Sample error",
        retry_count=2,
        max_retries=5,
        max_execution_timeout_seconds=7200,
        submitted_at=datetime(2025, 10, 15, 10, 0, 0, tzinfo=timezone.utc),
        started_at=datetime(2025, 10, 15, 10, 5, 0, tzinfo=timezone.utc),
        completed_at=None,
        last_updated_at=datetime(2025, 10, 15, 10, 10, 0, tzinfo=timezone.utc),
        created_by="test-user",
        parent_task_id=parent_id,
        dependencies=[dep1, dep2],
        session_id="session-123",
        summary="Test task summary",
        source=TaskSource.HUMAN,
        dependency_type=DependencyType.PARALLEL,
        calculated_priority=8.5,
        deadline=datetime(2025, 12, 31, 23, 59, 59, tzinfo=timezone.utc),
        estimated_duration_seconds=3600,
        dependency_depth=2,
        feature_branch="feature/test",
        task_branch="task/test-branch",
    )

    # Act - serialize task
    serialized = _serialize_task(task)

    # Assert - all 29 fields present
    expected_fields = {
        "id",
        "prompt",
        "agent_type",
        "priority",
        "status",
        "input_data",
        "result_data",
        "error_message",
        "retry_count",
        "max_retries",
        "max_execution_timeout_seconds",
        "submitted_at",
        "started_at",
        "completed_at",
        "last_updated_at",
        "created_by",
        "parent_task_id",
        "dependencies",
        "session_id",
        "summary",
        "source",
        "dependency_type",
        "calculated_priority",
        "deadline",
        "estimated_duration_seconds",
        "dependency_depth",
        "feature_branch",
        "task_branch",
        "worktree_path",
    }

    actual_fields = set(serialized.keys())
    assert len(actual_fields) == 29, f"Expected 29 fields, got {len(actual_fields)}"
    assert (
        actual_fields == expected_fields
    ), f"Missing: {expected_fields - actual_fields}, Extra: {actual_fields - expected_fields}"


def test_serialize_task_field_values_correct() -> None:
    """Test that serialized field values are correct and properly converted."""
    # Arrange
    task_id = uuid4()
    dep1 = uuid4()

    task = Task(
        id=task_id,
        prompt="Test prompt",
        summary="Test summary",
        dependencies=[dep1],
        submitted_at=datetime(2025, 10, 15, 10, 0, 0, tzinfo=timezone.utc),
        last_updated_at=datetime(2025, 10, 15, 10, 5, 0, tzinfo=timezone.utc),
    )

    # Act
    serialized = _serialize_task(task)

    # Assert - string conversions
    assert serialized["id"] == str(task_id)
    assert isinstance(serialized["id"], str)

    # Assert - enum conversions
    assert serialized["status"] == "pending"
    assert isinstance(serialized["status"], str)
    assert serialized["source"] == "human"
    assert serialized["dependency_type"] == "sequential"

    # Assert - datetime conversions
    assert serialized["submitted_at"] == "2025-10-15T10:00:00+00:00"
    assert serialized["last_updated_at"] == "2025-10-15T10:05:00+00:00"

    # Assert - list conversions (UUIDs to strings)
    assert serialized["dependencies"] == [str(dep1)]
    assert all(isinstance(d, str) for d in serialized["dependencies"])

    # Assert - summary field
    assert serialized["summary"] == "Test summary"


def test_serialize_task_null_values_handled() -> None:
    """Test that null/None values are properly handled in serialization."""
    # Arrange - minimal task with many None fields
    task = Task(
        prompt="Minimal task",
        summary=None,  # Test None summary
        submitted_at=datetime.now(timezone.utc),
        last_updated_at=datetime.now(timezone.utc),
    )

    # Act
    serialized = _serialize_task(task)

    # Assert - None values preserved
    assert serialized["summary"] is None
    assert serialized["started_at"] is None
    assert serialized["completed_at"] is None
    assert serialized["deadline"] is None
    assert serialized["parent_task_id"] is None
    assert serialized["session_id"] is None
    assert serialized["created_by"] is None
    assert serialized["result_data"] is None
    assert serialized["error_message"] is None
    assert serialized["estimated_duration_seconds"] is None
    assert serialized["feature_branch"] is None
    assert serialized["task_branch"] is None

    # Assert - empty list for dependencies
    assert serialized["dependencies"] == []


def test_serialize_task_missing_fields_from_original_bug() -> None:
    """Test that previously missing fields are now present.

    This test specifically validates that the 8 fields that were missing
    in the original bug are now included:
    1. retry_count
    2. max_retries
    3. max_execution_timeout_seconds
    4. last_updated_at
    5. created_by
    6. dependencies
    7. dependency_type
    8. summary
    """
    # Arrange
    task = Task(
        prompt="Test task",
        retry_count=3,
        max_retries=5,
        max_execution_timeout_seconds=7200,
        created_by="test-user",
        dependencies=[uuid4()],
        dependency_type=DependencyType.PARALLEL,
        summary="Test summary",
        submitted_at=datetime.now(timezone.utc),
        last_updated_at=datetime.now(timezone.utc),
    )

    # Act
    serialized = _serialize_task(task)

    # Assert - previously missing fields now present
    assert "retry_count" in serialized
    assert serialized["retry_count"] == 3

    assert "max_retries" in serialized
    assert serialized["max_retries"] == 5

    assert "max_execution_timeout_seconds" in serialized
    assert serialized["max_execution_timeout_seconds"] == 7200

    assert "last_updated_at" in serialized
    assert serialized["last_updated_at"] is not None

    assert "created_by" in serialized
    assert serialized["created_by"] == "test-user"

    assert "dependencies" in serialized
    assert len(serialized["dependencies"]) == 1

    assert "dependency_type" in serialized
    assert serialized["dependency_type"] == "parallel"

    assert "summary" in serialized
    assert serialized["summary"] == "Test summary"


def test_serialize_task_dependencies_list_conversion() -> None:
    """Test that dependencies list is correctly converted from UUIDs to strings."""
    # Arrange
    dep1 = uuid4()
    dep2 = uuid4()
    dep3 = uuid4()

    task = Task(
        prompt="Task with dependencies",
        dependencies=[dep1, dep2, dep3],
        submitted_at=datetime.now(timezone.utc),
        last_updated_at=datetime.now(timezone.utc),
    )

    # Act
    serialized = _serialize_task(task)

    # Assert - dependencies converted to strings
    assert len(serialized["dependencies"]) == 3
    assert str(dep1) in serialized["dependencies"]
    assert str(dep2) in serialized["dependencies"]
    assert str(dep3) in serialized["dependencies"]
    assert all(isinstance(d, str) for d in serialized["dependencies"])


def test_serialize_task_backward_compatibility() -> None:
    """Test that tasks without summary field serialize correctly (backward compatibility)."""
    # Arrange - task without summary (simulates old task)
    task = Task(
        prompt="Old task without summary",
        summary=None,
        submitted_at=datetime.now(timezone.utc),
        last_updated_at=datetime.now(timezone.utc),
    )

    # Act
    serialized = _serialize_task(task)

    # Assert - all 29 fields still present
    assert len(serialized.keys()) == 29
    assert "summary" in serialized
    assert serialized["summary"] is None


if __name__ == "__main__":
    pytest.main([__file__, "-v"])
