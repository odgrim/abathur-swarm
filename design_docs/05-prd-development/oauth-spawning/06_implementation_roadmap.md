# Implementation Roadmap - OAuth-Based Agent Spawning

**Date**: October 9, 2025
**Phase**: Phase 3 - Implementation Roadmap
**Agent**: prd-implementation-roadmap-specialist
**Project**: Abathur OAuth Integration
**Version**: 1.0

---

## 1. Executive Summary

### 1.1 Overview

This document defines a comprehensive 4-week implementation roadmap for adding OAuth-based agent spawning to Abathur. The implementation follows a phased approach with clear milestones, risk mitigation strategies, and quality gates.

**Timeline**: 4 weeks (110 developer hours)
**Team Size**: 1 full-time developer
**Scope**: ~600 LOC implementation + 600 LOC tests
**Architecture**: Zero breaking changes, Clean Architecture preserved

### 1.2 Implementation Scope Summary

**New Files** (4):
- `domain/ports/auth_provider.py` (~50 LOC) - AuthProvider interface
- `infrastructure/api_key_auth.py` (~30 LOC) - API key implementation
- `infrastructure/oauth_auth.py` (~200 LOC) - OAuth implementation with token refresh
- `infrastructure/exceptions.py` (~50 LOC) - Custom exception hierarchy

**Modified Files** (3):
- `application/claude_client.py` (~150 LOC modified) - AuthProvider DI, 401 retry logic
- `infrastructure/config.py` (~140 LOC added) - OAuth token methods, auto-detection
- `cli/main.py` (~190 LOC added) - OAuth commands, service initialization

**Test Files** (~600 LOC):
- Unit tests: ~400 LOC (AuthProvider, OAuth, Config, ClaudeClient)
- Integration tests: ~200 LOC (OAuth flow, token refresh, context window)

### 1.3 Critical Path

```
Week 1: Foundation → Week 2: OAuth Core → Week 3: CLI Integration → Week 4: Testing & Polish
```

**Key Dependencies**:
- AuthProvider abstraction (Week 1) blocks OAuth implementation (Week 2)
- OAuth implementation (Week 2) blocks ClaudeClient integration (Week 3)
- All features (Weeks 1-3) block comprehensive testing (Week 4)

### 1.4 Success Criteria

**Quality Gates**:
- ✅ Test coverage ≥90% (unit + integration)
- ✅ Zero breaking changes to existing API key workflows
- ✅ All 30 functional requirements met
- ✅ All 31 non-functional requirements met
- ✅ Security validation complete (no token exposure)

**Timeline Targets**:
- ✅ Week 1 checkpoint: AuthProvider abstraction complete
- ✅ Week 2 checkpoint: OAuth token lifecycle working
- ✅ Week 3 checkpoint: End-to-end OAuth flow operational
- ✅ Week 4 checkpoint: Production-ready with documentation

---

## 2. Phased Implementation Plan

### Phase 1 (Week 1): Foundation - AuthProvider Abstraction

**Objective**: Build authentication abstraction layer with zero breaking changes to existing API key workflows.

**Milestone**: API key authentication refactored to use AuthProvider interface while maintaining 100% backward compatibility.

#### Deliverables

1. **AuthProvider Interface** (`domain/ports/auth_provider.py`)
   - 5 abstract methods: `get_credentials()`, `refresh_credentials()`, `is_valid()`, `get_auth_method()`, `get_context_limit()`
   - Complete docstrings with type hints
   - Interface contracts clearly defined
   - **LOC**: ~50

2. **APIKeyAuthProvider Implementation** (`infrastructure/api_key_auth.py`)
   - Wraps existing API key authentication logic
   - Validates API key format (sk-ant-api prefix)
   - Returns 1M token context limit
   - **LOC**: ~30

3. **Custom Exception Hierarchy** (`infrastructure/exceptions.py`)
   - `AbathurError` base class
   - `AuthenticationError` with remediation field
   - `OAuthTokenExpiredError`, `OAuthRefreshError`, `APIKeyInvalidError`
   - `ContextWindowExceededError` with token tracking
   - **LOC**: ~50

4. **ClaudeClient Constructor Refactoring**
   - Add optional `auth_provider` parameter
   - Preserve existing `api_key` parameter (backward compatibility)
   - Auto-initialize APIKeyAuthProvider if api_key provided
   - **LOC Modified**: ~25 lines in `application/claude_client.py`

5. **Unit Tests - Phase 1**
   - `test_auth_provider.py`: Interface contract tests
   - `test_api_key_auth.py`: API key provider validation, context limit
   - `test_exceptions.py`: Exception message templates, remediation steps
   - **Test Coverage Target**: ≥90% for new code
   - **LOC**: ~100

#### Success Criteria

- [ ] AuthProvider interface defined with 5 methods
- [ ] APIKeyAuthProvider wraps existing API key logic (no functional changes)
- [ ] All existing API key tests pass (100% backward compatibility)
- [ ] Unit test coverage ≥90% for new components
- [ ] ClaudeClient constructor accepts both `api_key` and `auth_provider` parameters
- [ ] Existing agent templates work without modification

#### Dependencies

**None** - Foundation work with no external blockers

#### Risks & Mitigation

**Risk**: Refactoring ClaudeClient constructor breaks existing code
- **Likelihood**: Low
- **Impact**: High
- **Mitigation**: Make `auth_provider` parameter optional, preserve `api_key` parameter
- **Testing**: Run full regression test suite before proceeding to Week 2

#### Time Allocation

- **Design & Planning**: 4 hours
- **Implementation**: 12 hours
- **Testing**: 6 hours
- **Code Review & Fixes**: 3 hours
- **Total**: 25 hours

---

### Phase 2 (Week 2): OAuth Core - Token Lifecycle Implementation

**Objective**: Implement complete OAuth token lifecycle with automatic refresh, secure storage, and expiry detection.

**Milestone**: OAuth authentication fully functional with proactive and reactive token refresh working.

#### Deliverables

1. **OAuthAuthProvider Implementation** (`infrastructure/oauth_auth.py`)
   - Proactive refresh: 5-minute buffer before expiry
   - Reactive refresh: 401 retry loop with 3 attempts
   - Token rotation handling (server may return new refresh_token)
   - Expiry calculation with UTC timezone handling
   - Token refresh endpoint integration (`https://console.anthropic.com/v1/oauth/token`)
   - **LOC**: ~200

2. **ConfigManager OAuth Methods** (`infrastructure/config.py`)
   - `get_oauth_token()`: Retrieve from keychain/env/.env (priority order)
   - `set_oauth_token()`: Store in OS keychain or .env file
   - `detect_auth_method()`: Auto-detect from credential prefix
   - `clear_oauth_tokens()`: Cleanup on logout (keychain + env vars + .env file)
   - **LOC Added**: ~140

3. **AuthConfig Model** (`infrastructure/config.py`)
   - Configuration fields: `mode`, `oauth_token_storage`, `auto_refresh`, `refresh_retries`, `context_window_handling`
   - Pydantic validation with sensible defaults
   - Cross-field validation for OAuth config
   - **LOC Added**: ~20 (included in ConfigManager LOC above)

4. **Unit Tests - Phase 2**
   - `test_oauth_auth.py`: Token refresh success/failure, expiry detection, proactive/reactive refresh, concurrent refresh (mutex verification)
   - `test_config_oauth.py`: Token retrieval priority, storage (keychain/env/.env), auth method detection, token cleanup
   - Mock token refresh endpoint with `responses` library
   - Mock OS keychain with `unittest.mock`
   - **Test Coverage Target**: ≥90% for OAuth code
   - **LOC**: ~250

#### Success Criteria

- [ ] OAuthAuthProvider implements token refresh with 3-retry logic
- [ ] Proactive refresh (5-min buffer) and reactive refresh (401) working
- [ ] Token storage uses OS keychain (macOS/Linux verified)
- [ ] Token expiry calculation correct (UTC timezone, ISO 8601 format)
- [ ] Token rotation handled (new refresh_token persisted if returned)
- [ ] Unit test coverage ≥90% for OAuth code
- [ ] All OAuth methods logged with structured logging

#### Dependencies

**Required**: Phase 1 (AuthProvider interface)

#### Risks & Mitigation

**Risk 1: Token refresh endpoint changes format**
- **Likelihood**: Low (widely used in Claude Code)
- **Impact**: High (all token refreshes fail)
- **Mitigation**: 3-retry logic with fallback to manual re-auth, monitoring
- **Contingency**: Rapid update if endpoint changes, manual re-auth workflow tested

**Risk 2: OS keychain unavailable on Linux systems**
- **Likelihood**: Medium
- **Impact**: Low (fallback to .env file)
- **Mitigation**: Graceful fallback with user warning, test on Linux environments
- **Contingency**: Document .env file fallback, recommend keychain setup

**Risk 3: Token refresh race conditions (concurrent requests)**
- **Likelihood**: Medium
- **Impact**: Low (redundant refreshes, not failures)
- **Mitigation**: Mutex/lock around `refresh_credentials()`, load testing
- **Contingency**: Rate limiting at refresh endpoint, exponential backoff

**Risk 4: Insecure .env fallback exposes tokens**
- **Likelihood**: Medium (when keychain unavailable)
- **Impact**: Medium (file permissions mitigate)
- **Mitigation**: User warning, recommend keychain, .gitignore enforcement, permission checks
- **Contingency**: Document security best practices, consider encrypted file alternative

