# System Architecture

This document explains Abathur's architecture, design decisions, and technical rationale.

## Overview

Abathur is an agentic swarm orchestration system built in Rust following **Clean Architecture** (also known as Hexagonal Architecture) principles. The system manages AI agent lifecycles, task scheduling with dependencies, hierarchical memory, and Model Context Protocol (MCP) integration.

**Core Capabilities:**

- **Task Queue Management**: Priority-based scheduling with dependency resolution
- **Concurrent Agent Swarms**: Multiple AI agents executing tasks simultaneously
- **Hierarchical Memory**: Semantic, episodic, and procedural memory with namespace organization
- **MCP Integration**: Bidirectional communication with MCP servers via HTTP
- **Iterative Refinement**: Multi-strategy convergence loops with checkpointing
- **Observability**: Structured logging, audit trails, and resource monitoring

## System Architecture

### Layered Architecture

Abathur follows a strict layered architecture with clear separation of concerns:

```mermaid
graph TB
    subgraph "Presentation Layer"
        CLI[CLI Commands<br/>clap + comfy-table]
        TUI[Terminal Output<br/>Progress bars & trees]
    end

    subgraph "Application Layer"
        SwarmOrch[Swarm Orchestrator<br/>Concurrent agent management]
        TaskCoord[Task Coordinator<br/>Queue & priority management]
        LoopExec[Loop Executor<br/>Iterative refinement]
        ResourceMon[Resource Monitor<br/>System metrics]
    end

    subgraph "Service Layer"
        TaskQueueSvc[Task Queue Service<br/>CRUD + execution plan]
        MemorySvc[Memory Service<br/>Hierarchical storage]
        SessionSvc[Session Service<br/>Agent sessions]
        DepResolver[Dependency Resolver<br/>Task graph validation]
    end

    subgraph "Domain Layer"
        Task[Task<br/>Domain Model]
        Agent[Agent<br/>Domain Model]
        Memory[Memory<br/>Domain Model]
        Ports[Port Traits<br/>Abstractions]
    end

    subgraph "Infrastructure Layer"
        subgraph "Data Access"
            TaskRepo[Task Repository<br/>SQLite async]
            MemoryRepo[Memory Repository<br/>SQLite async]
            AgentRepo[Agent Repository<br/>SQLite async]
        end

        subgraph "External Integration"
            LLMSub[LLM Substrates<br/>Anthropic API/Claude Code]
            MCPClient[MCP HTTP Client<br/>JSON-RPC over HTTP]
            ConfigLoader[Config Loader<br/>Hierarchical YAML]
        end

        subgraph "Cross-Cutting"
            Logger[Structured Logger<br/>tracing + audit]
            RateLimiter[Rate Limiter<br/>Token bucket]
        end
    end

    CLI --> SwarmOrch
    CLI --> TaskCoord
    CLI --> LoopExec
    TUI --> CLI

    SwarmOrch --> TaskQueueSvc
    SwarmOrch --> MemorySvc
    SwarmOrch --> ResourceMon
    TaskCoord --> TaskQueueSvc
    TaskCoord --> DepResolver
    LoopExec --> TaskQueueSvc
    LoopExec --> MemorySvc

    TaskQueueSvc --> Ports
    MemorySvc --> Ports
    SessionSvc --> Ports
    DepResolver --> Ports

    Ports --> Task
    Ports --> Agent
    Ports --> Memory

    Ports -.->|implements| TaskRepo
    Ports -.->|implements| MemoryRepo
    Ports -.->|implements| AgentRepo
    Ports -.->|implements| LLMSub
    Ports -.->|implements| MCPClient

    TaskRepo --> Logger
    MemoryRepo --> Logger
    AgentRepo --> Logger
    LLMSub --> RateLimiter
    MCPClient --> Logger
    ConfigLoader --> Logger

    style CLI fill:#E8F5E9
    style Domain fill:#FFF9C4
    style Infrastructure fill:#E1F5FE
    style Application fill:#F3E5F5
    style Service fill:#FCE4EC
```

**Layer Responsibilities:**

