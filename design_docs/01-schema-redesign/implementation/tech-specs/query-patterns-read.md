# Read Query Patterns - Optimized for <50ms Performance

## Overview

This document specifies all common read query patterns with EXPLAIN QUERY PLAN analysis, performance characteristics, and prepared statement templates for Python integration.

**Performance Target:** <50ms for exact-match reads, <500ms for semantic search (post-sqlite-vss)

**Index Dependency:** All queries assume indexes from `ddl-indexes.sql` are created

---

## 1. Session Retrieval Queries

### 1.1 Get Session by ID

**Use Case:** Retrieve session for task execution context

**SQL Query:**
```sql
SELECT * FROM sessions
WHERE id = ?;
```

**Prepared Statement (Python):**
```python
cursor = await conn.execute(
    "SELECT * FROM sessions WHERE id = ?",
    (session_id,)
)
session_row = await cursor.fetchone()
```

**EXPLAIN QUERY PLAN:**
```
SEARCH TABLE sessions USING INDEX idx_sessions_pk (id=?)
```

**Performance:**
- Time Complexity: O(log n) - B-tree index lookup
- Expected Latency: <5ms (single-row primary key lookup)
- Index Used: `idx_sessions_pk` (PRIMARY KEY automatic index)

---

### 1.2 Get Active Sessions for User

**Use Case:** List user's recent active/paused sessions

**SQL Query:**
```sql
SELECT * FROM sessions
WHERE user_id = ?
  AND status IN ('active', 'paused')
ORDER BY last_update_time DESC
LIMIT 10;
```

**Prepared Statement (Python):**
```python
cursor = await conn.execute(
    """
    SELECT * FROM sessions
    WHERE user_id = ? AND status IN ('active', 'paused')
    ORDER BY last_update_time DESC
    LIMIT 10
    """,
    (user_id,)
)
sessions = await cursor.fetchall()
```

**EXPLAIN QUERY PLAN:**
```
SEARCH TABLE sessions USING INDEX idx_sessions_user_created (user_id=?)
USE TEMP B-TREE FOR ORDER BY
```

**Performance:**
- Time Complexity: O(log n + k) where k=10 results
- Expected Latency: <20ms (index scan + sort)
- Index Used: `idx_sessions_user_created`

**Optimization Note:** If filtering by status is critical, consider composite index on (user_id, status, last_update_time).

---

### 1.3 Get All Sessions for Project

**Use Case:** Cross-agent collaboration - find all sessions in project

**SQL Query:**
```sql
SELECT * FROM sessions
WHERE project_id = ?
ORDER BY created_at DESC
LIMIT 50;
```

**Prepared Statement (Python):**
```python
cursor = await conn.execute(
    """
    SELECT * FROM sessions
    WHERE project_id = ?
    ORDER BY created_at DESC
    LIMIT 50
    """,
    (project_id,)
)
project_sessions = await cursor.fetchall()
```

**EXPLAIN QUERY PLAN:**
```
SEARCH TABLE sessions USING INDEX idx_sessions_project (project_id=?)
```

**Performance:**
- Time Complexity: O(log n + k) where k=50 results
- Expected Latency: <30ms
- Index Used: `idx_sessions_project` (partial index, excludes NULL project_id)

---

## 2. Memory Retrieval Queries

### 2.1 Get Current Version of Memory Entry

**Use Case:** Retrieve latest version of specific memory (most common operation)

**SQL Query:**
```sql
SELECT * FROM memory_entries
WHERE namespace = ?
  AND key = ?
  AND is_deleted = 0
ORDER BY version DESC
LIMIT 1;
```

**Prepared Statement (Python):**
```python
cursor = await conn.execute(
    """
    SELECT * FROM memory_entries
    WHERE namespace = ? AND key = ? AND is_deleted = 0
    ORDER BY version DESC
    LIMIT 1
    """,
    (namespace, key)
)
memory = await cursor.fetchone()
```

**EXPLAIN QUERY PLAN:**
```
SEARCH TABLE memory_entries USING INDEX idx_memory_namespace_key_version (namespace=? AND key=? AND is_deleted=?)
```

**Performance:**
- Time Complexity: O(log n) - covering index with early termination
- Expected Latency: <10ms
- Index Used: `idx_memory_namespace_key_version` (CRITICAL index)

**Optimization Note:** This is a covering index - all columns in SELECT are in the index, no table lookup required.

---

### 2.2 Get All Active Memories in Namespace

**Use Case:** Retrieve all memories for user, project, or app

**SQL Query:**
```sql
SELECT * FROM memory_entries
WHERE namespace LIKE ?  -- e.g., 'user:alice:%'
  AND is_deleted = 0
ORDER BY updated_at DESC;
```

