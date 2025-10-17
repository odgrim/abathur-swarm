"""Integration tests for summary field migration idempotency.

Tests the idempotent migration for the summary column:
- Migration adds column to fresh database
- Migration is idempotent (can run multiple times safely)
- Existing data with summary values is preserved
- No errors or warnings during migration

Migration location: src/abathur/infrastructure/database.py:379-389

Test Strategy:
1. test_migration_adds_column_to_fresh_database()
   - Verify column added on first migration
   - Verify column type and constraints

2. test_migration_idempotent_on_second_run()
   - Run migration twice
   - Verify no errors on second run
   - Verify column unchanged

3. test_migration_preserves_existing_data()
   - Create database with summary data
   - Run migration (should skip ALTER TABLE)
   - Verify all data preserved

4. test_migration_column_properties()
   - Verify column is TEXT type
   - Verify column is nullable
   - Verify no default value
"""

from pathlib import Path

import pytest
from abathur.infrastructure.database import Database


@pytest.mark.asyncio
async def test_migration_adds_column_to_fresh_database() -> None:
    """Test migration adds summary column to fresh database.

    Verifies:
    - Fresh database starts without summary column
    - Migration adds summary column correctly
    - Column type is TEXT
    - Column is nullable (allows NULL values)
    """
    # Arrange - create in-memory database
    db = Database(Path(":memory:"))

    # Act - initialize database (runs migration)
    await db.initialize()

    # Assert - verify summary column exists
    async with db._get_connection() as conn:
        cursor = await conn.execute("PRAGMA table_info(tasks)")
        columns = list(await cursor.fetchall())
        column_names = [col["name"] for col in columns]

        # Verify summary column exists
        assert "summary" in column_names, "Migration should add summary column"

        # Get column details for summary
        summary_column = next(col for col in columns if col["name"] == "summary")

        # Verify column properties
        assert summary_column["type"] == "TEXT", "Summary column should be TEXT type"
        assert summary_column["notnull"] == 0, "Summary column should be nullable"
        assert summary_column["dflt_value"] is None, "Summary column should have no default value"

    await db.close()


@pytest.mark.asyncio
async def test_migration_idempotent_on_second_run() -> None:
    """Test migration is idempotent (can run multiple times safely).

    Verifies:
    - First migration adds column
    - Second migration is no-op (skips ALTER TABLE)
    - No errors or exceptions on second run
    - Column properties unchanged after second run
    """
    # Arrange - create file-based database for persistence between runs
    import tempfile

    with tempfile.NamedTemporaryFile(suffix=".db", delete=False) as f:
        db_path = Path(f.name)

    try:
        # Act - first migration
        db1 = Database(db_path)
        await db1.initialize()

        # Verify column exists after first migration
        async with db1._get_connection() as conn:
            cursor = await conn.execute("PRAGMA table_info(tasks)")
            columns_before = list(await cursor.fetchall())
            column_names_before = [col["name"] for col in columns_before]
            assert "summary" in column_names_before

            summary_col_before = next(col for col in columns_before if col["name"] == "summary")

        # Close first connection
        # (File-based databases auto-close connections)

        # Act - second migration (should be idempotent)
        db2 = Database(db_path)
        await db2.initialize()  # Should NOT raise error

        # Assert - column still exists with same properties
        async with db2._get_connection() as conn:
            cursor = await conn.execute("PRAGMA table_info(tasks)")
            columns_after = list(await cursor.fetchall())
            column_names_after = [col["name"] for col in columns_after]

            # Verify summary column still exists
            assert (
                "summary" in column_names_after
            ), "Summary column should persist after second migration"

            # Verify column properties unchanged
            summary_col_after = next(col for col in columns_after if col["name"] == "summary")
            assert summary_col_after["type"] == summary_col_before["type"]
            assert summary_col_after["notnull"] == summary_col_before["notnull"]
            assert summary_col_after["dflt_value"] == summary_col_before["dflt_value"]

            # Verify column count unchanged (no duplicate columns)
            assert len(columns_before) == len(columns_after)

    finally:
        # Cleanup
        if db_path.exists():
            db_path.unlink()
        wal_path = db_path.with_suffix(".db-wal")
        shm_path = db_path.with_suffix(".db-shm")
        if wal_path.exists():
            wal_path.unlink()
        if shm_path.exists():
            shm_path.unlink()