#### Time Allocation

- **Design & Planning**: 3 hours
- **OAuth Implementation**: 16 hours
- **ConfigManager Integration**: 8 hours
- **Testing**: 8 hours
- **Code Review & Fixes**: 3 hours
- **Total**: 38 hours

---

### Phase 3 (Week 3): CLI Integration - End-to-End OAuth Flow

**Objective**: Integrate OAuth authentication into ClaudeClient and CLI with full end-to-end workflow support.

**Milestone**: Users can oauth-login → spawn tasks → automatic token refresh → oauth-logout with zero manual intervention.

#### Deliverables

1. **ClaudeClient OAuth Integration** (`application/claude_client.py`)
   - Modify `execute_task()`: 401 retry loop, context validation, token counting
   - Add `_configure_sdk_auth()`: Set `ANTHROPIC_AUTH_TOKEN` or `ANTHROPIC_API_KEY` env var dynamically
   - Add `_estimate_tokens()`: 4-char approximation for context window validation
   - Context window warnings at 90% threshold (OAuth: 180K, API key: 900K)
   - **LOC Modified**: ~150

2. **CLI Service Initialization** (`cli/main.py`)
   - Modify `_get_services()`: Detect auth method, initialize AuthProvider
   - Priority order: API key env var → OAuth keychain → OAuth env → Error
   - **LOC Modified**: ~40

3. **CLI OAuth Commands** (`cli/main.py`)
   - `config oauth-login --manual`: Manual token input, store in keychain
   - `config oauth-logout`: Clear all stored tokens
   - `config oauth-status`: Display auth method, expiry, context limit
   - `config oauth-refresh`: Manually trigger token refresh
   - Rich terminal output with tables and status indicators
   - **LOC Added**: ~150

4. **Integration Tests - Phase 3**
   - `test_oauth_flow.py`: oauth-login → task execution → oauth-logout
   - `test_token_refresh_integration.py`: Token expiry during task, concurrent tasks with refresh
   - `test_context_window_integration.py`: Large input warnings, block mode
   - Mock OAuth server (Flask app) for testing
   - Temporary keychain/env file setup
   - **Test Coverage Target**: ≥70% end-to-end scenarios
   - **LOC**: ~200

#### Success Criteria

- [ ] `oauth-login` command stores tokens in keychain
- [ ] ClaudeClient auto-refreshes tokens on 401 Unauthorized
- [ ] Context window warnings triggered at 90% threshold (180K for OAuth, 900K for API key)
- [ ] `oauth-status` displays auth method, expiry timestamp, context limit
- [ ] Integration tests verify end-to-end OAuth flow
- [ ] All CLI commands have rich terminal output with clear error messages

#### Dependencies

**Required**: Phase 2 (OAuthAuthProvider, ConfigManager OAuth methods)

#### Risks & Mitigation

**Risk 1: SDK concurrent requests (race conditions)**
- **Likelihood**: Medium
- **Impact**: Medium (task failures under load)
- **Mitigation**: Mutex/lock around `_configure_sdk_auth()`, load testing with 10-100 concurrent tasks
- **Contingency**: Serialize SDK initialization, document concurrency limitations

**Risk 2: Context estimation inaccuracy for code-heavy tasks**
- **Likelihood**: Low
- **Impact**: Low (conservative bias acceptable)
- **Mitigation**: 4-char approximation (underestimates code), configurable handling modes (warn/block/ignore)
- **Contingency**: User feedback loop, adjust threshold based on real usage

**Risk 3: Token refresh during long-running tasks (>1 hour)**
- **Likelihood**: Medium
- **Impact**: Low (automatic refresh handles this)
- **Mitigation**: Proactive refresh every 55 minutes, 401 retry logic
- **Contingency**: Document expected behavior, test with long-running tasks

#### Time Allocation

- **Design & Planning**: 2 hours
- **ClaudeClient Integration**: 10 hours
- **CLI Commands**: 8 hours
- **Integration Testing**: 10 hours
- **Code Review & Fixes**: 4 hours
- **Total**: 34 hours

---

### Phase 4 (Week 4): Testing & Documentation - Production Readiness

**Objective**: Achieve production-ready quality with comprehensive testing, security validation, and user-facing documentation.

**Milestone**: All quality gates passed, documentation complete, deployment checklist verified.

#### Deliverables

1. **Comprehensive Unit Tests**
   - Expand existing tests to 400 LOC total
   - Edge case coverage: Invalid tokens, network failures, clock skew
   - Performance tests: Token refresh <100ms, context validation <50ms
   - **LOC**: ~150 (additional to previous phases)

2. **Security Testing**
   - Log sanitization tests: Scan logs for token patterns (regex: `sk-ant-api`, `Bearer`)
   - Error message sanitization: Trigger all error paths, verify no credentials
   - HTTPS enforcement: Monitor network traffic with `mitmproxy` or similar
   - Token cleanup tests: Verify keychain/env/.env cleared on logout
   - Automated security scanning: Bandit (static analysis), Safety (dependency vulnerabilities)
   - **Test Scenarios**: ≥6 security tests

3. **Load Testing**
   - Concurrent OAuth requests: 10-100 tasks, verify no race conditions
   - Token refresh under load: Expire token with 50 tasks running
   - Success rate target: ≥99% task completion
   - Tools: `pytest-xdist` for parallel execution, `Locust` for load generation
   - **Test Scenarios**: 2 load tests

4. **End-to-End Testing** (Manual)
   - Interactive OAuth login (if browser-based flow implemented)
   - Keychain storage on macOS and Linux (gnome-keyring)
   - .env file fallback when keychain unavailable
   - Token refresh during long-running task (>1 hour)
   - Context window warning for large inputs
   - **Test Checklist**: 7 manual scenarios

5. **Documentation**
   - **Migration Guide**: API key users → OAuth (step-by-step)
   - **OAuth Setup Guide**: Manual token input, browser OAuth (future)
   - **Troubleshooting Guide**: Token refresh failures, context warnings, rate limits
   - **API Reference Updates**: ClaudeClient with AuthProvider examples
   - **Configuration Reference**: AuthConfig options, precedence order
   - **LOC (Markdown)**: ~500 lines across 5 documents

#### Success Criteria

- [ ] Test coverage ≥90% overall (unit + integration)
- [ ] All security tests pass (no token exposure in logs/errors)
- [ ] Load tests: 100 concurrent requests succeed with ≥99% success rate
- [ ] Documentation complete (≥3 user guides)
- [ ] Backward compatibility validated (all existing API key tests pass)
- [ ] Deployment checklist reviewed and approved
- [ ] Code review completed with zero critical issues

#### Dependencies

**Required**: Phase 3 (CLI integration complete)

#### Risks & Mitigation

**Risk 1: Late-stage bugs discovered during integration testing**
- **Likelihood**: Medium
- **Impact**: Medium (timeline delay)
- **Mitigation**: Daily testing throughout Phases 1-3, early bug detection
- **Contingency**: 20-hour buffer built into overall timeline (160 total - 110 planned = 50-hour buffer)

**Risk 2: Documentation incomplete or unclear**
- **Likelihood**: Low
- **Impact**: Medium (user adoption delayed)
- **Mitigation**: Documentation in parallel with implementation, user feedback on drafts
- **Contingency**: Post-release documentation updates based on user feedback

**Risk 3: Security testing uncovers token exposure**
- **Likelihood**: Low (automated scanning mitigates)
- **Impact**: High (security incident)
- **Mitigation**: Log sanitization from day 1, automated scanning in CI/CD
- **Contingency**: Emergency patch process, security advisory if needed

#### Time Allocation

- **Additional Unit Tests**: 4 hours
- **Security Testing**: 6 hours
- **Load Testing**: 4 hours
- **E2E Testing (Manual)**: 4 hours
- **Documentation**: 8 hours
- **Code Review & Final Fixes**: 3 hours
- **Deployment Prep**: 2 hours
- **Total**: 31 hours

---

## 3. Task Breakdown by Phase

### Week 1: Foundation (25 hours)

| Task ID | Description | Hours | Owner | Dependencies |
|---------|-------------|-------|-------|--------------|
| W1-T1 | Design AuthProvider interface | 2 | Developer | None |
| W1-T2 | Implement AuthProvider interface | 3 | Developer | W1-T1 |
| W1-T3 | Implement APIKeyAuthProvider | 3 | Developer | W1-T2 |
| W1-T4 | Implement custom exception hierarchy | 2 | Developer | None |
| W1-T5 | Refactor ClaudeClient constructor | 4 | Developer | W1-T2, W1-T3 |
| W1-T6 | Write unit tests (auth_provider, api_key_auth, exceptions) | 6 | Developer | W1-T2, W1-T3, W1-T4 |
| W1-T7 | Run regression tests (API key workflows) | 2 | Developer | W1-T5 |
| W1-T8 | Code review and fixes | 3 | Developer | W1-T6, W1-T7 |

**Week 1 Checkpoint**: AuthProvider abstraction complete, all existing tests pass.

---

### Week 2: OAuth Core (38 hours)

