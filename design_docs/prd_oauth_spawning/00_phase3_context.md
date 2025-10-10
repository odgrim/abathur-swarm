# Phase 3 Context Summary - OAuth-Based Agent Spawning PRD

**Date**: October 9, 2025
**Phase**: Phase 3 - Security Architecture & Implementation Roadmap
**Source**: Phase 2 Validation Gate Output
**Status**: Ready for Agent Invocation

---

## 1. Executive Summary for Phase 3 Agents

### 1.1 Project State

**Phase 2 Status**: ✅ **APPROVED** - Both deliverables passed validation with 9.5/10 quality scores

**Critical Achievements**:
- **Issue H1 (SDK OAuth Support)**: RESOLVED - Anthropic SDK (^0.18.0) confirmed to support `ANTHROPIC_AUTH_TOKEN`
- **Issue H2 (Token Refresh Endpoint)**: CONFIRMED - Endpoint URL and request/response format validated
- **Requirements**: 30 Functional + 31 Non-Functional (61 total, exceeding all targets)
- **Architecture**: AuthProvider abstraction with zero breaking changes, ~600 LOC scope

**Phase 3 Objective**: Define security architecture and implementation roadmap to prepare for development handoff.

### 1.2 Phase 3 Deliverables Expected

**Deliverable 1: Security Architecture** (prd-security-specialist)
- **File**: `05_security_architecture.md`
- **Content**:
  1. Threat model for OAuth token lifecycle
  2. Encryption strategy verification (OS keychain)
  3. Security testing requirements
  4. Audit logging specification
  5. Compliance considerations (GDPR, data privacy)

**Deliverable 2: Implementation Roadmap** (prd-implementation-roadmap-specialist)
- **File**: `06_implementation_roadmap.md`
- **Content**:
  1. Phased implementation plan (4 weeks)
  2. Milestone definitions with deliverables
  3. Risk assessment and mitigation strategies
  4. Testing strategy (unit, integration, E2E)
  5. Deployment checklist

**Deliverable 3: PRD Master Document** (prd-documentation-specialist - Phase 4)
- **File**: `07_prd_master_document.md`
- **Content**: Consolidated PRD integrating all sections

---

## 2. Phase 2 Key Findings Summary

### 2.1 Technical Requirements Highlights

**30 Functional Requirements** across 6 categories:
1. **FR-AUTH (4)**: API key + OAuth dual-mode, auto-detection, manual override
2. **FR-TOKEN (5)**: Automatic refresh, proactive expiry, secure storage, persistence
3. **FR-CONTEXT (4)**: 200K/1M limit detection, token counting, user warnings
4. **FR-RATE (4)**: Usage tracking, warning thresholds, 429 handling, multi-tier support
5. **FR-CLI (5)**: oauth-login/logout/status/refresh commands, backward compatibility
6. **FR-ERROR (5)**: Actionable errors, retry logic, no fallback, graceful degradation

**31 Non-Functional Requirements** across 7 categories:
1. **NFR-PERF (4)**: Token refresh <100ms, auth detection <10ms, token counting <50ms
2. **NFR-SEC (5)**: AES-256 encryption, no logging, error sanitization, HTTPS-only, revocation
3. **NFR-REL (5)**: 99.5% refresh success, 95% retry success, 99% long task completion
4. **NFR-USE (5)**: Zero config for API key, ≤3 OAuth commands, actionable errors, clarity
5. **NFR-OBS (5)**: 100% auth/token event logging, usage/error metrics, performance tracking
6. **NFR-MAINT (5)**: Clean Architecture, 90% test coverage, documentation, ≤1 dependency
7. **NFR-COMPAT (2)**: Python 3.10+, SDK ^0.18.0

**Critical Security Requirements** (for prd-security-specialist):
- **NFR-SEC-001**: Encrypted token storage (OS keychain with AES-256 equivalent)
- **NFR-SEC-002**: Zero token logging in plaintext
- **NFR-SEC-003**: Error message sanitization (no credentials)
- **NFR-SEC-004**: HTTPS-only token transmission
- **NFR-SEC-005**: Immediate token revocation on logout

### 2.2 System Architecture Highlights

**AuthProvider Abstraction**:
- **Interface**: 5 methods (get_credentials, refresh_credentials, is_valid, get_auth_method, get_context_limit)
- **Implementations**: APIKeyAuthProvider (simple), OAuthAuthProvider (complex with refresh)
- **Design**: Clean separation, testable, extensible

