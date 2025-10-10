# Claude Code Kickoff Prompt
## OAuth-Based Agent Spawning PRD Development

**CRITICAL: This prompt should ONLY be used AFTER all decision points in DECISION_POINTS.md have been resolved by a human.**

---

## Copy This Entire Section Into Claude Code

```
🚀 **Project Kickoff: OAuth-Based Agent Spawning PRD Development**

I'm ready to develop a comprehensive Product Requirements Document (PRD) for adding OAuth-based agent spawning capabilities to the Abathur system using a coordinated multi-agent approach.

**IMPORTANT HANDOFF INSTRUCTION:**
If you are the general-purpose Claude agent, DO NOT attempt to create the PRD yourself. Instead, immediately invoke the project-specific orchestrator agent: `[prd-project-orchestrator]`

---

## Project Overview

**Objective**: Create a comprehensive PRD that documents how to add OAuth-based agent spawning to Abathur alongside the existing API key approach, enabling users to leverage Claude Max subscriptions for agent execution.

**Context**:
- Current Abathur uses Claude Agent SDK with API key authentication (x-api-key header)
- API key approach has usage plan and rate limit constraints
- Claude Max subscriptions offer OAuth authentication with different capabilities
- Research shows multiple OAuth interaction methods exist (Claude Code CLI, Agent SDK OAuth, community tools, MCP)
- Goal: Design dual-mode authentication architecture (API key + OAuth) for maximum flexibility

**Success Criteria**:
1. Comprehensive research on ALL OAuth-based Claude interaction methods
2. Complete technical requirements (functional + non-functional)
3. Dual-mode spawning architecture design
4. Security architecture for OAuth token management
5. Phased implementation roadmap
6. Production-ready PRD document

---

## Prerequisites Verification

**Before proceeding, verify**:
- [ ] DECISION_POINTS.md has been reviewed and all decisions marked as "RESOLVED"
- [ ] All 14 decision points have human input filled in
- [ ] Document status changed from "AWAITING HUMAN INPUT" to "RESOLVED"

**If prerequisites not met**: STOP and escalate to human to complete DECISION_POINTS.md first.

**If prerequisites met**: Proceed with agent orchestration.

---

## Agent Team & Model Classes

**Core Management (Sonnet)**
- `[prd-project-orchestrator]` - Project coordination, phase validation, go/no-go decisions (Sonnet)

**Research & Analysis (Sonnet + Thinking)**
- `[oauth-research-specialist]` - Comprehensive OAuth method research (Sonnet)
- `[code-analysis-specialist]` - Current codebase analysis (Thinking)

**Design & Planning (Sonnet)**
- `[technical-requirements-analyst]` - Requirements specification (Sonnet)
- `[system-architect]` - Architecture design (Sonnet)
- `[security-specialist]` - Security architecture (Sonnet)
- `[implementation-roadmap-planner]` - Implementation planning (Sonnet)

**Documentation (Haiku)**
- `[prd-documentation-specialist]` - PRD consolidation (Haiku)

---

## Phased Execution Sequence

### Phase 1: Research & Discovery
1. `[oauth-research-specialist]` - Research ALL OAuth-based Claude interaction methods
   - Claude Code CLI subshell invocation
   - Claude Agent SDK OAuth support
   - claude_max community tool
   - MCP with OAuth
   - Any other OAuth methods
   - Comparative analysis (features, rate limits, context windows, pros/cons)

2. `[code-analysis-specialist]` - Analyze current Abathur implementation
   - Review ClaudeClient, AgentExecutor, ConfigManager
   - Identify integration points for OAuth spawning
   - Document current architecture patterns

3. `[prd-project-orchestrator]` - **PHASE 1 VALIDATION GATE**
   - Review research comprehensiveness and current state analysis
   - **Decision**: APPROVE / CONDITIONAL / REVISE / ESCALATE
   - Generate Phase 2 context with research findings

### Phase 2: Requirements & Architecture
4. `[technical-requirements-analyst]` - Define technical requirements
   - Functional requirements for dual-mode spawning
   - Non-functional requirements (performance, security, reliability)
   - Requirements traceability matrix
   - Acceptance criteria
   - Reference DECISION_POINTS.md for resolved decisions

5. `[system-architect]` - Design dual-mode architecture
   - AgentSpawner abstraction with multiple implementations
   - Configuration system for mode selection
   - Integration with existing Clean Architecture
   - Component and integration diagrams
   - Reference DECISION_POINTS.md for architectural decisions

6. `[prd-project-orchestrator]` - **PHASE 2 VALIDATION GATE**
   - Review requirements and architecture
   - **Decision**: APPROVE / CONDITIONAL / REVISE / ESCALATE
   - Generate Phase 3 context

### Phase 3: Security & Implementation Planning
7. `[security-specialist]` - Design security architecture
   - OAuth token security and encryption
   - Threat modeling for dual-mode auth
   - Credential management architecture
   - Security testing requirements
   - Reference DECISION_POINTS.md for security decisions

8. `[implementation-roadmap-planner]` - Create implementation roadmap
   - Break down into implementation phases
   - Define milestones and deliverables
   - Timeline estimates and dependencies
   - Risk assessment and mitigation
   - Rollout strategy

9. `[prd-project-orchestrator]` - **PHASE 3 VALIDATION GATE**
   - Review security and roadmap
   - **Decision**: APPROVE / CONDITIONAL / REVISE / ESCALATE
   - Generate Phase 4 context

### Phase 4: Documentation & Consolidation
10. `[prd-documentation-specialist]` - Create comprehensive PRD
    - Consolidate all deliverables from phases 1-3
    - Create cohesive PRD structure
    - Ensure consistency and completeness
    - Add executive summary and appendices

11. `[prd-project-orchestrator]` - **FINAL VALIDATION GATE**
    - Review complete PRD
    - **Decision**: COMPLETE / CONDITIONAL / REVISE / ESCALATE
    - Generate final summary

---

## Critical Phase Validation Requirements

**At each validation gate, the orchestrator MUST**:
1. Thoroughly review all phase deliverables
2. Validate alignment with project objectives
3. Assess quality and completeness
4. Make explicit go/no-go decision
5. Update TODO list to reflect phase completion
6. Generate refined context for next phase
7. Document validation decision with rationale

**Validation Decisions**:
- **APPROVE**: All deliverables meet quality gates, proceed to next phase
- **CONDITIONAL**: Minor issues identified, proceed with monitoring
- **REVISE**: Significant gaps, return agents to address deficiencies
- **ESCALATE**: Fundamental problems requiring human oversight

---

## Context Passing Instructions

After each agent completes their work, invoke the next agent with:
- Summary of what was completed
- Key findings and decisions
- Files created/modified with absolute paths
- Relevant decisions from DECISION_POINTS.md
- Specific deliverables expected from next agent
- Success criteria for validation

---

## Important Guidelines

**For General-Purpose Agent**:
- **DO NOT** perform research, analysis, or design work directly
- Your ONLY job is to invoke `[prd-project-orchestrator]`
- Use bracket syntax: `[prd-project-orchestrator]`
- Never skip agent handoffs

**For All Agents**:
- Review DECISION_POINTS.md for resolved architectural decisions
- Complete your specific deliverables before handing off
- Provide clear, structured output for next agent
- Flag any newly discovered decision points
- Report blockers immediately to orchestrator
- Use absolute file paths in all deliverables

**For Orchestrator**:
- Enforce phase validation gates - never skip
- Make explicit validation decisions with rationale
- Track all work with TodoWrite tool
- Ensure agents have complete context
- Maintain architectural consistency across phases
- Escalate fundamental blockers to human

---

## Expected Final Deliverables

1. **OAuth Research Document** - Comprehensive analysis of all OAuth methods
2. **Current Architecture Analysis** - Abathur codebase integration points
3. **Technical Requirements** - Complete functional and non-functional specs
4. **System Architecture** - Dual-mode spawning design with diagrams
5. **Security Architecture** - OAuth token security and threat model
6. **Implementation Roadmap** - Phased plan with milestones and timelines
7. **Comprehensive PRD** - Consolidated document ready for implementation

All deliverables will be in:
`/Users/odgrim/dev/home/agentics/abathur/prd_oauth_spawning/`

---

## Initial Request

Please begin by invoking `[prd-project-orchestrator]` to:
1. Verify DECISION_POINTS.md has been resolved
2. Initialize the TODO list for the project
3. Begin Phase 1 by invoking `[oauth-research-specialist]` and `[code-analysis-specialist]`

The orchestrator should coordinate the entire multi-phase PRD development process, ensuring quality gates are met before phase transitions.

**🚨 MANDATORY FOR GENERAL-PURPOSE AGENT 🚨**
DO NOT perform PRD work directly - invoke `[prd-project-orchestrator]` immediately using bracket syntax.

Ready to develop a comprehensive, implementation-ready PRD for OAuth-based agent spawning!
```

---

## Post-Kickoff Notes for Human

After using this kickoff prompt:

1. **Monitor Phase Validations**: The orchestrator will provide validation reports after each phase
2. **Review Validation Decisions**: If orchestrator returns ESCALATE, human review is needed
3. **Track Progress**: Orchestrator maintains TODO list visible in Claude Code
4. **Final Review**: When orchestrator delivers FINAL_PRD.md, review for completeness
5. **Provide Feedback**: If revisions needed, provide specific guidance to orchestrator

**Expected Timeline** (with all decisions pre-resolved):
- Phase 1: 2-3 hours (research + analysis)
- Phase 2: 2-3 hours (requirements + architecture)
- Phase 3: 2-3 hours (security + roadmap)
- Phase 4: 1-2 hours (documentation consolidation)
- **Total**: ~8-11 hours of agent work

**Success Indicators**:
- All validation gates pass with APPROVE
- No ESCALATE decisions
- Final PRD is comprehensive and actionable
- All OAuth methods researched and documented
- Architecture supports both API key and OAuth
- Security design is robust
- Implementation roadmap is realistic

---

**Document Version**: 1.0
**Created**: 2025-10-09
**Status**: Ready for use (pending DECISION_POINTS.md resolution)
