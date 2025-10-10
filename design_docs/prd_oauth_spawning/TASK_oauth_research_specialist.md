# TASK SPECIFICATION: oauth-research-specialist

**Status**: READY FOR INVOCATION
**Agent**: oauth-research-specialist
**Phase**: Phase 1 - Research & Discovery
**Created**: 2025-10-09
**Priority**: HIGH (Phase 1 blocking task)

## Agent Invocation Command

```bash
# Invoke this agent with:
@oauth-research-specialist
```

## Task Overview

Conduct comprehensive research on ALL methods for interacting with Claude services using OAuth authentication tokens. This research will inform the PRD for adding OAuth-based agent spawning to Abathur.

## Context

**Project**: OAuth-Based Agent Spawning for Abathur
**Current State**: Abathur uses Claude Agent SDK with API key authentication (x-api-key header)
**Goal**: Design dual-mode authentication architecture (API key + OAuth) to leverage Claude Max subscriptions

**Read First**: `/Users/odgrim/dev/home/agentics/abathur/prd_oauth_spawning/00_phase1_context.md`
**Read Second**: `/Users/odgrim/dev/home/agentics/abathur/prd_oauth_spawning/DECISION_POINTS.md`

## Specific Research Questions

### 1. OAuth Method Catalog
Discover and document ALL methods for Claude OAuth interaction:
- [ ] Claude Code CLI subshell invocation
- [ ] Claude Agent SDK (formerly claude-code-sdk) OAuth support
- [ ] claude_max community tool/script
- [ ] MCP (Model Context Protocol) with OAuth
- [ ] Claude.ai web API endpoints (if OAuth-accessible)
- [ ] Third-party wrappers and tools
- [ ] Beta features or experimental APIs
- [ ] GitHub Actions integrations with OAuth
- [ ] Any other methods discovered through research

### 2. Method Deep Dive (For Each Discovered Method)
Document in detail:

#### Authentication Mechanism
- How are OAuth tokens obtained? (flow type, endpoints)
- How are tokens used? (header format, authentication pattern)
- What credentials are required? (client ID, client secret, etc.)
- Is PKCE required?
- What scopes are needed?

#### Capabilities
- Text generation support (streaming/non-streaming)
- Tool use / function calling support
- File operations (upload/download)
- MCP server integration capability
- Agent SDK compatibility
- Multi-turn conversation support
- Prompt caching support

#### Rate Limits & Restrictions
- Message limits (per hour, per day, per 5-hour window)
- Token limits (context window)
- Concurrent request limits
- Usage tier requirements (Max 5x vs Max 20x)
- Hard vs soft limits
- Rate limit reset windows

#### Context Window Size
- Maximum context window in tokens
- Comparison to API key context window (1M tokens)
- Impact on agent task complexity
- Truncation behavior

#### Model Access
- Which models are available? (Opus, Sonnet, Haiku, versions)
- Model availability by subscription tier
- Default model selection
- Model switching capabilities

#### Subscription Requirements
- Minimum Claude subscription tier
- Pricing implications
- Free tier availability (if any)
- Enterprise requirements

#### Technical Implementation
- Code examples (Python preferred)
- Configuration requirements
- Dependency requirements
- Error handling patterns
- Retry logic recommendations

#### Pros & Cons
- Strengths of this approach
- Weaknesses and limitations
- Use cases where this excels
- Use cases where this fails

#### Official vs Community Support
- Official Anthropic support level
- Documentation quality
- Community adoption
- Maintenance status
- Breaking change risk
- Deprecation timeline (if applicable)

### 3. Comparative Analysis

Create detailed comparison tables:

#### Feature Matrix
| Feature | API Key | Claude Code CLI | Agent SDK OAuth | claude_max | MCP OAuth | Other Methods |
|---------|---------|-----------------|-----------------|------------|-----------|---------------|
| Streaming | | | | | | |
| Tool Use | | | | | | |
| File Operations | | | | | | |
| MCP Integration | | | | | | |
| Agent SDK Compat | | | | | | |
| Prompt Caching | | | | | | |

#### Rate Limits Comparison
| Method | Messages/5h | Context Window | Concurrent | Notes |
|--------|-------------|----------------|------------|-------|
| API Key (Pay-per-token) | | 1M tokens | | |
| OAuth Max 5x | | | | |
| OAuth Max 20x | | | | |

#### Cost Analysis
- Subscription cost vs API pay-per-token
- Break-even analysis (messages per month)
- Cost per 1M tokens equivalent
- Hidden costs (setup, maintenance, complexity)

