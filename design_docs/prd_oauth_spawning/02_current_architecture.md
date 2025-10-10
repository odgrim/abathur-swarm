# Current Architecture Analysis - OAuth-Based Agent Spawning

**Date**: 2025-10-09
**Phase**: Phase 1 - Research & Discovery
**Agent**: code-analysis-specialist
**Project**: OAuth-Based Agent Spawning for Abathur

---

## Executive Summary

### Current Architecture Overview

Abathur is a well-architected Python application following **Clean Architecture** principles with clear separation between domain, application, and infrastructure layers. The system uses:

- **Authentication**: Simple API key-based authentication via Anthropic SDK
- **Architecture Pattern**: Clean Architecture with domain/application/infrastructure layers
- **Configuration**: Hierarchical YAML-based config with environment variable overrides
- **Credential Storage**: System keychain (macOS), environment variables, or .env file
- **Database**: SQLite with WAL mode for concurrent access
- **Async Runtime**: Python asyncio with aiosqlite for database operations

### Key Integration Points Identified

1. **ClaudeClient (`application/claude_client.py:18-43`)** - Primary authentication initialization point
2. **ConfigManager (`infrastructure/config.py:162-221`)** - Credential loading and storage
3. **CLI Service Initialization (`cli/main.py:28-71`)** - ClaudeClient instantiation
4. **AgentExecutor (`application/agent_executor.py:18-36`)** - Receives ClaudeClient via dependency injection

### Overall Assessment

**Strengths**:
- Clean separation of concerns with dependency injection
- Well-defined abstraction boundaries
- Comprehensive error handling and logging
- Strong type hints throughout
- Good test coverage patterns

**OAuth Integration Complexity**: **MODERATE**
- Single authentication initialization point (ClaudeClient.__init__)
- Well-isolated credential management (ConfigManager)
- Dependency injection makes testing straightforward
- No tight coupling between authentication and business logic

### Critical Recommendations

1. **Create Authentication Abstraction**: Introduce `AuthProvider` interface to support both API key and OAuth
2. **Extend ConfigManager**: Add OAuth-specific credential management methods
3. **Modify ClaudeClient**: Accept `AuthProvider` instead of raw API key
4. **Update CLI**: Add OAuth configuration commands and initialization logic
5. **Maintain Backward Compatibility**: Detect auth method from key prefix (per DECISION_POINTS.md)

---

## 1. Codebase Structure

### 1.1 Directory Tree

```
/Users/odgrim/dev/home/agentics/abathur/src/abathur/
├── __init__.py                     # Package initialization
├── application/                    # Application layer (use cases)
│   ├── __init__.py
│   ├── agent_executor.py          # Agent spawning and execution
│   ├── agent_pool.py              # Agent lifecycle management
│   ├── claude_client.py           # Claude API client (CRITICAL)
│   ├── failure_recovery.py        # Error recovery and retry
│   ├── loop_executor.py           # Iterative refinement loops
│   ├── mcp_manager.py             # MCP server management
│   ├── resource_monitor.py        # Resource usage monitoring
│   ├── swarm_orchestrator.py     # Multi-agent coordination
│   ├── task_coordinator.py        # Task queue management
│   └── template_manager.py        # Agent template management
├── cli/                           # CLI interface layer
│   ├── __init__.py
│   └── main.py                    # Typer-based CLI (CRITICAL)
├── domain/                        # Domain layer (entities)
│   ├── __init__.py
│   └── models.py                  # Domain models (Task, Agent, Result)
└── infrastructure/                # Infrastructure layer
    ├── __init__.py
    ├── config.py                  # Configuration management (CRITICAL)
    ├── database.py                # SQLite database access
    ├── logger.py                  # Structured logging (structlog)
    └── mcp_config.py              # MCP configuration

Additional key directories:
- tests/unit/                      # Unit tests
- tests/integration/               # Integration tests
- .claude/agents/                  # Agent template definitions (YAML)
- .abathur/                        # Runtime data (DB, logs, config)
```

### 1.2 Module Organization

**Clean Architecture Layers**:

1. **Domain Layer** (`domain/`):
   - Pure business entities (Task, Agent, Result, LoopState)
   - Enums (TaskStatus, AgentState)
   - No external dependencies
   - Framework-agnostic

2. **Application Layer** (`application/`):
   - Use cases and orchestration logic
   - ClaudeClient (external API integration)
   - Agent lifecycle management
   - Task coordination
   - Depends on: domain models, infrastructure abstractions

3. **Infrastructure Layer** (`infrastructure/`):
   - Configuration loading
   - Database persistence
   - Logging infrastructure
   - External system integrations
   - Implements interfaces required by application layer

4. **Interface Layer** (`cli/`):
   - User-facing CLI commands
   - Service initialization and wiring
   - Adapts user input to application layer calls

**Key Observations**:
- Dependency direction follows Clean Architecture: domain ← application ← infrastructure
- No circular dependencies detected
- Strong type hints enable static analysis
- Dependency injection used throughout

---

## 2. Current Authentication Architecture

### 2.1 ClaudeClient Deep Dive

**File**: `/Users/odgrim/dev/home/agentics/abathur/src/abathur/application/claude_client.py`

#### Authentication Flow

```
┌─────────────────────────────────────────────────────────────────┐
│ ClaudeClient.__init__()                                         │
│ Lines 18-43                                                     │
├─────────────────────────────────────────────────────────────────┤
│ 1. Accept api_key parameter (optional)                         │
│ 2. If no api_key provided, read ANTHROPIC_API_KEY from env     │
│ 3. Raise ValueError if no API key found                        │
│ 4. Initialize Anthropic SDK clients (sync + async)             │
│    - Anthropic(api_key=self.api_key, max_retries=max_retries) │
│    - AsyncAnthropic(api_key=...)                               │
└─────────────────────────────────────────────────────────────────┘
```

#### Code Analysis

**Initialization (Lines 18-43)**:
```python
class ClaudeClient:
    """Wrapper for Anthropic Claude API with retry logic and rate limiting."""

    def __init__(
        self,
        api_key: str | None = None,  # Line 20: Optional API key
        model: str = "claude-sonnet-4-20250514",
        max_retries: int = 3,
        timeout: int = 300,
    ):
        # Line 33: Fallback to environment variable
        self.api_key = api_key or os.getenv("ANTHROPIC_API_KEY")

        # Line 34-35: Validation
        if not self.api_key:
            raise ValueError("ANTHROPIC_API_KEY must be provided or set in environment")

        self.model = model
        self.max_retries = max_retries
        self.timeout = timeout

        # Line 42-43: SDK client initialization
        self.client = Anthropic(api_key=self.api_key, max_retries=max_retries)
        self.async_client = AsyncAnthropic(api_key=self.api_key, max_retries=max_retries)
```

**Key Methods**:

1. **execute_task()** (Lines 45-117):
   - Async task execution
   - Uses `self.async_client.messages.create()`
   - Authentication handled transparently by SDK
   - Returns structured result dict

2. **stream_task()** (Lines 119-157):
   - Async streaming execution
   - Uses `self.async_client.messages.stream()`
   - Yields text chunks as they arrive

3. **validate_api_key()** (Lines 159-175):
   - Makes test request to validate credentials
   - Uses smallest model (Haiku) for minimal cost
   - Returns bool (True if valid)

4. **batch_execute()** (Lines 177-209):
   - Concurrent task execution with semaphore
   - Rate limiting via asyncio.Semaphore

**Authentication Characteristics**:
- **Single initialization point**: All auth happens in `__init__()`
- **SDK-managed**: Anthropic SDK handles header formatting (`x-api-key`)
- **No refresh logic**: API keys don't expire (unlike OAuth tokens)
- **No token storage**: Key passed directly to SDK
- **Error handling**: Generic exception catching, no auth-specific errors

**OAuth Integration Points**:
```
MODIFICATION NEEDED: Lines 18-43
├── Accept AuthProvider abstraction instead of raw api_key
├── Use AuthProvider to get credentials for SDK
├── Handle OAuth token refresh on auth errors
└── Log authentication method being used
```

### 2.2 Configuration Management Analysis

**File**: `/Users/odgrim/dev/home/agentics/abathur/src/abathur/infrastructure/config.py`

#### Configuration Loading Flow

```
┌─────────────────────────────────────────────────────────────────┐
│ ConfigManager.load_config()                                     │
│ Lines 79-118                                                    │
├─────────────────────────────────────────────────────────────────┤
│ Hierarchy (lowest to highest priority):                        │
│ 1. System defaults (embedded in Config model)                  │
│ 2. Template config: .abathur/config.yaml                       │
│ 3. User config: ~/.abathur/config.yaml                         │
│ 4. Project config: .abathur/local.yaml                         │
│ 5. Environment variables: ABATHUR_* prefix                     │
└─────────────────────────────────────────────────────────────────┘
```

#### API Key Management

**get_api_key() Method (Lines 162-202)**:
```python
def get_api_key(self) -> str:
    """Get Anthropic API key from environment, keychain, or .env file.

    Priority:
    1. ANTHROPIC_API_KEY environment variable
    2. System keychain
    3. .env file

    Returns:
        API key

    Raises:
        ValueError: If API key not found
    """
    # 1. Environment variable (Line 177)
    if key := os.getenv("ANTHROPIC_API_KEY"):
        return key

    # 2. System keychain (Lines 181-186)
    try:
        key = keyring.get_password("abathur", "anthropic_api_key")
        if key:
            return key
    except Exception:
        pass

    # 3. .env file (Lines 189-195)
    env_file = self.project_root / ".env"
    if env_file.exists():
        with open(env_file) as f:
            for line in f:
                line = line.strip()
                if line.startswith("ANTHROPIC_API_KEY="):
                    return line.split("=", 1)[1].strip().strip('"').strip("'")

    # Raise error if not found (Lines 197-202)
    raise ValueError(
        "ANTHROPIC_API_KEY not found. Set it via:\n"
        "  1. Environment variable: export ANTHROPIC_API_KEY=your-key\n"
        "  2. Keychain: abathur config set-key\n"
        "  3. .env file: echo 'ANTHROPIC_API_KEY=your-key' > .env"
    )
```