| Task ID | Description | Hours | Owner | Dependencies |
|---------|-------------|-------|-------|--------------|
| W2-T1 | Design OAuth token lifecycle (proactive/reactive refresh) | 2 | Developer | Phase 1 |
| W2-T2 | Implement OAuthAuthProvider (token refresh logic) | 10 | Developer | W2-T1 |
| W2-T3 | Implement ConfigManager OAuth methods (get/set/clear) | 6 | Developer | W2-T2 |
| W2-T4 | Add AuthConfig model to Config | 2 | Developer | W2-T3 |
| W2-T5 | Mock token refresh endpoint for tests | 2 | Developer | W2-T2 |
| W2-T6 | Write unit tests (oauth_auth, config_oauth) | 8 | Developer | W2-T2, W2-T3, W2-T5 |
| W2-T7 | Test on macOS and Linux (keychain storage) | 2 | Developer | W2-T3, W2-T6 |
| W2-T8 | Test concurrent token refresh (mutex/lock) | 3 | Developer | W2-T2, W2-T6 |
| W2-T9 | Code review and fixes | 3 | Developer | W2-T6, W2-T7, W2-T8 |

**Week 2 Checkpoint**: OAuth token lifecycle working, token refresh tested.

---

### Week 3: CLI Integration (34 hours)

| Task ID | Description | Hours | Owner | Dependencies |
|---------|-------------|-------|-------|--------------|
| W3-T1 | Design ClaudeClient 401 retry loop | 1 | Developer | Phase 2 |
| W3-T2 | Implement ClaudeClient execute_task() modifications | 6 | Developer | W3-T1 |
| W3-T3 | Implement _configure_sdk_auth() and _estimate_tokens() | 3 | Developer | W3-T2 |
| W3-T4 | Modify CLI _get_services() (auth detection, provider init) | 3 | Developer | Phase 2 |
| W3-T5 | Implement oauth-login command | 3 | Developer | Phase 2 |
| W3-T6 | Implement oauth-logout, oauth-status, oauth-refresh commands | 3 | Developer | W3-T5 |
| W3-T7 | Rich terminal output (tables, status indicators) | 2 | Developer | W3-T6 |
| W3-T8 | Write integration tests (oauth_flow, token_refresh_integration) | 8 | Developer | W3-T2, W3-T6 |
| W3-T9 | Write context window integration tests | 2 | Developer | W3-T3 |
| W3-T10 | Code review and fixes | 3 | Developer | W3-T8, W3-T9 |

**Week 3 Checkpoint**: End-to-end OAuth flow operational via CLI.

---

### Week 4: Testing & Documentation (31 hours)

| Task ID | Description | Hours | Owner | Dependencies |
|---------|-------------|-------|-------|--------------|
| W4-T1 | Expand unit tests (edge cases, performance) | 4 | Developer | Phase 3 |
| W4-T2 | Implement security tests (log/error sanitization, HTTPS) | 6 | Developer | Phase 3 |
| W4-T3 | Implement load tests (concurrent tasks, token refresh) | 4 | Developer | Phase 3 |
| W4-T4 | Conduct manual E2E testing (keychain, browser flow, etc.) | 4 | Developer | Phase 3 |
| W4-T5 | Write migration guide (API key → OAuth) | 2 | Developer | None |
| W4-T6 | Write OAuth setup guide (manual + browser flow) | 2 | Developer | None |
| W4-T7 | Write troubleshooting guide (common errors, resolutions) | 2 | Developer | W4-T2 |
| W4-T8 | Update API reference (ClaudeClient with AuthProvider) | 1 | Developer | None |
| W4-T9 | Update configuration reference (AuthConfig options) | 1 | Developer | None |
| W4-T10 | Code review and final fixes | 3 | Developer | W4-T1, W4-T2, W4-T3 |
| W4-T11 | Deployment preparation (checklist, release notes) | 2 | Developer | W4-T5, W4-T6, W4-T7 |

**Week 4 Checkpoint**: Production-ready, all tests pass, documentation complete.

---

## 4. Dependency Graph & Critical Path

### 4.1 Critical Path Analysis

```
┌─────────────────────────────────────────────────────────────┐
│                     CRITICAL PATH                            │
│                                                              │
│  Phase 1     Phase 2      Phase 3       Phase 4             │
│  (Week 1) → (Week 2)  →  (Week 3)   →  (Week 4)            │
│                                                              │
│  Auth      OAuth Core   CLI          Testing &              │
│  Provider  Token        Integration  Documentation          │
│  Abstract  Lifecycle                                        │
└─────────────────────────────────────────────────────────────┘

Key Blockers:
1. AuthProvider interface (W1-T2) blocks OAuth implementation (W2-T2)
2. OAuth token methods (W2-T3) block CLI integration (W3-T4, W3-T5)
3. ClaudeClient integration (W3-T2) blocks integration testing (W3-T8)
4. All features (W1-W3) block comprehensive testing (W4-T1 through W4-T4)
```

### 4.2 Parallel Opportunities

**Week 1 Parallel Work**:
- Exception hierarchy (W1-T4) can run parallel with AuthProvider design (W1-T1, W1-T2)

**Week 2 Parallel Work**:
- AuthConfig model (W2-T4) can run parallel with OAuth implementation (W2-T2)
- Mock endpoint setup (W2-T5) can run parallel with ConfigManager work (W2-T3)

**Week 3 Parallel Work**:
- CLI commands (W3-T5, W3-T6) can run parallel with ClaudeClient work (W3-T2, W3-T3)
- Documentation drafts (W4-T5, W4-T6, W4-T7) can start in Week 3

**Week 4 Parallel Work**:
- Security testing (W4-T2) can run parallel with load testing (W4-T3)
- Documentation (W4-T5 through W4-T9) can run parallel with testing (W4-T1, W4-T2)

### 4.3 Dependency Matrix

| Task | Depends On | Blocks |
|------|------------|--------|
| W1-T2 (AuthProvider interface) | W1-T1 | W1-T3, W1-T5, W2-T2 |
| W1-T3 (APIKeyAuthProvider) | W1-T2 | W1-T5, W1-T6 |
| W2-T2 (OAuthAuthProvider) | W1-T2, W2-T1 | W2-T3, W2-T6, W3-T2 |
| W2-T3 (ConfigManager OAuth) | W2-T2 | W3-T4, W3-T5, W3-T8 |
| W3-T2 (ClaudeClient integration) | W2-T2, W3-T1 | W3-T8, W4-T1 |
| W3-T5 (oauth-login command) | W2-T3 | W3-T8, W4-T4 |
| W4-T2 (Security testing) | W3-T2, W3-T5 | W4-T7, W4-T11 |

---

## 5. Risk Assessment

### 5.1 Technical Risks

#### RISK-TECH-001: SDK Concurrent Requests (Race Conditions)

**Description**: Setting `ANTHROPIC_AUTH_TOKEN` environment variable may have race conditions when multiple tasks execute concurrently, causing environment variable overwrites or stale token usage.

**Likelihood**: Medium
- Environment variables are process-level, not thread-safe
- Concurrent task execution is a core Abathur feature (SwarmOrchestrator)

**Impact**: Medium
- Task failures under load (intermittent 401 errors)
- User experience degradation
- Not a data corruption or security issue

**Risk Level**: MEDIUM (Medium × Medium)

**Mitigation Strategy**:
1. **Mutex/Lock**: Wrap `_configure_sdk_auth()` with threading lock
   ```python
   import threading
   _sdk_auth_lock = threading.Lock()

   async def _configure_sdk_auth(self):
       with _sdk_auth_lock:
           # Set env var and reinitialize SDK
   ```
2. **Load Testing**: Test with 10-100 concurrent tasks in Phase 4 (W4-T3)
3. **Monitoring**: Log all SDK auth configuration events with task context

**Mitigation Phase**: Phase 3 (W3-T2 implementation)

**Residual Risk**: Low - Lock prevents race conditions; load testing validates

**Contingency Plan**: If race conditions persist, serialize SDK initialization at SwarmOrchestrator level

---

#### RISK-TECH-002: Token Refresh Race Conditions

**Description**: Multiple concurrent tasks may trigger token refresh simultaneously if all detect expiration, causing redundant refresh requests and potential token invalidation.

**Likelihood**: Medium
- Proactive refresh (5-min buffer) reduces likelihood
- Concurrent task execution common in swarm mode

**Impact**: Low
- Inefficiency (redundant API calls to refresh endpoint)
- No functional failures (duplicate refreshes return valid tokens)

**Risk Level**: LOW (Medium × Low)

**Mitigation Strategy**:
1. **Mutex/Lock**: Wrap `refresh_credentials()` with async lock
   ```python
   import asyncio
   _refresh_lock = asyncio.Lock()

   async def refresh_credentials(self):
       async with _refresh_lock:
           # Check if another task already refreshed
           if not self._is_expired():
               return True  # Already refreshed
           # Proceed with refresh
   ```
2. **Double-Check Locking**: Check expiry again inside lock
3. **Load Testing**: Test with concurrent tasks triggering refresh (W4-T3)

**Mitigation Phase**: Phase 2 (W2-T2 implementation)

**Residual Risk**: Very Low - Lock and double-check eliminate redundant refreshes

**Contingency Plan**: Monitor refresh endpoint rate limits; add exponential backoff if hit

---

#### RISK-TECH-003: Context Window Estimation Inaccuracy

**Description**: 4-character approximation may underestimate token count for code-heavy tasks, causing tasks to exceed context window despite warnings not triggering.

**Likelihood**: Low
- Code is ~3 chars/token (more conservative than English ~4 chars/token)
- 90% warning threshold provides buffer

