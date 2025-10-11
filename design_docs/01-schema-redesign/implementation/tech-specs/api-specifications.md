# API Specifications - Python Memory Management Services

## Overview

This document specifies complete Python APIs for memory management, session handling, and document indexing. All APIs include type annotations, comprehensive docstrings, error handling, and example usage.

**Integration Strategy:** Extend existing `Database` class with new service classes

**Dependencies:** Python 3.11+, aiosqlite, existing `abathur.infrastructure.database.Database`

---

## 1. SessionService Class

### Purpose

Manage conversation sessions with event tracking and state management.

### Class Definition

```python
from typing import Any, Dict, List, Optional
from uuid import UUID
from datetime import datetime
import json
import aiosqlite

class SessionService:
    """Service for managing conversation sessions with events and state.

    Provides CRUD operations for sessions, event appending, and state management
    with hierarchical namespace support (session:, temp:, user:, app:, project:).
    """

    def __init__(self, db: "Database") -> None:
        """Initialize session service.

        Args:
            db: Database instance for storage operations
        """
        self.db = db

    async def create_session(
        self,
        session_id: str,
        app_name: str,
        user_id: str,
        project_id: Optional[str] = None,
        initial_state: Optional[Dict[str, Any]] = None
    ) -> None:
        """Create new session with optional initial state.

        Args:
            session_id: UUID or unique session identifier
            app_name: Application context (e.g., "abathur")
            user_id: User identifier
            project_id: Optional project association for cross-agent collaboration
            initial_state: Optional initial state dictionary with namespace prefixes

        Raises:
            ValueError: If session_id already exists

        Example:
            >>> await session_service.create_session(
            ...     session_id="abc123",
            ...     app_name="abathur",
            ...     user_id="alice",
            ...     project_id="schema_redesign",
            ...     initial_state={"user:alice:theme": "dark"}
            ... )
        """
        state_json = json.dumps(initial_state or {})

        async with self.db._get_connection() as conn:
            async with conn.begin():
                try:
                    await conn.execute(
                        """
                        INSERT INTO sessions (id, app_name, user_id, project_id, status, events, state)
                        VALUES (?, ?, ?, ?, 'created', '[]', ?)
                        """,
                        (session_id, app_name, user_id, project_id, state_json)
                    )
                except aiosqlite.IntegrityError as e:
                    raise ValueError(f"Session {session_id} already exists") from e

    async def get_session(self, session_id: str) -> Optional[Dict[str, Any]]:
        """Retrieve session by ID with parsed JSON fields.

        Args:
            session_id: Session identifier

        Returns:
            Dictionary with session fields (events and state parsed as dicts/lists),
            or None if session not found

        Example:
            >>> session = await session_service.get_session("abc123")
            >>> print(session['status'])  # 'active'
            >>> print(session['events'][0]['event_type'])  # 'message'
        """
        async with self.db._get_connection() as conn:
            cursor = await conn.execute(
                "SELECT * FROM sessions WHERE id = ?",
                (session_id,)
            )
            row = await cursor.fetchone()

            if row is None:
                return None

            # Convert to dict and parse JSON fields
            session = dict(row)
            session['events'] = json.loads(session['events'])
            session['state'] = json.loads(session['state'])
            session['metadata'] = json.loads(session.get('metadata', '{}'))

            return session

    async def append_event(
        self,
        session_id: str,
        event: Dict[str, Any],
        state_delta: Optional[Dict[str, Any]] = None
    ) -> None:
        """Append event to session with optional state update.

        Args:
            session_id: Session identifier
            event: Event dictionary with keys:
                - event_id: str (unique event identifier)
                - timestamp: str (ISO 8601 timestamp)
                - event_type: str (message|action|tool_call|reflection)
                - actor: str (user|agent:<agent_id>|system)
                - content: dict (event-specific data)
                - is_final_response: bool
            state_delta: Optional state changes to merge into session state

        Raises:
            ValueError: If session not found

        Example:
            >>> await session_service.append_event(
            ...     session_id="abc123",
            ...     event={
            ...         "event_id": "evt_001",
            ...         "timestamp": "2025-10-10T10:00:00Z",
            ...         "event_type": "message",
            ...         "actor": "user",
            ...         "content": {"message": "Design the schema"},
            ...         "is_final_response": False
            ...     },
            ...     state_delta={"session:abc123:current_task": "schema_design"}
            ... )
        """
        async with self.db._get_connection() as conn:
            async with conn.begin():
                # Read current events and state (lock row)
                cursor = await conn.execute(
                    "SELECT events, state FROM sessions WHERE id = ? FOR UPDATE",
                    (session_id,)
                )
                row = await cursor.fetchone()

                if row is None:
                    raise ValueError(f"Session {session_id} not found")

                # Parse JSON
                events = json.loads(row['events'])
                state = json.loads(row['state'])

                # Append event
                events.append(event)

                # Merge state delta if provided
                if state_delta:
                    state.update(state_delta)

                # Update session
                await conn.execute(
                    """
                    UPDATE sessions
                    SET events = ?, state = ?, last_update_time = CURRENT_TIMESTAMP
                    WHERE id = ?
                    """,
                    (json.dumps(events), json.dumps(state), session_id)
                )

    async def update_status(
        self,
        session_id: str,
        status: str
    ) -> None:
        """Update session lifecycle status.

        Args:
            session_id: Session identifier
            status: New status (created|active|paused|terminated|archived)

        Raises:
            ValueError: If status is invalid or session not found

        Example:
            >>> await session_service.update_status("abc123", "active")
        """
        valid_statuses = {'created', 'active', 'paused', 'terminated', 'archived'}
        if status not in valid_statuses:
            raise ValueError(f"Invalid status: {status}. Must be one of {valid_statuses}")

        async with self.db._get_connection() as conn:
            async with conn.begin():
                if status == 'terminated':
                    await conn.execute(
                        """
                        UPDATE sessions
                        SET status = ?, terminated_at = CURRENT_TIMESTAMP, last_update_time = CURRENT_TIMESTAMP
                        WHERE id = ?
                        """,
                        (status, session_id)
                    )
                else:
                    await conn.execute(
                        """
                        UPDATE sessions
                        SET status = ?, last_update_time = CURRENT_TIMESTAMP
                        WHERE id = ?
                        """,
                        (status, session_id)
                    )

    async def get_state(
        self,
        session_id: str,
        key: str
    ) -> Optional[Any]:
        """Get specific state value from session.

        Args:
            session_id: Session identifier
            key: State key (with namespace prefix, e.g., "user:alice:theme")

        Returns:
            State value if key exists, None otherwise

        Example:
            >>> theme = await session_service.get_state("abc123", "user:alice:theme")
            >>> print(theme)  # "dark"
        """
        session = await self.get_session(session_id)
        if session is None:
            return None

        return session['state'].get(key)

    async def set_state(
        self,
        session_id: str,
        key: str,
        value: Any
    ) -> None:
        """Set specific state value in session.

        Args:
            session_id: Session identifier
            key: State key (with namespace prefix)
            value: State value (must be JSON-serializable)

        Example:
            >>> await session_service.set_state("abc123", "user:alice:theme", "dark")
        """
        async with self.db._get_connection() as conn:
            async with conn.begin():
                cursor = await conn.execute(
                    "SELECT state FROM sessions WHERE id = ? FOR UPDATE",
                    (session_id,)
                )
                row = await cursor.fetchone()

                if row is None:
                    raise ValueError(f"Session {session_id} not found")

                state = json.loads(row['state'])
                state[key] = value

                await conn.execute(
                    """
                    UPDATE sessions
                    SET state = ?, last_update_time = CURRENT_TIMESTAMP
                    WHERE id = ?
                    """,
                    (json.dumps(state), session_id)
                )
```