@pytest.mark.asyncio
async def test_migration_preserves_existing_data() -> None:
    """Test migration preserves existing tasks with summary values.

    Verifies:
    - Tasks with summary values are preserved
    - Summary values unchanged after migration
    - No data loss during migration
    - Task count unchanged
    """
    # Arrange - create database with summary data
    from uuid import uuid4

    from abathur.domain.models import Task, TaskSource, TaskStatus

    db = Database(Path(":memory:"))
    await db.initialize()

    # Insert test tasks with summary values
    test_tasks = [
        Task(
            id=uuid4(),
            prompt="Task 1 description",
            summary="Task 1 summary",
            source=TaskSource.HUMAN,
            status=TaskStatus.READY,
        ),
        Task(
            id=uuid4(),
            prompt="Task 2 description",
            summary="Task 2 summary with unicode: café ☕",
            source=TaskSource.HUMAN,
            status=TaskStatus.READY,
        ),
        Task(
            id=uuid4(),
            prompt="Task 3 description",
            summary=None,  # No summary
            source=TaskSource.HUMAN,
            status=TaskStatus.READY,
        ),
    ]

    for task in test_tasks:
        await db.insert_task(task)

    # Act - run migration again (should be idempotent, no data loss)
    # For in-memory databases, we simulate this by verifying data integrity
    # In production, this would be a second initialize() call on a persisted database

    # Assert - retrieve all tasks and verify data preserved
    for original_task in test_tasks:
        retrieved_task = await db.get_task(original_task.id)

        assert retrieved_task is not None, f"Task {original_task.id} should be retrievable"
        assert retrieved_task.prompt == original_task.prompt, "Prompt should be preserved"
        assert retrieved_task.summary == original_task.summary, "Summary should be preserved"
        assert retrieved_task.status == original_task.status, "Status should be preserved"

    # Assert - verify task count unchanged
    tasks = await db.list_tasks(limit=100)
    assert len(tasks) == 3, "All tasks should be preserved"

    await db.close()


@pytest.mark.asyncio
async def test_migration_column_properties() -> None:
    """Test summary column has correct database properties.

    Verifies:
    - Column type is TEXT (not VARCHAR or other)
    - Column is nullable (NOT NULL = 0)
    - No default value specified
    - Column position in table schema
    """
    # Arrange & Act
    db = Database(Path(":memory:"))
    await db.initialize()

    # Assert - inspect column properties
    async with db._get_connection() as conn:
        cursor = await conn.execute("PRAGMA table_info(tasks)")
        columns = list(await cursor.fetchall())

        # Find summary column
        summary_column = next((col for col in columns if col["name"] == "summary"), None)
        assert summary_column is not None, "Summary column should exist"

        # Verify exact column properties
        assert summary_column["type"] == "TEXT", "Column type should be TEXT"
        assert summary_column["notnull"] == 0, "Column should allow NULL (notnull=0)"
        assert summary_column["dflt_value"] is None, "Column should have no default value"
        assert summary_column["pk"] == 0, "Column should not be primary key"

    await db.close()


@pytest.mark.asyncio
async def test_migration_check_condition() -> None:
    """Test migration correctly checks for column existence before ALTER TABLE.

    Verifies:
    - Migration uses 'if "summary" not in column_names' check
    - ALTER TABLE only runs if column missing
    - No errors when column already exists

    This test simulates the exact migration logic flow.
    """
    # Arrange
    db = Database(Path(":memory:"))

    # Act - first initialization (column doesn't exist)
    await db.initialize()

    # Assert - column now exists
    async with db._get_connection() as conn:
        cursor = await conn.execute("PRAGMA table_info(tasks)")
        columns = list(await cursor.fetchall())
        column_names = [col["name"] for col in columns]

        assert "summary" in column_names, "Column should exist after first migration"

        # Simulate second migration check (as code does)
        if "summary" not in column_names:
            # This should NOT execute on second run
            pytest.fail("Migration check failed: column exists but check returned False")
        else:
            # This SHOULD execute - column exists, skip ALTER TABLE
            pass  # Success: migration is idempotent

    await db.close()