**Impact**: Low
- User receives API error if actual tokens exceed limit
- Conservative bias means false positives (warnings when under limit) not false negatives

**Risk Level**: LOW (Low × Low)

**Mitigation Strategy**:
1. **Conservative Threshold**: Use 90% threshold instead of 95% to account for estimation variance
2. **Configurable Handling**: Support `warn`, `block`, `ignore` modes (user choice)
3. **User Feedback**: Log actual vs estimated tokens for calibration
4. **Documentation**: Clearly document approximation nature and conservativeness

**Mitigation Phase**: Phase 3 (W3-T3 implementation)

**Residual Risk**: Very Low - Acceptable for warning system (not billing)

**Contingency Plan**: If user feedback indicates high false positive rate, adjust threshold to 85% or improve estimation

---

### 5.2 Security Risks

#### RISK-SEC-001: Token Exposure in Logs

**Description**: Accidental logging of OAuth tokens or API keys in plaintext logs, creating credential compromise risk.

**Likelihood**: Low
- Automated scanning mitigates (TruffleHog, regex patterns)
- Structured logging with exclusion lists

**Impact**: High
- Credential compromise
- Unauthorized API access
- Potential data breach

**Risk Level**: MEDIUM (Low × High)

**Mitigation Strategy**:
1. **Log Sanitization Rules**: Never log token values; log only metadata (expiry, auth method)
   ```python
   # GOOD
   logger.info("token_refreshed", expires_at=expires_at.isoformat(), auth_method="oauth")

   # BAD
   logger.info("token_refreshed", access_token=access_token)  # NEVER DO THIS
   ```
2. **Automated Scanning**: Run TruffleHog and regex scans in CI/CD
3. **Security Tests**: Scan logs for patterns `sk-ant-api`, `Bearer`, token-like strings (W4-T2)
4. **Code Review**: Explicit checklist item for token logging

**Mitigation Phase**: Phase 4 (W4-T2 security testing)

**Residual Risk**: Very Low - Automated scanning and manual review

**Contingency Plan**: If token exposure detected, rotate credentials immediately, security advisory

---

#### RISK-SEC-002: Insecure .env Fallback

**Description**: When OS keychain unavailable, tokens stored in plaintext .env file without encryption, creating file access risk.

**Likelihood**: Medium
- Common on Linux systems without gnome-keyring
- Docker containers may not have keychain

**Impact**: Medium
- File permissions (600) mitigate
- .gitignore prevents version control exposure
- Still plaintext on disk

**Risk Level**: MEDIUM (Medium × Medium)

**Mitigation Strategy**:
1. **User Warning**: Display warning when falling back to .env storage
   ```
   ⚠️  OS keychain unavailable. Storing tokens in .env file (less secure).
       Recommend: Install gnome-keyring or use environment variables.
   ```
2. **File Permissions**: Verify .env file is 600 (user-only read/write)
3. **Gitignore Enforcement**: Check .gitignore includes .env, warn if missing
4. **Documentation**: Document keychain setup for macOS/Linux

**Mitigation Phase**: Phase 2 (W2-T3 implementation)

**Residual Risk**: Low - Acceptable for fallback with warnings

**Contingency Plan**: Consider encrypted file storage alternative (e.g., `cryptography` library)

---

#### RISK-SEC-003: Token Refresh Endpoint Changes

**Description**: Anthropic changes token refresh endpoint URL, request format, or response format without notice, breaking all token refreshes.

**Likelihood**: Low
- Endpoint widely used in Claude Code (official tool)
- Breaking changes unlikely without migration period

**Impact**: High
- All OAuth token refreshes fail
- Users cannot continue work without manual re-authentication

**Risk Level**: MEDIUM (Low × High)

**Mitigation Strategy**:
1. **3-Retry Logic**: Retry transient failures with exponential backoff
2. **Fallback to Manual Re-auth**: Clear error message directing user to `oauth-login`
3. **Monitoring**: Track refresh success rate (alert if <95%)
4. **Community Engagement**: Monitor Claude Code releases for endpoint changes
5. **Graceful Degradation**: If refresh fails, prompt re-auth instead of crashing

**Mitigation Phase**: Phase 2 (W2-T2 implementation)

**Residual Risk**: Low - Manual re-auth workflow tested

**Contingency Plan**: Rapid update if endpoint changes; document manual re-auth prominently

---

### 5.3 Operational Risks

#### RISK-OPS-001: Token Refresh Endpoint Availability

**Description**: Service downtime or rate limiting on token refresh endpoint prevents token refreshes.

**Likelihood**: Low
- Anthropic infrastructure generally reliable
- No SLA for refresh endpoint

**Impact**: Medium
- Users cannot refresh tokens during outage
- Manual re-authentication required
- Work interruption

**Risk Level**: LOW (Low × Medium)

**Mitigation Strategy**:
1. **3-Retry Logic**: Retry with exponential backoff (1s, 2s, 4s)
2. **Respect Retry-After**: Honor 429 `Retry-After` header
3. **Fallback to Manual**: Clear error message with re-auth instructions
4. **Monitoring**: Alert if refresh failures spike (>5% rate)

**Mitigation Phase**: Phase 2 (W2-T2 implementation)

**Residual Risk**: Low - Acceptable with manual fallback

**Contingency Plan**: Document manual re-auth workflow, communicate outage to users

---

#### RISK-OPS-002: OS Keychain Unavailable

**Description**: Keychain access denied or service unavailable (especially on Linux systems), preventing secure token storage.

**Likelihood**: Medium
- Linux systems may not have gnome-keyring installed
- Docker containers often lack keychain

**Impact**: Low
- Fallback to .env file works (less secure but functional)
- User warned about security implications

**Risk Level**: LOW (Medium × Low)

**Mitigation Strategy**:
1. **Graceful Fallback**: Automatically fall back to .env file with warning
2. **Installation Guide**: Document keychain setup for Linux (gnome-keyring)
3. **Docker Docs**: Document environment variable approach for containers
4. **Test on Linux**: Verify fallback works on Ubuntu, Fedora (W2-T7)

**Mitigation Phase**: Phase 2 (W2-T3 implementation)

**Residual Risk**: Very Low - Fallback tested and documented

**Contingency Plan**: Recommend environment variables for production deployments

---

#### RISK-OPS-003: Rate Limiting for OAuth Users

**Description**: OAuth users hit 50-200 prompt limits unexpectedly, causing task failures.

**Likelihood**: Medium
- Max 5x users have 50-200 prompts/5h limit (confirmed in research)
- Heavy users may hit limit during large swarms

**Impact**: Low
- Graceful error handling (429 responses)
- Clear error messages
- No data loss

**Risk Level**: LOW (Medium × Low)

**Mitigation Strategy**:
1. **Usage Tracking**: Track prompts used (deferred to post-MVP, complexity)
2. **Warnings at 80%**: Alert user when approaching limit (deferred to post-MVP)
3. **Clear 429 Error Messages**: "Rate limit exceeded: 50/50 prompts used. Window resets in 2h 15m."
4. **Documentation**: Document OAuth rate limits prominently

**Mitigation Phase**: Post-MVP (not in initial 4-week scope)

**Residual Risk**: Low - Clear error messages guide user

**Contingency Plan**: Add usage tracking in v0.3.0 if user feedback indicates need

---

### 5.4 Migration Risks

#### RISK-MIG-001: Backward Compatibility Failures

**Description**: Existing API key workflows break due to refactoring, affecting all current users.

**Likelihood**: Low
- Changes well-isolated (AuthProvider abstraction)
- Comprehensive regression testing

**Impact**: High
- Existing users affected
- Trust damage
- Rollback required

**Risk Level**: MEDIUM (Low × High)

**Mitigation Strategy**:
1. **100% Test Coverage**: All existing API key scenarios tested (W1-T7)
2. **Optional AuthProvider**: Make parameter optional, preserve `api_key` parameter
3. **Regression Tests**: Run full test suite before each phase gate
4. **Canary Deployment**: Test with subset of users before full rollout
5. **Rollback Plan**: Document rollback procedure to v0.1.x

**Mitigation Phase**: Phase 1 (W1-T7 regression testing)

**Residual Risk**: Very Low - Comprehensive testing and rollback plan

**Contingency Plan**: Immediate rollback if critical bug detected; hotfix within 24 hours

---

### 5.5 Risk Summary Table

| Risk ID | Category | Likelihood | Impact | Risk Level | Mitigation Phase | Residual Risk |
|---------|----------|------------|--------|------------|------------------|---------------|
| RISK-TECH-001 | Technical | Medium | Medium | MEDIUM | Phase 3 | Low |
| RISK-TECH-002 | Technical | Medium | Low | LOW | Phase 2 | Very Low |
| RISK-TECH-003 | Technical | Low | Low | LOW | Phase 3 | Very Low |
| RISK-SEC-001 | Security | Low | High | MEDIUM | Phase 4 | Very Low |
| RISK-SEC-002 | Security | Medium | Medium | MEDIUM | Phase 2 | Low |
| RISK-SEC-003 | Security | Low | High | MEDIUM | Phase 2 | Low |
| RISK-OPS-001 | Operational | Low | Medium | LOW | Phase 2 | Low |
| RISK-OPS-002 | Operational | Medium | Low | LOW | Phase 2 | Very Low |
| RISK-OPS-003 | Operational | Medium | Low | LOW | Post-MVP | Low |
| RISK-MIG-001 | Migration | Low | High | MEDIUM | Phase 1 | Very Low |

