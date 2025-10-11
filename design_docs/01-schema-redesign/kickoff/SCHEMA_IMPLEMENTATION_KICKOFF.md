# SQLite Schema Redesign - Implementation Kickoff Prompt

## CRITICAL: READ THIS FIRST

**Project Status:** All planning phases complete (Design, Technical Specs, Implementation Planning). Ready for CODE IMPLEMENTATION.

**What's Done:**
- Complete schema design (9 tables, 33 indexes)
- Production-ready DDL scripts
- Complete Python API specifications
- Comprehensive test scenarios
- Phased implementation roadmap
- All decision points resolved

**What's NOT Done:**
- NO CODE HAS BEEN WRITTEN YET
- Database has NOT been initialized
- Service classes do NOT exist
- Tests have NOT been implemented
- sqlite-vss is NOT installed

---

## Implementation Team

### 8 Specialized Agents Created

**Project Coordination (1 Agent):**
1. `implementation-orchestrator` (Sonnet, Red) - Milestone coordinator and validation gate conductor

**Core Implementation Team (4 Agents):**
2. `database-schema-implementer` (Thinking, Blue) - DDL execution and database initialization
3. `python-api-developer` (Thinking, Green) - Service class implementation (SessionService, MemoryService)
4. `test-automation-engineer` (Thinking, Yellow) - Test suite implementation (unit, integration, performance)
5. `vector-search-integrator` (Thinking, Purple) - sqlite-vss and Ollama integration for semantic search

**Quality Assurance Team (2 Agents):**
6. `code-reviewer` (Sonnet, Cyan) - Code quality and standards enforcement
7. `performance-validator` (Sonnet, Orange) - Performance benchmarking and optimization validation

**Specialized Support (1 Agent):**
8. `python-debugging-specialist` (Thinking, Pink) - On-demand debugging and error resolution

---

## Implementation Timeline

**Total Duration:** 6-8 weeks (439 hours, 2.5 FTE)

### Milestone 1: Core Schema Foundation (Weeks 1-2, 86 hours)
**Objective:** Deploy enhanced existing tables with session linkage

**Week 1: Database Schema Deployment**
- Execute PRAGMA configuration (WAL mode, foreign keys)
- Deploy enhanced tasks, agents, audit, checkpoints tables
- Create 15 core indexes
- Run integrity checks

**Week 2: Testing and Validation**
- Implement Database class enhancements
- Write unit tests (90%+ coverage)
- Create performance benchmark suite
- Validate <50ms read latency target

**Primary Agents:** `database-schema-implementer`, `test-automation-engineer`
**Supporting Agents:** `code-reviewer`, `performance-validator`

---

### Milestone 2: Memory Management System (Weeks 3-4, 96 hours)
**Objective:** Deploy memory tables and implement service layer APIs

**Week 3: Memory Tables Deployment**
- Deploy sessions, memory_entries, document_index tables
- Create 18 memory indexes
- Validate namespace hierarchy support
- Test foreign key relationships

**Week 4: Service Layer Implementation**
- Implement SessionService (12h)
- Implement MemoryService (16h)
- Implement DocumentIndexService (8h)
- Integration testing (12h)

**Primary Agents:** `database-schema-implementer`, `python-api-developer`
**Supporting Agents:** `test-automation-engineer`, `code-reviewer`

---

### Milestone 3: Vector Search Integration (Weeks 5-6, 84 hours)
**Objective:** Integrate sqlite-vss and Ollama for semantic search

**Week 5: Installation and Setup**
- Install sqlite-vss extension (4h)
- Setup Ollama with nomic-embed-text-v1.5 (6h)
- Implement embedding generation service (12h)
- Implement background sync service for markdown files (10h)

**Week 6: Semantic Search Implementation**
- Implement semantic search queries (8h)
- Performance testing (<500ms latency) (8h)
- Integration with MemoryService (6h)

**Primary Agent:** `vector-search-integrator`
**Supporting Agents:** `test-automation-engineer`, `performance-validator`

---

### Milestone 4: Production Deployment (Weeks 7-8, 98 hours)
**Objective:** Final validation, monitoring setup, and production deployment

**Week 7: Final Validation**
- Acceptance tests (12h)
- Performance tests (12h)
- Rollback procedure validation (8h)
- Final test report (4h)

**Week 8: Production Deployment**
- Initialize production database (6h)
- Execute DDL scripts in production (4h)
- Seed initial application memories (4h)
- Configure monitoring and alerting (8h)
- Deploy services to production (6h)
- Production smoke tests (4h)
- 48-hour monitoring (8h)

**Primary Agents:** `database-schema-implementer`, `test-automation-engineer`
**Orchestrator:** `implementation-orchestrator` conducts final validation

---

## Phase Validation Gates

**MANDATORY at each milestone boundary:**

