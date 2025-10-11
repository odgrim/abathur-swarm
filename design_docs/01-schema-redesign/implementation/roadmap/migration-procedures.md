# Migration Procedures - Fresh Start Initialization

## Overview

This document provides step-by-step procedures for initializing the SQLite database for the Abathur memory management system.

**Migration Strategy:** Fresh start (new project, no data migration required)

**Target Environment:** Development, staging, and production

---

## 1. Pre-Migration Checklist

### 1.1 System Requirements Verification

**Required:**
- [x] Python 3.11+ installed
- [x] SQLite 3.35+ installed
- [x] aiosqlite package available
- [x] Disk space: 500MB minimum (10GB recommended)
- [x] File system: WAL mode compatible (not NFS)

**Verification Commands:**
```bash
# Check Python version
python --version  # Expected: 3.11 or higher

# Check SQLite version
sqlite3 --version  # Expected: 3.35.0 or higher

# Check disk space
df -h /var/lib/abathur  # Expected: 10GB+ free

# Check file system type
df -T /var/lib/abathur  # Expected: ext4, btrfs, apfs (not NFS)
```

### 1.2 Environment Preparation

**Create Database Directory:**
```bash
# Production
sudo mkdir -p /var/lib/abathur
sudo chown abathur:abathur /var/lib/abathur
sudo chmod 700 /var/lib/abathur

# Development
mkdir -p ~/abathur_data
```

**Backup Existing Database (if any):**
```bash
# Only if upgrading from existing system
if [ -f /var/lib/abathur/abathur.db ]; then
    cp /var/lib/abathur/abathur.db /backups/abathur_pre_migration_$(date +%Y%m%d_%H%M%S).db
fi
```

---

## 2. Database Initialization Procedure

### 2.1 Initialize Database File

**Step 1: Create Database Connection**

```python
import aiosqlite
from pathlib import Path

DB_PATH = Path("/var/lib/abathur/abathur.db")

async def initialize_database():
    """Initialize fresh Abathur database."""
    async with aiosqlite.connect(str(DB_PATH)) as conn:
        # Enable row factory for dict-like access
        conn.row_factory = aiosqlite.Row

        # Step 1: Set PRAGMA configuration
        await configure_database_pragmas(conn)

        # Step 2: Create memory management tables
        await create_memory_tables(conn)

        # Step 3: Create enhanced core tables
        await create_core_tables(conn)

        # Step 4: Create performance indexes
        await create_indexes(conn)

        # Step 5: Seed initial data
        await seed_initial_data(conn)

        # Step 6: Validate initialization
        await validate_database(conn)

        await conn.commit()
        print("Database initialized successfully")
```

### 2.2 Configure PRAGMA Settings

**Step 2: Set Critical PRAGMA Settings**

```python
async def configure_database_pragmas(conn: aiosqlite.Connection):
    """Configure SQLite PRAGMA settings for optimal performance."""

    # WAL mode: Enable concurrent reads
    await conn.execute("PRAGMA journal_mode = WAL")

    # Synchronous: Balance safety and performance
    await conn.execute("PRAGMA synchronous = NORMAL")

    # Foreign keys: Enforce referential integrity
    await conn.execute("PRAGMA foreign_keys = ON")

    # Busy timeout: Wait 5 seconds for locks
    await conn.execute("PRAGMA busy_timeout = 5000")

    # WAL autocheckpoint: Checkpoint every 1000 pages
    await conn.execute("PRAGMA wal_autocheckpoint = 1000")

    # Cache size: 64MB cache for better performance
    await conn.execute("PRAGMA cache_size = -64000")

    # Temp store: Use memory for temporary tables
    await conn.execute("PRAGMA temp_store = MEMORY")

    # Memory-mapped I/O: 256MB for faster reads
    await conn.execute("PRAGMA mmap_size = 268435456")

    # Verify WAL mode enabled
    cursor = await conn.execute("PRAGMA journal_mode")
    mode = await cursor.fetchone()
    assert mode[0] == 'wal', f"WAL mode not enabled: {mode[0]}"

    print("PRAGMA configuration complete")
```

