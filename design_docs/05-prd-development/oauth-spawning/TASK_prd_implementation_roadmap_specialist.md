# Task Specification: prd-implementation-roadmap-specialist

**Date**: October 9, 2025
**Phase**: Phase 3 - Implementation Roadmap
**Deliverable**: `06_implementation_roadmap.md`
**Timeline**: Week 6 (1 week)
**Status**: Ready for Agent Invocation (after security deliverable)

---

## 1. Objective

Define comprehensive 4-week implementation roadmap with phased milestones, risk assessment, testing strategy, and deployment checklist to prepare OAuth-based agent spawning for development handoff.

---

## 2. Inputs

### 2.1 Required Documents

**Phase 2 Deliverables**:
1. **03_technical_requirements.md** - 30 FRs + 31 NFRs with acceptance criteria
2. **04_system_architecture.md** - AuthProvider architecture, integration points, ~600 LOC scope
3. **02_current_architecture.md** - Current system architecture, integration points

**Phase 3 Deliverables**:
4. **05_security_architecture.md** - Threat model, encryption strategy, security testing plan (completed by prd-security-specialist)
5. **00_phase3_context.md** - Phase 3 context summary with implementation focus

**Supporting Documents**:
6. **DECISION_POINTS.md** - Architectural decisions and constraints
7. **PHASE2_VALIDATION_REPORT.md** - Validation findings, minor issues, recommendations

### 2.2 Implementation Scope from Phase 2

**New Files** (4):
1. `domain/ports/auth_provider.py` (~50 LOC): AuthProvider interface
2. `infrastructure/api_key_auth.py` (~30 LOC): APIKeyAuthProvider implementation
3. `infrastructure/oauth_auth.py` (~200 LOC): OAuthAuthProvider implementation
4. `infrastructure/exceptions.py` (~50 LOC): Custom exception hierarchy

**Modified Files** (3):
5. `application/claude_client.py` (~150 LOC modified): AuthProvider integration, 401 retry, context validation
6. `infrastructure/config.py` (~140 LOC added): OAuth methods, auth detection
7. `cli/main.py` (~190 LOC added): OAuth commands, service initialization

**Testing** (~600 LOC):
8. Unit tests: ~400 LOC (AuthProvider, OAuth, Config, ClaudeClient)
9. Integration tests: ~200 LOC (OAuth flow, token refresh, context window)

**Total**: ~1,200 LOC (600 implementation + 600 tests)

---

## 3. Deliverable Specification

### 3.1 Document Structure

**File**: `prd_oauth_spawning/06_implementation_roadmap.md`

**Required Sections**:

#### Section 1: Executive Summary
- Implementation overview (4-week phased approach)
- Scope summary (~600 LOC, 7 files)
- Critical path and dependencies
- Success criteria (90% test coverage, zero breaking changes, security validation)

#### Section 2: Phased Implementation Plan

**Week-by-Week Breakdown**:

**Phase 1 (Week 1): Foundation**
- **Milestone**: AuthProvider abstraction and API key refactoring
- **Deliverables**:
  1. Create `domain/ports/auth_provider.py` (AuthProvider interface)
  2. Create `infrastructure/api_key_auth.py` (APIKeyAuthProvider)
  3. Create `infrastructure/exceptions.py` (exception hierarchy)
  4. Unit tests for AuthProvider and APIKeyAuthProvider
  5. Refactor ClaudeClient constructor to accept AuthProvider (optional, backward compatible)
- **Success Criteria**:
  - [ ] AuthProvider interface defined with 5 methods
  - [ ] APIKeyAuthProvider wraps existing API key logic
  - [ ] All existing API key tests pass (100% backward compatibility)
  - [ ] Unit test coverage: ≥90% for new code
- **Dependencies**: None (foundational work)
- **Risks**: Refactoring ClaudeClient constructor may break existing code
- **Mitigation**: Make AuthProvider parameter optional, preserve api_key parameter
- **LOC**: ~130 (interface 50 + API key auth 30 + exceptions 50)
- **Developer Hours**: 20 hours

