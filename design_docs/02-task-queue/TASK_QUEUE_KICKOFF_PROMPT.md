# Task Queue System - Claude Code Kickoff Prompt

**IMPORTANT: Resolve all decision points in TASK_QUEUE_DECISION_POINTS.md BEFORE using this prompt!**

---

## COPY AND PASTE THIS INTO CLAUDE CODE:

---

I'm ready to begin implementing the enhanced task queue system with dependency management and priority scheduling for the Abathur multi-agent framework.

**CRITICAL HANDOFF INSTRUCTIONS:**
**If you are the general purpose agent, DO NOT attempt to do the work yourself!**
Instead, you MUST immediately invoke the `[task-queue-orchestrator]` agent who will coordinate the specialized implementation team.

**Project Overview:**
- **Objective:** Implement hierarchical task queue with dependency management, priority-based scheduling, and agent subtask submission
- **Tech Stack:** Python 3.12+, SQLite with aiosqlite, Pydantic, asyncio
- **Success Criteria:**
  1. Agents can submit subtasks programmatically
  2. Dependencies block task execution until prerequisites complete
  3. Priority-based scheduling with dynamic re-prioritization
  4. Source tracking (HUMAN vs AGENT_* origins)
  5. Circular dependency detection and prevention
  6. Performance: 1000+ tasks/sec enqueue, <10ms dependency resolution
  7. Integration with existing memory system
  8. Comprehensive test coverage

**Model Class Specifications:**
- Thinking: Algorithm design, Python implementation, debugging
- Sonnet: Architecture, orchestration, schema design
- Haiku: Documentation, content creation

**Agent Team & Execution Sequence:**

**Phase 1: Schema & Domain Models (2 days)**
1. `[task-queue-orchestrator]` - Validates decision points and kickoffs Phase 1 (Sonnet)
2. `[database-schema-architect]` - Designs and implements schema updates (Sonnet)

**Phase 2: Dependency Resolution (3 days)**
3. `[algorithm-design-specialist]` - Implements circular dependency detection algorithm (Thinking)

**Phase 3: Priority Calculation (2 days)**
4. `[python-backend-developer]` - Implements PriorityCalculator service (Thinking)

**Phase 4: Task Queue Service (3 days)**
5. `[python-backend-developer]` - Refactors TaskQueueService with dependency enforcement (Thinking)

**Phase 5: Testing & Integration (2 days)**
6. `[test-automation-engineer]` - Comprehensive test suite (Thinking)
7. `[performance-optimization-specialist]` - Performance validation and optimization (Thinking)
8. `[technical-documentation-writer]` - API docs and user guide (Haiku)
9. `[task-queue-orchestrator]` - Final validation and go/no-go decision (Sonnet)

**Support Agents (On-Demand):**
- `[python-debugging-specialist]` - Error escalation and debugging (Thinking)

**Context Passing Instructions:**
After each agent completes their work, the orchestrator will:
- Review deliverables against acceptance criteria
- Run validation tests at phase gate
- Make go/no-go decision (APPROVE/CONDITIONAL/REVISE/ESCALATE)
- Generate refined context for next phase agents

**CRITICAL: Phase Validation Requirements**
At each phase gate, the orchestrator must:
- Thoroughly review all deliverables
- Validate alignment with architecture specifications
- Assess readiness for next phase
- Make explicit go/no-go decision before proceeding
- Update implementation plan based on findings
- Generate refined context for next phase agents

**Architecture References:**
- Architecture: `/Users/odgrim/dev/home/agentics/abathur/design_docs/TASK_QUEUE_ARCHITECTURE.md`
- Decision Points: `/Users/odgrim/dev/home/agentics/abathur/design_docs/TASK_QUEUE_DECISION_POINTS.md` (must be resolved!)
- Current Implementation: `/Users/odgrim/dev/home/agentics/abathur/src/abathur/domain/models.py`
- Database Layer: `/Users/odgrim/dev/home/agentics/abathur/src/abathur/infrastructure/database.py`

**Initial Request:**
Please begin with the `[task-queue-orchestrator]` to start Phase 1. The orchestrator should:
1. Verify all decision points are resolved in TASK_QUEUE_DECISION_POINTS.md
2. Review architecture document and existing implementation
3. Create initial TODO list with phase gates
4. Invoke `[database-schema-architect]` with complete context for schema design

Each agent should:
- Review context from orchestrator
- Reference TASK_QUEUE_DECISION_POINTS.md for architectural decisions
- Complete specific deliverables per phase
- Report any newly discovered decision points to orchestrator
- Use TodoWrite to track progress and blockers
- Invoke debugging specialists if blocked (use `[python-debugging-specialist]`)

**MANDATORY FOR GENERAL PURPOSE AGENT:**
**DO NOT perform architecture, design, or implementation work directly!**
Your ONLY job is to invoke the `[task-queue-orchestrator]` who will coordinate the specialized team.

Ready to begin the coordinated implementation!

---

**End of Kickoff Prompt - Save this file for reference**
