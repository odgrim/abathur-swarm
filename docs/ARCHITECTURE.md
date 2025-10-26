# Abathur Architecture

This document describes the architecture of Abathur, a Rust-based CLI orchestration system for managing swarms of specialized Claude agents with task queues, concurrent execution, and iterative refinement.

## Table of Contents

- [Architecture Overview](#architecture-overview)
- [Clean Architecture Layers](#clean-architecture-layers)
- [Hexagonal Architecture (Ports & Adapters)](#hexagonal-architecture-ports--adapters)
- [Module Structure](#module-structure)
- [Dependency Injection Pattern](#dependency-injection-pattern)
- [Error Handling Strategy](#error-handling-strategy)
- [Testing Strategy](#testing-strategy)
- [Async Runtime Architecture](#async-runtime-architecture)
- [Database Architecture](#database-architecture)
- [Key Design Patterns](#key-design-patterns)

## Architecture Overview

Abathur follows **Clean Architecture** (also known as Hexagonal Architecture) principles, organizing code into concentric layers with strict dependency rules:

```
┌─────────────────────────────────────────────────────┐
│              CLI Layer (Abathur)                    │
│  • Command parsing (clap)                           │
│  • Terminal output (comfy-table, indicatif)         │
│  • User interaction                                 │
└─────────────────┬───────────────────────────────────┘
                  │ Uses
                  ▼
┌─────────────────────────────────────────────────────┐
│           Application Layer                         │
│  • SwarmOrchestrator                                │
│  • TaskCoordinator                                  │
│  • AgentExecutor                                    │
│  • LoopExecutor                                     │
│  Orchestrates domain logic, coordinates services    │
└─────────────────┬───────────────────────────────────┘
                  │ Uses
                  ▼
┌─────────────────────────────────────────────────────┐
│            Services Layer                           │
│  • Task lifecycle management                        │
│  • Priority calculation                             │
│  • Dependency resolution                            │
│  Business logic implementation                      │
└─────────────────┬───────────────────────────────────┘
                  │ Uses
                  ▼
┌─────────────────────────────────────────────────────┐
│             Domain Layer                            │
│  • Task, Agent, Result (models)                     │
│  • TaskRepository, ConfigLoader (ports/traits)      │
│  • Domain errors                                    │
│  Core business entities and rules                   │
└─────────────────┬───────────────────────────────────┘
                  │ Implements
                  ▼
┌─────────────────────────────────────────────────────┐
│         Infrastructure Layer                        │
│  • SQLite database (TaskRepository impl)            │
│  • File system (ConfigLoader impl)                  │
│  • HTTP clients (Claude API)                        │
│  • Logging (tracing)                                │
│  External system integrations                       │
└─────────────────────────────────────────────────────┘
```

### Dependency Rule

**Dependencies point inward**: Outer layers depend on inner layers, never the reverse.

- ✅ CLI can depend on Application
- ✅ Application can depend on Domain
- ✅ Infrastructure implements Domain interfaces (ports)
- ❌ Domain cannot depend on Infrastructure
- ❌ Domain cannot depend on Application

## Clean Architecture Layers

### 1. CLI Layer (`src/abathur/`)

**Responsibility**: User interface and command-line interaction

**Components**:
- `commands/`: CLI command implementations using `clap`
  - `task_commands.rs`: Task management commands
  - `swarm_commands.rs`: Swarm orchestration commands
  - `mcp_commands.rs`: MCP server management
- `output/`: Terminal output formatting
  - `table.rs`: Table rendering with `comfy-table`
  - `tree.rs`: Hierarchical tree visualization
  - `progress.rs`: Progress bars with `indicatif`

**Example**:
```rust
// src/abathur/commands/task_commands.rs
#[derive(Parser)]
pub struct TaskListCommand {
    #[arg(long)]
    status: Option<TaskStatus>,

    #[arg(long)]
    tree: bool,
}

impl TaskListCommand {
    pub async fn execute(&self, app: &Application) -> Result<()> {
        let tasks = app.task_coordinator.list_tasks(self.status).await?;
        if self.tree {
            output::render_task_tree(&tasks);
        } else {
            output::render_task_table(&tasks);
        }
        Ok(())
    }
}
```

### 2. Application Layer (`src/application/`)

**Responsibility**: Orchestrate use cases and coordinate domain logic

**Components**:
- `SwarmOrchestrator`: Manages concurrent agent execution
- `TaskCoordinator`: Coordinates task lifecycle and scheduling
- `AgentExecutor`: Executes individual agents
- `LoopExecutor`: Manages iterative refinement loops

**Example**:
```rust
// src/application/swarm_orchestrator.rs
pub struct SwarmOrchestrator {
    task_coordinator: Arc<TaskCoordinator>,
    agent_executor: Arc<AgentExecutor>,
    concurrency_limit: usize,
}

impl SwarmOrchestrator {
    pub async fn start(&self, max_agents: usize) -> Result<()> {
        let semaphore = Arc::new(Semaphore::new(max_agents));

        loop {
            let tasks = self.task_coordinator.get_ready_tasks(10).await?;
            if tasks.is_empty() {
                break;
            }

            for task in tasks {
                let permit = semaphore.clone().acquire_owned().await?;
                let executor = self.agent_executor.clone();

                tokio::spawn(async move {
                    let _permit = permit; // Hold permit until task completes
                    executor.execute(task).await
                });
            }
        }
        Ok(())
    }
}
```

### 3. Services Layer (`src/services/`)

**Responsibility**: Implement business logic and domain services

**Components**:
- `PriorityCalculator`: Calculate task priorities based on multiple factors
- `DependencyResolver`: Resolve task dependency graphs
- `RetryStrategy`: Implement exponential backoff retry logic

**Example**:
```rust
// src/services/priority_calculator.rs
pub struct PriorityCalculator;

impl PriorityCalculator {
    pub fn calculate(&self, task: &Task) -> u8 {
        let mut priority = task.base_priority();

        // Age-based boost
        let age_days = task.age().num_days();
        priority += (age_days / 7) as u8;

        // Dependency depth penalty
        priority = priority.saturating_sub(task.dependency_depth() * 2);

        priority.clamp(0, 10)
    }
}
```

### 4. Domain Layer (`src/domain/`)

**Responsibility**: Core business entities, rules, and abstractions

**Components**:
- `models/`: Domain entities (Task, Agent, ExecutionResult)
- `ports/`: Trait interfaces for external dependencies
- Domain-specific errors

**Example**:
```rust
// src/domain/models/task.rs
#[derive(Debug, Clone)]
pub struct Task {
    id: TaskId,
    description: String,
    status: TaskStatus,
    priority: u8,
    prerequisites: Vec<TaskId>,
    created_at: DateTime<Utc>,
}

impl Task {
    pub fn new(description: String, priority: u8) -> Self {
        Self {
            id: TaskId::new(),
            description,
            status: TaskStatus::Pending,
            priority,
            prerequisites: Vec::new(),
            created_at: Utc::now(),
        }
    }

    pub fn can_execute(&self) -> bool {
        self.status == TaskStatus::Ready
    }

    pub fn mark_completed(&mut self) -> Result<(), TaskError> {
        if !matches!(self.status, TaskStatus::Running) {
            return Err(TaskError::InvalidTransition {
                from: self.status,
                to: TaskStatus::Completed,
            });
        }
        self.status = TaskStatus::Completed;
        Ok(())
    }
}
```

### 5. Infrastructure Layer (`src/infrastructure/`)

**Responsibility**: Implement domain ports with external systems

**Components**:
- `database/`: SQLite repository implementations
- `config/`: Configuration loading from files/env
- `logging/`: Structured logging setup
- `http/`: HTTP clients for external APIs

**Example**:
```rust
// src/infrastructure/database/task_repository.rs
pub struct SqliteTaskRepository {
    pool: SqlitePool,
}

#[async_trait]
impl TaskRepository for SqliteTaskRepository {
    async fn save(&self, task: &Task) -> Result<()> {
        sqlx::query!(
            r#"
            INSERT INTO tasks (id, description, status, priority, created_at)
            VALUES (?, ?, ?, ?, ?)
            "#,
            task.id,
            task.description,
            task.status,
            task.priority,
            task.created_at
        )
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    async fn get_by_id(&self, id: &TaskId) -> Result<Option<Task>> {
        let row = sqlx::query_as!(
            TaskRow,
            "SELECT * FROM tasks WHERE id = ?",
            id
        )
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(Task::from))
    }
}
```

## Hexagonal Architecture (Ports & Adapters)

The domain defines **ports** (trait interfaces), and infrastructure provides **adapters** (implementations).

### Ports (Traits in Domain)

```rust
// src/domain/ports/task_repository.rs
#[async_trait]
pub trait TaskRepository: Send + Sync {
    async fn save(&self, task: &Task) -> Result<()>;
    async fn get_by_id(&self, id: &TaskId) -> Result<Option<Task>>;
    async fn list(&self, filter: TaskFilter) -> Result<Vec<Task>>;
    async fn update(&self, task: &Task) -> Result<()>;
}

// src/domain/ports/config_loader.rs
#[async_trait]
pub trait ConfigLoader: Send + Sync {
    async fn load(&self) -> Result<Config>;
    async fn save(&self, config: &Config) -> Result<()>;
}
```

### Adapters (Implementations in Infrastructure)

```rust
// src/infrastructure/database/task_repository.rs
pub struct SqliteTaskRepository { /* ... */ }

#[async_trait]
impl TaskRepository for SqliteTaskRepository {
    // Implementation using SQLx
}

// src/infrastructure/config/yaml_loader.rs
pub struct YamlConfigLoader { /* ... */ }

#[async_trait]
impl ConfigLoader for YamlConfigLoader {
    // Implementation using figment + YAML
}
```

### Benefits

1. **Testability**: Mock ports for unit testing
2. **Flexibility**: Swap implementations (SQLite → PostgreSQL)
3. **Separation of Concerns**: Business logic independent of infrastructure

## Module Structure

```
src/
├── main.rs                    # Application entry point
├── lib.rs                     # Library root
│
├── abathur/                   # CLI Layer
│   ├── mod.rs
│   ├── commands/
│   │   ├── mod.rs
│   │   ├── task_commands.rs
│   │   ├── swarm_commands.rs
│   │   └── mcp_commands.rs
│   └── output/
│       ├── mod.rs
│       ├── table.rs
│       ├── tree.rs
│       └── progress.rs
│
├── application/               # Application Layer
│   ├── mod.rs
│   └── services/
│       ├── mod.rs
│       ├── swarm_orchestrator.rs
│       ├── task_coordinator.rs
│       ├── agent_executor.rs
│       └── loop_executor.rs
│
├── domain/                    # Domain Layer
│   ├── mod.rs
│   ├── models/
│   │   ├── mod.rs
│   │   ├── task.rs
│   │   ├── agent.rs
│   │   └── result.rs
│   └── ports/
│       ├── mod.rs
│       ├── task_repository.rs
│       └── config_loader.rs
│
├── infrastructure/            # Infrastructure Layer
│   ├── mod.rs
│   ├── database/
│   │   ├── mod.rs
│   │   ├── pool.rs
│   │   ├── migrations.rs
│   │   └── task_repository.rs
│   ├── config/
│   │   ├── mod.rs
│   │   └── yaml_loader.rs
│   └── logging/
│       ├── mod.rs
│       └── setup.rs
│
└── services/                  # Services Layer
    ├── mod.rs
    ├── priority_calculator.rs
    └── dependency_resolver.rs
```

## Dependency Injection Pattern

Abathur uses **constructor injection** with `Arc<dyn Trait>` for dependency injection.

### Pattern

```rust
// Define port in domain
#[async_trait]
pub trait TaskRepository: Send + Sync {
    async fn save(&self, task: &Task) -> Result<()>;
}

// Application service depends on port
pub struct TaskCoordinator {
    repository: Arc<dyn TaskRepository>,
}

impl TaskCoordinator {
    pub fn new(repository: Arc<dyn TaskRepository>) -> Self {
        Self { repository }
    }

    pub async fn create_task(&self, desc: String) -> Result<Task> {
        let task = Task::new(desc, 5);
        self.repository.save(&task).await?;
        Ok(task)
    }
}

// Main wires up dependencies
#[tokio::main]
async fn main() -> Result<()> {
    let pool = setup_database().await?;
    let repository: Arc<dyn TaskRepository> = Arc::new(SqliteTaskRepository::new(pool));
    let coordinator = TaskCoordinator::new(repository);

    // Use coordinator...
}
```

### Benefits

- **Testability**: Inject mocks for testing
- **Flexibility**: Swap implementations at runtime
- **Explicit Dependencies**: Clear what each component needs

## Error Handling Strategy

### Domain Errors (thiserror)

Use for **library/domain errors** that are part of the public API:

```rust
use thiserror::Error;

#[derive(Error, Debug)]
pub enum TaskError {
    #[error("Task not found: {task_id}")]
    NotFound { task_id: String },

    #[error("Invalid status transition from {from:?} to {to:?}")]
    InvalidTransition { from: TaskStatus, to: TaskStatus },

    #[error("Circular dependency detected: {cycle:?}")]
    CircularDependency { cycle: Vec<String> },
}
```

**When to use**:
- Domain layer errors
- Errors that callers should handle explicitly
- Errors that need to be matched/destructured

### Application Errors (anyhow)

Use for **application-level errors** with rich context:

```rust
use anyhow::{Context, Result};

pub async fn execute_task(task_id: &str) -> Result<ExecutionResult> {
    let task = load_task(task_id)
        .await
        .context(format!("Failed to load task {}", task_id))?;

    let result = run_agent(&task)
        .await
        .context("Agent execution failed")?;

    save_result(&result)
        .await
        .context("Failed to persist execution result")?;

    Ok(result)
}
```

**When to use**:
- Application layer (orchestration)
- CLI layer
- When you need error context chains
- When the caller won't handle specific error types

### Error Conversion

Infrastructure errors convert to domain errors:

```rust
impl From<sqlx::Error> for TaskError {
    fn from(err: sqlx::Error) -> Self {
        match err {
            sqlx::Error::RowNotFound => TaskError::NotFound {
                task_id: "unknown".into(),
            },
            _ => TaskError::DatabaseError(err.to_string()),
        }
    }
}
```

## Testing Strategy

### 1. Unit Tests (Fast, Isolated)

Test individual functions/methods in isolation:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_task_creation() {
        let task = Task::new("Test task", 5);
        assert_eq!(task.status(), TaskStatus::Pending);
        assert_eq!(task.priority(), 5);
    }

    #[test]
    fn test_invalid_transition() {
        let mut task = Task::new("Test", 5);
        let result = task.mark_completed();
        assert!(matches!(result, Err(TaskError::InvalidTransition { .. })));
    }
}
```

### 2. Integration Tests (Tests/Integration/)

Test component interactions with real dependencies:

```rust
// tests/integration/task_repository_test.rs
#[tokio::test]
async fn test_task_persistence() {
    let pool = setup_test_database().await;
    let repo = SqliteTaskRepository::new(pool);

    let task = Task::new("Integration test", 7);
    repo.save(&task).await.unwrap();

    let loaded = repo.get_by_id(&task.id).await.unwrap().unwrap();
    assert_eq!(loaded.id, task.id);
    assert_eq!(loaded.description, task.description);
}
```

### 3. Property Tests (Proptest)

Test invariants with randomized inputs:

```rust
use proptest::prelude::*;

proptest! {
    #[test]
    fn test_priority_bounds(base in 0u8..=10) {
        let task = Task::new("Test", base);
        let calculator = PriorityCalculator;
        let calculated = calculator.calculate(&task);
        prop_assert!(calculated <= 10);
    }
}
```

### 4. Mocking (Mockall)

Mock trait implementations for testing:

```rust
use mockall::mock;

mock! {
    TaskRepo {}

    #[async_trait]
    impl TaskRepository for TaskRepo {
        async fn save(&self, task: &Task) -> Result<()>;
        async fn get_by_id(&self, id: &TaskId) -> Result<Option<Task>>;
    }
}

#[tokio::test]
async fn test_coordinator_with_mock() {
    let mut mock_repo = MockTaskRepo::new();
    mock_repo
        .expect_save()
        .returning(|_| Ok(()));

    let coordinator = TaskCoordinator::new(Arc::new(mock_repo));
    let result = coordinator.create_task("Test".into()).await;
    assert!(result.is_ok());
}
```

## Async Runtime Architecture

Abathur uses **Tokio** as the async runtime.

### Concurrency Model

```rust
// Semaphore-based concurrency control
let semaphore = Arc::new(Semaphore::new(max_concurrent));

for task in tasks {
    let permit = semaphore.clone().acquire_owned().await?;
    let executor = self.executor.clone();

    tokio::spawn(async move {
        let _permit = permit; // Released when dropped
        executor.execute(task).await
    });
}
```

### Graceful Shutdown

```rust
pub struct Application {
    shutdown_tx: broadcast::Sender<()>,
}

impl Application {
    pub async fn run(&self) -> Result<()> {
        let mut shutdown_rx = self.shutdown_tx.subscribe();

        tokio::select! {
            result = self.swarm.start() => result?,
            _ = shutdown_rx.recv() => {
                info!("Shutdown signal received");
            }
        }

        Ok(())
    }
}
```

## Database Architecture

### SQLite with WAL Mode

```rust
pub async fn create_pool() -> Result<SqlitePool> {
    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect("sqlite:.abathur/abathur.db?mode=rwc")
        .await?;

    // Enable WAL mode for concurrency
    sqlx::query("PRAGMA journal_mode = WAL")
        .execute(&pool)
        .await?;

    Ok(pool)
}
```

### Migrations

```rust
// migrations/001_create_tasks.sql
CREATE TABLE IF NOT EXISTS tasks (
    id TEXT PRIMARY KEY,
    description TEXT NOT NULL,
    status TEXT NOT NULL,
    priority INTEGER NOT NULL,
    created_at TEXT NOT NULL
);

CREATE INDEX idx_tasks_status ON tasks(status);
CREATE INDEX idx_tasks_priority ON tasks(priority DESC);
```

### Repository Pattern

```rust
#[async_trait]
impl TaskRepository for SqliteTaskRepository {
    async fn list(&self, filter: TaskFilter) -> Result<Vec<Task>> {
        let mut query = QueryBuilder::new("SELECT * FROM tasks WHERE 1=1");

        if let Some(status) = filter.status {
            query.push(" AND status = ").push_bind(status.to_string());
        }

        query.push(" ORDER BY priority DESC, created_at ASC");

        let tasks = query
            .build_query_as::<TaskRow>()
            .fetch_all(&self.pool)
            .await?
            .into_iter()
            .map(Task::from)
            .collect();

        Ok(tasks)
    }
}
```

## Key Design Patterns

### 1. Repository Pattern
Abstract data persistence behind trait interfaces.

### 2. Dependency Injection
Constructor injection with `Arc<dyn Trait>`.

### 3. Strategy Pattern
Configurable behaviors (e.g., RetryStrategy, ConvergenceStrategy).

### 4. Builder Pattern
Complex object construction (e.g., TaskBuilder, QueryBuilder).

### 5. Observer Pattern
Event notification (e.g., task status changes, agent lifecycle events).

### 6. Factory Pattern
Object creation (e.g., AgentFactory creates agents based on type).

---

**For more details**:
- [CONTRIBUTING.md](../CONTRIBUTING.md): Development guidelines
- [README.md](../README.md): Getting started guide
