"""Integration tests for schema migration."""

import sqlite3
from pathlib import Path
from uuid import uuid4

import pytest
from abathur.domain.models import (
    DependencyType,
    Task,
    TaskDependency,
    TaskSource,
    TaskStatus,
)
from abathur.infrastructure.database import Database


@pytest.mark.asyncio
async def test_migration_adds_new_columns() -> None:
    """Migration successfully adds enhanced task queue columns."""
    db = Database(Path(":memory:"))
    await db.initialize()

    # Verify new columns exist by inserting a task with new fields
    task = Task(
        prompt="Test migration",
        summary="Test migration",
        source=TaskSource.AGENT_PLANNER,
        calculated_priority=7.5,
        dependency_depth=1,
        dependency_type=DependencyType.PARALLEL,
    )

    await db.insert_task(task)

    # Retrieve and verify
    retrieved = await db.get_task(task.id)
    assert retrieved is not None
    assert retrieved.source == TaskSource.AGENT_PLANNER
    assert retrieved.calculated_priority == 7.5
    assert retrieved.dependency_depth == 1
    assert retrieved.dependency_type == DependencyType.PARALLEL

    await db.close()


@pytest.mark.asyncio
async def test_task_dependencies_table_created() -> None:
    """Migration creates task_dependencies table."""
    db = Database(Path(":memory:"))
    await db.initialize()

    # Create two tasks
    task1 = Task(prompt="Task 1", summary="Task 1")
    task2 = Task(prompt="Task 2", summary="Task 2")
    await db.insert_task(task1)
    await db.insert_task(task2)

    # Create dependency
    dependency = TaskDependency(
        dependent_task_id=task2.id,
        prerequisite_task_id=task1.id,
        dependency_type=DependencyType.SEQUENTIAL,
    )

    await db.insert_task_dependency(dependency)

    # Retrieve dependencies
    deps = await db.get_task_dependencies(task2.id)
    assert len(deps) == 1
    assert deps[0].prerequisite_task_id == task1.id
    assert deps[0].dependent_task_id == task2.id
    assert deps[0].dependency_type == DependencyType.SEQUENTIAL

    await db.close()


@pytest.mark.asyncio
async def test_foreign_key_constraints() -> None:
    """Foreign key constraints are enforced."""
    db = Database(Path(":memory:"))
    await db.initialize()

    violations = await db.validate_foreign_keys()
    assert len(violations) == 0

    await db.close()


@pytest.mark.asyncio
async def test_indexes_created() -> None:
    """All required indexes are created."""
    db = Database(Path(":memory:"))
    await db.initialize()

    index_info = await db.get_index_usage()
    index_names = [idx["name"] for idx in index_info["indexes"]]

    # Check for new indexes
    assert "idx_task_dependencies_prerequisite" in index_names
    assert "idx_task_dependencies_dependent" in index_names
    assert "idx_tasks_ready_priority" in index_names
    assert "idx_tasks_source_created" in index_names
    assert "idx_tasks_deadline" in index_names
    assert "idx_tasks_blocked" in index_names

    await db.close()


@pytest.mark.asyncio
async def test_dependency_resolution() -> None:
    """Dependencies can be resolved correctly."""
    db = Database(Path(":memory:"))
    await db.initialize()

    # Create prerequisite and dependent tasks
    prereq_task = Task(prompt="Prerequisite task", summary="Prerequisite task")
    dependent_task = Task(prompt="Dependent task", summary="Dependent task")
    await db.insert_task(prereq_task)
    await db.insert_task(dependent_task)

    # Create dependency
    dependency = TaskDependency(
        dependent_task_id=dependent_task.id,
        prerequisite_task_id=prereq_task.id,
        dependency_type=DependencyType.SEQUENTIAL,
    )
    await db.insert_task_dependency(dependency)

    # Verify dependency is unresolved
    deps = await db.get_task_dependencies(dependent_task.id)
    assert len(deps) == 1
    assert deps[0].resolved_at is None

    # Resolve dependency
    await db.resolve_dependency(prereq_task.id)

    # Verify dependency is now resolved
    deps = await db.get_task_dependencies(dependent_task.id)
    assert len(deps) == 1
    assert deps[0].resolved_at is not None

    await db.close()


@pytest.mark.asyncio
async def test_multiple_dependencies() -> None:
    """Tasks can have multiple dependencies."""
    db = Database(Path(":memory:"))
    await db.initialize()

    # Create prerequisite tasks
    prereq1 = Task(prompt="Prerequisite 1", summary="Prerequisite 1")
    prereq2 = Task(prompt="Prerequisite 2", summary="Prerequisite 2")
    prereq3 = Task(prompt="Prerequisite 3", summary="Prerequisite 3")
    dependent = Task(prompt="Dependent task", summary="Dependent task")

    await db.insert_task(prereq1)
    await db.insert_task(prereq2)
    await db.insert_task(prereq3)
    await db.insert_task(dependent)

    # Create multiple dependencies
    dep1 = TaskDependency(
        dependent_task_id=dependent.id,
        prerequisite_task_id=prereq1.id,
        dependency_type=DependencyType.PARALLEL,
    )
    dep2 = TaskDependency(
        dependent_task_id=dependent.id,
        prerequisite_task_id=prereq2.id,
        dependency_type=DependencyType.PARALLEL,
    )
    dep3 = TaskDependency(
        dependent_task_id=dependent.id,
        prerequisite_task_id=prereq3.id,
        dependency_type=DependencyType.PARALLEL,
    )

    await db.insert_task_dependency(dep1)
    await db.insert_task_dependency(dep2)
    await db.insert_task_dependency(dep3)

    # Verify all dependencies exist
    deps = await db.get_task_dependencies(dependent.id)
    assert len(deps) == 3
    prereq_ids = {dep.prerequisite_task_id for dep in deps}
    assert prereq1.id in prereq_ids
    assert prereq2.id in prereq_ids
    assert prereq3.id in prereq_ids

    await db.close()


