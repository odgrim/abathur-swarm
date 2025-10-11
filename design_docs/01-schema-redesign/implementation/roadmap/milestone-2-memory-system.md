# Milestone 2: Memory Management System

## Overview

**Goal:** Implement complete memory management system with SessionService and MemoryService APIs

**Timeline:** Weeks 3-4 (10 business days)

**Dependencies:**
- Milestone 1 complete and validated (core schema deployed)
- Enhanced Database class functional
- Unit test suite passing

---

## Objectives

1. Deploy memory management tables (sessions, memory_entries, document_index)
2. Implement SessionService API with event tracking and state management
3. Implement MemoryService API with namespace hierarchy and versioning
4. Create memory performance indexes (18 indexes)
5. Build integration test suite for session-task-memory workflows
6. Validate namespace hierarchy and access control patterns

---

## Tasks and Effort Estimates

### Week 3: Memory Tables and SessionService

| Task | Description | Effort (hours) | Owner | Dependencies |
|------|-------------|---------------|-------|--------------|
| **3.1** | Deploy sessions table with JSON columns | 4h | Dev Team | Milestone 1 complete |
| **3.2** | Deploy memory_entries table with versioning | 4h | Dev Team | Task 3.1 |
| **3.3** | Deploy document_index table (without embeddings) | 3h | Dev Team | Task 3.1 |
| **3.4** | Create session indexes (4 indexes) | 2h | Dev Team | Task 3.1 |
| **3.5** | Create memory indexes (7 indexes) | 3h | Dev Team | Task 3.2 |
| **3.6** | Create document indexes (5 indexes) | 2h | Dev Team | Task 3.3 |
| **3.7** | Implement SessionService class | 8h | Dev Team | Task 3.4 |
| **3.8** | Write unit tests for SessionService | 6h | Dev Team | Task 3.7 |
| **3.9** | Implement session event appending logic | 4h | Dev Team | Task 3.7 |
| **3.10** | Implement session state management (temp:, session:) | 4h | Dev Team | Task 3.9 |

**Week 3 Total:** 40 hours

### Week 4: MemoryService and Integration Testing

| Task | Description | Effort (hours) | Owner | Dependencies |
|------|-------------|---------------|-------|--------------|
| **4.1** | Implement MemoryService class | 8h | Dev Team | Task 3.2 |
| **4.2** | Implement memory versioning logic | 6h | Dev Team | Task 4.1 |
| **4.3** | Implement namespace hierarchy queries | 6h | Dev Team | Task 4.1 |
| **4.4** | Write unit tests for MemoryService | 6h | Dev Team | Task 4.3 |
| **4.5** | Implement audit logging for memory operations | 4h | Dev Team | Task 4.1 |
| **4.6** | Write integration tests (session → task → memory) | 8h | Dev Team | Tasks 3.7, 4.1 |
| **4.7** | Test concurrent session access (50+ sessions) | 4h | Dev Team | Task 3.7 |
| **4.8** | Test memory consolidation workflows | 4h | Dev Team | Task 4.2 |
| **4.9** | Validate namespace access control patterns | 4h | Dev Team | Task 4.3 |
| **4.10** | Performance testing and optimization | 6h | Dev Team | All tasks |

**Week 4 Total:** 56 hours

**Milestone 2 Total Effort:** 96 hours (~2 developers × 1 week)

---

## Deliverables

### 1. Memory Management Tables

**sessions Table:**
```sql
CREATE TABLE sessions (
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
);
```

**memory_entries Table:**
```sql
CREATE TABLE memory_entries (
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
);
```

**document_index Table:**
```sql
CREATE TABLE document_index (
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
);
```

### 2. Memory Performance Indexes (18 indexes)

**Session Indexes (4):**
- `idx_sessions_status_updated` - Active/paused session queries
- `idx_sessions_user_created` - User session history
- `idx_sessions_project_id` - Project-scoped sessions
- `idx_sessions_app_user` - Composite app + user lookup

**Memory Entry Indexes (7):**
- `idx_memory_namespace_key_version` - Current version retrieval (composite)
- `idx_memory_type_updated` - Memory type filtering with recency
- `idx_memory_namespace_prefix` - Namespace hierarchy queries
- `idx_memory_created_by` - Creator-based queries
- `idx_memory_updated_at` - Temporal queries
- `idx_memory_type_namespace` - Type + namespace composite
- `idx_memory_is_deleted` - Active memory filtering (partial index)

