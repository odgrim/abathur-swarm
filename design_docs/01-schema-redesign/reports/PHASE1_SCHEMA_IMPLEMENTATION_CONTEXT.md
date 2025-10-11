# Phase 1: Schema Implementation Context

**Project:** Abathur Enhanced Task Queue System
**Phase:** 1 - Database Schema & Domain Models
**Date:** 2025-10-10
**Agent:** database-schema-architect
**Orchestrator:** task-queue-orchestrator

## Executive Summary

This document provides complete context for implementing Phase 1 of the enhanced task queue system. Your objective is to design and implement database schema changes and enhanced domain models to support hierarchical task submission, dependency management, and priority-based scheduling.

## Decision Points Resolution

All 14 decision points have been resolved. Key decisions affecting schema design:

1. **Migration Strategy**: Automatic migration with backup/rollback
2. **Dependency Limits**: MAX_DEPENDENCIES_PER_TASK=50 (configurable), MAX_DEPENDENCY_DEPTH=10 (configurable)
3. **Priority Recalculation**: Real-time (on every state change)
4. **Task States**: PENDING, BLOCKED, READY, RUNNING, COMPLETED, FAILED, CANCELLED
5. **Dependency Semantics**: PARALLEL = AND logic (wait for all prerequisites)
6. **Backward Compatibility**: Breaking change allowed (no existing users)
7. **Testing Requirements**: Unit >90%, Integration all workflows, Performance all targets

## Current Implementation Analysis

### Existing Task Model
**Location:** `/Users/odgrim/dev/home/agentics/abathur/src/abathur/domain/models.py`

```python
class TaskStatus(str, Enum):
    PENDING = "pending"
    RUNNING = "running"
    COMPLETED = "completed"
    FAILED = "failed"
    CANCELLED = "cancelled"

class Task(BaseModel):
    id: UUID
    prompt: str
    agent_type: str = "general"
    priority: int = Field(default=5, ge=0, le=10)  # Base priority
    status: TaskStatus = Field(default=TaskStatus.PENDING)
    # ... existing fields
    parent_task_id: UUID | None = None
    dependencies: list[UUID] = Field(default_factory=list)
    session_id: str | None = None
```

### Existing Database Schema
**Location:** `/Users/odgrim/dev/home/agentics/abathur/src/abathur/infrastructure/database.py`

**Tasks Table** (existing columns):
- id, prompt, agent_type, priority, status
- input_data, result_data, error_message
- retry_count, max_retries, max_execution_timeout_seconds
- submitted_at, started_at, completed_at, last_updated_at
- created_by, parent_task_id, dependencies (JSON), session_id

**Migration Infrastructure**: Database class has `_run_migrations()` method that handles schema migrations automatically.

## Phase 1 Deliverables

### 1. Enhanced Domain Models

**File:** `/Users/odgrim/dev/home/agentics/abathur/src/abathur/domain/models.py`

#### 1.1 Update TaskStatus Enum
Add BLOCKED and READY states:
```python
class TaskStatus(str, Enum):
    PENDING = "pending"          # Submitted, dependencies not yet checked
    BLOCKED = "blocked"          # NEW: Waiting for dependencies
    READY = "ready"              # NEW: Dependencies met, ready for execution
    RUNNING = "running"
    COMPLETED = "completed"
    FAILED = "failed"
    CANCELLED = "cancelled"
```

#### 1.2 Create TaskSource Enum
```python
class TaskSource(str, Enum):
    """Origin of task submission."""
    HUMAN = "human"
    AGENT_REQUIREMENTS = "agent_requirements"
    AGENT_PLANNER = "agent_planner"
    AGENT_IMPLEMENTATION = "agent_implementation"
```

#### 1.3 Create DependencyType Enum
```python
class DependencyType(str, Enum):
    """Type of dependency relationship."""
    SEQUENTIAL = "sequential"  # B depends on A completing
    PARALLEL = "parallel"      # C depends on A AND B both completing (AND logic)
```

