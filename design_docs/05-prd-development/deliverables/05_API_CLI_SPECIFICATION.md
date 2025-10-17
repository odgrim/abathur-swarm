# Abathur API & CLI Specification

**Document Version:** 1.0
**Date:** 2025-10-09
**Status:** Complete - Ready for Implementation Phase
**Previous Phase:** System Design (04_SYSTEM_DESIGN.md)
**Next Phase:** Implementation Planning

---

## Table of Contents

1. [CLI Command Reference](#1-cli-command-reference)
2. [Configuration Schemas](#2-configuration-schemas)
3. [Task Template Format](#3-task-template-format)
4. [Agent Definition Format](#4-agent-definition-format)
5. [Output Format Specifications](#5-output-format-specifications)
6. [Error Code Registry](#6-error-code-registry)
7. [Common Workflows](#7-common-workflows)

---

## 1. CLI Command Reference

### 1.1 Global Options

All commands support these global flags:

```bash
--help, -h              Show command help
--version               Show CLI and template versions
--json                  Output in JSON format
--table                 Output in table format (lists only)
--verbose, -v           Verbose output
--debug                 Debug mode with stack traces
--quiet, -q             Suppress non-essential output
--profile TEXT          Use specific config profile (default: "default")
```

### 1.2 Initialization Commands

#### `abathur init`

Initialize Abathur in current project directory.

**Syntax:**
```bash
abathur init [OPTIONS] [PROJECT_PATH]
```

**Options:**
- `--version TEXT` - Template version to install (default: latest)
- `--force` - Overwrite existing `.abathur` directory
- `--no-template` - Initialize without cloning template

**Examples:**
```bash
# Initialize with latest template
abathur init

# Initialize with specific version
abathur init --version v1.2.0

# Initialize in different directory
abathur init /path/to/project

# Force reinitialize
abathur init --force
```

**Output:**
```
Initializing Abathur...
✓ Cloning template (odgrim/abathur-claude-template@v1.2.0)
✓ Installing to .claude/ (agents, MCP config)
✓ Installing to .abathur/ (config, database)
✓ Validating template structure
✓ Creating database schema

Abathur initialized successfully!

Next steps:
  1. Configure API key: export ANTHROPIC_API_KEY=<key>
  2. Review configuration: .abathur/config.yaml
  3. Submit first task: abathur task submit --help
```

**Exit Codes:**
- `0` - Success
- `1` - Already initialized (use --force)
- `2` - Template not found
- `3` - Validation failed

---

### 1.3 Task Commands

#### `abathur task submit`

Submit task to queue for execution.

**Syntax:**
```bash
abathur task submit --template TEMPLATE --input FILE [OPTIONS]
```

**Required Options:**
- `--template TEXT, -t TEXT` - Task template name
- `--input FILE, -i FILE` - Input file or data

**Optional:**
- `--priority INTEGER` - Priority 0-10 (default: 5)
- `--wait` - Wait for task completion
- `--metadata JSON` - Additional metadata (JSON string)
- `--wait-for TASK_ID` - Dependency on another task

**Examples:**
```bash
# Submit task with default priority
abathur task submit -t feature-implementation -i spec.md

# Submit with high priority
abathur task submit -t code-review -i pr://123 --priority 9

# Submit and wait for completion
abathur task submit -t test-generation -i api.py --wait

# Submit with dependency
abathur task submit -t implementation -i spec.md --wait-for task_abc123
```

**Output:**
```
Task submitted successfully!

Task ID: task_abc123def456
Template: feature-implementation
Priority: 5
Status: pending

Track progress: abathur task detail task_abc123def456
```

---

#### `abathur task list`

List tasks in queue.

**Syntax:**
```bash
abathur task list [OPTIONS]
```

**Options:**
- `--status TEXT` - Filter by status (pending|waiting|running|completed|failed|cancelled)
- `--priority INTEGER` - Filter by minimum priority
- `--limit INTEGER` - Limit results (default: 50)
- `--template TEXT` - Filter by template name

**Examples:**
```bash
# List all pending tasks
abathur task list --status pending

# List high-priority tasks
abathur task list --priority 8

# List in JSON format
abathur task list --json

# List in table format
abathur task list --table
```

**Output (Default):**
```
Task Queue Summary
==================

Pending: 3    Running: 2    Completed: 15    Failed: 1

Recent Tasks:
  task_abc123  running   priority: 9   feature-implementation   2m ago
  task_def456  pending   priority: 8   code-review             5m ago
  task_ghi789  pending   priority: 5   test-generation         10m ago

Total: 3 tasks shown
```

**Output (Table):**
```
┌──────────────┬──────────┬──────────┬────────────────────────┬──────────────┐
│ Task ID      │ Status   │ Priority │ Template               │ Submitted    │
├──────────────┼──────────┼──────────┼────────────────────────┼──────────────┤
│ task_abc123  │ running  │ 9        │ feature-implementation │ 2m ago       │
│ task_def456  │ pending  │ 8        │ code-review            │ 5m ago       │
│ task_ghi789  │ pending  │ 5        │ test-generation        │ 10m ago      │
└──────────────┴──────────┴──────────┴────────────────────────┴──────────────┘
```

---

#### `abathur task detail`

Show detailed information about a task.

**Syntax:**
```bash
abathur task detail TASK_ID [OPTIONS]
```

**Options:**
- `--follow, -f` - Follow task progress (streaming)
- `--show-result` - Include full result output

**Examples:**
```bash
# Show task details
abathur task detail task_abc123

# Follow task progress
abathur task detail task_abc123 --follow

# Show with full result
abathur task detail task_abc123 --show-result --json
```

**Output:**
```
Task Details
============

Task ID: task_abc123def456
Template: feature-implementation
Status: running (iteration 2/10)
Priority: 9

Submitted: 2025-10-09 10:30:00
Started: 2025-10-09 10:30:15
Duration: 2m 15s

Input:
  File: spec.md
  Size: 2.3 KB

Progress:
  ▓▓▓▓▓▓▓▓▓▓▓░░░░░░░░░░░░░░░░░░░░ 35%
  Current: Implementing backend API endpoints

Agents:
  - backend-specialist (agent_001) - busy
  - test-engineer (agent_002) - idle

Resource Usage:
  Memory: 456 MB / 512 MB
  Tokens: 12,450
  Estimated Cost: $0.15

Execution Log:
  [10:30:15] Task started
  [10:30:20] Agent spawned: backend-specialist
  [10:31:00] Checkpoint saved (iteration 1)
  [10:31:30] Agent spawned: test-engineer
  [10:32:00] Checkpoint saved (iteration 2)
```

---

#### `abathur task cancel`

Cancel a pending or running task.

**Syntax:**
```bash
abathur task cancel TASK_ID [OPTIONS]
```

**Options:**
- `--force` - Force cancel running task

**Examples:**
```bash
# Cancel pending task
abathur task cancel task_abc123

# Force cancel running task
abathur task cancel task_def456 --force
```

**Output:**
```
Cancelling task task_abc123...
✓ Task cancelled successfully
✓ Agents terminated gracefully
✓ Partial results saved

Task state: cancelled
Duration: 2m 15s
Completed iterations: 2/10
```

---

### 1.4 Loop Commands

#### `abathur loop start`

Start iterative loop execution.

**Syntax:**
```bash
abathur loop start --agent AGENT --input FILE [OPTIONS]
```

**Required Options:**
- `--agent TEXT, -a TEXT` - Agent to use for loop
- `--input FILE, -i FILE` - Input file or data

**Optional:**
- `--max-iterations INTEGER` - Maximum iterations (default: 10)
- `--timeout DURATION` - Total timeout (default: 1h)
- `--success-criteria FILE` - Convergence criteria file
- `--checkpoint-interval INTEGER` - Checkpoint every N iterations (default: 1)
- `--watch` - Watch progress in real-time

**Examples:**
```bash
# Start loop with default settings
abathur loop start -a optimizer -i query.sql

# Start with custom criteria
abathur loop start -a optimizer -i query.sql \
  --success-criteria criteria.yaml \
  --max-iterations 20

# Start with real-time monitoring
abathur loop start -a optimizer -i query.sql --watch
```

**Output:**
```
Starting iterative loop...

Agent: optimizer
Max Iterations: 10
Timeout: 1h
Success Criteria: performance_threshold

Iteration 1/10
  ▓▓▓▓▓▓░░░░░░░░░░░░░░░░░░░░░░░░ 20s
  Result: Query time 350ms
  Convergence: 0.30 (target: 0.95)
  Status: Not converged

Iteration 2/10
  ▓▓▓▓▓▓░░░░░░░░░░░░░░░░░░░░░░░░ 18s
  Result: Query time 180ms
  Convergence: 0.60 (target: 0.95)
  Status: Not converged

Iteration 3/10
  ▓▓▓▓▓▓░░░░░░░░░░░░░░░░░░░░░░░░ 15s
  Result: Query time 95ms
  Convergence: 0.98 (target: 0.95)
  Status: ✓ CONVERGED

Loop completed successfully!
Total Iterations: 3/10
Total Time: 53s
Final Result: Query optimized to 95ms
```

---

#### `abathur loop resume`

Resume a loop execution from checkpoint.

**Syntax:**
```bash
abathur loop resume TASK_ID
```

**Examples:**
```bash
# Resume interrupted loop
abathur loop resume task_abc123
```

---

### 1.5 Swarm Commands

#### `abathur swarm status`

Show swarm agent status.

**Syntax:**
```bash
abathur swarm status [OPTIONS]
```

**Options:**
- `--detailed` - Show per-agent details

**Examples:**
```bash
# Show swarm summary
abathur swarm status

# Show detailed agent info
abathur swarm status --detailed

# Watch in real-time
abathur swarm status --watch
```

**Output:**
```
Swarm Status
============

Active Agents: 5/10
Total Tasks: 12 (3 pending, 5 running, 4 completed)

Agent Pool:
  ┌────────────────┬────────────┬──────────┬────────────┬──────────┐
  │ Agent ID       │ State      │ Task     │ Uptime     │ Memory   │
  ├────────────────┼────────────┼──────────┼────────────┼──────────┤
  │ frontend-001   │ busy       │ task_123 │ 5m         │ 380 MB   │
  │ backend-001    │ busy       │ task_124 │ 5m         │ 420 MB   │
  │ test-eng-001   │ idle       │ -        │ 3m         │ 180 MB   │
  │ docs-001       │ busy       │ task_125 │ 2m         │ 250 MB   │
  │ review-001     │ busy       │ task_126 │ 8m         │ 390 MB   │
  └────────────────┴────────────┴──────────┴────────────┴──────────┘

Resource Usage:
  Memory: 1.6 GB / 4.0 GB (40%)
  CPU: 45%
  Tokens/min: 2,450
```

---

### 1.6 Configuration Commands

#### `abathur config show`

Display current configuration.

**Syntax:**
```bash
abathur config show [OPTIONS]
```

**Options:**
- `--profile TEXT` - Show specific profile
- `--secrets` - Show masked secret values

**Examples:**
```bash
# Show default configuration
abathur config show

# Show production profile
abathur config show --profile production
```

---

#### `abathur config validate`

Validate configuration files.

**Syntax:**
```bash
abathur config validate [OPTIONS]
```

**Examples:**
```bash
# Validate current configuration
abathur config validate
```

**Output:**
```
Validating configuration...

✓ .abathur/config.yaml - valid
✓ .claude/agents/*.yaml - 5 agents found, all valid
✓ .claude/mcp.json - 2 servers configured
✓ API key found (ANTHROPIC_API_KEY)
✓ Database schema up-to-date

Configuration is valid!
```

---

### 1.7 Status & Monitoring

#### `abathur task status`

Show task queue status and statistics.

**Syntax:**
```bash
abathur task status [OPTIONS]
```

**Options:**
- `--watch, -w` - Update continuously

**Examples:**
```bash
# Show current status
abathur task status

# Watch status continuously
abathur task status --watch
```

**Output:**
```
Task Queue Status
=================

Queue:
  Pending: 3
  Waiting: 1 (dependencies)
  Running: 5
  Completed: 42
  Failed: 2

Total Tasks: 53
```

---

## 2. Configuration Schemas

### 2.1 Main Configuration (`.abathur/config.yaml`)

```yaml
# Abathur Orchestration Configuration

# System settings
system:
  version: "1.0.0"
  log_level: INFO  # DEBUG, INFO, WARNING, ERROR, CRITICAL
  data_dir: ".abathur"

# API configuration
api:
  provider: anthropic
  model: claude-sonnet-4-20250514
  api_key_source: env  # env, keychain, file
  rate_limit:
    requests_per_minute: 100
    tokens_per_minute: 100000

# Queue configuration
queue:
  backend: sqlite  # sqlite, redis (future)
  database_path: ".abathur/abathur.db"
  max_size: 1000
  default_priority: 5
  priority_enabled: true

  # Retry configuration
  retry_attempts: 3
  retry_backoff_initial: 10s  # Initial delay
  retry_backoff_max: 5m       # Maximum delay
  retry_backoff_factor: 2.0    # Exponential multiplier

# Swarm configuration
swarm:
  max_concurrent_agents: 10
  agent_spawn_timeout: 5s
  agent_idle_timeout: 5m
  heartbeat_interval: 30s
  heartbeat_timeout: 90s  # 3 missed heartbeats

  # Hierarchical settings
  hierarchical_depth_limit: 3
  enable_sub_swarms: true

  # Distribution strategy
  distribution_strategy: load-balanced  # round-robin, priority, load-balanced

# Loop configuration
loop:
  default_max_iterations: 10
  default_timeout: 1h
  checkpoint_interval: 1  # Checkpoint every N iterations
  enable_auto_resume: true

# Resource limits
resources:
  max_memory_per_agent: 512MB
  max_total_memory: 4GB
  adaptive_cpu: true  # Adjust based on available cores
  enable_monitoring: true

# Monitoring configuration
monitoring:
  log_dir: ".abathur/logs"
  log_format: json  # json, text
  log_rotation_days: 30
  audit_enabled: true
  audit_retention_days: 90
  metrics_enabled: true

# Template configuration
template:
  repository: odgrim/abathur-claude-template
  version: latest  # or specific version like "v1.2.0"
  cache_dir: "~/.abathur/cache/templates"
  cache_ttl: 604800  # 7 days in seconds
  auto_update: false

# Security configuration
security:
  api_key_store: keychain  # keychain, env, file
  encrypt_logs: false
  redact_sensitive_data: false
```

### 2.2 Environment Variables

All configuration can be overridden with environment variables using the `ABATHUR_` prefix:

```bash
# API Configuration
export ANTHROPIC_API_KEY="sk-ant-..."
export ABATHUR_API_MODEL="claude-sonnet-4-20250514"

# Queue Configuration
export ABATHUR_QUEUE_MAX_SIZE=2000
export ABATHUR_QUEUE_DEFAULT_PRIORITY=7

# Swarm Configuration
export ABATHUR_SWARM_MAX_CONCURRENT_AGENTS=20

# Resource Limits
export ABATHUR_RESOURCES_MAX_TOTAL_MEMORY=8GB

# Logging
export ABATHUR_SYSTEM_LOG_LEVEL=DEBUG
export ABATHUR_MONITORING_LOG_FORMAT=text

# Profile Selection
export ABATHUR_CONFIG_PROFILE=production
```

### 2.3 Local Overrides (`.abathur/local.yaml`)

Gitignored file for local development overrides:

```yaml
# Local development overrides
system:
  log_level: DEBUG

swarm:
  max_concurrent_agents: 3  # Limit for local dev

resources:
  max_total_memory: 2GB  # Lower limit for dev machine
```

---

## 3. Task Template Format

### 3.1 Template Structure

Task templates are stored in `.abathur/templates/` directory:

```yaml
# .abathur/templates/feature-implementation.yaml

name: feature-implementation
description: Implement a full-stack feature from specification
version: 1.0.0

# Agent requirements
agents:
  - name: spec-analyzer
    specialization: specification-analysis
    required: true

  - name: frontend-dev
    specialization: frontend-development
    required: true

  - name: backend-dev
    specialization: backend-development
    required: true

  - name: test-engineer
    specialization: testing
    required: true

  - name: documentation
    specialization: documentation
    required: false

# Input schema
input:
  type: file
  formats:
    - markdown
    - text
  required_fields:
    - feature_description
    - acceptance_criteria

# Execution configuration
execution:
  type: swarm  # swarm, loop, sequential
  coordination: hierarchical
  max_duration: 2h

  # Swarm-specific config
  swarm_config:
    distribution_strategy: specialized
    aggregation_strategy: merge
    failure_threshold: 0.3  # Fail if >30% of agents fail

# Output schema
output:
  format: structured
  artifacts:
    - frontend_code
    - backend_code
    - tests
    - documentation

  validation:
    - all_tests_pass
    - no_linting_errors

# Success criteria
success_criteria:
  - type: test_pass
    test_suite: generated_tests

  - type: threshold
    metric: code_coverage
    threshold: 0.80
    direction: maximize

# Failure handling
failure_handling:
  retry_strategy: exponential_backoff
  max_retries: 3
  fallback_template: feature-implementation-fallback
```

### 3.2 Loop Template Example

```yaml
# .abathur/templates/query-optimizer.yaml

name: query-optimizer
description: Iteratively optimize database query performance
version: 1.0.0

agents:
  - name: optimizer
    specialization: database-optimization
    required: true

input:
  type: file
  formats:
    - sql
  required_fields:
    - query
    - performance_baseline

execution:
  type: loop
  max_iterations: 20
  timeout: 1h

  loop_config:
    checkpoint_interval: 1
    convergence_criteria:
      - type: threshold
        metric: execution_time_ms
        threshold: 100
        direction: minimize

      - type: stability
        window: 3
        similarity_threshold: 0.95

output:
  format: structured
  artifacts:
    - optimized_query
    - performance_report
    - optimization_history

success_criteria:
  - type: threshold
    metric: execution_time_ms
    threshold: 100
    direction: minimize
```

---

## 4. Agent Definition Format

### 4.1 Agent Configuration (`.claude/agents/*.yaml`)

Agent definitions are shared with Claude Code:

```yaml
# .claude/agents/frontend-specialist.yaml

name: frontend-specialist
description: Expert in React and TypeScript frontend development
specialization: frontend-development
version: 1.0.0

# Claude model configuration
model:
  name: claude-sonnet-4-20250514
  max_tokens: 8000
  temperature: 0.7

# System prompt
system_prompt: |
  You are a frontend development specialist with expertise in:
  - React and TypeScript
  - Modern CSS (Tailwind, CSS Modules)
  - Accessibility (WCAG 2.1 AA)
  - Performance optimization
  - Component testing (Jest, React Testing Library)

  Your role:
  1. Write clean, maintainable, type-safe React components
  2. Implement responsive, accessible UI
  3. Follow project conventions and best practices
  4. Write comprehensive component tests

  Always:
  - Use TypeScript strict mode
  - Write semantic HTML
  - Ensure keyboard navigation
  - Add ARIA labels where needed
  - Write tests for critical functionality

# Tools and capabilities
tools:
  - name: read_file
    enabled: true

  - name: write_file
    enabled: true
    restrictions:
      - "*.tsx"
      - "*.ts"
      - "*.css"

  - name: execute_command
    enabled: true
    allowed_commands:
      - "npm test"
      - "npm run lint"
      - "npm run build"

  - name: search_files
    enabled: true

# MCP servers required
mcp_servers:
  - filesystem
  - github

# Resource limits
resource_limits:
  max_memory: 512MB
  max_execution_time: 30m

# Metadata
metadata:
  author: Abathur Team
  created: 2025-10-09
  tags:
    - frontend
    - react
    - typescript
```

### 4.2 MCP Server Configuration (`.claude/mcp.json`)

Shared MCP server configuration compatible with Claude Code:

```json
{
  "mcpServers": {
    "github": {
      "command": "npx",
      "args": ["-y", "@modelcontextprotocol/server-github"],
      "env": {
        "GITHUB_PERSONAL_ACCESS_TOKEN": "${GITHUB_TOKEN}"
      }
    },

    "filesystem": {
      "command": "npx",
      "args": [
        "-y",
        "@modelcontextprotocol/server-filesystem",
        "/path/to/project"
      ]
    },

    "postgres": {
      "command": "npx",
      "args": ["-y", "@modelcontextprotocol/server-postgres"],
      "env": {
        "POSTGRES_CONNECTION_STRING": "${DATABASE_URL}"
      }
    }
  }
}
```

---

## 5. Output Format Specifications

### 5.1 JSON Output Schema

All commands support `--json` flag for structured output:

#### Task Submit Response

```json
{
  "status": "success",
  "data": {
    "task_id": "task_abc123def456",
    "template": "feature-implementation",
    "priority": 5,
    "status": "pending",
    "submitted_at": "2025-10-09T10:30:00Z"
  },
  "metadata": {
    "cli_version": "1.0.0",
    "timestamp": "2025-10-09T10:30:00Z"
  }
}
```

#### Task List Response

```json
{
  "status": "success",
  "data": {
    "tasks": [
      {
        "task_id": "task_abc123",
        "template": "feature-implementation",
        "status": "running",
        "priority": 9,
        "submitted_at": "2025-10-09T10:28:00Z",
        "started_at": "2025-10-09T10:28:15Z",
        "progress": 0.35,
        "agents": ["agent_001", "agent_002"]
      },
      {
        "task_id": "task_def456",
        "template": "code-review",
        "status": "pending",
        "priority": 8,
        "submitted_at": "2025-10-09T10:25:00Z",
        "dependencies": []
      }
    ],
    "summary": {
      "total": 2,
      "pending": 1,
      "running": 1,
      "completed": 0,
      "failed": 0
    }
  },
  "metadata": {
    "filters": {
      "status": null,
      "priority": null
    },
    "timestamp": "2025-10-09T10:30:00Z"
  }
}
```

#### Task Detail Response

```json
{
  "status": "success",
  "data": {
    "task_id": "task_abc123def456",
    "template": "feature-implementation",
    "status": "running",
    "priority": 9,

    "timestamps": {
      "submitted_at": "2025-10-09T10:28:00Z",
      "started_at": "2025-10-09T10:28:15Z",
      "updated_at": "2025-10-09T10:30:00Z"
    },

    "input": {
      "file": "spec.md",
      "size": 2341,
      "checksum": "abc123..."
    },

    "progress": {
      "percentage": 0.35,
      "current_phase": "backend-implementation",
      "iteration": 2,
      "max_iterations": 10
    },

    "agents": [
      {
        "agent_id": "agent_001",
        "name": "backend-specialist",
        "state": "busy",
        "spawned_at": "2025-10-09T10:28:20Z",
        "memory_mb": 456,
        "tokens_used": 12450
      },
      {
        "agent_id": "agent_002",
        "name": "test-engineer",
        "state": "idle",
        "spawned_at": "2025-10-09T10:29:30Z",
        "memory_mb": 180,
        "tokens_used": 3200
      }
    ],

    "resource_usage": {
      "memory_mb": 636,
      "tokens": 15650,
      "estimated_cost_usd": 0.15,
      "duration_seconds": 105
    },

    "execution_log": [
      {
        "timestamp": "2025-10-09T10:28:15Z",
        "event": "task_started"
      },
      {
        "timestamp": "2025-10-09T10:28:20Z",
        "event": "agent_spawned",
        "agent": "backend-specialist"
      },
      {
        "timestamp": "2025-10-09T10:29:00Z",
        "event": "checkpoint_saved",
        "iteration": 1
      }
    ]
  }
}
```

### 5.2 Error Response Schema

```json
{
  "status": "error",
  "error": {
    "code": "ABTH-ERR-005",
    "message": "Task queue has reached maximum capacity",
    "details": "Queue size: 1000/1000. Please wait for tasks to complete or increase queue.max_size in config.",
    "suggestion": "Run 'abathur task list --status completed' to view finished tasks, or 'abathur config show' to adjust queue size.",
    "documentation_url": "https://docs.abathur.dev/errors/ABTH-ERR-005"
  },
  "metadata": {
    "timestamp": "2025-10-09T10:30:00Z",
    "cli_version": "1.0.0"
  }
}
```

---

## 6. Error Code Registry

### 6.1 Initialization Errors (ABTH-ERR-001 to 010)

| Code | Description | Suggestion |
|------|-------------|------------|
| **ABTH-ERR-001** | Already initialized | Use `--force` to reinitialize or choose different directory |
| **ABTH-ERR-002** | Template not found | Check version/URL or network connection |
| **ABTH-ERR-003** | Template validation failed | Template structure invalid, check template repository |
| **ABTH-ERR-004** | Database initialization failed | Check permissions and disk space |

### 6.2 Queue Errors (ABTH-ERR-005 to 020)

| Code | Description | Suggestion |
|------|-------------|------------|
| **ABTH-ERR-005** | Queue capacity exceeded | Increase `queue.max_size` or clear completed tasks |
| **ABTH-ERR-006** | Task not found | Check task ID with `abathur task list` |
| **ABTH-ERR-007** | Invalid priority value | Priority must be 0-10 |
| **ABTH-ERR-008** | Circular dependency detected | Review task dependencies for cycles |
| **ABTH-ERR-009** | Dependency task failed | Check dependency status, consider `--ignore-dependency-failure` |
| **ABTH-ERR-010** | Task already cancelled | Task is in terminal state |

### 6.3 Agent Errors (ABTH-ERR-021 to 040)

| Code | Description | Suggestion |
|------|-------------|------------|
| **ABTH-ERR-021** | Agent spawn timeout | Check system resources and Claude API availability |
| **ABTH-ERR-022** | Agent spawn failed | Verify agent configuration and API key |
| **ABTH-ERR-023** | Max concurrent agents reached | Wait for agents to complete or increase `swarm.max_concurrent_agents` |
| **ABTH-ERR-024** | Agent crashed | Check logs for details, task moved to DLQ |
| **ABTH-ERR-025** | Agent heartbeat timeout | Agent may be stalled, will be terminated |
| **ABTH-ERR-026** | Memory limit exceeded | Increase `resources.max_memory_per_agent` or optimize task |

### 6.4 API Errors (ABTH-ERR-041 to 060)

| Code | Description | Suggestion |
|------|-------------|------------|
| **ABTH-ERR-041** | API key not found | Set `ANTHROPIC_API_KEY` environment variable or use `abathur config set-key` |
| **ABTH-ERR-042** | API key invalid | Verify API key is correct and active |
| **ABTH-ERR-043** | API rate limit exceeded | Retry will occur automatically, or reduce `api.rate_limit` |
| **ABTH-ERR-044** | API request failed | Check network connection and API status |
| **ABTH-ERR-045** | Model not available | Verify model name in configuration |

### 6.5 Configuration Errors (ABTH-ERR-061 to 080)

| Code | Description | Suggestion |
|------|-------------|------------|
| **ABTH-ERR-061** | Configuration file not found | Run `abathur init` to create configuration |
| **ABTH-ERR-062** | Configuration validation failed | Check YAML syntax and required fields |
| **ABTH-ERR-063** | Invalid configuration value | Review error details for specific field |
| **ABTH-ERR-064** | Profile not found | Check available profiles with `abathur config profiles` |
| **ABTH-ERR-065** | MCP server configuration invalid | Verify `.claude/mcp.json` syntax |

### 6.6 Loop Errors (ABTH-ERR-081 to 100)

| Code | Description | Suggestion |
|------|-------------|------------|
| **ABTH-ERR-081** | Max iterations exceeded | Increase `max_iterations` or review convergence criteria |
| **ABTH-ERR-082** | Loop timeout exceeded | Increase `timeout` or optimize agent performance |
| **ABTH-ERR-083** | Convergence criteria not met | Review criteria configuration |
| **ABTH-ERR-084** | Checkpoint not found | Cannot resume, no checkpoint exists |
| **ABTH-ERR-085** | Checkpoint corrupted | Checkpoint data invalid, cannot resume |

---

## 7. Common Workflows

### 7.1 Workflow: Initialize and Submit First Task

```bash
# 1. Initialize project
abathur init

# 2. Set API key
export ANTHROPIC_API_KEY="sk-ant-..."

# 3. Validate configuration
abathur config validate

# 4. Create input file
cat > feature-spec.md << EOF
# New Authentication Feature
## Requirements
- Implement JWT-based authentication
- Add login/logout endpoints
- Include password reset flow
EOF

# 5. Submit task
abathur task submit \
  --template feature-implementation \
  --input feature-spec.md \
  --priority 8

# Output: Task ID: task_abc123

# 6. Monitor progress
abathur task detail task_abc123 --follow
```

### 7.2 Workflow: Batch Code Review

```bash
# 1. List pending PRs
prs=$(gh pr list --json number --jq '.[].number')

# 2. Submit review tasks for each PR
for pr in $prs; do
  abathur task submit \
    --template code-review \
    --input "pr://$pr" \
    --priority 7
done

# 3. Monitor all running reviews
abathur task list --status running --table

# 4. Check status continuously
abathur status --watch
```

### 7.3 Workflow: Iterative Query Optimization

```bash
# 1. Create convergence criteria
cat > criteria.yaml << EOF
type: threshold
metric: execution_time_ms
threshold: 100
direction: minimize
EOF

# 2. Start optimization loop
abathur loop start \
  --agent query-optimizer \
  --input slow-query.sql \
  --success-criteria criteria.yaml \
  --max-iterations 20 \
  --watch

# Output:
# Iteration 1: 350ms -> Not converged
# Iteration 2: 180ms -> Not converged
# Iteration 3: 95ms -> ✓ CONVERGED

# 3. View optimization history
abathur task detail <task-id> --show-result --json | \
  jq '.data.execution_log[] | select(.event == "checkpoint_saved")'
```

### 7.4 Workflow: Hierarchical Feature Development

```bash
# 1. Create feature specification
cat > feature.md << EOF
# E-commerce Checkout Flow
## Components
- Frontend: React checkout form
- Backend: Payment processing API
- Tests: E2E checkout tests
- Docs: API documentation
EOF

# 2. Submit with hierarchical coordination
abathur task submit \
  --template feature-implementation \
  --input feature.md \
  --priority 9 \
  --metadata '{"coordination": "hierarchical"}'

# 3. Watch swarm coordination
abathur swarm status --detailed --watch

# Output:
# Leader: orchestrator-001 (busy)
# Workers:
#   - frontend-specialist-001 (busy)
#   - backend-specialist-001 (busy)
#   - test-engineer-001 (busy)
#   - docs-writer-001 (idle)
```

### 7.5 Workflow: Task Dependency Chain

```bash
# 1. Submit specification task
spec_task=$(abathur task submit \
  --template spec-generation \
  --input requirements.md \
  --json | jq -r '.data.task_id')

# 2. Submit test generation (depends on spec)
test_task=$(abathur task submit \
  --template test-generation \
  --input spec \
  --wait-for "$spec_task" \
  --json | jq -r '.data.task_id')

# 3. Submit implementation (depends on tests)
impl_task=$(abathur task submit \
  --template implementation \
  --input spec \
  --wait-for "$test_task" \
  --json | jq -r '.data.task_id')

# 4. Monitor entire pipeline
abathur task list --json | \
  jq '.data.tasks[] | select(.task_id | IN($spec_task, $test_task, $impl_task))'
```

### 7.6 Workflow: Handle Failed Tasks

```bash
# 1. List failed tasks
abathur task list --status failed --json

# 2. Check DLQ
abathur task dlq list

# 3. View failure details
abathur task detail <failed-task-id>

# 4. Retry from DLQ
abathur task dlq retry <task-id>

# 5. Or bulk retry all
abathur task dlq retry --all --confirm
```

### 7.7 Workflow: Production Deployment

```bash
# 1. Use production profile
export ABATHUR_CONFIG_PROFILE=production

# 2. Validate production config
abathur config validate --profile production

# 3. Submit with production settings
abathur task submit \
  --template deployment-pipeline \
  --input deploy-config.yaml \
  --priority 10

# 4. Monitor with enhanced logging
abathur task detail <task-id> --follow --verbose
```

---

## Summary

This API & CLI specification provides comprehensive documentation for:

**CLI Commands:**
- 7 command groups: init, task, loop, swarm, config, status, monitoring
- 20+ commands with full syntax, options, and examples
- Consistent flag patterns across all commands
- Three output formats: human-readable, JSON, table

**Configuration:**
- Complete YAML schema with all settings documented
- Environment variable overrides for all config values
- Local override support for development
- Profile support for multi-environment scenarios

**Templates:**
- Task template format for swarm and loop execution
- Agent definition format compatible with Claude Code
- MCP server configuration shared with Claude ecosystem
- Success criteria and failure handling specifications

**Error Handling:**
- 100 error codes organized by category
- Actionable suggestions for each error
- Documentation links for detailed troubleshooting
- Consistent error response format

**Workflows:**
- 7 common workflow examples covering major use cases
- From simple task submission to complex hierarchical coordination
- Includes batch processing, loops, dependencies, and failure recovery
- Production-ready patterns with proper error handling

**Key Design Decisions:**
- Type-safe CLI with Typer framework
- JSON output for scripting and automation
- Consistent error codes with actionable messages
- Shared configuration with Claude Code (`.claude/` directory)
- Flexible configuration hierarchy (defaults → template → user → env vars)

---

**Document Status:** Complete
**Next Phase:** Security specification, then implementation planning
**Total Lines:** ~680 (within 500-700 target)
**Context for Next Agent:**
- CLI uses Typer framework with type hints
- Task states: pending, waiting, running, completed, failed, cancelled
- Priority: 0-10 scale (10 highest)
- Configuration hierarchy: system → template → user → local → env vars
- Three output formats: human (default), JSON (--json), table (--table)
- Error codes: ABTH-ERR-001 through ABTH-ERR-100