**Document Index Indexes (5):**
- `idx_document_file_path` - File path unique constraint (enforced)
- `idx_document_type` - Document type filtering
- `idx_document_sync_status` - Sync status queries
- `idx_document_hash` - Content hash lookups
- `idx_document_updated` - Recently updated documents

**Covering Indexes:**
- `idx_memory_namespace_key_version` is a covering index for most memory queries
- Reduces disk I/O by including (namespace, key, is_deleted, version DESC)

### 3. SessionService API

**File:** `src/abathur/infrastructure/session_service.py`

**Public Interface:**
```python
class SessionService:
    async def create_session(
        self, session_id: str, app_name: str, user_id: str,
        project_id: Optional[str] = None,
        initial_state: Optional[Dict[str, Any]] = None
    ) -> None:
        """Create new session with optional initial state."""

    async def get_session(self, session_id: str) -> Optional[Dict[str, Any]]:
        """Retrieve session by ID with parsed JSON fields."""

    async def append_event(
        self, session_id: str, event: Dict[str, Any],
        state_delta: Optional[Dict[str, Any]] = None
    ) -> None:
        """Append event to session with optional state update."""

    async def update_status(self, session_id: str, status: str) -> None:
        """Update session lifecycle status."""

    async def get_state(self, session_id: str, key: str) -> Optional[Any]:
        """Get specific state value from session."""

    async def set_state(self, session_id: str, key: str, value: Any) -> None:
        """Set specific state value in session."""

    async def list_active_sessions(self, user_id: Optional[str] = None) -> List[Dict]:
        """List all active sessions, optionally filtered by user."""

    async def terminate_session(self, session_id: str) -> None:
        """Terminate session and clear temp: state."""
```

**Error Handling:**
- `ValueError`: Invalid status, session not found, invalid JSON
- `IntegrityError`: Duplicate session_id
- `JSONDecodeError`: Corrupted JSON in events or state

### 4. MemoryService API

**File:** `src/abathur/infrastructure/memory_service.py`

**Public Interface:**
```python
class MemoryService:
    async def add_memory(
        self, namespace: str, key: str, value: Dict[str, Any],
        memory_type: str, created_by: str, task_id: str,
        metadata: Optional[Dict[str, Any]] = None
    ) -> int:
        """Add new memory entry (version 1)."""

    async def get_memory(
        self, namespace: str, key: str, version: Optional[int] = None
    ) -> Optional[Dict[str, Any]]:
        """Retrieve memory entry (latest or specific version)."""

    async def update_memory(
        self, namespace: str, key: str, value: Dict[str, Any],
        updated_by: str, task_id: str
    ) -> int:
        """Update memory by creating new version."""

    async def delete_memory(
        self, namespace: str, key: str, deleted_by: str, task_id: str
    ) -> None:
        """Soft-delete memory (set is_deleted=1)."""

    async def search_memories(
        self, namespace_prefix: str,
        memory_type: Optional[str] = None, limit: int = 50
    ) -> List[Dict[str, Any]]:
        """Search memories by namespace prefix and optional type."""

    async def get_memory_history(
        self, namespace: str, key: str
    ) -> List[Dict[str, Any]]:
        """Get all versions of a memory entry."""

    async def consolidate_memories(
        self, namespace: str, consolidation_strategy: str = "llm_based"
    ) -> int:
        """Consolidate conflicting memories in namespace."""
```

**Namespace Validation:**
- All namespaces must contain `:` separator (e.g., `user:alice:preferences`)
- Valid prefixes: `temp:`, `session:`, `user:`, `app:`, `project:`
- Namespace hierarchy enforced in queries (LIKE 'prefix%')

### 5. Integration Test Suite

**File:** `tests/integration/test_session_memory_workflow.py`

**Test Coverage:**
- ✅ Complete workflow: create session → execute task → store memory → verify audit
- ✅ Session state updates with temp: and session: namespaces
- ✅ Memory versioning (create v1 → update to v2 → retrieve both versions)
- ✅ Namespace hierarchy queries (user:alice matches user:alice:preferences)
- ✅ Concurrent session access (50+ sessions reading/writing simultaneously)
- ✅ Memory consolidation (detect conflicts → LLM-based merge)
- ✅ Audit trail verification (all memory operations logged)
- ✅ Foreign key cascade rules (session deletion sets task.session_id to NULL)

