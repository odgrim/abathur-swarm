# Phase 1 Invocation Context - Vision & Requirements

## Project Overview

**Project Name:** Abathur Hivemind Swarm Management System
**Project Type:** Multi-Agent Orchestration Platform
**Technology Stack:** Python 3.10+, Claude Agent SDK, Typer CLI Framework
**Repositories:**
- `odgrim/abathur-swarm` (main codebase and CLI)
- `odgrim/abathur-claude-template` (template repository)

## Phase 1 Objectives

This phase establishes the foundational vision and requirements for Abathur. Two specialized agents will work to define:

1. **Product Vision & Use Cases** (prd-product-vision-specialist)
2. **Functional & Non-Functional Requirements** (prd-requirements-analyst)

## Core Functionality Summary

Abathur is a CLI-first multi-agent orchestration system that enables:

### 1. Template Management
- Clone and install project templates from GitHub
- Version-controlled template repository pattern
- Local caching with update mechanisms
- Template-driven project initialization

### 2. Task Queue Management
- SQLite-based persistent task queue
- Priority-based task scheduling (0-10 numeric scale)
- FIFO execution within same priority level
- Queue capacity: 1000 tasks (configurable)
- Task state tracking and persistence

### 3. Swarm Coordination
- Hierarchical leader-follower orchestration model
- Concurrent agent execution (5-10 agents default, configurable)
- Message queue-based asynchronous communication
- Shared state database for coordination
- Dynamic agent spawning based on workload

### 4. Loop Execution
- Iterative task execution with convergence criteria
- Multiple termination conditions:
  - Max iteration count
  - Success criteria evaluation
  - Timeout-based limits
  - Human override options
- Checkpoint and resume capability

### 5. CLI Tool
- Typer-based command-line interface
- Multiple output formats: text, JSON, table, TUI
- Progress indication: spinners, progress bars
- Actionable error messages with error codes
- Comprehensive command structure

## Resolved Architectural Decisions

### Technology Decisions (from DECISION_POINTS.md)

1. **Task Queue:** SQLite-based (persistent, single-node, simple)
2. **Agent Communication:** Message queue + shared state database (async coordination)
3. **State Management:** Centralized state store with event log
4. **CLI Framework:** Typer (modern, type-safe, excellent DX)
5. **Configuration:** Hybrid approach (.env for secrets, YAML for structured config)
6. **Agent Spawning:** Async/await with configurable concurrency limits
7. **Python Version:** 3.10+ (modern type hints, pattern matching)
8. **Dependency Management:** Poetry (comprehensive dependency + packaging)
9. **Template Strategy:** Versioned releases with user choice, local caching

### Business Logic Decisions

10. **Swarm Coordination:** Hierarchical with leader-follower elements
11. **Task Priority:** Numeric 0-10 scale
12. **Failure Recovery:** Retry + exponential backoff + dead letter queue + checkpointing
13. **Loop Termination:** Max iterations + success criteria + timeout (combination)
14. **Concurrency Limits:** 10 max concurrent agents (configurable)
15. **Queue Size:** 1000 task capacity (configurable)

### Security & Compliance

16. **API Key Management:** Environment variables (local development focus)
17. **Data Privacy:** Full logging (local tool, not hosted)
18. **Access Control:** Single user (no access control needed)

### Integration Specifications

19. **MCP Integration:** Auto-discover from template + user overrides + dynamic loading
20. **GitHub Integration:** Template cloning + user-configured issue/doc sources
21. **Monitoring:** Structured logging to file + CLI output

### UI/UX Decisions

22. **Output Format:** Multiple formats (text default, --json, --table, optional TUI)
23. **Progress Indication:** Progress bars + spinners + --verbose flag
24. **Error Reporting:** Actionable suggestions + error codes + --debug for stack traces

### Implementation Approach

25. **Development Phases:**
    - Phase 1: Core CLI + template management
    - Phase 2: Task queue + basic orchestration
    - Phase 3: Swarm coordination + looping
    - Phase 4: Advanced features (MCP, monitoring)

26. **Testing Strategy:** Comprehensive (unit + integration + E2E + property-based)
27. **Documentation:** Complete (README, API docs, user guide, developer guide, architecture docs)

## Swarm Design Philosophy

Abathur follows a specification-driven development process:

1. **Create Specification** - Define clear objectives
2. **Gather Requirements** - Understand product/project needs
3. **Write Technical Specs** - Document technical solutions
4. **Create Test Suites** - Validate specifications with tests
5. **Implement Solutions** - Build while validating against tests

### Core Pillar: Hyperspecialization

