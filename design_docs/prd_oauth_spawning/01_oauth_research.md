# OAuth Research Report: Claude Authentication Methods for Abathur

**Research Date**: October 9, 2025
**Researcher**: oauth-research-specialist agent
**Project**: Abathur OAuth-Based Agent Spawning PRD
**Phase**: Phase 1 - Research & Discovery

---

## Executive Summary

This report provides comprehensive research on all available OAuth-based methods for interacting with Claude services, evaluating their suitability for integration into Abathur's agent spawning architecture.

### Key Findings

1. **Official OAuth Support**: Anthropic's official Python SDK (`anthropic-sdk-python`) supports OAuth tokens via the `ANTHROPIC_AUTH_TOKEN` environment variable, but OAuth is primarily designed for Claude Code CLI, not direct API usage.

2. **Recommended Approach**: **anthropic-sdk-python with ANTHROPIC_AUTH_TOKEN** is the most production-ready OAuth method, leveraging official SDK support with session token authentication.

3. **Critical Context Window Difference**:
   - **API Key**: 1M tokens (1,000,000) - 5x larger context window
   - **OAuth/Subscription**: 200K tokens (200,000) standard, 500K for Enterprise
   - This is a **major architectural consideration** for Abathur's task complexity limits

4. **Rate Limits Comparison**:
   - **API Key**: Pay-per-token, no hard message limits (billing-based)
   - **Max 5x ($100/month)**: 50-200 prompts per 5 hours, ~140-280 hours Sonnet 4 weekly
   - **Max 20x ($200/month)**: 200-800 prompts per 5 hours, ~240-480 hours Sonnet 4 weekly

5. **Community Solutions**: The `claude_max` tool provides a workaround for programmatic Max subscription access, but relies on undocumented behavior and carries deprecation risk.

### Recommendation