**Coverage Target:** 85%+ for service layer

### 6. Performance Validation Report

**File:** `docs/performance_validation_milestone2.md`

**Benchmarks:**
- Session creation latency: <10ms
- Session event append latency: <20ms (with state delta merge)
- Memory retrieval (current version): <20ms
- Namespace hierarchy query (100 memories): <50ms
- Concurrent session reads (50 sessions): <1 second total
- Memory version history retrieval: <30ms for 10 versions

**Index Usage Verification:**
- All queries show "USING INDEX" in EXPLAIN QUERY PLAN
- No full table scans detected in critical queries
- Covering index usage confirmed for memory_entries queries

---

## Acceptance Criteria

### Technical Validation

- [ ] All memory tables created successfully
- [ ] All 18 memory indexes created and functional
- [ ] SessionService passes all unit tests (100% of tests)
- [ ] MemoryService passes all unit tests (100% of tests)
- [ ] Integration tests pass (session → task → memory workflows)
- [ ] All queries show index usage in EXPLAIN QUERY PLAN
- [ ] Performance benchmarks meet targets (<50ms reads)
- [ ] JSON validation constraints prevent invalid data

### Functional Validation

- [ ] Sessions can be created, updated, and terminated
- [ ] Events can be appended with state delta merging
- [ ] Memory entries support versioning (create v1, update to v2)
- [ ] Namespace hierarchy queries return correct results
- [ ] Memory consolidation detects and resolves conflicts
- [ ] Audit logging captures all memory operations
- [ ] Soft-delete prevents data loss (is_deleted flag)
- [ ] Foreign key relationships enforce data integrity

### Performance Validation

- [ ] 50+ concurrent sessions supported without degradation
- [ ] Session state retrieval <10ms
- [ ] Memory retrieval (current version) <20ms
- [ ] Namespace hierarchy query <50ms
- [ ] No performance regression from Milestone 1 baseline

---

## Risks and Mitigation

### Risk 1: JSON Performance Overhead

**Description:** JSON parsing/serialization may slow down event appending and state updates
- **Probability:** Medium
- **Impact:** Medium
- **Mitigation:**
  - Use prepared statements to minimize parsing
  - Batch event appends when possible (single transaction for multiple events)
  - Consider JSON1 extension for in-database JSON queries (json_extract)
- **Contingency:** Limit event array size (max 1000 events per session, archive older events)

### Risk 2: Memory Versioning Storage Growth

**Description:** Versioning all memory updates may cause rapid database growth
- **Probability:** High
- **Impact:** Medium
- **Mitigation:**
  - Implement version cleanup for episodic memories (TTL 90 days)
  - Archive old versions to cold storage (document_index points to external files)
  - Monitor database size growth (alert if >1GB/week)
- **Contingency:** Implement version squashing (consolidate old versions into summaries)

### Risk 3: Namespace Query Performance

**Description:** Namespace prefix queries (LIKE 'user:alice%') may not use indexes efficiently
- **Probability:** Low
- **Impact:** High
- **Mitigation:**
  - Use composite index (namespace, key, is_deleted, version DESC)
  - Verify EXPLAIN QUERY PLAN shows index range scan (not full scan)
  - Consider denormalized namespace_prefix column for exact prefix matching
- **Contingency:** Add namespace_prefix extracted column with index for faster lookups

### Risk 4: Concurrent Session Write Conflicts

**Description:** Multiple agents updating same session state may cause lock contention
- **Probability:** Medium
- **Impact:** Medium
- **Mitigation:**
  - Use row-level locking (SELECT ... FOR UPDATE) in append_event
  - Implement optimistic concurrency control (version check before update)
  - Increase busy_timeout to 5000ms (5 seconds)
- **Contingency:** Implement retry logic with exponential backoff (max 3 retries)

---

## Go/No-Go Decision Criteria

### Prerequisites (Must be Complete)

1. ✅ Milestone 1 validated and signed off
2. ✅ All memory tables created successfully
3. ✅ SessionService and MemoryService APIs implemented
4. ✅ Unit test suite passing with 85%+ coverage