---

## 2. MemoryService Class

### Purpose

Manage long-term persistent memory with versioning, namespace hierarchy, and semantic/episodic/procedural types.

### Class Definition

```python
class MemoryService:
    """Service for managing long-term memory storage and retrieval.

    Supports semantic memory (facts), episodic memory (experiences),
    and procedural memory (rules/instructions) with hierarchical
    namespace organization.
    """

    def __init__(self, db: "Database") -> None:
        """Initialize memory service.

        Args:
            db: Database instance for storage operations
        """
        self.db = db

    async def add_memory(
        self,
        namespace: str,
        key: str,
        value: Dict[str, Any],
        memory_type: str,
        created_by: str,
        task_id: str,
        metadata: Optional[Dict[str, Any]] = None
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
        valid_types = {'semantic', 'episodic', 'procedural'}
        if memory_type not in valid_types:
            raise ValueError(f"Invalid memory_type: {memory_type}. Must be one of {valid_types}")

        # Validate namespace format (basic check)
        if ':' not in namespace:
            raise ValueError(f"Invalid namespace format: {namespace}. Must contain ':' separator")

        async with self.db._get_connection() as conn:
            async with conn.begin():
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
                        created_by
                    )
                )
                memory_id = cursor.lastrowid

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
                        json.dumps({"key": key, "memory_type": memory_type})
                    )
                )

                return memory_id

    async def get_memory(
        self,
        namespace: str,
        key: str,
        version: Optional[int] = None
    ) -> Optional[Dict[str, Any]]:
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
                    (namespace, key)
                )
            else:
                # Get specific version
                cursor = await conn.execute(
                    """
                    SELECT * FROM memory_entries
                    WHERE namespace = ? AND key = ? AND version = ?
                    """,
                    (namespace, key, version)
                )

            row = await cursor.fetchone()
            if row is None:
                return None

            # Parse JSON fields
            memory = dict(row)
            memory['value'] = json.loads(memory['value'])
            memory['metadata'] = json.loads(memory.get('metadata', '{}'))

            return memory

    async def update_memory(
        self,
        namespace: str,
        key: str,
        value: Dict[str, Any],
        updated_by: str,
        task_id: str
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
            async with conn.begin():
                # Get current version and type
                cursor = await conn.execute(
                    """
                    SELECT MAX(version) as current_version, memory_type
                    FROM memory_entries
                    WHERE namespace = ? AND key = ? AND is_deleted = 0
                    """,
                    (namespace, key)
                )
                row = await cursor.fetchone()

                if row is None or row['current_version'] is None:
                    raise ValueError(f"Memory not found: {namespace}:{key}")

                new_version = row['current_version'] + 1
                memory_type = row['memory_type']

                # Insert new version
                cursor = await conn.execute(
                    """
                    INSERT INTO memory_entries (namespace, key, value, memory_type, version, updated_by)
                    VALUES (?, ?, ?, ?, ?, ?)
                    """,
                    (namespace, key, json.dumps(value), memory_type, new_version, updated_by)
                )
                memory_id = cursor.lastrowid

                # Audit log
                await conn.execute(
                    """
                    INSERT INTO audit (
                        timestamp, task_id, action_type, memory_operation_type,
                        memory_namespace, memory_entry_id, action_data
                    )
                    VALUES (CURRENT_TIMESTAMP, ?, 'memory_update', 'update', ?, ?, ?)
                    """,
                    (task_id, namespace, memory_id, json.dumps({"version": new_version}))
                )

                return memory_id

    async def search_memories(
        self,
        namespace_prefix: str,
        memory_type: Optional[str] = None,
        limit: int = 50
    ) -> List[Dict[str, Any]]:
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
                    (f"{namespace_prefix}%", memory_type, limit)
                )
            else:
                cursor = await conn.execute(
                    """
                    SELECT * FROM memory_entries
                    WHERE namespace LIKE ? AND is_deleted = 0
                    ORDER BY updated_at DESC
                    LIMIT ?
                    """,
                    (f"{namespace_prefix}%", limit)
                )

            rows = await cursor.fetchall()

            # Parse JSON fields
            memories = []
            for row in rows:
                memory = dict(row)
                memory['value'] = json.loads(memory['value'])
                memory['metadata'] = json.loads(memory.get('metadata', '{}'))
                memories.append(memory)

            return memories
```

