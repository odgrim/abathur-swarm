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
from pydantic import BaseModel, Field, field_validator, model_validator

from abathur.domain.models import (
    Agent,
    AgentState,
    DependencyType,
    Task,
    TaskDependency,
    TaskSource,
    TaskStatus,
)
from abathur.tui.models import TreeNode

if TYPE_CHECKING:
    from abathur.services.document_index_service import DocumentIndexService
    from abathur.services.memory_service import MemoryService
    from abathur.services.session_service import SessionService


# VACUUM threshold: only run conditional VACUUM if deleting this many tasks
VACUUM_THRESHOLD_TASKS = 100

# Auto-skip VACUUM threshold: automatically set vacuum_mode='never' for large prunes
# Rationale: VACUUM on 10,000+ tasks can take minutes, blocking the database
AUTO_SKIP_VACUUM_THRESHOLD = 10_000


class PruneFilters(BaseModel):
    """Filtering criteria for pruning operation.

    Supports three selection strategies:
    1. By IDs: task_ids list (direct selection)
    2. By status: statuses list (all tasks with given statuses)
    3. By time: older_than_days or before_date (time-based filtering)

    Can combine strategies (e.g., task_ids + statuses, or time + statuses).
    """

    task_ids: list[UUID] | None = Field(
        default=None,
        description="Specific task IDs to delete (direct selection)",
    )

    older_than_days: int | None = Field(
        default=None,
        ge=1,
        description="Delete tasks older than N days (completed_at/submitted_at)",
    )

    before_date: datetime | None = Field(
        default=None, description="Delete tasks completed/submitted before this date"
    )

    statuses: list[TaskStatus] | None = Field(
        default=None,
        description="Task statuses to prune (None = all pruneable statuses)",
    )

    limit: int | None = Field(
        default=None, ge=1, description="Maximum tasks to delete in one operation"
    )

    dry_run: bool = Field(default=False, description="Preview mode without deletion")

    vacuum_mode: str = Field(
        default="conditional",
        description="VACUUM strategy: 'always', 'conditional', or 'never'",
    )

    @model_validator(mode="after")
    def validate_filters(self) -> "PruneFilters":
        """Ensure at least one selection criterion is specified."""
        has_ids = self.task_ids is not None and len(self.task_ids) > 0
        has_time = self.older_than_days is not None or self.before_date is not None
        has_status = self.statuses is not None and len(self.statuses) > 0

        if not (has_ids or has_time or has_status):
            raise ValueError(
                "At least one selection criterion must be specified: "
                "'task_ids', 'older_than_days', 'before_date', or 'statuses'"
            )

        # Set default statuses if using time-based selection without explicit statuses
        if has_time and self.statuses is None:
            self.statuses = [
                TaskStatus.COMPLETED,
                TaskStatus.FAILED,
                TaskStatus.CANCELLED,
            ]

        return self

    @field_validator("statuses")
    @classmethod
    def validate_statuses(cls, v: list[TaskStatus]) -> list[TaskStatus]:
        """Ensure only pruneable statuses are specified."""
        forbidden = {
            TaskStatus.PENDING,
            TaskStatus.BLOCKED,
            TaskStatus.READY,
            TaskStatus.RUNNING,
        }
        invalid = set(v) & forbidden
        if invalid:
            raise ValueError(
                f"Cannot prune tasks with statuses: {invalid}. "
                f"Only COMPLETED, FAILED, or CANCELLED tasks can be pruned."
            )
        return v

    @field_validator("vacuum_mode")
    @classmethod
    def validate_vacuum_mode(cls, v: str) -> str:
        """Validate vacuum_mode is one of allowed values."""
        allowed = {"always", "conditional", "never"}
        if v not in allowed:
            raise ValueError(f"vacuum_mode must be one of {allowed}, got '{v}'")
        return v

    def build_where_clause(self) -> tuple[str, list[str]]:
        """Build SQL WHERE clause and parameters for task filtering.

        Handles multiple selection strategies:
        - ID-based: WHERE id IN (...)
        - Time-based: WHERE completed_at/submitted_at < ...
        - Status-based: WHERE status IN (...)

        Returns:
            Tuple of (where_clause_sql, parameters) where:
            - where_clause_sql: SQL WHERE condition without 'WHERE' keyword
            - parameters: List of parameter values for SQL placeholders

        Used by both CLI preview queries and database prune_tasks() execution
        to ensure consistent filtering logic.
        """
        where_clauses = []
        params = []

        # ID filter (if specified, most specific)
        if self.task_ids is not None and len(self.task_ids) > 0:
            id_placeholders = ",".join("?" * len(self.task_ids))
            where_clauses.append(f"id IN ({id_placeholders})")
            params.extend([str(task_id) for task_id in self.task_ids])

        # Time filter (optional)
        if self.older_than_days is not None:
            where_clauses.append(
                "(completed_at < date('now', ?) OR "
                "(completed_at IS NULL AND submitted_at < date('now', ?)))"
            )
            days_param = f"-{self.older_than_days} days"
            params.extend([days_param, days_param])
        elif self.before_date is not None:
            where_clauses.append(
                "(completed_at < ? OR (completed_at IS NULL AND submitted_at < ?))"
            )
            before_iso = self.before_date.isoformat()
            params.extend([before_iso, before_iso])

        # Status filter (optional)
        if self.statuses is not None and len(self.statuses) > 0:
            status_placeholders = ",".join("?" * len(self.statuses))
            where_clauses.append(f"status IN ({status_placeholders})")
            params.extend([status.value for status in self.statuses])

        # Default to always match if no filters (shouldn't happen due to validation)
        if not where_clauses:
            where_clauses.append("1=1")

        where_sql = " AND ".join(where_clauses)
        return (where_sql, params)


