# TASK SPECIFICATION: code-analysis-specialist

**Status**: READY FOR INVOCATION
**Agent**: code-analysis-specialist
**Phase**: Phase 1 - Research & Discovery
**Created**: 2025-10-09
**Priority**: HIGH (Phase 1 blocking task)

## Agent Invocation Command

```bash
# Invoke this agent with:
@code-analysis-specialist
```

## Task Overview

Analyze the current Abathur codebase to identify integration points for OAuth-based agent spawning, document current authentication patterns, and assess the impact of adding dual-mode (API key + OAuth) authentication.

## Context

**Project**: OAuth-Based Agent Spawning for Abathur
**Current State**: Abathur uses Claude Agent SDK with API key authentication (x-api-key header)
**Goal**: Understand current architecture to design OAuth integration strategy

**Read First**: `/Users/odgrim/dev/home/agentics/abathur/prd_oauth_spawning/00_phase1_context.md`
**Read Second**: `/Users/odgrim/dev/home/agentics/abathur/prd_oauth_spawning/DECISION_POINTS.md`

**Codebase Root**: `/Users/odgrim/dev/home/agentics/abathur/`
**Source Directory**: `/Users/odgrim/dev/home/agentics/abathur/src/abathur/`

## Analysis Requirements

### 1. Codebase Discovery & Mapping

#### Directory Structure Analysis
- [ ] Map complete directory structure
- [ ] Identify all Python modules
- [ ] Document package organization
- [ ] Note configuration file locations
- [ ] Identify test directories

#### Key Module Identification
Focus on these critical files:
- `src/abathur/application/claude_client.py` - Claude API client (PRIMARY FOCUS)
- `src/abathur/infrastructure/config.py` - Configuration management (PRIMARY FOCUS)
- `src/abathur/application/agent_executor.py` - Agent lifecycle and execution
- `src/abathur/application/agent_pool.py` - Agent pool management
- `src/abathur/cli/main.py` - CLI entry point
- `src/abathur/domain/models.py` - Domain models
- `src/abathur/infrastructure/logger.py` - Logging infrastructure

### 2. Current Authentication Architecture

#### ClaudeClient Analysis (`application/claude_client.py`)
**Questions to Answer**:
- How is the Claude API client initialized?
- Where is the API key loaded from?
- What authentication mechanism is used? (header format, SDK usage)
- How are requests authenticated?
- What error handling exists for auth failures?
- Is there connection pooling or session management?
- How are retries handled?
- What logging exists for auth events?

**Deliverables**:
- Complete code flow diagram for authentication
- API key usage pattern documentation
- Error handling pattern analysis
- Code snippets demonstrating current approach

#### Configuration Management (`infrastructure/config.py`)
**Questions to Answer**:
- How is configuration loaded? (file, env vars, keychain)
- What configuration format is used? (YAML, JSON, TOML)
- Where is API key stored?
- Is there environment-based config (dev, prod)?
- How are secrets managed?
- Is there validation of config values?
- Can config be reloaded dynamically?
- What configuration hierarchy exists?

**Deliverables**:
- Configuration loading flow diagram
- Config file structure documentation
- Secret management pattern analysis
- Environment variable usage documentation

#### Agent Spawning Workflow (`application/agent_executor.py`)
**Questions to Answer**:
- How are agents spawned?
- When is authentication performed?
- How is the Claude client passed to agents?
- What is the agent lifecycle?
- How are agent errors handled?
- Is there agent pooling or caching?
- How are concurrent agents managed?

**Deliverables**:
- Agent lifecycle diagram
- Spawning sequence documentation
- Error handling flow
- Code examples

### 3. Integration Point Identification

For OAuth-based authentication, identify:

#### Authentication Initialization Points
- [ ] Where is ClaudeClient instantiated?
- [ ] Where is authentication configured?
- [ ] Where are credentials loaded?
- [ ] Where would OAuth token validation occur?
- [ ] Where would token refresh be triggered?