---

## 3. Database Class Integration

### Enhanced Database Class

Add these methods to existing `Database` class:

```python
# In abathur/infrastructure/database.py

class Database:
    # ... existing methods ...

    def __init__(self, db_path: Path) -> None:
        self.db_path = db_path
        self._initialized = False

        # NEW: Initialize services
        self.sessions = SessionService(self)
        self.memory = MemoryService(self)

    async def _create_tables(self, conn: Connection) -> None:
        """Create all database tables (enhanced with new tables)."""
        # Execute ddl-memory-tables.sql
        # Execute ddl-core-tables.sql
        # Execute ddl-indexes.sql
        pass  # See implementation-guide.md for complete DDL execution
```

### Usage Example

```python
from abathur.infrastructure.database import Database
from pathlib import Path

# Initialize database
db = Database(Path("abathur.db"))
await db.initialize()

# Create session
await db.sessions.create_session(
    session_id="abc123",
    app_name="abathur",
    user_id="alice",
    project_id="schema_redesign"
)

# Add memory
memory_id = await db.memory.add_memory(
    namespace="user:alice:preferences",
    key="theme",
    value={"mode": "dark"},
    memory_type="semantic",
    created_by="session:abc123",
    task_id="task:xyz789"
)

# Retrieve memory
memory = await db.memory.get_memory("user:alice:preferences", "theme")
print(memory['value'])  # {"mode": "dark"}
```

---

**Document Version:** 1.0
**Author:** technical-specifications-writer
**Date:** 2025-10-10
**Related Files:** `query-patterns-read.md`, `query-patterns-write.md`, `test-scenarios.md`