### 2.3 Create Memory Tables

**Step 3: Execute DDL for Memory Tables**

```python
async def create_memory_tables(conn: aiosqlite.Connection):
    """Create sessions, memory_entries, and document_index tables."""

    # Read DDL from file
    ddl_path = Path("design_docs/phase2_tech_specs/ddl-memory-tables.sql")
    ddl_content = ddl_path.read_text()

    # Execute each CREATE TABLE statement
    statements = ddl_content.split(';')
    for statement in statements:
        statement = statement.strip()
        if statement and not statement.startswith('--'):
            await conn.execute(statement)

    print("Memory tables created: sessions, memory_entries, document_index")
```

### 2.4 Create Enhanced Core Tables

**Step 4: Execute DDL for Core Tables**

```python
async def create_core_tables(conn: aiosqlite.Connection):
    """Create enhanced tasks, agents, audit, checkpoints, state, metrics tables."""

    # Read DDL from file
    ddl_path = Path("design_docs/phase2_tech_specs/ddl-core-tables.sql")
    ddl_content = ddl_path.read_text()

    # Execute each CREATE TABLE statement
    statements = ddl_content.split(';')
    for statement in statements:
        statement = statement.strip()
        if statement and not statement.startswith('--'):
            await conn.execute(statement)

    print("Core tables created: tasks, agents, audit, checkpoints, state, metrics")
```

### 2.5 Create Performance Indexes

**Step 5: Execute DDL for Indexes**

```python
async def create_indexes(conn: aiosqlite.Connection):
    """Create all 33 performance indexes."""

    # Read index DDL from file
    ddl_path = Path("design_docs/phase2_tech_specs/ddl-indexes.sql")
    ddl_content = ddl_path.read_text()

    # Execute each CREATE INDEX statement
    statements = ddl_content.split(';')
    for statement in statements:
        statement = statement.strip()
        if statement and not statement.startswith('--'):
            await conn.execute(statement)

    # Verify index count
    cursor = await conn.execute(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='index'"
    )
    index_count = (await cursor.fetchone())[0]
    print(f"Indexes created: {index_count}")
```

### 2.6 Seed Initial Data

**Step 6: Insert Initial Application Data**

```python
import json
from datetime import datetime, timezone

async def seed_initial_data(conn: aiosqlite.Connection):
    """Seed initial procedural and semantic memories."""

    # Seed: Default agent workflow
    await conn.execute(
        """
        INSERT INTO memory_entries (namespace, key, value, memory_type, created_by, updated_by)
        VALUES (?, ?, ?, ?, 'system', 'system')
        """,
        (
            'app:abathur:agent_instructions',
            'default_workflow',
            json.dumps({
                "steps": [
                    "Read and analyze task requirements",
                    "Retrieve relevant context from memory",
                    "Execute task with best practices",
                    "Update session state and memory",
                    "Log comprehensive audit trail"
                ],
                "error_handling": "Always provide detailed error messages and rollback on failure",
                "logging": "Comprehensive audit logging enabled"
            }),
            'procedural'
        )
    )

    # Seed: Memory lifecycle policies
    await conn.execute(
        """
        INSERT INTO memory_entries (namespace, key, value, memory_type, created_by, updated_by)
        VALUES (?, ?, ?, ?, 'system', 'system')
        """,
        (
            'app:abathur:config',
            'memory_lifecycle_policies',
            json.dumps({
                "episodic_ttl_days": 90,
                "semantic_permanent": True,
                "procedural_versioned": True,
                "consolidation_frequency": "weekly",
                "archival_after_days": 365
            }),
            'procedural'
        )
    )

    # Seed: Default user preference template
    await conn.execute(
        """
        INSERT INTO memory_entries (namespace, key, value, memory_type, created_by, updated_by)
        VALUES (?, ?, ?, ?, 'system', 'system')
        """,
        (
            'app:abathur:defaults',
            'user_preference_template',
            json.dumps({
                "communication_style": "balanced",
                "technical_level": "intermediate",
                "response_length": "moderate",
                "code_comments": True
            }),
            'semantic'
        )
    )

    await conn.commit()
    print("Initial data seeded: 3 application memories")
```

