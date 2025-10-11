-- ================================================================
-- DDL Script: Performance Optimization Indexes
-- Purpose: All 31 indexes for optimized query performance across all tables
-- Phase: Phase 2 Technical Specifications
-- Author: technical-specifications-writer
-- Date: 2025-10-10
-- ================================================================

-- Prerequisites: All tables created (ddl-memory-tables.sql + ddl-core-tables.sql)
-- Execution: Run AFTER both table creation scripts
-- Performance Target: <50ms reads, <500ms semantic search, support 50+ concurrent sessions

-- ================================================================
-- SESSIONS TABLE INDEXES (4 indexes)
-- ================================================================

-- Index 1: Primary key index (automatic, included for documentation)
-- Purpose: Fast session retrieval by ID
-- Query Pattern: SELECT * FROM sessions WHERE id = ?
CREATE UNIQUE INDEX IF NOT EXISTS idx_sessions_pk
ON sessions(id);

-- Index 2: Status-based queries with temporal sorting
-- Purpose: Find active/paused sessions for monitoring and cleanup
-- Query Pattern: SELECT * FROM sessions WHERE status = 'active' ORDER BY last_update_time DESC
CREATE INDEX IF NOT EXISTS idx_sessions_status_updated
ON sessions(status, last_update_time DESC)
WHERE status IN ('active', 'paused');
-- EXPLAIN QUERY PLAN: Uses covering index for status filter + sort

-- Index 3: User session history
-- Purpose: Retrieve user's recent sessions for context synthesis
-- Query Pattern: SELECT * FROM sessions WHERE user_id = 'alice' ORDER BY created_at DESC LIMIT 10
CREATE INDEX IF NOT EXISTS idx_sessions_user_created
ON sessions(user_id, created_at DESC);
-- EXPLAIN QUERY PLAN: Index scan on (user_id, created_at)

-- Index 4: Project session lookup
-- Purpose: Find all sessions for a project (cross-agent collaboration)
-- Query Pattern: SELECT * FROM sessions WHERE project_id = 'schema_redesign' ORDER BY created_at DESC
CREATE INDEX IF NOT EXISTS idx_sessions_project
ON sessions(project_id, created_at DESC)
WHERE project_id IS NOT NULL;
-- EXPLAIN QUERY PLAN: Partial index scan (excludes NULL project_id)

-- ================================================================
-- MEMORY_ENTRIES TABLE INDEXES (7 indexes)
-- ================================================================

-- Index 5: Primary key index (automatic, included for documentation)
-- Purpose: Fast memory entry retrieval by ID
CREATE UNIQUE INDEX IF NOT EXISTS idx_memory_entries_pk
ON memory_entries(id);

-- Index 6: CRITICAL - Hierarchical namespace retrieval
-- Purpose: Fastest access pattern for namespace+key queries (most common operation)
-- Query Pattern: SELECT * FROM memory_entries WHERE namespace = 'user:alice:preferences' AND key = 'theme' AND is_deleted = 0 ORDER BY version DESC LIMIT 1
CREATE INDEX IF NOT EXISTS idx_memory_namespace_key_version
ON memory_entries(namespace, key, is_deleted, version DESC);
-- EXPLAIN QUERY PLAN: Index-only scan for current version retrieval
-- Performance: <10ms for single-key lookup

-- Index 7: Memory type filtering with temporal sorting
-- Purpose: Query memories by type (semantic, episodic, procedural) sorted by recency
-- Query Pattern: SELECT * FROM memory_entries WHERE memory_type = 'episodic' AND is_deleted = 0 ORDER BY updated_at DESC LIMIT 20
CREATE INDEX IF NOT EXISTS idx_memory_type_updated
ON memory_entries(memory_type, updated_at DESC)
WHERE is_deleted = 0;
-- EXPLAIN QUERY PLAN: Partial index scan (active memories only)

-- Index 8: Namespace prefix search
-- Purpose: Retrieve all memories in namespace hierarchy (e.g., all user:alice:* memories)
-- Query Pattern: SELECT * FROM memory_entries WHERE namespace LIKE 'user:alice%' AND is_deleted = 0 ORDER BY updated_at DESC
CREATE INDEX IF NOT EXISTS idx_memory_namespace_prefix
ON memory_entries(namespace, updated_at DESC)
WHERE is_deleted = 0;
-- EXPLAIN QUERY PLAN: Index scan with LIKE prefix match
-- Note: LIKE 'prefix%' can use index, LIKE '%suffix' cannot

