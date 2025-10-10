# Task Specification: prd-security-specialist

**Date**: October 9, 2025
**Phase**: Phase 3 - Security Architecture
**Deliverable**: `05_security_architecture.md`
**Timeline**: Week 5 (1 week)
**Status**: Ready for Agent Invocation

---

## 1. Objective

Define comprehensive security architecture for OAuth-based agent spawning, addressing threat modeling, encryption strategies, security testing requirements, and audit logging specifications.

---

## 2. Inputs

### 2.1 Required Documents

**Primary Inputs**:
1. **03_technical_requirements.md** - Security requirements (NFR-SEC-001 through NFR-SEC-005)
2. **04_system_architecture.md** - AuthProvider architecture, token lifecycle design
3. **00_phase3_context.md** - Phase 3 context summary with security focus
4. **DECISION_POINTS.md** - Decision #3 (Token Storage), Decision #12 (Observability)

**Supporting Inputs**:
5. **01_oauth_research.md** - OAuth method research and findings
6. **02_current_architecture.md** - Current system architecture and integration points
7. **PHASE2_VALIDATION_REPORT.md** - Validation findings and security gaps

### 2.2 Security Requirements to Address

**NFR-SEC-001: Encrypted Token Storage**
- **Requirement**: All OAuth tokens stored using OS-level encryption (AES-256 equivalent)
- **Target**: 100% of tokens encrypted at rest via OS keychain
- **Components**: ConfigManager.set_oauth_token(), OS keychain APIs

**NFR-SEC-002: No Token Logging**
- **Requirement**: 0 occurrences of credentials in log files
- **Target**: Automated scanning finds no token values in logs
- **Components**: Structured logging infrastructure, log sanitization rules

**NFR-SEC-003: Error Message Sanitization**
- **Requirement**: 0 credentials exposed in error messages
- **Target**: All exception messages exclude token values
- **Components**: Custom exception hierarchy, error message templates

**NFR-SEC-004: HTTPS-Only Token Transmission**
- **Requirement**: 100% of token refresh requests use HTTPS
- **Target**: All network traffic to `https://console.anthropic.com/v1/oauth/token` encrypted
- **Components**: OAuthAuthProvider.refresh_credentials(), httpx client configuration

**NFR-SEC-005: Token Revocation on Logout**
- **Requirement**: 100% of stored tokens cleared within 100ms of logout
- **Target**: Immediate cleanup of keychain, environment variables, .env file
- **Components**: config_oauth_logout() command, ConfigManager.clear_oauth_tokens()

---

## 3. Deliverable Specification

### 3.1 Document Structure

**File**: `prd_oauth_spawning/05_security_architecture.md`

**Required Sections**:

#### Section 1: Executive Summary
- Overview of security architecture approach
- Key security controls summary (preventive, detective, corrective)
- Compliance considerations (GDPR, data privacy)
- Risk summary (high/medium/low categorization)

#### Section 2: Threat Model
- **2.1 Threat Actors**: Insider threats, external attackers, compromised processes
- **2.2 Attack Vectors**: Token theft, interception, exposure, replay
- **2.3 Token Lifecycle Threat Analysis**:
  - Acquisition (oauth-login): MitM, CLI history exposure
  - Storage (keychain/.env): Unauthorized access, file exposure
  - Refresh (token endpoint): Network interception, token theft
  - Usage (SDK integration): SDK logs, exception traces
  - Revocation (oauth-logout): Incomplete cleanup, server-side gaps
- **2.4 STRIDE Analysis**:
  - **S**poofing: Fake token refresh endpoint
  - **T**ampering: Token modification in storage
  - **R**epudiation: Unlogged authentication events
  - **I**nformation Disclosure: Token logging, error exposure
  - **D**enial of Service: Token refresh endpoint unavailability
  - **E**levation of Privilege: Compromised refresh token
- **2.5 Attack Tree** (diagram): Visual representation of attack paths

#### Section 3: Encryption Strategy
- **3.1 OS Keychain Verification**:
  - macOS Keychain Access: Encryption level (verify AES-256 or equivalent)
  - Linux Secret Service: Encryption levels (gnome-keyring, kwallet)
  - Windows Credential Manager: Encryption level (future consideration)
- **3.2 Token Storage Security**:
  - Keychain storage: Encryption at rest, access control
  - Environment variable security: Process isolation, limited persistence
  - .env file fallback: File permissions, .gitignore enforcement, user warnings