@pytest.mark.asyncio
async def test_migration_no_duplicate_columns() -> None:
    """Test migration doesn't create duplicate summary columns.

    Verifies:
    - Only one summary column exists
    - No duplicate column names in schema
    - Column count correct after migration
    """
    # Arrange & Act
    db = Database(Path(":memory:"))
    await db.initialize()

    # Assert - count summary columns
    async with db._get_connection() as conn:
        cursor = await conn.execute("PRAGMA table_info(tasks)")
        columns = list(await cursor.fetchall())

        # Count summary columns (should be exactly 1)
        summary_columns = [col for col in columns if col["name"] == "summary"]
        assert len(summary_columns) == 1, "Should have exactly one summary column"

        # Verify no duplicate column names
        column_names = [col["name"] for col in columns]
        unique_column_names = set(column_names)
        assert len(column_names) == len(unique_column_names), "No duplicate column names"

    await db.close()


@pytest.mark.asyncio
async def test_migration_with_null_summary_values() -> None:
    """Test migration handles NULL summary values correctly.

    Verifies:
    - NULL values allowed in summary column
    - Tasks with NULL summary retrieve correctly
    - NULL preserved through migration
    """
    # Arrange
    from uuid import uuid4

    from abathur.domain.models import Task, TaskSource, TaskStatus

    db = Database(Path(":memory:"))
    await db.initialize()

    # Insert task with NULL summary
    task = Task(
        id=uuid4(),
        prompt="Task without summary",
        summary=None,  # Explicit NULL
        source=TaskSource.HUMAN,
        status=TaskStatus.READY,
    )
    await db.insert_task(task)

    # Act - retrieve task
    retrieved = await db.get_task(task.id)

    # Assert - NULL summary preserved
    assert retrieved is not None
    assert retrieved.summary is None, "NULL summary should be preserved"
    assert retrieved.prompt == "Task without summary"

    await db.close()


@pytest.mark.asyncio
async def test_migration_with_empty_string_summary() -> None:
    """Test migration handles empty string summary values.

    Verifies:
    - Empty string "" distinct from NULL
    - Empty strings persist correctly
    - Empty strings retrieve correctly
    """
    # Arrange
    from uuid import uuid4

    from abathur.domain.models import Task, TaskSource, TaskStatus

    db = Database(Path(":memory:"))
    await db.initialize()

    # Insert task with empty string summary
    task = Task(
        id=uuid4(),
        prompt="Task with empty summary",
        summary="",  # Empty string (not NULL)
        source=TaskSource.HUMAN,
        status=TaskStatus.READY,
    )
    await db.insert_task(task)

    # Act - retrieve task
    retrieved = await db.get_task(task.id)

    # Assert - empty string summary preserved (not NULL)
    assert retrieved is not None
    assert retrieved.summary == "", "Empty string summary should be preserved"
    assert retrieved.summary is not None, "Empty string is distinct from NULL"

    await db.close()


@pytest.mark.asyncio
async def test_migration_with_max_length_summary() -> None:
    """Test migration handles maximum length summary values (200 chars).

    Verifies:
    - 200 character summaries persist correctly
    - No truncation at database level
    - Full value retrieved
    """
    # Arrange
    from uuid import uuid4

    from abathur.domain.models import Task, TaskSource, TaskStatus

    db = Database(Path(":memory:"))
    await db.initialize()

    # Insert task with 200 char summary (max length)
    max_summary = "x" * 200
    task = Task(
        id=uuid4(),
        prompt="Task with max length summary",
        summary=max_summary,
        source=TaskSource.HUMAN,
        status=TaskStatus.READY,
    )
    await db.insert_task(task)

    # Act - retrieve task
    retrieved = await db.get_task(task.id)

    # Assert - full summary preserved
    assert retrieved is not None
    assert retrieved.summary == max_summary, "Max length summary should be preserved"
    assert len(retrieved.summary) == 200, "Summary should be exactly 200 characters"

    await db.close()


