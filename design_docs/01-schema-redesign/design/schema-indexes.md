# Schema Indexes - Performance Optimization

## 1. Index Design Philosophy

**Goals:**
- Achieve <50ms read latency for exact-match queries
- Support <500ms semantic search (post-sqlite-vss integration)
- Optimize for hierarchical namespace queries
- Enable efficient memory consolidation scans
- Support concurrent access (50+ agents) without lock contention

**Strategy:**
- Composite indexes for common query patterns
- Partial indexes for filtered queries (is_deleted=0)
- Covering indexes to avoid table lookups
- Avoid over-indexing (balance read speed vs write overhead)

---

## 2. Primary Indexes

### 2.1 sessions Table Indexes

```sql
-- Primary key index (automatic)
CREATE UNIQUE INDEX idx_sessions_pk ON sessions(id);

-- Status-based queries (find active/paused sessions)
CREATE INDEX idx_sessions_status_updated
ON sessions(status, last_update_time DESC)
WHERE status IN ('active', 'paused');

-- User session history
CREATE INDEX idx_sessions_user_created
ON sessions(user_id, created_at DESC);

-- Project session lookup
CREATE INDEX idx_sessions_project
ON sessions(project_id, created_at DESC)
WHERE project_id IS NOT NULL;
```

**Query Patterns Optimized:**

```sql
-- Active sessions for cleanup job
SELECT * FROM sessions
WHERE status = 'active'
ORDER BY last_update_time DESC;
-- Uses: idx_sessions_status_updated

-- User's recent sessions
SELECT * FROM sessions
WHERE user_id = 'alice'
ORDER BY created_at DESC
LIMIT 10;
-- Uses: idx_sessions_user_created
```

### 2.2 memory_entries Table Indexes

```sql
-- Primary key index (automatic)
CREATE UNIQUE INDEX idx_memory_entries_pk ON memory_entries(id);

-- CRITICAL: Hierarchical namespace retrieval (most common query)
CREATE INDEX idx_memory_namespace_key_version
ON memory_entries(namespace, key, is_deleted, version DESC);

-- Memory type filtering with temporal sorting
CREATE INDEX idx_memory_type_updated
ON memory_entries(memory_type, updated_at DESC)
WHERE is_deleted = 0;

-- Namespace prefix search (e.g., all user:alice:* memories)
CREATE INDEX idx_memory_namespace_prefix
ON memory_entries(namespace, updated_at DESC)
WHERE is_deleted = 0;

-- Conflict detection (find duplicate keys per namespace)
CREATE INDEX idx_memory_conflict_detection
ON memory_entries(namespace, key, is_deleted, version DESC);

-- TTL cleanup for episodic memories
CREATE INDEX idx_memory_episodic_ttl
ON memory_entries(memory_type, updated_at)
WHERE memory_type = 'episodic' AND is_deleted = 0;

-- Audit trail linkage
CREATE INDEX idx_memory_created_by
ON memory_entries(created_by, created_at DESC);
```

**Query Patterns Optimized:**

```sql
-- Get latest version of specific memory
SELECT * FROM memory_entries
WHERE namespace = 'user:alice:preferences'
  AND key = 'communication_style'
  AND is_deleted = 0
ORDER BY version DESC
LIMIT 1;
-- Uses: idx_memory_namespace_key_version

-- Get all active memories in namespace hierarchy
SELECT * FROM memory_entries
WHERE namespace IN ('user:alice:preferences', 'project:schema_redesign:status', 'app:abathur:config')
  AND is_deleted = 0
ORDER BY namespace, version DESC;
-- Uses: idx_memory_namespace_key_version

-- Find memories created by specific session
SELECT * FROM memory_entries
WHERE created_by = 'session:abc123'
ORDER BY created_at DESC;
-- Uses: idx_memory_created_by

-- Cleanup expired episodic memories
SELECT * FROM memory_entries
WHERE memory_type = 'episodic'
  AND updated_at < datetime('now', '-90 days')
  AND is_deleted = 0;
-- Uses: idx_memory_episodic_ttl
```

