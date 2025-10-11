# Rollback Procedures - Emergency Recovery and Disaster Recovery

## Overview

This document provides comprehensive rollback and disaster recovery procedures for all implementation milestones, ensuring safe recovery from failures at any stage.

**Recovery Principle:** Always have an escape hatch at every milestone

**Recovery Time Objective (RTO):** <1 hour for critical failures

---

## 1. Rollback Strategy Overview

### 1.1 Rollback Types

**Type 1: Fresh Start Rollback (Simple)**
- **When:** Database initialization failed or corrupted
- **Action:** Delete database and re-initialize
- **Data Loss:** None (fresh start project, no production data)
- **Duration:** 5-10 minutes

**Type 2: Milestone Rollback (Targeted)**
- **When:** Specific milestone deployment failed validation
- **Action:** Revert to previous milestone state
- **Data Loss:** Changes from failed milestone only
- **Duration:** 15-30 minutes

**Type 3: Backup Restore (Full)**
- **When:** Database corruption or data integrity failure
- **Action:** Restore from last known good backup
- **Data Loss:** Data since last backup (max 24 hours)
- **Duration:** 10-20 minutes

**Type 4: Emergency Production Rollback (Critical)**
- **When:** Critical production failure (data loss, unavailability)
- **Action:** Immediate service shutdown and restoration
- **Data Loss:** Minimized via WAL recovery
- **Duration:** 30-60 minutes

---

## 2. Milestone-Specific Rollback Procedures

### 2.1 Milestone 1: Core Schema Rollback

**Scenario:** Core table deployment failed or constraints violated

**Rollback Procedure:**

**Step 1: Stop All Services**
```bash
# Stop application services
systemctl stop abathur-api
systemctl stop abathur-workers

# Verify services stopped
systemctl status abathur-api
```

**Step 2: Delete Failed Database**
```bash
# Backup failed database for forensics
cp /var/lib/abathur/abathur.db /backups/failed_milestone1_$(date +%Y%m%d_%H%M%S).db

# Delete database files
rm /var/lib/abathur/abathur.db
rm /var/lib/abathur/abathur.db-wal
rm /var/lib/abathur/abathur.db-shm
```

**Step 3: Re-Initialize (if fixing immediately)**
```bash
# Re-run initialization with fixes
python -m abathur.infrastructure.database --initialize
```

**Step 4: Validate Database**
```bash
# Integrity check
sqlite3 /var/lib/abathur/abathur.db "PRAGMA integrity_check;"

# Foreign key check
sqlite3 /var/lib/abathur/abathur.db "PRAGMA foreign_key_check;"

# Table count
sqlite3 /var/lib/abathur/abathur.db "SELECT COUNT(*) FROM sqlite_master WHERE type='table';"
```

**Step 5: Restart Services**
```bash
systemctl start abathur-api
systemctl start abathur-workers

# Monitor logs
journalctl -u abathur-api -f
```

**Success Criteria:**
- [ ] All integrity checks pass
- [ ] All expected tables exist
- [ ] All services running without errors
- [ ] Performance baseline met

---

### 2.2 Milestone 2: Memory System Rollback

**Scenario:** Memory table deployment or SessionService/MemoryService failed

**Rollback Procedure:**

**Step 1: Stop Services and Checkpoint WAL**
```bash
# Stop services
systemctl stop abathur-api

# Checkpoint WAL to main database
sqlite3 /var/lib/abathur/abathur.db "PRAGMA wal_checkpoint(TRUNCATE);"
```

**Step 2: Drop Memory Tables and Indexes**
```sql
-- Drop memory tables (preserves Milestone 1 core tables)
DROP TABLE IF EXISTS sessions;
DROP TABLE IF EXISTS memory_entries;
DROP TABLE IF EXISTS document_index;

-- Drop memory indexes
DROP INDEX IF EXISTS idx_sessions_status_updated;
DROP INDEX IF EXISTS idx_sessions_user_created;
DROP INDEX IF EXISTS idx_sessions_project_id;
DROP INDEX IF EXISTS idx_sessions_app_user;
DROP INDEX IF EXISTS idx_memory_namespace_key_version;
DROP INDEX IF EXISTS idx_memory_type_updated;
DROP INDEX IF EXISTS idx_memory_namespace_prefix;
DROP INDEX IF EXISTS idx_memory_created_by;
DROP INDEX IF EXISTS idx_memory_updated_at;
DROP INDEX IF EXISTS idx_memory_type_namespace;
DROP INDEX IF EXISTS idx_memory_is_deleted;
DROP INDEX IF EXISTS idx_document_file_path;
DROP INDEX IF EXISTS idx_document_type;
DROP INDEX IF EXISTS idx_document_sync_status;
DROP INDEX IF EXISTS idx_document_hash;
DROP INDEX IF EXISTS idx_document_updated;
```

