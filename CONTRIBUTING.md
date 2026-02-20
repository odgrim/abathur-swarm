# Contributing to Abathur

Thank you for your interest in contributing to Abathur! This guide covers everything you need to get up and running.

## Prerequisites

- **Rust 1.85+** (Rust 2024 edition): Install via [rustup](https://rustup.rs/)
  ```bash
  rustup update stable
  rustc --version  # should be >= 1.85
  ```
- **Git** (required for worktree operations and source checkout)
- **No system SQLite needed** — Abathur bundles SQLite via `libsqlite3-sys` with the `bundled` feature

For real end-to-end agent tests:
- **Claude CLI** installed and authenticated
- **`ANTHROPIC_API_KEY`** environment variable set

For cross-compilation (optional):
```bash
cargo install cross
```

## Clone & Build

```bash
git clone https://github.com/odgrim/abathur-swarm.git
cd abathur-swarm
cargo build            # debug build
cargo build --release  # optimized binary at target/release/abathur
```

## Running Tests

### Unit and integration tests (fast, no external dependencies)

```bash
cargo test
```

All unit and integration tests use an in-memory SQLite database (`sqlite::memory:`) — no file system side-effects or network connections required.

### Run a specific test file

```bash
cargo test --test integration_test               # Goal/task/memory/agent lifecycle
cargo test --test cli_integration_test           # All CLI commands (uses real binary via assert_cmd)
cargo test --test convergence_integration_test   # Convergence engine behaviour
cargo test --test e2e_swarm_integration_test     # Full swarm E2E (mock agents)
cargo test --test event_system_integration       # Event bus and reactor
cargo test --test goal_task_integration          # Goal↔task tracing
```

### Real agent E2E tests (requires Claude CLI)

These tests are marked `#[ignore]` by default and actually spawn Claude agents:

```bash
# Run only the ignored real-agent tests
ANTHROPIC_API_KEY=your_key cargo test --test e2e_swarm_integration_test -- --ignored

# Include ignored tests alongside all others
ABATHUR_REAL_E2E=1 cargo test --test e2e_swarm_integration_test -- --include-ignored
```

## Environment Variables

### LLM execution

| Variable | Required? | Purpose |
|---|---|---|
| `ANTHROPIC_API_KEY` | For agent execution | Direct Anthropic API mode and real agent execution |
| `OPENAI_API_KEY` | Optional | OpenAI embeddings adapter |

### External adapter credentials

| Variable | Required for | Purpose |
|---|---|---|
| `ABATHUR_GITHUB_TOKEN` | GitHub Issues adapter | Authenticate with GitHub API |
| `CLICKUP_API_KEY` | ClickUp adapter | Authenticate with ClickUp API |

### Config overrides (`ABATHUR_` prefix)

| Variable | Config key overridden |
|---|---|
| `ABATHUR_LIMITS_MAX_DEPTH` | `limits.max_depth` |
| `ABATHUR_DATABASE_PATH` | `database.path` |
| `ABATHUR_LOG_LEVEL` | `logging.level` |
| `ABATHUR_DEFAULT_WORKFLOW` | `default_workflow` |
| `RUST_LOG` | Log filter (tracing env-filter) |

### Multi-process / distributed mode

| Variable | CLI flag equivalent | Purpose |
|---|---|---|
| `ABATHUR_MEMORY_SERVER` | `--memory-server` | Remote memory HTTP server URL |
| `ABATHUR_TASKS_SERVER` | `--tasks-server` | Remote tasks HTTP server URL |
| `ABATHUR_A2A_GATEWAY` | `--a2a-gateway` | Agent-to-agent gateway URL |
| `ABATHUR_EVENTS_SERVER` | `--events-server` | Remote events HTTP server URL |

### Test-only

| Variable | Purpose |
|---|---|
| `ABATHUR_REAL_E2E` | Set to `1` to include real-agent `#[ignore]` tests |
| `ABATHUR_TASK_ID` | Injected by the substrate into agent subprocesses |

## Code Style and Architecture

Abathur uses **hexagonal (ports and adapters) architecture** with four layers:

```
CLI Layer       (src/cli/)       — clap commands, user interaction
     │
Service Layer   (src/services/)  — business logic, orchestration
     │ (uses ports)
Domain Layer    (src/domain/)    — pure types and trait interfaces
     │ (implemented by)
Adapter Layer   (src/adapters/)  — SQLite, LLM substrates, HTTP servers, plugins
```

### Where to add new features

| What you're adding | Where it goes |
|---|---|
| New CLI subcommand | `src/cli/commands/<name>.rs` + register in `src/cli/mod.rs` Commands enum |
| New business logic | `src/services/<name>.rs` |
| New domain entity | `src/domain/models/<name>.rs` + port trait in `src/domain/ports/<name>_repository.rs` |
| New database table | `migrations/<NNN>_<name>.sql` (embedded) + `src/adapters/sqlite/<name>_repository.rs` |
| New LLM backend | `src/adapters/substrates/<name>.rs` implementing the `Substrate` port |
| New external plugin | `src/adapters/plugins/<name>/` with `ingestion.rs`, `egress.rs`, `client.rs`, `models.rs`, `mod.rs` |
| Integration tests | `tests/<name>_test.rs` using `create_migrated_test_pool()` |
| Unit tests | Inline `#[cfg(test)]` module in the same file as the code |

### Key architectural rules

- **Domain layer is pure**: `src/domain/` contains no I/O, no async runtimes, no database queries — only types and traits
- **Dependencies flow inward**: adapters depend on domain ports; domain never imports from services or adapters
- **Error handling**: use `thiserror` for domain errors (`DomainError`); use `anyhow` in service and CLI layers
- **Async**: service and adapter layers are fully async with tokio; use `async-trait` for port traits
- **No `.unwrap()` in production code**: always propagate errors with `?` or handle explicitly
- **Structured logging**: use `tracing::{info!, debug!, warn!, error!}` — never `println!` in library code
- **Logging in development**: `RUST_LOG=abathur=debug cargo run -- swarm start`

### Running lints

```bash
cargo clippy --all-targets -- -D warnings
cargo fmt --check
```

## Pull Request Process

1. Fork the repository and create a branch from `main`
2. Make changes following the architecture guidelines above
3. Add or update tests covering your changes
4. Ensure `cargo test` passes
5. Ensure `cargo clippy --all-targets -- -D warnings` passes
6. Ensure `cargo fmt` has been applied
7. Open a PR against `main` with a clear description of the change and its motivation

For significant features or design changes, open an issue first to discuss the approach.

## Commit Messages

Use conventional commits style:

```
<type>(<scope>): <short summary under 72 chars>

<optional longer body>
```

Types: `feat`, `fix`, `refactor`, `test`, `docs`, `chore`

Examples:
```
feat(cli): add `adapter sync` subcommand for manual adapter polls
fix(memory): correct decay calculation for semantic-tier entries
refactor(convergence): extract attractor classification into separate module
docs: add CONTRIBUTING.md with setup and architecture guide
test(evolution): add integration test for regression revert
```

Reference issue numbers in the body when relevant (`Fixes #123`).
