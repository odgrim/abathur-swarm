# Decision Points - OAuth-Based Agent Spawning Architecture

**CRITICAL: These decision points MUST be resolved before beginning implementation work.**

This document captures all architectural, technical, and business decisions requiring human input before the PRD implementation phase. Resolving these upfront prevents agent blockers during execution.

---

## 1. OAuth Method Selection

### Decision: Which OAuth-based interaction method(s) should Abathur support?

Based on research findings, multiple OAuth approaches exist:

- [ ] **Claude Code CLI Subshell** - Invoke `claude` CLI programmatically with OAuth authentication
  - Pros: Leverages Max subscription, official tool, full feature support
  - Cons: Requires Claude Code installation, subshell overhead, less portable

- [ x ] **Claude Agent SDK (Official)** - Use official Python SDK with OAuth token support
  - Pros: Official support, programmatic API, Python-native
  - Cons: OAuth flow may be complex in containers, token lifecycle management

- [ ] **claude_max Community Tool** - Use community workaround for Max subscription access
  - Pros: Proven workaround, enables programmatic Max access
  - Cons: Unofficial, potential breaking changes, maintenance concerns

- [ ] **Custom OAuth Flow** - Direct OAuth 2.1 implementation against Claude endpoints
  - Pros: Full control, no external dependencies
  - Cons: Significant development effort, unofficial API usage, stability risks

- [ ] **MCP with OAuth** - Use Model Context Protocol servers with OAuth
  - Pros: Standardized protocol, extensible
  - Cons: Additional complexity, limited to MCP-compatible operations

**Recommendation**:
1. **Primary**: Claude Agent SDK with OAuth (official, supported, programmatic)
2. **Secondary**: Claude Code CLI subshell (for Max subscription users without SDK OAuth setup)
3. **Future**: Monitor MCP ecosystem for additional opportunities

**Your Decision**:
- Primary OAuth method: Let key be provided by user.
- Include secondary fallback? (Yes/No): No

---

## 2. Authentication Mode Configuration

### Decision: How should users configure which authentication mode to use?

- [ ] **Environment Variable** - `ABATHUR_AUTH_MODE=api_key|oauth_cli|oauth_sdk`
  - Simple, familiar pattern, easy to override

- [ ] **Configuration File** - Set in `.abathur/config.yaml`
  - Hierarchical configuration, per-project customization

- [ x ] **Auto-Detection** - Automatically detect available auth methods and prefer OAuth if available
  - Zero configuration for users, smart defaults
  - Risk: Unexpected behavior if multiple methods available
  - answer: the oauth key and sdk api key have different text prefix. We should be able to figure out the auth method from the key itself

- [ ] **Explicit Per-Task** - Specify auth mode per task submission
  - Maximum flexibility, useful for testing
  - Verbose, could be error-prone

**Recommendation**:
- **Default**: Auto-detection with preference order: OAuth SDK > OAuth CLI > API Key
- **Override**: Environment variable `ABATHUR_AUTH_MODE` for explicit control
- **Fallback**: Configuration file for persistent project preferences

**Your Decision**:
- Use auto-detection? (Yes/No): Yes
- Preference order if auto-detecting: No preference, just use the text prefix of the key provided
- Allow environment variable override? (Yes/No): No
- Allow config file specification? (Yes/No): Yes

---

## 3. OAuth Token Storage

### Decision: Where and how should OAuth tokens be securely stored?

- [ x ] **System Keychain/Keyring** (current API key approach)
  - Pros: OS-level security, encrypted at rest
  - Cons: Per-user (not per-project), may require user interaction