**Primary Method**: Implement **anthropic-sdk-python with auth_token support** for OAuth-based authentication:
- Official SDK support (production-ready)
- Python-native integration (matches Abathur's stack)
- Direct authentication via `ANTHROPIC_AUTH_TOKEN` environment variable
- Compatible with existing Claude Agent SDK patterns

**Secondary Method**: **Claude Code CLI subshell invocation** as optional fallback:
- Enables Max subscription users without SDK OAuth setup
- Requires Claude Code installation
- Higher overhead but proven functionality

**Not Recommended**:
- `claude_max` community tool (unofficial, breaking change risk)
- Unofficial web API scraping (ToS violations, fragile)
- Custom OAuth implementation (high complexity, no official endpoints)

**Critical Design Decision**: Due to the 5x context window advantage of API keys (1M vs 200K tokens), Abathur should:
- Auto-detect context window based on authentication method
- Warn users when task inputs exceed OAuth context window
- Consider recommending API key authentication for complex, large-context tasks
- Clearly document context window limitations in OAuth mode

---

## 1. OAuth Method Catalog

### Discovered Methods

| Method | Type | Status | Official Support |
|--------|------|--------|-----------------|
| 1. anthropic-sdk-python with auth_token | Official SDK | Production | ✅ Official |
| 2. Claude Code CLI OAuth | Official CLI Tool | Production | ✅ Official |
| 3. claude_max community tool | Community Wrapper | Community | ❌ Unofficial |
| 4. MCP with OAuth 2.1 | Protocol Extension | Emerging | ✅ Official Protocol |
| 5. GitHub Actions OAuth | CI/CD Integration | Community | ⚠️ Mixed (official action + community OAuth) |
| 6. Unofficial Web API | Reverse Engineering | Experimental | ❌ Unofficial |

### Brief Overview

1. **anthropic-sdk-python with auth_token**: Official Python SDK supports `ANTHROPIC_AUTH_TOKEN` environment variable for OAuth session token authentication, enabling programmatic access with subscription credentials.

2. **Claude Code CLI OAuth**: Official CLI tool with built-in OAuth flow for Claude Max subscriptions, can be invoked programmatically via subshell.

3. **claude_max**: Community-created wrapper that forces Claude Code CLI to use subscription authentication by manipulating environment variables.

4. **MCP with OAuth 2.1**: Model Context Protocol supports OAuth 2.1 with PKCE for remote server authentication, enabling secure third-party integrations with Claude.

5. **GitHub Actions OAuth**: GitHub Actions integration supporting OAuth authentication for CI/CD workflows, combining official Claude Code action with community OAuth handling.

6. **Unofficial Web API**: Reverse-engineered claude.ai web API endpoints, requires session tokens from browser, high risk of breaking changes.

---

## 2. Method Deep Dives

### Method 1: anthropic-sdk-python with auth_token

#### Authentication Mechanism

**Token Type**: OAuth session tokens (not traditional OAuth 2.0 client credentials flow)

**How Tokens Are Obtained**:
- Tokens are obtained through Claude Code CLI authentication flow
- User authenticates via `claude login` command
- Tokens stored in:
  - macOS: Encrypted Keychain
  - Linux: `~/.claude/.credentials.json`

**How Tokens Are Used**:
```python
import os
from anthropic import Anthropic

# Set auth token instead of API key
os.environ['ANTHROPIC_AUTH_TOKEN'] = 'your-oauth-token-here'

# Initialize client (will use ANTHROPIC_AUTH_TOKEN if ANTHROPIC_API_KEY not set)
client = Anthropic()

# Use normally
response = client.messages.create(
    model="claude-sonnet-4-20250514",
    max_tokens=1024,
    messages=[{"role": "user", "content": "Hello, Claude!"}]
)
```

**Authentication Pattern**:
- Header format: `Authorization: Bearer <token>` (instead of `x-api-key: <api-key>`)
- SDK automatically detects and uses `ANTHROPIC_AUTH_TOKEN` if `ANTHROPIC_API_KEY` is not set
- Token validation occurs on first API request

**Credentials Required**:
- OAuth access token (from Claude Code authentication)
- Optional: Refresh token (for automatic renewal)
- No client ID/secret needed (tokens are pre-issued)

**PKCE**: Not applicable (tokens obtained via CLI, not direct OAuth flow)

**Scopes**:
- `user:inference` - API inference access
- `user:profile` - User profile access
- `org:create_api_key` - API key management (if applicable)

#### Capabilities

| Capability | Supported | Notes |
|------------|-----------|-------|
| Text Generation (streaming) | ✅ Yes | Full support via SDK |
| Text Generation (non-streaming) | ✅ Yes | Full support via SDK |
| Tool Use / Function Calling | ✅ Yes | Compatible with SDK tool use API |
| File Operations | ❌ Limited | No native file upload (use base64 encoding) |
| MCP Server Integration | ✅ Yes | Can use MCP via Claude Agent SDK |
| Agent SDK Compatibility | ✅ Yes | Native SDK integration |
| Multi-turn Conversations | ✅ Yes | Full conversation history support |
| Prompt Caching | ✅ Yes | Supported with appropriate API calls |

#### Rate Limits & Restrictions

**Message Limits** (5-hour reset cycle):
- Max 5x Plan ($100/month): 50-200 prompts per 5 hours
- Max 20x Plan ($200/month): 200-800 prompts per 5 hours

**Weekly Limits** (7-day reset cycle, introduced August 2025):
- Max 5x Plan: 140-280 hours of Sonnet 4, 15-35 hours of Opus 4
- Max 20x Plan: 240-480 hours of Sonnet 4, 24-40 hours of Opus 4

**Context Window**:
- **Standard Subscription**: 200,000 tokens
- **Enterprise Subscription**: 500,000 tokens
- **API Key (for comparison)**: 1,000,000 tokens (5x larger than standard subscription)

**Concurrent Requests**: Not explicitly documented, likely rate-limited per account

**Usage Tier Requirements**:
- Requires Claude Max 5x or 20x subscription
- Cannot mix API key billing with subscription usage

**Hard vs Soft Limits**:
- Hard limits: Message count per 5-hour and weekly windows
- Overage: Can purchase additional usage at standard API rates

**Rate Limit Reset Windows**:
- 5-hour rolling window for message limits
- 7-day rolling window for weekly limits (introduced 2025)

#### Context Window Size

**Maximum Context Window**: 200,000 tokens (standard), 500,000 tokens (Enterprise)

**Comparison to API Key**:
- API key: 1,000,000 tokens (1M)
- OAuth: 200,000 tokens (200K) - **5x smaller**
- **This is a critical limitation for complex agent tasks**

**Impact on Agent Task Complexity**:
- Large codebase analysis may hit context limits
- Multi-file operations require careful context management
- Long conversation histories consume context budget faster

**Truncation Behavior**:
- API returns error if context exceeds window
- No automatic truncation
- Client must manage context size

#### Model Access

**Available Models** (subscription-based):
- Claude Sonnet 4 and 4.5
- Claude Opus 4
- Model availability depends on subscription tier

**Model Availability by Tier**:
- All Max subscription tiers have access to latest models
- Enterprise may have early access to new models

**Default Model Selection**: User must specify model in API call

**Model Switching**: Supported, switch per request

#### Subscription Requirements

**Minimum Tier**: Claude Max 5x ($100/month)

**Pricing Implications**:
- Fixed monthly cost regardless of usage (until limits reached)
- Overage charged at standard API rates
- Break-even analysis needed vs pay-per-token API

**Free Tier**: Not available for OAuth access

**Enterprise Requirements**: Available but not required

#### Technical Implementation

**Python Code Example**:
```python
import os
from anthropic import Anthropic

# Option 1: Set environment variable
os.environ['ANTHROPIC_AUTH_TOKEN'] = 'your-oauth-token'

# Option 2: Load from credentials file
import json
creds_path = os.path.expanduser('~/.claude/.credentials.json')
with open(creds_path) as f:
    creds = json.load(f)
    os.environ['ANTHROPIC_AUTH_TOKEN'] = creds['accessToken']

# Initialize client
client = Anthropic()

# Use with Agent SDK
from claude_agent_sdk import query

async def main():
    async for message in query(prompt="Analyze this code..."):
        print(message)
```

**Configuration Requirements**:
- Must unset `ANTHROPIC_API_KEY` if both are present (API key takes precedence)
- Token refresh logic must be implemented separately
- Credentials file location varies by OS

**Dependency Requirements**:
- `anthropic>=0.40.0` (SDK with auth_token support)
- `claude-agent-sdk>=2.0.0` (for Agent SDK features)
- Python 3.10+

**Error Handling Patterns**:
```python
from anthropic import AuthenticationError

try:
    response = client.messages.create(...)
except AuthenticationError as e:
    if "OAuth token has expired" in str(e):
        # Trigger token refresh
        refresh_oauth_token()
        # Retry request
    else:
        raise
```

**Retry Logic Recommendations**:
- Retry on 401 with token refresh
- Exponential backoff for rate limits (429)
- 3 retry attempts maximum

#### Pros & Cons

**Strengths**:
- ✅ Official SDK support (production-ready)
- ✅ Python-native integration (matches Abathur stack)
- ✅ Full API feature parity with API key authentication
- ✅ Lower cost for high-volume users (fixed subscription vs pay-per-token)
- ✅ No breaking changes risk (official SDK)
- ✅ Tool use and function calling support

**Weaknesses**:
- ❌ **5x smaller context window** (200K vs 1M tokens) - critical limitation
- ❌ Hard message limits (50-800 per 5 hours depending on tier)
- ❌ Token management complexity (refresh logic required)
- ❌ Requires Claude Max subscription ($100-200/month)
- ❌ No official OAuth flow implementation (relies on Claude Code CLI for token acquisition)
- ❌ Token refresh issues reported in 2025 (GitHub issues #2633, #2830)

**Best Use Cases**:
- High-volume agent spawning within rate limits
- Fixed-budget operations (subscription vs variable API costs)
- Tasks fitting within 200K context window
- Organizations with existing Claude Max subscriptions

**Failure Cases**:
- Large codebase analysis (context window limit)
- Bursty workloads exceeding 5-hour limits
- Automated systems requiring 100% uptime (token refresh failures)

#### Official vs Community Support

**Official Support Level**: Official SDK feature (auth_token parameter), but OAuth flow itself is not directly documented for programmatic use

**Documentation Quality**:
- SDK documentation: Good (API reference complete)
- OAuth workflow documentation: Poor (relies on Claude Code CLI docs)
- Token refresh: Not documented

**Community Adoption**: Growing, but API key remains more popular

**Maintenance Status**: Actively maintained (official SDK)

**Breaking Change Risk**: Low (official SDK with semantic versioning)

**Deprecation Timeline**: None announced

---

### Method 2: Claude Code CLI OAuth (Subshell Invocation)

#### Authentication Mechanism

**Token Type**: OAuth 2.0 with PKCE (Proof Key for Code Exchange)

**How Tokens Are Obtained**:
1. User runs `claude login` command
2. Browser opens to `https://console.anthropic.com/oauth/authorize`
3. User authenticates with Claude.ai credentials
4. Authorization code returned to CLI via redirect
5. CLI exchanges code for access/refresh tokens
6. Tokens stored in encrypted keychain (macOS) or `~/.claude/.credentials.json` (Linux)

**OAuth Endpoints**:
- Authorization: `https://console.anthropic.com/oauth/authorize`
- Token: `https://console.anthropic.com/v1/oauth/token`
- Redirect URI: `https://console.anthropic.com/oauth/code/callback`

**Client ID**: `9d1c250a-e61b-44d9-88ed-5944d1962f5e` (Claude Code official client)

**How Tokens Are Used**:
- Claude Code CLI manages token lifecycle automatically
- Tokens stored in encrypted storage
- Automatic refresh on 401 responses
- Programmatic invocation via subshell: `claude -p "your prompt"`

**PKCE**: Yes, uses S256 code challenge method

**Scopes**:
- `user:inference` - Run inference requests
- `user:profile` - Access user profile
- `org:create_api_key` - API key management

#### Capabilities

| Capability | Supported | Notes |
|------------|-----------|-------|
| Text Generation (streaming) | ✅ Yes | Via `--stream` flag |
| Text Generation (non-streaming) | ✅ Yes | Default behavior |
| Tool Use / Function Calling | ✅ Yes | Via `--allowedTools` flag |
| File Operations | ✅ Yes | Full file system access via tools |
| MCP Server Integration | ✅ Yes | Native MCP support in Claude Code |
| Agent SDK Compatibility | ✅ Yes | Claude Code powers Agent SDK |
| Multi-turn Conversations | ✅ Yes | Session-based conversations |
| Prompt Caching | ✅ Yes | Automatic caching in Claude Code |

#### Rate Limits & Restrictions

Same as Method 1 (anthropic-sdk-python with auth_token):
- Max 5x: 50-200 prompts/5h, 140-280h Sonnet 4/week
- Max 20x: 200-800 prompts/5h, 240-480h Sonnet 4/week

#### Context Window Size

**Maximum Context Window**: 200,000 tokens (standard), 500,000 tokens (Enterprise)

**Comparison to API Key**: 5x smaller (200K vs 1M tokens)

#### Model Access

Same as Method 1 - access to Claude Sonnet 4, 4.5, and Opus 4 based on subscription tier

#### Subscription Requirements

**Minimum Tier**: Claude Max 5x ($100/month)

#### Technical Implementation

**Subshell Invocation Example (Python)**:
```python
import subprocess
import json

def invoke_claude_code(prompt: str, allowed_tools: list[str] = None) -> dict:
    """Invoke Claude Code CLI via subshell with OAuth authentication."""

    cmd = ["claude", "-p", prompt]

    if allowed_tools:
        cmd.extend(["--allowedTools", *allowed_tools])

    # Execute Claude Code
    result = subprocess.run(
        cmd,
        capture_output=True,
        text=True,
        timeout=300  # 5 minute timeout
    )

    if result.returncode != 0:
        raise RuntimeError(f"Claude Code failed: {result.stderr}")

    return {
        "success": True,
        "content": result.stdout,
        "error": None
    }

# Example usage
response = invoke_claude_code(
    prompt="Analyze this Python function for bugs",
    allowed_tools=["Read", "Grep"]
)
```

**Configuration Requirements**:
- Claude Code CLI must be installed: `npm install -g @anthropic-ai/claude-code`
- User must authenticate once: `claude login`
- Credentials persist in system keychain

**Dependency Requirements**:
- Node.js (for Claude Code installation)
- Claude Code 2.0.0+
- No Python SDK dependencies (standalone CLI)

**Error Handling Patterns**:
```python
def invoke_claude_with_retry(prompt: str, max_retries: int = 3) -> dict:
    """Invoke Claude Code with automatic retry on auth failure."""

    for attempt in range(max_retries):
        try:
            result = subprocess.run(
                ["claude", "-p", prompt],
                capture_output=True,
                text=True,
                timeout=300
            )

            if result.returncode == 0:
                return {"success": True, "content": result.stdout}

            # Check for auth error
            if "OAuth token has expired" in result.stderr:
                # Trigger re-authentication
                subprocess.run(["claude", "login"], check=True)
                continue
            else:
                raise RuntimeError(result.stderr)

        except subprocess.TimeoutExpired:
            if attempt == max_retries - 1:
                raise
            continue

    raise RuntimeError("Max retries exceeded")
```

**Token Refresh**: Automatic (handled by Claude Code CLI)

#### Pros & Cons

**Strengths**:
- ✅ Official Anthropic tool (production-ready)
- ✅ Automatic token lifecycle management (refresh handled by CLI)
- ✅ Full Claude Code feature set (tools, MCP, file operations)
- ✅ No manual OAuth flow implementation needed
- ✅ Encrypted credential storage (OS keychain)
- ✅ Simple invocation model (just call `claude` command)

**Weaknesses**:
- ❌ **5x smaller context window** (200K vs 1M tokens)
- ❌ Subshell overhead (~17ms per invocation)
- ❌ Requires Node.js and npm (additional dependencies)
- ❌ Not portable (requires CLI installation on target system)
- ❌ Less control over API interactions (CLI abstraction layer)
- ❌ Harder to debug (stderr parsing required)
- ❌ Not pure Python (external process execution)

**Best Use Cases**:
- Quick integration for Max subscription users
- Prototyping and development
- Systems where Claude Code is already installed
- File system operations requiring tool access

**Failure Cases**:
- Docker/container environments (CLI installation complexity)
- Lambda/serverless (cold start overhead, npm dependencies)
- High-throughput scenarios (subshell overhead)
- Systems without Node.js

#### Official vs Community Support

**Official Support Level**: Fully official (Anthropic-maintained CLI)

**Documentation Quality**: Excellent (comprehensive CLI docs)

**Community Adoption**: High (primary Claude Code interface)

**Maintenance Status**: Actively maintained

**Breaking Change Risk**: Low (stable CLI interface)

**Deprecation Timeline**: None (core Anthropic product)

---

### Method 3: claude_max Community Tool

#### Authentication Mechanism

**How It Works**:
- Wrapper script around Claude Code CLI
- Manipulates environment variables to force subscription authentication
- Removes `ANTHROPIC_API_KEY` to trigger OAuth fallback

**Token Acquisition**:
- Relies on existing Claude Code authentication
- Uses tokens from `~/.claude/.credentials.json`
- No separate OAuth flow

**How Tokens Are Used**:
```bash
#!/bin/bash
# claude_max wrapper script concept

# Remove API key to force OAuth
unset ANTHROPIC_API_KEY

# Find Claude CLI binary
CLAUDE_BIN=$(which claude)

# Execute with subscription auth
$CLAUDE_BIN "$@"
```

**PKCE**: Not applicable (uses Claude Code's existing OAuth)

**Scopes**: Same as Claude Code (user:inference, user:profile)

#### Capabilities

Same as Method 2 (Claude Code CLI) - full Claude Code feature set via wrapper

#### Rate Limits & Restrictions

Same as Method 1 & 2 - Max subscription limits apply

#### Context Window Size

**Maximum Context Window**: 200,000 tokens (standard), 500,000 tokens (Enterprise)

**Comparison to API Key**: 5x smaller (200K vs 1M tokens)

#### Model Access

Same as Claude Code - all Max subscription models available

#### Subscription Requirements

**Minimum Tier**: Claude Max 5x ($100/month)

#### Technical Implementation

**Installation** (conceptual):
```bash
# Install claude_max wrapper
pip install claude-max

# Use instead of claude command
claude_max "Analyze this codebase"
```

**Python Integration**:
```python
import subprocess

def invoke_claude_max(prompt: str) -> dict:
    """Invoke claude_max wrapper for subscription auth."""

    result = subprocess.run(
        ["claude_max", prompt],
        capture_output=True,
        text=True,
        timeout=300
    )

    return {
        "success": result.returncode == 0,
        "content": result.stdout,
        "error": result.stderr if result.returncode != 0 else None
    }
```

**Configuration Requirements**:
- Claude Code CLI must be installed and authenticated
- `claude_max` wrapper installed
- Credentials in `~/.claude/.credentials.json`

**Dependency Requirements**:
- Claude Code CLI
- `claude_max` package (PyPI)
- Existing Claude Max subscription authentication

**Performance**: Minimal overhead (~17ms authentication time reported)

#### Pros & Cons

**Strengths**:
- ✅ Enables programmatic Max subscription access
- ✅ Minimal overhead (thin wrapper)
- ✅ No additional authentication flow needed
- ✅ Leverages existing Claude Code infrastructure
- ✅ Simple to use (drop-in replacement for `claude` command)

**Weaknesses**:
- ❌ **5x smaller context window** (200K vs 1M tokens)
- ❌ **Unofficial tool** (not Anthropic-supported)
- ❌ **Breaking change risk** (relies on undocumented behavior)
- ❌ Environment variable manipulation (fragile approach)
- ❌ May violate Anthropic ToS (unclear)
- ❌ Limited documentation
- ❌ Maintenance uncertainty (community project)
- ❌ Could break with Claude Code updates

**Best Use Cases**:
- Development/testing with Max subscription
- Quick workaround for programmatic access
- Situations where official OAuth SDK isn't suitable

**Failure Cases**:
- Production environments (unofficial, unstable)
- Compliance-sensitive applications (ToS concerns)
- Long-term projects (deprecation risk)

#### Official vs Community Support

**Official Support Level**: None (community project)

**Documentation Quality**: Limited (blog post, basic README)

**Community Adoption**: Niche (small user base)

**Maintenance Status**: Unknown (single maintainer)

**Breaking Change Risk**: **High** (exploits undocumented behavior)

**Deprecation Timeline**: Could break at any time with Claude Code updates

---

### Method 4: MCP (Model Context Protocol) with OAuth 2.1

#### Authentication Mechanism

**Token Type**: OAuth 2.1 with PKCE (standardized in MCP spec March 2025)

**How Tokens Are Obtained**:
1. MCP server implements OAuth 2.1 authorization endpoints
2. MCP client (Claude.ai or Claude Code) initiates OAuth flow
3. User authenticates and grants permissions
4. MCP server issues access token
5. Client uses token for subsequent MCP requests

**OAuth Flow**:
- MCP servers act as **OAuth Resource Servers**
- Must support Dynamic Client Registration (RFC 7591)
- Must implement OAuth 2.0 Authorization Server Metadata (RFC 8414)

**Endpoints Required**:
- Authorization endpoint: `/.well-known/oauth-authorization-server`
- Token endpoint: `/oauth/token`
- Registration endpoint: `/oauth/register` (Dynamic Client Registration)

**How Tokens Are Used**:
```http
GET /mcp/v1/tools HTTP/1.1
Host: your-mcp-server.com
Authorization: Bearer <oauth-access-token>
```

**Client ID/Secret**:
- **Claude.ai**: Requires Dynamic Client Registration (no manual client ID/secret)
- **Other clients**: May support pre-registered clients

**PKCE**: Required (S256 code challenge method)

**Scopes**: Defined by MCP server (e.g., `read:tools`, `execute:tools`)

#### Capabilities

| Capability | Supported | Notes |
|------------|-----------|-------|
| Text Generation (streaming) | N/A | MCP provides tools to LLM, not LLM itself |
| Text Generation (non-streaming) | N/A | MCP provides tools to LLM, not LLM itself |
| Tool Use / Function Calling | ✅ Yes | Core MCP functionality |
| File Operations | ✅ Yes | If MCP server exposes file tools |
| MCP Server Integration | ✅ Yes | Native use case |
| Agent SDK Compatibility | ✅ Yes | Agent SDK can use MCP servers |
| Multi-turn Conversations | ✅ Yes | Stateful MCP servers supported |
| Prompt Caching | N/A | MCP server-dependent |

**Note**: MCP is not a replacement for Claude API authentication, but a way to authenticate **external tools and resources** that Claude can access.

#### Rate Limits & Restrictions

**MCP-Specific Limits**: Defined by individual MCP server implementations

**Claude Integration Limits**:
- Claude.ai rate limits still apply to LLM calls
- MCP tool calls count toward usage limits
- Remote MCP servers have their own rate limits

**Context Window Impact**:
- MCP responses consume Claude's context window
- Large MCP responses can fill context quickly

#### Context Window Size

**MCP Protocol**: No inherent context window (protocol spec only)

**Claude Integration**:
- MCP responses must fit within Claude's context window
- 200K tokens for subscription, 1M for API key
- MCP servers should return concise responses

#### Model Access

**N/A** - MCP provides tools to models, doesn't control model access

#### Subscription Requirements

**For Claude.ai Integration**:
- Claude Pro or Max subscription recommended
- Free tier may have limited MCP support

**For Claude Code**:
- Works with API key or subscription authentication
- MCP server authentication is separate

#### Technical Implementation

**MCP Server with OAuth (Python Example)**:
```python
from mcp.server import Server, MCPServer
from mcp.auth import OAuth2Config
import fastmcp
import mcpauth

# Initialize MCP server with OAuth
app = fastmcp.FastMCP("My Secure MCP Server")

# Configure OAuth with Auth0 (example)
oauth_config = mcpauth.configure(
    domain="your-domain.auth0.com",
    client_id="your-client-id",
    client_secret="your-client-secret",
    required_scopes=["read:tools", "execute:tools"]
)

@app.tool()
async def analyze_code(code: str) -> str:
    """Analyze code for security issues."""
    # OAuth token validated automatically by mcpauth
    # Implementation here
    return "Analysis results..."

# Run MCP server with OAuth
if __name__ == "__main__":
    app.run(oauth_config=oauth_config)
```

**Claude.ai Configuration**:
```json
{
  "mcpServers": {
    "secure-tools": {
      "url": "https://your-mcp-server.com",
      "oauth": {
        "requireOAuth": true,
        "scopes": ["read:tools", "execute:tools"]
      }
    }
  }
}
```

**Configuration Requirements**:
- MCP server must implement OAuth 2.1 spec
- Dynamic Client Registration endpoint required for Claude.ai
- TLS/HTTPS required for production
- OAuth provider configuration (Auth0, Okta, custom)

**Dependency Requirements** (Python MCP Server):
- `mcp-sdk-python>=1.0.0`
- `fastmcp` (for fast MCP server development)
- `mcpauth` (for OAuth authentication handling)
- OAuth provider SDK (Auth0, Okta, etc.)

**Error Handling**:
```python
from mcp.auth import OAuthError

@app.tool()
async def protected_tool():
    try:
        # Tool implementation
        pass
    except OAuthError as e:
        if e.error == "invalid_token":
            # Token expired or invalid
            return "Authentication required. Please re-authorize."
        raise
```

#### Pros & Cons

**Strengths**:
- ✅ **Standardized protocol** (OAuth 2.1 official spec)
- ✅ Secure third-party integrations (no credential sharing)
- ✅ Fine-grained permissions (scopes)
- ✅ Official Anthropic support (MCP is Anthropic-created)
- ✅ Claude.ai native integration (remote MCP)
- ✅ Extensible (add custom tools)
- ✅ Production-ready (used by Atlassian, Microsoft, others)

**Weaknesses**:
- ❌ **Not for Claude API authentication** (tools only, not LLM access)
- ❌ Additional complexity (OAuth + MCP server setup)
- ❌ Requires separate OAuth provider (Auth0, Okta, custom)
- ❌ Dynamic Client Registration required for Claude.ai
- ❌ Server deployment needed (infrastructure overhead)
- ❌ Limited to tool/resource access (doesn't solve agent spawning directly)

**Best Use Cases**:
- Secure third-party integrations with Claude
- Enterprise tools requiring fine-grained permissions
- Multi-tenant MCP servers (per-user authentication)
- Compliance requirements (OAuth audit trails)

**Failure Cases**:
- Simple single-user Abathur deployments (over-engineering)
- Direct LLM API authentication (wrong use case)
- Latency-sensitive applications (network round-trips)

#### Official vs Community Support

**Official Support Level**: Official protocol (created by Anthropic)

**Documentation Quality**: Excellent (comprehensive spec at modelcontextprotocol.io)

**Community Adoption**: Growing rapidly (major companies adopting)

**Maintenance Status**: Actively maintained (protocol evolution ongoing)

**Breaking Change Risk**: Low (semantic versioning, backward compatibility)

**Deprecation Timeline**: None (core Anthropic protocol)

**Note**: MCP with OAuth is the right solution for **securing external tools**, but **not the right solution for Abathur's agent spawning authentication**. It's included here for completeness and potential future tool integrations.

---

### Method 5: GitHub Actions with OAuth

#### Authentication Mechanism

**Token Type**: Claude Code OAuth tokens (access + refresh tokens)

**How Tokens Are Obtained**:
1. User authenticates Claude Code locally (`claude login`)
2. Tokens extracted from `~/.claude/.credentials.json`
3. Tokens added as GitHub repository secrets:
   - `CLAUDE_ACCESS_TOKEN`
   - `CLAUDE_REFRESH_TOKEN`
   - `CLAUDE_EXPIRES_AT`
4. GitHub Actions workflow uses secrets for authentication

**Token Refresh**:
- Requires `SECRETS_ADMIN_PAT` (GitHub Personal Access Token with `secrets:write` permission)
- Workflow automatically refreshes tokens and updates secrets

**OAuth Flow**: Uses existing Claude Code authentication (no in-workflow OAuth)

#### Capabilities

Same as Claude Code CLI (Method 2) - full feature set when invoked in GitHub Actions

#### Rate Limits & Restrictions

**Claude Limits**: Same Max subscription limits (50-800 prompts/5h)

**GitHub Actions Limits**:
- 6 hours max workflow runtime (public repos)
- 2,000 minutes/month free (private repos)
- Concurrent job limits by plan

#### Context Window Size

**Maximum Context Window**: 200,000 tokens (standard), 500,000 tokens (Enterprise)

**Comparison to API Key**: 5x smaller (200K vs 1M tokens)

#### Model Access

Same as Claude Code - all Max subscription models

#### Subscription Requirements

**Minimum Tier**: Claude Max 5x ($100/month) for OAuth authentication

**Alternative**: API key authentication for GitHub Actions (pay-per-token)

#### Technical Implementation

**GitHub Actions Workflow (OAuth)**:
```yaml
name: Claude Code with OAuth

on:
  pull_request:
    types: [opened, synchronize]

jobs:
  claude-review:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4

      - name: Claude Code OAuth Action
        uses: grll/claude-code-action@beta
        with:
          use_oauth: true
          claude_access_token: ${{ secrets.CLAUDE_ACCESS_TOKEN }}
          claude_refresh_token: ${{ secrets.CLAUDE_REFRESH_TOKEN }}
          claude_expires_at: ${{ secrets.CLAUDE_EXPIRES_AT }}
          secrets_admin_pat: ${{ secrets.SECRETS_ADMIN_PAT }}
          prompt: "Review this PR for security issues and code quality"
```

**Setup Steps**:
1. Install Claude GitHub App to repository
2. Authenticate Claude Code locally: `claude login`
3. Extract tokens from `~/.claude/.credentials.json`:
   ```bash
   cat ~/.claude/.credentials.json | jq -r '.accessToken'
   cat ~/.claude/.credentials.json | jq -r '.refreshToken'
   cat ~/.claude/.credentials.json | jq -r '.expiresAt'
   ```
4. Add tokens as GitHub repository secrets
5. Create GitHub PAT with `secrets:write` scope
6. Add PAT as `SECRETS_ADMIN_PAT` secret

**Dependency Requirements**:
- Claude GitHub App installed
- GitHub repository with Actions enabled
- Claude Max subscription
- GitHub PAT for token refresh

**Token Refresh Mechanism**:
- Action automatically refreshes tokens before expiration
- Updates GitHub secrets with new tokens
- Transparent to workflow (no manual intervention)

#### Pros & Cons

**Strengths**:
- ✅ Enables Max subscription in CI/CD
- ✅ Automatic token refresh (no manual re-auth)
- ✅ Full Claude Code features in Actions
- ✅ Cost-effective for high-volume CI (subscription vs API pay-per-token)
- ✅ Official Claude GitHub App integration

**Weaknesses**:
- ❌ **5x smaller context window** (200K vs 1M tokens)
- ❌ Complex setup (manual token extraction)
- ❌ Requires GitHub PAT with write access (security concern)
- ❌ Community OAuth action (not official Anthropic)
- ❌ Only works in GitHub Actions (not portable)
- ❌ Token exposure in workflow logs (if misconfigured)

**Best Use Cases**:
- PR reviews with Claude Max subscription
- High-volume GitHub Actions workflows
- Teams with existing Max subscriptions

**Failure Cases**:
- Other CI/CD platforms (GitLab, CircleCI, etc.)
- Organizations restricting PAT usage
- Security-sensitive environments (token handling concerns)

#### Official vs Community Support

**Official Support Level**:
- Official Claude GitHub App: ✅ Official
- OAuth action (`grll/claude-code-action`): ❌ Community

**Documentation Quality**:
- Official app: Excellent
- OAuth action: Limited (GitHub Marketplace page)

**Community Adoption**: Niche (primarily Max subscribers doing CI/CD)

**Maintenance Status**:
- Official app: Actively maintained
- OAuth action: Community-maintained (single maintainer)

**Breaking Change Risk**: Medium (community action could break)

**Deprecation Timeline**: None for official app, unknown for OAuth action

**Recommendation**: Use **official Claude Code Action with API key** for production CI/CD, or community OAuth action for cost optimization with acceptable risk.

---

### Method 6: Unofficial Web API (Reverse Engineering)

#### Authentication Mechanism

**Token Type**: Claude.ai session tokens (web application cookies)

**How Tokens Are Obtained**:
1. User logs into claude.ai in browser
2. Session cookie extracted from browser DevTools
3. Cookie used in API requests to unofficial endpoints

**Endpoints** (unofficial, subject to change):
- `https://claude.ai/api/organizations/{org_id}/chat_conversations`
- `https://claude.ai/api/append_message`
- Other undocumented endpoints

**How Tokens Are Used**:
```python
import requests

headers = {
    "Cookie": "sessionKey=your-session-cookie",
    "User-Agent": "Mozilla/5.0...",
    "Content-Type": "application/json"
}

# Unofficial API call
response = requests.post(
    "https://claude.ai/api/append_message",
    headers=headers,
    json={
        "prompt": "Hello, Claude!",
        "conversation_id": "...",
        # Other undocumented parameters
    }
)
```

**Session Duration**: Unknown (likely 24 hours to several days)

**PKCE**: Not applicable (web session, not OAuth)

**Scopes**: N/A (full web app access)

#### Capabilities

**Supported** (based on web app features):
- Text generation (streaming and non-streaming)
- File uploads
- Multi-turn conversations
- Projects (codebase context)

**Not Supported**:
- Official API features (tool use may differ)
- MCP server integration (web app only)
- Agent SDK compatibility (incompatible)

**Reliability**: Low (endpoints can change without notice)

#### Rate Limits & Restrictions

**Web App Limits**: Same as Claude.ai subscription limits

**Unofficial API Risks**:
- May trigger bot detection
- Could result in account suspension
- No documented rate limits (trial and error)

#### Context Window Size

**Same as Web App**: 200,000 tokens (standard), 500,000 tokens (Enterprise)

#### Model Access

**Same as Subscription**: Access to models available in claude.ai web app

#### Subscription Requirements

**Minimum Tier**: Claude Free (but limited), Pro or Max recommended

#### Technical Implementation

**Community Libraries**:
```python
# Example using unofficial community library
from claude_unofficial_api import ClaudeAPI

# Initialize with session key
claude = ClaudeAPI(session_key="your-session-cookie")

# Send message
response = claude.send_message(
    prompt="Analyze this code",
    conversation_id="existing-conversation-or-new"
)

print(response['completion'])
```

**Configuration Requirements**:
- Session cookie extraction from browser
- Cookie refresh when expired (manual re-login)
- User-Agent spoofing (to mimic browser)

**Dependency Requirements**:
- `requests` or `httpx` for HTTP calls
- Community libraries (e.g., `claude-unofficial-api`)
- Cookie management (manual or automated)

**Error Handling**:
- No official error codes
- HTML error pages instead of JSON
- Fragile parsing required

#### Pros & Cons

**Strengths**:
- ✅ Access to web app features (Projects, Artifacts)
- ✅ No API key needed (uses subscription)
- ✅ Free tier access (limited)

**Weaknesses**:
- ❌ **5x smaller context window** (200K vs 1M tokens)
- ❌ **Violates Anthropic Terms of Service** (critical risk)
- ❌ **Extremely high breaking change risk** (endpoints can change anytime)
- ❌ Account suspension risk
- ❌ No documentation (reverse engineering required)
- ❌ No community support (fragmented libraries)
- ❌ Cookie expiration (manual re-authentication)
- ❌ Bot detection risk
- ❌ Unpredictable errors
- ❌ Legal/compliance issues

**Best Use Cases**:
- **None for production systems** (too risky)
- Personal experimentation only

**Failure Cases**:
- Production environments (ToS violation, instability)
- Enterprise/commercial use (legal risk)
- Compliance-sensitive applications (audit failure)
- Long-term projects (endpoints change without notice)

#### Official vs Community Support

**Official Support Level**: **None - actively discouraged**

**Documentation Quality**: None (reverse-engineered, outdated wikis)

**Community Adoption**: Low (niche, experimental)

**Maintenance Status**: Fragmented (multiple abandoned projects)

**Breaking Change Risk**: **Extreme** (can break at any time)

**Deprecation Timeline**: Endpoints can disappear without notice

**Legal Risk**: Violates Anthropic Terms of Service

**Recommendation**: **DO NOT USE** for Abathur. Not a viable option for any production system.

---

## 3. Comparative Analysis

### Feature Matrix

| Feature | API Key | SDK auth_token | Claude Code CLI | claude_max | MCP OAuth | GitHub Actions OAuth | Unofficial API |
|---------|---------|----------------|-----------------|------------|-----------|---------------------|----------------|
| **Streaming** | ✅ Yes | ✅ Yes | ✅ Yes | ✅ Yes | N/A¹ | ✅ Yes | ⚠️ Limited |
| **Tool Use** | ✅ Yes | ✅ Yes | ✅ Yes | ✅ Yes | ✅ Yes² | ✅ Yes | ⚠️ Different |
| **File Operations** | ⚠️ Base64³ | ⚠️ Base64³ | ✅ Native | ✅ Native | ⚠️ Custom⁴ | ✅ Native | ⚠️ Web Only |
| **MCP Integration** | ✅ Yes | ✅ Yes | ✅ Native | ✅ Native | ✅ Yes² | ✅ Native | ❌ No |
| **Agent SDK Compat** | ✅ Native | ✅ Native | ✅ Yes | ✅ Yes | ⚠️ Partial | ✅ Yes | ❌ No |
| **Prompt Caching** | ✅ Yes | ✅ Yes | ✅ Auto | ✅ Auto | N/A¹ | ✅ Auto | ❌ Unknown |
| **Context Window** | **1M tokens** | **200K** | **200K** | **200K** | N/A¹ | **200K** | **200K** |
| **Official Support** | ✅ Full | ⚠️ Partial⁵ | ✅ Full | ❌ None | ✅ Full | ⚠️ Mixed⁶ | ❌ None |
| **Production Ready** | ✅ Yes | ✅ Yes | ✅ Yes | ❌ No | ✅ Yes | ⚠️ Partial | ❌ No |
| **Breaking Risk** | Low | Low | Low | **High** | Low | Medium | **Extreme** |
| **Setup Complexity** | Low | Medium | Medium | Low | **High** | **High** | Medium |
| **Python Native** | ✅ Yes | ✅ Yes | ❌ No⁷ | ❌ No⁷ | ✅ Yes | ❌ No⁸ | ⚠️ Partial |

**Footnotes**:
1. N/A - MCP provides tools, not LLM access
2. MCP's core functionality is tool integration
3. Base64 encoding required for file content in API
4. MCP servers can implement custom file tools
5. auth_token supported in SDK, but OAuth flow not documented
6. Official GitHub App, community OAuth action
7. Requires Node.js and CLI invocation
8. GitHub Actions environment

**Legend**:
- ✅ Fully supported, production-ready
- ⚠️ Partially supported or with caveats
- ❌ Not supported or not recommended

### Rate Limits Comparison

| Method | Messages/5h | Weekly Limit | Context Window | Concurrent | Cost Model | Overage |
|--------|-------------|--------------|----------------|------------|------------|---------|
| **API Key (Pay-per-token)** | Unlimited¹ | Unlimited¹ | **1M tokens** | Tier-based² | $3/MTok in, $15/MTok out³ | N/A (pay-as-you-go) |
| **OAuth Max 5x ($100/mo)** | 50-200⁴ | 140-280h Sonnet⁵ | 200K tokens | Unknown | Fixed $100/mo | API rates |
| **OAuth Max 20x ($200/mo)** | 200-800⁴ | 240-480h Sonnet⁵ | 200K tokens | Unknown | Fixed $200/mo | API rates |

**Footnotes**:
1. No hard message limit - soft limits based on usage tier and billing
2. Tier 1: 50 RPM, Tier 2: 1000 RPM, Tier 4: Custom (Claude Sonnet 4.5)
3. Sonnet 4.5 pricing, 2x for >200K context
4. Depends on prompt complexity and model
5. Weekly limits introduced August 2025
6. Less than 5% of subscribers affected by weekly limits

**Key Insights**:

1. **Context Window is Critical**: API key provides **5x larger context** (1M vs 200K tokens)
   - Abathur tasks with large codebases will hit OAuth limits
   - Complex multi-file operations favor API key

2. **Rate Limit Trade-offs**:
   - API key: Pay-per-token, no hard message limits (billing protection)
   - OAuth: Fixed cost, but hard limits on messages (50-800/5h)
   - Break-even analysis needed based on usage patterns

3. **Cost Optimization**:
   - High-volume, simple tasks: OAuth subscription may be cheaper
   - Low-volume, complex tasks: API key more cost-effective
   - Mixed workloads: Consider dual-mode with auto-selection

### Cost Analysis

#### Pricing Comparison

| Method | Upfront Cost | Variable Cost | Break-Even Point¹ | Best For |
|--------|--------------|---------------|-------------------|----------|
| API Key | $0 | $3/MTok in, $15/MTok out² | N/A | Variable workloads |
| Max 5x | $100/month | $0 (until limits) | ~67 Sonnet tasks/day³ | Consistent high usage |
| Max 20x | $200/month | $0 (until limits) | ~133 Sonnet tasks/day³ | Very high usage |

**Footnotes**:
1. Break-even assumes average task = 10K input, 5K output tokens
2. Claude Sonnet 4.5 pricing (2025)
3. Simplified estimate - actual depends on prompt complexity

#### Cost Calculation Examples

**Scenario 1: Low-Volume Development**
- Usage: 50 tasks/day, 10K input + 5K output each
- Monthly tokens: 50 × 30 × 15K = 22.5M tokens
- API Key Cost: (22.5M × 10K/15K × $3) + (22.5M × 5K/15K × $15) = $45 + $112.50 = **$157.50/month**
- Max 5x Cost: **$100/month** (within limits)
- **Winner**: Max 5x saves $57.50/month

**Scenario 2: High-Volume Production**
- Usage: 200 tasks/day, 10K input + 5K output each
- Monthly tokens: 200 × 30 × 15K = 90M tokens
- API Key Cost: (90M × 10K/15K × $3) + (90M × 5K/15K × $15) = $180 + $450 = **$630/month**
- Max 20x Cost: $200/month + overage
- Rate limit: 200 tasks/day ≈ 8 tasks/hour (within 200-800/5h limit)
- **Winner**: Max 20x saves ~$430/month (if within limits)

**Scenario 3: Large Context Tasks**
- Usage: 10 tasks/day, 200K input + 50K output each (large codebase)
- Monthly tokens: 10 × 30 × 250K = 75M tokens
- **API Key**: Fits in 1M context (✅ possible)
  - Cost: (75M × 200K/250K × $6⁴) + (75M × 50K/250K × $22.50⁴) = $360 + $337.50 = **$697.50/month**
- **OAuth**: Exceeds 200K context (❌ **cannot execute tasks**)
- **Winner**: API key (only option that works)

**Footnote 4**: Premium pricing for >200K context (2x input, 1.5x output)

#### Hidden Costs

| Cost Factor | API Key | OAuth (Max) |
|-------------|---------|-------------|
| Infrastructure | Minimal (SDK only) | Medium (CLI install or credential management) |
| Development Time | Low (simple SDK) | Medium (OAuth flow, token refresh) |
| Maintenance | Low (stable API) | Medium (token lifecycle, error handling) |
| Support | Official docs | Mixed (official CLI, unofficial OAuth patterns) |
| Lock-in Risk | Low (standard API) | Medium (subscription commitment) |

### Integration Complexity Assessment

| Method | Setup Difficulty | Code Changes | Config Complexity | Maintenance | Debug Difficulty | Overall Score⁵ |
|--------|-----------------|--------------|-------------------|-------------|------------------|---------------|
| **API Key** | ⭐ Easy | ⭐ Minimal | ⭐ Simple | ⭐ Low | ⭐ Easy | **1.0 (baseline)** |
| **SDK auth_token** | ⭐⭐ Medium | ⭐⭐ Moderate | ⭐⭐ Medium | ⭐⭐ Medium | ⭐⭐ Medium | **2.0** |
| **Claude Code CLI** | ⭐⭐ Medium | ⭐⭐⭐ Significant | ⭐⭐ Medium | ⭐⭐ Medium | ⭐⭐⭐ Hard | **2.4** |
| **claude_max** | ⭐ Easy | ⭐⭐ Moderate | ⭐ Simple | ⭐⭐⭐ High | ⭐⭐⭐⭐ Very Hard | **2.6** |
| **MCP OAuth** | ⭐⭐⭐⭐ Very Hard | ⭐⭐⭐⭐ Extensive | ⭐⭐⭐⭐ Complex | ⭐⭐⭐ High | ⭐⭐⭐ Hard | **3.6** |
| **GitHub Actions OAuth** | ⭐⭐⭐ Hard | ⭐⭐ Moderate | ⭐⭐⭐⭐ Complex | ⭐⭐⭐ High | ⭐⭐⭐ Hard | **3.0** |
| **Unofficial API** | ⭐⭐ Medium | ⭐⭐⭐ Significant | ⭐⭐ Medium | ⭐⭐⭐⭐ Very High | ⭐⭐⭐⭐⭐ Extreme | **3.6** |

**Footnote 5**: Overall score = weighted average (setup 20%, code 25%, config 15%, maintenance 25%, debug 15%)

**Legend**: ⭐ = 1 point (easy/simple), ⭐⭐⭐⭐⭐ = 5 points (very hard/complex)

#### Detailed Integration Complexity

**API Key (Baseline)**:
- Setup: Create API key in console, set environment variable
- Code: `client = Anthropic(api_key=os.getenv("ANTHROPIC_API_KEY"))`
- Config: Single environment variable
- Maintenance: Key rotation (optional), billing monitoring
- Debug: Clear error messages, extensive documentation

**SDK auth_token**:
- Setup: Authenticate Claude Code CLI, extract/set token
- Code: Switch from `ANTHROPIC_API_KEY` to `ANTHROPIC_AUTH_TOKEN`, add token refresh logic
- Config: Token file location, refresh parameters
- Maintenance: Token lifecycle management, refresh failure handling
- Debug: OAuth token errors less clear than API key errors

**Claude Code CLI**:
- Setup: Install Node.js, npm, Claude Code CLI, authenticate
- Code: Subprocess invocation, stdout/stderr parsing, error mapping
- Config: CLI installation path, allowed tools, timeout settings
- Maintenance: CLI version updates, token refresh (automatic)
- Debug: Parse CLI stderr, subshell debugging, limited error details

**claude_max**:
- Setup: Install Claude Code + claude_max wrapper
- Code: Subprocess invocation (similar to CLI)
- Config: Simple (wrapper handles auth)
- Maintenance: High risk - monitor for breaking changes, no official support
- Debug: Very difficult - undocumented behavior, fragile env manipulation

**MCP OAuth**:
- Setup: Deploy MCP server, configure OAuth provider (Auth0/Okta), implement Dynamic Client Registration
- Code: MCP server implementation, OAuth middleware, client configuration
- Config: OAuth endpoints, scopes, PKCE settings, server URL
- Maintenance: OAuth provider updates, MCP spec evolution, server uptime
- Debug: OAuth flow debugging, network tracing, protocol compliance

**GitHub Actions OAuth**:
- Setup: Extract tokens manually, add 4 GitHub secrets, create PAT with write access
- Code: Workflow YAML configuration, action parameters
- Config: Complex - tokens, PAT, action versioning, trigger rules
- Maintenance: Monitor token refresh, PAT expiration, action updates
- Debug: Workflow logs, action debugging, token exposure risks

**Unofficial API**:
- Setup: Extract session cookie from browser
- Code: HTTP requests with cookie headers, HTML parsing for errors
- Config: Cookie storage, refresh automation
- Maintenance: Very high - monitor for endpoint changes, cookie expiration, bot detection
- Debug: Extreme - no error codes, trial-and-error, reverse engineering

---

## 4. Security Analysis

### OAuth Token Lifecycle

#### Token Structure and Components

**Claude Code OAuth Tokens** (stored in `.credentials.json`):
```json
{
  "accessToken": "eyJhbGc...",
  "refreshToken": "rt_...",
  "expiresAt": 1728518400000,
  "scopes": ["user:inference", "user:profile"]
}
```

**Token Types**:
1. **Access Token**: Short-lived JWT for API authentication
   - Format: JWT (JSON Web Token)
   - Lifetime: Estimated 1-24 hours (not officially documented)
   - Usage: Bearer token in Authorization header

2. **Refresh Token**: Long-lived token for renewing access
   - Format: Opaque string (RT_ prefix observed)
   - Lifetime: Unknown (days to months, not documented)
   - Usage: POST to `/v1/oauth/token` with `grant_type=refresh_token`

3. **Session Token** (anthropic-sdk-python):
   - Alternative to access token for SDK usage
   - Lifetime: Unknown
   - Usage: `ANTHROPIC_AUTH_TOKEN` environment variable

#### Token Expiration and Refresh Flow

**Automatic Refresh (Claude Code CLI)**:
```
1. Request with expired access token
   ↓
2. API returns 401 Unauthorized
   ↓
3. CLI automatically uses refresh token
   ↓
4. POST https://console.anthropic.com/v1/oauth/token
   {
     "grant_type": "refresh_token",
     "refresh_token": "rt_...",
     "client_id": "9d1c250a-e61b-44d9-88ed-5944d1962f5e"
   }
   ↓
5. Response: new access_token, refresh_token, expires_in
   ↓
6. Update stored credentials
   ↓
7. Retry original request
```

**Manual Refresh (Python Implementation)**:
```python
import requests
import json
import os
from datetime import datetime

def refresh_claude_token():
    """Refresh Claude OAuth token using refresh token."""

    # Load credentials
    creds_path = os.path.expanduser('~/.claude/.credentials.json')
    with open(creds_path) as f:
        creds = json.load(f)

    # Check if refresh needed
    expires_at = creds['expiresAt'] / 1000  # Convert ms to seconds
    if datetime.now().timestamp() < expires_at - 300:  # 5 min buffer
        return creds['accessToken']  # Still valid

    # Refresh token
    response = requests.post(
        'https://console.anthropic.com/v1/oauth/token',
        json={
            'grant_type': 'refresh_token',
            'refresh_token': creds['refreshToken'],
            'client_id': '9d1c250a-e61b-44d9-88ed-5944d1962f5e'
        }
    )

    if response.status_code != 200:
        raise RuntimeError(f"Token refresh failed: {response.text}")

    # Update credentials
    new_creds = response.json()
    creds['accessToken'] = new_creds['access_token']
    creds['refreshToken'] = new_creds['refresh_token']
    creds['expiresAt'] = int(datetime.now().timestamp() * 1000) + (new_creds['expires_in'] * 1000)

    # Save updated credentials
    with open(creds_path, 'w') as f:
        json.dump(creds, f, indent=2)

    return creds['accessToken']
```

**Refresh Token Expiration**:
- Refresh tokens eventually expire (duration undocumented)
- User must re-authenticate: `claude login`
- No programmatic re-authentication flow available

**Known Issues** (GitHub Issues 2025):
- Issue #2633: Token refresh failure during active session
- Issue #2830: OAuth expired error without triggering refresh flow
- Issue #849: AWS token expiration (Bedrock integration)

#### Token Revocation

**Manual Revocation**:
- No documented revocation endpoint
- Logout via `claude logout` removes local credentials
- Tokens may remain valid server-side until expiration

**Security Implications**:
- Stolen tokens remain valid until expiration
- No immediate revocation mechanism for compromised tokens
- Recommendation: Short access token lifetimes

### Storage Best Practices

#### Secure Storage Options

| Storage Method | Security | Portability | Ease of Use | Recommendation |
|----------------|----------|-------------|-------------|----------------|
| **OS Keychain** (macOS Keychain, Windows Credential Manager, Linux Secret Service) | ⭐⭐⭐⭐⭐ Excellent | ⭐⭐ Low | ⭐⭐⭐⭐ Good | ✅ Recommended for local |
| **Encrypted File** (.credentials.json with encryption) | ⭐⭐⭐⭐ Good | ⭐⭐⭐⭐ Good | ⭐⭐⭐ Medium | ✅ Recommended for containers |
| **Environment Variables** (runtime only) | ⭐⭐⭐ Medium | ⭐⭐⭐⭐⭐ Excellent | ⭐⭐⭐⭐⭐ Excellent | ⚠️ OK for CI/CD, not persistent |
| **Configuration Files** (plaintext) | ⭐ Poor | ⭐⭐⭐⭐⭐ Excellent | ⭐⭐⭐⭐⭐ Excellent | ❌ **Never** - security risk |
| **Secrets Manager** (AWS Secrets Manager, HashiCorp Vault) | ⭐⭐⭐⭐⭐ Excellent | ⭐⭐⭐⭐⭐ Excellent | ⭐⭐ Low | ✅ Recommended for production |

#### Implementation Examples

**OS Keychain (macOS)**:
```python
import keyring

# Store OAuth token
keyring.set_password("abathur", "claude_oauth_token", access_token)

# Retrieve OAuth token
token = keyring.get_password("abathur", "claude_oauth_token")
```

**Encrypted File (Portable)**:
```python
from cryptography.fernet import Fernet
import json
import os

def save_encrypted_credentials(access_token, refresh_token, key_path='.abathur/.key'):
    """Save OAuth credentials encrypted with Fernet."""

    # Load or generate encryption key
    if os.path.exists(key_path):
        with open(key_path, 'rb') as f:
            key = f.read()
    else:
        key = Fernet.generate_key()
        os.makedirs(os.path.dirname(key_path), exist_ok=True)
        with open(key_path, 'wb') as f:
            f.write(key)
        os.chmod(key_path, 0o600)  # Restrict permissions

    # Encrypt credentials
    fernet = Fernet(key)
    creds = json.dumps({
        'accessToken': access_token,
        'refreshToken': refresh_token,
        'expiresAt': int(time.time() * 1000) + 3600000  # 1 hour
    }).encode()
    encrypted = fernet.encrypt(creds)

    # Save encrypted credentials
    creds_path = '.abathur/.credentials.enc'
    with open(creds_path, 'wb') as f:
        f.write(encrypted)
    os.chmod(creds_path, 0o600)

def load_encrypted_credentials(key_path='.abathur/.key'):
    """Load and decrypt OAuth credentials."""
    with open(key_path, 'rb') as f:
        key = f.read()

    fernet = Fernet(key)

    with open('.abathur/.credentials.enc', 'rb') as f:
        encrypted = f.read()

    decrypted = fernet.decrypt(encrypted)
    return json.loads(decrypted)
```

**Environment Variables (CI/CD)**:
```python
import os

# Load from environment
access_token = os.getenv('CLAUDE_OAUTH_ACCESS_TOKEN')
refresh_token = os.getenv('CLAUDE_OAUTH_REFRESH_TOKEN')

# Validate presence
if not access_token or not refresh_token:
    raise ValueError("OAuth credentials not found in environment")
```

**AWS Secrets Manager (Production)**:
```python
import boto3
import json

def get_oauth_credentials_from_secrets_manager(secret_name='abathur/claude-oauth'):
    """Retrieve OAuth credentials from AWS Secrets Manager."""

    client = boto3.client('secretsmanager')

    response = client.get_secret_value(SecretId=secret_name)
    secret = json.loads(response['SecretString'])

    return {
        'access_token': secret['accessToken'],
        'refresh_token': secret['refreshToken'],
        'expires_at': secret['expiresAt']
    }
```

#### Encryption Requirements

**At Rest**:
- OAuth tokens MUST be encrypted when stored in files
- Use industry-standard encryption (AES-256, Fernet)
- Encryption keys stored separately from credentials
- File permissions: 0600 (owner read/write only)

**In Transit**:
- HTTPS/TLS for all OAuth endpoints (enforced by Anthropic)
- No additional encryption needed (TLS 1.2+)

**In Memory**:
- No special encryption (OS memory protection)
- Clear tokens from memory after use (overwrite variables)
- Avoid logging tokens (redact in logs)

#### Access Control Considerations

**File-Based Storage**:
- Restrict file permissions: `chmod 600 .credentials.json`
- Store in user home directory: `~/.claude/`
- Never commit to version control (.gitignore)

**Environment Variables**:
- Avoid in shared environments (visible in process list)
- Clear after use if possible
- Use secret management in CI/CD (GitHub Secrets, GitLab CI/CD variables)

**Multi-User Systems**:
- Per-user credential storage (separate home directories)
- No shared credentials (each user authenticates separately)
- Audit logging for credential access

### Security Comparison: OAuth vs API Key

| Security Aspect | API Key | OAuth Token | Winner |
|-----------------|---------|-------------|--------|
| **Credential Format** | Static key (sk_...) | Short-lived JWT + refresh token | OAuth (time-limited) |
| **Expiration** | Never (manual rotation) | Access: hours, Refresh: days/months | OAuth (automatic expiration) |
| **Scope Control** | Full API access | Granular scopes (user:inference, etc.) | OAuth (principle of least privilege) |
| **Revocation** | Immediate (delete key in console) | Logout or wait for expiration | API Key (better revocation) |
| **Theft Impact** | Full access until revoked | Limited by expiration, requires refresh | OAuth (time-limited damage) |
| **Rotation** | Manual (create new key) | Automatic (refresh flow) | OAuth (automatic rotation) |
| **Audit Trail** | Key usage in API logs | OAuth flow + API usage logs | OAuth (more detailed) |
| **Credential Exposure** | Single point of failure | Access + refresh token (both needed for long-term) | OAuth (defense in depth) |
| **Phishing Risk** | High (steal key = full access) | Medium (steal access token = temporary access) | OAuth (time-limited) |
| **Compliance** (GDPR, SOC2) | Good (if rotated) | Better (automatic rotation, expiration) | OAuth (compliance-friendly) |

**Overall Security Winner**: **OAuth tokens** (time-limited, automatic rotation, granular scopes)

**Caveats**:
- API key simpler to manage securely (no refresh logic)
- OAuth complexity can introduce security bugs (token refresh failures, storage issues)
- Both require secure storage (encryption, access control)

#### Attack Surface Differences

**API Key Attack Vectors**:
1. Environment variable exposure (process dumps, logs)
2. Configuration file compromise (plaintext keys)
3. Source code commits (accidental .env commits)
4. Network interception (if not HTTPS - Anthropic enforces HTTPS)
5. Insider threat (developer with key access)

**OAuth Token Attack Vectors**:
1. Token file compromise (~/.claude/.credentials.json)
2. Refresh token theft (long-lived, high value)
3. OAuth flow interception (PKCE mitigates this)
4. Token refresh endpoint exploitation
5. Session fixation attacks
6. Clock skew attacks (token expiration validation)

**Mitigation Strategies**:

| Attack Vector | API Key Mitigation | OAuth Mitigation |
|---------------|-------------------|------------------|
| Credential Theft | Encrypt at rest, restrict access | Encrypt at rest, short access token lifetime |
| Network Interception | HTTPS (enforced) | HTTPS (enforced) + PKCE |
| Insider Threat | Key rotation, audit logs | Scope limitation, audit logs, token expiration |
| Accidental Exposure | Secret scanning (git hooks) | Same + refresh token separate storage |
| Compromised System | Revoke key immediately | Wait for expiration or implement revocation |

### Multi-User Considerations

#### Per-User Token Isolation

**Architecture**:
```
User A                          User B
  ↓                              ↓
~/.claude/.credentials.json    ~/.claude/.credentials.json
  ↓                              ↓
Abathur (User A context)       Abathur (User B context)
  ↓                              ↓
Claude API (User A limits)     Claude API (User B limits)
```

**Implementation**:
```python
import os
from pathlib import Path

def load_user_credentials(username: str):
    """Load OAuth credentials for specific user."""

    # Per-user credential path
    creds_path = Path.home() / '.claude' / f'.credentials_{username}.json'

    if not creds_path.exists():
        raise ValueError(f"No credentials found for user {username}")

    with open(creds_path) as f:
        return json.load(f)

def initialize_client_for_user(username: str):
    """Initialize Claude client with user-specific OAuth token."""

    creds = load_user_credentials(username)

    os.environ['ANTHROPIC_AUTH_TOKEN'] = creds['accessToken']

    return Anthropic()  # Will use ANTHROPIC_AUTH_TOKEN
```

#### Team/Organization Tokens

**Current State**: No official organization-level OAuth tokens for Claude API

**Workarounds**:
1. **Shared API Key** (not OAuth):
   - Organization creates API key in Console
   - Shared across team (not per-user)
   - Limits: API key approach, not OAuth

2. **Individual OAuth** (current OAuth model):
   - Each user authenticates with personal Claude Max subscription
   - No shared pool of usage
   - Limits: Per-user subscription limits apply

3. **Service Account** (not officially supported):
   - Create dedicated Claude account for automation
   - Use OAuth from that account
   - Risk: Violates ToS if not explicitly allowed

**Recommendation for Abathur**:
- **Single-user model** (per DECISION_POINTS.md)
- Each Abathur instance tied to one user's OAuth credentials
- Multi-user Abathur deployment = multiple instances, each with own credentials

#### Token Sharing Policies

**DO NOT SHARE**:
- ❌ Access tokens across users
- ❌ Refresh tokens across users
- ❌ Credentials files (`.credentials.json`)

**OK TO SHARE** (with caution):
- ⚠️ OAuth client ID (public in OAuth 2.0 spec)
- ⚠️ Claude Code CLI binary (each user authenticates separately)

**Security Implications of Sharing**:
- Shared tokens = shared usage limits (not isolated)
- Shared tokens = shared attribution (audit trail corruption)
- Shared tokens = security risk (one user's compromise affects all)

#### Audit Logging Requirements

**Required Audit Events**:
1. OAuth token acquisition (login)
2. Token refresh success/failure
3. Token usage (API requests)
4. Token expiration
5. Authentication failures

**Implementation Example**:
```python
import structlog
from datetime import datetime

logger = structlog.get_logger()

def audit_log_oauth_event(event_type: str, user: str, details: dict):
    """Centralized OAuth audit logging."""

    logger.info(
        "oauth_audit_event",
        event_type=event_type,
        user=user,
        timestamp=datetime.utcnow().isoformat(),
        **details
    )

# Usage examples
audit_log_oauth_event("token_acquired", "alice", {"method": "claude_login"})
audit_log_oauth_event("token_refreshed", "alice", {"expires_at": "2025-10-10T12:00:00Z"})
audit_log_oauth_event("api_request", "alice", {"model": "claude-sonnet-4", "tokens": 1500})
audit_log_oauth_event("auth_failure", "alice", {"error": "token_expired", "retry": True})
```

**Compliance Considerations**:
- **GDPR**: User consent for credential storage, right to erasure (delete tokens)
- **SOC2**: Audit logs, encryption at rest, access control
- **HIPAA** (if applicable): Additional encryption, access restrictions

---

## 5. Recommendations for Abathur

### Primary OAuth Method Recommendation

**Recommended Approach**: **anthropic-sdk-python with `ANTHROPIC_AUTH_TOKEN`**

#### Rationale

1. **Official SDK Support**: Production-ready, maintained by Anthropic
2. **Python-Native**: Direct integration into Abathur's existing Python stack
3. **Feature Parity**: Full API compatibility (streaming, tool use, function calling)
4. **Low Breaking Change Risk**: Official SDK with semantic versioning
5. **Minimal Code Changes**: Small modification to existing `ClaudeClient` class

#### Implementation Strategy

**Phase 1: Add OAuth Support to ClaudeClient**
```python
# src/abathur/application/claude_client.py (UPDATED)

import os
from anthropic import Anthropic, AsyncAnthropic

class ClaudeClient:
    """Wrapper for Anthropic Claude API with API key and OAuth support."""

    def __init__(
        self,
        api_key: str | None = None,
        auth_token: str | None = None,
        model: str = "claude-sonnet-4-20250514",
        max_retries: int = 3,
        timeout: int = 300,
    ):
        """Initialize Claude client with API key or OAuth token.

        Args:
            api_key: Anthropic API key (if None, reads from ANTHROPIC_API_KEY)
            auth_token: OAuth session token (if None, reads from ANTHROPIC_AUTH_TOKEN)
            model: Default model to use
            max_retries: Maximum retry attempts for transient errors
            timeout: Request timeout in seconds
        """
        # Auto-detect authentication method
        self.auth_mode = self._detect_auth_mode(api_key, auth_token)

        if self.auth_mode == "oauth":
            self.auth_token = auth_token or os.getenv("ANTHROPIC_AUTH_TOKEN")
            if not self.auth_token:
                raise ValueError("ANTHROPIC_AUTH_TOKEN must be provided")

            # Set auth_token in environment (SDK reads this)
            os.environ["ANTHROPIC_AUTH_TOKEN"] = self.auth_token

            # Unset API key to prevent conflicts
            if "ANTHROPIC_API_KEY" in os.environ:
                del os.environ["ANTHROPIC_API_KEY"]
        else:
            self.api_key = api_key or os.getenv("ANTHROPIC_API_KEY")
            if not self.api_key:
                raise ValueError("ANTHROPIC_API_KEY must be provided")

            os.environ["ANTHROPIC_API_KEY"] = self.api_key

        self.model = model
        self.max_retries = max_retries
        self.timeout = timeout

        # Initialize sync and async clients
        self.client = Anthropic(max_retries=max_retries)
        self.async_client = AsyncAnthropic(max_retries=max_retries)

        logger.info("claude_client_initialized", auth_mode=self.auth_mode, model=model)

    def _detect_auth_mode(self, api_key: str | None, auth_token: str | None) -> str:
        """Auto-detect authentication mode based on provided credentials.

        Detection logic (per DECISION_POINTS.md):
        - If auth_token provided or ANTHROPIC_AUTH_TOKEN set: OAuth
        - Else if api_key provided or ANTHROPIC_API_KEY set: API Key
        - Else: Error

        Args:
            api_key: Provided API key
            auth_token: Provided OAuth token

        Returns:
            "oauth" or "api_key"
        """
        auth_token = auth_token or os.getenv("ANTHROPIC_AUTH_TOKEN")
        api_key = api_key or os.getenv("ANTHROPIC_API_KEY")

        if auth_token:
            return "oauth"
        elif api_key:
            return "api_key"
        else:
            raise ValueError("No authentication credentials provided")

    def get_context_window(self) -> int:
        """Get context window size based on authentication mode.

        Returns:
            Context window in tokens
        """
        if self.auth_mode == "oauth":
            # OAuth (subscription): 200K standard, 500K enterprise
            # Conservative estimate: 200K
            return 200_000
        else:
            # API key: 1M tokens
            return 1_000_000

    def estimate_token_count(self, text: str) -> int:
        """Estimate token count (rough approximation: 1 token ≈ 4 characters)."""
        return len(text) // 4

    async def execute_task(
        self,
        system_prompt: str,
        user_message: str,
        max_tokens: int = 8000,
        temperature: float = 0.7,
        model: str | None = None,
    ) -> dict[str, Any]:
        """Execute a task with context window validation."""

        # Validate context window
        input_tokens = self.estimate_token_count(system_prompt + user_message)
        context_window = self.get_context_window()

        if input_tokens > context_window:
            logger.warning(
                "input_exceeds_context_window",
                input_tokens=input_tokens,
                context_window=context_window,
                auth_mode=self.auth_mode
            )

            if self.auth_mode == "oauth":
                raise ValueError(
                    f"Input ({input_tokens} tokens) exceeds OAuth context window "
                    f"({context_window} tokens). Consider using API key authentication "
                    f"for larger context (1M tokens)."
                )

        # Existing execute_task logic...
        # (unchanged from current implementation)
```

**Phase 2: Update ConfigManager for OAuth**
```python
# src/abathur/infrastructure/config.py (UPDATED)

class ConfigManager:
    """Configuration management with OAuth support."""

    def load_claude_credentials(self) -> dict:
        """Load Claude credentials (API key or OAuth token).

        Returns:
            {
                "auth_mode": "api_key" or "oauth",
                "api_key": str or None,
                "auth_token": str or None
            }
        """
        # Check environment variables first
        auth_token = os.getenv("ANTHROPIC_AUTH_TOKEN")
        api_key = os.getenv("ANTHROPIC_API_KEY")

        if auth_token:
            return {
                "auth_mode": "oauth",
                "api_key": None,
                "auth_token": auth_token
            }
        elif api_key:
            return {
                "auth_mode": "api_key",
                "api_key": api_key,
                "auth_token": None
            }
        else:
            # Try loading from keychain or credentials file
            # (implementation depends on storage strategy)
            raise ValueError("No Claude credentials found")
```

**Phase 3: Add Token Refresh Logic**
```python
# src/abathur/application/oauth_manager.py (NEW)

import requests
import json
from pathlib import Path
from datetime import datetime

class OAuthTokenManager:
    """Manage OAuth token lifecycle (refresh, expiration)."""

    def __init__(self, credentials_path: str = None):
        self.credentials_path = credentials_path or str(
            Path.home() / '.claude' / '.credentials.json'
        )
        self.client_id = "9d1c250a-e61b-44d9-88ed-5944d1962f5e"
        self.token_endpoint = "https://console.anthropic.com/v1/oauth/token"

    def get_valid_token(self) -> str:
        """Get valid access token, refreshing if needed.

        Returns:
            Valid access token
        """
        creds = self._load_credentials()

        # Check if token is expired (5 minute buffer)
        expires_at = creds['expiresAt'] / 1000  # ms to seconds
        if datetime.now().timestamp() < expires_at - 300:
            return creds['accessToken']  # Still valid

        # Refresh token
        logger.info("oauth_token_expired_refreshing")
        return self._refresh_token(creds)

    def _refresh_token(self, creds: dict, retry_count: int = 0) -> str:
        """Refresh OAuth token with exponential backoff retry.

        Args:
            creds: Current credentials dictionary
            retry_count: Current retry attempt (0-2)

        Returns:
            New access token

        Raises:
            RuntimeError: If refresh fails after 3 attempts
        """
        max_retries = 3  # Per DECISION_POINTS.md

        try:
            response = requests.post(
                self.token_endpoint,
                json={
                    'grant_type': 'refresh_token',
                    'refresh_token': creds['refreshToken'],
                    'client_id': self.client_id
                },
                timeout=10
            )

            if response.status_code != 200:
                raise RuntimeError(f"Token refresh failed: {response.text}")

            # Update credentials
            new_creds = response.json()
            creds['accessToken'] = new_creds['access_token']
            creds['refreshToken'] = new_creds['refresh_token']
            creds['expiresAt'] = int(
                datetime.now().timestamp() * 1000
            ) + (new_creds['expires_in'] * 1000)

            # Save updated credentials
            self._save_credentials(creds)

            logger.info("oauth_token_refreshed", expires_in=new_creds['expires_in'])
            return creds['accessToken']

        except Exception as e:
            if retry_count < max_retries - 1:
                # Exponential backoff: 1s, 2s, 4s
                sleep_time = 2 ** retry_count
                logger.warning(
                    "oauth_refresh_failed_retrying",
                    retry=retry_count + 1,
                    sleep=sleep_time,
                    error=str(e)
                )
                time.sleep(sleep_time)
                return self._refresh_token(creds, retry_count + 1)
            else:
                logger.error("oauth_refresh_failed_max_retries", error=str(e))
                raise RuntimeError(
                    "OAuth token refresh failed after 3 attempts. "
                    "Please re-authenticate with 'claude login'"
                ) from e

    def _load_credentials(self) -> dict:
        """Load credentials from file."""
        with open(self.credentials_path) as f:
            return json.load(f)

    def _save_credentials(self, creds: dict):
        """Save credentials to file with restricted permissions."""
        with open(self.credentials_path, 'w') as f:
            json.dump(creds, f, indent=2)
        os.chmod(self.credentials_path, 0o600)
```

### Fallback Options

**Secondary Method**: Claude Code CLI subshell invocation (optional)

**When to Use Fallback**:
- User prefers Claude Code CLI workflow
- OAuth token management issues
- Claude Code already authenticated on system

**Implementation** (optional module):
```python
# src/abathur/application/claude_code_client.py (OPTIONAL)

import subprocess

class ClaudeCodeClient:
    """Alternative client using Claude Code CLI subshell."""

    def __init__(self, claude_bin: str = "claude"):
        self.claude_bin = claude_bin
        self._validate_cli_available()

    def _validate_cli_available(self):
        """Check if Claude Code CLI is installed."""
        try:
            result = subprocess.run(
                [self.claude_bin, "--version"],
                capture_output=True,
                timeout=5
            )
            if result.returncode != 0:
                raise RuntimeError("Claude Code CLI not found")
        except FileNotFoundError:
            raise RuntimeError(
                "Claude Code CLI not installed. "
                "Install: npm install -g @anthropic-ai/claude-code"
            )

    async def execute_task(
        self,
        system_prompt: str,
        user_message: str,
        max_tokens: int = 8000,
        temperature: float = 0.7,
        model: str | None = None,
    ) -> dict:
        """Execute task via Claude Code CLI."""

        # Construct prompt (combine system + user)
        full_prompt = f"{system_prompt}\n\n{user_message}"

        # Execute CLI
        result = subprocess.run(
            [self.claude_bin, "-p", full_prompt],
            capture_output=True,
            text=True,
            timeout=300
        )

        if result.returncode != 0:
            return {
                "success": False,
                "content": "",
                "error": result.stderr
            }

        return {
            "success": True,
            "content": result.stdout,
            "error": None
        }
```

### Integration Approach

**Recommended Integration Path**:

1. **Phase 1: Add OAuth Support** (Week 1)
   - Modify `ClaudeClient` to accept `auth_token` parameter
   - Implement auto-detection logic (OAuth vs API key)
   - Add context window detection and validation

2. **Phase 2: Token Lifecycle Management** (Week 2)
   - Implement `OAuthTokenManager` for token refresh
   - Add retry logic (3 attempts with exponential backoff)
   - Integrate with `ClaudeClient` error handling

3. **Phase 3: Configuration Integration** (Week 3)
   - Update `ConfigManager` to load OAuth credentials
   - Support environment variables (`ANTHROPIC_AUTH_TOKEN`)
   - Add credential storage options (keychain, encrypted file)

4. **Phase 4: Testing & Documentation** (Week 4)
   - Unit tests with mocked OAuth flow
   - Integration tests with test OAuth account
   - Update documentation (setup guide, API reference)

**No Breaking Changes** (per DECISION_POINTS.md):
- Existing API key users: No changes required (auto-detection)
- New OAuth users: Set `ANTHROPIC_AUTH_TOKEN` instead of `ANTHROPIC_API_KEY`

### Security Best Practices

**For Abathur Implementation**:

1. **Token Storage**:
   - **Local Development**: OS keychain (macOS Keychain, Linux Secret Service)
   - **Containers/Docker**: Encrypted file with key in environment variable
   - **Production**: AWS Secrets Manager or HashiCorp Vault

2. **Token Refresh**:
   - Automatic refresh on 401 errors
   - 3 retry attempts with exponential backoff (1s, 2s, 4s)
   - Clear error messages prompting re-authentication after failures

3. **Credential Protection**:
   - Never log access or refresh tokens
   - Redact tokens in error messages: `logger.info("oauth_token", token="REDACTED")`
   - Restrict file permissions: `chmod 600 .credentials.json`

4. **Environment Variables**:
   - Use `.env` files for development (not committed to git)
   - Use secret management in CI/CD (GitHub Secrets, GitLab CI/CD variables)
   - Validate presence of credentials before initialization

5. **Audit Logging** (per DECISION_POINTS.md - full metrics tracking):
   ```python
   # Log all auth events
   logger.info("auth_initialized", auth_mode="oauth", user="system")
   logger.info("token_refreshed", expires_at="2025-10-10T12:00:00Z")
   logger.info("api_request", model="claude-sonnet-4", tokens=1500)
   logger.error("auth_failure", error="token_expired", retry_count=3)
   ```

### Testing Strategy

**Unit Tests** (Mocked OAuth):
```python
# tests/test_oauth_client.py

import pytest
from unittest.mock import patch, MagicMock
from abathur.application.claude_client import ClaudeClient

@pytest.fixture
def mock_oauth_token():
    return "mock-oauth-token-12345"

@pytest.fixture
def mock_anthropic_client():
    with patch('abathur.application.claude_client.AsyncAnthropic') as mock:
        yield mock

def test_oauth_auth_mode_detection(mock_oauth_token):
    """Test auto-detection of OAuth authentication mode."""
    client = ClaudeClient(auth_token=mock_oauth_token)
    assert client.auth_mode == "oauth"

def test_api_key_auth_mode_detection():
    """Test auto-detection of API key authentication mode."""
    client = ClaudeClient(api_key="sk-test-12345")
    assert client.auth_mode == "api_key"

def test_context_window_oauth():
    """Test OAuth context window is 200K tokens."""
    client = ClaudeClient(auth_token="mock-token")
    assert client.get_context_window() == 200_000

def test_context_window_api_key():
    """Test API key context window is 1M tokens."""
    client = ClaudeClient(api_key="sk-test")
    assert client.get_context_window() == 1_000_000

@pytest.mark.asyncio
async def test_execute_task_context_window_exceeded_oauth():
    """Test error when OAuth input exceeds 200K context window."""
    client = ClaudeClient(auth_token="mock-token")

    # Create input exceeding 200K tokens (~800K characters)
    large_input = "x" * 800_000

    with pytest.raises(ValueError, match="exceeds OAuth context window"):
        await client.execute_task(
            system_prompt="Test",
            user_message=large_input
        )

@pytest.mark.asyncio
async def test_token_refresh_on_401(mock_anthropic_client):
    """Test automatic token refresh on 401 error."""
    # Mock 401 error on first call, success on second
    mock_client = mock_anthropic_client.return_value
    mock_client.messages.create.side_effect = [
        AuthenticationError("OAuth token has expired"),
        MagicMock(content=[MagicMock(text="Success")])
    ]

    with patch('abathur.application.oauth_manager.OAuthTokenManager.get_valid_token') as mock_refresh:
        mock_refresh.return_value = "new-token"

        client = ClaudeClient(auth_token="old-token")
        result = await client.execute_task("system", "user")

        # Verify token refresh was called
        assert mock_refresh.called
        assert result["success"] is True
```

**Integration Tests** (Test Account):
```python
# tests/integration/test_oauth_integration.py

import pytest
import os

@pytest.mark.integration
@pytest.mark.skipif(not os.getenv("CLAUDE_TEST_OAUTH_TOKEN"), reason="No test OAuth token")
def test_real_oauth_authentication():
    """Integration test with real OAuth token (test account)."""
    client = ClaudeClient(auth_token=os.getenv("CLAUDE_TEST_OAUTH_TOKEN"))

    result = asyncio.run(client.execute_task(
        system_prompt="You are a helpful assistant.",
        user_message="Say 'OAuth integration test successful'",
        max_tokens=50
    ))

    assert result["success"] is True
    assert "successful" in result["content"].lower()

@pytest.mark.integration
def test_token_refresh_integration():
    """Test token refresh with real Claude OAuth endpoint (mocked credentials)."""
    # This test uses a test refresh token from a dedicated test account
    # NOT run in CI/CD - manual only
    pass
```

**Test Coverage Goals**:
- Unit tests: 90%+ coverage (mocked OAuth)
- Integration tests: Manual with test accounts (not CI/CD)
- Error handling: All retry paths tested
- Context window: Both OAuth and API key limits validated

---

## 6. Open Questions

### Questions Requiring Human Input

1. **OAuth Token Acquisition Flow**:
   - Q: Should Abathur implement a custom OAuth flow (browser-based), or rely on users authenticating via Claude Code CLI first?
   - Implications: Custom flow = more user-friendly, but significant implementation complexity
   - Recommendation: **Rely on Claude Code CLI** for initial authentication (simpler, leverages official tool)

2. **Context Window Mitigation**:
   - Q: How should Abathur handle tasks that exceed OAuth's 200K context window?
   - Options:
     a) Automatically fall back to API key if user has both configured
     b) Fail with error message recommending API key
     c) Implement automatic context truncation (risky - may break tasks)
   - Recommendation: **Option B** (fail with clear error) - per DECISION_POINTS.md, no automatic fallback

3. **Token Storage in Containers**:
   - Q: For Docker deployments, should Abathur support:
     a) Volume-mounted credentials file from host
     b) Environment variables only (ANTHROPIC_AUTH_TOKEN)
     c) Integration with secrets management (AWS Secrets Manager, Vault)
   - Recommendation: **Support all three**, with docs for each use case

