"""Memory management service for long-term persistent memory."""

import json
from typing import Any

from abathur.infrastructure.database import Database


class MemoryService:
    """Service for managing long-term memory storage and retrieval.

    Supports semantic memory (facts), episodic memory (experiences),
    and procedural memory (rules/instructions) with hierarchical
    namespace organization.
    """

    def __init__(self, db: Database) -> None:
        """Initialize memory service.

        Args:
            db: Database instance for storage operations
        """
        self.db = db

    async def add_memory(
        self,
        namespace: str,
        key: str,
        value: dict[str, Any],
        memory_type: str,
        created_by: str,
        task_id: str | None = None,
        metadata: dict[str, Any] | None = None,
    ) -> int:
        """Add new memory entry (version 1).

        Args:
            namespace: Hierarchical namespace (e.g., "user:alice:preferences")
            key: Unique key within namespace
            value: Memory content (must be JSON-serializable dict)
            memory_type: Type (semantic|episodic|procedural)
            created_by: Session or agent ID that created this memory
            task_id: Task context for audit logging
            metadata: Optional metadata dictionary

        Returns:
            ID of created memory entry

        Raises:
            ValueError: If namespace, memory_type, or value is invalid

        Example:
            >>> memory_id = await memory_service.add_memory(
            ...     namespace="user:alice:preferences",
            ...     key="communication_style",
            ...     value={"language": "concise", "technical_level": "expert"},
            ...     memory_type="semantic",
            ...     created_by="session:abc123",
            ...     task_id="task:xyz789"
            ... )
        """
        # Validate memory_type
        valid_types = {"semantic", "episodic", "procedural"}
        if memory_type not in valid_types:
            raise ValueError(f"Invalid memory_type: {memory_type}. Must be one of {valid_types}")

        # Validate namespace format (basic check)
        if ":" not in namespace:
            raise ValueError(f"Invalid namespace format: {namespace}. Must contain ':' separator")

        async with self.db._get_connection() as conn:
            try:
                async with conn.execute("BEGIN"):
                    # Insert memory entry
                    cursor = await conn.execute(
                        """
                        INSERT INTO memory_entries (
                            namespace, key, value, memory_type, version, metadata, created_by, updated_by
                        )
                        VALUES (?, ?, ?, ?, 1, ?, ?, ?)
                        """,
                        (
                            namespace,
                            key,
                            json.dumps(value),
                            memory_type,
                            json.dumps(metadata or {}),
                            created_by,
                            created_by,
                        ),
                    )
                    memory_id = cursor.lastrowid
                    assert memory_id is not None, "Failed to get memory_id after insert"

                    # Audit log
                    await conn.execute(
                        """
                        INSERT INTO audit (
                            timestamp, task_id, action_type, memory_operation_type,
                            memory_namespace, memory_entry_id, action_data
                        )
                        VALUES (CURRENT_TIMESTAMP, ?, 'memory_create', 'create', ?, ?, ?)
                        """,
                        (
                            task_id,
                            namespace,
                            memory_id,
                            json.dumps({"key": key, "memory_type": memory_type}),
                        ),
                    )

                    await conn.commit()
                    return int(memory_id)
            except Exception:
                await conn.rollback()
                raise

    async def get_memory(
        self, namespace: str, key: str, version: int | None = None
    ) -> dict[str, Any] | None:
        """Retrieve memory entry (latest version or specific version).

        Args:
            namespace: Hierarchical namespace
            key: Memory key
            version: Optional specific version (defaults to latest active version)

        Returns:
            Memory entry dict with parsed value, or None if not found

        Example:
            >>> memory = await memory_service.get_memory("user:alice:preferences", "theme")
            >>> print(memory['value'])  # {"mode": "dark"}
            >>> print(memory['version'])  # 3
        """
        async with self.db._get_connection() as conn:
            if version is None:
                # Get latest active version
                cursor = await conn.execute(
                    """
                    SELECT * FROM memory_entries
                    WHERE namespace = ? AND key = ? AND is_deleted = 0
                    ORDER BY version DESC
                    LIMIT 1
                    """,
                    (namespace, key),
                )
            else:
                # Get specific version
                cursor = await conn.execute(
                    """
                    SELECT * FROM memory_entries
                    WHERE namespace = ? AND key = ? AND version = ?
                    """,
                    (namespace, key, version),
                )

            row = await cursor.fetchone()
            if row is None:
                return None

            # Parse JSON fields
            memory = dict(row)
            memory["value"] = json.loads(memory["value"])
            memory["metadata"] = json.loads(memory.get("metadata", "{}"))

            return memory

    async def update_memory(
        self,
        namespace: str,
        key: str,
        value: dict[str, Any],
        updated_by: str,
        task_id: str | None = None,
    ) -> int:
        """Update memory by creating new version.

        Args:
            namespace: Hierarchical namespace
            key: Memory key
            value: New memory content
            updated_by: Session or agent ID making the update
            task_id: Task context for audit logging

        Returns:
            ID of new memory version

        Raises:
            ValueError: If memory not found

        Example:
            >>> new_id = await memory_service.update_memory(
            ...     namespace="user:alice:preferences",
            ...     key="theme",
            ...     value={"mode": "light"},
            ...     updated_by="session:abc123",
            ...     task_id="task:xyz789"
            ... )
        """
        async with self.db._get_connection() as conn:
            try:
                async with conn.execute("BEGIN"):
                    # Get current version and type
                    cursor = await conn.execute(
                        """
                        SELECT MAX(version) as current_version, memory_type
                        FROM memory_entries
                        WHERE namespace = ? AND key = ? AND is_deleted = 0
                        """,
                        (namespace, key),
                    )
                    row = await cursor.fetchone()

                    if row is None or row["current_version"] is None:
                        raise ValueError(f"Memory not found: {namespace}:{key}")

                    new_version = row["current_version"] + 1
                    memory_type = row["memory_type"]

                    # Insert new version
                    cursor = await conn.execute(
                        """
                        INSERT INTO memory_entries (namespace, key, value, memory_type, version, updated_by)
                        VALUES (?, ?, ?, ?, ?, ?)
                        """,
                        (namespace, key, json.dumps(value), memory_type, new_version, updated_by),
                    )
                    memory_id = cursor.lastrowid
                    assert memory_id is not None, "Failed to get memory_id after update"

                    # Audit log
                    await conn.execute(
                        """
                        INSERT INTO audit (
                            timestamp, task_id, action_type, memory_operation_type,
                            memory_namespace, memory_entry_id, action_data
                        )
                        VALUES (CURRENT_TIMESTAMP, ?, 'memory_update', 'update', ?, ?, ?)
                        """,
                        (task_id, namespace, memory_id, json.dumps({"version": new_version})),
                    )

                    await conn.commit()
                    return int(memory_id)
            except Exception:
                await conn.rollback()
                raise

    async def delete_memory(self, namespace: str, key: str, task_id: str | None = None) -> bool:
        """Soft-delete memory entry.

        Args:
            namespace: Memory namespace
            key: Memory key
            task_id: Task context for audit logging

        Returns:
            True if memory was deleted, False if not found

        Example:
            >>> deleted = await memory_service.delete_memory(
            ...     namespace="user:alice:preferences",
            ...     key="theme",
            ...     task_id="task:cleanup"
            ... )
        """
        async with self.db._get_connection() as conn:
            try:
                async with conn.execute("BEGIN"):
                    cursor = await conn.execute(
                        """
                        UPDATE memory_entries
                        SET is_deleted = 1, updated_at = CURRENT_TIMESTAMP
                        WHERE namespace = ? AND key = ? AND is_deleted = 0
                        """,
                        (namespace, key),
                    )

                    rows_affected = cursor.rowcount

                    if rows_affected > 0:
                        # Audit log
                        await conn.execute(
                            """
                            INSERT INTO audit (
                                timestamp, task_id, action_type, memory_operation_type,
                                memory_namespace, action_data
                            )
                            VALUES (CURRENT_TIMESTAMP, ?, 'memory_delete', 'delete', ?, ?)
                            """,
                            (task_id, namespace, json.dumps({"key": key})),
                        )

                    await conn.commit()
                    return bool(rows_affected > 0)
            except Exception:
                await conn.rollback()
                raise

    async def search_memories(
        self, namespace_prefix: str, memory_type: str | None = None, limit: int = 50
    ) -> list[dict[str, Any]]:
        """Search memories by namespace prefix and optional type.

        Args:
            namespace_prefix: Namespace prefix (e.g., "user:alice" matches "user:alice:*")
            memory_type: Optional filter by type (semantic|episodic|procedural)
            limit: Maximum results to return

        Returns:
            List of memory entry dicts with parsed values

        Example:
            >>> memories = await memory_service.search_memories(
            ...     namespace_prefix="user:alice",
            ...     memory_type="semantic",
            ...     limit=20
            ... )
            >>> for mem in memories:
            ...     print(f"{mem['key']}: {mem['value']}")
        """
        async with self.db._get_connection() as conn:
            if memory_type:
                cursor = await conn.execute(
                    """
                    SELECT * FROM memory_entries
                    WHERE namespace LIKE ? AND memory_type = ? AND is_deleted = 0
                    ORDER BY updated_at DESC
                    LIMIT ?
                    """,
                    (f"{namespace_prefix}%", memory_type, limit),
                )
            else:
                cursor = await conn.execute(
                    """
                    SELECT * FROM memory_entries
                    WHERE namespace LIKE ? AND is_deleted = 0
                    ORDER BY updated_at DESC
                    LIMIT ?
                    """,
                    (f"{namespace_prefix}%", limit),
                )

            rows = await cursor.fetchall()

            # Parse JSON fields
            memories = []
            for row in rows:
                memory = dict(row)
                memory["value"] = json.loads(memory["value"])
                memory["metadata"] = json.loads(memory.get("metadata", "{}"))
                memories.append(memory)

            return memories

    async def list_namespaces(self) -> list[str]:
        """List all unique namespaces in memory entries.

        Returns:
            List of unique namespace strings

        Example:
            >>> namespaces = await memory_service.list_namespaces()
            >>> print(namespaces)  # ['user:alice:preferences', 'app:abathur:config', ...]
        """
        async with self.db._get_connection() as conn:
            cursor = await conn.execute(
                """
                SELECT DISTINCT namespace FROM memory_entries
                WHERE is_deleted = 0
                ORDER BY namespace
                """
            )
            rows = await cursor.fetchall()
            return [row["namespace"] for row in rows]

    async def get_memory_history(self, namespace: str, key: str) -> list[dict[str, Any]]:
        """Get all versions of a memory entry (audit trail).

        Args:
            namespace: Memory namespace
            key: Memory key

        Returns:
            List of all versions ordered by version DESC

        Example:
            >>> history = await memory_service.get_memory_history(
            ...     namespace="user:alice:preferences",
            ...     key="theme"
            ... )
            >>> for version in history:
            ...     print(f"v{version['version']}: {version['value']}")
        """
        async with self.db._get_connection() as conn:
            cursor = await conn.execute(
                """
                SELECT * FROM memory_entries
                WHERE namespace = ? AND key = ?
                ORDER BY version DESC
                """,
                (namespace, key),
            )
            rows = await cursor.fetchall()

            # Parse JSON fields
            history = []
            for row in rows:
                memory = dict(row)
                memory["value"] = json.loads(memory["value"])
                memory["metadata"] = json.loads(memory.get("metadata", "{}"))
                history.append(memory)

            return history

    async def cleanup_expired_memories(self, ttl_days: int = 90) -> int:
        """Cleanup expired episodic memories older than TTL.

        Args:
            ttl_days: Time-to-live in days for episodic memories

        Returns:
            Number of memories deleted

        Example:
            >>> count = await memory_service.cleanup_expired_memories(ttl_days=90)
            >>> print(f"Cleaned up {count} expired memories")
        """
        async with self.db._get_connection() as conn:
            try:
                async with conn.execute("BEGIN"):
                    cursor = await conn.execute(
                        """
                        UPDATE memory_entries
                        SET is_deleted = 1, updated_at = CURRENT_TIMESTAMP
                        WHERE memory_type = 'episodic'
                          AND is_deleted = 0
                          AND (julianday('now') - julianday(created_at)) > ?
                        """,
                        (ttl_days,),
                    )
                    rows_affected = cursor.rowcount
                    await conn.commit()
                    return int(rows_affected)
            except Exception:
                await conn.rollback()
                raise