- **Presentation Layer**: User interaction, command parsing, terminal output formatting
- **Application Layer**: Use case orchestration, workflow management, concurrency control
- **Service Layer**: Business logic coordination, transaction boundaries, domain orchestration
- **Domain Layer**: Pure business logic, domain models, port abstractions (no external dependencies)
- **Infrastructure Layer**: External integrations, database access, configuration, logging

### Dependency Flow

Dependencies flow **inward**: outer layers depend on inner layers, never the reverse. The Domain layer has zero external dependencies. Infrastructure implements Domain ports.

## Component Architecture

### Core Components

```mermaid
graph LR
    subgraph "Task Management"
        TaskQueue[Task Queue<br/>Priority heap + dependencies]
        TaskRepo[Task Repository<br/>CRUD + execution plan]
        DepResolver[Dependency Resolver<br/>Cycle detection]
    end

    subgraph "Agent Management"
        SwarmOrch[Swarm Orchestrator<br/>Concurrent execution]
        AgentExec[Agent Executor<br/>Single agent lifecycle]
        SessionMgr[Session Manager<br/>Agent sessions]
    end

    subgraph "Memory System"
        MemoryService[Memory Service<br/>Hierarchical namespaces]
        MemoryRepo[Memory Repository<br/>Semantic/Episodic/Procedural]
    end

    subgraph "External Integration"
        LLMSubstrate[LLM Substrate<br/>Anthropic API/Claude Code]
        MCPClient[MCP Client<br/>HTTP JSON-RPC]
        MCPHandlers[MCP Handlers<br/>Task & Memory endpoints]
    end

    subgraph "Observability"
        Logger[Structured Logger<br/>Audit trail]
        ResourceMon[Resource Monitor<br/>System metrics]
    end

    TaskQueue --> TaskRepo
    TaskQueue --> DepResolver
    SwarmOrch --> TaskQueue
    SwarmOrch --> AgentExec
    SwarmOrch --> ResourceMon
    AgentExec --> SessionMgr
    AgentExec --> LLMSubstrate
    AgentExec --> MemoryService
    MemoryService --> MemoryRepo
    MCPClient --> MCPHandlers
    MCPHandlers --> TaskQueue
    MCPHandlers --> MemoryService
    TaskRepo --> Logger
    MemoryRepo --> Logger
    AgentExec --> Logger

    style TaskQueue fill:#FFEBEE
    style SwarmOrch fill:#E8F5E9
    style MemoryService fill:#FFF9C4
    style LLMSubstrate fill:#E1F5FE
    style Logger fill:#F3E5F5
```

## Data Flow

### Task Execution Flow

This sequence diagram shows how a task flows through the system from submission to completion:

```mermaid
sequenceDiagram
    participant User
    participant CLI
    participant TaskCoord as Task Coordinator
    participant TaskQueue as Task Queue Service
    participant DepResolver as Dependency Resolver
    participant DB as Task Repository
    participant SwarmOrch as Swarm Orchestrator
    participant AgentExec as Agent Executor
    participant LLMSub as LLM Substrate
    participant Memory as Memory Service

    User->>CLI: abathur task submit
    CLI->>TaskCoord: create_task(description, dependencies)
    TaskCoord->>DepResolver: validate_dependencies(task)
    DepResolver->>DB: fetch_dependency_tasks()
    DB-->>DepResolver: dependency_tasks
    DepResolver->>DepResolver: check_cycles()
    DepResolver-->>TaskCoord: ValidationResult::Valid
    TaskCoord->>TaskQueue: enqueue_task(task)
    TaskQueue->>DB: insert_task()
    DB-->>TaskQueue: task_id
    TaskQueue-->>CLI: TaskCreated(task_id)
    CLI-->>User: Task created: {task_id}

    User->>CLI: abathur swarm start
    CLI->>SwarmOrch: start_swarm(max_agents)

    loop Swarm Execution
        SwarmOrch->>TaskQueue: fetch_ready_tasks()
        TaskQueue->>DB: query_ready_tasks()
        DB-->>TaskQueue: ready_tasks
        TaskQueue-->>SwarmOrch: ready_tasks

        par Concurrent Agent Execution
            SwarmOrch->>AgentExec: spawn_agent(task)
            AgentExec->>Memory: get_context(namespace)
            Memory-->>AgentExec: context
            AgentExec->>LLMSub: generate_response(prompt, context)
            LLMSub-->>AgentExec: response
            AgentExec->>Memory: store_results(namespace, results)
            AgentExec->>TaskQueue: complete_task(task_id, results)
            TaskQueue->>DB: update_task_status(Completed)
            AgentExec-->>SwarmOrch: ExecutionResult
        end

        SwarmOrch->>SwarmOrch: check_resource_limits()
    end

    SwarmOrch-->>CLI: SwarmComplete(stats)
    CLI-->>User: All tasks completed
```