4. **Rate Limit Enforcement**:
   - Q: Should Abathur track OAuth rate limits (50-800 per 5h) and prevent task submission when limit reached?
   - Options:
     a) Track usage and block submission (require wait)
     b) Warn but allow (let Anthropic API enforce)
     c) Smart scheduling (queue tasks until window resets)
   - Decision (from DECISION_POINTS.md): **Option B** (ignore, let Anthropic handle)

5. **Multi-Subscription Support**:
   - Q: Should Abathur support multiple OAuth accounts (different Max subscriptions) for load balancing?
   - Implications: Complex implementation, but could bypass rate limits via multiple accounts
   - Recommendation: **Out of scope for initial implementation** (single-user model per DECISION_POINTS.md)

6. **OAuth Failure Recovery**:
   - Q: After OAuth token refresh fails 3 times, should Abathur:
     a) Require manual intervention (re-run `claude login`)
     b) Attempt automatic re-authentication (browser flow)
     c) Fall back to API key if configured
   - Decision (from DECISION_POINTS.md): **Option A** (manual re-auth, no API key fallback)

### Areas Needing Further Investigation

1. **Token Expiration Duration**:
   - Finding: Access token lifetime is not officially documented
   - Observed: Appears to be 1-24 hours based on GitHub issues
   - Action: Monitor token expiration in production, log metrics

