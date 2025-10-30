# Abathur

**AI-Powered Task Orchestration with Intelligent Agent Swarms**

Abathur is a powerful CLI orchestration system that manages swarms of specialized Claude agents, enabling concurrent task execution with dependency management, iterative refinement, and comprehensive observability.

---

## What is Abathur?

Abathur transforms complex software development workflows into manageable, automated tasks executed by intelligent AI agents. Built with Clean Architecture principles and written in Rust, it provides a robust foundation for orchestrating multiple Claude agents working together on sophisticated projects.

Whether you're building features, refactoring code, or managing intricate task dependencies, Abathur coordinates specialized agents that work in parallel, retry on failure, and converge on optimal solutions through iterative refinement.

At its core, Abathur combines three powerful capabilities: a priority-based task queue with dependency resolution, a swarm orchestration engine for concurrent agent execution, and an integrated memory system for context preservation across tasks.

---

## Key Features

### :material-format-list-checks: **Task Queue Management**
Priority-based queue with full dependency support, SQLite persistence with WAL mode, automatic retry with exponential backoff, and task cancellation capabilities. Visualize task hierarchies with the built-in tree view.

### :material-robot-outline: **Concurrent Agent Swarms**
Run multiple Claude agents simultaneously with semaphore-based concurrency control, dynamic lifecycle management, and health monitoring. Scale your automation with configurable agent limits.

### :material-refresh: **Iterative Refinement Loops**
Execute tasks with multiple convergence strategies, automatic checkpointing for crash recovery, and configurable iteration limits. Let agents refine their work until quality thresholds are met.

### :material-connection: **MCP Integration**
Full MCP (Model Context Protocol) server lifecycle management, agent-to-server binding, and health monitoring with auto-restart. Extend capabilities with filesystem, GitHub, and custom MCP servers.

### :material-chart-line: **Comprehensive Observability**
Structured logging with complete audit trails, rich CLI output with tables and progress bars, and detailed resource and failure statistics. Know exactly what's happening at every stage.

### :material-file-tree: **Task Tree Visualization**
Hierarchical task tree rendering in the CLI with color-coded status indicators, Unicode/ASCII box-drawing support, and priority-based sorting. Understand complex task relationships at a glance.

---

## Quick Start

Get started with Abathur in under 5 minutes:

```bash
# Install from source
git clone https://github.com/yourorg/abathur.git
cd abathur
cargo install --path .

# Initialize project
abathur init

# Set your Anthropic API key
export ANTHROPIC_API_KEY=YOUR_API_KEY

# Submit your first task
abathur task enqueue \
  --summary "Implement user authentication" \
  --agent-type "rust-backend-specialist" \
  --priority 8

# Start the swarm
abathur swarm start --max-agents 5

# Watch your tasks execute
abathur task list --tree
```

---

## Documentation Sections

<div class="grid cards" markdown>

-   :material-rocket-launch:{ .lg .middle } **Getting Started**

    ---

    Installation guides, quickstart tutorials, and your first task walkthrough.

    [:octicons-arrow-right-24: Get Started](getting-started/installation.md)

-   :material-book-open-variant:{ .lg .middle } **Tutorials**

    ---

    Step-by-step guides for common workflows and hands-on learning exercises.

    [:octicons-arrow-right-24: Learn](getting-started/first-task.md)

-   :material-tools:{ .lg .middle } **How-To Guides**

    ---

    Goal-oriented recipes for task management, troubleshooting, and advanced features.

    [:octicons-arrow-right-24: Solve Problems](how-to/task-management.md)

-   :material-code-braces:{ .lg .middle } **CLI Reference**

    ---

    Complete command documentation with all options, flags, and examples.

    [:octicons-arrow-right-24: Reference](reference/cli-commands.md)

-   :material-cog:{ .lg .middle } **Configuration**

    ---

    Hierarchical configuration system, MCP setup, and environment variables.

    [:octicons-arrow-right-24: Configure](reference/configuration.md)

-   :material-layers:{ .lg .middle } **Architecture**

    ---

    System design, Clean Architecture principles, and design patterns.

    [:octicons-arrow-right-24: Understand](explanation/architecture.md)

</div>

---

## Why Abathur?

**Intelligent Orchestration**: Abathur isn't just a task runner—it's an intelligent orchestration system that understands task dependencies, manages agent lifecycles, and ensures work converges on quality solutions through iterative refinement.

**Built for Scale**: With semaphore-based concurrency control and efficient SQLite persistence with WAL mode, Abathur scales from single tasks to complex multi-agent workflows processing dozens of concurrent operations.

**Developer-Friendly**: Rich CLI output with tables, progress bars, and tree visualizations makes monitoring straightforward. Structured logging with audit trails ensures you can track every decision and action.

**Rust Performance**: The ongoing Rust rewrite brings type safety, fearless concurrency, and blazing performance to AI agent orchestration. Built with Clean Architecture principles for maintainability and extensibility.

---

## Project Status

!!! info "Rust Rewrite in Progress"
    Abathur is being rewritten from Python to Rust for improved performance, type safety, and concurrency. The Python implementation remains fully functional while the Rust version is being completed.

**Current Phase**: Phase 2 - Service Layer Implementation

- ✅ **Phase 1**: Foundation (domain models, error types, database, configuration)
- 🚧 **Phase 2**: Service layer, repositories, ports (In Progress)
- 📋 **Phase 3**: CLI layer, orchestration, swarm coordination (Planned)
- 📋 **Phase 4**: MCP integration, advanced features (Planned)

See [Architecture](explanation/architecture.md) for complete implementation roadmap.

---

## Next Steps

Ready to dive in? Here's your path forward:

1. **[Install Abathur](getting-started/installation.md)** - Set up your environment in minutes
2. **[Run Your First Task](getting-started/first-task.md)** - Submit and execute a simple task
3. **[Explore Task Management](how-to/task-management.md)** - Learn to manage dependencies and priorities
4. **[Understand the Architecture](explanation/architecture.md)** - Dive deep into system design

!!! tip "Need Help?"
    - Check the [Troubleshooting Guide](how-to/troubleshooting.md) for common issues
    - Browse [GitHub Discussions](https://github.com/yourorg/abathur/discussions) for community support
    - Review [CLI Reference](reference/cli-commands.md) for complete command documentation

---

## Contributing

Abathur is open source and welcomes contributions! Whether you're fixing bugs, adding features, improving documentation, or sharing feedback, your contributions make Abathur better for everyone.

See our [Contributing Guide](contributing/contributing.md) for development setup, guidelines, and workflow.

---

## License

Abathur is released under the [MIT License](https://opensource.org/licenses/MIT).

---

**Built with** [Anthropic Claude](https://www.anthropic.com/) • **Inspired by** StarCraft II's Abathur character