**Overall Risk Profile**: LOW-MEDIUM
- 4 MEDIUM risks (all with strong mitigation)
- 6 LOW risks (acceptable with monitoring)
- 0 CRITICAL or HIGH risks
- All risks have defined mitigation and contingency plans

---

## 6. Testing Strategy

### 6.1 Unit Testing Strategy

**Test Coverage Target**: ≥90% for all new code

**Test Modules** (~400 LOC total):

#### 1. test_auth_provider.py (~100 LOC)

**Purpose**: Test AuthProvider interface contracts and implementations

**Test Cases**:
- `test_api_key_provider_get_credentials()`: Returns dict with type=api_key, value=key
- `test_api_key_provider_is_valid()`: Returns True for valid key, False for empty
- `test_api_key_provider_no_refresh()`: `refresh_credentials()` always returns True
- `test_api_key_provider_context_limit()`: Returns 1,000,000 tokens
- `test_oauth_provider_get_credentials()`: Returns dict with type=bearer, value=token, expires_at
- `test_oauth_provider_proactive_refresh()`: Refreshes when <5 min to expiry
- `test_oauth_provider_reactive_refresh()`: Not triggered on `get_credentials()` if valid
- `test_oauth_provider_token_expiry()`: Correctly calculates expiry from UTC timestamp
- `test_oauth_provider_refresh_failure()`: Returns False on 401, raises OAuthRefreshError on other errors
- `test_oauth_provider_context_limit()`: Returns 200,000 tokens

**Mocking**:
- Mock token refresh endpoint with `responses` library
- Mock `datetime.now()` for expiry testing

---

#### 2. test_oauth_auth.py (~150 LOC)

**Purpose**: Test OAuth token refresh logic in detail

**Test Cases**:
- `test_token_refresh_success()`: POST to refresh endpoint → New tokens returned → Stored in ConfigManager
- `test_token_refresh_401_retry()`: 401 response → No retry (refresh token expired)
- `test_token_refresh_429_backoff()`: 429 response → Wait `Retry-After` seconds → Retry
- `test_token_refresh_5xx_retry()`: 500/502/503 response → Exponential backoff → Retry 3x
- `test_token_rotation()`: Server returns new refresh_token → Both tokens updated
- `test_expiry_calculation()`: `expires_in` seconds → Correct UTC timestamp
- `test_proactive_refresh_timing()`: Token expires in 4 minutes → Refresh triggered
- `test_proactive_refresh_no_trigger()`: Token expires in 10 minutes → No refresh
- `test_concurrent_refresh()`: Multiple tasks trigger refresh → Only one refresh executes (mutex)
- `test_refresh_endpoint_url()`: Correct URL used (`https://console.anthropic.com/v1/oauth/token`)
- `test_refresh_request_format()`: Correct JSON body (grant_type, refresh_token, client_id)

**Mocking**:
- Mock `httpx.AsyncClient.post()` with `responses` library
- Mock `asyncio.sleep()` for backoff testing
- Mock `ConfigManager.set_oauth_token()` to verify persistence

---

#### 3. test_config_oauth.py (~100 LOC)

**Purpose**: Test ConfigManager OAuth token storage/retrieval

**Test Cases**:
- `test_get_oauth_token_from_env()`: Env vars set → Tokens retrieved
- `test_get_oauth_token_from_keychain()`: Keychain has tokens → Tokens retrieved
- `test_get_oauth_token_from_env_file()`: .env file has tokens → Tokens retrieved
- `test_get_oauth_token_priority()`: Env vars override keychain override .env
- `test_set_oauth_token_keychain()`: Tokens stored in keychain with correct keys
- `test_set_oauth_token_env_file()`: Tokens stored in .env file with correct format
- `test_set_oauth_token_fallback()`: Keychain unavailable → Falls back to .env with warning
- `test_detect_auth_method_api_key()`: `sk-ant-api` prefix → Returns "api_key"
- `test_detect_auth_method_oauth()`: Non-API-key format → Returns "oauth"
- `test_detect_auth_method_invalid()`: Invalid format → Raises ValueError with remediation
- `test_clear_oauth_tokens()`: Clears keychain, env vars, .env file
- `test_clear_oauth_tokens_partial()`: Keychain unavailable → Clears env vars and .env only

**Mocking**:
- Mock `keyring.get_password()` and `keyring.set_password()`
- Mock file system (`tempfile` for .env file)
- Mock `os.environ` for environment variable testing

---

#### 4. test_claude_client_oauth.py (~50 LOC)

**Purpose**: Test ClaudeClient OAuth integration

**Test Cases**:
- `test_init_with_api_key_provider()`: Constructor with APIKeyAuthProvider → Initialized correctly
- `test_init_with_oauth_provider()`: Constructor with OAuthAuthProvider → Initialized correctly
- `test_execute_task_with_oauth()`: OAuth provider → Task executes → Bearer token used
- `test_401_retry_with_token_refresh()`: 401 response → Token refreshed → Request retried → Success
- `test_401_max_retries_exceeded()`: 401 response → 3 refresh attempts → All fail → Raises OAuthTokenExpiredError
- `test_context_window_warning()`: Input 185K tokens (OAuth) → Warning logged
- `test_token_counting_accuracy()`: Known input → Estimated tokens within 10% of actual

**Mocking**:
- Mock `Anthropic.messages.create()` with various responses
- Mock `AuthProvider.refresh_credentials()`
- Mock `ConfigManager` for credential retrieval

---

### 6.2 Integration Testing Strategy

**Test Coverage Target**: ≥70% end-to-end scenarios

**Test Modules** (~200 LOC total):

#### 1. test_oauth_flow.py (~100 LOC)

**Purpose**: Test complete OAuth flow from CLI to API

**Test Cases**:
- `test_oauth_login_manual_mode()`: Run `oauth-login --manual` → Tokens stored in keychain
- `test_oauth_login_stores_tokens_in_keychain()`: Verify keychain contains correct tokens after login
- `test_oauth_login_fallback_to_env_file()`: Keychain unavailable → Tokens stored in .env
- `test_oauth_logout_clears_all_tokens()`: Run `oauth-logout` → Keychain, env vars, .env all cleared
- `test_oauth_status_displays_correct_info()`: Run `oauth-status` → Shows auth method, expiry, context limit
- `test_oauth_refresh_updates_tokens()`: Run `oauth-refresh` → New tokens stored
- `test_oauth_flow_end_to_end()`: oauth-login → spawn task → oauth-status → oauth-logout

**Test Environment**:
- Mock OAuth server (Flask app with `/oauth/token` endpoint)
- Temporary keychain (macOS testing)
- Temporary .env file
- Isolated test configuration

---

#### 2. test_token_refresh_integration.py (~100 LOC)

**Purpose**: Test token refresh during task execution

**Test Cases**:
- `test_token_expires_during_task()`: Long task (>1 hour simulated) → Token expires mid-task → Refresh triggered → Task completes
- `test_proactive_refresh_before_request()`: Token expires in 3 minutes → Proactive refresh → No 401 errors
- `test_reactive_refresh_on_401()`: Token expired → 401 response → Reactive refresh → Retry succeeds
- `test_refresh_failure_prompts_reauth()`: Refresh fails (401) → Clear error message → Instructs `oauth-login`
- `test_concurrent_tasks_with_token_refresh()`: 10 tasks running → Token expires → Only one refresh → All tasks succeed

**Test Environment**:
- Mock OAuth server with configurable responses
- Mock Anthropic API with 401 responses
- Concurrent task execution (`asyncio.gather`)

---

### 6.3 Security Testing Strategy

**Security Test Scenarios** (~6 tests):

#### 1. test_no_token_in_logs()
**Purpose**: Verify no token values in log output

**Method**:
1. Execute OAuth flow (login, task, refresh, logout)
2. Capture all log output (stdout, stderr, log files)
3. Scan logs with regex: `sk-ant-api`, `Bearer [A-Za-z0-9]+`, token-like patterns
4. Assert: Zero matches found

**Tools**: Custom regex scanner, TruffleHog integration

---

#### 2. test_no_token_in_errors()
**Purpose**: Verify no credentials in error messages or exception traces

**Method**:
1. Trigger all error paths (invalid key, expired token, refresh failure, etc.)
2. Capture exception messages and stack traces
3. Scan for token patterns
4. Assert: Zero token values found

---

#### 3. test_https_enforcement()
**Purpose**: Verify all token refresh requests use HTTPS

**Method**:
1. Mock `httpx` to log request URLs
2. Execute token refresh
3. Assert: All URLs start with `https://`
4. Assert: No HTTP requests made

**Tools**: `httpx` mock, network traffic monitoring (optional: `mitmproxy`)

---

#### 4. test_token_cleanup_on_logout()
**Purpose**: Verify complete token cleanup

**Method**:
1. Store tokens in all locations (keychain, env vars, .env file)
2. Run `oauth-logout`
3. Verify:
   - Keychain: No tokens found
   - Environment variables: `ANTHROPIC_AUTH_TOKEN`, `ANTHROPIC_OAUTH_REFRESH_TOKEN` unset
   - .env file: Token lines removed
4. Assert: Zero residual tokens

---

#### 5. test_env_file_permissions()
**Purpose**: Verify .env file has restrictive permissions