### 2.7 Validate Initialization

**Step 7: Run Integrity Checks**

```python
async def validate_database(conn: aiosqlite.Connection):
    """Validate database initialization."""

    # Check 1: SQLite integrity
    cursor = await conn.execute("PRAGMA integrity_check")
    result = await cursor.fetchone()
    assert result[0] == 'ok', f"Integrity check failed: {result[0]}"
    print("✓ SQLite integrity check passed")

    # Check 2: Foreign key consistency
    cursor = await conn.execute("PRAGMA foreign_key_check")
    violations = await cursor.fetchall()
    assert len(violations) == 0, f"Foreign key violations: {violations}"
    print("✓ Foreign key consistency verified")

    # Check 3: Verify tables exist
    expected_tables = [
        'sessions', 'memory_entries', 'document_index',
        'tasks', 'agents', 'audit', 'checkpoints', 'state', 'metrics'
    ]

    cursor = await conn.execute(
        "SELECT name FROM sqlite_master WHERE type='table' ORDER BY name"
    )
    tables = [row[0] for row in await cursor.fetchall()]

    for table in expected_tables:
        assert table in tables, f"Table {table} not created"
    print(f"✓ All {len(expected_tables)} tables created")

    # Check 4: Verify indexes exist
    cursor = await conn.execute(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='index'"
    )
    index_count = (await cursor.fetchone())[0]
    assert index_count >= 30, f"Expected 30+ indexes, found {index_count}"
    print(f"✓ {index_count} indexes created")

    # Check 5: Verify seed data
    cursor = await conn.execute(
        "SELECT COUNT(*) FROM memory_entries WHERE namespace LIKE 'app:abathur:%'"
    )
    seed_count = (await cursor.fetchone())[0]
    assert seed_count >= 3, f"Expected 3 seed memories, found {seed_count}"
    print(f"✓ {seed_count} initial memories seeded")
```

---

## 3. Environment-Specific Procedures

### 3.1 Development Environment

**Database Location:** `~/abathur_data/abathur.db`

**Initialization Command:**
```bash
cd ~/abathur_data
python -m abathur.infrastructure.database --initialize
```

**Verification:**
```bash
sqlite3 ~/abathur_data/abathur.db "SELECT name FROM sqlite_master WHERE type='table';"
```

### 3.2 Staging Environment

**Database Location:** `/var/lib/abathur-staging/abathur.db`

**Initialization Command:**
```bash
sudo -u abathur python -m abathur.infrastructure.database --initialize --env=staging
```

**Verification:**
```bash
sudo -u abathur sqlite3 /var/lib/abathur-staging/abathur.db "PRAGMA integrity_check;"
```

### 3.3 Production Environment

**Database Location:** `/var/lib/abathur/abathur.db`

**Pre-Production Checklist:**
- [ ] Staging environment validated
- [ ] All tests passing (unit, integration, performance)
- [ ] Backup automation configured
- [ ] Monitoring dashboards ready
- [ ] Rollback procedures tested

**Initialization Command:**
```bash
sudo -u abathur python -m abathur.infrastructure.database --initialize --env=production
```

**Post-Initialization Validation:**
```bash
# Integrity check
sudo -u abathur sqlite3 /var/lib/abathur/abathur.db "PRAGMA integrity_check;"

# Foreign key check
sudo -u abathur sqlite3 /var/lib/abathur/abathur.db "PRAGMA foreign_key_check;"

# Table count
sudo -u abathur sqlite3 /var/lib/abathur/abathur.db "SELECT COUNT(*) FROM sqlite_master WHERE type='table';"

# Index count
sudo -u abathur sqlite3 /var/lib/abathur/abathur.db "SELECT COUNT(*) FROM sqlite_master WHERE type='index';"
```

---

## 4. Data Validation Procedures

### 4.1 Schema Validation