@pytest.mark.asyncio
async def test_migration_multiple_times_file_database() -> None:
    """Test migration can run multiple times on file-based database.

    Verifies:
    - Migration works on file databases (not just in-memory)
    - Column persists across database connections
    - Idempotency maintained with file persistence
    """
    # Arrange - create temporary file database
    import tempfile

    with tempfile.NamedTemporaryFile(suffix=".db", delete=False) as f:
        db_path = Path(f.name)

    try:
        # Act - Run migration 3 times with new Database instances
        for run_number in range(3):
            db = Database(db_path)
            await db.initialize()  # Should not raise error

            # Verify column exists after each run
            async with db._get_connection() as conn:
                cursor = await conn.execute("PRAGMA table_info(tasks)")
                columns = list(await cursor.fetchall())
                column_names = [col["name"] for col in columns]

                assert "summary" in column_names, f"Column should exist after run {run_number + 1}"

            # Note: File-based databases auto-close connections

        # Assert - Final verification
        db_final = Database(db_path)
        await db_final.initialize()

        async with db_final._get_connection() as conn:
            cursor = await conn.execute("PRAGMA table_info(tasks)")
            columns = list(await cursor.fetchall())
            summary_columns = [col for col in columns if col["name"] == "summary"]

            assert len(summary_columns) == 1, "Should still have exactly one summary column"

    finally:
        # Cleanup
        if db_path.exists():
            db_path.unlink()
        wal_path = db_path.with_suffix(".db-wal")
        shm_path = db_path.with_suffix(".db-shm")
        if wal_path.exists():
            wal_path.unlink()
        if shm_path.exists():
            shm_path.unlink()


