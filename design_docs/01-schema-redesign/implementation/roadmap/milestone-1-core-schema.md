# Milestone 1: Core Schema Foundation

## Overview

**Goal:** Deploy enhanced existing tables with session linkage and establish database foundation with core indexes

**Timeline:** Weeks 1-2 (10 business days)

**Dependencies:**
- Phase 1 design documents approved
- Phase 2 technical specifications validated
- Development environment setup complete

---

## Objectives

1. Deploy enhanced existing tables (tasks, agents, audit, checkpoints) with session_id foreign keys
2. Create core performance indexes (15 indexes)
3. Implement Database class enhancements for new table support
4. Establish comprehensive unit test suite (90%+ coverage)
5. Create performance baseline benchmarks for query optimization

---

## Tasks and Effort Estimates

### Week 1: Database Schema Deployment

| Task | Description | Effort (hours) | Owner | Dependencies |
|------|-------------|---------------|-------|--------------|
| **1.1** | Review Phase 2 DDL scripts | 4h | Dev Team | Phase 2 complete |
| **1.2** | Setup development database environment | 4h | DevOps | Python 3.11+, SQLite 3.35+ |
| **1.3** | Execute PRAGMA configuration (WAL mode, foreign keys) | 2h | Dev Team | Task 1.2 |
| **1.4** | Deploy enhanced tasks table with session_id FK | 4h | Dev Team | Task 1.3 |
| **1.5** | Deploy enhanced agents table with session_id FK | 4h | Dev Team | Task 1.4 |
| **1.6** | Deploy enhanced audit table with memory operation tracking | 4h | Dev Team | Task 1.4 |
| **1.7** | Deploy enhanced checkpoints table with session_id FK | 2h | Dev Team | Task 1.4 |
| **1.8** | Verify all table constraints (CHECK, UNIQUE, FK) | 4h | Dev Team | Task 1.7 |
| **1.9** | Create core indexes (15 indexes for existing tables) | 4h | Dev Team | Task 1.8 |
| **1.10** | Run PRAGMA integrity_check and foreign_key_check | 2h | Dev Team | Task 1.9 |

**Week 1 Total:** 38 hours

### Week 2: Testing and Validation

| Task | Description | Effort (hours) | Owner | Dependencies |
|------|-------------|---------------|-------|--------------|
| **2.1** | Implement Database class enhancements | 8h | Dev Team | Week 1 complete |
| **2.2** | Write unit tests for tasks table operations | 6h | Dev Team | Task 2.1 |
| **2.3** | Write unit tests for agents table operations | 6h | Dev Team | Task 2.1 |
| **2.4** | Write unit tests for audit table operations | 4h | Dev Team | Task 2.1 |
| **2.5** | Write unit tests for foreign key constraints | 4h | Dev Team | Task 2.2-2.4 |
| **2.6** | Create performance benchmark suite | 6h | Dev Team | Task 2.1 |
| **2.7** | Run performance baseline tests (<50ms reads) | 4h | Dev Team | Task 2.6 |
| **2.8** | Verify EXPLAIN QUERY PLAN shows index usage | 4h | Dev Team | Task 2.7 |
| **2.9** | Document baseline metrics and results | 2h | Dev Team | Task 2.8 |
| **2.10** | Code review and quality assurance | 4h | Tech Lead | All tasks |

**Week 2 Total:** 48 hours

**Milestone 1 Total Effort:** 86 hours (~2 developers × 1 week)

---

## Deliverables

### 1. Enhanced Database Schema

**File:** `/var/lib/abathur/abathur.db` (development environment)

**Tables:**
- ✅ `tasks` table with `session_id` foreign key to `sessions.id`
- ✅ `agents` table with `session_id` foreign key to `sessions.id`
- ✅ `audit` table with memory operation columns (`memory_operation_type`, `memory_namespace`, `memory_entry_id`)
- ✅ `checkpoints` table with `session_id` foreign key to `sessions.id`
- ✅ `state`, `metrics` tables (unchanged, maintained for backward compatibility)

**PRAGMA Configuration:**
```sql
PRAGMA journal_mode = WAL;           -- Concurrent reads enabled
PRAGMA synchronous = NORMAL;          -- Balanced safety/performance
PRAGMA foreign_keys = ON;             -- FK constraints enforced
PRAGMA busy_timeout = 5000;           -- 5-second lock wait
PRAGMA wal_autocheckpoint = 1000;     -- Checkpoint every 1000 pages
```

### 2. Core Indexes (15 indexes)

**Tasks Indexes:**
- `idx_tasks_status_priority` - Status + priority queries
- `idx_tasks_session_id` - Session-task linkage
- `idx_tasks_submitted_at` - Temporal queries
- `idx_tasks_parent_task_id` - Parent-child relationships

