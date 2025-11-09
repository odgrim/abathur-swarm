# Abathur Claude Template

Official template repository for Abathur-enabled projects.

## What is Abathur?

Abathur is a hyperspecialized agent swarm orchestration framework for Claude Code. It enables:
- Automatic task decomposition and delegation
- Coordination of multiple specialized AI agents
- Dependency tracking and P2P communication
- Comprehensive task queue management

## Getting Started

### Installation

```bash
pip install abathur-swarm
```

### Initialize Your Project

```bash
abathur init
```

This will set up the `.abathur` directory for configuration and `.claude/agents` for agent definitions.

### Configure Your Environment

Create a `.env` file with your Anthropic API key:

```bash
ANTHROPIC_API_KEY=your_api_key_here
```

### Basic Usage

```bash
# Submit a task
abathur submit "Implement user authentication"

# Check swarm health
abathur health

# List available agents
abathur agents list

# View task queue
abathur queue list

# Monitor analytics
abathur analytics show
```

## Directory Structure

```
.abathur/
├── config.yaml      # Abathur configuration
├── hooks.yaml       # Task lifecycle hooks
├── hooks/           # Hook scripts
└── abathur.db       # Task queue database

.claude/
└── agents/          # Agent definitions
    ├── abathur/     # Core orchestration agents
    └── workers/     # Specialized worker agents
```

## Customization

### Adding Custom Agents

Create new agent definitions in `.claude/agents/`:

```markdown
---
name: my-custom-agent
description: What this agent does
model: sonnet
color: Blue
tools: Read, Edit, Bash
---

## Purpose
...
```

### Updating Agents

```bash
abathur update
```

This pulls the latest template changes while preserving your custom agents.

## Documentation

For full documentation, visit: https://github.com/odgrim/abathur-swarm

## License

Apache-2.0
