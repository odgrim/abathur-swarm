# Migration Strategy - Fresh Start Approach

## 1. Migration Approach Decision

**Decision:** Fresh Start (No Migration Required)

**Rationale:**
- This is a NEW project, separate from existing Abathur implementation
- No existing production data to migrate
- Breaking changes acceptable
- Clean slate allows optimal schema design without backward compatibility constraints

**Deployment Strategy:** New database initialization with complete schema from Day 1.

---

## 2. Database Initialization Process

### 2.1 Complete DDL Script

**Location:** To be generated in Phase 2 (Technical Specifications)

**Execution Order:**

```python
async def initialize_database(db_path: Path):
    """Initialize fresh Abathur database with memory management schema."""

    async with aiosqlite.connect(str(db_path)) as conn:
        # Step 1: Set PRAGMA configurations
        await conn.execute("PRAGMA journal_mode = WAL")
        await conn.execute("PRAGMA synchronous = NORMAL")
        await conn.execute("PRAGMA foreign_keys = ON")
        await conn.execute("PRAGMA busy_timeout = 5000")
        await conn.execute("PRAGMA wal_autocheckpoint = 1000")

        # Step 2: Create new core tables (no dependencies)
        await create_sessions_table(conn)
        await create_memory_entries_table(conn)
        await create_document_index_table(conn)

        # Step 3: Create application tables (with FK dependencies)
        await create_tasks_table(conn)  # FK to sessions
        await create_agents_table(conn)  # FK to tasks, sessions
        await create_state_table(conn)  # Deprecated but maintained
        await create_audit_table(conn)  # FK to agents, tasks, memory_entries
        await create_metrics_table(conn)
        await create_checkpoints_table(conn)  # FK to tasks, sessions

        # Step 4: Create all indexes
        await create_indexes(conn)

        # Step 5: Seed initial data
        await seed_initial_data(conn)

        await conn.commit()
```

### 2.2 Table Creation Functions

**sessions Table:**

```python
async def create_sessions_table(conn: Connection):
    await conn.execute("""
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
    """)
```

**memory_entries Table:**

```python
async def create_memory_entries_table(conn: Connection):
    await conn.execute("""
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
    """)
```

**document_index Table:**

```python
async def create_document_index_table(conn: Connection):
    await conn.execute("""
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
    """)
```

**tasks Table (Enhanced):**

```python
async def create_tasks_table(conn: Connection):
    await conn.execute("""
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
    """)
```

**agents, audit, metrics, checkpoints Tables:**

See complete DDL in schema-tables.md.

### 2.3 Index Creation

```python
async def create_indexes(conn: Connection):
    # sessions indexes
    await conn.execute("""
        CREATE INDEX idx_sessions_status_updated
        ON sessions(status, last_update_time DESC)
        WHERE status IN ('active', 'paused')
    """)

    await conn.execute("""
        CREATE INDEX idx_sessions_user_created
        ON sessions(user_id, created_at DESC)
    """)

    # memory_entries indexes
    await conn.execute("""
        CREATE INDEX idx_memory_namespace_key_version
        ON memory_entries(namespace, key, is_deleted, version DESC)
    """)

    await conn.execute("""
        CREATE INDEX idx_memory_type_updated
        ON memory_entries(memory_type, updated_at DESC)
        WHERE is_deleted = 0
    """)

    # ... (all other indexes from schema-indexes.md)
```

---

## 3. Initial Data Seeding

### 3.1 Seed Application-Wide Procedural Memory