**Verify Table Schemas:**
```sql
-- Check sessions table schema
.schema sessions

-- Expected output:
-- CREATE TABLE sessions (
--     id TEXT PRIMARY KEY,
--     app_name TEXT NOT NULL,
--     ...
-- );

-- Check memory_entries table schema
.schema memory_entries

-- Verify constraints
SELECT sql FROM sqlite_master WHERE type='table' AND name='sessions';
```

### 4.2 Index Validation

**Verify Index Usage:**
```sql
-- Test session query uses index
EXPLAIN QUERY PLAN
SELECT * FROM sessions WHERE status = 'active' ORDER BY last_update_time DESC;

-- Expected: USING INDEX idx_sessions_status_updated

-- Test memory query uses index
EXPLAIN QUERY PLAN
SELECT * FROM memory_entries
WHERE namespace = 'user:alice:pref' AND key = 'theme' AND is_deleted = 0
ORDER BY version DESC LIMIT 1;

-- Expected: USING INDEX idx_memory_namespace_key_version
```

### 4.3 Data Integrity Validation

**Verify Seed Data:**
```sql
-- Check application memories
SELECT namespace, key, memory_type
FROM memory_entries
WHERE namespace LIKE 'app:abathur:%';

-- Expected: 3 rows (default_workflow, memory_lifecycle_policies, user_preference_template)

-- Verify JSON validity
SELECT key, json_valid(value) as is_valid
FROM memory_entries
WHERE namespace LIKE 'app:abathur:%';

-- Expected: All rows return is_valid=1
```

---

## 5. Performance Baseline Testing

### 5.1 Query Performance Tests

**Session Retrieval Benchmark:**
```python
import time

async def benchmark_session_retrieval(db: Database):
    """Benchmark session retrieval latency."""
    # Create test session
    await db.sessions.create_session("test_123", "app", "user")

    # Benchmark
    latencies = []
    for _ in range(100):
        start = time.time()
        await db.sessions.get_session("test_123")
        latencies.append((time.time() - start) * 1000)

    p99 = sorted(latencies)[98]
    print(f"Session retrieval p99: {p99:.2f}ms (target <10ms)")
    assert p99 < 10, f"Performance target not met: {p99:.2f}ms"
```

**Memory Retrieval Benchmark:**
```python
async def benchmark_memory_retrieval(db: Database):
    """Benchmark memory retrieval latency."""
    # Create test memory
    await db.memory.add_memory(
        "user:alice:pref", "theme", {"mode": "dark"},
        "semantic", "session:abc", "task:xyz"
    )

    # Benchmark
    latencies = []
    for _ in range(100):
        start = time.time()
        await db.memory.get_memory("user:alice:pref", "theme")
        latencies.append((time.time() - start) * 1000)

    p99 = sorted(latencies)[98]
    print(f"Memory retrieval p99: {p99:.2f}ms (target <20ms)")
    assert p99 < 20, f"Performance target not met: {p99:.2f}ms"
```

### 5.2 Concurrent Access Tests

**50+ Concurrent Sessions:**
```python
import asyncio

async def benchmark_concurrent_access(db: Database):
    """Test 50+ concurrent sessions."""
    # Create 50 sessions
    session_ids = [f"sess_{i}" for i in range(50)]
    for sid in session_ids:
        await db.sessions.create_session(sid, "app", f"user_{sid}")

    # Concurrent reads
    start = time.time()
    tasks = [db.sessions.get_session(sid) for sid in session_ids]
    results = await asyncio.gather(*tasks)
    duration = time.time() - start

    print(f"50 concurrent reads: {duration:.2f}s (target <1s)")
    assert duration < 1.0, f"Concurrent access too slow: {duration:.2f}s"
    assert all(r is not None for r in results)
```

---

## 6. Troubleshooting Migration Issues

### 6.1 WAL Mode Not Enabled

**Symptom:** `PRAGMA journal_mode` returns "delete" instead of "wal"

**Diagnosis:**
```bash
sqlite3 abathur.db "PRAGMA journal_mode;"
# Output: delete (incorrect)
```

