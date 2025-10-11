# Agent Invocation Guide - Abathur PRD Development

## Current Status: Phase 1 - Vision & Requirements

The PRD orchestrator has prepared the context for Phase 1. The following agents need to be invoked:

## Phase 1 Agent Invocations

### Step 1: Invoke prd-product-vision-specialist

**Agent Name:** `prd-product-vision-specialist`
**Model:** Sonnet 4.5
**Purpose:** Define product vision, target users, use cases, and value proposition

**Invocation Command:**
```bash
claude-code --agent prd-product-vision-specialist
```

**Context to Provide:**

```markdown
You are being invoked as part of the Abathur PRD Development project.

**Current Phase:** Phase 1 - Vision & Requirements

**Project Context:**
- Project: Abathur Hivemind Swarm Management System
- Purpose: Multi-agent orchestration system for Claude agents
- Technology: Python 3.10+, Claude Agent SDK, Typer CLI Framework
- Repositories:
  - odgrim/abathur-swarm (main codebase and CLI)
  - odgrim/abathur-claude-template (template repository)

**Core Functionality:**
1. Template Management: Clone and install project templates from GitHub
2. Task Queue: SQLite-based persistent queue with priority (0-10 scale)
3. Swarm Coordination: Hierarchical orchestration of concurrent Claude agents
4. Loop Execution: Iterative task execution with convergence criteria
5. CLI Tool: Typer-based comprehensive command-line interface

**Key Architectural Decisions (from DECISION_POINTS.md):**
- Task Queue: SQLite-based for persistence
- Agent Communication: Message queue + shared state database
- CLI Framework: Typer (type-safe, modern)
- Configuration: Hybrid (.env for secrets, YAML for structure)
- Swarm Model: Hierarchical with leader-follower elements
- Agent Spawning: Async/await with concurrency limits (default 5-10)
- Failure Recovery: Retry + exponential backoff + dead letter queue

**Design Philosophy:**
- Named after Abathur (StarCraft evolution master)
- Specification-driven development process
- Hyperspecialized agent spawning capability
- Meta-agent for continuous agent improvement
- CLI-first, local development focus
- Developer productivity and flexibility

**Your Specific Task:**
Define the comprehensive product vision for Abathur including:
1. Product vision statement
2. Target user personas (at least 3)
3. Core use cases (at least 5)
4. User journey maps
5. Value proposition and market positioning
6. Success metrics and KPIs

**Reference Documents:**
- Full context: /Users/odgrim/dev/home/agentics/abathur/prd_deliverables/PHASE_1_INVOCATION_CONTEXT.md
- Decisions: /Users/odgrim/dev/home/agentics/abathur/DECISION_POINTS.md
- Orchestrator handoff: /Users/odgrim/dev/home/agentics/abathur/PRD_ORCHESTRATOR_HANDOFF.md

**Output Requirements:**
- Create file: /Users/odgrim/dev/home/agentics/abathur/prd_deliverables/01_PRODUCT_VISION.md
- Follow your agent-specific output schema
- Ensure vision is clear, compelling, and actionable
- Focus on developer productivity and agent orchestration value

**Working Directory:** /Users/odgrim/dev/home/agentics/abathur
```

**Expected Deliverable:** `/Users/odgrim/dev/home/agentics/abathur/prd_deliverables/01_PRODUCT_VISION.md`

---

### Step 2: Invoke prd-requirements-analyst

**Agent Name:** `prd-requirements-analyst`
**Model:** Sonnet 4.5
**Purpose:** Document functional and non-functional requirements

**Invocation Command:**
```bash
claude-code --agent prd-requirements-analyst
```

**Context to Provide:**