**Flow Stages:**

1. **Task Submission**: User submits task via CLI
2. **Validation**: Dependencies validated, cycles detected
3. **Queueing**: Task inserted into priority queue
4. **Scheduling**: Swarm orchestrator fetches ready tasks
5. **Execution**: Agents spawn concurrently, execute with LLM substrate
6. **Memory**: Context fetched/stored in hierarchical memory
7. **Completion**: Task status updated, results persisted

### Memory Access Flow

```mermaid
sequenceDiagram
    participant Agent
    participant MemorySvc as Memory Service
    participant MemoryRepo as Memory Repository
    participant DB as SQLite Database

    Agent->>MemorySvc: get_memory(namespace, key)
    MemorySvc->>MemorySvc: validate_namespace(namespace)
    MemorySvc->>MemoryRepo: fetch(namespace, key)
    MemoryRepo->>DB: SELECT * FROM memory WHERE namespace=? AND key=?
    DB-->>MemoryRepo: memory_row
    MemoryRepo->>MemoryRepo: deserialize_value()
    MemoryRepo-->>MemorySvc: Memory { value, metadata }
    MemorySvc-->>Agent: Memory

    Agent->>MemorySvc: add_memory(namespace, key, value, type)
    MemorySvc->>MemorySvc: validate_namespace(namespace)
    MemorySvc->>MemoryRepo: insert(namespace, key, value, type)
    MemoryRepo->>DB: INSERT INTO memory (namespace, key, value, type, created_at)
    DB-->>MemoryRepo: memory_id
    MemoryRepo-->>MemorySvc: memory_id
    MemorySvc-->>Agent: Success

    Agent->>MemorySvc: search_memory(namespace_prefix, type_filter)
    MemorySvc->>MemoryRepo: search(namespace_prefix, type_filter)
    MemoryRepo->>DB: SELECT * FROM memory WHERE namespace LIKE ? AND type=?
    DB-->>MemoryRepo: memory_rows
    MemoryRepo-->>MemorySvc: Vec<Memory>
    MemorySvc-->>Agent: Vec<Memory>
```

**Memory Types:**

- **Semantic**: Facts, knowledge, concepts (e.g., API specifications, architecture docs)
- **Episodic**: Events, experiences (e.g., task execution history, agent interactions)
- **Procedural**: How-to knowledge (e.g., workflows, best practices, patterns)

**Namespace Hierarchy:**

```
project/
├── task/{task_id}/
│   ├── context
│   ├── results
│   └── history
├── agent/{agent_type}/
│   ├── specialization
│   └── patterns
└── swarm/{swarm_id}/
    ├── coordination
    └── metrics
```

### MCP Integration Flow