**set_api_key() Method (Lines 204-221)**:
```python
def set_api_key(self, api_key: str, use_keychain: bool = True) -> None:
    """Store API key in keychain or .env file."""
    if use_keychain:
        try:
            keyring.set_password("abathur", "anthropic_api_key", api_key)
            return
        except Exception as e:
            raise ValueError(f"Failed to store API key in keychain: {e}") from e
    else:
        # Store in .env file
        env_file = self.project_root / ".env"
        with open(env_file, "a") as f:
            f.write(f"\nANTHROPIC_API_KEY={api_key}\n")
```

**Configuration Model (Lines 55-65)**:
```python
class Config(BaseModel):
    """Main configuration model."""

    version: str = "0.1.0"
    log_level: str = "INFO"
    queue: QueueConfig = Field(default_factory=QueueConfig)
    swarm: SwarmConfig = Field(default_factory=SwarmConfig)
    loop: LoopConfig = Field(default_factory=LoopConfig)
    resources: ResourceConfig = Field(default_factory=ResourceConfig)
    monitoring: MonitoringConfig = Field(default_factory=MonitoringConfig)
```

**OAuth Integration Points**:
```
NEW METHODS NEEDED:
├── get_oauth_token() - Retrieve OAuth token from storage
├── set_oauth_token() - Store OAuth token securely
├── get_oauth_refresh_token() - Retrieve refresh token
├── set_oauth_refresh_token() - Store refresh token
├── detect_auth_method() - Auto-detect from key prefix
└── get_auth_credentials() - Unified credential retrieval

NEW CONFIG FIELDS:
├── auth_mode: Literal["api_key", "oauth"] (auto-detected)
├── oauth_token_expiry: datetime | None
└── oauth_last_refresh: datetime | None
```

### 2.3 Agent Spawning Workflow

**File**: `/Users/odgrim/dev/home/agentics/abathur/src/abathur/application/agent_executor.py`

#### Agent Lifecycle with Authentication

```
┌──────────────────────────────────────────────────────────────────┐
│ AgentExecutor.execute_task()                                     │
│ Lines 38-150                                                     │
├──────────────────────────────────────────────────────────────────┤
│ 1. Load agent definition from YAML (Line 51)                    │
│ 2. Create Agent record (Lines 54-61)                            │
│ 3. Insert agent into database (Line 63)                         │
│ 4. Build system prompt and user message (Lines 76-82)           │
│ 5. Execute with ClaudeClient (Lines 87-93)                      │
│    └─> ClaudeClient.execute_task() uses authenticated client    │
│ 6. Create Result object (Lines 96-104)                          │
│ 7. Update agent state to terminated (Lines 107-108)             │
│ 8. Log audit trail (Lines 111-120)                              │
└──────────────────────────────────────────────────────────────────┘
```

#### Code Analysis

**Initialization (Lines 21-36)**:
```python
class AgentExecutor:
    """Executes tasks using Claude agents."""

    def __init__(
        self,
        database: Database,
        claude_client: ClaudeClient,  # Line 24: Receives pre-initialized client
        agents_dir: Path | None = None,
    ):
        """Initialize agent executor.

        Args:
            database: Database for state persistence
            claude_client: Claude API client
            agents_dir: Directory containing agent definitions (default: .claude/agents)
        """
        self.database = database
        self.claude_client = claude_client  # Line 35: Store injected client
        self.agents_dir = agents_dir or (Path.cwd() / ".claude" / "agents")
```

**Task Execution (Lines 87-93)**:
```python
# Execute with Claude
logger.info("executing_task", task_id=str(task.id), agent_id=str(agent_id))

response = await self.claude_client.execute_task(
    system_prompt=system_prompt,
    user_message=user_message,
    max_tokens=agent_def.get("resource_limits", {}).get("max_tokens", 8000),
    temperature=agent_def.get("resource_limits", {}).get("temperature", 0.7),
    model=agent.model,
)
```

**Key Observations**:
- **Dependency Injection**: ClaudeClient passed to constructor
- **No Direct Auth**: AgentExecutor doesn't handle authentication
- **Single Client Instance**: All tasks use same authenticated client
- **Error Propagation**: Auth errors bubble up from ClaudeClient

**OAuth Impact**:
```
NO CHANGES NEEDED in AgentExecutor
├── Already uses dependency injection
├── Receives ClaudeClient from caller
└── Agnostic to authentication method

SWARM ORCHESTRATOR (swarm_orchestrator.py:18-36):
├── Also receives ClaudeClient via AgentExecutor
└── No direct authentication handling
```

### 2.4 CLI Service Initialization

**File**: `/Users/odgrim/dev/home/agentics/abathur/src/abathur/cli/main.py`

#### Service Wiring with Authentication

```
┌──────────────────────────────────────────────────────────────────┐
│ _get_services() - Lines 28-71                                    │
├──────────────────────────────────────────────────────────────────┤
│ 1. Initialize ConfigManager (Line 43)                           │
│ 2. Initialize Database (Lines 44-45)                            │
│ 3. Get API key from ConfigManager (Line 48)                     │
│ 4. Initialize ClaudeClient with API key (Line 48)               │
│    └─> ClaudeClient(api_key=config_manager.get_api_key())      │
│ 5. Initialize AgentExecutor with ClaudeClient (Line 49)         │
│ 6. Wire all other services (Lines 50-58)                        │
│ 7. Return service dictionary (Lines 60-70)                      │
└──────────────────────────────────────────────────────────────────┘
```

#### Code Analysis

**Service Initialization (Lines 28-71)**:
```python
async def _get_services() -> dict[str, Any]:
    """Get initialized services."""
    from abathur.application import (
        AgentExecutor,
        ClaudeClient,
        FailureRecovery,
        LoopExecutor,
        MCPManager,
        ResourceMonitor,
        SwarmOrchestrator,
        TaskCoordinator,
        TemplateManager,
    )
    from abathur.infrastructure import ConfigManager, Database

    # Line 43: Initialize config manager
    config_manager = ConfigManager()
    database = Database(config_manager.get_database_path())
    await database.initialize()

    # Lines 47-49: CRITICAL AUTHENTICATION INITIALIZATION
    task_coordinator = TaskCoordinator(database)
    claude_client = ClaudeClient(api_key=config_manager.get_api_key())
    agent_executor = AgentExecutor(database, claude_client)

    # Lines 50-58: Wire remaining services
    swarm_orchestrator = SwarmOrchestrator(
        task_coordinator, agent_executor, max_concurrent_agents=10
    )
    template_manager = TemplateManager()
    mcp_manager = MCPManager()
    await mcp_manager.initialize()
    failure_recovery = FailureRecovery(task_coordinator, database)
    resource_monitor = ResourceMonitor()
    loop_executor = LoopExecutor(task_coordinator, agent_executor, database)

    return {
        "database": database,
        "task_coordinator": task_coordinator,
        "claude_client": claude_client,
        "agent_executor": agent_executor,
        "swarm_orchestrator": swarm_orchestrator,
        "template_manager": template_manager,
        "mcp_manager": mcp_manager,
        "failure_recovery": failure_recovery,
        "resource_monitor": resource_monitor,
        "loop_executor": loop_executor,
    }
```

**Config Commands (Lines 570-586)**:
```python
@config_app.command("set-key")
def config_set_key(
    api_key: str = typer.Argument(..., help="Anthropic API key"),
    use_keychain: bool = typer.Option(True, help="Store in system keychain"),
) -> None:
    """Set Anthropic API key."""
    try:
        from abathur.infrastructure.config import ConfigManager

        config_manager = ConfigManager()
        config_manager.set_api_key(api_key, use_keychain=use_keychain)
        storage = "keychain" if use_keychain else ".env file"
        console.print(f"[green]✓[/green] API key stored in {storage}")
    except Exception as e:
        console.print(f"[red]✗[/red] Failed to store API key: {e}")
        raise typer.Exit(1) from e
```

**OAuth Integration Points**:
```
MODIFICATIONS NEEDED: Lines 28-71
├── Detect auth method (API key vs OAuth)
├── Initialize appropriate AuthProvider
├── Pass AuthProvider to ClaudeClient factory
└── Handle OAuth initialization errors

NEW CLI COMMANDS NEEDED:
├── config oauth-login - Interactive OAuth flow
├── config oauth-logout - Clear OAuth tokens
├── config oauth-status - Show OAuth token status
└── config oauth-refresh - Manually refresh token
```

### 2.5 Complete Authentication Flow Diagram

```
┌─────────────────────────────────────────────────────────────────────────┐
│                        CURRENT AUTHENTICATION FLOW                       │
└─────────────────────────────────────────────────────────────────────────┘

User invokes CLI
     │
     ├─> cli/main.py:_get_services()
     │        │
     │        ├─> ConfigManager()
     │        │        │
     │        │        ├─> get_api_key()
     │        │        │        │
     │        │        │        ├─> Check ANTHROPIC_API_KEY env var
     │        │        │        ├─> Check system keychain
     │        │        │        └─> Check .env file
     │        │        │
     │        │        └─> Return API key string
     │        │
     │        ├─> ClaudeClient(api_key=...)
     │        │        │
     │        │        ├─> Store api_key as instance variable
     │        │        ├─> Initialize Anthropic SDK client
     │        │        └─> Initialize AsyncAnthropic SDK client
     │        │
     │        └─> AgentExecutor(database, claude_client)
     │                 │
     │                 └─> Store claude_client reference
     │
     └─> CLI command executes
              │
              └─> AgentExecutor.execute_task()
                       │
                       └─> ClaudeClient.execute_task()
                                │
                                └─> async_client.messages.create()
                                         │
                                         └─> Anthropic SDK adds x-api-key header
                                                  │
                                                  └─> HTTP request to Claude API
```

---

## 3. Integration Point Catalog

### 3.1 Authentication Initialization Points

| **Location** | **File:Line** | **Current Behavior** | **OAuth Modification** |
|--------------|---------------|----------------------|------------------------|
| **ClaudeClient.__init__()** | `application/claude_client.py:18-43` | Accepts optional `api_key` string, falls back to `ANTHROPIC_API_KEY` env var | Accept `AuthProvider` abstraction, use provider to get credentials |
| **ConfigManager.get_api_key()** | `infrastructure/config.py:162-202` | Returns API key from env/keychain/.env | Add `get_auth_credentials()` to return either API key or OAuth token |
| **CLI _get_services()** | `cli/main.py:48` | `ClaudeClient(api_key=config_manager.get_api_key())` | Initialize `AuthProvider` based on detected method, pass to `ClaudeClient` |
| **Anthropic SDK initialization** | `application/claude_client.py:42-43` | `Anthropic(api_key=...)` | Update SDK to use OAuth bearer token if available |