**Phase 2 (Week 2): OAuth Core**
- **Milestone**: OAuth token lifecycle fully implemented
- **Deliverables**:
  1. Create `infrastructure/oauth_auth.py` (OAuthAuthProvider with refresh logic)
  2. Add ConfigManager OAuth methods (get_oauth_token, set_oauth_token, detect_auth_method, clear_oauth_tokens)
  3. Add AuthConfig model to Config (auth.mode, oauth_token_storage, etc.)
  4. Unit tests for OAuthAuthProvider (proactive/reactive refresh, expiry detection)
  5. Unit tests for ConfigManager OAuth methods (keychain, env, .env fallback)
- **Success Criteria**:
  - [ ] OAuthAuthProvider implements token refresh with 3-retry logic
  - [ ] Proactive refresh (5-min buffer) and reactive refresh (401) working
  - [ ] Token storage uses OS keychain (macOS/Linux verified)
  - [ ] Token expiry calculation correct (UTC timezone, ISO 8601 format)
  - [ ] Unit test coverage: ≥90% for OAuth code
- **Dependencies**: Phase 1 (AuthProvider interface)
- **Risks**: Token refresh endpoint changes, OS keychain unavailable
- **Mitigation**: 3-retry logic with fallback to manual re-auth, .env file fallback
- **LOC**: ~340 (OAuth auth 200 + ConfigManager 140)
- **Developer Hours**: 30 hours

**Phase 3 (Week 3): CLI Integration**
- **Milestone**: End-to-end OAuth authentication working
- **Deliverables**:
  1. Modify ClaudeClient.execute_task() (401 retry loop, context validation, token counting)
  2. Modify CLI _get_services() (detect auth method, initialize AuthProvider)
  3. Add CLI OAuth commands (oauth-login --manual, oauth-logout, oauth-status, oauth-refresh)
  4. Integration tests: oauth-login → task execution → token refresh → oauth-logout
  5. Context window integration tests (warning at 90% threshold)
- **Success Criteria**:
  - [ ] oauth-login command stores tokens in keychain
  - [ ] ClaudeClient auto-refreshes tokens on 401
  - [ ] Context window warnings triggered at 90% threshold
  - [ ] oauth-status displays auth method, expiry, context limit
  - [ ] Integration test coverage: ≥70% (end-to-end scenarios)
- **Dependencies**: Phase 2 (OAuthAuthProvider, ConfigManager OAuth methods)
- **Risks**: SDK concurrent requests (race conditions), context estimation inaccuracy
- **Mitigation**: Mutex/lock around refresh_credentials(), conservative token estimation
- **LOC**: ~340 (ClaudeClient 150 + CLI 190)
- **Developer Hours**: 30 hours

**Phase 4 (Week 4): Testing & Documentation**
- **Milestone**: Production-ready implementation
- **Deliverables**:
  1. Comprehensive unit tests (~400 LOC total)
  2. Integration tests (~200 LOC total)
  3. Security tests (log sanitization, HTTPS enforcement, token cleanup)
  4. Load tests (10-100 concurrent tasks with OAuth)
  5. Migration guide (API key users → OAuth)
  6. OAuth setup guide (oauth-login workflow)
  7. Troubleshooting guide (common OAuth errors)
  8. API reference updates (ClaudeClient with AuthProvider examples)
- **Success Criteria**:
  - [ ] Test coverage: ≥90% overall (unit + integration)
  - [ ] All security tests pass (no token exposure)
  - [ ] Load tests: 100 concurrent requests succeed
  - [ ] Documentation complete (≥3 user guides)
  - [ ] Backward compatibility validated (all existing tests pass)
- **Dependencies**: Phase 3 (CLI integration)
- **Risks**: Testing uncovers late-stage bugs, documentation incomplete
- **Mitigation**: Daily testing throughout Phases 1-3, documentation in parallel
- **LOC**: ~600 (tests)
- **Developer Hours**: 30 hours

**Total Implementation**: 4 weeks, 110 developer hours

#### Section 3: Risk Assessment

**Risk Categories**: Technical, Security, Operational, Migration

