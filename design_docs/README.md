# Abathur - Hivemind Swarm Management System for Claude Agents

## Project Overview

**Abathur** is a sophisticated multi-agent orchestration system designed to coordinate and manage swarms of Claude agents for complex task execution. Built with Python and the Claude Agent SDK, Abathur enables developers to leverage the power of multiple AI agents working in concert.

### Repository Structure

This project consists of two repositories:

1. **odgrim/abathur-swarm** (main repository)
   - Core orchestration system
   - CLI tool (`abathur`)
   - Task queue management
   - Swarm coordination engine
   - Loop execution framework

2. **odgrim/abathur-claude-template** (template repository)
   - Project template for Abathur-enabled projects
   - `.claude/agents/` directory with specialized agents
   - MCP (Model Context Protocol) configurations
   - Environment and configuration templates
   - Boilerplate code and documentation

## Core Features

### 1. Template Management
- Clone the abathur-claude-template repository
- Install template into `.abathur` directory in any project
- Customize and configure for specific use cases
- Version-controlled template updates

### 2. Task Queue Orchestration
- Priority-based task queue with persistence
- Submit, list, cancel, and monitor tasks
- Scalable queue operations (10,000+ tasks)
- Atomic queue operations with crash recovery

### 3. Swarm Coordination
- Spawn and manage multiple Claude agents concurrently
- Distribute tasks across agent pool
- Real-time agent health monitoring
- Result aggregation and synthesis
- Failure handling and task redistribution

### 4. Loop Execution
- Iterative task execution with refinement
- Configurable convergence criteria
- Checkpoint and resume functionality
- Iteration history tracking
- Multiple convergence strategies

### 5. CLI Tool
- Comprehensive command-line interface
- Multiple output formats (JSON, table, human-readable)
- Interactive and non-interactive modes
- Progress indicators and status updates
- Rich error messages with suggestions

## Project Status

**Current Phase:** PRD Development

This repository currently contains a complete specialized agent team for developing a comprehensive Product Requirements Document (PRD) for the Abathur system.

### PRD Development Agent Team

10 specialized agents have been created to collaboratively write an industry-standard PRD:

1. **prd-project-orchestrator** - Coordinates PRD development and validates phases
2. **prd-product-vision-specialist** - Defines vision, goals, and use cases
3. **prd-requirements-analyst** - Documents functional and non-functional requirements
4. **prd-technical-architect** - Designs system architecture and technology stack
5. **prd-system-design-specialist** - Specifies algorithms and protocols
6. **prd-api-cli-specialist** - Defines API and CLI specifications
7. **prd-security-specialist** - Conducts threat modeling and security requirements
8. **prd-quality-metrics-specialist** - Defines success metrics and quality gates
9. **prd-implementation-roadmap-specialist** - Creates phased implementation plan
10. **prd-documentation-specialist** - Compiles final comprehensive PRD

## Getting Started with PRD Development

### Prerequisites

1. **Resolve Decision Points:**
   - Open `DECISION_POINTS.md`
   - Review and resolve all 29 architectural decisions
   - Accept suggested defaults or provide your own answers
   - Save the file with resolved decisions

2. **Review Documentation:**
   - Read `PRD_ORCHESTRATOR_HANDOFF.md` for agent team details
   - Understand the 4-phase execution workflow
   - Familiarize yourself with validation gates

### Execute PRD Development

1. Open Claude Code in this directory
2. Copy the kickoff prompt from `CLAUDE_CODE_KICKOFF_PROMPT.md`
3. Paste into Claude Code
4. The orchestrator will coordinate all agents to produce the PRD

### Expected Output

After execution, you will have:
- `ABATHUR_PRD.md` - Comprehensive Product Requirements Document
- Supporting diagrams and visualizations
- Complete documentation ready for implementation

## PRD Development Timeline