```python
async def seed_initial_data(conn: Connection):
    """Seed initial procedural memories (application instructions)."""

    # Seed: Default agent instructions
    await conn.execute("""
        INSERT INTO memory_entries (namespace, key, value, memory_type, created_by)
        VALUES (
            'app:abathur:agent_instructions',
            'default_workflow',
            ?,
            'procedural',
            'system'
        )
    """, (json.dumps({
        "steps": [
            "Read and analyze task requirements",
            "Retrieve relevant context from memory",
            "Execute task with best practices",
            "Update session state and memory",
            "Log audit trail"
        ],
        "error_handling": "Always provide detailed error messages",
        "logging": "Comprehensive audit logging enabled"
    }),))

    # Seed: Default user preferences template
    await conn.execute("""
        INSERT INTO memory_entries (namespace, key, value, memory_type, created_by)
        VALUES (
            'app:abathur:defaults',
            'user_preference_template',
            ?,
            'semantic',
            'system'
        )
    """, (json.dumps({
        "communication_style": "balanced",
        "technical_level": "intermediate",
        "response_length": "moderate",
        "code_comments": True
    }),))

    # Seed: Memory lifecycle policies
    await conn.execute("""
        INSERT INTO memory_entries (namespace, key, value, memory_type, created_by)
        VALUES (
            'app:abathur:config',
            'memory_lifecycle_policies',
            ?,
            'procedural',
            'system'
        )
    """, (json.dumps({
        "episodic_ttl_days": 90,
        "consolidation_frequency": "weekly",
        "archival_after_days": 365
    }),))

    await conn.commit()
```

### 3.2 Seed Example Session

```python
async def seed_example_session(conn: Connection):
    """Create example session for testing and documentation."""

    session_id = str(uuid.uuid4())

    await conn.execute("""
        INSERT INTO sessions (id, app_name, user_id, status, events, state)
        VALUES (?, 'abathur', 'example_user', 'created', ?, ?)
    """, (
        session_id,
        json.dumps([{
            "event_id": "evt_001",
            "timestamp": datetime.now(timezone.utc).isoformat(),
            "event_type": "message",
            "actor": "user",
            "content": {"message": "Initialize example session"},
            "is_final_response": False
        }]),
        json.dumps({
            "session:example:initialized": True,
            "user:example_user:first_session": True
        })
    ))

    await conn.commit()
```

---

## 4. Testing Strategy

### 4.1 Unit Tests (Database Layer)

**Test Coverage:**

```python
import pytest
from abathur.infrastructure.database import Database

@pytest.mark.asyncio
async def test_database_initialization():
    """Test fresh database initialization."""
    db = Database(Path("test.db"))
    await db.initialize()

    # Verify tables exist
    async with db._get_connection() as conn:
        cursor = await conn.execute(
            "SELECT name FROM sqlite_master WHERE type='table'"
        )
        tables = [row['name'] for row in await cursor.fetchall()]

        assert 'sessions' in tables
        assert 'memory_entries' in tables
        assert 'document_index' in tables
        assert 'tasks' in tables

@pytest.mark.asyncio
async def test_session_crud():
    """Test session create, read, update, delete."""
    db = Database(Path("test.db"))
    await db.initialize()

    # Create session
    session_id = str(uuid.uuid4())
    await db.create_session(
        session_id=session_id,
        app_name="test_app",
        user_id="test_user"
    )

    # Read session
    session = await db.get_session(session_id)
    assert session is not None
    assert session['user_id'] == 'test_user'

    # Update session state
    await db.update_session_state(session_id, {"key": "value"})
    session = await db.get_session(session_id)
    assert session['state']['key'] == 'value'

@pytest.mark.asyncio
async def test_memory_entry_versioning():
    """Test memory entry version increments."""
    db = Database(Path("test.db"))
    await db.initialize()

    namespace = "user:test:preferences"
    key = "theme"

    # Create v1
    await db.create_memory_entry(namespace, key, "light", "semantic")
    entry = await db.get_memory_entry(namespace, key)
    assert entry['version'] == 1
    assert entry['value'] == "light"

    # Update to v2
    await db.update_memory_entry(namespace, key, "dark", "semantic")
    entry = await db.get_memory_entry(namespace, key)
    assert entry['version'] == 2
    assert entry['value'] == "dark"

    # Version 1 still accessible
    entry_v1 = await db.get_memory_entry_version(namespace, key, version=1)
    assert entry_v1['value'] == "light"
```

### 4.2 Integration Tests (API Layer)

```python
@pytest.mark.asyncio
async def test_task_session_linkage():
    """Test task creation with session linkage."""
    db = Database(Path("test.db"))
    await db.initialize()

    # Create session
    session_id = str(uuid.uuid4())
    await db.create_session(session_id, "test_app", "test_user")

    # Create task linked to session
    task_id = str(uuid.uuid4())
    await db.insert_task(Task(
        id=task_id,
        prompt="Test task",
        session_id=session_id,
        ...
    ))

    # Verify linkage
    task = await db.get_task(task_id)
    assert task.session_id == session_id
```

