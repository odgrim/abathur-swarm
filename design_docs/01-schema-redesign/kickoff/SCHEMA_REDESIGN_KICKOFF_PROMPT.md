# SQLite Schema Redesign for Memory Management - Claude Code Kickoff Prompt

## CRITICAL: READ THIS FIRST

**WARNING: This kickoff prompt should ONLY be used AFTER all decision points in `SCHEMA_REDESIGN_DECISION_POINTS.md` have been resolved by a human.**

**Status Check:**
- [ ] All 17 decision points resolved in SCHEMA_REDESIGN_DECISION_POINTS.md (16 core + 1 additional)
- [ ] Design decisions documented with rationale
- [ ] Performance targets defined
- [ ] Security requirements specified
- [ ] Migration approach approved
- [ ] Embedding model and vector strategy confirmed
- [ ] Document storage strategy (markdown + SQLite) confirmed

**If all checkboxes above are checked, proceed with the prompt below. Otherwise, STOP and resolve decision points first.**

---

## Project Kickoff: SQLite Schema Redesign for Memory Management

I'm ready to begin redesigning the Abathur SQLite database schema to incorporate comprehensive memory management patterns based on "Chapter 8: Memory Management" from the AI agent systems book.

### Project Overview

**Objective:** Redesign the current SQLite schema to support short-term and long-term memory management, session state tracking, and hierarchical namespace organization following patterns from Google ADK, LangGraph, and Vertex AI Memory Bank.

**Current State:**
- Existing SQLite schema with tasks, agents, state, audit, metrics, and checkpoints tables
- WAL mode enabled for concurrent access
- Support for 10+ concurrent agents
- Basic state management without memory framework concepts

**Target State:**
- Comprehensive memory management supporting all memory types (short-term, long-term, semantic, episodic, procedural)
- Session management with event tracking and state isolation
- Hierarchical namespace organization (session:, user:, app:, temp:)
- Efficient query patterns for both exact-match and similarity search
- Backward-compatible migration strategy
- Enhanced auditability and observability

---

## Agent Team & Execution Sequence

**PROJECT ORCHESTRATOR: `schema-redesign-orchestrator`**

This orchestrator will coordinate the three-phase workflow with validation gates.

### Phase 1: Design Proposal

1. **`memory-systems-architect`** (Sonnet) - Design memory architecture
   - Analyzes memory management chapter and extracts all patterns
   - Designs comprehensive memory schema with hierarchical namespaces
   - Creates ER diagrams and access pattern specifications
   - **Deliverable:** Memory architecture document with table specifications

2. **`database-redesign-specialist`** (Opus) - Design complete schema
   - Analyzes current schema and integration points
   - Incorporates memory architecture into complete schema design
   - Creates migration strategy and rollback procedures
   - **Deliverable:** Complete schema redesign proposal with DDL and migration plan

3. **`schema-redesign-orchestrator`** (Sonnet) - **PHASE 1 VALIDATION GATE**
   - Reviews all design deliverables
   - Validates alignment with 10 core requirements
   - Makes go/no-go decision (APPROVE/CONDITIONAL/REVISE/ESCALATE)
   - Generates refined context for Phase 2

---

### Phase 2: Technical Specifications

4. **`technical-specifications-writer`** (Sonnet) - Create implementation specs
   - Transforms design into detailed technical specifications
   - Generates complete DDL with all constraints
   - Documents optimized query patterns with EXPLAIN QUERY PLAN
   - Defines Python API specifications with type annotations
   - **Deliverable:** Complete technical specifications (DDL, queries, APIs, tests)

5. **`schema-redesign-orchestrator`** (Sonnet) - **PHASE 2 VALIDATION GATE**
   - Reviews all technical specifications
   - Validates DDL syntax and query optimization
   - Assesses implementation readiness
   - Makes go/no-go decision (APPROVE/CONDITIONAL/REVISE/ESCALATE)
   - Generates refined context for Phase 3

---

### Phase 3: Implementation Plan

6. **`implementation-planner`** (Sonnet) - Create migration roadmap
   - Breaks implementation into phased milestones
   - Designs comprehensive testing strategy
   - Creates detailed migration and rollback procedures
   - Assesses risks and defines mitigation strategies
   - **Deliverable:** Complete implementation roadmap with testing and deployment plans

7. **`schema-redesign-orchestrator`** (Sonnet) - **PHASE 3 VALIDATION GATE (FINAL)**
   - Reviews all implementation deliverables
   - Validates deployment readiness
   - Makes final project approval decision
   - Generates project completion summary

---

## Success Criteria

### Phase 1 Success Criteria:
- [ ] Memory architecture covers all memory types (short-term, long-term, semantic, episodic, procedural)
- [ ] Hierarchical namespace design with clear scoping rules
- [ ] Complete ER diagrams showing all relationships
- [ ] Migration strategy addresses data preservation and rollback
- [ ] Schema supports current requirements plus memory management

### Phase 2 Success Criteria:
- [ ] Complete DDL for all tables with constraints and indexes
- [ ] Optimized query patterns with performance analysis
- [ ] Python API specifications with type annotations
- [ ] Comprehensive test scenarios defined
- [ ] Implementation guide with step-by-step instructions

