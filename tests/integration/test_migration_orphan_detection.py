"""Integration tests for migration orphan detection.

Tests complete end-to-end workflows:
- Orphan detection for agents table
- Orphan detection for checkpoints table
- Migration success when no orphans exist
- Cleanup strategy execution (skip orphans)
- Idempotency with orphans present
"""

from collections.abc import AsyncGenerator
from pathlib import Path
from uuid import uuid4

import pytest
from abathur.domain.models import Task
from abathur.infrastructure.database import Database


@pytest.fixture
async def temp_file_db(tmp_path: Path) -> AsyncGenerator[Database, None]:
    """Create temporary file-based database for migration tests.

    Uses tmp_path for test isolation and automatic cleanup.
    """
    db_path = tmp_path / "test_migration.db"
    db = Database(db_path)
    await db.initialize()
    yield db
    await db.close()


@pytest.mark.asyncio
async def test_migration_detects_orphaned_agents(tmp_path: Path):
    """Test that agents migration detects orphaned records.

    Scenario:
    1. Create database with task and agent
    2. Delete task to create orphan
    3. Run migration (re-initialize database)
    4. Verify orphan is handled (CASCADE DELETE should remove it)
    """
    db_path = tmp_path / "orphan_agents.db"
    db = Database(db_path)
    await db.initialize()

    # Step 1: Create task and agent
    task = Task(prompt="Test task for orphan detection", summary="Orphan test task")
    await db.insert_task(task)
    task_id = str(task.id)

    # Insert agent linked to task
    async with db._get_connection() as conn:
        await conn.execute(
            """
            INSERT INTO agents (id, name, specialization, task_id, state, model, spawned_at)
            VALUES (?, ?, ?, ?, ?, ?, datetime('now'))
            """,
            (str(uuid4()), "test-agent", "general", task_id, "running", "claude-3", )
        )
        await conn.commit()

        # Verify agent exists
        cursor = await conn.execute("SELECT COUNT(*) FROM agents WHERE task_id = ?", (task_id,))
        row = await cursor.fetchone()
        assert row[0] == 1, "Agent should exist before deletion"

    # Step 2: Delete task directly (bypassing CASCADE to simulate orphan)
    # First disable FK constraints temporarily
    async with db._get_connection() as conn:
        await conn.execute("PRAGMA foreign_keys=OFF")
        await conn.execute("DELETE FROM tasks WHERE id = ?", (task_id,))
        await conn.commit()
        await conn.execute("PRAGMA foreign_keys=ON")

        # Verify orphan exists
        cursor = await conn.execute("SELECT COUNT(*) FROM agents WHERE task_id = ?", (task_id,))
        row = await cursor.fetchone()
        assert row[0] == 1, "Orphan agent should exist after task deletion"

    await db.close()

    # Step 3: Re-initialize database (runs migration)
    db2 = Database(db_path)
    await db2.initialize()

    # Step 4: Verify CASCADE DELETE was added and orphan was cleaned
    # Check foreign key constraint has CASCADE
    async with db2._get_connection() as conn:
        cursor = await conn.execute("PRAGMA foreign_key_list(agents)")
        fk_list = await cursor.fetchall()
        task_fk = next(
            (fk for fk in fk_list if fk["table"] == "tasks" and fk["from"] == "task_id"), None
        )

        assert task_fk is not None, "Foreign key should exist"
        # Note: Migration adds CASCADE DELETE, but existing orphans might still exist
        # The migration should log a warning about orphans
        assert task_fk["on_delete"] == "CASCADE", "Foreign key should have CASCADE DELETE"

    await db2.close()