#### Integration Complexity
- Setup difficulty (1-10 scale)
- Code changes required
- Configuration complexity
- Maintenance burden
- Documentation quality

### 4. Security Research

#### OAuth Token Lifecycle
- Token expiration duration
- Refresh token availability
- Refresh flow mechanics
- Automatic refresh support
- Token revocation mechanisms

#### Storage Best Practices
- Secure storage options (keychain, vault, env vars)
- Encryption requirements
- Access control considerations
- Token sharing implications

#### Security Comparison: OAuth vs API Key
- Attack surface differences
- Credential exposure risks
- Rotation requirements
- Audit trail capabilities
- Compliance considerations (SOC2, GDPR, etc.)

#### Multi-User Considerations
- Per-user token isolation
- Team/organization tokens
- Token sharing policies
- Audit logging requirements

### 5. Edge Cases & Limitations

Document known issues:
- Token expiration during long-running tasks
- Network interruption handling
- Service outage behavior
- Fallback scenarios
- Error message clarity
- Debug difficulty

## Research Methodology

### Sources to Consult
1. **Official Anthropic Documentation** (PRIMARY)
   - docs.anthropic.com
   - GitHub: anthropics/anthropic-sdk-python
   - GitHub: anthropics/claude-code-sdk
   - Anthropic community forums

2. **Community Resources** (SECONDARY)
   - claude_max tool repository
   - Reddit r/ClaudeAI
   - HackerNews discussions
   - Medium/blog posts (2024-2025)
   - Stack Overflow

3. **Technical References** (TERTIARY)
   - OAuth 2.1 specification
   - PKCE RFC
   - Claude API changelog
   - SDK release notes

### Validation Approach
- Cross-reference findings across multiple sources
- Prioritize official Anthropic sources
- Flag contradictory information
- Note information currency (prefer 2025 > 2024 > older)
- Test claims when feasible

## Deliverable Format

**Output File**: `/Users/odgrim/dev/home/agentics/abathur/prd_oauth_spawning/01_oauth_research.md`

### Required Sections
1. **Executive Summary** (1-2 pages)
   - Key findings
   - Recommended approach
   - Critical decision points

2. **OAuth Method Catalog** (Complete list with brief overview)

3. **Method Deep Dives** (One section per method)
   - Follow structure from "Method Deep Dive" above

4. **Comparative Analysis** (Tables and narrative)
   - Feature matrix
   - Rate limits comparison
   - Cost analysis
   - Integration complexity

5. **Security Analysis**
   - Token lifecycle
   - Storage recommendations
   - Security comparison
   - Multi-user considerations

6. **Recommendations for Abathur**
   - Primary OAuth method recommendation
   - Fallback options
   - Integration approach
   - Security best practices
   - Testing strategy

7. **Open Questions**
   - Unresolved questions requiring human input
   - Areas needing further investigation
   - Assumptions made

8. **References**
   - All sources cited with URLs
   - Documentation versions
   - Research date range

## Success Criteria

- [ ] All OAuth methods comprehensively documented
- [ ] Each method has complete deep dive analysis
- [ ] Comparative tables are complete and accurate
- [ ] Security analysis covers token lifecycle completely
- [ ] Recommendations are clear and justified
- [ ] All sources cited with links
- [ ] No contradictory information left unresolved
- [ ] Code examples provided for each method
- [ ] Edge cases and limitations documented
- [ ] Open questions clearly identified

## Validation Gate

After completion, the prd-project-orchestrator will:
1. Review for completeness against this specification
2. Validate accuracy of findings
3. Check for consistency with DECISION_POINTS.md
4. Make validation decision: APPROVE / CONDITIONAL / REVISE / ESCALATE

## Timeline

**Target Completion**: Same session
**Estimated Effort**: 2-3 hours of research and documentation
**Blocking**: code-analysis-specialist (parallel, not blocking)
**Blocked By**: None (ready to start)

## Notes

- Use WebSearch and WebFetch tools extensively
- Prefer official Anthropic documentation
- Document research date for time-sensitive information
- Flag any Beta/experimental features clearly
- Note maintenance status of community tools
- Provide working code examples where possible
- Cross-reference with DECISION_POINTS.md for alignment

---

**FOR AGENT**: Read `/Users/odgrim/dev/home/agentics/abathur/prd_oauth_spawning/00_phase1_context.md` and `/Users/odgrim/dev/home/agentics/abathur/prd_oauth_spawning/DECISION_POINTS.md` before beginning research.
