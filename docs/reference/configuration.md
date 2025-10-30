# Configuration Reference

Complete reference for configuring Abathur Swarm via YAML configuration file or environment variables.

## Configuration File Location

**Default Path**: `.abathur/config.yaml`

The configuration file is automatically loaded from the `.abathur` directory in your project root. You can override the location using:

```bash
abathur --config /path/to/custom/config.yaml
```

**Environment Variable**: `ABATHUR_CONFIG`

```bash
export ABATHUR_CONFIG=/path/to/custom/config.yaml
```

---

## Configuration Schema

### Root Configuration

Top-level configuration structure.

| Field | Type | Required | Default | Description |
|-------|------|----------|---------|-------------|
| `max_agents` | `integer` | No | `10` | Maximum number of concurrent agents (1-100) |
| `database` | `object` | No | See [Database](#database-configuration) | Database configuration |
| `logging` | `object` | No | See [Logging](#logging-configuration) | Logging configuration |
| `rate_limit` | `object` | No | See [Rate Limiting](#rate-limiting-configuration) | Rate limiting configuration |
| `retry` | `object` | No | See [Retry Policy](#retry-policy-configuration) | Retry policy configuration |
| `mcp_servers` | `array` | No | `[]` | MCP server configurations |
| `substrates` | `object` | No | See [Substrates](#substrates-configuration) | LLM substrate configurations |

**Example**:

```yaml
max_agents: 10
database:
  path: ".abathur/abathur.db"
  max_connections: 10
logging:
  level: "info"
  format: "json"
  retention_days: 30
```

---

## Database Configuration

SQLite database configuration for task queue and memory storage.

| Field | Type | Required | Default | Description |
|-------|------|----------|---------|-------------|
| `path` | `string` | No | `.abathur/abathur.db` | Path to SQLite database file |
| `max_connections` | `integer` | No | `10` | Maximum number of connections in pool (1-100) |

**Example**:

```yaml
database:
  path: ".abathur/abathur.db"
  max_connections: 10
```

**Custom Database Location**:

```yaml
database:
  path: "/var/lib/abathur/production.db"
  max_connections: 20
```

**Environment Variables**:

```bash
export ABATHUR_DATABASE__PATH=/var/lib/abathur/production.db
export ABATHUR_DATABASE__MAX_CONNECTIONS=20
```

!!! note "Database File Creation"
    The database file and parent directories are created automatically if they don't exist. Migrations run automatically on first startup.

!!! tip "Connection Pool Sizing"
    For high-concurrency workloads, increase `max_connections` to match or exceed `max_agents`.

---

## Logging Configuration

Structured logging with JSON or pretty-printed output.

| Field | Type | Required | Default | Description |
|-------|------|----------|---------|-------------|
| `level` | `string` | No | `info` | Log level: `trace`, `debug`, `info`, `warn`, `error` |
| `format` | `string` | No | `json` | Output format: `json`, `pretty` |
| `retention_days` | `integer` | No | `30` | Number of days to retain logs (0-365) |

**Example**:

```yaml
logging:
  level: "info"
  format: "json"
  retention_days: 30
```

**Development Logging**:

```yaml
logging:
  level: "debug"
  format: "pretty"
  retention_days: 7
```

**Production Logging**:

```yaml
logging:
  level: "warn"
  format: "json"
  retention_days: 90
```

**Environment Variables**:

```bash
export ABATHUR_LOGGING__LEVEL=debug
export ABATHUR_LOGGING__FORMAT=pretty
export ABATHUR_LOGGING__RETENTION_DAYS=7
```

### Log Levels

| Level | Description | Use Case |
|-------|-------------|----------|
| `trace` | All events including internal details | Deep debugging, performance analysis |
| `debug` | Detailed diagnostic information | Development, troubleshooting |
| `info` | General informational messages | Production default, audit trail |
| `warn` | Warning messages for potential issues | Production monitoring |
| `error` | Error messages for failures | Production monitoring, alerting |

!!! tip "Performance Consideration"
    Use `json` format in production for structured logging. Use `pretty` format during development for human-readable output.

---

## Rate Limiting Configuration

Token bucket rate limiting for LLM API calls.

| Field | Type | Required | Default | Description |
|-------|------|----------|---------|-------------|
| `requests_per_second` | `float` | No | `10.0` | Sustained requests per second allowed (0.1-100.0) |
| `burst_size` | `integer` | No | `20` | Maximum burst size for token bucket (1-1000) |

**Example**:

```yaml
rate_limit:
  requests_per_second: 10.0
  burst_size: 20
```

**High-Throughput Configuration**:

```yaml
rate_limit:
  requests_per_second: 50.0
  burst_size: 100
```

**Conservative Configuration**:

```yaml
rate_limit:
  requests_per_second: 2.0
  burst_size: 5
```

**Environment Variables**:

```bash
export ABATHUR_RATE_LIMIT__REQUESTS_PER_SECOND=50.0
export ABATHUR_RATE_LIMIT__BURST_SIZE=100
```

!!! info "Token Bucket Algorithm"
    The rate limiter uses a token bucket algorithm: `requests_per_second` defines the steady-state rate, while `burst_size` allows temporary spikes above that rate.

!!! warning "API Rate Limits"
    Ensure rate limit settings comply with your LLM provider's API rate limits. Anthropic Claude API typically allows 50 requests/minute for free tier, higher for paid tiers.

---

## Retry Policy Configuration

Exponential backoff retry policy for transient failures.

| Field | Type | Required | Default | Description |
|-------|------|----------|---------|-------------|
| `max_retries` | `integer` | No | `3` | Maximum number of retry attempts (0-10) |
| `initial_backoff_ms` | `integer` | No | `10000` | Initial backoff delay in milliseconds (100-60000) |
| `max_backoff_ms` | `integer` | No | `300000` | Maximum backoff delay in milliseconds (1000-600000) |

**Example**:

```yaml
retry:
  max_retries: 3
  initial_backoff_ms: 10000
  max_backoff_ms: 300000
```

**Aggressive Retry Policy**:

```yaml
retry:
  max_retries: 5
  initial_backoff_ms: 5000
  max_backoff_ms: 120000
```

**Conservative Retry Policy**:

```yaml
retry:
  max_retries: 2
  initial_backoff_ms: 20000
  max_backoff_ms: 600000
```

**Environment Variables**:

```bash
export ABATHUR_RETRY__MAX_RETRIES=5
export ABATHUR_RETRY__INITIAL_BACKOFF_MS=5000
export ABATHUR_RETRY__MAX_BACKOFF_MS=120000
```

### Backoff Calculation

Backoff delay is calculated using exponential backoff with jitter:

```
delay = min(initial_backoff_ms * (2 ^ attempt) + random_jitter, max_backoff_ms)
```

| Attempt | Example Delay (initial=10s, max=300s) |
|---------|---------------------------------------|
| 1 | ~10 seconds |
| 2 | ~20 seconds |
| 3 | ~40 seconds |
| 4 | ~80 seconds |
| 5 | ~160 seconds |
| 6+ | 300 seconds (max) |

!!! tip "Retry Policy Tuning"
    For production systems with strict SLAs, use fewer retries with shorter backoffs. For batch processing, use more retries with longer backoffs to handle transient failures gracefully.

---

## MCP Server Configuration

Model Context Protocol (MCP) server configurations for extending capabilities.

| Field | Type | Required | Default | Description |
|-------|------|----------|---------|-------------|
| `name` | `string` | Yes | - | Unique server identifier |
| `command` | `string` | Yes | - | Command to execute |
| `args` | `array` | No | `[]` | Command arguments |
| `env` | `object` | No | `{}` | Environment variables (key-value pairs) |

**Example**:

```yaml
mcp_servers:
  - name: "memory"
    command: "npx"
    args:
      - "-y"
      - "@modelcontextprotocol/server-memory"
    env: {}

  - name: "github"
    command: "npx"
    args:
      - "-y"
      - "@modelcontextprotocol/server-github"
    env:
      GITHUB_TOKEN: "${GITHUB_TOKEN}"
```

**Custom MCP Server**:

```yaml
mcp_servers:
  - name: "custom-db"
    command: "/usr/local/bin/mcp-database-server"
    args:
      - "--port"
      - "9000"
    env:
      DB_CONNECTION_STRING: "postgresql://localhost/mydb"
      LOG_LEVEL: "debug"
```

!!! info "MCP Server Discovery"
    MCP servers are started automatically when Abathur initializes. Health checks run every 30 seconds to ensure server availability.

!!! warning "Environment Variable Substitution"
    Environment variables in `env` fields support `${VAR_NAME}` syntax for substitution from the parent process environment.

---

## Substrates Configuration

LLM substrate configuration for routing agent execution to different LLM providers.

| Field | Type | Required | Default | Description |
|-------|------|----------|---------|-------------|
| `default_substrate` | `string` | No | `claude-code` | Default substrate for agents with no mapping |
| `enabled` | `array` | No | `["claude-code"]` | List of enabled substrate types |
| `claude_code` | `object` | No | See [Claude Code](#claude-code-substrate) | Claude Code substrate configuration |
| `anthropic_api` | `object` | No | See [Anthropic API](#anthropic-api-substrate) | Anthropic API substrate configuration |
| `agent_mappings` | `object` | No | `{}` | Agent type to substrate mappings |

**Example**:

```yaml
substrates:
  default_substrate: "claude-code"
  enabled:
    - "claude-code"
  claude_code:
    claude_path: "claude"
    timeout_secs: 300
  agent_mappings: {}
```

**Multi-Substrate Configuration**:

```yaml
substrates:
  default_substrate: "claude-code"
  enabled:
    - "claude-code"
    - "anthropic-api"
  claude_code:
    claude_path: "claude"
    working_dir: "/workspace"
    timeout_secs: 600
  anthropic_api:
    enabled: true
    model: "claude-sonnet-4-5-20250929"
  agent_mappings:
    "rust-.*": "anthropic-api"
    "documentation-.*": "claude-code"
```

**Environment Variables**:

```bash
export ABATHUR_SUBSTRATES__DEFAULT_SUBSTRATE=anthropic-api
export ABATHUR_SUBSTRATES__ENABLED="claude-code,anthropic-api"
```

---

### Claude Code Substrate

Configuration for Claude Code CLI substrate.

| Field | Type | Required | Default | Description |
|-------|------|----------|---------|-------------|
| `claude_path` | `string` | No | `claude` | Path to Claude CLI executable |
| `working_dir` | `string` | No | `null` | Working directory for Claude execution |
| `timeout_secs` | `integer` | No | `300` | Default timeout in seconds (10-3600) |

**Example**:

```yaml
claude_code:
  claude_path: "claude"
  working_dir: null
  timeout_secs: 300
```

**Custom Claude Installation**:

```yaml
claude_code:
  claude_path: "/usr/local/bin/claude"
  working_dir: "/workspace/projects"
  timeout_secs: 600
```

**Environment Variables**:

```bash
export ABATHUR_SUBSTRATES__CLAUDE_CODE__CLAUDE_PATH=/usr/local/bin/claude
export ABATHUR_SUBSTRATES__CLAUDE_CODE__WORKING_DIR=/workspace/projects
export ABATHUR_SUBSTRATES__CLAUDE_CODE__TIMEOUT_SECS=600
```

!!! note "Claude CLI Requirement"
    The Claude Code substrate requires the Claude CLI to be installed and accessible in PATH or at the specified `claude_path`.

---

### Anthropic API Substrate

Configuration for direct Anthropic API substrate.

| Field | Type | Required | Default | Description |
|-------|------|----------|---------|-------------|
| `enabled` | `boolean` | No | `false` | Enable Anthropic API substrate |
| `api_key` | `string` | No | `null` | API key (or use `ANTHROPIC_API_KEY` env var) |
| `model` | `string` | No | `claude-sonnet-4-5-20250929` | Model identifier |
| `base_url` | `string` | No | `null` | Base URL for API (for testing/proxies) |

**Example**:

```yaml
anthropic_api:
  enabled: false
  api_key: null
  model: "claude-sonnet-4-5-20250929"
  base_url: null
```

**Enabled Configuration**:

```yaml
anthropic_api:
  enabled: true
  model: "claude-sonnet-4-5-20250929"
```

**Custom Model**:

```yaml
anthropic_api:
  enabled: true
  model: "claude-opus-4-5-20250929"
```

**Custom API Endpoint**:

```yaml
anthropic_api:
  enabled: true
  model: "claude-sonnet-4-5-20250929"
  base_url: "https://api-proxy.example.com/v1"
```

**Environment Variables**:

```bash
export ABATHUR_SUBSTRATES__ANTHROPIC_API__ENABLED=true
export ANTHROPIC_API_KEY=sk-ant-api03-...
export ABATHUR_SUBSTRATES__ANTHROPIC_API__MODEL=claude-sonnet-4-5-20250929
export ABATHUR_SUBSTRATES__ANTHROPIC_API__BASE_URL=https://api-proxy.example.com/v1
```

!!! danger "API Key Security"
    Never commit API keys to version control. Use environment variables or secure secret management systems.

!!! tip "Model Selection"
    - **claude-sonnet-4-5**: Balanced performance and cost
    - **claude-opus-4-5**: Maximum capability, higher cost
    - **claude-haiku-4-5**: Fast and economical

---

### Agent Mappings

Map agent type patterns to specific substrates.

**Type**: `object` (key-value pairs)

**Key**: Agent type pattern (supports regex patterns)
**Value**: Substrate name (`claude-code` or `anthropic-api`)

**Example**:

```yaml
agent_mappings:
  "rust-.*": "anthropic-api"
  "documentation-.*": "claude-code"
  "requirements-gatherer": "anthropic-api"
```

**Pattern Matching Rules**:

- Patterns are matched using regex
- First matching pattern wins
- If no pattern matches, `default_substrate` is used

**Example Matching**:

| Agent Type | Pattern | Substrate |
|------------|---------|-----------|
| `rust-testing-specialist` | `rust-.*` | `anthropic-api` |
| `documentation-content-writer` | `documentation-.*` | `claude-code` |
| `requirements-gatherer` | `requirements-gatherer` | `anthropic-api` |
| `custom-agent` | (no match) | `default_substrate` |

!!! tip "Performance Optimization"
    Route compute-intensive agents (Rust compilation, complex analysis) to API substrates for better performance. Route document editing and CLI tasks to Claude Code substrate.

---

## Environment Variables

All configuration options can be overridden using environment variables with the `ABATHUR_` prefix.

### Environment Variable Naming Convention

Configuration keys are mapped to environment variables using:

1. Prefix: `ABATHUR_`
2. Uppercase transformation
3. Nested keys separated by `__` (double underscore)
4. Array indices not supported (use YAML for arrays)

**Examples**:

| YAML Path | Environment Variable |
|-----------|---------------------|
| `max_agents` | `ABATHUR_MAX_AGENTS` |
| `database.path` | `ABATHUR_DATABASE__PATH` |
| `logging.level` | `ABATHUR_LOGGING__LEVEL` |
| `rate_limit.requests_per_second` | `ABATHUR_RATE_LIMIT__REQUESTS_PER_SECOND` |
| `substrates.default_substrate` | `ABATHUR_SUBSTRATES__DEFAULT_SUBSTRATE` |
| `substrates.claude_code.timeout_secs` | `ABATHUR_SUBSTRATES__CLAUDE_CODE__TIMEOUT_SECS` |

### Complete Environment Variable Reference

```bash
# Core Configuration
export ABATHUR_MAX_AGENTS=10

# Database
export ABATHUR_DATABASE__PATH=.abathur/abathur.db
export ABATHUR_DATABASE__MAX_CONNECTIONS=10

# Logging
export ABATHUR_LOGGING__LEVEL=info
export ABATHUR_LOGGING__FORMAT=json
export ABATHUR_LOGGING__RETENTION_DAYS=30

# Rate Limiting
export ABATHUR_RATE_LIMIT__REQUESTS_PER_SECOND=10.0
export ABATHUR_RATE_LIMIT__BURST_SIZE=20

# Retry Policy
export ABATHUR_RETRY__MAX_RETRIES=3
export ABATHUR_RETRY__INITIAL_BACKOFF_MS=10000
export ABATHUR_RETRY__MAX_BACKOFF_MS=300000

# Substrates
export ABATHUR_SUBSTRATES__DEFAULT_SUBSTRATE=claude-code
export ABATHUR_SUBSTRATES__ENABLED="claude-code,anthropic-api"

# Claude Code Substrate
export ABATHUR_SUBSTRATES__CLAUDE_CODE__CLAUDE_PATH=claude
export ABATHUR_SUBSTRATES__CLAUDE_CODE__TIMEOUT_SECS=300

# Anthropic API Substrate
export ABATHUR_SUBSTRATES__ANTHROPIC_API__ENABLED=true
export ANTHROPIC_API_KEY=sk-ant-api03-...
export ABATHUR_SUBSTRATES__ANTHROPIC_API__MODEL=claude-sonnet-4-5-20250929

# Configuration File Override
export ABATHUR_CONFIG=/path/to/custom/config.yaml
```

!!! tip "Environment Variable Precedence"
    Environment variables override YAML configuration file values. This allows environment-specific overrides without modifying configuration files.

---

## Configuration Validation

Abathur validates configuration at startup and reports errors with specific guidance.

### Validation Rules

| Rule | Description | Error Example |
|------|-------------|---------------|
| Range validation | Values within allowed ranges | `max_agents must be between 1 and 100` |
| Required fields | Required fields present | `mcp_servers[0].name is required` |
| Type validation | Correct data types | `max_agents must be an integer` |
| Enum validation | Values from allowed set | `logging.level must be one of: trace, debug, info, warn, error` |
| Path validation | File paths valid | `database.path parent directory not writable` |

**Example Validation Output**:

```
Error: Configuration validation failed

- max_agents: value 150 exceeds maximum allowed value of 100
- logging.level: "verbose" is not valid, must be one of: trace, debug, info, warn, error
- rate_limit.requests_per_second: value -5.0 must be positive
```

!!! info "Validation on Startup"
    Configuration is validated when Abathur starts. Invalid configuration prevents startup and displays validation errors.

---

## Complete Configuration Example

```yaml
# Abathur Configuration
# Complete example with all options

# Maximum concurrent agents (1-100)
max_agents: 20

# Database configuration
database:
  # Path to SQLite database file
  path: ".abathur/abathur.db"

  # Maximum connections in pool
  max_connections: 20

# Logging configuration
logging:
  # Log level: trace, debug, info, warn, error
  level: "info"

  # Output format: json, pretty
  format: "json"

  # Log retention in days
  retention_days: 30

# Rate limiting (token bucket)
rate_limit:
  # Sustained requests per second
  requests_per_second: 10.0

  # Burst capacity
  burst_size: 20

# Retry policy (exponential backoff)
retry:
  # Maximum retry attempts
  max_retries: 3

  # Initial backoff delay (ms)
  initial_backoff_ms: 10000

  # Maximum backoff delay (ms)
  max_backoff_ms: 300000

# MCP server configurations
mcp_servers:
  - name: "memory"
    command: "npx"
    args:
      - "-y"
      - "@modelcontextprotocol/server-memory"
    env: {}

  - name: "github"
    command: "npx"
    args:
      - "-y"
      - "@modelcontextprotocol/server-github"
    env:
      GITHUB_TOKEN: "${GITHUB_TOKEN}"

# LLM Substrate configurations
substrates:
  # Default substrate
  default_substrate: "claude-code"

  # Enabled substrates
  enabled:
    - "claude-code"
    - "anthropic-api"

  # Claude Code substrate
  claude_code:
    claude_path: "claude"
    working_dir: null
    timeout_secs: 300

  # Anthropic API substrate
  anthropic_api:
    enabled: true
    api_key: null  # Use ANTHROPIC_API_KEY env var
    model: "claude-sonnet-4-5-20250929"
    base_url: null

  # Agent type to substrate mappings
  agent_mappings:
    "rust-.*": "anthropic-api"
    "documentation-.*": "claude-code"
    "requirements-gatherer": "anthropic-api"
```

---

## Configuration Profiles

### Development Profile

Optimized for local development with verbose logging and relaxed limits.

```yaml
max_agents: 5
database:
  path: ".abathur/dev.db"
  max_connections: 5
logging:
  level: "debug"
  format: "pretty"
  retention_days: 7
rate_limit:
  requests_per_second: 5.0
  burst_size: 10
retry:
  max_retries: 2
  initial_backoff_ms: 5000
  max_backoff_ms: 60000
substrates:
  default_substrate: "claude-code"
  enabled:
    - "claude-code"
```

### Production Profile

Optimized for production with structured logging and appropriate limits.

```yaml
max_agents: 50
database:
  path: "/var/lib/abathur/production.db"
  max_connections: 50
logging:
  level: "warn"
  format: "json"
  retention_days: 90
rate_limit:
  requests_per_second: 25.0
  burst_size: 50
retry:
  max_retries: 5
  initial_backoff_ms: 10000
  max_backoff_ms: 300000
substrates:
  default_substrate: "anthropic-api"
  enabled:
    - "anthropic-api"
  anthropic_api:
    enabled: true
    model: "claude-sonnet-4-5-20250929"
  agent_mappings:
    ".*": "anthropic-api"
```

### Testing Profile

Optimized for integration testing with minimal retries and fast timeouts.

```yaml
max_agents: 3
database:
  path: ":memory:"  # In-memory database
  max_connections: 3
logging:
  level: "debug"
  format: "json"
  retention_days: 1
rate_limit:
  requests_per_second: 100.0
  burst_size: 100
retry:
  max_retries: 0  # No retries in tests
  initial_backoff_ms: 1000
  max_backoff_ms: 1000
substrates:
  default_substrate: "claude-code"
  enabled:
    - "claude-code"
  claude_code:
    timeout_secs: 60
```

---

## Troubleshooting Configuration

### Configuration Not Loading

**Problem**: Changes to `config.yaml` not taking effect.

**Solutions**:
1. Verify file location: `.abathur/config.yaml` relative to working directory
2. Check file permissions: ensure file is readable
3. Validate YAML syntax: use `yamllint .abathur/config.yaml`
4. Check for environment variable overrides

### Invalid YAML Syntax

**Problem**: Configuration fails to parse.

**Solution**: Use a YAML validator or linter:

```bash
# Install yamllint
pip install yamllint

# Validate configuration
yamllint .abathur/config.yaml
```

### Environment Variables Not Working

**Problem**: Environment variables not overriding configuration.

**Solutions**:
1. Verify naming: use `ABATHUR_` prefix and `__` for nesting
2. Check variable export: ensure variables are exported
3. Verify data types: strings need quotes, numbers don't
4. Check array limitations: arrays must be configured via YAML

### Database Connection Errors

**Problem**: Cannot connect to database.

**Solutions**:
1. Verify path is writable: `touch .abathur/abathur.db`
2. Check parent directory exists: `mkdir -p .abathur`
3. Verify SQLite installation: `sqlite3 --version`
4. Check file permissions

### MCP Server Startup Failures

**Problem**: MCP servers fail to start.

**Solutions**:
1. Verify command is in PATH: `which npx`
2. Check server installation: `npx -y @modelcontextprotocol/server-memory --version`
3. Review environment variables: ensure required vars are set
4. Check logs for specific error messages

---

## See Also

- [CLI Commands Reference](cli-commands.md) - Complete CLI command documentation
- [How-To: Configure Development Environment](../how-to/configuration-guide.md) - Step-by-step configuration guide
- [Explanation: Architecture Overview](../explanation/architecture.md) - System architecture and design