2. **Refresh Token Lifetime**:
   - Finding: No official documentation on refresh token expiration
   - Risk: Users may experience unexpected re-authentication requirements
   - Action: Implement telemetry to track refresh token age

3. **OAuth Scopes Evolution**:
   - Finding: Current scopes are `user:inference`, `user:profile`, `org:create_api_key`
   - Question: Will Anthropic add more granular scopes (e.g., per-model, per-feature)?
   - Action: Design scope configuration to be extensible

4. **Enterprise OAuth Features**:
   - Finding: Enterprise plans have 500K context window vs 200K standard
   - Question: Are there other OAuth benefits for Enterprise (rate limits, models)?
   - Action: Document Enterprise-specific OAuth features when available

5. **Claude Code SDK Alignment**:
   - Finding: Claude Agent SDK documentation doesn't explicitly cover OAuth authentication patterns
   - Question: Will official SDK documentation improve OAuth coverage?
   - Action: Monitor SDK docs for OAuth best practices

### Assumptions Made

1. **OAuth Token Format**:
   - Assumption: Access tokens are JWTs, refresh tokens are opaque strings
   - Basis: Observed format in GitHub issues and community tools
   - Risk: Low (standard OAuth 2.0 pattern)

2. **Client ID Stability**:
   - Assumption: Client ID `9d1c250a-e61b-44d9-88ed-5944d1962f5e` is stable for Claude Code
   - Basis: Consistent across all OAuth references
   - Risk: Low (public client ID, unlikely to change)