#### 1.4 Create TaskDependency Model
```python
class TaskDependency(BaseModel):
    """Represents a dependency relationship between tasks."""
    id: UUID = Field(default_factory=uuid4)
    dependent_task_id: UUID      # Task that depends
    prerequisite_task_id: UUID   # Task that must complete first
    dependency_type: DependencyType
    created_at: datetime = Field(default_factory=lambda: datetime.now(timezone.utc))
    resolved_at: datetime | None = None  # When prerequisite completed
```

#### 1.5 Enhance Task Model
Add these fields to existing Task model:
```python
# NEW: Source tracking
source: TaskSource = Field(default=TaskSource.HUMAN)

# NEW: Dependency type
dependency_type: DependencyType = Field(default=DependencyType.SEQUENTIAL)

# NEW: Priority calculation fields
calculated_priority: float = Field(default=5.0)
deadline: datetime | None = None
estimated_duration_seconds: int | None = None
dependency_depth: int = Field(default=0)
```

**Note:** Do NOT add `blocked_by` or `blocking_tasks` to the model - these will be computed dynamically from task_dependencies table.

### 2. Database Migration Script

**File:** `/Users/odgrim/dev/home/agentics/abathur/src/abathur/infrastructure/database.py`

Add migration logic to `_run_migrations()` method:

#### 2.1 Tasks Table Alterations
```python
# Check if enhanced task queue columns exist
if "source" not in column_names:
    print("Migrating database schema: adding enhanced task queue columns")

    # Add source column
    await conn.execute(
        """
        ALTER TABLE tasks
        ADD COLUMN source TEXT NOT NULL DEFAULT 'human'
        """
    )

    # Add calculated_priority column
    await conn.execute(
        """
        ALTER TABLE tasks
        ADD COLUMN calculated_priority REAL NOT NULL DEFAULT 5.0
        """
    )

    # Add deadline column
    await conn.execute(
        """
        ALTER TABLE tasks
        ADD COLUMN deadline TIMESTAMP
        """
    )

    # Add estimated_duration_seconds column
    await conn.execute(
        """
        ALTER TABLE tasks
        ADD COLUMN estimated_duration_seconds INTEGER
        """
    )

    # Add dependency_depth column
    await conn.execute(
        """
        ALTER TABLE tasks
        ADD COLUMN dependency_depth INTEGER DEFAULT 0
        """
    )

    await conn.commit()
    print("Added enhanced task queue columns successfully")
```

#### 2.2 Create task_dependencies Table
```python
# Create task_dependencies table
await conn.execute(
    """
    CREATE TABLE IF NOT EXISTS task_dependencies (
        id TEXT PRIMARY KEY,
        dependent_task_id TEXT NOT NULL,
        prerequisite_task_id TEXT NOT NULL,
        dependency_type TEXT NOT NULL DEFAULT 'sequential',
        created_at TIMESTAMP NOT NULL,
        resolved_at TIMESTAMP,

        FOREIGN KEY (dependent_task_id) REFERENCES tasks(id) ON DELETE CASCADE,
        FOREIGN KEY (prerequisite_task_id) REFERENCES tasks(id) ON DELETE CASCADE,
        CHECK(dependency_type IN ('sequential', 'parallel')),
        CHECK(dependent_task_id != prerequisite_task_id),
        UNIQUE(dependent_task_id, prerequisite_task_id)
    )
    """
)
```

#### 2.3 Update TaskStatus CHECK Constraint
The tasks table has a CHECK constraint on status. You need to update it:
```python
# Since SQLite doesn't support ALTER CHECK constraint, we need to:
# 1. Create new table with updated constraint
# 2. Copy data
# 3. Drop old table
# 4. Rename new table

# This should be part of the migration that adds BLOCKED and READY states
```

### 3. Performance Indexes

**File:** `/Users/odgrim/dev/home/agentics/abathur/src/abathur/infrastructure/database.py`

Add to `_create_indexes()` method:

