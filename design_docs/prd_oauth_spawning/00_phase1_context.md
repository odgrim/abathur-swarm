# Phase 1 Context: OAuth-Based Agent Spawning PRD

**Date**: 2025-10-09
**Phase**: Phase 1 - Research & Discovery
**Orchestrator**: prd-project-orchestrator

## Project Overview

**Objective**: Create a comprehensive PRD documenting how to add OAuth-based agent spawning to Abathur alongside the existing API key approach, enabling users to leverage Claude Max subscriptions.

**Current State**:
- Abathur uses Claude Agent SDK with API key authentication (x-api-key header)
- API key approach has usage plan and rate limit constraints
- Claude Max subscriptions offer OAuth authentication with different capabilities
- Goal: Design dual-mode authentication architecture (API key + OAuth)

## Key Decisions from DECISION_POINTS.md

### Authentication Architecture
- **Primary OAuth Method**: User-provided key with auto-detection via text prefix
- **Auth Mode Detection**: Auto-detect based on key prefix (no environment variable override)
- **Token Storage**: Environment variables or system keychain
- **Token Refresh**: Automatic refresh mechanism
- **Backward Compatibility**: Breaking changes acceptable (no current users)

### Rate Limiting & Usage
- **Rate Limit Enforcement**: Ignore - let Anthropic API/Claude Code handle limit enforcement
- **Context Window**: Automatic detection with user warnings
- **Model Selection**: User-specified with validation

### Security & Testing
- **Token Lifecycle**: Automatic refresh (3 retry attempts)
- **OAuth Failure Handling**: Retry with token refresh, no automatic fallback to API key
- **Testing Strategy**: Mock OAuth for unit tests, test accounts for integration
- **Observability**: Full metrics tracking (auth events, token lifecycle, usage, performance, errors)

### Deployment
- **User Model**: Single user (no multi-tenant)
- **Packaging**: Single package with OAuth as default feature
- **Documentation**: Configuration and API reference focus

## Abathur Architecture Overview

### Key Source Files
```
src/abathur/
├── application/
│   ├── agent_executor.py         # Agent lifecycle and execution
│   ├── agent_pool.py              # Agent pool management
│   ├── claude_client.py           # Claude API client (KEY INTEGRATION POINT)
│   ├── failure_recovery.py        # Error handling and recovery
│   ├── loop_executor.py           # Loop execution logic
│   ├── mcp_manager.py             # MCP server management
│   ├── resource_monitor.py        # Resource monitoring
│   ├── swarm_orchestrator.py     # Swarm coordination
│   ├── task_coordinator.py        # Task coordination
│   └── template_manager.py        # Agent template management
├── cli/
│   └── main.py                    # CLI entry point
├── domain/
│   └── models.py                  # Domain models
└── infrastructure/
    ├── config.py                  # Configuration management (KEY INTEGRATION POINT)
    ├── database.py                # Database access
    ├── logger.py                  # Logging infrastructure
    └── mcp_config.py              # MCP configuration
```

### Critical Integration Points
1. **ClaudeClient** (`application/claude_client.py`): Current API key authentication
2. **ConfigManager** (`infrastructure/config.py`): Configuration loading and management
3. **AgentExecutor** (`application/agent_executor.py`): Agent spawning logic
4. **CLI** (`cli/main.py`): User-facing interface for configuration

## Phase 1 Research Objectives

### For oauth-research-specialist
**Task**: Comprehensive OAuth method discovery and analysis

**Deliverables Required**:
1. **OAuth Method Catalog**: All discovered methods for Claude OAuth interaction
   - Claude Code CLI subshell invocation
   - Claude Agent SDK OAuth support
   - claude_max community tool
   - MCP with OAuth
   - Any other methods discovered

2. **Method Deep Dives**: For each method document:
   - Authentication mechanism (token acquisition & usage)
   - Capabilities (operations supported)
   - Rate limits and restrictions
   - Context window size vs API key
   - Model access
   - Subscription requirements
   - Implementation examples
   - Pros/cons analysis
   - Official vs community support

3. **Comparative Analysis**:
   - Feature matrix across all methods
   - Rate limit comparison table
   - Cost analysis (subscription vs pay-per-token)
   - Context window comparison
   - Tool/MCP support availability
   - Integration complexity assessment

4. **Security Research**:
   - OAuth token lifecycle (refresh, expiration)
   - Storage best practices
   - Comparison with API key security
   - Multi-user considerations

**Output File**: `/Users/odgrim/dev/home/agentics/abathur/prd_oauth_spawning/01_oauth_research.md`

### For code-analysis-specialist
**Task**: Analyze current Abathur implementation for OAuth integration

**Deliverables Required**:
1. **Current Architecture Analysis**:
   - ClaudeClient implementation review
   - Current API key authentication pattern
   - Agent spawning workflow documentation
   - Configuration loading mechanism
   - Error handling patterns

2. **Integration Point Identification**:
   - Authentication initialization locations
   - Configuration touchpoints for OAuth
   - Agent creation/spawning logic modifications needed
   - Error handling paths for OAuth failures
   - Logging/monitoring hooks

3. **Dependency Assessment**:
   - Anthropic SDK usage patterns
   - External library dependencies
   - Internal module dependencies
   - Configuration file structure

4. **Impact Analysis**:
   - Components requiring modification
   - New components to create
   - Testing requirements
   - Backward compatibility considerations (Note: breaking changes acceptable)

5. **Pattern Recognition**:
   - Clean Architecture adherence
   - Dependency injection usage
   - Interface abstractions
   - Current testing patterns

**Output File**: `/Users/odgrim/dev/home/agentics/abathur/prd_oauth_spawning/02_current_architecture.md`

## Success Criteria for Phase 1

- [ ] OAuth research covers ALL interaction methods comprehensively
- [ ] Each OAuth method has detailed pros/cons analysis
- [ ] Comparative analysis provides clear decision-making data
- [ ] Architecture analysis identifies ALL integration points in Abathur
- [ ] Current authentication patterns are fully documented
- [ ] Impact assessment is complete and accurate
- [ ] Code examples demonstrate current patterns
- [ ] Both deliverables are clear, detailed, and actionable

## Phase 1 Validation Gate

After both agents complete, the orchestrator will:
1. Review deliverables for completeness and accuracy
2. Validate findings against DECISION_POINTS.md
3. Make validation decision (APPROVE/CONDITIONAL/REVISE/ESCALATE)
4. Generate Phase 2 context summary with key findings

## Next Steps After Phase 1

**Phase 2 Focus**: Technical requirements and architecture design
- Technical requirements analyst will define functional/non-functional requirements
- System architect will design dual-mode authentication architecture
- Both will leverage Phase 1 research findings

---

**Orchestrator Notes**:
- Both agents should work in parallel (no dependencies)
- oauth-research-specialist should use WebSearch for latest 2025 information
- code-analysis-specialist should use Grep/Read for thorough code analysis
- Both should document sources and flag uncertainties
- Expected timeline: Phase 1 completion within this session