### 2.3 document_index Table Indexes

```sql
-- Primary key index (automatic)
CREATE UNIQUE INDEX idx_document_index_pk ON document_index(id);

-- File path lookup (UNIQUE constraint creates automatic index)
CREATE UNIQUE INDEX idx_document_file_path ON document_index(file_path);

-- Document type categorization
CREATE INDEX idx_document_type_created
ON document_index(document_type, created_at DESC)
WHERE document_type IS NOT NULL;

-- Sync status monitoring
CREATE INDEX idx_document_sync_status
ON document_index(sync_status, last_synced_at)
WHERE sync_status IN ('pending', 'stale');

-- Content hash for deduplication
CREATE INDEX idx_document_content_hash
ON document_index(content_hash);
```

**Query Patterns Optimized:**

```sql
-- Find document by file path
SELECT * FROM document_index
WHERE file_path = '/path/to/memory-architecture.md';
-- Uses: idx_document_file_path (UNIQUE)

-- Find documents needing embedding sync
SELECT * FROM document_index
WHERE sync_status = 'pending'
ORDER BY created_at ASC;
-- Uses: idx_document_sync_status

-- Check for duplicate content
SELECT COUNT(*) FROM document_index
WHERE content_hash = 'a3f5e8b2...';
-- Uses: idx_document_content_hash
```

---

## 3. Enhanced Existing Table Indexes

### 3.1 tasks Table Indexes (Enhanced)

```sql
-- Existing indexes (from database.py)
CREATE INDEX idx_tasks_status_priority
ON tasks(status, priority DESC, submitted_at ASC);

CREATE INDEX idx_tasks_submitted_at
ON tasks(submitted_at);

CREATE INDEX idx_tasks_parent
ON tasks(parent_task_id);

CREATE INDEX idx_tasks_running_timeout
ON tasks(status, last_updated_at)
WHERE status = 'running';

-- NEW: Session linkage
CREATE INDEX idx_tasks_session
ON tasks(session_id, submitted_at DESC)
WHERE session_id IS NOT NULL;
```

**New Query Pattern:**

```sql
-- Find all tasks in a session
SELECT * FROM tasks
WHERE session_id = 'abc123'
ORDER BY submitted_at DESC;
-- Uses: idx_tasks_session
```

### 3.2 agents Table Indexes (Enhanced)

```sql
-- Existing indexes
CREATE INDEX idx_agents_task
ON agents(task_id);

CREATE INDEX idx_agents_state
ON agents(state);

-- NEW: Session linkage
CREATE INDEX idx_agents_session
ON agents(session_id, spawned_at DESC)
WHERE session_id IS NOT NULL;
```

### 3.3 audit Table Indexes (Enhanced)

```sql
-- Existing indexes
CREATE INDEX idx_audit_task
ON audit(task_id, timestamp DESC);

CREATE INDEX idx_audit_agent
ON audit(agent_id, timestamp DESC);

CREATE INDEX idx_audit_timestamp
ON audit(timestamp DESC);

-- NEW: Memory operation filtering
CREATE INDEX idx_audit_memory_operations
ON audit(memory_operation_type, timestamp DESC)
WHERE memory_operation_type IS NOT NULL;

-- NEW: Memory namespace audit trail
CREATE INDEX idx_audit_memory_namespace
ON audit(memory_namespace, timestamp DESC)
WHERE memory_namespace IS NOT NULL;
```

**New Query Patterns:**

```sql
-- Find all memory operations in last 24 hours
SELECT * FROM audit
WHERE memory_operation_type IS NOT NULL
  AND timestamp > datetime('now', '-1 day')
ORDER BY timestamp DESC;
-- Uses: idx_audit_memory_operations

-- Audit trail for specific memory namespace
SELECT * FROM audit
WHERE memory_namespace LIKE 'user:alice%'
ORDER BY timestamp DESC;
-- Uses: idx_audit_memory_namespace
```

