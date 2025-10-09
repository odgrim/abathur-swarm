# Phase 2 Context Summary

**Date:** 2025-10-09
**Phase:** Phase 2 - Technical Architecture & Design
**Phase 1 Status:** APPROVED (CONDITIONAL)
**Source:** Condensed from PHASE_1_VALIDATION.md

---

## Executive Summary

Abathur is a CLI-first orchestration system for managing swarms of specialized Claude agents that collaborate on complex, multi-step development tasks. Phase 1 established a comprehensive vision and detailed requirements (58 FRs + 30 NFRs) with complete traceability.

**Phase 2 Mission:** Design the technical architecture, component specifications, and API contracts that will enable implementation of the vision while meeting aggressive performance targets (<100ms queue ops, <5s agent spawn, >99.9% persistence reliability).

---

## Vision in Brief

### Product Vision

Abathur transforms how developers leverage AI by orchestrating swarms of specialized Claude agents that work collaboratively on complex, multi-step tasks. It is the command center for AI-driven development—where developer intent becomes coordinated agent action, and complex problems are decomposed into specialized, parallelizable workstreams that converge into validated solutions.

### Core Value Proposition

**Three Pillars:**
1. **Systematic Specialization at Scale:** Claude-native design enabling fine-grained agent specialization with coordinated swarms (10+ concurrent agents)
2. **Developer-First Orchestration:** CLI-first, git-native, template-driven fitting naturally into existing developer workflows
3. **Production-Ready from Day One:** Persistence, failure recovery, resource management, and observability as core features (not afterthoughts)

**Key Differentiators:**
- Claude-native (not generic LLM framework like LangChain/CrewAI)
- CLI-first (not library-first)
- Persistent SQLite queue (not in-memory or external infrastructure)
- Built-in resource management and cost control
- Git-based template system with versioning

### Target Users

1. **Alex - AI-Forward Full-Stack Developer**
   - Needs: Parallel execution, automated testing, reduced context switching
   - Goal: Complete features 5-10x faster through parallel agent work
   - Success: Ship features in days that would take weeks manually

2. **Morgan - Platform Engineering Lead**
   - Needs: Standardized workflows, team-wide templates, measurable productivity
   - Goal: Increase team velocity 30-50% with consistent code quality
   - Success: Positive ROI within 3 months

3. **Jordan - Automation Specialist / DevOps Engineer**
   - Needs: Production-grade reliability, comprehensive observability, reusable automation
   - Goal: >99% reliability for AI-powered automation workflows
   - Success: Measurable reduction in operational toil

---

## Strategic Goals & Success Metrics

### Goal 1: Enable Scalable Multi-Agent Coordination
**Target:** 10+ concurrent agents (configurable to 50+), <5s spawn time, >80% resource utilization
**Requirements:** FR-SWARM-001-008, NFR-PERF-004, NFR-SCALE-001

### Goal 2: Provide Production-Grade Task Management
**Target:** <100ms queue ops, 1,000+ queued tasks, >99.9% persistence reliability
**Requirements:** FR-QUEUE-001-010, NFR-PERF-001, NFR-REL-001

### Goal 3: Support Iterative Solution Refinement
**Target:** Configurable convergence, checkpoint/resume, >95% convergence success
**Requirements:** FR-LOOP-001-007, NFR-PERF-002

### Goal 4: Accelerate Developer Productivity
**Target:** <5 min to first task, 5-10x time reduction, >70 NPS
**Requirements:** FR-CLI-001-009, FR-TMPL-001-003, NFR-USE-001-002

### Goal 5: Maintain Developer Control & Transparency
**Target:** <50ms status queries, complete audit trail, >4.5/5 satisfaction for control
**Requirements:** FR-MONITOR-001-005, NFR-PERF-003, NFR-USE-003

---

## Critical Requirements Summary

### Must-Have Functional Requirements (High Priority)