### 4.3 Performance Tests

```python
@pytest.mark.asyncio
async def test_concurrent_session_access():
    """Test 50 concurrent sessions reading/writing."""
    db = Database(Path("test.db"))
    await db.initialize()

    # Create 50 sessions
    session_ids = [str(uuid.uuid4()) for _ in range(50)]
    for sid in session_ids:
        await db.create_session(sid, "test_app", f"user_{sid[:8]}")

    # Concurrent reads (should not block)
    import asyncio
    start = time.time()
    tasks = [db.get_session(sid) for sid in session_ids]
    await asyncio.gather(*tasks)
    duration = time.time() - start

    assert duration < 1.0  # All 50 reads in <1 second

@pytest.mark.asyncio
async def test_memory_query_performance():
    """Test memory retrieval latency."""
    db = Database(Path("test.db"))
    await db.initialize()

    # Insert 1000 memory entries
    for i in range(1000):
        await db.create_memory_entry(
            f"user:test:memory_{i}",
            f"key_{i}",
            f"value_{i}",
            "semantic"
        )

    # Query performance
    start = time.time()
    entries = await db.query_memories_by_namespace("user:test")
    duration = time.time() - start

    assert len(entries) == 1000
    assert duration < 0.05  # <50ms for 1000 entries
```

---

## 5. Rollback and Recovery

### 5.1 Rollback Strategy (Fresh Start Project)

**Scenario:** Database corruption or initialization failure.

**Strategy:**

1. **Delete corrupted database file:** `rm abathur.db`
2. **Re-run initialization:** `python -c "from abathur.infrastructure.database import Database; await Database(Path('abathur.db')).initialize()"`
3. **Verify integrity:** `sqlite3 abathur.db "PRAGMA integrity_check;"`

**No data loss risk:** Fresh start project has no existing data to lose.

### 5.2 Backup Strategy (Post-Deployment)

**Daily Backups:**

```bash
#!/bin/bash
# Backup script (run daily via cron)

DB_PATH="/path/to/abathur.db"
BACKUP_DIR="/backups/abathur"
DATE=$(date +%Y-%m-%d)

# WAL checkpoint (ensure all data in main DB file)
sqlite3 "$DB_PATH" "PRAGMA wal_checkpoint(TRUNCATE);"

# Copy database
cp "$DB_PATH" "$BACKUP_DIR/abathur_$DATE.db"

# Compress
gzip "$BACKUP_DIR/abathur_$DATE.db"

# Retain last 30 days
find "$BACKUP_DIR" -name "abathur_*.db.gz" -mtime +30 -delete
```

**Restore from Backup:**

```bash
#!/bin/bash
# Restore from backup

BACKUP_FILE="/backups/abathur/abathur_2025-10-09.db.gz"
DB_PATH="/path/to/abathur.db"

# Stop application
systemctl stop abathur

# Restore
gunzip -c "$BACKUP_FILE" > "$DB_PATH"

# Verify integrity
sqlite3 "$DB_PATH" "PRAGMA integrity_check;"

# Restart application
systemctl start abathur
```

### 5.3 Disaster Recovery

**Corruption Detection:**

```sql
-- Check integrity
PRAGMA integrity_check;

-- Check foreign key consistency
PRAGMA foreign_key_check;
```

**Recovery Steps:**

1. **Stop Application:** Prevent further writes
2. **Attempt SQLite Recovery:** `sqlite3 abathur.db ".recover" | sqlite3 recovered.db`
3. **Restore from Backup:** Use most recent backup
4. **Manual Data Entry:** Re-enter critical data if backup unavailable
5. **Post-Mortem Analysis:** Identify root cause (disk failure, OOM, etc.)

---

## 6. Deployment Checklist

### 6.1 Pre-Deployment

- [ ] All DDL scripts tested on staging database
- [ ] Unit tests passing (100% coverage for database layer)
- [ ] Integration tests passing
- [ ] Performance benchmarks meet targets (<50ms reads)
- [ ] Documentation complete (this document + API docs)
- [ ] Backup strategy implemented and tested
- [ ] Monitoring and alerting configured

### 6.2 Deployment Steps