**Step 3: Revert to Milestone 1 State**
```bash
# Verify core tables still exist
sqlite3 /var/lib/abathur/abathur.db "SELECT name FROM sqlite_master WHERE type='table';"

# Expected output: tasks, agents, audit, checkpoints, state, metrics
```

**Step 4: Validate Rollback**
```bash
# Integrity check
sqlite3 /var/lib/abathur/abathur.db "PRAGMA integrity_check;"

# Verify memory tables removed
sqlite3 /var/lib/abathur/abathur.db "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='sessions';"
# Expected: 0
```

**Step 5: Restart Services (Milestone 1 Only)**
```bash
systemctl start abathur-api

# Monitor for errors
journalctl -u abathur-api -f
```

**Success Criteria:**
- [ ] Memory tables successfully removed
- [ ] Core tables (Milestone 1) intact
- [ ] No foreign key violations
- [ ] Services running with Milestone 1 functionality

---

### 2.3 Milestone 3: Vector Search Rollback

**Scenario:** sqlite-vss integration failed or semantic search performance unacceptable

**Rollback Procedure:**

**Step 1: Stop Embedding Services**
```bash
# Stop background sync service
systemctl stop abathur-document-sync

# Stop Ollama server (if dedicated)
systemctl stop ollama
```

**Step 2: Disable Vector Search Features**
```python
# In application configuration
FEATURE_FLAGS = {
    "semantic_search_enabled": False,
    "background_document_sync": False,
    "embedding_generation": False
}
```

**Step 3: Drop Vector Search Tables**
```sql
-- Drop vector search tables
DROP TABLE IF EXISTS document_embeddings;
DROP TABLE IF EXISTS document_embeddings_data;

-- Keep document_index table but mark all as pending
UPDATE document_index SET sync_status = 'pending', embedding_blob = NULL;
```

**Step 4: Unload sqlite-vss Extension**
```python
# In database initialization
async def initialize_database(db_path: Path):
    async with aiosqlite.connect(str(db_path)) as conn:
        # Do NOT load sqlite-vss extension
        # await conn.load_extension('/usr/lib/sqlite3/vss0')
        pass
```

**Step 5: Fallback to Exact-Match Search Only**
```python
# Update search service to use exact-match only
class SearchService:
    async def search_documents(self, query: str):
        # Fallback to exact keyword search (LIKE or FTS5)
        async with self.db._get_connection() as conn:
            cursor = await conn.execute(
                "SELECT * FROM document_index WHERE title LIKE ? OR metadata LIKE ?",
                (f"%{query}%", f"%{query}%")
            )
            return await cursor.fetchall()
```

**Success Criteria:**
- [ ] Vector search disabled
- [ ] Exact-match search functional
- [ ] Application services running
- [ ] No performance degradation on core features

---

### 2.4 Milestone 4: Production Rollback

**Scenario:** Critical production failure (data loss, unavailability, severe performance degradation)

**Emergency Rollback Procedure:**

**Step 1: Immediate Service Shutdown (1 minute)**
```bash
# IMMEDIATE ACTION: Stop all traffic
systemctl stop abathur-api
systemctl stop abathur-workers
systemctl stop abathur-document-sync

# Verify all services stopped
ps aux | grep abathur
```

**Step 2: Assess Failure Severity (5 minutes)**
```bash
# Check database integrity
sqlite3 /var/lib/abathur/abathur.db "PRAGMA integrity_check;" > /tmp/integrity_check.log

# Check for corruption
if grep -v "ok" /tmp/integrity_check.log; then
    echo "CRITICAL: Database corruption detected"
    SEVERITY="CRITICAL"
else
    echo "Database integrity OK"
    SEVERITY="HIGH"
fi
```

**Step 3: Restore from Latest Backup (10 minutes)**
```bash
# Find latest backup
LATEST_BACKUP=$(ls -t /backups/abathur_*.db.gz | head -1)
echo "Restoring from: $LATEST_BACKUP"

# Backup current database (for forensics)
cp /var/lib/abathur/abathur.db /backups/failed_production_$(date +%Y%m%d_%H%M%S).db

# Restore from backup
gunzip -c $LATEST_BACKUP > /var/lib/abathur/abathur.db

# Verify restoration
sqlite3 /var/lib/abathur/abathur.db "PRAGMA integrity_check;"
```

