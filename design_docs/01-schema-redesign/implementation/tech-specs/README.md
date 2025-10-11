# Phase 2: Technical Specifications - SQLite Schema Redesign for Memory Management

## Overview

This directory contains production-ready technical specifications for implementing the memory management schema design approved in Phase 1. These specifications transform the architectural design into executable DDL, optimized query patterns, complete Python APIs, and comprehensive implementation guides.

**Phase 1 Completion:** Design validated with score 9.5/10
**Phase 2 Objective:** Provide implementation-ready specifications for database initialization, query optimization, and API integration

**Created:** 2025-10-10
**Status:** Phase 2 Complete - Ready for Implementation Planning (Phase 3)

---

## Document Navigation

### 1. DDL Scripts (SQL)

**[ddl-core-tables.sql](./ddl-core-tables.sql)** - Enhanced existing tables
- Enhanced `tasks` table with session_id foreign key
- Enhanced `agents` table with session_id foreign key
- Enhanced `audit` table with memory operation tracking
- Enhanced `checkpoints` table with session_id foreign key
- Existing `state`, `metrics` tables (unchanged)
- Complete with all constraints, defaults, and CHECK clauses

**[ddl-memory-tables.sql](./ddl-memory-tables.sql)** - New memory management tables
- `sessions` table for conversation thread management
- `memory_entries` table for long-term persistent memory
- `document_index` table for markdown file indexing with embeddings
- Complete with JSON validation, versioning, and soft-delete support

**[ddl-indexes.sql](./ddl-indexes.sql)** - All 31 performance indexes
- Sessions indexes (4 indexes)
- Memory entries indexes (7 indexes)
- Document index indexes (5 indexes)
- Enhanced existing table indexes (15 indexes)
- Each index includes comments explaining supported query patterns

### 2. Query Specifications (Markdown)

**[query-patterns-read.md](./query-patterns-read.md)** - Read operations
- Session retrieval by composite key (app_name, user_id, session_id)
- Memory namespace hierarchy queries with EXPLAIN QUERY PLAN
- Document search by metadata and file path
- Task queries with session joins
- Performance analysis and optimization notes

**[query-patterns-write.md](./query-patterns-write.md)** - Write operations
- Session creation with event tracking
- Memory entry insert/update with versioning
- Document index updates with embedding sync
- Transaction patterns for ACID compliance
- Batch operation examples for performance

### 3. API Specifications (Markdown)

**[api-specifications.md](./api-specifications.md)** - Complete Python APIs
- `SessionService` class with full CRUD operations
- `MemoryService` class with namespace-aware retrieval
- `DocumentIndexService` class for file indexing
- Enhanced `Database` class integration
- Type annotations, docstrings, and example usage
- Error handling and validation logic

### 4. Testing & Integration (Markdown)

**[test-scenarios.md](./test-scenarios.md)** - Comprehensive test coverage
- Unit tests for each table (insert, update, delete, query)
- Constraint violation tests (foreign keys, unique constraints)
- Integration tests for workflows (session → task → memory)
- Performance tests (concurrency, latency benchmarks)
- Index usage verification with EXPLAIN QUERY PLAN

**[sqlite-vss-integration.md](./sqlite-vss-integration.md)** - Vector search guide
- sqlite-vss extension installation (macOS, Linux, Docker)
- Ollama setup for nomic-embed-text-v1.5 model
- Embedding generation workflow and background sync design
- Vector similarity search queries
- Hybrid exact + semantic search examples

**[implementation-guide.md](./implementation-guide.md)** - Step-by-step deployment
- Database initialization procedures (PRAGMA configuration)
- DDL execution order with dependency resolution
- Data seeding strategy (initial application memories)
- Validation procedures (integrity checks, query tests)
- Rollback procedures and disaster recovery
- Production deployment checklist

---

## Quick Start

### For Implementers

1. **Review Design Documents** (Phase 1)
   - Start with `../phase1_design/README.md` for context
   - Understand memory architecture, namespace hierarchy, session lifecycle

2. **Execute DDL Scripts** (in order)
   ```bash
   sqlite3 abathur.db < ddl-core-tables.sql
   sqlite3 abathur.db < ddl-memory-tables.sql
   sqlite3 abathur.db < ddl-indexes.sql
   ```

3. **Implement Python APIs**
   - Use `api-specifications.md` as reference
   - Copy code snippets from specifications
   - Integrate with existing `Database` class

4. **Run Tests**
   - Follow `test-scenarios.md` for test cases
   - Verify index usage with EXPLAIN QUERY PLAN
   - Benchmark performance targets (<50ms reads)

5. **Deploy to Production**
   - Follow `implementation-guide.md` checklist
   - Execute validation procedures
   - Monitor WAL size and checkpoint frequency

