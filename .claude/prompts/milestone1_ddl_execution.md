# Milestone 1: DDL Execution and Database Initialization

## Mission
Execute all DDL scripts to deploy the enhanced database schema with memory management features for Milestone 1 of the SQLite Schema Redesign project.

## Context

### Project Status
- **Phase 1-3:** COMPLETE (9.5/10 validation, zero unresolved decision points)
- **Current Milestone:** Milestone 1 - Core Schema Foundation (Weeks 1-2, 86 hours)
- **Your Role:** Execute DDL scripts and initialize database with new schema

### Current State
- Database exists at: `/Users/odgrim/dev/home/agentics/abathur/src/abathur/infrastructure/database.py`
- Current schema: OLD schema WITHOUT memory features
- NO CODE IMPLEMENTED YET for new schema

### Your Deliverables
1. Enhanced Database class with new table creation methods
2. All 9 tables created (3 new + 6 enhanced)
3. All 33 indexes created successfully
4. PRAGMA configuration (WAL mode, foreign keys ON)
5. Integrity validation (PRAGMA checks passing)
6. Documentation of any issues encountered

## DDL Scripts to Execute (IN ORDER)

### Script 1: ddl-memory-tables.sql
**Location:** `/Users/odgrim/dev/home/agentics/abathur/design_docs/phase2_tech_specs/ddl-memory-tables.sql`
**Creates:** sessions, memory_entries, document_index tables
**Execute:** FIRST (provides FK targets for core tables)

### Script 2: ddl-core-tables.sql
**Location:** `/Users/odgrim/dev/home/agentics/abathur/design_docs/phase2_tech_specs/ddl-core-tables.sql`
**Enhances:** tasks, agents, audit, checkpoints, state, metrics tables
**Adds:** session_id FK columns, memory operation tracking columns
**Execute:** SECOND (after sessions table exists)

### Script 3: ddl-indexes.sql
**Location:** `/Users/odgrim/dev/home/agentics/abathur/design_docs/phase2_tech_specs/ddl-indexes.sql`
**Creates:** 33 performance indexes
**Execute:** THIRD (after all tables exist)

## Implementation Strategy

### Approach: Enhance Existing Database Class
**DO NOT** create standalone scripts. Instead, enhance the existing Database class at:
`/Users/odgrim/dev/home/agentics/abathur/src/abathur/infrastructure/database.py`

### Required Changes

#### 1. Update _create_tables() Method
Replace existing table creation with DDL from scripts:
- Create NEW tables: sessions, memory_entries, document_index
- ENHANCE existing tables: tasks (add session_id), agents (add session_id), audit (add memory columns), checkpoints (add session_id)
- KEEP existing tables: state, metrics (no changes)

#### 2. Create New Helper Methods
```python
async def _create_memory_tables(self, conn: Connection) -> None:
    """Create new memory management tables (sessions, memory_entries, document_index)."""
    # Execute DDL from ddl-memory-tables.sql

async def _create_core_indexes(self, conn: Connection) -> None:
    """Create all 33 performance indexes."""
    # Execute DDL from ddl-indexes.sql

async def validate_foreign_keys(self) -> List[Tuple]:
    """Run PRAGMA foreign_key_check and return violations."""

async def explain_query_plan(self, query: str, params: Tuple = ()) -> List[str]:
    """Return EXPLAIN QUERY PLAN output for query optimization."""
```

#### 3. Update initialize() Method
Ensure PRAGMA settings are correct:
```python
await conn.execute("PRAGMA journal_mode=WAL")
await conn.execute("PRAGMA synchronous=NORMAL")
await conn.execute("PRAGMA foreign_keys=ON")
await conn.execute("PRAGMA busy_timeout=5000")
await conn.execute("PRAGMA wal_autocheckpoint=1000")  # ADD THIS
```

## Execution Steps