**Risk Template**:
- **Risk ID**: Unique identifier (e.g., RISK-TECH-001)
- **Description**: Clear description of risk
- **Likelihood**: Low / Medium / High
- **Impact**: Low / Medium / High
- **Mitigation**: Specific mitigation strategy
- **Owner**: Phase responsible for mitigation

**Required Risks** (≥10):

1. **RISK-TECH-001: SDK Concurrent Requests**
   - Description: Setting ANTHROPIC_AUTH_TOKEN env var may have race conditions in concurrent scenarios
   - Likelihood: Medium
   - Impact: Medium (task failures under load)
   - Mitigation: Mutex/lock around _configure_sdk_auth(), load testing in Phase 4
   - Owner: Phase 3

2. **RISK-TECH-002: Token Refresh Race Conditions**
   - Description: Multiple tasks may trigger refresh simultaneously, causing redundant refreshes
   - Likelihood: Medium
   - Impact: Low (inefficiency, not failures)
   - Mitigation: Mutex/lock around refresh_credentials(), test with concurrent tasks
   - Owner: Phase 2

3. **RISK-TECH-003: Context Window Estimation Inaccuracy**
   - Description: 4-char approximation may underestimate token count for code-heavy tasks
   - Likelihood: Low
   - Impact: Low (conservative bias acceptable)
   - Mitigation: User warnings, configurable handling modes (warn/block/ignore)
   - Owner: Phase 3

4. **RISK-SEC-001: Token Exposure in Logs**
   - Description: Accidental logging of token values in plaintext logs
   - Likelihood: Low (automated scanning mitigates)
   - Impact: High (credential compromise)
   - Mitigation: Log sanitization rules, automated scanning, security tests
   - Owner: Phase 4 (security testing)

5. **RISK-SEC-002: Insecure .env Fallback**
   - Description: Tokens stored in plaintext .env file when keychain unavailable
   - Likelihood: Medium (on Linux systems)
   - Impact: Medium (file permissions mitigate)
   - Mitigation: User warning, recommend keychain, .gitignore enforcement, permission checks
   - Owner: Phase 2

6. **RISK-SEC-003: Token Refresh Endpoint Changes**
   - Description: Anthropic changes endpoint URL or request/response format without notice
   - Likelihood: Low (widely used in Claude Code)
   - Impact: High (all token refreshes fail)
   - Mitigation: 3-retry logic, fallback to manual re-auth, monitoring, community engagement
   - Owner: Operational (post-deployment)

7. **RISK-OPS-001: Token Refresh Endpoint Availability**
   - Description: Service downtime or rate limiting on token refresh endpoint
   - Likelihood: Low
   - Impact: Medium (users cannot refresh tokens)
   - Mitigation: 3-retry logic with exponential backoff, fallback to manual re-auth
   - Owner: Phase 2

8. **RISK-OPS-002: OS Keychain Unavailable**
   - Description: Keychain access denied or service unavailable (Linux systems)
   - Likelihood: Medium
   - Impact: Low (fallback to .env file)
   - Mitigation: Graceful fallback with user warning, test on Linux
   - Owner: Phase 2

9. **RISK-OPS-003: Rate Limiting for OAuth Users**
   - Description: OAuth users hit 50-200 prompt limits unexpectedly
   - Likelihood: Medium (for heavy users)
   - Impact: Low (graceful error handling)
   - Mitigation: Usage tracking (deferred to post-MVP), warnings at 80%, clear 429 error messages
   - Owner: Post-MVP

10. **RISK-MIG-001: Backward Compatibility Failures**
    - Description: Existing API key workflows break due to refactoring
    - Likelihood: Low (well-isolated changes)
    - Impact: High (existing users affected)
    - Mitigation: 100% test coverage for API key scenarios, optional AuthProvider parameter
    - Owner: Phase 1