**Token Lifecycle**:
- **Proactive Refresh**: Refresh 5 minutes before expiry
- **Reactive Refresh**: Refresh on 401 Unauthorized (max 3 retries)
- **Storage**: OS keychain (priority 1), environment variables (priority 2), .env file (priority 3)
- **Rotation**: Server may return new refresh_token; update both tokens

**Integration Points**:
- **ClaudeClient** (MAJOR): ~150 LOC changes (accept AuthProvider, 401 retry loop, context validation)
- **ConfigManager** (MODERATE): ~140 LOC additions (OAuth token methods, auto-detection)
- **CLI** (MODERATE): ~190 LOC additions (oauth-login/logout/status/refresh commands)
- **Core Orchestration** (NONE): Zero changes (AgentExecutor, SwarmOrchestrator isolated via DI)

**Architecture Diagrams**:
1. Component Diagram: AuthProvider abstraction and implementations
2. Sequence Diagram: OAuth flow (login → token refresh → API request)
3. Class Diagram: Interfaces and relationships
4. Integration Diagram: File:line modification points
5. Data Flow Diagram: OAuth token lifecycle

**Estimated Implementation**:
- **New Files**: 4 (auth_provider.py, api_key_auth.py, oauth_auth.py, exceptions.py)
- **Modified Files**: 3 (claude_client.py, config.py, main.py)
- **LOC**: ~600 (400 new, 200 modified)
- **Test LOC**: ~600 (unit + integration)
- **Total**: ~1,200 LOC
- **Timeline**: 4 weeks (suggested phasing)

### 2.3 Resolved Critical Issues

**Issue H1: SDK OAuth Support**
- **Resolution**: VERIFIED - Anthropic SDK (^0.18.0) supports OAuth via `ANTHROPIC_AUTH_TOKEN` environment variable
- **Evidence**: SDK documentation, community usage patterns, code analysis
- **Implementation**: Set `ANTHROPIC_AUTH_TOKEN` env var → SDK uses Bearer token authentication
- **No Custom HTTP Client Needed**: Official SDK fully supports OAuth

**Issue H2: Token Refresh Endpoint**
- **Resolution**: CONFIRMED - `https://console.anthropic.com/v1/oauth/token`
- **Source**: Claude Code CLI implementation, community-validated
- **Request**: `POST {grant_type: "refresh_token", refresh_token, client_id: "9d1c250a-e61b-44d9-88ed-5944d1962f5e"}`
- **Response**: `{access_token, refresh_token, expires_in}`
- **Caveat**: Not officially documented; fallback to manual re-auth if refresh fails

---

## 3. Context for prd-security-specialist

### 3.1 Objective

Define comprehensive security architecture for OAuth token lifecycle, addressing threat vectors, encryption strategies, and security testing requirements.

### 3.2 Security Requirements to Address

**NFR-SEC-001: Encrypted Token Storage**
- **Requirement**: All tokens stored using OS-level encryption (AES-256 or equivalent)
- **Architecture**: ConfigManager.set_oauth_token() uses OS keychain (macOS Keychain, Linux Secret Service)
- **Task**: Verify encryption level on macOS and Linux, define fallback security for .env file

**NFR-SEC-002: No Token Logging**
- **Requirement**: 0 occurrences of credentials in log files
- **Architecture**: Structured logging with token value exclusion
- **Task**: Define log sanitization rules, implement automated log scanning for token patterns

**NFR-SEC-003: Error Message Sanitization**
- **Requirement**: 0 credentials exposed in error messages
- **Architecture**: Custom exception hierarchy with remediation steps
- **Task**: Define error message templates, implement sanitization in exception constructors

**NFR-SEC-004: HTTPS-Only Token Transmission**
- **Requirement**: 100% of token refresh requests use HTTPS
- **Architecture**: OAuthAuthProvider.refresh_credentials() uses `https://console.anthropic.com/v1/oauth/token`
- **Task**: Verify httpx client enforces HTTPS, define network security testing

**NFR-SEC-005: Token Revocation on Logout**
- **Requirement**: 100% of tokens cleared within 100ms of logout
- **Architecture**: config_oauth_logout() clears keychain and .env file
- **Task**: Define cleanup verification tests, handle partial cleanup scenarios

### 3.3 Threat Surface Analysis Required

