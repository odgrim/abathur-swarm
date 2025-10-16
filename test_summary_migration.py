"""Test script to verify summary column migration is idempotent."""

import asyncio
import sys
from pathlib import Path

# Add src to path
sys.path.insert(0, str(Path(__file__).parent / "src"))

from abathur.infrastructure.database import Database


async def test_migration():
    """Test the summary column migration."""
    db_path = Path(":memory:")
    db = Database(db_path)

    print("=== Creating database with old schema (no summary column) ===")
    # Manually create tables without summary column to simulate old database
    async with db._get_connection() as conn:
        await conn.execute("PRAGMA journal_mode=WAL")
        await conn.execute("PRAGMA foreign_keys=ON")

        # Create tasks table WITHOUT summary column (old schema)
        await conn.execute(
            """
            CREATE TABLE tasks (
                id TEXT PRIMARY KEY,
                prompt TEXT NOT NULL,
                agent_type TEXT NOT NULL DEFAULT 'general',
                priority INTEGER NOT NULL DEFAULT 5,
                status TEXT NOT NULL,
                input_data TEXT NOT NULL,
                result_data TEXT,
                error_message TEXT,
                retry_count INTEGER DEFAULT 0,
                max_retries INTEGER DEFAULT 3,
                submitted_at TIMESTAMP NOT NULL,
                started_at TIMESTAMP,
                completed_at TIMESTAMP,
                last_updated_at TIMESTAMP NOT NULL,
                created_by TEXT,
                parent_task_id TEXT,
                dependencies TEXT,
                session_id TEXT
            )
            """
        )

        # Insert test tasks with NO summary column
        await conn.execute(
            """
            INSERT INTO tasks (
                id, prompt, agent_type, priority, status, input_data,
                submitted_at, last_updated_at, dependencies
            ) VALUES (?, ?, ?, ?, ?, ?, datetime('now'), datetime('now'), '[]')
            """,
            (
                "test-task-1",
                "This is a test prompt for verifying summary backfill logic",
                "general",
                5,
                "pending",
                "{}",
            ),
        )

        # Insert task with empty prompt
        await conn.execute(
            """
            INSERT INTO tasks (
                id, prompt, agent_type, priority, status, input_data,
                submitted_at, last_updated_at, dependencies
            ) VALUES (?, ?, ?, ?, ?, ?, datetime('now'), datetime('now'), '[]')
            """,
            ("test-task-2", "", "general", 5, "pending", "{}"),
        )

        await conn.commit()
        print("✓ Created old schema tasks table with test data")

        # Verify summary column doesn't exist yet
        cursor = await conn.execute("PRAGMA table_info(tasks)")
        columns = await cursor.fetchall()
        column_names = [col["name"] for col in columns]
        if "summary" in column_names:
            print("✗ ERROR: Summary column exists in old schema (shouldn't)")
            return False
        print("✓ Confirmed summary column doesn't exist in old schema")

    print("\n=== Running migration (first initialization) ===")
    # Now run migrations which should add summary column and backfill
    async with db._get_connection() as conn:
        await db._run_migrations(conn)
        await conn.commit()
    print("✓ Migration completed")

    # Verify summary was backfilled
    async with db._get_connection() as conn:
        cursor = await conn.execute("SELECT summary FROM tasks WHERE id = ?", ("test-task-1",))
        row = await cursor.fetchone()
        summary = row["summary"] if row else None
        print(f"✓ Test task 1 summary: '{summary}'")

        if not summary:
            print("✗ ERROR: Summary was not backfilled for task 1")
            return False

        expected = "This is a test prompt for verifying summary backfill logic"[:100]
        if summary != expected:
            print(f"✗ ERROR: Summary mismatch. Expected: '{expected}', Got: '{summary}'")
            return False

        # Verify empty prompt backfill
        cursor = await conn.execute("SELECT summary FROM tasks WHERE id = ?", ("test-task-2",))
        row = await cursor.fetchone()
        summary = row["summary"] if row else None
        print(f"✓ Test task 2 (empty prompt) summary: '{summary}'")

        if summary != "Task":
            print(f"✗ ERROR: Expected 'Task' for empty prompt, got: '{summary}'")
            return False

    print("\n=== Testing idempotency (running migration again) ===")
    # Re-run migration to test idempotency
    async with db._get_connection() as conn:
        await db._run_migrations(conn)
        await conn.commit()
    print("✓ Second migration run completed (no errors)")

    # Verify column still exists and data is intact
    async with db._get_connection() as conn:
        cursor = await conn.execute("PRAGMA table_info(tasks)")
        columns = await cursor.fetchall()
        column_names = [col["name"] for col in columns]

        if "summary" not in column_names:
            print("✗ ERROR: Summary column missing after second migration")
            return False

        cursor = await conn.execute("SELECT summary FROM tasks WHERE id = ?", ("test-task-1",))
        row = await cursor.fetchone()
        summary = row["summary"] if row else None

        expected = "This is a test prompt for verifying summary backfill logic"[:100]
        if summary != expected:
            print(f"✗ ERROR: Summary changed after re-running migration: '{summary}'")
            return False

        # Verify empty prompt still correct
        cursor = await conn.execute("SELECT summary FROM tasks WHERE id = ?", ("test-task-2",))
        row = await cursor.fetchone()
        summary = row["summary"] if row else None

        if summary != "Task":
            print(f"✗ ERROR: Empty prompt summary changed: '{summary}'")
            return False

    print("✓ Summary column exists and data intact after re-running migration")
    print("\n=== All tests passed ===")
    await db.close()
    return True


if __name__ == "__main__":
    result = asyncio.run(test_migration())
    sys.exit(0 if result else 1)