-- Index 9: Conflict detection
-- Purpose: Find duplicate keys within namespace for consolidation
-- Query Pattern: SELECT namespace, key, COUNT(*) FROM memory_entries WHERE is_deleted = 0 GROUP BY namespace, key HAVING COUNT(*) > 1
CREATE INDEX IF NOT EXISTS idx_memory_conflict_detection
ON memory_entries(namespace, key, is_deleted, version DESC);
-- EXPLAIN QUERY PLAN: Index scan for GROUP BY operation
-- Note: Duplicate of idx_memory_namespace_key_version, but explicit for clarity

-- Index 10: TTL cleanup for episodic memories
-- Purpose: Efficiently find expired episodic memories for automated cleanup
-- Query Pattern: SELECT * FROM memory_entries WHERE memory_type = 'episodic' AND updated_at < datetime('now', '-90 days') AND is_deleted = 0
CREATE INDEX IF NOT EXISTS idx_memory_episodic_ttl
ON memory_entries(memory_type, updated_at)
WHERE memory_type = 'episodic' AND is_deleted = 0;
-- EXPLAIN QUERY PLAN: Partial index for episodic cleanup job
-- Performance: <100ms for scanning 10,000+ episodic memories

-- Index 11: Audit trail linkage
-- Purpose: Find all memories created/updated by specific session or agent
-- Query Pattern: SELECT * FROM memory_entries WHERE created_by = 'session:abc123' ORDER BY created_at DESC
CREATE INDEX IF NOT EXISTS idx_memory_created_by
ON memory_entries(created_by, created_at DESC);
-- EXPLAIN QUERY PLAN: Index scan on created_by + sort by created_at

-- ================================================================
-- DOCUMENT_INDEX TABLE INDEXES (5 indexes)
-- ================================================================

-- Index 12: Primary key index (automatic, included for documentation)
-- Purpose: Fast document retrieval by ID
CREATE UNIQUE INDEX IF NOT EXISTS idx_document_index_pk
ON document_index(id);

-- Index 13: File path lookup (UNIQUE constraint creates automatic index)
-- Purpose: Fast document lookup by file path (most common access pattern)
-- Query Pattern: SELECT * FROM document_index WHERE file_path = '/path/to/file.md'
CREATE UNIQUE INDEX IF NOT EXISTS idx_document_file_path
ON document_index(file_path);
-- EXPLAIN QUERY PLAN: Unique index scan (single-row lookup)
-- Performance: <5ms for file path lookup

-- Index 14: Document type categorization
-- Purpose: Retrieve documents by type (design, specification, plan, report)
-- Query Pattern: SELECT * FROM document_index WHERE document_type = 'design' ORDER BY created_at DESC
CREATE INDEX IF NOT EXISTS idx_document_type_created
ON document_index(document_type, created_at DESC)
WHERE document_type IS NOT NULL;
-- EXPLAIN QUERY PLAN: Partial index scan (excludes NULL types)

-- Index 15: Sync status monitoring
-- Purpose: Find documents needing embedding generation/update
-- Query Pattern: SELECT * FROM document_index WHERE sync_status = 'pending' ORDER BY created_at ASC
CREATE INDEX IF NOT EXISTS idx_document_sync_status
ON document_index(sync_status, last_synced_at)
WHERE sync_status IN ('pending', 'stale');
-- EXPLAIN QUERY PLAN: Partial index for sync job (only pending/stale docs)

-- Index 16: Content hash for deduplication
-- Purpose: Detect duplicate content across files
-- Query Pattern: SELECT COUNT(*) FROM document_index WHERE content_hash = 'a3f5e8b2...'
CREATE INDEX IF NOT EXISTS idx_document_content_hash
ON document_index(content_hash);
-- EXPLAIN QUERY PLAN: Index scan on content_hash

-- ================================================================
-- TASKS TABLE INDEXES (6 indexes - 5 existing + 1 new)
-- ================================================================

-- Index 17: Status and priority for task queue
-- Purpose: Dequeue next pending task with highest priority
-- Query Pattern: SELECT * FROM tasks WHERE status = 'pending' ORDER BY priority DESC, submitted_at ASC LIMIT 1
CREATE INDEX IF NOT EXISTS idx_tasks_status_priority
ON tasks(status, priority DESC, submitted_at ASC);
-- EXPLAIN QUERY PLAN: Index scan with early termination (LIMIT 1)

-- Index 18: Submitted timestamp for temporal queries
-- Purpose: Find tasks submitted within time range
-- Query Pattern: SELECT * FROM tasks WHERE submitted_at > datetime('now', '-1 day')
CREATE INDEX IF NOT EXISTS idx_tasks_submitted_at
ON tasks(submitted_at);
-- EXPLAIN QUERY PLAN: Index range scan

-- Index 19: Parent task relationships
-- Purpose: Find child tasks of parent
-- Query Pattern: SELECT * FROM tasks WHERE parent_task_id = ?
CREATE INDEX IF NOT EXISTS idx_tasks_parent
ON tasks(parent_task_id);
-- EXPLAIN QUERY PLAN: Index scan on parent_task_id