**Method**:
1. Store tokens in .env file
2. Check file permissions (`os.stat()`)
3. Assert: Permissions are 600 (user read/write only)
4. Warn if permissions are more permissive

---

#### 6. test_token_sanitization_in_exception_traces()
**Purpose**: Verify custom exceptions sanitize token values

**Method**:
1. Create exceptions with token values in context
2. Capture exception `__str__()` and `__repr__()`
3. Scan for token patterns
4. Assert: Tokens redacted or absent

---

**Security Tools**:
- **Bandit**: Static analysis for Python security issues (run in CI/CD)
- **Safety**: Dependency vulnerability scanning (run weekly)
- **TruffleHog**: Secrets scanning in codebase and git history
- **Automated log scanning**: Regex for token patterns (custom script)

---

### 6.4 Load Testing Strategy

**Load Test Scenarios** (~2 tests):

#### 1. Concurrent OAuth Requests (10-100 tasks)

**Purpose**: Verify no race conditions under load

**Method**:
1. Initialize 100 tasks with OAuth authentication
2. Execute concurrently using `asyncio.gather()`
3. Measure:
   - Success rate (target: ≥99%)
   - 401 error count (should be 0 with proactive refresh)
   - Token refresh count (should be 1, not 100)
4. Assert: All tasks succeed, no race conditions

**Tools**: `pytest-xdist` for parallel execution, `asyncio` for concurrency

---

#### 2. Token Refresh Under Load

**Purpose**: Verify token refresh during concurrent execution

**Method**:
1. Start 50 tasks with valid OAuth token
2. Expire token after 25 tasks complete (mock time progression)
3. Verify:
   - Token refresh triggered once
   - All remaining tasks succeed with new token
   - No tasks fail with 401
4. Measure:
   - Task completion rate (target: ≥99%)
   - Average task latency (should not spike)

**Tools**: `Locust` for load generation (optional), `asyncio` for concurrency

---

**Load Testing Tools**:
- **Locust**: Python-based load testing framework (optional)
- **pytest-xdist**: Parallel test execution
- **asyncio**: Built-in concurrency for async tasks

---

### 6.5 End-to-End Testing (Manual)

**E2E Test Scenarios** (~7 manual tests):

1. **Interactive OAuth Login** (if browser-based flow implemented)
   - Trigger browser OAuth flow
   - Authorize in browser
   - Verify tokens stored in keychain

2. **Keychain Storage on macOS**
   - Run `oauth-login --manual`
   - Open Keychain Access.app
   - Verify tokens stored under "abathur" service

3. **Keychain Storage on Linux (gnome-keyring)**
   - Run `oauth-login --manual`
   - Run `secret-tool search service abathur`
   - Verify tokens stored

4. **.env File Fallback**
   - Uninstall keychain (or block access)
   - Run `oauth-login --manual`
   - Verify .env file created with tokens
   - Verify warning displayed

5. **Token Refresh During Long-Running Task**
   - Start task with 55-minute timeout
   - Monitor logs for proactive refresh at 55 minutes
   - Verify task completes without 401 errors

6. **Context Window Warning for Large Inputs**
   - Submit task with 185K token input (OAuth mode)
   - Verify warning logged and displayed
   - Verify task proceeds (warn mode) or blocked (block mode)

7. **Rate Limit Warning at 80% Usage** (deferred to post-MVP)
   - Submit 40 tasks with OAuth (50 limit assumed)
   - Verify warning at 40th task
   - Submit 11 more tasks
   - Verify 429 error with clear message

**E2E Test Checklist**:
- [ ] OAuth login stores tokens in keychain (macOS verified)
- [ ] OAuth login stores tokens in keychain (Linux verified)
- [ ] Tokens persist across application restarts
- [ ] Token refresh happens automatically on 401
- [ ] Context window warnings display correctly
- [ ] oauth-status shows accurate information
- [ ] oauth-logout clears all tokens (verified manually)

---

### 6.6 Testing Timeline

| Week | Testing Focus | Test LOC | Coverage Target |
|------|---------------|----------|-----------------|
| Week 1 | Unit tests (auth_provider, api_key_auth, exceptions) | 100 | ≥90% |
| Week 2 | Unit tests (oauth_auth, config_oauth) | 250 | ≥90% |
| Week 3 | Integration tests (oauth_flow, token_refresh, context_window) | 200 | ≥70% |
| Week 4 | Security, load, E2E testing | 50 (additional) | ≥90% overall |

**Total Test LOC**: ~600
**Overall Coverage Target**: ≥90% (unit + integration)

---

## 7. Deployment Checklist

### 7.1 Pre-Deployment Validation

**Code Quality** (Critical):
- [ ] All unit tests pass (≥90% coverage)
  - **Validation**: Run `pytest tests/unit/ --cov=abathur --cov-report=term-missing`
  - **Owner**: Developer
  - **Priority**: Critical
  - **Estimated Time**: 5 minutes

- [ ] All integration tests pass (≥70% coverage)
  - **Validation**: Run `pytest tests/integration/`
  - **Owner**: Developer
  - **Priority**: Critical
  - **Estimated Time**: 10 minutes

- [ ] Security tests pass (no token exposure)
  - **Validation**: Run `pytest tests/security/` + manual log review
  - **Owner**: Developer
  - **Priority**: Critical
  - **Estimated Time**: 15 minutes

- [ ] Load tests pass (≥99% success rate under load)
  - **Validation**: Run `pytest tests/load/` with 100 concurrent tasks
  - **Owner**: Developer
  - **Priority**: High
  - **Estimated Time**: 20 minutes

- [ ] Linting passes (Ruff, Black, Mypy)
  - **Validation**: Run `ruff check .`, `black --check .`, `mypy src/`
  - **Owner**: Developer
  - **Priority**: High
  - **Estimated Time**: 5 minutes

- [ ] No critical security vulnerabilities (Bandit, Safety)
  - **Validation**: Run `bandit -r src/`, `safety check`
  - **Owner**: Developer
  - **Priority**: Critical
  - **Estimated Time**: 5 minutes

- [ ] No secrets in codebase (TruffleHog)
  - **Validation**: Run `trufflehog filesystem .`
  - **Owner**: Developer
  - **Priority**: Critical
  - **Estimated Time**: 10 minutes

**Backward Compatibility** (Critical):
- [ ] All existing API key tests pass (100%)
  - **Validation**: Run `pytest tests/` with API key mode only
  - **Owner**: Developer
  - **Priority**: Critical
  - **Estimated Time**: 5 minutes

- [ ] API key workflows unchanged (manual verification)
  - **Validation**: Execute 5 common API key tasks manually
  - **Owner**: Developer
  - **Priority**: Critical
  - **Estimated Time**: 30 minutes

- [ ] No breaking changes in ClaudeClient API
  - **Validation**: Code review of public API signatures
  - **Owner**: Developer
  - **Priority**: Critical
  - **Estimated Time**: 15 minutes

- [ ] Existing agent templates work without modification
  - **Validation**: Test 3 existing templates from `odgrim/abathur-claude-template`
  - **Owner**: Developer
  - **Priority**: High
  - **Estimated Time**: 20 minutes

**Documentation** (High):
- [ ] Migration guide complete (API key → OAuth)
  - **Validation**: Peer review for clarity and completeness
  - **Owner**: Developer
  - **Priority**: High
  - **Estimated Time**: 30 minutes (review)

- [ ] OAuth setup guide complete (oauth-login workflow)
  - **Validation**: Follow guide as new user, verify all steps work
  - **Owner**: Developer
  - **Priority**: High
  - **Estimated Time**: 20 minutes

- [ ] Troubleshooting guide complete (common errors, resolutions)
  - **Validation**: Trigger each error scenario, verify resolution steps
  - **Owner**: Developer
  - **Priority**: High
  - **Estimated Time**: 30 minutes

- [ ] API reference updated (ClaudeClient with AuthProvider)
  - **Validation**: Code examples run successfully
  - **Owner**: Developer
  - **Priority**: Medium
  - **Estimated Time**: 15 minutes

- [ ] Configuration reference updated (AuthConfig options)
  - **Validation**: All config options documented with examples
  - **Owner**: Developer
  - **Priority**: Medium
  - **Estimated Time**: 10 minutes

**Security Validation** (Critical):
- [ ] Threat model reviewed (all threats mitigated)
  - **Validation**: Review security architecture document (05_security_architecture.md)
  - **Owner**: Developer
  - **Priority**: Critical
  - **Estimated Time**: 30 minutes

- [ ] Encryption strategy verified (OS keychain encryption levels)
  - **Validation**: Check macOS Keychain encryption (AES-256), Linux Secret Service
  - **Owner**: Developer
  - **Priority**: Critical
  - **Estimated Time**: 20 minutes

- [ ] Security testing complete (penetration tests, vulnerability scans)
  - **Validation**: All security tests pass, vulnerability scan shows 0 critical/high
  - **Owner**: Developer
  - **Priority**: Critical
  - **Estimated Time**: 30 minutes

- [ ] Audit logging implemented (≥15 security events)
  - **Validation**: Trigger events, verify structured logs captured
  - **Owner**: Developer
  - **Priority**: High
  - **Estimated Time**: 20 minutes