```mermaid
sequenceDiagram
    participant External as External Client<br/>(Claude Code)
    participant MCPServer as MCP HTTP Server<br/>:8080
    participant TaskHandler as Task Handler
    participant MemoryHandler as Memory Handler
    participant TaskQueue as Task Queue Service
    participant Memory as Memory Service
    participant DB as SQLite Database

    External->>MCPServer: POST /tools/call<br/>{"name": "task_enqueue", "arguments": {...}}
    MCPServer->>TaskHandler: handle_task_enqueue(args)
    TaskHandler->>TaskQueue: enqueue_task(summary, description, agent_type)
    TaskQueue->>DB: INSERT INTO tasks
    DB-->>TaskQueue: task_id
    TaskQueue-->>TaskHandler: task_id
    TaskHandler-->>MCPServer: ToolResult { task_id }
    MCPServer-->>External: HTTP 200 { "task_id": "..." }

    External->>MCPServer: POST /tools/call<br/>{"name": "memory_add", "arguments": {...}}
    MCPServer->>MemoryHandler: handle_memory_add(namespace, key, value)
    MemoryHandler->>Memory: add_memory(namespace, key, value, type)
    Memory->>DB: INSERT INTO memory
    DB-->>Memory: memory_id
    Memory-->>MemoryHandler: Success
    MemoryHandler-->>MCPServer: ToolResult { success: true }
    MCPServer-->>External: HTTP 200 { "success": true }

    External->>MCPServer: POST /tools/call<br/>{"name": "task_list", "arguments": {...}}
    MCPServer->>TaskHandler: handle_task_list(filters)
    TaskHandler->>TaskQueue: list_tasks(filters)
    TaskQueue->>DB: SELECT * FROM tasks WHERE ...
    DB-->>TaskQueue: tasks
    TaskQueue-->>TaskHandler: Vec<Task>
    TaskHandler-->>MCPServer: ToolResult { tasks }
    MCPServer-->>External: HTTP 200 { "tasks": [...] }
```

**MCP Endpoints:**

- `task_enqueue`: Submit new task
- `task_get`: Fetch task by ID
- `task_list`: List tasks with filters
- `task_cancel`: Cancel task and dependents
- `task_queue_status`: Get queue statistics
- `task_execution_plan`: Get dependency execution order
- `memory_add`: Store memory entry
- `memory_get`: Fetch memory by namespace/key
- `memory_search`: Search memories by namespace prefix
- `memory_update`: Update existing memory
- `memory_delete`: Soft delete memory

## Technology Stack

### Language & Runtime

**Rust 2024 Edition**

*Why Rust?*

- **Type Safety**: Compile-time guarantees prevent entire classes of bugs
- **Concurrency**: Fearless concurrency with ownership system prevents data races
- **Performance**: Zero-cost abstractions, minimal runtime overhead
- **Async Runtime**: Tokio provides high-performance async/await
- **Ecosystem**: Excellent libraries for CLI, DB, HTTP, logging

*Trade-offs:*

- ✅ Memory safety without garbage collection
- ✅ Excellent performance for concurrent agent execution
- ✅ Strong type system catches errors at compile time
- ❌ Steeper learning curve than Python
- ❌ Longer compilation times
- ❌ Smaller AI/ML ecosystem compared to Python

### Database

**SQLite with WAL mode (via sqlx)**

*Why SQLite?*

- **Embedded**: No separate database server required
- **WAL Mode**: Write-Ahead Logging enables concurrent reads during writes
- **ACID**: Full transactional support
- **Zero Configuration**: Works out of the box
- **Async Support**: sqlx provides compile-time checked queries

*Trade-offs:*

- ✅ Zero operational overhead
- ✅ Fast local access
- ✅ File-based, easy backup
- ✅ Excellent for single-machine workloads
- ❌ Not suitable for distributed systems (future consideration)
- ❌ Limited horizontal scalability

**Schema Design:**

```sql
CREATE TABLE tasks (
    id TEXT PRIMARY KEY,
    summary TEXT,
    description TEXT NOT NULL,
    status TEXT NOT NULL,
    agent_type TEXT NOT NULL,
    parent_task_id TEXT,
    base_priority INTEGER NOT NULL,
    effective_priority REAL NOT NULL,
    dependency_type TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    completed_at TEXT,
    FOREIGN KEY (parent_task_id) REFERENCES tasks(id)
);

CREATE TABLE task_dependencies (
    task_id TEXT NOT NULL,
    depends_on_task_id TEXT NOT NULL,
    PRIMARY KEY (task_id, depends_on_task_id),
    FOREIGN KEY (task_id) REFERENCES tasks(id),
    FOREIGN KEY (depends_on_task_id) REFERENCES tasks(id)
);

CREATE TABLE memory (
    namespace TEXT NOT NULL,
    key TEXT NOT NULL,
    value TEXT NOT NULL,
    memory_type TEXT NOT NULL,
    created_by TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    deleted_at TEXT,
    PRIMARY KEY (namespace, key)
);
```

