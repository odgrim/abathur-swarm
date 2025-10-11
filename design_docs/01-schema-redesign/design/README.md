# Phase 1: Design Proposal - SQLite Schema Redesign for Memory Management

## Overview

This directory contains the comprehensive design proposal for redesigning the Abathur SQLite database schema to incorporate memory management patterns from Chapter 8: Memory Management.

**Project Objective:** Transform the current task-centric database into a comprehensive memory-aware system supporting all memory types (short-term, long-term, semantic, episodic, procedural) with session management and hierarchical namespace organization.

**Status:** Phase 1 Design Complete - Awaiting Validation Gate

**Created:** 2025-10-10

## Document Structure

### Core Design Documents

1. **[memory-architecture.md](./memory-architecture.md)** (15K tokens)
   - Complete memory system design
   - Memory types taxonomy (semantic, episodic, procedural)
   - Hierarchical namespace architecture (session:, user:, app:, temp:, project:)
   - Session lifecycle and event tracking patterns
   - Memory consolidation and conflict resolution strategies

2. **[schema-tables.md](./schema-tables.md)** (20K tokens)
   - Complete DDL for all tables (sessions, memory_entries, document_index)
   - Column specifications with data types and constraints
   - Integration with existing tables (tasks, agents, audit, metrics)
   - Audit enhancements for memory operations

3. **[schema-relationships.md](./schema-relationships.md)** (10K tokens)
   - ER diagrams showing all table relationships
   - Foreign key specifications
   - Cascade rules and referential integrity
   - Access pattern diagrams

4. **[schema-indexes.md](./schema-indexes.md)** (10K tokens)
   - Performance optimization indexes
   - Composite indexes for common query patterns
   - Partial indexes for memory types
   - Vector search preparation (sqlite-vss)

5. **[migration-strategy.md](./migration-strategy.md)** (15K tokens)
   - Fresh start approach (new project)
   - Data seeding strategies
   - Testing and validation procedures
   - Rollback and recovery plans

## Key Architectural Decisions

### Resolved Decision Points

1. **Vector Database Integration:** Schema now, infrastructure later (phased approach with sqlite-vss)
2. **Embedding Model:** nomic-embed-text-v1.5 (768 dims, deployed via Ollama)
3. **Memory Lifecycle:** Hybrid - automatic TTL for temp:, manual for user:/app:
4. **Session Isolation:** Hierarchical with session:, user:, app: prefixes
5. **Migration Approach:** Fresh start (new project, no migration needed)
6. **Memory Consolidation:** Versioning with soft-delete, LLM-based conflict resolution
7. **Cross-Agent Sharing:** Project-scoped with namespace hierarchy
8. **Concurrent Access:** Designed for 50+ concurrent agents
9. **Query Performance:** <50ms reads, <500ms semantic search
10. **Storage Scalability:** 10GB target with archival strategy
11. **Audit Requirements:** Comprehensive logging for all modifications
12. **Document Storage:** Hybrid (markdown source + SQLite index)

## Memory System Design Summary

### Memory Types Supported

1. **Short-Term Memory (Contextual)**
   - Session-scoped state with temp: prefix
   - Limited context window (ephemeral)
   - Automatic cleanup on session termination

2. **Long-Term Memory (Persistent)**
   - **Semantic Memory:** Facts, preferences, domain knowledge
   - **Episodic Memory:** Past events, successful/failed strategies (30-90 day TTL)
   - **Procedural Memory:** Rules, instructions, learned behaviors (permanent)

### Hierarchical Namespace Design

```
project:<project_id>:                  # Project-wide shared memory
  user:<user_id>:                      # User-specific across sessions
    session:<session_id>:              # Session-specific temporary
      temp:<temp_key>                  # Temporary (current turn only)
  app:<app_name>:                      # Application-wide shared memory
```

**Access Rules:**
- Sessions can READ from all parent namespaces (project, user, app)
- Sessions can WRITE to session: and user: namespaces only
- app: namespace requires elevated permissions
- temp: is never persisted (exists only during event processing)

### Session Management

- **Session:** Individual chat thread with unique ID
- **Events:** Chronological record of messages, actions, tool calls
- **State:** Key-value dictionary for session-specific temporary data
- **Lifecycle:** create → active → paused → terminated → archived

