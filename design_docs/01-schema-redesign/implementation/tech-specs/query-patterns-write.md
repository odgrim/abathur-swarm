# Write Query Patterns - Transaction Safety and Performance

## Overview

This document specifies all write operations (INSERT, UPDATE, DELETE) with proper transaction handling, versioning patterns, and batch optimization strategies.

**Performance Target:** <20ms for single writes, <500ms for batch operations (100+ records)

**ACID Compliance:** All critical write operations use explicit transactions

---

## 1. Session Management Writes

### 1.1 Create New Session

**Use Case:** Initialize conversation session for task execution

**SQL Transaction:**
```sql
BEGIN TRANSACTION;

INSERT INTO sessions (
    id,
    app_name,
    user_id,
    project_id,
    status,
    events,
    state,
    created_at,
    last_update_time
) VALUES (?, ?, ?, ?, 'created', '[]', '{}', CURRENT_TIMESTAMP, CURRENT_TIMESTAMP);

COMMIT;
```

**Python Implementation:**
```python
async def create_session(
    self,
    session_id: str,
    app_name: str,
    user_id: str,
    project_id: Optional[str] = None
) -> None:
    """Create new session with initial state."""
    async with self._get_connection() as conn:
        async with conn.begin():  # Automatic transaction
            await conn.execute(
                """
                INSERT INTO sessions (id, app_name, user_id, project_id, status, events, state)
                VALUES (?, ?, ?, ?, 'created', '[]', '{}')
                """,
                (session_id, app_name, user_id, project_id)
            )
```

**Transaction Boundary:** Single INSERT, auto-commit safe

**Performance:** <10ms

---

### 1.2 Append Event to Session

**Use Case:** Record message, action, or tool call in conversation history

**SQL Transaction:**
```sql
BEGIN TRANSACTION;

-- Read current events
SELECT events, state FROM sessions WHERE id = ? FOR UPDATE;

-- Update with new event (in application code)
UPDATE sessions
SET events = ?,  -- JSON with appended event
    state = ?,   -- JSON with merged state_delta
    last_update_time = CURRENT_TIMESTAMP
WHERE id = ?;

COMMIT;
```

**Python Implementation:**
```python
async def append_event(
    self,
    session_id: str,
    event: Dict[str, Any],
    state_delta: Optional[Dict[str, Any]] = None
) -> None:
    """Append event to session with optional state update."""
    async with self._get_connection() as conn:
        async with conn.begin():
            # Read current events and state (lock row)
            cursor = await conn.execute(
                "SELECT events, state FROM sessions WHERE id = ? FOR UPDATE",
                (session_id,)
            )
            row = await cursor.fetchone()

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
```

**Transaction Boundary:** SELECT FOR UPDATE + UPDATE (prevents race conditions)

**Performance:** <20ms

**Concurrency Note:** FOR UPDATE locks row, serializes concurrent event appends

---

### 1.3 Update Session Status

**Use Case:** Transition session lifecycle (active → paused → terminated)

**SQL Transaction:**
```sql
BEGIN TRANSACTION;

UPDATE sessions
SET status = ?,
    last_update_time = CURRENT_TIMESTAMP,
    terminated_at = CASE WHEN ? = 'terminated' THEN CURRENT_TIMESTAMP ELSE terminated_at END
WHERE id = ?;

COMMIT;
```

**Python Implementation:**
```python
async def update_session_status(
    self,
    session_id: str,
    status: str
) -> None:
    """Update session status with timestamp tracking."""
    async with self._get_connection() as conn:
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
```

**Transaction Boundary:** Single UPDATE

**Performance:** <10ms

---

## 2. Memory Entry Writes

### 2.1 Create Memory Entry (Version 1)

**Use Case:** Store new semantic/episodic/procedural memory

**SQL Transaction:**
```sql
BEGIN TRANSACTION;

INSERT INTO memory_entries (
    namespace,
    key,
    value,
    memory_type,
    version,
    created_by,
    updated_by
) VALUES (?, ?, ?, ?, 1, ?, ?);

-- Audit logging
INSERT INTO audit (timestamp, task_id, action_type, memory_operation_type, memory_namespace, memory_entry_id, action_data)
VALUES (CURRENT_TIMESTAMP, ?, 'memory_create', 'create', ?, last_insert_rowid(), ?);

COMMIT;
```