**Step 4: Apply WAL Recovery (if applicable) (5 minutes)**
```bash
# If WAL file exists and is recent
if [ -f /var/lib/abathur/abathur.db-wal ]; then
    # Checkpoint WAL to recover recent transactions
    sqlite3 /var/lib/abathur/abathur.db "PRAGMA wal_checkpoint(RECOVER);"
fi
```

**Step 5: Validate Restoration (5 minutes)**
```bash
# Integrity check
sqlite3 /var/lib/abathur/abathur.db "PRAGMA integrity_check;"

# Foreign key check
sqlite3 /var/lib/abathur/abathur.db "PRAGMA foreign_key_check;"

# Table count
sqlite3 /var/lib/abathur/abathur.db "SELECT COUNT(*) FROM sqlite_master WHERE type='table';"
# Expected: 9 tables

# Index count
sqlite3 /var/lib/abathur/abathur.db "SELECT COUNT(*) FROM sqlite_master WHERE type='index';"
# Expected: 33+ indexes

# Sample data query
sqlite3 /var/lib/abathur/abathur.db "SELECT COUNT(*) FROM sessions;"
```

**Step 6: Restart Services with Monitoring (5 minutes)**
```bash
# Restart services
systemctl start abathur-api
systemctl start abathur-workers

# Monitor logs for errors
journalctl -u abathur-api -f &

# Wait for health checks
sleep 30

# Verify health endpoint
curl http://localhost:8000/health
# Expected: {"status": "healthy"}
```

**Step 7: Post-Rollback Validation (10 minutes)**
```bash
# Run smoke tests
python -m pytest tests/smoke/production_smoke_tests.py -v

# Check performance
python -m abathur.utils.benchmark --quick

# Verify data integrity
python -m abathur.utils.validate_database
```

**Total Rollback Time:** ~40 minutes (within RTO of 1 hour)

---

## 3. Backup and Restore Procedures

### 3.1 Daily Backup Automation

**Backup Script:** `/usr/local/bin/abathur-backup.sh`

```bash
#!/bin/bash
set -e

DB_PATH="/var/lib/abathur/abathur.db"
BACKUP_DIR="/backups/abathur"
DATE=$(date +%Y-%m-%d_%H-%M-%S)
RETENTION_DAYS=30

# Create backup directory if not exists
mkdir -p "$BACKUP_DIR"

# WAL checkpoint (ensure all data in main DB file)
sqlite3 "$DB_PATH" "PRAGMA wal_checkpoint(TRUNCATE);"

# Copy database
cp "$DB_PATH" "$BACKUP_DIR/abathur_$DATE.db"

# Compress backup
gzip "$BACKUP_DIR/abathur_$DATE.db"

# Verify backup integrity
gunzip -c "$BACKUP_DIR/abathur_$DATE.db.gz" | sqlite3 /dev/stdin "PRAGMA integrity_check;" > /tmp/backup_verify.log

if grep -v "ok" /tmp/backup_verify.log; then
    echo "ERROR: Backup integrity check failed"
    exit 1
fi

# Delete old backups (retain last 30 days)
find "$BACKUP_DIR" -name "abathur_*.db.gz" -mtime +$RETENTION_DAYS -delete

echo "Backup completed: $BACKUP_DIR/abathur_$DATE.db.gz"
```

**Cron Schedule:**
```cron
# Daily backup at 2 AM
0 2 * * * /usr/local/bin/abathur-backup.sh >> /var/log/abathur-backup.log 2>&1
```

### 3.2 Manual Backup Procedure

**Before Critical Operations:**
```bash
# Create manual backup
DB_PATH="/var/lib/abathur/abathur.db"
BACKUP_PATH="/backups/abathur/manual_$(date +%Y%m%d_%H%M%S).db"

# Checkpoint WAL
sqlite3 "$DB_PATH" "PRAGMA wal_checkpoint(TRUNCATE);"

# Copy database
cp "$DB_PATH" "$BACKUP_PATH"

# Compress
gzip "$BACKUP_PATH"

# Verify
gunzip -c "${BACKUP_PATH}.gz" | sqlite3 /dev/stdin "PRAGMA integrity_check;"

echo "Manual backup created: ${BACKUP_PATH}.gz"
```

### 3.3 Restore from Backup