### HTTP Framework

**Axum + Tower**

*Why Axum?*

- **Tokio-native**: Deep integration with async runtime
- **Type-safe**: Compile-time verified extractors and handlers
- **Middleware**: Tower ecosystem for observability, CORS, rate limiting
- **Performance**: Minimal overhead, excellent throughput

### LLM Integration

**Dual Substrate Pattern**

Abathur supports two LLM substrate implementations:

1. **Anthropic API** (Direct HTTP): For production deployments
2. **Claude Code MCP** (Local development): For interactive development

*Why dual substrates?*

- ✅ Development flexibility: Test agents locally in Claude Code
- ✅ Production deployment: Direct API calls for lower latency
- ✅ Cost optimization: Choose substrate based on use case
- ✅ Vendor flexibility: Easy to add new LLM providers

**Substrate Trait:**

```rust
#[async_trait]
pub trait LLMSubstrate: Send + Sync {
    async fn generate_response(
        &self,
        prompt: &str,
        context: Option<&str>,
        model: &str,
    ) -> Result<String, DomainError>;

    fn substrate_type(&self) -> SubstrateType;
}
```

### CLI Framework

**clap 4.x (derive macros) + comfy-table**

*Why clap?*

- **Derive Macros**: Declarative command definitions
- **Auto-generated Help**: Consistent help text
- **Validation**: Built-in argument validation
- **Subcommands**: Natural command hierarchy

*Why comfy-table?*

- **Rich Formatting**: ANSI colors, borders, alignment
- **Unicode Support**: Box-drawing characters for trees
- **Dynamic Width**: Adapts to terminal size

### Logging & Observability

**tracing + tracing-subscriber**

*Why tracing?*

- **Structured Logging**: Key-value pairs, not string concatenation
- **Spans**: Track request lifecycles across async boundaries
- **Levels**: trace, debug, info, warn, error
- **Filtering**: Runtime configuration via `RUST_LOG`
- **JSON Output**: Machine-readable logs for analysis

**Audit Trail:**

All critical operations logged with:

- `task_id`: Task identifier
- `agent_id`: Agent identifier
- `operation`: Action performed
- `timestamp`: ISO 8601 timestamp
- `user`: Creator/updater

### Configuration

**figment (YAML + env)**

*Why figment?*

- **Hierarchical Merging**: Multiple config sources
- **Type Safety**: Deserialize into Rust structs
- **Environment Variables**: Override via `ABATHUR_*`
- **Validation**: Fail fast on invalid config

**Hierarchy (highest to lowest precedence):**

1. Environment variables: `ABATHUR_*`
2. Local overrides: `.abathur/local.yaml`
3. User config: `~/.abathur/config.yaml`
4. Template defaults: `.abathur/config.yaml`

## Design Decisions

### 1. Clean Architecture / Hexagonal Architecture

**Decision:** Strict layer separation with dependency inversion.

**Rationale:**

- **Testability**: Mock infrastructure layer, test domain/service logic in isolation
- **Flexibility**: Swap implementations (e.g., SQLite → PostgreSQL) without changing domain logic
- **Maintainability**: Changes in outer layers don't affect inner layers
- **Independence**: Domain layer has zero external dependencies

**Trade-offs:**

- ✅ Highly testable, maintainable codebase
- ✅ Easy to swap implementations (database, LLM provider, etc.)
- ✅ Domain logic remains pure and independent
- ❌ More upfront design effort
- ❌ More code (traits, adapters, implementations)
- ❌ Learning curve for developers unfamiliar with pattern

### 2. Async-First with Tokio

**Decision:** Fully asynchronous using Tokio runtime.

**Rationale:**