**Python Implementation:**
```python
async def create_memory(
    self,
    namespace: str,
    key: str,
    value: Dict[str, Any],
    memory_type: str,
    created_by: str,
    task_id: str
) -> int:
    """Create new memory entry with audit logging."""
    async with self._get_connection() as conn:
        async with conn.begin():
            # Insert memory entry
            cursor = await conn.execute(
                """
                INSERT INTO memory_entries (namespace, key, value, memory_type, version, created_by, updated_by)
                VALUES (?, ?, ?, ?, 1, ?, ?)
                """,
                (namespace, key, json.dumps(value), memory_type, created_by, created_by)
            )
            memory_id = cursor.lastrowid

            # Audit log
            await conn.execute(
                """
                INSERT INTO audit (timestamp, task_id, action_type, memory_operation_type, memory_namespace, memory_entry_id, action_data)
                VALUES (CURRENT_TIMESTAMP, ?, 'memory_create', 'create', ?, ?, ?)
                """,
                (task_id, namespace, memory_id, json.dumps({"key": key, "memory_type": memory_type}))
            )

            return memory_id
```

**Transaction Boundary:** INSERT memory + INSERT audit (atomic)

**Performance:** <20ms (two INSERTs with index updates)

---

### 2.2 Update Memory Entry (Create New Version)

**Use Case:** Update existing memory with version increment

**SQL Transaction:**
```sql
BEGIN TRANSACTION;

-- Get current version
SELECT MAX(version) as current_version
FROM memory_entries
WHERE namespace = ? AND key = ? AND is_deleted = 0;

-- Insert new version
INSERT INTO memory_entries (namespace, key, value, memory_type, version, updated_by)
VALUES (?, ?, ?, ?, current_version + 1, ?);

-- Audit log
INSERT INTO audit (timestamp, task_id, action_type, memory_operation_type, memory_namespace, memory_entry_id, action_data)
VALUES (CURRENT_TIMESTAMP, ?, 'memory_update', 'update', ?, last_insert_rowid(), ?);

COMMIT;
```

**Python Implementation:**
```python
async def update_memory(
    self,
    namespace: str,
    key: str,
    value: Dict[str, Any],
    updated_by: str,
    task_id: str
) -> int:
    """Update memory by creating new version."""
    async with self._get_connection() as conn:
        async with conn.begin():
            # Get current version
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

            current_version = row['current_version']
            memory_type = row['memory_type']
            new_version = current_version + 1

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
                INSERT INTO audit (timestamp, task_id, action_type, memory_operation_type, memory_namespace, memory_entry_id, action_data)
                VALUES (CURRENT_TIMESTAMP, ?, 'memory_update', 'update', ?, ?, ?)
                """,
                (task_id, namespace, memory_id, json.dumps({"version": new_version}))
            )

            return memory_id
```

**Transaction Boundary:** SELECT + INSERT memory + INSERT audit (atomic)

**Performance:** <30ms (query + two INSERTs)

**Versioning Strategy:** Old versions remain in database for audit trail

---

### 2.3 Soft-Delete Memory Entry

**Use Case:** Mark memory as deleted without data loss

**SQL Transaction:**
```sql
BEGIN TRANSACTION;

UPDATE memory_entries
SET is_deleted = 1, updated_at = CURRENT_TIMESTAMP
WHERE namespace = ? AND key = ? AND is_deleted = 0;

-- Audit log
INSERT INTO audit (timestamp, task_id, action_type, memory_operation_type, memory_namespace, action_data)
VALUES (CURRENT_TIMESTAMP, ?, 'memory_delete', 'delete', ?, ?);

COMMIT;
```

**Python Implementation:**
```python
async def delete_memory(
    self,
    namespace: str,
    key: str,
    task_id: str
) -> None:
    """Soft-delete memory (set is_deleted flag)."""
    async with self._get_connection() as conn:
        async with conn.begin():
            await conn.execute(
                """
                UPDATE memory_entries
                SET is_deleted = 1, updated_at = CURRENT_TIMESTAMP
                WHERE namespace = ? AND key = ? AND is_deleted = 0
                """,
                (namespace, key)
            )

            # Audit log
            await conn.execute(
                """
                INSERT INTO audit (timestamp, task_id, action_type, memory_operation_type, memory_namespace, action_data)
                VALUES (CURRENT_TIMESTAMP, ?, 'memory_delete', 'delete', ?, ?)
                """,
                (task_id, namespace, json.dumps({"key": key}))
            )
```

**Transaction Boundary:** UPDATE + INSERT audit (atomic)

**Performance:** <15ms

**Rollback Strategy:** Set is_deleted=0 to restore

---

## 3. Batch Operations

### 3.1 Batch Insert Memories

**Use Case:** Seed initial memories, import from backup