- **3.3 In-Transit Encryption**:
  - HTTPS enforcement for token refresh endpoint
  - TLS version requirements (TLS 1.2+ minimum)
  - Certificate validation (httpx default behavior)
- **3.4 In-Memory Security**:
  - Token lifetime in memory (minimize exposure)
  - Secure string handling (no string concatenation in logs)
  - Garbage collection considerations

#### Section 4: Security Testing Plan
- **4.1 Penetration Testing**:
  - Scope: Token storage, token transmission, token exposure
  - Methodology: OWASP guidelines, manual testing
  - Tools: Burp Suite, OWASP ZAP, custom scripts
  - Test scenarios (≥10):
    1. Token theft from keychain (unauthorized access)
    2. Token interception during refresh (network MitM)
    3. Token exposure in logs (automated scanning)
    4. Token exposure in error messages (exception triggers)
    5. Token exposure in CLI history (command line injection)
    6. Token exposure in .env file (version control leak)
    7. Incomplete token cleanup (logout verification)
    8. Refresh token replay attack (token reuse)
    9. HTTPS downgrade attack (force HTTP)
    10. Token rotation failure (old token persistence)
- **4.2 Vulnerability Scanning**:
  - Static analysis: Bandit, semgrep (Python security linting)
  - Dependency scanning: Safety, pip-audit (known CVEs)
  - Secrets scanning: TruffleHog, git-secrets (committed tokens)
  - Frequency: On every commit (CI/CD integration)
- **4.3 Security Unit Tests**:
  - Token sanitization in logs: `test_no_token_in_logs()`
  - Token sanitization in errors: `test_no_token_in_exceptions()`
  - HTTPS enforcement: `test_refresh_uses_https()`
  - Token cleanup verification: `test_logout_clears_all_tokens()`
  - Coverage target: 100% of security-critical code paths
- **4.4 Security Integration Tests**:
  - End-to-end OAuth flow with malicious inputs
  - Token refresh with network interception simulation
  - Keychain access denial handling
  - .env file with incorrect permissions
- **4.5 Compliance Testing**:
  - GDPR data privacy: Token retention, user consent
  - Audit logging: Security event coverage, log retention

#### Section 5: Audit Logging Specification
- **5.1 Security Events to Log** (≥15 events):
  1. Authentication success/failure
  2. OAuth token acquired (oauth-login)
  3. OAuth token refresh initiated (proactive/reactive)
  4. OAuth token refresh success
  5. OAuth token refresh failure (with error code)
  6. OAuth token expired
  7. OAuth token revoked (oauth-logout)
  8. API key authentication used
  9. Context window warning triggered
  10. Rate limit warning triggered
  11. 401 Unauthorized received
  12. 429 Rate Limit Exceeded received
  13. Token storage failure (keychain/env/file)
  14. Token cleanup failure (logout incomplete)
  15. Security test failure (automated scanning)
- **5.2 Log Format**:
  - Structured logging (JSON format)
  - Required fields: timestamp, event_type, auth_method, success/failure, error_code
  - Excluded fields: token values, refresh tokens, API keys
  - Example:
    ```json
    {
      "timestamp": "2025-10-09T14:30:00Z",
      "event": "oauth_token_refreshed",
      "auth_method": "oauth",
      "success": true,
      "previous_expiry": "2025-10-09T14:25:00Z",
      "new_expiry": "2025-10-09T15:30:00Z",
      "refresh_type": "proactive"
    }
    ```
- **5.3 Log Retention Policy**:
  - Security events: 90 days minimum (for incident investigation)
  - General logs: 30 days
  - Sensitive data: Never logged (tokens, API keys)
- **5.4 Monitoring and Alerting**:
  - Critical alerts: Token refresh failure rate >5%, multiple auth failures (>10 in 1 hour)
  - Warning alerts: Context window warnings >20/hour, rate limit warnings
  - Info alerts: OAuth usage trends, token rotation frequency

#### Section 6: Security Controls Summary
- **6.1 Preventive Controls**:
  - Encryption at rest (OS keychain)
  - Encryption in transit (HTTPS)
  - Token sanitization (logs, errors)
  - Access control (keychain permissions)
- **6.2 Detective Controls**:
  - Audit logging (security events)
  - Automated scanning (secrets, vulnerabilities)
  - Monitoring (refresh failures, auth failures)
