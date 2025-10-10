# Abathur Claude Template

Official template repository for Abathur-enabled projects.

## About This Repository

This repository contains the base template files that are automatically used when you run `abathur init` to set up a new Abathur-enabled project.

**For Abathur documentation, see `README.abathur.md` - that's what gets copied to your project.**

## Usage

You don't need to clone this repository manually. Simply install Abathur and run:

```bash
pip install abathur-swarm
abathur init
```

The `abathur init` command will automatically:
1. Clone this template repository to `.abathur-template/`
2. Copy template files to `.claude/` in your project
3. Set up the initial configuration

## What's Included

### Agent Definitions (`.claude/agents/`)

The template includes 8 core runtime agents organized by tier:

- **Meta Agents** (2) - Swarm-level coordination
  - `swarm-coordinator.md` - Swarm lifecycle and health management
  - `context-synthesizer.md` - Cross-swarm state coherence

- **Specialist Agents** (6) - Core framework capabilities
  - `task-planner.md` - Task decomposition with dependencies
  - `agent-creator.md` - Dynamic agent generation
  - `resource-allocator.md` - Resource and priority management
  - `conflict-resolver.md` - Inter-agent conflict resolution
  - `performance-monitor.md` - Swarm efficiency tracking
  - `learning-coordinator.md` - Pattern capture and improvement

- **Worker Agents** - Created dynamically by `agent-creator`
  - `custom/` directory for project-specific agents

### Configuration Files

- `.env.example` - Environment variable template
- `.gitignore` - Comprehensive gitignore patterns
- `README.abathur.md` - Getting started guide (copied to projects)

## Template Structure

```
abathur-claude-template/
├── .claude/
│   └── agents/
│       ├── README.md
│       ├── meta/ (2 agents)
│       ├── specialists/ (6 agents)
│       └── workers/custom/
├── .env.example
├── .gitignore
├── README.abathur.md     # Abathur documentation (for projects)
└── README.md             # This file (repo documentation)
```

## Updating Your Project

After initialization, you can update to the latest template:

```bash
abathur init update
```

This will:
- Pull the latest template changes
- Update agent definitions
- Preserve your custom agents in `.claude/agents/custom/`
- **Not** overwrite your project's `README.md`

## For Template Contributors

To update this template:

1. Make your changes to the template files in the main abathur repo
2. Run `./template/setup-template-repo.sh` to update this repo
3. Test with `abathur init` in a fresh project
4. Commit and push changes
5. Tag releases for stable versions

See `TEMPLATE_REPO_SETUP.md` for detailed setup instructions.

## License

Apache-2.0

---

**Part of the [Abathur Swarm Orchestration](https://github.com/odgrim/abathur-swarm) project**