3. **PKCE Requirement**:
   - Assumption: PKCE is required for all OAuth flows
   - Basis: Observed in OAuth URLs (code_challenge_method=S256)
   - Risk: Low (PKCE is OAuth 2.1 standard)

4. **Token Refresh Endpoint**:
   - Assumption: `https://console.anthropic.com/v1/oauth/token` is stable
   - Basis: Documented in community tools, GitHub issues
   - Risk: Medium (unofficial endpoint, could change)

5. **Context Window Detection**:
   - Assumption: OAuth always has 200K context (500K for Enterprise)
   - Basis: Official Anthropic documentation
   - Risk: Low (officially documented limits)

6. **Rate Limit Enforcement**:
   - Assumption: Anthropic enforces rate limits server-side (429 errors)
   - Basis: Standard API practice
   - Risk: Low (confirmed in docs)

---

## 7. References

### Official Anthropic Sources

1. **Claude API Documentation**
   - URL: https://docs.claude.com/en/api/overview
   - Accessed: October 9, 2025
   - Content: Official API reference, authentication methods

2. **Claude Code Identity & Access Management**
   - URL: https://docs.claude.com/en/docs/claude-code/iam
   - Accessed: October 9, 2025
   - Content: Claude Code authentication, credential storage, OAuth support

3. **Claude Agent SDK GitHub Repository**
   - URL: https://github.com/anthropics/claude-agent-sdk-python
   - Accessed: October 9, 2025
   - Version: 2.0.0+
   - Content: SDK installation, basic usage examples