---

## 4. Vector Search Preparation (Future)

### 4.1 sqlite-vss Integration Plan

**When sqlite-vss is deployed:**

```sql
-- Create virtual table for vector similarity search
CREATE VIRTUAL TABLE document_embeddings USING vss(
    embedding(768)  -- nomic-embed-text-v1.5 dimensions
);

-- Link to document_index
CREATE TABLE document_embedding_links (
    document_id INTEGER PRIMARY KEY,
    embedding_id INTEGER NOT NULL,
    FOREIGN KEY (document_id) REFERENCES document_index(id),
    FOREIGN KEY (embedding_id) REFERENCES document_embeddings(rowid)
);

-- Index for fast joining
CREATE INDEX idx_embedding_links_document
ON document_embedding_links(document_id);
```

**Semantic Search Query (Future):**

```sql
-- Find documents similar to query embedding
SELECT di.*, vss_distance(de.embedding, ?) as distance
FROM document_index di
JOIN document_embedding_links del ON di.id = del.document_id
JOIN document_embeddings de ON del.embedding_id = de.rowid
WHERE di.document_type = 'design'
  AND vss_distance(de.embedding, ?) < 0.5  -- Similarity threshold
ORDER BY distance ASC
LIMIT 10;
```

**Performance Target:** <500ms for 10 results from 10,000 documents.

### 4.2 Embedding Sync Strategy

**Background Sync Service (Future MCP Server):**

1. Monitor `document_index` for `sync_status = 'pending'` or `sync_status = 'stale'`
2. Read markdown file at `file_path`
3. Generate embedding via Ollama (nomic-embed-text-v1.5)
4. Store embedding in `document_embeddings` virtual table
5. Update `document_index.sync_status = 'synced'`, `last_synced_at = NOW()`

**File Watcher Integration:**

```python
# Pseudo-code for file watcher
async def on_markdown_file_modified(file_path: str):
    # Calculate new content hash
    new_hash = sha256(read_file(file_path))

    # Update document_index
    await db.execute(
        "UPDATE document_index SET content_hash = ?, sync_status = 'stale', updated_at = ? WHERE file_path = ?",
        (new_hash, datetime.now(), file_path)
    )

    # Trigger async embedding regeneration
    await embedding_service.queue_sync(file_path)
```

---

## 5. Composite Index Trade-offs

### 5.1 Write Performance Impact

**Index Overhead:**

Each index adds ~10-30% write overhead (INSERT/UPDATE must update indexes).

**Current Index Count:**
- sessions: 4 indexes
- memory_entries: 7 indexes
- document_index: 5 indexes
- tasks: 6 indexes (5 existing + 1 new)
- agents: 3 indexes (2 existing + 1 new)
- audit: 6 indexes (3 existing + 3 new)

**Total:** ~31 indexes

**Estimated Write Overhead:** ~20-40% slower writes (acceptable for read-heavy workload).