### Phase 3 Success Criteria:
- [ ] Phased implementation roadmap with clear milestones
- [ ] Testing strategy covering unit, integration, and performance tests
- [ ] Detailed migration scripts with validation checks
- [ ] Complete rollback procedures for all changes
- [ ] Risk assessment with mitigation strategies

### Overall Project Success Criteria:
- [ ] All 10 core requirements addressed:
  1. Task State Management
  2. Task Dependencies
  3. Task Context & State
  4. Project State Management
  5. Session Management
  6. Memory Management (semantic, episodic, procedural)
  7. Agent State Tracking
  8. Learning & Adaptation
  9. Context Synthesis
  10. Audit & History
- [ ] Backward-compatible migration strategy
- [ ] Performance meets targets (<50ms reads, <500ms semantic search)
- [ ] Comprehensive documentation and test coverage
- [ ] Ready for production deployment

---

## Context Passing Instructions

After each agent completes their work, the orchestrator must invoke the next agent with:
- Summary of what was completed
- Files created/modified with absolute paths
- Key design decisions and rationale
- Resolved decision points from SCHEMA_REDESIGN_DECISION_POINTS.md
- Any issues or constraints discovered
- Validation results from previous phase (if applicable)

---

## Document Structure Requirements

**CRITICAL:** To ensure documents remain manageable for both agents and humans:

### File Size Limits:
- **Maximum 20K tokens per file** (prevents context overflow)
- **Split large documents** into logical sub-documents
- **Use README.md files** for navigation (1-2K tokens max)

### Directory Structure:
```
design_docs/
├── phase1_design/
│   ├── README.md                    # 1-2K: Overview + navigation
│   ├── memory-architecture.md       # Max 15K: Memory system design
│   ├── schema-tables.md             # Max 20K: Table definitions
│   ├── schema-relationships.md      # Max 10K: ER diagrams
│   ├── migration-strategy.md        # Max 15K: Migration approach
│   └── schema-indexes.md            # Max 10K: Index strategy
├── phase2_tech_specs/
│   ├── README.md                    # 1-2K: Specs overview
│   ├── ddl-core-tables.sql          # Max 15K: Core DDL
│   ├── ddl-memory-tables.sql        # Max 15K: Memory DDL
│   ├── ddl-indexes.sql              # Max 10K: Index DDL
│   ├── query-patterns-read.md       # Max 15K: Read queries
│   ├── query-patterns-write.md      # Max 15K: Write queries
│   ├── api-specifications.md        # Max 20K: Python APIs
│   ├── test-scenarios.md            # Max 15K: Test cases
│   └── implementation-guide.md      # Max 15K: Step-by-step
└── phase3_implementation/
    ├── README.md                    # 1-2K: Implementation overview
    ├── milestone-*.md               # Max 10K each
    ├── testing-strategy.md          # Max 15K
    ├── migration-procedures.md      # Max 15K
    ├── rollback-procedures.md       # Max 10K
    └── risk-assessment.md           # Max 10K
```

### Document Guidelines:
- **Separation of Concerns:** Keep tables, relationships, and indexes in separate files
- **Progressive Disclosure:** README → Detailed docs → Implementation files
- **Cross-References:** Use relative links between documents
- **Source of Truth:** Markdown files (git tracked, human readable)
- **Search Index:** SQLite document_index table (created by schema, populated later)

### Phased Embedding Approach:
**Phase 1 (Current - Schema Redesign):**
- Agents write markdown files only
- Use Grep/Glob for search (no embeddings yet)
- Schema INCLUDES document_index table design

**Phase 2 (Post-Schema Deployment):**
- Build MCP server for semantic search
- Implement background sync service
- Auto-embed markdown files

**Phase 3 (Future Enhancement):**
- Agents use MCP search tools
- RAG workflows for context retrieval
- Full semantic memory capabilities

---

## Critical Requirements for All Agents

1. **Reference Resolved Decision Points:**
   - All agents MUST reference `SCHEMA_REDESIGN_DECISION_POINTS.md` for architectural decisions
   - Agents should flag any NEW decision points discovered during their work
   - Never make assumptions about decisions that should be human-resolved

2. **Maintain Context:**
   - Each agent receives complete context from previous phase
   - Agents must document all design decisions with rationale
   - Pass forward critical information for subsequent agents

3. **Quality Gates:**
   - Orchestrator conducts thorough validation at each phase boundary
   - No phase progression without explicit approval
   - Document all validation decisions with clear rationale

4. **Deliverable Standards:**
   - All deliverables must be complete and self-contained
   - Use absolute file paths for all artifacts
   - Provide structured JSON output for orchestrator parsing
   - Include comprehensive documentation

---

## Initial Invocation

**COPY AND PASTE THIS INTO CLAUDE CODE (after resolving decision points):**