**Interactive Restore:**
```bash
# List available backups
ls -lh /backups/abathur/

# Select backup to restore
BACKUP_FILE="/backups/abathur/abathur_2025-10-10_02-00-00.db.gz"

# Stop services
systemctl stop abathur-api abathur-workers

# Backup current database (safety)
cp /var/lib/abathur/abathur.db /var/lib/abathur/abathur_pre_restore_$(date +%Y%m%d_%H%M%S).db

# Restore from backup
gunzip -c "$BACKUP_FILE" > /var/lib/abathur/abathur.db

# Verify restoration
sqlite3 /var/lib/abathur/abathur.db "PRAGMA integrity_check; PRAGMA foreign_key_check;"

# Restart services
systemctl start abathur-api abathur-workers

# Monitor logs
journalctl -u abathur-api -f
```

---

## 4. Disaster Recovery Procedures

### 4.1 Database Corruption Recovery

**Scenario:** `PRAGMA integrity_check` fails, database corrupted

**Recovery Procedure:**

**Step 1: Attempt SQLite Recovery**
```bash
# Try to recover using .recover command
sqlite3 /var/lib/abathur/abathur.db ".recover" | sqlite3 /var/lib/abathur/abathur_recovered.db

# Verify recovered database
sqlite3 /var/lib/abathur/abathur_recovered.db "PRAGMA integrity_check;"
```

**Step 2: If Recovery Successful**
```bash
# Stop services
systemctl stop abathur-api

# Backup corrupted database (forensics)
mv /var/lib/abathur/abathur.db /var/lib/abathur/abathur_corrupted_$(date +%Y%m%d_%H%M%S).db

# Replace with recovered database
mv /var/lib/abathur/abathur_recovered.db /var/lib/abathur/abathur.db

# Restart services
systemctl start abathur-api
```

**Step 3: If Recovery Failed**
```bash
# Restore from latest backup (see Section 3.3)
LATEST_BACKUP=$(ls -t /backups/abathur/abathur_*.db.gz | head -1)
gunzip -c "$LATEST_BACKUP" > /var/lib/abathur/abathur.db
```

### 4.2 WAL File Corruption Recovery

**Scenario:** WAL file corrupted, checkpoint fails

**Recovery Procedure:**
```bash
# Stop services
systemctl stop abathur-api

# Delete corrupted WAL files (will lose uncommitted transactions)
rm /var/lib/abathur/abathur.db-wal
rm /var/lib/abathur/abathur.db-shm

# Verify main database still intact
sqlite3 /var/lib/abathur/abathur.db "PRAGMA integrity_check;"

# Restart services (will recreate WAL)
systemctl start abathur-api
```

### 4.3 Disk Space Exhaustion Recovery

**Scenario:** Disk full, database writes failing

**Recovery Procedure:**
```bash
# Check disk usage
df -h /var/lib/abathur

# Identify large files
du -h /var/lib/abathur/* | sort -h

# Immediate actions:
# 1. Checkpoint and truncate WAL
sqlite3 /var/lib/abathur/abathur.db "PRAGMA wal_checkpoint(TRUNCATE);"

# 2. Delete old backups (if in same partition)
find /backups/abathur -name "*.db.gz" -mtime +7 -delete

# 3. VACUUM to reclaim space
sqlite3 /var/lib/abathur/abathur.db "VACUUM;"

# Verify space reclaimed
df -h /var/lib/abathur
```

---

## 5. Rollback Testing Procedures

### 5.1 Scheduled Rollback Drills

**Quarterly Rollback Drill:**

**Week 1: Milestone 1 Rollback**
- Delete database and re-initialize
- Measure rollback time (target <10 minutes)
- Verify all services functional

**Week 2: Milestone 2 Rollback**
- Drop memory tables
- Verify core tables intact
- Restore memory tables from DDL

**Week 3: Backup Restore**
- Restore from 24-hour-old backup
- Verify data integrity
- Measure RTO (target <1 hour)

**Week 4: Full Disaster Recovery**
- Simulate database corruption
- Execute complete recovery procedure
- Document lessons learned

### 5.2 Rollback Validation Checklist

**After Every Rollback:**

- [ ] Database integrity check passes
- [ ] Foreign key constraints verified
- [ ] All expected tables exist
- [ ] All indexes created and functional
- [ ] Application services running without errors
- [ ] Performance baseline met
- [ ] Smoke tests passing (35 tests)
- [ ] No data loss detected (or within acceptable limits)
- [ ] Monitoring and alerting functional
- [ ] Incident documented and reviewed