- [ ] Compliance requirements met (GDPR, data privacy)
  - **Validation**: Legal review (if required), privacy policy updated
  - **Owner**: Developer
  - **Priority**: Medium
  - **Estimated Time**: 1 hour (legal review)

---

### 7.2 Deployment Process

**Version Bump** (Critical):
- [ ] Update version to v0.2.0 (minor version bump for new feature)
  - **Validation**: Check `pyproject.toml`, `__init__.py`, `VERSION` file
  - **Owner**: Developer
  - **Priority**: Critical
  - **Estimated Time**: 5 minutes

- [ ] Update CHANGELOG.md with OAuth support details
  - **Validation**: CHANGELOG includes all changes, migration notes, breaking changes section
  - **Owner**: Developer
  - **Priority**: High
  - **Estimated Time**: 20 minutes

**Release Notes** (High):
- [ ] Document new OAuth authentication support
  - **Validation**: Release notes clear, concise, benefits highlighted
  - **Owner**: Developer
  - **Priority**: High
  - **Estimated Time**: 30 minutes

- [ ] Document new CLI commands (oauth-login, oauth-logout, oauth-status, oauth-refresh)
  - **Validation**: Each command documented with examples
  - **Owner**: Developer
  - **Priority**: High
  - **Estimated Time**: 20 minutes

- [ ] Document context window warnings
  - **Validation**: Warning behavior clearly explained
  - **Owner**: Developer
  - **Priority**: Medium
  - **Estimated Time**: 10 minutes

- [ ] Document migration path for API key users
  - **Validation**: Step-by-step migration guide included
  - **Owner**: Developer
  - **Priority**: High
  - **Estimated Time**: 15 minutes

- [ ] Document known limitations (OAuth 200K context, rate limits)
  - **Validation**: Limitations section complete and accurate
  - **Owner**: Developer
  - **Priority**: Medium
  - **Estimated Time**: 10 minutes

**PyPI Package** (Critical):
- [ ] Build distribution packages (sdist, wheel)
  - **Validation**: Run `python -m build`, verify dist/ contains .tar.gz and .whl
  - **Owner**: Developer
  - **Priority**: Critical
  - **Estimated Time**: 5 minutes

- [ ] Test installation in clean environment
  - **Validation**: Create virtualenv, `pip install dist/abathur-0.2.0.tar.gz`, verify imports
  - **Owner**: Developer
  - **Priority**: Critical
  - **Estimated Time**: 10 minutes

- [ ] Upload to PyPI
  - **Validation**: Run `twine upload dist/*`, verify upload succeeds
  - **Owner**: Developer
  - **Priority**: Critical
  - **Estimated Time**: 5 minutes

- [ ] Verify installation from PyPI
  - **Validation**: `pip install abathur==0.2.0`, verify version and OAuth commands
  - **Owner**: Developer
  - **Priority**: Critical
  - **Estimated Time**: 5 minutes

**Documentation Site** (High):
- [ ] Update README.md with OAuth setup quickstart
  - **Validation**: Quickstart section prominent, clear, with examples
  - **Owner**: Developer
  - **Priority**: High
  - **Estimated Time**: 20 minutes

- [ ] Publish migration guide
  - **Validation**: Guide accessible from docs site, linked from README
  - **Owner**: Developer
  - **Priority**: High
  - **Estimated Time**: 10 minutes

- [ ] Publish OAuth setup guide
  - **Validation**: Guide accessible, linked from README and release notes
  - **Owner**: Developer
  - **Priority**: High
  - **Estimated Time**: 10 minutes

- [ ] Publish troubleshooting guide
  - **Validation**: Guide accessible, searchable, covers common issues
  - **Owner**: Developer
  - **Priority**: High
  - **Estimated Time**: 10 minutes

- [ ] Update API reference
  - **Validation**: API docs include AuthProvider examples, accurate
  - **Owner**: Developer
  - **Priority**: Medium
  - **Estimated Time**: 15 minutes

---

### 7.3 Post-Deployment Monitoring

**Metrics to Track** (first 30 days):
- [ ] OAuth vs API key usage ratio (target: ≥20% OAuth adoption)
  - **Validation**: Query logs for `auth_method=oauth` vs `auth_method=api_key`
  - **Owner**: Developer
  - **Priority**: High
  - **Estimated Time**: Ongoing (daily review)

- [ ] Token refresh success rate (target: ≥99.5%)
  - **Validation**: Query logs for `oauth_token_refreshed` vs `oauth_token_refresh_failed`
  - **Owner**: Developer
  - **Priority**: Critical
  - **Estimated Time**: Ongoing (daily review)

- [ ] Authentication failures (target: <5% of requests)
  - **Validation**: Query logs for `authentication_error` events
  - **Owner**: Developer
  - **Priority**: High
  - **Estimated Time**: Ongoing (daily review)

- [ ] Context window warnings (track frequency and auth method)
  - **Validation**: Query logs for `context_window_warning`, group by auth_method
  - **Owner**: Developer
  - **Priority**: Medium
  - **Estimated Time**: Ongoing (weekly review)

- [ ] Rate limit warnings (track frequency)
  - **Validation**: Query logs for `oauth_rate_limit_warning`
  - **Owner**: Developer
  - **Priority**: Medium
  - **Estimated Time**: Ongoing (weekly review)

**Error Monitoring** (Critical):
- [ ] Auth failures (alert if >10/hour)
  - **Validation**: Set up alert in monitoring system (Datadog, Sentry, etc.)
  - **Owner**: Developer
  - **Priority**: Critical
  - **Estimated Time**: 30 minutes setup

- [ ] Token refresh failures (alert if >5% rate)
  - **Validation**: Set up alert with threshold `oauth_token_refresh_failed / oauth_token_refreshed > 0.05`
  - **Owner**: Developer
  - **Priority**: Critical
  - **Estimated Time**: 20 minutes setup

- [ ] Context window exceeded errors (track and analyze)
  - **Validation**: Weekly review of `context_limit_exceeded` events
  - **Owner**: Developer
  - **Priority**: Medium
  - **Estimated Time**: 30 minutes weekly

- [ ] 429 rate limit errors (track and analyze)
  - **Validation**: Weekly review of 429 responses by auth method
  - **Owner**: Developer
  - **Priority**: Medium
  - **Estimated Time**: 30 minutes weekly

**User Support** (High):
- [ ] Monitor support tickets (OAuth-related issues)
  - **Validation**: Review GitHub issues, support email, Discord (if applicable)
  - **Owner**: Developer
  - **Priority**: High
  - **Estimated Time**: 1 hour daily (first week), 30 min daily (weeks 2-4)

- [ ] Update troubleshooting guide based on common issues
  - **Validation**: Add new issues to guide weekly
  - **Owner**: Developer
  - **Priority**: High
  - **Estimated Time**: 1 hour weekly

- [ ] Collect user feedback (OAuth UX, pain points)
  - **Validation**: Survey via GitHub discussions, collect anecdotal feedback
  - **Owner**: Developer
  - **Priority**: Medium
  - **Estimated Time**: 30 minutes weekly

**Incident Response** (Critical):
- [ ] Define escalation path for security incidents (token exposure)
  - **Validation**: Document incident response plan
  - **Owner**: Developer
  - **Priority**: Critical
  - **Estimated Time**: 1 hour

- [ ] Define rollback procedure (revert to v0.1.x if critical bug)
  - **Validation**: Document rollback steps, test in staging
  - **Owner**: Developer
  - **Priority**: Critical
  - **Estimated Time**: 1 hour

- [ ] Define hotfix process (patch OAuth bugs without full release)
  - **Validation**: Document hotfix workflow (branch, test, release)
  - **Owner**: Developer
  - **Priority**: High
  - **Estimated Time**: 30 minutes

---

## 8. Success Metrics

### 8.1 Development Metrics

**Timeline Adherence**:
- [ ] Phase 1 completed in 1 week (25 hours)
  - **Target**: ±2 hours acceptable
  - **Measurement**: Track actual hours per task

- [ ] Phase 2 completed in 1 week (38 hours)
  - **Target**: ±3 hours acceptable
  - **Measurement**: Track actual hours per task

- [ ] Phase 3 completed in 1 week (34 hours)
  - **Target**: ±3 hours acceptable
  - **Measurement**: Track actual hours per task

- [ ] Phase 4 completed in 1 week (31 hours)
  - **Target**: ±3 hours acceptable
  - **Measurement**: Track actual hours per task

- [ ] Total implementation: 4 weeks (110 hours)
  - **Target**: ±1 week acceptable (5 weeks max)
  - **Measurement**: Sum of all phases

**Quality Gates**:
- [ ] Test coverage ≥90% (unit + integration)
  - **Measurement**: `pytest --cov=abathur --cov-report=term`
  - **Target**: ≥90.0%

- [ ] Zero critical bugs in production (first 30 days)
  - **Measurement**: GitHub issues labeled "critical" or "blocker"
  - **Target**: 0

- [ ] Zero security incidents (first 90 days)
  - **Measurement**: Security audit log, incident reports
  - **Target**: 0

**Code Quality**:
- [ ] All NFRs met (31 non-functional requirements)
  - **Measurement**: Manual review against requirements document
  - **Target**: 31/31 (100%)

- [ ] All FRs met (30 functional requirements)
  - **Measurement**: Manual review against requirements document
  - **Target**: 30/30 (100%)

- [ ] Clean Architecture preserved (dependency audit)
  - **Measurement**: `import` analysis, architecture review
  - **Target**: Zero violations