### Memory Consolidation Strategy

1. **Versioning:** All user: and app: memories maintain version history
2. **Soft-Delete:** is_deleted flag prevents data loss
3. **Conflict Resolution:**
   - Last-write-wins for simple updates
   - LLM-based consolidation for complex contradictions
   - Manual review flag for critical conflicts

## Performance Targets

- **Read Latency:** <50ms for exact-match queries
- **Semantic Search:** <500ms for vector similarity (post-infrastructure)
- **Concurrent Agents:** 50+ simultaneous sessions
- **Storage:** 10GB working set, archival strategy for historical data
- **WAL Mode:** Enabled for concurrent read/write access

## Integration Points

### Existing Schema Preservation

Existing tables (tasks, agents, state, audit, metrics, checkpoints) are **preserved** with enhancements:

- **tasks:** Add session_id foreign key for session-task linkage
- **agents:** Add session_id for agent-session tracking
- **state:** Deprecated in favor of sessions.state (maintain for backward compat)
- **audit:** New memory_operation_type column for memory-specific logging
- **metrics:** No changes required
- **checkpoints:** Add session_id for session-based checkpointing

### New Tables

1. **sessions:** Core session management (events, state, lifecycle)
2. **memory_entries:** Long-term memory storage with namespace hierarchy
3. **document_index:** Document metadata + embeddings for semantic search

## Next Steps (Phase 2)

Upon approval of Phase 1 design:

1. **Technical Specifications Writer** creates:
   - Complete DDL implementation scripts
   - Query pattern specifications with prepared statements
   - API definitions for memory operations
   - Test scenarios and validation queries

2. **Implementation Planner** creates:
   - Phased rollout roadmap
   - Testing strategy and acceptance criteria
   - Migration procedures and rollback plans
   - Risk assessment and mitigation strategies

## Validation Criteria

This design must pass the following validation gates:

- [ ] All memory types from Chapter 8 addressed
- [ ] Hierarchical namespace design with clear scoping rules
- [ ] Complete ER diagrams showing all relationships
- [ ] Migration strategy addresses data preservation
- [ ] Schema supports current requirements plus memory management
- [ ] All 10 core requirements addressed
- [ ] Document size limits respected (max 20K tokens per file)
- [ ] Performance targets achievable with proposed indexes

## 10 Core Requirements Coverage

1. **Task State Management:** ✓ tasks table enhanced with session_id
2. **Task Dependencies:** ✓ existing dependencies column preserved
3. **Task Context & State:** ✓ session state + memory_entries
4. **Project State Management:** ✓ project: namespace in memory_entries
5. **Session Management:** ✓ new sessions table with events/state
6. **Memory Management:** ✓ memory_entries (semantic, episodic, procedural)
7. **Agent State Tracking:** ✓ agents table with session_id linkage
8. **Learning & Adaptation:** ✓ procedural memory + episodic feedback
9. **Context Synthesis:** ✓ hierarchical namespace retrieval
10. **Audit & History:** ✓ audit table enhanced for memory operations

## File Organization

```
phase1_design/
├── README.md                       # This file (navigation and summary)
├── memory-architecture.md          # Complete memory system design
├── schema-tables.md                # DDL and table specifications
├── schema-relationships.md         # ER diagrams and relationships
├── schema-indexes.md               # Performance indexes
└── migration-strategy.md           # Fresh start approach
```

## References

- **Memory Management Chapter:** `/Users/odgrim/dev/home/agentics/abathur/design_docs/Chapter 8_ Memory Management.md`
- **Current Schema:** `/Users/odgrim/dev/home/agentics/abathur/src/abathur/infrastructure/database.py`
- **Decision Points:** `/Users/odgrim/dev/home/agentics/abathur/design_docs/SCHEMA_REDESIGN_DECISION_POINTS.md`
- **Orchestration Plan:** `/Users/odgrim/dev/home/agentics/abathur/design_docs/SCHEMA_REDESIGN_ORCHESTRATION_REPORT.md`

---

**Document Version:** 1.0
**Last Updated:** 2025-10-10
**Status:** Awaiting Phase 1 Validation Gate
**Next Phase:** Technical Specifications (Phase 2)