**Resolution:**
```bash
# Enable WAL mode
sqlite3 abathur.db "PRAGMA journal_mode = WAL;"

# Verify
sqlite3 abathur.db "PRAGMA journal_mode;"
# Output: wal (correct)
```

**Root Cause:** File system not compatible with WAL mode (e.g., NFS)

### 6.2 Foreign Key Constraint Violations

**Symptom:** `PRAGMA foreign_key_check` returns violations

**Diagnosis:**
```sql
PRAGMA foreign_key_check;
```

**Resolution:**
```sql
-- Delete orphaned records
DELETE FROM tasks WHERE session_id NOT IN (SELECT id FROM sessions);

-- Re-verify
PRAGMA foreign_key_check;
-- Expected: No rows returned
```

### 6.3 JSON Validation Errors

**Symptom:** `CHECK constraint failed: json_valid(...)`

**Diagnosis:**
```sql
SELECT id, events FROM sessions WHERE NOT json_valid(events);
```

**Resolution:**
```sql
-- Fix invalid JSON
UPDATE sessions SET events = '[]' WHERE NOT json_valid(events);

-- Or restore from backup
```

### 6.4 Missing Indexes

**Symptom:** Queries using full table scans (SCAN TABLE in EXPLAIN QUERY PLAN)

**Diagnosis:**
```sql
EXPLAIN QUERY PLAN
SELECT * FROM memory_entries WHERE namespace = 'user:alice:pref';
-- Output: SCAN TABLE memory_entries (bad)
```

**Resolution:**
```sql
-- Rebuild indexes
REINDEX;

-- Or recreate specific index
DROP INDEX IF EXISTS idx_memory_namespace_key_version;
CREATE INDEX idx_memory_namespace_key_version ON memory_entries(namespace, key, is_deleted, version DESC);
```

---

## 7. Migration Completion Checklist

### Pre-Migration

- [ ] System requirements verified (Python 3.11+, SQLite 3.35+)
- [ ] Disk space available (10GB+)
- [ ] File system compatible with WAL mode
- [ ] Backup directory created
- [ ] Database directory created with correct permissions

### Initialization

- [ ] PRAGMA settings configured (WAL mode, foreign keys, etc.)
- [ ] Memory tables created (sessions, memory_entries, document_index)
- [ ] Core tables created (tasks, agents, audit, checkpoints, state, metrics)
- [ ] All 33 indexes created
- [ ] Initial data seeded (3 application memories)

### Validation

- [ ] PRAGMA integrity_check returns "ok"
- [ ] PRAGMA foreign_key_check returns no violations
- [ ] All expected tables exist (9 tables)
- [ ] All indexes created (33 indexes)
- [ ] Seed data verified (3 memories)
- [ ] Index usage verified (EXPLAIN QUERY PLAN)

### Performance

- [ ] Session retrieval <10ms (p99)
- [ ] Memory retrieval <20ms (p99)
- [ ] 50+ concurrent sessions supported
- [ ] No full table scans in critical queries

### Production Readiness

- [ ] Backup automation configured
- [ ] Monitoring dashboards deployed
- [ ] Alerting rules configured
- [ ] Operational runbooks accessible
- [ ] Team trained on procedures

---

## References

**Phase 2 Technical Specifications:**
- [DDL Memory Tables](../phase2_tech_specs/ddl-memory-tables.sql) - Memory table DDL
- [DDL Core Tables](../phase2_tech_specs/ddl-core-tables.sql) - Core table DDL
- [DDL Indexes](../phase2_tech_specs/ddl-indexes.sql) - Index definitions
- [Implementation Guide](../phase2_tech_specs/implementation-guide.md) - Detailed procedures

**Phase 3 Implementation Plan:**
- [Milestone 1](./milestone-1-core-schema.md) - Core schema deployment
- [Testing Strategy](./testing-strategy.md) - Validation testing
- [Rollback Procedures](./rollback-procedures.md) - Emergency rollback

---

**Document Version:** 1.0
**Author:** implementation-planner
**Date:** 2025-10-10
**Status:** Complete - Ready for Execution
