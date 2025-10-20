"""Unit tests for Database validation methods."""

import pytest
from abathur.domain.models import (
    DependencyType,
    Task,
    TaskDependency,
    TaskSource,
    TaskStatus,
)
from abathur.infrastructure.database import Database


class TestDatabaseValidation:
    """Test database validation and diagnostic methods."""

    @pytest.mark.asyncio
    async def test_validate_foreign_keys_empty_db(self, memory_db: Database) -> None:
        """Test foreign key validation on empty database."""
        violations = await memory_db.validate_foreign_keys()
        assert violations == [], f"Expected no violations, got: {violations}"

    @pytest.mark.asyncio
    async def test_explain_query_plan_memory_lookup(self, memory_db: Database) -> None:
        """Test EXPLAIN QUERY PLAN for memory namespace+key lookup."""
        query = """
            SELECT * FROM memory_entries
            WHERE namespace = ? AND key = ? AND is_deleted = 0
            ORDER BY version DESC LIMIT 1
        """
        plan = await memory_db.explain_query_plan(query, ("user:alice:pref", "theme"))

        # Verify plan uses index
        plan_text = " ".join(plan)
        assert (
            "idx_memory_namespace_key_version" in plan_text or "USING INDEX" in plan_text
        ), f"Expected index usage, got: {plan_text}"
        assert "SCAN TABLE" not in plan_text, f"Expected no table scan, got: {plan_text}"

    @pytest.mark.asyncio
    async def test_explain_query_plan_session_status_query(self, memory_db: Database) -> None:
        """Test EXPLAIN QUERY PLAN for session status query."""
        query = """
            SELECT * FROM sessions
            WHERE status = 'active'
            ORDER BY last_update_time DESC
        """
        plan = await memory_db.explain_query_plan(query, ())

        plan_text = " ".join(plan)
        # Should use idx_sessions_status_updated index
        assert (
            "idx_sessions_status_updated" in plan_text or "USING INDEX" in plan_text
        ), f"Expected index usage, got: {plan_text}"

    @pytest.mark.asyncio
    async def test_get_index_usage_reports_all_indexes(self, memory_db: Database) -> None:
        """Test that get_index_usage reports all 39 indexes."""
        index_info = await memory_db.get_index_usage()

        # We expect 39 indexes (excluding auto-generated sqlite_autoindex_*)
        # Count manual indexes only
        _manual_indexes = [
            idx for idx in index_info["indexes"] if not idx["name"].startswith("sqlite_autoindex_")
        ]

        assert (
            index_info["index_count"] >= 35
        ), f"Expected at least 35 indexes, got {index_info['index_count']}"

        # Verify key indexes exist
        index_names = [idx["name"] for idx in index_info["indexes"]]
        critical_indexes = [
            "idx_sessions_pk",
            "idx_memory_namespace_key_version",
            "idx_document_file_path",
            "idx_tasks_status_priority",
            "idx_audit_memory_operations",
        ]

        for idx_name in critical_indexes:
            assert idx_name in index_names, f"Critical index {idx_name} missing"

    @pytest.mark.asyncio
    async def test_pragma_journal_mode_wal(self, memory_db: Database) -> None:
        """Test that journal_mode is WAL."""
        async with memory_db._get_connection() as conn:
            cursor = await conn.execute("PRAGMA journal_mode")
            mode = await cursor.fetchone()
            assert mode is not None
            # In-memory databases use "memory" mode
            # File databases use "wal"
            assert mode[0] in ("memory", "wal"), f"Unexpected journal_mode: {mode[0]}"

    @pytest.mark.asyncio
    async def test_pragma_foreign_keys_enabled(self, memory_db: Database) -> None:
        """Test that foreign keys are enabled."""
        async with memory_db._get_connection() as conn:
            cursor = await conn.execute("PRAGMA foreign_keys")
            enabled = await cursor.fetchone()
            assert enabled is not None
            assert enabled[0] == 1, "Foreign keys not enabled"

    @pytest.mark.asyncio
    async def test_all_tables_exist(self, memory_db: Database) -> None:
        """Test that all required tables exist."""
        expected_tables = [
            "sessions",
            "memory_entries",
            "document_index",
            "tasks",
            "agents",
            "audit",
            "checkpoints",
            "state",
            "metrics",
        ]

        async with memory_db._get_connection() as conn:
            cursor = await conn.execute(
                "SELECT name FROM sqlite_master WHERE type='table' ORDER BY name"
            )
            tables = [row[0] for row in await cursor.fetchall()]

        for table in expected_tables:
            assert table in tables, f"Table {table} missing from database"

    @pytest.mark.asyncio
    async def test_integrity_check_passes(self, memory_db: Database) -> None:
        """Test PRAGMA integrity_check passes."""
        async with memory_db._get_connection() as conn:
            cursor = await conn.execute("PRAGMA integrity_check")
            result = await cursor.fetchone()
            assert result is not None
            assert result[0] == "ok", f"Integrity check failed: {result[0]}"