- **Concurrency**: Spawn hundreds of concurrent agents without thread overhead
- **Non-blocking I/O**: Database, HTTP, LLM calls don't block other tasks
- **Backpressure**: Semaphore-based concurrency control prevents resource exhaustion
- **Ecosystem**: Mature async ecosystem (sqlx, axum, reqwest, tracing)

**Trade-offs:**

- ✅ Excellent concurrency for I/O-bound workloads
- ✅ Low memory overhead per task
- ✅ Natural fit for agent orchestration
- ❌ Async rust has a learning curve
- ❌ Some dependencies don't support async
- ❌ Debugging async code can be challenging

### 3. Priority-Based Task Queue with Dependencies

**Decision:** Tasks have base priority + effective priority calculated from dependencies.

**Rationale:**

- **User Control**: Users can assign importance (base priority 0-10)
- **Smart Scheduling**: Critical path tasks automatically prioritized
- **Deadlock Prevention**: Cycle detection prevents dependency loops
- **Sequential/Parallel**: Support both execution models via dependency type

**Algorithm:**

```
effective_priority = base_priority + depth_in_dependency_tree * 0.1
```

**Trade-offs:**

- ✅ Flexible scheduling: manual + automatic priority
- ✅ Prevents deadlocks with cycle detection
- ✅ Supports complex workflows with mixed dependencies
- ❌ Priority calculation adds complexity
- ❌ Requires careful dependency graph design

### 4. Hierarchical Memory with Namespaces

**Decision:** Memory organized in hierarchical namespaces (e.g., `project/task/{task_id}/context`).

**Rationale:**

- **Organization**: Natural grouping by project, task, agent
- **Access Control**: Namespace-based permissions (future)
- **Search**: Prefix search for related memories
- **Isolation**: Agents can have private namespaces

**Trade-offs:**

- ✅ Intuitive organization
- ✅ Efficient prefix search
- ✅ Future-proof for access control
- ❌ Namespace design requires upfront planning
- ❌ Deeper namespaces = longer keys

### 5. MCP HTTP Server (not stdio)

**Decision:** MCP server uses HTTP JSON-RPC, not stdio transport.

**Rationale:**

- **Networking**: Remote access, load balancing, proxying
- **Language-agnostic**: Any HTTP client can integrate
- **Debugging**: Easy to test with curl, Postman
- **Scalability**: Multiple clients, stateless handlers

**Trade-offs:**

- ✅ Flexible deployment options
- ✅ Easy to integrate from any language
- ✅ Standard HTTP tooling (proxies, load balancers)
- ❌ More complex than stdio for local development
- ❌ Requires network configuration

### 6. Embedded SQLite (not PostgreSQL)

**Decision:** Use SQLite with WAL mode instead of PostgreSQL.

**Rationale:**

- **Simplicity**: Zero configuration, no database server
- **Performance**: Local file access is fast for single-machine workloads
- **Portability**: Database is a single file, easy to backup/restore
- **Development**: No Docker/services needed for development

**Trade-offs:**

- ✅ Zero operational overhead
- ✅ Excellent for single-machine use cases
- ✅ Fast for local access
- ❌ Not suitable for distributed systems (future limitation)
- ❌ WAL mode requires proper cleanup

**Future Consideration:** If distributed orchestration is needed, add PostgreSQL adapter via repository pattern.

## Scalability Considerations

### Current Architecture (Single Machine)

Abathur is optimized for **single-machine concurrency**:

- **Agents**: Concurrently execute up to configured limit (e.g., 10-100 agents)
- **Database**: SQLite WAL mode supports concurrent reads + single writer
- **Memory**: In-memory rate limiter, no shared state across processes

**Scalability Limits:**

- **Vertical Scaling**: Add CPU cores → spawn more agents
- **Agent Limit**: Anthropic API rate limits (tier-dependent)
- **Database**: SQLite WAL mode handles ~100K writes/second (sufficient)
- **Memory**: Limited by machine RAM

### Future Horizontal Scaling

To scale across multiple machines, consider:

1. **Distributed Task Queue**:
   - Replace SQLite with PostgreSQL
   - Add distributed locking (Redis, etcd)
   - Implement leader election for coordination