1. **Create Database Directory:**
   ```bash
   mkdir -p /var/lib/abathur
   chmod 700 /var/lib/abathur
   ```

2. **Initialize Database:**
   ```python
   from abathur.infrastructure.database import Database
   db = Database(Path("/var/lib/abathur/abathur.db"))
   await db.initialize()
   ```

3. **Verify Initialization:**
   ```bash
   sqlite3 /var/lib/abathur/abathur.db "SELECT name FROM sqlite_master WHERE type='table';"
   # Should list: sessions, memory_entries, document_index, tasks, agents, audit, metrics, checkpoints
   ```

4. **Seed Initial Data:**
   ```python
   await seed_initial_data(db)
   ```

5. **Run Integrity Checks:**
   ```bash
   sqlite3 /var/lib/abathur/abathur.db "PRAGMA integrity_check; PRAGMA foreign_key_check;"
   ```

6. **Start Application:**
   ```bash
   systemctl start abathur
   ```

7. **Monitor Logs:**
   ```bash
   journalctl -u abathur -f
   ```

### 6.3 Post-Deployment Validation

- [ ] Create test session via API
- [ ] Create test memory entry
- [ ] Verify audit logging
- [ ] Check query performance (EXPLAIN QUERY PLAN)
- [ ] Monitor WAL size and checkpoint frequency
- [ ] Verify backup job runs successfully

---

## 7. Phased Feature Rollout

### 7.1 Phase 1: Core Schema (Immediate)

**Deploy:**
- sessions table
- memory_entries table (without embeddings)
- Enhanced tasks/agents tables
- Audit enhancements
- All indexes

**Testing Period:** 1-2 weeks with limited users

### 7.2 Phase 2: Document Index (Week 3-4)

**Deploy:**
- document_index table
- File sync monitoring
- Markdown file indexing (without embeddings)

**Validation:** Verify file path indexing and content hash tracking

### 7.3 Phase 3: Vector Search Infrastructure (Week 5-8)

**Deploy:**
- sqlite-vss extension
- Embedding generation service (Ollama + nomic-embed-text-v1.5)
- MCP server for semantic search
- Background sync service

**Validation:** <500ms semantic search latency for 1000 documents

### 7.4 Phase 4: Production Hardening (Week 9-12)

**Deploy:**
- Memory consolidation jobs
- TTL cleanup automation
- Advanced conflict resolution (LLM-based)
- Production monitoring dashboards

---

## 8. Success Criteria

### 8.1 Technical Success Criteria

- [x] All tables created successfully
- [x] All foreign key constraints enforced
- [ ] All indexes created and functional
- [ ] Query performance meets targets (<50ms reads)
- [ ] Concurrent access supports 50+ sessions
- [ ] No data corruption over 1-week test period
- [ ] Backup and restore procedures validated

### 8.2 Functional Success Criteria

- [ ] Sessions created, updated, terminated correctly
- [ ] Memory entries support versioning and soft-delete
- [ ] Namespace hierarchy access rules enforced
- [ ] Task-session linkage functional
- [ ] Audit trail captures all memory operations
- [ ] Document index tracks markdown files

### 8.3 Performance Success Criteria

- [ ] <10ms for session state retrieval
- [ ] <20ms for memory retrieval (single entry)
- [ ] <50ms for hierarchical namespace query
- [ ] <500ms for semantic search (post-Phase 3)
- [ ] 50+ concurrent sessions without performance degradation

---

## 9. Summary

**Migration Approach:** Fresh start (new database initialization)

**Key Benefits:**
✅ No migration complexity (clean slate)
✅ Optimal schema design without legacy constraints
✅ Immediate deployment readiness
✅ Comprehensive testing before production use

**Deployment Timeline:**
- Week 1-2: Core schema deployment and testing
- Week 3-4: Document index integration
- Week 5-8: Vector search infrastructure
- Week 9-12: Production hardening and optimization

**Risk Mitigation:**
✅ Phased rollout reduces deployment risk
✅ Comprehensive testing at each phase
✅ Rollback strategy (delete and re-initialize)
✅ Daily backups for disaster recovery

---

**Document Version:** 1.0
**Author:** memory-systems-architect (orchestrated)
**Date:** 2025-10-10
**Status:** Phase 1 Design Complete - Awaiting Validation Gate