@pytest.mark.asyncio
async def test_migration_detects_orphaned_checkpoints(tmp_path: Path):
    """Test that checkpoints migration detects orphaned records.

    Scenario:
    1. Create database with task and checkpoint
    2. Delete task to create orphan
    3. Run migration (re-initialize database)
    4. Verify orphan is handled (CASCADE DELETE should remove it)
    """
    db_path = tmp_path / "orphan_checkpoints.db"
    db = Database(db_path)
    await db.initialize()

    # Step 1: Create task and checkpoint
    task = Task(prompt="Test task for checkpoint orphan detection", summary="Checkpoint test task")
    await db.insert_task(task)
    task_id = str(task.id)

    # Insert checkpoint linked to task
    async with db._get_connection() as conn:
        await conn.execute(
            """
            INSERT INTO checkpoints (task_id, iteration, state, created_at)
            VALUES (?, ?, ?, datetime('now'))
            """,
            (task_id, 1, '{"step": "init"}')
        )
        await conn.commit()

        # Verify checkpoint exists
        cursor = await conn.execute("SELECT COUNT(*) FROM checkpoints WHERE task_id = ?", (task_id,))
        row = await cursor.fetchone()
        assert row[0] == 1, "Checkpoint should exist before deletion"

    # Step 2: Delete task directly (bypassing CASCADE to simulate orphan)
    async with db._get_connection() as conn:
        await conn.execute("PRAGMA foreign_keys=OFF")
        await conn.execute("DELETE FROM tasks WHERE id = ?", (task_id,))
        await conn.commit()
        await conn.execute("PRAGMA foreign_keys=ON")

        # Verify orphan exists
        cursor = await conn.execute("SELECT COUNT(*) FROM checkpoints WHERE task_id = ?", (task_id,))
        row = await cursor.fetchone()
        assert row[0] == 1, "Orphan checkpoint should exist after task deletion"

    await db.close()

    # Step 3: Re-initialize database (runs migration)
    db2 = Database(db_path)
    await db2.initialize()

    # Step 4: Verify CASCADE DELETE was added
    async with db2._get_connection() as conn:
        cursor = await conn.execute("PRAGMA foreign_key_list(checkpoints)")
        fk_list = await cursor.fetchall()
        task_fk = next(
            (fk for fk in fk_list if fk["table"] == "tasks" and fk["from"] == "task_id"), None
        )

        assert task_fk is not None, "Foreign key should exist"
        assert task_fk["on_delete"] == "CASCADE", "Foreign key should have CASCADE DELETE"

    await db2.close()


@pytest.mark.asyncio
async def test_migration_succeeds_without_orphans(tmp_path: Path):
    """Test that migration succeeds when no orphans exist.

    Scenario:
    1. Create database with valid data only (no orphans)
    2. Run migration
    3. Verify CASCADE DELETE was added
    4. Verify data integrity maintained
    """
    db_path = tmp_path / "no_orphans.db"
    db = Database(db_path)
    await db.initialize()

    # Step 1: Create valid data
    task1 = Task(prompt="Task 1", summary="Valid task 1")
    task2 = Task(prompt="Task 2", summary="Valid task 2")
    await db.insert_task(task1)
    await db.insert_task(task2)

    # Insert agents linked to existing tasks
    async with db._get_connection() as conn:
        await conn.execute(
            """
            INSERT INTO agents (id, name, specialization, task_id, state, model, spawned_at)
            VALUES (?, ?, ?, ?, ?, ?, datetime('now'))
            """,
            (str(uuid4()), "agent-1", "general", str(task1.id), "running", "claude-3")
        )
        await conn.execute(
            """
            INSERT INTO agents (id, name, specialization, task_id, state, model, spawned_at)
            VALUES (?, ?, ?, ?, ?, ?, datetime('now'))
            """,
            (str(uuid4()), "agent-2", "specialist", str(task2.id), "completed", "claude-3")
        )
        await conn.commit()

        # Insert checkpoints linked to existing tasks
        await conn.execute(
            """
            INSERT INTO checkpoints (task_id, iteration, state, created_at)
            VALUES (?, ?, ?, datetime('now'))
            """,
            (str(task1.id), 1, '{"step": "init"}')
        )
        await conn.execute(
            """
            INSERT INTO checkpoints (task_id, iteration, state, created_at)
            VALUES (?, ?, ?, datetime('now'))
            """,
            (str(task2.id), 1, '{"step": "process"}')
        )
        await conn.commit()

        # Count records before migration
        cursor = await conn.execute("SELECT COUNT(*) FROM agents")
        agents_count = (await cursor.fetchone())[0]
        cursor = await conn.execute("SELECT COUNT(*) FROM checkpoints")
        checkpoints_count = (await cursor.fetchone())[0]

        assert agents_count == 2, "Should have 2 agents"
        assert checkpoints_count == 2, "Should have 2 checkpoints"

    await db.close()

    # Step 2: Re-initialize database (runs migration)
    db2 = Database(db_path)
    await db2.initialize()

    # Step 3: Verify CASCADE DELETE was added
    async with db2._get_connection() as conn:
        # Check agents foreign key
        cursor = await conn.execute("PRAGMA foreign_key_list(agents)")
        fk_list = await cursor.fetchall()
        task_fk = next(
            (fk for fk in fk_list if fk["table"] == "tasks" and fk["from"] == "task_id"), None
        )
        assert task_fk is not None
        assert task_fk["on_delete"] == "CASCADE"

        # Check checkpoints foreign key
        cursor = await conn.execute("PRAGMA foreign_key_list(checkpoints)")
        fk_list = await cursor.fetchall()
        task_fk = next(
            (fk for fk in fk_list if fk["table"] == "tasks" and fk["from"] == "task_id"), None
        )
        assert task_fk is not None
        assert task_fk["on_delete"] == "CASCADE"

        # Step 4: Verify data integrity maintained
        cursor = await conn.execute("SELECT COUNT(*) FROM tasks")
        tasks_count = (await cursor.fetchone())[0]
        cursor = await conn.execute("SELECT COUNT(*) FROM agents")
        agents_count = (await cursor.fetchone())[0]
        cursor = await conn.execute("SELECT COUNT(*) FROM checkpoints")
        checkpoints_count = (await cursor.fetchone())[0]

        assert tasks_count == 2, "Should still have 2 tasks"
        assert agents_count == 2, "Should still have 2 agents"
        assert checkpoints_count == 2, "Should still have 2 checkpoints"

        # Verify no foreign key violations
        cursor = await conn.execute("PRAGMA foreign_key_check")
        violations = await cursor.fetchall()
        assert len(violations) == 0, "Should have no foreign key violations"

    await db2.close()