**Template Management (6 FRs total, 4 High priority):**
- **FR-TMPL-001:** Clone `abathur-claude-template` from GitHub
  - Installs agents to `.claude/agents/` (shared with Claude Code)
  - Installs MCP config to `.claude/mcp.json` (compatible with Claude Code)
  - Installs Abathur orchestration config to `.abathur/config.yaml`
  - Initializes SQLite database at `.abathur/abathur.db`
  - Validates structure and retries on network failures

- **FR-TMPL-002:** Version-specific template fetching (tags, releases, commits, "latest")
- **FR-TMPL-004:** Validate template structure (`.abathur/config.yaml`, `.claude/agents/`, `.claude/mcp.json` optional)

**Task Queue Management (10 FRs total, 7 High priority):**
- **FR-QUEUE-001:** Submit task to persistent SQLite queue
  - Acceptance: Submit with metadata, persist immediately, return UUID, <100ms p95

- **FR-QUEUE-002:** List queued tasks with filtering
  - Acceptance: Display tasks, filter by status/priority, <50ms for 1000 tasks

- **FR-QUEUE-003:** Cancel pending/running tasks (graceful shutdown within 5s)
- **FR-QUEUE-004:** View task details and history (<100ms retrieval)
- **FR-QUEUE-005:** Persist state across crashes/restarts
  - Acceptance: >99.9% reliability, ACID transactions, recover queue on restart

- **FR-QUEUE-009:** Automatic retry with exponential backoff (3 retries, 10s→5min)

**Swarm Coordination (8 FRs total, 4 High priority):**
- **FR-SWARM-001:** Spawn multiple concurrent agents
  - Acceptance: 10+ agents (configurable), isolated async context, <5s spawn p95

- **FR-SWARM-002:** Distribute tasks across agent pool
  - Acceptance: Load balancing, specialization matching, <100ms distribution

- **FR-SWARM-003:** Collect and aggregate results from multiple agents
- **FR-SWARM-004:** Handle agent failures and recovery
  - Acceptance: Detect failures, reassign work, trigger retry logic, release resources

**Loop Execution (7 FRs total, 3 High priority):**
- **FR-LOOP-001:** Execute tasks iteratively with feedback
- **FR-LOOP-002:** Evaluate convergence criteria
  - Acceptance: Test pass, quality threshold, custom function, LLM-based evaluation

- **FR-LOOP-003:** Enforce max iteration limits (default 10, configurable)

**CLI Operations (9 FRs total, 3 High priority):**
- **FR-CLI-001:** Initialize project with `abathur init` (<30s)
- **FR-CLI-002:** Context-aware help for all commands (<100ms)
- **FR-CLI-006:** Actionable error messages
  - Acceptance: Error code, description, cause, resolution steps, docs link

**Configuration Management (6 FRs total, 4 High priority):**
- **FR-CONFIG-001:** Load YAML config hierarchy
  - System defaults → Template (`.abathur/config.yaml`) → User (`~/.abathur/config.yaml`) → Project (`.abathur/local.yaml`)
  - Load agent definitions from `.claude/agents/` (shared with Claude Code)
  - Load MCP config from `.claude/mcp.json` (if present)

- **FR-CONFIG-002:** Environment variable overrides (`ABATHUR_*` prefix)
- **FR-CONFIG-003:** Validate configuration schema (all errors reported, not just first)
- **FR-CONFIG-004:** Manage API keys securely
  - Acceptance: Check `ANTHROPIC_API_KEY` env var → keychain → `.env` file, never log keys

**Monitoring & Observability (5 FRs total, 1 High priority):**
- **FR-MONITOR-001:** Structured JSON logging
  - Acceptance: Logs to `.abathur/logs/abathur.log`, 30-day retention, never logs secrets

---

### Critical Non-Functional Requirements

**Performance (7 NFRs):**
- **NFR-PERF-001:** Queue operations <100ms at p95 (submit, list, cancel)
- **NFR-PERF-002:** Agent spawn <5s at p95 (from request to first action)
- **NFR-PERF-003:** Status queries <50ms at p95
- **NFR-PERF-004:** 10 concurrent agents with <10% performance degradation
- **NFR-PERF-005:** Queue scalability to 10,000 tasks with <100ms latency
- **NFR-PERF-006:** System overhead <200MB memory (excluding agents)
- **NFR-PERF-007:** CLI startup and help display <500ms