**Risk Summary Table**:
| Risk ID | Likelihood | Impact | Risk Level | Mitigation Phase |
|---------|------------|--------|------------|------------------|
| RISK-TECH-001 | Medium | Medium | MEDIUM | Phase 3 |
| RISK-TECH-002 | Medium | Low | LOW | Phase 2 |
| RISK-TECH-003 | Low | Low | LOW | Phase 3 |
| RISK-SEC-001 | Low | High | MEDIUM | Phase 4 |
| RISK-SEC-002 | Medium | Medium | MEDIUM | Phase 2 |
| RISK-SEC-003 | Low | High | MEDIUM | Operational |
| RISK-OPS-001 | Low | Medium | LOW | Phase 2 |
| RISK-OPS-002 | Medium | Low | LOW | Phase 2 |
| RISK-OPS-003 | Medium | Low | LOW | Post-MVP |
| RISK-MIG-001 | Low | High | MEDIUM | Phase 1 |

#### Section 4: Testing Strategy

**4.1 Unit Testing Strategy**

**Test Coverage Target**: ≥90% for all new code

**Test Modules** (~400 LOC):
1. **test_auth_provider.py** (~100 LOC):
   - test_api_key_provider_get_credentials()
   - test_api_key_provider_is_valid()
   - test_api_key_provider_no_refresh()
   - test_oauth_provider_get_credentials()
   - test_oauth_provider_proactive_refresh()
   - test_oauth_provider_reactive_refresh()
   - test_oauth_provider_token_expiry()
   - test_oauth_provider_refresh_failure()
   - Mock token refresh endpoint with responses library

2. **test_oauth_auth.py** (~150 LOC):
   - test_token_refresh_success()
   - test_token_refresh_401_retry()
   - test_token_refresh_429_backoff()
   - test_token_refresh_5xx_retry()
   - test_token_rotation()
   - test_expiry_calculation()
   - test_proactive_refresh_timing() (5-min buffer)
   - test_concurrent_refresh() (mutex/lock verification)

3. **test_config_oauth.py** (~100 LOC):
   - test_get_oauth_token_from_env()
   - test_get_oauth_token_from_keychain()
   - test_get_oauth_token_from_env_file()
   - test_set_oauth_token_keychain()
   - test_set_oauth_token_env_file()
   - test_detect_auth_method_api_key()
   - test_detect_auth_method_oauth()
   - test_clear_oauth_tokens()

4. **test_claude_client_oauth.py** (~50 LOC):
   - test_init_with_api_key_provider()
   - test_init_with_oauth_provider()
   - test_execute_task_with_oauth()
   - test_401_retry_with_token_refresh()
   - test_context_window_warning()
   - test_token_counting_accuracy()

**Mocking Strategy**:
- Mock token refresh endpoint with `responses` library
- Mock OS keychain with `unittest.mock`
- Mock Anthropic SDK with `unittest.mock`
- Mock ConfigManager for ClaudeClient tests

**4.2 Integration Testing Strategy**

**Test Coverage Target**: ≥70% end-to-end scenarios

**Test Modules** (~200 LOC):
1. **test_oauth_flow.py** (~100 LOC):
   - test_oauth_login_manual_mode()
   - test_oauth_login_stores_tokens_in_keychain()
   - test_oauth_login_fallback_to_env_file()
   - test_oauth_logout_clears_all_tokens()
   - test_oauth_status_displays_correct_info()
   - test_oauth_refresh_updates_tokens()

2. **test_token_refresh_integration.py** (~100 LOC):
   - test_token_expires_during_task() (long task with token expiry)
   - test_proactive_refresh_before_request()
   - test_reactive_refresh_on_401()
   - test_refresh_failure_prompts_reauth()
   - test_concurrent_tasks_with_token_refresh()

**Test Environment**:
- Mock OAuth server (Flask app with /oauth/token endpoint)
- Temporary keychain (macOS testing)
- Temporary .env file
- Isolated test database

**4.3 Security Testing Strategy**

**Security Test Scenarios** (from security architecture):
1. **test_no_token_in_logs()**: Scan logs for token patterns
2. **test_no_token_in_errors()**: Trigger all error paths, verify no credentials
3. **test_https_enforcement()**: Monitor network traffic, verify HTTPS
4. **test_token_cleanup_on_logout()**: Verify all storage locations cleared
5. **test_env_file_permissions()**: Verify .env file is 600 (user-only)
6. **test_token_sanitization_in_exception_traces()**: Trigger exceptions, verify no tokens in stack traces