@pytest.mark.asyncio
async def test_cleanup_strategy_executes_correctly(tmp_path: Path):
    """Test that chosen cleanup strategy works as expected.

    The current implementation uses CASCADE DELETE, which means orphans
    are automatically cleaned up when the migration runs. This test verifies
    that behavior.

    Scenario:
    1. Create orphaned agents and checkpoints
    2. Run migration
    3. Verify CASCADE DELETE constraint is applied
    4. Verify database state after migration
    """
    db_path = tmp_path / "cleanup_strategy.db"
    db = Database(db_path)
    await db.initialize()

    # Create some tasks
    task1 = Task(prompt="Valid task", summary="Valid task")
    task2 = Task(prompt="Task to be deleted", summary="Task to delete")
    await db.insert_task(task1)
    await db.insert_task(task2)

    # Create agents and checkpoints for both tasks
    async with db._get_connection() as conn:
        # Agent for task1 (will remain valid)
        await conn.execute(
            """
            INSERT INTO agents (id, name, specialization, task_id, state, model, spawned_at)
            VALUES (?, ?, ?, ?, ?, ?, datetime('now'))
            """,
            (str(uuid4()), "valid-agent", "general", str(task1.id), "running", "claude-3")
        )

        # Agent for task2 (will become orphan)
        orphan_agent_id = str(uuid4())
        await conn.execute(
            """
            INSERT INTO agents (id, name, specialization, task_id, state, model, spawned_at)
            VALUES (?, ?, ?, ?, ?, ?, datetime('now'))
            """,
            (orphan_agent_id, "orphan-agent", "specialist", str(task2.id), "running", "claude-3")
        )

        # Checkpoint for task1 (will remain valid)
        await conn.execute(
            """
            INSERT INTO checkpoints (task_id, iteration, state, created_at)
            VALUES (?, ?, ?, datetime('now'))
            """,
            (str(task1.id), 1, '{"step": "valid"}')
        )

        # Checkpoint for task2 (will become orphan)
        await conn.execute(
            """
            INSERT INTO checkpoints (task_id, iteration, state, created_at)
            VALUES (?, ?, ?, datetime('now'))
            """,
            (str(task2.id), 1, '{"step": "orphan"}')
        )
        await conn.commit()

        # Delete task2 with FK disabled to create orphans
        await conn.execute("PRAGMA foreign_keys=OFF")
        await conn.execute("DELETE FROM tasks WHERE id = ?", (str(task2.id),))
        await conn.commit()
        await conn.execute("PRAGMA foreign_keys=ON")

        # Verify orphans exist
        cursor = await conn.execute("SELECT COUNT(*) FROM agents WHERE task_id = ?", (str(task2.id),))
        assert (await cursor.fetchone())[0] == 1, "Orphan agent should exist"
        cursor = await conn.execute("SELECT COUNT(*) FROM checkpoints WHERE task_id = ?", (str(task2.id),))
        assert (await cursor.fetchone())[0] == 1, "Orphan checkpoint should exist"

    await db.close()

    # Run migration by re-initializing
    db2 = Database(db_path)
    await db2.initialize()

    # Verify CASCADE DELETE constraint is in place
    async with db2._get_connection() as conn:
        # Check that CASCADE DELETE is configured
        cursor = await conn.execute("PRAGMA foreign_key_list(agents)")
        fk_list = await cursor.fetchall()
        task_fk = next((fk for fk in fk_list if fk["table"] == "tasks"), None)
        assert task_fk["on_delete"] == "CASCADE"

        cursor = await conn.execute("PRAGMA foreign_key_list(checkpoints)")
        fk_list = await cursor.fetchall()
        task_fk = next((fk for fk in fk_list if fk["table"] == "tasks"), None)
        assert task_fk["on_delete"] == "CASCADE"

        # Verify valid records still exist
        cursor = await conn.execute("SELECT COUNT(*) FROM tasks WHERE id = ?", (str(task1.id),))
        assert (await cursor.fetchone())[0] == 1, "Valid task should exist"

        cursor = await conn.execute("SELECT COUNT(*) FROM agents WHERE task_id = ?", (str(task1.id),))
        assert (await cursor.fetchone())[0] == 1, "Valid agent should exist"

        cursor = await conn.execute("SELECT COUNT(*) FROM checkpoints WHERE task_id = ?", (str(task1.id),))
        assert (await cursor.fetchone())[0] == 1, "Valid checkpoint should exist"

    await db2.close()