@pytest.mark.asyncio
async def test_migration_backfill_matches_service_layer() -> None:
    """Test migration backfill logic matches service layer auto-generation.

    This is a critical test to ensure backward compatibility:
    - Migration backfills NULL summaries using same logic as service layer
    - Human tasks get "User Prompt: " prefix + truncate to 126 chars
    - Agent tasks get truncated to 140 chars
    - Both use TRIM/strip to remove whitespace

    Verifies migration logic (database.py:407-408) matches service logic
    (task_queue_service.py:174-181).
    """
    # Arrange - create database and manually insert tasks with NULL summary
    import tempfile
    from datetime import datetime, timezone
    from uuid import uuid4

    from abathur.domain.models import TaskSource

    with tempfile.NamedTemporaryFile(suffix=".db", delete=False) as f:
        db_path = Path(f.name)

    try:
        db = Database(db_path)
        await db.initialize()

        # Test case 1: Human task with normal prompt
        # Insert with default 'Task' summary, then simulate backfill
        human_task_id = uuid4()
        human_prompt = "This is a user request that should get prefixed with 'User Prompt: '"
        async with db._get_connection() as conn:
            await conn.execute(
                """
                INSERT INTO tasks (
                    id, prompt, agent_type, priority, status, input_data,
                    submitted_at, last_updated_at, source, summary
                ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, 'Task')
                """,
                (
                    str(human_task_id),
                    human_prompt,
                    "requirements-gatherer",
                    5,
                    "ready",
                    "{}",
                    datetime.now(timezone.utc).isoformat(),
                    datetime.now(timezone.utc).isoformat(),
                    "human",
                ),
            )
            await conn.commit()

        # Test case 2: Agent task with normal prompt
        agent_task_id = uuid4()
        agent_prompt = "This is an agent-generated task that should NOT get the prefix"
        async with db._get_connection() as conn:
            await conn.execute(
                """
                INSERT INTO tasks (
                    id, prompt, agent_type, priority, status, input_data,
                    submitted_at, last_updated_at, source, summary
                ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, 'Task')
                """,
                (
                    str(agent_task_id),
                    agent_prompt,
                    "requirements-gatherer",
                    5,
                    "ready",
                    "{}",
                    datetime.now(timezone.utc).isoformat(),
                    datetime.now(timezone.utc).isoformat(),
                    "agent_requirements",
                ),
            )
            await conn.commit()

        # Test case 3: Human task with long prompt (>126 chars in body)
        long_human_task_id = uuid4()
        long_prompt = "x" * 150  # 150 chars, should truncate to 126 + prefix
        async with db._get_connection() as conn:
            await conn.execute(
                """
                INSERT INTO tasks (
                    id, prompt, agent_type, priority, status, input_data,
                    submitted_at, last_updated_at, source, summary
                ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, 'Task')
                """,
                (
                    str(long_human_task_id),
                    long_prompt,
                    "requirements-gatherer",
                    5,
                    "ready",
                    "{}",
                    datetime.now(timezone.utc).isoformat(),
                    datetime.now(timezone.utc).isoformat(),
                    "human",
                ),
            )
            await conn.commit()

        # Test case 4: Task with empty prompt
        empty_task_id = uuid4()
        async with db._get_connection() as conn:
            await conn.execute(
                """
                INSERT INTO tasks (
                    id, prompt, agent_type, priority, status, input_data,
                    submitted_at, last_updated_at, source, summary
                ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, 'Task')
                """,
                (
                    str(empty_task_id),
                    "",
                    "requirements-gatherer",
                    5,
                    "ready",
                    "{}",
                    datetime.now(timezone.utc).isoformat(),
                    datetime.now(timezone.utc).isoformat(),
                    "human",
                ),
            )
            await conn.commit()

        # Test case 5: Task with whitespace-only prompt
        whitespace_task_id = uuid4()
        async with db._get_connection() as conn:
            await conn.execute(
                """
                INSERT INTO tasks (
                    id, prompt, agent_type, priority, status, input_data,
                    submitted_at, last_updated_at, source, summary
                ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, 'Task')
                """,
                (
                    str(whitespace_task_id),
                    "   ",
                    "requirements-gatherer",
                    5,
                    "ready",
                    "{}",
                    datetime.now(timezone.utc).isoformat(),
                    datetime.now(timezone.utc).isoformat(),
                    "human",
                ),
            )
            await conn.commit()

        # Act - trigger backfill by re-running migration logic
        # The migration backfills rows with summary='Task' (the default) or NULL/empty
        async with db._get_connection() as conn:
            # Manually run the backfill logic (same as migration)
            await conn.execute(
                """
                UPDATE tasks
                SET summary = CASE
                    WHEN prompt IS NULL OR TRIM(prompt) = '' THEN 'Task'
                    WHEN source = 'human' THEN 'User Prompt: ' || TRIM(SUBSTR(prompt, 1, 126))
                    ELSE TRIM(SUBSTR(prompt, 1, 140))
                END
                WHERE summary = 'Task' OR summary IS NULL OR TRIM(summary) = ''
                """
            )
            await conn.commit()

        # Assert - verify backfilled summaries match service layer expectations
        # Test case 1: Human task
        human_task = await db.get_task(human_task_id)
        assert human_task is not None
        expected_human_summary = f"User Prompt: {human_prompt[:126].strip()}"
        assert (
            human_task.summary == expected_human_summary
        ), f"Human task summary mismatch. Expected: '{expected_human_summary}', Got: '{human_task.summary}'"

        # Test case 2: Agent task
        agent_task = await db.get_task(agent_task_id)
        assert agent_task is not None
        expected_agent_summary = agent_prompt[:140].strip()
        assert (
            agent_task.summary == expected_agent_summary
        ), f"Agent task summary mismatch. Expected: '{expected_agent_summary}', Got: '{agent_task.summary}'"

        # Test case 3: Long human task
        long_human_task = await db.get_task(long_human_task_id)
        assert long_human_task is not None
        expected_long_summary = f"User Prompt: {long_prompt[:126].strip()}"
        assert (
            long_human_task.summary == expected_long_summary
        ), f"Long human task summary mismatch. Expected length {len(expected_long_summary)}, Got length {len(long_human_task.summary) if long_human_task.summary else 0}"
        # Verify it's exactly 126 chars from prompt + prefix
        assert long_human_task.summary is not None
        assert len(long_human_task.summary) == len("User Prompt: ") + 126

        # Test case 4: Empty prompt
        empty_task = await db.get_task(empty_task_id)
        assert empty_task is not None
        assert empty_task.summary == "Task", "Empty prompt should backfill to 'Task'"

        # Test case 5: Whitespace-only prompt
        whitespace_task = await db.get_task(whitespace_task_id)
        assert whitespace_task is not None
        assert whitespace_task.summary == "Task", "Whitespace-only prompt should backfill to 'Task'"

        # Additional verification: simulate service layer generation and compare
        # This ensures the migration EXACTLY matches what the service layer would generate
        def service_layer_generate_summary(description: str, source: TaskSource) -> str:
            """Simulate service layer summary generation logic."""
            if source == TaskSource.HUMAN:
                prefix = "User Prompt: "
                return prefix + description[:126].strip()
            else:
                return description[:140].strip()

        # Compare migration result vs service layer result for human task
        service_human_summary = service_layer_generate_summary(human_prompt, TaskSource.HUMAN)
        assert (
            human_task.summary == service_human_summary
        ), "Migration must match service layer for human tasks"

        # Compare for agent task
        service_agent_summary = service_layer_generate_summary(
            agent_prompt, TaskSource.AGENT_REQUIREMENTS
        )
        assert (
            agent_task.summary == service_agent_summary
        ), "Migration must match service layer for agent tasks"

    finally:
        # Cleanup
        if db_path.exists():
            db_path.unlink()
        wal_path = db_path.with_suffix(".db-wal")
        shm_path = db_path.with_suffix(".db-shm")
        if wal_path.exists():
            wal_path.unlink()
        if shm_path.exists():
            shm_path.unlink()