### Step 1: Read All DDL Scripts
- Read ddl-memory-tables.sql
- Read ddl-core-tables.sql
- Read ddl-indexes.sql
- Understand execution order and dependencies

### Step 2: Read Current Database Implementation
- Read `/Users/odgrim/dev/home/agentics/abathur/src/abathur/infrastructure/database.py`
- Identify where to integrate new DDL
- Preserve existing functionality

### Step 3: Enhance _create_tables() Method
- Add sessions, memory_entries, document_index table creation
- Enhance tasks table (add session_id column and FK)
- Enhance agents table (add session_id column and FK)
- Enhance audit table (add memory_operation_type, memory_namespace, memory_entry_id columns and FKs)
- Enhance checkpoints table (add session_id column and FK)
- Keep state and metrics tables unchanged

### Step 4: Create Index Creation Method
- Create _create_core_indexes() method
- Execute all 33 index creation statements from ddl-indexes.sql

### Step 5: Add Validation Utilities
- Create validate_foreign_keys() method
- Create explain_query_plan() method
- Add integrity check helpers

### Step 6: Test Database Initialization
- Create a test script to initialize database
- Verify all tables created
- Verify all indexes created
- Run PRAGMA integrity_check
- Run PRAGMA foreign_key_check

## Acceptance Criteria

### Must Pass
- [ ] All 9 tables created (sessions, memory_entries, document_index, tasks, agents, audit, checkpoints, state, metrics)
- [ ] All session_id FK columns added to tasks, agents, checkpoints
- [ ] All memory columns added to audit table
- [ ] All 33 indexes created successfully
- [ ] PRAGMA integrity_check returns "ok"
- [ ] PRAGMA foreign_key_check returns no violations
- [ ] PRAGMA journal_mode returns "wal"
- [ ] PRAGMA foreign_keys returns "1" (enabled)

### Code Quality
- [ ] No breaking changes to existing Task/Agent operations
- [ ] Clear comments documenting new tables/columns
- [ ] Proper error handling for DDL execution
- [ ] Type annotations maintained

## Error Handling
- If you encounter ANY errors during DDL execution:
  1. Document the exact error message
  2. Identify which DDL statement failed
  3. Check for FK dependencies issues
  4. Invoke @python-debugging-specialist if blocked
  5. DO NOT continue if tables are in inconsistent state

## Performance Validation
After successful execution, validate index usage:
```python
# Example query to verify index usage
query = "SELECT * FROM memory_entries WHERE namespace = ? AND key = ? AND is_deleted = 0 ORDER BY version DESC LIMIT 1"
plan = await db.explain_query_plan(query, ("user:test", "key"))
# Verify "USING INDEX" appears in plan output
```

## Deliverables
1. **Enhanced database.py** with all new tables and indexes
2. **Validation report** showing:
   - All tables created (list from sqlite_master)
   - All indexes created (list from sqlite_master)
   - PRAGMA check results
   - Sample EXPLAIN QUERY PLAN outputs
3. **Any issues encountered** and how they were resolved

## Reference Documents
- Design: `/Users/odgrim/dev/home/agentics/abathur/design_docs/phase1_design/`
- Tech Specs: `/Users/odgrim/dev/home/agentics/abathur/design_docs/phase2_tech_specs/`
- Implementation: `/Users/odgrim/dev/home/agentics/abathur/design_docs/phase3_implementation/milestone-1-core-schema.md`
- Current DB: `/Users/odgrim/dev/home/agentics/abathur/src/abathur/infrastructure/database.py`

## Success Criteria
- Database class successfully enhanced with all DDL integrated
- All validation checks pass
- No breaking changes to existing functionality
- Ready for python-api-developer to add SessionService and MemoryService
- Ready for test-automation-engineer to implement test suite

---
**Priority:** CRITICAL - Milestone 1 Week 1 blocker
**Estimated Effort:** 12-16 hours
**Dependencies:** None (first implementation task)
**Blocks:** python-api-developer, test-automation-engineer