**SQL Transaction:**
```sql
BEGIN TRANSACTION;

-- Use executemany for batch insert
INSERT INTO memory_entries (namespace, key, value, memory_type, version, created_by)
VALUES (?, ?, ?, ?, 1, ?);
-- Repeat for all entries...

COMMIT;
```

**Python Implementation:**
```python
async def batch_create_memories(
    self,
    memories: List[Dict[str, Any]],
    created_by: str
) -> List[int]:
    """Batch insert memories for performance."""
    async with self._get_connection() as conn:
        async with conn.begin():
            memory_ids = []

            # Use executemany for efficiency
            for memory in memories:
                cursor = await conn.execute(
                    """
                    INSERT INTO memory_entries (namespace, key, value, memory_type, version, created_by, updated_by)
                    VALUES (?, ?, ?, ?, 1, ?, ?)
                    """,
                    (
                        memory['namespace'],
                        memory['key'],
                        json.dumps(memory['value']),
                        memory['memory_type'],
                        created_by,
                        created_by
                    )
                )
                memory_ids.append(cursor.lastrowid)

            return memory_ids
```

**Performance:** <500ms for 100 memories (5ms per insert amortized)

**Optimization:** Consider using `executemany()` for even better performance

---

## 4. Document Index Writes

### 4.1 Index New Document

**Use Case:** Add markdown file to search index

**SQL Transaction:**
```sql
BEGIN TRANSACTION;

INSERT INTO document_index (
    file_path,
    title,
    document_type,
    content_hash,
    metadata,
    sync_status
) VALUES (?, ?, ?, ?, ?, 'pending');

COMMIT;
```

**Python Implementation:**
```python
import hashlib

async def index_document(
    self,
    file_path: str,
    title: str,
    document_type: str,
    content: str,
    metadata: Dict[str, Any]
) -> int:
    """Index new document with content hash."""
    content_hash = hashlib.sha256(content.encode()).hexdigest()

    async with self._get_connection() as conn:
        async with conn.begin():
            cursor = await conn.execute(
                """
                INSERT INTO document_index (file_path, title, document_type, content_hash, metadata, sync_status)
                VALUES (?, ?, ?, ?, ?, 'pending')
                """,
                (file_path, title, document_type, content_hash, json.dumps(metadata))
            )
            return cursor.lastrowid
```

**Performance:** <10ms

---

### 4.2 Update Document (Content Changed)

**Use Case:** File watcher detects markdown file modification

**SQL Transaction:**
```sql
BEGIN TRANSACTION;

UPDATE document_index
SET content_hash = ?,
    sync_status = 'stale',
    updated_at = CURRENT_TIMESTAMP
WHERE file_path = ?;

COMMIT;
```

**Python Implementation:**
```python
async def mark_document_stale(
    self,
    file_path: str,
    new_content: str
) -> None:
    """Mark document as stale when content changes."""
    new_hash = hashlib.sha256(new_content.encode()).hexdigest()

    async with self._get_connection() as conn:
        async with conn.begin():
            await conn.execute(
                """
                UPDATE document_index
                SET content_hash = ?, sync_status = 'stale', updated_at = CURRENT_TIMESTAMP
                WHERE file_path = ?
                """,
                (new_hash, file_path)
            )
```

**Performance:** <10ms

---

## 5. Transaction Best Practices

### 5.1 Explicit Transaction Boundaries

**Use async context managers:**
```python
async with conn.begin():
    # All operations here are atomic
    await conn.execute(...)
    await conn.execute(...)
    # Auto-commit on context exit
    # Auto-rollback on exception
```

### 5.2 Error Handling

**Rollback on error:**
```python
try:
    async with conn.begin():
        await conn.execute("INSERT ...")
        await conn.execute("UPDATE ...")
except Exception as e:
    # Transaction automatically rolled back
    logger.error(f"Transaction failed: {e}")
    raise
```

### 5.3 Avoiding Deadlocks

**Lock order consistency:**
- Always lock tables in same order: sessions → memory_entries → audit
- Use FOR UPDATE sparingly
- Keep transactions short

---

## 6. Performance Optimization Strategies

### Write Performance Tips

1. **Batch Operations:** Use `executemany()` for bulk inserts
2. **Reduce Index Overhead:** Partial indexes only index active records
3. **WAL Mode:** Concurrent reads don't block writes
4. **Async Processing:** Defer audit logging to background tasks if non-critical
5. **Prepared Statements:** Reuse compiled SQL for repeated operations

---

**Document Version:** 1.0
**Author:** technical-specifications-writer
**Date:** 2025-10-10
**Related Files:** `query-patterns-read.md`, `api-specifications.md`, `ddl-indexes.sql`
