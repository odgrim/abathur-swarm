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

# Set your Anthropic API key
abathur config set-key YOUR_API_KEY
```

### 2. Install a Template

```bash
# Clone a template from Git
abathur template install https://github.com/org/agent-template.git

# List installed templates
abathur template list
```

### 3. Submit a Task

```bash
# Submit a task with JSON input
abathur task submit my-agent --input-file input.json --priority 8

# Check task status
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
                                     ↘ DLQ
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
- Automatic failure recovery
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

### Task Status

```bash
# Get detailed task status
abathur task status <task-id>

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

# Watch resource usage
abathur resources
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

### Install Templates

```bash
# From Git repository
abathur template install https://github.com/org/template.git

# Specific version/branch
abathur template install https://github.com/org/template.git --version v1.0.0
```

### List Templates

```bash
# List installed templates
abathur template list
```

### Validate Templates

```bash
# Validate template structure
abathur template validate my-agent

# Output shows:
# - Valid/Invalid status
# - Validation errors (if any)
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

### Resource Monitoring

```bash
# Check resource usage
abathur resources

# Output shows:
# - CPU usage (%)
# - Memory usage (MB, %)
# - Available memory
# - Active agent count
```

### Failure Recovery

```bash
# Show recovery statistics
abathur recovery

# Output shows:
# - Total failures
# - Permanent vs transient failures
# - Retried/recovered tasks
# - DLQ count
```

### Dead Letter Queue

```bash
# List tasks in DLQ
abathur dlq list

# Reprocess failed task
abathur dlq reprocess <task-id>
```

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

resources:
  max_memory_per_agent: 512  # MB
  max_total_memory: 4096     # MB
  max_cpu_percent: 80.0

retry:
  max_retries: 3
  initial_backoff: 10        # seconds
  max_backoff: 300           # seconds
  backoff_multiplier: 2.0
```

### Set API Key

```bash
# Store in system keychain (recommended)
abathur config set-key YOUR_API_KEY

# Store in .env file
abathur config set-key YOUR_API_KEY --no-use-keychain
```

### Validate Configuration

```bash
# Validate configuration files
abathur config validate

# Show current configuration
abathur config show
```

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
3. **Use failure recovery**: Enable automatic retry for transient errors

### Template Development

1. **Clear system prompts**: Be specific about agent behavior
2. **Include examples**: Help agents understand expected input/output
3. **Validate before deployment**: Use `abathur template validate`
4. **Version templates**: Use Git tags for versioning

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
# Lower max_memory_per_agent
# Monitor with: abathur resources
```

**Issue: Tasks stuck in RUNNING**
```bash
# Check failure recovery stats
# Tasks will auto-recover after stall timeout (1 hour)
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