**Security Tools**:
- Bandit: Static analysis for Python security issues
- Safety: Dependency vulnerability scanning
- TruffleHog: Secrets scanning in codebase and git history
- Automated log scanning: Regex for token patterns

**4.4 Load Testing Strategy**

**Load Test Scenarios**:
1. **Concurrent OAuth Requests** (10-100 tasks):
   - Spawn 100 concurrent tasks with OAuth authentication
   - Verify no race conditions (env var, token refresh)
   - Measure success rate (≥99%)

2. **Token Refresh Under Load**:
   - Expire token while 50 tasks are running
   - Verify automatic refresh and retry
   - Measure task completion rate (≥99%)

**Load Testing Tools**:
- Locust: Python-based load testing framework
- pytest-xdist: Parallel test execution

**4.5 End-to-End Testing**

**E2E Test Scenarios** (manual):
1. Interactive OAuth login (if browser-based flow implemented)
2. Keychain storage and retrieval on macOS
3. Keychain storage and retrieval on Linux (gnome-keyring)
4. .env file fallback when keychain unavailable
5. Token refresh during long-running task (>1 hour)
6. Context window warning for large inputs
7. Rate limit warning at 80% usage

**E2E Test Checklist**:
- [ ] OAuth login stores tokens in keychain
- [ ] Tokens persist across application restarts
- [ ] Token refresh happens automatically on 401
- [ ] Context window warnings display correctly
- [ ] oauth-status shows accurate information
- [ ] oauth-logout clears all tokens

#### Section 5: Deployment Checklist

**5.1 Pre-Deployment Validation**

**Code Quality**:
- [ ] All unit tests pass (≥90% coverage)
- [ ] All integration tests pass (≥70% coverage)
- [ ] Security tests pass (no token exposure)
- [ ] Load tests pass (≥99% success rate under load)
- [ ] Linting passes (Ruff, Black, Mypy)
- [ ] No critical security vulnerabilities (Bandit, Safety)
- [ ] No secrets in codebase (TruffleHog)

**Backward Compatibility**:
- [ ] All existing API key tests pass (100%)
- [ ] API key workflows unchanged (manual verification)
- [ ] No breaking changes in ClaudeClient API
- [ ] Existing agent templates work without modification

**Documentation**:
- [ ] Migration guide complete (API key → OAuth)
- [ ] OAuth setup guide complete (oauth-login workflow)
- [ ] Troubleshooting guide complete (common errors, resolutions)
- [ ] API reference updated (ClaudeClient with AuthProvider)
- [ ] Configuration reference updated (AuthConfig options)

**Security Validation**:
- [ ] Threat model reviewed (all threats mitigated)
- [ ] Encryption strategy verified (OS keychain encryption levels)
- [ ] Security testing complete (penetration tests, vulnerability scans)
- [ ] Audit logging implemented (≥15 security events)
- [ ] Compliance requirements met (GDPR, data privacy)

**5.2 Deployment Process**

**Version Bump**:
- [ ] Update version to v0.2.0 (minor version bump for new feature)
- [ ] Update CHANGELOG.md with OAuth support details

**Release Notes**:
- [ ] Document new OAuth authentication support
- [ ] Document new CLI commands (oauth-login, oauth-logout, oauth-status, oauth-refresh)
- [ ] Document context window warnings
- [ ] Document migration path for API key users
- [ ] Document known limitations (OAuth 200K context, rate limits)

**PyPI Package**:
- [ ] Build distribution packages (sdist, wheel)
- [ ] Test installation in clean environment
- [ ] Upload to PyPI
- [ ] Verify installation from PyPI

**Documentation Site**:
- [ ] Update README.md with OAuth setup quickstart
- [ ] Publish migration guide
- [ ] Publish OAuth setup guide
- [ ] Publish troubleshooting guide
- [ ] Update API reference

**5.3 Post-Deployment Monitoring**

**Metrics to Track** (first 30 days):
- [ ] OAuth vs API key usage ratio (target: ≥20% OAuth adoption)
- [ ] Token refresh success rate (target: ≥99.5%)
- [ ] Authentication failures (target: <5% of requests)
- [ ] Context window warnings (track frequency and auth method)
- [ ] Rate limit warnings (track frequency)