**Token Lifecycle Stages**:
1. **Token Acquisition**: oauth-login command → Manual token input or browser OAuth flow
   - Threat: Man-in-the-middle during browser OAuth (if interactive flow implemented)
   - Threat: Token exposure in CLI history
2. **Token Storage**: Keychain → Environment variables → .env file
   - Threat: Keychain access without user consent
   - Threat: .env file committed to version control
   - Threat: Environment variable exposure in process listings
3. **Token Refresh**: POST to `https://console.anthropic.com/v1/oauth/token`
   - Threat: Network interception (mitigated by HTTPS)
   - Threat: Token exposure in network logs
   - Threat: Refresh token theft from storage
4. **Token Usage**: Set `ANTHROPIC_AUTH_TOKEN` env var → SDK uses Bearer token
   - Threat: Token exposure in SDK internal logs
   - Threat: Token leakage via exception stack traces
5. **Token Revocation**: oauth-logout command → Clear keychain and .env
   - Threat: Incomplete token cleanup (tokens remain in memory)
   - Threat: Revoked tokens still valid on server (if no server-side revocation)

### 3.4 Architecture Components to Secure

**OAuthAuthProvider** (`infrastructure/oauth_auth.py`):
- Token refresh logic (lines ~180-235 in architecture spec)
- Proactive refresh (5-min buffer)
- Reactive refresh (401 retry loop)
- Token rotation handling

**ConfigManager OAuth Methods** (`infrastructure/config.py`):
- `get_oauth_token()`: Retrieves tokens from keychain/env/.env
- `set_oauth_token()`: Stores tokens securely
- `clear_oauth_tokens()`: Cleanup on logout

**ClaudeClient** (`application/claude_client.py`):
- `_configure_sdk_auth()`: Sets `ANTHROPIC_AUTH_TOKEN` env var
- Error handling: Catches 401, triggers refresh, retries

**CLI Commands** (`cli/main.py`):
- `config_oauth_login()`: Token acquisition (manual mode)
- `config_oauth_logout()`: Token revocation
- `config_oauth_status()`: Display auth status (must not log tokens)

### 3.5 Security Testing Requirements

**Penetration Testing Scope**:
1. Token storage security (keychain access, .env file permissions)
2. Token transmission security (HTTPS enforcement, network interception)
3. Token exposure vectors (logs, error messages, exception traces)
4. Token refresh endpoint security (CSRF, replay attacks)

**Vulnerability Scanning**:
1. Static analysis: Scan code for token logging patterns
2. Dependency scan: Check httpx and anthropic SDK for known vulnerabilities
3. Secrets scanning: Detect hardcoded tokens in codebase

**Security Testing Plan Required**:
1. Unit tests: Token sanitization in logs and errors
2. Integration tests: Token refresh with malicious responses
3. E2E tests: Complete OAuth lifecycle security validation
4. Audit: Manual security review of token handling code

### 3.6 Compliance Considerations

**GDPR / Data Privacy**:
- Tokens may contain user-identifiable information
- Storage location (keychain vs .env) affects data residency
- Token retention policy (revocation on logout, expiry handling)

**Audit Logging Requirements**:
- Log all authentication events (success/failure)
- Log token lifecycle events (refresh, expiration, revocation)
- Exclude token values from audit logs
- Retain logs for security incident investigation

### 3.7 Deliverable Expectations

**05_security_architecture.md** must include:
1. **Threat Model**:
   - Threat actors (insider, external attacker, compromised process)
   - Attack vectors (token theft, interception, exposure)
   - STRIDE analysis (Spoofing, Tampering, Repudiation, Information Disclosure, Denial of Service, Elevation of Privilege)
2. **Encryption Strategy**:
   - OS keychain verification (macOS: Keychain Access, Linux: Secret Service)
   - .env file security (fallback when keychain unavailable)
   - In-transit encryption (HTTPS verification)
3. **Security Testing Plan**:
   - Penetration testing scope and methodology
   - Vulnerability scanning tools and schedule
   - Security test scenarios (token theft, exposure, interception)
4. **Audit Logging Specification**:
   - Security events to log (auth failures, token refresh failures, logout)
   - Log format and retention policy
   - Monitoring and alerting rules
5. **Security Controls Summary**:
   - Preventive controls (encryption, HTTPS)
   - Detective controls (logging, monitoring)
   - Corrective controls (token revocation, re-authentication)

