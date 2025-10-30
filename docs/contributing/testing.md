# Testing Guidelines

This guide covers how to run tests, write new tests, and ensure code quality for Abathur contributions.

## Overview

Abathur uses multiple testing strategies:

- **Unit Tests**: Test individual functions and modules in isolation
- **Integration Tests**: Test component interactions and workflows
- **Property Tests**: Test invariants with generated inputs
- **Benchmarks**: Measure performance characteristics

All tests must pass before code can be merged.

## Running Tests

### Run All Tests

```bash
# Run all tests
cargo test

# Run with output visible (show println! statements)
cargo test -- --nocapture

# Run tests with verbose output
cargo test -- --show-output
```

**Expected Output**:
```
running 47 tests
test domain::models::task::tests::test_task_creation ... ok
test domain::models::task::tests::test_task_priority ... ok
test infrastructure::database::tests::test_insert_task ... ok
...
test result: ok. 47 passed; 0 failed; 0 ignored; 0 measured
```

### Run Specific Tests

```bash
# Run tests in a specific module
cargo test domain::models

# Run a specific test by name
cargo test test_task_creation

# Run tests matching a pattern
cargo test task_

# Run tests in a specific file
cargo test --test integration_task_queue
```

### Run Tests by Category

```bash
# Run only unit tests (in src/)
cargo test --lib

# Run only integration tests (in tests/)
cargo test --test '*'

# Run only doc tests
cargo test --doc

# Run benchmarks
cargo bench
```

### Run Tests with Parallelism

```bash
# Run tests in parallel (default)
cargo test

# Run tests sequentially (useful for debugging)
cargo test -- --test-threads=1

# Run with specific number of threads
cargo test -- --test-threads=4
```

## Integration Tests

Integration tests live in the `tests/` directory and test component interactions.

### Running Integration Tests

```bash
# Run all integration tests
cargo test --test '*'

# Run specific integration test file
cargo test --test integration_task_queue

# Run specific test in integration file
cargo test --test integration_task_queue test_task_submission
```

### Writing Integration Tests

Create a new file in `tests/` directory:

```rust
// tests/integration_task_queue.rs
use abathur_cli::domain::models::{Task, TaskStatus};
use abathur_cli::infrastructure::database::TaskRepository;
use sqlx::SqlitePool;
use uuid::Uuid;

#[tokio::test]
async fn test_task_submission_and_retrieval() {
    // Setup: Create in-memory database
    let pool = SqlitePool::connect("sqlite::memory:")
        .await
        .expect("Failed to create test database");

    // Run migrations
    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .expect("Failed to run migrations");

    let repo = TaskRepository::new(pool.clone());

    // Test: Create and insert task
    let task = Task::new(
        Uuid::new_v4(),
        "Test task summary".to_string(),
        "Test task description".to_string(),
    );

    repo.insert(&task)
        .await
        .expect("Failed to insert task");

    // Assert: Retrieve and verify task
    let retrieved = repo.get_by_id(task.id)
        .await
        .expect("Failed to retrieve task")
        .expect("Task not found");

    assert_eq!(retrieved.id, task.id);
    assert_eq!(retrieved.summary, "Test task summary");
    assert_eq!(retrieved.status, TaskStatus::Pending);
}

#[tokio::test]
async fn test_task_dependencies() {
    let pool = SqlitePool::connect("sqlite::memory:")
        .await
        .expect("Failed to create test database");

    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .expect("Failed to run migrations");

    let repo = TaskRepository::new(pool);

    // Create parent task
    let parent = Task::new(
        Uuid::new_v4(),
        "Parent task".to_string(),
        "Parent description".to_string(),
    );
    repo.insert(&parent).await.expect("Failed to insert parent");

    // Create child task with dependency
    let mut child = Task::new(
        Uuid::new_v4(),
        "Child task".to_string(),
        "Child description".to_string(),
    );
    child.dependencies = vec![parent.id];

    repo.insert(&child).await.expect("Failed to insert child");

    // Verify dependency
    let retrieved_child = repo.get_by_id(child.id)
        .await
        .expect("Failed to retrieve child")
        .expect("Child not found");

    assert_eq!(retrieved_child.dependencies.len(), 1);
    assert_eq!(retrieved_child.dependencies[0], parent.id);
}
```

## Unit Tests

Unit tests are co-located with the code they test using `#[cfg(test)]` modules.

### Running Unit Tests

```bash
# Run all unit tests
cargo test --lib

# Run unit tests in specific module
cargo test domain::models::task::tests
```

### Writing Unit Tests

Add tests in the same file as your code:

```rust
// src/domain/models/task.rs
#[derive(Debug, Clone, PartialEq)]
pub struct Task {
    pub id: Uuid,
    pub summary: String,
    pub status: TaskStatus,
    pub priority: u8,
}

impl Task {
    pub fn new(id: Uuid, summary: String, description: String) -> Self {
        Self {
            id,
            summary,
            status: TaskStatus::Pending,
            priority: 5,
        }
    }

    pub fn with_priority(mut self, priority: u8) -> Self {
        assert!(priority <= 10, "Priority must be 0-10");
        self.priority = priority;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_task_creation() {
        let task = Task::new(
            Uuid::new_v4(),
            "Test summary".to_string(),
            "Test description".to_string(),
        );

        assert_eq!(task.summary, "Test summary");
        assert_eq!(task.status, TaskStatus::Pending);
        assert_eq!(task.priority, 5);
    }

    #[test]
    fn test_task_with_priority() {
        let task = Task::new(
            Uuid::new_v4(),
            "High priority".to_string(),
            "Description".to_string(),
        ).with_priority(9);

        assert_eq!(task.priority, 9);
    }

    #[test]
    #[should_panic(expected = "Priority must be 0-10")]
    fn test_invalid_priority_panics() {
        Task::new(
            Uuid::new_v4(),
            "Invalid".to_string(),
            "Description".to_string(),
        ).with_priority(11);
    }

    #[test]
    fn test_task_equality() {
        let id = Uuid::new_v4();
        let task1 = Task::new(id, "Summary".to_string(), "Desc".to_string());
        let task2 = Task::new(id, "Summary".to_string(), "Desc".to_string());

        assert_eq!(task1, task2);
    }
}
```

## Test Coverage

### Install cargo-tarpaulin

```bash
cargo install cargo-tarpaulin
```

### Generate Coverage Report

```bash
# Generate HTML coverage report
cargo tarpaulin --all-features --workspace --timeout 120 --out Html

# Generate XML coverage report (for CI)
cargo tarpaulin --all-features --workspace --timeout 120 --out Xml

# View HTML report
open tarpaulin-report.html
```

**Coverage Goals**:
- **Overall**: ≥80% line coverage
- **Critical paths**: ≥95% coverage (task queue, database operations)
- **New code**: ≥90% coverage required for PRs

### Coverage by Module

```bash
# Generate detailed coverage per file
cargo tarpaulin --all-features --workspace --timeout 120 --out Html --verbose
```

## Property-Based Testing

Use `proptest` for property-based testing with generated inputs.

### Install proptest

Add to `Cargo.toml` dev-dependencies:

```toml
[dev-dependencies]
proptest = "1.4"
```

### Writing Property Tests

```rust
// src/domain/models/task.rs
#[cfg(test)]
mod proptests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        #[test]
        fn test_priority_always_in_range(priority in 0u8..=10) {
            let task = Task::new(
                Uuid::new_v4(),
                "Summary".to_string(),
                "Description".to_string(),
            ).with_priority(priority);

            assert!(task.priority <= 10);
            assert_eq!(task.priority, priority);
        }

        #[test]
        fn test_task_summary_never_empty(
            summary in "[a-zA-Z0-9 ]{1,500}"
        ) {
            let task = Task::new(
                Uuid::new_v4(),
                summary.clone(),
                "Description".to_string(),
            );

            assert!(!task.summary.is_empty());
            assert_eq!(task.summary, summary);
        }

        #[test]
        fn test_task_id_uniqueness(
            id1 in prop::uuid::any(),
            id2 in prop::uuid::any()
        ) {
            if id1 != id2 {
                let task1 = Task::new(id1, "T1".to_string(), "D1".to_string());
                let task2 = Task::new(id2, "T2".to_string(), "D2".to_string());
                assert_ne!(task1.id, task2.id);
            }
        }
    }
}
```

### Running Property Tests

```bash
# Run all property tests
cargo test proptests

# Run with more cases (default is 256)
PROPTEST_CASES=10000 cargo test proptests
```

## Testing Agents

Testing agent behavior requires special considerations.