- **6.3 Corrective Controls**:
  - Token revocation (oauth-logout)
  - Fallback to manual re-authentication
  - Incident response procedures
- **6.4 NFR-SEC Requirements Traceability**:
  - Table mapping NFR-SEC-001 through NFR-SEC-005 to security controls

#### Section 7: Compliance and Privacy
- **7.1 GDPR Considerations**:
  - Token data: Contains user-identifiable information (yes/no)
  - Data residency: Keychain (local), .env file (local), refresh endpoint (Anthropic servers)
  - User consent: Required for OAuth token storage
  - Right to deletion: Token revocation on logout
- **7.2 Data Privacy**:
  - Token retention: Cleared on logout, expired after TTL
  - Third-party sharing: Tokens sent to Anthropic API only
  - User transparency: OAuth status command shows token status

#### Section 8: Security Recommendations
- **8.1 Immediate Priorities** (for Phase 3 implementation):
  - Implement log sanitization rules (NFR-SEC-002)
  - Verify OS keychain encryption levels (NFR-SEC-001)
  - Add HTTPS enforcement tests (NFR-SEC-004)
- **8.2 Post-MVP Enhancements**:
  - Interactive OAuth flow with browser-based authentication
  - Server-side token revocation (if Anthropic API supports)
  - Hardware security module (HSM) support for enterprise users
- **8.3 Operational Security**:
  - Regular security audits (quarterly)
  - Dependency updates (monthly)
  - Vulnerability scanning (on every commit)

---

### 3.2 Diagrams Required

**Diagram 1: Attack Tree** (ASCII art or markdown)
- Root: "Compromise OAuth Token"
- Branches: Storage attack, transmission attack, exposure attack

**Diagram 2: Security Controls Mapping**
- Threat → Control → NFR-SEC requirement
- Visual representation of defense-in-depth

**Optional Diagram 3: Data Flow with Security Boundaries**
- OAuth token flow with encryption boundaries (in-transit, at-rest, in-memory)

---

### 3.3 Success Criteria

**Document Completeness**:
- [ ] All 8 required sections present
- [ ] Threat model covers all 5 token lifecycle stages
- [ ] STRIDE analysis addresses all 6 threat categories
- [ ] ≥10 penetration test scenarios defined
- [ ] ≥15 security events specified for audit logging
- [ ] NFR-SEC-001 through NFR-SEC-005 traceable to controls

**Quality Standards**:
- [ ] Threat model identifies ≥5 critical threats
- [ ] Encryption strategy verified on macOS and Linux
- [ ] Security testing plan includes tools and methodology
- [ ] Audit logging includes JSON format examples
- [ ] All security controls mapped to requirements

**Traceability**:
- [ ] Every NFR-SEC requirement has ≥1 security control
- [ ] Every threat has ≥1 mitigation
- [ ] Every security test scenario maps to a threat

---

## 4. Architecture Components to Analyze

### 4.1 OAuthAuthProvider

**File**: `infrastructure/oauth_auth.py` (to be created)

**Security-Critical Code**:
- `refresh_credentials()`: Token refresh logic (lines ~180-235 in architecture spec)
  - Network request to `https://console.anthropic.com/v1/oauth/token`
  - Request body contains refresh_token (sensitive data)
  - Response body contains new access_token and refresh_token
  - Error handling: 401, 429, 5xx
- `get_credentials()`: Returns access_token
  - Proactive refresh (5-min buffer)
  - May trigger refresh before returning credentials
- `_is_expired()`, `_is_near_expiry()`: Expiry detection
  - Uses datetime comparison (clock skew considerations)

**Security Threats**:
- Token exposure in network logs (httpx logging)
- Token exposure in exception traces (error handling)
- Token theft from memory (in-memory string handling)

### 4.2 ConfigManager OAuth Methods

**File**: `infrastructure/config.py` (to be modified)

**Security-Critical Code**:
- `get_oauth_token()`: Retrieves tokens from storage
  - Priority: env vars → keychain → .env file
  - Each source has different security properties
- `set_oauth_token()`: Stores tokens securely
  - Keychain API calls (macOS, Linux)
  - .env file write operations (fallback)
- `clear_oauth_tokens()`: Token cleanup
  - Keychain deletion
  - Environment variable clearing
  - .env file scrubbing

