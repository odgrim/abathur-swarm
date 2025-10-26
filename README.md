# Abathur

A CLI orchestration system for managing swarms of specialized Claude agents with task queues, concurrent execution, and iterative refinement.

[![Rust 1.83+](https://img.shields.io/badge/rust-1.83+-orange.svg)](https://www.rust-lang.org/)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)
[![CI](https://img.shields.io/github/workflow/status/yourorg/abathur/CI)](https://github.com/yourorg/abathur/actions)

---

## Features

**Task Queue Management**
- Priority-based queue with task dependencies
- SQLite persistence with WAL mode
- Automatic retry with exponential backoff
- Task cancellation support

**Concurrent Agent Swarms**
- Multiple Claude agents running simultaneously
- Semaphore-based concurrency control
- Dynamic agent lifecycle management
- Health monitoring

**Iterative Refinement Loops**
- Multiple convergence strategies
- Automatic checkpointing and crash recovery
- Configurable iteration limits and timeouts

**MCP Integration**
- MCP server lifecycle management
- Agent-to-server binding
- Health monitoring with auto-restart

**Observability**
- Structured logging with audit trails
- Rich CLI output with tables and progress bars
- Resource and failure statistics

**Task Tree Visualization**
- Hierarchical task tree rendering in CLI
- Color-coded status indicators
- Unicode/ASCII box-drawing support
- Parent-child task relationships
- Priority-based sorting

---

## Requirements

- **Rust**: 1.83 or higher (install via [rustup](https://rustup.rs/))
- **SQLite**: For database operations (usually pre-installed)
- **Git**: For version control
- **Anthropic API Key**: For Claude access (optional for core development)

---

## Installation

### From Source (Recommended)

```bash
# Clone the repository
git clone https://github.com/yourorg/abathur.git
cd abathur

# Build the project
cargo build --release

# Install locally
cargo install --path .
```

### Using Cargo

```bash
cargo install abathur
```

### For Development

```bash
# Clone repository
git clone https://github.com/yourorg/abathur.git
cd abathur

# Build with development dependencies
cargo build

# Run tests
cargo test
```

---

## Quick Start

### 1. Initialize Project

```bash
# Initialize database and configuration
abathur init

# Set your Anthropic API key via environment variable
export ANTHROPIC_API_KEY=YOUR_API_KEY
```

### 2. Configure Templates

Templates are configured in `.abathur/config.yaml`. The default template is automatically configured:

```yaml
# .abathur/config.yaml
template_repos:
  - url: https://github.com/odgrim/abathur-claude-template.git
    version: main
```

Run `abathur init` to install templates.

### 3. Submit & Execute Tasks

```bash
# Submit a task (via MCP task_enqueue)
# With optional summary for quick identification
mcp_client.call_tool("task_enqueue", {
    "description": "Implement user authentication with JWT tokens and OAuth2 support",
    "source": "human",
    "agent_type": "python-backend-specialist",
    "summary": "Add user authentication to API",  # Optional: brief summary (max 500 chars)
    "base_priority": 8
})

# List tasks
abathur task list --status pending

# Start swarm to process tasks
abathur swarm start --max-agents 10

# Monitor task queue status
abathur task status
```

### 4. Use Loop Execution

```bash
# Execute task with iterative refinement
abathur loop start <task-id> \
  --max-iterations 10 \
  --convergence-threshold 0.95
```

---

## Architecture

Abathur follows **Clean Architecture** principles with clear layer separation:

```
┌──────────────────────────────────────────┐
│          CLI Layer (Typer + Rich)        │
│  20+ commands with rich terminal output  │
└────────────────┬─────────────────────────┘
                 │
┌────────────────▼─────────────────────────┐
│        Application Services Layer        │
│                                          │
│  • SwarmOrchestrator                     │
│  • LoopExecutor                          │
│  • TaskCoordinator                       │
│  • AgentExecutor                         │
│  • TemplateManager                       │
│  • MCPManager                            │
│  • ResourceMonitor                       │
│  • AgentPool                             │
└────────────────┬─────────────────────────┘
                 │
┌────────────────▼─────────────────────────┐
│          Domain Models Layer             │
│  Task, Agent, Result, ExecutionContext   │
└────────────────┬─────────────────────────┘
                 │
┌────────────────▼─────────────────────────┐
│       Infrastructure Layer               │
│  • Database (SQLite + WAL)               │
│  • ConfigManager (Hierarchical)          │
│  • Logger (Structlog)                    │
│  • MCPConfigLoader                       │
│  • ClaudeClient (Anthropic SDK)          │
└──────────────────────────────────────────┘
```

### Design Patterns

- Priority Queue: Task scheduling with dependency resolution
- Semaphore Control: Concurrent agent execution with resource limits
- Exponential Backoff: Retry with jitter for transient errors
- Checkpoint/Resume: Crash-resistant loop execution

---

## CLI Commands

### Task Management

```bash
abathur task submit <template> [--input-file FILE] [--priority 0-10]
abathur task list [--status STATUS] [--limit N]
abathur task list --tree         # Show tasks as hierarchical tree
abathur task show <task-id>
abathur task status              # Show task queue statistics
abathur task cancel <task-id>
abathur task retry <task-id>
```

### Swarm Orchestration

```bash
abathur swarm start [--task-limit N] [--max-agents N]
abathur swarm status
```

### Loop Execution

```bash
abathur loop start <task-id> [--max-iterations N] [--convergence-threshold F]
```

### Template Management

```bash
abathur init          # Install configured templates
```

### MCP Management

```bash
abathur mcp list
abathur mcp start <server>
abathur mcp stop <server>
abathur mcp restart <server>
```

### Task Tree Visualization

```bash
# Show tasks as hierarchical tree
abathur task list --tree

# Filter by status
abathur task list --tree --status pending
abathur task list --tree --status running

# Combine with other filters
abathur task list --tree --status pending --limit 20

# Feature branch task overview
abathur feature-branch summary <branch-name>
abathur feature-branch list
```

---

## Configuration

### Hierarchical Configuration

Abathur uses a 4-level configuration hierarchy:

1. **Template defaults**: `.abathur/config.yaml`
2. **User overrides**: `~/.abathur/config.yaml`
3. **Project overrides**: `.abathur/local.yaml`
4. **Environment variables**: `ABATHUR_*`

### Example Configuration

```yaml
version: "1.0"
log_level: INFO

template_repos:
  - url: https://github.com/odgrim/abathur-claude-template.git
    version: main

swarm:
  max_concurrent_agents: 10
  agent_spawn_timeout: 5
  agent_idle_timeout: 300

retry:
  max_retries: 3
  initial_backoff: 10        # seconds
  max_backoff: 300           # 5 minutes
  backoff_multiplier: 2.0
  jitter: true
```

### MCP Configuration

Create `.mcp.json` in your project root:

```json
{
  "mcpServers": {
    "filesystem": {
      "command": "npx",
      "args": ["-y", "@modelcontextprotocol/server-filesystem", "/allowed/path"],
      "env": {}
    },
    "github": {
      "command": "npx",
      "args": ["-y", "@modelcontextprotocol/server-github"],
      "env": {
        "GITHUB_TOKEN": "${GITHUB_TOKEN}"
      }
    }
  }
}
```

---

## Development

### Setup

```bash
# Clone repository
git clone https://github.com/yourorg/abathur.git
cd abathur

# Install Rust toolchain (if not already installed)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Install development tools
rustup component add rustfmt clippy

# Build the project
cargo build
```

### Building

```bash
# Debug build (fast compilation, slower runtime)
cargo build

# Release build (optimized, slower compilation)
cargo build --release

# Run without installing
cargo run -- <command>

# Example: List tasks
cargo run -- task list
```

### Testing

```bash
# Run all tests
cargo test

# Run tests with output
cargo test -- --nocapture

# Run specific test
cargo test test_task_creation

# Run tests for specific module
cargo test domain::models

# Run with coverage (requires cargo-tarpaulin)
cargo install cargo-tarpaulin
cargo tarpaulin --all-features --workspace --timeout 120 --out Html
```

### Linting & Formatting

```bash
# Format code
cargo fmt

# Check formatting without making changes
cargo fmt --check

# Run clippy (linter)
cargo clippy --all-targets --all-features

# Clippy with warnings as errors
cargo clippy --all-targets --all-features -- -D warnings

# Check all (format, clippy, test)
cargo fmt --check && cargo clippy --all-targets --all-features -- -D warnings && cargo test
```

### Running Locally

```bash
# Run in debug mode
cargo run -- --help

# Run with logging
RUST_LOG=debug cargo run -- task list

# Run with trace logging for specific module
RUST_LOG=abathur::domain::task=trace cargo run -- task show <id>
```

### Benchmarking

```bash
# Run all benchmarks
cargo bench

# Run specific benchmark
cargo bench task_queue

# Generate benchmark report
cargo bench -- --save-baseline main
```

---

## Documentation

- **[Contributing Guide](CONTRIBUTING.md)**: Development setup and guidelines
- **[Architecture](docs/ARCHITECTURE.md)**: System architecture and design patterns
- **[API Documentation](https://docs.rs/abathur)**: Rust API documentation (cargo doc)
- **[User Guide](docs/USER_GUIDE.md)**: Comprehensive usage guide

### Generating Documentation

```bash
# Generate and open API documentation
cargo doc --open

# Generate documentation for all dependencies
cargo doc --document-private-items --open
```

---

## Project Status

**Rust Rewrite in Progress** - This project is being rewritten from Python to Rust for improved performance, type safety, and concurrency.

### Implemented Components

**Phase 1: Foundation** ✅
- Cargo project structure with Clean Architecture
- Domain models (Task, Agent, ExecutionResult)
- Error types with thiserror
- SQLite database layer with sqlx
- Configuration management with figment

**Phase 2: In Progress** 🚧
- Service layer implementations
- Repository pattern with async traits
- Port definitions for hexagonal architecture

**Phase 3: Planned** 📋
- CLI layer with clap
- Application orchestration services
- Swarm coordination
- MCP integration

### Python Legacy Components (Maintained)

The Python implementation remains functional with all features:
- SQLite database with WAL mode
- Configuration system with hierarchy
- Structured logging with audit trails
- Task Coordinator (priority queue, retry logic)
- Swarm Orchestrator
- MCP Manager
- CLI with rich output

---

## Performance

- Task Scheduling: O(log n) with indexed queries
- Dependency Check: O(d) per task
- Concurrent Agents: Configurable limit
- Database: SQLite with WAL mode

---

## Contributing

Contributions are welcome! Please see [CONTRIBUTING.md](CONTRIBUTING.md) for detailed guidelines.

**Quick Start**:

1. Fork the repository
2. Create a feature branch (`git checkout -b feature/amazing-feature`)
3. Make your changes following the style guide
4. Add tests for new functionality
5. Ensure all checks pass:
   ```bash
   cargo fmt --check
   cargo clippy --all-targets --all-features -- -D warnings
   cargo test --all-features
   ```
6. Commit your changes (`git commit -m 'Add amazing feature'`)
7. Push to your branch (`git push origin feature/amazing-feature`)
8. Open a Pull Request

See [CONTRIBUTING.md](CONTRIBUTING.md) for complete development workflow, code style guidelines, and testing requirements.

---

## License

MIT License - see [LICENSE](LICENSE) file for details

---

## Acknowledgments

- Built with [Anthropic Claude](https://www.anthropic.com/)
- Inspired by StarCraft II's Abathur character
- Clean Architecture principles by Robert C. Martin

---

## Support

- **Documentation**: `docs/`
- **Issues**: [GitHub Issues](https://github.com/yourorg/abathur/issues)
- **Discussions**: [GitHub Discussions](https://github.com/yourorg/abathur/discussions)

---

**Version**: 0.1.0