**Error Monitoring**:
- [ ] Auth failures (alert if >10/hour)
- [ ] Token refresh failures (alert if >5% rate)
- [ ] Context window exceeded errors (track and analyze)
- [ ] 429 rate limit errors (track and analyze)

**User Support**:
- [ ] Monitor support tickets (OAuth-related issues)
- [ ] Update troubleshooting guide based on common issues
- [ ] Collect user feedback (OAuth UX, pain points)

**Incident Response**:
- [ ] Define escalation path for security incidents (token exposure)
- [ ] Define rollback procedure (revert to v0.1.x if critical bug)
- [ ] Define hotfix process (patch OAuth bugs without full release)

#### Section 6: Migration Guide (Outline)

**6.1 For Existing API Key Users**

**No Action Required**:
- All existing API key workflows continue to work
- No configuration changes needed
- v0.2.0 is fully backward compatible

**Optional: Transition to OAuth**:
1. Obtain OAuth tokens (see OAuth Setup Guide)
2. Run `abathur config oauth-login --manual`
3. Enter access token, refresh token, expires_in
4. Verify with `abathur config oauth-status`
5. (Optional) Remove API key: `unset ANTHROPIC_API_KEY`

**6.2 For New Users (OAuth)**

**OAuth Setup**:
1. Obtain OAuth tokens:
   - Option A: Use Claude Code CLI and extract tokens from keychain
   - Option B: (Future) Interactive browser-based OAuth flow
2. Run `abathur config oauth-login --manual`
3. Enter tokens when prompted
4. Verify with `abathur config oauth-status`
5. Start using Abathur: `abathur spawn task "..."`

**6.3 Troubleshooting Common Issues**

**Token Refresh Failures**:
- Error: "OAuth token expired. Refresh failed."
- Cause: Refresh token expired or revoked
- Solution: Re-authenticate with `abathur config oauth-login`

**Context Window Warnings**:
- Warning: "Task input (185K tokens) approaching OAuth limit (200K tokens)"
- Cause: Large task input with OAuth authentication
- Solution: Use API key authentication for large tasks or reduce input size

**Rate Limit Exceeded**:
- Error: "Rate limit exceeded: 50/50 prompts used"
- Cause: Hit OAuth rate limit (Max 5x: 50-200 prompts/5h)
- Solution: Wait for window reset or use API key authentication

#### Section 7: Success Metrics

**Development Metrics**:
- [ ] Implementation completed in 4 weeks (110 developer hours)
- [ ] Test coverage ≥90% (unit + integration)
- [ ] Zero critical bugs in production (first 30 days)
- [ ] Zero security incidents (first 90 days)

**Quality Metrics**:
- [ ] All NFRs met (31 non-functional requirements)
- [ ] All FRs met (30 functional requirements)
- [ ] Clean Architecture preserved (dependency audit)
- [ ] Documentation complete (≥3 user guides)

**Adoption Metrics** (first 30 days):
- [ ] ≥20% of users adopt OAuth
- [ ] ≥95% of OAuth users successful (no support tickets)
- [ ] Token refresh success rate ≥99.5%
- [ ] Zero token exposure incidents

#### Section 8: Dependencies and Assumptions

**Dependencies**:
- Anthropic SDK (^0.18.0) supports ANTHROPIC_AUTH_TOKEN
- Token refresh endpoint stable (`https://console.anthropic.com/v1/oauth/token`)
- OS keychain available (macOS Keychain, Linux Secret Service)
- httpx library supports HTTPS enforcement and certificate validation

**Assumptions**:
- Users have access to OAuth tokens (via Claude Code or future interactive flow)
- Token refresh endpoint request/response format remains stable
- OS keychain encryption meets security requirements (AES-256 equivalent)
- Development team has 110 hours available over 4 weeks (1 developer full-time)

---

### 3.2 Success Criteria