**Prepared Statement (Python):**
```python
# Namespace prefix search
namespace_prefix = f"{namespace_base}:%"
cursor = await conn.execute(
    """
    SELECT * FROM memory_entries
    WHERE namespace LIKE ? AND is_deleted = 0
    ORDER BY updated_at DESC
    """,
    (namespace_prefix,)
)
memories = await cursor.fetchall()
```

**EXPLAIN QUERY PLAN:**
```
SEARCH TABLE memory_entries USING INDEX idx_memory_namespace_prefix (namespace>? AND namespace<?)
```

**Performance:**
- Time Complexity: O(m log n) where m = result count
- Expected Latency: <50ms for 100 results
- Index Used: `idx_memory_namespace_prefix`

**Important:** LIKE with prefix (`prefix%`) can use index. LIKE with suffix (`%suffix`) cannot.

---

### 2.3 Get Memories by Type (Semantic, Episodic, Procedural)

**Use Case:** Retrieve all semantic memories, or episodic history, or procedural instructions

**SQL Query:**
```sql
SELECT * FROM memory_entries
WHERE memory_type = ?
  AND is_deleted = 0
ORDER BY updated_at DESC
LIMIT 50;
```

**Prepared Statement (Python):**
```python
cursor = await conn.execute(
    """
    SELECT * FROM memory_entries
    WHERE memory_type = ? AND is_deleted = 0
    ORDER BY updated_at DESC
    LIMIT 50
    """,
    (memory_type,)  # 'semantic', 'episodic', or 'procedural'
)
memories = await cursor.fetchall()
```

**EXPLAIN QUERY PLAN:**
```
SEARCH TABLE memory_entries USING INDEX idx_memory_type_updated (memory_type=?)
```

**Performance:**
- Time Complexity: O(log n + k) where k=50
- Expected Latency: <30ms
- Index Used: `idx_memory_type_updated` (partial index, is_deleted=0 filter)

---

### 2.4 Hierarchical Memory Retrieval (Context Synthesis)

**Use Case:** Agent needs context from all accessible namespaces (temp, session, user, app, project)

**SQL Query:**
```sql
SELECT * FROM memory_entries
WHERE (
    namespace LIKE ?  -- 'session:abc123:%'
    OR namespace LIKE ?  -- 'user:alice:%'
    OR namespace LIKE ?  -- 'app:abathur:%'
    OR namespace LIKE ?  -- 'project:schema_redesign:%'
)
AND is_deleted = 0
ORDER BY namespace, version DESC;
```

**Prepared Statement (Python):**
```python
# Context synthesis for agent
cursor = await conn.execute(
    """
    SELECT * FROM memory_entries
    WHERE (
        namespace LIKE ? OR
        namespace LIKE ? OR
        namespace LIKE ? OR
        namespace LIKE ?
    )
    AND is_deleted = 0
    ORDER BY namespace, version DESC
    """,
    (
        f"session:{session_id}:%",
        f"user:{user_id}:%",
        f"app:{app_name}:%",
        f"project:{project_id}:%"
    )
)
context_memories = await cursor.fetchall()
```

**EXPLAIN QUERY PLAN:**
```
MULTI-INDEX OR
  SEARCH TABLE memory_entries USING INDEX idx_memory_namespace_prefix (namespace>? AND namespace<?)
  SEARCH TABLE memory_entries USING INDEX idx_memory_namespace_prefix (namespace>? AND namespace<?)
  SEARCH TABLE memory_entries USING INDEX idx_memory_namespace_prefix (namespace>? AND namespace<?)
  SEARCH TABLE memory_entries USING INDEX idx_memory_namespace_prefix (namespace>? AND namespace<?)
USE TEMP B-TREE FOR ORDER BY
```

**Performance:**
- Time Complexity: O(m * log n) where m = number of OR clauses
- Expected Latency: <100ms for 200 total results
- Index Used: `idx_memory_namespace_prefix` (multiple times)

**Optimization Note:** For very frequent context synthesis, consider caching results or using a materialized view.

---

### 2.5 Get Memory Version History

**Use Case:** Audit trail or rollback - retrieve all versions of a memory

**SQL Query:**
```sql
SELECT * FROM memory_entries
WHERE namespace = ?
  AND key = ?
ORDER BY version DESC;
```

**Prepared Statement (Python):**
```python
cursor = await conn.execute(
    """
    SELECT * FROM memory_entries
    WHERE namespace = ? AND key = ?
    ORDER BY version DESC
    """,
    (namespace, key)
)
versions = await cursor.fetchall()
```

**EXPLAIN QUERY PLAN:**
```
SEARCH TABLE memory_entries USING INDEX idx_memory_namespace_key_version (namespace=? AND key=?)
```

**Performance:**
- Time Complexity: O(log n + v) where v = version count
- Expected Latency: <20ms for 10 versions
- Index Used: `idx_memory_namespace_key_version`