```markdown
You are being invoked as part of the Abathur PRD Development project.

**Current Phase:** Phase 1 - Vision & Requirements

**Project Context:**
- Project: Abathur Hivemind Swarm Management System
- Previous Agent Output: Vision document from prd-product-vision-specialist
- Vision Document Location: /Users/odgrim/dev/home/agentics/abathur/prd_deliverables/01_PRODUCT_VISION.md

**Core Functionality (from Vision):**
1. Template Management: Clone and install project templates
2. Task Queue Management: SQLite-based with priority and persistence
3. Swarm Coordination: Orchestrate multiple Claude agents concurrently
4. Loop Execution: Iterative task execution with convergence criteria
5. CLI Tool: Comprehensive command-line interface

**Architectural Decisions (from DECISION_POINTS.md):**
- Task Queue: SQLite-based
- Agent Communication: Message queue + shared state
- State Management: Centralized with event log
- CLI Framework: Typer
- Configuration: Hybrid (.env + YAML)
- Agent Spawning: Async/await with limits
- Coordination Model: Hierarchical leader-follower
- Task Priority: Numeric 0-10 scale
- Failure Recovery: Retry + backoff + DLQ + checkpointing
- Loop Termination: Max iterations + success criteria + timeout

**Performance Requirements:**
- Queue operations: <100ms
- Agent spawn time: <5s
- Status checks: <50ms
- Max concurrent agents: 10 (configurable)
- Queue capacity: 1000 tasks (configurable)
- Memory per agent: 512MB (configurable)
- Total memory: 4GB (configurable)

**Security Requirements:**
- API key management: Environment variables
- Data privacy: Full logging (local tool)
- Access control: Single user (no multi-tenancy)

**Your Specific Task:**
Document comprehensive functional and non-functional requirements including:

1. **Functional Requirements** (categorized by feature area):
   - FR-TEMPLATE-* (Template management)
   - FR-QUEUE-* (Task queue operations)
   - FR-SWARM-* (Swarm coordination)
   - FR-LOOP-* (Loop execution)
   - FR-CLI-* (CLI commands and interface)
   - FR-CONFIG-* (Configuration management)
   - FR-MCP-* (MCP server integration)

2. **Non-Functional Requirements**:
   - NFR-PERF-* (Performance)
   - NFR-SCALE-* (Scalability)
   - NFR-RELIAB-* (Reliability)
   - NFR-MAINT-* (Maintainability)
   - NFR-SECURITY-* (Security)
   - NFR-USABILITY-* (Usability)

3. **Additional Documentation**:
   - Requirements priority classification (P0/P1/P2/P3)
   - Acceptance criteria for each requirement
   - Traceability matrix (requirements → use cases)
   - Constraints documentation
   - Assumptions documentation

**Reference Documents:**
- Vision: /Users/odgrim/dev/home/agentics/abathur/prd_deliverables/01_PRODUCT_VISION.md
- Decisions: /Users/odgrim/dev/home/agentics/abathur/DECISION_POINTS.md
- Phase context: /Users/odgrim/dev/home/agentics/abathur/prd_deliverables/PHASE_1_INVOCATION_CONTEXT.md

**Output Requirements:**
- Create file: /Users/odgrim/dev/home/agentics/abathur/prd_deliverables/02_REQUIREMENTS.md
- Follow your agent-specific output schema
- Ensure requirements are SMART (Specific, Measurable, Achievable, Relevant, Time-bound)
- Create clear traceability to use cases from vision document
- Include comprehensive acceptance criteria

**Working Directory:** /Users/odgrim/dev/home/agentics/abathur
```

**Expected Deliverable:** `/Users/odgrim/dev/home/agentics/abathur/prd_deliverables/02_REQUIREMENTS.md`

---

## After Phase 1 Completion

Once both agents have completed their deliverables:

1. **Review Deliverables:**
   - Verify both files exist and are complete
   - Check for alignment between vision and requirements
   - Ensure no contradictions or gaps

2. **Invoke Orchestrator for Validation:**
   ```bash
   claude-code --agent prd-project-orchestrator
   ```

   Provide context:
   ```
   Phase 1 Complete - Perform Validation Gate Review

   Deliverables:
   - Vision: /Users/odgrim/dev/home/agentics/abathur/prd_deliverables/01_PRODUCT_VISION.md
   - Requirements: /Users/odgrim/dev/home/agentics/abathur/prd_deliverables/02_REQUIREMENTS.md

   Task: Review Phase 1 deliverables and make go/no-go decision for Phase 2
   ```

3. **Validation Outcomes:**
   - **APPROVE:** Proceed to Phase 2 (Technical Architecture & Design)
   - **CONDITIONAL:** Proceed with minor adjustments noted
   - **REVISE:** Return to Phase 1 agents with feedback
   - **ESCALATE:** Require human stakeholder review

---

## Phase 2 Preview (After Phase 1 Validation)

**Agents to Invoke:**
1. `prd-technical-architect` - System architecture and tech stack
2. `prd-system-design-specialist` - Orchestration algorithms and protocols
3. `prd-api-cli-specialist` - API specifications and CLI commands

**Will require:** Phase 1 deliverables as input context

---

**Current Status:** Awaiting Phase 1 agent invocations
**Next Action:** Invoke prd-product-vision-specialist
**Orchestrator:** Ready for validation gate review after Phase 1 completion