**Document Completeness**:
- [ ] All 8 required sections present
- [ ] 4-week phased plan with week-by-week milestones
- [ ] ≥10 risks identified with likelihood/impact/mitigation
- [ ] Testing strategy covers unit/integration/E2E/load/security
- [ ] ≥20 deployment validation steps

**Quality Standards**:
- [ ] Each phase has clear milestone and deliverables
- [ ] Each phase has LOC estimate and developer hours
- [ ] All risks from Phase 2 validation report addressed
- [ ] Testing strategy achieves ≥90% code coverage target
- [ ] Deployment checklist covers pre/during/post deployment

**Traceability**:
- [ ] Every FR requirement maps to implementation phase
- [ ] Every NFR requirement maps to testing strategy
- [ ] Every integration point maps to phased plan
- [ ] Every risk has mitigation phase

---

## 4. Phasing Guidance

### 4.1 Critical Path Analysis

**Phase 1 → Phase 2 → Phase 3 → Phase 4** (sequential dependencies)

**Critical Path**:
1. AuthProvider interface (Phase 1) → OAuthAuthProvider (Phase 2)
2. OAuthAuthProvider (Phase 2) → ClaudeClient integration (Phase 3)
3. ClaudeClient integration (Phase 3) → End-to-end testing (Phase 4)

**Parallel Opportunities**:
- Documentation can start in Phase 3 (while integration is happening)
- Security tests can be written in Phase 2 (before integration)
- Load tests can be designed in Phase 1 (mock implementations)

### 4.2 Resource Allocation

**Assumed Resources**:
- 1 developer full-time (40 hours/week × 4 weeks = 160 hours available)
- 110 hours allocated to implementation + testing
- 30 hours allocated to documentation
- 20 hours buffer for contingencies

**If Resources Change**:
- 2 developers: Parallel work on Phases 2 and 3 (complete in 3 weeks)
- 0.5 developer: Extend to 8 weeks (half-time allocation)

### 4.3 Milestone Checkpoints

**Week 1 Checkpoint**:
- AuthProvider abstraction complete
- All existing API key tests pass
- Phase 2 can begin

**Week 2 Checkpoint**:
- OAuth token lifecycle working
- Token refresh tested (unit tests)
- Phase 3 can begin

**Week 3 Checkpoint**:
- End-to-end OAuth flow working
- Integration tests pass
- Phase 4 can begin

**Week 4 Checkpoint**:
- All tests pass (≥90% coverage)
- Documentation complete
- Ready for deployment

---

## 5. Risk Assessment Template

**Risk Entry Format**:
```markdown
### RISK-[CATEGORY]-[NUMBER]: [Risk Title]

**Description**: [Clear description of the risk]

**Likelihood**: Low / Medium / High
- [Justification for likelihood rating]

**Impact**: Low / Medium / High
- [Justification for impact rating]

**Risk Level**: [Likelihood × Impact]
- Low × Low = LOW
- Low × Medium = LOW
- Low × High = MEDIUM
- Medium × Low = LOW
- Medium × Medium = MEDIUM
- Medium × High = HIGH
- High × Low = MEDIUM
- High × Medium = HIGH
- High × High = CRITICAL

**Mitigation Strategy**: [Specific actions to mitigate risk]

**Mitigation Phase**: [Which implementation phase addresses this risk]

**Residual Risk**: [Risk remaining after mitigation]

**Contingency Plan**: [What to do if risk materializes]
```

---

## 6. Testing Strategy Template

**Test Module Template**:
```python
# tests/unit/test_[module].py

import pytest
from unittest.mock import Mock, patch
from abathur.infrastructure.oauth_auth import OAuthAuthProvider

@pytest.fixture
def mock_oauth_provider():
    """Mock OAuth provider for testing."""
    return OAuthAuthProvider(
        access_token="test_access_token",
        refresh_token="test_refresh_token",
        expires_at=datetime.now() + timedelta(hours=1),
        config_manager=Mock()
    )

def test_token_refresh_success(mock_oauth_provider):
    """Test successful token refresh."""
    # Arrange
    with patch('httpx.AsyncClient.post') as mock_post:
        mock_post.return_value.status_code = 200
        mock_post.return_value.json.return_value = {
            "access_token": "new_access_token",
            "refresh_token": "new_refresh_token",
            "expires_in": 3600
        }

        # Act
        result = await mock_oauth_provider.refresh_credentials()

        # Assert
        assert result is True
        assert mock_oauth_provider.access_token == "new_access_token"
        assert mock_oauth_provider.refresh_token == "new_refresh_token"
```