### For Code Reviewers

**Critical Files to Review:**
1. `ddl-memory-tables.sql` - New schema additions
2. `api-specifications.md` - Public API surface area
3. `implementation-guide.md` - Deployment risks and rollback

**Validation Checklist:**
- [ ] All foreign key relationships correct
- [ ] JSON validation constraints on all JSON columns
- [ ] Indexes cover all common query patterns
- [ ] API type annotations complete and accurate
- [ ] Transaction boundaries properly defined
- [ ] Rollback procedures tested and documented

---

## Performance Targets

All specifications designed to meet these validated targets:

| Operation | Target | Validation Method |
|-----------|--------|-------------------|
| Exact-match memory read | <50ms | EXPLAIN QUERY PLAN shows index usage |
| Session state retrieval | <10ms | Single-row lookup by primary key |
| Hierarchical namespace query | <50ms | Composite index on (namespace, key, version) |
| Semantic search (future) | <500ms | sqlite-vss with nomic-embed-text-v1.5 |
| Concurrent sessions | 50+ agents | WAL mode with 5s busy_timeout |
| Database size capacity | 10GB target | Archival strategy for historical data |

---

## Integration Points

### Existing Abathur Codebase

**Current Implementation:** `/Users/odgrim/dev/home/agentics/abathur/src/abathur/infrastructure/database.py`

**Integration Strategy:**
- Extend existing `Database` class with new methods (backward compatible)
- Add `SessionService`, `MemoryService` as separate classes
- Deprecate `state` table in favor of `sessions.state` (transition period)
- Enhance `audit` table without breaking existing audit logging

**Migration Approach:** Fresh start (new project, no data migration required)

### External Dependencies

**Required:**
- Python 3.11+
- aiosqlite (already in use)
- SQLite 3.35+ (for JSON functions, generated columns support)

**Optional (Phase 2+):**
- sqlite-vss extension (for vector similarity search)
- Ollama + nomic-embed-text-v1.5 model (for embedding generation)

---

## Success Criteria

Phase 2 specifications are complete when:

- [x] All DDL scripts execute without syntax errors
- [x] All 31 indexes defined with CREATE INDEX statements
- [x] Query patterns include EXPLAIN QUERY PLAN analysis
- [x] Python APIs have complete type annotations and docstrings
- [x] Test scenarios cover unit, integration, and performance tests
- [x] sqlite-vss integration guide is actionable (step-by-step)
- [x] Implementation guide enables fresh start deployment
- [x] All files respect size limits (max 20K tokens per file)
- [x] Cross-references between documents are accurate

---

## File Size Summary

| File | Approximate Size | Purpose |
|------|------------------|---------|
| README.md | 1.5K tokens | Navigation and overview |
| ddl-core-tables.sql | 8K tokens | Enhanced existing tables |
| ddl-memory-tables.sql | 10K tokens | New memory tables |
| ddl-indexes.sql | 9K tokens | All performance indexes |
| query-patterns-read.md | 12K tokens | Read query optimization |
| query-patterns-write.md | 10K tokens | Write query patterns |
| api-specifications.md | 18K tokens | Complete Python APIs |
| test-scenarios.md | 13K tokens | Comprehensive test coverage |
| sqlite-vss-integration.md | 10K tokens | Vector search setup |
| implementation-guide.md | 12K tokens | Deployment procedures |

**Total:** ~103K tokens across 10 deliverable files

---

## Next Steps (Phase 3)

Upon approval of Phase 2 specifications:

1. **Implementation Planner** creates:
   - Phased rollout roadmap (4-week timeline)
   - Testing strategy and acceptance criteria
   - Risk assessment and mitigation strategies
   - Resource allocation and timeline

2. **Development Team** executes:
   - Database initialization (Week 1)
   - API implementation (Week 2)
   - Testing and validation (Week 3)
   - Production deployment (Week 4)

---

## References

**Phase 1 Design Documents:**
- [Memory Architecture](../phase1_design/memory-architecture.md)
- [Schema Tables](../phase1_design/schema-tables.md)
- [Schema Relationships](../phase1_design/schema-relationships.md)
- [Schema Indexes](../phase1_design/schema-indexes.md)
- [Migration Strategy](../phase1_design/migration-strategy.md)

**Project Context:**
- [Decision Points](../SCHEMA_REDESIGN_DECISION_POINTS.md)
- [Memory Management Chapter](../Chapter 8_ Memory Management.md)
- [Current Database Implementation](../../src/abathur/infrastructure/database.py)

---

**Document Version:** 1.0
**Author:** technical-specifications-writer
**Date:** 2025-10-10
**Status:** Phase 2 Complete - Awaiting Implementation Planning (Phase 3)
