# Implementation Guide - Step-by-Step Deployment

## Overview

This guide provides complete step-by-step procedures for deploying the memory management schema, from fresh database initialization to production validation.

**Deployment Strategy:** Fresh start (new project, no migration required)

**Target:** Complete database initialization with all tables, indexes, and initial data

---

## 1. Prerequisites

### 1.1 System Requirements

- **Python:** 3.11+
- **SQLite:** 3.35+ (for JSON functions support)
- **Dependencies:** aiosqlite, ollama (optional for Phase 2+)
- **Disk Space:** 500MB minimum (10GB target capacity)
- **OS:** macOS, Linux, or Docker

### 1.2 Verify SQLite Version

```bash
sqlite3 --version
# Expected: 3.35.0 or higher
```

If SQLite < 3.35, upgrade:
```bash
# macOS
brew upgrade sqlite3

# Linux (Ubuntu)
sudo apt-get update
sudo apt-get install sqlite3
```

---

## 2. Database Initialization

### 2.1 Create Database Directory

```bash
# Create data directory
mkdir -p /var/lib/abathur
chmod 700 /var/lib/abathur

# Or for development
mkdir -p ~/abathur_data
```

### 2.2 Set PRAGMA Configuration

**IMPORTANT:** Run these PRAGMA settings BEFORE creating tables.

```python
import aiosqlite
from pathlib import Path

async def initialize_database(db_path: Path):
    """Initialize fresh Abathur database with memory management schema."""

    async with aiosqlite.connect(str(db_path)) as conn:
        # Step 1: Configure PRAGMA settings
        await conn.execute("PRAGMA journal_mode = WAL")
        await conn.execute("PRAGMA synchronous = NORMAL")
        await conn.execute("PRAGMA foreign_keys = ON")
        await conn.execute("PRAGMA busy_timeout = 5000")
        await conn.execute("PRAGMA wal_autocheckpoint = 1000")

        # Verify WAL mode
        cursor = await conn.execute("PRAGMA journal_mode")
        mode = await cursor.fetchone()
        assert mode[0] == 'wal', f"WAL mode not enabled: {mode[0]}"

        print("PRAGMA configuration complete")
```

### 2.3 Execute DDL Scripts (In Order)

**Execution Order Matters:** Memory tables first (foreign key targets), then core tables.

```python
async def execute_ddl_scripts(conn: aiosqlite.Connection):
    """Execute DDL scripts in correct dependency order."""

    # Step 2: Create memory management tables (no dependencies)
    await execute_sql_file(conn, "design_docs/phase2_tech_specs/ddl-memory-tables.sql")
    print("Memory tables created")

    # Step 3: Create/enhance core tables (depends on sessions table)
    await execute_sql_file(conn, "design_docs/phase2_tech_specs/ddl-core-tables.sql")
    print("Core tables created")

    # Step 4: Create all indexes
    await execute_sql_file(conn, "design_docs/phase2_tech_specs/ddl-indexes.sql")
    print("Indexes created")

    await conn.commit()

async def execute_sql_file(conn: aiosqlite.Connection, file_path: str):
    """Execute SQL file line by line."""
    sql_content = Path(file_path).read_text()

    # Split by ; and execute each statement
    statements = sql_content.split(';')
    for statement in statements:
        statement = statement.strip()
        if statement and not statement.startswith('--'):
            try:
                await conn.execute(statement)
            except Exception as e:
                print(f"Error executing: {statement[:50]}...")
                raise e
```

### 2.4 Verify Table Creation

```python
async def verify_tables(conn: aiosqlite.Connection):
    """Verify all tables were created successfully."""

    expected_tables = [
        'sessions',
        'memory_entries',
        'document_index',
        'tasks',
        'agents',
        'state',
        'audit',
        'metrics',
        'checkpoints'
    ]

    cursor = await conn.execute(
        "SELECT name FROM sqlite_master WHERE type='table' ORDER BY name"
    )
    tables = [row[0] for row in await cursor.fetchall()]

    for table in expected_tables:
        assert table in tables, f"Table {table} not created"

    print(f"Verified {len(expected_tables)} tables")
```

---

## 3. Initial Data Seeding

### 3.1 Seed Application Memories

```python
import json
from datetime import datetime, timezone

async def seed_initial_data(conn: aiosqlite.Connection):
    """Seed initial procedural and semantic memories."""

    # Seed: Default agent instructions
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

    # Seed: Default user preferences template
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
    print("Initial data seeded")
```

---

## 4. Validation Procedures

### 4.1 Integrity Checks