- [ ] **Encrypted File in .abathur/**
  - Pros: Per-project isolation, portable
  - Cons: Requires encryption key management, security complexity

- [ ] **Environment Variables Only**
  - Pros: Simple, cloud-native, no persistent storage
  - Cons: Not persistent, must set per session, exposure risk in process lists

- [ ] **Delegate to Claude Code/SDK** - Let official tools manage tokens
  - Pros: Official token lifecycle management
  - Cons: Less control, dependent on external tool configuration

**Recommendation**:
1. **Primary**: Delegate to Claude Code/SDK for OAuth methods (official token management)
2. **Fallback**: System keychain for long-lived OAuth tokens if SDK doesn't manage
3. **Environment Variable**: `CLAUDE_CODE_OAUTH_TOKEN` for explicit token injection

**Your Decision**:
- Token storage mechanism: Env vars, or system keychain

---

## 4. Token Refresh and Lifecycle Management

### Decision: How should Abathur handle OAuth token expiration and refresh?

- [ ] **Manual Refresh** - Require user to re-authenticate when token expires
  - Simple implementation, clear user action

- [ x ] **Automatic Refresh** - Implement OAuth refresh token flow
  - Better UX, uninterrupted operation
  - Requires refresh token storage, additional complexity

- [ ] **Delegate to SDK** - Let Claude Agent SDK handle token refresh
  - Official implementation, lower maintenance
  - Less control over refresh behavior

- [ ] **Hybrid** - Automatic if refresh token available, else prompt user
  - Best UX, graceful degradation
  - Most complex implementation

**Recommendation**: Delegate to Claude Agent SDK for OAuth methods, manual re-authentication for CLI approach

---

## 5. Backward Compatibility

### Decision: How should existing Abathur deployments migrate to the new dual-mode system?

- [ ] **Fully Backward Compatible** - Existing API key config works without changes
  - Zero migration burden
  - Default behavior: If no OAuth configured, use API key

- [ ] **Migration Required** - Require users to update configuration
  - Breaking change, clearer separation
  - Requires migration guide and version bump

- [ ] **Deprecation Period** - Support both, deprecate API-key-only in future version
  - Smooth transition
  - Longer maintenance burden

**Recommendation**: Fully backward compatible - detect API key and use it if no OAuth configured

**Your Decision**:
- Backward compatibility approach: don't bother, no one uses this yet. Just make the changes.

---

## 6. Rate Limiting and Usage Tracking

### Decision: How should Abathur handle different rate limits across auth methods?

Research shows:
- **API Key**: Pay-per-token, no fixed rate limit (billing-based)
- **OAuth (Max 5x)**: ~50-200 prompts per 5 hours
- **OAuth (Max 20x)**: ~200-800 prompts per 5 hours

Options:
- [ ] **Track and Warn** - Monitor usage, warn user approaching limits
- [ ] **Track and Block** - Prevent task submission when limit reached
- [ x ] **Ignore** - Let Anthropic API/Claude Code handle limit enforcement
- [ ] **Smart Scheduling** - Defer tasks when approaching limits, resume when window resets

**Recommendation**: Track and warn, with optional smart scheduling for advanced users

**Your Decision**:
- Rate limiting enforcement: ____________________
- Track usage metrics per auth mode? (Yes/No): ____
- Implement smart scheduling? (Yes/No): ____

---

## 7. Context Window Handling

### Decision: How should Abathur handle different context windows across auth methods?

Research shows:
- **API Key (Sonnet 4.5)**: 1M token context window
- **OAuth/Subscription models**: 200K token context window

Options:
- [ ] **Agent-Level Configuration** - Specify context window per agent template
- [ x ] **Auto-Detection** - Detect from auth mode and model, adjust automatically
- [ x ] **User Warning** - Warn if task inputs exceed context window for auth mode
- [ ] **Automatic Truncation** - Truncate inputs to fit context window

**Recommendation**: Auto-detection with user warning if inputs exceed available context

**Your Decision**:
- Context window handling: Automatic
- Warn user about truncation? (Yes/No): Yes
- Allow per-agent context override? (Yes/No): ____

---

## 8. Model Selection Across Auth Methods

### Decision: How should model selection work with different auth modes?

Different models may be available via different auth methods:
- API Key: All API models (Opus, Sonnet, Haiku)
- OAuth: Subscription-tier-dependent model access

Options:
- [ ] **Auth-Aware Model Selection** - Auto-select best model for auth mode
- [ x ] **User-Specified with Validation** - User specifies, validate availability per mode
- [ ] **Fallback Hierarchy** - Try preferred model, fall back to available alternatives
- [ ] **Error on Unavailable** - Fail task if requested model not available for auth mode

**Recommendation**: User-specified with validation and clear error messages

---

## 9. Testing and Validation Strategy

### Decision: What testing infrastructure is needed for dual-mode authentication?

- [ x ] **Mock OAuth Flow** - Mock OAuth endpoints for unit testing
- [ x ] **Test Accounts** - Require Claude Max test account for integration testing
- [ ] **API Key Only Tests** - Test only API key path in CI/CD, OAuth manually
- [ ] **Dual-Path Testing** - Test both auth paths in CI/CD

**Recommendation**: Mock OAuth for unit tests, API key for automated CI/CD, manual OAuth testing

---

## 10. Error Handling and Fallback

### Decision: What should happen when OAuth authentication fails?

- [  ] **Fail Immediately** - Return error, require user to fix auth
- [ ] **Fallback to API Key** - Automatically try API key if OAuth fails
- [ x ] **Retry OAuth** - Retry OAuth authentication (e.g., token refresh)
- [ ] **Graceful Degradation** - Queue task for later retry

**Recommendation**: Retry OAuth once, then fail with clear error message (no automatic fallback to prevent unexpected billing)

**Your Decision**:
- OAuth failure handling: Refresh token
- Automatic fallback to API key? (Yes/No): No
- Retry attempts before failing: 3

---

## 11. Multi-User / Multi-Tenant Support

### Decision: Should Abathur support multiple users/tenants with different auth credentials?

- [ x ] **Single User** - One set of credentials per Abathur instance (current state)
- [ ] **Multi-User** - Support multiple users with separate credentials
- [ ] **Multi-Tenant** - Full tenant isolation with per-tenant auth, quotas, audit

**Recommendation**: Start with single user, design architecture to support multi-user in future

**Your Decision**:
- User model: ____________________
- Design for future multi-user expansion? (Yes/No): No

---

## 12. Observability and Monitoring

### Decision: What metrics and logs should be tracked for OAuth-based spawning?

- [ x ] **Authentication Events** - Log all auth attempts (success/failure)
- [ x ] **Token Lifecycle Events** - Log token refresh, expiration
- [ x ] **Usage Metrics** - Track tokens used per auth mode
- [ x ] **Performance Metrics** - Compare latency across auth modes
- [ x ] **Error Metrics** - Track error types per auth mode

**Recommendation**: Implement all of the above with structured logging (existing structlog)


---

## 13. Documentation and User Guidance

### Decision: What documentation is needed for OAuth-based spawning?

- [ ] **Migration Guide** - For existing API key users
- [ ] **OAuth Setup Guide** - Step-by-step OAuth configuration
- [ ] **Troubleshooting Guide** - Common OAuth issues and solutions
- [ x ] **Configuration Reference** - Complete config options
- [ x ] **API Reference Updates** - Updated Python API docs

**Recommendation**: All of the above, plus examples for common scenarios

---

## 14. Deployment and Packaging

### Decision: How should OAuth support be deployed?

- [ x ] **Single Package** - Include OAuth support in standard Abathur package
- [ ] **Optional Dependency** - Make OAuth support an optional install (`pip install abathur[oauth]`)
- [ ] **Separate Package** - Create `abathur-oauth` extension package
- [ ] **Feature Flag** - Include code but gate behind feature flag

**Recommendation**: Single package with OAuth as default feature (Claude Agent SDK is official)

**Your Decision**:
- Packaging approach: single-package
