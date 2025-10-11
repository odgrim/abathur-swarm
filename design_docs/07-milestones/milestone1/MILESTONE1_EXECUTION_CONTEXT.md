# Milestone 1: Core Schema Foundation - Execution Context

## Project Status
- **Phase 1 Design:** COMPLETE (9.5/10 validation score)
- **Phase 2 Technical Specs:** COMPLETE (9.5/10 validation score)
- **Phase 3 Implementation Planning:** COMPLETE (9.5/10 validation score)
- **Current Phase:** IMPLEMENTATION EXECUTION
- **Milestone:** Milestone 1 - Core Schema Foundation (Weeks 1-2)

## Implementation Team
- **database-schema-implementer** (Thinking, Blue) - DDL execution, database initialization
- **python-api-developer** (Thinking, Green) - Database class enhancements
- **test-automation-engineer** (Thinking, Yellow) - Test suite and benchmarks
- **code-reviewer** (Sonnet, Cyan) - Code quality validation
- **performance-validator** (Sonnet, Orange) - Performance benchmarking
- **python-debugging-specialist** (Thinking, Pink) - On-demand debugging (invoke if blocked)

## Critical Context

### Current Database State
- **Location:** `/Users/odgrim/dev/home/agentics/abathur/src/abathur/infrastructure/database.py`
- **Schema:** OLD schema WITHOUT memory management features
- **Tables:** tasks, agents, state, audit, metrics, checkpoints
- **Missing:** sessions, memory_entries, document_index tables
- **Missing:** session_id foreign keys in tasks, agents, checkpoints
- **Missing:** memory operation columns in audit table
- **Missing:** 15 new performance indexes

### DDL Scripts Available
1. `/Users/odgrim/dev/home/agentics/abathur/design_docs/phase2_tech_specs/ddl-memory-tables.sql`
   - Creates: sessions, memory_entries, document_index tables
   - Execute FIRST (provides FK targets)

2. `/Users/odgrim/dev/home/agentics/abathur/design_docs/phase2_tech_specs/ddl-core-tables.sql`
   - Enhances: tasks, agents, audit, checkpoints, state, metrics
   - Adds session_id FK columns
   - Execute SECOND (references sessions table)

3. `/Users/odgrim/dev/home/agentics/abathur/design_docs/phase2_tech_specs/ddl-indexes.sql`
   - Creates 33 performance indexes
   - Execute THIRD (after all tables exist)

### Execution Order (CRITICAL)
1. Configure PRAGMA settings (WAL mode, foreign keys ON)
2. Execute ddl-memory-tables.sql
3. Execute ddl-core-tables.sql
4. Execute ddl-indexes.sql
5. Run PRAGMA integrity_check
6. Run PRAGMA foreign_key_check
7. Verify all 33 indexes created

## Milestone 1 Objectives

### Week 1: Database Schema Deployment (38 hours)
- [ ] Review Phase 2 DDL scripts (4h)
- [ ] Setup development database environment (4h)
- [ ] Execute PRAGMA configuration (WAL mode, foreign keys) (2h)
- [ ] Deploy enhanced tasks table with session_id FK (4h)
- [ ] Deploy enhanced agents table with session_id FK (4h)
- [ ] Deploy enhanced audit table with memory operation tracking (4h)
- [ ] Deploy enhanced checkpoints table with session_id FK (2h)
- [ ] Verify all table constraints (CHECK, UNIQUE, FK) (4h)
- [ ] Create core indexes (15 indexes for existing tables) (4h)
- [ ] Run PRAGMA integrity_check and foreign_key_check (2h)

### Week 2: Testing and Validation (48 hours)
- [ ] Implement Database class enhancements (8h)
- [ ] Write unit tests for tasks table operations (6h)
- [ ] Write unit tests for agents table operations (6h)
- [ ] Write unit tests for audit table operations (4h)
- [ ] Write unit tests for foreign key constraints (4h)
- [ ] Create performance benchmark suite (6h)
- [ ] Run performance baseline tests (<50ms reads) (4h)
- [ ] Verify EXPLAIN QUERY PLAN shows index usage (4h)
- [ ] Document baseline metrics and results (2h)
- [ ] Code review and quality assurance (4h)

## Acceptance Criteria

### Technical Validation
- [ ] All DDL scripts executed without errors
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

## Performance Targets
- **Exact-match reads:** <50ms (99th percentile)
- **Index usage:** 100% (all queries must use indexes via EXPLAIN QUERY PLAN)
- **Concurrent sessions:** Support 50+ concurrent sessions
- **Write overhead:** Acceptable 20-40% slower writes (read-heavy workload)

## Key Design Decisions (From Phase 1-3)
1. **Fresh Start Strategy:** No migration required, new database from scratch
2. **WAL Mode:** Enabled for concurrent reads (50+ sessions)
3. **Foreign Keys:** Always ON, enforced at database level
4. **Soft-Delete Pattern:** memory_entries uses is_deleted flag
5. **JSON Validation:** All JSON columns have CHECK(json_valid(...)) constraints
6. **Namespace Hierarchy:** Colon-separated (user:alice:preferences)
7. **Versioning:** memory_entries supports version history
8. **Indexes:** 33 total indexes (15 core + 18 new)

## Reference Documentation
- **Phase 1 Design:** `/Users/odgrim/dev/home/agentics/abathur/design_docs/phase1_design/`
- **Phase 2 Tech Specs:** `/Users/odgrim/dev/home/agentics/abathur/design_docs/phase2_tech_specs/`
- **Phase 3 Implementation:** `/Users/odgrim/dev/home/agentics/abathur/design_docs/phase3_implementation/`
- **Current Database:** `/Users/odgrim/dev/home/agentics/abathur/src/abathur/infrastructure/database.py`

## Validation Gate Requirements
At completion of Milestone 1, conduct MANDATORY validation:
1. **Deliverable Review:** All Week 1 + Week 2 tasks complete
2. **Technical Validation:** All acceptance criteria met
3. **Performance Assessment:** <50ms reads, 100% index usage
4. **Integration Testing:** All components work together
5. **Go/No-Go Decision:** Explicit approval required for Milestone 2

## Error Handling Protocol
- If ANY agent encounters errors or blocking issues:
  1. Document the error with full context
  2. Invoke @python-debugging-specialist with complete state
  3. Update TODO list to track blocking issue
  4. Do NOT proceed until issue resolved

## Success Metrics
- Unit test coverage: 90%+
- Query latency (99th percentile): <50ms
- Index usage rate: 100%
- Foreign key violations: 0
- Code review approval: 100%

---
**Document Version:** 1.0
**Created:** 2025-10-10
**Status:** READY FOR EXECUTION
**Next Step:** Invoke database-schema-implementer for DDL execution