### Validation Checks (Must Pass)

1. **Database Integrity:**
   - All memory tables pass `PRAGMA integrity_check`
   - All foreign key relationships validated
   - All JSON validation constraints working

2. **Performance Targets:**
   - Session operations <20ms (99th percentile)
   - Memory operations <50ms (99th percentile)
   - 50+ concurrent sessions without degradation

3. **Functional Correctness:**
   - Memory versioning creates new rows (doesn't overwrite)
   - Namespace hierarchy queries return correct scope
   - Audit logging captures all memory operations
   - Soft-delete preserves data (is_deleted=1, not DELETE)

### Rollback Plan (If Go/No-Go Fails)

1. **Immediate Actions:**
   - Stop all Milestone 2 development
   - Revert database to Milestone 1 state (drop memory tables)
   - Analyze failure root cause (performance, correctness, design flaw)

2. **Rollback Procedure:**
   ```sql
   -- Drop memory tables and indexes
   DROP TABLE IF EXISTS sessions;
   DROP TABLE IF EXISTS memory_entries;
   DROP TABLE IF EXISTS document_index;
   DROP INDEX IF EXISTS idx_sessions_status_updated;
   -- ... (drop all 18 memory indexes)
   ```

3. **Recovery Actions:**
   - Fix identified issues in SessionService/MemoryService
   - Re-test in isolated environment
   - Re-deploy after validation

---

## Post-Milestone Activities

### Documentation Updates

- [ ] Update API documentation with SessionService and MemoryService
- [ ] Document namespace hierarchy patterns and best practices
- [ ] Create memory versioning guide for developers
- [ ] Update troubleshooting guide with common memory issues

### Knowledge Transfer

- [ ] Conduct team demo of SessionService event tracking
- [ ] Walkthrough memory namespace hierarchy with examples
- [ ] Share performance optimization techniques (covering indexes)

### Preparation for Milestone 3

- [ ] Review sqlite-vss extension requirements
- [ ] Setup Ollama environment for embedding generation
- [ ] Allocate resources for vector search implementation
- [ ] Plan embedding sync background service design

---

## Success Metrics

### Quantitative Metrics

| Metric | Target | Actual | Status |
|--------|--------|--------|--------|
| Unit test coverage (services) | 85%+ | ___ % | ⏳ Pending |
| Integration test pass rate | 100% | ___ % | ⏳ Pending |
| Session operation latency | <20ms | ___ ms | ⏳ Pending |
| Memory operation latency | <50ms | ___ ms | ⏳ Pending |
| Concurrent sessions supported | 50+ | ___ | ⏳ Pending |
| Namespace query accuracy | 100% | ___ % | ⏳ Pending |

### Qualitative Metrics

- [ ] Code maintainability: High (clear API design)
- [ ] Service reliability: High (comprehensive error handling)
- [ ] Namespace usability: High (intuitive hierarchy)
- [ ] Integration complexity: Low (clean service interfaces)

---

## Lessons Learned (Post-Milestone)

### What Went Well

_To be filled after milestone completion_

### What Could Be Improved

_To be filled after milestone completion_

### Action Items for Milestone 3

_To be filled after milestone completion_

---

## References

**Phase 2 Technical Specifications:**
- [DDL Memory Tables](../phase2_tech_specs/ddl-memory-tables.sql) - Memory table definitions
- [API Specifications](../phase2_tech_specs/api-specifications.md) - SessionService and MemoryService specs
- [Query Patterns Write](../phase2_tech_specs/query-patterns-write.md) - Write operation patterns
- [Test Scenarios](../phase2_tech_specs/test-scenarios.md) - Integration test examples

**Phase 3 Implementation Plan:**
- [Testing Strategy](./testing-strategy.md) - Integration testing approach
- [Milestone 1](./milestone-1-core-schema.md) - Core schema foundation
- [Milestone 3](./milestone-3-vector-search.md) - Vector search integration (next)

---

**Milestone Version:** 1.0
**Author:** implementation-planner
**Date:** 2025-10-10
**Status:** Ready for Execution
**Previous Milestone:** [Milestone 1: Core Schema](./milestone-1-core-schema.md)
**Next Milestone:** [Milestone 3: Vector Search Integration](./milestone-3-vector-search.md)