### 3.2 Configuration Touchpoints

| **Touchpoint** | **File:Line** | **Current Implementation** | **OAuth Changes Needed** |
|----------------|---------------|----------------------------|--------------------------|
| **Config file format** | `infrastructure/config.py:55-65` | No auth-specific config (relies on env vars) | Add `auth_mode`, `oauth_token_storage_path` |
| **Environment variables** | `infrastructure/config.py:177, 194` | `ANTHROPIC_API_KEY` | Add `ANTHROPIC_OAUTH_TOKEN`, `ANTHROPIC_OAUTH_REFRESH_TOKEN` |
| **Keychain storage** | `infrastructure/config.py:182-186` | Store/retrieve `anthropic_api_key` | Add `anthropic_oauth_token`, `anthropic_oauth_refresh_token` |
| **CLI config commands** | `cli/main.py:570-586` | `config set-key` | Add `config oauth-login`, `config oauth-logout`, `config oauth-status` |
| **.env file parsing** | `infrastructure/config.py:189-195` | Parse `ANTHROPIC_API_KEY=...` | Parse OAuth token variables |

### 3.3 Agent Creation & Spawning Logic

| **Component** | **File:Line** | **Auth Dependency** | **OAuth Impact** |
|---------------|---------------|---------------------|------------------|
| **AgentExecutor** | `application/agent_executor.py:21-36` | Receives `ClaudeClient` via DI | **NO CHANGES** - Already decoupled |
| **SwarmOrchestrator** | `application/swarm_orchestrator.py:18-36` | Receives `AgentExecutor` via DI | **NO CHANGES** - No direct auth handling |
| **AgentPool** | `application/agent_pool.py:28-55` | No auth dependency | **NO CHANGES** - Manages agent lifecycle only |
| **TaskCoordinator** | `application/task_coordinator.py:12-21` | No auth dependency | **NO CHANGES** - Queue management only |

**Key Finding**: Clean dependency injection means **zero changes** needed in core orchestration logic.

### 3.4 Error Handling Paths

| **Error Type** | **Current Handling** | **OAuth-Specific Handling Needed** |
|----------------|----------------------|-------------------------------------|
| **API Key Missing** | `ValueError` in `ClaudeClient.__init__` (line 35) | Extend to check OAuth token if no API key |
| **API Key Invalid** | Generic exception in `validate_api_key()` (line 174) | Add OAuth-specific validation endpoint |
| **Request Failure** | Exception caught in `execute_task()` (line 109) | Add token refresh logic on 401 Unauthorized |
| **Rate Limiting** | SDK handles retries (max_retries=3) | Distinguish between rate limits and auth failures |

**New Error Handling Locations**:
```
ClaudeClient.execute_task() - Lines 72-117
├── Catch 401 Unauthorized
├── Trigger AuthProvider.refresh_token()
├── Retry request with new token
└── Fail after 3 refresh attempts
```

### 3.5 Logging & Monitoring Hooks

| **Log Location** | **File:Line** | **Current Logging** | **OAuth Additions** |
|------------------|---------------|---------------------|---------------------|
| **Client initialization** | `claude_client.py:42-43` | No logging | Log auth method (API key / OAuth) |
| **Task execution start** | `claude_client.py:73` | Log model name | Log auth method used |
| **Task completion** | `claude_client.py:101-105` | Log tokens, stop_reason | Add auth method to metadata |
| **API key validation** | `claude_client.py:159-175` | Log validation result | Add OAuth token validation logging |
| **Error logging** | `claude_client.py:110, 156, 174` | Generic error logs | Add OAuth-specific error context |

**New Logging Requirements**:
```python
# Authentication method logging
logger.info("claude_client_initialized", auth_method="oauth", token_expiry=...)

# Token refresh logging
logger.info("oauth_token_refreshed", previous_expiry=..., new_expiry=...)

# OAuth-specific errors
logger.error("oauth_token_refresh_failed", attempt=1, error=...)
```

---

## 4. Dependency Analysis

### 4.1 Anthropic SDK Usage

**Current Version**: `anthropic = "^0.18.0"` (pyproject.toml:15)

**SDK Features Used**:

1. **Synchronous Client** (claude_client.py:42):
   ```python
   self.client = Anthropic(api_key=self.api_key, max_retries=max_retries)
   ```
   - Used in: `validate_api_key()` method
   - Purpose: Blocking API key validation

2. **Asynchronous Client** (claude_client.py:43):
   ```python
   self.async_client = AsyncAnthropic(api_key=self.api_key, max_retries=max_retries)
   ```
   - Used in: `execute_task()`, `stream_task()`, `batch_execute()`
   - Purpose: Non-blocking API requests

3. **Messages API** (claude_client.py:75-82):
   ```python
   response = await self.async_client.messages.create(
       model=model_to_use,
       max_tokens=max_tokens,
       temperature=temperature,
       system=system_prompt,
       messages=[{"role": "user", "content": user_message}],
       timeout=self.timeout,
   )
   ```

4. **Streaming API** (claude_client.py:144-153):
   ```python
   async with self.async_client.messages.stream(...) as stream:
       async for text in stream.text_stream:
           yield text
   ```

**SDK Configuration Options Used**:
- `api_key`: Authentication (x-api-key header)
- `max_retries`: Retry logic (default: 3)
- `timeout`: Request timeout (default: 300s)

**OAuth Support Research Needed**:
```
UNKNOWN: Does Anthropic SDK (^0.18.0) support OAuth bearer tokens?
├── Check: Does SDK accept bearer_token parameter?
├── Check: Can we override auth header mechanism?
└── Fallback: Direct HTTP requests with httpx if SDK doesn't support OAuth
```

### 4.2 External Dependencies

| **Library** | **Version** | **Purpose** | **OAuth Impact** |
|-------------|-------------|-------------|------------------|
| **anthropic** | ^0.18.0 | Claude API SDK | **CRITICAL** - May need OAuth support verification |
| **typer** | ^0.12.0 | CLI framework | **LOW** - Add new OAuth commands |
| **pydantic** | ^2.5.0 | Data validation | **LOW** - Add OAuth credential models |
| **keyring** | ^24.3.0 | Secure credential storage | **MEDIUM** - Store OAuth tokens |
| **structlog** | ^24.1.0 | Structured logging | **LOW** - Log OAuth events |
| **aiosqlite** | ^0.19.0 | Async SQLite | **NONE** - No auth-related changes |
| **psutil** | ^5.9.0 | Resource monitoring | **NONE** |
| **pyyaml** | ^6.0.1 | Config parsing | **LOW** - Parse OAuth config |
| **python-dotenv** | ^1.0.0 | .env file loading | **LOW** - Load OAuth env vars |
| **rich** | ^13.7.0 | CLI formatting | **LOW** - Format OAuth status output |