---

## 6. Escalation Procedures

### 6.1 Escalation Levels

**Level 1: On-Call Engineer (Self-Service Recovery)**
- **Scope:** Minor issues, standard rollback procedures
- **Action:** Execute documented rollback procedure
- **Time Limit:** 30 minutes
- **Escalate If:** Rollback unsuccessful or uncertainty about procedure

**Level 2: Senior Engineer (Expert Guidance)**
- **Scope:** Complex issues, multiple failures, ambiguous root cause
- **Action:** Diagnose root cause, provide guidance to on-call
- **Time Limit:** 1 hour
- **Escalate If:** Data loss risk, major production impact, unresolved after 1 hour

**Level 3: Database Specialist (Critical Recovery)**
- **Scope:** Database corruption, data loss, manual recovery required
- **Action:** Direct recovery operations, manual data reconstruction
- **Time Limit:** 2 hours
- **Escalate If:** Permanent data loss, recovery impossible

**Level 4: Incident Commander (Crisis Management)**
- **Scope:** Major outage, customer impact, stakeholder communication
- **Action:** Coordinate recovery, manage communications, post-incident review
- **Time Limit:** Until incident resolved

### 6.2 Contact Information

**On-Call Rotation:**
- Primary On-Call: [Phone/Slack]
- Secondary On-Call: [Phone/Slack]

**Escalation Contacts:**
- Senior Engineer: [Contact]
- Database Specialist: [Contact]
- Incident Commander: [Contact]

**Emergency Slack Channel:** #abathur-incidents

---

## 7. Post-Rollback Procedures

### 7.1 Immediate Post-Rollback

**Within 1 Hour:**
- [ ] Verify all services healthy
- [ ] Run smoke tests (35 tests)
- [ ] Check performance metrics
- [ ] Monitor for errors (30 minutes continuous)
- [ ] Notify stakeholders of resolution

**Within 24 Hours:**
- [ ] Analyze root cause
- [ ] Document incident timeline
- [ ] Update rollback procedures (if gaps found)
- [ ] Schedule post-incident review meeting

### 7.2 Post-Incident Review

**Within 1 Week:**
- [ ] Conduct blameless post-mortem
- [ ] Identify systemic issues
- [ ] Create action items (with owners and due dates)
- [ ] Update documentation
- [ ] Share learnings with team

**Post-Mortem Template:**
```markdown
# Incident Post-Mortem: [Date]

## Summary
- Incident start time:
- Incident end time:
- Duration:
- Severity:

## Impact
- Services affected:
- Users impacted:
- Data loss:

## Root Cause
- Primary cause:
- Contributing factors:

## Timeline
- [Time] Event 1
- [Time] Event 2
...

## Resolution
- Actions taken:
- Rollback procedure used:

## Lessons Learned
- What went well:
- What could be improved:

## Action Items
- [ ] Action 1 (Owner, Due Date)
- [ ] Action 2 (Owner, Due Date)
```

---

## 8. Rollback Success Criteria

### 8.1 Technical Validation

- [ ] Database integrity check passes
- [ ] All foreign key constraints valid
- [ ] All expected tables and indexes exist
- [ ] Performance meets baseline targets
- [ ] No errors in application logs (30 minutes)

### 8.2 Operational Validation

- [ ] All services running and healthy
- [ ] Monitoring dashboards showing data
- [ ] Alerts functional (test alert sent)
- [ ] Backup automation running
- [ ] Rollback documented in incident log

### 8.3 Business Validation

- [ ] Services accessible to users
- [ ] Core functionality working (tested)
- [ ] No data loss (or within acceptable limits)
- [ ] Stakeholders notified
- [ ] Post-incident review scheduled

---

## References

**Phase 3 Implementation Plan:**
- [Milestone 1](./milestone-1-core-schema.md) - Core schema deployment
- [Milestone 2](./milestone-2-memory-system.md) - Memory system deployment
- [Milestone 3](./milestone-3-vector-search.md) - Vector search deployment
- [Milestone 4](./milestone-4-production-deployment.md) - Production deployment
- [Migration Procedures](./migration-procedures.md) - Fresh start initialization

**Operational Documentation:**
- Production Operations Runbook: `docs/runbooks/production_operations.md`
- Incident Response Guide: `docs/operations/incident_response.md`

---

**Document Version:** 1.0
**Author:** implementation-planner
**Date:** 2025-10-10
**Status:** Complete - Ready for Rollback Drills