**Success Criteria**:
- [ ] Threat model covers all token lifecycle stages
- [ ] Encryption strategy verified on macOS and Linux
- [ ] Security testing plan includes penetration testing and vulnerability scanning
- [ ] Audit logging specification defines events, format, retention
- [ ] All NFR-SEC requirements traceable to security controls

---

## 4. Context for prd-implementation-roadmap-specialist

### 4.1 Objective

Define phased implementation roadmap with 4-week timeline, milestone definitions, risk assessment, testing strategy, and deployment checklist.

### 4.2 Implementation Scope

**New Components** (4 files):
1. **domain/ports/auth_provider.py** (~50 LOC):
   - AuthProvider interface (5 methods)
   - Base for APIKeyAuthProvider and OAuthAuthProvider

2. **infrastructure/api_key_auth.py** (~30 LOC):
   - APIKeyAuthProvider implementation
   - Wrapper around existing API key logic
   - Simple, no expiry, no refresh

3. **infrastructure/oauth_auth.py** (~200 LOC):
   - OAuthAuthProvider implementation
   - Proactive refresh (5-min buffer)
   - Reactive refresh (401 retry loop)
   - Token rotation handling
   - Persistent storage via ConfigManager

4. **infrastructure/exceptions.py** (~50 LOC):
   - Custom exception hierarchy
   - AuthenticationError base class
   - OAuthTokenExpiredError, OAuthRefreshError, APIKeyInvalidError
   - ContextWindowExceededError

**Modified Components** (3 files):
1. **application/claude_client.py** (~150 LOC modified):
   - Accept AuthProvider in constructor (lines 18-43)
   - Add 401 retry loop with token refresh (lines 45-117)
   - Add _configure_sdk_auth() method (~20 LOC)
   - Add _estimate_tokens() method (~10 LOC)
   - Add context window validation (~20 LOC)

2. **infrastructure/config.py** (~140 LOC added):
   - Add get_oauth_token() method (~40 LOC)
   - Add set_oauth_token() method (~30 LOC)
   - Add detect_auth_method() method (~20 LOC)
   - Add clear_oauth_tokens() method (~30 LOC)
   - Add AuthConfig model to Config (~20 LOC)

3. **cli/main.py** (~190 LOC added):
   - Modify _get_services() to initialize AuthProvider (~40 LOC)
   - Add config_oauth_login() command (~60 LOC)
   - Add config_oauth_logout() command (~20 LOC)
   - Add config_oauth_status() command (~40 LOC)
   - Add config_oauth_refresh() command (~30 LOC)

**Testing** (~600 LOC):
- Unit tests: ~400 LOC
  - test_auth_provider.py (100 LOC)
  - test_oauth_auth.py (150 LOC)
  - test_config_oauth.py (100 LOC)
  - test_claude_client_oauth.py (50 LOC)
- Integration tests: ~200 LOC
  - test_oauth_flow.py (100 LOC)
  - test_token_refresh_integration.py (100 LOC)

**Total Estimated**: ~1,200 LOC (600 implementation + 600 tests)

### 4.3 Integration Points from Phase 2

**MAJOR Changes**:
- **ClaudeClient.__init__** (application/claude_client.py:18-43):
  - Accept AuthProvider parameter (optional, backward compatible)
  - Initialize auth provider from api_key or auth_provider
- **ClaudeClient.execute_task** (application/claude_client.py:45-117):
  - Add 401 retry loop with token refresh
  - Add context window validation
  - Add token counting

**MODERATE Changes**:
- **ConfigManager** (infrastructure/config.py:162-221):
  - Add OAuth token retrieval methods
  - Add auth method auto-detection
- **CLI _get_services** (cli/main.py:48):
  - Detect auth method
  - Initialize appropriate AuthProvider
- **CLI config commands** (cli/main.py:570-586):
  - Add OAuth-specific commands

**NO CHANGES**:
- AgentExecutor (application/agent_executor.py)
- SwarmOrchestrator (application/swarm_orchestrator.py)
- TaskCoordinator (application/task_coordinator.py)
- Database (infrastructure/database.py)

### 4.4 Suggested Implementation Phases

