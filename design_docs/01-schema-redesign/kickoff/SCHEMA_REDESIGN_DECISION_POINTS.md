# Decision Points - SQLite Schema Redesign for Memory Management

## Project Context
This document captures all architectural, technical, and business decisions that require human input BEFORE implementation begins. Resolving these decisions upfront prevents agent blockages during execution.

**IMPORTANT:** This is a SEPARATE project from the main Abathur implementation. This focuses specifically on redesigning the database schema to incorporate memory management patterns from Chapter 8.

---

## Architecture Decisions

### 1. Vector Database Integration Strategy

**Question:** How should we integrate vector database capabilities for semantic memory search?

**Context:** Claude Code CLI agents cannot directly interact with embeddings. Phased approach needed: (1) Design schema with embedding support, (2) Build infrastructure post-schema, (3) Add MCP tools for agent access.

**Options:**
- [ ] Embedded SQLite extension (sqlite-vss or similar) - keeps everything in SQLite
- [ ] Separate vector database (ChromaDB, Pinecone, Weaviate) - dedicated solution
- [ ] Hybrid approach - store embeddings in SQLite, use external service for similarity search
- [ ] Defer vector support to Phase 2 - implement structured memory first
- [ ] Schema now, infrastructure later - design tables for embeddings, build MCP/CLI tools after

**Suggestion:** Design schema WITH embedding support (document_index table, vector columns), but defer infrastructure implementation until after schema is deployed. This allows agents to use traditional search (Grep/Glob) during schema redesign, then add MCP server + auto-sync service in Phase 2.

**Decision:** Schema now, infrastructure later (phased approach)

**Rationale:**
- Claude Code CLI has no native vector search capabilities
- This schema redesign project builds the foundation for embeddings
- Agents can write markdown files effectively without embeddings
- Schema includes document_index table with embedding BLOB columns
- Post-schema deployment: build MCP server, sync service, CLI tools
- Phased rollout: Phase 1 (basic schema) → Phase 2 (embedding infrastructure) → Phase 3 (MCP integration)
- sqlite-vss extension chosen for unified storage (can add post-schema)
- Maintains markdown files as source of truth (git tracked, human readable)
- Embeddings serve as search index, not primary storage

---

### 2. Memory Lifecycle Management

**Question:** How should we handle memory cleanup and retention policies?

**Options:**
- [ ] Manual cleanup - human decides when to archive/delete memory
- [ ] Automatic TTL (time-to-live) - memories expire after X days/hours
- [ ] Importance-based - keep high-value memories, archive low-value
- [ x ] Hybrid - automatic TTL for temp:, manual for user: and app:

**Decision** Hybrid approach - automatic cleanup for temp: prefix (session-scoped), TTL for episodic memory (30-90 days), manual/permanent for semantic and procedural memory.

---

### 3. Session Isolation Strategy

**Question:** How should we isolate session data for concurrent agent swarms?

**Options:**
- [ ] Full isolation - each session has completely separate state
- [ ] Shared memory - sessions can read from shared memory pool
- [ x ] Hierarchical - session-specific overrides global memory
- [ ] Configurable - per-task setting for isolation level

**Suggestion:** Hierarchical approach with session: prefix for session-specific, user: for user-scoped, app: for application-wide. Sessions can read all levels but only write to session: and user:.

**Decision** Hierarchical approach with session: prefix for session-specific, user: for user-scoped, app: for application-wide. Sessions can read all levels but only write to session: and user:.


---

## Technology Stack Decisions

### 4. Embedding Model Selection

**Question:** Which embedding model should we use for semantic memory?

**Context:** Abathur memory primarily contains text/markdown (95%: user interactions, task descriptions, agent instructions, preferences, documentation) with rare code snippets (5%). Optimization for general text retrieval is more important than code-specific embeddings.