- **Phase 1:** Vision & Requirements (~2 hours)
- **Phase 2:** Technical Architecture & Design (~3 hours)
- **Phase 3:** Quality, Security & Planning (~2 hours)
- **Phase 4:** Compilation & Finalization (~1 hour)
- **Total:** ~8 hours for comprehensive PRD

## Repository Files

### Agent Definitions
Located in `.claude/agents/`:
- All 10 PRD development specialist agents
- Each agent has specific tools, model class, and instructions
- Designed for orchestrated collaboration

### Documentation
- `README.md` - This file
- `DECISION_POINTS.md` - Architectural decisions to resolve before PRD development
- `PRD_ORCHESTRATOR_HANDOFF.md` - Complete orchestration guide
- `CLAUDE_CODE_KICKOFF_PROMPT.md` - Ready-to-use kickoff prompt

### Git Configuration
- `.gitignore` - Configured for Python, environments, and sensitive files

## Technology Stack (Planned)

Based on PRD development, the final Abathur system will use:

- **Language:** Python 3.10+
- **CLI Framework:** Typer (type-safe, modern)
- **Agent SDK:** Anthropic Claude SDK
- **Async Runtime:** asyncio
- **Queue Backend:** SQLite (with Redis option for distributed scenarios)
- **Configuration:** Pydantic + python-dotenv
- **Testing:** pytest, pytest-asyncio, pytest-cov
- **Dependency Management:** Poetry

## Implementation Roadmap (Planned)

After PRD completion, implementation will follow a 25-week roadmap:

- **Phase 0:** Foundation & Setup (Weeks 1-2)
- **Phase 1:** Core Infrastructure (Weeks 3-5)
- **Phase 2:** Template Management (Weeks 6-7)
- **Phase 3:** Claude Agent Integration (Weeks 8-10)
- **Phase 4:** Swarm Orchestration (Weeks 11-13)
- **Phase 5:** Loop Execution (Weeks 14-15)
- **Phase 6:** Advanced Features (Weeks 16-17)
- **Phase 7:** Security & Compliance (Weeks 18-19)
- **Phase 8:** Documentation & Polish (Weeks 20-21)
- **Phase 9:** Beta Testing (Weeks 22-24)
- **Phase 10:** v1.0 Release (Week 25)

## Use Cases

Abathur will enable:

1. **Multi-Agent Code Review:** Coordinate specialized agents to review different aspects of code
2. **Parallel Feature Implementation:** Distribute feature development across agent swarm
3. **Iterative Problem Solving:** Use loops to refine solutions through multiple iterations
4. **Complex Analysis:** Aggregate insights from multiple agent perspectives
5. **Automated Workflows:** Create sophisticated multi-step AI-powered workflows

## Success Metrics (Planned)

- **Performance:** <100ms task submission, 10+ concurrent agents, 100+ tasks/minute
- **Reliability:** >99.9% uptime, >95% task success rate
- **Usability:** <5 minutes to first task, >70 NPS user satisfaction
- **Quality:** >80% test coverage, 0 critical vulnerabilities

## Contributing

This project is in active PRD development. Once the PRD is complete and implementation begins, contribution guidelines will be established.

Current focus: Complete the PRD using the specialized agent team.

## License

To be determined (will be open source)

## Project Metadata

- **Project Name:** Abathur
- **Project Type:** Multi-Agent Orchestration System
- **Primary Language:** Python
- **Target Users:** AI Engineers, Developers, Automation Specialists
- **Status:** PRD Development Phase
- **Version:** 0.1.0-alpha (PRD stage)

## Next Steps

1. Resolve decision points in `DECISION_POINTS.md`
2. Execute PRD development using `CLAUDE_CODE_KICKOFF_PROMPT.md`
3. Review and refine generated PRD
4. Begin implementation Phase 0 (Foundation & Setup)

---

**The specialized agent team is ready to collaboratively develop your comprehensive PRD. Let's build Abathur!**