### Mock Claude API Responses

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use mockall::predicate::*;
    use mockall::mock;

    mock! {
        ClaudeClient {}

        #[async_trait]
        impl ClaudeClientTrait for ClaudeClient {
            async fn execute_task(&self, task: &Task) -> Result<ExecutionResult>;
        }
    }

    #[tokio::test]
    async fn test_agent_executes_task() {
        let mut mock_client = MockClaudeClient::new();

        mock_client
            .expect_execute_task()
            .times(1)
            .returning(|task| {
                Ok(ExecutionResult {
                    task_id: task.id,
                    status: TaskStatus::Completed,
                    output: "Success".to_string(),
                })
            });

        let agent = Agent::new(mock_client);
        let task = Task::new(
            Uuid::new_v4(),
            "Test task".to_string(),
            "Description".to_string(),
        );

        let result = agent.execute(&task).await.unwrap();
        assert_eq!(result.status, TaskStatus::Completed);
    }
}
```

## Testing CLI Commands

Use `assert_cmd` for testing CLI behavior.

### Add Testing Dependencies

```toml
[dev-dependencies]
assert_cmd = "2.0"
predicates = "3.1"
tempfile = "3.23"
```

### Write CLI Tests

```rust
// tests/cli_tests.rs
use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::TempDir;

#[test]
fn test_cli_help() {
    let mut cmd = Command::cargo_bin("abathur").unwrap();

    cmd.arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("Abathur"))
        .stdout(predicate::str::contains("task"))
        .stdout(predicate::str::contains("swarm"));
}

#[test]
fn test_task_list_empty() {
    let temp_dir = TempDir::new().unwrap();
    let mut cmd = Command::cargo_bin("abathur").unwrap();

    cmd.env("ABATHUR_DB_PATH", temp_dir.path().join("tasks.db"))
        .arg("task")
        .arg("list")
        .assert()
        .success()
        .stdout(predicate::str::contains("No tasks found"));
}

#[test]
fn test_invalid_command() {
    let mut cmd = Command::cargo_bin("abathur").unwrap();

    cmd.arg("nonexistent")
        .assert()
        .failure()
        .stderr(predicate::str::contains("error"));
}
```

### Running CLI Tests

```bash
# Run CLI tests
cargo test --test cli_tests

# Run with binary rebuilt
cargo test --test cli_tests -- --nocapture
```

## Best Practices

### Test Organization

- **Co-locate unit tests**: Place tests in same file as implementation
- **Separate integration tests**: Use `tests/` directory for integration tests
- **Group related tests**: Use nested `mod tests` modules
- **Descriptive names**: Use `test_<what>_<condition>_<expected>` pattern

### Test Quality

- **Test one thing**: Each test should verify a single behavior
- **Arrange-Act-Assert**: Structure tests clearly
- **Use assertions**: Prefer specific assertions over `assert!()`
- **Avoid test interdependence**: Tests should run in any order
- **Clean up resources**: Use RAII or explicit cleanup

### Async Testing

```rust
#[tokio::test]
async fn test_async_function() {
    // Use tokio::test for async tests
    let result = async_operation().await;
    assert!(result.is_ok());
}

#[tokio::test]
#[should_panic(expected = "timeout")]
async fn test_timeout_behavior() {
    // Test async panic behavior
    tokio::time::timeout(
        Duration::from_secs(1),
        slow_operation()
    ).await.unwrap();
}
```

### Database Testing

```rust
// Use in-memory database for tests
async fn setup_test_db() -> SqlitePool {
    let pool = SqlitePool::connect("sqlite::memory:")
        .await
        .expect("Failed to create test database");

    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .expect("Failed to run migrations");

    pool
}

#[tokio::test]
async fn test_with_database() {
    let pool = setup_test_db().await;
    // Test code using pool
}
```

## Continuous Integration

Our CI pipeline runs all tests on every pull request.

### CI Test Commands

```bash
# Format check
cargo fmt --check

# Linting
cargo clippy --all-targets --all-features -- -D warnings

# Run all tests
cargo test --all-features

# Coverage report
cargo tarpaulin --all-features --workspace --timeout 120 --out Xml
```

### Pre-Commit Checks

Run these locally before pushing:

```bash
# Run all checks
./scripts/check.sh

# Or manually:
cargo fmt --check && \
cargo clippy --all-targets --all-features -- -D warnings && \
cargo test --all-features
```

## Troubleshooting Tests

### Test Fails with Database Lock

**Problem**: SQLite database locked during tests

**Solution**: Run tests sequentially:
```bash
cargo test -- --test-threads=1
```

### Test Timeout

**Problem**: Async test times out

**Solution**: Increase timeout or check for deadlocks:
```rust
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_with_more_threads() {
    // Test code
}
```

### Flaky Tests

**Problem**: Tests pass/fail inconsistently

**Solution**: Check for:
- Race conditions in async code
- Uninitialized state
- Time-dependent assertions
- Resource cleanup

## Related Documentation

- [Development Setup](development.md) - Setting up your environment
- [Style Guide](style-guide.md) - Code and documentation standards
- [Architecture](../explanation/architecture.md) - Understanding the system
