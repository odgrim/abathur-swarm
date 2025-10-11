"""Database infrastructure using SQLite with WAL mode."""

import json
from collections.abc import AsyncIterator
from contextlib import asynccontextmanager
from datetime import datetime, timezone
from pathlib import Path
from typing import TYPE_CHECKING, Any, cast
from uuid import UUID

import aiosqlite
from aiosqlite import Connection

from abathur.domain.models import Agent, AgentState, Task, TaskStatus

if TYPE_CHECKING:
    from abathur.services.document_index_service import DocumentIndexService
    from abathur.services.memory_service import MemoryService
    from abathur.services.session_service import SessionService


class Database:
    """SQLite database with WAL mode for concurrent access."""

    def __init__(self, db_path: Path) -> None:
        """Initialize database.

        Args:
            db_path: Path to SQLite database file
        """
        self.db_path = db_path
        self._initialized = False
        self._memory_service: MemoryService | None = None
        self._session_service: SessionService | None = None
        self._document_service: DocumentIndexService | None = None
        self._shared_conn: Connection | None = None  # For :memory: databases

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
            await conn.execute("PRAGMA wal_autocheckpoint=1000")

            # Run migrations before creating tables
            await self._run_migrations(conn)

            # Create tables
            await self._create_tables(conn)
            await conn.commit()

        self._initialized = True

    async def close(self) -> None:
        """Close the database connection.

        Only needed for :memory: databases to clean up the shared connection.
        File-based databases close connections automatically.
        """
        if self._shared_conn is not None:
            await self._shared_conn.close()
            self._shared_conn = None
            self._initialized = False

    @asynccontextmanager
    async def _get_connection(self) -> AsyncIterator[Connection]:
        """Get database connection with proper settings.

        For :memory: databases, maintains a shared connection to preserve data
        across multiple operations. For file databases, creates a new connection
        each time.
        """
        if str(self.db_path) == ":memory:":
            # Reuse same connection for memory databases to maintain data
            if self._shared_conn is None:
                self._shared_conn = await aiosqlite.connect(":memory:")
                self._shared_conn.row_factory = aiosqlite.Row
            yield self._shared_conn
        else:
            # File databases get new connections each time
            async with aiosqlite.connect(str(self.db_path)) as conn:
                conn.row_factory = aiosqlite.Row
                yield conn

    async def validate_foreign_keys(self) -> list[tuple[str, ...]]:
        """Run PRAGMA foreign_key_check and return violations.

        Returns:
            List of foreign key violations (empty if valid)
        """
        async with self._get_connection() as conn:
            cursor = await conn.execute("PRAGMA foreign_key_check")
            violations = await cursor.fetchall()
            return [tuple(row) for row in violations]

    async def explain_query_plan(self, query: str, params: tuple[Any, ...] = ()) -> list[str]:
        """Return EXPLAIN QUERY PLAN output for optimization.

        Args:
            query: SQL query to analyze
            params: Query parameters

        Returns:
            List of query plan lines
        """
        async with self._get_connection() as conn:
            cursor = await conn.execute(f"EXPLAIN QUERY PLAN {query}", params)
            rows = await cursor.fetchall()
            return [" ".join(str(col) for col in row) for row in rows]

    async def get_index_usage(self) -> dict[str, Any]:
        """Report which indexes exist and basic statistics.

        Returns:
            Dictionary with index information
        """
        async with self._get_connection() as conn:
            cursor = await conn.execute(
                "SELECT name, tbl_name FROM sqlite_master WHERE type='index' ORDER BY tbl_name, name"
            )
            indexes = list(await cursor.fetchall())
            return {
                "index_count": len(indexes),
                "indexes": [{"name": row[0], "table": row[1]} for row in indexes],
            }

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

            # Migration: Add session_id column to tasks
            if "session_id" not in column_names:
                print("Migrating database schema: adding session_id to tasks")
                await conn.execute(
                    """
                    ALTER TABLE tasks
                    ADD COLUMN session_id TEXT
                    """
                )
                await conn.commit()
                print("Added session_id column to tasks")

        # Check if agents table exists and needs session_id column
        cursor = await conn.execute(
            "SELECT name FROM sqlite_master WHERE type='table' AND name='agents'"
        )
        agents_exists = await cursor.fetchone()

        if agents_exists:
            cursor = await conn.execute("PRAGMA table_info(agents)")
            agent_columns = await cursor.fetchall()
            agent_column_names = [col["name"] for col in agent_columns]

            if "session_id" not in agent_column_names:
                print("Migrating database schema: adding session_id to agents")
                await conn.execute(
                    """
                    ALTER TABLE agents
                    ADD COLUMN session_id TEXT
                    """
                )
                await conn.commit()
                print("Added session_id column to agents")

        # Check if audit table exists and needs memory columns
        cursor = await conn.execute(
            "SELECT name FROM sqlite_master WHERE type='table' AND name='audit'"
        )
        audit_exists = await cursor.fetchone()

        if audit_exists:
            cursor = await conn.execute("PRAGMA table_info(audit)")
            audit_columns = await cursor.fetchall()
            audit_column_names = [col["name"] for col in audit_columns]

            if "memory_operation_type" not in audit_column_names:
                print("Migrating database schema: adding memory columns to audit")
                await conn.execute(
                    """
                    ALTER TABLE audit
                    ADD COLUMN memory_operation_type TEXT
                    """
                )
                await conn.execute(
                    """
                    ALTER TABLE audit
                    ADD COLUMN memory_namespace TEXT
                    """
                )
                await conn.execute(
                    """
                    ALTER TABLE audit
                    ADD COLUMN memory_entry_id INTEGER
                    """
                )
                await conn.commit()
                print("Added memory columns to audit")

        # Check if checkpoints table exists and needs session_id column
        cursor = await conn.execute(
            "SELECT name FROM sqlite_master WHERE type='table' AND name='checkpoints'"
        )
        checkpoints_exists = await cursor.fetchone()

        if checkpoints_exists:
            cursor = await conn.execute("PRAGMA table_info(checkpoints)")
            checkpoint_columns = await cursor.fetchall()
            checkpoint_column_names = [col["name"] for col in checkpoint_columns]

            if "session_id" not in checkpoint_column_names:
                print("Migrating database schema: adding session_id to checkpoints")
                await conn.execute(
                    """
                    ALTER TABLE checkpoints
                    ADD COLUMN session_id TEXT
                    """
                )
                await conn.commit()
                print("Added session_id column to checkpoints")

        # Check if audit table needs task_id to be nullable
        cursor = await conn.execute(
            "SELECT name FROM sqlite_master WHERE type='table' AND name='audit'"
        )
        audit_table_exists = await cursor.fetchone()

        if audit_table_exists:
            cursor = await conn.execute("PRAGMA table_info(audit)")
            audit_cols = await cursor.fetchall()
            task_id_col = next((col for col in audit_cols if col["name"] == "task_id"), None)

            # If task_id is NOT NULL (notnull=1), we need to recreate the table
            if task_id_col and task_id_col["notnull"] == 1:
                print("Migrating database schema: making audit.task_id nullable")

                # Temporarily disable FK
                await conn.execute("PRAGMA foreign_keys=OFF")

                # Recreate audit table with nullable task_id (no FK constraint)
                await conn.execute("ALTER TABLE audit RENAME TO audit_old")
                await conn.execute(
                    """
                    CREATE TABLE audit (
                        id INTEGER PRIMARY KEY AUTOINCREMENT,
                        timestamp TIMESTAMP NOT NULL,
                        agent_id TEXT,
                        task_id TEXT,
                        action_type TEXT NOT NULL,
                        action_data TEXT,
                        result TEXT,
                        memory_operation_type TEXT,
                        memory_namespace TEXT,
                        memory_entry_id INTEGER,
                        FOREIGN KEY (agent_id) REFERENCES agents(id),
                        FOREIGN KEY (memory_entry_id) REFERENCES memory_entries(id) ON DELETE SET NULL
                    )
                    """
                )
                await conn.execute(
                    """
                    INSERT INTO audit SELECT * FROM audit_old
                    """
                )
                await conn.execute("DROP TABLE audit_old")

                # Re-enable FK
                await conn.execute("PRAGMA foreign_keys=ON")
                await conn.commit()
                print("Made audit.task_id nullable")

    async def _create_tables(self, conn: Connection) -> None:
        """Create database tables."""
        # Create memory management tables first (foreign key targets)
        await self._create_memory_tables(conn)

        # Create/enhance core tables
        await self._create_core_tables(conn)

        # Create indexes last
        await self._create_indexes(conn)

    async def _create_memory_tables(self, conn: Connection) -> None:
        """Create new memory management tables."""
        # Sessions table
        await conn.execute(
            """
            CREATE TABLE IF NOT EXISTS sessions (
                id TEXT PRIMARY KEY,
                app_name TEXT NOT NULL,
                user_id TEXT NOT NULL,
                project_id TEXT,
                status TEXT NOT NULL DEFAULT 'created',
                events TEXT NOT NULL DEFAULT '[]',
                state TEXT NOT NULL DEFAULT '{}',
                metadata TEXT DEFAULT '{}',
                created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
                last_update_time TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
                terminated_at TIMESTAMP,
                archived_at TIMESTAMP,
                CHECK(status IN ('created', 'active', 'paused', 'terminated', 'archived')),
                CHECK(json_valid(events)),
                CHECK(json_valid(state)),
                CHECK(json_valid(metadata))
            )
        """
        )

        # Memory entries table
        await conn.execute(
            """
            CREATE TABLE IF NOT EXISTS memory_entries (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                namespace TEXT NOT NULL,
                key TEXT NOT NULL,
                value TEXT NOT NULL,
                memory_type TEXT NOT NULL,
                version INTEGER NOT NULL DEFAULT 1,
                is_deleted BOOLEAN NOT NULL DEFAULT 0,
                metadata TEXT DEFAULT '{}',
                created_by TEXT,
                updated_by TEXT,
                created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
                updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
                CHECK(memory_type IN ('semantic', 'episodic', 'procedural')),
                CHECK(json_valid(value)),
                CHECK(json_valid(metadata)),
                CHECK(version > 0),
                UNIQUE(namespace, key, version)
            )
        """
        )

        # Document index table
        await conn.execute(
            """
            CREATE TABLE IF NOT EXISTS document_index (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                file_path TEXT NOT NULL UNIQUE,
                title TEXT NOT NULL,
                document_type TEXT,
                content_hash TEXT NOT NULL,
                chunk_count INTEGER DEFAULT 1,
                embedding_model TEXT,
                embedding_blob BLOB,
                metadata TEXT DEFAULT '{}',
                last_synced_at TIMESTAMP,
                sync_status TEXT DEFAULT 'pending',
                created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
                updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
                CHECK(sync_status IN ('pending', 'synced', 'failed', 'stale')),
                CHECK(json_valid(metadata))
            )
        """
        )

        # Load sqlite-vss extensions for vector search
        await self._load_vss_extensions(conn)

        # Document embeddings virtual table (using sqlite-vss)
        await conn.execute(
            """
            CREATE VIRTUAL TABLE IF NOT EXISTS document_embeddings USING vss0(
                embedding(768)
            )
            """
        )

        # Document embedding metadata table
        await conn.execute(
            """
            CREATE TABLE IF NOT EXISTS document_embedding_metadata (
                rowid INTEGER PRIMARY KEY,
                document_id INTEGER NOT NULL,
                namespace TEXT NOT NULL,
                file_path TEXT NOT NULL,
                embedding_model TEXT NOT NULL DEFAULT 'nomic-embed-text-v1.5',
                created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
                FOREIGN KEY (document_id) REFERENCES document_index(id) ON DELETE CASCADE
            )
            """
        )

    async def _load_vss_extensions(self, conn: Connection) -> None:
        """Load sqlite-vss extensions for vector search.

        Raises:
            RuntimeError: If extensions cannot be loaded
        """
        try:
            await conn.enable_load_extension(True)

            # Load vector0 extension first (dependency for vss0)
            import os

            home = os.path.expanduser("~")
            vector_ext = f"{home}/.sqlite-extensions/vector0"
            vss_ext = f"{home}/.sqlite-extensions/vss0"

            await conn.load_extension(vector_ext)
            await conn.load_extension(vss_ext)

            await conn.enable_load_extension(False)
        except Exception as e:
            raise RuntimeError(
                f"Failed to load sqlite-vss extensions. "
                f"Ensure extensions are installed at ~/.sqlite-extensions/. "
                f"Error: {e}"
            ) from e

    async def _create_core_tables(self, conn: Connection) -> None:
        """Create core tables with session linkage."""
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
                session_id TEXT,
                FOREIGN KEY (parent_task_id) REFERENCES tasks(id),
                FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE SET NULL
            )
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
                session_id TEXT,
                FOREIGN KEY (task_id) REFERENCES tasks(id),
                FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE SET NULL
            )
        """
        )

        # State table (deprecated, maintained for backward compatibility)
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

        # Audit table with memory operation tracking
        # Note: task_id has no FK constraint for flexibility in audit logging
        await conn.execute(
            """
            CREATE TABLE IF NOT EXISTS audit (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                timestamp TIMESTAMP NOT NULL,
                agent_id TEXT,
                task_id TEXT,
                action_type TEXT NOT NULL,
                action_data TEXT,
                result TEXT,
                memory_operation_type TEXT,
                memory_namespace TEXT,
                memory_entry_id INTEGER,
                FOREIGN KEY (agent_id) REFERENCES agents(id),
                FOREIGN KEY (memory_entry_id) REFERENCES memory_entries(id) ON DELETE SET NULL
            )
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

        # Checkpoints table with session linkage
        await conn.execute(
            """
            CREATE TABLE IF NOT EXISTS checkpoints (
                task_id TEXT NOT NULL,
                iteration INTEGER NOT NULL,
                state TEXT NOT NULL,
                created_at TIMESTAMP NOT NULL,
                session_id TEXT,
                PRIMARY KEY (task_id, iteration),
                FOREIGN KEY (task_id) REFERENCES tasks(id),
                FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE SET NULL
            )
        """
        )

    async def _create_indexes(self, conn: Connection) -> None:
        """Create all performance indexes."""
        # Sessions indexes (4 indexes)
        await conn.execute("CREATE UNIQUE INDEX IF NOT EXISTS idx_sessions_pk ON sessions(id)")
        await conn.execute(
            """CREATE INDEX IF NOT EXISTS idx_sessions_status_updated
               ON sessions(status, last_update_time DESC)
               WHERE status IN ('active', 'paused')"""
        )
        await conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_sessions_user_created ON sessions(user_id, created_at DESC)"
        )
        await conn.execute(
            """CREATE INDEX IF NOT EXISTS idx_sessions_project
               ON sessions(project_id, created_at DESC)
               WHERE project_id IS NOT NULL"""
        )

        # Memory entries indexes (7 indexes)
        await conn.execute(
            "CREATE UNIQUE INDEX IF NOT EXISTS idx_memory_entries_pk ON memory_entries(id)"
        )
        await conn.execute(
            """CREATE INDEX IF NOT EXISTS idx_memory_namespace_key_version
               ON memory_entries(namespace, key, is_deleted, version DESC)"""
        )
        await conn.execute(
            """CREATE INDEX IF NOT EXISTS idx_memory_type_updated
               ON memory_entries(memory_type, updated_at DESC)
               WHERE is_deleted = 0"""
        )
        await conn.execute(
            """CREATE INDEX IF NOT EXISTS idx_memory_namespace_prefix
               ON memory_entries(namespace, updated_at DESC)
               WHERE is_deleted = 0"""
        )
        await conn.execute(
            """CREATE INDEX IF NOT EXISTS idx_memory_episodic_ttl
               ON memory_entries(memory_type, updated_at)
               WHERE memory_type = 'episodic' AND is_deleted = 0"""
        )
        await conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_memory_created_by ON memory_entries(created_by, created_at DESC)"
        )

        # Document index indexes (5 indexes)
        await conn.execute(
            "CREATE UNIQUE INDEX IF NOT EXISTS idx_document_index_pk ON document_index(id)"
        )
        await conn.execute(
            "CREATE UNIQUE INDEX IF NOT EXISTS idx_document_file_path ON document_index(file_path)"
        )
        await conn.execute(
            """CREATE INDEX IF NOT EXISTS idx_document_type_created
               ON document_index(document_type, created_at DESC)
               WHERE document_type IS NOT NULL"""
        )
        await conn.execute(
            """CREATE INDEX IF NOT EXISTS idx_document_sync_status
               ON document_index(sync_status, last_synced_at)
               WHERE sync_status IN ('pending', 'stale')"""
        )
        await conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_document_content_hash ON document_index(content_hash)"
        )

        # Tasks indexes (6 indexes)
        await conn.execute(
            """CREATE INDEX IF NOT EXISTS idx_tasks_status_priority
               ON tasks(status, priority DESC, submitted_at ASC)"""
        )
        await conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_tasks_submitted_at ON tasks(submitted_at)"
        )
        await conn.execute("CREATE INDEX IF NOT EXISTS idx_tasks_parent ON tasks(parent_task_id)")
        await conn.execute(
            """CREATE INDEX IF NOT EXISTS idx_tasks_running_timeout
               ON tasks(status, last_updated_at)
               WHERE status = 'running'"""
        )
        await conn.execute(
            """CREATE INDEX IF NOT EXISTS idx_tasks_session
               ON tasks(session_id, submitted_at DESC)
               WHERE session_id IS NOT NULL"""
        )

        # Agents indexes (3 indexes)
        await conn.execute("CREATE INDEX IF NOT EXISTS idx_agents_task ON agents(task_id)")
        await conn.execute("CREATE INDEX IF NOT EXISTS idx_agents_state ON agents(state)")
        await conn.execute(
            """CREATE INDEX IF NOT EXISTS idx_agents_session
               ON agents(session_id, spawned_at DESC)
               WHERE session_id IS NOT NULL"""
        )

        # Audit indexes (6 indexes)
        await conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_audit_task ON audit(task_id, timestamp DESC)"
        )
        await conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_audit_agent ON audit(agent_id, timestamp DESC)"
        )
        await conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_audit_timestamp ON audit(timestamp DESC)"
        )
        await conn.execute(
            """CREATE INDEX IF NOT EXISTS idx_audit_memory_operations
               ON audit(memory_operation_type, timestamp DESC)
               WHERE memory_operation_type IS NOT NULL"""
        )
        await conn.execute(
            """CREATE INDEX IF NOT EXISTS idx_audit_memory_namespace
               ON audit(memory_namespace, timestamp DESC)
               WHERE memory_namespace IS NOT NULL"""
        )
        await conn.execute(
            """CREATE INDEX IF NOT EXISTS idx_audit_memory_entry
               ON audit(memory_entry_id, timestamp DESC)
               WHERE memory_entry_id IS NOT NULL"""
        )

        # State index (1 index - legacy)
        await conn.execute("CREATE INDEX IF NOT EXISTS idx_state_task_key ON state(task_id, key)")

        # Metrics index (1 index)
        await conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_metrics_name_timestamp ON metrics(metric_name, timestamp DESC)"
        )

        # Checkpoints index (1 index)
        await conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_checkpoints_task ON checkpoints(task_id, iteration DESC)"
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
                    created_by, parent_task_id, dependencies, session_id
                ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
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
                    task.session_id,
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
            session_id=row_dict.get("session_id"),
        )

    # Agent operations
    async def insert_agent(self, agent: Agent) -> None:
        """Insert a new agent into the database."""
        async with self._get_connection() as conn:
            await conn.execute(
                """
                INSERT INTO agents (
                    id, name, specialization, task_id, state, model,
                    spawned_at, terminated_at, resource_usage, session_id
                ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
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
                    agent.session_id,
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

    # Service properties (lazy-loaded)
    @property
    def memory(self) -> "MemoryService":
        """Get memory service instance.

        Returns:
            MemoryService instance for managing long-term memory
        """
        if self._memory_service is None:
            from abathur.services.memory_service import MemoryService

            self._memory_service = MemoryService(self)
        return self._memory_service

    @property
    def sessions(self) -> "SessionService":
        """Get session service instance.

        Returns:
            SessionService instance for managing conversation sessions
        """
        if self._session_service is None:
            from abathur.services.session_service import SessionService

            self._session_service = SessionService(self)
        return self._session_service

    @property
    def documents(self) -> "DocumentIndexService":
        """Get document index service instance.

        Returns:
            DocumentIndexService instance for managing document indexing and search
        """
        if self._document_service is None:
            from abathur.services.document_index_service import DocumentIndexService

            self._document_service = DocumentIndexService(self)
        return self._document_service