- [ ] Documentation complete (≥3 user guides)
  - **Measurement**: Count of guides (migration, setup, troubleshooting)
  - **Target**: ≥3 guides, ≥500 lines total

---

### 8.2 Adoption Metrics (First 30 Days)

**Usage**:
- [ ] ≥20% of users adopt OAuth
  - **Measurement**: `(oauth_logins / total_logins) * 100`
  - **Target**: ≥20%
  - **Data Source**: Structured logs (auth_method field)

- [ ] ≥95% of OAuth users successful (no support tickets)
  - **Measurement**: `1 - (oauth_support_tickets / oauth_users)`
  - **Target**: ≥95%
  - **Data Source**: GitHub issues, support email

**Reliability**:
- [ ] Token refresh success rate ≥99.5%
  - **Measurement**: `oauth_token_refreshed / (oauth_token_refreshed + oauth_token_refresh_failed)`
  - **Target**: ≥99.5%
  - **Data Source**: Structured logs

- [ ] Zero token exposure incidents
  - **Measurement**: Security audit, log scanning
  - **Target**: 0
  - **Data Source**: Automated scanning reports, incident reports

**User Satisfaction**:
- [ ] User satisfaction >4.0/5.0 (survey)
  - **Measurement**: User survey (email, GitHub discussions)
  - **Target**: ≥4.0/5.0
  - **Data Source**: Survey responses (minimum 20 responses)

- [ ] <10 OAuth-related support tickets
  - **Measurement**: Count of GitHub issues with "oauth" label
  - **Target**: <10
  - **Data Source**: GitHub issue tracker

---

### 8.3 Performance Metrics

**Latency Targets** (from NFR-PERF):
- [ ] Token refresh latency <100ms (p95)
  - **Measurement**: `refresh_latency_ms` log field, p95 percentile
  - **Target**: <100ms
  - **Data Source**: Structured logs

- [ ] Auth detection latency <10ms (p95)
  - **Measurement**: `auth_detection_latency_ms` log field, p95 percentile
  - **Target**: <10ms
  - **Data Source**: Structured logs

- [ ] Token counting latency <50ms (p95)
  - **Measurement**: `validation_latency_ms` log field, p95 percentile
  - **Target**: <50ms
  - **Data Source**: Structured logs

**Throughput**:
- [ ] 100 concurrent tasks succeed (load test)
  - **Measurement**: Load test success rate
  - **Target**: ≥99%
  - **Data Source**: Load test results (W4-T3)

---

## 9. Migration Guide (Outline)

### 9.1 For Existing API Key Users

**No Action Required**:
- All existing API key workflows continue to work
- No configuration changes needed
- v0.2.0 is fully backward compatible

**Optional: Transition to OAuth**:
1. Obtain OAuth tokens:
   - Option A: Use Claude Code CLI and extract tokens from keychain
   - Option B: Manual token extraction from browser dev tools (advanced)
2. Run `abathur config oauth-login --manual`
3. Enter access token, refresh token, expires_in when prompted
4. Verify with `abathur config oauth-status`
5. (Optional) Remove API key: `unset ANTHROPIC_API_KEY`

**When to Use OAuth**:
- You have Claude Max subscription (50-200 prompts/5h included)
- Tasks fit within 200K token context window
- Want to avoid API key billing

**When to Use API Key**:
- Large tasks requiring 1M token context window
- High-volume usage (>200 prompts/5h)
- Production deployments with billing oversight

---

### 9.2 For New Users (OAuth)

**OAuth Setup**:
1. **Obtain OAuth tokens**:
   - Option A: Use Claude Code CLI (`claude auth login`) and extract tokens from keychain
   - Option B: (Future) Interactive browser-based OAuth flow in Abathur
2. **Configure Abathur**:
   ```bash
   abathur config oauth-login --manual
   # Enter access token: <paste-token>
   # Enter refresh token: <paste-token>
   # Enter expires in (seconds): 3600
   ```
3. **Verify setup**:
   ```bash
   abathur config oauth-status
   # ✓ Auth Method: OAuth
   # ✓ Context Limit: 200,000 tokens
   # ✓ Token Expiry: 59m 30s
   ```
4. **Start using Abathur**:
   ```bash
   abathur spawn task "Implement user authentication"
   # ✓ Task spawned with OAuth authentication
   ```

---

### 9.3 Troubleshooting Common Issues

#### Issue: Token Refresh Failures

**Error**: `OAuth token expired. Refresh failed.`

**Cause**: Refresh token expired or revoked

**Solution**:
1. Re-authenticate:
   ```bash
   abathur config oauth-login --manual
   ```
2. Verify tokens stored:
   ```bash
   abathur config oauth-status
   ```

---

#### Issue: Context Window Warnings

**Warning**: `Task input (185K tokens) approaching OAuth limit (200K tokens)`

**Cause**: Large task input with OAuth authentication (200K limit)

**Solutions**:
1. **Use API key authentication** (recommended for large tasks):
   ```bash
   export ANTHROPIC_API_KEY="sk-ant-api03-..."
   abathur spawn task "Large refactoring task"
   ```
2. **Reduce input size**:
   - Remove unnecessary files/context
   - Shorten system prompt
   - Split task into smaller subtasks

---

#### Issue: Rate Limit Exceeded

**Error**: `Rate limit exceeded: 50/50 prompts used in current 5-hour window`

**Cause**: Hit OAuth rate limit (Max 5x: 50-200 prompts/5h)

**Solutions**:
1. **Wait for window reset** (shown in error message):
   ```
   Rate limit resets in: 2h 15m
   ```
2. **Use API key authentication** (no rate limit):
   ```bash
   export ANTHROPIC_API_KEY="sk-ant-api03-..."
   ```

---

#### Issue: Keychain Unavailable (Linux)

**Warning**: `OS keychain unavailable. Storing tokens in .env file (less secure).`

**Cause**: Linux system without gnome-keyring installed

**Solutions**:
1. **Install gnome-keyring** (recommended):
   ```bash
   # Ubuntu/Debian
   sudo apt install gnome-keyring

   # Fedora
   sudo dnf install gnome-keyring
   ```
2. **Accept .env fallback** (less secure):
   - Tokens stored in `.abathur/.env`
   - Ensure `.gitignore` includes `.env`
   - File permissions set to 600 (user-only)

---

## 10. Dependencies and Assumptions

### 10.1 Dependencies

**Python Dependencies**:
- `anthropic = "^0.18.0"` - Supports `ANTHROPIC_AUTH_TOKEN` environment variable
- `httpx` - Already in dependencies (for token refresh endpoint)
- `keyring` - Already in dependencies (for token storage)
- No new dependencies required

**External Dependencies**:
- Token refresh endpoint stable: `https://console.anthropic.com/v1/oauth/token`
- OS keychain available: macOS Keychain, Linux Secret Service (gnome-keyring)
- HTTPS connectivity to Anthropic APIs

**Development Dependencies**:
- `pytest` - Unit and integration testing
- `pytest-cov` - Code coverage measurement
- `pytest-xdist` - Parallel test execution
- `responses` - Mock HTTP responses for testing
- `bandit` - Static security analysis
- `safety` - Dependency vulnerability scanning
- `trufflehog` - Secrets scanning

---

### 10.2 Assumptions

**User Environment**:
- Users have access to OAuth tokens (via Claude Code CLI or future interactive flow)
- Development environment: Python 3.10+, pip, virtualenv
- Production environment: HTTPS connectivity, keychain or environment variables available

**Anthropic API Stability**:
- Token refresh endpoint request/response format remains stable
- SDK `ANTHROPIC_AUTH_TOKEN` support continues in future versions
- API behavior (401 on expired token) consistent

**Security**:
- OS keychain encryption meets security requirements (AES-256 equivalent on macOS, Secret Service encryption on Linux)
- HTTPS enforced at SDK and httpx level (no manual certificate validation needed)

**Team**:
- Development team has 110 hours available over 4 weeks (1 developer full-time at 27.5 hrs/week)
- Developer familiar with Python, async programming, OAuth 2.1, Clean Architecture
- Code review available (self-review acceptable for solo developer)

---

## 11. Conclusion

This implementation roadmap provides a comprehensive 4-week plan for adding OAuth-based agent spawning to Abathur with minimal risk and zero breaking changes.

**Key Success Factors**:
1. **Phased Approach**: Each week has clear milestone and success criteria
2. **Risk Mitigation**: All major risks identified with mitigation strategies
3. **Testing Focus**: ≥90% test coverage with security and load testing
4. **Documentation**: Complete user guides and troubleshooting
5. **Quality Gates**: Pre-deployment validation checklist ensures production readiness

**Timeline Confidence**: HIGH
- Buffer built in (110 planned hours vs 160 available hours = 50-hour buffer)
- Well-defined tasks with realistic estimates
- Minimal external dependencies
- Backward compatibility reduces migration risk

**Next Steps**:
1. Review and approve this roadmap
2. Begin Phase 1 (Week 1): AuthProvider abstraction
3. Daily standup to track progress and blockers
4. Weekly milestone reviews at phase gates
5. Deploy v0.2.0 with OAuth support after Week 4 validation

---

**Document Status**: READY FOR REVIEW
**Approval Required**: prd-project-orchestrator
**Next Phase**: Implementation (Weeks 1-4)
**Estimated Start Date**: Upon approval
