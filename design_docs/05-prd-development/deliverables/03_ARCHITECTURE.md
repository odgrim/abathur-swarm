# Abathur System Architecture

**Document Version:** 1.0
**Date:** 2025-10-09
**Status:** Complete - Ready for System Design Phase
**Previous Phase:** Requirements Specification (02_REQUIREMENTS.md)
**Next Phase:** System Design and API Specification

---

## 1. Architecture Overview

### 1.1 System Context

Abathur is a CLI-first orchestration system for managing swarms of specialized Claude agents. It provides production-grade task queue management, concurrent agent coordination, iterative refinement loops, and comprehensive observability - all running locally with SQLite persistence.

### 1.2 High-Level Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                        CLI Interface Layer                       │
│  (Typer commands: init, task, swarm, loop, config, status)     │
└────────────────────────────┬────────────────────────────────────┘
                             │
┌────────────────────────────▼────────────────────────────────────┐
│                    Application Service Layer                     │
│  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐         │
│  │   Template   │  │    Swarm     │  │     Loop     │         │
│  │   Manager    │  │ Orchestrator │  │   Executor   │         │
│  └──────────────┘  └──────────────┘  └──────────────┘         │
│  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐         │
│  │     Task     │  │   Monitor    │  │  Meta-Agent  │         │
│  │ Coordinator  │  │   Manager    │  │   Improver   │         │
│  └──────────────┘  └──────────────┘  └──────────────┘         │
└────────────────────────────┬────────────────────────────────────┘
                             │
┌────────────────────────────▼────────────────────────────────────┐
│                       Core Domain Layer                          │
│  Task, Agent, Queue, ExecutionContext, Result, LoopState        │
└────────────────────────────┬────────────────────────────────────┘
                             │
┌────────────────────────────▼────────────────────────────────────┐
│                    Infrastructure Layer                          │
│  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐         │
│  │    Queue     │  │    State     │  │    Claude    │         │
│  │  Repository  │  │    Store     │  │    Client    │         │
│  │  (SQLite)    │  │  (SQLite)    │  │  (SDK Wrap)  │         │
│  └──────────────┘  └──────────────┘  └──────────────┘         │
│  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐         │
│  │   Template   │  │    Config    │  │   Logger     │         │
│  │  Repository  │  │   Manager    │  │  (structlog) │         │
│  │    (Git)     │  │  (YAML+env)  │  │              │         │
│  └──────────────┘  └──────────────┘  └──────────────┘         │
└─────────────────────────────────────────────────────────────────┘

Data Flow:
  CLI → TaskCoordinator → QueueRepository → SwarmOrchestrator
      → AgentPool (async) → ClaudeClient → Results → StateStore