@pytest.mark.asyncio
async def test_backward_compatibility() -> None:
    """Tasks created without new fields still work."""
    db = Database(Path(":memory:"))
    await db.initialize()

    # Create task with minimal fields (defaults should be used)
    task = Task(prompt="Minimal task", summary="Minimal task")
    await db.insert_task(task)

    # Retrieve and verify defaults
    retrieved = await db.get_task(task.id)
    assert retrieved is not None
    assert retrieved.source == TaskSource.HUMAN
    assert retrieved.dependency_type == DependencyType.SEQUENTIAL
    assert retrieved.calculated_priority == 5.0
    assert retrieved.dependency_depth == 0

    await db.close()


@pytest.mark.asyncio
async def test_task_status_values() -> None:
    """All TaskStatus values are supported."""
    db = Database(Path(":memory:"))
    await db.initialize()

    statuses = [
        TaskStatus.PENDING,
        TaskStatus.BLOCKED,
        TaskStatus.READY,
        TaskStatus.RUNNING,
        TaskStatus.COMPLETED,
        TaskStatus.FAILED,
        TaskStatus.CANCELLED,
    ]

    for status in statuses:
        task = Task(
            prompt=f"Task with status {status.value}",
            summary=f"Task with status {status.value}",
            status=status
        )
        await db.insert_task(task)

        retrieved = await db.get_task(task.id)
        assert retrieved is not None
        assert retrieved.status == status

    await db.close()


@pytest.mark.asyncio
async def test_task_source_values() -> None:
    """All TaskSource values are supported."""
    db = Database(Path(":memory:"))
    await db.initialize()

    sources = [
        TaskSource.HUMAN,
        TaskSource.AGENT_REQUIREMENTS,
        TaskSource.AGENT_PLANNER,
        TaskSource.AGENT_IMPLEMENTATION,
    ]

    for source in sources:
        task = Task(
            prompt=f"Task from {source.value}",
            summary=f"Task from {source.value}",
            source=source
        )
        await db.insert_task(task)

        retrieved = await db.get_task(task.id)
        assert retrieved is not None
        assert retrieved.source == source

    await db.close()


@pytest.mark.asyncio
async def test_deadline_persistence() -> None:
    """Task deadlines are persisted correctly."""
    from datetime import datetime, timezone

    db = Database(Path(":memory:"))
    await db.initialize()

    deadline = datetime.now(timezone.utc)
    task = Task(
        prompt="Task with deadline",
        summary="Task with deadline",
        deadline=deadline,
        estimated_duration_seconds=7200,
    )
    await db.insert_task(task)

    retrieved = await db.get_task(task.id)
    assert retrieved is not None
    assert retrieved.deadline is not None
    # Compare timestamps (allowing for small precision differences)
    assert abs((retrieved.deadline - deadline).total_seconds()) < 1
    assert retrieved.estimated_duration_seconds == 7200

    await db.close()


@pytest.mark.asyncio
async def test_query_plan_uses_indexes() -> None:
    """Verify critical queries use indexes."""
    db = Database(Path(":memory:"))
    await db.initialize()

    # Test priority queue query uses index
    query = """
        SELECT * FROM tasks
        WHERE status = 'ready'
        ORDER BY calculated_priority DESC, submitted_at ASC
        LIMIT 1
    """
    plan = await db.explain_query_plan(query, ())
    plan_text = " ".join(plan).lower()
    # Should use the idx_tasks_ready_priority index
    assert "idx_tasks_ready_priority" in plan_text or "index" in plan_text

    # Test dependency resolution query uses index
    test_id = str(uuid4())
    query = """
        SELECT * FROM task_dependencies
        WHERE prerequisite_task_id = ? AND resolved_at IS NULL
    """
    plan = await db.explain_query_plan(query, (test_id,))
    plan_text = " ".join(plan).lower()
    # Should use the idx_task_dependencies_prerequisite index
    assert "idx_task_dependencies_prerequisite" in plan_text or "index" in plan_text

    await db.close()


@pytest.mark.asyncio
async def test_unique_dependency_constraint() -> None:
    """Cannot create duplicate dependencies between same tasks."""
    db = Database(Path(":memory:"))
    await db.initialize()

    task1 = Task(prompt="Task 1", summary="Task 1")
    task2 = Task(prompt="Task 2", summary="Task 2")
    await db.insert_task(task1)
    await db.insert_task(task2)

    # Create first dependency
    dep1 = TaskDependency(
        dependent_task_id=task2.id,
        prerequisite_task_id=task1.id,
        dependency_type=DependencyType.SEQUENTIAL,
    )
    await db.insert_task_dependency(dep1)

    # Attempt to create duplicate dependency (should fail)
    dep2 = TaskDependency(
        dependent_task_id=task2.id,
        prerequisite_task_id=task1.id,
        dependency_type=DependencyType.SEQUENTIAL,
    )

    with pytest.raises(sqlite3.IntegrityError):  # SQLite UNIQUE constraint violation
        await db.insert_task_dependency(dep2)

    await db.close()
