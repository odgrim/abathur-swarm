# Abathur Security & Compliance Specification

**Document Version:** 1.0
**Date:** 2025-10-09
**Status:** Complete - Ready for Quality Metrics Phase
**Previous Phase:** API/CLI Specification (05_API_CLI_SPECIFICATION.md)
**Next Phase:** Quality Metrics & Testing Strategy

---

## Table of Contents

1. [Threat Model (STRIDE Analysis)](#1-threat-model-stride-analysis)
2. [Security Requirements](#2-security-requirements)
3. [Secure Architecture](#3-secure-architecture)
4. [Security Controls](#4-security-controls)
5. [Compliance & Privacy](#5-compliance--privacy)
6. [Secure Development Practices](#6-secure-development-practices)

---

## 1. Threat Model (STRIDE Analysis)

### 1.1 Spoofing Identity

**Threat T-SPOOF-001: Unauthorized API Key Usage**
- **Attack Vector:** Attacker gains access to `.env` file or keychain credentials
- **Impact:** Unauthorized Claude API usage, financial liability, quota exhaustion
- **Likelihood:** Medium (local file access required)
- **Mitigation:** Encrypted storage, file permissions (0600), keychain integration

**Threat T-SPOOF-002: Agent Impersonation**
- **Attack Vector:** Malicious process spoofs agent identity to access shared state
- **Impact:** Data corruption, unauthorized task execution, audit trail tampering
- **Likelihood:** Low (requires local process compromise)
- **Mitigation:** Agent ID generation (UUID), state isolation by task_id, audit logging

**Threat T-SPOOF-003: Template Repository Spoofing**
- **Attack Vector:** Man-in-the-middle attack redirects GitHub clone to malicious repository
- **Impact:** Installation of backdoored templates, code execution
- **Likelihood:** Low (HTTPS prevents MITM)
- **Mitigation:** HTTPS enforcement, certificate validation, integrity checksums

### 1.2 Tampering with Data

**Threat T-TAMP-001: Task Queue Manipulation**
- **Attack Vector:** Direct SQLite database modification while system offline
- **Impact:** Task deletion, priority manipulation, unauthorized task injection
- **Likelihood:** Low (requires filesystem access)
- **Mitigation:** File permissions (0600 for abathur.db), ACID transactions, audit trail

**Threat T-TAMP-002: Configuration File Tampering**
- **Attack Vector:** Malicious modification of `.abathur/config.yaml` or `.claude/agents/*.yaml`
- **Impact:** Resource limit bypass, malicious agent prompts, API key theft
- **Likelihood:** Medium (same user can modify files)
- **Mitigation:** Configuration validation (Pydantic schema), file integrity monitoring, gitignore for sensitive files

**Threat T-TAMP-003: Log File Tampering**
- **Attack Vector:** Attacker deletes or modifies audit logs to hide malicious activity
- **Impact:** Loss of audit trail, inability to detect intrusions
- **Likelihood:** Low (requires filesystem access)
- **Mitigation:** File permissions (0640), append-only logging, external log shipping (optional)

### 1.3 Repudiation

**Threat T-REPUD-001: Denial of Task Submission**
- **Attack Vector:** User denies submitting expensive task, disputes API charges
- **Impact:** Accountability gaps, cost disputes
- **Likelihood:** Low (internal tool, not multi-tenant)
- **Mitigation:** Audit trail with timestamps, task submission logged with user context

**Threat T-REPUD-002: Agent Action Denial**
- **Attack Vector:** User claims agent performed unauthorized action
- **Impact:** Trust erosion, inability to debug issues
- **Likelihood:** Low (single-user system)
- **Mitigation:** Complete audit trail (audit table), immutable logs

### 1.4 Information Disclosure

**Threat T-INFO-001: API Key Exposure in Logs**
- **Attack Vector:** API key appears in error messages, debug logs, or crash dumps
- **Impact:** Credential theft, unauthorized API access
- **Likelihood:** High (developer error during logging)
- **Mitigation:** Secret redaction (regex patterns), log sanitization, never log API keys even in --debug mode

**Threat T-INFO-002: Sensitive Data in Task Descriptions**
- **Attack Vector:** User includes passwords, tokens, or PII in task inputs
- **Impact:** Data exposure in logs, database, audit trail
- **Likelihood:** Medium (user error)
- **Mitigation:** User documentation, data classification guidance, optional task input encryption

**Threat T-INFO-003: SQLite Database Exposure**
- **Attack Vector:** Attacker copies `abathur.db` file containing task history and results
- **Impact:** Data breach of all historical task data
- **Likelihood:** Medium (local filesystem access)
- **Mitigation:** File permissions (0600), optional database encryption (SQLCipher), data retention policies

**Threat T-INFO-004: Template Repository Leakage**
- **Attack Vector:** Private repository credentials exposed in MCP config or logs
- **Impact:** Unauthorized access to private codebases
- **Likelihood:** Low (GitHub tokens in environment variables)
- **Mitigation:** Environment variable precedence, credential scanning, gitignore for `.env`

### 1.5 Denial of Service

**Threat T-DOS-001: Queue Flooding**
- **Attack Vector:** Malicious or buggy script submits unlimited tasks
- **Impact:** Queue exhaustion, system unresponsiveness
- **Likelihood:** Medium (local access or buggy automation)
- **Mitigation:** Queue size limit (1,000 default), priority enforcement, rate limiting per source

**Threat T-DOS-002: Resource Exhaustion (Memory)**
- **Attack Vector:** Spawning excessive agents or large task inputs
- **Impact:** System crash, OOM killer
- **Likelihood:** Medium (misconfiguration or malicious templates)
- **Mitigation:** Agent concurrency limits (10 default), memory monitoring, graceful degradation at 80% threshold

**Threat T-DOS-003: Infinite Loop Exploitation**
- **Attack Vector:** Convergence criteria never met, loop runs indefinitely
- **Impact:** Resource exhaustion, API cost overrun
- **Likelihood:** High (developer error in convergence logic)
- **Mitigation:** Max iterations (10 default), timeout (1h default), circuit breaker on repeated failures

**Threat T-DOS-004: Log Disk Exhaustion**
- **Attack Vector:** Verbose logging fills disk space
- **Impact:** System failure, data loss
- **Likelihood:** Low (log rotation enabled)
- **Mitigation:** Log rotation (30-day retention), disk space monitoring, automatic log compression

### 1.6 Elevation of Privilege

**Threat T-PRIV-001: Template Code Execution**
- **Attack Vector:** Malicious template executes arbitrary code during initialization
- **Impact:** System compromise, data exfiltration, credential theft
- **Likelihood:** Medium (template validation is permissive)
- **Mitigation:** Template structure validation, sandboxed execution (future), community review process

**Threat T-PRIV-002: SQL Injection via Task Inputs**
- **Attack Vector:** Malicious task input contains SQL escape sequences
- **Impact:** Database corruption, unauthorized data access
- **Likelihood:** Low (parameterized queries used)
- **Mitigation:** Parameterized SQLite queries, input validation, ORM usage

**Threat T-PRIV-003: Path Traversal in File Operations**
- **Attack Vector:** Task input specifies `../../etc/passwd` in file path
- **Impact:** Unauthorized file system access
- **Likelihood:** Medium (agents have filesystem tools)
- **Mitigation:** Path validation, restrict to project directory, whitelist allowed paths

---

## 2. Security Requirements

### 2.1 Authentication & Authorization (SR-AUTH)

**SR-AUTH-001: API Key Validation**
- API keys must be validated before first Claude API call
- Invalid keys must trigger clear error message with resolution steps
- System must fail gracefully if API key unavailable

**SR-AUTH-002: Keychain Integration**
- System must attempt keychain storage (macOS Keychain, Windows Credential Manager, Linux Secret Service)
- Fallback to encrypted `.env` file if keychain unavailable
- Command `abathur config set-key` stores key in keychain

**SR-AUTH-003: API Key Precedence**
- Order: Environment variable `ANTHROPIC_API_KEY` > Keychain > `.env` file
- Precedence must be documented and consistent
- Precedence visible via `abathur config show --api-key-source`

**SR-AUTH-004: Multi-User Isolation (Future)**
- Reserved for future multi-user support
- Current v1.0 assumes single-user, local-first architecture
- No multi-tenancy isolation required in MVP

**SR-AUTH-005: Audit All Authentication Attempts**
- Log API key retrieval source (env/keychain/.env) to audit trail
- Log validation success/failure (without logging key value)
- Track API key rotation events

### 2.2 Data Protection (SR-DATA)

**SR-DATA-001: API Key Encryption at Rest**
- If stored in `.env` file, encrypt using AES-256-GCM
- Encryption key derived from system-specific entropy (machine ID + user)
- Keychain storage preferred (already encrypted by OS)

**SR-DATA-002: Never Log API Keys**
- API keys must be redacted in all log levels (DEBUG, INFO, ERROR)
- Regex patterns: `sk-ant-[a-zA-Z0-9-]+`, `ANTHROPIC_API_KEY=.*`
- Pre-commit hook checks for accidental key commits

**SR-DATA-003: Sensitive Pattern Redaction**
- Redact additional patterns: email addresses, GitHub tokens, private keys
- Apply redaction to logs, error messages, crash dumps
- Configurable redaction rules in `.abathur/config.yaml`

**SR-DATA-004: Task Result Encryption (Optional)**
- Support optional encryption for sensitive task results
- Use task-level encryption flag: `abathur task submit --encrypt-results`
- Encryption key stored in keychain, tied to task ID

**SR-DATA-005: Secure Deletion**
- Implement secure deletion for completed tasks via `abathur task purge --secure`
- Overwrite database records before deletion (not just DELETE)
- Shred log files containing sensitive data

### 2.3 Communication Security (SR-COMM)

**SR-COMM-001: TLS for All External Calls**
- Enforce TLS 1.3+ for Claude API, Git operations (HTTPS)
- Reject connections with invalid certificates
- Pin Anthropic certificate (optional, future enhancement)

**SR-COMM-002: Certificate Validation**
- Validate SSL certificates against system CA store
- No self-signed certificate acceptance
- Log certificate validation failures to audit trail

**SR-COMM-003: GitHub Repository Validation**
- Validate GitHub repository URLs against allowlist (default: `github.com`)
- Warn users when cloning from non-official template sources
- Checksum validation after clone (SHA-256)

**SR-COMM-004: MCP Server Security**
- MCP servers spawn as subprocesses with limited privileges
- Environment variables sanitized before subprocess spawn
- No credential passing via command-line arguments (use env vars)

### 2.4 Input Validation (SR-INPUT)

**SR-INPUT-001: CLI Argument Validation**
- Typer enforces type validation (int, str, Path, UUID)
- Range validation for priority (0-10), max_iterations (1-100)
- Path validation for file inputs (exists, readable)

**SR-INPUT-002: Task Description Sanitization**
- Reject task inputs containing SQL keywords (DROP, DELETE, UPDATE)
- Validate JSON/YAML task inputs against schema
- Length limits: task description 10KB, input data 1MB

**SR-INPUT-003: Configuration File Validation**
- Pydantic schema validation for all YAML config files
- Reject unknown keys (strict mode)
- Type and range validation for all config values

**SR-INPUT-004: Template Validation**
- Required files: `.abathur/config.yaml`, `.claude/agents/`
- YAML syntax validation before installation
- Agent definition schema validation (name, model, specialization)

**SR-INPUT-005: Size Limits**
- CLI input: 10KB per argument
- Task input: 1MB per task
- Configuration files: 100KB
- Queue size: 1,000 tasks (configurable, max 10,000)

### 2.5 Audit & Logging (SR-AUDIT)

**SR-AUDIT-001: Security Event Logging**
- Log all security-relevant events: authentication, authorization, configuration changes
- Include: timestamp (ISO 8601), event_type, component, context, outcome
- Store in `audit` table with 90-day retention

**SR-AUDIT-002: Correlation IDs**
- Generate correlation ID (UUID) for each CLI invocation
- Include correlation ID in all log entries for tracing
- Expose via `abathur task detail <task-id> --correlation-id`

**SR-AUDIT-003: Log File Integrity**
- File permissions: 0640 (owner read/write, group read)
- Append-only logging (no log modification after write)
- Optional: External log shipping to SIEM (future)

**SR-AUDIT-004: Retention Policy**
- Logs: 30-day rotation, configurable
- Audit trail: 90-day retention, configurable
- Task history: Indefinite (user-controlled purge)

**SR-AUDIT-005: Audit Query Interface**
- Command: `abathur audit query --since "2025-10-01" --event-type "authentication"`
- Export to JSON for external analysis
- Privacy-preserving (redact sensitive data in audit logs)

### 2.6 Dependency Security (SR-DEPEND)

**SR-DEPEND-001: Pinned Dependency Versions**
- Poetry lock file (`poetry.lock`) pins exact versions
- No floating version constraints in production
- Document rationale for version choices

**SR-DEPEND-002: Vulnerability Scanning**
- CI pipeline runs `safety check` on every commit
- Block merges if critical/high vulnerabilities detected
- Monthly dependency update review

**SR-DEPEND-003: Dependency Update Policy**
- Security patches: Apply within 7 days
- Minor updates: Quarterly review
- Major updates: Compatibility testing required

**SR-DEPEND-004: Package Integrity Verification**
- Verify PyPI package checksums during installation
- Use `--require-hashes` for production deployments
- Document trusted package sources

**SR-DEPEND-005: License Compliance**
- All dependencies must be MIT, Apache 2.0, or BSD-3-Clause compatible
- GPL/AGPL dependencies prohibited (copyleft conflict)
- License audit in CI pipeline (`pip-licenses --fail-on "GPL"`)

---

## 3. Secure Architecture

### 3.1 API Key Management

**Storage Hierarchy:**
```
1. Environment Variable (ANTHROPIC_API_KEY)
   ├─ Highest priority
   ├─ Cleared after process exit
   └─ Recommended for CI/CD

2. System Keychain
   ├─ macOS Keychain (via keyring library)
   ├─ Windows Credential Manager (via keyring library)
   ├─ Linux Secret Service (via keyring library)
   ├─ OS-level encryption
   └─ Recommended for local development

3. .env File (Fallback)
   ├─ Located at project root
   ├─ Gitignored by default
   ├─ Encrypted with AES-256-GCM (if implemented)
   └─ Machine-specific encryption key
```

**Key Rotation:**
- No automatic rotation (user-initiated)
- Command: `abathur config rotate-key --new-key <key>`
- Graceful transition: Old key cached for 5 minutes (in-flight requests)
- Audit log entry: "API key rotated at <timestamp>"

**Key Revocation:**
- User revokes key at Anthropic console
- System detects 401 Unauthorized on next request
- Clear error: "API key invalid or revoked. Run `abathur config set-key` to update."

### 3.2 Template Security

**Template Validation Pipeline:**
```
1. Clone from GitHub
   └─ HTTPS enforcement, certificate validation

2. Structure Validation
   ├─ Required files present (.abathur/config.yaml, .claude/agents/)
   ├─ YAML syntax valid
   └─ Agent definitions schema-compliant

3. Content Inspection (Future)
   ├─ No suspicious shell commands (rm -rf, curl | bash)
   ├─ No obfuscated code
   └─ No external network calls in templates

4. Integrity Checksum
   ├─ SHA-256 hash of entire template directory
   ├─ Store in .abathur/metadata.json
   └─ Verify on update (detect tampering)

5. Community Review (Future)
   ├─ Official templates signed by maintainers
   ├─ Community templates require approval
   └─ Security audit for popular templates
```

**Sandboxing Strategy (Future Enhancement):**
- Phase 1 (MVP): Trust user-selected templates
- Phase 2 (Future): Docker-based agent sandboxing
  - Read-only filesystem except `/workspace`
  - No network access except Claude API
  - Resource limits enforced by cgroups

### 3.3 Data Protection at Rest

**SQLite Database Security:**
- File permissions: 0600 (owner read/write only)
- Location: `.abathur/abathur.db` (within project, not system-wide)
- Optional encryption: SQLCipher integration (future)
  - Encryption key derived from user keychain
  - Transparent encryption/decryption

**Configuration File Security:**
- `.abathur/config.yaml`: 0644 (readable by group for team scenarios)
- `.abathur/local.yaml`: 0600 (user-specific secrets)
- `.env`: 0600 (API keys, gitignored)
- `.claude/agents/*.yaml`: 0644 (shared definitions)

**Log File Security:**
- `.abathur/logs/abathur.log`: 0640 (owner read/write, group read)
- JSON format enables parsing by log aggregators
- Rotation preserves permissions
- Compressed logs: 0440 (read-only archive)

### 3.4 Network Security

**Outbound Connections:**
- Claude API: `api.anthropic.com:443` (HTTPS only)
- Git operations: `github.com:443` or other Git hosts (HTTPS/SSH)
- MCP Servers: localhost only (no remote MCP in v1.0)

**Firewall Recommendations:**
- Allow outbound HTTPS to Anthropic, GitHub
- Block all inbound connections (local-only tool)
- MCP servers: localhost communication only

**Proxy Support:**
- Respect `HTTP_PROXY`, `HTTPS_PROXY` environment variables
- Proxy authentication via environment variables
- Document proxy configuration for corporate networks

---

## 4. Security Controls

### Control SC-001: API Key Redaction in Logs
- **Threat Mitigated:** T-INFO-001 (API Key Exposure in Logs)
- **Implementation:** Regex-based redaction before log write
- **Validation:** Unit test with API key in error message, assert redacted

### Control SC-002: File Permission Enforcement
- **Threat Mitigated:** T-INFO-003 (SQLite Database Exposure), T-TAMP-001 (Queue Manipulation)
- **Implementation:** `os.chmod()` on file creation (0600 for DB, 0640 for logs)
- **Validation:** Integration test verifies permissions after `abathur init`

### Control SC-003: Configuration Schema Validation
- **Threat Mitigated:** T-TAMP-002 (Configuration Tampering), T-PRIV-001 (Code Execution)
- **Implementation:** Pydantic schema validation, reject unknown keys
- **Validation:** Unit test with malformed config, assert validation error

### Control SC-004: Queue Size Limit
- **Threat Mitigated:** T-DOS-001 (Queue Flooding)
- **Implementation:** Check queue size before task submission, reject if at limit
- **Validation:** Integration test submits 1001 tasks, assert 1001st rejected

### Control SC-005: Agent Concurrency Limit
- **Threat Mitigated:** T-DOS-002 (Resource Exhaustion)
- **Implementation:** Asyncio semaphore with configurable limit (default: 10)
- **Validation:** Spawn 15 agents, assert only 10 active simultaneously

### Control SC-006: Loop Iteration Limit
- **Threat Mitigated:** T-DOS-003 (Infinite Loop)
- **Implementation:** Max iterations (default: 10), timeout (default: 1h)
- **Validation:** Loop with never-satisfied convergence, assert terminates at limit

### Control SC-007: Memory Monitoring
- **Threat Mitigated:** T-DOS-002 (Resource Exhaustion)
- **Implementation:** psutil monitoring every 5s, throttle at 80%, terminate at 100%
- **Validation:** Load test with memory growth, assert throttling triggered

### Control SC-008: Parameterized SQL Queries
- **Threat Mitigated:** T-PRIV-002 (SQL Injection)
- **Implementation:** aiosqlite parameterized queries, no string interpolation
- **Validation:** Static analysis (Bandit) checks for SQL injection patterns

### Control SC-009: Path Validation
- **Threat Mitigated:** T-PRIV-003 (Path Traversal)
- **Implementation:** `Path.resolve()` normalizes paths, reject if outside project root
- **Validation:** Unit test with `../../etc/passwd`, assert rejected

### Control SC-010: Template Checksum Verification
- **Threat Mitigated:** T-SPOOF-003 (Template Spoofing), T-TAMP-002 (Template Tampering)
- **Implementation:** SHA-256 hash of template directory, store in metadata.json
- **Validation:** Integration test modifies template, assert update detects tampering

### Control SC-011: Certificate Validation
- **Threat Mitigated:** T-SPOOF-003 (Repository Spoofing), T-COMM-001 (MITM)
- **Implementation:** HTTPS with certificate validation, use system CA store
- **Validation:** Integration test with invalid certificate, assert connection refused

### Control SC-012: Audit Trail Immutability
- **Threat Mitigated:** T-TAMP-003 (Log Tampering), T-REPUD-001 (Repudiation)
- **Implementation:** Append-only audit table, foreign key constraints
- **Validation:** Attempt to modify audit record, assert constraint violation

### Control SC-013: Input Size Limits
- **Threat Mitigated:** T-DOS-001 (Queue Flooding via large inputs)
- **Implementation:** Validate input size before accepting (10KB CLI, 1MB task)
- **Validation:** Submit task with 2MB input, assert rejected

### Control SC-014: Dependency Vulnerability Scanning
- **Threat Mitigated:** SR-DEPEND-005 (Vulnerable Dependencies)
- **Implementation:** `safety check` in CI pipeline, block merge on critical
- **Validation:** CI job fails if critical vulnerability detected

### Control SC-015: Retry with Exponential Backoff
- **Threat Mitigated:** T-DOS-003 (API Rate Limit Exhaustion)
- **Implementation:** 10s → 20s → 40s → 80s → 160s → 5min (capped)
- **Validation:** Simulate rate limit error, assert backoff delays correct

---

## 5. Compliance & Privacy

### 5.1 Open Source Licensing

**License Selection: MIT or Apache 2.0**
- **Chosen:** MIT License (simplest, maximum permissiveness)
- **Rationale:** Maximum adoption, minimal restrictions, commercial-friendly
- **Requirements:**
  - All dependencies must be MIT/Apache/BSD-3-Clause compatible
  - No GPL/AGPL dependencies (copyleft prevents MIT licensing)
  - License file in repository root
  - Copyright notice in all source files

**Dependency License Audit:**
```bash
# CI pipeline checks
pip-licenses --fail-on "GPL" --fail-on "AGPL"
pip-licenses --summary --format json > licenses.json
```

**License Compliance Checklist:**
- [ ] LICENSE file in repository root (MIT)
- [ ] Copyright notice in all source files
- [ ] NOTICE file for Apache 2.0 dependencies (if any)
- [ ] Third-party licenses documented in docs/licenses/
- [ ] No GPL/AGPL dependencies

### 5.2 Data Privacy

**GDPR Considerations (if applicable):**
- **Applicability:** If users in EU process personal data
- **Data Minimization:** Only collect data necessary for functionality (task inputs, results, logs)
- **Purpose Limitation:** Data used only for task execution and system monitoring
- **Storage Limitation:** Implement data retention policies (30-day logs, 90-day audit, user-controlled task purge)
- **Right to Erasure:** `abathur task purge --all --secure` deletes all user data
- **Data Portability:** `abathur task export --format json` exports all task data

**CCPA Considerations (California users):**
- No sale of personal data (local-first, no cloud service)
- Right to deletion via `abathur task purge`
- Privacy policy in documentation (if personal data processed)

**Data Classification:**
- **Public:** Configuration defaults, template metadata
- **Internal:** Task queue, agent states, system logs
- **Confidential:** API keys, task inputs/outputs (may contain sensitive data)
- **Restricted:** User credentials (if multi-user support added)

### 5.3 Anthropic API Terms of Service

**Compliance Requirements:**
- No data storage for training (local-only processing complies)
- No API abuse (rate limiting, concurrency limits enforce)
- No prompt injection attacks (user-controlled prompts, documented risks)
- Respect rate limits (exponential backoff, retry logic)

**Usage Monitoring:**
- Track token consumption via `metrics` table
- Alert on anomalous usage (>10x average)
- Command: `abathur metrics token-usage --since "2025-10-01"`

### 5.4 Audit Trail Retention

**Regulatory Requirements:**
- SOC 2 (if applicable): 90-day audit retention minimum
- ISO 27001 (if applicable): 1-year audit retention
- HIPAA (if healthcare): 6-year audit retention

**Abathur Default Policy:**
- Logs: 30 days (configurable)
- Audit trail: 90 days (configurable)
- Task history: Indefinite (user-controlled)

**Configuration:**
```yaml
monitoring:
  log_rotation_days: 30
  audit_retention_days: 90
  task_retention_days: -1  # Indefinite
```

### 5.5 Vulnerability Disclosure

**Security Policy (SECURITY.md):**
```markdown
# Security Policy

## Reporting a Vulnerability
Email security@abathur.dev with:
- Description of vulnerability
- Steps to reproduce
- Impact assessment
- Proposed fix (optional)

## Response Timeline
- Acknowledgment: 48 hours
- Initial assessment: 7 days
- Fix release: 30 days (critical), 90 days (non-critical)

## Disclosure Policy
- Coordinated disclosure (90-day embargo)
- Public CVE assignment for critical vulnerabilities
- Credit to reporter in release notes
```

**Vulnerability Handling Process:**
1. Reporter submits vulnerability via security@abathur.dev
2. Maintainer acknowledges within 48 hours
3. Severity assessment (CVSS scoring)
4. Fix development and testing
5. Security patch release
6. Public disclosure with CVE (if critical)
7. Credit to reporter

---

## 6. Secure Development Practices

### 6.1 Code Review Requirements

**PR Review Checklist:**
- [ ] No hardcoded secrets (API keys, passwords)
- [ ] Input validation for all user inputs
- [ ] Parameterized queries (no SQL string interpolation)
- [ ] Error messages don't leak sensitive data
- [ ] Logging doesn't include API keys or PII
- [ ] New dependencies have licenses checked
- [ ] Tests cover security edge cases

**Automated PR Checks:**
- Linting: ruff (PEP 8, security rules)
- Type checking: mypy (strict mode)
- Security scanning: Bandit (Python security linter)
- Dependency audit: safety check
- Test coverage: >80% (pytest-cov)

### 6.2 Static Analysis

**Tools:**
- **Bandit:** Python security linter (SQL injection, hardcoded passwords, weak crypto)
- **ruff:** Fast Python linter (includes security rules)
- **mypy:** Type checking (prevents type confusion vulnerabilities)
- **pip-audit:** PyPI vulnerability scanner

**CI Pipeline:**
```yaml
security_checks:
  - name: Bandit
    command: bandit -r abathur/ -f json -o bandit-report.json
    fail_on: high_severity

  - name: Safety
    command: safety check --json
    fail_on: critical

  - name: License Audit
    command: pip-licenses --fail-on "GPL"
```

### 6.3 Dependency Management

**Security Practices:**
- Pin exact versions in `poetry.lock`
- Monthly dependency update review
- Automated security patch PRs (Dependabot)
- Test compatibility before updating
- Document version constraints rationale

**Example `pyproject.toml`:**
```toml
[tool.poetry.dependencies]
python = "^3.10"
anthropic = "^0.18.0"  # Pin major.minor, allow patches
typer = "^0.9.0"
pydantic = "^2.5.0"
```

### 6.4 Secret Management

**Pre-commit Hooks:**
```yaml
# .pre-commit-config.yaml
repos:
  - repo: https://github.com/Yelp/detect-secrets
    hooks:
      - id: detect-secrets
        args: ['--baseline', '.secrets.baseline']
```

**Secret Scanning:**
- Detect API keys, tokens, passwords in commits
- Reject commits containing secrets
- Baseline file for false positives
- GitHub secret scanning enabled

**Developer Guidelines:**
- Never commit `.env` files
- Use `.env.example` with placeholder values
- Store real keys in keychain
- Rotate keys if accidentally committed

### 6.5 Testing Strategy

**Security Test Types:**

**1. Input Validation Tests:**
```python
def test_task_submit_rejects_oversized_input():
    large_input = "x" * (1024 * 1024 + 1)  # 1MB + 1 byte
    result = runner.invoke(app, ["task", "submit", "--input", large_input])
    assert result.exit_code != 0
    assert "Input size exceeds limit" in result.output
```

**2. SQL Injection Tests:**
```python
def test_task_list_rejects_sql_injection():
    malicious_filter = "1' OR '1'='1"
    result = runner.invoke(app, ["task", "list", "--filter", malicious_filter])
    assert result.exit_code != 0  # Should fail validation
```

**3. Path Traversal Tests:**
```python
def test_read_file_rejects_path_traversal():
    result = runner.invoke(app, ["task", "submit", "--input-file", "../../etc/passwd"])
    assert result.exit_code != 0
    assert "Path outside project directory" in result.output
```

**4. API Key Redaction Tests:**
```python
def test_api_key_not_logged_in_error():
    with patch("abathur.core.claude.anthropic.Client", side_effect=Exception("Invalid API key: sk-ant-abc123")):
        result = runner.invoke(app, ["task", "submit", "--template", "test"])
        assert "sk-ant-" not in result.output
        assert "REDACTED" in result.output
```

**5. Resource Limit Tests:**
```python
@pytest.mark.asyncio
async def test_agent_concurrency_limit_enforced():
    async with AgentPool(max_agents=10) as pool:
        agents = [pool.spawn() for _ in range(15)]
        active = [a for a in agents if await a.is_active()]
        assert len(active) <= 10
```

### 6.6 Incident Response Plan

**Incident Categories:**
- **P0 - Critical:** API key compromise, active data breach
- **P1 - High:** Vulnerability discovered in production, significant security bug
- **P2 - Medium:** Dependency vulnerability (not exploited)
- **P3 - Low:** Security enhancement request

**Response Procedures:**

**P0 - API Key Compromise:**
1. Immediately revoke compromised key at Anthropic console
2. Notify affected users via email/Discord
3. Rotate key: `abathur config rotate-key`
4. Audit logs for unauthorized usage
5. Document incident in post-mortem
6. Implement additional controls (if needed)

**P1 - Vulnerability Discovered:**
1. Acknowledge report within 48 hours
2. Assess severity (CVSS scoring)
3. Develop and test fix
4. Release security patch within 30 days (critical) or 90 days (high)
5. Publish security advisory (GitHub Security Advisories)
6. Notify users via release notes

**Post-Incident Review:**
- Root cause analysis (5 Whys)
- Timeline of events
- Impact assessment
- Lessons learned
- Action items to prevent recurrence

---

## Summary

This security specification defines a comprehensive threat model, security requirements, and controls for Abathur. Key security principles:

1. **Defense in Depth:** Multiple layers (file permissions, encryption, validation, monitoring)
2. **Principle of Least Privilege:** Minimal permissions for all components
3. **Secure by Default:** Safe configurations out-of-the-box (API keys in keychain, limits enforced)
4. **Transparency:** Complete audit trail, structured logging, user control

**Critical Security Controls:**
- API key encryption in keychain or AES-256 encrypted `.env`
- Template validation with checksum verification
- Input validation and size limits
- Resource limits (queue, agents, memory, iterations)
- Complete audit trail with 90-day retention
- Dependency vulnerability scanning in CI/CD

**Compliance Considerations:**
- MIT open source license (dependency compatibility required)
- GDPR/CCPA data privacy (local-first architecture complies)
- Anthropic API terms of service (rate limiting, no data storage)
- Vulnerability disclosure policy (coordinated disclosure)

**Next Steps:**
- Implement security controls in development (Phase 1-4)
- Security testing integration in CI/CD
- Penetration testing before v1.0 release
- Security documentation for users (API key management, template validation)

---

**Document Status:** Complete - Ready for Quality Metrics Phase
**Security Requirements:** 30 requirements across 6 categories
**Threats Identified:** 20 threats (STRIDE analysis)
**Security Controls:** 15 controls mapped to threats
**Next Phase:** Quality Metrics & Testing Strategy (prd-quality-metrics-specialist)