```

### 1.3 Architectural Principles

1. **Clean Separation of Concerns:** CLI, application logic, domain models, and infrastructure are cleanly separated
2. **Async-First Design:** Asyncio-based concurrency enables efficient multi-agent coordination
3. **Persistence-First:** SQLite ACID transactions ensure >99.9% data reliability
4. **Resource-Aware:** Configurable limits (10+ agents, 4GB memory) with adaptive scaling
5. **Observability-First:** Structured logging and audit trail are core, not afterthoughts

---

## 2. Component Architecture

### 2.1 CLI Interface Layer

**Component:** CLI Command Handler
- **Responsibility:** Parse user commands, validate arguments, invoke application services
- **Key Interfaces:**
  - `abathur init [--version]` - Initialize project with template
  - `abathur task submit/list/cancel/detail` - Task queue operations
  - `abathur swarm status` - Agent pool monitoring
  - `abathur loop start/history` - Iterative execution
  - `abathur status [--watch]` - Real-time system status
  - `abathur config validate/set-key` - Configuration management
- **Technology:** Typer (type-safe CLI framework built on Click)
- **Performance Target:** <100ms command parsing, <500ms startup time

### 2.2 Application Service Layer

#### 2.2.1 TemplateManager
- **Responsibility:** Clone, cache, and validate agent templates using git
- **Key Interfaces:**
  - `fetch_template(version: str) -> Template`
  - `install_template(path: Path) -> InstallResult`
  - `validate_template(template: Template) -> ValidationResult`
  - `update_template(strategy: MergeStrategy) -> UpdateResult`
- **Dependencies:** Git (via subprocess), FileSystem, ConfigManager
- **Data Flow:** Git Repository → Local Cache (`~/.abathur/cache/`) → Project (`.claude/`, `.abathur/`)

#### 2.2.2 SwarmOrchestrator
- **Responsibility:** Spawn and coordinate multiple Claude agents concurrently
- **Key Interfaces:**
  - `spawn_agents(count: int, config: AgentConfig) -> List[Agent]`
  - `distribute_tasks(tasks: List[Task], agents: List[Agent]) -> None`
  - `collect_results(agents: List[Agent]) -> AggregatedResult`
  - `handle_agent_failure(agent: Agent, error: Error) -> None`
- **Dependencies:** ClaudeClient, TaskQueue, StateStore, ResourceMonitor
- **Concurrency Pattern:** Asyncio task groups with configurable semaphore (default: 10)
- **Performance Target:** <5s agent spawn, <100ms task distribution

#### 2.2.3 LoopExecutor
- **Responsibility:** Execute iterative task loops until convergence or limit
- **Key Interfaces:**
  - `execute_loop(task: Task, criteria: Convergence) -> LoopResult`
  - `evaluate_convergence(result: Result, criteria: Convergence) -> bool`
  - `checkpoint_state(iteration: int, state: LoopState) -> None`
  - `resume_from_checkpoint(task_id: UUID) -> LoopState`
- **Dependencies:** ClaudeClient, StateStore, ConvergenceEvaluator
- **State Management:** Checkpoint after each iteration to `.abathur/abathur.db`
- **Termination:** Max iterations (default: 10) OR success criteria OR timeout (default: 1h)

#### 2.2.4 TaskCoordinator
- **Responsibility:** Manage task queue, prioritization, and lifecycle
- **Key Interfaces:**
  - `submit_task(task: Task) -> UUID`
  - `dequeue_next(priority_min: int = 0) -> Optional[Task]`
  - `cancel_task(task_id: UUID) -> CancelResult`
  - `update_status(task_id: UUID, status: TaskStatus) -> None`
- **Dependencies:** QueueRepository, StateStore
- **Scheduling:** Priority 0-10 (10=highest), FIFO tiebreaker, configurable capacity (1,000 default)
- **Performance Target:** <100ms for submit/list/cancel operations

#### 2.2.5 MonitorManager
- **Responsibility:** Structured logging, metrics collection, audit trail
- **Key Interfaces:**
  - `log_event(level: LogLevel, component: str, event: Event) -> None`
  - `record_metric(name: str, value: float, labels: Dict) -> None`
  - `query_audit_trail(filters: AuditFilter) -> List[AuditEntry]`
  - `get_system_status() -> SystemStatus`
- **Dependencies:** Logger (structlog), StateStore
- **Storage:** Logs to `.abathur/logs/abathur.log`, 30-day rotation, JSON format
- **Performance Target:** <50ms status queries, async logging (non-blocking)

#### 2.2.6 ConfigManager
- **Responsibility:** Load, validate, and merge configuration from multiple sources
- **Key Interfaces:**
  - `load_config(profile: str = "default") -> Config`
  - `validate_config(config: Config) -> ValidationResult`
  - `get_setting(key: str, default: Any = None) -> Any`
  - `set_api_key(key: str, store: KeyStore = "keychain") -> None`
- **Configuration Hierarchy:**
  1. System defaults (embedded in code)
  2. Template defaults (`.abathur/config.yaml`)
  3. User overrides (`~/.abathur/config.yaml`)
  4. Project overrides (`.abathur/local.yaml`, gitignored)
  5. Environment variables (`ABATHUR_*` prefix, highest priority)
- **Dependencies:** Pydantic (validation), python-dotenv, keyring (keychain)

#### 2.2.7 MetaAgentImprover
- **Responsibility:** Analyze agent performance and generate improvements
- **Key Interfaces:**
  - `analyze_performance(agent_name: str) -> PerformanceReport`
  - `collect_feedback(task_id: UUID, feedback: Feedback) -> None`
  - `generate_improvement(agent_name: str, feedback_ids: List[UUID]) -> ImprovedAgent`
  - `validate_improvement(original: Agent, improved: Agent) -> ValidationResult`
- **Dependencies:** SwarmOrchestrator (spawns meta-agent), StateStore, TemplateManager
- **Priority:** Low (Could Have) - Future enhancement for self-improvement

### 2.3 Core Domain Layer

**Domain Models:**
- `Task` - Represents a unit of work (ID, template, inputs, priority, status, timestamps)
- `Agent` - Represents a Claude agent instance (ID, specialization, state, resource usage)
- `Queue` - Priority queue abstraction (pending, running, completed, failed tasks)
- `ExecutionContext` - Runtime context for agent execution (task, config, shared state)
- `Result` - Output from agent execution (success/failure, data, metadata, token usage)
- `LoopState` - State of iterative loop (iteration count, history, convergence status)

**Business Rules:**
- Priority range: 0-10 (10=highest priority)
- Task states: pending → running → completed/failed/cancelled
- Agent lifecycle: spawning → idle → busy → terminating → terminated
- Loop termination: max_iterations OR convergence_met OR timeout_exceeded

### 2.4 Infrastructure Layer

#### 2.4.1 QueueRepository (SQLite)
- **Responsibility:** Persist task queue with ACID guarantees
- **Key Methods:**
  - `enqueue(task: Task) -> None`
  - `dequeue_highest_priority() -> Optional[Task]`
  - `update_task_status(task_id: UUID, status: TaskStatus) -> None`
  - `list_tasks(filters: TaskFilter) -> List[Task]`
- **Schema:** See Section 4 for full database schema
- **Performance:** SQLite WAL mode for concurrent reads, connection pool

#### 2.4.2 StateStore (SQLite)
- **Responsibility:** Persist agent state, shared data, checkpoints
- **Key Methods:**
  - `save_agent_state(agent_id: UUID, state: AgentState) -> None`
  - `get_shared_state(task_id: UUID, key: str) -> Any`
  - `set_shared_state(task_id: UUID, key: str, value: Any) -> None`
  - `save_checkpoint(task_id: UUID, checkpoint: Checkpoint) -> None`
- **Isolation:** State scoped to task_id for multi-task isolation

#### 2.4.3 ClaudeClient (SDK Wrapper)
- **Responsibility:** Wrap Anthropic Python SDK, manage rate limits
- **Key Methods:**
  - `create_agent(config: AgentConfig) -> Agent`
  - `execute_task(agent: Agent, task: Task) -> Result`
  - `stream_response(agent: Agent, prompt: str) -> AsyncIterator[Chunk]`
- **Rate Limiting:** Token bucket algorithm, configurable requests/minute
- **Retry Logic:** Exponential backoff (10s → 5min), 3 retries default

#### 2.4.4 TemplateRepository (Git)
- **Responsibility:** Clone templates using git, manage local cache
- **Key Methods:**
  - `clone_template(repo: str, version: str) -> Path`
  - `check_for_updates(cached_version: str) -> Optional[str]`
  - `get_cached_template(version: str) -> Optional[Path]`
- **Implementation:** Uses `git clone` command via subprocess
- **Cache Location:** `~/.abathur/cache/templates/`
- **Cache TTL:** 7 days (configurable)

#### 2.4.5 Logger (structlog)
- **Responsibility:** Structured JSON logging with rotation
- **Key Features:**
  - Structured fields: timestamp, level, component, event_type, context, message
  - Never logs secrets (API keys, tokens)
  - Log rotation: daily, 30-day retention
  - Async I/O: non-blocking writes
- **Output:** `.abathur/logs/abathur.log`

---

## 3. Directory & File Structure

### 3.1 Overview

Abathur uses two top-level directories:
- `.claude/` - Shared with Claude Code (agents, MCP config)
- `.abathur/` - Abathur-specific (orchestration, queue, logs)

### 3.2 Complete Structure

```
project-root/
├── .claude/                           # Shared with Claude Code
│   ├── agents/                        # Agent definitions (YAML)
│   │   ├── frontend-specialist.yaml
│   │   ├── backend-specialist.yaml
│   │   ├── test-engineer.yaml
│   │   └── documentation-writer.yaml
│   └── mcp.json                       # MCP server configurations
│
├── .abathur/                          # Abathur-specific directory
│   ├── config.yaml                    # Orchestration configuration
│   ├── local.yaml                     # Local overrides (gitignored)
│   ├── metadata.json                  # Template version metadata
│   ├── abathur.db                     # SQLite database (queue, state, audit)
│   ├── logs/                          # Log files
│   │   ├── abathur.log                # Current log
│   │   ├── abathur.log.2025-10-08     # Rotated logs
│   │   └── abathur.log.2025-10-07
│   └── templates/                     # Task templates (optional)
│       └── feature-implementation.yaml
│
├── .env                               # API keys (gitignored)
└── .gitignore                         # Ignore .abathur/local.yaml, .env, logs, *.db
```

### 3.3 Key Files

**`.claude/agents/*.yaml`** (Agent Definitions - Shared with Claude Code)
```yaml
name: frontend-specialist
specialization: React/TypeScript frontend development
model: claude-sonnet-4-20250514
system_prompt: |
  You are a frontend specialist focusing on React and TypeScript.
  You write clean, testable, accessible UI components.
tools:
  - name: read_file
  - name: write_file
  - name: execute_command
resource_limits:
  max_tokens: 8000
  temperature: 0.7
```

**`.claude/mcp.json`** (MCP Server Config - Shared with Claude Code)
```json
{
  "mcpServers": {
    "github": {
      "command": "npx",
      "args": ["-y", "@modelcontextprotocol/server-github"],
      "env": {
        "GITHUB_PERSONAL_ACCESS_TOKEN": "${GITHUB_TOKEN}"
      }
    },
    "filesystem": {
      "command": "npx",
      "args": ["-y", "@modelcontextprotocol/server-filesystem", "/path/to/project"]
    }
  }
}
```

**`.abathur/config.yaml`** (Orchestration Config)
```yaml
system:
  version: "1.0.0"
  log_level: INFO

queue:
  max_size: 1000
  default_priority: 5
  retry_attempts: 3
  retry_backoff_initial: 10s
  retry_backoff_max: 5m

swarm:
  max_concurrent_agents: 10
  agent_spawn_timeout: 5s
  agent_idle_timeout: 5m
  hierarchical_depth_limit: 3

loop:
  max_iterations: 10
  default_timeout: 1h
  checkpoint_interval: 1  # Checkpoint after each iteration

resources:
  max_memory_per_agent: 512MB
  max_total_memory: 4GB
  adaptive_cpu: true

monitoring:
  log_rotation_days: 30
  audit_retention_days: 90
  metrics_enabled: true
```

**`.abathur/metadata.json`** (Template Version Tracking)
```json
{
  "template_repo": "odgrim/abathur-claude-template",
  "template_version": "v1.2.0",
  "template_commit": "abc123def456",
  "installed_at": "2025-10-09T10:30:00Z",
  "cli_version": "1.0.0"
}
```

---

## 4. Data Architecture

### 4.1 SQLite Database Schema (`.abathur/abathur.db`)

#### Table: `tasks`
```sql
CREATE TABLE tasks (
    id TEXT PRIMARY KEY,                    -- UUID
    template_name TEXT NOT NULL,
    priority INTEGER NOT NULL DEFAULT 5,    -- 0-10 scale
    status TEXT NOT NULL,                   -- pending, running, completed, failed, cancelled
    input_data TEXT NOT NULL,               -- JSON blob
    result_data TEXT,                       -- JSON blob (populated on completion)
    error_message TEXT,
    retry_count INTEGER DEFAULT 0,
    max_retries INTEGER DEFAULT 3,
    submitted_at TIMESTAMP NOT NULL,
    started_at TIMESTAMP,
    completed_at TIMESTAMP,
    created_by TEXT,                        -- User or parent task ID
    parent_task_id TEXT,                    -- For hierarchical spawning
    dependencies TEXT,                      -- JSON array of task IDs

    FOREIGN KEY (parent_task_id) REFERENCES tasks(id)
);

CREATE INDEX idx_tasks_status_priority ON tasks(status, priority DESC, submitted_at ASC);
CREATE INDEX idx_tasks_submitted_at ON tasks(submitted_at);
CREATE INDEX idx_tasks_parent ON tasks(parent_task_id);
```

#### Table: `agents`
```sql
CREATE TABLE agents (
    id TEXT PRIMARY KEY,                    -- UUID
    name TEXT NOT NULL,
    specialization TEXT NOT NULL,
    task_id TEXT NOT NULL,
    state TEXT NOT NULL,                    -- spawning, idle, busy, terminating, terminated
    model TEXT NOT NULL,
    spawned_at TIMESTAMP NOT NULL,
    terminated_at TIMESTAMP,
    resource_usage TEXT,                    -- JSON blob (memory, tokens, execution time)

    FOREIGN KEY (task_id) REFERENCES tasks(id)
);

CREATE INDEX idx_agents_task ON agents(task_id);
CREATE INDEX idx_agents_state ON agents(state);
```

#### Table: `state`
```sql
CREATE TABLE state (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    task_id TEXT NOT NULL,
    key TEXT NOT NULL,
    value TEXT NOT NULL,                    -- JSON blob
    created_at TIMESTAMP NOT NULL,
    updated_at TIMESTAMP NOT NULL,

    UNIQUE(task_id, key),
    FOREIGN KEY (task_id) REFERENCES tasks(id)
);

CREATE INDEX idx_state_task_key ON state(task_id, key);
```

#### Table: `audit`
```sql
CREATE TABLE audit (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    timestamp TIMESTAMP NOT NULL,
    agent_id TEXT,
    task_id TEXT NOT NULL,
    action_type TEXT NOT NULL,              -- spawn, execute, read_file, write_file, etc.
    action_data TEXT,                       -- JSON blob
    result TEXT,                            -- success, failure, error details

    FOREIGN KEY (agent_id) REFERENCES agents(id),
    FOREIGN KEY (task_id) REFERENCES tasks(id)
);

CREATE INDEX idx_audit_task ON audit(task_id, timestamp DESC);
CREATE INDEX idx_audit_agent ON audit(agent_id, timestamp DESC);
CREATE INDEX idx_audit_timestamp ON audit(timestamp DESC);
```

#### Table: `metrics`
```sql
CREATE TABLE metrics (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    timestamp TIMESTAMP NOT NULL,
    metric_name TEXT NOT NULL,
    metric_value REAL NOT NULL,
    labels TEXT,                            -- JSON blob (key-value pairs)

    CHECK(metric_value >= 0)
);

CREATE INDEX idx_metrics_name_timestamp ON metrics(metric_name, timestamp DESC);
```

### 4.2 Database Configuration

**SQLite Settings:**
- **WAL Mode:** `PRAGMA journal_mode=WAL;` - Enables concurrent reads during writes
- **Synchronous:** `PRAGMA synchronous=NORMAL;` - Balance durability and performance
- **Foreign Keys:** `PRAGMA foreign_keys=ON;` - Enforce referential integrity
- **Connection Pool:** 5 connections (1 writer, 4 readers)
- **Busy Timeout:** 5000ms - Wait for locks instead of immediate failure

**Transaction Strategy:**
- All state changes wrapped in transactions
- Task submission: Single transaction (submit + initial audit entry)
- Agent lifecycle: Transaction per state transition
- Checkpoint: Transaction includes loop state + audit entry

---

## 5. Concurrency Architecture

### 5.1 Asyncio-Based Execution Model

**Event Loop Strategy:**
```python
# Pseudocode
async def main():
    async with AsyncExitStack() as stack:
        # Initialize resources
        db_pool = await stack.enter_async_context(DatabasePool())
        claude_client = await stack.enter_async_context(ClaudeClient())

        # Create semaphore for agent concurrency control
        agent_semaphore = asyncio.Semaphore(config.max_concurrent_agents)

        # Spawn task coordinator and swarm orchestrator
        task_queue = asyncio.Queue()
        coordinator_task = asyncio.create_task(
            task_coordinator(task_queue, db_pool)
        )
        orchestrator_task = asyncio.create_task(
            swarm_orchestrator(task_queue, claude_client, agent_semaphore)
        )

        # Wait for completion or shutdown signal
        await asyncio.gather(coordinator_task, orchestrator_task)
```

### 5.2 Agent Spawn Pattern

**Lifecycle:**
1. **Spawning** - Create agent instance, load config, initialize Claude client
2. **Idle** - Agent ready, waiting for task assignment
3. **Busy** - Agent executing task
4. **Terminating** - Agent cleaning up resources
5. **Terminated** - Agent destroyed, resources released

**Spawn Implementation:**
```python
async def spawn_agent(config: AgentConfig, semaphore: asyncio.Semaphore) -> Agent:
    async with semaphore:  # Respect concurrency limit
        agent = Agent(
            id=uuid4(),
            config=config,
            claude_client=ClaudeClient(model=config.model)
        )
        await agent.initialize()  # Load MCP servers, validate tools
        return agent
```

**Agent Pool Management:**
```python
async def agent_pool_manager(max_agents: int):
    agents: Dict[UUID, Agent] = {}
    semaphore = asyncio.Semaphore(max_agents)

    async def spawn_and_track(config: AgentConfig) -> Agent:
        agent = await spawn_agent(config, semaphore)
        agents[agent.id] = agent
        return agent

    async def terminate_agent(agent_id: UUID):
        agent = agents.pop(agent_id)
        await agent.terminate()
        # Semaphore automatically released
```

### 5.3 Task Distribution

**Round-Robin with Specialization Matching:**
```python
async def distribute_tasks(tasks: List[Task], agents: List[Agent]):
    # Group tasks by required specialization
    task_groups = group_by_specialization(tasks)

    for specialization, task_group in task_groups.items():
        # Find agents matching specialization
        matching_agents = [a for a in agents if a.specialization == specialization]

        # Distribute round-robin
        for i, task in enumerate(task_group):
            agent = matching_agents[i % len(matching_agents)]
            asyncio.create_task(execute_task(agent, task))
```

### 5.4 Failure Handling

**Agent Failure Detection:**
- Heartbeat mechanism: Agent reports progress every 30s
- Timeout detection: No heartbeat for 5 minutes → mark as stalled
- Exception handling: Catch and log all agent exceptions

**Failure Recovery:**
```python
async def execute_with_retry(agent: Agent, task: Task, max_retries: int = 3):
    for attempt in range(max_retries):
        try:
            result = await agent.execute(task)
            return result
        except TransientError as e:
            if attempt < max_retries - 1:
                backoff = min(10 * (2 ** attempt), 300)  # Exponential backoff
                await asyncio.sleep(backoff)
            else:
                await move_to_dlq(task, error=e)
                raise
        except PermanentError as e:
            await move_to_dlq(task, error=e)
            raise
```

### 5.5 Resource Limit Enforcement

**Memory Monitoring:**
```python
import psutil

async def monitor_resources(agents: List[Agent], max_memory: int):
    while True:
        total_memory = sum(agent.get_memory_usage() for agent in agents)

        if total_memory > max_memory * 0.8:  # 80% threshold
            logger.warning("Approaching memory limit", total=total_memory, limit=max_memory)
            # Throttle new agent spawning
            await asyncio.sleep(10)

        await asyncio.sleep(5)  # Check every 5 seconds
```

---

## 6. Integration Points

### 6.1 Claude SDK Integration

**Anthropic Python SDK Wrapper:**
- Version: Latest stable (pinned in `pyproject.toml`)
- Authentication: `ANTHROPIC_API_KEY` from environment or keychain
- Rate Limiting: Built-in token bucket algorithm
- Streaming: Support for streaming responses (`stream_response()` method)

**Error Classification:**
- `TransientError` - Retry (rate limits, network failures)
- `PermanentError` - Don't retry (invalid API key, malformed request)
- `ResourceExhaustedError` - Backoff and retry (token quota exceeded)

### 6.2 MCP Server Integration

**Configuration Loading:**
1. Read `.claude/mcp.json` or `.mcp.json` during initialization (Claude Code standard format)
2. Parse server definitions (command, args, env)
3. Configure MCP servers in Claude Agent SDK options
4. SDK manages server lifecycle automatically

**Configuration Parsing:**
```python
async def load_mcp_config(project_root: Path) -> Dict[str, Any]:
    # Try project-scoped .mcp.json first, then .claude/mcp.json
    mcp_paths = [
        project_root / ".mcp.json",
        project_root / ".claude" / "mcp.json"
    ]

    for mcp_path in mcp_paths:
        if mcp_path.exists():
            config = json.loads(mcp_path.read_text())
            return config.get("mcpServers", {})

    return {}

async def create_claude_client_with_mcp(mcp_config: Dict[str, Any]) -> ClaudeClient:
    # Convert .mcp.json format to SDK format
    mcp_servers = {}
    for name, server_config in mcp_config.items():
        mcp_servers[name] = {
            "type": "stdio",
            "command": server_config["command"],
            "args": server_config.get("args", []),
            "env": server_config.get("env", {})
        }

    # Configure Claude Agent SDK with MCP servers
    options = ClaudeAgentOptions(mcp_servers=mcp_servers)
    return ClaudeClient(options)
```

**Agent-to-MCP Binding:**
- MCP servers configured from standard .claude/mcp.json format
- Claude Agent SDK manages server lifecycle (start, health check, stop)
- Shared servers across agents (configured once in SDK options)

### 6.3 Template Cloning (Git)

**Git Clone Implementation:**
- Uses `git clone` command via subprocess (no external Python dependencies)
- Authentication: Uses user's existing git credentials (SSH keys, credential helper)
- Version support: Clone specific tags/branches with `--branch` flag
- Shallow clone: `--depth 1` for faster downloads

**Clone Implementation:**
```python
async def clone_template(repo_url: str, version: str, dest: Path) -> Path:
    """Clone template repository using git command."""
    cmd = [
        "git", "clone",
        "--depth", "1",
        "--branch", version,
        repo_url,
        str(dest)
    ]

    result = await asyncio.create_subprocess_exec(
        *cmd,
        stdout=asyncio.subprocess.PIPE,
        stderr=asyncio.subprocess.PIPE
    )

    stdout, stderr = await result.communicate()

    if result.returncode != 0:
        raise TemplateCloneError(f"Failed to clone template: {stderr.decode()}")

    # Remove .git directory to avoid nested repos
    shutil.rmtree(dest / ".git", ignore_errors=True)

    return dest

def validate_template(template_path: Path) -> ValidationResult:
    required_files = [
        ".abathur/config.yaml",
        ".claude/agents/",
    ]
    optional_files = [
        ".claude/mcp.json",
        ".abathur/templates/",
    ]

    errors = []
    for file in required_files:
        if not (template_path / file).exists():
            errors.append(f"Missing required file: {file}")

    return ValidationResult(valid=len(errors) == 0, errors=errors)
```

### 6.4 Keychain Integration

**Platform-Specific:**
- macOS: `keyring` library → macOS Keychain
- Windows: `keyring` library → Windows Credential Manager
- Linux: `keyring` library → Secret Service API (libsecret)

**Fallback Strategy:**
```python
def get_api_key() -> str:
    # 1. Environment variable (highest priority)
    if key := os.getenv("ANTHROPIC_API_KEY"):
        return key

    # 2. System keychain
    try:
        if key := keyring.get_password("abathur", "anthropic_api_key"):
            return key
    except KeyringError:
        pass

    # 3. .env file (local, gitignored)
    if Path(".env").exists():
        load_dotenv()
        if key := os.getenv("ANTHROPIC_API_KEY"):
            return key

    raise APIKeyNotFoundError("API key not found in environment, keychain, or .env file")
```

---

## 7. Performance Validation

### 7.1 Feasibility Analysis

**NFR-PERF-001: Queue Operations <100ms (p95)**
- **Validated:** SQLite with WAL mode and indexed queries easily achieves <10ms for submit/list/cancel
- **Mitigation:** Connection pooling, prepared statements, batch operations
- **Confidence:** High - SQLite optimized for this workload

**NFR-PERF-002: Agent Spawn <5s (p95)**
- **Validated:** Claude API latency dominates (1-3s for first request), initialization overhead <1s
- **Mitigation:** Warm connection pool, pre-load MCP servers, parallel spawn
- **Confidence:** High - Async spawn enables <5s target even with API variance

**NFR-PERF-003: Status Queries <50ms (p95)**
- **Validated:** Simple SELECT queries on indexed tables easily meet target
- **Mitigation:** Materialized views for complex queries, in-memory status cache
- **Confidence:** High - Database indexes provide <10ms query time

**NFR-PERF-004: 10 Concurrent Agents <10% Degradation**
- **Validated:** Asyncio handles 10+ concurrent I/O-bound tasks efficiently
- **Mitigation:** Semaphore-based concurrency control, resource monitoring
- **Confidence:** High - Python asyncio designed for this pattern

**NFR-PERF-005: Queue Scalability to 10,000 Tasks**
- **Validated:** SQLite handles 10k+ rows with proper indexing (<50ms queries)
- **Mitigation:** Pagination for list operations, index optimization
- **Confidence:** Medium-High - May need query optimization at scale

**NFR-REL-001: >99.9% Task Persistence**
- **Validated:** SQLite ACID transactions + WAL mode provide durability
- **Mitigation:** Fsync on critical operations, write-ahead logging
- **Confidence:** High - SQLite battle-tested for durability

### 7.2 Performance Budget Allocation

| Component | Latency Budget | Memory Budget |
|-----------|----------------|---------------|
| CLI Parsing | 50ms | 20MB |
| Config Loading | 100ms | 30MB |
| Queue Operation | 50ms | 50MB |
| Agent Spawn | 4000ms | 512MB (per agent) |
| Status Query | 30ms | 50MB |
| Logging (async) | Non-blocking | 50MB |
| System Overhead | - | 200MB |
| **Total** | - | 200MB + (512MB × agents) |

### 7.3 Optimization Strategy

**Phase 1 (MVP):** Focus on correctness, basic performance
- Implement core functionality with straightforward approaches
- Profile to identify actual bottlenecks (not premature optimization)

**Phase 2 (Optimization):** Targeted improvements based on profiling
- Database query optimization (explain analyze)
- Connection pooling and caching
- Async I/O for all blocking operations

**Phase 3 (Scale):** Advanced patterns if needed
- Redis fallback for distributed scenarios
- Read replicas for query scaling
- Agent pool pre-warming

---

## 8. Technology Stack

### 8.1 Core Technologies

**Language & Runtime:**
- **Python 3.10+** - Modern type hints, pattern matching, async/await
- **Rationale:** Claude SDK is Python-native, rich ecosystem for CLI tools, excellent async support

**CLI Framework:**
- **Typer 0.9+** - Type-safe CLI built on Click
- **Rationale:** Automatic help generation, type validation, excellent developer experience

**Persistence:**
- **SQLite 3.35+** - ACID-compliant embedded database
- **Rationale:** Zero external dependencies, ACID guarantees, WAL mode for concurrency, proven durability

**Async Runtime:**
- **asyncio** - Python standard library async framework
- **Rationale:** Native support, mature ecosystem, efficient I/O multiplexing

### 8.2 Dependencies

**Core Dependencies:**
```toml
[tool.poetry.dependencies]
python = "^3.10"
anthropic = "^0.18.0"              # Claude Agent SDK
typer = "^0.9.0"                   # CLI framework
pydantic = "^2.5.0"                # Config validation
python-dotenv = "^1.0.0"           # .env file loading
keyring = "^24.3.0"                # Keychain integration
structlog = "^24.1.0"              # Structured logging
aiosqlite = "^0.19.0"              # Async SQLite
psutil = "^5.9.0"                  # Resource monitoring
pyyaml = "^6.0.1"                  # YAML parsing
```

**Development Dependencies:**
```toml
[tool.poetry.group.dev.dependencies]
pytest = "^7.4.0"                  # Test framework
pytest-asyncio = "^0.21.0"         # Async test support
pytest-cov = "^4.1.0"              # Coverage reporting
mypy = "^1.7.0"                    # Type checking
ruff = "^0.1.9"                    # Linting and formatting
black = "^23.12.0"                 # Code formatting
pre-commit = "^3.6.0"              # Git hooks
```

### 8.3 Technology Alternatives Considered

| Decision | Chosen | Alternative | Rationale for Choice |
|----------|--------|-------------|---------------------|
| Queue Backend | SQLite | Redis, PostgreSQL, RabbitMQ | Zero external dependencies, sufficient for single-node, ACID guarantees |
| CLI Framework | Typer | Click, argparse, Fire | Type-safe, modern, excellent DX, auto-generated help |
| Config Format | YAML | TOML, JSON, INI | Human-readable, comments support, widely adopted |
| Logging | structlog | stdlib logging, loguru | Structured JSON output, excellent filtering, battle-tested |
| Async Framework | asyncio | Trio, Curio | Standard library, mature ecosystem, Claude SDK compatibility |
| Dependency Mgmt | Poetry | pip-tools, Pipenv | Modern, excellent lockfile support, packaging integration |

### 8.4 Rationale Summary

**Why SQLite?**
- ACID transactions ensure >99.9% reliability (NFR-REL-001)
- WAL mode enables concurrent reads during writes
- Zero external infrastructure reduces installation friction (NFR-USE-001: <5min to first task)
- Sufficient for 10,000+ tasks (NFR-PERF-005)
- Future migration path to Redis if distributed scenarios emerge

**Why Typer?**
- Type-safe CLI reduces runtime errors
- Automatic help generation improves discoverability (NFR-USE-002: 80% tasks without docs)
- Excellent error messages (NFR-USE-003: 90% include suggestions)
- Fast startup <500ms (NFR-PERF-007)

**Why asyncio?**
- Native Python support, no external dependencies
- Efficient I/O multiplexing for concurrent agents (NFR-PERF-004: 10+ agents)
- Claude SDK compatible
- Well-understood patterns and extensive documentation

**Why Python 3.10+?**
- Claude SDK is Python-native
- Modern type hints enable mypy type checking (NFR-MAINT-002)
- Pattern matching improves code clarity
- Rich ecosystem for CLI, async, and data processing

---

## Summary

This architecture document defines a **clean, layered architecture** for Abathur that separates CLI, application logic, domain models, and infrastructure. The **asyncio-based concurrency model** enables efficient multi-agent coordination, while **SQLite persistence** ensures production-grade reliability with zero external dependencies.

**Key Architectural Decisions:**
1. **Clean separation:** CLI → Application Services → Domain → Infrastructure
2. **Async-first:** asyncio for concurrent agent management (10+ agents)
3. **Persistence-first:** SQLite ACID transactions for >99.9% reliability
4. **Resource-aware:** Configurable limits with adaptive scaling
5. **Observability-first:** Structured logging and audit trail as core features

**Performance Validation:**
- All NFR performance targets validated as feasible with chosen stack
- SQLite + asyncio + indexing support <100ms queue ops, <5s agent spawn, <50ms status queries
- Performance budgets allocated across components

**Directory Structure:**
- `.claude/` shared with Claude Code (agents, MCP config)
- `.abathur/` Abathur-specific (orchestration, queue, logs)
- Clean separation enables coexistence without conflicts

**Next Steps:** Proceed to system design phase with detailed orchestration patterns, API specifications, and implementation roadmap.

---

**Document Status:** Complete - Ready for System Design Phase
**Validation:** All conditional items addressed (performance feasibility, directory structure, MCP integration)
**Next Phase:** System Design and API Specification (prd-system-design-specialist, prd-api-cli-specialist)
