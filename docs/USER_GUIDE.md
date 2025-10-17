# Abathur User Guide

## Table of Contents

1. [Installation](#installation)
2. [Quick Start](#quick-start)
3. [Core Concepts](#core-concepts)
4. [Task Management](#task-management)
5. [Swarm Orchestration](#swarm-orchestration)
6. [Loop Execution](#loop-execution)
7. [Template Management](#template-management)
8. [MCP Integration](#mcp-integration)
9. [Monitoring & Recovery](#monitoring--recovery)
10. [Configuration](#configuration)
11. [Best Practices](#best-practices)
12. [Troubleshooting](#troubleshooting)

---

## Installation

### From PyPI

```bash
pip install abathur
```

### From Source

```bash
git clone https://github.com/yourorg/abathur.git
cd abathur
poetry install
```

### Docker

```bash
docker pull abathur/abathur:latest
docker run -it abathur/abathur:latest abathur version
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

Templates are configured in `.abathur/config.yaml`. The default template is already configured, but you can add more:

```yaml
# .abathur/config.yaml
template_repos:
  - url: https://github.com/odgrim/abathur-claude-template.git
    version: main
```

Run `abathur init` to install templates.

### 3. Submit a Task

```bash
# Submit a task with JSON input
abathur task submit my-agent --input-file input.json --priority 8

# Check tasks
abathur task list
```

### 4. Start Swarm

```bash
# Start swarm to process tasks
abathur swarm start --max-agents 10

# Monitor status
abathur status
```

---

## Core Concepts

### Tasks

Tasks are units of work submitted to Abathur for execution by specialized Claude agents.

**Properties:**
- `template_name`: Agent template to use
- `input_data`: JSON input for the agent
- `priority`: 0-10 scale (10 = highest)
- `max_retries`: Maximum retry attempts
- `dependencies`: Task IDs that must complete first

**Lifecycle:**
```
PENDING → RUNNING → COMPLETED
                  ↘ FAILED → (retry) → PENDING
```

### Agents

Agents are instances of Claude spawned from templates to execute tasks.

**States:**
- `SPAWNING`: Agent process starting
- `IDLE`: Ready for task assignment
- `BUSY`: Executing assigned task
- `TERMINATED`: Agent destroyed
- `FAILED`: Agent crashed or exceeded limits

### Swarm Orchestration

The swarm orchestrator manages concurrent agent execution with:
- Semaphore-based concurrency control (10+ agents)
- Priority-based task queue
- Resource monitoring

---

## Task Management

### Submit Tasks

```bash
# Simple task
abathur task submit analyzer --priority 5

# Task with JSON input
abathur task submit processor --input-file data.json --priority 8

# High-priority task
abathur task submit urgent-task --priority 10
```

### List & Filter Tasks

```bash
# List all tasks
abathur task list

# Filter by status
abathur task list --status pending
abathur task list --status running
abathur task list --status failed

# Limit results
abathur task list --limit 50
```

### Task Details

```bash
# Get detailed task information
abathur task show <task-id>

# Output shows:
# - Template name
# - Priority
# - Status
# - Timestamps (submitted, started, completed)
# - Error messages (if failed)
```

### Cancel & Retry

```bash
# Cancel a pending/running task
abathur task cancel <task-id>

# Retry a failed task
abathur task retry <task-id>
```

---

## Swarm Orchestration

### Start Swarm

```bash
# Start swarm with default settings (10 concurrent agents)
abathur swarm start

# Limit tasks processed
abathur swarm start --task-limit 100

# Adjust concurrent agents
abathur swarm start --max-agents 20
```

### Monitor Swarm

```bash
# Get current swarm status
abathur swarm status

# Monitor system status
abathur status
```

---

## Loop Execution

Loop execution enables iterative refinement with convergence detection.

### Start Loop

```bash
# Execute task with loop
abathur loop start <task-id> \
  --max-iterations 10 \
  --convergence-threshold 0.95
```

### Convergence Criteria

**Threshold-based:**
```python
criteria = ConvergenceCriteria(
    type=ConvergenceType.THRESHOLD,
    metric_name="accuracy",
    threshold=0.95,
    direction="maximize"
)
```

**Stability-based:**
```python
criteria = ConvergenceCriteria(
    type=ConvergenceType.STABILITY,
    stability_window=3,
    similarity_threshold=0.95
)
```

### Checkpoint & Resume

Loop execution automatically checkpoints state every iteration:

```python
# Checkpoint saved to database
# Automatically restored on crash/restart
result = await loop_executor.execute_loop(
    task, criteria, max_iterations=10, checkpoint_interval=1
)
```

---

## Template Management

Templates are configured in the `.abathur/config.yaml` file under the `template_repos` field. Multiple templates can be specified and will be installed in order when running `abathur init`.

### Configure Templates

Edit `.abathur/config.yaml` to configure template repositories:

```yaml
template_repos:
  - url: https://github.com/org/template.git
    version: main
  - url: https://github.com/org/another-template.git
    version: v1.0.0
```

### Install Templates

Templates are automatically installed when you run `abathur init`:

```bash
# Initialize database and install configured templates
abathur init

# Skip template installation (only init database)
abathur init --skip-template
```

### View Configured Templates

Check the `.abathur/config.yaml` file to see configured templates:

```yaml
template_repos:
  - url: https://github.com/org/template.git
    version: main
  - url: https://github.com/org/another-template.git
    version: v1.0.0
```

### Template Structure

```
template-name/
├── agent.yaml          # Agent definition
├── system_prompt.md    # System prompt
├── README.md           # Documentation
└── examples/           # Example inputs
    └── example1.json
```

**agent.yaml:**
```yaml
name: analyzer
description: Code analysis agent
model: claude-sonnet-4
specialization: code_analysis
temperature: 0.0
max_tokens: 4096
```

---

## MCP Integration

### List MCP Servers

```bash
# List configured MCP servers
abathur mcp list
```

### Start/Stop Servers

```bash
# Start MCP server
abathur mcp start filesystem

# Stop MCP server
abathur mcp stop filesystem

# Restart MCP server
abathur mcp restart filesystem
```

### Configuration

Create `.mcp.json` in project root:

```json
{
  "mcpServers": {
    "filesystem": {
      "command": "npx",
      "args": ["-y", "@modelcontextprotocol/server-filesystem", "/path/to/allowed/dir"],
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

## Monitoring & Recovery

---

## Configuration

### Configuration Files

Abathur uses hierarchical configuration:

1. **Template defaults**: `.abathur/config.yaml`
2. **User overrides**: `~/.abathur/config.yaml`
3. **Project overrides**: `.abathur/local.yaml`
4. **Environment variables**: `ABATHUR_*`

### Example Configuration

```yaml
version: "1.0"
log_level: INFO

swarm:
  max_concurrent_agents: 10
  agent_spawn_timeout: 5
  agent_idle_timeout: 300

retry:
  max_retries: 3
  initial_backoff: 10        # seconds
  max_backoff: 300           # seconds
  backoff_multiplier: 2.0
```

### Set API Key

Set your Anthropic API key via environment variable:

```bash
# Set as environment variable
export ANTHROPIC_API_KEY=YOUR_API_KEY

# Or add to your shell profile (.bashrc, .zshrc, etc.)
echo 'export ANTHROPIC_API_KEY=YOUR_API_KEY' >> ~/.zshrc
```

### Validate Configuration

Check your configuration files manually:
- `.abathur/config.yaml` - Template defaults
- `~/.abathur/config.yaml` - User overrides
- `.abathur/local.yaml` - Project overrides

---

## Best Practices

### Task Submission

1. **Use appropriate priorities**: Reserve 10 for truly urgent tasks
2. **Provide clear input data**: Well-structured JSON improves agent performance
3. **Set reasonable max_retries**: Avoid infinite retry loops
4. **Use dependencies**: For tasks that must execute in order

### Swarm Configuration

1. **Start with 10 concurrent agents**: Adjust based on resource availability
2. **Monitor resource usage**: Ensure CPU/memory stay under limits

### Template Development

1. **Clear system prompts**: Be specific about agent behavior
2. **Include examples**: Help agents understand expected input/output
3. **Version templates**: Use Git tags for versioning
4. **Add to config**: Configure templates in `.abathur/config.yaml`

### Loop Execution

1. **Choose appropriate convergence criteria**: Match to your use case
2. **Set reasonable max iterations**: Avoid runaway loops
3. **Use checkpointing**: Enable recovery from crashes
4. **Monitor convergence**: Review iteration history

---

## Troubleshooting

### Common Issues

**Issue: CLI command not found**
```bash
# Workaround: Use module invocation
python -m abathur.cli.main <command>

# Or reinstall
poetry install
```

**Issue: Database locked**
```bash
# WAL mode should prevent this, but if it occurs:
# Stop all Abathur processes
# Remove .db-wal and .db-shm files
# Restart
```

**Issue: Agent spawn timeout**
```bash
# Increase timeout in config
# Check Claude API connectivity
# Verify API key is valid
```

**Issue: High memory usage**
```bash
# Reduce max_concurrent_agents
# Check with: abathur swarm status
```

**Issue: Tasks stuck in RUNNING**
```bash
# Check task details: abathur task show <task-id>
# Check for stale tasks: abathur task check-stale
# Or manually retry: abathur task retry <task-id>
```

### Debug Mode

Enable debug logging:

```bash
# Set in config.yaml
log_level: DEBUG

# Or via environment variable
export ABATHUR_LOG_LEVEL=DEBUG
```

### Getting Help

- Documentation: `docs/`
- API Reference: `docs/API_REFERENCE.md`
- Issues: GitHub Issues
- Community: Discord/Slack

---

## Next Steps

1. **Read API Reference**: `docs/API_REFERENCE.md`
2. **Explore Examples**: `examples/`
3. **Develop Templates**: Create custom agent templates
4. **Scale Up**: Tune for your workload

---

**Version:** 0.1.0
**Last Updated:** 2025-10-09