**Mitigation:**
- Batch inserts for bulk operations (reduce transaction count)
- Async memory extraction (don't block task completion)
- Partial indexes reduce overhead (only index active records)

### 5.2 Covering Index Strategy

**Covering Index:** Index contains all columns needed by query (no table lookup required).

**Example:**

```sql
-- Not covering (requires table lookup)
CREATE INDEX idx_memory_namespace ON memory_entries(namespace);

SELECT namespace, key, value FROM memory_entries
WHERE namespace = 'user:alice';
-- Index scan + table lookup for value column

-- Covering (no table lookup)
CREATE INDEX idx_memory_namespace_key_value
ON memory_entries(namespace, key, value);  -- Large index!

SELECT namespace, key, value FROM memory_entries
WHERE namespace = 'user:alice';
-- Index-only scan (faster)
```

**Trade-off Decision:**

Do NOT create covering indexes for `value` column (large JSON):
- Index size would balloon (value is multi-KB JSON)
- Marginal performance gain (<10ms saved)
- Significant write overhead
- Storage waste

**Recommendation:** Use covering indexes only for small columns (namespace, key, memory_type, timestamps).

---

## 6. Index Maintenance

### 6.1 Automatic Maintenance

**SQLite Auto-Vacuum:**

```sql
PRAGMA auto_vacuum = FULL;  -- Reclaim space after DELETE
```

**SQLite ANALYZE:**

```sql
-- Update query optimizer statistics (run weekly)
ANALYZE;
```

**WAL Checkpoint:**

```sql
-- Automatic via PRAGMA wal_autocheckpoint
PRAGMA wal_autocheckpoint = 1000;  -- Every 1000 pages
```

### 6.2 Manual Maintenance Commands

```sql
-- Rebuild all indexes (if corruption suspected)
REINDEX;

-- Rebuild specific index
REINDEX idx_memory_namespace_key_version;

-- Check integrity
PRAGMA integrity_check;

-- Vacuum database (reclaim space)
VACUUM;
```

### 6.3 Monitoring Index Usage

**SQLite Query Plan Analysis:**

```sql
EXPLAIN QUERY PLAN
SELECT * FROM memory_entries
WHERE namespace = 'user:alice'
  AND is_deleted = 0
ORDER BY version DESC;

-- Output:
-- SEARCH TABLE memory_entries USING INDEX idx_memory_namespace_key_version (namespace=?)
```

**Index Not Used (Requires Optimization):**

```sql
EXPLAIN QUERY PLAN
SELECT * FROM memory_entries
WHERE value LIKE '%schema%';  -- Full table scan (no index on value)

-- Output:
-- SCAN TABLE memory_entries  (BAD: full table scan)
```

---

## 7. Performance Benchmarks (Expected)

### 7.1 Read Performance Targets

| Query Type | Target | Index Used | Expected Actual |
|------------|--------|------------|-----------------|
| Get memory by namespace+key | <10ms | idx_memory_namespace_key_version | 2-5ms |
| List user's memories | <20ms | idx_memory_namespace_prefix | 10-15ms |
| Find active sessions | <10ms | idx_sessions_status_updated | 3-7ms |
| Audit trail lookup | <30ms | idx_audit_memory_namespace | 15-25ms |
| Semantic search (future) | <500ms | vss + indexes | 200-400ms |

### 7.2 Write Performance Targets

| Operation | Target | Index Overhead | Expected Actual |
|-----------|--------|----------------|-----------------|
| Insert memory entry | <20ms | 7 indexes updated | 15-30ms |
| Append session event | <15ms | 2 indexes updated | 10-20ms |
| Create document index | <10ms | 3 indexes updated | 8-15ms |

### 7.3 Concurrency Performance

**50 Concurrent Sessions:**
- Read Throughput: ~1000 queries/sec (WAL mode)
- Write Throughput: ~50 writes/sec (single writer serialization)
- Lock Contention: Minimal (namespace isolation reduces conflicts)

---

## 8. Summary

**Index Coverage:**
✅ All critical query patterns have optimized indexes
✅ Hierarchical namespace queries optimized (<10ms)
✅ Memory type and temporal filtering optimized
✅ Conflict detection and consolidation optimized
✅ Audit trail queries optimized
✅ Vector search preparation (sqlite-vss ready)

**Performance Targets:**
✅ <50ms read latency (most queries <20ms)
✅ <500ms semantic search (when sqlite-vss deployed)
✅ Supports 50+ concurrent agents
✅ Efficient memory consolidation scans

**Trade-offs Balanced:**
✅ Read performance prioritized (20-40% write overhead acceptable)
✅ No covering indexes for large JSON columns
✅ Partial indexes reduce unnecessary indexing
✅ Strategic composite indexes minimize index count

---

**Document Version:** 1.0
**Author:** memory-systems-architect (orchestrated)
**Date:** 2025-10-10
**Status:** Phase 1 Design - Awaiting Validation
**Next Document:** migration-strategy.md (Fresh start deployment approach)