**Phase 1 (Week 1): Foundation**
- Create AuthProvider interface (domain/ports/auth_provider.py)
- Implement APIKeyAuthProvider (infrastructure/api_key_auth.py)
- Create custom exception hierarchy (infrastructure/exceptions.py)
- Unit tests for AuthProvider and APIKeyAuthProvider
- **Milestone**: API key authentication refactored to use AuthProvider abstraction

**Phase 2 (Week 2): OAuth Core**
- Implement OAuthAuthProvider (infrastructure/oauth_auth.py)
- Add ConfigManager OAuth methods (get_oauth_token, set_oauth_token, detect_auth_method)
- Unit tests for OAuthAuthProvider and ConfigManager OAuth methods
- **Milestone**: OAuth token lifecycle fully implemented and tested

**Phase 3 (Week 3): CLI Integration**
- Modify ClaudeClient to accept AuthProvider (application/claude_client.py)
- Add 401 retry loop and context window validation to ClaudeClient
- Modify CLI _get_services() to initialize AuthProvider (cli/main.py)
- Add OAuth CLI commands (oauth-login, oauth-logout, oauth-status, oauth-refresh)
- Integration tests for OAuth flow
- **Milestone**: End-to-end OAuth authentication working via CLI

**Phase 4 (Week 4): Testing & Documentation**
- Integration tests: Full OAuth lifecycle (login → task execution → token refresh → logout)
- Load testing: Concurrent requests with OAuth authentication
- Security testing: Token exposure, error message sanitization, HTTPS enforcement
- Documentation: Migration guide, OAuth setup guide, troubleshooting guide
- **Milestone**: Production-ready implementation with comprehensive tests and docs

### 4.5 Risk Assessment Required

**Technical Risks**:
1. **SDK Concurrent Requests**: Setting ANTHROPIC_AUTH_TOKEN env var may have race conditions
   - Likelihood: Medium
   - Impact: Medium (task failures under load)
   - Mitigation: Thread-safe env var manipulation, load testing
2. **Token Refresh Race Conditions**: Multiple tasks may trigger refresh simultaneously
   - Likelihood: Medium
   - Impact: Low (redundant refreshes, not failures)
   - Mitigation: Mutex/lock around refresh_credentials()
3. **Context Window Estimation Inaccuracy**: 4-char approximation may underestimate
   - Likelihood: Low
   - Impact: Low (conservative bias acceptable)
   - Mitigation: User warnings, configurable handling modes

**Security Risks**:
1. **Token Exposure in Logs**: Accidental logging of token values
   - Likelihood: Low (automated scanning mitigates)
   - Impact: High (credential compromise)
   - Mitigation: Log sanitization, automated scanning, security tests
2. **Insecure .env Fallback**: Tokens stored in plaintext .env file
   - Likelihood: Medium (when keychain unavailable)
   - Impact: Medium (file permissions mitigate)
   - Mitigation: User warning, recommend keychain, .gitignore enforcement
3. **Token Refresh Endpoint Changes**: Anthropic changes endpoint without notice
   - Likelihood: Low (widely used in Claude Code)
   - Impact: High (all token refreshes fail)
   - Mitigation: 3-retry logic, fallback to manual re-auth, monitoring

**Operational Risks**:
1. **Token Refresh Endpoint Availability**: Service downtime or rate limiting
   - Likelihood: Low
   - Impact: Medium (users cannot refresh tokens)
   - Mitigation: 3-retry logic with backoff, fallback to manual re-auth
2. **OS Keychain Unavailable**: Keychain access denied or service unavailable
   - Likelihood: Medium (on Linux systems)
   - Impact: Low (fallback to .env file)
   - Mitigation: Graceful fallback, user warning
3. **Rate Limiting**: OAuth users hit 50-200 prompt limits unexpectedly
   - Likelihood: Medium (for heavy users)
   - Impact: Low (graceful error handling)
   - Mitigation: Usage tracking, warnings at 80%, clear error messages

**Migration Risks**:
1. **Breaking Changes for Edge Cases**: Unusual API key formats or configurations
   - Likelihood: Low
   - Impact: Medium (user workflow disruption)
   - Mitigation: Comprehensive backward compatibility tests, migration guide
2. **Backward Compatibility Testing**: Ensuring API key workflows unchanged
   - Likelihood: Low (well-isolated changes)
   - Impact: High (existing users affected)
   - Mitigation: 100% test coverage for existing API key scenarios

### 4.6 Testing Strategy Required

