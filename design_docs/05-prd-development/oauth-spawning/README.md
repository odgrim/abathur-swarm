# OAuth-Based Agent Spawning PRD Project

This directory contains the complete meta-orchestration deliverables for developing a comprehensive Product Requirements Document (PRD) for adding OAuth-based agent spawning to Abathur.

## Quick Start

**IMPORTANT**: Follow these steps in order:

1. **Read the Research Summary** - See "Key Research Findings" section below
2. **Review Decision Points** - Open `DECISION_POINTS.md` and resolve all 14 decisions
3. **Mark Decisions Complete** - Update document status to "RESOLVED"
4. **Use Kickoff Prompt** - Copy content from `KICKOFF_PROMPT.md` into Claude Code
5. **Monitor Progress** - Watch for phase validation reports from orchestrator

## Key Research Findings

### OAuth vs API Keys: Critical Insight

**IMPORTANT**: OAuth tokens CANNOT be used as API keys. They serve different purposes:
- **API Keys**: `x-api-key` header format (sk-ant-...), direct API endpoint calls
- **OAuth Tokens**: User authentication for Claude Code/Max plans, NOT for direct API use

### OAuth-Based Claude Interaction Methods

Based on comprehensive research, we identified these methods:

1. **Claude Agent SDK with OAuth** (RECOMMENDED PRIMARY)
   - Official Python package with OAuth 2.0 + PKCE
   - Programmatic API, Python-native
   - Context: 200K tokens (vs 1M for API key)
   - Rate limits: ~50-200 or ~200-800 per 5 hours (Max 5x/20x)

2. **Claude Code CLI Subshell** (RECOMMENDED SECONDARY)
   - Invoke `claude -p "prompt" --json` programmatically
   - Leverages Max subscription
   - Requires Claude Code installation
   - Reported OAuth authentication issues in 2025

3. **claude_max Community Tool** (NOT RECOMMENDED)
   - Unofficial workaround for programmatic Max access
   - Potential breaking changes, maintenance concerns

4. **MCP with OAuth** (NOT FOR AGENT SPAWNING)
   - Focused on service integrations, not Claude API access
   - Overkill for agent spawning use case

### Comparative Analysis

| Method | Context Window | Rate Limits | Integration Ease | Stability |
|--------|----------------|-------------|------------------|-----------|
| Agent SDK (API Key) | 1M tokens | Pay-per-token | High | High |
| Agent SDK (OAuth) | 200K tokens | 50-800/5h | High | High |
| Claude Code CLI | 200K tokens | 50-800/5h | Medium | Medium |
| claude_max | 200K tokens | 50-800/5h | Low | Low |

## Project Structure

```
prd_oauth_spawning/
├── README.md (this file)
├── DECISION_POINTS.md (HUMAN INPUT REQUIRED - resolve before proceeding)
├── ORCHESTRATION_PLAN.md (agent execution sequence)
├── KICKOFF_PROMPT.md (ready-to-paste Claude Code prompt)
├── META_ORCHESTRATOR_REPORT.md (comprehensive findings and recommendations)
├── phase1/ (created by agents)
├── phase2/ (created by agents)
├── phase3/ (created by agents)
├── phase4/ (created by agents)
└── validation_reports/ (created by agents)
```

## Agent Team (8 Agents)

All agents located in: `/Users/odgrim/dev/home/agentics/abathur/.claude/agents/`

### Management
- **prd-project-orchestrator** (Sonnet) - Coordination, phase validation, go/no-go decisions

### Specialists
- **oauth-research-specialist** (Sonnet) - OAuth method research
- **code-analysis-specialist** (Thinking) - Codebase analysis
- **technical-requirements-analyst** (Sonnet) - Requirements specification
- **system-architect** (Sonnet) - Architecture design
- **security-specialist** (Sonnet) - Security architecture
- **implementation-roadmap-planner** (Sonnet) - Implementation planning
- **prd-documentation-specialist** (Haiku) - PRD consolidation

## Phased Execution Plan

### Phase 0: Decision Resolution (HUMAN - REQUIRED FIRST)
- Review and complete all 14 decision points in DECISION_POINTS.md
- Mark document status as "RESOLVED"

### Phase 1: Research & Discovery (2-3 hours)
- OAuth method research (comprehensive)
- Current Abathur architecture analysis
- Validation gate: Orchestrator reviews completeness