-- Index 20: Running task timeout detection
-- Purpose: Find stale running tasks that exceeded execution timeout
-- Query Pattern: SELECT * FROM tasks WHERE status = 'running' AND (julianday('now') - julianday(last_updated_at)) * 86400 > max_execution_timeout_seconds
CREATE INDEX IF NOT EXISTS idx_tasks_running_timeout
ON tasks(status, last_updated_at)
WHERE status = 'running';
-- EXPLAIN QUERY PLAN: Partial index for timeout monitoring job

-- Index 21: NEW - Session linkage
-- Purpose: Find all tasks in a session for context retrieval
-- Query Pattern: SELECT * FROM tasks WHERE session_id = 'abc123' ORDER BY submitted_at DESC
CREATE INDEX IF NOT EXISTS idx_tasks_session
ON tasks(session_id, submitted_at DESC)
WHERE session_id IS NOT NULL;
-- EXPLAIN QUERY PLAN: Partial index scan (excludes orphaned tasks with NULL session_id)

-- ================================================================
-- AGENTS TABLE INDEXES (3 indexes - 2 existing + 1 new)
-- ================================================================

-- Index 22: Task relationship
-- Purpose: Find all agents for a task
-- Query Pattern: SELECT * FROM agents WHERE task_id = ?
CREATE INDEX IF NOT EXISTS idx_agents_task
ON agents(task_id);
-- EXPLAIN QUERY PLAN: Index scan on task_id

-- Index 23: Agent state for monitoring
-- Purpose: Find agents by state (idle, working, waiting, terminated)
-- Query Pattern: SELECT * FROM agents WHERE state = 'working'
CREATE INDEX IF NOT EXISTS idx_agents_state
ON agents(state);
-- EXPLAIN QUERY PLAN: Index scan on state

-- Index 24: NEW - Session linkage
-- Purpose: Find all agents spawned in a session
-- Query Pattern: SELECT * FROM agents WHERE session_id = 'abc123' ORDER BY spawned_at DESC
CREATE INDEX IF NOT EXISTS idx_agents_session
ON agents(session_id, spawned_at DESC)
WHERE session_id IS NOT NULL;
-- EXPLAIN QUERY PLAN: Partial index scan

-- ================================================================
-- AUDIT TABLE INDEXES (6 indexes - 3 existing + 3 new)
-- ================================================================

-- Index 25: Task audit trail
-- Purpose: Retrieve all audit entries for a task
-- Query Pattern: SELECT * FROM audit WHERE task_id = ? ORDER BY timestamp DESC
CREATE INDEX IF NOT EXISTS idx_audit_task
ON audit(task_id, timestamp DESC);
-- EXPLAIN QUERY PLAN: Index scan with sort on timestamp

-- Index 26: Agent audit trail
-- Purpose: Retrieve all actions by an agent
-- Query Pattern: SELECT * FROM audit WHERE agent_id = ? ORDER BY timestamp DESC
CREATE INDEX IF NOT EXISTS idx_audit_agent
ON audit(agent_id, timestamp DESC);
-- EXPLAIN QUERY PLAN: Index scan with sort on timestamp

-- Index 27: Global audit timeline
-- Purpose: Recent audit events across all tasks/agents
-- Query Pattern: SELECT * FROM audit ORDER BY timestamp DESC LIMIT 100
CREATE INDEX IF NOT EXISTS idx_audit_timestamp
ON audit(timestamp DESC);
-- EXPLAIN QUERY PLAN: Index-only scan for timeline queries

-- Index 28: NEW - Memory operation filtering
-- Purpose: Find all memory-specific operations (create, update, delete, consolidate)
-- Query Pattern: SELECT * FROM audit WHERE memory_operation_type IS NOT NULL AND timestamp > datetime('now', '-1 day') ORDER BY timestamp DESC
CREATE INDEX IF NOT EXISTS idx_audit_memory_operations
ON audit(memory_operation_type, timestamp DESC)
WHERE memory_operation_type IS NOT NULL;
-- EXPLAIN QUERY PLAN: Partial index for memory operation audit

-- Index 29: NEW - Memory namespace audit trail
-- Purpose: Audit trail for specific namespace (e.g., all changes to user:alice:*)
-- Query Pattern: SELECT * FROM audit WHERE memory_namespace LIKE 'user:alice%' ORDER BY timestamp DESC
CREATE INDEX IF NOT EXISTS idx_audit_memory_namespace
ON audit(memory_namespace, timestamp DESC)
WHERE memory_namespace IS NOT NULL;
-- EXPLAIN QUERY PLAN: Index scan with LIKE prefix match