4. **anthropic-sdk-python GitHub Repository**
   - URL: https://github.com/anthropics/anthropic-sdk-python
   - Accessed: October 9, 2025
   - Content: Authentication options (api_key, auth_token), client initialization

5. **Claude Context Windows Documentation**
   - URL: https://docs.claude.com/en/docs/build-with-claude/context-windows
   - Accessed: October 9, 2025
   - Content: 1M token context window for API, 200K for subscriptions

6. **Claude Max Plan Usage**
   - URL: https://support.claude.com/en/articles/11014257-about-claude-s-max-plan-usage
   - Accessed: October 9, 2025
   - Content: Max 5x/20x rate limits, pricing, weekly limits

7. **Model Context Protocol (MCP) Specification**
   - URL: https://modelcontextprotocol.io/specification/2025-03-26/basic/authorization
   - Accessed: October 9, 2025
   - Content: OAuth 2.1 requirements, Dynamic Client Registration

8. **Claude Code GitHub Actions Documentation**
   - URL: https://docs.claude.com/en/docs/claude-code/github-actions
   - Accessed: October 9, 2025
   - Content: Official GitHub Actions integration, authentication methods

### Community Resources

9. **"How I Built claude_max" Blog Post**
   - URL: https://idsc2025.substack.com/p/how-i-built-claude_max-to-unlock
   - Author: Arthur Colle (@arthurcolle)
   - Date: June 2025
   - Content: claude_max tool implementation, OAuth PKCE details