#### Configuration Touchpoints
- [ ] Config file modifications needed
- [ ] New environment variables required
- [ ] Keychain/vault integration points
- [ ] CLI argument additions needed
- [ ] Runtime config reload locations

#### Agent Creation & Spawning Logic
- [ ] Agent executor modifications needed
- [ ] Client factory pattern requirements
- [ ] Agent pool integration points
- [ ] Swarm orchestrator impact

#### Error Handling Paths
- [ ] Auth failure handling locations
- [ ] Token expiration handling points
- [ ] Refresh retry logic locations
- [ ] Fallback mechanism insertion points
- [ ] User notification touchpoints

#### Logging & Monitoring Hooks
- [ ] Auth event logging locations
- [ ] Token lifecycle logging points
- [ ] Usage metrics collection points
- [ ] Performance metrics integration
- [ ] Error metrics tracking points

### 4. Dependency Analysis

#### Anthropic SDK Usage
- What version of anthropic-sdk is used?
- What SDK features are leveraged?
- Are there direct API calls bypassing SDK?
- What SDK configuration options are used?
- Is streaming used?
- Are tools/function calling used?
- Is prompt caching used?

#### External Dependencies
List all external libraries and their purposes:
- Configuration libraries (e.g., pydantic, python-dotenv)
- HTTP clients (e.g., httpx, requests)
- Secret management (e.g., keyring)
- Logging libraries (e.g., structlog)
- CLI frameworks (e.g., click, typer)

#### Internal Module Dependencies
- Document module import graph
- Identify circular dependencies
- Note tight coupling points
- Map abstraction layers

#### Configuration File Dependencies
- Config file formats
- Schema validation
- Default value handling
- Required vs optional fields

### 5. Architectural Pattern Recognition

#### Clean Architecture Adherence
- Are domain, application, infrastructure layers separated?
- Is dependency injection used?
- Are there clear boundaries between layers?
- Is business logic isolated from infrastructure?

#### Interface Abstractions
- Are there abstract base classes for clients?
- Is there a port/adapter pattern?
- Are there protocol/interface definitions?
- Is there dependency inversion?

#### Error Handling Patterns
- Custom exception hierarchy?
- Error propagation strategy?
- Retry mechanisms?
- Circuit breaker patterns?

#### Testing Patterns
- Unit test coverage?
- Integration test patterns?
- Mock/stub usage?
- Fixture organization?

### 6. Impact Assessment

#### Components Requiring Modification
For each component, assess:
- **Modification Scope**: Minor / Moderate / Major
- **Breaking Changes**: Yes / No
- **Test Impact**: Low / Medium / High
- **Risk Level**: Low / Medium / High

**Components to Assess**:
- ClaudeClient
- ConfigManager
- AgentExecutor
- AgentPool
- CLI Main
- Logger Infrastructure
- Domain Models (if auth-related)

#### New Components to Create
Identify potential new components:
- OAuthClient or OAuthAuthenticator?
- TokenManager or TokenRefresher?
- AuthenticationFactory or AuthStrategyFactory?
- OAuthConfigValidator?
- TokenStorage abstraction?

#### Testing Requirements
- New unit tests needed
- Integration test modifications
- Mock OAuth server requirements
- Test fixture requirements
- CI/CD pipeline updates

#### Backward Compatibility Considerations
**Note**: Per DECISION_POINTS.md, breaking changes are acceptable (no current users)
- Document what would break
- Migration path if needed in future
- Version bump requirements

### 7. Code Quality & Technical Debt

#### Strengths
- Well-architected patterns
- Good separation of concerns
- Comprehensive error handling
- Strong type hints
- Good test coverage

#### Weaknesses
- Technical debt items
- Code smells
- Refactoring opportunities
- Documentation gaps
- Testing gaps

#### Refactoring Recommendations
- Pre-OAuth refactoring suggestions
- Code consolidation opportunities
- Abstraction improvements
- Test coverage enhancements

## Deliverable Format

