# Database Schema Tables - Complete DDL Specifications

## Table of Contents
1. [New Tables](#new-tables)
2. [Enhanced Existing Tables](#enhanced-existing-tables)
3. [Table Relationships](#table-relationships)
4. [Data Types and Constraints](#data-types-and-constraints)

---

## 1. New Tables

### 1.1 sessions Table

**Purpose:** Core session management with event tracking and state storage.

```sql
CREATE TABLE IF NOT EXISTS sessions (
    -- Primary identifiers
    id TEXT PRIMARY KEY,                    -- UUID: session identifier
    app_name TEXT NOT NULL,                 -- Application context (e.g., "abathur")
    user_id TEXT NOT NULL,                  -- User identifier
    project_id TEXT,                        -- Optional project association (for cross-agent collaboration)

    -- Lifecycle management
    status TEXT NOT NULL DEFAULT 'created', -- created|active|paused|terminated|archived

    -- Session data (JSON columns)
    events TEXT NOT NULL DEFAULT '[]',      -- JSON array of Event objects (chronological history)
    state TEXT NOT NULL DEFAULT '{}',       -- JSON dict of session state (key-value pairs with namespace prefixes)

    -- Metadata
    metadata TEXT DEFAULT '{}',             -- JSON dict for extensibility (tags, labels, etc.)

    -- Timestamps
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    last_update_time TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    terminated_at TIMESTAMP,                -- When session ended
    archived_at TIMESTAMP,                  -- When session archived to cold storage

    -- Constraints
    CHECK(status IN ('created', 'active', 'paused', 'terminated', 'archived')),
    CHECK(json_valid(events)),              -- Ensure events is valid JSON
    CHECK(json_valid(state)),               -- Ensure state is valid JSON
    CHECK(json_valid(metadata))             -- Ensure metadata is valid JSON
);
```

**events JSON Structure Example:**
```json
[
  {
    "event_id": "evt_001",
    "timestamp": "2025-10-10T10:00:00Z",
    "event_type": "message",
    "actor": "user",
    "content": {"message": "Design the memory schema"},
    "state_delta": {"session:current_task": "memory_architecture"},
    "is_final_response": false
  },
  {
    "event_id": "evt_002",
    "timestamp": "2025-10-10T10:01:00Z",
    "event_type": "action",
    "actor": "agent:memory-systems-architect",
    "content": {"action": "analyze_chapter", "file": "Chapter 8_ Memory Management.md"},
    "state_delta": {"session:analysis_complete": true},
    "is_final_response": false
  }
]
```

**state JSON Structure Example:**
```json
{
  "session:abc123:current_task": "schema_redesign",
  "session:abc123:progress_steps": [1, 2, 3],
  "temp:validation_needed": true,
  "user:alice:last_interaction": "2025-10-10T09:00:00Z"
}
```

### 1.2 memory_entries Table

**Purpose:** Long-term persistent memory storage with hierarchical namespaces and versioning.

```sql
CREATE TABLE IF NOT EXISTS memory_entries (
    -- Primary key
    id INTEGER PRIMARY KEY AUTOINCREMENT,

    -- Hierarchical namespace (project:user:session pattern)
    namespace TEXT NOT NULL,                -- e.g., "user:alice:preferences" or "project:schema_redesign:status"
    key TEXT NOT NULL,                      -- Memory key (unique within namespace+version)

    -- Memory content
    value TEXT NOT NULL,                    -- JSON-serialized memory value
    memory_type TEXT NOT NULL,              -- semantic|episodic|procedural

    -- Versioning and soft-delete
    version INTEGER NOT NULL DEFAULT 1,     -- Version number (increments on update)
    is_deleted BOOLEAN NOT NULL DEFAULT 0,  -- Soft-delete flag (0=active, 1=deleted)

    -- Metadata
    metadata TEXT DEFAULT '{}',             -- JSON dict for extensibility (tags, importance_score, etc.)

    -- Audit trail
    created_by TEXT,                        -- Session or agent ID that created this entry
    updated_by TEXT,                        -- Session or agent ID that last updated this entry

    -- Timestamps
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,

    -- Constraints
    CHECK(memory_type IN ('semantic', 'episodic', 'procedural')),
    CHECK(json_valid(value)),
    CHECK(json_valid(metadata)),
    CHECK(version > 0),
    UNIQUE(namespace, key, version)          -- Enforce unique versions per key
);
```

**Example Entries:**

```sql
-- Semantic memory: User preference
INSERT INTO memory_entries (namespace, key, value, memory_type, created_by)
VALUES (
    'user:alice:preferences',
    'communication_style',
    '{"language": "concise", "technical_level": "expert", "code_comments": true}',
    'semantic',
    'session:abc123'
);

-- Episodic memory: Task execution history
INSERT INTO memory_entries (namespace, key, value, memory_type, created_by)
VALUES (
    'project:schema_redesign:task_history',
    'migration_attempt_2025-10-09',
    '{"task": "schema_migration", "approach": "full_migration", "outcome": "failed", "error": "data_loss_detected", "lesson": "require_rollback_capability"}',
    'episodic',
    'agent:database-redesign-specialist'
);

-- Procedural memory: Agent instructions
INSERT INTO memory_entries (namespace, key, value, memory_type, created_by)
VALUES (
    'app:abathur:agent_instructions',
    'schema_redesign_workflow',
    '{"steps": ["analyze_existing_schema", "design_new_tables", "create_migration_scripts", "test_migration", "execute_migration"], "error_handling": "Always create backup before migration"}',
    'procedural',
    'system'
);
```

### 1.3 document_index Table

**Purpose:** Index for markdown documents with embeddings for semantic search (hybrid storage model).

```sql
CREATE TABLE IF NOT EXISTS document_index (
    -- Primary key
    id INTEGER PRIMARY KEY AUTOINCREMENT,

    -- Document identification
    file_path TEXT NOT NULL UNIQUE,         -- Absolute path to markdown file (source of truth)
    title TEXT NOT NULL,                    -- Document title (extracted from front matter or first # heading)
    document_type TEXT,                     -- design|specification|plan|report (for categorization)

    -- Content tracking
    content_hash TEXT NOT NULL,             -- SHA-256 hash of file content (detect changes)
    chunk_count INTEGER DEFAULT 1,          -- Number of chunks this document was split into

    -- Embeddings (BLOB for future sqlite-vss integration)
    embedding_model TEXT,                   -- e.g., "nomic-embed-text-v1.5" (768 dims)
    embedding_blob BLOB,                    -- Serialized embedding vector (JSON array or binary format)

    -- Metadata
    metadata TEXT DEFAULT '{}',             -- JSON dict: author, tags, phase, project_id, etc.

    -- Sync tracking
    last_synced_at TIMESTAMP,               -- When embeddings were last generated
    sync_status TEXT DEFAULT 'pending',     -- pending|synced|failed|stale

    -- Timestamps
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,

    -- Constraints
    CHECK(sync_status IN ('pending', 'synced', 'failed', 'stale')),
    CHECK(json_valid(metadata))
);
```

**Example Entries:**

```sql
INSERT INTO document_index (file_path, title, document_type, content_hash, metadata, sync_status)
VALUES (
    '/Users/odgrim/dev/home/agentics/abathur/design_docs/phase1_design/memory-architecture.md',
    'Memory Architecture Design - Abathur Schema Redesign',
    'design',
    'a3f5e8b2c1d4...',  -- SHA-256 hash
    '{"author": "memory-systems-architect", "phase": "phase1_design", "project_id": "schema_redesign", "tags": ["memory", "architecture", "design"]}',
    'pending'  -- Embeddings not yet generated
);
```

**Embedding Storage Format (Future):**

```json
{
  "model": "nomic-embed-text-v1.5",
  "dimensions": 768,
  "vector": [0.123, -0.456, 0.789, ...],  // 768 float values
  "chunk_index": 0,  // For multi-chunk documents
  "chunk_text": "First 512 tokens of document..."
}
```

---

## 2. Enhanced Existing Tables

### 2.1 tasks Table (Enhanced)

**Changes:** Add `session_id` foreign key to link tasks with sessions.

```sql
-- Existing table structure (from database.py) with enhancements
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

    -- NEW: Session linkage
    session_id TEXT,                        -- Link task to session for context

    -- Foreign keys
    FOREIGN KEY (parent_task_id) REFERENCES tasks(id),
    FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE SET NULL
);
```

**Migration Note:** This is a fresh start project, so no migration needed. New table created with `session_id` from the start.

### 2.2 agents Table (Enhanced)

**Changes:** Add `session_id` foreign key to track which session spawned the agent.

```sql
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

    -- NEW: Session linkage
    session_id TEXT,                        -- Link agent to session

    -- Foreign keys
    FOREIGN KEY (task_id) REFERENCES tasks(id),
    FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE SET NULL
);
```

### 2.3 audit Table (Enhanced)

**Changes:** Add `memory_operation_type` column for tracking memory-specific operations.

```sql
CREATE TABLE IF NOT EXISTS audit (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    timestamp TIMESTAMP NOT NULL,
    agent_id TEXT,
    task_id TEXT NOT NULL,
    action_type TEXT NOT NULL,
    action_data TEXT,
    result TEXT,

    -- NEW: Memory operation tracking
    memory_operation_type TEXT,             -- create|update|delete|consolidate|publish (NULL for non-memory operations)
    memory_namespace TEXT,                  -- Namespace of affected memory (for filtering audit logs)
    memory_entry_id INTEGER,                -- Foreign key to memory_entries.id

    -- Foreign keys
    FOREIGN KEY (agent_id) REFERENCES agents(id),
    FOREIGN KEY (task_id) REFERENCES tasks(id),
    FOREIGN KEY (memory_entry_id) REFERENCES memory_entries(id) ON DELETE SET NULL
);
```

**Example Audit Entries:**

```sql
-- Memory creation audit
INSERT INTO audit (timestamp, agent_id, task_id, action_type, memory_operation_type, memory_namespace, memory_entry_id, action_data)
VALUES (
    CURRENT_TIMESTAMP,
    'agent:memory-systems-architect',
    'task:abc123',
    'memory_create',
    'create',
    'project:schema_redesign:architecture_complete',
    12345,  -- memory_entries.id
    '{"key": "memory_system_design", "memory_type": "procedural"}'
);

-- Memory consolidation audit
INSERT INTO audit (timestamp, agent_id, task_id, action_type, memory_operation_type, memory_namespace, action_data, result)
VALUES (
    CURRENT_TIMESTAMP,
    'system:consolidation_job',
    'task:consolidation',
    'memory_consolidate',
    'consolidate',
    'user:alice:preferences',
    '{"conflicting_ids": [101, 102, 103]}',
    '{"consolidated_id": 104, "strategy": "llm_based"}'
);
```

### 2.4 state Table (Deprecated but Maintained)

**Status:** Kept for backward compatibility, but functionality moved to `sessions.state`.

```sql
-- Existing table (unchanged, maintained for compatibility)
CREATE TABLE IF NOT EXISTS state (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    task_id TEXT NOT NULL,
    key TEXT NOT NULL,
    value TEXT NOT NULL,
    created_at TIMESTAMP NOT NULL,
    updated_at TIMESTAMP NOT NULL,
    UNIQUE(task_id, key),
    FOREIGN KEY (task_id) REFERENCES tasks(id)
);
```

**Migration Strategy:**
- Existing code can continue using `state` table
- New code uses `sessions.state` JSON column
- Deprecation warning in docs: "Use sessions.state instead"
- Remove in v2.1 after transition period

### 2.5 metrics and checkpoints Tables (Unchanged)

**No changes required:** These tables function independently of memory system.

```sql
-- metrics table (unchanged)
CREATE TABLE IF NOT EXISTS metrics (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    timestamp TIMESTAMP NOT NULL,
    metric_name TEXT NOT NULL,
    metric_value REAL NOT NULL,
    labels TEXT,
    CHECK(metric_value >= 0)
);

-- checkpoints table (enhanced with session_id)
CREATE TABLE IF NOT EXISTS checkpoints (
    task_id TEXT NOT NULL,
    iteration INTEGER NOT NULL,
    state TEXT NOT NULL,
    created_at TIMESTAMP NOT NULL,

    -- NEW: Session linkage (optional)
    session_id TEXT,

    PRIMARY KEY (task_id, iteration),
    FOREIGN KEY (task_id) REFERENCES tasks(id),
    FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE SET NULL
);
```

---

## 3. Table Relationships

### 3.1 Entity Relationship Summary

```
sessions (1) ────< (N) tasks         # One session can execute multiple tasks
sessions (1) ────< (N) agents        # One session can spawn multiple agents
sessions (1) ────< (N) checkpoints   # One session can have multiple checkpoints

tasks (1) ────< (N) agents           # One task can have multiple agents (swarm)
tasks (1) ────< (N) tasks            # Parent-child task relationships
tasks (1) ────< (N) audit            # One task generates multiple audit entries

agents (1) ────< (N) audit           # One agent generates multiple audit entries

memory_entries (1) ────< (N) audit   # One memory entry can have multiple audit logs

document_index (standalone)          # No direct FK relationships (indexed via file_path)
```

### 3.2 Cascade Rules

**sessions deletion:**
- `tasks.session_id` → SET NULL (tasks persist independently)
- `agents.session_id` → SET NULL (agents persist independently)
- `checkpoints.session_id` → SET NULL (checkpoints persist independently)

**tasks deletion:**
- Child tasks → NO CASCADE (preserve task hierarchy)
- Audit entries → NO CASCADE (preserve audit trail)

**memory_entries deletion:**
- Audit entries → SET NULL (preserve audit trail)

**Rationale:** Preserve audit and historical data even when parent entities are deleted.

---

## 4. Data Types and Constraints

### 4.1 SQLite Data Types Used

| Type | Usage | Example |
|------|-------|---------|
| TEXT | IDs, JSON, strings | session.id, memory_entries.value |
| INTEGER | Counters, versions | memory_entries.version |
| REAL | Metrics | metrics.metric_value |
| BLOB | Binary data | document_index.embedding_blob |
| TIMESTAMP | Dates/times | sessions.created_at |
| BOOLEAN | Flags | memory_entries.is_deleted (0 or 1) |

### 4.2 JSON Validation Constraints

All JSON columns include `CHECK(json_valid(...))` constraints:

```sql
CHECK(json_valid(events))              -- sessions.events
CHECK(json_valid(state))               -- sessions.state
CHECK(json_valid(metadata))            -- sessions.metadata, memory_entries.metadata, document_index.metadata
CHECK(json_valid(value))               -- memory_entries.value
```

**Rationale:** Prevent corrupt JSON from being inserted, ensures data integrity.

### 4.3 UNIQUE Constraints

```sql
-- document_index: file_path must be unique
file_path TEXT NOT NULL UNIQUE

-- memory_entries: Unique versions per namespace+key
UNIQUE(namespace, key, version)

-- state (deprecated): Unique task+key
UNIQUE(task_id, key)
```

### 4.4 CHECK Constraints

```sql
-- sessions.status: Enforce valid lifecycle states
CHECK(status IN ('created', 'active', 'paused', 'terminated', 'archived'))

-- memory_entries.memory_type: Enforce valid memory types
CHECK(memory_type IN ('semantic', 'episodic', 'procedural'))

-- memory_entries.version: Versions must be positive
CHECK(version > 0)

-- document_index.sync_status: Enforce valid sync states
CHECK(sync_status IN ('pending', 'synced', 'failed', 'stale'))

-- metrics.metric_value: Metrics cannot be negative
CHECK(metric_value >= 0)
```

### 4.5 Default Values

```sql
-- Lifecycle defaults
status TEXT NOT NULL DEFAULT 'created'
version INTEGER NOT NULL DEFAULT 1
is_deleted BOOLEAN NOT NULL DEFAULT 0
sync_status TEXT DEFAULT 'pending'

-- JSON defaults
events TEXT NOT NULL DEFAULT '[]'
state TEXT NOT NULL DEFAULT '{}'
metadata TEXT DEFAULT '{}'

-- Timestamp defaults
created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
```

---

## 5. Complete DDL Script

**Location:** See `migration-strategy.md` for complete DDL generation script.

**Execution Order:**
1. Create `sessions` table (no dependencies)
2. Create `memory_entries` table (no dependencies)
3. Create `document_index` table (no dependencies)
4. Alter `tasks` table (add `session_id` FK)
5. Alter `agents` table (add `session_id` FK)
6. Alter `audit` table (add memory columns)
7. Alter `checkpoints` table (add `session_id` FK)
8. Create all indexes (see `schema-indexes.md`)

**PRAGMA Settings:**

```sql
PRAGMA journal_mode = WAL;           -- Concurrent reads
PRAGMA synchronous = NORMAL;          -- Balance safety/performance
PRAGMA foreign_keys = ON;             -- Enforce FK constraints
PRAGMA busy_timeout = 5000;           -- Wait 5s for locks
PRAGMA wal_autocheckpoint = 1000;     -- Checkpoint every 1000 pages
```

---

## 6. Table Size Estimates

**Assumptions:**
- 50 concurrent sessions (active)
- 10,000 total sessions (archived)
- 100,000 memory entries (semantic: 40%, episodic: 50%, procedural: 10%)
- 1,000 documents indexed

**Storage Estimates:**

| Table | Rows | Avg Row Size | Total Size |
|-------|------|--------------|------------|
| sessions | 10,000 | 5 KB (events+state JSON) | 50 MB |
| memory_entries | 100,000 | 2 KB (JSON value) | 200 MB |
| document_index | 1,000 | 10 KB (embeddings) | 10 MB |
| tasks | 50,000 | 1 KB | 50 MB |
| agents | 50,000 | 0.5 KB | 25 MB |
| audit | 500,000 | 0.5 KB | 250 MB |
| Total | - | - | **~585 MB** |

**10GB Target:** Well within limit, room for growth to ~17x current size.

---

## 7. Data Integrity Measures

### 7.1 Foreign Key Enforcement

```sql
PRAGMA foreign_keys = ON;  -- Enabled at database init
```

**Enforced Relationships:**
- tasks.session_id → sessions.id
- agents.session_id → sessions.id
- audit.memory_entry_id → memory_entries.id

### 7.2 Transaction Isolation

**All write operations use transactions:**

```python
async with conn.begin():  # Start transaction
    await conn.execute("INSERT INTO memory_entries ...")
    await conn.execute("INSERT INTO audit ...")
    # Both succeed or both rollback (ACID compliance)
```

### 7.3 Soft-Delete Pattern

**Never hard-delete critical data:**
- memory_entries: `is_deleted=1` instead of DELETE
- sessions: Move to `status='archived'` instead of DELETE

**Benefits:**
- Preserve audit trail
- Enable rollback/restore
- Simplify conflict resolution

---

**Document Version:** 1.0
**Author:** memory-systems-architect (orchestrated)
**Date:** 2025-10-10
**Status:** Phase 1 Design - Awaiting Validation
**Next Document:** schema-relationships.md (ER diagrams)
