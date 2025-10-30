# Abathur Swarm

**Intelligent CLI orchestration for autonomous agent swarms**

Abathur is a Rust-powered CLI system for managing swarms of specialized Claude agents with hierarchical task queues, concurrent execution, and iterative refinement. Build sophisticated agentic workflows with dependency management, persistent state, and seamless MCP integration.

---

## Quick Start

Get up and running in minutes:

```bash
# Install from source
git clone https://github.com/yourorg/abathur.git
cd abathur
cargo install --path .

# Initialize your project
abathur init
export ANTHROPIC_API_KEY=YOUR_API_KEY

# Start orchestrating agents
abathur task list
abathur swarm start --max-agents 10
```

[Get Started →](tutorials/quickstart.md){ .md-button .md-button--primary }
[Architecture Overview →](explanation/architecture.md){ .md-button }

---

## Key Features

### 🎯 Hierarchical Task Queue
Priority-based task scheduling with dependency resolution, automatic retry logic, and persistent state management using SQLite with WAL mode.

[Learn More →](explanation/task-queue.md)

### 🤖 Autonomous Agent Swarm
Deploy multiple Claude agents concurrently with semaphore-based resource control, dynamic lifecycle management, and health monitoring.

[Explore Agents →](reference/agent-types.md)

### 🧠 Memory System
Persistent memory with semantic, episodic, and procedural storage patterns. Enable agents to learn, adapt, and build on previous work.

[Memory Guide →](how-to/memory-management.md)

### 🔌 MCP Integration
Native Model Context Protocol support with server lifecycle management, agent-to-server binding, and automatic health monitoring.

[MCP Setup →](how-to/mcp-integration.md)

### 🔄 Iterative Refinement
Execute tasks with configurable convergence strategies, automatic checkpointing, and crash recovery for complex multi-step workflows.

[Loop Execution →](reference/loop-execution.md)

### 📊 Rich Observability
Structured logging with audit trails, beautiful terminal output with tables and progress bars, and comprehensive resource monitoring.

[Monitoring →](how-to/monitoring.md)

---

## Documentation Structure

Our documentation follows the [Diátaxis framework](https://diataxis.fr/) for clarity and ease of navigation:

<div class="grid cards" markdown>

-   :material-school:{ .lg .middle } __Tutorials__

    ---

    Step-by-step learning paths for beginners. Start here if you're new to Abathur.

    [:octicons-arrow-right-24: Getting Started](tutorials/index.md)

-   :material-wrench:{ .lg .middle } __How-To Guides__

    ---

    Problem-oriented recipes for accomplishing specific tasks and workflows.

    [:octicons-arrow-right-24: Practical Guides](how-to/index.md)

-   :material-book-open-variant:{ .lg .middle } __Reference__

    ---

    Comprehensive technical details on CLI commands, configuration, and APIs.

    [:octicons-arrow-right-24: Technical Reference](reference/index.md)

-   :material-lightbulb:{ .lg .middle } __Explanation__

    ---

    Conceptual deep-dives into architecture, design patterns, and system internals.

    [:octicons-arrow-right-24: Understanding Abathur](explanation/index.md)

</div>

---

## Why Abathur?

Abathur combines the power of Claude's capabilities with robust orchestration:

- **Type-Safe**: Built in Rust for performance, safety, and reliability
- **Scalable**: Handle complex multi-agent workflows with confidence
- **Persistent**: Never lose work with automatic state management
- **Observable**: Rich logging and monitoring for production deployments
- **Extensible**: Plugin architecture with MCP support

!!! tip "Coming from Python?"
    Abathur is being rewritten from Python to Rust. The Python implementation remains functional and feature-complete. See our [migration guide](how-to/python-migration.md) for transition details.

---

## Project Status

**Active Development** | Rust Rewrite in Progress

- ✅ **Phase 1 Complete**: Domain models, database layer, configuration system
- 🚧 **Phase 2 In Progress**: Service layer, repository pattern, async infrastructure
- 📋 **Phase 3 Planned**: CLI layer, orchestration services, full MCP integration

[View Roadmap →](explanation/roadmap.md) | [Contributing →](contributing.md)

---

## Community & Support

<div class="grid cards" markdown>

-   :material-chat-question:{ .lg .middle } __Questions?__

    ---

    Join our GitHub Discussions for help and community support.

    [:octicons-arrow-right-24: Ask Questions](https://github.com/yourorg/abathur/discussions)

-   :material-bug:{ .lg .middle } __Found a Bug?__

    ---

    Report issues and track development on GitHub Issues.

    [:octicons-arrow-right-24: Report Issue](https://github.com/yourorg/abathur/issues)

-   :material-code-braces:{ .lg .middle } __Want to Contribute?__

    ---

    Check out our contributing guide and development setup.

    [:octicons-arrow-right-24: Contributing Guide](contributing.md)

-   :material-license:{ .lg .middle } __License__

    ---

    Abathur is open source under the MIT License.

    [:octicons-arrow-right-24: View License](https://github.com/yourorg/abathur/blob/main/LICENSE)

</div>

---

## Quick Links

- [Installation Guide](tutorials/installation.md) - Detailed setup instructions
- [First Task Tutorial](tutorials/first-task.md) - Submit and execute your first task
- [CLI Command Reference](reference/cli-commands.md) - Complete command documentation
- [Architecture Overview](explanation/architecture.md) - System design and patterns
- [Configuration Reference](reference/configuration.md) - Comprehensive config options

---

!!! info "Documentation Version"
    This documentation covers Abathur version **0.1.0** (Rust Edition). Last updated: October 29, 2025.

*Built with [MkDocs Material](https://squidfunk.github.io/mkdocs-material/)*
