# Abathur PRD Development - Claude Code Kickoff Prompt

## IMPORTANT: Pre-Execution Checklist

**CRITICAL - RESOLVE DECISION POINTS FIRST:**

Before using this kickoff prompt, you MUST review and resolve all decision points in `DECISION_POINTS.md`. This ensures agents are not blocked during execution waiting for architectural decisions.

**Quick Decision Resolution:**
1. Open `DECISION_POINTS.md`
2. Review each decision category (29 total decisions)
3. Fill in your answers (or accept suggested defaults)
4. Save the file with resolved decisions
5. THEN proceed with the kickoff prompt below

**Minimum Required Decisions:**
- Decision 1: Task Queue Implementation (recommended: SQLite)
- Decision 4: CLI Framework (recommended: Typer)
- Decision 6: Agent Spawning Strategy (recommended: Async/await)
- Decision 11: Swarm Coordination Model (recommended: Leader-follower)

Once decision points are resolved, copy and paste the prompt below into Claude Code.

---

## Claude Code Kickoff Prompt

**COPY AND PASTE THIS INTO CLAUDE CODE:**

---

I'm ready to develop a comprehensive Product Requirements Document (PRD) for Abathur, a hivemind swarm management system for Claude agents, using a coordinated multi-agent approach.

**Project Overview:**
- **Name**: Abathur
- **Purpose**: Multi-agent orchestration system for Claude agents
- **Technology**: Python 3.10+, Claude Agent SDK, CLI-first design
- **Repositories**:
  - `odgrim/abathur-swarm` (main codebase and CLI)
  - `odgrim/abathur-claude-template` (template repository)

**Core Functionality:**
1. **Template Management**: Clone and install project templates
2. **Task Queue**: Manage task queue with priority and persistence
3. **Swarm Coordination**: Orchestrate multiple Claude agents concurrently
4. **Loop Execution**: Iterative task execution with convergence criteria
5. **CLI Tool**: Comprehensive command-line interface

**PRD Scope:**
The PRD must cover:
- Product vision, goals, and target users
- Detailed use cases and user scenarios
- Functional and non-functional requirements
- System architecture and technology stack
- Orchestration algorithms and protocols
- API and CLI specifications
- Security and compliance considerations
- Success metrics and quality gates
- Phased implementation roadmap
- Comprehensive documentation

**Model Class Specifications:**
- **Orchestrator & Specialists**: Sonnet for coordination, architecture, planning, and analysis
- **Documentation**: Haiku for final PRD compilation and formatting

**CRITICAL HANDOFF INSTRUCTIONS:**
**If you are the general-purpose agent, DO NOT attempt to write the PRD yourself!**
Instead, you MUST immediately invoke the `[prd-project-orchestrator]` agent to coordinate the specialized agent team.

**Agent Team & Execution Sequence:**

**Phase 1: Vision & Requirements**
1. `[prd-product-vision-specialist]` - Define product vision, goals, target users, and use cases (Sonnet)
2. `[prd-requirements-analyst]` - Document functional and non-functional requirements (Sonnet)
3. `[prd-project-orchestrator]` - **PHASE 1 VALIDATION GATE** - Review and validate all planning deliverables (Sonnet)

**Phase 2: Technical Architecture & Design**
4. `[prd-technical-architect]` - Design system architecture and technology stack (Sonnet)
5. `[prd-system-design-specialist]` - Specify orchestration algorithms and protocols (Sonnet)
6. `[prd-api-cli-specialist]` - Define API specifications and CLI commands (Sonnet)
7. `[prd-project-orchestrator]` - **PHASE 2 VALIDATION GATE** - Review and validate all technical deliverables (Sonnet)

**Phase 3: Quality, Security & Implementation Planning**
8. `[prd-security-specialist]` - Conduct threat modeling and define security requirements (Sonnet)
9. `[prd-quality-metrics-specialist]` - Define success metrics and quality gates (Sonnet)
10. `[prd-implementation-roadmap-specialist]` - Create phased implementation plan with timeline (Sonnet)
11. `[prd-project-orchestrator]` - **PHASE 3 VALIDATION GATE** - Review and validate all quality/planning deliverables (Sonnet)

**Phase 4: PRD Compilation & Finalization**
12. `[prd-documentation-specialist]` - Compile all sections into final comprehensive PRD (Haiku)
13. `[prd-project-orchestrator]` - **FINAL VALIDATION** - Review PRD completeness and quality (Sonnet)