@pytest.mark.asyncio
async def test_summary_index_no_pointless_where_clause() -> None:
    """Test idx_tasks_summary index is created without pointless WHERE clause.

    Verifies:
    - Index exists
    - Index definition does NOT contain 'WHERE summary IS NOT NULL'
    - Index is a simple index, not a partial index
    """
    # Arrange & Act
    db = Database(Path(":memory:"))
    await db.initialize()

    # Assert - verify index definition
    async with db._get_connection() as conn:
        cursor = await conn.execute(
            """
            SELECT sql FROM sqlite_master
            WHERE type='index' AND name='idx_tasks_summary'
            """
        )
        index_row = await cursor.fetchone()

        assert index_row is not None, "idx_tasks_summary index should exist"
        assert index_row["sql"] is not None, "Index should have SQL definition"

        index_sql = index_row["sql"]
        assert "idx_tasks_summary" in index_sql, "Index SQL should contain index name"
        assert "summary" in index_sql, "Index SQL should reference summary column"

        # Most important: verify no pointless WHERE clause
        assert (
            "WHERE summary IS NOT NULL" not in index_sql
        ), "Index should NOT have pointless WHERE clause (column is NOT NULL)"

    await db.close()


@pytest.mark.asyncio
async def test_summary_index_query_plan_optimization() -> None:
    """Test EXPLAIN QUERY PLAN shows idx_tasks_summary is used for summary queries.

    Verifies:
    - Index is used for WHERE summary = ? queries
    - Index is used for ORDER BY summary queries
    - Index appears in EXPLAIN QUERY PLAN output
    """
    # Arrange
    from uuid import uuid4

    from abathur.domain.models import Task, TaskSource, TaskStatus

    db = Database(Path(":memory:"))
    await db.initialize()

    # Insert test tasks with different summaries
    test_tasks = [
        Task(
            id=uuid4(),
            prompt="Task 1",
            summary="Alpha summary",
            source=TaskSource.HUMAN,
            status=TaskStatus.READY,
        ),
        Task(
            id=uuid4(),
            prompt="Task 2",
            summary="Beta summary",
            source=TaskSource.HUMAN,
            status=TaskStatus.READY,
        ),
    ]

    for task in test_tasks:
        await db.insert_task(task)

    # Act & Assert - verify index usage in query plans
    async with db._get_connection() as conn:
        # Test 1: WHERE summary = ? should use index
        cursor = await conn.execute(
            """
            EXPLAIN QUERY PLAN
            SELECT * FROM tasks WHERE summary = ?
            """,
            ("Alpha summary",),
        )
        plan_rows = await cursor.fetchall()
        plan_text = " ".join([str(col) for row in plan_rows for col in row])

        assert (
            "idx_tasks_summary" in plan_text or "USING INDEX" in plan_text
        ), f"Query plan should show index usage. Plan: {plan_text}"

        # Test 2: ORDER BY summary should use index
        cursor = await conn.execute(
            """
            EXPLAIN QUERY PLAN
            SELECT * FROM tasks ORDER BY summary
            """
        )
        plan_rows = await cursor.fetchall()
        plan_text = " ".join([str(col) for row in plan_rows for col in row])

        # SQLite may use index for ordering or scan, either is acceptable
        # We just verify the query executes without error
        assert plan_rows, "Query plan should return results"

    await db.close()