**Agents Indexes:**
- `idx_agents_task_id` - Task-agent linkage
- `idx_agents_session_id` - Session-agent linkage
- `idx_agents_spawned_at` - Temporal queries

**Audit Indexes:**
- `idx_audit_task_id` - Task audit trail
- `idx_audit_timestamp` - Temporal audit queries
- `idx_audit_memory_operation` - Memory operation filtering
- `idx_audit_memory_entry_id` - Memory entry audit trail

**Checkpoints Indexes:**
- `idx_checkpoints_task_id_iteration` - Composite primary lookup
- `idx_checkpoints_session_id` - Session-checkpoint linkage
- `idx_checkpoints_created_at` - Temporal queries

**Metrics Indexes:**
- `idx_metrics_timestamp` - Temporal metrics queries

### 3. Enhanced Database Class

**File:** `src/abathur/infrastructure/database.py`

**New Methods:**
```python
class Database:
    # Enhanced initialization
    async def _create_enhanced_tables(self, conn: Connection) -> None:
        """Create enhanced existing tables with session linkage."""

    async def _create_core_indexes(self, conn: Connection) -> None:
        """Create core performance indexes."""

    # Foreign key validation
    async def validate_foreign_keys(self) -> List[Tuple]:
        """Run PRAGMA foreign_key_check and return violations."""

    # Performance utilities
    async def explain_query_plan(self, query: str, params: Tuple = ()) -> List[str]:
        """Return EXPLAIN QUERY PLAN output for query optimization."""
```

### 4. Unit Test Suite

**File:** `tests/infrastructure/test_database_core.py`

**Test Coverage:**
- ✅ Table creation and schema validation
- ✅ Foreign key constraint enforcement
- ✅ Tasks table CRUD operations with session linkage
- ✅ Agents table CRUD operations with session linkage
- ✅ Audit table memory operation logging
- ✅ Checkpoints table with session linkage
- ✅ Constraint violation handling (FK, UNIQUE, CHECK)
- ✅ Index usage verification with EXPLAIN QUERY PLAN

**Coverage Target:** 90%+ for database layer

### 5. Performance Baseline Report

**File:** `docs/performance_baseline_milestone1.md`

**Metrics:**
- Task query latency (by status, priority, session_id)
- Agent query latency (by task_id, session_id)
- Audit query latency (by task_id, timestamp, memory_entry_id)
- Index usage statistics (all queries show "USING INDEX" in EXPLAIN QUERY PLAN)
- Concurrent access performance (10 simultaneous connections)

**Success Criteria:** All queries <50ms at 99th percentile

---

## Acceptance Criteria

### Technical Validation

- [ ] All DDL scripts execute without errors
- [ ] All 15 core indexes created successfully
- [ ] PRAGMA integrity_check returns "ok"
- [ ] PRAGMA foreign_key_check returns no violations
- [ ] All unit tests pass (90%+ coverage)
- [ ] All queries show index usage in EXPLAIN QUERY PLAN
- [ ] Performance benchmarks meet <50ms target (99th percentile)
- [ ] Foreign key constraints properly enforce referential integrity

### Functional Validation

- [ ] Tasks can be created with session_id linkage
- [ ] Agents can be spawned with session_id linkage
- [ ] Audit table captures all database operations correctly
- [ ] Checkpoints support session-based state storage
- [ ] Cascade rules work correctly (SET NULL on session deletion)
- [ ] JSON validation constraints prevent invalid data

### Code Quality

- [ ] Code review completed and approved
- [ ] All code follows project style guidelines
- [ ] Documentation updated (API docs, comments)
- [ ] No code smells or technical debt introduced

---

## Risks and Mitigation

### Risk 1: Foreign Key Performance Impact

**Description:** Foreign key constraints may slow down write operations
- **Probability:** Medium
- **Impact:** Medium
- **Mitigation:**
  - Benchmark write performance with and without FK constraints
  - Use prepared statements to minimize parsing overhead
  - Monitor WAL file size and checkpoint frequency
- **Contingency:** Adjust busy_timeout if lock contention occurs

### Risk 2: Index Overhead on Writes

**Description:** 15 new indexes may degrade insert/update performance
- **Probability:** Low
- **Impact:** Medium
- **Mitigation:**
  - Create indexes AFTER bulk data operations
  - Use batch inserts (BEGIN TRANSACTION ... COMMIT) for multiple writes
  - Monitor index rebuild time during VACUUM operations
- **Contingency:** Remove redundant indexes if overhead >20% on writes

### Risk 3: WAL Mode Compatibility Issues