10. **Claude Code OAuth Login GitHub Action**
    - URL: https://github.com/marketplace/actions/claude-code-oauth-login
    - Author: grll (Guillaume Rouleau)
    - Content: Community OAuth action for GitHub workflows

11. **MCP Server Setup with OAuth (Medium)**
    - URL: https://medium.com/neural-engineer/mcp-server-setup-with-oauth-authentication-using-auth0-and-claude-ai-remote-mcp-integration-8329b65e6664
    - Author: PI | Neural Engineer
    - Date: 2025
    - Content: MCP OAuth integration with Auth0

12. **An Introduction to MCP and Authorization (Auth0 Blog)**
    - URL: https://auth0.com/blog/an-introduction-to-mcp-and-authorization/
    - Date: 2025
    - Content: MCP OAuth 2.1 specification overview

13. **MCP Spec Updates from June 2025 (Auth0 Blog)**
    - URL: https://auth0.com/blog/mcp-specs-update-all-about-auth/
    - Date: June 2025
    - Content: MCP June 2025 changelog, Resource Server classification

### Technical References

14. **OAuth 2.1 Specification**
    - URL: https://oauth.net/2.1/
    - Content: OAuth 2.1 standard, PKCE requirement

15. **OAuth 2.0 PKCE (RFC 7636)**
    - URL: https://datatracker.ietf.org/doc/html/rfc7636
    - Content: Proof Key for Code Exchange specification