-- Index 30: NEW - Memory entry audit linkage
-- Purpose: Find all audit entries for a specific memory entry
-- Query Pattern: SELECT * FROM audit WHERE memory_entry_id = 12345 ORDER BY timestamp DESC
CREATE INDEX IF NOT EXISTS idx_audit_memory_entry
ON audit(memory_entry_id, timestamp DESC)
WHERE memory_entry_id IS NOT NULL;
-- EXPLAIN QUERY PLAN: Partial index scan

-- ================================================================
-- STATE TABLE INDEX (1 index - existing, deprecated)
-- ================================================================

-- Index 31: Task and key lookup
-- Purpose: Fast state retrieval (legacy, superseded by sessions.state)
-- Query Pattern: SELECT value FROM state WHERE task_id = ? AND key = ?
CREATE INDEX IF NOT EXISTS idx_state_task_key
ON state(task_id, key);
-- EXPLAIN QUERY PLAN: Composite index scan
-- Note: UNIQUE constraint on (task_id, key) already creates an index, this is explicit

-- ================================================================
-- METRICS TABLE INDEX (1 index - existing)
-- ================================================================

-- Index 32: Metric name and timestamp
-- Purpose: Retrieve metrics by name within time range
-- Query Pattern: SELECT * FROM metrics WHERE metric_name = 'task_duration_ms' AND timestamp > datetime('now', '-1 hour') ORDER BY timestamp DESC
CREATE INDEX IF NOT EXISTS idx_metrics_name_timestamp
ON metrics(metric_name, timestamp DESC);
-- EXPLAIN QUERY PLAN: Composite index scan with range filter

-- ================================================================
-- CHECKPOINTS TABLE INDEX (1 index - existing)
-- ================================================================

-- Index 33: Task and iteration lookup
-- Purpose: Retrieve checkpoints for a task in descending order
-- Query Pattern: SELECT * FROM checkpoints WHERE task_id = ? ORDER BY iteration DESC
CREATE INDEX IF NOT EXISTS idx_checkpoints_task
ON checkpoints(task_id, iteration DESC);
-- EXPLAIN QUERY PLAN: Composite index scan
-- Note: PRIMARY KEY (task_id, iteration) already creates an index, this is explicit

-- ================================================================
-- INDEX VERIFICATION AND MAINTENANCE
-- ================================================================

-- After executing this script, verify all indexes:
-- SELECT name, tbl_name FROM sqlite_master WHERE type='index' ORDER BY tbl_name, name;

-- Check index usage with EXPLAIN QUERY PLAN:
-- EXPLAIN QUERY PLAN SELECT * FROM memory_entries WHERE namespace = 'user:alice' AND is_deleted = 0 ORDER BY version DESC LIMIT 1;
-- Expected output: SEARCH TABLE memory_entries USING INDEX idx_memory_namespace_key_version (namespace=? AND is_deleted=?)

-- Analyze query optimizer statistics (run weekly in production):
-- ANALYZE;

-- Rebuild all indexes if corruption suspected:
-- REINDEX;

-- ================================================================
-- PERFORMANCE BENCHMARKS
-- ================================================================

-- Expected query performance with these indexes:
-- - Session retrieval by ID: <5ms (index scan on id)
-- - Memory retrieval by namespace+key: <10ms (covering index)
-- - Hierarchical namespace query: <50ms (partial index with LIKE prefix)
-- - Task dequeue (priority queue): <10ms (composite index with early termination)
-- - Audit trail retrieval: <30ms (composite index with timestamp sort)
-- - Document lookup by file path: <5ms (unique index)
-- - Memory cleanup job (10,000 entries): <100ms (partial index on episodic TTL)

-- Concurrent access performance (WAL mode):
-- - 50+ concurrent read sessions: ~1000 queries/sec
-- - Single writer: ~50 writes/sec (serialized by SQLite)
-- - Lock contention: Minimal (namespace isolation reduces conflicts)

-- ================================================================
-- INDEX OVERHEAD ANALYSIS
-- ================================================================

-- Write overhead estimation:
-- - Each INSERT/UPDATE must update relevant indexes
-- - Total indexes: 33 (some are partial, reducing overhead)
-- - Estimated write overhead: 20-40% slower writes
-- - Mitigation: Batch inserts, async memory extraction, partial indexes

-- Storage overhead estimation:
-- - Index size: ~10-30% of table size (depending on column types)
-- - Critical indexes are on TEXT/INTEGER columns (efficient)
-- - BLOB columns (embeddings) not indexed (avoid large index size)
-- - Total database size growth: ~15-20% from indexes

-- ================================================================
-- END OF SCRIPT
-- ================================================================