The system can spawn highly specialized agents for specific tasks, following the biological Abathur pattern of continuous improvement and adaptation.

### Meta-Agent Capability

An Abathur meta-agent can improve other agents based on feedback, enabling continuous evolution of the agent ecosystem.

## Performance Requirements

### Response Time Targets
- Queue operations: <100ms
- Agent spawn time: <5s
- Status checks: <50ms

### Resource Constraints (Configurable)
- Max memory per agent: 512MB
- Max total memory: 4GB
- CPU allocation: Adaptive based on system cores

## Project Constraints

### Technical Constraints
- Must use Python 3.10+ for modern language features
- Claude Agent SDK as primary agent interface
- GitHub-based template repository pattern
- CLI-first interface (no GUI in initial release)
- Local development focus (not designed for hosted deployment)

### Operational Constraints
- Single-user operation (no multi-tenancy)
- Local file system for persistence
- No distributed deployment in v1.0
- English language only initially

## Target Users & Use Cases (To Be Defined by Vision Specialist)

The product vision specialist will define:
- Primary user personas
- User journey maps
- Core use cases and scenarios
- Value proposition
- Market positioning
- Success metrics

## Requirements Scope (To Be Defined by Requirements Analyst)

The requirements analyst will define:
- Functional requirements (categorized by feature area)
- Non-functional requirements (performance, scalability, reliability)
- Constraints and assumptions
- Acceptance criteria
- Requirements traceability matrix

## Success Criteria for Phase 1

### Vision Document Must Include:
- Clear product vision statement
- Target user personas (at least 3)
- Core value proposition
- Primary use cases (at least 5)
- User journey maps
- Success metrics and KPIs
- Market positioning

### Requirements Document Must Include:
- Comprehensive functional requirements (categorized)
- Complete non-functional requirements
- Requirements priority classification
- Acceptance criteria for each requirement
- Traceability to use cases
- Constraints documentation
- Assumptions documented

## Validation Criteria for Phase 1 Gate

The orchestrator will validate:

1. **Completeness:** All required sections present in both documents
2. **Consistency:** Vision aligns with requirements, no contradictions
3. **Clarity:** Documents readable by both technical and business stakeholders
4. **Actionability:** Requirements specific enough to guide architecture design
5. **Measurability:** Success metrics clearly defined and measurable
6. **Feasibility:** Requirements achievable within project constraints
7. **Traceability:** Clear linkage between use cases and requirements

## Context for Agent Invocations

### For prd-product-vision-specialist:

**Your Task:** Define the product vision, target users, use cases, and value proposition for Abathur.

**Key Considerations:**
- Abathur is named after the evolution master from StarCraft
- Focus on developer productivity and agent orchestration
- CLI-first design philosophy
- Local development tool (not cloud service)
- Emphasis on flexibility and extensibility

**Required Deliverables:**
- Product vision statement
- Target user personas
- Core use cases
- User journey maps
- Value proposition
- Success metrics

**Output Location:** `/Users/odgrim/dev/home/agentics/abathur/prd_deliverables/01_PRODUCT_VISION.md`

### For prd-requirements-analyst:

**Your Task:** Document comprehensive functional and non-functional requirements based on the vision document.

**Key Considerations:**
- Reference architectural decisions from DECISION_POINTS.md
- Ensure requirements support all core functionality areas
- Include performance requirements from targets
- Document constraints and assumptions
- Create traceability matrix

**Required Deliverables:**
- Functional requirements (categorized)
- Non-functional requirements
- Requirements priority classification
- Acceptance criteria
- Traceability matrix
- Constraints documentation

**Input Dependency:** Vision document from prd-product-vision-specialist

**Output Location:** `/Users/odgrim/dev/home/agentics/abathur/prd_deliverables/02_REQUIREMENTS.md`

## Reference Documents

- **Decision Points:** `/Users/odgrim/dev/home/agentics/abathur/DECISION_POINTS.md`
- **Orchestrator Handoff:** `/Users/odgrim/dev/home/agentics/abathur/PRD_ORCHESTRATOR_HANDOFF.md`
- **Working Directory:** `/Users/odgrim/dev/home/agentics/abathur`

## Next Steps After Phase 1

Upon successful validation of Phase 1 deliverables:
- Phase 2: Technical Architecture & Design
- Agents: prd-technical-architect, prd-system-design-specialist, prd-api-cli-specialist
- Focus: System architecture, orchestration algorithms, API/CLI specifications

---

**Status:** Ready for Phase 1 agent invocations
**Date:** 2025-10-09
**Orchestrator:** prd-project-orchestrator