```python
# NEW: Dependency resolution indexes
await conn.execute(
    """
    CREATE INDEX IF NOT EXISTS idx_task_dependencies_prerequisite
    ON task_dependencies(prerequisite_task_id, resolved_at)
    WHERE resolved_at IS NULL
    """
)

await conn.execute(
    """
    CREATE INDEX IF NOT EXISTS idx_task_dependencies_dependent
    ON task_dependencies(dependent_task_id, resolved_at)
    WHERE resolved_at IS NULL
    """
)

# NEW: Priority queue index (composite for calculated priority)
await conn.execute(
    """
    CREATE INDEX IF NOT EXISTS idx_tasks_ready_priority
    ON tasks(status, calculated_priority DESC, submitted_at ASC)
    WHERE status = 'ready'
    """
)

# NEW: Source tracking index
await conn.execute(
    """
    CREATE INDEX IF NOT EXISTS idx_tasks_source_created
    ON tasks(source, created_by, submitted_at DESC)
    """
)

# NEW: Deadline urgency index
await conn.execute(
    """
    CREATE INDEX IF NOT EXISTS idx_tasks_deadline
    ON tasks(deadline, status)
    WHERE deadline IS NOT NULL AND status IN ('pending', 'blocked', 'ready')
    """
)

# NEW: Blocked tasks index
await conn.execute(
    """
    CREATE INDEX IF NOT EXISTS idx_tasks_blocked
    ON tasks(status, submitted_at ASC)
    WHERE status = 'blocked'
    """
)
```

### 4. Database Helper Methods

Add these methods to the Database class:

```python
async def insert_task_dependency(self, dependency: TaskDependency) -> None:
    """Insert a task dependency relationship."""
    async with self._get_connection() as conn:
        await conn.execute(
            """
            INSERT INTO task_dependencies (
                id, dependent_task_id, prerequisite_task_id,
                dependency_type, created_at, resolved_at
            ) VALUES (?, ?, ?, ?, ?, ?)
            """,
            (
                str(dependency.id),
                str(dependency.dependent_task_id),
                str(dependency.prerequisite_task_id),
                dependency.dependency_type.value,
                dependency.created_at.isoformat(),
                dependency.resolved_at.isoformat() if dependency.resolved_at else None,
            ),
        )
        await conn.commit()

async def get_task_dependencies(self, task_id: UUID) -> list[TaskDependency]:
    """Get all dependencies for a task."""
    async with self._get_connection() as conn:
        cursor = await conn.execute(
            """
            SELECT * FROM task_dependencies
            WHERE dependent_task_id = ?
            ORDER BY created_at ASC
            """,
            (str(task_id),),
        )
        rows = await cursor.fetchall()
        return [self._row_to_task_dependency(row) for row in rows]

async def resolve_dependency(self, prerequisite_task_id: UUID) -> None:
    """Mark all dependencies on a prerequisite task as resolved."""
    async with self._get_connection() as conn:
        await conn.execute(
            """
            UPDATE task_dependencies
            SET resolved_at = ?
            WHERE prerequisite_task_id = ? AND resolved_at IS NULL
            """,
            (datetime.now(timezone.utc).isoformat(), str(prerequisite_task_id)),
        )
        await conn.commit()

def _row_to_task_dependency(self, row: aiosqlite.Row) -> TaskDependency:
    """Convert database row to TaskDependency model."""
    return TaskDependency(
        id=UUID(row["id"]),
        dependent_task_id=UUID(row["dependent_task_id"]),
        prerequisite_task_id=UUID(row["prerequisite_task_id"]),
        dependency_type=DependencyType(row["dependency_type"]),
        created_at=datetime.fromisoformat(row["created_at"]),
        resolved_at=datetime.fromisoformat(row["resolved_at"]) if row["resolved_at"] else None,
    )
```