class TestPhase1SchemaValidation:
    """Test Phase 1 enhanced task queue schema implementation."""

    @pytest.mark.asyncio
    async def test_tasks_table_has_required_columns(self, memory_db: Database) -> None:
        """Test that tasks table has all Phase 1 required columns."""
        async with memory_db._get_connection() as conn:
            cursor = await conn.execute("PRAGMA table_info(tasks)")
            columns = await cursor.fetchall()
            column_names = {col["name"] for col in columns}

            required_columns = {
                "id",
                "prompt",
                "agent_type",
                "priority",
                "status",
                "source",
                "dependency_type",
                "calculated_priority",
                "deadline",
                "estimated_duration_seconds",
                "dependency_depth",
            }

            missing = required_columns - column_names
            assert not missing, f"Missing columns in tasks table: {missing}"

    @pytest.mark.asyncio
    async def test_task_dependencies_table_exists(self, memory_db: Database) -> None:
        """Test that task_dependencies table exists with required columns."""
        async with memory_db._get_connection() as conn:
            cursor = await conn.execute(
                "SELECT name FROM sqlite_master WHERE type='table' AND name='task_dependencies'"
            )
            result = await cursor.fetchone()
            assert result is not None, "task_dependencies table not found"

            # Check columns
            cursor = await conn.execute("PRAGMA table_info(task_dependencies)")
            dep_columns = await cursor.fetchall()
            dep_column_names = {col["name"] for col in dep_columns}

            required_dep_columns = {
                "id",
                "dependent_task_id",
                "prerequisite_task_id",
                "dependency_type",
                "created_at",
                "resolved_at",
            }

            missing_dep = required_dep_columns - dep_column_names
            assert not missing_dep, f"Missing columns in task_dependencies table: {missing_dep}"

    @pytest.mark.asyncio
    async def test_task_queue_indexes_exist(self, memory_db: Database) -> None:
        """Test that all required task queue indexes exist."""
        index_info = await memory_db.get_index_usage()
        index_names = {idx["name"] for idx in index_info["indexes"]}

        required_indexes = {
            "idx_task_dependencies_prerequisite",
            "idx_task_dependencies_dependent",
            "idx_tasks_ready_priority",
            "idx_tasks_source_created",
            "idx_tasks_deadline",
            "idx_tasks_blocked",
        }

        missing_indexes = required_indexes - index_names
        assert not missing_indexes, f"Missing required task queue indexes: {missing_indexes}"

    @pytest.mark.asyncio
    async def test_task_insert_and_retrieve_with_phase1_fields(self, memory_db: Database) -> None:
        """Test inserting and retrieving task with Phase 1 fields."""
        # Create test task with all Phase 1 fields
        test_task = Task(
            prompt="Phase 1 validation test task",
            summary="Phase 1 validation test task",
            source=TaskSource.AGENT_PLANNER,
            dependency_type=DependencyType.PARALLEL,
            calculated_priority=7.5,
            dependency_depth=2,
            estimated_duration_seconds=3600,
        )

        await memory_db.insert_task(test_task)

        # Retrieve and verify
        retrieved = await memory_db.get_task(test_task.id)
        assert retrieved is not None, "Failed to retrieve test task"
        assert retrieved.source == TaskSource.AGENT_PLANNER, "Source field mismatch"
        assert retrieved.calculated_priority == 7.5, "Calculated priority mismatch"
        assert retrieved.dependency_depth == 2, "Dependency depth mismatch"
        assert retrieved.estimated_duration_seconds == 3600, "Estimated duration mismatch"
        assert retrieved.dependency_type == DependencyType.PARALLEL, "Dependency type mismatch"

    @pytest.mark.asyncio
    async def test_task_dependency_insert_and_retrieve(self, memory_db: Database) -> None:
        """Test inserting and retrieving task dependencies."""
        # Create prerequisite task
        prereq_task = Task(prompt="Prerequisite task", summary="Prerequisite task")
        await memory_db.insert_task(prereq_task)

        # Create dependent task
        dependent_task = Task(prompt="Dependent task", summary="Dependent task")
        await memory_db.insert_task(dependent_task)

        # Create dependency
        test_dep = TaskDependency(
            dependent_task_id=dependent_task.id,
            prerequisite_task_id=prereq_task.id,
            dependency_type=DependencyType.SEQUENTIAL,
        )

        await memory_db.insert_task_dependency(test_dep)

        # Retrieve dependencies
        deps = await memory_db.get_task_dependencies(dependent_task.id)
        assert len(deps) == 1, "Failed to retrieve task dependency"
        assert deps[0].prerequisite_task_id == prereq_task.id, "Retrieved dependency doesn't match"
        assert deps[0].dependency_type == DependencyType.SEQUENTIAL, "Dependency type mismatch"

    @pytest.mark.asyncio
    async def test_dependency_resolution_workflow(self, memory_db: Database) -> None:
        """Test dependency resolution marks dependencies as resolved."""
        # Create prerequisite task
        prereq_task = Task(prompt="Prerequisite task", summary="Prerequisite task")
        await memory_db.insert_task(prereq_task)

        # Create dependent task
        dependent_task = Task(prompt="Dependent task", summary="Dependent task")
        await memory_db.insert_task(dependent_task)

        # Create dependency
        test_dep = TaskDependency(
            dependent_task_id=dependent_task.id,
            prerequisite_task_id=prereq_task.id,
            dependency_type=DependencyType.SEQUENTIAL,
        )
        await memory_db.insert_task_dependency(test_dep)

        # Initial state: dependency should not be resolved
        deps = await memory_db.get_task_dependencies(dependent_task.id)
        assert deps[0].resolved_at is None, "Dependency should not be resolved initially"

        # Resolve dependency
        await memory_db.resolve_dependency(prereq_task.id)

        # Verify dependency is resolved
        resolved_deps = await memory_db.get_task_dependencies(dependent_task.id)
        assert (
            resolved_deps[0].resolved_at is not None
        ), "Dependency should be resolved after resolution"

    @pytest.mark.asyncio
    async def test_all_task_status_enums_supported(self, memory_db: Database) -> None:
        """Test that all TaskStatus enum values are properly persisted."""
        for status in TaskStatus:
            task = Task(prompt=f"Status test {status.value}", summary=f"Status test {status.value}", status=status)
            await memory_db.insert_task(task)
            retrieved = await memory_db.get_task(task.id)
            assert (
                retrieved is not None and retrieved.status == status
            ), f"TaskStatus {status.value} not persisted correctly"

    @pytest.mark.asyncio
    async def test_all_task_source_enums_supported(self, memory_db: Database) -> None:
        """Test that all TaskSource enum values are properly persisted."""
        for source in TaskSource:
            task = Task(prompt=f"Source test {source.value}", summary=f"Source test {source.value}", source=source)
            await memory_db.insert_task(task)
            retrieved = await memory_db.get_task(task.id)
            assert (
                retrieved is not None and retrieved.source == source
            ), f"TaskSource {source.value} not persisted correctly"

    @pytest.mark.asyncio
    async def test_all_dependency_type_enums_supported(self, memory_db: Database) -> None:
        """Test that all DependencyType enum values are properly persisted."""
        for dep_type in DependencyType:
            task = Task(prompt=f"DepType test {dep_type.value}", summary=f"DepType test {dep_type.value}", dependency_type=dep_type)
            await memory_db.insert_task(task)
            retrieved = await memory_db.get_task(task.id)
            assert (
                retrieved is not None and retrieved.dependency_type == dep_type
            ), f"DependencyType {dep_type.value} not persisted correctly"

    @pytest.mark.asyncio
    async def test_priority_queue_query_uses_index(self, memory_db: Database) -> None:
        """Test that priority queue query uses appropriate index."""
        query = """
            SELECT * FROM tasks
            WHERE status = 'ready'
            ORDER BY calculated_priority DESC, submitted_at ASC
            LIMIT 1
        """
        plan = await memory_db.explain_query_plan(query, ())
        plan_text = " ".join(plan).lower()

        # Should use an index for efficient lookups
        assert (
            "index" in plan_text or "idx_tasks_ready_priority" in plan_text
        ), f"Priority queue query should use index, plan: {plan_text}"

    @pytest.mark.asyncio
    async def test_dependency_resolution_query_uses_index(self, memory_db: Database) -> None:
        """Test that dependency resolution query uses appropriate index."""
        from uuid import uuid4

        test_id = str(uuid4())
        query = """
            SELECT * FROM task_dependencies
            WHERE prerequisite_task_id = ? AND resolved_at IS NULL
        """
        plan = await memory_db.explain_query_plan(query, (test_id,))
        plan_text = " ".join(plan).lower()

        # Should use idx_task_dependencies_prerequisite index
        assert (
            "index" in plan_text or "idx_task_dependencies_prerequisite" in plan_text
        ), f"Dependency resolution query should use index, plan: {plan_text}"

    @pytest.mark.asyncio
    async def test_task_foreign_key_constraints_enforced(self, memory_db: Database) -> None:
        """Test that foreign key constraints are enforced for task dependencies."""
        # Create dependent task
        dependent_task = Task(prompt="Dependent task", summary="Dependent task")
        await memory_db.insert_task(dependent_task)

        # Try to create dependency with non-existent prerequisite
        from uuid import UUID, uuid4

        fake_prereq_id = UUID(str(uuid4()))
        test_dep = TaskDependency(
            dependent_task_id=dependent_task.id,
            prerequisite_task_id=fake_prereq_id,
            dependency_type=DependencyType.SEQUENTIAL,
        )

        # Should fail with foreign key constraint error
        with pytest.raises(Exception) as exc_info:
            await memory_db.insert_task_dependency(test_dep)

        error_msg = str(exc_info.value).lower()
        assert (
            "foreign key" in error_msg or "constraint" in error_msg
        ), f"Expected foreign key error, got: {exc_info.value}"

    @pytest.mark.asyncio
    async def test_tasks_table_supports_nullable_optional_fields(self, memory_db: Database) -> None:
        """Test that optional Phase 1 fields can be NULL."""
        # Create task with minimal fields (let optional fields be None)
        minimal_task = Task(prompt="Minimal task", summary="Minimal task")

        await memory_db.insert_task(minimal_task)
        retrieved = await memory_db.get_task(minimal_task.id)

        assert retrieved is not None, "Failed to retrieve minimal task"
        # Optional fields should have default values or be None
        assert retrieved.deadline is None, "Deadline should be None"
        assert retrieved.dependency_depth == 0, "Dependency depth should default to 0"