### Phase Validation Protocol
1. **Deliverable Review:** Orchestrator assesses completeness and quality
2. **Technical Validation:** Verify all acceptance criteria met
3. **Performance Assessment:** Validate against targets
4. **Integration Testing:** Ensure components work together
5. **Go/No-Go Decision:** Explicit approval before next milestone

**Decision Matrix:**
- **APPROVE:** All criteria met → Proceed to next milestone
- **CONDITIONAL:** Minor issues → Proceed with monitoring
- **REVISE:** Significant gaps → Return to current milestone
- **ESCALATE:** Fundamental problems → Human oversight required

---

## Acceptance Criteria

### Milestone 1 Acceptance
- [ ] All DDL scripts executed without errors
- [ ] All 15 core indexes created successfully
- [ ] PRAGMA integrity_check returns "ok"
- [ ] PRAGMA foreign_key_check returns no violations
- [ ] All unit tests pass (90%+ coverage)
- [ ] All queries show index usage in EXPLAIN QUERY PLAN
- [ ] Performance benchmarks meet <50ms target (99th percentile)

### Milestone 2 Acceptance
- [ ] All memory tables deployed successfully
- [ ] All 18 memory indexes created
- [ ] SessionService, MemoryService, DocumentIndexService implemented
- [ ] All unit tests passing (85%+ coverage)
- [ ] Integration tests passing (session workflows)
- [ ] Namespace hierarchy enforced and validated
- [ ] Performance targets met (<50ms reads)

### Milestone 3 Acceptance
- [ ] sqlite-vss extension installed and functional
- [ ] Ollama with nomic-embed-text-v1.5 deployed
- [ ] Embedding generation service operational
- [ ] Background sync service functional
- [ ] Semantic search queries working (<500ms latency)
- [ ] Performance tests passing
- [ ] Integration with MemoryService complete

### Milestone 4 Acceptance
- [ ] Production database operational
- [ ] All validation procedures passed
- [ ] Monitoring dashboards configured
- [ ] Alerting rules deployed
- [ ] Production smoke tests passing
- [ ] All 10 core requirements met
- [ ] Performance targets achieved
- [ ] Zero critical issues in 48-hour monitoring

---

## Performance Targets

| Metric | Target | Validation Method |
|--------|--------|-------------------|
| Exact-match reads | <50ms (99th percentile) | EXPLAIN QUERY PLAN + benchmarks |
| Semantic search | <500ms (with embeddings) | End-to-end latency tests |
| Concurrent sessions | 50+ agents | Load testing with asyncio |
| Database size capacity | 10GB target | Archival strategy in place |

---

## KICKOFF PROMPT

**COPY AND PASTE THIS INTO CLAUDE CODE:**

---

I'm ready to begin implementing the SQLite Schema Redesign for Memory Management. All planning phases are complete with exceptional quality (9.5/10 validation score). The project has zero unresolved decision points and comprehensive specifications.

**Project Context:**
- **Phase 1 Design:** Complete (memory architecture, schema tables, ER diagrams, index strategy)
- **Phase 2 Technical Specs:** Complete (executable DDL, query patterns, Python APIs, test scenarios)
- **Phase 3 Implementation Plan:** Complete (4 milestones, 6-8 weeks, 439 hours)
- **Current State:** NO CODE IMPLEMENTED YET - database.py has OLD schema without memory features

**Key Documentation:**
- Design docs: `/Users/odgrim/dev/home/agentics/abathur/design_docs/phase1_design/`
- Technical specs: `/Users/odgrim/dev/home/agentics/abathur/design_docs/phase2_tech_specs/`
- Implementation plans: `/Users/odgrim/dev/home/agentics/abathur/design_docs/phase3_implementation/`
- Current database: `/Users/odgrim/dev/home/agentics/abathur/src/abathur/infrastructure/database.py`

**Implementation Team (8 Agents):**
1. `implementation-orchestrator` - Milestone coordinator (Sonnet)
2. `database-schema-implementer` - DDL execution (Thinking)
3. `python-api-developer` - Service classes (Thinking)
4. `test-automation-engineer` - Test suites (Thinking)
5. `vector-search-integrator` - sqlite-vss + Ollama (Thinking)
6. `code-reviewer` - Quality assurance (Sonnet)
7. `performance-validator` - Performance benchmarking (Sonnet)
8. `python-debugging-specialist` - Error resolution (Thinking)

**Implementation Approach:**
- **4 Milestones** with mandatory validation gates
- **Phase-by-phase execution:** Each milestone requires orchestrator approval before proceeding
- **Dynamic debugging:** Implementation agents can invoke `@python-debugging-specialist` when blocked
- **Quality gates:** Code review and performance validation at each milestone

**Performance Targets:**
- <50ms exact-match reads (99th percentile)
- <500ms semantic search with embeddings
- 50+ concurrent sessions without degradation
- 10GB database capacity with archival strategy