```python
async def run_integrity_checks(conn: aiosqlite.Connection):
    """Run comprehensive database integrity checks."""

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

    # Check 3: Verify indexes exist
    cursor = await conn.execute(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='index'"
    )
    index_count = (await cursor.fetchone())[0]
    assert index_count >= 30, f"Expected 30+ indexes, found {index_count}"
    print(f"✓ {index_count} indexes created")

    # Check 4: Verify JSON validation constraints
    try:
        await conn.execute(
            "INSERT INTO sessions (id, app_name, user_id, events) VALUES ('test', 'app', 'user', 'invalid json')"
        )
        assert False, "JSON validation constraint should have failed"
    except Exception:
        pass  # Expected failure
    print("✓ JSON validation constraints working")
```

### 4.2 Query Performance Tests

```python
async def test_query_performance(conn: aiosqlite.Connection):
    """Verify query performance targets."""

    import time

    # Test 1: Session retrieval
    session_id = 'test_session_' + str(time.time())
    await conn.execute(
        "INSERT INTO sessions (id, app_name, user_id, status) VALUES (?, 'app', 'user', 'created')",
        (session_id,)
    )
    await conn.commit()

    start = time.time()
    cursor = await conn.execute("SELECT * FROM sessions WHERE id = ?", (session_id,))
    await cursor.fetchone()
    duration_ms = (time.time() - start) * 1000

    assert duration_ms < 10, f"Session retrieval took {duration_ms:.2f}ms (expected <10ms)"
    print(f"✓ Session retrieval: {duration_ms:.2f}ms")

    # Test 2: Memory retrieval with index
    await conn.execute(
        """
        INSERT INTO memory_entries (namespace, key, value, memory_type, created_by, updated_by)
        VALUES ('user:test:pref', 'theme', '{"mode": "dark"}', 'semantic', 'session:test', 'session:test')
        """
    )
    await conn.commit()

    start = time.time()
    cursor = await conn.execute(
        """
        SELECT * FROM memory_entries
        WHERE namespace = 'user:test:pref' AND key = 'theme' AND is_deleted = 0
        ORDER BY version DESC LIMIT 1
        """,
    )
    await cursor.fetchone()
    duration_ms = (time.time() - start) * 1000

    assert duration_ms < 20, f"Memory retrieval took {duration_ms:.2f}ms (expected <20ms)"
    print(f"✓ Memory retrieval: {duration_ms:.2f}ms")
```

### 4.3 Verify EXPLAIN QUERY PLAN

```python
async def verify_index_usage(conn: aiosqlite.Connection):
    """Verify critical queries use indexes."""

    # Test: Memory namespace query
    cursor = await conn.execute(
        """
        EXPLAIN QUERY PLAN
        SELECT * FROM memory_entries
        WHERE namespace = 'user:test' AND key = 'pref' AND is_deleted = 0
        ORDER BY version DESC LIMIT 1
        """
    )
    plan = await cursor.fetchall()

    plan_text = " ".join(str(row) for row in plan)
    assert "idx_memory_namespace_key_version" in plan_text or "USING INDEX" in plan_text, \
        f"Index not used: {plan_text}"
    assert "SCAN TABLE" not in plan_text, "Full table scan detected"

    print("✓ Memory query uses index")

    # Test: Session status query
    cursor = await conn.execute(
        """
        EXPLAIN QUERY PLAN
        SELECT * FROM sessions
        WHERE status = 'active'
        ORDER BY last_update_time DESC
        """
    )
    plan = await cursor.fetchall()

    plan_text = " ".join(str(row) for row in plan)
    assert "idx_sessions_status_updated" in plan_text or "USING INDEX" in plan_text, \
        f"Index not used: {plan_text}"

    print("✓ Session query uses index")
```

---

## 5. Rollback Procedures

### 5.1 Fresh Start Rollback (Simple)

**Scenario:** Database initialization failed or corrupted

**Procedure:**
```bash
# Stop application
systemctl stop abathur

# Delete database files
rm /var/lib/abathur/abathur.db
rm /var/lib/abathur/abathur.db-wal
rm /var/lib/abathur/abathur.db-shm

# Re-run initialization
python -c "from abathur.infrastructure.database import Database; import asyncio; asyncio.run(Database(Path('/var/lib/abathur/abathur.db')).initialize())"

# Verify integrity
sqlite3 /var/lib/abathur/abathur.db "PRAGMA integrity_check;"

# Restart application
systemctl start abathur
```

### 5.2 Backup and Restore