16. **OAuth 2.0 Dynamic Client Registration (RFC 7591)**
    - URL: https://datatracker.ietf.org/doc/html/rfc7591
    - Content: Dynamic Client Registration Protocol

17. **OAuth 2.0 Authorization Server Metadata (RFC 8414)**
    - URL: https://datatracker.ietf.org/doc/html/rfc8414
    - Content: Authorization server discovery

### GitHub Issues (Error Documentation)

18. **Issue #2633: OAuth Token Refresh Failure During Active Session**
    - URL: https://github.com/anthropics/claude-code/issues/2633
    - Date: June 2025
    - Content: Token refresh bug during active sessions

19. **Issue #2830: Claude Code OAuth Expired Error**
    - URL: https://github.com/anthropics/claude-code/issues/2830
    - Date: 2025
    - Content: OAuth flow not triggering on expiration

20. **Issue #3591: Anthropic API Authentication Token Expired**
    - URL: https://github.com/anthropics/claude-code/issues/3591
    - Date: 2025
    - Content: Token expiration error reporting

21. **Issue #954: OAuth Authentication Error - Missing client_id in WSL**
    - URL: https://github.com/anthropics/claude-code/issues/954
    - Date: 2025
    - Content: OAuth client_id and endpoint details

### Documentation Versions

- Anthropic API Documentation: Version as of October 9, 2025
- Claude Agent SDK: Version 2.0.0+
- anthropic-sdk-python: Latest stable release
- MCP Specification: Version 2025-03-26 (latest)

### Research Date Range

- Primary research conducted: October 9, 2025
- Sources accessed: October 9, 2025
- Latest information from: August-October 2025
- Oldest reference: June 2025 (MCP spec updates)

---

## Appendix A: Code Examples

### Complete OAuth Integration Example

```python
# Complete example: Abathur with OAuth support

import os
import asyncio
from abathur.application.claude_client import ClaudeClient
from abathur.application.oauth_manager import OAuthTokenManager

async def main():
    """Example: Initialize Abathur with OAuth authentication."""

    # Option 1: Environment variable (recommended for production)
    os.environ['ANTHROPIC_AUTH_TOKEN'] = 'your-oauth-token'

    # Option 2: Load from Claude Code credentials
    oauth_manager = OAuthTokenManager()
    token = oauth_manager.get_valid_token()  # Auto-refreshes if needed

    # Initialize client with OAuth
    client = ClaudeClient(auth_token=token)

    print(f"Authentication mode: {client.auth_mode}")
    print(f"Context window: {client.get_context_window():,} tokens")

    # Execute task
    result = await client.execute_task(
        system_prompt="You are a Python code analyzer.",
        user_message="Analyze this function for bugs: def add(a, b): return a - b",
        max_tokens=2000
    )

    if result["success"]:
        print(f"Analysis: {result['content']}")
    else:
        print(f"Error: {result['error']}")

if __name__ == "__main__":
    asyncio.run(main())
```

### Token Refresh Example

```python
# Example: Manual token refresh with error handling

from abathur.application.oauth_manager import OAuthTokenManager
import logging

logging.basicConfig(level=logging.INFO)

def refresh_token_example():
    """Example: Refresh OAuth token with retry logic."""

    manager = OAuthTokenManager()

    try:
        # Get valid token (auto-refreshes if expired)
        token = manager.get_valid_token()
        print(f"Valid token obtained: {token[:20]}...")

    except RuntimeError as e:
        # Refresh failed after 3 retries
        print(f"Token refresh failed: {e}")
        print("Please re-authenticate: claude login")

        # In automated systems, trigger alert
        # send_alert("OAuth authentication required")

refresh_token_example()
```

---

## Appendix B: Glossary

**Access Token**: Short-lived JWT used for authenticating API requests (typically 1-24 hours)

**API Key**: Static authentication credential for pay-per-token Claude API access (format: `sk_...`)

**Auth Token**: OAuth session token used by anthropic-sdk-python (via `ANTHROPIC_AUTH_TOKEN`)

**Client ID**: Public identifier for OAuth client application (Claude Code: `9d1c250a-e61b-44d9-88ed-5944d1962f5e`)

**Context Window**: Maximum number of tokens (input + output) a model can process in one request

**DCR (Dynamic Client Registration)**: OAuth mechanism for clients to register programmatically (RFC 7591)

**JWT (JSON Web Token)**: Encoded token format used for access tokens

**MCP (Model Context Protocol)**: Anthropic protocol for standardizing LLM tool integrations

**OAuth 2.1**: Modern OAuth specification with PKCE requirement

**PKCE (Proof Key for Code Exchange)**: Security extension for OAuth preventing authorization code interception

**Refresh Token**: Long-lived token used to obtain new access tokens without re-authentication

**Scope**: Permission granted to OAuth token (e.g., `user:inference`, `user:profile`)

**Session Token**: Alternative term for OAuth access token in Claude context

---

**END OF REPORT**

**Total Research Duration**: ~3 hours
**Sources Consulted**: 21 primary sources
**OAuth Methods Analyzed**: 6 methods
**Recommendation Confidence**: High (official SDK support, production-ready)