**Options:**
- [ x ] nomic-embed-text-v1.5 (768 dims, 8K context, local via Ollama, optimized for text)
- [ ] stella-en-400M-v5 (#1 on MTEB retrieval, 1024 dims, MRL-enabled, local)
- [ ] all-mpnet-base-v2 (768 dims, proven/stable, local)
- [ ] OpenAI text-embedding-3-small (1536 dims, API-based, higher cost)
- [ ] Code-specific models like SFR-Embedding-Code (NOT recommended - optimized for code search, worse for general text)

**Suggestion:** Use nomic-embed-text-v1.5 as primary model deployed via Ollama for zero-cost local inference. This model excels at long-context text retrieval (8192 tokens), outperforms OpenAI on MTEB benchmarks, and handles occasional code snippets adequately. Alternative: stella-en-400M-v5 for best open-source retrieval performance.

**Decision:** nomic-embed-text-v1.5 (via Ollama)

**Rationale:**
- 95% of memory content is text/markdown (user preferences, task descriptions, agent instructions, episodic/semantic/procedural memory)
- nomic-embed-text-v1.5 optimized for general text and long-context retrieval (8K tokens)
- Outperforms OpenAI on MTEB benchmarks for text tasks
- Local deployment via Ollama: zero API costs, complete privacy, no rate limits
- 768 dimensions: efficient storage (~3KB per embedding), proven performance
- Apache 2.0 license: fully open source
- Handles rare code snippets adequately (code-specific models would sacrifice text performance)
- MTEB benchmark (not CoIR) is the correct evaluation metric for this use case

---

### 5. Migration Approach

**Question:** How should we handle migration from current schema to new schema?

**Options:**
- [ ] Full migration - migrate all data in one operation (requires downtime)
- [ ] Incremental migration - gradual rollout with dual-write period
- [ ] Parallel deployment - run old and new schemas side-by-side
- [ ] Blue-green deployment - switch traffic to new schema atomically

**Suggestion:** Full migration with comprehensive testing on copy of production database. Schedule maintenance window for production migration. Estimated 2-4 hours with rollback capability.

**Decision:** Don't bother, this is a brand new project so we can start from scratch.

---

## Business Logic Decisions

### 6. Memory Consolidation Strategy

**Question:** How should we handle conflicting memories (e.g., user changes preference)?

**Options:**
- [ ] Last-write-wins - newer memory overwrites older
- [ ] Versioning - keep history of all memory changes
- [ ] Conflict resolution - flag conflicts for manual resolution
- [ ] LLM-based - use Claude to intelligently merge conflicting memories

**Suggestion:** Versioning for critical user: and app: memories with soft-delete (updated_at timestamp). LLM-based consolidation for complex conflicts.
**Decision** Versioning for critical user: and app: memories with soft-delete (updated_at timestamp). LLM-based consolidation for complex conflicts.


---

### 7. Cross-Agent Memory Sharing

**Question:** How should agents share memories across different tasks/swarms?

**Options:**
- [ ] Fully shared - all agents see all memories
- [ ] Project-scoped - memories scoped to project_id
- [ ] Agent-type-scoped - only similar agents share memories
- [ ] Explicit sharing - require explicit memory publication

**Suggestion:** Project-scoped with namespace hierarchy: project:task:agent for granular control. Default read access to parent namespaces.

**Decision** Project-scoped with namespace hierarchy: project:task:agent for granular control. Default read access to parent namespaces.

---

## Performance Requirements

### 8. Concurrent Access Patterns

**Question:** What level of concurrent access must the new schema support?

**Options:**
- [ ] 10+ concurrent agents (current maximum)
- [ x ] 50+ concurrent agents (anticipated growth)
- [ ] 100+ concurrent agents (large-scale deployment)
- [ ] Unlimited (design for horizontal scalability)

**Suggestion:** Design for 50+ concurrent agents with WAL mode, consider sharding strategy for 100+ future scale.

---

### 9. Query Performance Targets

**Question:** What are the acceptable latency targets for memory operations?

**Options:**
- [ ] <10ms for memory reads (aggressive)
- [ x ] <50ms for memory reads (reasonable)
- [ ] <100ms for memory reads (acceptable)
- [ ] <500ms for semantic search (complex queries)

**Suggestion:** <50ms for exact-match reads, <500ms for semantic similarity search. Design indexes to support these targets.


---

### 10. Storage Scalability

**Question:** How much memory data should we plan to store?

**Options:**
- [ ] 1GB total (small-scale)
- [ x ] 10GB total (medium-scale)
- [ ] 100GB total (large-scale)
- [ ] 1TB+ total (enterprise-scale)

**Suggestion:** Design for 10GB with clear archival strategy. SQLite handles this well with WAL mode.


---

## Security & Compliance

### 11. Sensitive Data Handling

**Question:** How should we handle sensitive data in memory storage?

**Options:**
- [ x ] No special handling - store as-is
- [ ] Encryption at rest - encrypt sensitive fields
- [ ] Encryption in transit - TLS for all access
- [ ] Both encryption at rest and in transit

**Suggestion:** Encryption at rest for user: prefixed memories using SQLite encryption extension (SQLCipher). TLS already handled by application layer.

---

### 12. Audit Requirements

**Question:** What level of audit logging is required for memory operations?

**Options:**
- [ ] None - rely on application logging
- [ ] Basic - log all write operations
- [ x ] Comprehensive - log all read and write operations with timestamps
- [ ] Full audit trail - log with user context and timestamps

**Suggestion:** Comprehensive audit logging for all memory modifications, basic logging for reads. Use existing audit table with memory-specific columns.

---

## Integration Specifications

### 13. Backward Compatibility

**Question:** Must the new schema maintain backward compatibility with existing code?

**Options:**
- [ ] Full compatibility - existing code works without changes
- [ ] API compatibility - maintain same APIs, change internal structure
- [ x ] Breaking changes acceptable - update all calling code
- [ ] Deprecation period - support both old and new for transition

**Suggestion:** API compatibility with deprecation warnings for old patterns. Provide migration guide for application code updates.


---

### 14. Project Structure Integration

**Question:** How should we integrate project/workspace concepts into memory?

**Options:**
- [ ] Add project_id to all tables
- [ ] Namespace-based (project: prefix)
- [ ] Separate databases per project
- [ ] No project isolation - single global memory

**Suggestion:** Namespace-based with project: prefix, allows flexible multi-tenant isolation without database proliferation.
**Decision** Namespace-based with project: prefix, allows flexible multi-tenant isolation without database proliferation.


---

## UI/UX Decisions

### 15. Memory Visualization

**Question:** How should users interact with and visualize memory data?

**Options:**
- [ x ] CLI commands only (abathur memory list, etc.)
- [ ] Web dashboard (future feature)
- [ ] API endpoints for external tools
- [ ] All of the above

**Suggestion:** Start with comprehensive CLI commands, design API for future dashboard. Provide JSON output for tool integration.

---

## Implementation Timeline

### 16. Deployment Schedule

**Question:** When should this schema redesign be deployed?

**Options:**
- [ x ] Immediate (next release)
- [ ] Next major version (breaking changes acceptable)
- [ ] Phased rollout (incremental deployment)
- [ ] Experimental feature (opt-in beta)

**Suggestion:** Next major version (v2.0) with comprehensive testing period. Experimental opt-in during v1.x for early adopters.

---

## Additional Decisions Discovered During Implementation

### Decision 17: Document Storage Strategy

**Question:** Should design documents be stored as markdown files, SQLite records, or both?

**Context:** Agents generate design documents during projects. Need to balance human readability (git, PR reviews) with agent searchability (semantic retrieval).

**Options:**
- [ ] Markdown files only - traditional file-based storage
- [ ] SQLite only - database as source of truth
- [ x ] Hybrid - markdown files as source, SQLite as index
- [ ] Hybrid - SQLite as source, markdown as export

**Suggestion:** Hybrid approach with markdown files as source of truth. Agents write .md files to design_docs/, organized in phase directories. Schema includes document_index table that stores embeddings + metadata pointing to file paths. Background sync service (future) auto-embeds markdown changes.

**Decision:** Hybrid (markdown source + SQLite index)

**Rationale:**
- Markdown files = source of truth (human readable, git tracked, portable)
- SQLite document_index = search optimization (embeddings, metadata, fast retrieval)
- Agents write markdown natively (Claude Code CLI excellent at file operations)
- Humans review via GitHub PRs (standard workflow)
- Version control via git (full history, diffs, rollback)
- No vendor lock-in (documents exist independently of database)
- Future: MCP server enables semantic search via embeddings
- Background sync keeps embeddings fresh when files change
- Best of both worlds: human collaboration + agent search

---

## Notes

- All decisions should be resolved BEFORE invoking the schema-redesign-orchestrator
- Document rationale for all decisions to maintain context
- Update this document if new decision points are discovered during implementation
- Archive this document with project deliverables for future reference

**Last Updated:** 2025-10-10 (Decisions #1, #4, #17 resolved - phased embedding approach)
**Status:** 3/17 decisions resolved - awaiting remaining human input