2. **Agent Distribution**:
   - Agent pool across multiple machines
   - Central coordinator assigns tasks to worker nodes
   - Shared state via distributed database

3. **Memory Distribution**:
   - Replace SQLite memory with Redis/DynamoDB
   - Namespace-based sharding
   - Eventual consistency model

4. **Load Balancing**:
   - Multiple MCP server instances behind load balancer
   - Sticky sessions for agent continuity
   - Horizontal pod autoscaling in Kubernetes

**Design Principle:** Current architecture uses repository pattern and port abstractions, making it straightforward to swap implementations for distributed deployment.

## Integration Points

### External Systems

```mermaid
graph LR
    Abathur[Abathur CLI]

    subgraph "LLM Providers"
        Anthropic[Anthropic API<br/>Claude Sonnet/Opus]
        ClaudeCode[Claude Code MCP<br/>Local development]
    end

    subgraph "Version Control"
        Git[Git<br/>Agent artifact storage]
    end

    subgraph "External Clients"
        MCPClients[MCP Clients<br/>Claude Code, etc.]
    end

    subgraph "Configuration"
        YAML[YAML Config<br/>.abathur/config.yaml]
        EnvVars[Environment Variables<br/>ANTHROPIC_API_KEY]
    end

    Abathur -->|Generate responses| Anthropic
    Abathur -->|Generate responses| ClaudeCode
    Abathur -->|Read agent templates| Git
    MCPClients -->|HTTP JSON-RPC| Abathur
    YAML -.->|Load config| Abathur
    EnvVars -.->|Override config| Abathur

    style Abathur fill:#E8F5E9
    style Anthropic fill:#FFF9C4
    style MCPClients fill:#E1F5FE
```

**Integration Methods:**

- **Anthropic API**: Direct HTTP REST API calls with rate limiting
- **Claude Code**: MCP protocol (JSON-RPC over HTTP)
- **Git**: CLI commands for template repository management
- **External MCP Clients**: HTTP server on `:8080` with JSON-RPC endpoints

### Data Persistence

```mermaid
graph TB
    subgraph "Application Data"
        Tasks[Tasks<br/>tasks table]
        Deps[Dependencies<br/>task_dependencies table]
        Memory[Memory<br/>memory table]
        Sessions[Sessions<br/>sessions table]
        Agents[Agents<br/>agents table]
    end

    subgraph "Configuration Data"
        Config[Config<br/>YAML files]
    end

    subgraph "Audit Data"
        Logs[Logs<br/>Structured JSON]
    end

    subgraph "Storage"
        SQLite[(SQLite WAL<br/>.abathur/abathur.db)]
        FS[Filesystem<br/>.abathur/]
    end

    Tasks --> SQLite
    Deps --> SQLite
    Memory --> SQLite
    Sessions --> SQLite
    Agents --> SQLite
    Config --> FS
    Logs --> FS

    style SQLite fill:#E1F5FE
    style FS fill:#FFF9C4
```

**Storage Locations:**

- `.abathur/abathur.db`: SQLite database (tasks, memory, sessions, agents)
- `.abathur/config.yaml`: Template configuration
- `.abathur/local.yaml`: User overrides
- `.abathur/logs/`: Structured logs
- `~/.abathur/`: User-level configuration

## Summary

Abathur's architecture prioritizes:

1. **Clean Architecture**: Testable, maintainable, flexible
2. **Type Safety**: Rust's type system prevents entire classes of bugs
3. **Concurrency**: Tokio async runtime for high-concurrency agent execution
4. **Observability**: Structured logging, audit trails, resource monitoring
5. **Extensibility**: Port abstractions make it easy to add new integrations
6. **Developer Experience**: Rich CLI output, comprehensive documentation

**Trade-offs:**

- Single-machine optimization (future: distributed scaling)
- Rust learning curve (benefit: safety + performance)
- More upfront design (benefit: long-term maintainability)

**Next Steps:**

- [Getting Started Guide](../getting-started/quickstart.md): Install and run Abathur
- [How-To Guides](../how-to/submit-task.md): Common operations
- [API Reference](../reference/cli-commands.md): Complete CLI documentation