### Phase 2: Requirements & Architecture (2-3 hours)
- Technical requirements specification
- Dual-mode spawning architecture design
- Validation gate: Orchestrator reviews feasibility

### Phase 3: Security & Planning (2-3 hours)
- Security architecture and threat modeling
- Implementation roadmap with phases
- Validation gate: Orchestrator reviews completeness

### Phase 4: Documentation (1-2 hours)
- PRD consolidation and synthesis
- Final validation: Orchestrator reviews actionability

**Total**: ~8-11 hours of agent work (after decisions resolved)

## Key Recommendations

1. **Prioritize Official Methods**: Focus on Claude Agent SDK OAuth (primary) and Claude Code CLI (secondary)
2. **Design for Extensibility**: Use AgentSpawner abstraction with multiple implementations
3. **Maintain Backward Compatibility**: Existing API key deployments must work unchanged
4. **Delegate Token Management**: Let official SDKs/tools handle OAuth token lifecycle
5. **Front-Load Decisions**: Complete all DECISION_POINTS.md before agent work begins
6. **Track Usage per Mode**: Separate metrics for API key vs OAuth authentication

## Critical Decision Points

Must resolve before proceeding (14 total in DECISION_POINTS.md):

1. OAuth method selection (CLI, SDK, both?)
2. Authentication mode configuration (auto-detect vs explicit)
3. OAuth token storage location
4. Token refresh strategy
5. Backward compatibility approach
6. Rate limiting handling
7. Context window management (1M vs 200K)
8. Model selection across auth modes
9. Testing strategy
10. Error handling and fallback
11. Multi-user support
12. Observability requirements
13. Documentation deliverables
14. Deployment and packaging

## Current Abathur Architecture

### Key Components for OAuth Integration

1. **ClaudeClient** (`src/abathur/application/claude_client.py`)
   - Wraps Anthropic SDK, uses API keys currently
   - Needs enhancement for OAuth modes

2. **AgentExecutor** (`src/abathur/application/agent_executor.py`)
   - Manages agent lifecycle, invokes ClaudeClient
   - Needs spawner abstraction layer

3. **ConfigManager** (`src/abathur/infrastructure/config.py`)
   - Handles API key storage (keychain/env/.env)
   - Needs OAuth token storage support

### Proposed Architecture

```
AgentSpawner (ABC)
├── ApiKeyAgentSpawner (existing Anthropic SDK)
├── OAuthSdkAgentSpawner (Agent SDK with OAuth)
└── OAuthCliAgentSpawner (Claude Code CLI subshell)
```

## Success Metrics

- [ ] All OAuth methods researched and documented
- [ ] Dual-mode architecture designed and validated
- [ ] Security architecture comprehensive
- [ ] Implementation roadmap actionable
- [ ] Final PRD complete and approved
- [ ] Zero ESCALATE decisions (all blockers resolved)

## Files in This Directory

### Meta-Orchestration Deliverables (Complete)
- **DECISION_POINTS.md** - 14 critical decisions (STATUS: AWAITING HUMAN INPUT)
- **ORCHESTRATION_PLAN.md** - Agent execution sequence with validation gates
- **KICKOFF_PROMPT.md** - Ready-to-paste Claude Code prompt
- **META_ORCHESTRATOR_REPORT.md** - Comprehensive research findings and recommendations
- **README.md** - This file

### Future Deliverables (Created by Agents)
- **phase1/** - OAuth research and architecture analysis
- **phase2/** - Requirements and architecture design
- **phase3/** - Security design and implementation roadmap
- **phase4/** - Final consolidated PRD
- **validation_reports/** - Phase validation decisions

## Next Actions

1. **Human**: Review and complete DECISION_POINTS.md
2. **Human**: Copy KICKOFF_PROMPT.md into Claude Code
3. **Agents**: Execute 4-phase PRD development
4. **Human**: Review final PRD and provide approval

## Questions or Issues?

Refer to META_ORCHESTRATOR_REPORT.md for:
- Detailed research findings
- Agent team rationale
- Risk assessment
- Comprehensive recommendations

---

**Project Status**: Ready for human decision resolution
**Created**: 2025-10-09
**Meta-Orchestrator**: Claude Sonnet 4.5