**Unit Tests** (~400 LOC):
- AuthProvider interface mocking
- APIKeyAuthProvider: get_credentials, is_valid, get_auth_method
- OAuthAuthProvider: token refresh, expiry detection, proactive/reactive refresh
- ConfigManager: OAuth token storage/retrieval, auth method detection
- ClaudeClient: 401 retry loop, context window validation, token counting
- Exception hierarchy: Error message sanitization, remediation steps

**Integration Tests** (~200 LOC):
- End-to-end OAuth flow: oauth-login → task execution → token refresh → oauth-logout
- Token refresh integration: Expire token → Trigger refresh → Verify new token
- Context window integration: Large input → Warning triggered → User notified
- Rate limit integration: 429 error → Graceful handling → User guidance

**E2E Tests** (manual):
- Interactive OAuth login (browser-based, if implemented)
- Keychain storage and retrieval (macOS and Linux)
- .env file fallback (when keychain unavailable)
- Token refresh during long-running task

**Load Tests**:
- Concurrent requests with OAuth authentication (10-100 concurrent tasks)
- Token refresh under load (multiple tasks triggering refresh simultaneously)
- SDK environment variable race conditions

**Security Tests**:
- Token sanitization: Scan logs and errors for token patterns
- HTTPS enforcement: Monitor network traffic for HTTP requests
- Keychain encryption: Verify token storage encryption level
- Error message sanitization: Trigger all error paths, verify no credentials

### 4.7 Deployment Checklist Required

**Pre-Deployment**:
- [ ] All unit tests pass (≥90% coverage)
- [ ] All integration tests pass
- [ ] Security tests pass (no token exposure)
- [ ] Load tests pass (concurrent auth under load)
- [ ] Documentation complete (migration guide, OAuth setup, troubleshooting)
- [ ] Backward compatibility validated (API key workflows unchanged)

**Deployment**:
- [ ] Release notes published (OAuth support, migration guide)
- [ ] Version bump to v0.2.0
- [ ] PyPI package published
- [ ] Documentation site updated

**Post-Deployment**:
- [ ] Monitor metrics: OAuth vs API key usage, token refresh success rate
- [ ] Monitor errors: Auth failures, token refresh failures, context window warnings
- [ ] User support: Respond to OAuth setup questions, update troubleshooting guide
- [ ] Collect feedback: User experience with OAuth, pain points, feature requests

### 4.8 Deliverable Expectations

**06_implementation_roadmap.md** must include:
1. **Phased Implementation Plan**:
   - Week-by-week breakdown
   - Milestones with deliverables
   - Dependencies between phases
   - Resource allocation (developer hours)
2. **Risk Assessment**:
   - Technical, security, operational, migration risks
   - Likelihood and impact scores
   - Mitigation strategies
3. **Testing Strategy**:
   - Unit test plan (400 LOC)
   - Integration test plan (200 LOC)
   - E2E test scenarios
   - Load testing approach
   - Security testing requirements
4. **Deployment Checklist**:
   - Pre-deployment validation steps
   - Deployment process
   - Post-deployment monitoring
5. **Migration Guide** (outline):
   - API key users transitioning to OAuth
   - Step-by-step OAuth setup
   - Troubleshooting common issues

**Success Criteria**:
- [ ] 4-week phased plan with clear milestones
- [ ] Risk assessment with likelihood/impact/mitigation
- [ ] Testing strategy covers unit/integration/E2E/load/security
- [ ] Deployment checklist includes pre/during/post steps
- [ ] Migration guide provides user-facing instructions

---

## 5. Cross-Phase Dependencies

### 5.1 Security → Implementation Dependencies

**Security Deliverables Feeding Implementation**:
1. **Threat Model** → Security test scenarios (validate mitigations)
2. **Encryption Strategy** → Token storage implementation (keychain vs .env decision logic)
3. **Audit Logging Spec** → Logging implementation (security event logging)
4. **Security Testing Plan** → Testing strategy (penetration tests, vulnerability scans)

**Implementation Constraints from Security**:
- Token storage must use verified encryption (OS keychain or .env with warnings)
- All logs must be sanitized (no token values)
- All network communication must use HTTPS
- Token cleanup must be complete (no residual tokens)

### 5.2 Implementation → Documentation Dependencies

**Implementation Deliverables Feeding Documentation** (Phase 4):
1. **Phased Plan** → Implementation timeline in PRD
2. **Risk Assessment** → Known issues and limitations section
3. **Testing Strategy** → Testing approach in PRD
4. **Migration Guide** → User-facing documentation

