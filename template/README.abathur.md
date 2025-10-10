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

This will set up the `.abathur` directory with agent definitions and configuration.

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
├── agents/           # Agent definitions
│   ├── meta/        # Coordination agents
│   ├── specialists/ # Specialized agents
│   └── workers/     # General-purpose agents
└── config/          # Configuration files
```

## Customization

### Adding Custom Agents

Create new agent definitions in `.abathur/agents/`:

```markdown
---
name: my-custom-agent
description: What this agent does
model: thinking
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
