# Contributing to Abathur

Thank you for your interest in contributing to Abathur! This document provides guidelines for developing and contributing to the project.

## Table of Contents

- [Development Environment Setup](#development-environment-setup)
- [Project Architecture](#project-architecture)
- [Development Workflow](#development-workflow)
- [Code Style Guidelines](#code-style-guidelines)
- [Testing Requirements](#testing-requirements)
- [Pull Request Process](#pull-request-process)
- [Getting Help](#getting-help)

## Development Environment Setup

### Prerequisites

- **Rust**: 1.83 or higher (install via [rustup](https://rustup.rs/))
- **Git**: For version control
- **SQLite**: For database operations (usually pre-installed on Unix systems)
- **Anthropic API Key**: For Claude integration (optional for core development)

### Installation

1. **Clone the repository**:
   ```bash
   git clone https://github.com/yourorg/abathur.git
   cd abathur
   ```

2. **Install Rust toolchain** (if not already installed):
   ```bash
   curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
   source $HOME/.cargo/env
   ```

3. **Install development tools**:
   ```bash
   # Install rustfmt (code formatter)
   rustup component add rustfmt

   # Install clippy (linter)
   rustup component add clippy

   # Install cargo-tarpaulin (code coverage)
   cargo install cargo-tarpaulin
   ```

4. **Build the project**:
   ```bash
   cargo build
   ```

5. **Run tests** to verify setup:
   ```bash
   cargo test
   ```

### IDE Setup

**VS Code** (Recommended):
- Install the [rust-analyzer](https://marketplace.visualstudio.com/items?itemName=rust-lang.rust-analyzer) extension
- Install the [Even Better TOML](https://marketplace.visualstudio.com/items?itemName=tamasfe.even-better-toml) extension
- Enable format on save in settings

**IntelliJ IDEA / CLion**:
- Install the official Rust plugin
- Enable rustfmt and clippy in settings

## Project Architecture

Abathur follows **Clean Architecture** (Hexagonal Architecture) principles with clear layer separation. For detailed architecture documentation, see [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).

### Directory Structure

```
src/
├── main.rs                 # CLI entry point
├── lib.rs                  # Library root
├── abathur/               # CLI layer (commands, output formatting)
│   ├── commands/          # CLI command implementations
│   └── output/            # Terminal output formatting
├── application/           # Application services (orchestration)
│   ├── services/          # Service implementations
│   └── mod.rs
├── domain/                # Domain models and business logic
│   ├── models/            # Domain entities
│   ├── ports/             # Trait interfaces (hexagonal ports)
│   └── mod.rs
├── infrastructure/        # External integrations
│   ├── database/          # SQLite repository implementations
│   ├── config/            # Configuration management
│   └── logging/           # Structured logging setup
└── services/              # Service layer implementations
    └── mod.rs
```

### Key Principles

1. **Dependency Inversion**: Domain layer has no dependencies on infrastructure
2. **Port and Adapter Pattern**: Domain defines traits (ports), infrastructure implements them (adapters)
3. **Async-first**: All I/O operations use async/await with tokio runtime
4. **Type Safety**: Leverage Rust's type system for correctness
5. **Error Handling**: Use `thiserror` for domain errors, `anyhow` for application errors

## Development Workflow

### Making Changes

1. **Create a feature branch**:
   ```bash
   git checkout -b feature/your-feature-name
   ```

2. **Make your changes** following the code style guidelines

3. **Run the formatter**:
   ```bash
   cargo fmt
   ```

4. **Run the linter**:
   ```bash
   cargo clippy --all-targets --all-features -- -D warnings
   ```

5. **Run tests**:
   ```bash
   cargo test --all-features
   ```

6. **Build the project**:
   ```bash
   cargo build --release
   ```

### Common Development Tasks

#### Running the CLI locally

```bash
# Run with debug logging
RUST_LOG=debug cargo run -- <command>

# Example: List tasks
cargo run -- task list

# Example: Initialize database
cargo run -- init
```

#### Running specific tests

```bash
# Run all tests
cargo test

# Run tests for a specific module
cargo test domain::models

# Run a specific test
cargo test test_task_creation

# Run tests with output
cargo test -- --nocapture

# Run tests with verbose output
cargo test -- --show-output
```

#### Checking code coverage

```bash
# Generate coverage report
cargo tarpaulin --all-features --workspace --timeout 120 --out Html

# Open coverage report
open tarpaulin-report.html
```

#### Running benchmarks

```bash
# Run all benchmarks
cargo bench

# Run specific benchmark
cargo bench task_queue
```

## Code Style Guidelines

### General Style

- **Follow Rust conventions**: Use snake_case for functions/variables, PascalCase for types
- **Maximum line width**: 100 characters (enforced by rustfmt)
- **Use explicit types** for public APIs, type inference for internal code
- **Document all public APIs** with `///` doc comments
- **Use `//!` for module-level documentation**

### Naming Conventions

```rust
// Types: PascalCase
struct TaskQueue { }
enum TaskStatus { }

// Functions/variables: snake_case
fn calculate_priority(task: &Task) -> u8 { }
let max_retries = 3;

// Constants: SCREAMING_SNAKE_CASE
const MAX_CONCURRENT_AGENTS: usize = 10;

// Traits: PascalCase (often adjectives or nouns)
trait TaskRepository { }
trait Executable { }
```

### Error Handling

Use `thiserror` for **domain/library errors** (errors that are part of the API):

```rust
use thiserror::Error;

#[derive(Error, Debug)]
pub enum TaskError {
    #[error("Task not found: {task_id}")]
    NotFound { task_id: String },

    #[error("Invalid task status transition from {from} to {to}")]
    InvalidTransition { from: TaskStatus, to: TaskStatus },
}
```

Use `anyhow` for **application errors** (errors in CLI/main):

```rust
use anyhow::{Context, Result};

pub async fn load_config() -> Result<Config> {
    let path = config_path().context("Failed to determine config path")?;
    let contents = tokio::fs::read_to_string(&path)
        .await
        .context(format!("Failed to read config from {}", path.display()))?;
    Ok(serde_yaml::from_str(&contents)?)
}
```

### Async Code

```rust
// Prefer async/await over futures combinators
async fn good_example() -> Result<Task> {
    let task = repository.get_task(id).await?;
    let result = process_task(&task).await?;
    Ok(result)
}

// Use #[instrument] for tracing
use tracing::instrument;

#[instrument(skip(repository))]
async fn fetch_task(repository: &dyn TaskRepository, id: &str) -> Result<Task> {
    repository.get_task(id).await
}
```

### Documentation

```rust
//! Module-level documentation goes here
//!
//! This module provides task queue management with priority-based scheduling.

/// Represents a task in the queue.
///
/// Tasks are the fundamental unit of work in Abathur. Each task has a unique
/// identifier, priority, status, and optional dependencies.
///
/// # Examples
///
/// ```
/// use abathur::domain::Task;
///
/// let task = Task::new("Write documentation", Priority::High);
/// assert_eq!(task.status(), TaskStatus::Pending);
/// ```
pub struct Task {
    // fields...
}
```

## Testing Requirements

### Test Categories

1. **Unit Tests**: Test individual functions/methods in isolation
2. **Integration Tests**: Test component interactions
3. **Property Tests**: Test invariants with randomized inputs using `proptest`

### Writing Tests

#### Unit Tests

Place unit tests in the same file as the code they test:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_task_creation() {
        let task = Task::new("Test task", Priority::Medium);
        assert_eq!(task.priority(), Priority::Medium);
        assert_eq!(task.status(), TaskStatus::Pending);
    }

    #[tokio::test]
    async fn test_async_operation() {
        let result = async_function().await;
        assert!(result.is_ok());
    }
}
```

#### Integration Tests

Place integration tests in `tests/`:

```rust
// tests/integration/task_queue.rs
use abathur::infrastructure::database::TaskRepository;
use sqlx::SqlitePool;

#[tokio::test]
async fn test_task_persistence() {
    let pool = setup_test_db().await;
    let repo = TaskRepository::new(pool);

    let task = Task::new("Integration test", Priority::High);
    repo.save(&task).await.unwrap();

    let loaded = repo.get(&task.id).await.unwrap();
    assert_eq!(loaded.id, task.id);
}
```

#### Property Tests

Use `proptest` for property-based testing:

```rust
use proptest::prelude::*;

proptest! {
    #[test]
    fn test_priority_ordering(p1 in 0u8..10, p2 in 0u8..10) {
        let priority1 = Priority::from(p1);
        let priority2 = Priority::from(p2);

        if p1 > p2 {
            assert!(priority1 > priority2);
        }
    }
}
```

### Test Coverage Requirements

- **Minimum coverage**: 80% for new code
- **Domain layer**: Aim for 90%+ coverage
- **Critical paths**: 100% coverage (task scheduling, dependency resolution)

### Running Tests

```bash
# Run all tests
cargo test

# Run tests with coverage
cargo tarpaulin --all-features --workspace --timeout 120

# Run property tests with more cases
cargo test --release -- --ignored proptest

# Run benchmarks
cargo bench
```

## Pull Request Process

### Before Submitting

1. **Ensure all tests pass**:
   ```bash
   cargo test --all-features
   ```

2. **Run clippy without warnings**:
   ```bash
   cargo clippy --all-targets --all-features -- -D warnings
   ```

3. **Format code**:
   ```bash
   cargo fmt
   ```

4. **Update documentation** if you changed public APIs

5. **Add tests** for new functionality

### PR Description Template

```markdown
## Summary
Brief description of the changes

## Motivation
Why are these changes needed?

## Changes
- Bullet point list of changes
- Include any breaking changes

## Testing
- Describe how you tested these changes
- Include test coverage metrics

## Checklist
- [ ] Tests pass (`cargo test`)
- [ ] Linter passes (`cargo clippy`)
- [ ] Code is formatted (`cargo fmt`)
- [ ] Documentation updated
- [ ] CHANGELOG.md updated (if applicable)
```

### Review Process

1. **CI checks must pass**: All automated checks must be green
2. **Code review**: At least one maintainer approval required
3. **Documentation review**: Ensure public APIs are documented
4. **Test coverage**: Verify adequate test coverage

### Merge Strategy

- **Squash and merge** for feature branches
- **Commit message format**:
  ```
  Brief summary (50 chars or less)

  More detailed explanation if needed. Wrap at 72 characters.

  - Bullet points for key changes
  - Reference issues: Fixes #123
  ```

## Getting Help

### Resources

- **Documentation**: [docs/](docs/)
- **Architecture Guide**: [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md)
- **Issues**: [GitHub Issues](https://github.com/yourorg/abathur/issues)
- **Discussions**: [GitHub Discussions](https://github.com/yourorg/abathur/discussions)

### Communication

- **Bug reports**: Use GitHub Issues with the "bug" label
- **Feature requests**: Use GitHub Issues with the "enhancement" label
- **Questions**: Use GitHub Discussions

### Debugging Tips

1. **Enable debug logging**:
   ```bash
   RUST_LOG=debug cargo run -- <command>
   ```

2. **Enable trace logging** for specific modules:
   ```bash
   RUST_LOG=abathur::domain::task=trace cargo run
   ```

3. **Use rust-gdb or rust-lldb** for debugging:
   ```bash
   rust-gdb target/debug/abathur
   ```

4. **Check SQL queries** with SQLite CLI:
   ```bash
   sqlite3 .abathur/abathur.db
   ```

## Code of Conduct

- Be respectful and inclusive
- Provide constructive feedback
- Focus on code quality and correctness
- Help newcomers get started

## License

By contributing to Abathur, you agree that your contributions will be licensed under the MIT License.