**Description:** WAL mode may not be supported on all file systems (e.g., NFS)
- **Probability:** Low
- **Impact:** High
- **Mitigation:**
  - Verify file system compatibility before deployment
  - Test WAL mode in staging environment identical to production
  - Document rollback to DELETE journal mode if needed
- **Contingency:** Use DELETE journal mode with serialized access pattern

### Risk 4: Test Coverage Gaps

**Description:** Unit tests may miss edge cases or constraint violations
- **Probability:** Medium
- **Impact:** Medium
- **Mitigation:**
  - Use property-based testing (Hypothesis) for constraint validation
  - Test all foreign key cascade rules (SET NULL, CASCADE)
  - Stress test with 1000+ inserts/updates to detect race conditions
- **Contingency:** Add integration tests in Milestone 2 to catch gaps

---

## Go/No-Go Decision Criteria

### Prerequisites (Must be Complete)

1. ✅ All DDL scripts reviewed and approved by tech lead
2. ✅ Development database initialized successfully
3. ✅ All foreign key relationships validated
4. ✅ Unit test suite passing with 90%+ coverage

### Validation Checks (Must Pass)

1. **Database Integrity:**
   - `PRAGMA integrity_check` returns "ok"
   - `PRAGMA foreign_key_check` returns no violations
   - All indexes show "USING INDEX" in EXPLAIN QUERY PLAN

2. **Performance Targets:**
   - Task queries <50ms (99th percentile)
   - Agent queries <50ms (99th percentile)
   - Audit queries <50ms (99th percentile)

3. **Functional Correctness:**
   - Tasks with invalid session_id are allowed (NULL) but not non-existent FK
   - Agents properly link to both tasks and sessions
   - Audit table captures all memory operations with correct metadata

### Rollback Plan (If Go/No-Go Fails)

1. **Stop all development activity** on Milestone 1
2. **Analyze failure root cause** (performance, correctness, compatibility)
3. **Execute rollback procedure:**
   - Delete development database: `rm abathur.db`
   - Re-initialize from Phase 2 specifications
   - Apply fixes based on root cause analysis
4. **Re-test and re-validate** before proceeding

---

## Post-Milestone Activities

### Documentation Updates

- [ ] Update API documentation with new Database methods
- [ ] Document performance baseline metrics
- [ ] Create runbook for database initialization
- [ ] Update developer onboarding guide with setup instructions

### Knowledge Transfer

- [ ] Conduct team walkthrough of enhanced schema
- [ ] Demonstrate EXPLAIN QUERY PLAN usage for query optimization
- [ ] Share performance baseline results with stakeholders

### Preparation for Milestone 2

- [ ] Review Milestone 2 requirements (sessions, memory_entries tables)
- [ ] Allocate development resources for SessionService and MemoryService
- [ ] Setup integration test environment for session-task-memory workflows

---

## Success Metrics

### Quantitative Metrics

| Metric | Target | Actual | Status |
|--------|--------|--------|--------|
| Unit test coverage | 90%+ | ___ % | ⏳ Pending |
| Query latency (99th %ile) | <50ms | ___ ms | ⏳ Pending |
| Index usage rate | 100% | ___ % | ⏳ Pending |
| Foreign key violations | 0 | ___ | ⏳ Pending |
| Code review approval | 100% | ___ % | ⏳ Pending |

### Qualitative Metrics

- [ ] Code maintainability: High (clear structure, documented)
- [ ] Test reliability: High (deterministic, no flaky tests)
- [ ] Performance predictability: High (consistent query times)
- [ ] Error handling: Comprehensive (all edge cases covered)

---

## Lessons Learned (Post-Milestone)

### What Went Well

_To be filled after milestone completion_

### What Could Be Improved

_To be filled after milestone completion_

### Action Items for Future Milestones

_To be filled after milestone completion_

---

## References

**Phase 2 Technical Specifications:**
- [DDL Core Tables](../phase2_tech_specs/ddl-core-tables.sql) - Enhanced table definitions
- [DDL Indexes](../phase2_tech_specs/ddl-indexes.sql) - Core index definitions
- [Implementation Guide](../phase2_tech_specs/implementation-guide.md) - Deployment procedures
- [Test Scenarios](../phase2_tech_specs/test-scenarios.md) - Unit test examples

**Phase 3 Implementation Plan:**
- [Testing Strategy](./testing-strategy.md) - Comprehensive testing approach
- [Migration Procedures](./migration-procedures.md) - Database initialization steps
- [Rollback Procedures](./rollback-procedures.md) - Emergency rollback guide

---

**Milestone Version:** 1.0
**Author:** implementation-planner
**Date:** 2025-10-10
**Status:** Ready for Execution
**Next Milestone:** [Milestone 2: Memory Management System](./milestone-2-memory-system.md)