---

## 3. Document Index Queries

### 3.1 Get Document by File Path

**Use Case:** Check if document already indexed, retrieve embedding status

**SQL Query:**
```sql
SELECT * FROM document_index
WHERE file_path = ?;
```

**Prepared Statement (Python):**
```python
cursor = await conn.execute(
    "SELECT * FROM document_index WHERE file_path = ?",
    (file_path,)
)
document = await cursor.fetchone()
```

**EXPLAIN QUERY PLAN:**
```
SEARCH TABLE document_index USING INDEX idx_document_file_path (file_path=?)
```

**Performance:**
- Time Complexity: O(log n)
- Expected Latency: <5ms (unique index)
- Index Used: `idx_document_file_path` (UNIQUE)

---

### 3.2 Get Documents Needing Embedding Sync

**Use Case:** Background sync service finds documents with pending/stale embeddings

**SQL Query:**
```sql
SELECT * FROM document_index
WHERE sync_status IN ('pending', 'stale')
ORDER BY created_at ASC
LIMIT 100;
```

**Prepared Statement (Python):**
```python
cursor = await conn.execute(
    """
    SELECT * FROM document_index
    WHERE sync_status IN ('pending', 'stale')
    ORDER BY created_at ASC
    LIMIT 100
    """,
)
pending_docs = await cursor.fetchall()
```

**EXPLAIN QUERY PLAN:**
```
SEARCH TABLE document_index USING INDEX idx_document_sync_status (sync_status=? OR sync_status=?)
```

**Performance:**
- Time Complexity: O(log n + k) where k=100
- Expected Latency: <30ms
- Index Used: `idx_document_sync_status` (partial index)

---

### 3.3 Search Documents by Type

**Use Case:** Find all design documents, or specifications, or plans

**SQL Query:**
```sql
SELECT * FROM document_index
WHERE document_type = ?
ORDER BY created_at DESC
LIMIT 50;
```

**Prepared Statement (Python):**
```python
cursor = await conn.execute(
    """
    SELECT * FROM document_index
    WHERE document_type = ?
    ORDER BY created_at DESC
    LIMIT 50
    """,
    (document_type,)  # 'design', 'specification', 'plan', 'report'
)
documents = await cursor.fetchall()
```

**EXPLAIN QUERY PLAN:**
```
SEARCH TABLE document_index USING INDEX idx_document_type_created (document_type=?)
```

**Performance:**
- Time Complexity: O(log n + k) where k=50
- Expected Latency: <25ms
- Index Used: `idx_document_type_created`

---

## 4. Task Queries with Session Context

### 4.1 Get Task with Session Details (JOIN)

**Use Case:** Retrieve task with full session context for execution

**SQL Query:**
```sql
SELECT
    t.*,
    s.user_id,
    s.project_id,
    s.state as session_state,
    s.events as session_events
FROM tasks t
LEFT JOIN sessions s ON t.session_id = s.id
WHERE t.id = ?;
```

**Prepared Statement (Python):**
```python
cursor = await conn.execute(
    """
    SELECT
        t.*,
        s.user_id,
        s.project_id,
        s.state as session_state,
        s.events as session_events
    FROM tasks t
    LEFT JOIN sessions s ON t.session_id = s.id
    WHERE t.id = ?
    """,
    (task_id,)
)
task_with_session = await cursor.fetchone()
```

**EXPLAIN QUERY PLAN:**
```
SEARCH TABLE tasks USING INDEX sqlite_autoindex_tasks_1 (id=?)
SEARCH TABLE sessions USING INDEX idx_sessions_pk (id=?)
```

**Performance:**
- Time Complexity: O(log n) for each table
- Expected Latency: <15ms (two index lookups + join)
- Indexes Used: tasks PK + sessions PK

---

### 4.2 Get All Tasks in Session

**Use Case:** Session completion - find all tasks executed in session

**SQL Query:**
```sql
SELECT * FROM tasks
WHERE session_id = ?
ORDER BY submitted_at DESC;
```

**Prepared Statement (Python):**
```python
cursor = await conn.execute(
    """
    SELECT * FROM tasks
    WHERE session_id = ?
    ORDER BY submitted_at DESC
    """,
    (session_id,)
)
session_tasks = await cursor.fetchall()
```

**EXPLAIN QUERY PLAN:**
```
SEARCH TABLE tasks USING INDEX idx_tasks_session (session_id=?)
```

**Performance:**
- Time Complexity: O(log n + k) where k = task count
- Expected Latency: <20ms for 10 tasks
- Index Used: `idx_tasks_session`

---

## 5. Audit Trail Queries

### 5.1 Get Memory Operation Audit Trail

**Use Case:** Compliance - track all memory modifications