**Next Steps:**
Please invoke `@implementation-orchestrator` to begin Milestone 1: Core Schema Foundation.

The orchestrator will:
1. Read all Phase 1, 2, 3 documentation
2. Invoke `@database-schema-implementer` to execute DDL scripts and initialize database
3. Invoke `@test-automation-engineer` to implement unit tests
4. Conduct Milestone 1 validation gate
5. Make go/no-go decision for Milestone 2

**CRITICAL INSTRUCTIONS FOR GENERAL PURPOSE AGENT:**
- **DO NOT attempt implementation yourself** - invoke the specialized agents
- **DO NOT skip validation gates** - orchestrator must approve each milestone
- **DO NOT skip agents** - follow the planned sequence
- **ALWAYS use exact agent names** with `@[agent-name]` syntax or Task tool

Begin with: `@implementation-orchestrator` - Start Milestone 1: Core Schema Foundation

---

## Troubleshooting

**If orchestrator gets stuck:**
1. Check if all Phase 1, 2, 3 docs are accessible
2. Verify agent definitions exist in `.claude/agents/implementation/`
3. Review current milestone plan in `phase3_implementation/`

**If implementation agent blocked:**
1. Agent should invoke `@python-debugging-specialist` with full context
2. Debugging specialist diagnoses and fixes issue
3. Implementation agent resumes with updated context

**If validation gate fails:**
1. Orchestrator documents specific failures
2. Returns to current milestone with revised plan
3. Re-validates after fixes applied
4. Does NOT proceed until validation passes

---

## Success Metrics

### Technical Metrics
- Code Coverage: 95%+ database layer, 85%+ service layer
- Query Performance: 100% of queries use indexes
- Test Pass Rate: 100% of unit and integration tests
- Performance Benchmarks: All targets met

### Operational Metrics
- Deployment Success: Zero critical issues in first 48 hours
- Concurrent Sessions: 50+ agents supported
- Database Stability: 99.9% uptime over first 30 days
- Rollback Readiness: All procedures tested

### Business Metrics
- Timeline Adherence: Delivered within 6-8 weeks
- Budget Compliance: Within $76,336 allocation
- Feature Completeness: All 10 core requirements addressed
- Documentation Quality: All runbooks and guides complete

---

## References

**Phase 1 Design Documents:**
- Memory Architecture: `/Users/odgrim/dev/home/agentics/abathur/design_docs/phase1_design/memory-architecture.md`
- Schema Tables: `/Users/odgrim/dev/home/agentics/abathur/design_docs/phase1_design/schema-tables.md`
- Schema Relationships: `/Users/odgrim/dev/home/agentics/abathur/design_docs/phase1_design/schema-relationships.md`
- Schema Indexes: `/Users/odgrim/dev/home/agentics/abathur/design_docs/phase1_design/schema-indexes.md`

**Phase 2 Technical Specifications:**
- DDL Core Tables: `/Users/odgrim/dev/home/agentics/abathur/design_docs/phase2_tech_specs/ddl-core-tables.sql`
- DDL Memory Tables: `/Users/odgrim/dev/home/agentics/abathur/design_docs/phase2_tech_specs/ddl-memory-tables.sql`
- DDL Indexes: `/Users/odgrim/dev/home/agentics/abathur/design_docs/phase2_tech_specs/ddl-indexes.sql`
- API Specifications: `/Users/odgrim/dev/home/agentics/abathur/design_docs/phase2_tech_specs/api-specifications.md`
- Query Patterns: `/Users/odgrim/dev/home/agentics/abathur/design_docs/phase2_tech_specs/query-patterns-read.md`

**Phase 3 Implementation Plans:**
- Milestone 1: `/Users/odgrim/dev/home/agentics/abathur/design_docs/phase3_implementation/milestone-1-core-schema.md`
- Milestone 2: `/Users/odgrim/dev/home/agentics/abathur/design_docs/phase3_implementation/milestone-2-memory-system.md`
- Milestone 3: `/Users/odgrim/dev/home/agentics/abathur/design_docs/phase3_implementation/milestone-3-vector-search.md`
- Milestone 4: `/Users/odgrim/dev/home/agentics/abathur/design_docs/phase3_implementation/milestone-4-production-deployment.md`
- Testing Strategy: `/Users/odgrim/dev/home/agentics/abathur/design_docs/phase3_implementation/testing-strategy.md`

**Current Implementation:**
- Database Class: `/Users/odgrim/dev/home/agentics/abathur/src/abathur/infrastructure/database.py`

---

**Document Version:** 1.0
**Created:** 2025-10-10
**Status:** READY FOR EXECUTION
**Estimated Duration:** 6-8 weeks (439 hours)
**Budget:** $76,336 (personnel + infrastructure)