**Documentation Requirements**:
- PRD consolidates all Phase 2-3 sections
- Executive summary for stakeholders
- Implementation-ready specification for developers
- User-facing guides (OAuth setup, troubleshooting)

---

## 6. Success Criteria for Phase 3

### 6.1 Security Deliverable (05_security_architecture.md)

**Must Include**:
- [ ] Threat model with STRIDE analysis
- [ ] Encryption strategy verified on macOS and Linux
- [ ] Security testing plan (penetration, vulnerability, audit)
- [ ] Audit logging specification (events, format, retention)
- [ ] All NFR-SEC-001 through NFR-SEC-005 traceable to controls

**Quality Gates**:
- [ ] Threat model covers all 5 token lifecycle stages
- [ ] Encryption strategy addresses keychain and .env fallback
- [ ] Security testing plan includes specific tools and methodology
- [ ] Audit logging specification defines ≥10 security events

### 6.2 Implementation Roadmap Deliverable (06_implementation_roadmap.md)

**Must Include**:
- [ ] 4-week phased plan with week-by-week milestones
- [ ] Risk assessment (≥10 risks with likelihood/impact/mitigation)
- [ ] Testing strategy (unit/integration/E2E/load/security)
- [ ] Deployment checklist (≥20 validation steps)
- [ ] Migration guide outline

**Quality Gates**:
- [ ] Each phase has clear milestone and deliverables
- [ ] All risks from Phase 2 validation report addressed
- [ ] Testing strategy achieves ≥90% code coverage target
- [ ] Deployment checklist covers pre/during/post deployment

### 6.3 Phase 3 Overall

**Completion Criteria**:
- [ ] Both deliverables approved by prd-project-orchestrator
- [ ] All Phase 3 success criteria met
- [ ] Cross-phase dependencies documented
- [ ] Phase 4 context prepared (PRD consolidation inputs)

**Phase 3 Validation Gate**:
- Review security architecture for completeness and correctness
- Review implementation roadmap for feasibility and risk mitigation
- Validate alignment between security and implementation plans
- Approve or request revisions

---

## 7. Agent Invocation Checklist

### 7.1 prd-security-specialist

**Inputs**:
- ✅ 03_technical_requirements.md (NFR-SEC-001 through NFR-SEC-005)
- ✅ 04_system_architecture.md (AuthProvider, OAuthAuthProvider, token lifecycle)
- ✅ DECISION_POINTS.md (Decision #3: Token storage, Decision #12: Observability)
- ✅ This context document (00_phase3_context.md)

**Task Specification**: `TASK_prd_security_specialist.md`

**Expected Output**: `05_security_architecture.md`

**Timeline**: 1 week (Week 5)

### 7.2 prd-implementation-roadmap-specialist

**Inputs**:
- ✅ All Phase 2 deliverables (requirements, architecture)
- ✅ 05_security_architecture.md (security constraints)
- ✅ 02_current_architecture.md (integration points)
- ✅ This context document (00_phase3_context.md)

**Task Specification**: `TASK_prd_implementation_roadmap_specialist.md`

**Expected Output**: `06_implementation_roadmap.md`

**Timeline**: 1 week (Week 6)

---

## 8. Phase 4 Preview

**Phase 4 Objective**: Consolidate all PRD sections into master document for stakeholder review and development handoff.

**prd-documentation-specialist Deliverables**:
1. **07_prd_master_document.md**: Complete PRD with executive summary, all sections, appendices
2. **README.md**: Project overview, OAuth setup quickstart
3. **MIGRATION_GUIDE.md**: API key users transitioning to OAuth
4. **TROUBLESHOOTING.md**: Common OAuth issues and resolutions

**Inputs from Phase 3**:
- Security architecture (threat model, encryption, testing)
- Implementation roadmap (phased plan, risks, testing strategy)
- All Phase 2 deliverables (requirements, architecture)

**Timeline**: Week 7-8

---

**END OF PHASE 3 CONTEXT SUMMARY**

**Next Actions**:
1. Create `TASK_prd_security_specialist.md`
2. Create `TASK_prd_implementation_roadmap_specialist.md`
3. Invoke prd-security-specialist agent
4. Invoke prd-implementation-roadmap-specialist agent (after security deliverable complete)