**SQL Query:**
```sql
SELECT * FROM audit
WHERE memory_operation_type IS NOT NULL
  AND timestamp > ?
ORDER BY timestamp DESC
LIMIT 100;
```

**Prepared Statement (Python):**
```python
cursor = await conn.execute(
    """
    SELECT * FROM audit
    WHERE memory_operation_type IS NOT NULL
      AND timestamp > ?
    ORDER BY timestamp DESC
    LIMIT 100
    """,
    (start_timestamp,)
)
memory_audit = await cursor.fetchall()
```

**EXPLAIN QUERY PLAN:**
```
SEARCH TABLE audit USING INDEX idx_audit_memory_operations (memory_operation_type>?)
```

**Performance:**
- Time Complexity: O(log n + k) where k=100
- Expected Latency: <40ms
- Index Used: `idx_audit_memory_operations` (partial index)

---

### 5.2 Get Audit Trail for Namespace

**Use Case:** Investigate changes to specific user or project namespace

**SQL Query:**
```sql
SELECT * FROM audit
WHERE memory_namespace LIKE ?
ORDER BY timestamp DESC
LIMIT 50;
```

**Prepared Statement (Python):**
```python
cursor = await conn.execute(
    """
    SELECT * FROM audit
    WHERE memory_namespace LIKE ?
    ORDER BY timestamp DESC
    LIMIT 50
    """,
    (f"{namespace_prefix}%",)
)
namespace_audit = await cursor.fetchall()
```

**EXPLAIN QUERY PLAN:**
```
SEARCH TABLE audit USING INDEX idx_audit_memory_namespace (memory_namespace>? AND memory_namespace<?)
```

**Performance:**
- Time Complexity: O(log n + k) where k=50
- Expected Latency: <35ms
- Index Used: `idx_audit_memory_namespace`

---

## 6. Semantic Search Queries (Future - sqlite-vss)

### 6.1 Vector Similarity Search

**Use Case:** Find documents similar to query embedding (post-infrastructure)

**SQL Query (Future):**
```sql
-- Requires sqlite-vss extension
SELECT
    di.*,
    vss_distance(di.embedding_blob, ?) as distance
FROM document_index di
WHERE di.sync_status = 'synced'
  AND di.embedding_model = 'nomic-embed-text-v1.5'
ORDER BY distance ASC
LIMIT 10;
```

**Prepared Statement (Python - Future):**
```python
# Generate query embedding via Ollama
query_embedding = await ollama_client.embed(
    model="nomic-embed-text-v1.5",
    text=query_text
)

# Search similar documents
cursor = await conn.execute(
    """
    SELECT
        di.*,
        vss_distance(di.embedding_blob, ?) as distance
    FROM document_index di
    WHERE di.sync_status = 'synced'
      AND di.embedding_model = 'nomic-embed-text-v1.5'
    ORDER BY distance ASC
    LIMIT 10
    """,
    (query_embedding,)
)
similar_docs = await cursor.fetchall()
```

**Performance Target:**
- Time Complexity: O(n) for brute-force, O(log n) with HNSW index
- Expected Latency: <500ms for 1,000 documents
- Requires: sqlite-vss extension + vector index

**See:** `sqlite-vss-integration.md` for complete setup guide

---

## 7. Performance Validation

### Verifying Index Usage

**Check if query uses index:**
```sql
EXPLAIN QUERY PLAN
SELECT * FROM memory_entries
WHERE namespace = 'user:alice' AND is_deleted = 0
ORDER BY version DESC LIMIT 1;
```

**Good Output (index used):**
```
SEARCH TABLE memory_entries USING INDEX idx_memory_namespace_key_version (namespace=? AND is_deleted=?)
```

**Bad Output (full table scan):**
```
SCAN TABLE memory_entries
```

**Fixing Missing Indexes:**
- If SCAN appears, check that indexes from `ddl-indexes.sql` are created
- Run `ANALYZE;` to update query optimizer statistics
- Consider adding covering index if table lookups are slow

---

## 8. Query Optimization Checklist

**For Read Performance:**
- [ ] All WHERE clauses use indexed columns
- [ ] EXPLAIN QUERY PLAN shows index usage (no SCAN)
- [ ] Composite indexes match query column order
- [ ] LIMIT used to cap result set size
- [ ] Partial indexes used for filtered queries (is_deleted=0)

**For Joins:**
- [ ] JOIN columns have indexes on both tables
- [ ] LEFT JOIN used when one side might be NULL
- [ ] INNER JOIN used when both sides required

**For Sorting:**
- [ ] ORDER BY columns in index (avoid TEMP B-TREE)
- [ ] DESC specified in index definition if sorting descending

---

**Document Version:** 1.0
**Author:** technical-specifications-writer
**Date:** 2025-10-10
**Related Files:** `ddl-indexes.sql`, `query-patterns-write.md`, `api-specifications.md`