---

## 7. Deployment Checklist Template

**Checklist Item Format**:
```markdown
- [ ] **[Category]**: [Item Description]
  - **Validation Method**: [How to verify this item]
  - **Owner**: [Who is responsible]
  - **Priority**: Critical / High / Medium / Low
  - **Estimated Time**: [Minutes/Hours]
```

**Example**:
```markdown
- [ ] **Code Quality**: All unit tests pass
  - **Validation Method**: Run `pytest tests/unit/ --cov=abathur`
  - **Owner**: Developer
  - **Priority**: Critical
  - **Estimated Time**: 5 minutes
```

---

## 8. Migration Guide Template

**Section Format**:
```markdown
### [User Scenario]

**For**: [Target audience]

**Steps**:
1. [Step 1]
2. [Step 2]
3. [Step 3]

**Verification**:
- [How to verify successful migration]

**Troubleshooting**:
- **Issue**: [Common problem]
  - **Cause**: [Why it happens]
  - **Solution**: [How to fix]
```

---

## 9. Deliverable Timeline

**Week 6 Breakdown**:

**Day 1: Phased Implementation Plan**
- Define 4-week phases with milestones
- Estimate LOC and developer hours per phase
- Identify dependencies and critical path
- Create resource allocation plan

**Day 2-3: Risk Assessment**
- Identify ≥10 risks (technical, security, operational, migration)
- Assess likelihood and impact for each risk
- Define mitigation strategies
- Assign risks to implementation phases

**Day 4: Testing Strategy**
- Define unit test plan (~400 LOC)
- Define integration test plan (~200 LOC)
- Define E2E test scenarios
- Define load testing approach
- Define security testing requirements
- Map tests to requirements and phases

**Day 5: Deployment Checklist**
- Define ≥20 pre-deployment validation steps
- Define deployment process
- Define post-deployment monitoring
- Create incident response plan

**Day 6: Migration Guide & Documentation**
- Outline migration guide (API key → OAuth)
- Outline OAuth setup guide
- Outline troubleshooting guide
- Identify documentation dependencies

**Day 7: Review and Refinement**
- Review all sections for completeness
- Validate against success criteria
- Add success metrics section
- Finalize deliverable

---

## 10. Collaboration with Other Agents

**Inputs from prd-security-specialist**:
- Security testing plan informs testing strategy (Section 4.3)
- Threat model informs risk assessment (Section 3)
- Audit logging informs post-deployment monitoring (Section 5.3)

**Outputs to prd-documentation-specialist** (Phase 4):
- Migration guide outline (Section 6)
- OAuth setup guide outline
- Troubleshooting guide outline
- Phased plan for PRD timeline section

---

## 11. Validation Criteria

**Self-Check Before Submission**:
- [ ] All 8 required sections present and complete
- [ ] 4-week phased plan with milestones and LOC estimates
- [ ] ≥10 risks with likelihood/impact/mitigation
- [ ] Testing strategy covers all test types (unit/integration/E2E/load/security)
- [ ] ≥20 deployment validation steps
- [ ] Migration guide outline complete
- [ ] Success metrics defined
- [ ] Document length: 2000-3000 lines (comprehensive but actionable)

**prd-project-orchestrator Validation**:
- Phased plan feasibility (4 weeks achievable with resources)
- Risk assessment completeness (all Phase 2 risks addressed)
- Testing strategy adequacy (≥90% coverage achievable)
- Deployment checklist thoroughness (pre/during/post steps)
- Migration guide usability (clear user-facing instructions)

---

**END OF TASK SPECIFICATION: prd-implementation-roadmap-specialist**

**Status**: Ready for agent invocation (after security deliverable)
**Expected Completion**: End of Week 6
**Next Agent**: prd-documentation-specialist (Phase 4, depends on this deliverable)