Update `_row_to_task()` method to handle new fields:
```python
def _row_to_task(self, row: aiosqlite.Row) -> Task:
    """Convert database row to Task model."""
    row_dict = dict(row)

    return Task(
        # ... existing fields ...
        source=TaskSource(row_dict.get("source", "human")),
        calculated_priority=row_dict.get("calculated_priority", 5.0),
        deadline=datetime.fromisoformat(row_dict["deadline"]) if row_dict.get("deadline") else None,
        estimated_duration_seconds=row_dict.get("estimated_duration_seconds"),
        dependency_depth=row_dict.get("dependency_depth", 0),
    )
```

### 5. Unit Tests

**File:** `/Users/odgrim/dev/home/agentics/abathur/tests/unit/test_enhanced_task_models.py`

Create comprehensive unit tests:

```python
"""Unit tests for enhanced task queue models."""

import pytest
from datetime import datetime, timezone
from uuid import uuid4

from abathur.domain.models import (
    Task, TaskStatus, TaskSource, DependencyType, TaskDependency
)


class TestTaskStatus:
    """Test enhanced TaskStatus enum."""

    def test_all_statuses_defined(self):
        """All required statuses are defined."""
        assert TaskStatus.PENDING.value == "pending"
        assert TaskStatus.BLOCKED.value == "blocked"
        assert TaskStatus.READY.value == "ready"
        assert TaskStatus.RUNNING.value == "running"
        assert TaskStatus.COMPLETED.value == "completed"
        assert TaskStatus.FAILED.value == "failed"
        assert TaskStatus.CANCELLED.value == "cancelled"


class TestTaskSource:
    """Test TaskSource enum."""

    def test_all_sources_defined(self):
        """All required sources are defined."""
        assert TaskSource.HUMAN.value == "human"
        assert TaskSource.AGENT_REQUIREMENTS.value == "agent_requirements"
        assert TaskSource.AGENT_PLANNER.value == "agent_planner"
        assert TaskSource.AGENT_IMPLEMENTATION.value == "agent_implementation"


class TestDependencyType:
    """Test DependencyType enum."""

    def test_all_types_defined(self):
        """All required dependency types are defined."""
        assert DependencyType.SEQUENTIAL.value == "sequential"
        assert DependencyType.PARALLEL.value == "parallel"


class TestTaskModel:
    """Test enhanced Task model."""

    def test_task_with_defaults(self):
        """Task can be created with minimal fields."""
        task = Task(prompt="Test task")
        assert task.id is not None
        assert task.prompt == "Test task"
        assert task.source == TaskSource.HUMAN
        assert task.calculated_priority == 5.0
        assert task.deadline is None
        assert task.estimated_duration_seconds is None
        assert task.dependency_depth == 0

    def test_task_with_source(self):
        """Task can be created with specific source."""
        task = Task(
            prompt="Agent task",
            source=TaskSource.AGENT_PLANNER
        )
        assert task.source == TaskSource.AGENT_PLANNER

    def test_task_with_priority_fields(self):
        """Task can be created with priority calculation fields."""
        deadline = datetime.now(timezone.utc)
        task = Task(
            prompt="Urgent task",
            calculated_priority=8.5,
            deadline=deadline,
            estimated_duration_seconds=3600,
            dependency_depth=2
        )
        assert task.calculated_priority == 8.5
        assert task.deadline == deadline
        assert task.estimated_duration_seconds == 3600
        assert task.dependency_depth == 2


class TestTaskDependencyModel:
    """Test TaskDependency model."""

    def test_dependency_creation(self):
        """TaskDependency can be created."""
        dep_id = uuid4()
        prereq_id = uuid4()

        dependency = TaskDependency(
            dependent_task_id=dep_id,
            prerequisite_task_id=prereq_id,
            dependency_type=DependencyType.SEQUENTIAL
        )

        assert dependency.id is not None
        assert dependency.dependent_task_id == dep_id
        assert dependency.prerequisite_task_id == prereq_id
        assert dependency.dependency_type == DependencyType.SEQUENTIAL
        assert dependency.resolved_at is None

    def test_dependency_resolution(self):
        """TaskDependency can be resolved."""
        dependency = TaskDependency(
            dependent_task_id=uuid4(),
            prerequisite_task_id=uuid4(),
            dependency_type=DependencyType.PARALLEL
        )

        resolved_time = datetime.now(timezone.utc)
        dependency.resolved_at = resolved_time

        assert dependency.resolved_at == resolved_time
```