```
I'm ready to begin the SQLite Schema Redesign for Memory Management project.

I have completed all decision points in SCHEMA_REDESIGN_DECISION_POINTS.md and I'm ready to execute the three-phase workflow:
- Phase 1: Design Proposal (Memory Architecture + Schema Design)
- Phase 2: Technical Specifications (DDL + Queries + APIs)
- Phase 3: Implementation Plan (Migration + Testing + Deployment)

Please invoke the `schema-redesign-orchestrator` to begin Phase 1 with the memory-systems-architect.

Project Context:
- Current schema: /Users/odgrim/dev/home/agentics/abathur/src/abathur/infrastructure/database.py
- Memory chapter: /Users/odgrim/dev/home/agentics/abathur/design_docs/Chapter 8_ Memory Management.md
- Decision points: /Users/odgrim/dev/home/agentics/abathur/design_docs/SCHEMA_REDESIGN_DECISION_POINTS.md
- Deliverables directory: /Users/odgrim/dev/home/agentics/abathur/design_docs/

Begin Phase 1: Design Proposal
```

---

## Expected Timeline

**Phase 1: Design Proposal** - 30-45 minutes
- memory-systems-architect: 15-20 minutes
- database-redesign-specialist: 15-20 minutes
- Validation gate: 5 minutes

**Phase 2: Technical Specifications** - 20-30 minutes
- technical-specifications-writer: 20-25 minutes
- Validation gate: 5 minutes

**Phase 3: Implementation Plan** - 15-20 minutes
- implementation-planner: 15 minutes
- Final validation gate: 5 minutes

**Total Estimated Time:** 65-95 minutes for complete project

---

## Deliverables Location

All project deliverables will be created in:
```
/Users/odgrim/dev/home/agentics/abathur/design_docs/
```

**Phase 1 Deliverables:** `/Users/odgrim/dev/home/agentics/abathur/design_docs/phase1_design/`
- `README.md` - Phase 1 overview and navigation (1-2K tokens)
- `memory-architecture.md` - Complete memory system design (max 15K)
- `schema-tables.md` - All table definitions (max 20K)
- `schema-relationships.md` - ER diagrams and relationships (max 10K)
- `migration-strategy.md` - Migration approach and rollback (max 15K)
- `schema-indexes.md` - Indexing strategy (max 10K)

**Phase 2 Deliverables:** `/Users/odgrim/dev/home/agentics/abathur/design_docs/phase2_tech_specs/`
- `README.md` - Phase 2 overview and navigation (1-2K tokens)
- `ddl-core-tables.sql` - Core table DDL (max 15K)
- `ddl-memory-tables.sql` - Memory table DDL including document_index (max 15K)
- `ddl-indexes.sql` - All index definitions (max 10K)
- `query-patterns-read.md` - Optimized read queries (max 15K)
- `query-patterns-write.md` - Optimized write queries (max 15K)
- `api-specifications.md` - Python API definitions (max 20K)
- `test-scenarios.md` - Comprehensive test cases (max 15K)
- `implementation-guide.md` - Step-by-step instructions (max 15K)

**Phase 3 Deliverables:** `/Users/odgrim/dev/home/agentics/abathur/design_docs/phase3_implementation/`
- `README.md` - Phase 3 overview and navigation (1-2K tokens)
- `milestone-1-core-schema.md` - Core schema milestone (max 10K)
- `milestone-2-memory-system.md` - Memory system milestone (max 10K)
- `milestone-3-migration.md` - Migration milestone (max 10K)
- `testing-strategy.md` - Complete test strategy (max 15K)
- `migration-procedures.md` - Detailed migration scripts (max 15K)
- `rollback-procedures.md` - Rollback instructions (max 10K)
- `risk-assessment.md` - Risks and mitigation (max 10K)

**Final Deliverables:**
- `SCHEMA_REDESIGN_FINAL_REPORT.md` - Project summary (5-8K tokens)
- `SCHEMA_REDESIGN_INDEX.md` - Navigation index for all documents (3-5K tokens)

---

## Troubleshooting

**If an agent gets stuck:**
1. Check if decision points in SCHEMA_REDESIGN_DECISION_POINTS.md are resolved
2. Verify agent has access to required files (database.py, Chapter 8)
3. Review context passed from previous agent
4. Check for missing dependencies or unclear requirements

**If validation gate fails:**
1. Review orchestrator's validation feedback
2. Identify specific gaps or issues
3. Re-invoke affected agents with clarified requirements
4. Do NOT proceed to next phase until validation passes

**If project needs human input:**
1. Orchestrator will escalate with clear rationale
2. Human resolves issue or provides decision
3. Update SCHEMA_REDESIGN_DECISION_POINTS.md with decision
4. Resume project with updated context

---

## Post-Project Actions

After successful completion:
1. Review all deliverables for completeness
2. Validate against original 10 core requirements
3. Schedule migration testing on non-production database
4. Plan deployment timeline and communication
5. Archive project documentation for future reference

---

**Version:** 1.1
**Created:** 2025-10-10
**Updated:** 2025-10-10 (Added document structure requirements, phased embedding approach)
**Status:** Ready for Execution (after 17 decision points resolved)
**Estimated Duration:** 65-95 minutes