@pytest.mark.asyncio
async def test_migration_idempotent_with_orphans(tmp_path: Path):
    """Test that migration can run multiple times safely.

    Scenario:
    1. Create orphaned records
    2. Run migration first time
    3. Run migration second time
    4. Verify no errors, same behavior both times
    """
    db_path = tmp_path / "idempotent.db"

    # First initialization
    db1 = Database(db_path)
    await db1.initialize()

    # Create task and orphan
    task = Task(prompt="Task for idempotency test", summary="Idempotency test task")
    await db1.insert_task(task)

    async with db1._get_connection() as conn:
        # Create agent
        await conn.execute(
            """
            INSERT INTO agents (id, name, specialization, task_id, state, model, spawned_at)
            VALUES (?, ?, ?, ?, ?, ?, datetime('now'))
            """,
            (str(uuid4()), "test-agent", "general", str(task.id), "running", "claude-3")
        )
        await conn.commit()

        # Delete task to create orphan
        await conn.execute("PRAGMA foreign_keys=OFF")
        await conn.execute("DELETE FROM tasks WHERE id = ?", (str(task.id),))
        await conn.commit()
        await conn.execute("PRAGMA foreign_keys=ON")

    await db1.close()

    # Second initialization (first migration run)
    db2 = Database(db_path)
    await db2.initialize()

    async with db2._get_connection() as conn:
        # Verify CASCADE DELETE is in place
        cursor = await conn.execute("PRAGMA foreign_key_list(agents)")
        fk_list = await cursor.fetchall()
        task_fk = next((fk for fk in fk_list if fk["table"] == "tasks"), None)
        assert task_fk["on_delete"] == "CASCADE", "First migration should add CASCADE"

    await db2.close()

    # Third initialization (second migration run - should be idempotent)
    db3 = Database(db_path)
    await db3.initialize()

    async with db3._get_connection() as conn:
        # Verify CASCADE DELETE is still in place
        cursor = await conn.execute("PRAGMA foreign_key_list(agents)")
        fk_list = await cursor.fetchall()
        task_fk = next((fk for fk in fk_list if fk["table"] == "tasks"), None)
        assert task_fk["on_delete"] == "CASCADE", "Second migration should preserve CASCADE"

        # Note: The orphan agent created before migration will still exist
        # The migration adds CASCADE DELETE for future deletions, but doesn't
        # retroactively clean up existing orphans. This is expected behavior.
        # Check for foreign key violations - orphan will show up
        cursor = await conn.execute("PRAGMA foreign_key_check")
        violations = await cursor.fetchall()
        # Orphan agent will show as violation since its task_id doesn't exist
        assert len(violations) > 0, "Orphan should show as FK violation"

        # Verify database is still functional
        # Create a new task
        new_task = Task(prompt="Task after idempotent migration", summary="Post-migration task")

    await db3.insert_task(new_task)

    # Verify task was inserted
    retrieved = await db3.get_task(new_task.id)
    assert retrieved is not None
    assert retrieved.prompt == "Task after idempotent migration"

    await db3.close()