**Output File**: `/Users/odgrim/dev/home/agentics/abathur/prd_oauth_spawning/02_current_architecture.md`

### Required Sections

1. **Executive Summary** (1 page)
   - Current architecture overview
   - Key integration points identified
   - Overall assessment
   - Critical recommendations

2. **Codebase Structure**
   - Directory tree
   - Module organization
   - Key file descriptions

3. **Current Authentication Architecture**
   - ClaudeClient deep dive
   - Configuration management analysis
   - Agent spawning workflow
   - Complete code flow diagrams

4. **Integration Point Catalog**
   - Authentication initialization points
   - Configuration touchpoints
   - Agent creation/spawning modifications
   - Error handling paths
   - Logging/monitoring hooks

5. **Dependency Analysis**
   - Anthropic SDK usage
   - External dependencies
   - Internal module dependencies
   - Configuration dependencies

6. **Architectural Patterns**
   - Clean Architecture assessment
   - Interface abstractions
   - Error handling patterns
   - Testing patterns

7. **Impact Assessment**
   - Components requiring modification (with scope/risk)
   - New components to create
   - Testing requirements
   - Backward compatibility analysis

8. **Code Quality Analysis**
   - Strengths
   - Weaknesses / Technical debt
   - Refactoring recommendations

9. **Integration Strategy Recommendations**
   - Recommended integration approach
   - Phasing suggestions
   - Risk mitigation strategies
   - Testing strategy

10. **Code Examples**
    - Current authentication pattern examples
    - Current configuration loading examples
    - Current agent spawning examples
    - Suggested abstraction examples

11. **Open Questions**
    - Unclear architectural decisions
    - Areas needing clarification
    - Assumptions made

12. **Appendix**
    - Complete file listings
    - Dependency tree
    - Import graph

## Success Criteria

- [ ] All key modules analyzed in depth
- [ ] Current authentication flow completely documented
- [ ] All integration points identified with code references
- [ ] Dependency analysis complete
- [ ] Architectural patterns documented
- [ ] Impact assessment covers all components
- [ ] Code examples demonstrate current patterns
- [ ] Integration recommendations are actionable
- [ ] Diagrams are clear and accurate
- [ ] No critical code paths left unanalyzed

## Validation Gate

After completion, the prd-project-orchestrator will:
1. Review for completeness against this specification
2. Validate accuracy of code analysis
3. Check integration points are comprehensive
4. Make validation decision: APPROVE / CONDITIONAL / REVISE / ESCALATE

## Timeline

**Target Completion**: Same session
**Estimated Effort**: 2-3 hours of code analysis and documentation
**Blocking**: oauth-research-specialist (parallel, not blocking)
**Blocked By**: None (ready to start)

## Research Methodology

### Tools to Use
- **Grep**: For targeted code searches (e.g., API key usage, auth patterns)
- **Read**: For complete file analysis
- **Glob**: For file discovery

### Analysis Approach
1. Start with key files (ClaudeClient, Config)
2. Trace authentication flow end-to-end
3. Map all integration points
4. Document with code examples
5. Create flow diagrams
6. Assess impact systematically

### Documentation Style
- Include code snippets with file paths and line numbers
- Create ASCII diagrams for flows
- Use markdown tables for structured data
- Link related sections
- Highlight critical findings

## Notes

- Focus on authentication-related code paths
- Document current patterns to inform OAuth integration
- Identify abstraction opportunities
- Note areas of technical debt
- Suggest pre-OAuth refactoring if beneficial
- Cross-reference with DECISION_POINTS.md for alignment

---

**FOR AGENT**: Read `/Users/odgrim/dev/home/agentics/abathur/prd_oauth_spawning/00_phase1_context.md` and `/Users/odgrim/dev/home/agentics/abathur/prd_oauth_spawning/DECISION_POINTS.md` before beginning analysis. Start with `src/abathur/application/claude_client.py` and `src/abathur/infrastructure/config.py`.