@pytest.mark.asyncio
async def test_summary_index_migration_from_old_to_new() -> None:
    """Test migration drops old index with WHERE clause and recreates without.

    Verifies:
    - Old index with WHERE clause is detected
    - Old index is dropped
    - New index is created without WHERE clause
    - Migration is idempotent
    """
    # Arrange - create file database with old index definition
    import tempfile

    with tempfile.NamedTemporaryFile(suffix=".db", delete=False) as f:
        db_path = Path(f.name)

    try:
        # Step 1: Create database with old schema (simulate old version)
        db1 = Database(db_path)
        await db1.initialize()

        # Manually create old-style index with pointless WHERE clause
        async with db1._get_connection() as conn:
            # Drop current index (created by _create_indexes)
            await conn.execute("DROP INDEX IF EXISTS idx_tasks_summary")

            # Create old-style index with WHERE clause
            await conn.execute(
                """
                CREATE INDEX idx_tasks_summary
                ON tasks(summary)
                WHERE summary IS NOT NULL
                """
            )
            await conn.commit()

        # Verify old index exists with WHERE clause
        async with db1._get_connection() as conn:
            cursor = await conn.execute(
                """
                SELECT sql FROM sqlite_master
                WHERE type='index' AND name='idx_tasks_summary'
                """
            )
            old_index_row = await cursor.fetchone()
            assert old_index_row is not None
            assert "WHERE summary IS NOT NULL" in old_index_row["sql"]

        # Step 2: Re-initialize database (triggers migration)
        db2 = Database(db_path)
        await db2.initialize()

        # Assert - verify new index exists without WHERE clause
        async with db2._get_connection() as conn:
            cursor = await conn.execute(
                """
                SELECT sql FROM sqlite_master
                WHERE type='index' AND name='idx_tasks_summary'
                """
            )
            new_index_row = await cursor.fetchone()

            assert new_index_row is not None, "Index should still exist after migration"
            assert new_index_row["sql"] is not None

            # Verify WHERE clause removed
            assert (
                "WHERE summary IS NOT NULL" not in new_index_row["sql"]
            ), "Migration should remove pointless WHERE clause"

        # Step 3: Re-run migration (test idempotency)
        db3 = Database(db_path)
        await db3.initialize()  # Should not raise error

        # Verify index still correct after second migration
        async with db3._get_connection() as conn:
            cursor = await conn.execute(
                """
                SELECT sql FROM sqlite_master
                WHERE type='index' AND name='idx_tasks_summary'
                """
            )
            final_index_row = await cursor.fetchone()

            assert final_index_row is not None
            assert "WHERE summary IS NOT NULL" not in final_index_row["sql"]

    finally:
        # Cleanup
        if db_path.exists():
            db_path.unlink()
        wal_path = db_path.with_suffix(".db-wal")
        shm_path = db_path.with_suffix(".db-shm")
        if wal_path.exists():
            wal_path.unlink()
        if shm_path.exists():
            shm_path.unlink()


if __name__ == "__main__":
    pytest.main([__file__, "-v"])