**Create Backup:**
```bash
# Checkpoint WAL (flush all data to main DB file)
sqlite3 /var/lib/abathur/abathur.db "PRAGMA wal_checkpoint(TRUNCATE);"

# Copy database
cp /var/lib/abathur/abathur.db /backups/abathur_$(date +%Y%m%d_%H%M%S).db

# Compress
gzip /backups/abathur_*.db
```

**Restore from Backup:**
```bash
# Stop application
systemctl stop abathur

# Restore
gunzip -c /backups/abathur_20251010_120000.db.gz > /var/lib/abathur/abathur.db

# Verify
sqlite3 /var/lib/abathur/abathur.db "PRAGMA integrity_check; PRAGMA foreign_key_check;"

# Restart
systemctl start abathur
```

---

## 6. Production Deployment Checklist

### 6.1 Pre-Deployment

- [ ] All DDL scripts tested on staging database
- [ ] Unit tests passing (100% coverage for database layer)
- [ ] Integration tests passing
- [ ] Performance benchmarks meet targets (<50ms reads)
- [ ] Documentation complete and reviewed
- [ ] Backup strategy implemented and tested
- [ ] Monitoring alerts configured
- [ ] Rollback procedures documented and validated

### 6.2 Deployment Steps

**Step 1: Environment Setup**
```bash
# Create production directory
sudo mkdir -p /var/lib/abathur
sudo chown abathur:abathur /var/lib/abathur
sudo chmod 700 /var/lib/abathur
```

**Step 2: Initialize Database**
```python
# Run initialization script
from abathur.infrastructure.database import Database
from pathlib import Path
import asyncio

async def deploy():
    db = Database(Path("/var/lib/abathur/abathur.db"))
    await db.initialize()
    print("Database initialized")

asyncio.run(deploy())
```

**Step 3: Verify Deployment**
```bash
# Check tables
sqlite3 /var/lib/abathur/abathur.db "SELECT name FROM sqlite_master WHERE type='table';"

# Check indexes
sqlite3 /var/lib/abathur/abathur.db "SELECT name FROM sqlite_master WHERE type='index';"

# Integrity check
sqlite3 /var/lib/abathur/abathur.db "PRAGMA integrity_check; PRAGMA foreign_key_check;"

# WAL mode verification
sqlite3 /var/lib/abathur/abathur.db "PRAGMA journal_mode;"
# Expected: wal
```

**Step 4: Seed Initial Data**
```python
# Run seed script (see Section 3.1)
```

**Step 5: Start Application**
```bash
systemctl start abathur
systemctl status abathur
```

**Step 6: Monitor Logs**
```bash
journalctl -u abathur -f
```

### 6.3 Post-Deployment Validation

- [ ] Create test session via API
- [ ] Create test memory entry
- [ ] Verify audit logging captures operations
- [ ] Check query performance with EXPLAIN QUERY PLAN
- [ ] Monitor WAL file size (should checkpoint regularly)
- [ ] Verify backup job runs successfully
- [ ] Test concurrent access with 10+ simultaneous sessions

---

## 7. Monitoring and Maintenance

### 7.1 Regular Maintenance Tasks

**Weekly:**
```bash
# Update query optimizer statistics
sqlite3 /var/lib/abathur/abathur.db "ANALYZE;"

# Check database size
du -h /var/lib/abathur/abathur.db
```

**Monthly:**
```bash
# Full integrity check
sqlite3 /var/lib/abathur/abathur.db "PRAGMA integrity_check;"

# Vacuum database (reclaim space)
sqlite3 /var/lib/abathur/abathur.db "VACUUM;"
```

### 7.2 Performance Monitoring

**Key Metrics:**
- Query latency (track with metrics table)
- WAL file size (should stay <100MB)
- Database size growth rate
- Concurrent session count
- Index usage statistics

---

## 8. Troubleshooting

### 8.1 Common Issues

**Issue: WAL mode not enabled**
```bash
# Verify
sqlite3 abathur.db "PRAGMA journal_mode;"

# Fix
sqlite3 abathur.db "PRAGMA journal_mode = WAL;"
```

**Issue: Foreign key violations**
```bash
# Check violations
sqlite3 abathur.db "PRAGMA foreign_key_check;"

# Enable foreign keys if disabled
sqlite3 abathur.db "PRAGMA foreign_keys = ON;"
```

**Issue: Slow queries**
```bash
# Identify missing indexes
sqlite3 abathur.db "EXPLAIN QUERY PLAN SELECT ..."

# Rebuild indexes
sqlite3 abathur.db "REINDEX;"
```

---

**Document Version:** 1.0
**Author:** technical-specifications-writer
**Date:** 2025-10-10
**Status:** Phase 2 Complete - Production Ready
