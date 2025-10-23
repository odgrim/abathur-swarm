# Abathur

A CLI orchestration system for managing swarms of specialized Claude agents with task queues, concurrent execution, and iterative refinement.

[![Python 3.10+](https://img.shields.io/badge/python-3.10+-blue.svg)](https://www.python.org/downloads/)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)
[![Code style: black](https://img.shields.io/badge/code%20style-black-000000.svg)](https://github.com/psf/black)

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

**TUI Task Queue Visualizer**
- Interactive terminal UI for task visualization
- Multiple view modes (tree, dependency, timeline, feature branch)
- Real-time task filtering and search
- Hierarchical task tree rendering
- Color-coded status indicators

---

## Requirements

- **Python**: 3.10 or higher
- **Git**: For template cloning
- **Anthropic API Key**: For Claude access

---

## Installation

### From Source

```bash
git clone https://github.com/yourorg/abathur.git
cd abathur
poetry install
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

### TUI Task Visualizer

```bash
# Launch interactive TUI
abathur tui

# TUI supports multiple view modes:
# - Tree view (parent-child relationships)
# - Dependency view (prerequisite relationships)
# - Timeline view (chronological)
# - Feature branch view (grouped by feature)
# - Flat list view

# Interactive features:
# - f: Toggle filter modal
# - /: Search tasks
# - ↑↓: Navigate tasks
# - Enter: Expand/collapse task details
# - q: Quit
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

# Install with development dependencies
poetry install

# Install pre-commit hooks
pre-commit install
```

### Testing

```bash
# Run all tests
pytest

# Run with coverage
pytest --cov=abathur --cov-report=html

# Run specific test file
pytest tests/integration/test_database.py

# Run with verbose output
pytest -v
```

### Linting & Formatting

```bash
# Type checking
mypy src/abathur

# Linting
ruff check src/ tests/

# Formatting
black src/ tests/

# Run all pre-commit hooks
pre-commit run --all-files
```

### Running Locally

```bash
# Use module invocation (workaround for entry point issue)
python -m abathur.cli.main --help

# Or install in editable mode
poetry install
abathur --help
```

---

## Documentation

- **[User Guide](docs/USER_GUIDE.md)**: Comprehensive usage guide
- **[TUI Visualizer Guide](docs/TUI_GUIDE.md)**: Interactive terminal UI documentation
- **[API Reference](docs/API_REFERENCE.md)**: Python API documentation
- **[Architecture](design_docs/prd_deliverables/03_ARCHITECTURE.md)**: System architecture
- **[System Design](design_docs/prd_deliverables/04_SYSTEM_DESIGN.md)**: Algorithms and protocols
- **[Implementation Roadmap](design_docs/prd_deliverables/08_IMPLEMENTATION_ROADMAP.md)**: Development phases

---

## Project Status

This is a working system with the following components implemented:

- SQLite database with WAL mode
- Configuration system with hierarchy
- Structured logging with audit trails
- Domain models with Pydantic validation
- Template Manager (Git-based cloning, caching, validation)
- Task Coordinator (priority queue, retry logic)
- Claude Client (async API, retry with backoff)
- Agent Executor (YAML-based agents, lifecycle management)
- Swarm Orchestrator
- Agent Pool (dynamic lifecycle, health monitoring)
- Resource Monitor (CPU/memory tracking, limits)
- Loop Executor (iterative refinement, convergence detection, checkpointing)
- MCP Manager (server lifecycle, health monitoring, auto-restart)
- CLI with rich output
- TUI Task Visualizer (interactive terminal UI, multiple view modes, filtering)

---

## Performance

- Task Scheduling: O(log n) with indexed queries
- Dependency Check: O(d) per task
- Concurrent Agents: Configurable limit
- Database: SQLite with WAL mode

---

## Known Issues

### CLI Entry Point

**Status**: Documented, workaround available

**Issue**: Entry point may not work with some Typer versions

**Workaround**:
```bash
python -m abathur.cli.main <command>
```

**Priority**: Low (all functionality works via workaround)

---

## Contributing

Contributions are welcome! Please:

1. Fork the repository
2. Create a feature branch
3. Add tests for new functionality
4. Ensure all tests pass
5. Submit a pull request

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