**Reliability & Availability (5 NFRs):**
- **NFR-REL-001:** >99.9% task persistence through crashes/restarts
- **NFR-REL-002:** Graceful degradation (non-critical failures don't stop execution)
- **NFR-REL-003:** 95% eventual success rate for transient API errors (via retry)
- **NFR-REL-004:** ACID guarantees for all state transitions
- **NFR-REL-005:** Recovery from component failures within 30s

**Security (5 NFRs):**
- **NFR-SEC-001:** API keys encrypted at rest (keychain or AES-256)
- **NFR-SEC-002:** Never log API keys, tokens, or sensitive data
- **NFR-SEC-003:** Validate and sanitize all user inputs (prevent injection attacks)
- **NFR-SEC-004:** Template integrity validation (checksum/signature)
- **NFR-SEC-005:** Zero critical/high vulnerabilities in dependencies

**Usability (5 NFRs):**
- **NFR-USE-001:** Complete first task within 5 minutes of installation
- **NFR-USE-002:** 80% of users complete tasks without consulting documentation (intuitive CLI)
- **NFR-USE-003:** 90% of errors include actionable resolution suggestions

**Maintainability (5 NFRs):**
- **NFR-MAINT-001:** >80% line coverage, >90% critical path coverage
- **NFR-MAINT-002:** Pass linting (ruff), type checking (mypy), formatting (black)
- **NFR-MAINT-003:** Loosely coupled modular architecture with clear interfaces
- **NFR-MAINT-004:** All modules/classes/public functions have Google-style docstrings
- **NFR-MAINT-005:** Backward compatibility within major versions

**Portability (5 NFRs):**
- **NFR-PORT-001:** macOS, Linux (Ubuntu 20.04+), Windows 10+ with feature parity
- **NFR-PORT-002:** Python 3.10, 3.11, 3.12+ support
- **NFR-PORT-003:** Only Python + SQLite dependencies (no external databases/queues)

---

## Technical Constraints & Decisions

### Technology Stack (from DECISION_POINTS.md)

**Language & Runtime:**
- Python 3.10+ (modern type hints, pattern matching)
- Async/await coroutines for agent spawning (#6)
- asyncio for concurrency (native Python)

**CLI & Configuration:**
- Typer framework (type-safe, modern, excellent DX) (#4)
- YAML configuration files + .env for secrets (#5)
- Environment variable overrides (`ABATHUR_*` prefix) (#5)
- Poetry for dependency management (#10)

**Persistence & State:**
- SQLite for task queue and state (#1)
- Centralized state store with event log (#3)
- ACID transactions for reliability
- Message queue + shared state database for agent communication (#2)

**Coordination & Patterns:**
- Leader-follower swarm coordination (#11)
- Hierarchical orchestration (nested agents) (#11)
- Numeric 0-10 priority scale with FIFO tiebreaker (#12)
- Retry with exponential backoff + DLQ + checkpoint (#13)
- Loop termination: max iterations + success criteria + timeout (#14)

### Architectural Constraints

**Deployment Model:**
- Single-node deployment (v1 scope) (TC-004)
- Local-first architecture (all processing local except Claude API) (OC-001)
- Zero external infrastructure (no Redis, PostgreSQL, cloud queues) (BC-002)

**Directory Structure (Critical):**
- **`.claude/` directory** (shared with Claude Code):
  - `.claude/agents/` - Agent definitions
  - `.claude/mcp.json` - MCP server configurations

- **`.abathur/` directory** (Abathur-specific):
  - `.abathur/config.yaml` - Orchestration configuration
  - `.abathur/abathur.db` - SQLite task queue and state database
  - `.abathur/logs/` - Abathur-specific logs (separated from Claude Code)
  - `.abathur/metadata.json` - Template version metadata

**Integration Requirements:**
- Must coexist seamlessly with Claude Code
- Must not conflict with Claude Code file watchers
- MCP config `.claude/mcp.json` shared but loading sequence needs design
- Agent definitions in `.claude/agents/` shared but Abathur loads for orchestration

**Resource Constraints:**
- Default: 10 concurrent agents (configurable to 50+) (DP#15)
- Default: 1,000 task queue capacity (configurable to 10,000) (DP#15)
- Default: 512MB per agent, 4GB total memory (configurable) (DP#17)
- System overhead target: <200MB (NFR-PERF-006)

---

## Performance Targets (for Architecture Validation)

All Phase 2 architectures must support:

| Metric | Target | Requirement | Validation Method |
|--------|--------|-------------|-------------------|
| Queue submit/list/cancel | <100ms p95 | NFR-PERF-001 | Instrumented timing of DB ops |
| Agent spawn time | <5s p95 | NFR-PERF-002 | Time from spawn call to first action |
| Status query latency | <50ms p95 | NFR-PERF-003 | End-to-end status command latency |
| Concurrent agents | 10+ with <10% degradation | NFR-PERF-004 | Throughput test: 1 vs 10 agents |
| Queue scalability | <100ms at 10,000 tasks | NFR-PERF-005 | Performance test with increasing queue size |
| Task persistence reliability | >99.9% | NFR-REL-001 | Fault injection: kill -9 during operations |
| API retry success | 95% eventual success | NFR-REL-003 | Simulated API failures, measure retry success |
| CLI startup time | <500ms | NFR-PERF-007 | Time from command invocation to output |
| Memory efficiency | <200MB system overhead | NFR-PERF-006 | Memory profiling excluding agents |

**Note:** These are aggressive but realistic targets for modern Python async architectures. Phase 2 must validate feasibility or propose adjusted targets with rationale (Conditional Item #1).

---

## Conditional Validation Items (from Phase 1 Review)

Phase 1 was approved with three conditional items requiring validation during Phase 2:

### Conditional Item #1: Performance Target Feasibility

**Issue:** NFR performance targets are aggressive (<100ms queue ops, <5s agent spawn, <50ms status)

**Validation Required:**
- Validate targets against Python asyncio performance characteristics
- Validate SQLite transaction latency under concurrency
- Assess Claude API spawn time variability
- Evaluate system resource monitoring overhead

**Success Criteria:** Confirm targets are achievable OR propose adjusted targets with technical rationale

**Priority:** High (early validation)

### Conditional Item #2: Directory Structure Implementation

**Issue:** `.claude/` and `.abathur/` separation requires technical validation

**Design Required:**
- File system layout for both directories
- File watcher exclusions to prevent Claude Code conflicts
- MCP config file loading sequence (who loads `.claude/mcp.json` first?)
- SQLite database file locations and initialization
- Agent definition loading from `.claude/agents/`

**Success Criteria:** Directory structure that coexists seamlessly with Claude Code without conflicts

**Priority:** High (architectural foundation)

### Conditional Item #3: MCP Integration Patterns

**Issue:** MCP server auto-discovery and dynamic loading (DP#21) needs architectural design

**Design Required:**
- MCP server discovery and registration mechanism
- Dynamic MCP server loading for task-specific requirements
- Agent-to-MCP-server binding patterns
- MCP lifecycle management (start, stop, health check)

**Success Criteria:** Clear MCP integration architecture supporting auto-discovery from template with user overrides

**Priority:** Medium (needed for complete system design)

---

## Use Cases Overview (for Context)

**UC1: Full-Stack Feature Development**
- Parallel execution across frontend, backend, database, testing, documentation agents
- Expected: 2-4 hours vs 2-3 days (5-10x improvement)
- Requirements: Template management, swarm coordination, result aggregation

**UC2: Automated Code Review**
- Multi-perspective analysis (security, performance, testing, documentation, architecture agents)
- Expected: 15-30 min vs 2 hours (4x improvement)
- Requirements: Swarm coordination, template system, result aggregation

**UC3: Iterative Solution Refinement**
- Test-driven convergence (measure → optimize → verify → repeat)
- Expected: 30-45 min to meet performance target
- Requirements: Loop execution, convergence criteria, checkpointing

**UC4: Batch Processing Across Repositories**
- Process 20 repos concurrently with failure handling
- Expected: 1-2 hours vs 2-3 days (10-20x improvement)
- Requirements: Batch submission, priority scheduling, DLQ, concurrency

**UC5: Specification-Driven Development**
- Spec → Tests → Implementation workflow with dependencies
- Expected: High-reliability payment module in 3-4 hours
- Requirements: Task dependencies, loop execution, template system

**UC6: Long-Running Research and Analysis**
- Parallel research across multiple dimensions
- Expected: 30-45 min vs 4-6 hours (75-80% reduction)
- Requirements: Persistent queue, swarm coordination, checkpointing

**UC7: Self-Improving Agent Evolution (Meta-Abathur)**
- Meta-agent improves other agents based on feedback
- Expected: Agent improvement in 15-30 min vs hours of manual prompt engineering
- Requirements: Meta-agent framework, template versioning, validation

---

## Phase 2 Priorities & Guidance

### Priority 1: Validate Conditional Items

**Actions:**
1. Performance target feasibility analysis (Python asyncio + SQLite benchmarking)
2. Directory structure coexistence design (file watchers, loading sequences)
3. MCP integration architecture (discovery, binding, lifecycle)

**Deliverable:** Validation report with confirmed or adjusted targets + directory structure specification + MCP integration design

### Priority 2: Core Component Architecture

**Actions:**
1. Design modular architecture for 8 functional areas (templates, queue, swarm, loops, CLI, config, monitoring, meta)
2. Define component interfaces and dependencies
3. Create component diagrams with data flow
4. Address NFR-MAINT-003 (loosely coupled modules)

**Deliverable:** System architecture document with component specifications

### Priority 3: Database & State Management Design

**Actions:**
1. Design SQLite schema for task queue, shared state, audit trail
2. Specify ACID transaction boundaries
3. Plan database migration strategy
4. Consider WAL mode for concurrent access
5. Design checkpoint/resume mechanism

**Deliverable:** Database schema specification with migration strategy

### Priority 4: Async & Concurrency Architecture

**Actions:**
1. Design async/await patterns for concurrent agent management
2. Specify agent lifecycle (spawn, execute, terminate)
3. Plan resource limit enforcement (10+ agents, 4GB memory)
4. Design failure detection and recovery flows

**Deliverable:** Concurrency architecture with agent lifecycle specification

### Priority 5: API & CLI Design

**Actions:**
1. Define complete CLI command hierarchy and options
2. Specify Python API surface for programmatic usage
3. Design configuration schema (YAML + env vars)
4. Plan error handling and messaging

**Deliverable:** API/CLI specification with command reference

### Priority 6: Security Architecture

**Actions:**
1. Design API key encryption (keychain integration + encrypted fallback)
2. Specify input validation and sanitization patterns
3. Plan template integrity validation (checksum/signature)
4. Design audit trail data model

**Deliverable:** Security architecture addressing NFR-SEC-001-005

---

## Design Principles for Phase 2

**1. Developer Experience First**
- Intuitive CLI (NFR-USE-002: 80% tasks without docs)
- Fast operations (queue <100ms, status <50ms)
- Clear, actionable errors (NFR-USE-003: 90% include suggestions)
- <5 min to first task (NFR-USE-001)

**2. Production-Ready from Day One**
- Persistence and ACID transactions (NFR-REL-001: >99.9% reliability)
- Graceful failure recovery (NFR-REL-002, NFR-REL-003)
- Comprehensive observability (structured logging, audit trail)
- Resource limits and cost controls

**3. Seamless Claude Code Integration**
- Shared `.claude/` directory (agents, MCP config)
- No conflicts with Claude Code file watchers
- MCP compatibility (shared `.claude/mcp.json`)
- Clear separation of concerns (`.abathur/` for orchestration)

**4. Resource Efficiency**
- Configurable limits (agents, memory, queue capacity)
- Adaptive resource usage based on availability
- Cost visibility (token tracking, metrics)
- <200MB system overhead (NFR-PERF-006)

**5. Systematic Quality**
- Template-driven workflows (reproducible, shareable)
- Test-validated (>80% coverage, NFR-MAINT-001)
- Audit-trailed (complete agent action history)
- Type-safe (mypy type checking, NFR-MAINT-002)

**6. Extensibility & Evolution**
- Modular architecture (loosely coupled, NFR-MAINT-003)
- Plugin architecture for future extensions
- Backward compatibility (NFR-MAINT-005)
- Meta-agent capability for self-improvement (FR-META-003)

---

## Architectural Patterns to Consider

**From DECISION_POINTS.md and Requirements:**

1. **Leader-Follower Pattern** (DP#11, FR-SWARM-006)
   - Orchestrator agent coordinates worker agents
   - Hierarchical nesting for complex workflows
   - Limit nesting depth (configurable, suggest 3)

2. **Event Sourcing** (for audit trail, FR-MONITOR-004)
   - Complete history of agent actions
   - Enables replay and debugging
   - 90-day retention (configurable)

3. **Checkpoint Pattern** (FR-LOOP-006)
   - Checkpoint state at iteration boundaries
   - Enable crash recovery and resume
   - Store in SQLite for persistence

4. **Dead Letter Queue** (FR-QUEUE-010)
   - Failed tasks after max retries
   - Manual inspection and retry
   - Prevents blocking main queue

5. **Circuit Breaker** (for API failures, NFR-REL-003)
   - Detect repeated failures
   - Implement exponential backoff
   - 95% eventual success target

6. **Repository Pattern** (for database abstraction)
   - Abstract SQLite access
   - Enable future migration to Redis
   - Support testing with mock implementations

---

## Risks & Mitigation Strategies

**Risk 1: Performance Targets May Be Challenging**
- **Impact:** <100ms queue ops, <5s agent spawn may be difficult
- **Mitigation:** Early validation, performance budgets, optimization from start
- **Phase 2 Action:** Benchmark Python asyncio + SQLite, confirm feasibility or adjust targets

**Risk 2: SQLite Concurrency Under High Load**
- **Impact:** SQLite has limited concurrent write capability
- **Mitigation:** WAL mode, connection pooling, batch operations, async queue
- **Phase 2 Action:** Design abstraction layer for future Redis migration if needed

**Risk 3: Directory Structure Conflicts with Claude Code**
- **Impact:** File watchers or MCP loading may conflict
- **Mitigation:** Clear file watcher exclusions, defined loading sequences
- **Phase 2 Action:** Validate coexistence, document loading order, test with Claude Code

**Risk 4: MCP Integration Complexity**
- **Impact:** Auto-discovery and dynamic loading adds complexity
- **Mitigation:** Start simple (static config), add dynamic loading in phases
- **Phase 2 Action:** Design clear lifecycle, test with example MCP servers

**Risk 5: Swarm Coordination Complexity**
- **Impact:** Hierarchical spawning, shared state, failure recovery are complex
- **Mitigation:** Clear component boundaries, state machines, comprehensive testing
- **Phase 2 Action:** Start with simple leader-follower, add hierarchy later

---

## Success Criteria for Phase 2

Phase 2 will be considered successful if it delivers:

### Required Deliverables

1. **System Architecture Document**
   - Component diagrams (8 functional areas)
   - Data flow diagrams
   - Deployment architecture
   - Technology stack justification

2. **Component Specifications**
   - Detailed design for each functional area
   - Interface definitions (APIs, contracts)
   - State machines for task lifecycle
   - Error handling strategies

3. **Database Schema Specification**
   - SQLite table definitions (task queue, shared state, audit trail)
   - Indexes for performance
   - Migration strategy
   - ACID transaction boundaries

4. **API/CLI Specification**
   - Complete CLI command reference
   - Python API surface (classes, methods)
   - Configuration schema (YAML structure)
   - Error codes and messages

5. **Security Architecture**
   - API key management (keychain + fallback)
   - Input validation patterns
   - Template integrity validation
   - Audit trail design

6. **Concurrency & Performance Architecture**
   - Async/await patterns
   - Agent lifecycle management
   - Resource limit enforcement
   - Performance optimization strategy

7. **Implementation Roadmap**
   - Phased development plan
   - Dependencies and sequencing
   - Risk mitigation strategies
   - Testing approach

### Quality Gates

**Traceability:**
- All components trace to specific requirements (FR-XXX-NNN, NFR-XXX-NNN)
- All design decisions reference constraints (TC-XXX, DP#XX)

**Performance:**
- Architecture supports performance targets (or proposes adjusted targets)
- Performance budgets allocated to components
- Optimization strategy defined

**Security:**
- All NFR-SEC requirements addressed
- Threat model documented
- Security review completed

**Maintainability:**
- Modular architecture with clear interfaces (NFR-MAINT-003)
- Type-safe design (supports mypy, NFR-MAINT-002)
- Testing strategy enables >80% coverage (NFR-MAINT-001)

**Portability:**
- Cross-platform design (macOS, Linux, Windows)
- Python 3.10+ compatibility
- No external dependencies beyond Python + SQLite

---

## Next Steps

**Immediate Actions:**
1. Review Phase 1 deliverables (PRODUCT_VISION.md, REQUIREMENTS.md)
2. Review DECISION_POINTS.md for resolved architectural decisions
3. Validate conditional items (performance, directory structure, MCP)
4. Begin system architecture design

**Phase 2 Agent Invocation Sequence:**
1. `[prd-technical-architect]` - System architecture and component design (first)
2. `[prd-system-design-specialist]` - Orchestration patterns and state management (second)
3. `[prd-api-cli-specialist]` - API specifications and CLI command structure (third)

**Context Handoff:**
- Each agent receives this PHASE_2_CONTEXT.md
- Each agent references PRODUCT_VISION.md and REQUIREMENTS.md
- Each agent applies constraints from DECISION_POINTS.md
- Each agent addresses conditional validation items

---

## Reference Documents

**Primary Sources:**
- Product Vision: `/Users/odgrim/dev/home/agentics/abathur/prd_deliverables/01_PRODUCT_VISION.md` (704 lines)
- Requirements: `/Users/odgrim/dev/home/agentics/abathur/prd_deliverables/02_REQUIREMENTS.md` (1639 lines)
- Decision Points: `/Users/odgrim/dev/home/agentics/abathur/DECISION_POINTS.md` (304 lines)
- Phase 1 Validation: `/Users/odgrim/dev/home/agentics/abathur/prd_deliverables/PHASE_1_VALIDATION.md`

**Key Metrics Quick Reference:**
- Queue operations: <100ms p95
- Agent spawn: <5s p95
- Status queries: <50ms p95
- Concurrent agents: 10+ (configurable to 50+)
- Task persistence: >99.9% reliability
- Queue capacity: 1,000+ tasks (configurable to 10,000)
- System overhead: <200MB memory
- Test coverage: >80% line, >90% critical path

**Technology Stack Quick Reference:**
- Python 3.10+, Typer CLI, SQLite, asyncio, Poetry
- Leader-follower pattern, numeric 0-10 priority, retry + DLQ
- Directory structure: `.claude/` (shared), `.abathur/` (Abathur-specific)

---

**Phase 2 Teams:** You have a comprehensive foundation. Focus on designing clean, modular architecture that enables the vision while meeting aggressive performance targets. Address conditional validation items early. Good luck!

---

**Document Status:** Complete
**Phase:** Phase 2 Entry
**Approval Status:** APPROVED (CONDITIONAL) - 3 validation items
**Date:** 2025-10-09
