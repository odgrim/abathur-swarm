"""Database infrastructure using SQLite with WAL mode."""

import json
from collections.abc import AsyncIterator
from contextlib import asynccontextmanager
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, cast
from uuid import UUID

import aiosqlite
from aiosqlite import Connection

from abathur.domain.models import Agent, AgentState, Task, TaskStatus


class Database:
    """SQLite database with WAL mode for concurrent access."""

    def __init__(self, db_path: Path) -> None:
        """Initialize database.

        Args:
            db_path: Path to SQLite database file
        """
        self.db_path = db_path
        self._initialized = False

    async def initialize(self) -> None:
        """Initialize database schema and settings."""
        if self._initialized:
            return

        # Create parent directory if it doesn't exist
        self.db_path.parent.mkdir(parents=True, exist_ok=True)

        async with self._get_connection() as conn:
            # Enable WAL mode for concurrent reads
            await conn.execute("PRAGMA journal_mode=WAL")
            await conn.execute("PRAGMA synchronous=NORMAL")
            await conn.execute("PRAGMA foreign_keys=ON")
            await conn.execute("PRAGMA busy_timeout=5000")

            # Run migrations before creating tables
            await self._run_migrations(conn)

            # Create tables
            await self._create_tables(conn)
            await conn.commit()

        self._initialized = True

    @asynccontextmanager
    async def _get_connection(self) -> AsyncIterator[Connection]:
        """Get database connection with proper settings."""
        async with aiosqlite.connect(str(self.db_path)) as conn:
            conn.row_factory = aiosqlite.Row
            yield conn

    async def _run_migrations(self, conn: Connection) -> None:
        """Run database migrations."""
        # Check if tasks table exists and has old schema
        cursor = await conn.execute(
            "SELECT name FROM sqlite_master WHERE type='table' AND name='tasks'"
        )
        table_exists = await cursor.fetchone()

        if table_exists:
            # Check if old schema (has template_name column)
            cursor = await conn.execute("PRAGMA table_info(tasks)")
            columns = await cursor.fetchall()
            column_names = [col["name"] for col in columns]

            if "template_name" in column_names and "prompt" not in column_names:
                # Migrate from old schema to new schema
                print("Migrating database schema: template_name → prompt + agent_type")

                # Temporarily disable foreign keys for migration
                await conn.execute("PRAGMA foreign_keys=OFF")

                # Create new table with updated schema
                await conn.execute(
                    """
                    CREATE TABLE tasks_new (
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
                        created_by TEXT,
                        parent_task_id TEXT,
                        dependencies TEXT,
                        FOREIGN KEY (parent_task_id) REFERENCES tasks(id)
                    )
                    """
                )

                # Copy data from old table to new table
                # template_name becomes prompt, agent_type defaults to 'general'
                await conn.execute(
                    """
                    INSERT INTO tasks_new (
                        id, prompt, agent_type, priority, status, input_data,
                        result_data, error_message, retry_count, max_retries,
                        submitted_at, started_at, completed_at, created_by,
                        parent_task_id, dependencies
                    )
                    SELECT
                        id, template_name, 'general', priority, status, input_data,
                        result_data, error_message, retry_count, max_retries,
                        submitted_at, started_at, completed_at, created_by,
                        parent_task_id, dependencies
                    FROM tasks
                    """
                )

                # Drop old table
                await conn.execute("DROP TABLE tasks")

                # Rename new table to tasks
                await conn.execute("ALTER TABLE tasks_new RENAME TO tasks")

                # Recreate indexes
                await conn.execute(
                    """
                    CREATE INDEX IF NOT EXISTS idx_tasks_status_priority
                    ON tasks(status, priority DESC, submitted_at ASC)
                    """
                )

                await conn.execute(
                    """
                    CREATE INDEX IF NOT EXISTS idx_tasks_submitted_at
                    ON tasks(submitted_at)
                    """
                )

                await conn.execute(
                    """
                    CREATE INDEX IF NOT EXISTS idx_tasks_parent
                    ON tasks(parent_task_id)
                    """
                )

                # Re-enable foreign keys
                await conn.execute("PRAGMA foreign_keys=ON")

                await conn.commit()
                print("Database migration completed successfully")

            # Migration: Add last_updated_at and max_execution_timeout_seconds columns
            if "last_updated_at" not in column_names:
                print(
                    "Migrating database schema: adding last_updated_at and max_execution_timeout_seconds columns"
                )

                # Add last_updated_at column
                await conn.execute(
                    """
                    ALTER TABLE tasks
                    ADD COLUMN last_updated_at TIMESTAMP
                    """
                )

                # Set last_updated_at to submitted_at for existing tasks
                await conn.execute(
                    """
                    UPDATE tasks
                    SET last_updated_at = COALESCE(completed_at, started_at, submitted_at)
                    WHERE last_updated_at IS NULL
                    """
                )

                # Add max_execution_timeout_seconds column with default of 1 hour (3600 seconds)
                await conn.execute(
                    """
                    ALTER TABLE tasks
                    ADD COLUMN max_execution_timeout_seconds INTEGER DEFAULT 3600
                    """
                )

                # Create index for efficient timeout detection queries
                await conn.execute(
                    """
                    CREATE INDEX IF NOT EXISTS idx_tasks_running_timeout
                    ON tasks(status, last_updated_at)
                    WHERE status = 'running'
                    """
                )

                await conn.commit()
                print(
                    "Added last_updated_at and max_execution_timeout_seconds columns successfully"
                )

    async def _create_tables(self, conn: Connection) -> None:
        """Create database tables."""
        # Tasks table
        await conn.execute(
            """
            CREATE TABLE IF NOT EXISTS tasks (
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
                max_execution_timeout_seconds INTEGER DEFAULT 3600,
                submitted_at TIMESTAMP NOT NULL,
                started_at TIMESTAMP,
                completed_at TIMESTAMP,
                last_updated_at TIMESTAMP NOT NULL,
                created_by TEXT,
                parent_task_id TEXT,
                dependencies TEXT,
                FOREIGN KEY (parent_task_id) REFERENCES tasks(id)
            )
        """
        )

        await conn.execute(
            """
            CREATE INDEX IF NOT EXISTS idx_tasks_status_priority
            ON tasks(status, priority DESC, submitted_at ASC)
        """
        )

        await conn.execute(
            """
            CREATE INDEX IF NOT EXISTS idx_tasks_submitted_at
            ON tasks(submitted_at)
        """
        )

        await conn.execute(
            """
            CREATE INDEX IF NOT EXISTS idx_tasks_parent
            ON tasks(parent_task_id)
        """
        )

        await conn.execute(
            """
            CREATE INDEX IF NOT EXISTS idx_tasks_running_timeout
            ON tasks(status, last_updated_at)
            WHERE status = 'running'
        """
        )

        # Agents table
        await conn.execute(
            """
            CREATE TABLE IF NOT EXISTS agents (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                specialization TEXT NOT NULL,
                task_id TEXT NOT NULL,
                state TEXT NOT NULL,
                model TEXT NOT NULL,
                spawned_at TIMESTAMP NOT NULL,
                terminated_at TIMESTAMP,
                resource_usage TEXT,
                FOREIGN KEY (task_id) REFERENCES tasks(id)
            )
        """
        )

        await conn.execute(
            """
            CREATE INDEX IF NOT EXISTS idx_agents_task
            ON agents(task_id)
        """
        )

        await conn.execute(
            """
            CREATE INDEX IF NOT EXISTS idx_agents_state
            ON agents(state)
        """
        )

        # State table
        await conn.execute(
            """
            CREATE TABLE IF NOT EXISTS state (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                task_id TEXT NOT NULL,
                key TEXT NOT NULL,
                value TEXT NOT NULL,
                created_at TIMESTAMP NOT NULL,
                updated_at TIMESTAMP NOT NULL,
                UNIQUE(task_id, key),
                FOREIGN KEY (task_id) REFERENCES tasks(id)
            )
        """
        )

        await conn.execute(
            """
            CREATE INDEX IF NOT EXISTS idx_state_task_key
            ON state(task_id, key)
        """
        )

        # Audit table
        await conn.execute(
            """
            CREATE TABLE IF NOT EXISTS audit (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                timestamp TIMESTAMP NOT NULL,
                agent_id TEXT,
                task_id TEXT NOT NULL,
                action_type TEXT NOT NULL,
                action_data TEXT,
                result TEXT,
                FOREIGN KEY (agent_id) REFERENCES agents(id),
                FOREIGN KEY (task_id) REFERENCES tasks(id)
            )
        """
        )

        await conn.execute(
            """
            CREATE INDEX IF NOT EXISTS idx_audit_task
            ON audit(task_id, timestamp DESC)
        """
        )

        await conn.execute(
            """
            CREATE INDEX IF NOT EXISTS idx_audit_agent
            ON audit(agent_id, timestamp DESC)
        """
        )

        await conn.execute(
            """
            CREATE INDEX IF NOT EXISTS idx_audit_timestamp
            ON audit(timestamp DESC)
        """
        )

        # Metrics table
        await conn.execute(
            """
            CREATE TABLE IF NOT EXISTS metrics (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                timestamp TIMESTAMP NOT NULL,
                metric_name TEXT NOT NULL,
                metric_value REAL NOT NULL,
                labels TEXT,
                CHECK(metric_value >= 0)
            )
        """
        )

        await conn.execute(
            """
            CREATE INDEX IF NOT EXISTS idx_metrics_name_timestamp
            ON metrics(metric_name, timestamp DESC)
        """
        )

        # Checkpoints table for loop execution
        await conn.execute(
            """
            CREATE TABLE IF NOT EXISTS checkpoints (
                task_id TEXT NOT NULL,
                iteration INTEGER NOT NULL,
                state TEXT NOT NULL,
                created_at TIMESTAMP NOT NULL,
                PRIMARY KEY (task_id, iteration),
                FOREIGN KEY (task_id) REFERENCES tasks(id)
            )
        """
        )

        await conn.execute(
            """
            CREATE INDEX IF NOT EXISTS idx_checkpoints_task
            ON checkpoints(task_id, iteration DESC)
        """
        )

    # Task operations
    async def insert_task(self, task: Task) -> None:
        """Insert a new task into the database."""
        async with self._get_connection() as conn:
            await conn.execute(
                """
                INSERT INTO tasks (
                    id, prompt, agent_type, priority, status, input_data,
                    result_data, error_message, retry_count, max_retries,
                    max_execution_timeout_seconds,
                    submitted_at, started_at, completed_at, last_updated_at,
                    created_by, parent_task_id, dependencies
                ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                """,
                (
                    str(task.id),
                    task.prompt,
                    task.agent_type,
                    task.priority,
                    task.status.value,
                    json.dumps(task.input_data),
                    json.dumps(task.result_data) if task.result_data else None,
                    task.error_message,
                    task.retry_count,
                    task.max_retries,
                    task.max_execution_timeout_seconds,
                    task.submitted_at.isoformat(),
                    task.started_at.isoformat() if task.started_at else None,
                    task.completed_at.isoformat() if task.completed_at else None,
                    task.last_updated_at.isoformat(),
                    task.created_by,
                    str(task.parent_task_id) if task.parent_task_id else None,
                    json.dumps([str(dep) for dep in task.dependencies]),
                ),
            )
            await conn.commit()

    async def update_task_status(
        self, task_id: UUID, status: TaskStatus, error_message: str | None = None
    ) -> None:
        """Update task status and last_updated_at timestamp."""
        async with self._get_connection() as conn:
            now = datetime.now(timezone.utc).isoformat()
            if status == TaskStatus.RUNNING:
                await conn.execute(
                    "UPDATE tasks SET status = ?, started_at = ?, last_updated_at = ? WHERE id = ?",
                    (status.value, now, now, str(task_id)),
                )
            elif status in (TaskStatus.COMPLETED, TaskStatus.FAILED, TaskStatus.CANCELLED):
                await conn.execute(
                    "UPDATE tasks SET status = ?, completed_at = ?, error_message = ?, last_updated_at = ? WHERE id = ?",
                    (status.value, now, error_message, now, str(task_id)),
                )
            else:
                await conn.execute(
                    "UPDATE tasks SET status = ?, last_updated_at = ? WHERE id = ?",
                    (status.value, now, str(task_id)),
                )
            await conn.commit()

    async def increment_task_retry_count(self, task_id: UUID) -> None:
        """Increment the retry count for a task.

        Args:
            task_id: Task ID
        """
        async with self._get_connection() as conn:
            await conn.execute(
                "UPDATE tasks SET retry_count = retry_count + 1 WHERE id = ?",
                (str(task_id),),
            )
            await conn.commit()

    async def get_task(self, task_id: UUID) -> Task | None:
        """Get task by ID."""
        async with self._get_connection() as conn:
            cursor = await conn.execute(
                "SELECT * FROM tasks WHERE id = ?",
                (str(task_id),),
            )
            row = await cursor.fetchone()
            if row:
                return self._row_to_task(row)
            return None

    async def list_tasks(self, status: TaskStatus | None = None, limit: int = 100) -> list[Task]:
        """List tasks with optional status filter."""
        async with self._get_connection() as conn:
            if status:
                cursor = await conn.execute(
                    """
                    SELECT * FROM tasks
                    WHERE status = ?
                    ORDER BY priority DESC, submitted_at ASC
                    LIMIT ?
                    """,
                    (status.value, limit),
                )
            else:
                cursor = await conn.execute(
                    """
                    SELECT * FROM tasks
                    ORDER BY priority DESC, submitted_at ASC
                    LIMIT ?
                    """,
                    (limit,),
                )
            rows = await cursor.fetchall()
            return [self._row_to_task(row) for row in rows]

    async def dequeue_next_task(self) -> Task | None:
        """Get next pending task with highest priority."""
        async with self._get_connection() as conn:
            cursor = await conn.execute(
                """
                SELECT * FROM tasks
                WHERE status = ?
                ORDER BY priority DESC, submitted_at ASC
                LIMIT 1
                """,
                (TaskStatus.PENDING.value,),
            )
            row = await cursor.fetchone()
            if row:
                task = self._row_to_task(row)
                # Update status to running
                await self.update_task_status(task.id, TaskStatus.RUNNING)
                # Update the task object to reflect the new status
                task.status = TaskStatus.RUNNING
                task.started_at = datetime.now(timezone.utc)
                return task
            return None

    async def get_stale_running_tasks(self) -> list[Task]:
        """Get running tasks that have exceeded their execution timeout.

        Returns:
            List of stale running tasks that need to be handled
        """
        async with self._get_connection() as conn:
            now = datetime.now(timezone.utc)
            cursor = await conn.execute(
                """
                SELECT * FROM tasks
                WHERE status = ?
                AND (julianday(?) - julianday(last_updated_at)) * 86400 > max_execution_timeout_seconds
                ORDER BY last_updated_at ASC
                """,
                (TaskStatus.RUNNING.value, now.isoformat()),
            )
            rows = await cursor.fetchall()
            return [self._row_to_task(row) for row in rows]

    def _row_to_task(self, row: aiosqlite.Row) -> Task:
        """Convert database row to Task model."""
        # Convert row to dict for easier access with fallbacks
        row_dict = dict(row)

        return Task(
            id=UUID(row_dict["id"]),
            prompt=row_dict["prompt"],
            agent_type=row_dict["agent_type"],
            priority=row_dict["priority"],
            status=TaskStatus(row_dict["status"]),
            input_data=json.loads(row_dict["input_data"]),
            result_data=json.loads(row_dict["result_data"]) if row_dict["result_data"] else None,
            error_message=row_dict["error_message"],
            retry_count=row_dict["retry_count"],
            max_retries=row_dict["max_retries"],
            max_execution_timeout_seconds=row_dict.get("max_execution_timeout_seconds", 3600),
            submitted_at=datetime.fromisoformat(row_dict["submitted_at"]),
            started_at=datetime.fromisoformat(row_dict["started_at"])
            if row_dict["started_at"]
            else None,
            completed_at=(
                datetime.fromisoformat(row_dict["completed_at"])
                if row_dict["completed_at"]
                else None
            ),
            last_updated_at=datetime.fromisoformat(row_dict["last_updated_at"])
            if row_dict.get("last_updated_at")
            else datetime.now(timezone.utc),
            created_by=row_dict["created_by"],
            parent_task_id=UUID(row_dict["parent_task_id"]) if row_dict["parent_task_id"] else None,
            dependencies=[UUID(dep) for dep in json.loads(row_dict["dependencies"])],
        )

    # Agent operations
    async def insert_agent(self, agent: Agent) -> None:
        """Insert a new agent into the database."""
        async with self._get_connection() as conn:
            await conn.execute(
                """
                INSERT INTO agents (
                    id, name, specialization, task_id, state, model,
                    spawned_at, terminated_at, resource_usage
                ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
                """,
                (
                    str(agent.id),
                    agent.name,
                    agent.specialization,
                    str(agent.task_id),
                    agent.state.value,
                    agent.model,
                    agent.spawned_at.isoformat(),
                    agent.terminated_at.isoformat() if agent.terminated_at else None,
                    json.dumps(agent.resource_usage),
                ),
            )
            await conn.commit()

    async def update_agent_state(self, agent_id: UUID, state: AgentState) -> None:
        """Update agent state."""
        async with self._get_connection() as conn:
            if state == AgentState.TERMINATED:
                await conn.execute(
                    "UPDATE agents SET state = ?, terminated_at = ? WHERE id = ?",
                    (state.value, datetime.now(timezone.utc).isoformat(), str(agent_id)),
                )
            else:
                await conn.execute(
                    "UPDATE agents SET state = ? WHERE id = ?",
                    (state.value, str(agent_id)),
                )
            await conn.commit()

    # State operations
    async def set_state(self, task_id: UUID, key: str, value: dict[str, Any]) -> None:
        """Set shared state for a task."""
        async with self._get_connection() as conn:
            now = datetime.now(timezone.utc).isoformat()
            await conn.execute(
                """
                INSERT INTO state (task_id, key, value, created_at, updated_at)
                VALUES (?, ?, ?, ?, ?)
                ON CONFLICT(task_id, key) DO UPDATE SET
                    value = excluded.value,
                    updated_at = excluded.updated_at
                """,
                (str(task_id), key, json.dumps(value), now, now),
            )
            await conn.commit()

    async def get_state(self, task_id: UUID, key: str) -> dict[str, Any] | None:
        """Get shared state for a task."""
        async with self._get_connection() as conn:
            cursor = await conn.execute(
                "SELECT value FROM state WHERE task_id = ? AND key = ?",
                (str(task_id), key),
            )
            row = await cursor.fetchone()
            if row:
                return cast(dict[str, Any], json.loads(row["value"]))
            return None

    # Audit operations
    async def log_audit(
        self,
        task_id: UUID,
        action_type: str,
        agent_id: UUID | None = None,
        action_data: dict[str, Any] | None = None,
        result: str | None = None,
    ) -> None:
        """Log an audit entry."""
        async with self._get_connection() as conn:
            await conn.execute(
                """
                INSERT INTO audit (timestamp, agent_id, task_id, action_type, action_data, result)
                VALUES (?, ?, ?, ?, ?, ?)
                """,
                (
                    datetime.now(timezone.utc).isoformat(),
                    str(agent_id) if agent_id else None,
                    str(task_id),
                    action_type,
                    json.dumps(action_data) if action_data else None,
                    result,
                ),
            )
            await conn.commit()