class PruneResult(BaseModel):
    """Statistics from prune operation.

    Contains comprehensive metrics about the pruning operation including
    the number of tasks and dependencies deleted, space reclaimed, and
    a breakdown of deleted tasks by status.
    """

    deleted_tasks: int = Field(ge=0, description="Number of tasks deleted")

    deleted_dependencies: int = Field(
        ge=0, description="Number of task_dependencies records deleted"
    )

    reclaimed_bytes: int | None = Field(
        default=None, ge=0, description="Bytes reclaimed by VACUUM (optional)"
    )

    dry_run: bool = Field(description="Whether this was a dry run")

    breakdown_by_status: dict[TaskStatus, int] = Field(
        default_factory=dict, description="Count of deleted tasks by status"
    )

    vacuum_auto_skipped: bool = Field(
        default=False,
        description="Whether VACUUM was automatically skipped due to large task count (>10,000 tasks)"
    )

    @field_validator("breakdown_by_status")
    @classmethod
    def validate_breakdown_values(cls, v: dict[TaskStatus, int]) -> dict[TaskStatus, int]:
        """Ensure all breakdown values are non-negative."""
        for status, count in v.items():
            if count < 0:
                raise ValueError(
                    f"Breakdown count for status {status} must be non-negative, got {count}"
                )
        return v


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
                # Enable foreign keys for shared memory connection
                await self._shared_conn.execute("PRAGMA foreign_keys=ON")
            yield self._shared_conn
        else:
            # File databases get new connections each time
            async with aiosqlite.connect(str(self.db_path)) as conn:
                conn.row_factory = aiosqlite.Row
                # CRITICAL: Enable foreign keys for EVERY new connection
                # SQLite defaults to foreign_keys=OFF, so we must enable it explicitly
                await conn.execute("PRAGMA foreign_keys=ON")
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

                try:
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

                    await conn.commit()
                    print("Database migration completed successfully")

                except Exception:
                    await conn.execute("ROLLBACK")
                    raise
                finally:
                    # Always re-enable foreign keys, even if re-enable fails
                    await conn.execute("PRAGMA foreign_keys=ON")

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

            # Migration: Add enhanced task queue columns
            if "source" not in column_names:
                print("Migrating database schema: adding enhanced task queue columns")

                # Add source column
                await conn.execute(
                    """
                    ALTER TABLE tasks
                    ADD COLUMN source TEXT NOT NULL DEFAULT 'human'
                    """
                )

                # Add dependency_type column
                await conn.execute(
                    """
                    ALTER TABLE tasks
                    ADD COLUMN dependency_type TEXT NOT NULL DEFAULT 'sequential'
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

            # Migration: Add feature_branch column to tasks
            if "feature_branch" not in column_names:
                print("Migrating database schema: adding feature_branch to tasks")
                await conn.execute(
                    """
                    ALTER TABLE tasks
                    ADD COLUMN feature_branch TEXT
                    """
                )
                await conn.commit()
                print("Added feature_branch column to tasks")

            # Migration: Add task_branch column to tasks
            if "task_branch" not in column_names:
                print("Migrating database schema: adding task_branch to tasks")
                await conn.execute(
                    """
                    ALTER TABLE tasks
                    ADD COLUMN task_branch TEXT
                    """
                )
                await conn.commit()
                print("Added task_branch column to tasks")

            # Migration: Add worktree_path column to tasks
            if "worktree_path" not in column_names:
                print("Migrating database schema: adding worktree_path to tasks")
                await conn.execute(
                    """
                    ALTER TABLE tasks
                    ADD COLUMN worktree_path TEXT
                    """
                )
                await conn.commit()
                print("Added worktree_path column to tasks")

            # Migration: Add summary column to tasks
            if "summary" not in column_names:
                print("Migrating database schema: adding summary to tasks")
                await conn.execute(
                    """
                    ALTER TABLE tasks
                    ADD COLUMN summary TEXT NOT NULL DEFAULT 'Task'
                    """
                )
                # Backfill existing rows with auto-generated summaries
                # This logic MUST match the service layer auto-generation in task_queue_service.py:174-181
                #
                # Service layer logic (for reference):
                #   if source == TaskSource.HUMAN:
                #       summary = "User Prompt: " + description[:126].strip()
                #   else:
                #       summary = description[:140].strip()
                #
                # SQL equivalent:
                #   TRIM(SUBSTR(prompt, 1, 126)) matches description[:126].strip()
                #   TRIM(SUBSTR(prompt, 1, 140)) matches description[:140].strip()
                #
                # Both truncate first, then trim whitespace, ensuring identical behavior.
                await conn.execute(
                    """
                    UPDATE tasks
                    SET summary = CASE
                        WHEN prompt IS NULL OR TRIM(prompt) = '' THEN 'Task'
                        WHEN source = 'human' THEN 'User Prompt: ' || TRIM(SUBSTR(prompt, 1, 126))
                        ELSE TRIM(SUBSTR(prompt, 1, 140))
                    END
                    WHERE summary IS NULL OR TRIM(summary) = ''
                    """
                )
                await conn.commit()
                print("Added summary column to tasks and backfilled existing rows")

            # Migration: Fix idx_tasks_summary partial index with pointless WHERE clause
            # Check if old index exists with WHERE clause
            cursor = await conn.execute(
                """
                SELECT sql FROM sqlite_master
                WHERE type='index' AND name='idx_tasks_summary'
                """
            )
            index_row = await cursor.fetchone()
            if index_row and index_row["sql"] and "WHERE summary IS NOT NULL" in index_row["sql"]:
                print("Migrating index: fixing idx_tasks_summary partial index condition")
                # Drop old index with pointless WHERE clause
                await conn.execute("DROP INDEX IF EXISTS idx_tasks_summary")
                # Recreate without WHERE clause
                await conn.execute("""CREATE INDEX idx_tasks_summary ON tasks(summary)""")
                await conn.commit()
                print("Fixed idx_tasks_summary index")

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

            # Migration: Add CASCADE DELETE to agents.task_id foreign key
            # Check if CASCADE already exists
            cursor = await conn.execute("PRAGMA foreign_key_list(agents)")
            fk_list = await cursor.fetchall()
            task_fk = next(
                (fk for fk in fk_list if fk["table"] == "tasks" and fk["from"] == "task_id"), None
            )

            if task_fk and task_fk["on_delete"] != "CASCADE":
                # Pre-migration data integrity check: detect orphaned agents
                cursor = await conn.execute("""
                    SELECT COUNT(*) as orphan_count
                    FROM agents a
                    LEFT JOIN tasks t ON a.task_id = t.id
                    WHERE a.task_id IS NOT NULL AND t.id IS NULL
                """)
                orphan_result = await cursor.fetchone()
                orphan_count = orphan_result["orphan_count"]

                if orphan_count > 0:
                    print(f"\n{'='*80}")
                    print(f"CASCADE DELETE MIGRATION BLOCKED: Orphaned agent records detected")
                    print(f"{'='*80}")
                    print(f"Found {orphan_count} agent record(s) with task_id references to non-existent tasks.")
                    print(f"Enabling CASCADE DELETE would cause these agents to be deleted when their")
                    print(f"referenced tasks are deleted (but the tasks are already missing).")

                    # Query and display first 5 orphaned records
                    cursor = await conn.execute("""
                        SELECT a.id, a.name, a.task_id, a.spawned_at
                        FROM agents a
                        LEFT JOIN tasks t ON a.task_id = t.id
                        WHERE a.task_id IS NOT NULL AND t.id IS NULL
                        LIMIT 5
                    """)
                    orphans = await cursor.fetchall()

                    print(f"\nSample orphaned records (showing {min(orphan_count, 5)} of {orphan_count}):")
                    for orphan in orphans:
                        print(f"  - Agent ID: {orphan['id']}, Name: {orphan['name']}, "
                              f"Task ID: {orphan['task_id']}, Spawned At: {orphan['spawned_at']}")

                    print(f"\nRECOMMENDED ACTIONS:")
                    print(f"1. Identify all orphaned agents:")
                    print(f"   SELECT a.id, a.name, a.task_id, a.spawned_at")
                    print(f"   FROM agents a")
                    print(f"   LEFT JOIN tasks t ON a.task_id = t.id")
                    print(f"   WHERE a.task_id IS NOT NULL AND t.id IS NULL;")
                    print(f"\n2. Choose one of the following:")
                    print(f"   a) Delete orphaned agents (if they're obsolete):")
                    print(f"      DELETE FROM agents")
                    print(f"      WHERE task_id NOT IN (SELECT id FROM tasks);")
                    print(f"\n   b) Restore missing tasks (if they were deleted by mistake)")
                    print(f"\n   c) Set task_id to NULL for orphaned agents (keep agents but break reference):")
                    print(f"      UPDATE agents SET task_id = NULL")
                    print(f"      WHERE task_id NOT IN (SELECT id FROM tasks);")

                    print(f"\nSKIPPING CASCADE DELETE migration to prevent unintended data loss.")
                    print(f"Please resolve orphaned records and restart to retry migration.")
                    print(f"{'='*80}\n")
                else:
                    # No orphans detected - proceed with migration
                    print("Migrating database schema: adding CASCADE DELETE to agents.task_id foreign key")

                    try:
                        # Temporarily disable foreign keys
                        await conn.execute("PRAGMA foreign_keys=OFF")

                        # Create new table with CASCADE DELETE
                        await conn.execute(
                            """
                            CREATE TABLE agents_new (
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
                                FOREIGN KEY (task_id) REFERENCES tasks(id) ON DELETE CASCADE,
                                FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE SET NULL
                            )
                            """
                        )

                        # Copy data from old table to new table
                        await conn.execute(
                            """
                            INSERT INTO agents_new
                            SELECT * FROM agents
                            """
                        )

                        # Drop old table
                        await conn.execute("DROP TABLE agents")

                        # Rename new table to agents
                        await conn.execute("ALTER TABLE agents_new RENAME TO agents")

                        # Recreate indexes
                        await conn.execute(
                            "CREATE INDEX IF NOT EXISTS idx_agents_task ON agents(task_id)"
                        )
                        await conn.execute(
                            "CREATE INDEX IF NOT EXISTS idx_agents_state ON agents(state)"
                        )
                        await conn.execute(
                            """CREATE INDEX IF NOT EXISTS idx_agents_session
                               ON agents(session_id, spawned_at DESC)
                               WHERE session_id IS NOT NULL"""
                        )

                        await conn.commit()
                        print("Added CASCADE DELETE to agents.task_id foreign key")

                    except Exception:
                        await conn.execute("ROLLBACK")
                        raise
                    finally:
                        # Always re-enable foreign keys, even if re-enable fails
                        await conn.execute("PRAGMA foreign_keys=ON")

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

            # Migration: Add CASCADE DELETE to checkpoints.task_id foreign key
            # Check if CASCADE already exists
            cursor = await conn.execute("PRAGMA foreign_key_list(checkpoints)")
            fk_list = await cursor.fetchall()
            task_fk = next(
                (fk for fk in fk_list if fk["table"] == "tasks" and fk["from"] == "task_id"), None
            )

            if task_fk and task_fk["on_delete"] != "CASCADE":
                # Pre-migration data integrity check: detect orphaned checkpoints
                cursor = await conn.execute("""
                    SELECT COUNT(*) as orphan_count
                    FROM checkpoints c
                    LEFT JOIN tasks t ON c.task_id = t.id
                    WHERE c.task_id IS NOT NULL AND t.id IS NULL
                """)
                orphan_result = await cursor.fetchone()
                orphan_count = orphan_result["orphan_count"]

                if orphan_count > 0:
                    print(f"\n{'='*80}")
                    print(f"CASCADE DELETE MIGRATION BLOCKED: Orphaned checkpoint records detected")
                    print(f"{'='*80}")
                    print(f"Found {orphan_count} checkpoint record(s) with task_id references to non-existent tasks.")
                    print(f"Enabling CASCADE DELETE would cause these checkpoints to be deleted when their")
                    print(f"referenced tasks are deleted (but the tasks are already missing).")

                    # Query and display first 5 orphaned records
                    cursor = await conn.execute("""
                        SELECT c.task_id, c.iteration, c.created_at
                        FROM checkpoints c
                        LEFT JOIN tasks t ON c.task_id = t.id
                        WHERE c.task_id IS NOT NULL AND t.id IS NULL
                        LIMIT 5
                    """)
                    orphans = await cursor.fetchall()

                    print(f"\nSample orphaned records (showing {min(orphan_count, 5)} of {orphan_count}):")
                    for orphan in orphans:
                        print(f"  - Task ID: {orphan['task_id']}, Iteration: {orphan['iteration']}, "
                              f"Created At: {orphan['created_at']}")

                    print(f"\nRECOMMENDED ACTIONS:")
                    print(f"1. Identify all orphaned checkpoints:")
                    print(f"   SELECT c.task_id, c.iteration, c.created_at")
                    print(f"   FROM checkpoints c")
                    print(f"   LEFT JOIN tasks t ON c.task_id = t.id")
                    print(f"   WHERE c.task_id IS NOT NULL AND t.id IS NULL;")
                    print(f"\n2. Choose one of the following:")
                    print(f"   a) Delete orphaned checkpoints (if they're obsolete):")
                    print(f"      DELETE FROM checkpoints")
                    print(f"      WHERE task_id NOT IN (SELECT id FROM tasks);")
                    print(f"\n   b) Restore missing tasks (if they were deleted by mistake)")

                    print(f"\nSKIPPING CASCADE DELETE migration to prevent unintended data loss.")
                    print(f"Please resolve orphaned records and restart to retry migration.")
                    print(f"{'='*80}\n")
                else:
                    # No orphans detected - proceed with migration
                    print("Migrating database schema: adding CASCADE DELETE to checkpoints.task_id foreign key")

                    try:
                        # Temporarily disable foreign keys
                        await conn.execute("PRAGMA foreign_keys=OFF")

                        # Create new table with CASCADE DELETE
                        await conn.execute(
                            """
                            CREATE TABLE checkpoints_new (
                                task_id TEXT NOT NULL,
                                iteration INTEGER NOT NULL,
                                state TEXT NOT NULL,
                                created_at TIMESTAMP NOT NULL,
                                session_id TEXT,
                                PRIMARY KEY (task_id, iteration),
                                FOREIGN KEY (task_id) REFERENCES tasks(id) ON DELETE CASCADE,
                                FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE SET NULL
                            )
                            """
                        )

                        # Copy data from old table to new table
                        await conn.execute(
                            """
                            INSERT INTO checkpoints_new
                            SELECT * FROM checkpoints
                            """
                        )

                        # Drop old table
                        await conn.execute("DROP TABLE checkpoints")

                        # Rename new table to checkpoints
                        await conn.execute("ALTER TABLE checkpoints_new RENAME TO checkpoints")

                        # Recreate indexes
                        await conn.execute(
                            "CREATE INDEX IF NOT EXISTS idx_checkpoints_task ON checkpoints(task_id, iteration DESC)"
                        )

                        await conn.commit()
                        print("Added CASCADE DELETE to checkpoints.task_id foreign key")

                    except Exception:
                        await conn.execute("ROLLBACK")
                        raise
                    finally:
                        # Always re-enable foreign keys, even if re-enable fails
                        await conn.execute("PRAGMA foreign_keys=ON")

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

                try:
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

                except Exception as e:
                    # Re-enable foreign keys even on error
                    await conn.execute("PRAGMA foreign_keys=ON")
                    await conn.rollback()
                    print(f"✗ Migration failed: {type(e).__name__}: {e}")
                    print("Database rolled back to previous state")
                    raise  # Re-raise to prevent application from starting with failed migration

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
                source TEXT NOT NULL DEFAULT 'human',
                dependency_type TEXT NOT NULL DEFAULT 'sequential',
                calculated_priority REAL NOT NULL DEFAULT 5.0,
                deadline TIMESTAMP,
                estimated_duration_seconds INTEGER,
                dependency_depth INTEGER DEFAULT 0,
                feature_branch TEXT,
                task_branch TEXT,
                worktree_path TEXT,
                summary TEXT NOT NULL DEFAULT 'Task',
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
                FOREIGN KEY (task_id) REFERENCES tasks(id) ON DELETE CASCADE,
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
                FOREIGN KEY (task_id) REFERENCES tasks(id) ON DELETE CASCADE,
                FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE SET NULL
            )
        """
        )

        # Task dependencies table
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

    async def _create_indexes(self, conn: Connection) -> None:
        """Create all performance indexes."""
        # Sessions indexes (5 indexes)
        await conn.execute("CREATE UNIQUE INDEX IF NOT EXISTS idx_sessions_pk ON sessions(id)")
        await conn.execute(
            """CREATE INDEX IF NOT EXISTS idx_sessions_status_updated
               ON sessions(status, last_update_time DESC)
               WHERE status IN ('active', 'paused')"""
        )
        await conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_sessions_status ON sessions(status)"
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

        # Summary field index for search and filtering
        await conn.execute(
            """CREATE INDEX IF NOT EXISTS idx_tasks_summary
               ON tasks(summary)"""
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

        # NEW: Task dependencies indexes (2 indexes)
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

        # Feature branch coordination index
        await conn.execute(
            """
            CREATE INDEX IF NOT EXISTS idx_tasks_feature_branch
            ON tasks(feature_branch, status, submitted_at ASC)
            WHERE feature_branch IS NOT NULL
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
                    created_by, parent_task_id, dependencies, session_id,
                    source, dependency_type, calculated_priority, deadline,
                    estimated_duration_seconds, dependency_depth, feature_branch, task_branch, worktree_path, summary
                ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
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
                    task.source.value,
                    task.dependency_type.value,
                    task.calculated_priority,
                    task.deadline.isoformat() if task.deadline else None,
                    task.estimated_duration_seconds,
                    task.dependency_depth,
                    task.feature_branch,
                    task.task_branch,
                    task.worktree_path,
                    task.summary,
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

    async def list_tasks(
        self,
        status: TaskStatus | None = None,
        limit: int = 100,
        source: TaskSource | None = None,
        agent_type: str | None = None,
        feature_branch: str | None = None,
    ) -> list[Task]:
        """List tasks with optional filters.

        Args:
            status: Filter by task status
            limit: Maximum number of tasks to return
            source: Filter by task source
            agent_type: Filter by agent type
            feature_branch: Filter by feature branch

        Returns:
            List of tasks matching the filters
        """
        async with self._get_connection() as conn:
            # Build dynamic query based on filters
            where_clauses: list[str] = []
            params: list[Any] = []

            if status:
                where_clauses.append("status = ?")
                params.append(status.value)

            if source:
                where_clauses.append("source = ?")
                params.append(source.value)

            if agent_type:
                where_clauses.append("agent_type = ?")
                params.append(agent_type)

            if feature_branch:
                where_clauses.append("feature_branch = ?")
                params.append(feature_branch)

            where_sql = f"WHERE {' AND '.join(where_clauses)}" if where_clauses else ""

            query = f"""
                SELECT * FROM tasks
                {where_sql}
                ORDER BY priority DESC, submitted_at ASC
                LIMIT ?
            """
            params.append(limit)

            cursor = await conn.execute(query, tuple(params))
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

    async def get_feature_branch_summary(self, feature_branch: str) -> dict[str, Any]:
        """Get comprehensive summary of all tasks for a feature branch.

        Args:
            feature_branch: Feature branch name to summarize

        Returns:
            Dictionary with task counts, status breakdown, and progress metrics
        """
        async with self._get_connection() as conn:
            # Get overall task counts by status
            cursor = await conn.execute(
                """
                SELECT status, COUNT(*) as count
                FROM tasks
                WHERE feature_branch = ?
                GROUP BY status
                """,
                (feature_branch,),
            )
            status_counts = {row["status"]: row["count"] for row in await cursor.fetchall()}

            # Get total task count
            total_tasks = sum(status_counts.values())

            # Calculate progress metrics
            completed_count = status_counts.get(TaskStatus.COMPLETED.value, 0)
            failed_count = status_counts.get(TaskStatus.FAILED.value, 0)
            running_count = status_counts.get(TaskStatus.RUNNING.value, 0)
            pending_count = status_counts.get(TaskStatus.PENDING.value, 0)
            blocked_count = status_counts.get(TaskStatus.BLOCKED.value, 0)
            ready_count = status_counts.get(TaskStatus.READY.value, 0)

            # Get earliest and latest task timestamps
            cursor = await conn.execute(
                """
                SELECT MIN(submitted_at) as earliest,
                       MAX(COALESCE(completed_at, last_updated_at)) as latest
                FROM tasks
                WHERE feature_branch = ?
                """,
                (feature_branch,),
            )
            timestamps = await cursor.fetchone()

            # Get agent type breakdown
            cursor = await conn.execute(
                """
                SELECT agent_type, COUNT(*) as count,
                       SUM(CASE WHEN status = 'completed' THEN 1 ELSE 0 END) as completed
                FROM tasks
                WHERE feature_branch = ?
                GROUP BY agent_type
                ORDER BY count DESC
                """,
                (feature_branch,),
            )
            agent_breakdown = [
                {
                    "agent_type": row["agent_type"],
                    "total": row["count"],
                    "completed": row["completed"],
                }
                for row in await cursor.fetchall()
            ]

            return {
                "feature_branch": feature_branch,
                "total_tasks": total_tasks,
                "status_breakdown": status_counts,
                "progress": {
                    "completed": completed_count,
                    "failed": failed_count,
                    "running": running_count,
                    "pending": pending_count,
                    "blocked": blocked_count,
                    "ready": ready_count,
                    "completion_rate": (
                        round(completed_count / total_tasks * 100, 2) if total_tasks > 0 else 0
                    ),
                },
                "agent_breakdown": agent_breakdown,
                "timestamps": {
                    "earliest_task": timestamps["earliest"]
                    if timestamps and timestamps["earliest"]
                    else None,
                    "latest_activity": timestamps["latest"]
                    if timestamps and timestamps["latest"]
                    else None,
                },
            }

    async def get_feature_branch_tasks(
        self, feature_branch: str, status: TaskStatus | None = None
    ) -> list[Task]:
        """Get all tasks for a specific feature branch, optionally filtered by status.

        Args:
            feature_branch: Feature branch name
            status: Optional status filter

        Returns:
            List of tasks for the feature branch
        """
        return await self.list_tasks(feature_branch=feature_branch, status=status, limit=1000)

    async def get_feature_branch_blockers(self, feature_branch: str) -> list[dict[str, Any]]:
        """Identify blocking issues for a feature branch.

        Returns tasks that are failed or blocked, potentially preventing feature completion.

        Args:
            feature_branch: Feature branch name

        Returns:
            List of blocker task information
        """
        async with self._get_connection() as conn:
            cursor = await conn.execute(
                """
                SELECT id, prompt, agent_type, status, error_message, retry_count,
                       submitted_at, started_at
                FROM tasks
                WHERE feature_branch = ?
                  AND status IN ('failed', 'blocked')
                ORDER BY submitted_at ASC
                """,
                (feature_branch,),
            )
            rows = await cursor.fetchall()
            return [
                {
                    "task_id": row["id"],
                    "prompt": row["prompt"],
                    "agent_type": row["agent_type"],
                    "status": row["status"],
                    "error_message": row["error_message"],
                    "retry_count": row["retry_count"],
                    "submitted_at": row["submitted_at"],
                    "started_at": row["started_at"],
                }
                for row in rows
            ]

    async def list_feature_branches(self) -> list[dict[str, Any]]:
        """List all feature branches with task statistics.

        Returns:
            List of feature branch summaries
        """
        async with self._get_connection() as conn:
            cursor = await conn.execute(
                """
                SELECT feature_branch,
                       COUNT(*) as total_tasks,
                       SUM(CASE WHEN status = 'completed' THEN 1 ELSE 0 END) as completed,
                       SUM(CASE WHEN status = 'failed' THEN 1 ELSE 0 END) as failed,
                       SUM(CASE WHEN status = 'running' THEN 1 ELSE 0 END) as running,
                       SUM(CASE WHEN status IN ('pending', 'blocked', 'ready') THEN 1 ELSE 0 END) as pending,
                       MIN(submitted_at) as earliest_task,
                       MAX(last_updated_at) as latest_activity
                FROM tasks
                WHERE feature_branch IS NOT NULL
                GROUP BY feature_branch
                ORDER BY latest_activity DESC
                """
            )
            rows = await cursor.fetchall()
            return [
                {
                    "feature_branch": row["feature_branch"],
                    "total_tasks": row["total_tasks"],
                    "completed": row["completed"],
                    "failed": row["failed"],
                    "running": row["running"],
                    "pending": row["pending"],
                    "completion_rate": (
                        round(row["completed"] / row["total_tasks"] * 100, 2)
                        if row["total_tasks"] > 0
                        else 0
                    ),
                    "earliest_task": row["earliest_task"],
                    "latest_activity": row["latest_activity"],
                }
                for row in rows
            ]

    async def get_child_tasks(self, parent_task_ids: list[UUID]) -> list[Task]:
        """Query all tasks that have any of the given parent_task_ids.

        Args:
            parent_task_ids: List of parent task UUIDs to check for children

        Returns:
            List of Task domain objects representing child tasks

        Raises:
            ValueError: If parent_task_ids is empty
        """
        if not parent_task_ids:
            raise ValueError("parent_task_ids cannot be empty")

        async with self._get_connection() as conn:
            # Build dynamic IN clause with placeholders
            placeholders = ",".join("?" * len(parent_task_ids))
            query = f"""
                SELECT * FROM tasks
                WHERE parent_task_id IN ({placeholders})
                ORDER BY submitted_at ASC
            """

            cursor = await conn.execute(
                query,
                tuple(str(task_id) for task_id in parent_task_ids),
            )
            rows = await cursor.fetchall()
            return [self._row_to_task(row) for row in rows]

    async def delete_task(self, task_id: UUID) -> bool:
        """Delete a single task by UUID.

        Args:
            task_id: Task ID to delete

        Returns:
            True if task was deleted, False if not found
        """
        async with self._get_connection() as conn:
            cursor = await conn.execute(
                "DELETE FROM tasks WHERE id = ?",
                (str(task_id),),
            )
            await conn.commit()
            return cursor.rowcount > 0

    async def delete_task_by_id(self, task_id: UUID) -> bool:
        """Delete a single task by ID with CASCADE to dependent tables.

        Args:
            task_id: Task ID to delete

        Returns:
            True if task was deleted, False if not found

        Raises:
            DatabaseError: If deletion fails
        """
        async with self._get_connection() as conn:
            cursor = await conn.execute(
                "DELETE FROM tasks WHERE id = ?",
                (str(task_id),),
            )
            await conn.commit()
            return cursor.rowcount > 0

    async def delete_tasks_by_status(self, status: TaskStatus) -> int:
        """Delete all tasks matching a status filter with CASCADE to dependent tables.

        Args:
            status: Status filter for deletion (TaskStatus enum)

        Returns:
            Number of tasks deleted

        Raises:
            DatabaseError: If deletion fails
        """
        async with self._get_connection() as conn:
            cursor = await conn.execute(
                "DELETE FROM tasks WHERE status = ?",
                (status.value,),
            )
            await conn.commit()
            return cursor.rowcount

    async def _delete_tasks_by_ids(
        self,
        conn: Connection,
        task_ids: list[str],
        collect_stats: bool = False
    ) -> dict[str, Any]:
        """Core task deletion logic - unified implementation.

        This is the single source of truth for task deletion.
        Both delete_tasks() and prune_tasks() use this.

        Handles large deletions by batching task IDs to avoid SQLite's
        999 parameter limit (SQLITE_MAX_VARIABLE_NUMBER).

        Args:
            conn: Active database connection (with foreign keys enabled)
            task_ids: List of task ID strings to delete
            collect_stats: Whether to collect status breakdown statistics

        Returns:
            Dictionary with:
                - deleted_count: Number of tasks deleted
                - deleted_dependencies: Number of dependencies deleted
                - breakdown_by_status: Dict of status -> count (if collect_stats=True)

        Side Effects:
            - Orphans children (sets parent_task_id to NULL)
            - Deletes state entries
            - Deletes task dependencies
            - Deletes tasks
        """
        if not task_ids:
            return {
                "deleted_count": 0,
                "deleted_dependencies": 0,
                "breakdown_by_status": {}
            }

        # SQLite has a limit of 999 SQL parameters per query (SQLITE_MAX_VARIABLE_NUMBER).
        # To safely handle large deletions, we batch task IDs into chunks of 900.
        # This ensures we stay well below the limit even when parameters are used multiple times
        # in a single query (e.g., "WHERE id IN (?) OR parent_id IN (?)").
        BATCH_SIZE = 900

        # Accumulate statistics across all batches
        total_deleted_count = 0
        total_deleted_dependencies = 0
        combined_breakdown_by_status: dict[TaskStatus, int] = {}

        # Process task IDs in batches
        for i in range(0, len(task_ids), BATCH_SIZE):
            batch = task_ids[i:i + BATCH_SIZE]
            task_id_placeholders = ",".join("?" * len(batch))

            # Collect statistics if requested
            if collect_stats:
                cursor = await conn.execute(
                    f"""
                    SELECT status, COUNT(*) as count
                    FROM tasks
                    WHERE id IN ({task_id_placeholders})
                    GROUP BY status
                    """,
                    tuple(batch),
                )
                status_rows = await cursor.fetchall()
                for row in status_rows:
                    status = TaskStatus(row["status"])
                    combined_breakdown_by_status[status] = (
                        combined_breakdown_by_status.get(status, 0) + row["count"]
                    )

            # Count dependencies before deletion for this batch
            cursor = await conn.execute(
                f"""
                SELECT COUNT(*) as count
                FROM task_dependencies
                WHERE prerequisite_task_id IN ({task_id_placeholders})
                   OR dependent_task_id IN ({task_id_placeholders})
                """,
                tuple(batch + batch),
            )
            dep_row = await cursor.fetchone()
            batch_deleted_dependencies = dep_row["count"] if dep_row else 0
            total_deleted_dependencies += batch_deleted_dependencies

            # Execute deletion for this batch (orphans children, cleans state, deletes tasks)
            await self._orphan_children_and_delete_tasks(conn, batch)

            total_deleted_count += len(batch)

        return {
            "deleted_count": total_deleted_count,
            "deleted_dependencies": total_deleted_dependencies,
            "breakdown_by_status": combined_breakdown_by_status
        }

    async def _orphan_children_and_delete_tasks(
        self, conn: Connection, task_ids: list[str]
    ) -> None:
        """Delete tasks and orphan their children (set parent_task_id to NULL).

        This is the correct behavior for prune operations - only delete tasks
        that match the filter criteria, and orphan any children that don't match.

        Handles large deletions by batching task IDs to avoid SQLite's
        999 parameter limit (SQLITE_MAX_VARIABLE_NUMBER).

        Handles foreign key constraints by:
        1. Setting children's parent_task_id to NULL (orphaning)
        2. Nullifying audit.agent_id to allow agent CASCADE deletion
        3. Deleting state table entries (no CASCADE on state.task_id FK)
        4. Deleting task dependencies (CASCADE but explicit for clarity)
        5. Deleting the tasks themselves (agents, checkpoints CASCADE automatically)

        Args:
            conn: Active database connection
            task_ids: List of task ID strings to delete

        Side Effects:
            - Updates tasks.parent_task_id to NULL for children (orphaning)
            - Updates audit.agent_id to NULL for affected agents
            - Deletes from state table (explicit cleanup)
            - Deletes from task_dependencies (explicit cleanup)
            - Deletes from tasks table (agents, checkpoints CASCADE)
        """
        if not task_ids:
            return

        # SQLite has a limit of 999 SQL parameters per query (SQLITE_MAX_VARIABLE_NUMBER).
        # Batch task IDs into chunks of 900 to safely handle large deletions.
        BATCH_SIZE = 900

        # Process task IDs in batches, executing all 5 deletion steps for each batch
        for i in range(0, len(task_ids), BATCH_SIZE):
            batch = task_ids[i:i + BATCH_SIZE]
            task_id_placeholders = ",".join("?" * len(batch))

            # Step 1: Orphan children (set parent_task_id to NULL)
            # This allows us to delete parents without violating FK constraints
            await conn.execute(
                f"""
                UPDATE tasks
                SET parent_task_id = NULL
                WHERE parent_task_id IN ({task_id_placeholders})
                """,
                tuple(batch),
            )

            # Step 2: Clean up audit.agent_id to allow agent CASCADE deletion
            # audit.agent_id has FK to agents WITHOUT CASCADE, so we must NULL it first
            await conn.execute(
                f"""
                UPDATE audit
                SET agent_id = NULL
                WHERE agent_id IN (
                    SELECT id FROM agents WHERE task_id IN ({task_id_placeholders})
                )
                """,
                tuple(batch),
            )

            # Step 3: Delete state entries for current tasks
            await conn.execute(
                f"""
                DELETE FROM state
                WHERE task_id IN ({task_id_placeholders})
                """,
                tuple(batch),
            )

            # Step 4: Delete task dependencies
            await conn.execute(
                f"""
                DELETE FROM task_dependencies
                WHERE prerequisite_task_id IN ({task_id_placeholders})
                   OR dependent_task_id IN ({task_id_placeholders})
                """,
                tuple(batch + batch),
            )

            # Step 5: Delete the tasks themselves
            # At this point:
            # - Children are orphaned (parent_task_id=NULL)
            # - Audit entries won't block agent deletion (agent_id=NULL)
            # - State entries are deleted
            # - Task dependencies are deleted
            # - Agents, checkpoints will CASCADE
            await conn.execute(
                f"""
                DELETE FROM tasks
                WHERE id IN ({task_id_placeholders})
                """,
                tuple(batch),
            )

    def _order_tasks_by_depth(self, tree_nodes: list[TreeNode]) -> list[UUID]:
        """Order tasks for deletion in leaves-to-root order (deepest first).

        Groups tasks by their depth in the tree hierarchy and returns them
        ordered from deepest (leaves) to shallowest (roots). This ensures
        that child tasks are always deleted before their parents, maintaining
        referential integrity.

        Args:
            tree_nodes: Validated deletable nodes with level (depth) information

        Returns:
            Task IDs ordered deepest-to-shallowest for safe deletion

        Example:
            Given a tree:
                Root (level=0)
                ├── Child1 (level=1)
                │   └── Grandchild1 (level=2)
                └── Child2 (level=1)

            Returns: [Grandchild1, Child1, Child2, Root]
        """
        # Group nodes by depth (using 'level' attribute from TreeNode)
        depth_groups: dict[int, list[UUID]] = {}

        for node in tree_nodes:
            if node.level not in depth_groups:
                depth_groups[node.level] = []
            depth_groups[node.level].append(node.task_id)

        # Order by depth descending (deepest first)
        ordered_ids = []
        for depth in sorted(depth_groups.keys(), reverse=True):
            ordered_ids.extend(depth_groups[depth])

        return ordered_ids

    async def prune_tasks(self, filters: PruneFilters) -> PruneResult:
        """Prune tasks based on age and status criteria.

        This method handles:
        1. Task selection (via filters)
        2. Task deletion (via unified core)
        3. Statistics collection
        4. Optional VACUUM

        VACUUM behavior depends on filters.vacuum_mode:
        - "always": Always run VACUUM after deletion (may be slow)
        - "conditional": Only run VACUUM if deleted_tasks >= 100 (default)
        - "never": Never run VACUUM (fastest, but doesn't reclaim space)

        Args:
            filters: PruneFilters with deletion criteria and vacuum_mode

        Returns:
            PruneResult with deletion counts and reclaimed bytes (if VACUUM ran)

        Raises:
            ValueError: If filters are invalid
            DatabaseError: If deletion fails
        """
        async with self._get_connection() as conn:
            # STEP 1: SELECT tasks to delete using filters
            where_sql, params = filters.build_where_clause()
            limit_sql = f"LIMIT {filters.limit}" if filters.limit else ""

            cursor = await conn.execute(
                f"""
                SELECT id FROM tasks
                WHERE {where_sql}
                ORDER BY submitted_at ASC
                {limit_sql}
                """,
                tuple(params),
            )
            task_rows = await cursor.fetchall()
            task_ids = [row["id"] for row in task_rows]

            if not task_ids:
                # Nothing to delete
                return PruneResult(
                    deleted_tasks=0,
                    deleted_dependencies=0,
                    reclaimed_bytes=None,
                    dry_run=filters.dry_run,
                    breakdown_by_status={},
                    vacuum_auto_skipped=False,
                )

            # Auto-selection: override vacuum_mode to 'never' for large prune operations
            # Only applies to 'conditional' mode - 'always' is never overridden
            vacuum_auto_skipped = False
            effective_vacuum_mode = filters.vacuum_mode

            if len(task_ids) >= AUTO_SKIP_VACUUM_THRESHOLD and filters.vacuum_mode == "conditional":
                effective_vacuum_mode = "never"
                vacuum_auto_skipped = True

            # Start transaction
            await conn.execute("BEGIN TRANSACTION")

            try:
                # STEP 2: DELETE tasks using unified core (collects stats)
                if filters.dry_run:
                    # Dry run: collect stats without deleting
                    result = await self._delete_tasks_by_ids(
                        conn, task_ids, collect_stats=True
                    )
                    await conn.execute("ROLLBACK")

                    return PruneResult(
                        deleted_tasks=result["deleted_count"],
                        deleted_dependencies=result["deleted_dependencies"],
                        reclaimed_bytes=None,
                        dry_run=True,
                        breakdown_by_status=result["breakdown_by_status"],
                        vacuum_auto_skipped=vacuum_auto_skipped,
                    )
                else:
                    # Real deletion
                    result = await self._delete_tasks_by_ids(
                        conn, task_ids, collect_stats=True
                    )
                    await conn.commit()

                    # STEP 3: VACUUM (outside transaction, conditional)
                    # Use effective_vacuum_mode which may have been auto-overridden
                    reclaimed_bytes = None
                    should_vacuum = False

                    if effective_vacuum_mode == "always":
                        should_vacuum = True
                    elif effective_vacuum_mode == "conditional":
                        should_vacuum = result["deleted_count"] >= VACUUM_THRESHOLD_TASKS
                    # "never" mode: should_vacuum stays False

                    if should_vacuum:
                        # Get database size before VACUUM
                        cursor = await conn.execute("PRAGMA page_count")
                        page_count_row = await cursor.fetchone()
                        cursor = await conn.execute("PRAGMA page_size")
                        page_size_row = await cursor.fetchone()

                        if page_count_row and page_size_row:
                            page_count_before = page_count_row[0]
                            page_size = page_size_row[0]
                            size_before = page_count_before * page_size

                            # Run VACUUM
                            await conn.execute("VACUUM")

                            # Get database size after VACUUM
                            cursor = await conn.execute("PRAGMA page_count")
                            page_count_after_row = await cursor.fetchone()
                            if page_count_after_row:
                                page_count_after = page_count_after_row[0]
                                size_after = page_count_after * page_size
                                reclaimed_bytes = size_before - size_after

                    return PruneResult(
                        deleted_tasks=result["deleted_count"],
                        deleted_dependencies=result["deleted_dependencies"],
                        reclaimed_bytes=reclaimed_bytes,
                        dry_run=False,
                        breakdown_by_status=result["breakdown_by_status"],
                        vacuum_auto_skipped=vacuum_auto_skipped,
                    )

            except Exception as e:
                await conn.execute("ROLLBACK")
                raise RuntimeError(f"Failed to prune tasks: {e}") from e

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
            dependencies=[UUID(dep) for dep in json.loads(row_dict["dependencies"])]
            if row_dict.get("dependencies")
            else [],
            session_id=row_dict.get("session_id"),
            # NEW: Enhanced task queue fields
            source=TaskSource(row_dict.get("source", "human")),
            dependency_type=DependencyType(row_dict.get("dependency_type", "sequential")),
            calculated_priority=row_dict.get("calculated_priority", 5.0),
            deadline=datetime.fromisoformat(row_dict["deadline"])
            if row_dict.get("deadline")
            else None,
            estimated_duration_seconds=row_dict.get("estimated_duration_seconds"),
            dependency_depth=row_dict.get("dependency_depth", 0),
            feature_branch=row_dict.get("feature_branch"),
            task_branch=row_dict.get("task_branch"),
            worktree_path=row_dict.get("worktree_path"),
            summary=row_dict.get("summary") or "Task",
        )

    # Task dependency operations
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