**Security Threats**:
- Keychain access without user consent
- .env file with incorrect permissions (world-readable)
- Incomplete token cleanup (residual tokens)

### 4.3 ClaudeClient

**File**: `application/claude_client.py` (to be modified)

**Security-Critical Code**:
- `_configure_sdk_auth()`: Sets `ANTHROPIC_AUTH_TOKEN` env var
  - Overwrites environment variable (global state)
  - Concurrent request considerations (race conditions)
- `execute_task()`: 401 retry loop
  - Catches authentication errors
  - May log error details (token exposure risk)
- Error handling: Exception messages
  - Must not include token values in error text

**Security Threats**:
- Token exposure in SDK logs (anthropic library)
- Token exposure in exception stack traces
- Environment variable race conditions (concurrent tasks)

### 4.4 CLI Commands

**File**: `cli/main.py` (to be modified)

**Security-Critical Code**:
- `config_oauth_login()`: Token acquisition
  - Manual token input (user types token)
  - Token stored in CLI history (shell history risk)
- `config_oauth_status()`: Display auth status
  - Must not log or display token values
  - Safe to show expiry timestamp
- `config_oauth_logout()`: Token revocation
  - Must clear all token storage locations
  - Verify cleanup success

**Security Threats**:
- Token exposure in CLI command history
- Token display in oauth-status output
- Incomplete token cleanup on logout

---

## 5. Threat Modeling Guidance

### 5.1 STRIDE Framework Application

**Spoofing Identity**:
- Fake token refresh endpoint (DNS spoofing, MitM)
- Impersonation via stolen refresh token
- Mitigation: HTTPS certificate validation, token expiry

**Tampering with Data**:
- Token modification in keychain storage
- Token modification in .env file
- Mitigation: OS keychain integrity checks, file permissions

**Repudiation**:
- Unlogged authentication events
- No audit trail for token refresh
- Mitigation: Comprehensive audit logging (NFR-OBS-001, NFR-OBS-002)

**Information Disclosure**:
- Token logging in plaintext (NFR-SEC-002)
- Token exposure in error messages (NFR-SEC-003)
- Token exposure in CLI history
- Mitigation: Log sanitization, error message templates

**Denial of Service**:
- Token refresh endpoint unavailability
- Rate limiting on refresh endpoint
- Mitigation: 3-retry logic, fallback to manual re-auth

**Elevation of Privilege**:
- Compromised refresh token used for unauthorized access
- Stolen access token used before expiry
- Mitigation: Token expiry (1 hour TTL), token rotation

### 5.2 Attack Scenarios to Model

**Scenario 1: Insider Threat (Malicious User)**
- Attacker has access to user's workstation
- Attempts to steal tokens from keychain
- Attempts to read tokens from .env file
- Attempts to intercept tokens from process memory

**Scenario 2: External Attacker (Network MitM)**
- Attacker intercepts network traffic during token refresh
- Attempts to downgrade HTTPS to HTTP
- Attempts to steal tokens from network packets

**Scenario 3: Compromised Process (Malware)**
- Malware running on user's workstation
- Attempts to read tokens from environment variables
- Attempts to intercept SDK API calls
- Attempts to read tokens from logs or error messages

**Scenario 4: Accidental Exposure (User Error)**
- User commits .env file to version control
- User shares logs containing tokens
- User copies error message with token to support ticket

---

## 6. Encryption Strategy Verification

### 6.1 OS Keychain Research Required

**macOS Keychain Access**:
- Encryption algorithm: Verify AES-256 or equivalent
- Access control: Verify user consent required
- Storage location: Verify encrypted file location
- API: Verify `keyring` Python library uses macOS Keychain API

**Linux Secret Service**:
- Implementations: gnome-keyring, kwallet, KWallet
- Encryption algorithm: Varies by implementation (verify each)
- Access control: Varies (some require user unlock)
- API: Verify `keyring` Python library uses Secret Service API

**Fallback Security (.env file)**:
- File permissions: Recommend 600 (user read/write only)
- .gitignore: Enforce .env exclusion
- User warning: Display security notice when using .env fallback

### 6.2 httpx HTTPS Enforcement

**Verification Required**:
- httpx default behavior: HTTPS only or HTTP allowed?
- Certificate validation: Enabled by default?
- TLS version: Minimum TLS 1.2?
- Configuration: Explicitly enforce HTTPS in OAuthAuthProvider

### 6.3 Security Recommendations