**File:** `/Users/odgrim/dev/home/agentics/abathur/tests/integration/test_schema_migration.py`

Integration tests for migration:

```python
"""Integration tests for schema migration."""

import pytest
from pathlib import Path
from abathur.infrastructure.database import Database
from abathur.domain.models import Task, TaskSource, TaskDependency, DependencyType


@pytest.mark.asyncio
async def test_migration_adds_new_columns():
    """Migration successfully adds enhanced task queue columns."""
    db = Database(Path(":memory:"))
    await db.initialize()

    # Verify new columns exist by inserting a task with new fields
    task = Task(
        prompt="Test migration",
        source=TaskSource.AGENT_PLANNER,
        calculated_priority=7.5,
        dependency_depth=1
    )

    await db.insert_task(task)

    # Retrieve and verify
    retrieved = await db.get_task(task.id)
    assert retrieved is not None
    assert retrieved.source == TaskSource.AGENT_PLANNER
    assert retrieved.calculated_priority == 7.5
    assert retrieved.dependency_depth == 1


@pytest.mark.asyncio
async def test_task_dependencies_table_created():
    """Migration creates task_dependencies table."""
    db = Database(Path(":memory:"))
    await db.initialize()

    # Create two tasks
    task1 = Task(prompt="Task 1")
    task2 = Task(prompt="Task 2")
    await db.insert_task(task1)
    await db.insert_task(task2)

    # Create dependency
    dependency = TaskDependency(
        dependent_task_id=task2.id,
        prerequisite_task_id=task1.id,
        dependency_type=DependencyType.SEQUENTIAL
    )

    await db.insert_task_dependency(dependency)

    # Retrieve dependencies
    deps = await db.get_task_dependencies(task2.id)
    assert len(deps) == 1
    assert deps[0].prerequisite_task_id == task1.id


@pytest.mark.asyncio
async def test_foreign_key_constraints():
    """Foreign key constraints are enforced."""
    db = Database(Path(":memory:"))
    await db.initialize()

    violations = await db.validate_foreign_keys()
    assert len(violations) == 0


@pytest.mark.asyncio
async def test_indexes_created():
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
```

## Acceptance Criteria

Phase 1 is APPROVED if ALL of the following are met:

1. **Schema Migration**:
   - [ ] Migration runs successfully on clean database
   - [ ] Migration runs successfully on existing database (idempotent)
   - [ ] No data loss during migration
   - [ ] All new columns added to tasks table
   - [ ] task_dependencies table created

2. **Data Integrity**:
   - [ ] Foreign key constraints enforced
   - [ ] CHECK constraints work correctly
   - [ ] UNIQUE constraints prevent duplicate dependencies
   - [ ] Self-dependencies prevented (dependent_task_id != prerequisite_task_id)

3. **Domain Models**:
   - [ ] TaskStatus enum has 7 states (PENDING, BLOCKED, READY, RUNNING, COMPLETED, FAILED, CANCELLED)
   - [ ] TaskSource enum has 4 sources
   - [ ] DependencyType enum has 2 types
   - [ ] TaskDependency model defined
   - [ ] Task model has all new fields

4. **Database Methods**:
   - [ ] insert_task_dependency() works
   - [ ] get_task_dependencies() works
   - [ ] resolve_dependency() works
   - [ ] _row_to_task() handles new fields
   - [ ] _row_to_task_dependency() works

5. **Indexes**:
   - [ ] All 6 new indexes created
   - [ ] Query plans use indexes (validate with explain_query_plan)

6. **Testing**:
   - [ ] Unit tests pass (>90% coverage target)
   - [ ] Integration tests pass
   - [ ] Foreign key validation passes
   - [ ] Index usage validation passes

## Performance Validation

After implementation, validate:

1. **Query Plan Analysis**:
```python
# Test that priority queue query uses index
query = """
    SELECT * FROM tasks
    WHERE status = 'ready'
    ORDER BY calculated_priority DESC, submitted_at ASC
    LIMIT 1
"""
plan = await db.explain_query_plan(query, ())
# Should show "USING INDEX idx_tasks_ready_priority"
```

2. **Dependency Query Performance**:
```python
# Test dependency resolution query uses index
query = """
    SELECT * FROM task_dependencies
    WHERE prerequisite_task_id = ? AND resolved_at IS NULL
"""
plan = await db.explain_query_plan(query, (str(task_id),))
# Should show "USING INDEX idx_task_dependencies_prerequisite"
```

## Critical Implementation Notes

1. **TaskStatus CHECK Constraint Update**:
   - SQLite doesn't support ALTER CHECK constraint
   - You'll need to recreate the tasks table to update the CHECK constraint
   - Use the same pattern as existing migrations (rename, copy, drop, rename back)
   - Do this carefully to avoid data loss

2. **Backward Compatibility**:
   - New columns must have DEFAULT values
   - Existing code should continue to work (graceful degradation)
   - Use `.get()` with defaults when reading new fields

3. **Migration Idempotency**:
   - Check if columns/tables exist before adding
   - Use `IF NOT EXISTS` for table creation
   - Use `IF NOT EXISTS` for index creation

4. **Foreign Key Constraints**:
   - Ensure foreign keys are enabled (`PRAGMA foreign_keys=ON`)
   - Test with `validate_foreign_keys()` after migration

## Files to Modify

1. `/Users/odgrim/dev/home/agentics/abathur/src/abathur/domain/models.py`
   - Update TaskStatus enum
   - Add TaskSource enum
   - Add DependencyType enum
   - Add TaskDependency model
   - Enhance Task model with new fields

2. `/Users/odgrim/dev/home/agentics/abathur/src/abathur/infrastructure/database.py`
   - Add migration logic to `_run_migrations()`
   - Add task_dependencies table creation
   - Add new indexes to `_create_indexes()`
   - Add `insert_task_dependency()` method
   - Add `get_task_dependencies()` method
   - Add `resolve_dependency()` method
   - Add `_row_to_task_dependency()` helper
   - Update `_row_to_task()` to handle new fields

3. `/Users/odgrim/dev/home/agentics/abathur/tests/unit/test_enhanced_task_models.py` (NEW)
   - Unit tests for new enums
   - Unit tests for enhanced Task model
   - Unit tests for TaskDependency model

4. `/Users/odgrim/dev/home/agentics/abathur/tests/integration/test_schema_migration.py` (NEW)
   - Migration integration tests
   - Foreign key validation tests
   - Index validation tests

## Success Criteria Summary

Phase 1 will be marked as **APPROVED** if:
1. All code files modified correctly
2. All unit tests pass (>90% coverage)
3. All integration tests pass
4. Migration runs successfully on both clean and existing databases
5. No foreign key violations
6. All indexes created and used by query planner
7. Performance baseline established

Phase 1 will be marked as **CONDITIONAL** if:
1. Minor issues that don't block Phase 2 (e.g., < 90% coverage but > 80%)
2. Migration works but requires manual intervention
3. Some indexes not optimal but functional

Phase 1 will be marked as **REVISE** if:
1. Migration fails
2. Data loss during migration
3. Foreign key violations
4. Critical tests failing
5. Schema doesn't match architecture specification

Phase 1 will be marked as **ESCALATE** if:
1. Fundamental design issues discovered
2. Architecture requires significant changes
3. Unresolved technical blockers

## Next Steps After Phase 1

Upon APPROVAL of Phase 1:
1. Orchestrator will validate all deliverables
2. Generate Phase 1 validation report
3. Proceed to Phase 2: Dependency Resolution Algorithm
4. Invoke `algorithm-design-specialist` agent

---

**You are now cleared to begin Phase 1 implementation. Good luck!**