**Context Passing Instructions:**
After each agent completes their work, the orchestrator will invoke the next agent with:
- Summary of what was completed
- Key findings and recommendations
- Files created with absolute paths
- Context needed for next agent's work
- Any issues or decision points discovered

**CRITICAL: Phase Validation Requirements**
At each validation gate, the orchestrator MUST:
- Thoroughly review all phase deliverables
- Validate completeness and quality
- Check alignment with project objectives
- Assess readiness for next phase
- Make explicit go/no-go decision (APPROVE/CONDITIONAL/REVISE/ESCALATE)
- Generate refined context for next phase agents
- Update project state based on findings

**Decision Points Reference:**
All agents should reference the resolved `DECISION_POINTS.md` file for:
- Architectural decisions (queue implementation, CLI framework, etc.)
- Technology choices (Python version, dependencies, etc.)
- Business logic (coordination model, priority system, etc.)
- Performance requirements (concurrency, latency targets, etc.)
- Security constraints (API key management, logging, etc.)

Agents should flag any newly discovered decision points for orchestrator resolution.

**Initial Request:**
Please begin by invoking the `[prd-project-orchestrator]` to coordinate the entire PRD development process. The orchestrator will manage phase validation, agent coordination, and ensure comprehensive PRD coverage.

**MANDATORY FOR GENERAL-PURPOSE AGENT:**
**DO NOT write PRD sections directly!**
Your ONLY job is to invoke the `[prd-project-orchestrator]` agent:
- Use the `[prd-project-orchestrator]` bracket syntax to invoke
- Let the orchestrator manage all specialist agents
- Trust the agent team to collaboratively develop the PRD
- Never skip the orchestration workflow

**Expected Timeline:**
- Phase 1: ~2 hours (Vision & Requirements)
- Phase 2: ~3 hours (Technical Architecture & Design)
- Phase 3: ~2 hours (Quality, Security & Planning)
- Phase 4: ~1 hour (Compilation & Finalization)
- Total: ~8 hours for comprehensive PRD

**Expected Deliverable:**
A complete, industry-standard PRD document (`ABATHUR_PRD.md`) covering all aspects of the Abathur system, ready to guide a 25-week implementation timeline.

**Success Criteria:**
- All 9 specialist agents contribute deliverables
- All 4 phase validation gates passed
- Final PRD is comprehensive, consistent, and actionable
- No critical gaps or contradictions
- Development team can immediately begin implementation planning

Ready to begin the coordinated PRD development! Please invoke `[prd-project-orchestrator]` to start.

---

## What Happens Next

1. **General-Purpose Agent** invokes `[prd-project-orchestrator]`
2. **Orchestrator** manages the entire workflow:
   - Invokes Phase 1 agents (vision, requirements)
   - Validates Phase 1 deliverables
   - Invokes Phase 2 agents (architecture, design, API)
   - Validates Phase 2 deliverables
   - Invokes Phase 3 agents (security, metrics, roadmap)
   - Validates Phase 3 deliverables
   - Invokes Phase 4 agent (documentation compilation)
   - Performs final validation
3. **Final Output**: Comprehensive `ABATHUR_PRD.md` document

## Troubleshooting

**If general-purpose agent tries to write PRD directly:**
- Stop and redirect to `[prd-project-orchestrator]`
- Emphasize that orchestration is mandatory

**If an agent fails to produce output:**
- Orchestrator will retry with enhanced context
- Maximum 2 retries before escalation

**If validation gate identifies issues:**
- Orchestrator will return to specific agents for revision
- Context will include specific feedback and improvement areas

**If new decision points discovered:**
- Agent flags to orchestrator
- Orchestrator documents in decision log
- Human stakeholder resolves if critical
- Otherwise, orchestrator makes reasonable default choice

## Support Documents

- **Agent Definitions**: `.claude/agents/prd-*.md` (10 agent files)
- **Decision Points**: `DECISION_POINTS.md` (must be resolved first)
- **Handoff Package**: `PRD_ORCHESTRATOR_HANDOFF.md` (detailed coordination guide)
- **This Prompt**: `CLAUDE_CODE_KICKOFF_PROMPT.md`

## Post-PRD Next Steps

Once the PRD is complete:
1. Review `ABATHUR_PRD.md` with stakeholders
2. Use PRD to guide implementation planning
3. Create implementation agents based on roadmap phases
4. Begin Phase 0 (Foundation & Setup) of development
5. Iterate on PRD as implementation insights emerge

---

**The specialized agent team is ready. Let's build the Abathur PRD!**