**New Dependencies Potentially Needed**:
- `httpx` (if Anthropic SDK doesn't support OAuth): HTTP client with OAuth support
- None if SDK supports OAuth tokens

### 4.3 Internal Module Dependencies

**Import Graph**:
```
domain/models.py (no dependencies)
     ↑
infrastructure/config.py ← infrastructure/logger.py
     ↑                          ↑
infrastructure/database.py ←────┤
     ↑                          ↑
application/claude_client.py ───┤
     ↑                          ↑
application/task_coordinator.py ┤
     ↑                          ↑
application/agent_executor.py ──┤
     ↑                          ↑
application/swarm_orchestrator.py
     ↑
cli/main.py
```

**Dependency Characteristics**:
- **No circular dependencies**: Clean dependency graph
- **Clear layering**: infrastructure ← application ← cli
- **Dependency injection**: All major components use DI

**OAuth Impact on Module Dependencies**:
```
NEW MODULE: application/auth_provider.py
├── Imported by: application/claude_client.py
├── Imports: infrastructure/config.py, infrastructure/logger.py
└── Interface: AuthProvider (abstract base class)
    ├── APIKeyAuthProvider (concrete implementation)
    └── OAuthAuthProvider (concrete implementation)
```

### 4.4 Configuration File Dependencies

**Configuration Hierarchy** (config.py:79-118):

1. **System Defaults** (embedded in Pydantic models)
2. **Template Config**: `.abathur/config.yaml`
3. **User Config**: `~/.abathur/config.yaml`
4. **Project Config**: `.abathur/local.yaml`
5. **Environment Variables**: `ABATHUR_*`

**Current Config Schema**:
```yaml
version: "0.1.0"
log_level: "INFO"
queue:
  max_size: 1000
  default_priority: 5
  retry_attempts: 3
swarm:
  max_concurrent_agents: 10
  agent_spawn_timeout: "5s"
loop:
  max_iterations: 10
  default_timeout: "1h"
resources:
  max_memory_per_agent: "512MB"
monitoring:
  metrics_enabled: true
```

**OAuth Config Extensions Needed**:
```yaml
# NEW SECTION
auth:
  mode: "auto"  # auto | api_key | oauth
  oauth_token_storage: "keychain"  # keychain | env | file
  oauth_auto_refresh: true
  oauth_refresh_retries: 3
```

**Required vs Optional Fields**:
- **Required**: None (all have defaults)
- **Optional**: All configuration values
- **OAuth Fields**: All optional with sensible defaults

---

## 5. Architectural Patterns

### 5.1 Clean Architecture Adherence

**Layer Separation Analysis**:

| **Layer** | **Directory** | **Dependencies** | **Clean Architecture Compliance** |
|-----------|---------------|------------------|-----------------------------------|
| **Domain** | `domain/` | None (pure entities) | **EXCELLENT** - No framework dependencies |
| **Application** | `application/` | domain, infrastructure abstractions | **GOOD** - Some infrastructure coupling in ClaudeClient |
| **Infrastructure** | `infrastructure/` | domain, external libs | **EXCELLENT** - Clear boundary |
| **Interface** | `cli/` | All layers | **GOOD** - Service initialization logic |

**Dependency Rule Compliance**:
```
✓ Domain has no dependencies
✓ Application depends on domain only
✓ Infrastructure depends on domain and implements interfaces
✓ CLI depends on all layers (composition root)
```

**ClaudeClient Placement Analysis**:
- **Current**: `application/claude_client.py`
- **Issue**: Directly imports Anthropic SDK (infrastructure concern)
- **Better**: Move to `infrastructure/` and create application-layer interface

**Recommendation**:
```
REFACTORING OPPORTUNITY (pre-OAuth):
├── Create: domain/ports/llm_client.py (interface)
├── Move: ClaudeClient to infrastructure/anthropic_client.py
└── Update: application layer to depend on LLMClient interface
```

### 5.2 Interface Abstractions

**Current Abstractions**:

1. **Database** (`infrastructure/database.py`):
   - No explicit interface, but could be swapped for PostgreSQL
   - Methods are well-defined and testable

2. **Logger** (`infrastructure/logger.py`):
   - `get_logger()` function returns structlog logger
   - Could be swapped for different logging backend

3. **Config** (`infrastructure/config.py`):
   - Pydantic models provide schema validation
   - No explicit interface

**Missing Abstractions**:
```
NO AUTH ABSTRACTION EXISTS
├── ClaudeClient directly handles authentication
├── No interface for credential providers
└── Hard to test different auth methods
```

**Recommended Auth Abstraction**:
```python
# domain/ports/auth_provider.py
from abc import ABC, abstractmethod

class AuthProvider(ABC):
    """Abstract authentication provider."""

    @abstractmethod
    async def get_credentials(self) -> dict[str, str]:
        """Get credentials for API requests.

        Returns:
            Dict with 'type' (api_key | bearer) and 'value'
        """
        pass

    @abstractmethod
    async def refresh_credentials(self) -> bool:
        """Refresh expired credentials.

        Returns:
            True if refresh successful
        """
        pass

    @abstractmethod
    def is_valid(self) -> bool:
        """Check if current credentials are valid."""
        pass


# infrastructure/api_key_auth.py
class APIKeyAuthProvider(AuthProvider):
    """API key authentication provider."""

    async def get_credentials(self) -> dict[str, str]:
        return {"type": "api_key", "value": self.api_key}

    async def refresh_credentials(self) -> bool:
        return True  # API keys don't expire

    def is_valid(self) -> bool:
        return self.api_key is not None


# infrastructure/oauth_auth.py
class OAuthAuthProvider(AuthProvider):
    """OAuth authentication provider with token refresh."""

    async def get_credentials(self) -> dict[str, str]:
        if self._is_token_expired():
            await self.refresh_credentials()
        return {"type": "bearer", "value": self.access_token}

    async def refresh_credentials(self) -> bool:
        # Implement OAuth token refresh logic
        ...
```

### 5.3 Error Handling Patterns

**Current Error Strategy**:

1. **Exception Propagation** (claude_client.py:109-117):
   ```python
   except Exception as e:
       logger.error("claude_task_failed", error=str(e))
       return {
           "success": False,
           "content": "",
           "stop_reason": "error",
           "usage": {"input_tokens": 0, "output_tokens": 0},
           "error": str(e),
       }
   ```
   - Catch all exceptions
   - Log error
   - Return structured error response
   - Don't re-raise (graceful degradation)

2. **Retry Logic** (claude_client.py:22, 42):
   ```python
   max_retries: int = 3
   self.client = Anthropic(api_key=self.api_key, max_retries=max_retries)
   ```
   - SDK handles retries transparently
   - No custom retry logic

3. **Validation Errors** (config.py:197-202):
   ```python
   raise ValueError(
       "ANTHROPIC_API_KEY not found. Set it via:\n"
       "  1. Environment variable: export ANTHROPIC_API_KEY=your-key\n"
       "  2. Keychain: abathur config set-key\n"
       "  3. .env file: echo 'ANTHROPIC_API_KEY=your-key' > .env"
   )
   ```
   - Explicit error messages with remediation steps

**Custom Exception Hierarchy**:
```python
# Currently: NO custom exception hierarchy
# All errors use built-in exceptions (ValueError, RuntimeError, Exception)
```

**OAuth Error Handling Needs**:
```python
# NEW: infrastructure/exceptions.py

class AbathurError(Exception):
    """Base exception for Abathur."""
    pass

class AuthenticationError(AbathurError):
    """Authentication failed."""
    pass

class OAuthTokenExpiredError(AuthenticationError):
    """OAuth token has expired and refresh failed."""
    pass

class OAuthRefreshError(AuthenticationError):
    """Failed to refresh OAuth token."""
    pass

class APIKeyInvalidError(AuthenticationError):
    """API key is invalid."""
    pass
```

### 5.4 Testing Patterns

**Test Organization**:
- `tests/unit/` - Unit tests for individual components
- `tests/integration/` - Integration tests for component interactions

**Sample Test Pattern** (test_config.py:168-181):
```python
def test_get_api_key_from_env(self) -> None:
    """Test getting API key from environment variable."""
    with TemporaryDirectory() as tmpdir:
        project_root = Path(tmpdir)
        config_manager = ConfigManager(project_root=project_root)

        # Set environment variable
        os.environ["ANTHROPIC_API_KEY"] = "test-key-123"

        try:
            api_key = config_manager.get_api_key()
            assert api_key == "test-key-123"
        finally:
            del os.environ["ANTHROPIC_API_KEY"]
```

**Testing Characteristics**:
- **Fixtures**: Use TemporaryDirectory for isolation
- **Cleanup**: Always cleanup environment variables
- **Mocking**: Likely uses mocks for Anthropic SDK (need to verify)
- **Async Tests**: pytest-asyncio for async code

**OAuth Testing Requirements**:
```python
# tests/unit/test_oauth_auth.py

@pytest.fixture
def mock_oauth_server():
    """Mock OAuth server for testing."""
    # Return mock server that simulates OAuth flow
    pass

async def test_oauth_token_refresh():
    """Test OAuth token refresh logic."""
    provider = OAuthAuthProvider(...)

    # Expire token
    provider._access_token_expiry = datetime.now() - timedelta(hours=1)

    # Should trigger refresh
    credentials = await provider.get_credentials()

    assert credentials["type"] == "bearer"
    assert credentials["value"] != old_token
```

---

## 6. Impact Assessment

### 6.1 Components Requiring Modification

| **Component** | **File** | **Scope** | **Breaking?** | **Test Impact** | **Risk** |
|---------------|----------|-----------|---------------|-----------------|----------|
| **ClaudeClient** | `application/claude_client.py` | **MAJOR** | No (add constructor overload) | **HIGH** - Need OAuth mocks | **MEDIUM** |
| **ConfigManager** | `infrastructure/config.py` | **MODERATE** | No (add new methods) | **MEDIUM** - New credential tests | **LOW** |
| **Config Model** | `infrastructure/config.py:55-65` | **MINOR** | No (add optional fields) | **LOW** - Schema validation tests | **LOW** |
| **CLI Main** | `cli/main.py:28-71` | **MODERATE** | No (detect auth method) | **MEDIUM** - Service wiring tests | **LOW** |
| **CLI Config Commands** | `cli/main.py:570-586` | **MINOR** | No (add new commands) | **LOW** - Command tests | **LOW** |
| **Logger** | `infrastructure/logger.py` | **NONE** | No | **NONE** | **NONE** |
| **AgentExecutor** | `application/agent_executor.py` | **NONE** | No | **NONE** | **NONE** |
| **SwarmOrchestrator** | `application/swarm_orchestrator.py` | **NONE** | No | **NONE** | **NONE** |
| **Database** | `infrastructure/database.py` | **NONE** | No | **NONE** | **NONE** |
| **Domain Models** | `domain/models.py` | **NONE** | No | **NONE** | **NONE** |

**Modification Details**:

#### ClaudeClient (MAJOR)
**Changes**:
- Accept `AuthProvider` in constructor (alternative to `api_key`)
- Add token refresh logic in `execute_task()` and `stream_task()`
- Update error handling for OAuth-specific errors
- Add authentication method logging

**Code Diff**:
```python
# BEFORE (lines 18-24)
def __init__(
    self,
    api_key: str | None = None,
    model: str = "claude-sonnet-4-20250514",
    max_retries: int = 3,
    timeout: int = 300,
):

# AFTER
def __init__(
    self,
    api_key: str | None = None,
    auth_provider: AuthProvider | None = None,  # NEW
    model: str = "claude-sonnet-4-20250514",
    max_retries: int = 3,
    timeout: int = 300,
):
    if auth_provider:
        self.auth_provider = auth_provider
    else:
        # Backward compatibility: create API key provider
        self.auth_provider = APIKeyAuthProvider(api_key or os.getenv("ANTHROPIC_API_KEY"))
```

#### ConfigManager (MODERATE)
**Changes**:
- Add `get_oauth_token()`, `set_oauth_token()` methods
- Add `detect_auth_method()` to auto-detect from key prefix
- Add `get_auth_credentials()` unified retrieval method

**New Methods**:
```python
def detect_auth_method(self, key: str) -> Literal["api_key", "oauth"]:
    """Detect auth method from key prefix.

    Per DECISION_POINTS.md:
    - API keys start with: sk-ant-api03-...
    - OAuth tokens start with: different prefix (TBD from research)
    """
    if key.startswith("sk-ant-api"):
        return "api_key"
    elif key.startswith("oauth-"):  # Placeholder
        return "oauth"
    else:
        raise ValueError(f"Unrecognized key format: {key[:10]}...")

def get_oauth_token(self) -> str:
    """Get OAuth access token from storage."""
    # Priority: env var > keychain > .env file
    ...

def set_oauth_token(self, token: str, refresh_token: str, use_keychain: bool = True) -> None:
    """Store OAuth tokens securely."""
    ...
```

### 6.2 New Components to Create

| **Component** | **File Path** | **Purpose** | **Dependencies** | **Priority** |
|---------------|---------------|-------------|------------------|--------------|
| **AuthProvider Interface** | `domain/ports/auth_provider.py` | Abstract authentication provider | None | **HIGH** |
| **APIKeyAuthProvider** | `infrastructure/api_key_auth.py` | API key authentication | AuthProvider, ConfigManager | **HIGH** |
| **OAuthAuthProvider** | `infrastructure/oauth_auth.py` | OAuth with token refresh | AuthProvider, ConfigManager, httpx? | **HIGH** |
| **OAuth CLI Commands** | `cli/main.py` (extend) | `oauth-login`, `oauth-logout`, etc. | ConfigManager, OAuthAuthProvider | **MEDIUM** |
| **Auth Exceptions** | `infrastructure/exceptions.py` | Custom exception hierarchy | None | **MEDIUM** |
| **OAuth Config Models** | `infrastructure/config.py` (extend) | OAuth-specific config fields | Pydantic | **LOW** |

**New File: domain/ports/auth_provider.py**
```python
"""Authentication provider interface."""

from abc import ABC, abstractmethod
from typing import Literal


class AuthProvider(ABC):
    """Abstract authentication provider for Claude API."""

    @abstractmethod
    async def get_credentials(self) -> dict[str, str]:
        """Get credentials for API requests.

        Returns:
            Dict with:
            - 'type': 'api_key' | 'bearer'
            - 'value': credential value
            - 'expires_at': expiry timestamp (for OAuth)
        """
        pass

    @abstractmethod
    async def refresh_credentials(self) -> bool:
        """Refresh expired credentials.

        Returns:
            True if refresh successful, False otherwise
        """
        pass

    @abstractmethod
    def is_valid(self) -> bool:
        """Check if current credentials are valid and not expired."""
        pass

    @abstractmethod
    def get_auth_method(self) -> Literal["api_key", "oauth"]:
        """Get authentication method type."""
        pass
```

### 6.3 Testing Requirements

**New Unit Tests**:
```
tests/unit/test_auth_provider.py
├── test_api_key_provider_get_credentials()
├── test_api_key_provider_no_refresh()
├── test_oauth_provider_get_credentials()
├── test_oauth_provider_refresh_token()
├── test_oauth_provider_token_expiry()
└── test_oauth_provider_refresh_failure()

tests/unit/test_config_oauth.py
├── test_get_oauth_token_from_env()
├── test_get_oauth_token_from_keychain()
├── test_set_oauth_token_keychain()
├── test_detect_auth_method_api_key()
├── test_detect_auth_method_oauth()
└── test_oauth_token_storage_path()

tests/unit/test_claude_client_oauth.py
├── test_init_with_api_key_provider()
├── test_init_with_oauth_provider()
├── test_execute_task_with_oauth()
├── test_token_refresh_on_401()
├── test_token_refresh_failure()
└── test_auth_method_logging()
```

**Integration Tests**:
```
tests/integration/test_oauth_flow.py
├── test_oauth_login_flow()  # Requires mock OAuth server
├── test_oauth_token_refresh_integration()
├── test_fallback_to_api_key()  # If implemented
└── test_oauth_service_initialization()
```

**Mock Requirements**:
- Mock OAuth authorization server (token endpoint, refresh endpoint)
- Mock Anthropic API responses with 401 errors
- Mock keyring for credential storage

### 6.4 Backward Compatibility Analysis

**Decision**: Breaking changes acceptable (per DECISION_POINTS.md: "no current users")

**Breaking Changes**:
1. **NONE** - All changes are additive
   - ClaudeClient accepts new optional parameter (`auth_provider`)
   - ConfigManager adds new methods
   - CLI adds new commands
   - All existing API key workflows continue to work

**Migration Path** (if needed in future):
```
Version 0.1.x (Current):
└── API key only via ANTHROPIC_API_KEY

Version 0.2.0 (OAuth support):
├── API key still works (backward compatible)
└── OAuth via new commands (opt-in)

Version 1.0.0 (Hypothetical future):
└── Could deprecate API key mode if desired
```

**Deprecation Strategy** (not needed now):
```python
# Future consideration:
import warnings

if not auth_provider and api_key:
    warnings.warn(
        "Passing api_key directly is deprecated. Use APIKeyAuthProvider instead.",
        DeprecationWarning,
        stacklevel=2
    )
```

---

## 7. Code Quality Analysis

### 7.1 Strengths

1. **Clean Architecture**:
   - Clear layer separation (domain, application, infrastructure)
   - Dependency injection throughout
   - No circular dependencies

2. **Type Safety**:
   - Comprehensive type hints (PEP 484)
   - Mypy strict mode enabled (pyproject.toml:62-73)
   - Pydantic models for data validation

3. **Error Handling**:
   - Structured error responses
   - Graceful degradation
   - Informative error messages with remediation steps

4. **Logging**:
   - Structured logging with structlog
   - Consistent logging patterns
   - Rich context in log messages

5. **Testing Infrastructure**:
   - Unit and integration test separation
   - Async test support (pytest-asyncio)
   - Test coverage tracking

6. **Code Quality Tools**:
   - Ruff (linter)
   - Black (formatter)
   - Mypy (type checker)
   - Pre-commit hooks

### 7.2 Weaknesses / Technical Debt

1. **No Auth Abstraction**:
   - ClaudeClient tightly coupled to API key authentication
   - Hard to test different auth methods
   - No interface for credential providers

2. **ClaudeClient Layer Violation**:
   - Lives in `application/` but directly imports Anthropic SDK (infrastructure)
   - Should be in `infrastructure/` with application-layer interface

3. **No Custom Exception Hierarchy**:
   - Uses built-in exceptions (ValueError, RuntimeError)
   - Hard to catch specific error types
   - No domain-specific error context

4. **Config Model Coupling**:
   - Config class mixes multiple concerns (queue, swarm, loop, resources, monitoring)
   - Could be split into focused sub-configs
   - No validation for cross-field constraints

5. **Limited Retry Strategy**:
   - Relies entirely on Anthropic SDK retry logic
   - No custom retry strategies for specific error types
   - No circuit breaker pattern

6. **Database Schema Evolution**:
   - No migration system (alembic, etc.)
   - Schema changes require manual intervention
   - Could be problematic for production deployments

### 7.3 Refactoring Recommendations

**Pre-OAuth Refactorings** (Optional, but beneficial):

1. **Extract Auth Abstraction** (Priority: HIGH):
   ```
   BEFORE:
   application/claude_client.py - directly uses api_key

   AFTER:
   domain/ports/auth_provider.py - AuthProvider interface
   infrastructure/api_key_auth.py - APIKeyAuthProvider implementation
   infrastructure/anthropic_client.py - Move ClaudeClient here
   application/claude_client.py - Remove or create thin wrapper
   ```

2. **Custom Exception Hierarchy** (Priority: MEDIUM):
   ```
   Create: infrastructure/exceptions.py
   ├── AbathurError (base)
   ├── AuthenticationError
   ├── ConfigurationError
   ├── DatabaseError
   └── TaskExecutionError
   ```

3. **Config Model Refactoring** (Priority: LOW):
   ```
   BEFORE:
   class Config(BaseModel):
       queue: QueueConfig
       swarm: SwarmConfig
       loop: LoopConfig
       resources: ResourceConfig
       monitoring: MonitoringConfig

   AFTER:
   class Config(BaseModel):
       queue: QueueConfig
       swarm: SwarmConfig
       loop: LoopConfig
       resources: ResourceConfig
       monitoring: MonitoringConfig
       auth: AuthConfig  # NEW
   ```

4. **Retry Strategy Enhancement** (Priority: LOW):
   ```python
   # infrastructure/retry.py
   class RetryStrategy:
       async def execute_with_retry(self, func, *args, **kwargs):
           # Custom retry logic with exponential backoff
           # Circuit breaker pattern
           # Error-specific retry rules
   ```

**Refactoring Impact on OAuth Implementation**:
- **Pre-OAuth refactoring**: Makes OAuth integration cleaner
- **During OAuth implementation**: Can be done incrementally
- **Post-OAuth refactoring**: Can improve overall code quality

---

## 8. Integration Strategy Recommendations

### 8.1 Recommended Integration Approach

**Phased Implementation**:

**Phase 1: Foundation (Week 1)**
1. Create `AuthProvider` interface
2. Implement `APIKeyAuthProvider` (wrap existing logic)
3. Add custom exception hierarchy
4. Update tests for new abstractions

**Phase 2: OAuth Core (Week 2)**
1. Implement `OAuthAuthProvider` with token refresh
2. Extend `ConfigManager` with OAuth methods
3. Update `ClaudeClient` to accept `AuthProvider`
4. Add OAuth-specific logging

**Phase 3: CLI Integration (Week 3)**
1. Add OAuth CLI commands (`oauth-login`, `oauth-logout`, etc.)
2. Update service initialization to detect auth method
3. Add OAuth status display
4. Integration testing

**Phase 4: Testing & Documentation (Week 4)**
1. Comprehensive unit tests
2. Integration tests with mock OAuth server
3. Update documentation
4. Add migration guide

### 8.2 Phasing Suggestions

**Incremental Rollout**:

```
Iteration 1: Minimal OAuth Support
├── OAuthAuthProvider (basic token storage)
├── CLI oauth-login command
├── Auto-detection from key prefix
└── Manual token refresh only

Iteration 2: Automatic Token Refresh
├── Token expiry detection
├── Automatic refresh on 401
├── Retry logic with refresh
└── Token refresh logging

Iteration 3: Full OAuth Lifecycle
├── Interactive OAuth flow (if applicable)
├── Refresh token rotation
├── Token expiry warnings
└── Comprehensive error handling

Iteration 4: Production Hardening
├── Circuit breaker for refresh failures
├── Rate limit handling
├── Metrics and monitoring
└── Security audit
```

### 8.3 Risk Mitigation Strategies

| **Risk** | **Likelihood** | **Impact** | **Mitigation** |
|----------|----------------|------------|----------------|
| **Anthropic SDK doesn't support OAuth** | Medium | High | Implement custom HTTP client with httpx |
| **Token refresh failures** | High | Medium | Retry logic, fallback to manual refresh |
| **Breaking existing workflows** | Low | High | Comprehensive backward compatibility tests |
| **Keychain access issues** | Medium | Medium | Fallback to environment variables |
| **OAuth token security** | Medium | High | Use system keychain, encrypt at rest |
| **Performance regression** | Low | Medium | Benchmark auth overhead, optimize hot paths |

**Specific Mitigations**:

1. **SDK Compatibility**:
   ```python
   # Fallback strategy if SDK doesn't support OAuth
   if anthropic_sdk_supports_oauth():
       client = AsyncAnthropic(bearer_token=token)
   else:
       # Use httpx directly with Bearer auth header
       client = CustomAnthropicClient(bearer_token=token)
   ```

2. **Token Refresh Failures**:
   ```python
   async def execute_task_with_retry(self, ...):
       for attempt in range(self.max_refresh_retries):
           try:
               return await self._execute_task(...)
           except OAuthTokenExpiredError:
               if attempt < self.max_refresh_retries - 1:
                   await self.auth_provider.refresh_credentials()
               else:
                   raise
   ```

3. **Backward Compatibility**:
   ```python
   # Test suite to verify API key workflows still work
   def test_api_key_workflow_unchanged():
       # Ensure all existing API key tests pass
       pass
   ```

### 8.4 Testing Strategy

**Test Pyramid**:

```
                    ┌───────────────┐
                    │  E2E Tests    │ (5%) - Full OAuth flow with mock server
                    │  (Manual)     │
                    └───────────────┘
                          ▲
                    ┌─────────────────┐
                    │ Integration     │ (15%) - Service wiring, auth flow
                    │ Tests           │
                    └─────────────────┘
                          ▲
                  ┌───────────────────────┐
                  │   Unit Tests          │ (80%) - AuthProvider, ConfigManager
                  │   (Mocked)            │
                  └───────────────────────┘
```

**Test Coverage Goals**:
- Unit tests: >90%
- Integration tests: >70%
- E2E tests: Critical paths only

**Mock Strategy**:
```python
# tests/mocks/oauth_server.py
class MockOAuthServer:
    """Mock OAuth server for testing."""

    def __init__(self):
        self.tokens = {}
        self.refresh_count = 0

    async def token_endpoint(self, grant_type, code):
        """Mock token issuance."""
        token = f"mock_token_{uuid4()}"
        refresh = f"mock_refresh_{uuid4()}"
        self.tokens[token] = {
            "access_token": token,
            "refresh_token": refresh,
            "expires_in": 3600,
        }
        return self.tokens[token]

    async def refresh_endpoint(self, refresh_token):
        """Mock token refresh."""
        self.refresh_count += 1
        return await self.token_endpoint("refresh_token", None)
```

**CI/CD Integration**:
```yaml
# .github/workflows/test.yml
- name: Run unit tests
  run: pytest tests/unit/ --cov=abathur --cov-report=term-missing

- name: Run integration tests
  run: pytest tests/integration/ --cov=abathur --cov-report=term-missing

- name: Check type hints
  run: mypy src/abathur

- name: Lint code
  run: ruff check src/

- name: Check formatting
  run: black --check src/
```

---

## 9. Code Examples

### 9.1 Current Authentication Pattern

**API Key Loading** (config.py:162-202):
```python
def get_api_key(self) -> str:
    """Get Anthropic API key from environment, keychain, or .env file."""

    # Priority 1: Environment variable
    if key := os.getenv("ANTHROPIC_API_KEY"):
        return key

    # Priority 2: System keychain
    try:
        key = keyring.get_password("abathur", "anthropic_api_key")
        if key:
            return key
    except Exception:
        pass

    # Priority 3: .env file
    env_file = self.project_root / ".env"
    if env_file.exists():
        with open(env_file) as f:
            for line in f:
                line = line.strip()
                if line.startswith("ANTHROPIC_API_KEY="):
                    return line.split("=", 1)[1].strip().strip('"').strip("'")

    raise ValueError("ANTHROPIC_API_KEY not found. Set it via:...")
```

**ClaudeClient Initialization** (claude_client.py:18-43):
```python
class ClaudeClient:
    def __init__(
        self,
        api_key: str | None = None,
        model: str = "claude-sonnet-4-20250514",
        max_retries: int = 3,
        timeout: int = 300,
    ):
        # Get API key from parameter or environment
        self.api_key = api_key or os.getenv("ANTHROPIC_API_KEY")
        if not self.api_key:
            raise ValueError("ANTHROPIC_API_KEY must be provided or set in environment")

        # Store configuration
        self.model = model
        self.max_retries = max_retries
        self.timeout = timeout

        # Initialize Anthropic SDK clients
        self.client = Anthropic(api_key=self.api_key, max_retries=max_retries)
        self.async_client = AsyncAnthropic(api_key=self.api_key, max_retries=max_retries)
```

**Task Execution** (claude_client.py:45-107):
```python
async def execute_task(
    self,
    system_prompt: str,
    user_message: str,
    max_tokens: int = 8000,
    temperature: float = 0.7,
    model: str | None = None,
) -> dict[str, Any]:
    """Execute a task using Claude."""
    model_to_use = model or self.model

    try:
        logger.info("executing_claude_task", model=model_to_use)

        # Make authenticated API request
        response = await self.async_client.messages.create(
            model=model_to_use,
            max_tokens=max_tokens,
            temperature=temperature,
            system=system_prompt,
            messages=[{"role": "user", "content": user_message}],
            timeout=self.timeout,
        )

        # Extract text content
        content_text = ""
        for block in response.content:
            if hasattr(block, "text"):
                content_text += block.text

        # Return structured result
        return {
            "success": True,
            "content": content_text,
            "stop_reason": response.stop_reason,
            "usage": {
                "input_tokens": response.usage.input_tokens,
                "output_tokens": response.usage.output_tokens,
            },
            "error": None,
        }

    except Exception as e:
        logger.error("claude_task_failed", error=str(e))
        return {
            "success": False,
            "content": "",
            "stop_reason": "error",
            "usage": {"input_tokens": 0, "output_tokens": 0},
            "error": str(e),
        }
```

### 9.2 Current Configuration Loading

**Hierarchical Config Loading** (config.py:79-118):
```python
def load_config(self) -> Config:
    """Load configuration from all sources in hierarchy order."""
    if self._config is not None:
        return self._config

    # Start with empty dict
    config_dict: dict[str, Any] = {}

    # Load template defaults
    template_config_path = self.project_root / ".abathur" / "config.yaml"
    if template_config_path.exists():
        config_dict = self._merge_dicts(config_dict, self._load_yaml(template_config_path))

    # Load user overrides
    user_config_path = Path.home() / ".abathur" / "config.yaml"
    if user_config_path.exists():
        config_dict = self._merge_dicts(config_dict, self._load_yaml(user_config_path))

    # Load project overrides
    local_config_path = self.project_root / ".abathur" / "local.yaml"
    if local_config_path.exists():
        config_dict = self._merge_dicts(config_dict, self._load_yaml(local_config_path))

    # Apply environment variables
    config_dict = self._apply_env_vars(config_dict)

    # Create and validate config
    self._config = Config(**config_dict)
    return self._config
```

**Environment Variable Mapping** (config.py:135-160):
```python
def _apply_env_vars(self, config_dict: dict[str, Any]) -> dict[str, Any]:
    """Apply environment variables with ABATHUR_ prefix."""
    # Map of env var names to config paths
    env_mappings = {
        "ABATHUR_LOG_LEVEL": ["log_level"],
        "ABATHUR_QUEUE_MAX_SIZE": ["queue", "max_size"],
        "ABATHUR_MAX_CONCURRENT_AGENTS": ["swarm", "max_concurrent_agents"],
        "ABATHUR_MAX_ITERATIONS": ["loop", "max_iterations"],
    }

    for env_var, path in env_mappings.items():
        value = os.getenv(env_var)
        if value is not None:
            # Navigate to the nested dict
            current = config_dict
            for key in path[:-1]:
                if key not in current:
                    current[key] = {}
                current = current[key]
            # Set the value (convert to int if needed)
            try:
                current[path[-1]] = int(value)
            except ValueError:
                current[path[-1]] = value

    return config_dict
```

### 9.3 Current Agent Spawning

**Service Initialization** (cli/main.py:28-71):
```python
async def _get_services() -> dict[str, Any]:
    """Get initialized services."""
    from abathur.application import (
        AgentExecutor,
        ClaudeClient,
        FailureRecovery,
        LoopExecutor,
        MCPManager,
        ResourceMonitor,
        SwarmOrchestrator,
        TaskCoordinator,
        TemplateManager,
    )
    from abathur.infrastructure import ConfigManager, Database

    # Initialize config and database
    config_manager = ConfigManager()
    database = Database(config_manager.get_database_path())
    await database.initialize()

    # Initialize task coordinator
    task_coordinator = TaskCoordinator(database)

    # Initialize ClaudeClient with API key
    claude_client = ClaudeClient(api_key=config_manager.get_api_key())

    # Initialize agent executor with ClaudeClient
    agent_executor = AgentExecutor(database, claude_client)

    # Initialize swarm orchestrator
    swarm_orchestrator = SwarmOrchestrator(
        task_coordinator, agent_executor, max_concurrent_agents=10
    )

    # Initialize remaining services
    template_manager = TemplateManager()
    mcp_manager = MCPManager()
    await mcp_manager.initialize()
    failure_recovery = FailureRecovery(task_coordinator, database)
    resource_monitor = ResourceMonitor()
    loop_executor = LoopExecutor(task_coordinator, agent_executor, database)

    # Return service dictionary
    return {
        "database": database,
        "task_coordinator": task_coordinator,
        "claude_client": claude_client,
        "agent_executor": agent_executor,
        "swarm_orchestrator": swarm_orchestrator,
        "template_manager": template_manager,
        "mcp_manager": mcp_manager,
        "failure_recovery": failure_recovery,
        "resource_monitor": resource_monitor,
        "loop_executor": loop_executor,
    }
```

**Agent Task Execution** (agent_executor.py:38-130):
```python
async def execute_task(self, task: Task) -> Result:
    """Execute a task using an agent."""
    agent_id = uuid4()

    try:
        # Load agent definition from YAML
        agent_def = self._load_agent_definition(task.template_name)

        # Create agent record
        agent = Agent(
            id=agent_id,
            name=task.template_name,
            specialization=agent_def.get("specialization", task.template_name),
            task_id=task.id,
            state=AgentState.SPAWNING,
            model=agent_def.get("model", "claude-sonnet-4-20250514"),
        )

        # Insert agent into database
        await self.database.insert_agent(agent)
        await self.database.update_agent_state(agent_id, AgentState.IDLE)

        logger.info("agent_spawned", agent_id=str(agent_id), task_id=str(task.id))

        # Update agent to busy
        await self.database.update_agent_state(agent_id, AgentState.BUSY)

        # Build prompts
        system_prompt = agent_def.get("system_prompt", "")
        user_message = self._build_user_message(task, agent_def)

        # Execute with ClaudeClient
        logger.info("executing_task", task_id=str(task.id), agent_id=str(agent_id))

        response = await self.claude_client.execute_task(
            system_prompt=system_prompt,
            user_message=user_message,
            max_tokens=agent_def.get("resource_limits", {}).get("max_tokens", 8000),
            temperature=agent_def.get("resource_limits", {}).get("temperature", 0.7),
            model=agent.model,
        )

        # Create result
        result = Result(
            task_id=task.id,
            agent_id=agent_id,
            success=response["success"],
            data={"output": response["content"]} if response["success"] else None,
            error=response.get("error"),
            metadata={"stop_reason": response["stop_reason"]},
            token_usage=response["usage"],
        )

        # Terminate agent
        await self.database.update_agent_state(agent_id, AgentState.TERMINATING)
        await self.database.update_agent_state(agent_id, AgentState.TERMINATED)

        # Log audit
        await self.database.log_audit(
            task_id=task.id,
            agent_id=agent_id,
            action_type="task_executed",
            action_data={
                "template": task.template_name,
                "tokens_used": sum(response["usage"].values()),
            },
            result="success" if response["success"] else "failed",
        )

        logger.info("task_execution_complete", task_id=str(task.id), success=result.success)

        return result

    except Exception as e:
        logger.error("task_execution_error", task_id=str(task.id), error=str(e))

        # Try to update agent state
        try:
            await self.database.update_agent_state(agent_id, AgentState.TERMINATED)
        except Exception:
            pass

        return Result(
            task_id=task.id,
            agent_id=agent_id,
            success=False,
            error=f"Execution error: {e}",
        )
```

### 9.4 Suggested Abstraction Examples

**AuthProvider Interface** (NEW):
```python
# domain/ports/auth_provider.py

from abc import ABC, abstractmethod
from typing import Literal
from datetime import datetime


class AuthProvider(ABC):
    """Abstract authentication provider for Claude API."""

    @abstractmethod
    async def get_credentials(self) -> dict[str, str]:
        """Get credentials for API requests.

        Returns:
            Dict with:
            - 'type': 'api_key' | 'bearer'
            - 'value': credential value
            - 'expires_at': expiry timestamp (ISO format, for OAuth)
        """
        pass

    @abstractmethod
    async def refresh_credentials(self) -> bool:
        """Refresh expired credentials.

        Returns:
            True if refresh successful, False otherwise
        """
        pass

    @abstractmethod
    def is_valid(self) -> bool:
        """Check if current credentials are valid and not expired."""
        pass

    @abstractmethod
    def get_auth_method(self) -> Literal["api_key", "oauth"]:
        """Get authentication method type."""
        pass


class APIKeyAuthProvider(AuthProvider):
    """API key authentication provider."""

    def __init__(self, api_key: str):
        """Initialize with API key.

        Args:
            api_key: Anthropic API key
        """
        self.api_key = api_key

    async def get_credentials(self) -> dict[str, str]:
        """Get API key credentials."""
        return {
            "type": "api_key",
            "value": self.api_key,
        }

    async def refresh_credentials(self) -> bool:
        """API keys don't expire, no refresh needed."""
        return True

    def is_valid(self) -> bool:
        """Check if API key is set."""
        return self.api_key is not None and len(self.api_key) > 0

    def get_auth_method(self) -> Literal["api_key", "oauth"]:
        """Return 'api_key'."""
        return "api_key"


class OAuthAuthProvider(AuthProvider):
    """OAuth authentication provider with token refresh."""

    def __init__(
        self,
        access_token: str,
        refresh_token: str,
        expires_at: datetime,
        config_manager: "ConfigManager",  # For storing refreshed tokens
    ):
        """Initialize with OAuth tokens.

        Args:
            access_token: OAuth access token
            refresh_token: OAuth refresh token
            expires_at: Token expiry timestamp
            config_manager: ConfigManager for token storage
        """
        self.access_token = access_token
        self.refresh_token = refresh_token
        self.expires_at = expires_at
        self.config_manager = config_manager

    async def get_credentials(self) -> dict[str, str]:
        """Get OAuth credentials, refreshing if expired."""
        # Check if token is expired
        if self._is_expired():
            # Attempt to refresh
            if not await self.refresh_credentials():
                raise OAuthTokenExpiredError("Failed to refresh expired token")

        return {
            "type": "bearer",
            "value": self.access_token,
            "expires_at": self.expires_at.isoformat(),
        }

    async def refresh_credentials(self) -> bool:
        """Refresh OAuth token using refresh token."""
        try:
            # Call OAuth token refresh endpoint
            # (Implementation depends on Anthropic's OAuth flow)
            new_tokens = await self._call_refresh_endpoint()

            # Update tokens
            self.access_token = new_tokens["access_token"]
            self.refresh_token = new_tokens.get("refresh_token", self.refresh_token)
            self.expires_at = datetime.now() + timedelta(seconds=new_tokens["expires_in"])

            # Persist new tokens
            await self.config_manager.set_oauth_token(
                self.access_token,
                self.refresh_token,
                self.expires_at,
            )

            logger.info("oauth_token_refreshed", expires_at=self.expires_at.isoformat())
            return True

        except Exception as e:
            logger.error("oauth_token_refresh_failed", error=str(e))
            return False

    def is_valid(self) -> bool:
        """Check if token is valid and not expired."""
        return self.access_token is not None and not self._is_expired()

    def get_auth_method(self) -> Literal["api_key", "oauth"]:
        """Return 'oauth'."""
        return "oauth"

    def _is_expired(self) -> bool:
        """Check if access token is expired."""
        from datetime import datetime, timezone
        now = datetime.now(timezone.utc)
        # Add 5-minute buffer for clock skew
        return now >= (self.expires_at - timedelta(minutes=5))

    async def _call_refresh_endpoint(self) -> dict:
        """Call OAuth refresh endpoint.

        Returns:
            Dict with new access_token, refresh_token, expires_in
        """
        # TODO: Implement based on Anthropic's OAuth spec
        # This is a placeholder
        async with httpx.AsyncClient() as client:
            response = await client.post(
                "https://api.anthropic.com/oauth/token",  # Placeholder URL
                data={
                    "grant_type": "refresh_token",
                    "refresh_token": self.refresh_token,
                },
            )
            response.raise_for_status()
            return response.json()
```

**Updated ClaudeClient** (MODIFIED):
```python
# application/claude_client.py (UPDATED)

class ClaudeClient:
    """Wrapper for Anthropic Claude API with retry logic and rate limiting."""

    def __init__(
        self,
        api_key: str | None = None,  # Backward compatibility
        auth_provider: AuthProvider | None = None,  # NEW
        model: str = "claude-sonnet-4-20250514",
        max_retries: int = 3,
        timeout: int = 300,
    ):
        """Initialize Claude client.

        Args:
            api_key: Anthropic API key (deprecated, use auth_provider)
            auth_provider: Authentication provider (API key or OAuth)
            model: Default model to use
            max_retries: Maximum retry attempts for transient errors
            timeout: Request timeout in seconds
        """
        # Initialize auth provider
        if auth_provider:
            self.auth_provider = auth_provider
        elif api_key:
            # Backward compatibility: create API key provider
            self.auth_provider = APIKeyAuthProvider(api_key)
        else:
            # Fallback to environment variable
            env_api_key = os.getenv("ANTHROPIC_API_KEY")
            if env_api_key:
                self.auth_provider = APIKeyAuthProvider(env_api_key)
            else:
                raise ValueError(
                    "Authentication required. Provide api_key, auth_provider, "
                    "or set ANTHROPIC_API_KEY environment variable."
                )

        self.model = model
        self.max_retries = max_retries
        self.timeout = timeout

        # Log authentication method
        logger.info(
            "claude_client_initialized",
            auth_method=self.auth_provider.get_auth_method(),
        )

        # Initialize SDK clients (will be configured with auth on first use)
        self._sync_client = None
        self._async_client = None

    async def _get_async_client(self) -> AsyncAnthropic:
        """Get or create async client with current credentials."""
        credentials = await self.auth_provider.get_credentials()

        if credentials["type"] == "api_key":
            return AsyncAnthropic(
                api_key=credentials["value"],
                max_retries=self.max_retries,
            )
        elif credentials["type"] == "bearer":
            # OAuth bearer token
            # NOTE: This assumes Anthropic SDK supports bearer tokens
            # If not, we'll need to use httpx directly
            return AsyncAnthropic(
                bearer_token=credentials["value"],  # Check if SDK supports this
                max_retries=self.max_retries,
            )
        else:
            raise ValueError(f"Unsupported auth type: {credentials['type']}")

    async def execute_task(
        self,
        system_prompt: str,
        user_message: str,
        max_tokens: int = 8000,
        temperature: float = 0.7,
        model: str | None = None,
    ) -> dict[str, Any]:
        """Execute a task using Claude."""
        model_to_use = model or self.model

        # Retry loop for token refresh
        for attempt in range(self.max_retries):
            try:
                logger.info(
                    "executing_claude_task",
                    model=model_to_use,
                    auth_method=self.auth_provider.get_auth_method(),
                )

                # Get client with current credentials
                async_client = await self._get_async_client()

                # Make API request
                response = await async_client.messages.create(
                    model=model_to_use,
                    max_tokens=max_tokens,
                    temperature=temperature,
                    system=system_prompt,
                    messages=[{"role": "user", "content": user_message}],
                    timeout=self.timeout,
                )

                # Extract text content
                content_text = ""
                for block in response.content:
                    if hasattr(block, "text"):
                        content_text += block.text

                result = {
                    "success": True,
                    "content": content_text,
                    "stop_reason": response.stop_reason,
                    "usage": {
                        "input_tokens": response.usage.input_tokens,
                        "output_tokens": response.usage.output_tokens,
                    },
                    "error": None,
                }

                logger.info(
                    "claude_task_completed",
                    tokens_used=response.usage.input_tokens + response.usage.output_tokens,
                    stop_reason=response.stop_reason,
                )

                return result

            except HTTPStatusError as e:
                # Check for 401 Unauthorized (token expired)
                if e.response.status_code == 401:
                    logger.warning(
                        "auth_failed_attempting_refresh",
                        attempt=attempt + 1,
                        max_attempts=self.max_retries,
                    )

                    # Attempt to refresh credentials
                    if attempt < self.max_retries - 1:
                        if await self.auth_provider.refresh_credentials():
                            logger.info("credentials_refreshed_retrying")
                            continue  # Retry with refreshed credentials
                        else:
                            logger.error("credential_refresh_failed")
                            raise OAuthRefreshError("Failed to refresh credentials") from e
                    else:
                        raise OAuthTokenExpiredError("Max refresh attempts exceeded") from e
                else:
                    # Non-auth error, don't retry
                    raise

            except Exception as e:
                logger.error("claude_task_failed", error=str(e), attempt=attempt + 1)

                # Don't retry on non-auth errors
                return {
                    "success": False,
                    "content": "",
                    "stop_reason": "error",
                    "usage": {"input_tokens": 0, "output_tokens": 0},
                    "error": str(e),
                }

        # Should not reach here
        return {
            "success": False,
            "content": "",
            "stop_reason": "error",
            "usage": {"input_tokens": 0, "output_tokens": 0},
            "error": "Max retries exceeded",
        }
```

---

## 10. Open Questions

### 10.1 Anthropic SDK OAuth Support

**Question**: Does Anthropic Python SDK (^0.18.0) support OAuth bearer tokens?

**Current Status**: UNKNOWN - Requires investigation

**Investigation Plan**:
1. Check Anthropic SDK documentation for OAuth/bearer token support
2. Review SDK source code for authentication mechanisms
3. Test SDK with mock bearer token
4. Contact Anthropic support if unclear

**Implications**:
- **If YES**: Use SDK with bearer token parameter
- **If NO**: Implement custom HTTP client with httpx

### 10.2 OAuth Flow Details

**Question**: What OAuth flow does Anthropic support?

**Possible Flows**:
1. **Authorization Code Flow** (standard OAuth 2.0)
2. **Device Code Flow** (for CLI tools)
3. **Client Credentials Flow** (service-to-service)
4. **Pre-issued tokens** (user provides token manually)

**Investigation Needed**:
- Anthropic OAuth documentation
- Claude Max subscription OAuth integration guide
- Community resources (claude_max tool analysis)

**Impact on Design**:
- Different flows require different CLI commands
- Device flow requires polling mechanism
- Authorization code flow requires callback URL

### 10.3 Token Storage Security

**Question**: What is the most secure way to store OAuth tokens on macOS?

**Options**:
1. **System Keychain** (current approach for API keys)
   - Pros: OS-level encryption, secure
   - Cons: Requires user interaction on first access

2. **Encrypted File** (custom encryption)
   - Pros: Portable, no OS dependencies
   - Cons: Key management complexity

3. **Environment Variables Only**
   - Pros: Simple, cloud-native
   - Cons: Exposed in process listings, not persistent

**Decision Criteria**:
- Security level required
- User experience (minimal interaction)
- Cross-platform compatibility (future)

**Recommendation**: System keychain (per DECISION_POINTS.md)

### 10.4 Token Refresh Timing

**Question**: When should OAuth tokens be refreshed?

**Options**:
1. **Proactive** (refresh before expiry)
   - Refresh when 80% of TTL has passed
   - Avoid mid-request expiry

2. **Reactive** (refresh on 401 error)
   - Only refresh when necessary
   - Simpler implementation

3. **Hybrid** (proactive with reactive fallback)
   - Check expiry before requests
   - Refresh on 401 as fallback

**Recommendation**: Hybrid approach
- Check token expiry in `get_credentials()`
- Refresh if within 5 minutes of expiry
- Fallback to reactive refresh on 401

### 10.5 Multi-User Support

**Question**: Should we design for multi-user from the start?

**Current Decision**: Single-user (per DECISION_POINTS.md)

**Future Considerations**:
- Multiple users on same system
- Team credentials vs individual credentials
- Credential switching mechanism

**Design Impact**:
- If single-user: Store tokens globally
- If multi-user: Store tokens per-user or per-project

**Recommendation**: Design abstraction to support future multi-user without breaking changes

---

## 11. Appendix

### 11.1 Complete File Listings

**Source Files** (22 files):
```
src/abathur/
├── __init__.py
├── application/
│   ├── __init__.py
│   ├── agent_executor.py (207 lines)
│   ├── agent_pool.py (223 lines)
│   ├── claude_client.py (210 lines) [CRITICAL]
│   ├── failure_recovery.py
│   ├── loop_executor.py
│   ├── mcp_manager.py
│   ├── resource_monitor.py
│   ├── swarm_orchestrator.py (211 lines)
│   ├── task_coordinator.py (163 lines)
│   └── template_manager.py
├── cli/
│   ├── __init__.py
│   └── main.py (676 lines) [CRITICAL]
├── domain/
│   ├── __init__.py
│   └── models.py (121 lines)
└── infrastructure/
    ├── __init__.py
    ├── config.py (234 lines) [CRITICAL]
    ├── database.py (472 lines)
    ├── logger.py (98 lines)
    └── mcp_config.py
```

**Test Files** (8 files):
```
tests/
├── __init__.py
├── unit/
│   ├── __init__.py
│   ├── test_config.py (195 lines)
│   ├── test_loop_executor.py
│   ├── test_mcp_manager.py
│   └── test_models.py
└── integration/
    ├── __init__.py
    └── test_database.py
```

**Total LOC Analysis**:
- ClaudeClient: 210 lines (primary auth component)
- ConfigManager: 234 lines (credential management)
- CLI Main: 676 lines (service initialization)
- **Estimated OAuth Changes**: ~500-800 new lines across all components

### 11.2 Dependency Tree

**Direct Dependencies** (pyproject.toml:13-25):
```
anthropic = "^0.18.0"       # Claude API SDK
typer = "^0.12.0"           # CLI framework
rich = "^13.7.0"            # Terminal formatting
pydantic = "^2.5.0"         # Data validation
python-dotenv = "^1.0.0"    # .env file loading
keyring = "^24.3.0"         # Credential storage
structlog = "^24.1.0"       # Structured logging
aiosqlite = "^0.19.0"       # Async SQLite
psutil = "^5.9.0"           # Resource monitoring
pyyaml = "^6.0.1"           # YAML parsing
```

**Development Dependencies** (pyproject.toml:27-36):
```
pytest = "^7.4.0"           # Testing framework
pytest-asyncio = "^0.21.0"  # Async test support
pytest-cov = "^4.1.0"       # Coverage reporting
mypy = "^1.7.0"             # Type checking
ruff = "^0.1.9"             # Linting
black = "^23.12.0"          # Code formatting
pre-commit = "^3.6.0"       # Git hooks
types-pyyaml = "^6.0.12"    # Type stubs
types-psutil = "^7.0.0"     # Type stubs
```

### 11.3 Import Graph

**Visualization**:
```
CLI Layer (cli/main.py)
    │
    ├──> ConfigManager (infrastructure/config.py)
    │       │
    │       ├──> Config (pydantic models)
    │       ├──> keyring (external)
    │       └──> yaml (external)
    │
    ├──> ClaudeClient (application/claude_client.py)
    │       │
    │       ├──> Anthropic, AsyncAnthropic (external SDK)
    │       └──> logger (infrastructure/logger.py)
    │
    ├──> AgentExecutor (application/agent_executor.py)
    │       │
    │       ├──> ClaudeClient (receives via DI)
    │       ├──> Database (infrastructure/database.py)
    │       ├──> Agent, Task, Result (domain/models.py)
    │       └──> logger
    │
    ├──> SwarmOrchestrator (application/swarm_orchestrator.py)
    │       │
    │       ├──> AgentExecutor (receives via DI)
    │       ├──> TaskCoordinator (application/task_coordinator.py)
    │       └──> logger
    │
    └──> TaskCoordinator (application/task_coordinator.py)
            │
            ├──> Database
            ├──> Task (domain/models.py)
            └──> logger

Database (infrastructure/database.py)
    │
    ├──> aiosqlite (external)
    └──> Agent, Task (domain/models.py)

Domain Models (domain/models.py)
    │
    └──> pydantic (external)
```

**Key Observations**:
- Clean dependency direction (domain ← application ← infrastructure ← interface)
- No circular dependencies
- Dependency injection prevents tight coupling
- External dependencies isolated to infrastructure layer (except ClaudeClient)

---

## 12. Summary

### 12.1 Key Findings

1. **Clean Architecture**: Abathur follows Clean Architecture principles with clear layer separation
2. **Single Auth Point**: Authentication initialized in one location (ClaudeClient.__init__)
3. **Dependency Injection**: All components use DI, making auth changes localized
4. **No Auth Abstraction**: Current design tightly couples to API key authentication
5. **Good Test Coverage**: Testing infrastructure in place for new OAuth features

### 12.2 Critical Integration Points

| **Component** | **File:Line** | **Change Type** |
|---------------|---------------|-----------------|
| ClaudeClient.__init__ | application/claude_client.py:18-43 | MAJOR - Accept AuthProvider |
| ConfigManager.get_api_key | infrastructure/config.py:162-202 | MODERATE - Add OAuth methods |
| CLI _get_services | cli/main.py:48 | MODERATE - Detect and initialize auth |

### 12.3 Recommended Next Steps

**Phase 1**: Create auth abstraction (Week 1)
- Implement `AuthProvider` interface
- Create `APIKeyAuthProvider` (wrap existing logic)
- Add custom exception hierarchy

**Phase 2**: Implement OAuth core (Week 2)
- Create `OAuthAuthProvider` with token refresh
- Extend `ConfigManager` with OAuth methods
- Update `ClaudeClient` to use `AuthProvider`

**Phase 3**: CLI integration (Week 3)
- Add OAuth CLI commands
- Update service initialization
- Integration testing

**Phase 4**: Testing and documentation (Week 4)
- Comprehensive test coverage
- Documentation updates
- Migration guide

### 12.4 Risk Mitigation

- **Anthropic SDK OAuth support**: Research and fallback to httpx if needed
- **Token refresh failures**: Implement retry logic with exponential backoff
- **Backward compatibility**: Comprehensive test suite for API key workflows
- **Security**: Use system keychain for token storage

---

**End of Architecture Analysis**

**Next Phase**: Phase 2 - Technical Requirements and Architecture Design
**Blocking On**: oauth-research-specialist deliverable (01_oauth_research.md)
**Estimated OAuth Implementation**: 2-3 weeks with 500-800 LOC changes