**Immediate**:
- Verify OS keychain encryption levels (macOS and Linux testing)
- Explicitly configure httpx to enforce HTTPS and certificate validation
- Implement .env file permission checks (warn if world-readable)

**Post-MVP**:
- Support Hardware Security Module (HSM) for enterprise users
- Support encrypted .env file (using symmetric encryption with key derivation)

---

## 7. Audit Logging Examples

### 7.1 Authentication Success
```json
{
  "timestamp": "2025-10-09T14:30:00Z",
  "event": "auth_initialized",
  "auth_method": "oauth",
  "context_limit": 200000,
  "source": "keychain",
  "success": true
}
```

### 7.2 Token Refresh Success
```json
{
  "timestamp": "2025-10-09T14:30:00Z",
  "event": "oauth_token_refreshed",
  "auth_method": "oauth",
  "success": true,
  "refresh_type": "proactive",
  "previous_expiry": "2025-10-09T14:25:00Z",
  "new_expiry": "2025-10-09T15:30:00Z",
  "latency_ms": 85
}
```

### 7.3 Token Refresh Failure
```json
{
  "timestamp": "2025-10-09T14:30:00Z",
  "event": "oauth_token_refresh_failed",
  "auth_method": "oauth",
  "success": false,
  "error_code": 401,
  "error_type": "refresh_token_expired",
  "attempt": 3,
  "max_attempts": 3,
  "remediation": "abathur config oauth-login"
}
```

### 7.4 Security Violation (Token in Log Detected)
```json
{
  "timestamp": "2025-10-09T14:30:00Z",
  "event": "security_violation_detected",
  "violation_type": "token_in_log",
  "severity": "critical",
  "log_file": "/path/to/abathur.log",
  "line_number": 1234,
  "action_taken": "log_redacted"
}
```

---

## 8. Deliverable Timeline

**Week 5 Breakdown**:

**Day 1-2: Threat Modeling**
- Conduct STRIDE analysis for all token lifecycle stages
- Identify attack vectors and threat actors
- Create attack tree diagram
- Document ≥5 critical threats

**Day 3: Encryption Strategy**
- Verify OS keychain encryption on macOS (test environment)
- Research Linux Secret Service encryption levels
- Define .env file security requirements
- Verify httpx HTTPS enforcement

**Day 4: Security Testing Plan**
- Define ≥10 penetration test scenarios
- Identify security testing tools (Bandit, Safety, TruffleHog)
- Specify security unit tests (100% coverage target)
- Define security integration test strategy

**Day 5: Audit Logging & Documentation**
- Specify ≥15 security events for audit logging
- Define JSON log format with examples
- Specify log retention policy (90 days for security events)
- Document monitoring and alerting rules

**Day 6-7: Review and Refinement**
- Create security controls summary (preventive, detective, corrective)
- Map all NFR-SEC requirements to controls
- Add compliance section (GDPR, data privacy)
- Review document for completeness and quality

---

## 9. Collaboration with Other Agents

**Inputs from prd-implementation-roadmap-specialist** (Week 6):
- Security testing will inform testing strategy
- Threat model will inform risk assessment
- Audit logging will inform monitoring strategy

**Feedback Loop**:
- If implementation roadmap identifies additional security risks, revisit threat model
- If security testing plan is too costly, negotiate scope with roadmap specialist

---

## 10. Validation Criteria

**Self-Check Before Submission**:
- [ ] All 8 required sections present and complete
- [ ] Threat model covers all 5 token lifecycle stages
- [ ] STRIDE analysis addresses all 6 categories
- [ ] ≥10 penetration test scenarios defined
- [ ] ≥15 security events specified
- [ ] Encryption strategy verified on macOS and Linux
- [ ] All NFR-SEC requirements traceable to controls
- [ ] Document length: 2000-3000 lines (comprehensive but readable)

**prd-project-orchestrator Validation**:
- Threat model completeness (all lifecycle stages covered)
- Encryption strategy verification (OS-specific testing)
- Security testing plan feasibility (tools, methodology, coverage)
- Audit logging specification completeness (events, format, retention)
- NFR-SEC requirements traceability (100% mapping to controls)

---

**END OF TASK SPECIFICATION: prd-security-specialist**

**Status**: Ready for agent invocation
**Expected Completion**: End of Week 5
**Next Agent**: prd-implementation-roadmap-specialist (depends on this deliverable)
